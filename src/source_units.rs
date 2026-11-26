//! Source text made from a JavaScript string (`eval`'s, `Function`'s),
//! which may hold a lone surrogate where a Rust `str` cannot: in a
//! string literal, a template or a regexp literal. Each lone surrogate
//! is written as a stand-in, a code point of the sixteenth plane's
//! private use area, from a window of 2,048 that no other character of
//! the text falls in; the lexer and the code generator write the
//! surrogate back where the text becomes a value. A text without a lone
//! surrogate has no window and is the string as it is.

/// The size of a window: one stand-in for each surrogate code unit.
const WINDOW: u32 = 0x800;

/// The sixteenth plane, where the windows are.
const PLANE_16: u32 = 0x10_0000;

/// The source text of `units`, and the first code point of its window
/// when a lone surrogate needed a stand-in.
pub(crate) fn text_of(units: &[u16]) -> (String, Option<u32>) {
    let decoded: Vec<Result<char, u16>> = char::decode_utf16(units.iter().copied())
        .map(|r| r.map_err(|e| e.unpaired_surrogate()))
        .collect();
    if decoded.iter().all(Result::is_ok) {
        return (decoded.into_iter().flatten().collect(), None);
    }
    let in_window = |base: u32| {
        decoded
            .iter()
            .any(|r| matches!(r, Ok(c) if (base..base + WINDOW).contains(&u32::from(*c))))
    };
    // The last window the text leaves free; were none (a text with
    // characters in all 32), its lone surrogates become U+FFFD.
    let base = (0..32u32)
        .rev()
        .map(|i| PLANE_16 + i * WINDOW)
        .find(|&b| !in_window(b));
    let text = decoded
        .iter()
        .map(|r| match (r, base) {
            (Ok(c), _) => *c,
            (Err(u), Some(b)) => {
                char::from_u32(b + u32::from(*u) - 0xD800).unwrap_or(char::REPLACEMENT_CHARACTER)
            }
            (Err(_), None) => char::REPLACEMENT_CHARACTER,
        })
        .collect();
    (text, base)
}

/// Writes `c` as UTF-16 into `out`: a stand-in of `window` as the lone
/// surrogate it stands for.
pub(crate) fn push_char(out: &mut Vec<u16>, c: char, window: Option<u32>) {
    let v = u32::from(c);
    if let Some(base) = window
        && (base..base + WINDOW).contains(&v)
    {
        out.push(u16::try_from(0xD800 + v - base).unwrap_or(0xFFFD));
        return;
    }
    let mut buf = [0u16; 2];
    out.extend_from_slice(c.encode_utf16(&mut buf));
}

/// `s` as UTF-16, its stand-ins of `window` written back.
pub(crate) fn units_of(s: &str, window: Option<u32>) -> Vec<u16> {
    if window.is_none() {
        return s.encode_utf16().collect();
    }
    let mut out = Vec::with_capacity(s.len());
    for c in s.chars() {
        push_char(&mut out, c, window);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{push_char, text_of, units_of};

    #[test]
    fn a_text_without_lone_surrogates_is_itself() {
        let units: Vec<u16> = "a\u{FFFD}\u{10FFFF}b".encode_utf16().collect();
        assert_eq!(text_of(&units), ("a\u{FFFD}\u{10FFFF}b".to_owned(), None));
    }

    #[test]
    fn lone_surrogates_take_a_window_the_text_leaves_free() {
        // The last window (U+10F800 up) is in use, so the one before.
        let mut units: Vec<u16> = "\u{10F900}".encode_utf16().collect();
        units.extend([0xD800, u16::from(b'x'), 0xDFFF]);
        let (text, window) = text_of(&units);
        assert_eq!(window, Some(0x10_F000));
        assert_eq!(units_of(&text, window), units);
        let mut out = Vec::new();
        push_char(&mut out, '\u{10F000}', window);
        push_char(&mut out, '\u{10F900}', window);
        assert_eq!(out, [0xD800, 0xDBFE, 0xDD00]);
    }
}
