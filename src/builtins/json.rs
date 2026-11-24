//! `JSON` (ECMA-262 section 25.5): `parse` with its reviver and the
//! source text it is given (the JSON.parse source text access proposal),
//! `stringify`, `rawJSON` and `isRawJSON`. Parsing, the reviver's walk
//! and serializing each keep their own stack, never the Rust one, so a
//! deeply nested text or object is as safe as a flat one. A text refused
//! is refused with the reference browser's message (the numbers probe,
//! Chrome 158), its position counted in UTF-16 units.

use super::{CONFIGURABLE, Call, Entry, Name, What, Wk, entry, method};
use crate::agent::Agent;
use crate::heap::ObjectCell;
use crate::interp::{Abrupt, Res};
use crate::object::{Descriptor, ObjectKind, OwnProperty, same_value};
use crate::string::PropertyKey;
use crate::value::Value;

/// `JSON`.
pub(crate) const JSON: &[Entry] = &[
    method("parse", 2, parse),
    method("stringify", 3, stringify),
    method("rawJSON", 1, raw_json),
    method("isRawJSON", 1, is_raw_json),
    entry(Name::Sym(Wk::ToStringTag), What::Str("JSON"), CONFIGURABLE),
];

/// Why a text is refused.
#[derive(Debug, Clone, Copy)]
enum Fault {
    /// A token no rule wants here: the message names the token.
    Token,
    /// A message of its own: what was wanted, or what the string or
    /// number scanner found.
    Says(&'static str),
}

/// A refusal and the position it names.
#[derive(Debug, Clone, Copy)]
struct Refusal {
    fault: Fault,
    at: usize,
}

enum Fail {
    Syntax(Refusal),
    Abrupt(Abrupt),
}

impl From<Abrupt> for Fail {
    fn from(a: Abrupt) -> Fail {
        Fail::Abrupt(a)
    }
}

fn refuse<T>(fault: Fault, at: usize) -> Result<T, Fail> {
    Err(Fail::Syntax(Refusal { fault, at }))
}

/// The characters of context the reference browser quotes either side
/// of a token it does not want, and the length of text it quotes whole.
const CONTEXT: usize = 10;

const QUOTE: u16 = 0x22;
const BACKSLASH: u16 = 0x5c;

fn is_space(u: u16) -> bool {
    matches!(u, 0x20 | 0x09 | 0x0a | 0x0d)
}

fn is_digit(u: u16) -> bool {
    (0x30..=0x39).contains(&u)
}

/// The reference browser's message for `r` in `s`.
fn message(s: &[u16], r: Refusal) -> String {
    let at = r.at;
    let place = || {
        let before = s.get(..at).unwrap_or(s);
        let line = 1 + before.iter().filter(|&&u| u == 0x0a).count();
        let column = at - before.iter().rposition(|&u| u == 0x0a).map_or(0, |p| p + 1) + 1;
        format!("at position {at} (line {line} column {column})")
    };
    if let Fault::Says(m) = r.fault {
        // "... after JSON at position 2", but "... in JSON at position 2".
        let within = if m.ends_with("JSON") { "" } else { "in JSON " };
        return format!("{m} {within}{}", place());
    }
    let Some(&u) = s.get(at) else {
        return "Unexpected end of JSON input".to_owned();
    };
    match r.fault {
        Fault::Says(_) => String::new(),
        Fault::Token if u == QUOTE => format!("Unexpected string in JSON {}", place()),
        Fault::Token if u == u16::from(b'-') || is_digit(u) => {
            format!("Unexpected number in JSON {}", place())
        }
        Fault::Token => {
            let whole = String::from_utf16_lossy(s);
            if ["undefined", "NaN", "Infinity", "[object Object]"].contains(&whole.as_str()) {
                return format!("\"{whole}\" is not valid JSON");
            }
            let token = String::from_utf16_lossy(&[u]);
            let len = s.len();
            let text = |a: usize, b: usize| String::from_utf16_lossy(s.get(a..b).unwrap_or(&[]));
            let quoted = if len <= CONTEXT * 2 + 1 {
                format!("\"{whole}\"")
            } else if at < CONTEXT {
                format!("\"{}\"...", text(0, at + CONTEXT))
            } else if at < len - CONTEXT {
                format!("...\"{}\"...", text(at - CONTEXT, at + CONTEXT))
            } else {
                format!("...\"{}\"", text(at - CONTEXT, len))
            };
            format!("Unexpected token '{token}', {quoted} is not valid JSON")
        }
    }
}

/// What the reviver's walk knows of a parsed value: the value, its
/// source text (a primitive's) and its elements or members.
struct Node {
    value: Value,
    span: Option<(usize, usize)>,
    kids: Vec<(PropertyKey, usize)>,
}

/// A container being filled.
struct Open {
    value: Value,
    array: bool,
    n: u32,
    key: PropertyKey,
    node: usize,
}

/// A string's contents from `s[at]`, its opening quote; the index past
/// its closing one.
fn scan_string(s: &[u16], at: usize) -> Result<(Vec<u16>, usize), Fail> {
    let mut out = Vec::new();
    let mut i = at + 1;
    loop {
        let Some(&u) = s.get(i) else {
            return refuse(Fault::Says("Unterminated string"), s.len());
        };
        match u {
            QUOTE => return Ok((out, i + 1)),
            BACKSLASH => {
                let Some(&e) = s.get(i + 1) else {
                    return refuse(Fault::Says("Unterminated string"), s.len());
                };
                let plain = match e {
                    0x22 | 0x5c | 0x2f => Some(e),
                    0x62 => Some(0x08),
                    0x66 => Some(0x0c),
                    0x6e => Some(0x0a),
                    0x72 => Some(0x0d),
                    0x74 => Some(0x09),
                    _ => None,
                };
                if let Some(p) = plain {
                    out.push(p);
                    i += 2;
                } else if e == u16::from(b'u') {
                    let mut n: u16 = 0;
                    for k in 0..4 {
                        let d = s
                            .get(i + 2 + k)
                            .and_then(|&h| char::from_u32(u32::from(h)))
                            .and_then(|c| c.to_digit(16));
                        let Some(d) = d else {
                            return refuse(Fault::Says("Bad Unicode escape"), i + 2 + k);
                        };
                        n = n * 16 + d as u16;
                    }
                    out.push(n);
                    i += 6;
                } else {
                    return refuse(Fault::Says("Bad escaped character"), i + 1);
                }
            }
            u if u < 0x20 => {
                return refuse(Fault::Says("Bad control character in string literal"), i);
            }
            u => {
                out.push(u);
                i += 1;
            }
        }
    }
}

/// A number's end, from its first character at `at`.
fn scan_number(s: &[u16], at: usize) -> Result<usize, Fail> {
    let mut i = at;
    let digits = |mut i: usize| {
        while s.get(i).is_some_and(|&u| is_digit(u)) {
            i += 1;
        }
        i
    };
    if s.get(i) == Some(&u16::from(b'-')) {
        i += 1;
    }
    match s.get(i) {
        Some(&0x30) => {
            i += 1;
            if s.get(i).is_some_and(|&u| is_digit(u)) {
                return refuse(Fault::Token, i);
            }
        }
        Some(&u) if is_digit(u) => i = digits(i),
        _ => return refuse(Fault::Says("No number after minus sign"), i),
    }
    if s.get(i) == Some(&u16::from(b'.')) {
        i += 1;
        if !s.get(i).is_some_and(|&u| is_digit(u)) {
            return refuse(Fault::Says("Unterminated fractional number"), i);
        }
        i = digits(i);
    }
    if matches!(s.get(i), Some(&0x65 | &0x45)) {
        i += 1;
        if matches!(s.get(i), Some(&0x2b | &0x2d)) {
            i += 1;
        }
        if !s.get(i).is_some_and(|&u| is_digit(u)) {
            return refuse(Fault::Says("Exponent part is missing a number"), i);
        }
        i = digits(i);
    }
    Ok(i)
}

fn skip_space(s: &[u16], mut i: usize) -> usize {
    while s.get(i).is_some_and(|&u| is_space(u)) {
        i += 1;
    }
    i
}

/// A double-quoted property name at `i` (after a comma), and the index
/// past its colon.
fn member_name(s: &[u16], i: usize, wanted: &'static str) -> Result<(Vec<u16>, usize), Fail> {
    if s.get(i) != Some(&QUOTE) {
        return refuse(Fault::Says(wanted), i);
    }
    let (name, i) = scan_string(s, i)?;
    let i = skip_space(s, i);
    if s.get(i) != Some(&u16::from(b':')) {
        return refuse(Fault::Says("Expected ':' after property name"), i);
    }
    Ok((name, i + 1))
}

impl Agent {
    /// ParseJSON's value of `s`, its heap values rooted in `temps` (the
    /// caller truncates them), and, when `nodes` is asked for, the parse
    /// records of every value (the root's first).
    fn parse_json(&mut self, s: &[u16], nodes: &mut Option<Vec<Node>>) -> Result<Value, Fail> {
        let mut stack: Vec<Open> = Vec::new();
        let mut i = 0;
        let record = |nodes: &mut Option<Vec<Node>>, value: Value, span| {
            nodes.as_mut().map_or(0, |n| {
                n.push(Node {
                    value,
                    span,
                    kids: Vec::new(),
                });
                n.len() - 1
            })
        };
        loop {
            // A value.
            i = skip_space(s, i);
            let Some(&u) = s.get(i) else {
                return refuse(Fault::Token, i);
            };
            let start = i;
            let (mut value, mut node) = match u {
                0x7b | 0x5b => {
                    let array = u == 0x5b;
                    let o = if array {
                        self.array_of(&[])?
                    } else {
                        self.plain_object()?
                    };
                    self.vm.temps.push(o);
                    let node = record(nodes, o, None);
                    i = skip_space(s, i + 1);
                    let close = if array { 0x5d } else { 0x7d };
                    if s.get(i) == Some(&close) {
                        i += 1;
                        (o, node)
                    } else {
                        let key = if array {
                            PropertyKey::Index(0)
                        } else {
                            let (name, next) = member_name(s, i, "Expected property name or '}'")?;
                            i = next;
                            self.heap.atoms.key(&name)
                        };
                        stack.push(Open {
                            value: o,
                            array,
                            n: 0,
                            key,
                            node,
                        });
                        continue;
                    }
                }
                QUOTE => {
                    let (units, next) = scan_string(s, i)?;
                    i = next;
                    let v = self.new_string_units(&units)?;
                    self.vm.temps.push(v);
                    (v, record(nodes, v, Some((start, i))))
                }
                0x74 | 0x66 | 0x6e => {
                    let (word, v): (&[u8], Value) = match u {
                        0x74 => (b"true", Value::TRUE),
                        0x66 => (b"false", Value::FALSE),
                        _ => (b"null", Value::NULL),
                    };
                    for (k, &w) in word.iter().enumerate() {
                        if s.get(i + k) != Some(&u16::from(w)) {
                            return refuse(Fault::Token, i + k);
                        }
                    }
                    i += word.len();
                    (v, record(nodes, v, Some((start, i))))
                }
                u if u == u16::from(b'-') || is_digit(u) => {
                    let end = scan_number(s, i)?;
                    let n = crate::interp::ops::string_to_number(s.get(i..end).unwrap_or(&[]));
                    i = end;
                    let v = Value::number(n);
                    (v, record(nodes, v, Some((start, i))))
                }
                _ => return refuse(Fault::Token, i),
            };
            // The value into its container, and each container it closes
            // into the one around it.
            loop {
                let Some(top) = stack.last_mut() else {
                    i = skip_space(s, i);
                    if i < s.len() {
                        return refuse(
                            Fault::Says("Unexpected non-whitespace character after JSON"),
                            i,
                        );
                    }
                    return Ok(value);
                };
                let (container, key, parent) = (top.value, top.key, top.node);
                self.create_data_property(container, key, value);
                if let Some(n) = nodes.as_mut() {
                    if let Some(p) = n.get_mut(parent) {
                        match p.kids.iter_mut().find(|(k, _)| *k == key) {
                            Some(kid) => kid.1 = node,
                            None => p.kids.push((key, node)),
                        }
                    }
                }
                i = skip_space(s, i);
                let array = top.array;
                let next = s.get(i).copied();
                if next == Some(u16::from(b',')) {
                    i = skip_space(s, i + 1);
                    if array {
                        top.n += 1;
                        top.key = PropertyKey::Index(top.n);
                    } else {
                        let (name, after) =
                            member_name(s, i, "Expected double-quoted property name")?;
                        i = after;
                        let key = self.heap.atoms.key(&name);
                        if let Some(top) = stack.last_mut() {
                            top.key = key;
                        }
                    }
                    break;
                }
                let close = if array { 0x5d } else { 0x7d };
                if next != Some(close) {
                    let wanted = if array {
                        "Expected ',' or ']' after array element"
                    } else {
                        "Expected ',' or '}' after property value"
                    };
                    return refuse(Fault::Says(wanted), i);
                }
                i += 1;
                value = container;
                node = parent;
                stack.pop();
            }
        }
    }

