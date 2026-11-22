//! `String` (ECMA-262 section 22.1). `match`, `matchAll` and `search`
//! come with RegExp (J4), `normalize` with the normalization tables,
//! and `[Symbol.iterator]` with J3's iterators; `replace`, `replaceAll`
//! and `split` already hand a pattern with `@@replace` or `@@split` to
//! it, as RegExp's will be.

use super::global::{trim_end, trim_start};
use super::{CONSTRUCTOR, Call, Entry, HIDDEN, Name, What, Wk, entry, method};
use crate::agent::Agent;
use crate::interp::Res;
use crate::object::ObjectKind;
use crate::string::PropertyKey;
use crate::value::{RefKind, Value};

/// The longest string, in units: past it, the reference browser's
/// "Invalid string length".
pub(crate) const MAX_LENGTH: usize = (1 << 29) - 24;

pub(crate) const STATICS: &[Entry] = &[
    method("fromCharCode", 1, from_char_code),
    method("fromCodePoint", 1, from_code_point),
    method("raw", 1, raw),
];

pub(crate) const PROTO: &[Entry] = &[
    CONSTRUCTOR,
    method("anchor", 1, anchor),
    method("at", 1, at),
    method("big", 0, big),
    method("blink", 0, blink),
    method("bold", 0, bold),
    method("charAt", 1, char_at),
    method("charCodeAt", 1, char_code_at),
    method("codePointAt", 1, code_point_at),
    method("concat", 1, concat),
    method("endsWith", 1, ends_with),
    method("fontcolor", 1, fontcolor),
    method("fontsize", 1, fontsize),
    method("fixed", 0, fixed),
    method("includes", 1, includes),
    method("indexOf", 1, index_of),
    method("isWellFormed", 0, is_well_formed),
    method("italics", 0, italics),
    method("lastIndexOf", 1, last_index_of),
    method("link", 1, link),
    method("localeCompare", 1, locale_compare),
    method("padEnd", 1, pad_end),
    method("padStart", 1, pad_start),
    method("repeat", 1, repeat),
    method("replace", 2, replace),
    method("replaceAll", 2, replace_all),
    method("slice", 2, slice),
    method("small", 0, small),
    method("split", 2, split),
    method("strike", 0, strike),
    method("sub", 0, sub),
    method("substr", 2, substr),
    method("substring", 2, substring),
    method("sup", 0, sup),
    method("startsWith", 1, starts_with),
    method("toString", 0, to_string),
    method("toWellFormed", 0, to_well_formed),
    method("trim", 0, trim),
    method("trimStart", 0, trim_start_native),
    entry(Name::Str("trimLeft"), What::Alias("trimStart"), HIDDEN),
    method("trimEnd", 0, trim_end_native),
    entry(Name::Str("trimRight"), What::Alias("trimEnd"), HIDDEN),
    method("toLocaleLowerCase", 0, to_lower_case),
    method("toLocaleUpperCase", 0, to_upper_case),
    method("toLowerCase", 0, to_lower_case),
    method("toUpperCase", 0, to_upper_case),
    method("valueOf", 0, value_of),
];

/// `String(value)` (section 22.1.1.1).
pub(crate) fn construct(agent: &mut Agent, c: Call) -> Res<Value> {
    let v = agent.arg(c, 0);
    let s = if c.argc == 0 {
        agent.realm().strings.empty
    } else if c.new_target.is_undefined() && matches!(v.as_reference(), Some((RefKind::Symbol, _)))
    {
        let text = agent.symbol_text(v);
        agent.new_string_units(&text)?
    } else {
        agent.string_value(v)?
    };
    if c.new_target.is_undefined() {
        return Ok(s);
    }
    agent.vm.temps.push(s);
    let proto = agent.prototype_from(c.new_target, |r| r.string_prototype);
    let w = proto.and_then(|p| agent.wrap(s, Some(p)));
    agent.vm.temps.pop();
    w
}

