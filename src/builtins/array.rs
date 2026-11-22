//! `Array` (ECMA-262 section 23.1), its methods generic over array-likes
//! as the spec writes them; `keys`, `values`, `entries` and
//! `[Symbol.iterator]` are the iterators' ([`super::iterator`]).
//! `Array.fromAsync` waits for J3.4's promises.

use super::{CONFIGURABLE, CONSTRUCTOR, Call, Entry, Name, What, Wk, entry, method};
use crate::agent::Agent;
use crate::heap::ObjectCell;
use crate::interp::Res;
use crate::object::ObjectKind;
use crate::string::PropertyKey;
use crate::value::Value;

/// The largest array-like length (2^53 - 1).
const MAX_SAFE: f64 = 9_007_199_254_740_991.0;

pub(crate) const STATICS: &[Entry] = &[
    method("isArray", 1, is_array),
    method("from", 1, from),
    method("of", 0, of),
    entry(Name::Sym(Wk::Species), What::Getter(species), CONFIGURABLE),
];

pub(crate) const PROTO: &[Entry] = &[
    CONSTRUCTOR,
    method("at", 1, at),
    method("concat", 1, concat),
    method("copyWithin", 2, copy_within),
    method("fill", 1, fill),
    method("find", 1, find),
    method("findIndex", 1, find_index),
    method("findLast", 1, find_last),
    method("findLastIndex", 1, find_last_index),
    method("lastIndexOf", 1, last_index_of),
    method("pop", 0, pop),
    method("push", 1, push),
    method("reverse", 0, reverse),
    method("shift", 0, shift),
    method("unshift", 1, unshift),
    method("slice", 2, slice),
    method("sort", 1, sort),
    method("splice", 2, splice),
    method("includes", 1, includes),
    method("indexOf", 1, index_of),
    method("join", 1, join),
    method("keys", 0, super::iterator::keys),
    method("entries", 0, super::iterator::entries),
    method("values", 0, super::iterator::values),
    method("forEach", 1, for_each),
    method("filter", 1, filter),
    method("flat", 0, flat),
    method("flatMap", 1, flat_map),
    method("map", 1, map),
    method("every", 1, every),
    method("some", 1, some),
    method("reduce", 1, reduce),
    method("reduceRight", 1, reduce_right),
    method("toReversed", 0, to_reversed),
    method("toSorted", 1, to_sorted),
    method("toSpliced", 2, to_spliced),
    method("with", 2, with),
    method("toLocaleString", 0, to_locale_string),
    method("toString", 0, to_string),
    entry(
        Name::Sym(Wk::Iterator),
        What::Alias("values"),
        super::HIDDEN,
    ),
    entry(Name::Sym(Wk::Unscopables), What::Given(0), CONFIGURABLE),
];

/// `Array.prototype[@@unscopables]`'s names (section 23.1.3.41).
pub(crate) const UNSCOPABLES: &[&str] = &[
    "at",
    "copyWithin",
    "entries",
    "fill",
    "find",
    "findIndex",
    "findLast",
    "findLastIndex",
    "flat",
    "flatMap",
    "includes",
    "keys",
    "toReversed",
    "toSorted",
    "toSpliced",
    "values",
];

impl Agent {
    /// ArrayCreate (section 10.4.2.2), with prototype `proto`.
    pub(crate) fn array_create(&mut self, length: f64, proto: Option<Value>) -> Res<Value> {
        if length > 4_294_967_295.0 {
            return Err(self.range_error("Invalid array length"));
        }
        let proto = proto.unwrap_or(self.realm().array_prototype);
        let a = self.alloc_object(ObjectCell::new(proto, ObjectKind::Array))?;
        if length > 0.0
            && let Some(o) = a.as_object()
        {
            let key = self.heap.length_key;
            self.heap.define_own_property(
                o,
                key,
                crate::object::Descriptor {
                    value: Some(Value::number(length)),
                    ..crate::object::Descriptor::default()
                },
            );
        }
        Ok(a)
    }

    /// ArraySpeciesCreate (section 10.4.2.3).
    fn array_species_create(&mut self, original: Value, length: f64) -> Res<Value> {
        if !self.is_array(original) {
            return self.array_create(length, None);
        }
        let key = self.realm().constructor_key;
        let mut c = self.get_value(original, key)?;
        // Another realm's `Array` makes this realm's arrays.
        if self.is_constructor_value(c) {
            let realm = self.function_realm(c);
            if realm != self.vm.current
                && self
                    .vm
                    .realms
                    .get(realm as usize)
                    .is_some_and(|r| r.intrinsics.array == c)
            {
                c = Value::UNDEFINED;
            }
        }
        if c.as_object().is_some() {
            let key = self.well_known_key(Wk::Species);
            c = self.get_value(c, key)?;
            if c.is_null() {
                c = Value::UNDEFINED;
            }
        }
        if c.is_undefined() {
            return self.array_create(length, None);
        }
        if !self.is_constructor_value(c) {
            return Err(self.type_error("object.constructor[Symbol.species] is not a constructor"));
        }
        self.construct_value(c, &[Value::number(length)], c)
    }

    fn set_length(&mut self, o: Value, len: f64) -> Res<()> {
        let key = self.heap.length_key;
        self.put(o, key, Value::number(len))
    }

    /// Get of index `i` (which may be past 2^32 - 2).
    fn get_index(&mut self, o: Value, i: f64) -> Res<Value> {
        let key = self.index_key(i);
        self.get_value(o, key)
    }

