//! `BigInt` (ECMA-262 section 21.2): the constructor, `asIntN` and
//! `asUintN`, the prototype's methods, and the agent's conversions to
//! and from BigInts (ToBigInt, ToNumeric) and the errors their
//! arithmetic refuses with. The values are [`crate::bigint::BigInt`]s
//! in the heap's arena of their own.

use super::{CONFIGURABLE, CONSTRUCTOR, Call, Entry, Name, What, Wk, entry, method};
use crate::agent::Agent;
use std::cmp::Ordering;

use crate::bigint::{BigInt, Fuel, Refused};
use crate::compiler::Op;
use crate::interp::{Abrupt, Res};
use crate::object::ObjectKind;
use crate::value::{RefKind, Value};

/// `BigInt`.
pub(crate) const BIGINT: &[Entry] = &[
    method("asUintN", 2, as_uint_n),
    method("asIntN", 2, as_int_n),
];

/// `BigInt.prototype`.
pub(crate) const BIGINT_PROTO: &[Entry] = &[
    CONSTRUCTOR,
    method("toLocaleString", 0, to_locale_string),
    method("toString", 0, to_string),
    method("valueOf", 0, value_of),
    entry(
        Name::Sym(Wk::ToStringTag),
        What::Str("BigInt"),
        CONFIGURABLE,
    ),
];

pub(crate) const MIXED: &str = "Cannot mix BigInt and other types, use explicit conversions";
pub(crate) const TO_NUMBER: &str = "Cannot convert a BigInt value to a number";

impl Agent {
    /// A BigInt value on the heap.
    pub(crate) fn alloc_bigint(&mut self, b: BigInt) -> Res<Value> {
        if self.reserve(64 + b.bytes()).is_err() {
            return Err(self.out_of_memory());
        }
        let (i, _) = self.heap.bigints.alloc(b);
        Ok(Value::reference(RefKind::BigInt, i))
    }

    /// The BigInt `v` is, if it is one.
    pub(crate) fn bigint_of(&self, v: Value) -> Option<&BigInt> {
        match v.as_reference() {
            Some((RefKind::BigInt, i)) => self.heap.bigints.get(i),
            _ => None,
        }
    }

    pub(crate) fn is_bigint(&self, v: Value) -> bool {
        matches!(v.as_reference(), Some((RefKind::BigInt, _)))
    }

