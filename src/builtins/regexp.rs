//! `RegExp` (ECMA-262 section 22.2): the constructor, its prototype's
//! methods and accessors, the `@@match`, `@@matchAll`, `@@replace`,
//! `@@search` and `@@split` protocols, and `%RegExpStringIteratorPrototype%`.
//! A regexp object keeps its source and flags in its internal slots and
//! the index of its compiled program in the agent's cache, where each
//! source and flags is compiled once (design section 1.6). The messages
//! are the reference browser's (the J4.5 probe, Chrome 158).

use std::collections::HashMap;
use std::rc::Rc;

use super::{CONFIGURABLE, Call, Entry, HIDDEN, Name, What, Wk, entry, getter, method};
use crate::agent::Agent;
use crate::heap::ObjectCell;
use crate::interp::{Abrupt, ErrorType, Res};
use crate::object::{Descriptor, ObjectKind};
use crate::regexp::parse::parse_utf16;
use crate::regexp::vm::{Flags, Program, compile};
use crate::string::PropertyKey;
use crate::value::Value;

/// The compiled programs of an agent, each source and flags once.
#[derive(Default)]
pub(crate) struct Cache {
    programs: Vec<Rc<Program>>,
    index: HashMap<(Vec<u16>, u8), u32>,
}

/// `RegExp`, in the reference browser's order: Annex B's legacy statics
/// (each an accessor of its own, a no-op setter but `input`'s), `escape`
/// and `@@species`.
pub(crate) const REGEXP: &[Entry] = &[
    legacy("input", legacy_input, set_legacy_input),
    legacy("$_", legacy_input, set_legacy_input),
    legacy("lastMatch", legacy_last_match, set_legacy_none),
    legacy("$&", legacy_last_match, set_legacy_none),
    legacy("lastParen", legacy_last_paren, set_legacy_none),
    legacy("$+", legacy_last_paren, set_legacy_none),
    legacy("leftContext", legacy_left, set_legacy_none),
    legacy("$`", legacy_left, set_legacy_none),
    legacy("rightContext", legacy_right, set_legacy_none),
    legacy("$'", legacy_right, set_legacy_none),
    legacy("$1", legacy_1, set_legacy_none),
    legacy("$2", legacy_2, set_legacy_none),
    legacy("$3", legacy_3, set_legacy_none),
    legacy("$4", legacy_4, set_legacy_none),
    legacy("$5", legacy_5, set_legacy_none),
    legacy("$6", legacy_6, set_legacy_none),
    legacy("$7", legacy_7, set_legacy_none),
    legacy("$8", legacy_8, set_legacy_none),
    legacy("$9", legacy_9, set_legacy_none),
    method("escape", 1, escape),
    entry(Name::Sym(Wk::Species), What::Getter(species), CONFIGURABLE),
];

const fn legacy(name: &'static str, get: super::Native, set: super::Native) -> Entry {
    entry(Name::Str(name), What::Accessor(get, set), CONFIGURABLE)
}

/// `RegExp.prototype`, in the reference browser's order.
pub(crate) const REGEXP_PROTO: &[Entry] = &[
    super::CONSTRUCTOR,
    method("exec", 1, exec),
    getter("dotAll", dot_all),
    getter("flags", flags),
    getter("global", global),
    getter("hasIndices", has_indices),
    getter("ignoreCase", ignore_case),
    getter("multiline", multiline),
    getter("source", source),
    getter("sticky", sticky),
    getter("unicode", unicode),
    getter("unicodeSets", unicode_sets),
    method("compile", 2, compile_method),
    method("toString", 0, to_string),
    method("test", 1, test),
    entry(Name::Sym(Wk::Match), What::Method(1, symbol_match), HIDDEN),
    entry(
        Name::Sym(Wk::MatchAll),
        What::Method(1, symbol_match_all),
        HIDDEN,
    ),
    entry(
        Name::Sym(Wk::Replace),
        What::Method(2, symbol_replace),
        HIDDEN,
    ),
    entry(
        Name::Sym(Wk::Search),
        What::Method(1, symbol_search),
        HIDDEN,
    ),
    entry(Name::Sym(Wk::Split), What::Method(2, symbol_split), HIDDEN),
];

/// `%RegExpStringIteratorPrototype%`.
pub(crate) const STRING_ITERATOR_PROTO: &[Entry] = &[
    method("next", 0, iterator_next),
    entry(
        Name::Sym(Wk::ToStringTag),
        What::Str("RegExp String Iterator"),
        CONFIGURABLE,
    ),
];

/// RegExpExec's TypeError for an `exec` that gives neither an object nor
/// null.
const EXEC_RESULT: &str = "RegExp exec method returned something other than an Object or null";

/// ToUint32 (section 7.1.7).
fn to_uint32(n: f64) -> u32 {
    if !n.is_finite() {
        return 0;
    }
    n.trunc().rem_euclid(4_294_967_296.0) as u32
}

// A regexp's internal slots.
const S_SOURCE: usize = 0;
const S_FLAGS: usize = 1;
const S_PROGRAM: usize = 2;

// The flags as bits, in the order `flags` lists them.
const D: u8 = 1;
const G: u8 = 2;
const I: u8 = 4;
const M: u8 = 8;
const S: u8 = 16;
const U: u8 = 32;
const V: u8 = 64;
const Y: u8 = 128;

/// Each flag's letter and bit, in `flags`'s order (`dgimsuvy`).
const LETTERS: [(u16, u8); 8] = [
    (b'd' as u16, D),
    (b'g' as u16, G),
    (b'i' as u16, I),
    (b'm' as u16, M),
    (b's' as u16, S),
    (b'u' as u16, U),
    (b'v' as u16, V),
    (b'y' as u16, Y),
];

/// The bits of a flags string, or `None` if it is not a valid one.
fn flag_bits(flags: &[u16]) -> Option<u8> {
    let mut bits = 0u8;
    for &f in flags {
        let (_, bit) = LETTERS.iter().find(|(l, _)| *l == f)?;
        if bits & bit != 0 {
            return None;
        }
        bits |= bit;
    }
    (bits & U == 0 || bits & V == 0).then_some(bits)
}

// The iterator's internal slots.
const IT_MATCHER: usize = 0;
const IT_STRING: usize = 1;
const IT_GLOBAL: usize = 2;
const IT_UNICODE: usize = 3;
const IT_DONE: usize = 4;

impl Agent {
    fn rslot(&self, o: Value, i: usize) -> Value {
        o.as_object()
            .and_then(|o| self.heap.objects.get(o))
            .and_then(|c| c.internal().get(i).copied())
            .unwrap_or(Value::UNDEFINED)
    }

    fn set_rslot(&mut self, o: Value, i: usize, v: Value) {
        if let Some(cell) = o.as_object().and_then(|o| self.heap.objects.get_mut(o)) {
            cell.set_internal(i, v);
        }
    }

