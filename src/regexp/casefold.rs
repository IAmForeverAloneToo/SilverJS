//! Canonicalize (section 22.2.2.7.3) for the `i` flag: with `u` or `v`,
//! simple case folding (the table of `tools/gen_casefold.py`); without,
//! the uppercase of a code unit when it is one code unit and does not
//! turn a non-ASCII character into an ASCII one. And each character's
//! equivalents, the characters that canonicalize as it does, for classes.

use std::collections::HashMap;
use std::sync::OnceLock;

use super::casefold_tables::SIMPLE_FOLD;
use super::charset::CharSet;

/// `c` canonicalized: `unicode` for the `u` or `v` flag.
pub(crate) fn canonicalize(c: u32, unicode: bool) -> u32 {
    if unicode {
        return SIMPLE_FOLD
            .binary_search_by_key(&c, |&(a, _)| a)
            .ok()
            .and_then(|i| SIMPLE_FOLD.get(i))
            .map_or(c, |&(_, b)| b);
    }
    legacy_table().get(c as usize).map_or(c, |&u| u32::from(u))
}

/// The non-Unicode canonical form of every code unit, made once.
fn legacy_table() -> &'static [u16] {
    static TABLE: OnceLock<Vec<u16>> = OnceLock::new();
    TABLE.get_or_init(|| {
        (0..=0xFFFFu32)
            .map(|cu| {
                let Some(c) = char::from_u32(cu) else {
                    // A surrogate code unit is its own.
                    return cu as u16;
                };
                let mut up = c.to_uppercase();
                match (up.next(), up.next()) {
                    (Some(u), None) => {
                        let u = u32::from(u);
                        if u > 0xFFFF || (cu >= 128 && u < 128) {
                            cu as u16
                        } else {
                            u as u16
                        }
                    }
                    _ => cu as u16,
                }
            })
            .collect()
    })
}

/// Each canonical value's characters, for a mode, made once.
fn classes(unicode: bool) -> &'static HashMap<u32, Vec<u32>> {
    static UNICODE: OnceLock<HashMap<u32, Vec<u32>>> = OnceLock::new();
    static LEGACY: OnceLock<HashMap<u32, Vec<u32>>> = OnceLock::new();
    if unicode {
        UNICODE.get_or_init(|| {
            let mut m: HashMap<u32, Vec<u32>> = HashMap::new();
            for &(a, b) in SIMPLE_FOLD {
                let e = m.entry(b).or_default();
                if e.is_empty() {
                    e.push(b);
                }
                e.push(a);
            }
            m
        })
    } else {
        LEGACY.get_or_init(|| {
            let mut m: HashMap<u32, Vec<u32>> = HashMap::new();
            for (cu, &canon) in legacy_table().iter().enumerate() {
                let canon = u32::from(canon);
                if canon != cu as u32 {
                    let e = m.entry(canon).or_default();
                    if e.is_empty() {
                        e.push(canon);
                    }
                    e.push(cu as u32);
                }
            }
            m
        })
    }
}

/// The characters simple case folding changes (CaseFolding.txt's
/// statuses C and S), made once.
fn folding() -> &'static CharSet {
    static SET: OnceLock<CharSet> = OnceLock::new();
    SET.get_or_init(|| {
        let mut s = CharSet::new();
        for &(a, _) in SIMPLE_FOLD {
            s.add(a);
        }
        s
    })
}

/// `set` simple-case-folded (MaybeSimpleCaseFolding, section 22.2.2.9.3,
/// with the `i` flag in `v` mode): each character made its fold.
pub(crate) fn simple_fold_set(set: &CharSet) -> CharSet {
    let mut out = set.subtract(folding());
    for &(a, b) in SIMPLE_FOLD {
        if set.contains(a) {
            out.add(b);
        }
    }
    out
}

/// AllCharacters for the `i` flag in `v` mode (section 22.2.2.9.5): the
/// characters simple case folding keeps as they are.
pub(crate) fn unfolded_characters() -> CharSet {
    folding().complement(super::parse::MAX_CODE_POINT)
}

/// Whether any character that canonicalizes as `c` does passes `f`.
pub(crate) fn any_equivalent(c: u32, unicode: bool, f: impl Fn(u32) -> bool) -> bool {
    let canon = canonicalize(c, unicode);
    match classes(unicode).get(&canon) {
        Some(v) => v.iter().any(|&e| f(e)),
        None => f(c) || f(canon),
    }
}

/// The characters that canonicalize as `c` does, `c` among them.
#[cfg(test)]
pub(crate) fn equivalents(c: u32, unicode: bool) -> Vec<u32> {
    let canon = canonicalize(c, unicode);
    match classes(unicode).get(&canon) {
        Some(v) => v.clone(),
        None if canon == c => vec![c],
        None => vec![c, canon],
    }
}

#[cfg(test)]
mod tests {
    use super::{canonicalize, equivalents};

    #[test]
    fn both_modes_canonicalize_as_the_spec_says() {
        // Without `u`: uppercase, one unit, no non-ASCII to ASCII.
        assert_eq!(canonicalize(u32::from('a'), false), u32::from('A'));
        assert_eq!(canonicalize(0x17F, false), 0x17F); // long s: S is ASCII
        assert_eq!(canonicalize(0xDF, false), 0xDF); // sharp s: SS is two
        // With `u`: simple case folding.
        assert_eq!(canonicalize(u32::from('A'), true), u32::from('a'));
        assert_eq!(canonicalize(0x17F, true), u32::from('s'));
        assert_eq!(canonicalize(0x212A, true), u32::from('k')); // Kelvin
        let mut k = equivalents(u32::from('k'), true);
        k.sort_unstable();
        assert_eq!(k, vec![u32::from('K'), u32::from('k'), 0x212A]);
        let mut s = equivalents(u32::from('s'), false);
        s.sort_unstable();
        assert_eq!(s, vec![u32::from('S'), u32::from('s')]);
    }
}
