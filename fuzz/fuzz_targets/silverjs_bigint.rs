//! BigInt arithmetic against an independent model: the bytes make pairs
//! of BigInts (each sign, zero, one limb, many limbs, a power of two and
//! its neighbours) and an operator each, the script computes every
//! result, and each must be what num-bigint (a test-only oracle) says:
//! `+ - * / % **`, the bitwise operators and `~`, shifts both ways,
//! `BigInt.asIntN` and `asUintN` at the edge widths (0, 1, 63, 64, 65 and
//! more), comparisons, `toString(radix)` and `BigInt(string)` in every
//! radix prefix. A division by zero and a negative exponent must be the
//! RangeError. The run must never panic.

#![no_main]

use std::fmt::Write;

use libfuzzer_sys::fuzz_target;
use num_bigint::BigInt;
use num_traits::{Signed, ToPrimitive, Zero};
use silverjs::agent::{Agent, AgentOptions, Outcome};

/// The cases one input may make.
const MAX_CASES: usize = 40;

/// A BigInt from the bytes at `at`: a shape byte, then limbs.
fn operand(bytes: &[u8], at: &mut usize) -> BigInt {
    let mut next = || {
        let b = bytes.get(*at).copied().unwrap_or(0);
        *at += 1;
        b
    };
    let shape = next();
    let negative = shape & 0x80 != 0;
    let magnitude = match shape & 0x0f {
        0 => BigInt::zero(),
        1 => BigInt::from(1),
        2 => BigInt::from(next()),
        // A power of two and its neighbours: 2^k - 1, 2^k, 2^k + 1.
        3..=5 => {
            let k = u32::from(next()) % 200;
            let p = BigInt::from(1) << k;
            match shape & 0x0f {
                3 => p - 1,
                4 => p,
                _ => p + 1,
            }
        }
        n => {
            // Up to 24 limbs of 32 bits.
            let limbs = usize::from(n) * 2 + usize::from(next() % 4);
            let mut v = BigInt::zero();
            for _ in 0..limbs {
                let word = u32::from_le_bytes([next(), next(), next(), next()]);
                v = (v << 32) + word;
            }
            v
        }
    };
    if negative { -magnitude } else { magnitude }
}

/// `v` as a JavaScript BigInt expression.
fn js(v: &BigInt) -> String {
    if v.is_negative() {
        format!("(-0x{}n)", (-v).to_str_radix(16))
    } else {
        format!("0x{}n", v.to_str_radix(16))
    }
}

fn floor_shift_right(a: &BigInt, n: u32) -> BigInt {
    // num-bigint's `>>` on a negative value rounds toward negative
    // infinity, as the spec's does.
    a >> n
}

fn as_uint_n(a: &BigInt, w: u32) -> BigInt {
    let m = BigInt::from(1) << w;
    ((a % &m) + &m) % &m
}

fn as_int_n(a: &BigInt, w: u32) -> BigInt {
    if w == 0 {
        return BigInt::zero();
    }
    let r = as_uint_n(a, w);
    if r >= BigInt::from(1) << (w - 1) {
        r - (BigInt::from(1) << w)
    } else {
        r
    }
}

