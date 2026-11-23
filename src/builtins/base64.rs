//! `Uint8Array`'s base64 and hex (ECMA-262 sections 23.3): `fromBase64`
//! and `fromHex`, `toBase64`, `toHex`, and the `setFrom` methods that
//! decode into an array. Decoding runs no script, so an array checked in
//! bounds before it stays so.

use super::{Call, Entry, Name, method};
use crate::agent::Agent;
use crate::buffer::Elem;
use crate::interp::Res;
use crate::value::Value;

/// `Uint8Array`'s own.
pub(crate) const UINT8_ARRAY: &[Entry] = &[
    method("fromBase64", 1, from_base64),
    method("fromHex", 1, from_hex),
];

/// `Uint8Array.prototype`'s own.
pub(crate) const UINT8_ARRAY_PROTO: &[Entry] = &[
    method("toBase64", 0, to_base64),
    method("setFromBase64", 1, set_from_base64),
    method("toHex", 0, to_hex),
    method("setFromHex", 1, set_from_hex),
];

const BAD_CHARACTER: &str = "Found a character that cannot be part of a valid base64 string.";
const SINGLE_CHARACTER: &str =
    "The base64 input terminates with a single character, excluding padding (=).";
const EXTRA_BITS: &str = "The base64 input terminates with non-zero padding bits.";
const BAD_HEX: &str = "Input string must contain hex characters in even length";

const STANDARD: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// What to do with a last chunk of fewer than four characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LastChunk {
    Loose,
    Strict,
    StopBeforePartial,
}

/// What a decoding read: the code units consumed, the bytes, and the
/// SyntaxError's message if it stopped at one.
struct Decoded {
    read: usize,
    bytes: Vec<u8>,
    error: Option<&'static str>,
}

fn is_ascii_space(u: u16) -> bool {
    matches!(u, 0x09 | 0x0a | 0x0c | 0x0d | 0x20)
}

fn skip_space(s: &[u16], mut i: usize) -> usize {
    while s.get(i).is_some_and(|&u| is_ascii_space(u)) {
        i += 1;
    }
    i
}

/// The six bits a standard-alphabet character stands for.
fn sextet(u: u16) -> Option<u8> {
    let b = u8::try_from(u).ok()?;
    STANDARD.iter().position(|&c| c == b).map(|p| p as u8)
}

/// DecodeBase64Chunk of a whole chunk.
fn decode_chunk(c: &[u8]) -> [u8; 3] {
    let at = |i: usize| u32::from(c.get(i).copied().unwrap_or(0));
    let n = (at(0) << 18) | (at(1) << 12) | (at(2) << 6) | at(3);
    [(n >> 16) as u8, (n >> 8) as u8, n as u8]
}

/// DecodeFinalBase64Chunk: two or three characters' bytes, `None` when
/// `strict` and the bits past them are not zero.
fn decode_final(c: &[u8], strict: bool) -> Option<Vec<u8>> {
    let whole = decode_chunk(c);
    let keep = c.len() - 1;
    if strict && whole.get(keep).is_some_and(|&b| b != 0) {
        return None;
    }
    Some(whole.get(..keep).unwrap_or_default().to_vec())
}

/// FromBase64 (section 23.3.2.2's): at most `max` bytes of `s`.
fn decode_base64(s: &[u16], url: bool, last: LastChunk, max: usize) -> Decoded {
    let mut out = Decoded {
        read: 0,
        bytes: Vec::new(),
        error: None,
    };
    if max == 0 {
        return out;
    }
    let mut chunk: Vec<u8> = Vec::with_capacity(4);
    let mut index = 0;
    let len = s.len();
    let fail = |mut out: Decoded, e: &'static str| {
        out.error = Some(e);
        out
    };
    loop {
        index = skip_space(s, index);
        let Some(&u) = s.get(index) else {
            if !chunk.is_empty() {
                match last {
                    LastChunk::StopBeforePartial => return out,
                    LastChunk::Strict => return fail(out, SINGLE_CHARACTER),
                    LastChunk::Loose if chunk.len() == 1 => return fail(out, SINGLE_CHARACTER),
                    LastChunk::Loose => {
                        out.bytes
                            .extend(decode_final(&chunk, false).unwrap_or_default());
                    }
                }
            }
            out.read = len;
            return out;
        };
        index += 1;
        if u == u16::from(b'=') {
            if chunk.len() < 2 {
                return fail(out, BAD_CHARACTER);
            }
            index = skip_space(s, index);
            if chunk.len() == 2 {
                let Some(&next) = s.get(index) else {
                    if last == LastChunk::StopBeforePartial {
                        return out;
                    }
                    return fail(out, BAD_CHARACTER);
                };
                if next == u16::from(b'=') {
                    index = skip_space(s, index + 1);
                }
            }
            if index < len {
                return fail(out, BAD_CHARACTER);
            }
            let Some(bytes) = decode_final(&chunk, last == LastChunk::Strict) else {
                return fail(out, EXTRA_BITS);
            };
            out.bytes.extend(bytes);
            out.read = len;
            return out;
        }
        let u = if url {
            match u {
                0x2b | 0x2f => return fail(out, BAD_CHARACTER),
                0x2d => 0x2b,
                0x5f => 0x2f,
                u => u,
            }
        } else {
            u
        };
        let Some(bits) = sextet(u) else {
            return fail(out, BAD_CHARACTER);
        };
        let remaining = max - out.bytes.len();
        if (remaining == 1 && chunk.len() == 2) || (remaining == 2 && chunk.len() == 3) {
            return out;
        }
        chunk.push(bits);
        if chunk.len() == 4 {
            out.bytes.extend(decode_chunk(&chunk));
            chunk.clear();
            out.read = index;
            if out.bytes.len() == max {
                return out;
            }
        }
    }
}

