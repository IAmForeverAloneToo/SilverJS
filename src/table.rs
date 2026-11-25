//! The ordered hash tables behind `Map`, `Set`, `WeakMap`, `WeakSet`,
//! `WeakRef` and `FinalizationRegistry` (ECMA-262 sections 24.1 to 24.4,
//! 26.1 and 26.2): entries in insertion order, a deleted one left as a
//! hole so that an iterator keeps its place, a chained hash index over
//! the entries by SameValueZero, and the holes squeezed out once they
//! outnumber the live entries, each iterator still walking the table
//! moved to its entry's new place.
//!
//! A table is an internal cell ([`crate::internal::InternalCell`]); its
//! object (a `Map`, a `WeakRef`...) names it in an internal slot. What a
//! weak table holds the collector decides (`Heap::collect`): a weak
//! table's value only while its key lives (an ephemeron), a registry's
//! held values always and its targets never.

use crate::heap::{Arena, Heap, StringCell};
use crate::object::ObjectKind;
use crate::value::{RefKind, Value};

/// No entry: the end of a chain, an empty bucket.
const NONE: u32 = u32::MAX;

/// What a table holds of its entries.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Strength {
    /// `Map` and `Set`: keys and values.
    Strong,
    /// `WeakMap`, `WeakSet` and `WeakRef`: a value while its key lives,
    /// never the key.
    Weak,
    /// `FinalizationRegistry`: its cells' held values, never their
    /// targets or unregister tokens.
    Registry,
}

#[derive(Debug, Clone)]
pub(crate) struct Table {
    pub strength: Strength,
    /// The entries in insertion order; a deleted one's key is a hole.
    pub entries: Vec<(Value, Value)>,
    /// Each entry's hash.
    hashes: Vec<u32>,
    /// Each entry's next in its bucket's chain.
    next: Vec<u32>,
    /// The chains' heads: a power of two of them, none before the first
    /// entry.
    buckets: Vec<u32>,
    /// How many entries are not holes.
    pub live: u32,
    /// A registry's cells' unregister tokens (undefined for none), one an
    /// entry.
    pub tokens: Vec<Value>,
    /// A registry's cleanup callback.
    pub cleanup: Value,
    /// A registry's held values whose targets died, for its cleanup job.
    pub cleared: Vec<Value>,
    /// Whether a cleanup job for the registry is queued.
    pub cleanup_queued: bool,
    /// The iterator objects that may still walk the table, by object
    /// index; not held (see [`Heap::table_compact`]).
    iterators: Vec<u32>,
    /// The iterators' count after their last pruning.
    iterators_kept: usize,
}

impl Table {
    pub(crate) fn new(strength: Strength) -> Table {
        Table {
            strength,
            entries: Vec::new(),
            hashes: Vec::new(),
            next: Vec::new(),
            buckets: Vec::new(),
            live: 0,
            tokens: Vec::new(),
            cleanup: Value::UNDEFINED,
            cleared: Vec::new(),
            cleanup_queued: false,
            iterators: Vec::new(),
            iterators_kept: 0,
        }
    }

    /// The bytes it owns beyond its cell.
    pub(crate) fn extra_bytes(&self) -> usize {
        self.entries.capacity() * 16
            + (self.hashes.capacity() + self.next.capacity() + self.buckets.capacity()) * 4
            + (self.tokens.capacity() + self.cleared.capacity()) * 8
            + self.iterators.capacity() * 4
    }

    /// The values a strong table holds; a weak one's live entries (which
    /// the collector has kept by then), for the verifier.
    pub(crate) fn trace(&self, out: &mut Vec<Value>) {
        for &(k, v) in &self.entries {
            if k != Value::HOLE {
                out.push(k);
                out.push(v);
            }
        }
        out.extend(self.tokens.iter().filter(|t| **t != Value::HOLE));
        out.push(self.cleanup);
        out.extend_from_slice(&self.cleared);
    }

    /// The values a weak table holds whatever its keys: a registry's held
    /// values and callback.
    pub(crate) fn trace_held(&self, out: &mut Vec<Value>) {
        if self.strength == Strength::Registry {
            out.extend(
                self.entries
                    .iter()
                    .filter(|e| e.0 != Value::HOLE)
                    .map(|e| e.1),
            );
            out.push(self.cleanup);
            out.extend_from_slice(&self.cleared);
        }
    }

