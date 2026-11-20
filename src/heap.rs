//! The heap (docs/design.md sections 1.3 and 1.4): typed arenas of
//! cells named by `u32` index, and a precise, non-moving mark-sweep
//! collector, all in safe Rust.
//!
//! - Every cell carries a generation, bumped when it is freed, which the
//!   long-lived handles check ([`crate::agent`]).
//! - Marking is iterative, with a stack of its own: no structure of any
//!   depth recurses.
//! - Sweeping is lazy: after marking, an arena frees its unmarked cells
//!   as it next needs one, so a collection's pause is its marking. A
//!   cell made while sweep is under way is marked, so that it lives.
//! - In debug builds and under `gc-stress`, a verifier checks after each
//!   marking that every value a marked cell holds names a live cell.
//!
//! The cells: objects ([`crate::object`]), strings
//! ([`crate::string`]), symbols, and the interpreter's internal cells
//! ([`crate::internal`]). BigInts come with J4.

pub(crate) use crate::internal::InternalCell;
pub(crate) use crate::object::ObjectCell;
pub(crate) use crate::string::StringCell;
use crate::string::{Atoms, PropertyKey};
use crate::value::{RefKind, Value};

/// What a cell reaches, for marking.
pub(crate) trait Trace {
    /// Pushes every value the cell holds.
    fn trace(&self, out: &mut Vec<Value>);
    /// Bytes the cell owns outside its slot (vectors, boxes), for the
    /// heap's count.
    fn extra_bytes(&self) -> usize {
        0
    }
}

struct Slot<T> {
    generation: u32,
    cell: Option<T>,
}

/// An arena of cells of one type.
pub(crate) struct Arena<T> {
    slots: Vec<Slot<T>>,
    /// One bit a slot: marked by the last collection, or made since.
    marks: Vec<u64>,
    /// Slots freed and not yet reused.
    free: Vec<u32>,
    /// Where lazy sweeping has reached since the last collection; all
    /// slots from here up are unswept.
    sweep: usize,
    /// Cells occupied, and the bytes they own outside their slots.
    live: usize,
    extra: usize,
    /// Of those, the ones the last collection left unmarked and the
    /// sweep has yet to free: dead already, counted out of the heap's
    /// size so that a limit checked right after a collection sees the
    /// room it made.
    dead: usize,
    dead_extra: usize,
    /// What the marking under way has kept.
    kept: usize,
    kept_extra: usize,
}

fn bit(index: usize) -> (usize, u64) {
    (index / 64, 1u64 << (index % 64))
}

impl<T: Trace> Arena<T> {
    pub(crate) fn new() -> Arena<T> {
        Arena {
            slots: Vec::new(),
            marks: Vec::new(),
            free: Vec::new(),
            sweep: 0,
            live: 0,
            extra: 0,
            dead: 0,
            dead_extra: 0,
            kept: 0,
            kept_extra: 0,
        }
    }

    fn marked(&self, index: usize) -> bool {
        let (word, mask) = bit(index);
        self.marks.get(word).is_some_and(|w| w & mask != 0)
    }

    fn set_mark(&mut self, index: usize) -> bool {
        let (word, mask) = bit(index);
        if self.marks.len() <= word {
            self.marks.resize(word + 1, 0);
        }
        match self.marks.get_mut(word) {
            Some(w) if *w & mask == 0 => {
                *w |= mask;
                true
            }
            _ => false,
        }
    }

    /// The bytes the arena's live cells take (dead ones awaiting the
    /// sweep not counted).
    pub(crate) fn bytes(&self) -> usize {
        (self.live - self.dead) * std::mem::size_of::<Slot<T>>() + self.extra - self.dead_extra
    }

    /// The bytes the arena has taken from the allocator: its slots,
    /// marks and free list at capacity, and what the cells own (dead ones
    /// awaiting the sweep included).
    pub(crate) fn committed(&self) -> usize {
        self.slots.capacity() * std::mem::size_of::<Slot<T>>()
            + self.marks.capacity() * 8
            + self.free.capacity() * 4
            + self.extra
    }

