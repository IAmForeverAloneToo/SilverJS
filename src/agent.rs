//! An agent and its handles (docs/design.md sections 1.4 and 1.5).
//!
//! An agent owns one heap. Code that holds engine values does so through
//! handles the collector knows:
//!
//! - a [`Scope`] owns a frame of the agent's handle stack, and a
//!   [`Local`] is an index into it, `Copy`, valid while its scope lives
//!   (the borrow checker ties the two), scanned by every collection;
//! - a [`Global`] is a strong entry of the agent's root table, held by
//!   Rust structures across scopes, and a [`Weak`] the same without
//!   keeping its target. Both carry the cell's generation and the
//!   agent's id, and check both on every use, in release builds too: a
//!   stale or foreign handle panics rather than read the wrong cell.
//!
//! Anything that may collect takes the scope by `&mut`, and values are
//! read out of it only as handles or plain data, so no unrooted value
//! survives a collection.
//!
//! J2.1's cells are placeholders (an object is a prototype and a vector
//! of values); J2.2 brings strings and objects of their own.

use std::marker::PhantomData;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::heap::{Heap, HeapStats, ObjectCell, StringCell};
use crate::value::{RefKind, Value};

static NEXT_AGENT: AtomicU32 = AtomicU32::new(1);

/// How an agent is set up.
#[derive(Debug, Clone, Copy)]
pub struct AgentOptions {
    /// The bytes the heap may hold; an allocation past it collects first,
    /// and fails with [`OutOfMemory`] if that does not make room.
    pub heap_limit: usize,
}

impl Default for AgentOptions {
    fn default() -> AgentOptions {
        AgentOptions {
            heap_limit: 256 << 20,
        }
    }
}

/// The heap limit was reached (script sees a `RangeError`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutOfMemory;

impl std::fmt::Display for OutOfMemory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RangeError: Out of memory")
    }
}

impl std::error::Error for OutOfMemory {}

/// The heap grows by this factor between collections.
const GROWTH: f64 = 1.5;
/// No collection before the heap reaches this.
const FIRST_COLLECTION: usize = 1 << 20;

struct RootEntry {
    value: Value,
    strong: bool,
}

/// An agent: one heap, one thread.
pub struct Agent {
    id: u32,
    heap: Heap,
    handles: Vec<Value>,
    roots: Vec<Option<RootEntry>>,
    free_roots: Vec<u32>,
    limit: usize,
    next_collection: usize,
    /// Collect at every allocation (the `gc-stress` feature, or a test).
    stress: bool,
}

impl Agent {
    pub fn new(options: AgentOptions) -> Agent {
        Agent {
            id: NEXT_AGENT.fetch_add(1, Ordering::Relaxed),
            heap: Heap::new(),
            handles: Vec::new(),
            roots: Vec::new(),
            free_roots: Vec::new(),
            limit: options.heap_limit,
            next_collection: FIRST_COLLECTION,
            stress: cfg!(feature = "gc-stress"),
        }
    }

    /// The agent's id, which its long-lived handles carry.
    pub fn id(&self) -> u32 {
        self.id
    }

    /// Collects at every allocation from now on (as `gc-stress` builds
    /// always do), so that a rooting bug fails the first test reaching it.
    pub fn set_stress(&mut self, stress: bool) {
        self.stress = stress;
    }

    /// Runs `f` with a scope of its own; its locals are dropped after.
    pub fn scope<R>(&mut self, f: impl FnOnce(&mut Scope<'_>) -> R) -> R {
        let base = self.handles.len();
        let mut scope = Scope { agent: self };
        let result = f(&mut scope);
        self.handles.truncate(base);
        result
    }

    /// A collection now: the roots are the handle stack and the strong
    /// entries of the root table.
    pub fn collect(&mut self) {
        let strong = self
            .roots
            .iter()
            .flatten()
            .filter(|r| r.strong)
            .map(|r| r.value);
        let roots: Vec<Value> = self.handles.iter().copied().chain(strong).collect();
        self.heap.collect(roots);
        self.next_collection = ((self.heap.bytes() as f64 * GROWTH) as usize).max(FIRST_COLLECTION);
    }

    pub fn heap_stats(&self) -> HeapStats {
        self.heap.stats()
    }

    /// Makes room for `bytes` more: a collection when the heap has grown
    /// enough since the last (or always, under stress), and the limit.
    fn reserve(&mut self, bytes: usize) -> Result<(), OutOfMemory> {
        let wanted = self.heap.bytes() + bytes;
        if self.stress || wanted > self.next_collection || wanted > self.limit {
            self.collect();
        }
        if self.heap.bytes() + bytes > self.limit {
            return Err(OutOfMemory);
        }
        Ok(())
    }

    /// The generation of the cell `value` names (0 for non-references).
    fn generation_of(&self, value: Value) -> Option<u32> {
        match value.as_reference() {
            None => Some(0),
            Some((RefKind::Object, i)) => self.heap.objects.generation(i),
            Some((RefKind::String, i)) => self.heap.strings.generation(i),
            Some(_) => None,
        }
    }

    fn add_root(&mut self, value: Value, strong: bool) -> u32 {
        let entry = Some(RootEntry { value, strong });
        match self.free_roots.pop() {
            Some(slot) => {
                if let Some(e) = self.roots.get_mut(slot as usize) {
                    *e = entry;
                }
                slot
            }
            None => {
                self.roots.push(entry);
                u32::try_from(self.roots.len() - 1).unwrap_or(u32::MAX)
            }
        }
    }

    /// The root entry a long-lived handle names, checked: its agent, and
    /// that the entry is still there. A failure is a bug of the
    /// embedder's (a handle used in the wrong agent, or after its
    /// release), and panics.
    fn root(&self, agent: u32, slot: u32) -> &RootEntry {
        assert_eq!(
            agent, self.id,
            "a handle of agent {agent} used in agent {}",
            self.id
        );
        match self.roots.get(slot as usize) {
            Some(Some(entry)) => entry,
            _ => panic!("a released handle used"),
        }
    }
}

/// A frame of the handle stack, and the way into the agent while it
/// lives.
pub struct Scope<'a> {
    agent: &'a mut Agent,
}