    fn has_index(&mut self, o: Value, i: f64) -> bool {
        let key = self.index_key(i);
        self.has_property(o, key)
    }

    fn put_index(&mut self, o: Value, i: f64, v: Value) -> Res<()> {
        let key = self.index_key(i);
        self.put(o, key, v)
    }

    fn create_index(&mut self, o: Value, i: f64, v: Value) -> Res<()> {
        let key = self.index_key(i);
        self.create_data_property_or_throw(o, key, v)
    }

    fn delete_index(&mut self, o: Value, i: f64) -> Res<()> {
        let key = self.index_key(i);
        self.delete_or_throw(o, key)
    }

    /// The callback argument, or the reference browser's TypeError.
    fn callback(&mut self, f: Value) -> Res<Value> {
        if self.is_callable(f) {
            Ok(f)
        } else {
            let text = self.display(f);
            Err(self.type_error(format!("{text} is not a function")))
        }
    }
}

/// Runs `f` with `o` rooted.
fn rooted<T>(agent: &mut Agent, o: Value, f: impl FnOnce(&mut Agent) -> Res<T>) -> Res<T> {
    agent.vm.temps.push(o);
    let r = f(agent);
    agent.vm.temps.pop();
    r
}

/// `Array(...)` (section 23.1.1.1).
pub(crate) fn construct(agent: &mut Agent, c: Call) -> Res<Value> {
    let nt = if c.new_target.is_undefined() {
        c.callee
    } else {
        c.new_target
    };
    let proto = agent.prototype_from(nt, |r| r.array_prototype)?;
    match c.argc {
        0 => agent.array_create(0.0, Some(proto)),
        1 => {
            let len = agent.arg(c, 0);
            if let Some(n) = len.as_number() {
                let int = crate::interp::ops::to_int32(n) as u32;
                if f64::from(int) != n {
                    return Err(agent.range_error("Invalid array length"));
                }
                agent.array_create(n, Some(proto))
            } else {
                let a = agent.array_create(0.0, Some(proto))?;
                agent.create_data_property(a, PropertyKey::Index(0), len);
                Ok(a)
            }
        }
        _ => {
            let a = agent.array_create(0.0, Some(proto))?;
            for i in 0..c.argc {
                let v = agent.arg(c, i);
                agent.create_data_property(a, PropertyKey::Index(i as u32), v);
            }
            Ok(a)
        }
    }
}

fn is_array(agent: &mut Agent, c: Call) -> Res<Value> {
    Ok(Value::bool(agent.is_array(agent.arg(c, 0))))
}

fn species(_: &mut Agent, c: Call) -> Res<Value> {
    Ok(c.this)
}

/// `this` as a constructor, or a new array: what `from` and `of` fill.
fn construct_or_array(agent: &mut Agent, c: Call, args: &[Value]) -> Res<Value> {
    if agent.is_constructor_value(c.this) {
        agent.construct_value(c.this, args, c.this)
    } else {
        let len = args.first().and_then(|v| v.as_number()).unwrap_or(0.0);
        agent.array_create(len, None)
    }
}

fn from(agent: &mut Agent, c: Call) -> Res<Value> {
    let items = agent.arg(c, 0);
    let mapfn = agent.arg(c, 1);
    let this_arg = agent.arg(c, 2);
    if !mapfn.is_undefined() {
        agent.callback(mapfn)?;
    }
    let key = agent.well_known_key(Wk::Iterator);
    let using = if items.is_nullish() {
        Value::UNDEFINED
    } else {
        agent.get_method(items, key)?
    };
    let base = agent.vm.temps.len();
    let result = (|| {
        if !using.is_undefined() {
            let a = construct_or_array(agent, c, &[])?;
            agent.vm.temps.push(a);
            let it = agent.call_value(using, items, &[])?;
            if it.as_object().is_none() {
                return Err(
                    agent.type_error("Result of the Symbol.iterator method is not an object")
                );
            }
            agent.vm.temps.push(it);
            let next_key = agent.name_key(Name::Str("next"));
            let next = agent.get_value(it, next_key)?;
            agent.vm.temps.push(next);
            let (done_key, value_key) = (
                agent.name_key(Name::Str("done")),
                agent.name_key(Name::Str("value")),
            );
            let mut k = 0.0;
            loop {
                agent.tick()?;
                let r = agent.call_value(next, it, &[])?;
                if r.as_object().is_none() {
                    let text = agent.display(r);
                    return Err(
                        agent.type_error(format!("Iterator result {text} is not an object"))
                    );
                }
                let done = agent.get_value(r, done_key)?;
                if agent.to_boolean(done) {
                    agent.set_length(a, k)?;
                    return Ok(a);
                }
                let mut v = agent.get_value(r, value_key)?;
                let step = (|| {
                    if !mapfn.is_undefined() {
                        v = agent.call_value(mapfn, this_arg, &[v, Value::number(k)])?;
                    }
                    agent.create_index(a, k, v)
                })();
                if let Err(e) = step {
                    // IteratorClose: `return`, its own failure ignored.
                    let saved = agent.vm.exception;
                    agent.vm.temps.push(saved);
                    let ret_key = agent.name_key(Name::Str("return"));
                    if let Ok(ret) = agent.get_method(it, ret_key)
                        && !ret.is_undefined()
                    {
                        let _ = agent.call_value(ret, it, &[]);
                    }
                    agent.vm.exception = saved;
                    return Err(e);
                }
                k += 1.0;
            }
        }
        let array_like = agent.object_of(items)?;
        agent.vm.temps.push(array_like);
        let len = agent.length_of(array_like)?;
        let a = construct_or_array(agent, c, &[Value::number(len)])?;
        agent.vm.temps.push(a);
        let mut k = 0.0;
        while k < len {
            agent.tick()?;
            let mut v = agent.get_index(array_like, k)?;
            if !mapfn.is_undefined() {
                v = agent.call_value(mapfn, this_arg, &[v, Value::number(k)])?;
            }
            agent.create_index(a, k, v)?;
            k += 1.0;
        }
        agent.set_length(a, len)?;
        Ok(a)
    })();
    agent.vm.temps.truncate(base);
    result
}