impl Agent {
    /// A string of `units`, or the reference browser's RangeError past
    /// the longest.
    pub(crate) fn string_checked(&mut self, units: &[u16]) -> Res<Value> {
        if units.len() > MAX_LENGTH {
            return Err(self.range_error("Invalid string length"));
        }
        self.new_string_units(units)
    }

    /// RequireObjectCoercible(this) and ToString, for `method`.
    fn this_units(&mut self, this: Value, method: &str) -> Res<Vec<u16>> {
        if this.is_nullish() {
            return Err(self.type_error(format!(
                "String.prototype.{method} called on null or undefined"
            )));
        }
        self.text_units(this)
    }

    /// IsRegExp (section 7.2.6), as far as `@@match` tells it.
    fn is_regexp(&mut self, v: Value) -> Res<bool> {
        if v.as_object().is_none() {
            return Ok(false);
        }
        let key = self.well_known_key(Wk::Match);
        let m = self.get_value(v, key)?;
        Ok(!m.is_undefined() && self.to_boolean(m))
    }

    fn no_regexp(&mut self, v: Value, method: &str) -> Res<()> {
        if self.is_regexp(v)? {
            return Err(self.type_error(format!(
                "First argument to String.prototype.{method} must not be a regular expression"
            )));
        }
        Ok(())
    }
}

/// The first index at or after `from` where `needle` is in `hay`.
pub(crate) fn find(hay: &[u16], needle: &[u16], from: usize) -> Option<usize> {
    if needle.is_empty() {
        return (from <= hay.len()).then_some(from);
    }
    if needle.len() > hay.len() {
        return None;
    }
    (from..=hay.len() - needle.len()).find(|&i| hay.get(i..i + needle.len()) == Some(needle))
}

fn rfind(hay: &[u16], needle: &[u16], from: usize) -> Option<usize> {
    if needle.len() > hay.len() {
        return None;
    }
    let start = from.min(hay.len() - needle.len());
    (0..=start)
        .rev()
        .find(|&i| hay.get(i..i + needle.len()) == Some(needle))
}