/// A handle to a value, valid while its scope lives.
pub struct Local<'s> {
    index: u32,
    _scope: PhantomData<&'s ()>,
}

impl Clone for Local<'_> {
    fn clone(&self) -> Self {
        *self
    }
}

impl Copy for Local<'_> {}

/// A strong handle held across scopes; released with
/// [`Scope::release`].
pub struct Global {
    agent: u32,
    slot: u32,
    generation: u32,
}

impl Global {
    /// A second handle to the same entry, for testing that a released
    /// one is refused (the API never makes one).
    #[cfg(test)]
    pub(crate) fn clone_for_test(&self) -> Global {
        Global {
            agent: self.agent,
            slot: self.slot,
            generation: self.generation,
        }
    }
}

/// A handle that does not keep its target alive.
pub struct Weak {
    agent: u32,
    slot: u32,
    generation: u32,
}

impl<'a> Scope<'a> {
    /// The agent's id.
    pub fn agent_id(&self) -> u32 {
        self.agent.id
    }

    /// Runs `f` in a scope nested in this one: its locals are dropped
    /// when it returns, and the ones of this scope stay valid in it.
    pub fn nested<R>(&mut self, f: impl FnOnce(&mut Scope<'_>) -> R) -> R {
        let base = self.agent.handles.len();
        let mut inner = Scope {
            agent: &mut *self.agent,
        };
        let result = f(&mut inner);
        self.agent.handles.truncate(base);
        result
    }

    /// The same, keeping the one local `f` returns: it is moved out into
    /// this scope.
    pub fn escape(&mut self, f: impl for<'b> FnOnce(&mut Scope<'b>) -> Local<'b>) -> Local<'a> {
        let base = self.agent.handles.len();
        let mut inner = Scope {
            agent: &mut *self.agent,
        };
        let kept = f(&mut inner);
        let value = inner.value(kept);
        self.agent.handles.truncate(base);
        self.local(value)
    }

    /// A local for `value`, pushed on the handle stack.
    fn local(&mut self, value: Value) -> Local<'a> {
        self.agent.handles.push(value);
        Local {
            index: u32::try_from(self.agent.handles.len() - 1).unwrap_or(u32::MAX),
            _scope: PhantomData,
        }
    }

    /// The value a local holds. Under `gc-stress` the local is checked to
    /// name a live cell, so that a rooting bug fails where it is.
    fn value(&self, local: Local<'_>) -> Value {
        let value = self
            .agent
            .handles
            .get(local.index as usize)
            .copied()
            .unwrap_or(Value::UNDEFINED);
        if cfg!(feature = "gc-stress") {
            assert!(
                self.agent.heap.is_live(value),
                "gc-stress: a local names a dead cell"
            );
        }
        value
    }