    /// IsArray, throwing for a revoked proxy as section 7.2.2 does.
    fn json_is_array(&mut self, v: Value) -> Res<bool> {
        let mut at = v;
        loop {
            let Some(c) = at.as_object().and_then(|o| self.heap.objects.get(o)) else {
                return Ok(false);
            };
            match c.kind() {
                ObjectKind::Array => return Ok(true),
                ObjectKind::Proxy => {
                    let target = c.internal().first().copied().unwrap_or(Value::NULL);
                    if target.is_null() {
                        return Err(self.type_error(
                            "Cannot perform 'IsArray' on a proxy that has been revoked",
                        ));
                    }
                    at = target;
                }
                _ => return Ok(false),
            }
        }
    }

    /// EnumerableOwnProperties(`o`, key) (section 7.3.24): its own
    /// enumerable string keys.
    fn enumerable_keys(&mut self, o: Value) -> Res<Vec<PropertyKey>> {
        let mut out = Vec::new();
        for key in self.own_property_keys(o)? {
            if matches!(key, PropertyKey::Symbol(_)) {
                continue;
            }
            let enumerable = match self.own_property(o, key)? {
                Some(OwnProperty::Data { enumerable, .. })
                | Some(OwnProperty::Accessor { enumerable, .. }) => enumerable,
                None => false,
            };
            if enumerable {
                out.push(key);
            }
        }
        Ok(out)
    }
}

/// One value of the reviver's walk (InternalizeJSONProperty, section
/// 25.5.1.1): its holder and name, the value, the context given to the
/// reviver, the parse record still matching it, and its elements' or
/// members' keys and how far through them it is.
struct Walk {
    holder: Value,
    name: PropertyKey,
    value: Value,
    context: Value,
    node: Option<usize>,
    keys: Vec<PropertyKey>,
    len: Option<f64>,
    next: usize,
}

enum Begun {
    Done(Value),
    Walk(Walk),
}

impl Agent {
    /// InternalizeJSONProperty's first steps for `holder[name]`: the
    /// reviver's result for a primitive, the walk of an object.
    fn begin_walk(
        &mut self,
        holder: Value,
        name: PropertyKey,
        node: Option<usize>,
        nodes: &[Node],
        source: &[u16],
        reviver: Value,
    ) -> Res<Begun> {
        let value = self.get_value(holder, name)?;
        self.vm.temps.push(value);
        let context = self.plain_object()?;
        self.vm.temps.push(context);
        let node = node.filter(|&n| {
            nodes
                .get(n)
                .is_some_and(|n| same_value(&self.heap, n.value, value))
        });
        if value.as_object().is_none() {
            if let Some((a, b)) = node.and_then(|n| nodes.get(n)).and_then(|n| n.span) {
                let text = self.new_string_units(source.get(a..b).unwrap_or(&[]))?;
                let key = self.name_key(Name::Str("source"));
                self.create_data_property(context, key, text);
            }
            let r = self.revive(reviver, holder, name, value, context);
            self.vm.temps.truncate(self.vm.temps.len() - 2);
            return r.map(Begun::Done);
        }
        let (keys, len) = if self.json_is_array(value)? {
            (Vec::new(), Some(self.length_of(value)?))
        } else {
            (self.enumerable_keys(value)?, None)
        };
        Ok(Begun::Walk(Walk {
            holder,
            name,
            value,
            context,
            node,
            keys,
            len,
            next: 0,
        }))
    }

