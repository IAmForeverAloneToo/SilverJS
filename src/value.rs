//! Values: NaN-boxed, 64 bits (docs/design.md section 1.3).
//!
//! A value is a `u64`. Every bit pattern below `0xFFF9 << 48` is a
//! double, as itself. The patterns from there up are NaNs with the sign
//! bit set and a payload, which no arithmetic makes; they carry a tag in
//! bits 48 to 50 and a 32-bit payload:
//!
//! | tag | payload |
//! |-----|---------|
//! | 1 int32 | the integer |
//! | 2 special | undefined, null, false, true, the array hole, the uninitialized binding |
//! | 3 object | an index into the heap's objects |
//! | 4 string | an index into the heap's strings |
//! | 5 symbol | an index into the heap's symbols |
//! | 6 BigInt | an index into the heap's BigInts |
//! | 7 internal | an index into the heap's internal cells (environments, iterators): never seen by script |
//!
//! Every double made from outside the engine's own arithmetic goes
//! through [`Value::number`] or [`Value::double`], which canonicalize a
//! NaN to `0x7FF8 << 48`, so that no input can forge a tagged value.
//! References are indices checked against their arena: a forged one
//! could break invariants but never memory, and the fuzzers check that
//! none is made.

/// Where the tagged patterns begin.
const TAGGED: u64 = 0xFFF9 << 48;
/// The one NaN a value holds.
const CANONICAL_NAN: u64 = 0x7FF8 << 48;
const TAG_SHIFT: u32 = 48;
const PAYLOAD: u64 = 0xFFFF_FFFF;

const TAG_INT: u64 = 1;
const TAG_SPECIAL: u64 = 2;
const TAG_OBJECT: u64 = 3;
const TAG_STRING: u64 = 4;
const TAG_SYMBOL: u64 = 5;
const TAG_BIGINT: u64 = 6;
const TAG_INTERNAL: u64 = 7;

const fn tagged(tag: u64, payload: u32) -> u64 {
    // 0xFFF8 with the tag's three bits beneath it: tag 1 is 0xFFF9.
    (0xFFF8 | tag) << TAG_SHIFT | payload as u64
}

/// What a reference names, by the arena it is an index into.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RefKind {
    Object,
    String,
    Symbol,
    BigInt,
    /// An environment or an iterator of the interpreter's own.
    Internal,
}

/// A value, as the engine holds it: in registers, slots and elements.
/// It names heap cells by index and carries no generation: a value is
/// reached only by marking or through a handle (section 1.5).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Value(u64);

impl Value {
    pub const UNDEFINED: Value = Value(tagged(TAG_SPECIAL, 0));
    pub const NULL: Value = Value(tagged(TAG_SPECIAL, 1));
    pub const FALSE: Value = Value(tagged(TAG_SPECIAL, 2));
    pub const TRUE: Value = Value(tagged(TAG_SPECIAL, 3));
    /// An array's missing element (never seen by script).
    pub const HOLE: Value = Value(tagged(TAG_SPECIAL, 4));
    /// A `let`, `const` or class binding before its declaration runs.
    pub const UNINITIALIZED: Value = Value(tagged(TAG_SPECIAL, 5));

    /// A number: an int32 when the double is one exactly (not `-0`),
    /// else a double, a NaN made canonical.
    pub fn number(f: f64) -> Value {
        let i = f as i32;
        if f64::from(i) == f && !(i == 0 && f.is_sign_negative()) {
            Value::int(i)
        } else {
            Value::double(f)
        }
    }

    /// A double, a NaN made canonical.
    pub fn double(f: f64) -> Value {
        if f.is_nan() {
            Value(CANONICAL_NAN)
        } else {
            Value(f.to_bits())
        }
    }

    pub const fn int(i: i32) -> Value {
        Value(tagged(TAG_INT, i as u32))
    }

    pub const fn bool(b: bool) -> Value {
        if b { Value::TRUE } else { Value::FALSE }
    }

    pub(crate) const fn reference(kind: RefKind, index: u32) -> Value {
        let tag = match kind {
            RefKind::Object => TAG_OBJECT,
            RefKind::String => TAG_STRING,
            RefKind::Symbol => TAG_SYMBOL,
            RefKind::BigInt => TAG_BIGINT,
            RefKind::Internal => TAG_INTERNAL,
        };
        Value(tagged(tag, index))
    }

    /// The bits, for serializing and hashing.
    pub fn bits(self) -> u64 {
        self.0
    }

    fn tag(self) -> Option<u64> {
        (self.0 >= TAGGED).then_some((self.0 >> TAG_SHIFT) & 0x7)
    }

    fn payload(self) -> u32 {
        (self.0 & PAYLOAD) as u32
    }

    pub fn is_double(self) -> bool {
        self.0 < TAGGED
    }

    pub fn is_int(self) -> bool {
        self.tag() == Some(TAG_INT)
    }

    pub fn is_number(self) -> bool {
        self.is_double() || self.is_int()
    }

    pub fn is_undefined(self) -> bool {
        self == Value::UNDEFINED
    }

    pub fn is_null(self) -> bool {
        self == Value::NULL
    }