    pub fn undefined(&mut self) -> Local<'a> {
        self.local(Value::UNDEFINED)
    }

    pub fn number(&mut self, n: f64) -> Local<'a> {
        self.local(Value::number(n))
    }

    /// The number a local holds, if it holds one.
    pub fn as_number(&self, local: Local<'_>) -> Option<f64> {
        self.value(local).as_number()
    }

    /// Whether two locals hold the same value (the same cell, for
    /// references).
    pub fn same(&self, a: Local<'_>, b: Local<'_>) -> bool {
        self.value(a) == self.value(b)
    }

    /// A string of `text`'s UTF-16 units.
    pub fn string(&mut self, text: &str) -> Result<Local<'a>, OutOfMemory> {
        let units: Box<[u16]> = text.encode_utf16().collect();
        self.agent.reserve(units.len() * 2 + 32)?;
        let (index, _) = self.agent.heap.strings.alloc(StringCell { units });
        Ok(self.local(Value::reference(RefKind::String, index)))
    }

    /// A string's text, if the local holds a string.
    pub fn string_text(&self, local: Local<'_>) -> Option<String> {
        let index = self.value(local).as_string()?;
        let cell = self.agent.heap.strings.get(index)?;
        Some(String::from_utf16_lossy(&cell.units))
    }

    /// A plain object with prototype `proto` (`None` for null).
    pub fn object(&mut self, proto: Option<Local<'_>>) -> Result<Local<'a>, OutOfMemory> {
        let proto = proto.map_or(Value::NULL, |p| self.value(p));
        self.agent.reserve(64)?;
        let (index, _) = self.agent.heap.objects.alloc(ObjectCell {
            proto,
            slots: Vec::new(),
        });
        Ok(self.local(Value::reference(RefKind::Object, index)))
    }

    /// Appends `value` to an object's slots (J2.1's stand-in for
    /// properties); false if `object` is no object.
    pub fn push_slot(&mut self, object: Local<'_>, value: Local<'_>) -> bool {
        let value = self.value(value);
        let Some(index) = self.value(object).as_object() else {
            return false;
        };
        let objects = &mut self.agent.heap.objects;
        let Some(cell) = objects.get_mut(index) else {
            return false;
        };
        let before = cell.slots.capacity() * std::mem::size_of::<Value>();
        cell.slots.push(value);
        let after = cell.slots.capacity() * std::mem::size_of::<Value>();
        objects.update_extra(before, after);
        true
    }

    /// An object's slot `i`, as a local.
    pub fn slot(&mut self, object: Local<'_>, i: usize) -> Option<Local<'a>> {
        let index = self.value(object).as_object()?;
        let value = *self.agent.heap.objects.get(index)?.slots.get(i)?;
        Some(self.local(value))
    }

    /// An object's prototype, as a local.
    pub fn proto(&mut self, object: Local<'_>) -> Option<Local<'a>> {
        let index = self.value(object).as_object()?;
        let value = self.agent.heap.objects.get(index)?.proto;
        Some(self.local(value))
    }

    /// A strong handle to the local's value, valid across scopes until
    /// released.
    pub fn global(&mut self, local: Local<'_>) -> Global {
        let value = self.value(local);
        let generation = self.agent.generation_of(value).unwrap_or(0);
        let slot = self.agent.add_root(value, true);
        Global {
            agent: self.agent.id,
            slot,
            generation,
        }
    }

    /// The value a global holds, as a local. Panics if the global is of
    /// another agent or its cell is not the one it was made for.
    pub fn get_global(&mut self, global: &Global) -> Local<'a> {
        let value = self.agent.root(global.agent, global.slot).value;
        assert_eq!(
            self.agent.generation_of(value),
            Some(global.generation),
            "a stale handle used"
        );
        self.local(value)
    }

    /// Releases a global: its value may be collected.
    pub fn release(&mut self, global: Global) {
        self.agent.root(global.agent, global.slot);
        if let Some(entry) = self.agent.roots.get_mut(global.slot as usize) {
            *entry = None;
        }
        self.agent.free_roots.push(global.slot);
    }

    /// A weak handle to the local's value.
    pub fn weak(&mut self, local: Local<'_>) -> Weak {
        let value = self.value(local);
        let generation = self.agent.generation_of(value).unwrap_or(0);
        let slot = self.agent.add_root(value, false);
        Weak {
            agent: self.agent.id,
            slot,
            generation,
        }
    }

    /// A weak handle's target if it still lives, as a local; `None` once
    /// it has died. Panics if the handle is of another agent.
    pub fn get_weak(&mut self, weak: &Weak) -> Option<Local<'a>> {
        let value = self.agent.root(weak.agent, weak.slot).value;
        if self.agent.generation_of(value) != Some(weak.generation) {
            return None;
        }
        Some(self.local(value))
    }

    /// Releases a weak handle's entry.
    pub fn release_weak(&mut self, weak: Weak) {
        self.agent.root(weak.agent, weak.slot);
        if let Some(entry) = self.agent.roots.get_mut(weak.slot as usize) {
            *entry = None;
        }
        self.agent.free_roots.push(weak.slot);
    }

    /// A collection now.
    pub fn collect(&mut self) {
        self.agent.collect();
    }

    pub fn heap_stats(&self) -> HeapStats {
        self.agent.heap_stats()
    }
}
