//! What `\p{...}` and `\P{...}` match (section 22.2.2.9): the names and
//! characters of `property_tables.rs`, which tools/gen_properties.py
//! generates from the Unicode version the reference browser ships.

use super::charset::CharSet;
use super::property_tables::{BINARY, GENERAL_CATEGORY, SCRIPT, STRINGS};

/// A property's characters, and its strings of more than one character
/// (a property of strings only).
pub(super) struct Property {
    pub set: CharSet,
    pub strings: Vec<Vec<u32>>,
    /// A property of strings (`v` mode only), whatever its strings.
    pub of_strings: bool,
}

fn characters(flat: &[u32]) -> Property {
    Property {
        set: CharSet::from_flat(flat),
        strings: Vec::new(),
        of_strings: false,
    }
}

fn general_category(value: &str) -> Option<Property> {
    let i = GENERAL_CATEGORY
        .binary_search_by_key(&value, |&(n, _)| n)
        .ok()?;
    GENERAL_CATEGORY.get(i).map(|&(_, t)| characters(t))
}

fn script(value: &str, extensions: bool) -> Option<Property> {
    let i = SCRIPT.binary_search_by_key(&value, |&(n, _, _)| n).ok()?;
    SCRIPT
        .get(i)
        .map(|&(_, sc, scx)| characters(if extensions { scx } else { sc }))
}

fn binary(name: &str) -> Option<Property> {
    let i = BINARY.binary_search_by_key(&name, |&(n, _)| n).ok()?;
    BINARY.get(i).map(|&(_, t)| characters(t))
}

fn of_strings(name: &str) -> Option<Property> {
    let i = STRINGS.binary_search_by_key(&name, |&(n, _, _)| n).ok()?;
    STRINGS.get(i).map(|&(_, singles, strings)| Property {
        set: CharSet::from_flat(singles),
        strings: strings.iter().map(|s| s.to_vec()).collect(),
        of_strings: true,
    })
}

/// `\p{name}` or `\p{name=value}`: a General_Category value or a binary
/// property alone, or General_Category, Script or Script_Extensions with
/// a value; a property of strings only where `strings` allows one (`\p`
/// in `v` mode). None for a name or value the tables do not have.
pub(super) fn lookup(name: &str, value: Option<&str>, strings: bool) -> Option<Property> {
    match value {
        Some(v) => match name {
            "General_Category" | "gc" => general_category(v),
            "Script" | "sc" => script(v, false),
            "Script_Extensions" | "scx" => script(v, true),
            _ => None,
        },
        None => general_category(name)
            .or_else(|| binary(name))
            .or_else(|| if strings { of_strings(name) } else { None }),
    }
}