    fn is_regexp_object(&self, v: Value) -> bool {
        self.kind_of(v) == Some(ObjectKind::RegExp)
    }

    /// IsRegExp (section 7.2.6).
    pub(crate) fn is_regexp_value(&mut self, v: Value) -> Res<bool> {
        if v.as_object().is_none() {
            return Ok(false);
        }
        let key = self.well_known_key(Wk::Match);
        let m = self.get_value(v, key)?;
        if !m.is_undefined() {
            return Ok(self.to_boolean(m));
        }
        Ok(self.is_regexp_object(v))
    }

    /// A regexp's flag bits.
    fn regexp_bits(&self, r: Value) -> u8 {
        self.rslot(r, S_FLAGS)
            .as_int()
            .and_then(|b| u8::try_from(b).ok())
            .unwrap_or(0)
    }

    /// The compiled program of `source` and `bits`, from the cache or
    /// compiled now; a SyntaxError for a pattern that does not parse.
    fn regexp_program(&mut self, source: &[u16], bits: u8, flags_text: &[u16]) -> Res<u32> {
        let key = (source.to_vec(), bits);
        if let Some(&i) = self.vm.regexps.index.get(&key) {
            return Ok(i);
        }
        let unicode = bits & (U | V) != 0;
        let stack = crate::stack::StackGuard::new(crate::stack::DEFAULT_STACK_BUDGET);
        let parsed = parse_utf16(source, unicode, bits & V != 0, bits & I != 0, stack);
        let shown = |a: &Agent, m: &str| {
            let _ = a;
            format!(
                "Invalid regular expression: /{}/{}: {m}",
                String::from_utf16_lossy(source),
                String::from_utf16_lossy(flags_text)
            )
        };
        let pattern = match parsed {
            Ok(p) => p,
            Err(m) => {
                let text = shown(self, &m);
                return Err(self.throw_error(ErrorType::SyntaxError, text));
            }
        };
        let flags = Flags {
            ignore_case: bits & I != 0,
            multiline: bits & M != 0,
            dot_all: bits & S != 0,
            unicode,
        };
        let program = match compile(&pattern, flags) {
            Ok(p) => p,
            Err(m) => {
                let text = shown(self, &m);
                return Err(self.throw_error(ErrorType::SyntaxError, text));
            }
        };
        let i = self.vm.regexps.programs.len() as u32;
        self.vm.regexps.programs.push(Rc::new(program));
        self.vm.regexps.index.insert(key, i);
        Ok(i)
    }

    /// RegExpInitialize (section 22.2.3.3): `o` given its source, flags
    /// and program, and `lastIndex` 0.
    fn regexp_initialize(&mut self, o: Value, pattern: Value, flags: Value) -> Res<Value> {
        let p = if pattern.is_undefined() {
            Vec::new()
        } else {
            self.text_units(pattern)?
        };
        let f = if flags.is_undefined() {
            Vec::new()
        } else {
            self.text_units(flags)?
        };
        let Some(bits) = flag_bits(&f) else {
            return Err(self.throw_error(
                ErrorType::SyntaxError,
                format!(
                    "Invalid flags supplied to RegExp constructor '{}'",
                    String::from_utf16_lossy(&f)
                ),
            ));
        };
        let program = self.regexp_program(&p, bits, &f)?;
        let source = self.new_string_units(&p)?;
        self.set_rslot(o, S_SOURCE, source);
        self.set_rslot(o, S_FLAGS, Value::int(i32::from(bits)));
        self.set_rslot(o, S_PROGRAM, Value::int(program as i32));
        let key = self.name_key(Name::Str("lastIndex"));
        self.put(o, key, Value::int(0))?;
        Ok(o)
    }

    /// RegExpAlloc (section 22.2.3.2): an object of `new_target`'s
    /// prototype with a `lastIndex` of its own.
    fn regexp_alloc(&mut self, new_target: Value) -> Res<Value> {
        let proto = self.prototype_from(new_target, |r| r.intrinsics.regexp_prototype)?;
        let o = self.alloc_object(ObjectCell::with_internal(
            proto,
            ObjectKind::RegExp,
            vec![Value::UNDEFINED, Value::int(0), Value::int(-1)],
        ))?;
        let key = self.name_key(Name::Str("lastIndex"));
        if let Some(i) = o.as_object() {
            self.heap.define_own_property(
                i,
                key,
                Descriptor {
                    value: Some(Value::int(0)),
                    writable: Some(true),
                    enumerable: Some(false),
                    configurable: Some(false),
                    ..Descriptor::default()
                },
            );
        }
        Ok(o)
    }

    /// RegExpCreate (section 22.2.3.1).
    pub(crate) fn regexp_create(&mut self, pattern: Value, flags: Value) -> Res<Value> {
        let ctor = self.realm().intrinsics.regexp;
        // The caller's pattern and flags may be fresh strings that only
        // this call holds: rooted across the allocation.
        let base = self.vm.temps.len();
        self.vm.temps.extend([pattern, flags]);
        let r = self.regexp_alloc(ctor).and_then(|o| {
            self.vm.temps.push(o);
            self.regexp_initialize(o, pattern, flags)
        });
        self.vm.temps.truncate(base);
        r
    }

    /// A regexp literal's object: a new one each time it is evaluated,
    /// its program compiled once.
    #[inline(never)]
    pub(crate) fn regexp_literal(&mut self, body: Value, flags: Value) -> Res<Value> {
        self.vm.temps.extend([body, flags]);
        let r = self.regexp_create(body, flags);
        let n = self.vm.temps.len();
        self.vm.temps.truncate(n.saturating_sub(2));
        r
    }

    fn last_index_key(&mut self) -> PropertyKey {
        self.name_key(Name::Str("lastIndex"))
    }

    fn set_last_index(&mut self, r: Value, v: f64) -> Res<()> {
        let key = self.last_index_key();
        self.put(r, key, Value::number(v))
    }