fn of(agent: &mut Agent, c: Call) -> Res<Value> {
    let len = c.argc as f64;
    let a = construct_or_array(agent, c, &[Value::number(len)])?;
    rooted(agent, a, |agent| {
        for i in 0..c.argc {
            let v = agent.arg(c, i);
            agent.create_index(a, i as f64, v)?;
        }
        agent.set_length(a, len)?;
        Ok(a)
    })
}

/// `this` as an object and its length, for the methods.
fn this_object(agent: &mut Agent, c: Call) -> Res<(Value, f64)> {
    let o = agent.object_of(c.this)?;
    let len = rooted(agent, o, |agent| agent.length_of(o))?;
    Ok((o, len))
}

fn at(agent: &mut Agent, c: Call) -> Res<Value> {
    let (o, len) = this_object(agent, c)?;
    rooted(agent, o, |agent| {
        let n = agent.integer(agent.arg(c, 0))?;
        let k = if n >= 0.0 { n } else { len + n };
        if k < 0.0 || k >= len {
            return Ok(Value::UNDEFINED);
        }
        agent.get_index(o, k)
    })
}

/// IsConcatSpreadable (section 23.1.3.2.1).
fn spreadable(agent: &mut Agent, v: Value) -> Res<bool> {
    if v.as_object().is_none() {
        return Ok(false);
    }
    let key = agent.well_known_key(Wk::IsConcatSpreadable);
    let s = agent.get_value(v, key)?;
    if !s.is_undefined() {
        return Ok(agent.to_boolean(s));
    }
    Ok(agent.is_array(v))
}

fn concat(agent: &mut Agent, c: Call) -> Res<Value> {
    let o = agent.object_of(c.this)?;
    rooted(agent, o, |agent| {
        let a = agent.array_species_create(o, 0.0)?;
        rooted(agent, a, |agent| {
            let mut n = 0.0;
            let mut items = vec![o];
            items.extend(agent.args(c));
            for e in items {
                if spreadable(agent, e)? {
                    let len = agent.length_of(e)?;
                    if n + len > MAX_SAFE {
                        return Err(agent.type_error("Invalid array length"));
                    }
                    let mut k = 0.0;
                    while k < len {
                        agent.tick()?;
                        if agent.has_index(e, k) {
                            let v = agent.get_index(e, k)?;
                            agent.create_index(a, n, v)?;
                        }
                        n += 1.0;
                        k += 1.0;
                    }
                } else {
                    if n >= MAX_SAFE {
                        return Err(agent.type_error("Invalid array length"));
                    }
                    agent.create_index(a, n, e)?;
                    n += 1.0;
                }
            }
            agent.set_length(a, n)?;
            Ok(a)
        })
    })
}

fn copy_within(agent: &mut Agent, c: Call) -> Res<Value> {
    let (o, len) = this_object(agent, c)?;
    rooted(agent, o, |agent| {
        let to = agent.relative(agent.arg(c, 0), len, 0.0)?;
        let from = agent.relative(agent.arg(c, 1), len, 0.0)?;
        let fin = agent.relative(agent.arg(c, 2), len, len)?;
        let mut count = (fin - from).min(len - to);
        let (mut from, mut to, dir) = if from < to && to < from + count {
            (from + count - 1.0, to + count - 1.0, -1.0)
        } else {
            (from, to, 1.0)
        };
        while count > 0.0 {
            agent.tick()?;
            if agent.has_index(o, from) {
                let v = agent.get_index(o, from)?;
                agent.put_index(o, to, v)?;
            } else {
                agent.delete_index(o, to)?;
            }
            from += dir;
            to += dir;
            count -= 1.0;
        }
        Ok(o)
    })
}

fn fill(agent: &mut Agent, c: Call) -> Res<Value> {
    let (o, len) = this_object(agent, c)?;
    rooted(agent, o, |agent| {
        let v = agent.arg(c, 0);
        let mut k = agent.relative(agent.arg(c, 1), len, 0.0)?;
        let fin = agent.relative(agent.arg(c, 2), len, len)?;
        while k < fin {
            agent.tick()?;
            agent.put_index(o, k, v)?;
            k += 1.0;
        }
        Ok(o)
    })
}

/// FindViaPredicate (section 23.1.3.12.1): the index and value found.
fn find_via(agent: &mut Agent, c: Call, last: bool) -> Res<(f64, Value)> {
    let (o, len) = this_object(agent, c)?;
    rooted(agent, o, |agent| {
        let f = agent.callback(agent.arg(c, 0))?;
        let this_arg = agent.arg(c, 1);
        let mut i = 0.0;
        while i < len {
            agent.tick()?;
            let k = if last { len - 1.0 - i } else { i };
            let v = agent.get_index(o, k)?;
            agent.vm.temps.push(v);
            let r = agent.call_value(f, this_arg, &[v, Value::number(k), o]);
            agent.vm.temps.pop();
            if agent.to_boolean(r?) {
                return Ok((k, v));
            }
            i += 1.0;
        }
        Ok((-1.0, Value::UNDEFINED))
    })
}