    fn revive(
        &mut self,
        reviver: Value,
        holder: Value,
        name: PropertyKey,
        value: Value,
        context: Value,
    ) -> Res<Value> {
        let key = self.key_value(name)?;
        self.vm.temps.push(key);
        let r = self.call_value(reviver, holder, &[key, value, context]);
        self.vm.temps.pop();
        r
    }

    /// The reviver's result for `holder[name]` put back: deleted when
    /// undefined, else defined as a data property.
    fn put_revived(&mut self, holder: Value, name: PropertyKey, r: Value) -> Res<()> {
        let Some(o) = holder.as_object() else {
            return Ok(());
        };
        self.vm.temps.push(r);
        let done = if r.is_undefined() {
            self.o_delete(o, name).map(|_| ())
        } else {
            self.o_define_own_property(o, name, Descriptor::data(r))
                .map(|_| ())
        };
        self.vm.temps.pop();
        done
    }

    /// InternalizeJSONProperty of the whole text (section 25.5.1.1).
    fn internalize(
        &mut self,
        root: Value,
        nodes: &[Node],
        source: &[u16],
        reviver: Value,
    ) -> Res<Value> {
        let empty = self.heap.atoms.key(&[]);
        let top = (!nodes.is_empty()).then_some(0);
        let mut walks = match self.begin_walk(root, empty, top, nodes, source, reviver)? {
            Begun::Done(v) => return Ok(v),
            Begun::Walk(w) => vec![w],
        };
        loop {
            let Some(w) = walks.last_mut() else {
                return Ok(Value::UNDEFINED);
            };
            let child = match w.len {
                Some(len) if (w.next as f64) < len => Some(self.index_key(w.next as f64)),
                Some(_) => None,
                None => w.keys.get(w.next).copied(),
            };
            if let Some(name) = child {
                w.next += 1;
                let holder = w.value;
                let node = w
                    .node
                    .and_then(|n| nodes.get(n))
                    .and_then(|n| n.kids.iter().find(|(k, _)| *k == name))
                    .map(|&(_, kid)| kid);
                match self.begin_walk(holder, name, node, nodes, source, reviver)? {
                    Begun::Done(r) => self.put_revived(holder, name, r)?,
                    Begun::Walk(w) => walks.push(w),
                }
                continue;
            }
            let Some(w) = walks.pop() else {
                return Ok(Value::UNDEFINED);
            };
            let r = self.revive(reviver, w.holder, w.name, w.value, w.context);
            self.vm.temps.truncate(self.vm.temps.len() - 2);
            let r = r?;
            if walks.is_empty() {
                return Ok(r);
            }
            self.put_revived(w.holder, w.name, r)?;
        }
    }