    /// RegExpBuiltinExec (section 22.2.7.2): the match array, or null.
    fn regexp_builtin_exec(&mut self, r: Value, s: Value) -> Res<Value> {
        let key = self.last_index_key();
        let li = self.get_value(r, key)?;
        let mut last_index = self.length_value(li)?;
        let bits = self.regexp_bits(r);
        let (global, sticky, has_indices) = (bits & G != 0, bits & Y != 0, bits & D != 0);
        if !global && !sticky {
            last_index = 0.0;
        }
        let text = self.string_units(s).unwrap_or_default();
        if last_index > text.len() as f64 {
            if global || sticky {
                self.set_last_index(r, 0.0)?;
            }
            return Ok(Value::NULL);
        }
        let index = self.rslot(r, S_PROGRAM).as_int().unwrap_or(-1);
        let Some(program) = usize::try_from(index)
            .ok()
            .and_then(|i| self.vm.regexps.programs.get(i))
            .cloned()
        else {
            return Ok(Value::NULL);
        };
        let mut start = last_index as usize;
        // Unicode mode at the trail of a surrogate pair starts at its lead,
        // as the reference browser does (the spec reads a lone trail).
        if bits & (U | V) != 0
            && start > 0
            && text
                .get(start)
                .is_some_and(|&u| (0xDC00..0xE000).contains(&u))
            && text
                .get(start - 1)
                .is_some_and(|&u| (0xD800..0xDC00).contains(&u))
        {
            start -= 1;
        }
        let result = {
            let steps = &mut self.vm.steps;
            let interrupt = &mut self.vm.interrupt;
            let mut tick = || {
                *steps -= 1;
                if *steps == 0 {
                    *steps = crate::interp::STEPS_PER_CHECK;
                    if let Some(hook) = interrupt.as_mut() {
                        return hook();
                    }
                }
                false
            };
            program.exec(&text, start, sticky, &mut tick)
        };
        let caps = match result {
            Err(_) => return Err(Abrupt::Terminate),
            Ok(None) => {
                if global || sticky {
                    self.set_last_index(r, 0.0)?;
                }
                return Ok(Value::NULL);
            }
            Ok(Some(caps)) => caps,
        };
        let (m_start, m_end) = (
            caps.first().copied().unwrap_or(0) as usize,
            caps.get(1).copied().unwrap_or(0) as usize,
        );
        if global || sticky {
            self.set_last_index(r, m_end as f64)?;
        }
        self.record_last_match(s, &caps);
        self.match_array(&program, &text, s, &caps, m_start, has_indices)
    }

    /// The array of a match: its captures, `index`, `input`, `groups`
    /// and, with `d`, `indices`.
    fn match_array(
        &mut self,
        program: &Program,
        text: &[u16],
        s: Value,
        caps: &[i64],
        m_start: usize,
        has_indices: bool,
    ) -> Res<Value> {
        let base = self.vm.temps.len();
        let r = (|| {
            let n = program.groups as usize;
            let a = self.array_of(&[])?;
            self.vm.temps.push(a);
            let key = self.name_key(Name::Str("index"));
            self.define_value(a, key, Value::number(m_start as f64))?;
            let key = self.name_key(Name::Str("input"));
            self.define_value(a, key, s)?;
            // The captures, as strings or undefined.
            let mut values = Vec::with_capacity(n + 1);
            for g in 0..=n {
                let (st, en) = (
                    caps.get(g * 2).copied().unwrap_or(-1),
                    caps.get(g * 2 + 1).copied().unwrap_or(-1),
                );
                let v = if st < 0 || en < 0 {
                    Value::UNDEFINED
                } else {
                    let part = text.get(st as usize..en as usize).unwrap_or(&[]);
                    self.new_string_units(part)?
                };
                self.vm.temps.push(v);
                values.push(v);
            }
            self.create_data_property(
                a,
                PropertyKey::Index(0),
                values.first().copied().unwrap_or(Value::UNDEFINED),
            );
            let named = program.names.iter().any(Option::is_some);
            let groups = if named {
                let g = self.alloc_object(ObjectCell::new(Value::NULL, ObjectKind::Ordinary))?;
                self.vm.temps.push(g);
                g
            } else {
                Value::UNDEFINED
            };
            let key = self.name_key(Name::Str("groups"));
            self.define_value(a, key, groups)?;
            for g in 1..=n {
                let v = values.get(g).copied().unwrap_or(Value::UNDEFINED);
                self.create_data_property(a, PropertyKey::Index(g as u32), v);
                if let Some(Some(name)) = program.names.get(g) {
                    let k = self.new_string(name)?;
                    let k = self.property_key(k)?;
                    // A name that stands twice keeps the group that took
                    // part.
                    let current = self.get_value(groups, k)?;
                    if current.is_undefined() || !v.is_undefined() {
                        self.define_value(groups, k, v)?;
                    }
                }
            }
            if has_indices {
                let indices = self.match_indices(program, caps)?;
                let key = self.name_key(Name::Str("indices"));
                self.define_value(a, key, indices)?;
            }
            Ok(a)
        })();
        self.vm.temps.truncate(base);
        r
    }

    /// MakeMatchIndicesIndexPairArray (section 22.2.7.8).
    fn match_indices(&mut self, program: &Program, caps: &[i64]) -> Res<Value> {
        let base = self.vm.temps.len();
        let r = (|| {
            let n = program.groups as usize;
            let a = self.array_of(&[])?;
            self.vm.temps.push(a);
            let mut pairs = Vec::with_capacity(n + 1);
            for g in 0..=n {
                let (st, en) = (
                    caps.get(g * 2).copied().unwrap_or(-1),
                    caps.get(g * 2 + 1).copied().unwrap_or(-1),
                );
                let v = if st < 0 || en < 0 {
                    Value::UNDEFINED
                } else {
                    self.array_of(&[Value::number(st as f64), Value::number(en as f64)])?
                };
                self.vm.temps.push(v);
                self.create_data_property(a, PropertyKey::Index(g as u32), v);
                pairs.push(v);
            }
            let named = program.names.iter().any(Option::is_some);
            let groups = if named {
                let g = self.alloc_object(ObjectCell::new(Value::NULL, ObjectKind::Ordinary))?;
                self.vm.temps.push(g);
                for k in 1..=n {
                    if let Some(Some(name)) = program.names.get(k) {
                        let v = pairs.get(k).copied().unwrap_or(Value::UNDEFINED);
                        let key = self.new_string(name)?;
                        let key = self.property_key(key)?;
                        let current = self.get_value(g, key)?;
                        if current.is_undefined() || !v.is_undefined() {
                            self.define_value(g, key, v)?;
                        }
                    }
                }
                g
            } else {
                Value::UNDEFINED
            };
            let key = self.name_key(Name::Str("groups"));
            self.define_value(a, key, groups)?;
            Ok(a)
        })();
        self.vm.temps.truncate(base);
        r
    }

    /// RegExpExec (section 22.2.7.1): `exec` if callable, else the
    /// built-in one.
    fn regexp_exec(&mut self, r: Value, s: Value) -> Res<Value> {
        let key = self.name_key(Name::Str("exec"));
        let exec = self.get_value(r, key)?;
        if self.is_callable(exec) {
            let result = self.call_value(exec, r, &[s])?;
            if result.as_object().is_none() && !result.is_null() {
                return Err(self.type_error(EXEC_RESULT));
            }
            return Ok(result);
        }
        if !self.is_regexp_object(r) {
            return Err(self.incompatible("RegExp.prototype.exec", r));
        }
        self.regexp_builtin_exec(r, s)
    }