fn find(agent: &mut Agent, c: Call) -> Res<Value> {
    find_via(agent, c, false).map(|(_, v)| v)
}

fn find_index(agent: &mut Agent, c: Call) -> Res<Value> {
    find_via(agent, c, false).map(|(k, _)| Value::number(k))
}

fn find_last(agent: &mut Agent, c: Call) -> Res<Value> {
    find_via(agent, c, true).map(|(_, v)| v)
}

fn find_last_index(agent: &mut Agent, c: Call) -> Res<Value> {
    find_via(agent, c, true).map(|(k, _)| Value::number(k))
}

fn index_of(agent: &mut Agent, c: Call) -> Res<Value> {
    let (o, len) = this_object(agent, c)?;
    rooted(agent, o, |agent| {
        if len == 0.0 {
            return Ok(Value::int(-1));
        }
        let target = agent.arg(c, 0);
        let mut k = agent.relative(agent.arg(c, 1), len, 0.0)?;
        while k < len {
            agent.tick()?;
            if agent.has_index(o, k) {
                let v = agent.get_index(o, k)?;
                if agent.strict_equals(v, target) {
                    return Ok(Value::number(k));
                }
            }
            k += 1.0;
        }
        Ok(Value::int(-1))
    })
}

fn last_index_of(agent: &mut Agent, c: Call) -> Res<Value> {
    let (o, len) = this_object(agent, c)?;
    rooted(agent, o, |agent| {
        if len == 0.0 {
            return Ok(Value::int(-1));
        }
        let target = agent.arg(c, 0);
        let mut k = if c.argc > 1 {
            let n = agent.integer(agent.arg(c, 1))?;
            if n >= 0.0 { n.min(len - 1.0) } else { len + n }
        } else {
            len - 1.0
        };
        while k >= 0.0 {
            agent.tick()?;
            if agent.has_index(o, k) {
                let v = agent.get_index(o, k)?;
                if agent.strict_equals(v, target) {
                    return Ok(Value::number(k));
                }
            }
            k -= 1.0;
        }
        Ok(Value::int(-1))
    })
}

fn includes(agent: &mut Agent, c: Call) -> Res<Value> {
    let (o, len) = this_object(agent, c)?;
    rooted(agent, o, |agent| {
        if len == 0.0 {
            return Ok(Value::FALSE);
        }
        let target = agent.arg(c, 0);
        let mut k = agent.relative(agent.arg(c, 1), len, 0.0)?;
        while k < len {
            agent.tick()?;
            let v = agent.get_index(o, k)?;
            // SameValueZero.
            let same = match (v.as_number(), target.as_number()) {
                (Some(x), Some(y)) => x == y || (x.is_nan() && y.is_nan()),
                _ => agent.strict_equals(v, target),
            };
            if same {
                return Ok(Value::TRUE);
            }
            k += 1.0;
        }
        Ok(Value::FALSE)
    })
}

fn pop(agent: &mut Agent, c: Call) -> Res<Value> {
    let (o, len) = this_object(agent, c)?;
    rooted(agent, o, |agent| {
        if len == 0.0 {
            agent.set_length(o, 0.0)?;
            return Ok(Value::UNDEFINED);
        }
        let v = agent.get_index(o, len - 1.0)?;
        agent.delete_index(o, len - 1.0)?;
        agent.set_length(o, len - 1.0)?;
        Ok(v)
    })
}

/// `Array.prototype.push` (section 23.1.3.23).
pub(crate) fn push(agent: &mut Agent, c: Call) -> Res<Value> {
    let (o, len) = this_object(agent, c)?;
    rooted(agent, o, |agent| {
        if len + c.argc as f64 > MAX_SAFE {
            return Err(agent.type_error("Pushing elements past 2^53-1 length is disallowed"));
        }
        for i in 0..c.argc {
            let v = agent.arg(c, i);
            agent.put_index(o, len + i as f64, v)?;
        }
        let n = len + c.argc as f64;
        agent.set_length(o, n)?;
        Ok(Value::number(n))
    })
}

fn reverse(agent: &mut Agent, c: Call) -> Res<Value> {
    let (o, len) = this_object(agent, c)?;
    rooted(agent, o, |agent| {
        let middle = (len / 2.0).floor();
        let mut lower = 0.0;
        while lower != middle {
            agent.tick()?;
            let upper = len - lower - 1.0;
            let has_lower = agent.has_index(o, lower);
            let lv = if has_lower {
                agent.get_index(o, lower)?
            } else {
                Value::UNDEFINED
            };
            agent.vm.temps.push(lv);
            let r = (|| {
                let has_upper = agent.has_index(o, upper);
                let uv = if has_upper {
                    agent.get_index(o, upper)?
                } else {
                    Value::UNDEFINED
                };
                match (has_lower, has_upper) {
                    (true, true) => {
                        agent.put_index(o, lower, uv)?;
                        agent.put_index(o, upper, lv)
                    }
                    (false, true) => {
                        agent.put_index(o, lower, uv)?;
                        agent.delete_index(o, upper)
                    }
                    (true, false) => {
                        agent.delete_index(o, lower)?;
                        agent.put_index(o, upper, lv)
                    }
                    (false, false) => Ok(()),
                }
            })();
            agent.vm.temps.pop();
            r?;
            lower += 1.0;
        }
        Ok(o)
    })
}