    /// How many cells live.
    pub(crate) fn len(&self) -> usize {
        self.live - self.dead
    }

    /// Frees the slot at `index` if it holds a cell the last collection
    /// left unmarked; whether it did.
    fn sweep_one(&mut self, index: usize) -> bool {
        if self.marked(index) {
            return false;
        }
        let Some(slot) = self.slots.get_mut(index) else {
            return false;
        };
        match slot.cell.take() {
            Some(cell) => {
                slot.generation = slot.generation.wrapping_add(1);
                let extra = cell.extra_bytes();
                self.live -= 1;
                self.extra -= extra;
                self.dead = self.dead.saturating_sub(1);
                self.dead_extra = self.dead_extra.saturating_sub(extra);
                true
            }
            None => false,
        }
    }

    /// Sweeps every slot not yet swept (before the next marking).
    fn finish_sweep(&mut self) {
        while self.sweep < self.slots.len() {
            let index = self.sweep;
            self.sweep += 1;
            if self.sweep_one(index) {
                self.free.push(index as u32);
            }
        }
    }

    /// Places `cell`, reusing a free or newly swept slot before growing:
    /// its index and generation.
    pub(crate) fn alloc(&mut self, cell: T) -> (u32, u32) {
        let index = match self.free.pop() {
            Some(i) => Some(i as usize),
            None => {
                let mut found = None;
                while self.sweep < self.slots.len() {
                    let i = self.sweep;
                    self.sweep += 1;
                    if self.sweep_one(i) {
                        found = Some(i);
                        break;
                    }
                }
                found
            }
        };
        self.live += 1;
        self.extra += cell.extra_bytes();
        let index = match index {
            Some(i) => i,
            None => {
                self.slots.push(Slot {
                    generation: 0,
                    cell: None,
                });
                self.sweep = self.slots.len();
                self.slots.len() - 1
            }
        };
        // Made now, it outlives the sweep under way.
        self.set_mark(index);
        let generation = match self.slots.get_mut(index) {
            Some(slot) => {
                slot.cell = Some(cell);
                slot.generation
            }
            None => 0,
        };
        (u32::try_from(index).unwrap_or(u32::MAX), generation)
    }

    /// The cell at `index`, if one lives there: not freed, and not left
    /// unmarked by the last collection where the sweep has yet to reach.
    pub(crate) fn get(&self, index: u32) -> Option<&T> {
        let i = index as usize;
        if i >= self.sweep && !self.marked(i) {
            return None;
        }
        self.slots.get(i).and_then(|s| s.cell.as_ref())
    }

    pub(crate) fn get_mut(&mut self, index: u32) -> Option<&mut T> {
        let i = index as usize;
        if i >= self.sweep && !self.marked(i) {
            return None;
        }
        self.slots.get_mut(i).and_then(|s| s.cell.as_mut())
    }

    /// Rewrites the cell's count of owned bytes after it changed.
    pub(crate) fn update_extra(&mut self, before: usize, after: usize) {
        self.extra = self.extra + after - before;
    }

    /// The generation of the slot at `index` while it holds a live cell.
    pub(crate) fn generation(&self, index: u32) -> Option<u32> {
        self.get(index)?;
        self.slots.get(index as usize).map(|s| s.generation)
    }

    /// Begins a collection: the last sweep is finished and every mark
    /// cleared.
    fn begin_collection(&mut self) {
        self.finish_sweep();
        self.dead = 0;
        self.dead_extra = 0;
        self.kept = 0;
        self.kept_extra = 0;
        self.marks.iter_mut().for_each(|w| *w = 0);
    }

    /// Marks the cell at `index` for the collection under way, counting
    /// what it keeps; whether it was newly marked (and so is to be
    /// traced).
    fn keep(&mut self, index: u32) -> bool {
        let Some(extra) = self.get_raw(index).map(Trace::extra_bytes) else {
            return false;
        };
        if !self.set_mark(index as usize) {
            return false;
        }
        self.kept += 1;
        self.kept_extra += extra;
        true
    }

