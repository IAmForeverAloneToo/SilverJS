//! `Number` (ECMA-262 section 21.1).

use super::{CONSTRUCTOR, Call, Entry, HIDDEN, Name, What, constant, entry, method};
use crate::agent::Agent;
use crate::interp::Res;
use crate::number::{number_to_string, to_exponential, to_fixed, to_precision, to_radix};
use crate::object::ObjectKind;
use crate::value::Value;

pub(crate) const STATICS: &[Entry] = &[
    method("isFinite", 1, is_finite),
    method("isInteger", 1, is_integer),
    method("isNaN", 1, is_nan),
    method("isSafeInteger", 1, is_safe_integer),
    entry(
        Name::Str("parseFloat"),
        What::Intrinsic("parseFloat"),
        HIDDEN,
    ),
    entry(Name::Str("parseInt"), What::Intrinsic("parseInt"), HIDDEN),
    constant("MAX_VALUE", f64::MAX),
    constant("MIN_VALUE", 5e-324),
    constant("NaN", f64::NAN),
    constant("NEGATIVE_INFINITY", f64::NEG_INFINITY),
    constant("POSITIVE_INFINITY", f64::INFINITY),
    constant("MAX_SAFE_INTEGER", 9_007_199_254_740_991.0),
    constant("MIN_SAFE_INTEGER", -9_007_199_254_740_991.0),
    constant("EPSILON", f64::EPSILON),
];

pub(crate) const PROTO: &[Entry] = &[
    CONSTRUCTOR,
    method("toExponential", 1, to_exponential_native),
    method("toFixed", 1, to_fixed_native),
    method("toPrecision", 1, to_precision_native),
    method("toString", 1, to_string),
    method("valueOf", 0, value_of),
    method("toLocaleString", 0, to_locale_string),
];

/// `Number(value)` (section 21.1.1.1).
pub(crate) fn construct(agent: &mut Agent, c: Call) -> Res<Value> {
    let n = if c.argc == 0 {
        0.0
    } else {
        agent.number(agent.arg(c, 0))?
    };
    let n = Value::number(n);
    if c.new_target.is_undefined() {
        return Ok(n);
    }
    let default = agent.realm().number_prototype;
    let proto = agent.prototype_from(c.new_target, default)?;
    agent.wrap(n, Some(proto))
}

fn is_finite(agent: &mut Agent, c: Call) -> Res<Value> {
    Ok(Value::bool(
        agent.arg(c, 0).as_number().is_some_and(f64::is_finite),
    ))
}

fn integral(v: Value) -> Option<f64> {
    v.as_number().filter(|n| n.is_finite() && n.trunc() == *n)
}

fn is_integer(agent: &mut Agent, c: Call) -> Res<Value> {
    Ok(Value::bool(integral(agent.arg(c, 0)).is_some()))
}

fn is_nan(agent: &mut Agent, c: Call) -> Res<Value> {
    Ok(Value::bool(
        agent.arg(c, 0).as_number().is_some_and(f64::is_nan),
    ))
}

fn is_safe_integer(agent: &mut Agent, c: Call) -> Res<Value> {
    Ok(Value::bool(
        integral(agent.arg(c, 0)).is_some_and(|n| n.abs() <= 9_007_199_254_740_991.0),
    ))
}

/// thisNumberValue (section 21.1.3.7.1).
fn this_number(agent: &mut Agent, v: Value, method: &str) -> Res<f64> {
    match agent
        .this_primitive(v, ObjectKind::Number)
        .and_then(Value::as_number)
    {
        Some(n) => Ok(n),
        None => Err(agent.type_error(format!(
            "Number.prototype.{method} requires that 'this' be a Number"
        ))),
    }
}

fn to_exponential_native(agent: &mut Agent, c: Call) -> Res<Value> {
    let x = this_number(agent, c.this, "toExponential")?;
    let fd = agent.arg(c, 0);
    let f = agent.integer(fd)?;
    if !x.is_finite() {
        return agent.new_string(&number_to_string(x));
    }
    if !(0.0..=100.0).contains(&f) {
        return Err(agent.range_error("toExponential() argument must be between 0 and 100"));
    }
    let f = (!fd.is_undefined()).then_some(f as usize);
    agent.new_string(&to_exponential(x, f))
}

fn to_fixed_native(agent: &mut Agent, c: Call) -> Res<Value> {
    let x = this_number(agent, c.this, "toFixed")?;
    let f = agent.integer(agent.arg(c, 0))?;
    if !(0.0..=100.0).contains(&f) {
        return Err(agent.range_error("toFixed() digits argument must be between 0 and 100"));
    }
    if !x.is_finite() || x.abs() >= 1e21 {
        return agent.new_string(&number_to_string(x));
    }
    agent.new_string(&to_fixed(x, f as usize))
}

fn to_precision_native(agent: &mut Agent, c: Call) -> Res<Value> {
    let x = this_number(agent, c.this, "toPrecision")?;
    let pv = agent.arg(c, 0);
    if pv.is_undefined() {
        return agent.new_string(&number_to_string(x));
    }
    let p = agent.integer(pv)?;
    if !x.is_finite() {
        return agent.new_string(&number_to_string(x));
    }
    if !(1.0..=100.0).contains(&p) {
        return Err(agent.range_error("toPrecision() argument must be between 1 and 100"));
    }
    agent.new_string(&to_precision(x, p as usize))
}

fn to_string(agent: &mut Agent, c: Call) -> Res<Value> {
    let x = this_number(agent, c.this, "toString")?;
    let r = agent.arg(c, 0);
    let radix = if r.is_undefined() {
        10.0
    } else {
        agent.integer(r)?
    };
    if !(2.0..=36.0).contains(&radix) {
        return Err(agent.range_error("toString() radix must be between 2 and 36"));
    }
    if radix == 10.0 {
        return agent.new_string(&number_to_string(x));
    }
    agent.new_string(&to_radix(x, radix as u32))
}

fn value_of(agent: &mut Agent, c: Call) -> Res<Value> {
    this_number(agent, c.this, "valueOf").map(Value::number)
}

/// The reference browser's default locale (en-US): grouped thousands,
/// at most three fraction digits. Locales are the embedder's Intl (J7).
fn to_locale_string(agent: &mut Agent, c: Call) -> Res<Value> {
    let x = this_number(agent, c.this, "toLocaleString")?;
    if !x.is_finite() {
        let text = if x.is_nan() {
            "NaN"
        } else if x > 0.0 {
            "\u{221e}"
        } else {
            "-\u{221e}"
        };
        return agent.new_string(text);
    }
    let fixed = to_fixed(x.abs(), 3);
    let (int, frac) = fixed.split_once('.').unwrap_or((&fixed, ""));
    let frac = frac.trim_end_matches('0');
    let mut grouped = String::new();
    for (i, ch) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    let sign = if x < 0.0 && (int != "0" || !frac.is_empty()) {
        "-"
    } else {
        ""
    };
    let text = if frac.is_empty() {
        format!("{sign}{grouped}")
    } else {
        format!("{sign}{grouped}.{frac}")
    };
    agent.new_string(&text)
}
