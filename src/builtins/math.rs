//! `Math` (ECMA-262 section 21.3). `Math.sumPrecise` takes an iterable,
//! and comes with J3's iterators.

use super::{CONFIGURABLE, Call, Entry, Name, What, Wk, constant, entry, method};
use crate::agent::Agent;
use crate::interp::Res;
use crate::value::Value;

pub(crate) const MATH: &[Entry] = &[
    method("abs", 1, abs),
    method("acos", 1, acos),
    method("acosh", 1, acosh),
    method("asin", 1, asin),
    method("asinh", 1, asinh),
    method("atan", 1, atan),
    method("atanh", 1, atanh),
    method("atan2", 2, atan2),
    method("ceil", 1, ceil),
    method("cbrt", 1, cbrt),
    method("expm1", 1, expm1),
    method("clz32", 1, clz32),
    method("cos", 1, cos),
    method("cosh", 1, cosh),
    method("exp", 1, exp),
    method("floor", 1, floor),
    method("f16round", 1, f16round),
    method("fround", 1, fround),
    method("hypot", 2, hypot),
    method("imul", 2, imul),
    method("log", 1, log),
    method("log1p", 1, log1p),
    method("log2", 1, log2),
    method("log10", 1, log10),
    method("max", 2, max),
    method("min", 2, min),
    method("pow", 2, pow),
    method("random", 0, random),
    method("round", 1, round),
    method("sign", 1, sign),
    method("sin", 1, sin),
    method("sinh", 1, sinh),
    method("sqrt", 1, sqrt),
    method("tan", 1, tan),
    method("tanh", 1, tanh),
    method("trunc", 1, trunc),
    constant("E", std::f64::consts::E),
    constant("LN10", std::f64::consts::LN_10),
    constant("LN2", std::f64::consts::LN_2),
    constant("LOG10E", std::f64::consts::LOG10_E),
    constant("LOG2E", std::f64::consts::LOG2_E),
    constant("PI", std::f64::consts::PI),
    constant("SQRT1_2", std::f64::consts::FRAC_1_SQRT_2),
    constant("SQRT2", std::f64::consts::SQRT_2),
    entry(Name::Sym(Wk::ToStringTag), What::Str("Math"), CONFIGURABLE),
];

fn num(agent: &mut Agent, c: Call, i: usize) -> Res<f64> {
    agent.number(agent.arg(c, i))
}

macro_rules! unary {
    ($($name:ident => $f:expr;)*) => {$(
        fn $name(agent: &mut Agent, c: Call) -> Res<Value> {
            let x = num(agent, c, 0)?;
            let f: fn(f64) -> f64 = $f;
            Ok(Value::number(f(x)))
        }
    )*};
}

unary! {
    abs => f64::abs;
    acos => f64::acos;
    acosh => f64::acosh;
    asin => f64::asin;
    asinh => |x| if x == 0.0 { x } else { x.asinh() };
    atan => f64::atan;
    atanh => |x| if x == 0.0 { x } else { x.atanh() };
    ceil => f64::ceil;
    cbrt => f64::cbrt;
    expm1 => |x| if x == 0.0 { x } else { x.exp_m1() };
    cos => f64::cos;
    cosh => f64::cosh;
    exp => f64::exp;
    floor => f64::floor;
    f16round => to_f16;
    fround => |x| f64::from(x as f32);
    log => f64::ln;
    log1p => |x| if x == 0.0 { x } else { x.ln_1p() };
    log2 => f64::log2;
    log10 => f64::log10;
    round => round_half_up;
    sign => |x| if x.is_nan() || x == 0.0 { x } else { x.signum() };
    sin => f64::sin;
    sinh => |x| if x == 0.0 { x } else { x.sinh() };
    sqrt => f64::sqrt;
    tan => f64::tan;
    tanh => |x| if x == 0.0 { x } else { x.tanh() };
    trunc => f64::trunc;
}

/// `Math.round` (section 21.3.2.28): ties toward +Infinity, a value in
/// `[-0.5, 0)` to -0.
fn round_half_up(x: f64) -> f64 {
    if !x.is_finite() || x == 0.0 || x.abs() >= 4_503_599_627_370_496.0 {
        return x;
    }
    if x > 0.0 && x < 0.5 {
        return 0.0;
    }
    if (-0.5..0.0).contains(&x) {
        return -0.0;
    }
    let f = x.floor();
    if x - f >= 0.5 { f + 1.0 } else { f }
}