    /// `this` as an object, or `method`'s TypeError.
    fn regexp_this(&mut self, this: Value, method: &str) -> Res<Value> {
        if this.as_object().is_some() {
            Ok(this)
        } else {
            Err(self.incompatible(method, this))
        }
    }

    /// AdvanceStringIndex (section 22.2.7.3).
    fn advance_index(&self, s: &[u16], at: f64, unicode: bool) -> f64 {
        if !unicode || at + 1.0 >= s.len() as f64 {
            return at + 1.0;
        }
        let i = at as usize;
        let pair = s.get(i).is_some_and(|&u| (0xD800..0xDC00).contains(&u))
            && s.get(i + 1).is_some_and(|&u| (0xDC00..0xE000).contains(&u));
        at + if pair { 2.0 } else { 1.0 }
    }

    /// `ToString(Get(rx, "flags"))`.
    fn flags_of(&mut self, rx: Value) -> Res<Vec<u16>> {
        let key = self.name_key(Name::Str("flags"));
        let f = self.get_value(rx, key)?;
        self.text_units(f)
    }
}

/// `RegExp(pattern, flags)` (section 22.2.4.1).
pub(crate) fn construct(agent: &mut Agent, c: Call) -> Res<Value> {
    let (pattern, flags) = (agent.arg(c, 0), agent.arg(c, 1));
    let pattern_is_regexp = agent.is_regexp_value(pattern)?;
    let new_target = if c.new_target.is_undefined() {
        let callee = c.callee;
        if pattern_is_regexp && flags.is_undefined() {
            let key = agent.realm().constructor_key;
            let pc = agent.get_value(pattern, key)?;
            if pc == callee {
                return Ok(pattern);
            }
        }
        callee
    } else {
        c.new_target
    };
    let base = agent.vm.temps.len();
    let r = (|| {
        let (p, f) = if agent.is_regexp_object(pattern) {
            let p = agent.rslot(pattern, S_SOURCE);
            let f = if flags.is_undefined() {
                let bits = agent.regexp_bits(pattern);
                let letters: Vec<u16> = LETTERS
                    .iter()
                    .filter(|(_, b)| bits & b != 0)
                    .map(|(l, _)| *l)
                    .collect();
                agent.new_string_units(&letters)?
            } else {
                flags
            };
            (p, f)
        } else if pattern_is_regexp {
            let key = agent.name_key(Name::Str("source"));
            let p = agent.get_value(pattern, key)?;
            agent.vm.temps.push(p);
            let f = if flags.is_undefined() {
                let key = agent.name_key(Name::Str("flags"));
                agent.get_value(pattern, key)?
            } else {
                flags
            };
            (p, f)
        } else {
            (pattern, flags)
        };
        agent.vm.temps.extend([p, f]);
        let o = agent.regexp_alloc(new_target)?;
        agent.vm.temps.push(o);
        agent.regexp_initialize(o, p, f)
    })();
    agent.vm.temps.truncate(base);
    r
}

/// What a legacy static's getter reads of the last match.
#[derive(Clone, Copy)]
enum Legacy {
    Group(usize),
    LastMatch,
    LastParen,
    Left,
    Right,
}

// The legacy statics holder's internal slots: the input, the string of
// the last successful match, then each capture's start and end (-1 when
// it took no part), the whole match's first.
const L_INPUT: usize = 0;
const L_SUBJECT: usize = 1;
const L_SPANS: usize = 2;

impl Agent {
    /// Records a successful builtin exec on `s` in the running realm's
    /// legacy statics: every regexp's, a subclass's or another realm's
    /// object's alike, as the reference browser does (the J4.5c probe).
    fn record_last_match(&mut self, s: Value, caps: &[i64]) {
        let Some(o) = self.realm().intrinsics.regexp_statics.as_object() else {
            return;
        };
        // Through the heap, which counts the slots' bytes as they grow.
        self.heap.with_internal_slots(o, |slots| {
            slots.clear();
            slots.extend([s, s]);
            slots.extend(
                caps.iter()
                    .map(|&c| Value::int(i32::try_from(c).unwrap_or(-1))),
            );
        });
    }

    fn legacy_slots(&self) -> Vec<Value> {
        self.realm()
            .intrinsics
            .regexp_statics
            .as_object()
            .and_then(|o| self.heap.objects.get(o))
            .map(|c| c.internal().to_vec())
            .unwrap_or_default()
    }

    /// A legacy static's value: a part of the last match's string; the
    /// empty string before any match, past the last group or for a group
    /// that took no part.
    fn legacy_value(&mut self, which: Legacy) -> Res<Value> {
        let slots = self.legacy_slots();
        let text = slots
            .get(L_SUBJECT)
            .and_then(|&s| self.string_units(s))
            .unwrap_or_default();
        let spans: Vec<i32> = slots
            .get(L_SPANS..)
            .unwrap_or(&[])
            .iter()
            .map(|v| v.as_int().unwrap_or(-1))
            .collect();
        let span = |g: usize| -> Option<(usize, usize)> {
            let (s, e) = (*spans.get(g * 2)?, *spans.get(g * 2 + 1)?);
            (s >= 0 && e >= s).then_some((s as usize, e as usize))
        };
        let groups = (spans.len() / 2).saturating_sub(1);
        let part = match which {
            Legacy::Group(n) if n <= groups => span(n),
            Legacy::Group(_) => None,
            Legacy::LastMatch => span(0),
            // The highest-numbered group, whether or not it took part.
            Legacy::LastParen if groups > 0 => span(groups),
            Legacy::LastParen => None,
            Legacy::Left => span(0).map(|(s, _)| (0, s)),
            Legacy::Right => span(0).map(|(_, e)| (e, text.len())),
        };
        let units = part.and_then(|(s, e)| text.get(s..e)).unwrap_or(&[]);
        self.new_string_units(units)
    }
}

// The legacy statics' getters and setters (Annex B, as the reference
// browser has them: no check of the receiver).
fn legacy_input(agent: &mut Agent, _: Call) -> Res<Value> {
    match agent.legacy_slots().get(L_INPUT) {
        Some(&v) => Ok(v),
        None => agent.new_string_units(&[]),
    }
}

/// `RegExp.input = v`: the input becomes ToString(v) until the next match.
fn set_legacy_input(agent: &mut Agent, c: Call) -> Res<Value> {
    let v = agent.arg(c, 0);
    let units = agent.text_units(v)?;
    let s = agent.new_string_units(&units)?;
    if let Some(o) = agent.realm().intrinsics.regexp_statics.as_object() {
        agent
            .heap
            .with_internal_slots(o, |slots| match slots.get_mut(L_INPUT) {
                Some(slot) => *slot = s,
                None => slots.push(s),
            });
    }
    Ok(Value::UNDEFINED)
}