    fn find(&self, hash: u32, eq: impl Fn(Value) -> bool) -> Option<usize> {
        let mask = self.buckets.len().checked_sub(1)?;
        let mut at = *self.buckets.get(hash as usize & mask)?;
        while at != NONE {
            let i = at as usize;
            if self.hashes.get(i) == Some(&hash) && self.entries.get(i).is_some_and(|e| eq(e.0)) {
                return Some(i);
            }
            at = self.next.get(i).copied().unwrap_or(NONE);
        }
        None
    }

    fn push(&mut self, hash: u32, key: Value, value: Value) {
        if self.entries.len() >= self.buckets.len() {
            self.rehash((self.buckets.len() * 2).max(8));
        }
        let i = self.entries.len() as u32;
        let mask = self.buckets.len() - 1;
        let head = self
            .buckets
            .get(hash as usize & mask)
            .copied()
            .unwrap_or(NONE);
        self.entries.push((key, value));
        self.hashes.push(hash);
        self.next.push(head);
        if let Some(b) = self.buckets.get_mut(hash as usize & mask) {
            *b = i;
        }
        self.live += 1;
    }

    fn rehash(&mut self, size: usize) {
        self.buckets.clear();
        self.buckets.resize(size, NONE);
        let mask = size - 1;
        for i in 0..self.entries.len() {
            let live = self.entries.get(i).is_some_and(|e| e.0 != Value::HOLE);
            let h = self.hashes.get(i).copied().unwrap_or(0) as usize & mask;
            let head = if live {
                let head = self.buckets.get(h).copied().unwrap_or(NONE);
                if let Some(b) = self.buckets.get_mut(h) {
                    *b = i as u32;
                }
                head
            } else {
                NONE
            };
            if let Some(n) = self.next.get_mut(i) {
                *n = head;
            }
        }
    }

    /// Makes entry `i` a hole, out of its chain.
    pub(crate) fn remove_at(&mut self, i: usize) {
        let Some(&(k, _)) = self.entries.get(i) else {
            return;
        };
        if k == Value::HOLE {
            return;
        }
        let mask = self.buckets.len().saturating_sub(1);
        let h = self.hashes.get(i).copied().unwrap_or(0) as usize & mask;
        let after = self.next.get(i).copied().unwrap_or(NONE);
        if self.buckets.get(h) == Some(&(i as u32)) {
            if let Some(b) = self.buckets.get_mut(h) {
                *b = after;
            }
        } else {
            let mut at = self.buckets.get(h).copied().unwrap_or(NONE);
            while at != NONE {
                let n = self.next.get(at as usize).copied().unwrap_or(NONE);
                if n == i as u32 {
                    if let Some(slot) = self.next.get_mut(at as usize) {
                        *slot = after;
                    }
                    break;
                }
                at = n;
            }
        }
        if let Some(e) = self.entries.get_mut(i) {
            *e = (Value::HOLE, Value::HOLE);
        }
        if let Some(t) = self.tokens.get_mut(i) {
            *t = Value::HOLE;
        }
        self.live -= 1;
    }

    /// Every entry made a hole.
    pub(crate) fn clear(&mut self) {
        self.entries.fill((Value::HOLE, Value::HOLE));
        self.tokens.fill(Value::HOLE);
        self.buckets.iter_mut().for_each(|b| *b = NONE);
        self.live = 0;
    }

    /// The first entry at or after `from` that is not a hole.
    pub(crate) fn next_live(&self, from: usize) -> Option<(usize, Value, Value)> {
        self.entries
            .iter()
            .enumerate()
            .skip(from)
            .find(|(_, e)| e.0 != Value::HOLE)
            .map(|(i, e)| (i, e.0, e.1))
    }

    /// Whether the holes are many enough to squeeze out.
    fn worth_compacting(&self) -> bool {
        let holes = self.entries.len() - self.live as usize;
        holes >= 16 && holes > self.live as usize
    }

    /// Squeezes the holes out; returns, for each old index (and the end),
    /// how many holes stood before it.
    fn compact(&mut self) -> Vec<u32> {
        let mut before = Vec::with_capacity(self.entries.len() + 1);
        let mut holes = 0u32;
        let mut w = 0;
        for r in 0..self.entries.len() {
            before.push(holes);
            let e = self
                .entries
                .get(r)
                .copied()
                .unwrap_or((Value::HOLE, Value::HOLE));
            if e.0 == Value::HOLE {
                holes += 1;
                continue;
            }
            let h = self.hashes.get(r).copied().unwrap_or(0);
            let t = self.tokens.get(r).copied();
            if let Some(slot) = self.entries.get_mut(w) {
                *slot = e;
            }
            if let Some(slot) = self.hashes.get_mut(w) {
                *slot = h;
            }
            if let (Some(t), Some(slot)) = (t, self.tokens.get_mut(w)) {
                *slot = t;
            }
            w += 1;
        }
        before.push(holes);
        self.entries.truncate(w);
        self.hashes.truncate(w);
        self.next.truncate(w);
        if self.tokens.len() > w {
            self.tokens.truncate(w);
        }
        let size = self.buckets.len().max(8);
        let size = if w * 4 < size && size > 8 {
            size / 2
        } else {
            size
        };
        self.rehash(size);
        before
    }
}