fn units_of(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

fn from_char_code(agent: &mut Agent, c: Call) -> Res<Value> {
    let mut out = Vec::with_capacity(c.argc);
    for i in 0..c.argc {
        let n = agent.number(agent.arg(c, i))?;
        out.push(crate::interp::ops::to_int32(n) as u16);
    }
    if let [u] = out.as_slice() {
        return agent.unit_string(*u);
    }
    agent.new_string_units(&out)
}

fn from_code_point(agent: &mut Agent, c: Call) -> Res<Value> {
    let mut out = Vec::with_capacity(c.argc);
    for i in 0..c.argc {
        let v = agent.arg(c, i);
        let n = agent.number(v)?;
        if n.trunc() != n || !(0.0..=1_114_111.0).contains(&n) {
            let text = crate::number::number_to_string(n);
            return Err(agent.range_error(format!("Invalid code point {text}")));
        }
        push_code_point(&mut out, n as u32);
    }
    agent.string_checked(&out)
}

fn push_code_point(out: &mut Vec<u16>, cp: u32) {
    if cp < 0x10000 {
        out.push(cp as u16);
    } else {
        let v = cp - 0x10000;
        out.push(0xd800 | (v >> 10) as u16);
        out.push(0xdc00 | (v & 0x3ff) as u16);
    }
}

fn raw(agent: &mut Agent, c: Call) -> Res<Value> {
    let cooked = agent.object_of(agent.arg(c, 0))?;
    let key = agent.name_key(Name::Str("raw"));
    let r = agent.get_value(cooked, key)?;
    let r = agent.object_of(r)?;
    agent.vm.temps.push(r);
    let result = (|| {
        let len = agent.length_of(r)?;
        let mut out = Vec::new();
        let mut i = 0.0;
        while i < len {
            let key = agent.index_key(i);
            let seg = agent.get_value(r, key)?;
            out.extend(agent.text_units(seg)?);
            if out.len() > MAX_LENGTH {
                return Err(agent.range_error("Invalid string length"));
            }
            i += 1.0;
            if i < len && (i as usize) < c.argc {
                let sub = agent.arg(c, i as usize);
                out.extend(agent.text_units(sub)?);
            }
        }
        agent.string_checked(&out)
    })();
    agent.vm.temps.pop();
    result
}

/// CreateHTML (section 22.1.3.2.1, Annex B).
fn html(agent: &mut Agent, c: Call, method: &str, tag: &str, attr: Option<&str>) -> Res<Value> {
    let s = agent.this_units(c.this, method)?;
    let mut out = units_of(&format!("<{tag}"));
    if let Some(attr) = attr {
        let v = agent.text_units(agent.arg(c, 0))?;
        out.extend(units_of(&format!(" {attr}=\"")));
        for u in v {
            if u == u16::from(b'"') {
                out.extend(units_of("&quot;"));
            } else {
                out.push(u);
            }
        }
        out.push(u16::from(b'"'));
    }
    out.push(u16::from(b'>'));
    out.extend(s);
    out.extend(units_of(&format!("</{tag}>")));
    agent.string_checked(&out)
}

macro_rules! html_methods {
    ($($name:ident => $method:literal, $tag:literal, $attr:expr;)*) => {$(
        fn $name(agent: &mut Agent, c: Call) -> Res<Value> {
            html(agent, c, $method, $tag, $attr)
        }
    )*};
}

html_methods! {
    anchor => "anchor", "a", Some("name");
    big => "big", "big", None;
    blink => "blink", "blink", None;
    bold => "bold", "b", None;
    fixed => "fixed", "tt", None;
    fontcolor => "fontcolor", "font", Some("color");
    fontsize => "fontsize", "font", Some("size");
    italics => "italics", "i", None;
    link => "link", "a", Some("href");
    small => "small", "small", None;
    strike => "strike", "strike", None;
    sub => "sub", "sub", None;
    sup => "sup", "sup", None;
}

fn at(agent: &mut Agent, c: Call) -> Res<Value> {
    let s = agent.this_units(c.this, "at")?;
    let len = s.len() as f64;
    let n = agent.integer(agent.arg(c, 0))?;
    let k = if n >= 0.0 { n } else { len + n };
    if k < 0.0 || k >= len {
        return Ok(Value::UNDEFINED);
    }
    let u = s.get(k as usize).copied().unwrap_or(0);
    agent.unit_string(u)
}

fn char_at(agent: &mut Agent, c: Call) -> Res<Value> {
    let s = agent.this_units(c.this, "charAt")?;
    let n = agent.integer(agent.arg(c, 0))?;
    if n < 0.0 || n >= s.len() as f64 {
        return Ok(agent.realm().strings.empty);
    }
    let u = s.get(n as usize).copied().unwrap_or(0);
    agent.unit_string(u)
}

fn char_code_at(agent: &mut Agent, c: Call) -> Res<Value> {
    let s = agent.this_units(c.this, "charCodeAt")?;
    let n = agent.integer(agent.arg(c, 0))?;
    if n < 0.0 || n >= s.len() as f64 {
        return Ok(Value::number(f64::NAN));
    }
    Ok(Value::int(i32::from(
        s.get(n as usize).copied().unwrap_or(0),
    )))
}

/// CodePointAt (section 11.1.4): the code point at `i`, and its length.
pub(crate) fn code_point(s: &[u16], i: usize) -> (u32, usize) {
    let first = s.get(i).copied().unwrap_or(0);
    if (0xd800..0xdc00).contains(&first)
        && let Some(&second) = s.get(i + 1)
        && (0xdc00..0xe000).contains(&second)
    {
        let cp = 0x10000 + ((u32::from(first) - 0xd800) << 10) + (u32::from(second) - 0xdc00);
        return (cp, 2);
    }
    (u32::from(first), 1)
}

fn code_point_at(agent: &mut Agent, c: Call) -> Res<Value> {
    let s = agent.this_units(c.this, "codePointAt")?;
    let n = agent.integer(agent.arg(c, 0))?;
    if n < 0.0 || n >= s.len() as f64 {
        return Ok(Value::UNDEFINED);
    }
    let (cp, _) = code_point(&s, n as usize);
    Ok(Value::int(cp as i32))
}

fn concat(agent: &mut Agent, c: Call) -> Res<Value> {
    let mut s = agent.this_units(c.this, "concat")?;
    for i in 0..c.argc {
        let next = agent.text_units(agent.arg(c, i))?;
        s.extend(next);
        if s.len() > MAX_LENGTH {
            return Err(agent.range_error("Invalid string length"));
        }
    }
    agent.new_string_units(&s)
}

fn ends_with(agent: &mut Agent, c: Call) -> Res<Value> {
    let s = agent.this_units(c.this, "endsWith")?;
    let search = agent.arg(c, 0);
    agent.no_regexp(search, "endsWith")?;
    let search = agent.text_units(search)?;
    let len = s.len() as f64;
    let end = agent.arg(c, 1);
    let end = if end.is_undefined() {
        len
    } else {
        agent.integer(end)?.clamp(0.0, len)
    } as usize;
    let Some(start) = end.checked_sub(search.len()) else {
        return Ok(Value::FALSE);
    };
    Ok(Value::bool(s.get(start..end) == Some(&search[..])))
}

fn includes(agent: &mut Agent, c: Call) -> Res<Value> {
    let s = agent.this_units(c.this, "includes")?;
    let search = agent.arg(c, 0);
    agent.no_regexp(search, "includes")?;
    let search = agent.text_units(search)?;
    let pos = agent.integer(agent.arg(c, 1))?.clamp(0.0, s.len() as f64) as usize;
    Ok(Value::bool(find(&s, &search, pos).is_some()))
}

fn index_of(agent: &mut Agent, c: Call) -> Res<Value> {
    let s = agent.this_units(c.this, "indexOf")?;
    let search = agent.text_units(agent.arg(c, 0))?;
    let pos = agent.integer(agent.arg(c, 1))?.clamp(0.0, s.len() as f64) as usize;
    Ok(Value::number(
        find(&s, &search, pos).map_or(-1.0, |i| i as f64),
    ))
}

fn last_index_of(agent: &mut Agent, c: Call) -> Res<Value> {
    let s = agent.this_units(c.this, "lastIndexOf")?;
    let search = agent.text_units(agent.arg(c, 0))?;
    let n = agent.number(agent.arg(c, 1))?;
    let pos = if n.is_nan() { f64::INFINITY } else { n.trunc() };
    let start = pos.clamp(0.0, s.len() as f64) as usize;
    Ok(Value::number(
        rfind(&s, &search, start).map_or(-1.0, |i| i as f64),
    ))
}

fn is_lone(s: &[u16], i: usize) -> bool {
    let u = s.get(i).copied().unwrap_or(0);
    if (0xd800..0xdc00).contains(&u) {
        !s.get(i + 1).is_some_and(|n| (0xdc00..0xe000).contains(n))
    } else if (0xdc00..0xe000).contains(&u) {
        i == 0 || !s.get(i - 1).is_some_and(|p| (0xd800..0xdc00).contains(p))
    } else {
        false
    }
}

fn is_well_formed(agent: &mut Agent, c: Call) -> Res<Value> {
    let s = agent.this_units(c.this, "isWellFormed")?;
    Ok(Value::bool((0..s.len()).all(|i| !is_lone(&s, i))))
}

fn to_well_formed(agent: &mut Agent, c: Call) -> Res<Value> {
    let s = agent.this_units(c.this, "toWellFormed")?;
    let out: Vec<u16> = (0..s.len())
        .map(|i| {
            if is_lone(&s, i) {
                0xfffd
            } else {
                s.get(i).copied().unwrap_or(0)
            }
        })
        .collect();
    agent.new_string_units(&out)
}

/// By code points; locales are the embedder's Intl (J7).
fn locale_compare(agent: &mut Agent, c: Call) -> Res<Value> {
    let s = agent.this_units(c.this, "localeCompare")?;
    let that = agent.text_units(agent.arg(c, 0))?;
    let a: Vec<char> = char::decode_utf16(s)
        .map(|r| r.unwrap_or('\u{fffd}'))
        .collect();
    let b: Vec<char> = char::decode_utf16(that)
        .map(|r| r.unwrap_or('\u{fffd}'))
        .collect();
    Ok(Value::int(match a.cmp(&b) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }))
}