    /// A refusal as the SyntaxError it is.
    fn json_error(&mut self, s: &[u16], f: Fail) -> Abrupt {
        match f {
            Fail::Syntax(r) => self.syntax_error(message(s, r)),
            Fail::Abrupt(a) => a,
        }
    }
}

/// `JSON.parse` (section 25.5.1).
fn parse(agent: &mut Agent, c: Call) -> Res<Value> {
    let s = agent.text_units(agent.arg(c, 0))?;
    let reviver = agent.arg(c, 1);
    let revive = agent.is_callable(reviver);
    let base = agent.vm.temps.len();
    let r = (|| {
        let mut nodes = revive.then(Vec::new);
        let value = match agent.parse_json(&s, &mut nodes) {
            Ok(v) => v,
            Err(f) => return Err(agent.json_error(&s, f)),
        };
        let Some(nodes) = nodes else {
            return Ok(value);
        };
        let root = agent.plain_object()?;
        agent.vm.temps.push(root);
        let empty = agent.heap.atoms.key(&[]);
        agent.create_data_property(root, empty, value);
        agent.internalize(root, &nodes, &s, reviver)
    })();
    agent.vm.temps.truncate(base);
    r
}

/// `JSON.rawJSON` (section 25.5.3).
fn raw_json(agent: &mut Agent, c: Call) -> Res<Value> {
    let s = agent.text_units(agent.arg(c, 0))?;
    let (Some(&first), Some(&last)) = (s.first(), s.last()) else {
        return Err(agent.syntax_error("Invalid value for JSON.rawJSON"));
    };
    let wrong = if is_space(first) || first == 0x7b || first == 0x5b {
        Some(0)
    } else if is_space(last) {
        Some(s.len() - 1)
    } else {
        None
    };
    if let Some(at) = wrong {
        return Err(agent.syntax_error(message(
            &s,
            Refusal {
                fault: Fault::Token,
                at,
            },
        )));
    }
    let base = agent.vm.temps.len();
    let r = (|| {
        if let Err(f) = agent.parse_json(&s, &mut None) {
            return Err(agent.json_error(&s, f));
        }
        let text = agent.new_string_units(&s)?;
        agent.vm.temps.push(text);
        let o = agent.alloc_object(ObjectCell::new(Value::NULL, ObjectKind::RawJson))?;
        agent.vm.temps.push(o);
        let key = agent.name_key(Name::Str("rawJSON"));
        agent.create_data_property(o, key, text);
        agent.freeze_value(o)?;
        Ok(o)
    })();
    agent.vm.temps.truncate(base);
    r
}

/// `JSON.isRawJSON` (section 25.5.2).
fn is_raw_json(agent: &mut Agent, c: Call) -> Res<Value> {
    let v = agent.arg(c, 0);
    Ok(Value::bool(
        v.as_object()
            .and_then(|o| agent.heap.objects.get(o))
            .is_some_and(|c| c.kind() == ObjectKind::RawJson),
    ))
}

/// QuoteJSONString (section 25.5.2.3), onto `out`.
fn quote(out: &mut Vec<u16>, s: &[u16]) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let escape = |out: &mut Vec<u16>, u: u16| {
        out.extend([BACKSLASH, u16::from(b'u')]);
        for shift in [12, 8, 4, 0] {
            out.push(u16::from(
                HEX.get(usize::from((u >> shift) & 15))
                    .copied()
                    .unwrap_or(b'0'),
            ));
        }
    };
    out.push(QUOTE);
    let mut i = 0;
    while let Some(&u) = s.get(i) {
        let short = match u {
            0x08 => Some(b'b'),
            0x09 => Some(b't'),
            0x0a => Some(b'n'),
            0x0c => Some(b'f'),
            0x0d => Some(b'r'),
            QUOTE => Some(b'"'),
            BACKSLASH => Some(b'\\'),
            _ => None,
        };
        if let Some(c) = short {
            out.extend([BACKSLASH, u16::from(c)]);
        } else if u < 0x20 {
            escape(out, u);
        } else if (0xd800..0xdc00).contains(&u) {
            match s.get(i + 1) {
                Some(&l) if (0xdc00..0xe000).contains(&l) => {
                    out.extend([u, l]);
                    i += 1;
                }
                _ => escape(out, u),
            }
        } else if (0xdc00..0xe000).contains(&u) {
            escape(out, u);
        } else {
            out.push(u);
        }
        i += 1;
    }
    out.push(QUOTE);
}

