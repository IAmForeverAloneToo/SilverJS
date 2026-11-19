//! Regular expressions: for now the early errors of a literal's flags
//! (section 13.2.7.2). The pattern's own grammar (section 22.2.1) comes
//! in a later step of J1, and the matcher with the built-ins (J4).

use crate::stack::StackGuard;

/// The early errors of a RegularExpressionLiteral that are checked so
/// far: its flags.
pub(crate) fn check_literal(body: &str, flags: &str, stack: StackGuard) -> Result<(), String> {
    let _ = (body, stack);
    check_flags(flags).map_err(|()| "Invalid regular expression flags".to_owned())
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