/// FromHex (section 23.3.2.3's): at most `max` bytes of `s`.
fn decode_hex(s: &[u16], max: usize) -> Decoded {
    let mut out = Decoded {
        read: 0,
        bytes: Vec::new(),
        error: None,
    };
    if s.len() % 2 != 0 {
        out.error = Some(BAD_HEX);
        return out;
    }
    let digit = |u: u16| char::from_u32(u32::from(u)).and_then(|c| c.to_digit(16));
    for pair in s.chunks(2) {
        if out.bytes.len() >= max {
            break;
        }
        match (
            pair.first().and_then(|&u| digit(u)),
            pair.get(1).and_then(|&u| digit(u)),
        ) {
            (Some(h), Some(l)) => out.bytes.push((h * 16 + l) as u8),
            _ => {
                out.error = Some(BAD_HEX);
                return out;
            }
        }
        out.read += 2;
    }
    out
}

fn encode_base64(bytes: &[u8], url: bool, omit_padding: bool) -> String {
    let table = if url { URL } else { STANDARD };
    let at = |i: u32| char::from(table.get((i & 63) as usize).copied().unwrap_or(b'A'));
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let b = |i: usize| u32::from(c.get(i).copied().unwrap_or(0));
        let n = (b(0) << 16) | (b(1) << 8) | b(2);
        out.push(at(n >> 18));
        out.push(at(n >> 12));
        if c.len() > 1 {
            out.push(at(n >> 6));
        } else if !omit_padding {
            out.push('=');
        }
        if c.len() > 2 {
            out.push(at(n));
        } else if !omit_padding {
            out.push('=');
        }
    }
    out
}

impl Agent {
    /// ValidateUint8Array (section 23.3.2.1's): `this` as a `Uint8Array`.
    fn this_uint8_array(&mut self, this: Value, method: &str) -> Res<u32> {
        let Some(o) = this.as_object().filter(|&o| self.heap.view(o).is_some()) else {
            return Err(self.type_error("this is not a typed array."));
        };
        if self.heap.view(o).is_some_and(|v| v.elem != Elem::Uint8) {
            // The reference browser names no receiver here.
            return Err(self.type_error(format!(
                "Method {method} called on incompatible receiver undefined"
            )));
        }
        Ok(o)
    }

    /// The string argument the decoders take.
    fn input_string(&mut self, v: Value) -> Res<Vec<u16>> {
        match v.as_string().and_then(|_| self.string_units(v)) {
            Some(u) => Ok(u),
            None => Err(self.type_error("input argument must be a string")),
        }
    }

    /// GetOptionsObject (section 14.5.2.1's): undefined for none.
    fn options_object(&mut self, v: Value) -> Res<Value> {
        if v.is_undefined() || v.as_object().is_some() {
            Ok(v)
        } else {
            Err(self.type_error("invalid_argument"))
        }
    }

    /// The option `name` of `opts`, one of `allowed` (the first when it
    /// is undefined): its index in `allowed`.
    fn option_of(&mut self, opts: Value, name: &'static str, allowed: &[&str]) -> Res<usize> {
        if opts.is_undefined() {
            return Ok(0);
        }
        let key = self.name_key(Name::Str(name));
        let v = self.get_value(opts, key)?;
        if v.is_undefined() {
            return Ok(0);
        }
        let text = v
            .as_string()
            .and_then(|_| self.string_units(v))
            .map(|u| String::from_utf16_lossy(&u));
        match text.and_then(|t| allowed.iter().position(|a| *a == t)) {
            Some(i) => Ok(i),
            None => {
                let shown = self.display(v);
                Err(self.type_error(format!("invalid option {shown}")))
            }
        }
    }

    /// The options `fromBase64` and `setFromBase64` read: whether the
    /// alphabet is base64url, and the last chunk's handling.
    fn decode_options(&mut self, options: Value) -> Res<(bool, LastChunk)> {
        let opts = self.options_object(options)?;
        let url = self.option_of(opts, "alphabet", &["base64", "base64url"])? == 1;
        let last = match self.option_of(
            opts,
            "lastChunkHandling",
            &["loose", "strict", "stop-before-partial"],
        )? {
            0 => LastChunk::Loose,
            1 => LastChunk::Strict,
            _ => LastChunk::StopBeforePartial,
        };
        Ok((url, last))
    }