/// What SerializeJSONProperty's first steps make of a value.
enum Serial {
    /// Nothing: undefined, a function or a symbol.
    Nothing,
    /// The text, done.
    Text(Vec<u16>),
    /// An object or array, to serialize member by member.
    Object(Value),
}

/// An object or array being serialized: its members' keys (or length),
/// how far through them, whether one was written, the indent to go
/// back to, and the key it was reached by (for a cycle's message).
struct Frame {
    value: Value,
    array: bool,
    keys: Vec<PropertyKey>,
    len: f64,
    next: usize,
    any: bool,
    stepback: usize,
    name: PropertyKey,
}

/// JSON.stringify's state (section 25.5.2's JSON Serialization Record).
struct Serializer {
    replacer: Option<Value>,
    list: Option<Vec<PropertyKey>>,
    gap: Vec<u16>,
    indent: Vec<u16>,
    out: Vec<u16>,
    /// The objects being serialized, for the cycle check.
    open: std::collections::HashSet<u32>,
}

impl Agent {
    /// SerializeJSONProperty's steps before the value is written (section
    /// 25.5.2.2).
    fn serial(&mut self, st: &Serializer, holder: Value, key: PropertyKey) -> Res<Serial> {
        let base = self.vm.temps.len();
        let r = (|| {
            let mut value = self.get_value(holder, key)?;
            self.vm.temps.push(value);
            let is_bigint = matches!(
                value.as_reference(),
                Some((crate::value::RefKind::BigInt, _))
            );
            let mut key_string = None;
            if value.as_object().is_some() || is_bigint {
                let to_json = self.name_key(Name::Str("toJSON"));
                let f = self.get_value(value, to_json)?;
                if self.is_callable(f) {
                    let k = self.key_value(key)?;
                    self.vm.temps.push(k);
                    key_string = Some(k);
                    value = self.call_value(f, value, &[k])?;
                    self.vm.temps.push(value);
                }
            }
            if let Some(replacer) = st.replacer {
                let k = match key_string {
                    Some(k) => k,
                    None => {
                        let k = self.key_value(key)?;
                        self.vm.temps.push(k);
                        k
                    }
                };
                value = self.call_value(replacer, holder, &[k, value])?;
                self.vm.temps.push(value);
            }
            if let Some(o) = value.as_object() {
                let kind = self.heap.objects.get(o).map(|c| c.kind());
                let inner = self
                    .heap
                    .objects
                    .get(o)
                    .and_then(|c| c.internal().first().copied());
                match kind {
                    Some(ObjectKind::RawJson) => {
                        let key = self.name_key(Name::Str("rawJSON"));
                        let raw = self.get_value(value, key)?;
                        return Ok(Serial::Text(self.text_units(raw)?));
                    }
                    Some(ObjectKind::Number) => value = Value::number(self.number(value)?),
                    Some(ObjectKind::String) => {
                        let mut out = Vec::new();
                        quote(&mut out, &self.text_units(value)?);
                        return Ok(Serial::Text(out));
                    }
                    Some(ObjectKind::Boolean) => value = inner.unwrap_or(Value::FALSE),
                    _ => {}
                }
            }
            let word = |w: &str| Ok(Serial::Text(w.encode_utf16().collect()));
            if value.is_null() {
                return word("null");
            }
            if let Some(b) = value.as_bool() {
                return word(if b { "true" } else { "false" });
            }
            if let Some(n) = value.as_number() {
                if n.is_finite() {
                    return word(&crate::number::number_to_string(n));
                }
                return word("null");
            }
            if value.as_string().is_some() {
                let mut out = Vec::new();
                quote(&mut out, &self.text_units(value)?);
                return Ok(Serial::Text(out));
            }
            if matches!(
                value.as_reference(),
                Some((crate::value::RefKind::BigInt, _))
            ) {
                return Err(self.type_error("Do not know how to serialize a BigInt"));
            }
            if value.as_object().is_some() && !self.is_callable(value) {
                return Ok(Serial::Object(value));
            }
            Ok(Serial::Nothing)
        })();
        self.vm.temps.truncate(base);
        r
    }