/// The nearest binary16 value (ties to even), as a double: rounded once,
/// from the double itself.
fn to_f16(x: f64) -> f64 {
    if !x.is_finite() || x == 0.0 {
        return x;
    }
    let a = x.abs();
    // 65520 and above round to infinity.
    if a >= 65520.0 {
        return f64::INFINITY.copysign(x);
    }
    // The spacing of binary16 values around `a`: 2^(e-10), the subnormals
    // spaced 2^-24.
    let e = a.log2().floor().max(-14.0);
    let ulp = (e - 10.0).exp2();
    let q = a / ulp;
    let fl = q.floor();
    let r = q - fl;
    let n = if r > 0.5 || (r == 0.5 && fl % 2.0 == 1.0) {
        fl + 1.0
    } else {
        fl
    };
    (n * ulp).copysign(x)
}

fn atan2(agent: &mut Agent, c: Call) -> Res<Value> {
    let y = num(agent, c, 0)?;
    let x = num(agent, c, 1)?;
    Ok(Value::number(y.atan2(x)))
}

fn clz32(agent: &mut Agent, c: Call) -> Res<Value> {
    let x = num(agent, c, 0)?;
    let n = crate::interp::ops::to_int32(x) as u32;
    Ok(Value::int(n.leading_zeros() as i32))
}

fn imul(agent: &mut Agent, c: Call) -> Res<Value> {
    let a = crate::interp::ops::to_int32(num(agent, c, 0)?);
    let b = crate::interp::ops::to_int32(num(agent, c, 1)?);
    Ok(Value::int(a.wrapping_mul(b)))
}

fn hypot(agent: &mut Agent, c: Call) -> Res<Value> {
    let mut xs = Vec::with_capacity(c.argc);
    for i in 0..c.argc {
        xs.push(num(agent, c, i)?);
    }
    if xs.iter().any(|x| x.is_infinite()) {
        return Ok(Value::number(f64::INFINITY));
    }
    if xs.iter().any(|x| x.is_nan()) {
        return Ok(Value::number(f64::NAN));
    }
    let m = xs.iter().fold(0.0f64, |m, x| m.max(x.abs()));
    if m == 0.0 {
        return Ok(Value::number(0.0));
    }
    // Scaled, with compensated summation.
    let (mut sum, mut comp) = (0.0f64, 0.0f64);
    for x in xs {
        let r = x / m;
        let y = r * r - comp;
        let t = sum + y;
        comp = (t - sum) - y;
        sum = t;
    }
    Ok(Value::number(sum.sqrt() * m))
}

fn extremum(agent: &mut Agent, c: Call, max: bool) -> Res<Value> {
    let mut result = if max {
        f64::NEG_INFINITY
    } else {
        f64::INFINITY
    };
    // Every argument is converted, even after a NaN.
    let mut nan = false;
    for i in 0..c.argc {
        let x = num(agent, c, i)?;
        if x.is_nan() {
            nan = true;
        } else if max {
            if x > result || (x == 0.0 && result == 0.0 && result.is_sign_negative()) {
                result = x;
            }
        } else if x < result || (x == 0.0 && result == 0.0 && x.is_sign_negative()) {
            result = x;
        }
    }
    Ok(Value::number(if nan { f64::NAN } else { result }))
}

fn max(agent: &mut Agent, c: Call) -> Res<Value> {
    extremum(agent, c, true)
}

fn min(agent: &mut Agent, c: Call) -> Res<Value> {
    extremum(agent, c, false)
}

fn pow(agent: &mut Agent, c: Call) -> Res<Value> {
    let b = num(agent, c, 0)?;
    let e = num(agent, c, 1)?;
    Ok(Value::number(crate::interp::ops::exponentiate(b, e)))
}

/// The xorshift128+ state a seed starts (never all zero).
pub(crate) fn random_state(seed: u64) -> (u64, u64) {
    (seed | 1, seed.rotate_left(29) ^ 0x9e37_79b9_7f4a_7c15)
}

/// `Math.random`: xorshift128+, seeded from the clock unless the
/// embedder seeded it (`Agent::set_random_seed`, design note section
/// 1.10).
fn random(agent: &mut Agent, _: Call) -> Res<Value> {
    if agent.vm.random == (0, 0) {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0x9e37_79b9_7f4a_7c15, |d| d.as_nanos() as u64);
        agent.vm.random = random_state(t);
    }
    let state = &mut agent.vm.random;
    let (mut s1, s0) = *state;
    s1 ^= s1 << 23;
    s1 ^= s1 >> 17;
    s1 ^= s0;
    s1 ^= s0 >> 26;
    *state = (s0, s1);
    let bits = (s0.wrapping_add(s1)) >> 11;
    Ok(Value::number(bits as f64 / (1u64 << 53) as f64))
}