fn shift(agent: &mut Agent, c: Call) -> Res<Value> {
    let (o, len) = this_object(agent, c)?;
    rooted(agent, o, |agent| {
        if len == 0.0 {
            agent.set_length(o, 0.0)?;
            return Ok(Value::UNDEFINED);
        }
        let first = agent.get_index(o, 0.0)?;
        rooted(agent, first, |agent| {
            let mut k = 1.0;
            while k < len {
                agent.tick()?;
                if agent.has_index(o, k) {
                    let v = agent.get_index(o, k)?;
                    agent.put_index(o, k - 1.0, v)?;
                } else {
                    agent.delete_index(o, k - 1.0)?;
                }
                k += 1.0;
            }
            agent.delete_index(o, len - 1.0)?;
            agent.set_length(o, len - 1.0)?;
            Ok(first)
        })
    })
}

fn unshift(agent: &mut Agent, c: Call) -> Res<Value> {
    let (o, len) = this_object(agent, c)?;
    rooted(agent, o, |agent| {
        let n = c.argc as f64;
        if n > 0.0 {
            if len + n > MAX_SAFE {
                return Err(agent.type_error("Invalid array length"));
            }
            let mut k = len;
            while k > 0.0 {
                agent.tick()?;
                let from = k - 1.0;
                if agent.has_index(o, from) {
                    let v = agent.get_index(o, from)?;
                    agent.put_index(o, k + n - 1.0, v)?;
                } else {
                    agent.delete_index(o, k + n - 1.0)?;
                }
                k -= 1.0;
            }
            for j in 0..c.argc {
                let v = agent.arg(c, j);
                agent.put_index(o, j as f64, v)?;
            }
        }
        agent.set_length(o, len + n)?;
        Ok(Value::number(len + n))
    })
}

fn slice(agent: &mut Agent, c: Call) -> Res<Value> {
    let (o, len) = this_object(agent, c)?;
    rooted(agent, o, |agent| {
        let mut k = agent.relative(agent.arg(c, 0), len, 0.0)?;
        let fin = agent.relative(agent.arg(c, 1), len, len)?;
        let count = (fin - k).max(0.0);
        let a = agent.array_species_create(o, count)?;
        rooted(agent, a, |agent| {
            let mut n = 0.0;
            while k < fin {
                agent.tick()?;
                if agent.has_index(o, k) {
                    let v = agent.get_index(o, k)?;
                    agent.create_index(a, n, v)?;
                }
                k += 1.0;
                n += 1.0;
            }
            agent.set_length(a, n)?;
            Ok(a)
        })
    })
}

/// SortCompare (section 23.1.3.30.2) of two values, neither undefined.
fn sort_compare(agent: &mut Agent, f: Value, x: Value, y: Value) -> Res<std::cmp::Ordering> {
    use std::cmp::Ordering;
    if !f.is_undefined() {
        let v = agent.call_value(f, Value::UNDEFINED, &[x, y])?;
        let n = agent.number(v)?;
        return Ok(if n < 0.0 {
            Ordering::Less
        } else if n > 0.0 {
            Ordering::Greater
        } else {
            Ordering::Equal
        });
    }
    if let (Some(a), Some(b)) = (x.as_int(), y.as_int()) {
        // Integers compare as their decimal text would.
        if (a >= 0) == (b >= 0) && a.abs() < 10 && b.abs() < 10 {
            return Ok(a.cmp(&b));
        }
    }
    let xs = agent.text_units(x)?;
    let ys = agent.text_units(y)?;
    Ok(xs.cmp(&ys))
}

impl Agent {
    /// SortIndexedProperties (section 23.1.3.30.1) with holes skipped,
    /// then sorted: a stable merge sort whose comparisons may throw. The
    /// values are rooted (in `temps`) until the caller truncates.
    fn sorted_values(&mut self, o: Value, len: f64, f: Value, holes: bool) -> Res<Vec<Value>> {
        let mut values = Vec::new();
        let mut undefined = 0usize;
        let mut k = 0.0;
        while k < len {
            self.tick()?;
            if holes || self.has_index(o, k) {
                let v = self.get_index(o, k)?;
                if v.is_undefined() {
                    undefined += 1;
                } else {
                    self.vm.temps.push(v);
                    values.push(v);
                }
            }
            k += 1.0;
        }
        // Bottom-up merge sort.
        let n = values.len();
        let mut buf = values.clone();
        let mut width = 1;
        while width < n {
            let mut i = 0;
            while i < n {
                let mid = (i + width).min(n);
                let hi = (i + 2 * width).min(n);
                let (mut a, mut b, mut out) = (i, mid, i);
                while a < mid && b < hi {
                    self.tick()?;
                    let (x, y) = (
                        values.get(a).copied().unwrap_or(Value::UNDEFINED),
                        values.get(b).copied().unwrap_or(Value::UNDEFINED),
                    );
                    let take_b = sort_compare(self, f, x, y)? == std::cmp::Ordering::Greater;
                    if let Some(slot) = buf.get_mut(out) {
                        *slot = if take_b { y } else { x };
                    }
                    if take_b {
                        b += 1;
                    } else {
                        a += 1;
                    }
                    out += 1;
                }
                for j in (a..mid).chain(b..hi) {
                    if let (Some(slot), Some(&v)) = (buf.get_mut(out), values.get(j)) {
                        *slot = v;
                    }
                    out += 1;
                }
                i += 2 * width;
            }
            std::mem::swap(&mut values, &mut buf);
            width *= 2;
        }
        values.extend(std::iter::repeat_n(Value::UNDEFINED, undefined));
        Ok(values)
    }
}

