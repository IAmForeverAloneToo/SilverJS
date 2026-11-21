//! Numbers' text (design note, section 1.3): ECMA-262's
//! Number::toString(x, 10) (section 6.1.6.1.20), on the shortest
//! round-tripping digits Rust's `core` formats `f64` with.

/// The decimal digits and exponent of a finite, positive `x`: `x` is
/// `0.d1d2...dk * 10^n`, `digits` the k digits, as the spec's `s` and
/// `n`.
fn shortest(x: f64) -> (Vec<u8>, i32) {
    // `{:e}` prints the shortest digits that round-trip, `d.ddde-7`.
    let text = format!("{x:e}");
    let (mantissa, exponent) = text.split_once('e').unwrap_or((&text, "0"));
    let digits: Vec<u8> = mantissa.bytes().filter(u8::is_ascii_digit).collect();
    let exponent: i32 = exponent.parse().unwrap_or(0);
    (digits, exponent + 1)
}

/// Number::toString(x) in radix 10.
pub fn number_to_string(x: f64) -> String {
    if x.is_nan() {
        return "NaN".into();
    }
    if x == 0.0 {
        return "0".into();
    }
    if x < 0.0 {
        return format!("-{}", number_to_string(-x));
    }
    if x.is_infinite() {
        return "Infinity".into();
    }
    let (digits, n) = shortest(x);
    let k = i32::try_from(digits.len()).unwrap_or(i32::MAX);
    let text = |d: &[u8]| String::from_utf8_lossy(d).into_owned();
    let (head, tail) = digits.split_at(digits.len().min(n.max(0) as usize));
    if k <= n && n <= 21 {
        // Digits, then n - k zeros.
        let mut s = text(&digits);
        s.extend(std::iter::repeat_n('0', (n - k) as usize));
        s
    } else if 0 < n && n <= 21 {
        format!("{}.{}", text(head), text(tail))
    } else if -6 < n && n <= 0 {
        let mut s = String::from("0.");
        s.extend(std::iter::repeat_n('0', (-n) as usize));
        s.push_str(&text(&digits));
        s
    } else {
        let e = n - 1;
        let sign = if e < 0 { '-' } else { '+' };
        let (first, rest) = digits.split_at(1);
        if rest.is_empty() {
            format!("{}e{sign}{}", text(first), e.abs())
        } else {
            format!("{}.{}e{sign}{}", text(first), text(rest), e.abs())
        }
    }
}

/// The exact decimal digits of a finite, positive `x`, as [`shortest`]
/// gives its shortest: `x` is exactly `0.d1d2...dk * 10^n`. A double's
/// expansion ends within 1,074 places after the point, which Rust's
/// fixed formatting prints exactly.
fn exact(x: f64) -> (Vec<u8>, i32) {
    let text = format!("{x:.1100}");
    let (int, frac) = text.split_once('.').unwrap_or((&text, ""));
    let int = int.trim_start_matches('0');
    let mut digits: Vec<u8> = int.bytes().chain(frac.bytes()).collect();
    let mut n = i32::try_from(int.len()).unwrap_or(0);
    let lead = digits
        .iter()
        .position(|&d| d != b'0')
        .unwrap_or(digits.len());
    digits.drain(..lead);
    n -= i32::try_from(lead).unwrap_or(0);
    while digits.last() == Some(&b'0') {
        digits.pop();
    }
    (digits, n)
}

/// `digits` (of `0.d1d2... * 10^n`) rounded to the first `keep`, a tie
/// rounded up, as `toFixed` and its kin pick "the larger n": the digits,
/// padded with zeros to `keep`, and the exponent (one more on a carry).
fn round_half_up(digits: &[u8], n: i32, keep: i32) -> (Vec<u8>, i32) {
    if keep < 0 {
        return (Vec::new(), n);
    }
    let k = keep as usize;
    let mut out: Vec<u8> = digits.iter().take(k).copied().collect();
    let up = digits.get(k).is_some_and(|&d| d >= b'5');
    out.resize(k, b'0');
    if up {
        let mut i = k;
        loop {
            if i == 0 {
                out.insert(0, b'1');
                out.truncate(k.max(1));
                return (out, n + 1);
            }
            i -= 1;
            match out.get_mut(i) {
                Some(d) if *d == b'9' => *d = b'0',
                Some(d) => {
                    *d += 1;
                    break;
                }
                None => break,
            }
        }
    }
    (out, n)
}

fn text(d: &[u8]) -> String {
    String::from_utf8_lossy(d).into_owned()
}