fn pad(agent: &mut Agent, c: Call, at_start: bool) -> Res<Value> {
    let method = if at_start { "padStart" } else { "padEnd" };
    let s = agent.this_units(c.this, method)?;
    let max = agent.length_value(agent.arg(c, 0))?;
    let len = s.len() as f64;
    if max <= len {
        return agent.new_string_units(&s);
    }
    let fill = agent.arg(c, 1);
    let fill = if fill.is_undefined() {
        vec![u16::from(b' ')]
    } else {
        agent.text_units(fill)?
    };
    if fill.is_empty() {
        return agent.new_string_units(&s);
    }
    if max > MAX_LENGTH as f64 {
        return Err(agent.range_error("Invalid string length"));
    }
    let n = max as usize - s.len();
    let padding: Vec<u16> = fill.iter().copied().cycle().take(n).collect();
    let out = if at_start {
        [padding, s].concat()
    } else {
        [s, padding].concat()
    };
    agent.new_string_units(&out)
}

fn pad_start(agent: &mut Agent, c: Call) -> Res<Value> {
    pad(agent, c, true)
}

fn pad_end(agent: &mut Agent, c: Call) -> Res<Value> {
    pad(agent, c, false)
}

fn repeat(agent: &mut Agent, c: Call) -> Res<Value> {
    let s = agent.this_units(c.this, "repeat")?;
    let n = agent.integer(agent.arg(c, 0))?;
    if n < 0.0 || n.is_infinite() {
        let text = crate::number::number_to_string(n);
        return Err(agent.range_error(format!("Invalid count value: {text}")));
    }
    if n == 0.0 || s.is_empty() {
        return Ok(agent.realm().strings.empty);
    }
    if s.len() as f64 * n > MAX_LENGTH as f64 {
        return Err(agent.range_error("Invalid string length"));
    }
    agent.new_string_units(&s.repeat(n as usize))
}