fn comparator(agent: &mut Agent, f: Value) -> Res<Value> {
    if !f.is_undefined() && !agent.is_callable(f) {
        return Err(
            agent.type_error("The comparison function must be either a function or undefined")
        );
    }
    Ok(f)
}

fn sort(agent: &mut Agent, c: Call) -> Res<Value> {
    let f = comparator(agent, agent.arg(c, 0))?;
    let (o, len) = this_object(agent, c)?;
    let base = agent.vm.temps.len();
    agent.vm.temps.push(o);
    let result = (|| {
        let values = agent.sorted_values(o, len, f, false)?;
        let count = values.len() as f64;
        for (i, v) in values.into_iter().enumerate() {
            agent.put_index(o, i as f64, v)?;
        }
        let mut k = count;
        while k < len {
            agent.tick()?;
            agent.delete_index(o, k)?;
            k += 1.0;
        }
        Ok(o)
    })();
    agent.vm.temps.truncate(base);
    result
}

fn to_sorted(agent: &mut Agent, c: Call) -> Res<Value> {
    let f = comparator(agent, agent.arg(c, 0))?;
    let (o, len) = this_object(agent, c)?;
    let base = agent.vm.temps.len();
    agent.vm.temps.push(o);
    let result = (|| {
        let a = agent.array_create(len, None)?;
        agent.vm.temps.push(a);
        let values = agent.sorted_values(o, len, f, true)?;
        for (i, v) in values.into_iter().enumerate() {
            agent.create_index(a, i as f64, v)?;
        }
        Ok(a)
    })();
    agent.vm.temps.truncate(base);
    result
}

fn splice(agent: &mut Agent, c: Call) -> Res<Value> {
    let (o, len) = this_object(agent, c)?;
    rooted(agent, o, |agent| {
        let start = agent.relative(agent.arg(c, 0), len, 0.0)?;
        let items: Vec<Value> = (2..c.argc).map(|i| agent.arg(c, i)).collect();
        let delete = match c.argc {
            0 => 0.0,
            1 => len - start,
            _ => agent.integer(agent.arg(c, 1))?.clamp(0.0, len - start),
        };
        let insert = items.len() as f64;
        if len + insert - delete > MAX_SAFE {
            return Err(agent.type_error("Invalid array length"));
        }
        let a = agent.array_species_create(o, delete)?;
        rooted(agent, a, |agent| {
            let mut k = 0.0;
            while k < delete {
                agent.tick()?;
                if agent.has_index(o, start + k) {
                    let v = agent.get_index(o, start + k)?;
                    agent.create_index(a, k, v)?;
                }
                k += 1.0;
            }
            agent.set_length(a, delete)?;
            if insert < delete {
                let mut k = start;
                while k < len - delete {
                    agent.tick()?;
                    let (from, to) = (k + delete, k + insert);
                    if agent.has_index(o, from) {
                        let v = agent.get_index(o, from)?;
                        agent.put_index(o, to, v)?;
                    } else {
                        agent.delete_index(o, to)?;
                    }
                    k += 1.0;
                }
                let mut k = len;
                while k > len - delete + insert {
                    agent.tick()?;
                    agent.delete_index(o, k - 1.0)?;
                    k -= 1.0;
                }
            } else if insert > delete {
                let mut k = len - delete;
                while k > start {
                    agent.tick()?;
                    let (from, to) = (k + delete - 1.0, k + insert - 1.0);
                    if agent.has_index(o, from) {
                        let v = agent.get_index(o, from)?;
                        agent.put_index(o, to, v)?;
                    } else {
                        agent.delete_index(o, to)?;
                    }
                    k -= 1.0;
                }
            }
            for (i, v) in items.into_iter().enumerate() {
                agent.put_index(o, start + i as f64, v)?;
            }
            agent.set_length(o, len - delete + insert)?;
            Ok(a)
        })
    })
}

fn to_spliced(agent: &mut Agent, c: Call) -> Res<Value> {
    let (o, len) = this_object(agent, c)?;
    rooted(agent, o, |agent| {
        let start = agent.relative(agent.arg(c, 0), len, 0.0)?;
        let items: Vec<Value> = (2..c.argc).map(|i| agent.arg(c, i)).collect();
        let skip = match c.argc {
            0 => 0.0,
            1 => len - start,
            _ => agent.integer(agent.arg(c, 1))?.clamp(0.0, len - start),
        };
        let new_len = len + items.len() as f64 - skip;
        if new_len > MAX_SAFE {
            return Err(agent.type_error("Invalid array length"));
        }
        let a = agent.array_create(new_len, None)?;
        rooted(agent, a, |agent| {
            let mut i = 0.0;
            let mut r = start + skip;
            while i < start {
                agent.tick()?;
                let v = agent.get_index(o, i)?;
                agent.create_index(a, i, v)?;
                i += 1.0;
            }
            for v in items {
                agent.create_index(a, i, v)?;
                i += 1.0;
            }
            while i < new_len {
                agent.tick()?;
                let v = agent.get_index(o, r)?;
                agent.create_index(a, i, v)?;
                i += 1.0;
                r += 1.0;
            }
            Ok(a)
        })
    })
}