    /// Ends one: sweeping starts over from the arena's first slot (the
    /// free list keeps what earlier sweeps freed: only an occupied slot
    /// is ever swept, so none is listed twice).
    fn end_collection(&mut self) {
        self.sweep = 0;
        self.dead = self.live - self.kept;
        self.dead_extra = self.extra - self.kept_extra;
    }
}

impl Trace for StringCell {
    fn trace(&self, _: &mut Vec<Value>) {}

    fn extra_bytes(&self) -> usize {
        self.owned_bytes()
    }
}

/// A symbol: its description (a string, or undefined).
pub(crate) struct SymbolCell {
    pub(crate) description: Value,
}

impl Trace for SymbolCell {
    fn trace(&self, out: &mut Vec<Value>) {
        out.push(self.description);
    }
}

/// What the heap holds, by kind.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HeapStats {
    pub objects: usize,
    pub strings: usize,
    pub symbols: usize,
    /// The bytes the live cells take, and what they own: the heap in use
    /// (`performance.memory`'s `usedJSHeapSize`).
    pub bytes: usize,
    /// The bytes the heap has taken from the allocator: the arenas at
    /// capacity, the dead cells not yet swept, and the interpreter's
    /// register file (`totalJSHeapSize`).
    pub committed: usize,
    pub collections: u64,
}

/// The heap of one agent.
pub(crate) struct Heap {
    pub(crate) objects: Arena<ObjectCell>,
    pub(crate) strings: Arena<StringCell>,
    pub(crate) symbols: Arena<SymbolCell>,
    pub(crate) internals: Arena<InternalCell>,
    pub(crate) atoms: Atoms,
    /// The key `length`, which arrays answer for themselves.
    pub(crate) length_key: PropertyKey,
    pub(crate) collections: u64,
}

impl Heap {
    pub(crate) fn new() -> Heap {
        let mut atoms = Atoms::default();
        let length_key = PropertyKey::Atom(atoms.intern_str("length"));
        Heap {
            objects: Arena::new(),
            strings: Arena::new(),
            symbols: Arena::new(),
            internals: Arena::new(),
            atoms,
            length_key,
            collections: 0,
        }
    }

    pub(crate) fn bytes(&self) -> usize {
        self.objects.bytes()
            + self.strings.bytes()
            + self.symbols.bytes()
            + self.internals.bytes()
            + self.atoms.bytes()
    }

    pub(crate) fn stats(&self) -> HeapStats {
        HeapStats {
            objects: self.objects.len(),
            strings: self.strings.len(),
            symbols: self.symbols.len(),
            bytes: self.bytes(),
            committed: self.objects.committed()
                + self.strings.committed()
                + self.symbols.committed()
                + self.internals.committed()
                + self.atoms.bytes(),
            collections: self.collections,
        }
    }

    /// Runs `f` on the internal cell `i` and keeps the heap's count of
    /// its bytes right.
    pub(crate) fn with_internal<R>(
        &mut self,
        i: u32,
        f: impl FnOnce(&mut InternalCell) -> R,
    ) -> Option<R> {
        let cell = self.internals.get_mut(i)?;
        let before = cell.extra_bytes();
        let result = f(cell);
        let after = cell.extra_bytes();
        self.internals.update_extra(before, after);
        Some(result)
    }

    /// Whether `value` names a cell that lives (any non-reference does).
    pub(crate) fn is_live(&self, value: Value) -> bool {
        match value.as_reference() {
            None => true,
            Some((RefKind::Object, i)) => self.objects.get(i).is_some(),
            Some((RefKind::String, i)) => self.strings.get(i).is_some(),
            Some((RefKind::Symbol, i)) => self.symbols.get(i).is_some(),
            Some((RefKind::Internal, i)) => self.internals.get(i).is_some(),
            // BigInts have no cells before J4.
            Some(_) => false,
        }
    }