impl Agent {
    /// GetSubstitution (section 22.1.3.19.1): `replacement`'s `$`
    /// patterns for `matched` at `position` in `s`, with `captures`.
    pub(crate) fn substitution(
        &mut self,
        matched: &[u16],
        s: &[u16],
        position: usize,
        captures: &[Option<Vec<u16>>],
        named: Value,
        replacement: &[u16],
    ) -> Res<Vec<u16>> {
        let mut out = Vec::new();
        let tail = (position + matched.len()).min(s.len());
        let m = captures.len();
        let mut i = 0;
        let at = |i: usize| replacement.get(i).copied().unwrap_or(0);
        let digit = |u: u16| (u16::from(b'0')..=u16::from(b'9')).contains(&u);
        while i < replacement.len() {
            let u = at(i);
            if u != u16::from(b'$') || i + 1 >= replacement.len() {
                out.push(u);
                i += 1;
                continue;
            }
            let next = at(i + 1);
            match next {
                0x24 => {
                    out.push(0x24);
                    i += 2;
                }
                0x26 => {
                    out.extend_from_slice(matched);
                    i += 2;
                }
                0x60 => {
                    out.extend_from_slice(s.get(..position.min(s.len())).unwrap_or(&[]));
                    i += 2;
                }
                0x27 => {
                    out.extend_from_slice(s.get(tail..).unwrap_or(&[]));
                    i += 2;
                }
                _ if digit(next) => {
                    let d1 = usize::from(next - 0x30);
                    let two = digit(at(i + 2)).then(|| d1 * 10 + usize::from(at(i + 2) - 0x30));
                    let (index, len) = match two {
                        Some(n) if (1..=m).contains(&n) => (n, 3),
                        _ => (d1, 2),
                    };
                    if (1..=m).contains(&index) {
                        if let Some(Some(cap)) = captures.get(index - 1) {
                            out.extend_from_slice(cap);
                        }
                        i += len;
                    } else {
                        out.push(0x24);
                        i += 1;
                    }
                }
                0x3c if !named.is_undefined() => {
                    let close = replacement
                        .get(i + 2..)
                        .and_then(|r| r.iter().position(|&x| x == u16::from(b'>')));
                    match close {
                        None => {
                            out.push(0x24);
                            i += 1;
                        }
                        Some(k) => {
                            let name = replacement.get(i + 2..i + 2 + k).unwrap_or(&[]).to_vec();
                            let key = self.heap.atoms.key(&name);
                            let v = self.get_value(named, key)?;
                            if !v.is_undefined() {
                                out.extend(self.text_units(v)?);
                            }
                            i += k + 3;
                        }
                    }
                }
                _ => {
                    out.push(0x24);
                    i += 1;
                }
            }
        }
        Ok(out)
    }

