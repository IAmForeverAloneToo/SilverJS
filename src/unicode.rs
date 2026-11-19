//! The character classes the lexer asks about (ECMA-262 sections 12.2,
//! 12.3 and 12.7), over the generated tables (`unicode_tables.rs`).

use crate::unicode_tables::{ID_CONTINUE, ID_START, SPACE_SEPARATOR};

/// ZERO WIDTH NON-JOINER and ZERO WIDTH JOINER, which an identifier may
/// hold past its first character.
pub(crate) const ZWNJ: char = '\u{200C}';
pub(crate) const ZWJ: char = '\u{200D}';
/// ZERO WIDTH NO-BREAK SPACE, white space in a source.
pub(crate) const ZWNBSP: char = '\u{FEFF}';
/// LINE SEPARATOR and PARAGRAPH SEPARATOR, line terminators.
pub(crate) const LS: char = '\u{2028}';
pub(crate) const PS: char = '\u{2029}';

/// Whether `c` is in the inclusive ranges of `table`, sorted.
fn in_table(table: &[(u32, u32)], c: char) -> bool {
    let c = u32::from(c);
    let at = table.partition_point(|&(_, hi)| hi < c);
    table.get(at).is_some_and(|&(lo, _)| lo <= c)
}

/// IdentifierStartChar (section 12.7): `ID_Start`, `$` or `_`.
pub(crate) fn is_id_start(c: char) -> bool {
    if c.is_ascii() {
        c.is_ascii_alphabetic() || c == '$' || c == '_'
    } else {
        in_table(ID_START, c)
    }
}

/// IdentifierPartChar (section 12.7): `ID_Continue`, `$`, ZWNJ or ZWJ.
pub(crate) fn is_id_continue(c: char) -> bool {
    if c.is_ascii() {
        c.is_ascii_alphanumeric() || c == '$' || c == '_'
    } else {
        c == ZWNJ || c == ZWJ || in_table(ID_CONTINUE, c)
    }
}

/// WhiteSpace (section 12.2): TAB, VT, FF, ZWNBSP and any `Zs`.
pub(crate) fn is_white_space(c: char) -> bool {
    match c {
        '\t' | '\u{000B}' | '\u{000C}' | ' ' | ZWNBSP => true,
        _ if c.is_ascii() => false,
        _ => in_table(SPACE_SEPARATOR, c),
    }
}

/// LineTerminator (section 12.3): LF, CR, LS and PS.
pub(crate) fn is_line_terminator(c: char) -> bool {
    matches!(c, '\n' | '\r' | LS | PS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_classes_answer_as_the_standard_says() {
        assert!(is_id_start('a') && is_id_start('$') && is_id_start('_'));
        assert!(!is_id_start('1') && is_id_continue('1'));
        // Greek, CJK, and an astral letter (MATHEMATICAL BOLD SCRIPT A).
        assert!(is_id_start('\u{03B1}') && is_id_start('\u{4E00}'));
        assert!(is_id_start('\u{1D4D0}'));
        // A combining mark continues but does not start.
        assert!(!is_id_start('\u{0301}') && is_id_continue('\u{0301}'));
        assert!(is_id_continue(ZWNJ) && !is_id_start(ZWNJ));
        // Emoji are neither.
        assert!(!is_id_start('\u{1F600}') && !is_id_continue('\u{1F600}'));
        assert!(is_white_space('\u{00A0}') && is_white_space('\u{3000}'));
        assert!(is_white_space(ZWNBSP) && !is_white_space('\u{180E}'));
        assert!(is_line_terminator(LS) && !is_white_space(LS));
    }
}