/// The expression for `op` on `a` and `b`, and what the model says it
/// gives.
fn case(op: u8, a: &BigInt, b: &BigInt, extra: u8) -> (String, String) {
    let (ja, jb) = (js(a), js(b));
    let text = |v: BigInt| v.to_str_radix(10);
    match op % 18 {
        0 => (format!("{ja} + {jb}"), text(a + b)),
        1 => (format!("{ja} - {jb}"), text(a - b)),
        2 => (format!("{ja} * {jb}"), text(a * b)),
        3 => (
            format!("{ja} / {jb}"),
            if b.is_zero() {
                "RangeError".into()
            } else {
                text(a / b)
            },
        ),
        4 => (
            format!("{ja} % {jb}"),
            if b.is_zero() {
                "RangeError".into()
            } else {
                text(a % b)
            },
        ),
        5 => {
            // A small base and exponent: the result stays small.
            let base: BigInt = a % BigInt::from(1000);
            let e = i64::from(extra % 40) - 4;
            let want = if e < 0 {
                "RangeError".into()
            } else {
                text(num_traits::pow(base.clone(), e as usize))
            };
            (format!("{} ** {e}n", js(&base)), want)
        }
        6 => (format!("{ja} & {jb}"), text(a & b)),
        7 => (format!("{ja} | {jb}"), text(a | b)),
        8 => (format!("{ja} ^ {jb}"), text(a ^ b)),
        9 => (format!("~{ja}"), text(-a - 1)),
        10 => {
            let n = i32::from(extra) - 128;
            let want = if n >= 0 {
                a << n as u32
            } else {
                floor_shift_right(a, n.unsigned_abs())
            };
            (format!("{ja} << {n}n"), text(want))
        }
        11 => {
            let n = i32::from(extra) - 128;
            let want = if n >= 0 {
                floor_shift_right(a, n as u32)
            } else {
                a << n.unsigned_abs()
            };
            (format!("{ja} >> {n}n"), text(want))
        }
        12 | 13 => {
            let w = match extra % 8 {
                0 => 0,
                1 => 1,
                2 => 63,
                3 => 64,
                4 => 65,
                _ => u32::from(extra),
            };
            if op % 18 == 12 {
                (format!("BigInt.asIntN({w}, {ja})"), text(as_int_n(a, w)))
            } else {
                (format!("BigInt.asUintN({w}, {ja})"), text(as_uint_n(a, w)))
            }
        }
        14 => (
            format!("[{ja} < {jb}, {ja} == {jb}, {ja} >= {jb}]"),
            format!("{},{},{}", a < b, a == b, a >= b),
        ),
        15 => {
            let radix = u32::from(extra % 35) + 2;
            (format!("{ja}.toString({radix})"), a.to_str_radix(radix))
        }
        16 => {
            // BigInt(string) in each prefix, and decimal with a sign.
            let (s, want) = match extra % 4 {
                0 => (format!("0x{}", a.abs().to_str_radix(16)), a.abs()),
                1 => (format!("0o{}", a.abs().to_str_radix(8)), a.abs()),
                2 => (format!("0b{}", a.abs().to_str_radix(2)), a.abs()),
                _ => (format!(" {} ", a.to_str_radix(10)), a.clone()),
            };
            (format!("BigInt('{s}')"), text(want))
        }
        _ => {
            // Number(a) against the model's nearest double.
            let want = a.to_f64().unwrap_or(f64::NAN);
            let literal = if want.is_infinite() {
                if want > 0.0 { "Infinity" } else { "-Infinity" }.to_owned()
            } else {
                format!("{want:e}")
            };
            (format!("Number({ja}) === {literal}"), "true".into())
        }
    }
}

fuzz_target!(|data: &[u8]| {
    let mut at = 0;
    let mut script = String::from(
        "var r = [];\nfunction t(f) { try { r.push(String(f())); } catch (e) { r.push(e.name); } }\n",
    );
    let mut want = Vec::new();
    while at + 2 < data.len() && want.len() < MAX_CASES {
        let op = data.get(at).copied().unwrap_or(0);
        let extra = data.get(at + 1).copied().unwrap_or(0);
        at += 2;
        let a = operand(data, &mut at);
        let b = operand(data, &mut at);
        let (expr, expected) = case(op, &a, &b, extra);
        let _ = writeln!(script, "t(function () {{ return {expr}; }});");
        want.push(expected);
    }
    script.push_str("r.join('|')");
    let mut agent = Agent::new(AgentOptions::default());
    let got = agent.scope(|scope| match scope.run_script(&script) {
        Ok(Outcome::Value(v)) => scope.to_text(v).unwrap_or_default(),
        Ok(Outcome::Threw(v)) => format!("threw {}", scope.to_text(v).unwrap_or_default()),
        Ok(Outcome::Terminated) => "terminated".to_owned(),
        Err(e) => format!("did not compile: {e}"),
    });
    let got: Vec<&str> = got.split('|').collect();
    for (k, w) in want.iter().enumerate() {
        assert_eq!(
            got.get(k).copied(),
            Some(w.as_str()),
            "case {k}\n--- script ---\n{script}"
        );
    }
});