    /// `@@replace` or `@@split` of a pattern object, when it has one (a
    /// primitive's are never looked up).
    fn pattern_method(&mut self, pattern: Value, w: Wk) -> Res<Value> {
        if pattern.as_object().is_none() {
            return Ok(Value::UNDEFINED);
        }
        let key = self.well_known_key(w);
        self.get_method(pattern, key)
    }
}

fn replace_impl(agent: &mut Agent, c: Call, all: bool) -> Res<Value> {
    let method = if all { "replaceAll" } else { "replace" };
    if c.this.is_nullish() {
        return Err(agent.type_error(format!(
            "String.prototype.{method} called on null or undefined"
        )));
    }
    let search = agent.arg(c, 0);
    let replace_value = agent.arg(c, 1);
    if all && agent.is_regexp(search)? {
        let key = agent.name_key(Name::Str("flags"));
        let flags = agent.get_value(search, key)?;
        if flags.is_nullish() {
            return Err(agent.type_error(crate::interp::messages::NULLISH_TO_OBJECT));
        }
        if !agent.text_units(flags)?.contains(&u16::from(b'g')) {
            return Err(agent.type_error("replaceAll must be called with a global RegExp"));
        }
    }
    let replacer = agent.pattern_method(search, Wk::Replace)?;
    if !replacer.is_undefined() {
        return agent.call_value(replacer, search, &[c.this, replace_value]);
    }
    let s = agent.text_units(c.this)?;
    let needle = agent.text_units(search)?;
    let functional = agent.is_callable(replace_value);
    let template = if functional {
        Vec::new()
    } else {
        agent.text_units(replace_value)?
    };
    let mut positions = Vec::new();
    let mut from = find(&s, &needle, 0);
    while let Some(p) = from {
        positions.push(p);
        if !all {
            break;
        }
        from = find(&s, &needle, p + needle.len().max(1));
    }
    let mut out = Vec::new();
    let mut end = 0;
    for p in positions {
        out.extend_from_slice(s.get(end..p).unwrap_or(&[]));
        let replacement = if functional {
            let m = agent.new_string_units(&needle)?;
            agent.vm.temps.push(m);
            let whole = agent.new_string_units(&s);
            let r = whole.and_then(|whole| {
                agent.vm.temps.push(whole);
                let r = agent.call_value(
                    replace_value,
                    Value::UNDEFINED,
                    &[m, Value::number(p as f64), whole],
                );
                agent.vm.temps.pop();
                r
            });
            agent.vm.temps.pop();
            agent.text_units(r?)?
        } else {
            agent.substitution(&needle, &s, p, &[], Value::UNDEFINED, &template)?
        };
        out.extend(replacement);
        if out.len() > MAX_LENGTH {
            return Err(agent.range_error("Invalid string length"));
        }
        end = p + needle.len();
    }
    out.extend_from_slice(s.get(end..).unwrap_or(&[]));
    agent.string_checked(&out)
}