fn with(agent: &mut Agent, c: Call) -> Res<Value> {
    let (o, len) = this_object(agent, c)?;
    rooted(agent, o, |agent| {
        let rel = agent.integer(agent.arg(c, 0))?;
        let index = if rel >= 0.0 { rel } else { len + rel };
        if index >= len || index < 0.0 {
            return Err(agent.range_error(
                "Invalid index : ".to_owned() + &crate::number::number_to_string(rel),
            ));
        }
        let value = agent.arg(c, 1);
        let a = agent.array_create(len, None)?;
        rooted(agent, a, |agent| {
            let mut k = 0.0;
            while k < len {
                agent.tick()?;
                let v = if k == index {
                    value
                } else {
                    agent.get_index(o, k)?
                };
                agent.create_index(a, k, v)?;
                k += 1.0;
            }
            Ok(a)
        })
    })
}

fn to_reversed(agent: &mut Agent, c: Call) -> Res<Value> {
    let (o, len) = this_object(agent, c)?;
    rooted(agent, o, |agent| {
        let a = agent.array_create(len, None)?;
        rooted(agent, a, |agent| {
            let mut k = 0.0;
            while k < len {
                agent.tick()?;
                let v = agent.get_index(o, len - k - 1.0)?;
                agent.create_index(a, k, v)?;
                k += 1.0;
            }
            Ok(a)
        })
    })
}

fn join_with(agent: &mut Agent, c: Call, locale: bool) -> Res<Value> {
    let (o, len) = this_object(agent, c)?;
    // A cycle joins as the empty string, as in the reference browser.
    if agent.vm.joining.contains(&o) {
        return Ok(agent.realm().strings.empty);
    }
    agent.vm.joining.push(o);
    let result = (|| {
        let sep = if locale {
            vec![u16::from(b',')]
        } else {
            let s = agent.arg(c, 0);
            if s.is_undefined() {
                vec![u16::from(b',')]
            } else {
                agent.text_units(s)?
            }
        };
        let mut out = Vec::new();
        let mut k = 0.0;
        while k < len {
            agent.tick()?;
            if k > 0.0 {
                out.extend_from_slice(&sep);
            }
            let v = agent.get_index(o, k)?;
            if !v.is_nullish() {
                if locale {
                    let key = agent.name_key(Name::Str("toLocaleString"));
                    let f = agent.get_value(v, key)?;
                    let r = agent.call_value(f, v, &[])?;
                    out.extend(agent.text_units(r)?);
                } else {
                    out.extend(agent.text_units(v)?);
                }
            }
            if out.len() > super::string::MAX_LENGTH {
                return Err(agent.range_error("Invalid string length"));
            }
            k += 1.0;
        }
        agent.new_string_units(&out)
    })();
    agent.vm.joining.pop();
    result
}

fn join(agent: &mut Agent, c: Call) -> Res<Value> {
    join_with(agent, c, false)
}

fn to_locale_string(agent: &mut Agent, c: Call) -> Res<Value> {
    join_with(agent, c, true)
}

fn to_string(agent: &mut Agent, c: Call) -> Res<Value> {
    let o = agent.object_of(c.this)?;
    rooted(agent, o, |agent| {
        let key = agent.name_key(Name::Str("join"));
        let f = agent.get_value(o, key)?;
        if agent.is_callable(f) {
            agent.call_value(f, o, &[])
        } else {
            super::object::to_string(agent, Call { this: o, ..c })
        }
    })
}

/// The callback methods' loop: `f(value, index, o)` for each index
/// present, `each` given the index, value and result; `true` from
/// `each` stops it.
fn each(
    agent: &mut Agent,
    c: Call,
    mut on: impl FnMut(&mut Agent, f64, Value, Value) -> Res<bool>,
) -> Res<()> {
    let (o, len) = this_object(agent, c)?;
    rooted(agent, o, |agent| {
        let f = agent.callback(agent.arg(c, 0))?;
        let this_arg = agent.arg(c, 1);
        let mut k = 0.0;
        while k < len {
            agent.tick()?;
            if agent.has_index(o, k) {
                let v = agent.get_index(o, k)?;
                agent.vm.temps.push(v);
                let r = agent.call_value(f, this_arg, &[v, Value::number(k), o]);
                let stop = r.and_then(|r| on(agent, k, v, r));
                agent.vm.temps.pop();
                if stop? {
                    break;
                }
            }
            k += 1.0;
        }
        Ok(())
    })
}

fn for_each(agent: &mut Agent, c: Call) -> Res<Value> {
    each(agent, c, |_, _, _, _| Ok(false))?;
    Ok(Value::UNDEFINED)
}

fn every(agent: &mut Agent, c: Call) -> Res<Value> {
    let mut all = true;
    each(agent, c, |agent, _, _, r| {
        all = agent.to_boolean(r);
        Ok(!all)
    })?;
    Ok(Value::bool(all))
}

fn some(agent: &mut Agent, c: Call) -> Res<Value> {
    let mut any = false;
    each(agent, c, |agent, _, _, r| {
        any = agent.to_boolean(r);
        Ok(any)
    })?;
    Ok(Value::bool(any))
}

