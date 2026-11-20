//! Strings (docs/design.md section 1.3): immutable, with WTF-16
//! semantics (a lone surrogate is a value of its own), held as Latin-1
//! (one byte a unit) when every unit is under 256 and as UTF-16
//! otherwise, each with its hash cached. Ropes come with J6.
//!
//! Property keys are [`PropertyKey`]s: an array index as a number, a
//! name as an [`Atom`] (an interned string with an agent-wide id, so
//! that keys compare as integers), or a symbol.

use std::collections::HashMap;

/// A string's units.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Units {
    Latin1(Box<[u8]>),
    Utf16(Box<[u16]>),
}

/// A string cell.
#[derive(Debug, Clone)]
pub(crate) struct StringCell {
    pub(crate) units: Units,
    pub(crate) hash: u32,
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

impl StringCell {
    /// A string of `units`, held as Latin-1 when they all fit.
    pub(crate) fn from_units(units: &[u16]) -> StringCell {
        let hash = hash_units(units.iter().copied());
        let data = if units.iter().all(|&u| u < 256) {
            Units::Latin1(units.iter().map(|&u| u as u8).collect())
        } else {
            Units::Utf16(units.into())
        };
        StringCell { units: data, hash }
    }

    pub(crate) fn from_str(text: &str) -> StringCell {
        if text.is_ascii() {
            let bytes: Box<[u8]> = text.as_bytes().into();
            let hash = hash_units(bytes.iter().map(|&b| u16::from(b)));
            return StringCell {
                units: Units::Latin1(bytes),
                hash,
            };
        }
        let units: Vec<u16> = text.encode_utf16().collect();
        StringCell::from_units(&units)
    }

    pub(crate) fn len(&self) -> usize {
        match &self.units {
            Units::Latin1(b) => b.len(),
            Units::Utf16(u) => u.len(),
        }
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = u16> + '_ {
        let (latin1, utf16): (&[u8], &[u16]) = match &self.units {
            Units::Latin1(b) => (b, &[]),
            Units::Utf16(u) => (&[], u),
        };
        latin1
            .iter()
            .map(|&b| u16::from(b))
            .chain(utf16.iter().copied())
    }

    pub(crate) fn to_units(&self) -> Vec<u16> {
        self.iter().collect()
    }

    /// The string as Rust text, a lone surrogate replaced.
    pub(crate) fn to_text(&self) -> String {
        match &self.units {
            Units::Latin1(b) => b.iter().map(|&b| char::from(b)).collect(),
            Units::Utf16(u) => String::from_utf16_lossy(u),
        }
    }

    /// Two strings' concatenation.
    pub(crate) fn concat(a: &StringCell, b: &StringCell) -> StringCell {
        if let (Units::Latin1(x), Units::Latin1(y)) = (&a.units, &b.units) {
            let mut bytes = Vec::with_capacity(x.len() + y.len());
            bytes.extend_from_slice(x);
            bytes.extend_from_slice(y);
            let hash = hash_units(bytes.iter().map(|&b| u16::from(b)));
            return StringCell {
                units: Units::Latin1(bytes.into()),
                hash,
            };
        }
        let units: Vec<u16> = a.iter().chain(b.iter()).collect();
        StringCell::from_units(&units)
    }

    pub(crate) fn same_units(&self, other: &StringCell) -> bool {
        self.hash == other.hash && self.len() == other.len() && self.iter().eq(other.iter())
    }

    /// The bytes the cell owns outside itself.
    pub(crate) fn owned_bytes(&self) -> usize {
        match &self.units {
            Units::Latin1(b) => b.len(),
            Units::Utf16(u) => u.len() * 2,
        }
    }
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
}

impl Atoms {
    pub(crate) fn intern(&mut self, units: &[u16]) -> Atom {
        if let Some(&atom) = self.ids.get(units) {
            return atom;
        }
        let atom = Atom(u32::try_from(self.names.len()).unwrap_or(u32::MAX));
        let owned: Box<[u16]> = units.into();
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
        self.names.iter().map(|n| n.len() * 4 + 48).sum()
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
        let wide = StringCell {
            units: Units::Utf16(units("abc").into()),
            hash: hash_units(units("abc").into_iter()),
        };
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