    /// Whether `value` names a cell the marking under way has kept; for
    /// the verifier.
    fn kept(&self, value: Value) -> bool {
        match value.as_reference() {
            None => true,
            Some((RefKind::Object, j)) => {
                self.objects.get_raw(j).is_some() && self.objects.marked(j as usize)
            }
            Some((RefKind::String, j)) => {
                self.strings.get_raw(j).is_some() && self.strings.marked(j as usize)
            }
            Some((RefKind::Symbol, j)) => {
                self.symbols.get_raw(j).is_some() && self.symbols.marked(j as usize)
            }
            Some((RefKind::Internal, j)) => {
                self.internals.get_raw(j).is_some() && self.internals.marked(j as usize)
            }
            Some(_) => false,
        }
    }

    /// Marks everything reachable from `roots`, with a stack of its own,
    /// then leaves the unmarked cells to the arenas' lazy sweeps.
    pub(crate) fn collect(&mut self, roots: impl IntoIterator<Item = Value>) {
        self.objects.begin_collection();
        self.strings.begin_collection();
        self.symbols.begin_collection();
        self.internals.begin_collection();
        let mut stack: Vec<Value> = roots.into_iter().collect();
        while let Some(value) = stack.pop() {
            match value.as_reference() {
                Some((RefKind::Object, i)) => {
                    if self.objects.keep(i)
                        && let Some(cell) = self.objects.get_raw(i)
                    {
                        cell.trace(&mut stack);
                    }
                }
                Some((RefKind::String, i)) => {
                    self.strings.keep(i);
                }
                Some((RefKind::Symbol, i)) => {
                    if self.symbols.keep(i)
                        && let Some(cell) = self.symbols.get_raw(i)
                    {
                        cell.trace(&mut stack);
                    }
                }
                Some((RefKind::Internal, i)) => {
                    if self.internals.keep(i)
                        && let Some(cell) = self.internals.get_raw(i)
                    {
                        cell.trace(&mut stack);
                    }
                }
                _ => {}
            }
        }
        if cfg!(any(debug_assertions, feature = "gc-stress")) {
            self.verify();
        }
        self.objects.end_collection();
        self.strings.end_collection();
        self.symbols.end_collection();
        self.internals.end_collection();
        self.collections += 1;
    }

    /// The heap verifier: every value a marked cell holds names a marked,
    /// occupied cell. A failure is a bug of the engine's own rooting or
    /// tracing, and panics.
    fn verify(&self) {
        let mut held = Vec::new();
        for (i, slot) in self.objects.slots.iter().enumerate() {
            if let Some(cell) = &slot.cell
                && self.objects.marked(i)
            {
                held.clear();
                cell.trace(&mut held);
                for value in &held {
                    assert!(
                        self.kept(*value),
                        "heap verifier: object {i} holds {value:?}, not a live cell"
                    );
                }
            }
        }
        for (i, slot) in self.internals.slots.iter().enumerate() {
            if let Some(cell) = &slot.cell
                && self.internals.marked(i)
            {
                held.clear();
                cell.trace(&mut held);
                for value in &held {
                    assert!(
                        self.kept(*value),
                        "heap verifier: internal cell {i} holds {value:?}, not a live cell"
                    );
                }
            }
        }
        for (i, slot) in self.symbols.slots.iter().enumerate() {
            if let Some(cell) = &slot.cell
                && self.symbols.marked(i)
            {
                assert!(
                    self.kept(cell.description),
                    "heap verifier: symbol {i}'s description is not a live cell"
                );
            }
        }
    }
}

impl<T: Trace> Arena<T> {
    /// The cell at `index` whether or not the last collection marked it:
    /// for marking itself.
    fn get_raw(&self, index: u32) -> Option<&T> {
        self.slots.get(index as usize).and_then(|s| s.cell.as_ref())
    }
}
