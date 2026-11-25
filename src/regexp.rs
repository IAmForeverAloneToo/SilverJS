//! Regular expressions (design section 1.6): a pattern and its flags
//! checked (sections 13.2.7.2 and 22.2.1, with Annex B.1.2's grammar for
//! patterns without `u` or `v`) and parsed to a tree ([`parse`]), the
//! sets its classes stand for ([`charset`]), and the names `\p{...}`
//! accepts ([`properties`]).

use crate::stack::StackGuard;

pub(crate) mod casefold;
mod casefold_tables;
pub(crate) mod charset;
pub(crate) mod parse;
mod properties;
mod property_tables;
pub(crate) mod vm;

/// The early errors of a RegularExpressionLiteral.
pub(crate) fn check_literal(body: &str, flags: &str, stack: StackGuard) -> Result<(), String> {
    check_flags(flags).map_err(|()| "Invalid regular expression flags".to_owned())?;
    let unicode = flags.contains('u') || flags.contains('v');
    let sets = flags.contains('v');
    check_pattern(body, unicode, sets, stack)
        .map_err(|m| format!("Invalid regular expression: /{body}/{flags}: {m}"))
}

/// The flags' rules: each one of `dgimsuyv`, none twice, not both `u`
/// and `v`.
pub(crate) fn check_flags(flags: &str) -> Result<(), ()> {
    let mut seen = 0u32;
    for c in flags.chars() {
        let bit = match c {
            'd' => 1,
            'g' => 2,
            'i' => 4,
            'm' => 8,
            's' => 16,
            'u' => 32,
            'y' => 64,
            'v' => 128,
            _ => return Err(()),
        };
        if seen & bit != 0 {
            return Err(());
        }
        seen |= bit;
    }
    if seen & 32 != 0 && seen & 128 != 0 {
        return Err(());
    }
    Ok(())
}

/// Checks a pattern: `unicode` for the `u` or `v` flag, `sets` for `v`.
pub(crate) fn check_pattern(
    pattern: &str,
    unicode: bool,
    sets: bool,
    stack: StackGuard,
) -> Result<(), String> {
    parse::parse(pattern, unicode, sets, false, stack).map(|_| ())
}
