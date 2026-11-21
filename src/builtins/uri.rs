//! The URI functions (ECMA-262 section 19.2.6) and Annex B's `escape`
//! and `unescape`.

use super::Call;
use crate::agent::Agent;
use crate::interp::{ErrorType, Res};
use crate::value::Value;

const URI_RESERVED: &str = ";/?:@&=+$,";
const URI_MARK: &str = "-_.!~*'()";

fn is_alnum(u: u16) -> bool {
    u < 0x80 && (u as u8).is_ascii_alphanumeric()
}

fn in_set(u: u16, set: &str) -> bool {
    u < 0x80 && set.as_bytes().contains(&(u as u8))
}

fn malformed(agent: &mut Agent) -> crate::interp::Abrupt {
    agent.throw_error(ErrorType::UriError, "URI malformed")
}

const HEX: &[u8; 16] = b"0123456789ABCDEF";

fn hex_digit(d: u16) -> u16 {
    HEX.get(usize::from(d & 15)).map_or(0, |&h| u16::from(h))
}

fn push_byte(out: &mut Vec<u16>, b: u8) {
    out.push(u16::from(b'%'));
    out.push(hex_digit(u16::from(b >> 4)));
    out.push(hex_digit(u16::from(b)));
}

/// Encode (section 19.2.6.5): the units outside `extra`, alphanumerics
/// and the marks as `%XX` of their UTF-8.
fn encode(agent: &mut Agent, c: Call, extra: &str) -> Res<Value> {
    let s = agent.text_units(agent.arg(c, 0))?;
    let mut out = Vec::with_capacity(s.len());
    let mut k = 0;
    while k < s.len() {
        let u = s.get(k).copied().unwrap_or(0);
        if is_alnum(u) || in_set(u, URI_MARK) || in_set(u, extra) {
            out.push(u);
            k += 1;
            continue;
        }
        let (cp, len) = super::string::code_point(&s, k);
        if (0xd800..0xe000).contains(&cp) {
            return Err(malformed(agent));
        }
        let Some(ch) = char::from_u32(cp) else {
            return Err(malformed(agent));
        };
        let mut buf = [0u8; 4];
        for &b in ch.encode_utf8(&mut buf).as_bytes() {
            push_byte(&mut out, b);
        }
        k += len;
    }
    agent.new_string_units(&out)
}

fn hex(u: u16) -> Option<u8> {
    char::from_u32(u32::from(u))?.to_digit(16).map(|d| d as u8)
}

/// The byte `%XX` at `k`, if it is one.
fn byte_at(s: &[u16], k: usize) -> Option<u8> {
    if s.get(k) != Some(&u16::from(b'%')) {
        return None;
    }
    Some(hex(*s.get(k + 1)?)? << 4 | hex(*s.get(k + 2)?)?)
}

/// Decode (section 19.2.6.6): `%XX` sequences of UTF-8, those of one
/// byte in `reserved` kept as written.
fn decode(agent: &mut Agent, c: Call, reserved: &str) -> Res<Value> {
    let s = agent.text_units(agent.arg(c, 0))?;
    let mut out = Vec::with_capacity(s.len());
    let mut k = 0;
    while k < s.len() {
        let u = s.get(k).copied().unwrap_or(0);
        if u != u16::from(b'%') {
            out.push(u);
            k += 1;
            continue;
        }
        let Some(b) = byte_at(&s, k) else {
            return Err(malformed(agent));
        };
        if b < 0x80 {
            if in_set(u16::from(b), reserved) {
                out.extend_from_slice(s.get(k..k + 3).unwrap_or(&[]));
            } else {
                out.push(u16::from(b));
            }
            k += 3;
            continue;
        }
        let n = b.leading_ones() as usize;
        if !(2..=4).contains(&n) {
            return Err(malformed(agent));
        }
        let mut bytes = vec![b];
        for j in 1..n {
            match byte_at(&s, k + 3 * j) {
                Some(x) if x & 0xc0 == 0x80 => bytes.push(x),
                _ => return Err(malformed(agent)),
            }
        }
        let Ok(text) = std::str::from_utf8(&bytes) else {
            return Err(malformed(agent));
        };
        out.extend(text.encode_utf16());
        k += 3 * n;
    }
    agent.new_string_units(&out)
}

pub(crate) fn decode_uri(agent: &mut Agent, c: Call) -> Res<Value> {
    decode(agent, c, ";/?:@&=+$,#")
}

pub(crate) fn decode_uri_component(agent: &mut Agent, c: Call) -> Res<Value> {
    decode(agent, c, "")
}

pub(crate) fn encode_uri(agent: &mut Agent, c: Call) -> Res<Value> {
    let extra = format!("{URI_RESERVED}#");
    encode(agent, c, &extra)
}

pub(crate) fn encode_uri_component(agent: &mut Agent, c: Call) -> Res<Value> {
    encode(agent, c, "")
}

/// `escape` (section B.2.1.1).
pub(crate) fn escape(agent: &mut Agent, c: Call) -> Res<Value> {
    let s = agent.text_units(agent.arg(c, 0))?;
    let mut out = Vec::with_capacity(s.len());
    for u in s {
        if is_alnum(u) || in_set(u, "@*_+-./") {
            out.push(u);
        } else if u < 256 {
            push_byte(&mut out, u as u8);
        } else {
            out.extend("%u".encode_utf16());
            for shift in [12, 8, 4, 0] {
                out.push(hex_digit(u >> shift));
            }
        }
    }
    agent.new_string_units(&out)
}

/// `unescape` (section B.2.1.2).
pub(crate) fn unescape(agent: &mut Agent, c: Call) -> Res<Value> {
    let s = agent.text_units(agent.arg(c, 0))?;
    let mut out = Vec::with_capacity(s.len());
    let mut k = 0;
    while k < s.len() {
        let u = s.get(k).copied().unwrap_or(0);
        if u == u16::from(b'%') {
            if s.get(k + 1) == Some(&u16::from(b'u')) {
                let digits: Option<Vec<u8>> =
                    (2..6).map(|j| s.get(k + j).and_then(|&d| hex(d))).collect();
                if let Some(d) = digits {
                    let v = d.iter().fold(0u16, |acc, &x| acc << 4 | u16::from(x));
                    out.push(v);
                    k += 6;
                    continue;
                }
            } else if let Some(b) = byte_at(&s, k) {
                out.push(u16::from(b));
                k += 3;
                continue;
            }
        }
        out.push(u);
        k += 1;
    }
    agent.new_string_units(&out)
}