fn map(agent: &mut Agent, c: Call) -> Res<Value> {
    let (o, len) = this_object(agent, c)?;
    rooted(agent, o, |agent| {
        agent.callback(agent.arg(c, 0))?;
        let a = agent.array_species_create(o, len)?;
        rooted(agent, a, |agent| {
            each(agent, Call { this: o, ..c }, |agent, k, _, r| {
                agent.create_index(a, k, r)?;
                Ok(false)
            })?;
            Ok(a)
        })
    })
}

fn filter(agent: &mut Agent, c: Call) -> Res<Value> {
    let o = agent.object_of(c.this)?;
    rooted(agent, o, |agent| {
        agent.length_of(o)?;
        agent.callback(agent.arg(c, 0))?;
        let a = agent.array_species_create(o, 0.0)?;
        rooted(agent, a, |agent| {
            let mut n = 0.0;
            each(agent, Call { this: o, ..c }, |agent, _, v, r| {
                if agent.to_boolean(r) {
                    agent.create_index(a, n, v)?;
                    n += 1.0;
                }
                Ok(false)
            })?;
            Ok(a)
        })
    })
}

fn reduce_with(agent: &mut Agent, c: Call, right: bool) -> Res<Value> {
    let (o, len) = this_object(agent, c)?;
    rooted(agent, o, |agent| {
        let f = agent.callback(agent.arg(c, 0))?;
        let mut i = 0.0;
        let index = |i: f64| if right { len - 1.0 - i } else { i };
        let mut acc = if c.argc >= 2 {
            agent.arg(c, 1)
        } else {
            loop {
                if i >= len {
                    return Err(agent.type_error("Reduce of empty array with no initial value"));
                }
                let k = index(i);
                i += 1.0;
                if agent.has_index(o, k) {
                    break agent.get_index(o, k)?;
                }
            }
        };
        let base = agent.vm.temps.len();
        let result = (|| {
            while i < len {
                agent.tick()?;
                let k = index(i);
                if agent.has_index(o, k) {
                    let v = agent.get_index(o, k)?;
                    agent.vm.temps.push(acc);
                    acc = agent.call_value(f, Value::UNDEFINED, &[acc, v, Value::number(k), o])?;
                    agent.vm.temps.pop();
                }
                i += 1.0;
            }
            Ok(acc)
        })();
        agent.vm.temps.truncate(base);
        result
    })
}

fn reduce(agent: &mut Agent, c: Call) -> Res<Value> {
    reduce_with(agent, c, false)
}

fn reduce_right(agent: &mut Agent, c: Call) -> Res<Value> {
    reduce_with(agent, c, true)
}

impl Agent {
    /// FlattenIntoArray (section 23.1.3.13.1), with a work stack of its
    /// own rather than recursion: nesting costs no Rust stack.
    fn flatten_into(
        &mut self,
        target: Value,
        source: Value,
        source_len: f64,
        depth: f64,
        mapper: Option<(Value, Value)>,
    ) -> Res<()> {
        let base = self.vm.temps.len();
        let result = (|| {
            // (array, its length, next index, depth left)
            let mut work = vec![(source, source_len, 0.0, depth)];
            let mut n = 0.0;
            while let Some((src, len, k, d)) = work.pop() {
                if k >= len {
                    continue;
                }
                work.push((src, len, k + 1.0, d));
                self.tick()?;
                if !self.has_index(src, k) {
                    continue;
                }
                let mut v = self.get_index(src, k)?;
                if let Some((f, this_arg)) = mapper.filter(|_| work.len() == 1) {
                    v = self.call_value(f, this_arg, &[v, Value::number(k), src])?;
                }
                if d > 0.0 && self.is_array(v) {
                    self.vm.temps.push(v);
                    let l = self.length_of(v)?;
                    work.push((v, l, 0.0, d - 1.0));
                } else {
                    if n >= MAX_SAFE {
                        return Err(self.type_error("Invalid array length"));
                    }
                    self.create_index(target, n, v)?;
                    n += 1.0;
                }
            }
            Ok(())
        })();
        self.vm.temps.truncate(base);
        result
    }
}

fn flat(agent: &mut Agent, c: Call) -> Res<Value> {
    let (o, len) = this_object(agent, c)?;
    rooted(agent, o, |agent| {
        let d = agent.arg(c, 0);
        let depth = if d.is_undefined() {
            1.0
        } else {
            agent.integer(d)?.max(0.0)
        };
        let a = agent.array_species_create(o, 0.0)?;
        rooted(agent, a, |agent| {
            agent.flatten_into(a, o, len, depth, None)?;
            Ok(a)
        })
    })
}

fn flat_map(agent: &mut Agent, c: Call) -> Res<Value> {
    let (o, len) = this_object(agent, c)?;
    rooted(agent, o, |agent| {
        let f = agent.callback(agent.arg(c, 0))?;
        let this_arg = agent.arg(c, 1);
        let a = agent.array_species_create(o, 0.0)?;
        rooted(agent, a, |agent| {
            agent.flatten_into(a, o, len, 1.0, Some((f, this_arg)))?;
            Ok(a)
        })
    })
}

/// `Array.prototype[@@unscopables]`: a null-prototype object of `true`s.
pub(crate) fn make_unscopables(agent: &mut Agent) -> Res<Value> {
    let o = agent.alloc_object(ObjectCell::new(Value::NULL, ObjectKind::Ordinary))?;
    for name in UNSCOPABLES {
        let key = agent.name_key(Name::Str(name));
        agent.create_data_property(o, key, Value::TRUE);
    }
    Ok(o)
}