/// What a key's hash and comparison read: the arenas of the primitives
/// compared by their contents.
#[derive(Clone, Copy)]
pub(crate) struct Prims<'a> {
    strings: &'a Arena<StringCell>,
    bigints: &'a Arena<crate::bigint::BigInt>,
}

fn bigint_of(p: Prims<'_>, v: Value) -> Option<&crate::bigint::BigInt> {
    match v.as_reference() {
        Some((RefKind::BigInt, i)) => p.bigints.get(i),
        _ => None,
    }
}

/// A key's hash, alike for keys SameValueZero calls the same.
fn hash_of(p: Prims<'_>, v: Value) -> u32 {
    let strings = p.strings;
    if let Some(n) = v.as_number() {
        let bits = if n.is_nan() {
            0x7ff8_0000_0000_0000
        } else if n == 0.0 {
            0
        } else {
            n.to_bits()
        };
        return mix(bits);
    }
    if let Some(s) = v.as_string() {
        return strings.get(s).map_or(0, StringCell::hash);
    }
    if let Some(b) = bigint_of(p, v) {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        b.hash(&mut h);
        return mix(h.finish());
    }
    mix(v.bits())
}

fn mix(bits: u64) -> u32 {
    let x = bits.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    (x >> 32) as u32 ^ x as u32
}

/// SameValueZero (section 7.2.11).
pub(crate) fn same_value_zero(p: Prims<'_>, a: Value, b: Value) -> bool {
    let strings = p.strings;
    if let (Some(x), Some(y)) = (a.as_number(), b.as_number()) {
        return x == y || (x.is_nan() && y.is_nan());
    }
    if let (Some(x), Some(y)) = (a.as_string(), b.as_string()) {
        return match (strings.get(x), strings.get(y)) {
            (Some(x), Some(y)) => x.same_units(y),
            _ => false,
        };
    }
    if let (Some(x), Some(y)) = (bigint_of(p, a), bigint_of(p, b)) {
        return x == y;
    }
    a == b
}

impl Heap {
    /// The table internal cell `t` names.
    pub(crate) fn table(&self, t: Value) -> Option<&Table> {
        match t.as_reference() {
            Some((RefKind::Internal, i)) => match self.internals.get(i)? {
                crate::internal::InternalCell::Table(table) => Some(table),
                _ => None,
            },
            _ => None,
        }
    }

    /// Runs `f` on table `t` (and the strings, for its keys), keeping the
    /// SameValueZero (section 7.2.11) of two values.
    pub(crate) fn same_value_zero(&self, a: Value, b: Value) -> bool {
        let prims = Prims {
            strings: &self.strings,
            bigints: &self.bigints,
        };
        same_value_zero(prims, a, b)
    }