/// Number.prototype.toFixed's text (section 21.1.3.3) of a finite `x`
/// below 1e21 in magnitude, `f` places in `0..=100`.
pub fn to_fixed(x: f64, f: usize) -> String {
    let (sign, x) = if x < 0.0 { ("-", -x) } else { ("", x) };
    let fi = i32::try_from(f).unwrap_or(0);
    let (digits, n) = if x == 0.0 { (Vec::new(), 1) } else { exact(x) };
    let (mut d, n) = round_half_up(&digits, n, n + fi);
    // Digits of the integer n: the rounded ones, with zeros before for a
    // value below one.
    let int_len = n.max(0) as usize;
    if n < 0 {
        let pad = (-n) as usize;
        let mut z = vec![b'0'; pad];
        z.append(&mut d);
        d = z;
    }
    d.resize(int_len + f, b'0');
    let (int, frac) = d.split_at(int_len.min(d.len()));
    let int = if int.is_empty() {
        "0".into()
    } else {
        text(int)
    };
    if f == 0 {
        format!("{sign}{int}")
    } else {
        format!("{sign}{int}.{}", text(frac))
    }
}

/// Number.prototype.toExponential's text (section 21.1.3.2) of a finite
/// `x`: `f` places after the point, or as many as it takes (`None`).
pub fn to_exponential(x: f64, f: Option<usize>) -> String {
    let (sign, x) = if x < 0.0 { ("-", -x) } else { ("", x) };
    let (d, e) = if x == 0.0 {
        (vec![b'0'; f.unwrap_or(0) + 1], 0)
    } else {
        match f {
            None => {
                let (d, n) = shortest(x);
                (d, n - 1)
            }
            Some(f) => {
                let (digits, n) = exact(x);
                let (d, n) = round_half_up(&digits, n, i32::try_from(f).unwrap_or(0) + 1);
                (d, n - 1)
            }
        }
    };
    let (first, rest) = d.split_at(1.min(d.len()));
    let es = if e < 0 { '-' } else { '+' };
    if rest.is_empty() {
        format!("{sign}{}e{es}{}", text(first), e.abs())
    } else {
        format!("{sign}{}.{}e{es}{}", text(first), text(rest), e.abs())
    }
}

/// Number.prototype.toPrecision's text (section 21.1.3.5) of a finite
/// `x`, `p` significant digits in `1..=100`.
pub fn to_precision(x: f64, p: usize) -> String {
    let (sign, x) = if x < 0.0 { ("-", -x) } else { ("", x) };
    let pi = i32::try_from(p).unwrap_or(1);
    let (d, e) = if x == 0.0 {
        (vec![b'0'; p], 0)
    } else {
        let (digits, n) = exact(x);
        let (d, n) = round_half_up(&digits, n, pi);
        (d, n - 1)
    };
    if e < -6 || e >= pi {
        let (first, rest) = d.split_at(1.min(d.len()));
        let es = if e < 0 { '-' } else { '+' };
        return if rest.is_empty() {
            format!("{sign}{}e{es}{}", text(first), e.abs())
        } else {
            format!("{sign}{}.{}e{es}{}", text(first), text(rest), e.abs())
        };
    }
    if e == pi - 1 {
        return format!("{sign}{}", text(&d));
    }
    if e >= 0 {
        let (a, b) = d.split_at((e + 1) as usize);
        return format!("{sign}{}.{}", text(a), text(b));
    }
    let zeros = "0".repeat((-(e + 1)) as usize);
    format!("{sign}0.{zeros}{}", text(&d))
}

