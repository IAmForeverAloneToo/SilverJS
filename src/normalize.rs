//! Unicode normalization (UAX #15) for `String.prototype.normalize`:
//! the four forms over UTF-16, from the tables of
//! `tools/gen_normalize.py` (the UCD version the reference browser
//! ships). A lone surrogate is a code point of class 0 with no mapping,
//! so it stays as it is.

use crate::normalize_tables::{COMBINING_CLASS, COMPOSITION, DECOMPOSITION, DECOMPOSITION_DATA};

/// A normalization form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Form {
    Nfc,
    Nfd,
    Nfkc,
    Nfkd,
}

impl Form {
    /// The form named `name` (`"NFC"`, `"NFD"`, `"NFKC"`, `"NFKD"`).
    pub(crate) fn named(name: &[u16]) -> Option<Form> {
        match String::from_utf16_lossy(name).as_str() {
            "NFC" => Some(Form::Nfc),
            "NFD" => Some(Form::Nfd),
            "NFKC" => Some(Form::Nfkc),
            "NFKD" => Some(Form::Nfkd),
            _ => None,
        }
    }

    fn compatibility(self) -> bool {
        matches!(self, Form::Nfkc | Form::Nfkd)
    }

    fn composes(self) -> bool {
        matches!(self, Form::Nfc | Form::Nfkc)
    }
}

// Hangul syllables (section 3.12 of the Unicode Standard).
const S_BASE: u32 = 0xAC00;
const L_BASE: u32 = 0x1100;
const V_BASE: u32 = 0x1161;
const T_BASE: u32 = 0x11A7;
const L_COUNT: u32 = 19;
const V_COUNT: u32 = 21;
const T_COUNT: u32 = 28;
const N_COUNT: u32 = V_COUNT * T_COUNT;
const S_COUNT: u32 = L_COUNT * N_COUNT;

/// A code point's Canonical_Combining_Class.
fn class(cp: u32) -> u8 {
    COMBINING_CLASS
        .binary_search_by_key(&cp, |&(c, _)| c)
        .ok()
        .and_then(|i| COMBINING_CLASS.get(i))
        .map_or(0, |&(_, k)| k)
}

/// `cp`'s full decomposition (canonical, or with `compat` compatibility
/// too) appended to `out`.
fn decompose(cp: u32, compat: bool, out: &mut Vec<u32>) {
    if (S_BASE..S_BASE + S_COUNT).contains(&cp) {
        let s = cp - S_BASE;
        out.push(L_BASE + s / N_COUNT);
        out.push(V_BASE + (s % N_COUNT) / T_COUNT);
        let t = s % T_COUNT;
        if t != 0 {
            out.push(T_BASE + t);
        }
        return;
    }
    let mapping = DECOMPOSITION
        .binary_search_by_key(&cp, |&(c, ..)| c)
        .ok()
        .and_then(|i| DECOMPOSITION.get(i))
        .filter(|&&(_, _, _, is_compat)| compat || !is_compat)
        .and_then(|&(_, start, len, _)| {
            DECOMPOSITION_DATA.get(usize::from(start)..usize::from(start) + usize::from(len))
        });
    match mapping {
        Some(m) => {
            for &c in m {
                decompose(c, compat, out);
            }
        }
        None => out.push(cp),
    }
}

/// The canonical ordering algorithm: each run of non-starters sorted by
/// class, stably.
fn reorder(cps: &mut [u32]) {
    let mut i = 0;
    while i < cps.len() {
        if cps.get(i).is_some_and(|&c| class(c) != 0) {
            let start = i;
            while cps.get(i).is_some_and(|&c| class(c) != 0) {
                i += 1;
            }
            if let Some(run) = cps.get_mut(start..i) {
                run.sort_by_key(|&c| class(c));
            }
        } else {
            i += 1;
        }
    }
}

/// The primary composite of `a` and `b`, if they compose.
fn composite(a: u32, b: u32) -> Option<u32> {
    if (L_BASE..L_BASE + L_COUNT).contains(&a) && (V_BASE..V_BASE + V_COUNT).contains(&b) {
        return Some(S_BASE + ((a - L_BASE) * V_COUNT + (b - V_BASE)) * T_COUNT);
    }
    if (S_BASE..S_BASE + S_COUNT).contains(&a)
        && (a - S_BASE) % T_COUNT == 0
        && (T_BASE + 1..T_BASE + T_COUNT).contains(&b)
    {
        return Some(a + (b - T_BASE));
    }
    COMPOSITION
        .binary_search_by(|&(x, y, _)| (x, y).cmp(&(a, b)))
        .ok()
        .and_then(|i| COMPOSITION.get(i))
        .map(|&(_, _, c)| c)
}

/// The canonical composition algorithm, in place.
fn compose(cps: &mut Vec<u32>) {
    let mut out: Vec<u32> = Vec::with_capacity(cps.len());
    // The last starter in `out`, and the class of what was last added.
    let mut starter: Option<usize> = None;
    let mut last_class: u8 = 0;
    for &c in cps.iter() {
        let k = class(c);
        if let Some(si) = starter
            && let Some(&s) = out.get(si)
        {
            // Not blocked: nothing between, or what is between has a
            // lower class.
            let adjacent = si + 1 == out.len();
            if (adjacent || (last_class != 0 && last_class < k))
                && let Some(p) = composite(s, c)
            {
                if let Some(slot) = out.get_mut(si) {
                    *slot = p;
                }
                continue;
            }
        }
        if k == 0 {
            starter = Some(out.len());
        }
        last_class = k;
        out.push(c);
    }
    *cps = out;
}

/// `units` normalized to `form`.
pub(crate) fn normalize(units: &[u16], form: Form) -> Vec<u16> {
    // ASCII is in every form already.
    if units.iter().all(|&u| u < 0x80) {
        return units.to_vec();
    }
    let mut cps = Vec::with_capacity(units.len());
    for r in char::decode_utf16(units.iter().copied()) {
        let cp = match r {
            Ok(c) => u32::from(c),
            Err(e) => u32::from(e.unpaired_surrogate()),
        };
        decompose(cp, form.compatibility(), &mut cps);
    }
    reorder(&mut cps);
    if form.composes() {
        compose(&mut cps);
    }
    let mut out = Vec::with_capacity(cps.len());
    for cp in cps {
        match char::from_u32(cp) {
            Some(c) => {
                let mut buf = [0u16; 2];
                out.extend_from_slice(c.encode_utf16(&mut buf));
            }
            // A lone surrogate, as it came.
            None => out.push(u16::try_from(cp).unwrap_or(0xFFFD)),
        }
    }
    out
}