    /// The bytes of `Uint8Array` `o`, or the TypeError of `method` when it
    /// is out of bounds.
    fn uint8_bytes(&mut self, o: u32, method: &str) -> Res<Vec<u8>> {
        match self.heap.view_length(o) {
            Some(len) => Ok(self.heap.view_bytes(o, 0, len)),
            None => Err(self.out_of_bounds(method)),
        }
    }

    /// A new `Uint8Array` of `bytes`, or the SyntaxError `d` stopped at.
    fn uint8_array_of(&mut self, d: Decoded) -> Res<Value> {
        if let Some(e) = d.error {
            return Err(self.syntax_error(e));
        }
        let proto = self.realm().intrinsics.typed_proto(Elem::Uint8);
        let a = self.view_with_length(proto, Elem::Uint8, d.bytes.len())?;
        if let Some(o) = a.as_object() {
            self.heap.store_view_bytes(o, 0, &d.bytes);
        }
        Ok(a)
    }

    /// The `setFrom` methods' end: what `d` decoded written into `o`,
    /// then its SyntaxError, or `{ read, written }`.
    fn set_from(&mut self, o: u32, d: Decoded) -> Res<Value> {
        self.heap.store_view_bytes(o, 0, &d.bytes);
        if let Some(e) = d.error {
            return Err(self.syntax_error(e));
        }
        let r = self.plain_object()?;
        self.vm.temps.push(r);
        let key = self.name_key(Name::Str("read"));
        self.create_data_property(r, key, Value::number(d.read as f64));
        let key = self.name_key(Name::Str("written"));
        self.create_data_property(r, key, Value::number(d.bytes.len() as f64));
        self.vm.temps.pop();
        Ok(r)
    }
}

/// `Uint8Array.fromBase64` (section 23.3.1.1).
fn from_base64(agent: &mut Agent, c: Call) -> Res<Value> {
    let s = agent.input_string(agent.arg(c, 0))?;
    let (url, last) = agent.decode_options(agent.arg(c, 1))?;
    let d = decode_base64(&s, url, last, usize::MAX);
    agent.uint8_array_of(d)
}

/// `Uint8Array.fromHex` (section 23.3.1.2).
fn from_hex(agent: &mut Agent, c: Call) -> Res<Value> {
    let s = agent.input_string(agent.arg(c, 0))?;
    let d = decode_hex(&s, usize::MAX);
    agent.uint8_array_of(d)
}

/// `Uint8Array.prototype.toBase64` (section 23.3.3.3).
fn to_base64(agent: &mut Agent, c: Call) -> Res<Value> {
    const M: &str = "Uint8Array.prototype.toBase64";
    let o = agent.this_uint8_array(c.this, M)?;
    let opts = agent.options_object(agent.arg(c, 0))?;
    let url = agent.option_of(opts, "alphabet", &["base64", "base64url"])? == 1;
    let omit = if opts.is_undefined() {
        false
    } else {
        let key = agent.name_key(Name::Str("omitPadding"));
        let v = agent.get_value(opts, key)?;
        agent.to_boolean(v)
    };
    let bytes = agent.uint8_bytes(o, M)?;
    let text = encode_base64(&bytes, url, omit);
    agent.new_string(&text)
}

/// `Uint8Array.prototype.toHex` (section 23.3.3.4).
fn to_hex(agent: &mut Agent, c: Call) -> Res<Value> {
    const M: &str = "Uint8Array.prototype.toHex";
    let o = agent.this_uint8_array(c.this, M)?;
    let bytes = agent.uint8_bytes(o, M)?;
    let mut text = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        text.push_str(&format!("{b:02x}"));
    }
    agent.new_string(&text)
}

/// `Uint8Array.prototype.setFromBase64` (section 23.3.3.1).
fn set_from_base64(agent: &mut Agent, c: Call) -> Res<Value> {
    const M: &str = "Uint8Array.prototype.setFromBase64";
    let o = agent.this_uint8_array(c.this, M)?;
    let s = agent.input_string(agent.arg(c, 0))?;
    let (url, last) = agent.decode_options(agent.arg(c, 1))?;
    let Some(len) = agent.heap.view_length(o) else {
        return Err(agent.out_of_bounds(M));
    };
    let d = decode_base64(&s, url, last, len);
    agent.set_from(o, d)
}

/// `Uint8Array.prototype.setFromHex` (section 23.3.3.2).
fn set_from_hex(agent: &mut Agent, c: Call) -> Res<Value> {
    const M: &str = "Uint8Array.prototype.setFromHex";
    let o = agent.this_uint8_array(c.this, M)?;
    let s = agent.input_string(agent.arg(c, 0))?;
    let Some(len) = agent.heap.view_length(o) else {
        return Err(agent.out_of_bounds(M));
    };
    let d = decode_hex(&s, len);
    agent.set_from(o, d)
}