    /// The constructor's name the cycle message gives `o`.
    fn constructor_name(&self, o: Value) -> String {
        let r = self.realm();
        let name = o
            .as_object()
            .and_then(|i| self.data_property(i, r.constructor_key))
            .and_then(|c| c.as_object())
            .and_then(|c| self.data_property(c, r.name_key))
            .and_then(|n| self.string_units(n))
            .map(|u| String::from_utf16_lossy(&u));
        match name {
            Some(n) if !n.is_empty() => n,
            _ => "Object".to_owned(),
        }
    }

    /// The reference browser's message for a structure that comes back
    /// to `frames[from]` by `name` from the last frame.
    fn cycle_error(&mut self, frames: &[Frame], from: usize, name: PropertyKey) -> Abrupt {
        const FIRST: usize = 2;
        const LAST: usize = 1;
        let step = |agent: &Agent, holder: &Frame, key: PropertyKey| {
            if holder.array {
                format!("index {}", agent.key_display(key))
            } else {
                format!("property '{}'", agent.key_display(key))
            }
        };
        let chain = frames.get(from..).unwrap_or(&[]);
        let mut text = String::from("Converting circular structure to JSON");
        let start = chain.first().map_or(Value::UNDEFINED, |f| f.value);
        text.push_str(&format!(
            "\n    --> starting at object with constructor '{}'",
            self.constructor_name(start)
        ));
        let links: Vec<String> = chain
            .windows(2)
            .filter_map(|w| match w {
                [holder, f] => Some(format!(
                    "\n    |     {} -> object with constructor '{}'",
                    step(self, holder, f.name),
                    self.constructor_name(f.value)
                )),
                _ => None,
            })
            .collect();
        if links.len() <= FIRST + LAST {
            links.iter().for_each(|l| text.push_str(l));
        } else {
            links.iter().take(FIRST).for_each(|l| text.push_str(l));
            text.push_str("\n    |     ...");
            links
                .iter()
                .skip(links.len() - LAST)
                .for_each(|l| text.push_str(l));
        }
        if let Some(last) = chain.last() {
            text.push_str(&format!(
                "\n    --- {} closes the circle",
                step(self, last, name)
            ));
        }
        self.type_error(text)
    }