fn replace(agent: &mut Agent, c: Call) -> Res<Value> {
    replace_impl(agent, c, false)
}

fn replace_all(agent: &mut Agent, c: Call) -> Res<Value> {
    replace_impl(agent, c, true)
}

fn slice(agent: &mut Agent, c: Call) -> Res<Value> {
    let s = agent.this_units(c.this, "slice")?;
    let len = s.len() as f64;
    let from = agent.relative(agent.arg(c, 0), len, 0.0)?;
    let to = agent.relative(agent.arg(c, 1), len, len)?;
    if from >= to {
        return Ok(agent.realm().strings.empty);
    }
    let part = s.get(from as usize..to as usize).unwrap_or(&[]);
    agent.new_string_units(part)
}

fn split(agent: &mut Agent, c: Call) -> Res<Value> {
    if c.this.is_nullish() {
        return Err(agent.type_error("String.prototype.split called on null or undefined"));
    }
    let sep = agent.arg(c, 0);
    let limit = agent.arg(c, 1);
    let splitter = agent.pattern_method(sep, Wk::Split)?;
    if !splitter.is_undefined() {
        return agent.call_value(splitter, sep, &[c.this, limit]);
    }
    let s = agent.text_units(c.this)?;
    let lim = if limit.is_undefined() {
        u32::MAX
    } else {
        let n = agent.number(limit)?;
        crate::interp::ops::to_int32(n) as u32
    };
    let r = agent.text_units(sep)?;
    let out = agent.array_of(&[])?;
    if lim == 0 {
        return Ok(out);
    }
    agent.vm.temps.push(out);
    let result = (|| {
        let mut parts: Vec<Vec<u16>> = Vec::new();
        if sep.is_undefined() {
            parts.push(s);
        } else if s.is_empty() {
            if !r.is_empty() {
                parts.push(s);
            }
        } else if r.is_empty() {
            parts.extend(s.iter().take(lim as usize).map(|&u| vec![u]));
        } else {
            let mut p = 0;
            while let Some(q) = find(&s, &r, p) {
                parts.push(s.get(p..q).unwrap_or(&[]).to_vec());
                if parts.len() as u32 >= lim {
                    break;
                }
                p = q + r.len();
            }
            if (parts.len() as u32) < lim {
                parts.push(s.get(p..).unwrap_or(&[]).to_vec());
            }
        }
        for (i, part) in parts.into_iter().take(lim as usize).enumerate() {
            let v = if let [u] = part.as_slice() {
                agent.unit_string(*u)?
            } else {
                agent.new_string_units(&part)?
            };
            agent.create_data_property(out, PropertyKey::Index(i as u32), v);
        }
        Ok(out)
    })();
    agent.vm.temps.pop();
    result
}

fn substr(agent: &mut Agent, c: Call) -> Res<Value> {
    let s = agent.this_units(c.this, "substr")?;
    let size = s.len() as f64;
    let start = agent.relative(agent.arg(c, 0), size, 0.0)?;
    let length = agent.arg(c, 1);
    let length = if length.is_undefined() {
        size
    } else {
        agent.integer(length)?
    };
    let end = (start + length).clamp(start, size);
    if start >= end {
        return Ok(agent.realm().strings.empty);
    }
    let part = s.get(start as usize..end as usize).unwrap_or(&[]);
    agent.new_string_units(part)
}

