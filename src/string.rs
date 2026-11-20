//! Strings (docs/design.md section 1.3): immutable, with WTF-16
//! semantics (a lone surrogate is a value of its own), held as Latin-1
//! (one byte a unit) when every unit is under 256 and as UTF-16
//! otherwise, each hashed when first compared.
//!
//! A string built by concatenation is a window on a buffer shared with
//! the strings it was built from (J2.4b, the lead's ruling of
//! 2026-11-20, in place of J2's ropes): the buffer keeps the range its
//! windows have used, with room at both ends, grown by doubling. `a + b`
//! writes `b` in place after `a` when `a`'s window ends exactly where the
//! buffer's used range does, and `a` in place before `b` when `b`'s
//! window starts where it starts; otherwise it copies. So `s += x`,
//! `s = x + s`, either alternating, and `a + (b + (c + ...))` copy each
//! unit a constant number of times, amortized, while every string stays
//! flat to read. Branching off one prefix (`p + "x"`, then `p + "y"`)
//! copies the second time: that pattern stays quadratic in a loop until
//! J6's ropes.
//!
//! A buffer's bytes are counted once, by the window that last extended
//! it; the collector copies the live windows of a buffer they cover
//! under a quarter of out to buffers of their own, so that a short string
//! never keeps a long one's buffer.
//!
//! Property keys are [`PropertyKey`]s: an array index as a number, a
//! name as an [`Atom`] (an interned string with an agent-wide id, so
//! that keys compare as integers), or a symbol.

