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

#[cfg(test)]
mod tests {
    use super::number_to_string;

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