/// The setter of every legacy static but `input`: it does nothing.
fn set_legacy_none(_: &mut Agent, _: Call) -> Res<Value> {
    Ok(Value::UNDEFINED)
}

fn legacy_last_match(agent: &mut Agent, _: Call) -> Res<Value> {
    agent.legacy_value(Legacy::LastMatch)
}

fn legacy_last_paren(agent: &mut Agent, _: Call) -> Res<Value> {
    agent.legacy_value(Legacy::LastParen)
}

fn legacy_left(agent: &mut Agent, _: Call) -> Res<Value> {
    agent.legacy_value(Legacy::Left)
}

fn legacy_right(agent: &mut Agent, _: Call) -> Res<Value> {
    agent.legacy_value(Legacy::Right)
}

macro_rules! legacy_groups {
    ($($name:ident = $n:literal),*) => {
        $(fn $name(agent: &mut Agent, _: Call) -> Res<Value> {
            agent.legacy_value(Legacy::Group($n))
        })*
    };
}

legacy_groups!(
    legacy_1 = 1,
    legacy_2 = 2,
    legacy_3 = 3,
    legacy_4 = 4,
    legacy_5 = 5,
    legacy_6 = 6,
    legacy_7 = 7,
    legacy_8 = 8,
    legacy_9 = 9
);

/// `RegExp.prototype.compile` (Annex B): `this` made anew from a pattern
/// and flags, its `lastIndex` then set to 0. As in the reference browser,
/// a subclass's or another realm's regexp may be compiled.
fn compile_method(agent: &mut Agent, c: Call) -> Res<Value> {
    let o = c.this;
    if !agent.is_regexp_object(o) {
        return Err(agent.incompatible("RegExp.prototype.compile", o));
    }
    let (pattern, flags) = (agent.arg(c, 0), agent.arg(c, 1));
    let base = agent.vm.temps.len();
    let r = (|| {
        let (p, f) = if agent.is_regexp_object(pattern) {
            if !flags.is_undefined() {
                return Err(agent
                    .type_error("Cannot supply flags when constructing one RegExp from another"));
            }
            let bits = agent.regexp_bits(pattern);
            let letters: Vec<u16> = LETTERS
                .iter()
                .filter(|(_, b)| bits & b != 0)
                .map(|(l, _)| *l)
                .collect();
            let f = agent.new_string_units(&letters)?;
            (agent.rslot(pattern, S_SOURCE), f)
        } else {
            (pattern, flags)
        };
        agent.vm.temps.extend([p, f]);
        agent.regexp_initialize(o, p, f)
    })();
    agent.vm.temps.truncate(base);
    r
}

/// `RegExp.escape` (section 22.2.5.1): a string's every character that
/// could mean something in a pattern escaped, hexadecimal in lower case
/// as the reference browser writes it.
fn escape(agent: &mut Agent, c: Call) -> Res<Value> {
    let Some(units) = agent.string_units(agent.arg(c, 0)) else {
        return Err(agent.type_error("input argument must be a string"));
    };
    let mut out: Vec<u16> = Vec::with_capacity(units.len());
    let push = |out: &mut Vec<u16>, s: String| out.extend(s.encode_utf16());
    let mut i = 0;
    while let Some(&u) = units.get(i) {
        let pair = units
            .get(i + 1)
            .filter(|&&t| (0xD800..0xDC00).contains(&u) && (0xDC00..0xE000).contains(&t));
        let (cp, n) = match pair {
            Some(&t) => (
                0x10000 + ((u32::from(u) - 0xD800) << 10) + (u32::from(t) - 0xDC00),
                2,
            ),
            None => (u32::from(u), 1),
        };
        let ch = char::from_u32(cp);
        let ascii = |s: &str| ch.is_some_and(|c| c.is_ascii() && s.contains(c));
        let control = match cp {
            0x09 => Some('t'),
            0x0A => Some('n'),
            0x0B => Some('v'),
            0x0C => Some('f'),
            0x0D => Some('r'),
            _ => None,
        };
        if i == 0 && ch.is_some_and(|c| c.is_ascii_alphanumeric()) {
            push(&mut out, format!("\\x{cp:02x}"));
        } else if ascii("^$\\.*+?()[]{}|/") {
            out.push(u16::from(b'\\'));
            out.push(u);
        } else if let Some(e) = control {
            push(&mut out, format!("\\{e}"));
        } else if ascii(",-=<>#&!%:;@~'`\"")
            || ch.is_none_or(|c| {
                crate::unicode::is_white_space(c) || crate::unicode::is_line_terminator(c)
            })
        {
            if cp <= 0xFF {
                push(&mut out, format!("\\x{cp:02x}"));
            } else {
                for &cu in units.get(i..i + n).unwrap_or(&[]) {
                    push(&mut out, format!("\\u{cu:04x}"));
                }
            }
        } else {
            out.extend_from_slice(units.get(i..i + n).unwrap_or(&[]));
        }
        i += n;
    }
    agent.new_string_units(&out)
}

fn species(_: &mut Agent, c: Call) -> Res<Value> {
    Ok(c.this)
}

/// `RegExp.prototype.exec` (section 22.2.6.2).
fn exec(agent: &mut Agent, c: Call) -> Res<Value> {
    if !agent.is_regexp_object(c.this) {
        return Err(agent.incompatible("RegExp.prototype.exec", c.this));
    }
    let s = agent.arg(c, 0);
    let s = agent.string_value(s)?;
    agent.vm.temps.push(s);
    let r = agent.regexp_builtin_exec(c.this, s);
    agent.vm.temps.pop();
    r
}

/// A flag accessor: the flag of a regexp, undefined on the prototype.
fn flag_getter(agent: &mut Agent, c: Call, bit: u8, name: &str) -> Res<Value> {
    if c.this.as_object().is_none() {
        let text = agent.display(c.this);
        return Err(agent.type_error(format!(
            "RegExp.prototype.{name} getter called on non-object {text}"
        )));
    }
    if !agent.is_regexp_object(c.this) {
        if c.this == agent.realm().intrinsics.regexp_prototype {
            return Ok(Value::UNDEFINED);
        }
        return Err(agent.type_error(format!(
            "RegExp.prototype.{name} getter called on non-RegExp object"
        )));
    }
    Ok(Value::bool(agent.regexp_bits(c.this) & bit != 0))
}

fn dot_all(agent: &mut Agent, c: Call) -> Res<Value> {
    flag_getter(agent, c, S, "dotAll")
}

fn global(agent: &mut Agent, c: Call) -> Res<Value> {
    flag_getter(agent, c, G, "global")
}

fn has_indices(agent: &mut Agent, c: Call) -> Res<Value> {
    flag_getter(agent, c, D, "hasIndices")
}

fn ignore_case(agent: &mut Agent, c: Call) -> Res<Value> {
    flag_getter(agent, c, I, "ignoreCase")
}

