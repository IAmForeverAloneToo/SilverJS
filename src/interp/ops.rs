//! The abstract operations the instructions use (ECMA-262 sections 7
//! and 13): conversions, operators, property access, bindings by name,
//! and making closures, arrays and errors.

use super::messages::{self, ErrorType};
use super::{Abrupt, Res};
use crate::agent::Agent;
use crate::compiler::{FunctionKind, Op};
use crate::heap::{InternalCell, ObjectCell, StringCell};
use crate::internal::ForIn;
use crate::number::number_to_string;
use crate::object::{Callable, Descriptor, ObjectKind, OwnProperty, SetOutcome};
use crate::string::PropertyKey;
use crate::value::{RefKind, Value};

/// What a lookup by name finds outside the global environment.
enum Found {
    /// Environment `.0`'s slot `.1`.
    Slot(u32, usize),
    /// A property of a `with` statement's object.
    With(Value),
    /// A property of the object of the `var`s a sloppy direct eval made.
    Vars(Value),
}

pub(crate) fn configurable(p: &OwnProperty) -> bool {
    match *p {
        OwnProperty::Data { configurable, .. } | OwnProperty::Accessor { configurable, .. } => {
            configurable
        }
    }
}

fn enumerable(p: &OwnProperty) -> bool {
    match *p {
        OwnProperty::Data { enumerable, .. } | OwnProperty::Accessor { enumerable, .. } => {
            enumerable
        }
    }
}

/// ToInt32 (section 7.1.6).
pub(crate) fn to_int32(n: f64) -> i32 {
    if !n.is_finite() {
        return 0;
    }
    let m = n.trunc() % 4_294_967_296.0;
    let m = if m < 0.0 { m + 4_294_967_296.0 } else { m };
    (m as u64 as u32) as i32
}

fn to_uint32(n: f64) -> u32 {
    to_int32(n) as u32
}

/// Whether `c` is WhiteSpace or a LineTerminator (sections 12.2, 12.3),
/// which StringToNumber trims.
fn is_space(c: u16) -> bool {
    matches!(
        c,
        0x09 | 0x0A | 0x0B | 0x0C | 0x0D | 0x20 | 0xA0 | 0x1680 | 0x2000
            ..=0x200A | 0x2028 | 0x2029 | 0x202F | 0x205F | 0x3000 | 0xFEFF
    )
}