    /// Runs `f` with fuel drawn from the embedder's interrupt hook, so
    /// that a long BigInt operation stops when the hook says so.
    pub(crate) fn with_fuel<R>(&mut self, f: impl FnOnce(&mut Fuel<'_>) -> R) -> R {
        let mut hook = self.vm.interrupt.take();
        let r = {
            let mut poll = || hook.as_mut().is_some_and(|h| h());
            f(&mut Fuel::new(&mut poll))
        };
        self.vm.interrupt = hook;
        r
    }

    /// What the reference browser throws when BigInt arithmetic refuses;
    /// an interrupt stops the script, as at any other step.
    pub(crate) fn bigint_refused(&mut self, r: Refused) -> Abrupt {
        match r {
            Refused::Interrupted => Abrupt::Terminate,
            Refused::TooBig => self.range_error("Maximum BigInt size exceeded"),
            Refused::DivisionByZero => self.range_error("Division by zero"),
            Refused::NegativeExponent => self.range_error("Exponent must be positive"),
        }
    }

    /// A BigInt result on the heap, or the error it was refused with.
    pub(crate) fn bigint_result(&mut self, r: Result<BigInt, Refused>) -> Res<Value> {
        match r {
            Ok(b) => self.alloc_bigint(b),
            Err(e) => Err(self.bigint_refused(e)),
        }
    }

    /// ToBigInt (section 7.1.13).
    pub(crate) fn bigint_from(&mut self, v: Value) -> Res<BigInt> {
        let p = self.primitive(v, Some(false))?;
        if let Some(b) = self.bigint_of(p) {
            return Ok(b.clone());
        }
        if let Some(b) = p.as_bool() {
            return Ok(BigInt::from_u64(u64::from(b)));
        }
        if let Some(units) = p.as_string().and_then(|_| self.string_units(p)) {
            return match self.with_fuel(|f| BigInt::parse(&units, f)) {
                Err(r) => Err(self.bigint_refused(r)),
                Ok(Some(b)) => Ok(b),
                Ok(None) => {
                    let text = String::from_utf16_lossy(&units);
                    Err(self.syntax_error(format!("Cannot convert {text} to a BigInt")))
                }
            };
        }
        let text = match p.as_reference() {
            Some((RefKind::Symbol, i)) => self.key_display(crate::string::PropertyKey::Symbol(i)),
            _ => self.display(p),
        };
        Err(self.type_error(format!("Cannot convert {text} to a BigInt")))
    }

    /// ToNumeric (section 7.1.3): a Number or a BigInt.
    pub(crate) fn numeric(&mut self, v: Value) -> Res<Value> {
        if v.is_number() || self.is_bigint(v) {
            return Ok(v);
        }
        let p = self.primitive(v, Some(false))?;
        if self.is_bigint(p) {
            return Ok(p);
        }
        Ok(Value::number(self.number(p)?))
    }

    /// thisBigIntValue (section 21.2.3.4.1): the BigInt primitive.
    fn this_bigint_value(&mut self, v: Value, method: &str) -> Res<Value> {
        match self.this_primitive(v, ObjectKind::BigInt) {
            Some(p) if self.is_bigint(p) => Ok(p),
            _ => Err(self.type_error(format!(
                "BigInt.prototype.{method} requires that 'this' be a BigInt"
            ))),
        }
    }

    fn this_bigint(&mut self, v: Value, method: &str) -> Res<BigInt> {
        let p = self.this_bigint_value(v, method)?;
        Ok(self.bigint_of(p).cloned().unwrap_or_default())
    }

    /// A binary operator of two BigInts (section 6.1.6.2); either not
    /// a BigInt, the reference browser's TypeError for mixing.
    pub(crate) fn bigint_binary(&mut self, op: Op, a: Value, b: Value) -> Res<Value> {
        let (Some(x), Some(y)) = (self.bigint_of(a).cloned(), self.bigint_of(b).cloned()) else {
            return Err(self.type_error(MIXED));
        };
        if !matches!(
            op,
            Op::Add
                | Op::Sub
                | Op::Mul
                | Op::Div
                | Op::Mod
                | Op::Exp
                | Op::BitAnd
                | Op::BitOr
                | Op::BitXor
                | Op::Shl
                | Op::Shr
        ) {
            return Err(self.type_error("BigInts have no unsigned right shift, use >> instead"));
        }
        let r = self.with_fuel(|f| match op {
            Op::Add => x.add(&y, f),
            Op::Sub => x.sub(&y, f),
            Op::Mul => x.mul(&y, f),
            Op::Div => x.divrem(&y, f).map(|(q, _)| q),
            Op::Mod => x.divrem(&y, f).map(|(_, r)| r),
            Op::Exp => x.pow(&y, f),
            Op::BitAnd => x.and(&y, f),
            Op::BitOr => x.or(&y, f),
            Op::BitXor => x.xor(&y, f),
            Op::Shl => x.shift_left(&y, f),
            _ => x.shift_left(&y.neg(), f),
        });
        self.bigint_result(r)
    }

    /// The relational operators where a primitive operand is a BigInt
    /// (IsLessThan, section 7.2.13): a string as a BigInt if it is one
    /// (else every comparison false), a number by its mathematical value.
    pub(crate) fn relational_bigint(&mut self, op: Op, a: Value, b: Value) -> Res<bool> {
        let as_big = |agent: &mut Agent, v: Value| -> Res<Option<Option<BigInt>>> {
            if let Some(b) = agent.bigint_of(v) {
                return Ok(Some(Some(b.clone())));
            }
            let Some(units) = v.as_string().and_then(|_| agent.string_units(v)) else {
                return Ok(None);
            };
            match agent.with_fuel(|f| BigInt::parse(&units, f)) {
                Ok(b) => Ok(Some(b)),
                Err(r) => Err(agent.bigint_refused(r)),
            }
        };
        let (x, y) = (as_big(self, a)?, as_big(self, b)?);
        let order = match (x, y) {
            (Some(None), _) | (_, Some(None)) => None,
            (Some(Some(x)), Some(Some(y))) => Some(x.compare(&y)),
            (Some(Some(x)), None) => {
                let n = self.number(b)?;
                x.compare_number(n)
            }
            (None, Some(Some(y))) => {
                let n = self.number(a)?;
                y.compare_number(n).map(Ordering::reverse)
            }
            (None, None) => None,
        };
        Ok(match order {
            None => false,
            Some(o) => match op {
                Op::TestLt => o == Ordering::Less,
                Op::TestGt => o == Ordering::Greater,
                Op::TestLtEq => o != Ordering::Greater,
                _ => o != Ordering::Less,
            },
        })
    }

    /// ToIndex of BigInt.asIntN's and asUintN's bits.
    fn bit_count(&mut self, v: Value) -> Res<u64> {
        let n = self.integer(v)?;
        if !(0.0..=9_007_199_254_740_991.0).contains(&n) {
            let text = crate::number::number_to_string(n);
            return Err(self.range_error(format!(
                "Invalid value {text}: not (convertible to) a safe integer"
            )));
        }
        Ok(n as u64)
    }
}

/// The `BigInt` function (section 21.2.1.1): no constructor.
pub(crate) fn construct(agent: &mut Agent, c: Call) -> Res<Value> {
    if !c.new_target.is_undefined() {
        return Err(agent.type_error("BigInt is not a constructor"));
    }
    let p = agent.primitive(agent.arg(c, 0), Some(false))?;
    if let Some(n) = p.as_number() {
        let Some(b) = BigInt::from_f64(n) else {
            let text = crate::number::number_to_string(n);
            return Err(agent.range_error(format!(
                "The number {text} cannot be converted to a BigInt because it is not an integer"
            )));
        };
        return agent.alloc_bigint(b);
    }
    let b = agent.bigint_from(p)?;
    agent.alloc_bigint(b)
}

/// `BigInt.asUintN` (section 21.2.2.3).
fn as_uint_n(agent: &mut Agent, c: Call) -> Res<Value> {
    let bits = agent.bit_count(agent.arg(c, 0))?;
    let b = agent.bigint_from(agent.arg(c, 1))?;
    let r = agent.with_fuel(|f| b.as_uint_n(bits, f));
    agent.bigint_result(r)
}

/// `BigInt.asIntN` (section 21.2.2.2).
fn as_int_n(agent: &mut Agent, c: Call) -> Res<Value> {
    let bits = agent.bit_count(agent.arg(c, 0))?;
    let b = agent.bigint_from(agent.arg(c, 1))?;
    let r = agent.with_fuel(|f| b.as_int_n(bits, f));
    agent.bigint_result(r)
}

/// `BigInt.prototype.toString` (section 21.2.3.3).
fn to_string(agent: &mut Agent, c: Call) -> Res<Value> {
    let b = agent.this_bigint(c.this, "toString")?;
    let r = agent.arg(c, 0);
    let radix = if r.is_undefined() {
        10.0
    } else {
        agent.integer(r)?
    };
    if !(2.0..=36.0).contains(&radix) {
        return Err(agent.range_error("toString() radix argument must be between 2 and 36"));
    }
    let text = agent.with_fuel(|f| b.to_string_radix(radix as u32, f));
    match text {
        Ok(text) => agent.new_string(&text),
        Err(r) => Err(agent.bigint_refused(r)),
    }
}

/// `BigInt.prototype.toLocaleString` (section 21.2.3.2): the digits
/// grouped by threes, as the reference browser's default locale shows
/// them (Intl itself is Silver's).
fn to_locale_string(agent: &mut Agent, c: Call) -> Res<Value> {
    let b = agent.this_bigint(c.this, "toLocaleString")?;
    let digits = match agent.with_fuel(|f| b.to_string_radix(10, f)) {
        Ok(d) => d,
        Err(r) => return Err(agent.bigint_refused(r)),
    };
    let (sign, digits) = match digits.strip_prefix('-') {
        Some(d) => ("-", d),
        None => ("", digits.as_str()),
    };
    let mut grouped = String::from(sign);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    agent.new_string(&grouped)
}

/// `BigInt.prototype.valueOf` (section 21.2.3.4).
fn value_of(agent: &mut Agent, c: Call) -> Res<Value> {
    agent.this_bigint_value(c.this, "valueOf")
}