    /// heap's count of its bytes right.
    fn with_table<R>(&mut self, t: Value, f: impl FnOnce(&mut Table, Prims<'_>) -> R) -> Option<R> {
        let Some((RefKind::Internal, i)) = t.as_reference() else {
            return None;
        };
        let prims = Prims {
            strings: &self.strings,
            bigints: &self.bigints,
        };
        let crate::internal::InternalCell::Table(table) = self.internals.get_mut(i)? else {
            return None;
        };
        let before = table.extra_bytes();
        let r = f(table, prims);
        let after = table.extra_bytes();
        self.internals.update_extra(before, after);
        Some(r)
    }

    /// The index of `key`'s entry in table `t`.
    pub(crate) fn table_find(&self, t: Value, key: Value) -> Option<usize> {
        let table = self.table(t)?;
        let prims = Prims {
            strings: &self.strings,
            bigints: &self.bigints,
        };
        let h = hash_of(prims, key);
        table.find(h, |k| same_value_zero(prims, k, key))
    }

    /// `key`'s value in table `t`.
    pub(crate) fn table_get(&self, t: Value, key: Value) -> Option<Value> {
        let i = self.table_find(t, key)?;
        self.table(t)?.entries.get(i).map(|e| e.1)
    }

    /// Sets `key`'s value in table `t`, adding the entry at the end if it
    /// has none.
    pub(crate) fn table_set(&mut self, t: Value, key: Value, value: Value) {
        self.with_table(t, |table, prims| {
            let h = hash_of(prims, key);
            match table.find(h, |k| same_value_zero(prims, k, key)) {
                Some(i) => {
                    if let Some(e) = table.entries.get_mut(i) {
                        e.1 = value;
                    }
                }
                None => table.push(h, key, value),
            }
        });
    }

    /// Adds a registry's cell: its target, held value and token.
    pub(crate) fn table_register(&mut self, t: Value, target: Value, held: Value, token: Value) {
        self.with_table(t, |table, prims| {
            let h = hash_of(prims, target);
            table.push(h, target, held);
            table
                .tokens
                .resize(table.entries.len() - 1, Value::UNDEFINED);
            table.tokens.push(token);
        });
    }

    /// Deletes `key`'s entry from table `t`; whether it had one.
    pub(crate) fn table_delete(&mut self, t: Value, key: Value) -> bool {
        let Some(i) = self.table_find(t, key) else {
            return false;
        };
        self.with_table(t, |table, _| table.remove_at(i));
        self.table_compact(t);
        true
    }

    /// Deletes a registry's cells whose token is `token`; whether any.
    pub(crate) fn table_unregister(&mut self, t: Value, token: Value) -> bool {
        let found = self
            .with_table(t, |table, _| {
                let doomed: Vec<usize> = table
                    .tokens
                    .iter()
                    .enumerate()
                    .filter(|(_, k)| **k == token)
                    .map(|(i, _)| i)
                    .collect();
                for &i in &doomed {
                    table.remove_at(i);
                }
                !doomed.is_empty()
            })
            .unwrap_or(false);
        self.table_compact(t);
        found
    }

    /// Makes every entry of table `t` a hole.
    pub(crate) fn table_clear(&mut self, t: Value) {
        self.with_table(t, |table, _| table.clear());
        self.table_compact(t);
    }

    /// Table `t`'s first entry at or after `from` that is not a hole.
    pub(crate) fn table_next(&self, t: Value, from: usize) -> Option<(usize, Value, Value)> {
        self.table(t)?.next_live(from)
    }

    /// Notes that iterator object `o` walks table `t`, so that a
    /// compaction moves it; the dead and finished ones are dropped from
    /// the list as it doubles.
    pub(crate) fn table_add_iterator(&mut self, t: Value, o: u32) {
        let alive = self.live_iterators(t);
        self.with_table(t, |table, _| {
            if table.iterators.len() >= (table.iterators_kept * 2).max(16) {
                table.iterators = alive;
                table.iterators_kept = table.iterators.len();
            }
            table.iterators.push(o);
        });
    }

    /// The iterators of table `t` that still live and walk it, each once.
    fn live_iterators(&self, t: Value) -> Vec<u32> {
        let Some(table) = self.table(t) else {
            return Vec::new();
        };
        let mut alive: Vec<u32> = table
            .iterators
            .iter()
            .copied()
            .filter(|&o| {
                self.objects.get(o).is_some_and(|c| {
                    matches!(c.kind(), ObjectKind::MapIterator | ObjectKind::SetIterator)
                        && c.internal().first() == Some(&t)
                })
            })
            .collect();
        alive.sort_unstable();
        alive.dedup();
        alive
    }

    /// Squeezes table `t`'s holes out if they are many, and moves its
    /// iterators to their entries' new places.
    pub(crate) fn table_compact(&mut self, t: Value) {
        if !self.table(t).is_some_and(Table::worth_compacting) {
            return;
        }
        let alive = self.live_iterators(t);
        let Some(before) = self.with_table(t, |table, _| {
            table.iterators.clone_from(&alive);
            table.iterators_kept = alive.len();
            table.compact()
        }) else {
            return;
        };
        let end = before.len().saturating_sub(1);
        for o in alive {
            if let Some(cell) = self.objects.get_mut(o) {
                let at = cell
                    .internal()
                    .get(1)
                    .and_then(|v| v.as_number())
                    .unwrap_or(0.0) as usize;
                let shift = before.get(at.min(end)).copied().unwrap_or(0) as usize;
                cell.set_internal(1, Value::number((at - shift) as f64));
            }
        }
    }
}