/// StringToNumber (section 7.1.4.1.1).
pub(crate) fn string_to_number(units: &[u16]) -> f64 {
    let start = units
        .iter()
        .position(|&c| !is_space(c))
        .unwrap_or(units.len());
    let end = units
        .iter()
        .rposition(|&c| !is_space(c))
        .map_or(start, |e| e + 1);
    let Some(trimmed) = units.get(start..end) else {
        return 0.0;
    };
    if trimmed.is_empty() {
        return 0.0;
    }
    let Ok(text) = String::from_utf16(trimmed) else {
        return f64::NAN;
    };
    let radix = match text.get(..2) {
        Some("0x" | "0X") => Some(16),
        Some("0o" | "0O") => Some(8),
        Some("0b" | "0B") => Some(2),
        _ => None,
    };
    if let Some(radix) = radix {
        let digits = text.get(2..).unwrap_or("");
        if digits.is_empty() {
            return f64::NAN;
        }
        let mut n = 0.0f64;
        for c in digits.chars() {
            let Some(d) = c.to_digit(radix) else {
                return f64::NAN;
            };
            n = n * f64::from(radix) + f64::from(d);
        }
        return n;
    }
    let (sign, body) = match text.as_bytes().first() {
        Some(b'+') => (1.0, text.get(1..).unwrap_or("")),
        Some(b'-') => (-1.0, text.get(1..).unwrap_or("")),
        _ => (1.0, text.as_str()),
    };
    if body == "Infinity" {
        return sign * f64::INFINITY;
    }
    // StrUnsignedDecimalLiteral: digits, a point, digits, an exponent;
    // checked here, since Rust's parser takes "inf" and "nan" too.
    let bytes = body.as_bytes();
    let mut i = 0;
    let digits = |i: &mut usize| {
        let from = *i;
        while bytes.get(*i).is_some_and(u8::is_ascii_digit) {
            *i += 1;
        }
        *i - from
    };
    let whole = digits(&mut i);
    let mut fraction = 0;
    if bytes.get(i) == Some(&b'.') {
        i += 1;
        fraction = digits(&mut i);
    }
    if whole + fraction == 0 {
        return f64::NAN;
    }
    if matches!(bytes.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(bytes.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        if digits(&mut i) == 0 {
            return f64::NAN;
        }
    }
    if i != bytes.len() {
        return f64::NAN;
    }
    sign * body.parse::<f64>().unwrap_or(f64::NAN)
}

/// A numeric binary operator but `+` on two numbers.
fn number_binary(op: Op, x: f64, y: f64) -> Value {
    match op {
        Op::Sub => Value::number(x - y),
        Op::Mul => Value::number(x * y),
        Op::Div => Value::number(x / y),
        Op::Mod => Value::number(x % y),
        Op::Exp => Value::number(exponentiate(x, y)),
        Op::BitOr => Value::int(to_int32(x) | to_int32(y)),
        Op::BitAnd => Value::int(to_int32(x) & to_int32(y)),
        Op::BitXor => Value::int(to_int32(x) ^ to_int32(y)),
        Op::Shl => Value::int(to_int32(x).wrapping_shl(to_uint32(y) & 31)),
        Op::Shr => Value::int(to_int32(x) >> (to_uint32(y) & 31)),
        _ => Value::number(f64::from(to_uint32(x) >> (to_uint32(y) & 31))),
    }
}

/// Number::exponentiate (section 6.1.6.1.3), where it differs from
/// `powf`: a NaN exponent, and a base of magnitude 1 to an infinity.
pub(crate) fn exponentiate(base: f64, exponent: f64) -> f64 {
    if exponent.is_nan() {
        return f64::NAN;
    }
    if exponent == 0.0 {
        return 1.0;
    }
    if base.abs() == 1.0 && exponent.is_infinite() {
        return f64::NAN;
    }
    base.powf(exponent)
}

impl Agent {
    // Allocation.

    pub(crate) fn alloc_object(&mut self, cell: ObjectCell) -> Res<Value> {
        if self.reserve(64).is_err() {
            return Err(self.out_of_memory());
        }
        let (i, _) = self.heap.objects.alloc(cell);
        Ok(Value::reference(RefKind::Object, i))
    }

    pub(crate) fn alloc_internal(&mut self, cell: InternalCell) -> Res<Value> {
        if self.reserve(64).is_err() {
            return Err(self.out_of_memory());
        }
        let (i, _) = self.heap.internals.alloc(cell);
        Ok(Value::reference(RefKind::Internal, i))
    }

    pub(crate) fn new_string_units(&mut self, units: &[u16]) -> Res<Value> {
        let cell = StringCell::from_units(units);
        if self.reserve(cell.owned_bytes() + 48).is_err() {
            return Err(self.out_of_memory());
        }
        let (i, _) = self.heap.strings.alloc(cell);
        Ok(Value::reference(RefKind::String, i))
    }

    pub(crate) fn new_string(&mut self, text: &str) -> Res<Value> {
        let units: Vec<u16> = text.encode_utf16().collect();
        self.new_string_units(&units)
    }

    /// The heap limit was reached: a `RangeError`, made past the limit
    /// (it is small), so that script can catch it.
    pub(crate) fn out_of_memory(&mut self) -> Abrupt {
        let proto = self.realm().error_proto(ErrorType::RangeError);
        let (o, _) = self
            .heap
            .objects
            .alloc(ObjectCell::new(proto, ObjectKind::Error));
        let (m, _) = self
            .heap
            .strings
            .alloc(StringCell::from_str(messages::OUT_OF_MEMORY));
        let key = self.realm().message_key;
        self.heap
            .define_own_property(o, key, hidden(Value::reference(RefKind::String, m)));
        self.vm.exception = Value::reference(RefKind::Object, o);
        Abrupt::Throw
    }

    /// Throws a new error of type `t`; what the instruction returns.
    pub(crate) fn throw_error(&mut self, t: ErrorType, message: impl AsRef<str>) -> Abrupt {
        match self.make_error(t, message.as_ref()) {
            Ok(e) => {
                self.vm.exception = e;
                Abrupt::Throw
            }
            Err(a) => a,
        }
    }

    pub(crate) fn make_error(&mut self, t: ErrorType, message: &str) -> Res<Value> {
        let proto = self.realm().error_proto(t);
        let e = self.alloc_object(ObjectCell::new(proto, ObjectKind::Error))?;
        self.vm.temps.push(e);
        let m = self.new_string(message).and_then(|m| {
            // `stack` first, then `message`: the reference browser's order.
            self.vm.temps.push(m);
            let captured = self.capture_stack(e, m, false, Value::UNDEFINED);
            self.vm.temps.pop();
            captured.map(|()| m)
        });
        self.vm.temps.pop();
        let m = m?;
        let key = self.realm().message_key;
        if let Some(o) = e.as_object() {
            self.heap.define_own_property(o, key, hidden(m));
        }
        Ok(e)
    }

    // Conversions.

    pub(crate) fn string_units(&self, v: Value) -> Option<Vec<u16>> {
        Some(self.heap.strings.get(v.as_string()?)?.to_units())
    }

    /// ToBoolean (section 7.1.2).
    pub(crate) fn to_boolean(&self, v: Value) -> bool {
        if let Some(b) = v.as_bool() {
            return b;
        }
        if let Some(n) = v.as_number() {
            return n != 0.0 && !n.is_nan();
        }
        if let Some(s) = v.as_string() {
            return self.heap.strings.get(s).is_some_and(|c| c.len() > 0);
        }
        if let Some(b) = self.bigint_of(v) {
            return !b.is_zero();
        }
        !v.is_nullish()
    }

    /// ToNumber (section 7.1.4).
    pub(crate) fn number(&mut self, v: Value) -> Res<f64> {
        if let Some(n) = v.as_number() {
            return Ok(n);
        }
        match v {
            Value::UNDEFINED => return Ok(f64::NAN),
            Value::NULL | Value::FALSE => return Ok(0.0),
            Value::TRUE => return Ok(1.0),
            _ => {}
        }
        match v.as_reference() {
            Some((RefKind::String, _)) => {
                Ok(string_to_number(&self.string_units(v).unwrap_or_default()))
            }
            Some((RefKind::Symbol, _)) => {
                Err(self.throw_error(ErrorType::TypeError, messages::SYMBOL_TO_NUMBER))
            }
            Some((RefKind::Object, _)) => {
                let p = self.primitive(v, Some(false))?;
                self.number(p)
            }
            _ => Err(self.type_error(crate::builtins::bigint::TO_NUMBER)),
        }
    }

    /// ToPrimitive (section 7.1.1) by OrdinaryToPrimitive: `hint`
    /// `Some(true)` string, `Some(false)` number, `None` default.
    pub(crate) fn primitive(&mut self, v: Value, hint: Option<bool>) -> Res<Value> {
        if v.as_object().is_none() {
            return Ok(v);
        }
        // `@@toPrimitive` first, with the hint's name.
        let key = self.well_known_key(crate::builtins::Wk::ToPrimitive);
        self.vm.temps.push(v);
        let exotic = self.get_method(v, key);
        self.vm.temps.pop();
        let exotic = exotic?;
        if !exotic.is_undefined() {
            let hint = match hint {
                Some(true) => "string",
                Some(false) => "number",
                None => "default",
            };
            let h = self.new_string(hint)?;
            self.vm.temps.push(v);
            let r = self.call_value(exotic, v, &[h]);
            self.vm.temps.pop();
            let r = r?;
            if r.as_object().is_some() {
                return Err(self.throw_error(ErrorType::TypeError, messages::OBJECT_TO_PRIMITIVE));
            }
            return Ok(r);
        }
        self.ordinary_primitive(v, hint == Some(true))
    }

    /// OrdinaryToPrimitive (section 7.1.1.1): `toString` first when
    /// `string_first`, else `valueOf`.
    pub(crate) fn ordinary_primitive(&mut self, v: Value, string_first: bool) -> Res<Value> {
        let realm = self.realm();
        let order = if string_first {
            [realm.to_string_key, realm.value_of_key]
        } else {
            [realm.value_of_key, realm.to_string_key]
        };
        self.vm.temps.push(v);
        let result = (|| {
            for key in order {
                let method = self.get_value(v, key)?;
                if self.callable(method).is_some() {
                    let r = self.call_value(method, v, &[])?;
                    if r.as_object().is_none() {
                        return Ok(r);
                    }
                }
            }
            Err(self.throw_error(ErrorType::TypeError, messages::OBJECT_TO_PRIMITIVE))
        })();
        self.vm.temps.pop();
        result
    }

    /// ToString (section 7.1.17), as a string value.
    pub(crate) fn string_value(&mut self, v: Value) -> Res<Value> {
        if v.as_string().is_some() {
            return Ok(v);
        }
        if let Some(n) = v.as_number() {
            return self.new_string(&number_to_string(n));
        }
        let known = match v {
            Value::UNDEFINED => Some(self.realm().strings.undefined),
            Value::NULL => Some(self.realm().strings.null),
            Value::TRUE => Some(self.realm().strings.true_),
            Value::FALSE => Some(self.realm().strings.false_),
            _ => None,
        };
        if let Some(s) = known {
            return Ok(s);
        }
        match v.as_reference() {
            Some((RefKind::Symbol, _)) => {
                Err(self.throw_error(ErrorType::TypeError, messages::SYMBOL_TO_STRING))
            }
            Some((RefKind::Object, _)) => {
                let p = self.primitive(v, Some(true))?;
                self.string_value(p)
            }
            _ => {
                let b = self.bigint_of(v).cloned().unwrap_or_default();
                match self.with_fuel(|f| b.to_string_radix(10, f)) {
                    Ok(text) => self.new_string(&text),
                    Err(r) => Err(self.bigint_refused(r)),
                }
            }
        }
    }

    /// ToPropertyKey (section 7.1.19).
    pub(crate) fn property_key(&mut self, v: Value) -> Res<PropertyKey> {
        if let Some(i) = v.as_int()
            && i >= 0
        {
            return Ok(PropertyKey::Index(i as u32));
        }
        match v.as_reference() {
            Some((RefKind::Symbol, s)) => Ok(PropertyKey::Symbol(s)),
            Some((RefKind::String, _)) => {
                let units = self.string_units(v).unwrap_or_default();
                Ok(self.heap.atoms.key(&units))
            }
            _ => {
                let p = self.primitive(v, Some(true))?;
                if let Some((RefKind::Symbol, s)) = p.as_reference() {
                    return Ok(PropertyKey::Symbol(s));
                }
                let s = self.string_value(p)?;
                let units = self.string_units(s).unwrap_or_default();
                Ok(self.heap.atoms.key(&units))
            }
        }
    }

    /// ToPropertyKey's result as a value (a string or a symbol), for a
    /// computed key evaluated once.
    pub(crate) fn property_key_value(&mut self, v: Value) -> Res<Value> {
        if v.as_string().is_some() || matches!(v.as_reference(), Some((RefKind::Symbol, _))) {
            return Ok(v);
        }
        let p = self.primitive(v, Some(true))?;
        if matches!(p.as_reference(), Some((RefKind::Symbol, _))) {
            return Ok(p);
        }
        self.string_value(p)
    }

    /// `typeof` (section 13.5.3).
    pub(crate) fn type_of(&self, v: Value) -> Value {
        let s = &self.realm().strings;
        if v.is_number() {
            return s.number;
        }
        match v {
            Value::UNDEFINED => return s.undefined,
            Value::NULL => return s.object,
            Value::TRUE | Value::FALSE => return s.boolean,
            _ => {}
        }
        match v.as_reference() {
            Some((RefKind::String, _)) => s.string,
            Some((RefKind::Symbol, _)) => s.symbol,
            Some((RefKind::BigInt, _)) => s.bigint,
            Some((RefKind::Object, _)) if self.callable(v).is_some() => s.function,
            _ => s.object,
        }
    }

    // Comparison.

    /// IsStrictlyEqual (section 7.2.15).
    pub(crate) fn strict_equals(&self, a: Value, b: Value) -> bool {
        if let (Some(x), Some(y)) = (a.as_number(), b.as_number()) {
            return x == y;
        }
        if let (Some(x), Some(y)) = (self.bigint_of(a), self.bigint_of(b)) {
            return x == y;
        }
        if let (Some(x), Some(y)) = (a.as_string(), b.as_string()) {
            return match (self.heap.strings.get(x), self.heap.strings.get(y)) {
                (Some(x), Some(y)) => x.same_units(y),
                _ => false,
            };
        }
        a == b
    }

    /// IsLooselyEqual (section 7.2.14).
    pub(crate) fn loose_equals(&mut self, a: Value, b: Value) -> Res<bool> {
        let same_type = (a.is_number() && b.is_number())
            || match (a.as_reference(), b.as_reference()) {
                (Some((x, _)), Some((y, _))) => x == y,
                (None, None) => (a.as_bool().is_some() && b.as_bool().is_some()) || (a == b),
                _ => false,
            };
        if same_type {
            return Ok(self.strict_equals(a, b));
        }
        if a.is_nullish() && b.is_nullish() {
            return Ok(true);
        }
        if a.is_nullish() || b.is_nullish() {
            return Ok(false);
        }
        if a.is_number() && b.as_string().is_some() {
            let n = self.number(b)?;
            return Ok(a.as_number() == Some(n));
        }
        if a.as_string().is_some() && b.is_number() {
            let n = self.number(a)?;
            return Ok(b.as_number() == Some(n));
        }
        // A BigInt and a string: the string as a BigInt, if it is one.
        let (big, other) = if self.is_bigint(a) { (a, b) } else { (b, a) };
        if self.is_bigint(big)
            && let Some(units) = other.as_string().and_then(|_| self.string_units(other))
        {
            let n = match self.with_fuel(|f| crate::bigint::BigInt::parse(&units, f)) {
                Ok(n) => n,
                Err(r) => return Err(self.bigint_refused(r)),
            };
            return Ok(n.is_some_and(|n| self.bigint_of(big) == Some(&n)));
        }
        if a.as_bool().is_some() {
            let n = Value::number(self.number(a)?);
            return self.loose_equals(n, b);
        }
        if b.as_bool().is_some() {
            let n = Value::number(self.number(b)?);
            return self.loose_equals(a, n);
        }
        let is_object = |v: Value| v.as_object().is_some();
        if is_object(b) && !is_object(a) {
            let p = self.primitive(b, None)?;
            return self.loose_equals(a, p);
        }
        if is_object(a) && !is_object(b) {
            let p = self.primitive(a, None)?;
            return self.loose_equals(p, b);
        }
        // A BigInt and a number, compared as mathematical values.
        if let (Some(x), Some(n)) = (self.bigint_of(big), other.as_number()) {
            return Ok(x.compare_number(n) == Some(std::cmp::Ordering::Equal));
        }
        Ok(false)
    }

    /// The relational operators (section 13.10.1), left operand first.
    pub(crate) fn relational(&mut self, op: Op, a: Value, b: Value) -> Res<bool> {
        let pa = self.primitive(a, Some(false))?;
        self.vm.temps.push(pa);
        let pb = self.primitive(b, Some(false));
        self.vm.temps.pop();
        let pb = pb?;
        if let (Some(x), Some(y)) = (self.string_units(pa), self.string_units(pb)) {
            return Ok(match op {
                Op::TestLt => x < y,
                Op::TestGt => x > y,
                Op::TestLtEq => x <= y,
                _ => x >= y,
            });
        }
        if self.is_bigint(pa) || self.is_bigint(pb) {
            return self.relational_bigint(op, pa, pb);
        }
        let x = self.number(pa)?;
        let y = self.number(pb)?;
        Ok(match op {
            Op::TestLt => x < y,
            Op::TestGt => x > y,
            Op::TestLtEq => x <= y,
            _ => x >= y,
        })
    }

    // Arithmetic.

    /// The `+` operator (section 13.15.3, ApplyStringOrNumericBinaryOperator).
    pub(crate) fn add(&mut self, a: Value, b: Value) -> Res<Value> {
        if let (Some(x), Some(y)) = (a.as_number(), b.as_number()) {
            return Ok(Value::number(x + y));
        }
        let pa = self.primitive(a, None)?;
        self.vm.temps.push(pa);
        let pb = self.primitive(b, None);
        let result = match pb {
            Ok(pb) => {
                self.vm.temps.push(pb);
                let r = if pa.as_string().is_some() || pb.as_string().is_some() {
                    self.concat(pa, pb)
                } else if self.is_bigint(pa) || self.is_bigint(pb) {
                    self.bigint_binary(Op::Add, pa, pb)
                } else {
                    match self.number(pa) {
                        Ok(x) => self.number(pb).map(|y| Value::number(x + y)),
                        Err(e) => Err(e),
                    }
                };
                self.vm.temps.pop();
                r
            }
            Err(e) => Err(e),
        };
        self.vm.temps.pop();
        result
    }

    fn concat(&mut self, a: Value, b: Value) -> Res<Value> {
        let a = self.string_value(a)?;
        self.vm.temps.push(a);
        let b = self.string_value(b);
        self.vm.temps.pop();
        let b = b?;
        let (Some(ia), Some(ib)) = (a.as_string(), b.as_string()) else {
            return Ok(a);
        };
        let len = |i| self.heap.strings.get(i).map_or(0, StringCell::len);
        if len(ib) == 0 {
            return Ok(a);
        }
        if len(ia) == 0 {
            return Ok(b);
        }
        let Some((cell, buffer)) = self.heap.concat_cells(ia, ib) else {
            return Ok(a);
        };
        // Both stay rooted while the room is made.
        self.vm.temps.extend([a, b]);
        let room = self.reserve(cell.owned_bytes() + 48);
        self.vm.temps.truncate(self.vm.temps.len() - 2);
        if room.is_err() {
            return Err(self.out_of_memory());
        }
        let (i, _) = self.heap.strings.alloc(cell);
        if let Some(buffer) = buffer {
            self.heap.own_buffer(&buffer, i);
        }
        Ok(Value::reference(RefKind::String, i))
    }

    /// The numeric binary operators but `+`.
    pub(crate) fn numeric_binary(&mut self, op: Op, a: Value, b: Value) -> Res<Value> {
        // Two numbers, the common case, skip ToNumeric and the BigInt test.
        if let (Some(x), Some(y)) = (a.as_number(), b.as_number()) {
            return Ok(number_binary(op, x, y));
        }
        let a = self.numeric(a)?;
        self.vm.temps.push(a);
        let b = self.numeric(b);
        self.vm.temps.pop();
        let b = b?;
        if self.is_bigint(a) || self.is_bigint(b) {
            return self.bigint_binary(op, a, b);
        }
        let x = self.number(a)?;
        let y = self.number(b)?;
        Ok(number_binary(op, x, y))
    }

    // Properties.

    /// A key's text for a message: a name, an index, or `Symbol(desc)`.
    pub(crate) fn key_display(&self, key: PropertyKey) -> String {
        match key {
            PropertyKey::Index(i) => i.to_string(),
            PropertyKey::Atom(a) => String::from_utf16_lossy(self.heap.atoms.units(a)),
            PropertyKey::Symbol(s) => {
                let d = self
                    .heap
                    .symbols
                    .get(s)
                    .and_then(|c| self.string_units(c.description))
                    .map(|u| String::from_utf16_lossy(&u))
                    .unwrap_or_default();
                format!("Symbol({d})")
            }
        }
    }

    /// A value with its type, as some of the reference browser's messages
    /// name it (`number 1`): a primitive's type and text, an object as
    /// `object`.
    pub(crate) fn typed_display(&self, v: Value) -> String {
        if v.is_nullish() {
            return self.display(v);
        }
        if v.as_object().is_some() {
            return if self.callable(v).is_some() {
                "function".into()
            } else {
                "object".into()
            };
        }
        let ty = if v.is_number() {
            "number"
        } else if v.as_bool().is_some() {
            "boolean"
        } else if self.string_units(v).is_some() {
            "string"
        } else {
            "symbol"
        };
        format!("{ty} {}", self.display(v))
    }

    /// A value's text for a message, as the reference browser shows it.
    pub(crate) fn display(&self, v: Value) -> String {
        if let Some(n) = v.as_number() {
            return number_to_string(n);
        }
        if let Some(u) = self.string_units(v) {
            return String::from_utf16_lossy(&u);
        }
        match v {
            Value::UNDEFINED => "undefined".into(),
            Value::NULL => "null".into(),
            Value::TRUE => "true".into(),
            Value::FALSE => "false".into(),
            _ if self.callable(v).is_some() => "function".into(),
            _ => match v.as_object() {
                Some(o) => self.object_display(o),
                None => match self.bigint_of(v) {
                    Some(b) => {
                        crate::bigint::unmetered(|f| b.to_string_radix(10, f)).unwrap_or_default()
                    }
                    None => "Symbol()".into(),
                },
            },
        }
    }

    /// The object a read-only property belongs to, as Chrome names it
    /// in the TypeError: `#<Ctor>` for one that keeps
    /// `Object.prototype.toString`, `[object RegExp]` and the like for
    /// one with a `toString` of its own (a function stays `#<Object>`).
    #[cold]
    #[inline(never)]
    fn read_only_display(&self, target: Value) -> String {
        match target.as_object() {
            Some(o) if self.callable(target).is_none() => self.object_display(o),
            _ => "#<Object>".into(),
        }
    }

    /// The prototype a primitive's properties are read from.
    fn primitive_proto(&self, v: Value) -> Value {
        let r = self.realm();
        if v.is_number() {
            return r.number_prototype;
        }
        if v.as_bool().is_some() {
            return r.boolean_prototype;
        }
        match v.as_reference() {
            Some((RefKind::String, _)) => r.string_prototype,
            Some((RefKind::Symbol, _)) => r.symbol_prototype,
            Some((RefKind::BigInt, _)) => r.intrinsics.bigint_prototype,
            _ => Value::NULL,
        }
    }

    /// GetV (section 7.3.3): `target[key]`, a getter called with
    /// `target`; for a primitive, its prototype's (a string's own
    /// `length` and indices first).
    pub(crate) fn get_value(&mut self, target: Value, key: PropertyKey) -> Res<Value> {
        let holder = match target.as_object() {
            Some(o) => o,
            None => {
                if target.is_nullish() {
                    let of = if target.is_null() {
                        "null"
                    } else {
                        "undefined"
                    };
                    let k = self.key_display(key);
                    return Err(
                        self.throw_error(ErrorType::TypeError, messages::read_properties(of, &k))
                    );
                }
                if let Some(s) = target.as_string().and_then(|i| self.heap.strings.get(i)) {
                    if key == self.heap.length_key {
                        return Ok(Value::int(i32::try_from(s.len()).unwrap_or(i32::MAX)));
                    }
                    if let PropertyKey::Index(i) = key
                        && (i as usize) < s.len()
                    {
                        let unit = s.unit(i as usize).unwrap_or(0);
                        return self.new_string_units(&[unit]);
                    }
                }
                match self.primitive_proto(target).as_object() {
                    Some(p) => p,
                    None => return Ok(Value::UNDEFINED),
                }
            }
        };
        self.o_get(holder, key, target)
    }

    pub(crate) fn get_keyed(&mut self, target: Value, k: Value) -> Res<Value> {
        // Elements of an array by an int: the common case.
        if let (Some(o), Some(i)) = (target.as_object(), k.as_int())
            && i >= 0
        {
            return self.get_value(
                Value::reference(RefKind::Object, o),
                PropertyKey::Index(i as u32),
            );
        }
        if target.is_nullish() {
            // An object key is not converted first, and the message
            // leaves it out, as the reference browser's (the J4.6 refs
            // probe).
            if k.as_object().is_some() {
                return Err(self.nullish_keyed(target, false));
            }
            let key = self.property_key(k)?;
            return self.get_value(target, key);
        }
        let key = self.property_key(k)?;
        self.get_value(target, key)
    }

    /// The TypeError of `null[k]` or `undefined[k]`, read or written,
    /// for a key not yet converted.
    #[cold]
    #[inline(never)]
    fn nullish_keyed(&mut self, target: Value, set: bool) -> Abrupt {
        let of = if target.is_null() {
            "null"
        } else {
            "undefined"
        };
        let verb = if set { "set" } else { "read" };
        self.throw_error(ErrorType::TypeError, messages::properties_of(verb, of))
    }

    /// ArraySetLength's conversions (section 10.4.2.4): ToUint32 and
    /// ToNumber of the new length, a RangeError where they differ.
    pub(crate) fn array_length_value(&mut self, v: Value) -> Res<Value> {
        if v.as_int().is_some_and(|i| i >= 0) {
            return Ok(v);
        }
        let n = self.number(v)?;
        let len = to_uint32(n);
        let again = self.number(v)?;
        if f64::from(len) != again {
            return Err(self.throw_error(ErrorType::RangeError, "Invalid array length"));
        }
        Ok(Value::number(f64::from(len)))
    }

    /// PutValue's `[[Set]]` (section 6.2.5.6): `target[key] = value`, a
    /// failure thrown in strict code.
    pub(crate) fn set_value(
        &mut self,
        target: Value,
        key: PropertyKey,
        value: Value,
        strict: bool,
    ) -> Res<()> {
        let Some(o) = target.as_object() else {
            if target.is_nullish() {
                let of = if target.is_null() {
                    "null"
                } else {
                    "undefined"
                };
                let k = self.key_display(key);
                return Err(
                    self.throw_error(ErrorType::TypeError, messages::set_properties(of, &k))
                );
            }
            if strict {
                let k = self.key_display(key);
                let kind = self.display_type(target);
                let text = self.display(target);
                return Err(self.throw_error(
                    ErrorType::TypeError,
                    messages::create_on_primitive(&k, kind, &text),
                ));
            }
            return Ok(());
        };
        let mut value = value;
        if key == self.heap.length_key
            && let Some((_, true)) = self.heap.objects.get(o).and_then(ObjectCell::array_length)
        {
            value = self.array_length_value(value)?;
        }
        match self.heap.set(o, key, value, target) {
            SetOutcome::Done(true) => Ok(()),
            SetOutcome::Done(false) => {
                if strict {
                    let k = self.key_display(key);
                    let of = self.read_only_display(target);
                    Err(self.throw_error(ErrorType::TypeError, messages::read_only(&k, &of)))
                } else {
                    Ok(())
                }
            }
            SetOutcome::Setter(s) => {
                self.vm.accessor_call = true;
                self.call_value(s, target, &[value]).map(|_| ())
            }
            SetOutcome::Receiver(r) => {
                if self.set_on_receiver(r, key, value)? || !strict {
                    Ok(())
                } else {
                    let k = self.key_display(key);
                    let of = self.read_only_display(target);
                    Err(self.throw_error(ErrorType::TypeError, messages::read_only(&k, &of)))
                }
            }
            SetOutcome::Typed(t) => self.typed_set(t, key, value),
            SetOutcome::Proxy(p) => {
                if self.proxy_set(p, key, value, target)? || !strict {
                    Ok(())
                } else {
                    let k = self.key_display(key);
                    Err(self.throw_error(ErrorType::TypeError, messages::proxy_set_false(&k)))
                }
            }
        }
    }

    fn display_type(&self, v: Value) -> &'static str {
        if v.is_number() {
            "number"
        } else if v.as_bool().is_some() {
            "boolean"
        } else if v.as_string().is_some() {
            "string"
        } else {
            "symbol"
        }
    }

    pub(crate) fn set_keyed(
        &mut self,
        target: Value,
        k: Value,
        value: Value,
        strict: bool,
    ) -> Res<()> {
        if target.is_nullish() {
            if k.as_object().is_some() {
                return Err(self.nullish_keyed(target, true));
            }
            let key = self.property_key(k)?;
            return self.set_value(target, key, value, strict);
        }
        let key = self.property_key(k)?;
        self.set_value(target, key, value, strict)
    }

    /// CreateDataPropertyOrThrow (section 7.3.7), for literals.
    pub(crate) fn define_value(
        &mut self,
        target: Value,
        key: PropertyKey,
        value: Value,
    ) -> Res<()> {
        if let Some(o) = target.as_object()
            && !self
                .heap
                .define_own_property(o, key, Descriptor::data(value))
        {
            let k = self.key_display(key);
            return Err(self.throw_error(
                ErrorType::TypeError,
                format!("Cannot redefine property: {k}"),
            ));
        }
        Ok(())
    }

    /// The `delete` operator on a property (section 13.5.1.2).
    pub(crate) fn delete_property(&mut self, target: Value, k: Value, strict: bool) -> Res<Value> {
        if target.is_nullish() {
            return Err(self.throw_error(ErrorType::TypeError, messages::NULLISH_TO_OBJECT));
        }
        let key = self.property_key(k)?;
        let Some(o) = target.as_object() else {
            // A string's own length and indices are not configurable.
            let own = match target.as_string().and_then(|i| self.heap.strings.get(i)) {
                Some(s) => {
                    key == self.heap.length_key
                        || matches!(key, PropertyKey::Index(i) if (i as usize) < s.len())
                }
                None => false,
            };
            if own && strict {
                let k = self.key_display(key);
                return Err(self.throw_error(
                    ErrorType::TypeError,
                    messages::cannot_delete(&k, "[object String]"),
                ));
            }
            return Ok(Value::bool(!own));
        };
        let done = self.o_delete(o, key)?;
        if !done && strict {
            let k = self.key_display(key);
            let text = if self.is_proxy(o) {
                messages::proxy_delete_false(&k)
            } else {
                messages::cannot_delete(&k, "#<Object>")
            };
            return Err(self.throw_error(ErrorType::TypeError, text));
        }
        Ok(Value::bool(done))
    }

    /// CopyDataProperties (section 7.3.25), for `{...source}`.
    pub(crate) fn copy_data_properties(&mut self, target: Value, source: Value) -> Res<()> {
        self.copy_data_properties_except(target, source, &[])
    }

    /// CopyDataProperties with `excluded` keys left out.
    fn copy_data_properties_except(
        &mut self,
        target: Value,
        source: Value,
        excluded: &[PropertyKey],
    ) -> Res<()> {
        let Some(s) = source.as_object() else {
            // Primitives but strings have no own enumerable properties.
            if let Some(units) = self.string_units(source) {
                for (i, unit) in units.iter().enumerate() {
                    let key = PropertyKey::Index(i as u32);
                    if excluded.contains(&key) {
                        continue;
                    }
                    let c = self.new_string_units(&[*unit])?;
                    self.define_value(target, key, c)?;
                }
            }
            return Ok(());
        };
        for key in self.o_own_keys(s)? {
            if excluded.contains(&key) {
                continue;
            }
            if let Some(p) = self.o_get_own_property(s, key)?
                && enumerable(&p)
            {
                let v = self.get_value(source, key)?;
                self.define_value(target, key, v)?;
            }
        }
        Ok(())
    }

    /// An object pattern's rest (section 14.3.3.1): a new object with
    /// `source`'s own enumerable properties but the keys in the array
    /// `excluded`, which the pattern's properties named.
    #[inline(never)]
    pub(crate) fn copy_rest(&mut self, source: Value, excluded: Value) -> Res<Value> {
        let mut keys = Vec::new();
        if let Some(o) = excluded.as_object() {
            let len = self
                .heap
                .objects
                .get(o)
                .and_then(|c| c.array_length())
                .map_or(0, |(len, _)| len);
            for i in 0..len {
                let v = self.get_value(excluded, PropertyKey::Index(i))?;
                keys.push(self.property_key(v)?);
            }
        }
        let proto = self.realm().object_prototype;
        let target = self.alloc_object(ObjectCell::new(proto, ObjectKind::Ordinary))?;
        self.vm.temps.push(target);
        let copied = self.copy_data_properties_except(target, source, &keys);
        self.vm.temps.pop();
        copied.map(|()| target)
    }

    /// SetFunctionName (section 10.2.9) by a computed key.
    pub(crate) fn set_function_name(&mut self, f: Value, key: Value, prefix: usize) -> Res<()> {
        let mut name: Vec<u16> = match prefix {
            1 => "get ".encode_utf16().collect(),
            2 => "set ".encode_utf16().collect(),
            _ => Vec::new(),
        };
        if let Some((RefKind::Symbol, s)) = key.as_reference() {
            let d = self
                .heap
                .symbols
                .get(s)
                .map(|c| c.description)
                .unwrap_or(Value::UNDEFINED);
            if let Some(d) = self.string_units(d) {
                name.push(u16::from(b'['));
                name.extend(d);
                name.push(u16::from(b']'));
            }
        } else if let Some(u) = self.string_units(key) {
            name.extend(u);
        }
        let s = self.new_string_units(&name)?;
        let name_key = self.realm().name_key;
        if let Some(o) = f.as_object() {
            self.heap
                .define_own_property(o, name_key, hidden_readonly(s));
        }
        Ok(())
    }

    /// The `in` operator (section 13.10.1).
    pub(crate) fn has_in(&mut self, key: Value, object: Value) -> Res<bool> {
        let Some(o) = object.as_object() else {
            let k = self.display(key);
            let v = self.display(object);
            return Err(self.throw_error(ErrorType::TypeError, messages::in_not_object(&k, &v)));
        };
        let key = self.property_key(key)?;
        self.o_has_property(o, key)
    }

    /// InstanceofOperator (section 13.10.2).
    pub(crate) fn instance_of(&mut self, value: Value, target: Value) -> Res<bool> {
        if target.as_object().is_none() {
            return Err(self.throw_error(ErrorType::TypeError, messages::INSTANCEOF_NOT_OBJECT));
        }
        let key = self.well_known_key(crate::builtins::Wk::HasInstance);
        let h = self.get_method(target, key)?;
        if !h.is_undefined() {
            let r = self.call_value(h, target, &[value])?;
            return Ok(self.to_boolean(r));
        }
        if self.callable(target).is_none() {
            return Err(self.throw_error(ErrorType::TypeError, messages::INSTANCEOF_NOT_CALLABLE));
        }
        self.ordinary_has_instance(target, value)
    }

    // Bindings by name.

    /// A global read (section 9.1.1.4.6): the script's lexical bindings,
    /// then the global object's properties.
    pub(crate) fn global_get(&mut self, key: PropertyKey, typeof_: bool) -> Res<Value> {
        if let Some(b) = self.realm().lexical.get(&key) {
            if b.value == Value::UNINITIALIZED {
                let name = self.key_display(key);
                return Err(self.throw_error(
                    ErrorType::ReferenceError,
                    messages::before_initialization(&name),
                ));
            }
            return Ok(b.value);
        }
        let global = self.realm().global;
        let g = global.as_object().unwrap_or(0);
        if !self.heap.has_property(g, key) {
            if typeof_ {
                return Ok(Value::UNDEFINED);
            }
            let name = self.key_display(key);
            return Err(self.throw_error(ErrorType::ReferenceError, messages::not_defined(&name)));
        }
        self.get_value(global, key)
    }

    pub(crate) fn global_set(&mut self, key: PropertyKey, value: Value, strict: bool) -> Res<()> {
        if let Some(b) = self.realm().lexical.get(&key).copied() {
            if b.value == Value::UNINITIALIZED {
                let name = self.key_display(key);
                return Err(self.throw_error(
                    ErrorType::ReferenceError,
                    messages::before_initialization(&name),
                ));
            }
            if b.constant {
                return Err(self.throw_error(ErrorType::TypeError, messages::CONST_ASSIGNMENT));
            }
            if let Some(realm) = self.realm_mut()
                && let Some(b) = realm.lexical.get_mut(&key)
            {
                b.value = value;
            }
            return Ok(());
        }
        let global = self.realm().global;
        let g = global.as_object().unwrap_or(0);
        if strict && !self.heap.has_property(g, key) {
            let name = self.key_display(key);
            return Err(self.throw_error(ErrorType::ReferenceError, messages::not_defined(&name)));
        }
        self.set_value(global, key, value, strict)
    }

    /// A binding looked up by name through the environments (`with`,
    /// `eval`), then the global ones. A `with` object's binding is found
    /// through its `[[HasProperty]]` (a proxy's `has` trap), then
    /// unless its `@@unscopables` names it (section 9.1.1.2.1); each
    /// lookup asks again, as the reference browser does for a compound
    /// assignment's read and its write (the J4.6 refs probe).
    fn find_dynamic(&mut self, context: Value, key: PropertyKey) -> Res<Option<Found>> {
        let name = match key {
            PropertyKey::Atom(a) => Some(self.heap.atoms.units(a).to_vec()),
            _ => None,
        };
        let mut at = context;
        while let Some((RefKind::Internal, i)) = at.as_reference() {
            let Some(InternalCell::Env(e)) = self.heap.internals.get(i) else {
                break;
            };
            let (object, scope, vars, parent) = (e.object, e.scope, e.vars, e.parent);
            if let Some(o) = object.as_object() {
                // The environment stays reachable from the frame's
                // context while the traps and getters run.
                if self.o_has_property(o, key)? && !self.unscopable(o, key)? {
                    return Ok(Some(Found::With(object)));
                }
            } else if let (Some((unit, scope)), Some(name)) = (scope, &name)
                && let Some(info) = self
                    .vm
                    .units
                    .get(unit as usize)
                    .and_then(|u| u.code.scopes.get(scope as usize))
                && let Some(slot) = info.names.iter().position(|n| **n == **name)
            {
                return Ok(Some(Found::Slot(i, slot)));
            }
            // What a sloppy direct eval declared here.
            if let Some(o) = vars.as_object()
                && self.heap.has_property(o, key)
            {
                return Ok(Some(Found::Vars(vars)));
            }
            at = parent;
        }
        Ok(None)
    }

    /// Whether `with` object `o`'s `@@unscopables` blocks `key`
    /// (section 9.1.1.2.1, step 5).
    fn unscopable(&mut self, o: u32, key: PropertyKey) -> Res<bool> {
        let k = self.well_known_key(crate::builtins::Wk::Unscopables);
        let u = self.o_get(o, k, Value::reference(RefKind::Object, o))?;
        let Some(uo) = u.as_object() else {
            return Ok(false);
        };
        self.vm.temps.push(u);
        let blocked = self.o_get(uo, key, u);
        self.vm.temps.pop();
        Ok(self.to_boolean(blocked?))
    }

    pub(crate) fn dynamic_get(
        &mut self,
        context: Value,
        key: PropertyKey,
        typeof_: bool,
    ) -> Res<Value> {
        match self.find_dynamic(context, key)? {
            Some(Found::Slot(env, slot)) => {
                let v = self.slot(Some(env), slot);
                if v == Value::UNINITIALIZED {
                    let name = self.key_display(key);
                    return Err(self.throw_error(
                        ErrorType::ReferenceError,
                        messages::before_initialization(&name),
                    ));
                }
                Ok(v)
            }
            Some(Found::With(object) | Found::Vars(object)) => self.get_value(object, key),
            None => self.global_get(key, typeof_),
        }
    }

    /// A callee looked up by name, and its `this`: a `with` object it
    /// was found in, otherwise undefined (WithBaseObject, section
    /// 9.1.1.2.10).
    pub(crate) fn dynamic_callee(
        &mut self,
        context: Value,
        key: PropertyKey,
    ) -> Res<(Value, Value)> {
        match self.find_dynamic(context, key)? {
            Some(Found::With(object)) => Ok((self.get_value(object, key)?, object)),
            Some(Found::Vars(object)) => Ok((self.get_value(object, key)?, Value::UNDEFINED)),
            Some(Found::Slot(env, slot)) => {
                let v = self.slot(Some(env), slot);
                if v == Value::UNINITIALIZED {
                    let name = self.key_display(key);
                    return Err(self.throw_error(
                        ErrorType::ReferenceError,
                        messages::before_initialization(&name),
                    ));
                }
                Ok((v, Value::UNDEFINED))
            }
            None => Ok((self.global_get(key, false)?, Value::UNDEFINED)),
        }
    }

    pub(crate) fn dynamic_set(
        &mut self,
        context: Value,
        key: PropertyKey,
        value: Value,
        strict: bool,
    ) -> Res<()> {
        // The lookup may run a proxy's `has` trap or an `@@unscopables`
        // getter, which can collect: the value is held across it.
        self.vm.temps.push(value);
        let found = self.find_dynamic(context, key);
        self.vm.temps.pop();
        match found? {
            Some(Found::Slot(env, slot)) => {
                if self.slot(Some(env), slot) == Value::UNINITIALIZED {
                    let name = self.key_display(key);
                    return Err(self.throw_error(
                        ErrorType::ReferenceError,
                        messages::before_initialization(&name),
                    ));
                }
                match self.slot_constant(env, slot) {
                    1 => Err(self.throw_error(ErrorType::TypeError, messages::CONST_ASSIGNMENT)),
                    2 if strict => {
                        Err(self.throw_error(ErrorType::TypeError, messages::CONST_ASSIGNMENT))
                    }
                    2 => Ok(()),
                    _ => {
                        self.set_slot(Some(env), slot, value);
                        Ok(())
                    }
                }
            }
            Some(Found::With(object) | Found::Vars(object)) => {
                self.set_value(object, key, value, strict)
            }
            None => self.global_set(key, value, strict),
        }
    }

    /// What an assignment by name meets in environment `env`'s slot
    /// ([`crate::compiler::ScopeInfo::constant`]).
    fn slot_constant(&self, env: u32, slot: usize) -> u8 {
        let Some(InternalCell::Env(e)) = self.heap.internals.get(env) else {
            return 0;
        };
        e.scope
            .and_then(|(unit, scope)| {
                self.vm
                    .units
                    .get(unit as usize)?
                    .code
                    .scopes
                    .get(scope as usize)?
                    .constant
                    .get(slot)
                    .copied()
            })
            .unwrap_or(0)
    }

    pub(crate) fn dynamic_delete(&mut self, context: Value, key: PropertyKey) -> Res<bool> {
        match self.find_dynamic(context, key)? {
            Some(Found::Slot(..)) => Ok(false),
            Some(Found::With(object) | Found::Vars(object)) => match object.as_object() {
                Some(o) => self.o_delete(o, key),
                None => Ok(false),
            },
            None => {
                if self.realm().lexical.contains_key(&key) {
                    return Ok(false);
                }
                let g = self.realm().global.as_object().unwrap_or(0);
                Ok(self.heap.delete(g, key))
            }
        }
    }

    // Making things.

    /// A closure of function `func` of unit `unit` over `env`, with its
    /// `length`, `name` and, for an ordinary function, `prototype`.
    pub(crate) fn make_closure(&mut self, unit: u32, func: u32, env: Value) -> Res<Value> {
        let (kind, length, name) = match self
            .vm
            .units
            .get(unit as usize)
            .and_then(|u| u.code.functions.get(func as usize))
        {
            Some(f) => (f.kind, f.length, f.name.clone()),
            None => return Err(Abrupt::Throw),
        };
        let generator = kind == FunctionKind::Generator;
        let proto = match kind {
            FunctionKind::Generator => self.realm().intrinsics.generator_function_prototype,
            FunctionKind::Async => self.realm().intrinsics.async_function_prototype,
            FunctionKind::AsyncGenerator => {
                self.realm().intrinsics.async_generator_function_prototype
            }
            _ => self.realm().function_prototype,
        };
        let f = self.alloc_object(ObjectCell::function(
            proto,
            Callable::Code { unit, func, env },
        ))?;
        self.vm.temps.push(f);
        let mut result = self.finish_function(f, length, &name, kind == FunctionKind::Normal);
        let async_generator = kind == FunctionKind::AsyncGenerator;
        if (generator || async_generator) && result.is_ok() {
            // Its `prototype`: what its generators inherit from, writable
            // only, with no `constructor` (section 15.5.4).
            let gp = if async_generator {
                self.realm().intrinsics.async_generator_prototype
            } else {
                self.realm().intrinsics.generator_prototype
            };
            result = self
                .alloc_object(ObjectCell::new(gp, ObjectKind::Ordinary))
                .map(|p| {
                    if let Some(o) = f.as_object() {
                        let key = self.realm().prototype_key;
                        self.heap.define_own_property(
                            o,
                            key,
                            Descriptor {
                                value: Some(p),
                                writable: Some(true),
                                enumerable: Some(false),
                                configurable: Some(false),
                                ..Descriptor::default()
                            },
                        );
                    }
                });
        }
        self.vm.temps.pop();
        result.map(|()| f)
    }

    /// A function's own `length`, `name` and maybe `prototype`.
    pub(crate) fn finish_function(
        &mut self,
        f: Value,
        length: u32,
        name: &[u16],
        with_prototype: bool,
    ) -> Res<()> {
        let Some(o) = f.as_object() else {
            return Ok(());
        };
        let (length_key, name_key) = (self.heap.length_key, self.realm().name_key);
        self.heap.define_own_property(
            o,
            length_key,
            hidden_readonly(Value::number(f64::from(length))),
        );
        let n = if name.is_empty() {
            self.realm().strings.empty
        } else {
            self.new_string_units(name)?
        };
        self.heap
            .define_own_property(o, name_key, hidden_readonly(n));
        if with_prototype {
            let object_proto = self.realm().object_prototype;
            let p = self.alloc_object(ObjectCell::new(object_proto, ObjectKind::Ordinary))?;
            let (prototype_key, constructor_key) =
                (self.realm().prototype_key, self.realm().constructor_key);
            if let Some(po) = p.as_object() {
                self.heap
                    .define_own_property(po, constructor_key, hidden(f));
            }
            self.heap.define_own_property(
                o,
                prototype_key,
                Descriptor {
                    value: Some(p),
                    writable: Some(true),
                    enumerable: Some(false),
                    configurable: Some(false),
                    ..Descriptor::default()
                },
            );
        }
        Ok(())
    }

    /// The arguments object of `frame` (sections 10.4.4.6 and 10.4.4.7):
    /// its arguments' values, `length` and `callee`; mapped, each index
    /// below the parameter count reads and writes its parameter's slot of
    /// the function's environment, and `callee` is the function; else
    /// `callee` throws.
    pub(crate) fn make_arguments(&mut self, frame: &super::Frame, mapped: bool) -> Res<Value> {
        let (start, argc) = (frame.args as usize, (frame.base - frame.args) as usize);
        let mut internal = Vec::new();
        if mapped {
            let map = self
                .vm
                .units
                .get(frame.unit as usize)
                .and_then(|u| u.code.functions.get(frame.func as usize))
                .map(|f| f.mapped.clone())
                .unwrap_or_default();
            internal.push(frame.fn_context);
            internal.extend(map.iter().take(argc).map(|&m| {
                if m == 0 {
                    Value::HOLE
                } else {
                    Value::int(i32::try_from(m - 1).unwrap_or(0))
                }
            }));
        }
        let proto = self.realm().object_prototype;
        let a = self.alloc_object(ObjectCell::with_internal(
            proto,
            ObjectKind::Arguments,
            internal,
        ))?;
        let Some(o) = a.as_object() else {
            return Ok(a);
        };
        for i in 0..argc {
            let v = self
                .vm
                .stack
                .get(start + i)
                .copied()
                .unwrap_or(Value::UNDEFINED);
            self.heap
                .define_own_property(o, PropertyKey::Index(i as u32), Descriptor::data(v));
        }
        let length_key = self.heap.length_key;
        self.heap
            .define_own_property(o, length_key, hidden(Value::number(argc as f64)));
        let callee_key = self
            .heap
            .atoms
            .key(&"callee".encode_utf16().collect::<Vec<u16>>());
        let callee = if mapped {
            hidden(frame.closure)
        } else {
            let t = self.realm().intrinsics.throw_type_error;
            Descriptor {
                get: Some(t),
                set: Some(t),
                enumerable: Some(false),
                configurable: Some(false),
                ..Descriptor::default()
            }
        };
        self.heap.define_own_property(o, callee_key, callee);
        // `@@iterator`: `Array.prototype.values` (sections 10.4.4.6 and
        // 10.4.4.7).
        let key = self.well_known_key(crate::builtins::Wk::Iterator);
        let values = self.realm().intrinsics.array_values;
        self.heap.define_own_property(o, key, hidden(values));
        Ok(a)
    }

    /// An array of `count` values from `start` on the register file.
    pub(crate) fn make_array_from_stack(&mut self, start: usize, count: usize) -> Res<Value> {
        let proto = self.realm().array_prototype;
        let a = self.alloc_object(ObjectCell::new(proto, ObjectKind::Array))?;
        if let Some(o) = a.as_object() {
            for i in 0..count {
                let v = self
                    .vm
                    .stack
                    .get(start + i)
                    .copied()
                    .unwrap_or(Value::UNDEFINED);
                self.heap
                    .define_own_property(o, PropertyKey::Index(i as u32), Descriptor::data(v));
            }
        }
        Ok(a)
    }

    // for-in.

    /// EnumerateObjectProperties (section 14.7.5.9): the enumerable string
    /// keys along the prototype chain, each once, a shadowing property
    /// hiding its name further out whether enumerable or not.
    pub(crate) fn for_in_start(&mut self, v: Value) -> Res<Value> {
        let object = if v.is_nullish() { Value::UNDEFINED } else { v };
        let mut names: Vec<(PropertyKey, Value)> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut at = object.as_object();
        if let Some(units) = self.string_units(object) {
            names
                .extend((0..units.len()).map(|i| (PropertyKey::Index(i as u32), Value::UNDEFINED)));
            at = self.primitive_proto(object).as_object();
        }
        // Each object's keys, then its prototype; an ordinary object's
        // keys filtered now, a proxy's each as it comes (its descriptor
        // read then): the reference browser's trap order.
        let base = self.vm.temps.len();
        // The object, rooted while its traps run (they may collect).
        self.vm.temps.push(object);
        let walked = (|| -> Res<()> {
            while let Some(o) = at {
                let keys = self.o_own_keys(o)?;
                let proxy = self.is_proxy(o);
                if proxy {
                    // Kept until the keys are made: each key holds it.
                    self.vm.temps.push(Value::reference(RefKind::Object, o));
                }
                at = self.o_get_prototype_of(o)?.as_object();
                for key in keys {
                    if matches!(key, PropertyKey::Symbol(_)) || !seen.insert(key) {
                        continue;
                    }
                    if proxy {
                        names.push((key, Value::reference(RefKind::Object, o)));
                    } else if self
                        .o_get_own_property(o, key)?
                        .is_some_and(|p| enumerable(&p))
                    {
                        names.push((key, Value::UNDEFINED));
                    }
                }
            }
            Ok(())
        })();
        if let Err(e) = walked {
            self.vm.temps.truncate(base);
            return Err(e);
        }
        let made = self.alloc_internal(InternalCell::ForIn(ForIn {
            object,
            keys: Vec::new(),
            sources: Vec::new(),
            next: 0,
        }));
        let it = match made {
            Ok(it) => it,
            Err(e) => {
                self.vm.temps.truncate(base);
                return Err(e);
            }
        };
        self.vm.temps.push(it);
        let mut result = Ok(it);
        for (key, source) in names {
            let s = match key {
                PropertyKey::Index(i) => self.new_string(&i.to_string()),
                PropertyKey::Atom(a) => {
                    let units = self.heap.atoms.units(a).to_vec();
                    self.new_string_units(&units)
                }
                PropertyKey::Symbol(_) => continue,
            };
            match s {
                Ok(s) => {
                    if let Some((_, i)) = it.as_reference() {
                        self.heap.with_internal(i, |c| {
                            if let InternalCell::ForIn(f) = c {
                                f.keys.push(s);
                                f.sources.push(source);
                            }
                        });
                    }
                }
                Err(e) => {
                    result = Err(e);
                    break;
                }
            }
        }
        self.vm.temps.truncate(base);
        result
    }

    /// The next key a for-in visits (one deleted since its start is
    /// skipped), or undefined when done. A proxy's key is visited if its
    /// descriptor is enumerable; without one, if the proxy's prototype
    /// has it (the reference browser reads no `has` trap).
    pub(crate) fn for_in_next(&mut self, it: Value) -> Res<Value> {
        let Some((RefKind::Internal, i)) = it.as_reference() else {
            return Ok(Value::UNDEFINED);
        };
        loop {
            let (object, key, source) = match self.heap.internals.get_mut(i) {
                Some(InternalCell::ForIn(f)) => {
                    let Some(&k) = f.keys.get(f.next) else {
                        return Ok(Value::UNDEFINED);
                    };
                    let source = f.sources.get(f.next).copied().unwrap_or(Value::UNDEFINED);
                    f.next += 1;
                    (f.object, k, source)
                }
                _ => return Ok(Value::UNDEFINED),
            };
            let units = self.string_units(key).unwrap_or_default();
            let pk = self.heap.atoms.key(&units);
            let still = match (source.as_object(), object.as_object()) {
                (Some(p), _) => match self.o_get_own_property(p, pk)? {
                    Some(d) => enumerable(&d),
                    None => match self.o_get_prototype_of(p)?.as_object() {
                        Some(up) => self.o_has_property(up, pk)?,
                        None => false,
                    },
                },
                (None, Some(o)) => self.heap.has_property(o, pk),
                (None, None) => true,
            };
            if still {
                return Ok(key);
            }
        }
    }
}

/// A non-enumerable, writable, configurable data property.
pub(crate) fn hidden(value: Value) -> Descriptor {
    Descriptor {
        value: Some(value),
        writable: Some(true),
        enumerable: Some(false),
        configurable: Some(true),
        ..Descriptor::default()
    }
}

/// A non-enumerable, read-only, configurable data property (a
/// function's `length` and `name`).
pub(crate) fn hidden_readonly(value: Value) -> Descriptor {
    Descriptor {
        value: Some(value),
        writable: Some(false),
        enumerable: Some(false),
        configurable: Some(true),
        ..Descriptor::default()
    }
}