fn multiline(agent: &mut Agent, c: Call) -> Res<Value> {
    flag_getter(agent, c, M, "multiline")
}

fn sticky(agent: &mut Agent, c: Call) -> Res<Value> {
    flag_getter(agent, c, Y, "sticky")
}

fn unicode(agent: &mut Agent, c: Call) -> Res<Value> {
    flag_getter(agent, c, U, "unicode")
}

fn unicode_sets(agent: &mut Agent, c: Call) -> Res<Value> {
    flag_getter(agent, c, V, "unicodeSets")
}

/// `get RegExp.prototype.flags` (section 22.2.6.4): each flag read
/// through its accessor, in `dgimsuvy` order.
fn flags(agent: &mut Agent, c: Call) -> Res<Value> {
    if c.this.as_object().is_none() {
        let text = agent.display(c.this);
        return Err(agent.type_error(format!(
            "RegExp.prototype.flags getter called on non-object {text}"
        )));
    }
    let mut out = Vec::new();
    for (letter, name) in [
        ('d', "hasIndices"),
        ('g', "global"),
        ('i', "ignoreCase"),
        ('m', "multiline"),
        ('s', "dotAll"),
        ('u', "unicode"),
        ('v', "unicodeSets"),
        ('y', "sticky"),
    ] {
        let key = agent.name_key(Name::Str(name));
        let v = agent.get_value(c.this, key)?;
        if agent.to_boolean(v) {
            out.push(letter as u16);
        }
    }
    agent.new_string_units(&out)
}

/// EscapeRegExpPattern (section 22.2.6.13.1): `/` and line terminators
/// escaped, so that the source reads back as the same literal.
fn escape_pattern(src: &[u16]) -> Vec<u16> {
    if src.is_empty() {
        return "(?:)".encode_utf16().collect();
    }
    let mut out = Vec::with_capacity(src.len());
    let mut in_class = false;
    let mut i = 0;
    while let Some(&u) = src.get(i) {
        match u {
            0x5C => {
                out.push(u);
                if let Some(&next) = src.get(i + 1) {
                    match next {
                        0x0A => out.extend("n".encode_utf16()),
                        0x0D => out.extend("r".encode_utf16()),
                        0x2028 => out.extend("u2028".encode_utf16()),
                        0x2029 => out.extend("u2029".encode_utf16()),
                        _ => out.push(next),
                    }
                    i += 1;
                }
            }
            0x2F if !in_class => out.extend("\\/".encode_utf16()),
            0x5B => {
                in_class = true;
                out.push(u);
            }
            0x5D => {
                in_class = false;
                out.push(u);
            }
            0x0A => out.extend("\\n".encode_utf16()),
            0x0D => out.extend("\\r".encode_utf16()),
            0x2028 => out.extend("\\u2028".encode_utf16()),
            0x2029 => out.extend("\\u2029".encode_utf16()),
            _ => out.push(u),
        }
        i += 1;
    }
    out
}

/// `get RegExp.prototype.source` (section 22.2.6.13).
fn source(agent: &mut Agent, c: Call) -> Res<Value> {
    if c.this.as_object().is_none() {
        let text = agent.display(c.this);
        return Err(agent.type_error(format!(
            "RegExp.prototype.source getter called on non-object {text}"
        )));
    }
    if !agent.is_regexp_object(c.this) {
        if c.this == agent.realm().intrinsics.regexp_prototype {
            return agent.new_string("(?:)");
        }
        return Err(agent.type_error("RegExp.prototype.source getter called on non-RegExp object"));
    }
    let src = agent.rslot(c.this, S_SOURCE);
    let units = agent.string_units(src).unwrap_or_default();
    agent.new_string_units(&escape_pattern(&units))
}

/// `RegExp.prototype.test` (section 22.2.6.16).
fn test(agent: &mut Agent, c: Call) -> Res<Value> {
    let r = agent.regexp_this(c.this, "RegExp.prototype.test")?;
    let s = agent.arg(c, 0);
    let s = agent.string_value(s)?;
    agent.vm.temps.push(s);
    let m = agent.regexp_exec(r, s);
    agent.vm.temps.pop();
    Ok(Value::bool(!m?.is_null()))
}

/// `RegExp.prototype.toString` (section 22.2.6.17).
fn to_string(agent: &mut Agent, c: Call) -> Res<Value> {
    let r = agent.regexp_this(c.this, "RegExp.prototype.toString")?;
    let key = agent.name_key(Name::Str("source"));
    let p = agent.get_value(r, key)?;
    let mut out = vec![u16::from(b'/')];
    out.extend(agent.text_units(p)?);
    out.push(u16::from(b'/'));
    out.extend(agent.flags_of(r)?);
    agent.new_string_units(&out)
}

/// `RegExp.prototype[@@match]` (section 22.2.6.8).
fn symbol_match(agent: &mut Agent, c: Call) -> Res<Value> {
    let rx = agent.regexp_this(c.this, "RegExp.prototype.@@match")?;
    let s = agent.arg(c, 0);
    let s = agent.string_value(s)?;
    let base = agent.vm.temps.len();
    agent.vm.temps.push(s);
    let r = (|| {
        let flags = agent.flags_of(rx)?;
        if !flags.contains(&u16::from(b'g')) {
            return agent.regexp_exec(rx, s);
        }
        let full_unicode = flags.contains(&u16::from(b'u')) || flags.contains(&u16::from(b'v'));
        agent.set_last_index(rx, 0.0)?;
        let a = agent.array_of(&[])?;
        agent.vm.temps.push(a);
        let text = agent.string_units(s).unwrap_or_default();
        let mut n = 0u32;
        loop {
            let result = agent.regexp_exec(rx, s)?;
            if result.is_null() {
                return Ok(if n == 0 { Value::NULL } else { a });
            }
            agent.vm.temps.push(result);
            let m = agent.get_value(result, PropertyKey::Index(0))?;
            let m = agent.string_value(m)?;
            agent.vm.temps.pop();
            agent.create_data_property(a, PropertyKey::Index(n), m);
            n += 1;
            if agent.string_units(m).is_some_and(|u| u.is_empty()) {
                let key = agent.last_index_key();
                let li = agent.get_value(rx, key)?;
                let this_index = agent.length_value(li)?;
                let next = agent.advance_index(&text, this_index, full_unicode);
                agent.set_last_index(rx, next)?;
            }
        }
    })();
    agent.vm.temps.truncate(base);
    r
}