    /// SerializeJSONObject's and SerializeJSONArray's opening (sections
    /// 25.5.2.5 and 25.5.2.6).
    fn open_frame(
        &mut self,
        st: &mut Serializer,
        frames: &mut Vec<Frame>,
        value: Value,
        name: PropertyKey,
    ) -> Res<()> {
        let o = value.as_object().unwrap_or(u32::MAX);
        if st.open.contains(&o) {
            let from = frames.iter().position(|f| f.value == value).unwrap_or(0);
            return Err(self.cycle_error(frames, from, name));
        }
        st.open.insert(o);
        let array = self.json_is_array(value)?;
        let (keys, len) = if array {
            (Vec::new(), self.length_of(value)?)
        } else {
            match &st.list {
                Some(list) => (list.clone(), 0.0),
                None => (self.enumerable_keys(value)?, 0.0),
            }
        };
        st.out.push(u16::from(if array { b'[' } else { b'{' }));
        let stepback = st.indent.len();
        st.indent.extend_from_slice(&st.gap);
        self.vm.temps.push(value);
        frames.push(Frame {
            value,
            array,
            keys,
            len,
            next: 0,
            any: false,
            stepback,
            name,
        });
        Ok(())
    }

    /// The separator before a member: a comma after the first, and a
    /// newline and the indent when there is a gap.
    fn separate(st: &mut Serializer, any: bool) {
        if any {
            st.out.push(u16::from(b','));
        }
        if !st.gap.is_empty() {
            st.out.push(0x0a);
            st.out.extend_from_slice(&st.indent);
        }
    }