    /// `undefined` or `null`.
    pub fn is_nullish(self) -> bool {
        self.is_undefined() || self.is_null()
    }

    pub fn as_int(self) -> Option<i32> {
        self.is_int().then_some(self.payload() as i32)
    }

    /// The number, whichever form it is held in.
    pub fn as_number(self) -> Option<f64> {
        if self.is_double() {
            Some(f64::from_bits(self.0))
        } else {
            self.as_int().map(f64::from)
        }
    }

    pub fn as_bool(self) -> Option<bool> {
        match self {
            Value::TRUE => Some(true),
            Value::FALSE => Some(false),
            _ => None,
        }
    }

    /// The arena and index a reference names.
    pub fn as_reference(self) -> Option<(RefKind, u32)> {
        let kind = match self.tag()? {
            TAG_OBJECT => RefKind::Object,
            TAG_STRING => RefKind::String,
            TAG_SYMBOL => RefKind::Symbol,
            TAG_BIGINT => RefKind::BigInt,
            TAG_INTERNAL => RefKind::Internal,
            _ => return None,
        };
        Some((kind, self.payload()))
    }

    pub fn as_object(self) -> Option<u32> {
        match self.as_reference() {
            Some((RefKind::Object, i)) => Some(i),
            _ => None,
        }
    }

    pub fn as_string(self) -> Option<u32> {
        match self.as_reference() {
            Some((RefKind::String, i)) => Some(i),
            _ => None,
        }
    }
}

impl std::fmt::Debug for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(i) = self.as_int() {
            return write!(f, "{i}");
        }
        if self.is_double() {
            return write!(f, "{:?}", f64::from_bits(self.0));
        }
        if let Some((kind, index)) = self.as_reference() {
            return write!(f, "{kind:?}#{index}");
        }
        let name = match *self {
            Value::UNDEFINED => "undefined",
            Value::NULL => "null",
            Value::TRUE => "true",
            Value::FALSE => "false",
            Value::HOLE => "<hole>",
            Value::UNINITIALIZED => "<uninitialized>",
            _ => "<invalid>",
        };
        f.write_str(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_ints_when_exact_and_doubles_otherwise() {
        assert_eq!(Value::number(3.0).as_int(), Some(3));
        assert_eq!(Value::number(-2_147_483_648.0).as_int(), Some(i32::MIN));
        assert!(Value::number(2_147_483_648.0).is_double());
        assert!(Value::number(0.5).is_double());
        let minus_zero = Value::number(-0.0);
        assert!(minus_zero.is_double());
        assert!(minus_zero.as_number().unwrap().is_sign_negative());
        assert_eq!(Value::number(0.0), Value::int(0));
        for f in [
            1.5,
            -1e300,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::MIN_POSITIVE,
        ] {
            assert_eq!(Value::number(f).as_number(), Some(f));
        }
    }

    #[test]
    fn every_nan_is_made_canonical_and_never_a_tag() {
        // A NaN with the sign bit and a payload looks like a tagged
        // object reference if kept as it came.
        let forged = f64::from_bits(tagged(TAG_OBJECT, 7));
        assert!(forged.is_nan());
        for nan in [
            f64::NAN,
            -f64::NAN,
            forged,
            f64::from_bits(0xFFFF_FFFF_FFFF_FFFF),
        ] {
            let v = Value::double(nan);
            assert!(v.is_double());
            assert_eq!(v.as_reference(), None);
            assert!(v.as_number().unwrap().is_nan());
            assert_eq!(v, Value::number(nan));
        }
    }

    #[test]
    fn singletons_ints_and_references_are_told_apart() {
        let all = [
            Value::UNDEFINED,
            Value::NULL,
            Value::FALSE,
            Value::TRUE,
            Value::HOLE,
            Value::UNINITIALIZED,
            Value::int(0),
            Value::int(-1),
            Value::reference(RefKind::Object, 0),
            Value::reference(RefKind::String, 0),
            Value::reference(RefKind::Symbol, 0),
            Value::reference(RefKind::BigInt, 0),
            Value::reference(RefKind::Internal, 0),
            Value::double(0.5),
        ];
        for (i, a) in all.iter().enumerate() {
            for (j, b) in all.iter().enumerate() {
                assert_eq!(i == j, a == b, "{a:?} {b:?}");
            }
        }
        assert_eq!(Value::int(-1).as_int(), Some(-1));
        // Int 0 is the first tagged pattern of all.
        assert_eq!(Value::int(0).as_int(), Some(0));
        assert!(!Value::int(0).is_double());
        assert_eq!(
            Value::reference(RefKind::Object, u32::MAX).as_object(),
            Some(u32::MAX)
        );
        assert_eq!(Value::reference(RefKind::String, 9).as_string(), Some(9));
        assert_eq!(Value::reference(RefKind::String, 9).as_object(), None);
        assert!(Value::NULL.is_nullish() && Value::UNDEFINED.is_nullish());
        assert!(!Value::FALSE.is_nullish());
        assert_eq!(Value::bool(true).as_bool(), Some(true));
        assert_eq!(Value::HOLE.as_bool(), None);
    }
}