/// `RegExp.prototype[@@matchAll]` (section 22.2.6.9).
fn symbol_match_all(agent: &mut Agent, c: Call) -> Res<Value> {
    let r = agent.regexp_this(c.this, "RegExp.prototype.@@matchAll")?;
    let s = agent.arg(c, 0);
    let s = agent.string_value(s)?;
    let base = agent.vm.temps.len();
    agent.vm.temps.push(s);
    let out = (|| {
        let default = agent.realm().intrinsics.regexp;
        let ctor = agent.species_constructor(r, default)?;
        agent.vm.temps.push(ctor);
        let flags = agent.flags_of(r)?;
        let f = agent.new_string_units(&flags)?;
        agent.vm.temps.push(f);
        let matcher = agent.construct_value(ctor, &[r, f], ctor)?;
        agent.vm.temps.push(matcher);
        let key = agent.last_index_key();
        let li = agent.get_value(r, key)?;
        let last_index = agent.length_value(li)?;
        agent.set_last_index(matcher, last_index)?;
        let global = flags.contains(&u16::from(b'g'));
        let full_unicode = flags.contains(&u16::from(b'u')) || flags.contains(&u16::from(b'v'));
        let proto = agent.realm().intrinsics.regexp_string_iterator_prototype;
        agent.alloc_object(ObjectCell::with_internal(
            proto,
            ObjectKind::RegExpStringIterator,
            vec![
                matcher,
                s,
                Value::bool(global),
                Value::bool(full_unicode),
                Value::FALSE,
            ],
        ))
    })();
    agent.vm.temps.truncate(base);
    out
}

/// `%RegExpStringIteratorPrototype%.next` (section 22.2.9.2.1).
fn iterator_next(agent: &mut Agent, c: Call) -> Res<Value> {
    let it = c.this;
    if agent.kind_of(it) != Some(ObjectKind::RegExpStringIterator) {
        return Err(agent.incompatible("%RegExpStringIterator%.prototype.next", it));
    }
    if agent.rslot(it, IT_DONE) == Value::TRUE {
        return agent.iter_result(Value::UNDEFINED, true);
    }
    let (rx, s) = (agent.rslot(it, IT_MATCHER), agent.rslot(it, IT_STRING));
    let global = agent.rslot(it, IT_GLOBAL) == Value::TRUE;
    let full_unicode = agent.rslot(it, IT_UNICODE) == Value::TRUE;
    let m = agent.regexp_exec(rx, s)?;
    if m.is_null() {
        agent.set_rslot(it, IT_DONE, Value::TRUE);
        return agent.iter_result(Value::UNDEFINED, true);
    }
    if !global {
        agent.set_rslot(it, IT_DONE, Value::TRUE);
        return agent.iter_result(m, false);
    }
    agent.vm.temps.push(m);
    let r = (|| {
        let first = agent.get_value(m, PropertyKey::Index(0))?;
        let first = agent.string_value(first)?;
        if agent.string_units(first).is_some_and(|u| u.is_empty()) {
            let key = agent.last_index_key();
            let li = agent.get_value(rx, key)?;
            let this_index = agent.length_value(li)?;
            let text = agent.string_units(s).unwrap_or_default();
            let next = agent.advance_index(&text, this_index, full_unicode);
            agent.set_last_index(rx, next)?;
        }
        agent.iter_result(m, false)
    })();
    agent.vm.temps.pop();
    r
}

/// `RegExp.prototype[@@replace]` (section 22.2.6.11).
fn symbol_replace(agent: &mut Agent, c: Call) -> Res<Value> {
    let rx = agent.regexp_this(c.this, "RegExp.prototype.@@replace")?;
    let s = agent.arg(c, 0);
    let s = agent.string_value(s)?;
    let base = agent.vm.temps.len();
    agent.vm.temps.push(s);
    let out = (|| {
        let text = agent.string_units(s).unwrap_or_default();
        let length_s = text.len();
        let mut replace_value = agent.arg(c, 1);
        let functional = agent.is_callable(replace_value);
        if !functional {
            replace_value = agent.string_value(replace_value)?;
            agent.vm.temps.push(replace_value);
        }
        let template = if functional {
            Vec::new()
        } else {
            agent.string_units(replace_value).unwrap_or_default()
        };
        let flags = agent.flags_of(rx)?;
        let global = flags.contains(&u16::from(b'g'));
        let full_unicode = flags.contains(&u16::from(b'u')) || flags.contains(&u16::from(b'v'));
        if global {
            agent.set_last_index(rx, 0.0)?;
        }
        let mut results = Vec::new();
        loop {
            let result = agent.regexp_exec(rx, s)?;
            if result.is_null() {
                break;
            }
            agent.vm.temps.push(result);
            results.push(result);
            if !global {
                break;
            }
            let m = agent.get_value(result, PropertyKey::Index(0))?;
            let m = agent.text_units(m)?;
            if m.is_empty() {
                let key = agent.last_index_key();
                let li = agent.get_value(rx, key)?;
                let this_index = agent.length_value(li)?;
                let next = agent.advance_index(&text, this_index, full_unicode);
                agent.set_last_index(rx, next)?;
            }
        }
        let mut acc: Vec<u16> = Vec::new();
        let mut next_source = 0usize;
        for result in results {
            let n_captures = (agent.length_of(result)? - 1.0).max(0.0) as u32;
            let matched = agent.get_value(result, PropertyKey::Index(0))?;
            let matched = agent.text_units(matched)?;
            let key = agent.name_key(Name::Str("index"));
            let pos = agent.get_value(result, key)?;
            let pos = agent.integer(pos)?.clamp(0.0, length_s as f64) as usize;
            let mut captures: Vec<Option<Vec<u16>>> = Vec::new();
            let mut capture_values = Vec::new();
            for k in 1..=n_captures {
                let v = agent.get_value(result, PropertyKey::Index(k))?;
                if v.is_undefined() {
                    captures.push(None);
                    capture_values.push(Value::UNDEFINED);
                } else {
                    let sv = agent.string_value(v)?;
                    agent.vm.temps.push(sv);
                    captures.push(agent.string_units(sv));
                    capture_values.push(sv);
                }
            }
            let key = agent.name_key(Name::Str("groups"));
            let mut named = agent.get_value(result, key)?;
            agent.vm.temps.push(named);
            let replacement = if functional {
                let m = agent.new_string_units(&matched)?;
                agent.vm.temps.push(m);
                let mut args = vec![m];
                args.extend(capture_values);
                args.push(Value::number(pos as f64));
                args.push(s);
                if !named.is_undefined() {
                    args.push(named);
                }
                let r = agent.call_value(replace_value, Value::UNDEFINED, &args)?;
                agent.text_units(r)?
            } else {
                if !named.is_undefined() {
                    named = agent.object_of(named)?;
                    agent.vm.temps.push(named);
                }
                agent.substitution(&matched, &text, pos, &captures, named, &template)?
            };
            if pos >= next_source {
                acc.extend_from_slice(text.get(next_source..pos).unwrap_or(&[]));
                acc.extend(replacement);
                next_source = pos + matched.len();
            }
        }
        if next_source < length_s {
            acc.extend_from_slice(text.get(next_source..).unwrap_or(&[]));
        }
        agent.string_checked(&acc)
    })();
    agent.vm.temps.truncate(base);
    out
}