use std::cell::{Cell, Ref, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

/// Below this many units a concatenation copies into a flat string: a
/// buffer is for strings that grow.
pub(crate) const BUFFER_FROM: usize = 64;

/// A buffer's units, in one width.
#[derive(Debug, Clone)]
pub(crate) enum Data {
    Latin1(Vec<u8>),
    Utf16(Vec<u16>),
}

impl Data {
    fn len(&self) -> usize {
        match self {
            Data::Latin1(v) => v.len(),
            Data::Utf16(v) => v.len(),
        }
    }

    fn bytes(&self) -> usize {
        match self {
            Data::Latin1(v) => v.capacity(),
            Data::Utf16(v) => v.capacity() * 2,
        }
    }
}

/// A buffer strings are windows on. Positions are logical: the unit at
/// position `p` is `data[p - origin]`, so that growing at the front moves
/// no window.
#[derive(Debug)]
pub(crate) struct Buffer {
    pub(crate) data: Data,
    pub(crate) origin: isize,
    /// The range some window has used: only a window ending at
    /// `used_end` may write after it, and one starting at `used_start`
    /// before it.
    pub(crate) used_start: isize,
    pub(crate) used_end: isize,
    /// The string cell its bytes are counted by.
    pub(crate) owner: Option<u32>,
}

impl Buffer {
    fn unit(&self, p: isize) -> u16 {
        let i = usize::try_from(p - self.origin).unwrap_or(usize::MAX);
        match &self.data {
            Data::Latin1(v) => v.get(i).map_or(0, |&b| u16::from(b)),
            Data::Utf16(v) => v.get(i).copied().unwrap_or(0),
        }
    }

    pub(crate) fn bytes(&self) -> usize {
        self.data.bytes()
    }

    /// The units the buffer has room for.
    pub(crate) fn data_len(&self) -> usize {
        self.data.len()
    }
}

pub(crate) type Shared = Rc<RefCell<Buffer>>;

/// A string's units.
#[derive(Debug, Clone)]
pub(crate) enum Units {
    Latin1(Box<[u8]>),
    Utf16(Box<[u16]>),
    /// The positions `start..end` of a shared buffer.
    Window {
        buffer: Shared,
        start: isize,
        end: isize,
    },
}

/// A string cell.
#[derive(Debug, Clone)]
pub(crate) struct StringCell {
    pub(crate) units: Units,
    /// FNV-1a over the units, made when first wanted.
    hash: Cell<Option<u32>>,
    /// The bytes the heap counts for it: its own units, or for a window,
    /// its buffer's if it is the buffer's owner, else none.
    pub(crate) accounted: usize,
}

/// FNV-1a over the units as 16-bit values, whichever form holds them,
/// so that equal strings hash alike.
fn hash_units(units: impl Iterator<Item = u16>) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for u in units {
        h ^= u32::from(u);
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

/// A string's units, in order.
pub(crate) enum StringIter<'a> {
    Latin1(std::slice::Iter<'a, u8>),
    Utf16(std::slice::Iter<'a, u16>),
    Window(Ref<'a, Buffer>, isize, isize),
}

impl Iterator for StringIter<'_> {
    type Item = u16;

    fn next(&mut self) -> Option<u16> {
        match self {
            StringIter::Latin1(i) => i.next().map(|&b| u16::from(b)),
            StringIter::Utf16(i) => i.next().copied(),
            StringIter::Window(buffer, at, end) => {
                if *at >= *end {
                    return None;
                }
                let u = buffer.unit(*at);
                *at += 1;
                Some(u)
            }
        }
    }
}

impl StringCell {
    fn flat(units: Units) -> StringCell {
        let accounted = match &units {
            Units::Latin1(b) => b.len(),
            Units::Utf16(u) => u.len() * 2,
            Units::Window { .. } => 0,
        };
        StringCell {
            units,
            hash: Cell::new(None),
            accounted,
        }
    }

    /// A string of `units`, held as Latin-1 when they all fit.
    pub(crate) fn from_units(units: &[u16]) -> StringCell {
        if units.iter().all(|&u| u < 256) {
            StringCell::flat(Units::Latin1(units.iter().map(|&u| u as u8).collect()))
        } else {
            StringCell::flat(Units::Utf16(units.into()))
        }
    }

    pub(crate) fn from_str(text: &str) -> StringCell {
        if text.is_ascii() {
            return StringCell::flat(Units::Latin1(text.as_bytes().into()));
        }
        let units: Vec<u16> = text.encode_utf16().collect();
        StringCell::from_units(&units)
    }

    pub(crate) fn len(&self) -> usize {
        match &self.units {
            Units::Latin1(b) => b.len(),
            Units::Utf16(u) => u.len(),
            Units::Window { start, end, .. } => usize::try_from(end - start).unwrap_or(0),
        }
    }

    pub(crate) fn iter(&self) -> StringIter<'_> {
        match &self.units {
            Units::Latin1(b) => StringIter::Latin1(b.iter()),
            Units::Utf16(u) => StringIter::Utf16(u.iter()),
            Units::Window { buffer, start, end } => {
                StringIter::Window(buffer.borrow(), *start, *end)
            }
        }
    }

    /// The unit at `i`, read in constant time.
    pub(crate) fn unit(&self, i: usize) -> Option<u16> {
        match &self.units {
            Units::Latin1(b) => b.get(i).map(|&b| u16::from(b)),
            Units::Utf16(u) => u.get(i).copied(),
            Units::Window { buffer, start, end } => {
                let p = start + isize::try_from(i).ok()?;
                (p < *end).then(|| buffer.borrow().unit(p))
            }
        }
    }

    pub(crate) fn to_units(&self) -> Vec<u16> {
        self.iter().collect()
    }

    /// The string as Rust text, a lone surrogate replaced.
    pub(crate) fn to_text(&self) -> String {
        String::from_utf16_lossy(&self.to_units())
    }

    /// Whether every unit fits in a byte.
    fn latin1(&self) -> bool {
        match &self.units {
            Units::Latin1(_) => true,
            Units::Utf16(_) => false,
            Units::Window { buffer, .. } => matches!(buffer.borrow().data, Data::Latin1(_)),
        }
    }

    pub(crate) fn hash(&self) -> u32 {
        if let Some(h) = self.hash.get() {
            return h;
        }
        let h = hash_units(self.iter());
        self.hash.set(Some(h));
        h
    }

    /// Two strings' concatenation, flat.
    pub(crate) fn concat(a: &StringCell, b: &StringCell) -> StringCell {
        if a.latin1() && b.latin1() {
            let bytes: Box<[u8]> = a.iter().chain(b.iter()).map(|u| u as u8).collect();
            return StringCell::flat(Units::Latin1(bytes));
        }
        let units: Vec<u16> = a.iter().chain(b.iter()).collect();
        StringCell::from_units(&units)
    }

    pub(crate) fn same_units(&self, other: &StringCell) -> bool {
        self.len() == other.len() && self.hash() == other.hash() && self.iter().eq(other.iter())
    }

    /// The bytes the heap counts for the cell.
    pub(crate) fn owned_bytes(&self) -> usize {
        self.accounted
    }

    /// A flat copy of a window, for the collector (the bytes copied).
    pub(crate) fn to_flat(&self) -> (StringCell, usize) {
        let units = self.to_units();
        let cell = StringCell::from_units(&units);
        let copied = cell.accounted;
        (cell, copied)
    }
}

/// Writes `units` into `data` from index `at`, widening the data to
/// UTF-16 first if a unit needs it (the bytes copied).
fn write_units(data: &mut Data, at: usize, units: &[u16]) -> usize {
    let mut copied = 0;
    if let Data::Latin1(v) = data
        && units.iter().any(|&u| u >= 256)
    {
        let wide: Vec<u16> = v.iter().map(|&b| u16::from(b)).collect();
        copied += wide.len() * 2;
        *data = Data::Utf16(wide);
    }
    match data {
        Data::Latin1(v) => {
            for (i, &u) in units.iter().enumerate() {
                if let Some(slot) = v.get_mut(at + i) {
                    *slot = u as u8;
                }
            }
            copied += units.len();
        }
        Data::Utf16(v) => {
            if let Some(dst) = v.get_mut(at..at + units.len()) {
                dst.copy_from_slice(units);
            }
            copied += units.len() * 2;
        }
    }
    copied
}

/// Grows `buffer` to hold `before` more units in front of its origin's
/// used range and `after` more behind it, doubling (the bytes copied).
fn grow(buffer: &mut Buffer, before: usize, after: usize) -> usize {
    let used = usize::try_from(buffer.used_end - buffer.used_start).unwrap_or(0);
    let front_room = usize::try_from(buffer.used_start - buffer.origin).unwrap_or(0);
    let back_room = buffer.data.len().saturating_sub(front_room + used);
    if front_room >= before && back_room >= after {
        return 0;
    }
    let new_front = if front_room >= before {
        front_room
    } else {
        (used + before).max(before * 2)
    };
    let new_back = if back_room >= after {
        back_room
    } else {
        (used + after).max(after * 2)
    };
    let len = new_front + used + new_back;
    let from = front_room;
    let copied = match &mut buffer.data {
        Data::Latin1(v) => {
            let mut fresh = vec![0u8; len];
            if let (Some(dst), Some(src)) = (
                fresh.get_mut(new_front..new_front + used),
                v.get(from..from + used),
            ) {
                dst.copy_from_slice(src);
            }
            *v = fresh;
            used
        }
        Data::Utf16(v) => {
            let mut fresh = vec![0u16; len];
            if let (Some(dst), Some(src)) = (
                fresh.get_mut(new_front..new_front + used),
                v.get(from..from + used),
            ) {
                dst.copy_from_slice(src);
            }
            *v = fresh;
            used * 2
        }
    };
    buffer.origin = buffer.used_start - isize::try_from(new_front).unwrap_or(0);
    copied
}

/// The concatenation of `a` and `b`, at least [`BUFFER_FROM`] units
/// long, as a window: written into `a`'s buffer after it, into `b`'s
/// before it, or into a new buffer. Returns the cell (its bytes counted
/// as its buffer's), the buffer, the cell that counted the buffer before
/// (now to count nothing), and the bytes copied.
pub(crate) fn concat_window(
    a: &StringCell,
    b: &StringCell,
) -> (StringCell, Shared, Option<u32>, usize) {
    let (la, lb) = (a.len(), b.len());
    let window = |buffer: Shared, start: isize, end: isize| {
        let mut cell = StringCell::flat(Units::Window {
            buffer: Rc::clone(&buffer),
            start,
            end,
        });
        cell.accounted = buffer.borrow().bytes();
        cell
    };
    // Which may extend in place: a's window ending where its buffer is
    // used to, b's starting where its is. Where both may, the shorter is
    // the one written, so that `small + big` copies the small one.
    let after_a =
        matches!(&a.units, Units::Window { buffer, end, .. } if *end == buffer.borrow().used_end);
    let before_b = matches!(&b.units, Units::Window { buffer, start, .. } if *start == buffer.borrow().used_start);
    let append = after_a && (!before_b || lb <= la);
    // After a, in place. (The units written are read before the buffer
    // is borrowed to write: `s + s` reads the buffer it writes.)
    if append && let Units::Window { buffer, start, .. } = &a.units {
        let units = b.to_units();
        let mut buf = buffer.borrow_mut();
        {
            let mut copied = grow(&mut buf, 0, lb);
            let at = usize::try_from(buf.used_end - buf.origin).unwrap_or(0);
            copied += write_units(&mut buf.data, at, &units);
            buf.used_end += isize::try_from(lb).unwrap_or(0);
            let new_end = buf.used_end;
            let previous = buf.owner.take();
            drop(buf);
            let cell = window(Rc::clone(buffer), *start, new_end);
            return (cell, Rc::clone(buffer), previous, copied);
        }
    }
    // Before b, in place.
    if before_b && let Units::Window { buffer, end, .. } = &b.units {
        let units = a.to_units();
        let mut buf = buffer.borrow_mut();
        {
            let mut copied = grow(&mut buf, la, 0);
            buf.used_start -= isize::try_from(la).unwrap_or(0);
            let at = usize::try_from(buf.used_start - buf.origin).unwrap_or(0);
            copied += write_units(&mut buf.data, at, &units);
            let new_start = buf.used_start;
            let previous = buf.owner.take();
            drop(buf);
            let cell = window(Rc::clone(buffer), new_start, *end);
            return (cell, Rc::clone(buffer), previous, copied);
        }
    }
    // A new buffer, with room behind for what comes next.
    let total = la + lb;
    let latin1 = a.latin1() && b.latin1();
    let room = total * 2;
    let mut data = if latin1 {
        Data::Latin1(vec![0u8; room])
    } else {
        Data::Utf16(vec![0u16; room])
    };
    let mut copied = write_units(&mut data, 0, &a.to_units());
    copied += write_units(&mut data, la, &b.to_units());
    let end = isize::try_from(total).unwrap_or(0);
    let buffer = Rc::new(RefCell::new(Buffer {
        data,
        origin: 0,
        used_start: 0,
        used_end: end,
        owner: None,
    }));
    let cell = window(Rc::clone(&buffer), 0, end);
    (cell, buffer, None, copied)
}

/// An array index (section 6.1.7): the canonical decimal form of an
/// integer from 0 to 2^32 - 2, which keys elements rather than names.
pub(crate) fn array_index(units: &[u16]) -> Option<u32> {
    let (&first, rest) = units.split_first()?;
    if units.len() > 10 || !(u16::from(b'0')..=u16::from(b'9')).contains(&first) {
        return None;
    }
    if first == u16::from(b'0') && !rest.is_empty() {
        return None;
    }
    let mut n: u64 = 0;
    for &u in units {
        let d = u.checked_sub(u16::from(b'0')).filter(|d| *d <= 9)?;
        n = n * 10 + u64::from(d);
    }
    (n < u64::from(u32::MAX)).then_some(n as u32)
}

/// An interned name: an agent-wide id that compares as an integer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Atom(pub(crate) u32);

/// A property key (section 6.1.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PropertyKey {
    /// An array index.
    Index(u32),
    /// Any other string, interned.
    Atom(Atom),
    /// A symbol, by its cell's index.
    Symbol(u32),
}

/// The agent's atoms. They are never collected in J2 (an id is
/// permanent); collecting the ones no key holds is J6's.
#[derive(Default)]
pub(crate) struct Atoms {
    ids: HashMap<Box<[u16]>, Atom>,
    names: Vec<Box<[u16]>>,
    /// What the names take, kept as they are added: read in constant
    /// time.
    bytes: usize,
}

impl Atoms {
    pub(crate) fn intern(&mut self, units: &[u16]) -> Atom {
        if let Some(&atom) = self.ids.get(units) {
            return atom;
        }
        let atom = Atom(u32::try_from(self.names.len()).unwrap_or(u32::MAX));
        let owned: Box<[u16]> = units.into();
        self.bytes += owned.len() * 4 + 48;
        self.names.push(owned.clone());
        self.ids.insert(owned, atom);
        atom
    }

    pub(crate) fn intern_str(&mut self, text: &str) -> Atom {
        let units: Vec<u16> = text.encode_utf16().collect();
        self.intern(&units)
    }

    pub(crate) fn units(&self, atom: Atom) -> &[u16] {
        self.names.get(atom.0 as usize).map_or(&[], |n| n)
    }

    /// The key a string's units make: an index or an atom.
    pub(crate) fn key(&mut self, units: &[u16]) -> PropertyKey {
        match array_index(units) {
            Some(i) => PropertyKey::Index(i),
            None => PropertyKey::Atom(self.intern(units)),
        }
    }

    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn units(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn latin1_when_every_unit_fits() {
        let a = StringCell::from_str("caf\u{e9}");
        assert!(matches!(a.units, Units::Latin1(_)));
        assert_eq!(a.len(), 4);
        assert_eq!(a.to_text(), "caf\u{e9}");
        let b = StringCell::from_str("\u{3b1}\u{3b2}");
        assert!(matches!(b.units, Units::Utf16(_)));
        // A lone surrogate is kept.
        let lone = StringCell::from_units(&[0xD800, 0x61]);
        assert_eq!(lone.to_units(), vec![0xD800, 0x61]);
        // Equal strings hash alike in either form.
        let wide = StringCell::flat(Units::Utf16(units("abc").into()));
        assert!(StringCell::from_str("abc").same_units(&wide));
    }

    #[test]
    fn concatenation_keeps_the_narrow_form_while_it_can() {
        let ab = StringCell::concat(&StringCell::from_str("a"), &StringCell::from_str("b"));
        assert!(matches!(ab.units, Units::Latin1(_)));
        assert_eq!(ab.to_text(), "ab");
        let mixed =
            StringCell::concat(&StringCell::from_str("a"), &StringCell::from_str("\u{3b1}"));
        assert!(matches!(mixed.units, Units::Utf16(_)));
        assert!(mixed.same_units(&StringCell::from_str("a\u{3b1}")));
    }

    #[test]
    fn array_indices_are_canonical_integers_below_two_to_the_32_minus_one() {
        assert_eq!(array_index(&units("0")), Some(0));
        assert_eq!(array_index(&units("42")), Some(42));
        assert_eq!(array_index(&units("4294967294")), Some(4_294_967_294));
        for not in [
            "",
            "01",
            "-1",
            "1.0",
            "4294967295",
            "99999999999",
            "1e3",
            "a",
        ] {
            assert_eq!(array_index(&units(not)), None, "{not}");
        }
    }

    #[test]
    fn atoms_are_interned_once() {
        let mut atoms = Atoms::default();
        let a = atoms.intern_str("length");
        let b = atoms.intern_str("length");
        let c = atoms.intern_str("x");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(atoms.units(a), units("length").as_slice());
        assert_eq!(atoms.key(&units("7")), PropertyKey::Index(7));
        assert_eq!(
            atoms.key(&units("07")),
            PropertyKey::Atom(atoms.intern_str("07"))
        );
    }
}
