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
use crate::object::{Callable, Descriptor, Got, ObjectKind, OwnProperty, SetOutcome};
use crate::string::PropertyKey;
use crate::value::{RefKind, Value};

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

/// Number::exponentiate (section 6.1.6.1.3), where it differs from
/// `powf`: a NaN exponent, and a base of magnitude 1 to an infinity.
fn exponentiate(base: f64, exponent: f64) -> f64 {
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
    fn out_of_memory(&mut self) -> Abrupt {
        let proto = self.realm().error_proto(ErrorType::RangeError);
        let (o, _) = self
            .heap
            .objects
            .alloc(ObjectCell::new(proto, ObjectKind::Ordinary));
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
        let e = self.alloc_object(ObjectCell::new(proto, ObjectKind::Ordinary))?;
        self.vm.temps.push(e);
        let m = self.new_string(message);
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
            _ => Err(self.throw_error(ErrorType::TypeError, messages::not_supported("BigInt"))),
        }
    }

    /// ToPrimitive (section 7.1.1) by OrdinaryToPrimitive: `hint`
    /// `Some(true)` string, `Some(false)` number, `None` default.
    pub(crate) fn primitive(&mut self, v: Value, hint: Option<bool>) -> Res<Value> {
        if v.as_object().is_none() {
            return Ok(v);
        }
        let realm = self.realm();
        let order = if hint == Some(true) {
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
            _ => Err(self.throw_error(ErrorType::TypeError, messages::not_supported("BigInt"))),
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
        let x = self.number(a)?;
        let y = self.number(b)?;
        Ok(match op {
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
        })
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
            _ if v.as_object().is_some() => "#<Object>".into(),
            _ => "Symbol()".into(),
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
        match self.heap.get(holder, key) {
            Got::Value(v) => Ok(v),
            Got::Getter(g) if g.is_undefined() => Ok(Value::UNDEFINED),
            Got::Getter(g) => self.call_value(g, target, &[]),
        }
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
            let key = self.property_key(k)?;
            return self.get_value(target, key);
        }
        let key = self.property_key(k)?;
        self.get_value(target, key)
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
        match self.heap.set(o, key, value, target) {
            SetOutcome::Done(true) => Ok(()),
            SetOutcome::Done(false) => {
                if strict {
                    let k = self.key_display(key);
                    Err(self
                        .throw_error(ErrorType::TypeError, messages::read_only(&k, "#<Object>")))
                } else {
                    Ok(())
                }
            }
            SetOutcome::Setter(s) => self.call_value(s, target, &[value]).map(|_| ()),
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
        let done = self.heap.delete(o, key);
        if !done && strict {
            let k = self.key_display(key);
            return Err(self.throw_error(
                ErrorType::TypeError,
                messages::cannot_delete(&k, "#<Object>"),
            ));
        }
        Ok(Value::bool(done))
    }

    /// CopyDataProperties (section 7.3.25), for `{...source}`.
    pub(crate) fn copy_data_properties(&mut self, target: Value, source: Value) -> Res<()> {
        let Some(s) = source.as_object() else {
            // Primitives but strings have no own enumerable properties.
            if let Some(units) = self.string_units(source) {
                for (i, unit) in units.iter().enumerate() {
                    let c = self.new_string_units(&[*unit])?;
                    self.define_value(target, PropertyKey::Index(i as u32), c)?;
                }
            }
            return Ok(());
        };
        for key in self.heap.own_keys(s) {
            if let Some(p) = self.heap.get_own_property(s, key)
                && enumerable(&p)
            {
                let v = self.get_value(source, key)?;
                self.define_value(target, key, v)?;
            }
        }
        Ok(())
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
        Ok(self.heap.has_property(o, key))
    }

    /// InstanceofOperator (section 13.10.2), without `@@hasInstance`
    /// (J2.5).
    pub(crate) fn instance_of(&mut self, value: Value, target: Value) -> Res<bool> {
        if target.as_object().is_none() {
            return Err(self.throw_error(ErrorType::TypeError, messages::INSTANCEOF_NOT_OBJECT));
        }
        if self.callable(target).is_none() {
            return Err(self.throw_error(ErrorType::TypeError, messages::INSTANCEOF_NOT_CALLABLE));
        }
        let Some(mut o) = value.as_object() else {
            return Ok(false);
        };
        let key = self.realm().prototype_key;
        let proto = self.get_value(target, key)?;
        if proto.as_object().is_none() {
            let text = self.display(proto);
            return Err(self.throw_error(
                ErrorType::TypeError,
                format!("Function has non-object prototype '{text}' in instanceof check"),
            ));
        }
        loop {
            let p = self.heap.get_prototype_of(o);
            if p == proto {
                return Ok(true);
            }
            match p.as_object() {
                Some(next) => o = next,
                None => return Ok(false),
            }
        }
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
            if let Some(realm) = self.vm.realm.as_mut()
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
    /// `eval`), then the global ones.
    fn find_dynamic(
        &self,
        context: Value,
        key: PropertyKey,
    ) -> Option<(Option<(u32, usize)>, Value)> {
        let name = match key {
            PropertyKey::Atom(a) => Some(self.heap.atoms.units(a).to_vec()),
            _ => None,
        };
        let mut at = context;
        while let Some((RefKind::Internal, i)) = at.as_reference() {
            let Some(InternalCell::Env(e)) = self.heap.internals.get(i) else {
                break;
            };
            if let Some(o) = e.object.as_object() {
                if self.heap.has_property(o, key) {
                    return Some((None, e.object));
                }
            } else if let (Some((unit, scope)), Some(name)) = (e.scope, &name)
                && let Some(info) = self
                    .vm
                    .units
                    .get(unit as usize)
                    .and_then(|u| u.code.scopes.get(scope as usize))
                && let Some(slot) = info.names.iter().position(|n| **n == **name)
            {
                return Some((Some((i, slot)), Value::UNDEFINED));
            }
            at = e.parent;
        }
        None
    }

    pub(crate) fn dynamic_get(
        &mut self,
        context: Value,
        key: PropertyKey,
        typeof_: bool,
    ) -> Res<Value> {
        match self.find_dynamic(context, key) {
            Some((Some((env, slot)), _)) => {
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
            Some((None, object)) => self.get_value(object, key),
            None => self.global_get(key, typeof_),
        }
    }

    pub(crate) fn dynamic_set(
        &mut self,
        context: Value,
        key: PropertyKey,
        value: Value,
        strict: bool,
    ) -> Res<()> {
        match self.find_dynamic(context, key) {
            Some((Some((env, slot)), _)) => {
                self.set_slot(Some(env), slot, value);
                Ok(())
            }
            Some((None, object)) => self.set_value(object, key, value, strict),
            None => self.global_set(key, value, strict),
        }
    }

    pub(crate) fn dynamic_delete(&mut self, context: Value, key: PropertyKey) -> bool {
        match self.find_dynamic(context, key) {
            Some((Some(_), _)) => false,
            Some((None, object)) => object.as_object().is_some_and(|o| self.heap.delete(o, key)),
            None => {
                if self.realm().lexical.contains_key(&key) {
                    return false;
                }
                let g = self.realm().global.as_object().unwrap_or(0);
                self.heap.delete(g, key)
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
        let proto = self.realm().function_prototype;
        let f = self.alloc_object(ObjectCell::function(
            proto,
            Callable::Code { unit, func, env },
        ))?;
        self.vm.temps.push(f);
        let result = self.finish_function(f, length, &name, kind == FunctionKind::Normal);
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

    /// An unmapped arguments object (section 10.4.4.6) of `argc` values
    /// from `start` on the register file. (A sloppy function's mapped
    /// one is J2.5's; its indices are not yet tied to the parameters.)
    pub(crate) fn make_arguments(&mut self, start: usize, argc: usize) -> Res<Value> {
        let proto = self.realm().object_prototype;
        let a = self.alloc_object(ObjectCell::new(proto, ObjectKind::Ordinary))?;
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
        let mut names: Vec<PropertyKey> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut at = object.as_object();
        if let Some(units) = self.string_units(object) {
            names.extend((0..units.len()).map(|i| PropertyKey::Index(i as u32)));
            at = self.primitive_proto(object).as_object();
        }
        while let Some(o) = at {
            for key in self.heap.own_keys(o) {
                if matches!(key, PropertyKey::Symbol(_)) || !seen.insert(key) {
                    continue;
                }
                if self
                    .heap
                    .get_own_property(o, key)
                    .is_some_and(|p| enumerable(&p))
                {
                    names.push(key);
                }
            }
            at = self.heap.get_prototype_of(o).as_object();
        }
        let it = self.alloc_internal(InternalCell::ForIn(ForIn {
            object,
            keys: Vec::new(),
            next: 0,
        }))?;
        self.vm.temps.push(it);
        let mut result = Ok(it);
        for key in names {
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
        self.vm.temps.pop();
        result
    }

    /// The next key a for-in visits (one deleted since its start is
    /// skipped), or undefined when done.
    pub(crate) fn for_in_next(&mut self, it: Value) -> Value {
        let Some((RefKind::Internal, i)) = it.as_reference() else {
            return Value::UNDEFINED;
        };
        loop {
            let (object, key) = match self.heap.internals.get_mut(i) {
                Some(InternalCell::ForIn(f)) => {
                    let Some(&k) = f.keys.get(f.next) else {
                        return Value::UNDEFINED;
                    };
                    f.next += 1;
                    (f.object, k)
                }
                _ => return Value::UNDEFINED,
            };
            let units = self.string_units(key).unwrap_or_default();
            let pk = self.heap.atoms.key(&units);
            let still = match object.as_object() {
                Some(o) => self.heap.has_property(o, pk),
                None => true,
            };
            if still {
                return key;
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