    /// SerializeJSONProperty of the wrapper's "" (section 25.5.2.2), all
    /// of it; `None` for undefined.
    fn serialize(&mut self, st: &mut Serializer, wrapper: Value) -> Res<Option<Vec<u16>>> {
        let empty = self.heap.atoms.key(&[]);
        let mut frames: Vec<Frame> = Vec::new();
        match self.serial(st, wrapper, empty)? {
            Serial::Nothing => return Ok(None),
            Serial::Text(t) => return Ok(Some(t)),
            Serial::Object(o) => self.open_frame(st, &mut frames, o, empty)?,
        }
        loop {
            let Some(f) = frames.last_mut() else {
                return Ok(Some(std::mem::take(&mut st.out)));
            };
            let key = if f.array {
                ((f.next as f64) < f.len).then(|| self.index_key(f.next as f64))
            } else {
                f.keys.get(f.next).copied()
            };
            let (holder, array, any) = (f.value, f.array, f.any);
            let Some(key) = key else {
                if any && !st.gap.is_empty() {
                    st.out.push(0x0a);
                    let back = st.indent.get(..f.stepback).unwrap_or(&[]).to_vec();
                    st.out.extend_from_slice(&back);
                }
                st.out.push(u16::from(if array { b']' } else { b'}' }));
                st.indent.truncate(f.stepback);
                if let Some(o) = holder.as_object() {
                    st.open.remove(&o);
                }
                frames.pop();
                self.vm.temps.pop();
                continue;
            };
            f.next += 1;
            let serial = self.serial(st, holder, key)?;
            let lead = |agent: &mut Agent, st: &mut Serializer, frames: &mut Vec<Frame>| {
                Agent::separate(st, any);
                if let Some(f) = frames.last_mut() {
                    f.any = true;
                }
                if !array {
                    let units = match key {
                        PropertyKey::Index(i) => i.to_string().encode_utf16().collect(),
                        PropertyKey::Atom(a) => agent.heap.atoms.units(a).to_vec(),
                        PropertyKey::Symbol(_) => Vec::new(),
                    };
                    quote(&mut st.out, &units);
                    st.out.push(u16::from(b':'));
                    if !st.gap.is_empty() {
                        st.out.push(0x20);
                    }
                }
            };
            match serial {
                Serial::Nothing if !array => {}
                Serial::Nothing => {
                    lead(self, st, &mut frames);
                    st.out.extend("null".encode_utf16());
                }
                Serial::Text(t) => {
                    lead(self, st, &mut frames);
                    st.out.extend_from_slice(&t);
                }
                Serial::Object(o) => {
                    lead(self, st, &mut frames);
                    self.vm.temps.push(o);
                    let r = self.open_frame(st, &mut frames, o, key);
                    self.vm.temps.pop();
                    r?;
                }
            }
        }
    }

    /// The property list a replacer array gives (section 25.5.2's step
    /// 4.b), first occurrences in order.
    fn replacer_list(&mut self, replacer: Value) -> Res<Vec<PropertyKey>> {
        let len = self.length_of(replacer)?;
        let mut list = Vec::new();
        let mut k = 0.0;
        while k < len {
            let key = self.index_key(k);
            let v = self.get_value(replacer, key)?;
            k += 1.0;
            let wrapper = v
                .as_object()
                .and_then(|o| self.heap.objects.get(o))
                .map(|c| c.kind());
            let item = if v.as_string().is_some()
                || v.as_number().is_some()
                || matches!(wrapper, Some(ObjectKind::String | ObjectKind::Number))
            {
                self.property_key(v)?
            } else {
                continue;
            };
            if !list.contains(&item) {
                list.push(item);
            }
        }
        Ok(list)
    }

    /// The gap `space` gives (section 25.5.2's steps 5 to 8).
    fn json_gap(&mut self, space: Value) -> Res<Vec<u16>> {
        let mut space = space;
        match space
            .as_object()
            .and_then(|o| self.heap.objects.get(o))
            .map(|c| c.kind())
        {
            Some(ObjectKind::Number) => space = Value::number(self.number(space)?),
            Some(ObjectKind::String) => {
                let u = self.text_units(space)?;
                space = self.new_string_units(&u)?;
            }
            _ => {}
        }
        if let Some(n) = space.as_number() {
            let n = n.trunc().clamp(0.0, 10.0);
            return Ok(vec![0x20; n as usize]);
        }
        if space.as_string().is_some() {
            let u = self.text_units(space)?;
            return Ok(u.into_iter().take(10).collect());
        }
        Ok(Vec::new())
    }
}

/// `JSON.stringify` (section 25.5.2).
fn stringify(agent: &mut Agent, c: Call) -> Res<Value> {
    let (value, replacer, space) = (agent.arg(c, 0), agent.arg(c, 1), agent.arg(c, 2));
    let mut st = Serializer {
        replacer: None,
        list: None,
        gap: Vec::new(),
        indent: Vec::new(),
        out: Vec::new(),
        open: std::collections::HashSet::new(),
    };
    if agent.is_callable(replacer) {
        st.replacer = Some(replacer);
    } else if replacer.as_object().is_some() && agent.json_is_array(replacer)? {
        st.list = Some(agent.replacer_list(replacer)?);
    }
    st.gap = agent.json_gap(space)?;
    let base = agent.vm.temps.len();
    let r = (|| {
        let wrapper = agent.plain_object()?;
        agent.vm.temps.push(wrapper);
        let empty = agent.heap.atoms.key(&[]);
        agent.create_data_property(wrapper, empty, value);
        match agent.serialize(&mut st, wrapper)? {
            Some(units) => agent.new_string_units(&units),
            None => Ok(Value::UNDEFINED),
        }
    })();
    agent.vm.temps.truncate(base);
    r
}