fn substring(agent: &mut Agent, c: Call) -> Res<Value> {
    let s = agent.this_units(c.this, "substring")?;
    let len = s.len() as f64;
    let a = agent.integer(agent.arg(c, 0))?.clamp(0.0, len);
    let end = agent.arg(c, 1);
    let b = if end.is_undefined() {
        len
    } else {
        agent.integer(end)?.clamp(0.0, len)
    };
    let (from, to) = if a < b { (a, b) } else { (b, a) };
    let part = s.get(from as usize..to as usize).unwrap_or(&[]);
    agent.new_string_units(part)
}

fn starts_with(agent: &mut Agent, c: Call) -> Res<Value> {
    let s = agent.this_units(c.this, "startsWith")?;
    let search = agent.arg(c, 0);
    agent.no_regexp(search, "startsWith")?;
    let search = agent.text_units(search)?;
    let start = agent.integer(agent.arg(c, 1))?.clamp(0.0, s.len() as f64) as usize;
    Ok(Value::bool(
        s.get(start..start + search.len()) == Some(&search[..]),
    ))
}

fn this_string(agent: &mut Agent, v: Value, method: &str) -> Res<Value> {
    match agent.this_primitive(v, ObjectKind::String) {
        Some(s) => Ok(s),
        None => Err(agent.type_error(format!(
            "String.prototype.{method} requires that 'this' be a String"
        ))),
    }
}

fn to_string(agent: &mut Agent, c: Call) -> Res<Value> {
    this_string(agent, c.this, "toString")
}

fn value_of(agent: &mut Agent, c: Call) -> Res<Value> {
    this_string(agent, c.this, "valueOf")
}

fn trimmed(agent: &mut Agent, c: Call, method: &str, start: bool, end: bool) -> Res<Value> {
    let s = agent.this_units(c.this, method)?;
    let mut t: &[u16] = &s;
    if start {
        t = trim_start(t);
    }
    if end {
        t = trim_end(t);
    }
    let t = t.to_vec();
    agent.new_string_units(&t)
}

fn trim(agent: &mut Agent, c: Call) -> Res<Value> {
    trimmed(agent, c, "trim", true, true)
}

fn trim_start_native(agent: &mut Agent, c: Call) -> Res<Value> {
    trimmed(agent, c, "trimStart", true, false)
}

fn trim_end_native(agent: &mut Agent, c: Call) -> Res<Value> {
    trimmed(agent, c, "trimEnd", false, true)
}

/// Case mapping over the string's runs of well-formed text (Rust's
/// tables: full mappings, and the final sigma), lone surrogates kept.
fn map_case(s: &[u16], upper: bool) -> Vec<u16> {
    let mut out = Vec::with_capacity(s.len());
    let mut run = String::new();
    let flush = |run: &mut String, out: &mut Vec<u16>| {
        let mapped = if upper {
            run.to_uppercase()
        } else {
            run.to_lowercase()
        };
        out.extend(mapped.encode_utf16());
        run.clear();
    };
    for r in char::decode_utf16(s.iter().copied()) {
        match r {
            Ok(ch) => run.push(ch),
            Err(e) => {
                flush(&mut run, &mut out);
                out.push(e.unpaired_surrogate());
            }
        }
    }
    flush(&mut run, &mut out);
    out
}

fn to_lower_case(agent: &mut Agent, c: Call) -> Res<Value> {
    let s = agent.this_units(c.this, "toLowerCase")?;
    if s.iter().all(|&u| u < 0x80 && !(0x41..=0x5a).contains(&u)) {
        return agent.new_string_units(&s);
    }
    agent.new_string_units(&map_case(&s, false))
}

fn to_upper_case(agent: &mut Agent, c: Call) -> Res<Value> {
    let s = agent.this_units(c.this, "toUpperCase")?;
    agent.string_checked(&map_case(&s, true))
}