/// Number::toString(x, radix) for a radix other than 10: the integer
/// part exactly, the fraction to as many digits as tell `x` from its
/// neighbours.
pub fn to_radix(x: f64, radix: u32) -> String {
    if x.is_nan() {
        return "NaN".into();
    }
    if x == 0.0 {
        return "0".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    let (sign, x) = if x < 0.0 { ("-", -x) } else { ("", x) };
    let digit = |d: u32| char::from_digit(d, radix).unwrap_or('0');
    let int = x.trunc();
    let mut frac = x - int;
    // The integer part: an exact big integer (the double's mantissa,
    // shifted), divided down by the radix.
    let bits = int.to_bits();
    let exp = ((bits >> 52) & 0x7ff) as i32;
    let mut int_digits = Vec::new();
    if int >= 1.0 {
        let mantissa = (bits & ((1u64 << 52) - 1)) | (1u64 << 52);
        let shift = exp - 1075;
        let mut big: Vec<u32> = vec![mantissa as u32, (mantissa >> 32) as u32];
        if shift > 0 {
            let words = (shift / 32) as usize;
            let rem = shift % 32;
            let mut shifted = vec![0u32; words];
            let mut carry = 0u32;
            for w in big {
                let v = (u64::from(w) << rem) | u64::from(carry);
                shifted.push(v as u32);
                carry = if rem == 0 {
                    0
                } else {
                    (u64::from(w) >> (32 - rem)) as u32
                };
            }
            shifted.push(carry);
            big = shifted;
        } else if shift < 0 {
            let m = mantissa >> (-shift);
            big = vec![m as u32, (m >> 32) as u32];
        }
        while big.iter().any(|&w| w != 0) {
            let mut rem = 0u64;
            for w in big.iter_mut().rev() {
                let cur = (rem << 32) | u64::from(*w);
                *w = (cur / u64::from(radix)) as u32;
                rem = cur % u64::from(radix);
            }
            int_digits.push(digit(rem as u32));
            while big.last() == Some(&0) {
                big.pop();
            }
        }
        int_digits.reverse();
    } else {
        int_digits.push('0');
    }
    let mut out: String = sign.into();
    out.extend(int_digits);
    // The fraction: digits until what is left is within half the gap to
    // the next double, rounding the last digit.
    let mut delta = 0.5 * (f64::from_bits(x.to_bits() + 1) - x);
    delta = delta.max(f64::from_bits(1));
    if frac >= delta {
        let mut fd: Vec<u32> = Vec::new();
        loop {
            frac *= f64::from(radix);
            delta *= f64::from(radix);
            let d = frac.floor();
            fd.push(d as u32);
            frac -= d;
            if (frac > 0.5 || (frac == 0.5 && d as u32 & 1 == 1)) && frac + delta > 1.0 {
                // Round up, carrying.
                while let Some(last) = fd.last_mut() {
                    *last += 1;
                    if *last < radix {
                        break;
                    }
                    fd.pop();
                }
                break;
            }
            if frac < delta {
                break;
            }
        }
        if !fd.is_empty() {
            out.push('.');
            out.extend(fd.into_iter().map(digit));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{number_to_string, to_exponential, to_fixed, to_precision, to_radix};

    #[test]
    fn fixed_exponential_and_precision_round_ties_up() {
        assert_eq!(to_fixed(0.5, 0), "1");
        assert_eq!(to_fixed(2.5, 0), "3");
        assert_eq!(to_fixed(1.25, 1), "1.3");
        assert_eq!(to_fixed(1.005, 2), "1.00");
        assert_eq!(to_fixed(0.0, 2), "0.00");
        assert_eq!(to_fixed(-1.5, 0), "-2");
        assert_eq!(to_fixed(123.456, 1), "123.5");
        assert_eq!(to_fixed(0.000001, 3), "0.000");
        assert_eq!(to_fixed(0.0006, 3), "0.001");
        assert_eq!(to_fixed(999.99, 1), "1000.0");
        assert_eq!(to_exponential(123456.0, Some(2)), "1.23e+5");
        assert_eq!(to_exponential(0.00015, None), "1.5e-4");
        assert_eq!(to_exponential(0.0, Some(2)), "0.00e+0");
        assert_eq!(to_exponential(9.99, Some(1)), "1.0e+1");
        assert_eq!(to_precision(123.456, 4), "123.5");
        assert_eq!(to_precision(0.000123, 2), "0.00012");
        assert_eq!(to_precision(123456.0, 2), "1.2e+5");
        assert_eq!(to_precision(1e-7, 1), "1e-7");
        assert_eq!(to_precision(0.0, 3), "0.00");
        assert_eq!(to_precision(25.0, 1), "3e+1");
    }

    #[test]
    fn radix_text_is_exact_for_integers_and_short_for_fractions() {
        assert_eq!(to_radix(255.0, 16), "ff");
        assert_eq!(to_radix(-255.0, 2), "-11111111");
        assert_eq!(to_radix(0.5, 2), "0.1");
        assert_eq!(to_radix(2f64.powi(60), 16), "1000000000000000");
        assert!(to_radix(0.1, 3).starts_with("0.00220022002200220022"));
        assert_eq!(to_radix(35.0, 36), "z");
    }

    #[test]
    fn numbers_print_as_the_spec_says() {
        let cases: &[(f64, &str)] = &[
            (0.0, "0"),
            (-0.0, "0"),
            (1.0, "1"),
            (-1.5, "-1.5"),
            (123.0, "123"),
            (0.1, "0.1"),
            (0.000001, "0.000001"),
            (0.0000001, "1e-7"),
            (1.5e-7, "1.5e-7"),
            (1e21, "1e+21"),
            (1e20, "100000000000000000000"),
            (123456789012345680000.0, "123456789012345680000"),
            (1.7976931348623157e308, "1.7976931348623157e+308"),
            (5e-324, "5e-324"),
            (f64::INFINITY, "Infinity"),
            (f64::NEG_INFINITY, "-Infinity"),
            (f64::NAN, "NaN"),
            (4294967295.0, "4294967295"),
            (0.5, "0.5"),
            (100.25, "100.25"),
        ];
        for (x, want) in cases {
            assert_eq!(number_to_string(*x), *want, "{x:?}");
        }
    }
}