/// `RegExp.prototype[@@search]` (section 22.2.6.12).
fn symbol_search(agent: &mut Agent, c: Call) -> Res<Value> {
    let rx = agent.regexp_this(c.this, "RegExp.prototype.@@search")?;
    let s = agent.arg(c, 0);
    let s = agent.string_value(s)?;
    let base = agent.vm.temps.len();
    agent.vm.temps.push(s);
    let out = (|| {
        let key = agent.last_index_key();
        let previous = agent.get_value(rx, key)?;
        agent.vm.temps.push(previous);
        if !crate::object::same_value(&agent.heap, previous, Value::int(0)) {
            agent.put(rx, key, Value::int(0))?;
        }
        let result = agent.regexp_exec(rx, s)?;
        agent.vm.temps.push(result);
        let current = agent.get_value(rx, key)?;
        if !crate::object::same_value(&agent.heap, current, previous) {
            agent.put(rx, key, previous)?;
        }
        if result.is_null() {
            return Ok(Value::int(-1));
        }
        let key = agent.name_key(Name::Str("index"));
        agent.get_value(result, key)
    })();
    agent.vm.temps.truncate(base);
    out
}

/// `RegExp.prototype[@@split]` (section 22.2.6.14).
fn symbol_split(agent: &mut Agent, c: Call) -> Res<Value> {
    let rx = agent.regexp_this(c.this, "RegExp.prototype.@@split")?;
    let s = agent.arg(c, 0);
    let s = agent.string_value(s)?;
    let base = agent.vm.temps.len();
    agent.vm.temps.push(s);
    let out = (|| {
        let default = agent.realm().intrinsics.regexp;
        let ctor = agent.species_constructor(rx, default)?;
        agent.vm.temps.push(ctor);
        let mut flags = agent.flags_of(rx)?;
        let unicode = flags.contains(&u16::from(b'u')) || flags.contains(&u16::from(b'v'));
        if !flags.contains(&u16::from(b'y')) {
            flags.push(u16::from(b'y'));
        }
        let f = agent.new_string_units(&flags)?;
        agent.vm.temps.push(f);
        let splitter = agent.construct_value(ctor, &[rx, f], ctor)?;
        agent.vm.temps.push(splitter);
        let a = agent.array_of(&[])?;
        agent.vm.temps.push(a);
        let limit = agent.arg(c, 1);
        let lim: u32 = if limit.is_undefined() {
            u32::MAX
        } else {
            let n = agent.number(limit)?;
            to_uint32(n)
        };
        if lim == 0 {
            return Ok(a);
        }
        let text = agent.string_units(s).unwrap_or_default();
        let size = text.len();
        if size == 0 {
            let z = agent.regexp_exec(splitter, s)?;
            if !z.is_null() {
                return Ok(a);
            }
            agent.create_data_property(a, PropertyKey::Index(0), s);
            return Ok(a);
        }
        let mut len_a = 0u32;
        let (mut p, mut q) = (0usize, 0usize);
        while q < size {
            agent.set_last_index(splitter, q as f64)?;
            let z = agent.regexp_exec(splitter, s)?;
            if z.is_null() {
                q = agent.advance_index(&text, q as f64, unicode) as usize;
                continue;
            }
            agent.vm.temps.push(z);
            let key = agent.last_index_key();
            let li = agent.get_value(splitter, key)?;
            let e = (agent.length_value(li)? as usize).min(size);
            if e == p {
                q = agent.advance_index(&text, q as f64, unicode) as usize;
                continue;
            }
            let t = agent.new_string_units(text.get(p..q).unwrap_or(&[]))?;
            agent.create_data_property(a, PropertyKey::Index(len_a), t);
            len_a += 1;
            if len_a == lim {
                return Ok(a);
            }
            p = e;
            let n = (agent.length_of(z)? - 1.0).max(0.0) as u32;
            for i in 1..=n {
                let v = agent.get_value(z, PropertyKey::Index(i))?;
                agent.create_data_property(a, PropertyKey::Index(len_a), v);
                len_a += 1;
                if len_a == lim {
                    return Ok(a);
                }
            }
            q = p;
        }
        let t = agent.new_string_units(text.get(p..size).unwrap_or(&[]))?;
        agent.create_data_property(a, PropertyKey::Index(len_a), t);
        Ok(a)
    })();
    agent.vm.temps.truncate(base);
    out
}

/// The String methods that take a regexp: `match`, `matchAll` and
/// `search` (sections 22.1.3.13, 22.1.3.14 and 22.1.3.23).
pub(crate) fn string_match(agent: &mut Agent, c: Call) -> Res<Value> {
    string_regexp(agent, c, Wk::Match, "match")
}

pub(crate) fn string_match_all(agent: &mut Agent, c: Call) -> Res<Value> {
    string_regexp(agent, c, Wk::MatchAll, "matchAll")
}

pub(crate) fn string_search(agent: &mut Agent, c: Call) -> Res<Value> {
    string_regexp(agent, c, Wk::Search, "search")
}

fn string_regexp(agent: &mut Agent, c: Call, w: Wk, method: &str) -> Res<Value> {
    if c.this.is_nullish() {
        return Err(agent.type_error(format!(
            "String.prototype.{method} called on null or undefined"
        )));
    }
    let regexp = agent.arg(c, 0);
    if !regexp.is_nullish() {
        if w == Wk::MatchAll && agent.is_regexp_value(regexp)? {
            let key = agent.name_key(Name::Str("flags"));
            let flags = agent.get_value(regexp, key)?;
            if flags.is_nullish() {
                return Err(agent.type_error(crate::interp::messages::NULLISH_TO_OBJECT));
            }
            if !agent.text_units(flags)?.contains(&u16::from(b'g')) {
                return Err(agent.type_error(
                    "String.prototype.matchAll called with a non-global RegExp argument",
                ));
            }
        }
        // Only an object's method counts: a primitive's (its prototype's)
        // is never looked up, as in Chrome (ecma262 PR 3009).
        let m = agent.pattern_method(regexp, w)?;
        if !m.is_undefined() {
            return agent.call_value(m, regexp, &[c.this]);
        }
    }
    let s = agent.string_value(c.this)?;
    let base = agent.vm.temps.len();
    agent.vm.temps.push(s);
    let out = (|| {
        let flags = if w == Wk::MatchAll {
            agent.new_string("g")?
        } else {
            Value::UNDEFINED
        };
        let rx = agent.regexp_create(regexp, flags)?;
        agent.vm.temps.push(rx);
        let key = agent.well_known_key(w);
        let f = agent.get_value(rx, key)?;
        agent.call_value(f, rx, &[s])
    })();
    agent.vm.temps.truncate(base);
    out
}
