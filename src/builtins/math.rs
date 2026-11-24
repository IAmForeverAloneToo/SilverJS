//! `Math` (ECMA-262 section 21.3). `Math.sumPrecise` adds its numbers
//! exactly, in a fixed-point accumulator wide enough for any finite
//! double's bits, and rounds once at the end.

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
    method("sumPrecise", 1, sum_precise),
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

/// The bits of a sum of doubles, in units of 2^-1074 (the smallest
/// subnormal): 32 of them a limb, each limb allowed to run over until
/// the next normalization. 70 limbs hold 2^1024 times 2^53 numbers.
struct ExactSum {
    limbs: [i64; 70],
    adds: u32,
}

impl ExactSum {
    fn new() -> ExactSum {
        ExactSum {
            limbs: [0; 70],
            adds: 0,
        }
    }

    /// Adds finite `x` exactly.
    fn add(&mut self, x: f64) {
        let bits = x.to_bits();
        let negative = bits >> 63 == 1;
        let field = ((bits >> 52) & 0x7ff) as u32;
        let fraction = bits & ((1 << 52) - 1);
        let (m, shift) = if field == 0 {
            (fraction, 0)
        } else {
            (fraction | 1 << 52, field - 1)
        };
        let mut wide = u128::from(m) << (shift % 32);
        let mut k = (shift / 32) as usize;
        while wide != 0 {
            let piece = (wide & 0xffff_ffff) as i64;
            if let Some(l) = self.limbs.get_mut(k) {
                *l += if negative { -piece } else { piece };
            }
            wide >>= 32;
            k += 1;
        }
        self.adds += 1;
        if self.adds >= 1 << 20 {
            self.normalize();
        }
    }

    /// Every limb but the last in 0..2^32, the last holding the sign.
    fn normalize(&mut self) {
        let mut carry = 0i64;
        let top = self.limbs.len() - 1;
        for (k, l) in self.limbs.iter_mut().enumerate() {
            let v = *l + carry;
            if k == top {
                *l = v;
            } else {
                *l = v & 0xffff_ffff;
                carry = v >> 32;
            }
        }
        self.adds = 0;
    }

    /// The sum rounded to the nearest double, ties to even; +0 for zero.
    fn round(&mut self) -> f64 {
        self.normalize();
        let negative = self.limbs.last().is_some_and(|&l| l < 0);
        if negative {
            self.limbs.iter_mut().for_each(|l| *l = -*l);
            self.normalize();
        }
        let bit = |i: usize| {
            self.limbs
                .get(i / 32)
                .is_some_and(|&l| (l >> (i % 32)) & 1 == 1)
        };
        let Some(high) = (0..self.limbs.len() * 32).rev().find(|&i| bit(i)) else {
            return 0.0;
        };
        // The 53 bits from the highest, and the rounding bit and the
        // sticky bits below them; fewer for a subnormal, which is exact.
        let shift = high.saturating_sub(52);
        let mut mantissa = (shift..=high)
            .rev()
            .fold(0u64, |m, i| m << 1 | u64::from(bit(i)));
        let mut shift = shift as u64;
        if shift > 0 {
            let half = bit(shift as usize - 1);
            let sticky = (0..shift as usize - 1).any(bit);
            if half && (sticky || mantissa & 1 == 1) {
                mantissa += 1;
                if mantissa == 1 << 53 {
                    mantissa >>= 1;
                    shift += 1;
                }
            }
        }
        let magnitude = if mantissa < 1 << 52 {
            f64::from_bits(mantissa)
        } else {
            let field = shift + 1;
            if field >= 2047 {
                f64::INFINITY
            } else {
                f64::from_bits(field << 52 | (mantissa - (1 << 52)))
            }
        };
        if negative { -magnitude } else { magnitude }
    }
}

/// `Math.sumPrecise` (section 21.3.2.34).
fn sum_precise(agent: &mut Agent, c: Call) -> Res<Value> {
    #[derive(PartialEq)]
    enum State {
        MinusZero,
        Finite,
        PlusInfinity,
        MinusInfinity,
        NotANumber,
    }
    let items = agent.arg(c, 0);
    let text = if items.as_object().is_some() || items.is_nullish() {
        agent.display(items)
    } else {
        let kind = match items.as_reference() {
            Some((crate::value::RefKind::Symbol, _)) => "Symbol",
            Some((crate::value::RefKind::BigInt, _)) => "BigInt",
            _ if items.as_bool().is_some() => "Boolean",
            _ => "Number",
        };
        format!("[object {kind}]")
    };
    let (it, next) = agent.get_iterator(items, &text)?;
    agent.vm.temps.extend([it, next]);
    let r = (|| {
        let mut state = State::MinusZero;
        let mut sum = ExactSum::new();
        let mut count = 0.0;
        while let Some(v) = agent.iterator_step(it, next)? {
            count += 1.0;
            if count >= 9_007_199_254_740_992.0 {
                let e = agent.range_error("Too many values to sum precisely");
                agent.iterator_close_quiet(it)?;
                return Err(e);
            }
            let Some(n) = v.as_number() else {
                let t = agent.type_of(v);
                let found = agent.display(t);
                let e = agent.type_error(format!(
                    "Type of 'Iterator value' must be 'number', found '{found}'"
                ));
                agent.iterator_close_quiet(it)?;
                return Err(e);
            };
            if state == State::NotANumber {
                continue;
            }
            if n.is_nan() {
                state = State::NotANumber;
            } else if n == f64::INFINITY {
                state = if state == State::MinusInfinity {
                    State::NotANumber
                } else {
                    State::PlusInfinity
                };
            } else if n == f64::NEG_INFINITY {
                state = if state == State::PlusInfinity {
                    State::NotANumber
                } else {
                    State::MinusInfinity
                };
            } else if !(n == 0.0 && n.is_sign_negative())
                && matches!(state, State::MinusZero | State::Finite)
            {
                state = State::Finite;
                sum.add(n);
            }
        }
        Ok(Value::number(match state {
            State::NotANumber => f64::NAN,
            State::PlusInfinity => f64::INFINITY,
            State::MinusInfinity => f64::NEG_INFINITY,
            State::MinusZero => -0.0,
            State::Finite => sum.round(),
        }))
    })();
    agent.vm.temps.truncate(agent.vm.temps.len() - 2);
    r
}
