//! `Iterator` and the iterator helpers (ECMA-262 section 27.1.3 and the
//! iterator sequencing, joint iteration, `join` and `includes`
//! proposals the reference browser ships): the abstract constructor,
//! `Iterator.from`, `concat`, `zip` and `zipKeyed`, `%IteratorPrototype%`'s
//! methods, and the helper objects they make. A helper is a native
//! object with a generator's states (suspended at its start or at a
//! yield, running, completed): `next` steps it, `return` closes what it
//! iterates. The messages are the reference browser's (the J4.4 probe,
//! Chrome 158).

use super::{CONFIGURABLE, Call, Entry, HIDDEN, Name, What, Wk, entry, method};
use crate::agent::Agent;
use crate::heap::ObjectCell;
use crate::interp::{Abrupt, Res, messages};
use crate::object::ObjectKind;
use crate::string::PropertyKey;
use crate::value::Value;

/// `Iterator`.
pub(crate) const ITERATOR: &[Entry] = &[
    method("from", 1, from),
    method("concat", 0, concat),
    method("zip", 1, zip),
    method("zipKeyed", 1, zip_keyed),
];

/// `%IteratorPrototype%`, in the reference browser's order.
pub(crate) const ITERATOR_PROTO: &[Entry] = &[
    entry(
        Name::Str("constructor"),
        What::Accessor(get_constructor, set_constructor),
        CONFIGURABLE,
    ),
    method("reduce", 1, reduce),
    method("toArray", 0, to_array),
    method("forEach", 1, for_each),
    method("some", 1, some),
    method("every", 1, every),
    method("find", 1, find),
    method("map", 1, map),
    method("filter", 1, filter),
    method("take", 1, take),
    method("drop", 1, drop),
    method("flatMap", 1, flat_map),
    method("join", 1, join),
    method("includes", 1, includes),
    entry(
        Name::Sym(Wk::Iterator),
        What::Method(0, iterator_self),
        HIDDEN,
    ),
    entry(
        Name::Sym(Wk::ToStringTag),
        What::Accessor(get_tag, set_tag),
        CONFIGURABLE,
    ),
    entry(Name::Sym(Wk::Dispose), What::Method(0, dispose), HIDDEN),
];

/// `%IteratorHelperPrototype%`.
pub(crate) const ITERATOR_HELPER_PROTO: &[Entry] = &[
    method("next", 0, helper_next),
    method("return", 0, helper_return),
    entry(
        Name::Sym(Wk::ToStringTag),
        What::Str("Iterator Helper"),
        CONFIGURABLE,
    ),
];

/// `%WrapForValidIteratorPrototype%`.
pub(crate) const WRAP_PROTO: &[Entry] = &[
    method("next", 0, wrap_next),
    method("return", 0, wrap_return),
];

/// What a helper does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Map = 0,
    Filter = 1,
    Take = 2,
    Drop = 3,
    FlatMap = 4,
    Concat = 5,
    Zip = 6,
}

impl Op {
    fn of(v: Value) -> Op {
        match v.as_int() {
            Some(1) => Op::Filter,
            Some(2) => Op::Take,
            Some(3) => Op::Drop,
            Some(4) => Op::FlatMap,
            Some(5) => Op::Concat,
            Some(6) => Op::Zip,
            _ => Op::Map,
        }
    }
}

/// A helper's state, its generator's.
const START: i32 = 0;
const YIELDED: i32 = 1;
const RUNNING: i32 = 2;
const DONE: i32 = 3;

/// A helper's internal slots.
const S_OP: usize = 0;
/// The iterator it iterates (for `concat`, the items' array; for `zip`,
/// the iterators' array).
const S_ITER: usize = 1;
/// Its `next` (for `zip`, the `next` methods' array).
const S_NEXT: usize = 2;
/// The callback, or the count left (`take`, `drop`), or the padding
/// (`zip`).
const S_ARG: usize = 3;
/// The counter (for `concat`, the next item's index).
const S_COUNT: usize = 4;
const S_STATE: usize = 5;
/// `flatMap`'s and `concat`'s inner iterator and its `next`.
const S_INNER: usize = 6;
const S_INNER_NEXT: usize = 7;
/// `zip`'s mode (0 shortest, 1 longest, 2 strict) and keys (an array, or
/// undefined for `zip`).
const S_MODE: usize = 8;
const S_KEYS: usize = 9;

/// `zip`'s modes.
const SHORTEST: i32 = 0;
const LONGEST: i32 = 1;
const STRICT: i32 = 2;

fn iterator_self(_: &mut Agent, c: Call) -> Res<Value> {
    Ok(c.this)
}

impl Agent {
    /// A helper's slot.
    fn hslot(&self, o: Value, i: usize) -> Value {
        o.as_object()
            .and_then(|o| self.heap.objects.get(o))
            .and_then(|c| c.internal().get(i).copied())
            .unwrap_or(Value::UNDEFINED)
    }

    fn set_hslot(&mut self, o: Value, i: usize, v: Value) {
        if let Some(cell) = o.as_object().and_then(|o| self.heap.objects.get_mut(o)) {
            cell.set_internal(i, v);
        }
    }

    /// `this` as an object, or `method`'s "called on non-object".
    fn iter_this(&mut self, this: Value, method: &str) -> Res<Value> {
        if this.as_object().is_some() {
            Ok(this)
        } else {
            Err(self.type_error(format!("{method} called on non-object")))
        }
    }

    /// GetIteratorDirect (section 7.4.4): `o`'s `next`.
    fn next_of(&mut self, o: Value) -> Res<Value> {
        let key = self.realm().next_key;
        self.get_value(o, key)
    }

    /// The TypeError a value that is not callable throws: "number 1 is
    /// not a function".
    fn not_callable(&mut self, v: Value) -> Abrupt {
        let text = self.typed_display(v);
        self.type_error(messages::not_a_function(&text))
    }

    /// Closes `iterator` after `e` and gives `e` back (IfAbruptCloseIterator
    /// on a throw).
    fn close_after(&mut self, iterator: Value, e: Abrupt) -> Abrupt {
        match self.iterator_close_quiet(iterator) {
            Err(t) => t,
            Ok(()) => e,
        }
    }

    /// `f` called on `iterator`'s next value and the counter, the
    /// iterator closed if it throws.
    fn call_closing(&mut self, f: Value, iterator: Value, args: &[Value]) -> Res<Value> {
        match self.call_value(f, Value::UNDEFINED, args) {
            Ok(v) => Ok(v),
            Err(e) => Err(self.close_after(iterator, e)),
        }
    }

    /// GetIteratorFlattenable (section 7.4.5): `v`'s iterator and its
    /// `next`; strings are iterated only when `strings`.
    fn flattenable(&mut self, v: Value, strings: bool, method: &str) -> Res<(Value, Value)> {
        // The reference browser's messages: `zip`'s items are "not
        // iterable", the others' arguments "called on non-object".
        let item = method.starts_with("Iterator.zip");
        if v.as_object().is_none() && !(strings && v.as_string().is_some()) {
            if item {
                let text = self.display(v);
                return Err(self.type_error(messages::not_iterable(&text)));
            }
            return Err(self.type_error(format!("{method} called on non-object")));
        }
        let key = self.well_known_key(Wk::Iterator);
        let m = self.get_method(v, key)?;
        let iterator = if m.is_undefined() {
            v
        } else {
            self.call_value(m, v, &[])?
        };
        if iterator.as_object().is_none() {
            if item || method == "Iterator.from" {
                let text = self.display(v);
                return Err(self.type_error(messages::not_iterable(&text)));
            }
            return Err(self.type_error(format!("{method} called on non-object")));
        }
        self.vm.temps.push(iterator);
        let next = self.next_of(iterator);
        self.vm.temps.pop();
        Ok((iterator, next?))
    }

    /// A helper of `op` over `iterator`, with `arg`.
    fn new_helper(&mut self, op: Op, iterator: Value, next: Value, arg: Value) -> Res<Value> {
        let proto = self.realm().intrinsics.iterator_helper_prototype;
        self.alloc_object(ObjectCell::with_internal(
            proto,
            ObjectKind::IteratorHelper,
            vec![
                Value::int(op as i32),
                iterator,
                next,
                arg,
                Value::int(0),
                Value::int(START),
                Value::UNDEFINED,
                Value::UNDEFINED,
                Value::int(SHORTEST),
                Value::UNDEFINED,
            ],
        ))
    }

    /// The counter, then counted on.
    fn count(&mut self, h: Value) -> Value {
        let n = self.hslot(h, S_COUNT);
        let next = n.as_number().unwrap_or(0.0) + 1.0;
        self.set_hslot(h, S_COUNT, Value::number(next));
        n
    }

    /// One step of helper `h`: its next value, or `None` when it is done.
    fn helper_step(&mut self, h: Value) -> Res<Option<Value>> {
        let it = self.hslot(h, S_ITER);
        let next = self.hslot(h, S_NEXT);
        let arg = self.hslot(h, S_ARG);
        match Op::of(self.hslot(h, S_OP)) {
            Op::Map => {
                let Some(v) = self.iterator_step(it, next)? else {
                    return Ok(None);
                };
                let n = self.count(h);
                self.call_closing(arg, it, &[v, n]).map(Some)
            }
            Op::Filter => loop {
                let Some(v) = self.iterator_step(it, next)? else {
                    return Ok(None);
                };
                let n = self.count(h);
                let keep = self.call_closing(arg, it, &[v, n])?;
                if self.to_boolean(keep) {
                    return Ok(Some(v));
                }
            },
            Op::Take => {
                let left = arg.as_number().unwrap_or(0.0);
                if left == 0.0 {
                    self.iterator_close(it)?;
                    return Ok(None);
                }
                self.set_hslot(h, S_ARG, Value::number(left - 1.0));
                self.iterator_step(it, next)
            }
            Op::Drop => {
                let mut left = arg.as_number().unwrap_or(0.0);
                while left > 0.0 {
                    left -= 1.0;
                    self.set_hslot(h, S_ARG, Value::number(left));
                    if self.iterator_step(it, next)?.is_none() {
                        return Ok(None);
                    }
                }
                self.iterator_step(it, next)
            }
            Op::FlatMap => loop {
                let inner = self.hslot(h, S_INNER);
                if !inner.is_undefined() {
                    let inner_next = self.hslot(h, S_INNER_NEXT);
                    match self.iterator_step(inner, inner_next) {
                        Ok(Some(v)) => return Ok(Some(v)),
                        Ok(None) => {
                            self.set_hslot(h, S_INNER, Value::UNDEFINED);
                            continue;
                        }
                        Err(e) => return Err(self.close_after(it, e)),
                    }
                }
                let Some(v) = self.iterator_step(it, next)? else {
                    return Ok(None);
                };
                let n = self.count(h);
                let mapped = self.call_closing(arg, it, &[v, n])?;
                self.vm.temps.push(mapped);
                let inner = self.flattenable(mapped, false, "Iterator.prototype.flatMap");
                self.vm.temps.pop();
                match inner {
                    Ok((i, n)) => {
                        self.set_hslot(h, S_INNER, i);
                        self.set_hslot(h, S_INNER_NEXT, n);
                    }
                    Err(e) => return Err(self.close_after(it, e)),
                }
            },
            Op::Concat => loop {
                let inner = self.hslot(h, S_INNER);
                if !inner.is_undefined() {
                    let inner_next = self.hslot(h, S_INNER_NEXT);
                    if let Some(v) = self.iterator_step(inner, inner_next)? {
                        return Ok(Some(v));
                    }
                    self.set_hslot(h, S_INNER, Value::UNDEFINED);
                }
                // The next item: its value and its iterator method.
                let k = self.hslot(h, S_COUNT).as_number().unwrap_or(0.0) as u32;
                let items = it;
                let len = self.array_len(items);
                if k * 2 >= len {
                    return Ok(None);
                }
                self.set_hslot(h, S_COUNT, Value::number(f64::from(k + 1)));
                let item = self.get_value(items, PropertyKey::Index(k * 2))?;
                let m = self.get_value(items, PropertyKey::Index(k * 2 + 1))?;
                let i = self.call_value(m, item, &[])?;
                if i.as_object().is_none() {
                    let text = self.display(i);
                    return Err(self.type_error(messages::not_iterable(&text)));
                }
                self.vm.temps.push(i);
                let n = self.next_of(i);
                self.vm.temps.pop();
                let n = n?;
                self.set_hslot(h, S_INNER, i);
                self.set_hslot(h, S_INNER_NEXT, n);
            },
            Op::Zip => self.zip_step(h),
        }
    }

    /// The length of an array the helpers keep.
    fn array_len(&self, a: Value) -> u32 {
        a.as_object()
            .and_then(|o| self.heap.objects.get(o))
            .and_then(|c| c.array_length())
            .map_or(0, |(len, _)| len)
    }

    /// Closes a helper's iterators (it is completed): `flatMap`'s inner
    /// one first, then the one it iterates; `zip`'s from the last.
    fn helper_close(&mut self, h: Value) -> Res<()> {
        let op = Op::of(self.hslot(h, S_OP));
        let it = self.hslot(h, S_ITER);
        match op {
            Op::Concat => {
                let inner = self.hslot(h, S_INNER);
                if !inner.is_undefined() {
                    self.iterator_close(inner)?;
                }
                Ok(())
            }
            Op::Zip => self.zip_close_all(h, None),
            Op::FlatMap => {
                let inner = self.hslot(h, S_INNER);
                if !inner.is_undefined()
                    && let Err(e) = self.iterator_close(inner)
                {
                    return Err(self.close_after(it, e));
                }
                self.iterator_close(it)
            }
            _ => self.iterator_close(it),
        }
    }

    /// `this` as an iterator helper, or the TypeError.
    fn this_helper(&mut self, this: Value, method: &str) -> Res<Value> {
        let is = this
            .as_object()
            .and_then(|o| self.heap.objects.get(o))
            .is_some_and(|c| c.kind() == ObjectKind::IteratorHelper);
        if is {
            Ok(this)
        } else {
            Err(self.incompatible(&format!("Iterator Helper.prototype.{method}"), this))
        }
    }
}

/// `%IteratorHelperPrototype%.next` (section 27.1.2.1.1).
fn helper_next(agent: &mut Agent, c: Call) -> Res<Value> {
    let h = agent.this_helper(c.this, "next")?;
    match agent.hslot(h, S_STATE).as_int() {
        Some(RUNNING) => return Err(agent.type_error(messages::GENERATOR_RUNNING)),
        Some(DONE) => return agent.iter_result(Value::UNDEFINED, true),
        _ => {}
    }
    agent.set_hslot(h, S_STATE, Value::int(RUNNING));
    match agent.helper_step(h) {
        Ok(Some(v)) => {
            agent.set_hslot(h, S_STATE, Value::int(YIELDED));
            agent.iter_result(v, false)
        }
        Ok(None) => {
            agent.set_hslot(h, S_STATE, Value::int(DONE));
            agent.iter_result(Value::UNDEFINED, true)
        }
        Err(e) => {
            agent.set_hslot(h, S_STATE, Value::int(DONE));
            Err(e)
        }
    }
}

/// `%IteratorHelperPrototype%.return` (section 27.1.2.1.2).
fn helper_return(agent: &mut Agent, c: Call) -> Res<Value> {
    let h = agent.this_helper(c.this, "return")?;
    match agent.hslot(h, S_STATE).as_int() {
        Some(RUNNING) => return Err(agent.type_error(messages::GENERATOR_RUNNING)),
        Some(DONE) => return agent.iter_result(Value::UNDEFINED, true),
        Some(START) => {
            // Not started: only what it iterates is closed.
            agent.set_hslot(h, S_STATE, Value::int(DONE));
            let op = Op::of(agent.hslot(h, S_OP));
            if op == Op::Zip {
                agent.zip_close_all(h, None)?;
            } else if op != Op::Concat {
                let it = agent.hslot(h, S_ITER);
                agent.iterator_close(it)?;
            }
            return agent.iter_result(Value::UNDEFINED, true);
        }
        _ => {}
    }
    // Running while it closes: a call back into it is refused.
    agent.set_hslot(h, S_STATE, Value::int(RUNNING));
    let closed = agent.helper_close(h);
    agent.set_hslot(h, S_STATE, Value::int(DONE));
    closed?;
    agent.iter_result(Value::UNDEFINED, true)
}

/// The constructor (section 27.1.3.1): abstract.
pub(crate) fn construct(agent: &mut Agent, c: Call) -> Res<Value> {
    if c.new_target.is_undefined() {
        return Err(agent.type_error("Constructor Iterator requires 'new'"));
    }
    if c.new_target == agent.realm().intrinsics.iterator {
        return Err(agent.type_error("Abstract class Iterator not directly constructable"));
    }
    let proto = agent.prototype_from(c.new_target, |r| r.intrinsics.iterator_prototype)?;
    agent.alloc_object(ObjectCell::new(proto, ObjectKind::Ordinary))
}

/// SetterThatIgnoresPrototypeProperties (section 10.4.7.2 of the
/// proposal): the home object's own property is read-only; any other
/// object gets its own.
fn set_ignoring_home(
    agent: &mut Agent,
    this: Value,
    key: PropertyKey,
    v: Value,
    name: &str,
) -> Res<Value> {
    let home = agent.realm().intrinsics.iterator_prototype;
    if this.as_object().is_none() {
        return Err(agent.type_error(format!("set Iterator.prototype{name} called on non-object")));
    }
    if this == home {
        let name = agent.key_display(key);
        return Err(agent.type_error(messages::read_only(&name, "[object Object]")));
    }
    if agent.own_property(this, key)?.is_none() {
        agent.create_data_property(this, key, v);
    } else {
        agent.put(this, key, v)?;
    }
    Ok(Value::UNDEFINED)
}

fn get_constructor(agent: &mut Agent, _: Call) -> Res<Value> {
    Ok(agent.realm().intrinsics.iterator)
}

fn set_constructor(agent: &mut Agent, c: Call) -> Res<Value> {
    let key = agent.realm().constructor_key;
    set_ignoring_home(agent, c.this, key, agent.arg(c, 0), ".constructor")
}

fn get_tag(agent: &mut Agent, _: Call) -> Res<Value> {
    agent.new_string("Iterator")
}

fn set_tag(agent: &mut Agent, c: Call) -> Res<Value> {
    let key = agent.well_known_key(Wk::ToStringTag);
    set_ignoring_home(agent, c.this, key, agent.arg(c, 0), "[Symbol.toStringTag]")
}

/// `%IteratorPrototype%[@@dispose]`: `return`, if it has one.
fn dispose(agent: &mut Agent, c: Call) -> Res<Value> {
    let key = agent.realm().return_key;
    let m = agent.get_method(c.this, key)?;
    if !m.is_undefined() {
        agent.call_value(m, c.this, &[])?;
    }
    Ok(Value::UNDEFINED)
}

/// The helpers that take a callback: `this`, the callback checked (the
/// iterator closed if it is not callable), then `next`.
fn with_callback(agent: &mut Agent, c: Call, method: &str) -> Res<(Value, Value, Value)> {
    let o = agent.iter_this(c.this, method)?;
    let f = agent.arg(c, 0);
    if !agent.is_callable(f) {
        let e = agent.not_callable(f);
        return Err(agent.close_after(o, e));
    }
    let next = agent.next_of(o)?;
    Ok((o, next, f))
}

fn map(agent: &mut Agent, c: Call) -> Res<Value> {
    let (o, next, f) = with_callback(agent, c, "Iterator.prototype.map")?;
    agent.new_helper(Op::Map, o, next, f)
}

fn filter(agent: &mut Agent, c: Call) -> Res<Value> {
    let (o, next, f) = with_callback(agent, c, "Iterator.prototype.filter")?;
    agent.new_helper(Op::Filter, o, next, f)
}

fn flat_map(agent: &mut Agent, c: Call) -> Res<Value> {
    let (o, next, f) = with_callback(agent, c, "Iterator.prototype.flatMap")?;
    agent.new_helper(Op::FlatMap, o, next, f)
}

/// `take` and `drop` (sections 27.1.4.11 and 27.1.4.3): the count's
/// checks, the iterator closed if they fail.
fn with_limit(agent: &mut Agent, c: Call, method: &str) -> Res<(Value, Value, f64)> {
    let o = agent.iter_this(c.this, method)?;
    let checked = (|| {
        let n = agent.number(agent.arg(c, 0))?;
        if n.is_nan() {
            return Err(agent.range_error("NaN must be positive"));
        }
        let n = n.trunc();
        if n < 0.0 {
            let text = crate::number::number_to_string(n);
            return Err(agent.range_error(format!("{text} must be positive")));
        }
        if n.is_finite() && n > super::typed::MAX_SAFE {
            let text = crate::number::number_to_string(n);
            return Err(agent.range_error(format!("{text} must be positive")));
        }
        Ok(n)
    })();
    let n = match checked {
        Ok(n) => n,
        Err(e) => return Err(agent.close_after(o, e)),
    };
    let next = agent.next_of(o)?;
    Ok((o, next, n))
}

fn take(agent: &mut Agent, c: Call) -> Res<Value> {
    let (o, next, n) = with_limit(agent, c, "Iterator.prototype.take")?;
    agent.new_helper(Op::Take, o, next, Value::number(n))
}

fn drop(agent: &mut Agent, c: Call) -> Res<Value> {
    let (o, next, n) = with_limit(agent, c, "Iterator.prototype.drop")?;
    agent.new_helper(Op::Drop, o, next, Value::number(n))
}

/// `reduce` (section 27.1.4.8).
fn reduce(agent: &mut Agent, c: Call) -> Res<Value> {
    let (o, next, f) = with_callback(agent, c, "Iterator.prototype.reduce")?;
    let (mut acc, mut counter) = if c.argc < 2 {
        match agent.iterator_step(o, next)? {
            Some(v) => (v, 1.0),
            None => {
                return Err(agent.type_error("Reduce of a done iterator with no initial value"));
            }
        }
    } else {
        (agent.arg(c, 1), 0.0)
    };
    let base = agent.vm.temps.len();
    let r = (|| {
        loop {
            agent.vm.temps.push(acc);
            let Some(v) = agent.iterator_step(o, next)? else {
                return Ok(acc);
            };
            acc = agent.call_closing(f, o, &[acc, v, Value::number(counter)])?;
            agent.vm.temps.pop();
            counter += 1.0;
        }
    })();
    agent.vm.temps.truncate(base);
    r
}

/// `toArray` (section 27.1.4.12).
fn to_array(agent: &mut Agent, c: Call) -> Res<Value> {
    let o = agent.iter_this(c.this, "Iterator.prototype.toArray")?;
    let next = agent.next_of(o)?;
    let a = agent.array_create(0.0, None)?;
    agent.vm.temps.push(a);
    let r = (|| {
        while let Some(v) = agent.iterator_step(o, next)? {
            agent.array_push(a, v)?;
        }
        Ok(a)
    })();
    agent.vm.temps.pop();
    r
}

/// `forEach`, `some`, `every` and `find` (sections 27.1.4.5, 27.1.4.10,
/// 27.1.4.2, 27.1.4.4): the callback on each value until `stop` says
/// what to give.
fn each(
    agent: &mut Agent,
    c: Call,
    method: &str,
    stop: fn(&mut Agent, Value, Value) -> Option<Value>,
    otherwise: Value,
) -> Res<Value> {
    let (o, next, f) = with_callback(agent, c, method)?;
    let mut counter = 0.0;
    while let Some(v) = agent.iterator_step(o, next)? {
        let r = agent.call_closing(f, o, &[v, Value::number(counter)])?;
        if let Some(result) = stop(agent, v, r) {
            agent.vm.temps.push(result);
            let closed = agent.iterator_close(o);
            agent.vm.temps.pop();
            closed?;
            return Ok(result);
        }
        counter += 1.0;
    }
    Ok(otherwise)
}

fn for_each(agent: &mut Agent, c: Call) -> Res<Value> {
    each(
        agent,
        c,
        "Iterator.prototype.forEach",
        |_, _, _| None,
        Value::UNDEFINED,
    )
}

fn some(agent: &mut Agent, c: Call) -> Res<Value> {
    each(
        agent,
        c,
        "Iterator.prototype.some",
        |a, _, r| a.to_boolean(r).then_some(Value::TRUE),
        Value::FALSE,
    )
}

fn every(agent: &mut Agent, c: Call) -> Res<Value> {
    each(
        agent,
        c,
        "Iterator.prototype.every",
        |a, _, r| (!a.to_boolean(r)).then_some(Value::FALSE),
        Value::TRUE,
    )
}

fn find(agent: &mut Agent, c: Call) -> Res<Value> {
    each(
        agent,
        c,
        "Iterator.prototype.find",
        |a, v, r| a.to_boolean(r).then_some(v),
        Value::UNDEFINED,
    )
}

/// `join` (the iterator join proposal): the separator's ToString first
/// (the iterator closed if it throws), then each value's, null and
/// undefined as empty.
fn join(agent: &mut Agent, c: Call) -> Res<Value> {
    let o = agent.iter_this(c.this, "Iterator.prototype.join")?;
    let sep = agent.arg(c, 0);
    let sep = if sep.is_undefined() {
        vec![u16::from(b',')]
    } else {
        match agent.text_units(sep) {
            Ok(s) => s,
            Err(e) => return Err(agent.close_after(o, e)),
        }
    };
    let next = agent.next_of(o)?;
    let mut out: Vec<u16> = Vec::new();
    let mut first = true;
    while let Some(v) = agent.iterator_step(o, next)? {
        if !first {
            out.extend_from_slice(&sep);
        }
        first = false;
        if !v.is_nullish() {
            match agent.text_units(v) {
                Ok(s) => out.extend(s),
                Err(e) => return Err(agent.close_after(o, e)),
            }
        }
        if out.len() > super::string::MAX_LENGTH {
            let e = agent.range_error("Invalid string length");
            return Err(agent.close_after(o, e));
        }
    }
    agent.new_string_units(&out)
}

/// `includes` (the iterator includes proposal): the values skipped
/// first (a Number, integral, from 0 to 2^53 - 1 or +Infinity; the
/// iterator closed if it is not), then SameValueZero.
fn includes(agent: &mut Agent, c: Call) -> Res<Value> {
    let o = agent.iter_this(c.this, "Iterator.prototype.includes")?;
    let search = agent.arg(c, 0);
    let skipped = agent.arg(c, 1);
    let skip = if skipped.is_undefined() {
        0.0
    } else {
        let checked = match skipped.as_number() {
            Some(n) if n.is_nan() || (n.is_finite() && n.trunc() != n) => Err(()),
            None => Err(()),
            Some(n) if n < 0.0 || (n.is_finite() && n > super::typed::MAX_SAFE) => {
                let text = crate::number::number_to_string(n);
                Ok(Err(agent.range_error(format!("{text} must be positive"))))
            }
            Some(n) => Ok(Ok(n)),
        };
        let checked = match checked {
            Err(()) => {
                Err(agent.type_error("skippedElements argument must be undefined or an integer"))
            }
            Ok(r) => r,
        };
        match checked {
            Ok(n) => n,
            Err(e) => return Err(agent.close_after(o, e)),
        }
    };
    let next = agent.next_of(o)?;
    let mut left = skip;
    while let Some(v) = agent.iterator_step(o, next)? {
        if left > 0.0 {
            left -= 1.0;
            continue;
        }
        if agent.heap.same_value_zero(v, search) {
            agent.iterator_close(o)?;
            return Ok(Value::TRUE);
        }
    }
    Ok(Value::FALSE)
}

/// `Iterator.from` (section 27.1.3.2.1).
fn from(agent: &mut Agent, c: Call) -> Res<Value> {
    let (iterator, next) = agent.flattenable(agent.arg(c, 0), true, "Iterator.from")?;
    let ctor = agent.realm().intrinsics.iterator;
    agent.vm.temps.extend([iterator, next]);
    let has = agent.ordinary_has_instance(ctor, iterator);
    agent.vm.temps.truncate(agent.vm.temps.len() - 2);
    if has? {
        return Ok(iterator);
    }
    let wp = agent.realm().intrinsics.wrap_for_valid_iterator_prototype;
    agent.vm.temps.extend([iterator, next]);
    let w = agent.alloc_object(ObjectCell::with_internal(
        wp,
        ObjectKind::IteratorWrap,
        vec![iterator, next],
    ));
    agent.vm.temps.truncate(agent.vm.temps.len() - 2);
    w
}

/// `this` as a wrapped iterator's record.
fn wrapped(agent: &mut Agent, this: Value, method: &str) -> Res<(Value, Value)> {
    let found = this
        .as_object()
        .and_then(|o| agent.heap.objects.get(o))
        .filter(|c| c.kind() == ObjectKind::IteratorWrap)
        .map(|c| (c.internal().first().copied(), c.internal().get(1).copied()));
    match found {
        Some((Some(i), Some(n))) => Ok((i, n)),
        _ => Err(agent.incompatible(&format!("%WrapForValidIteratorPrototype%.{method}"), this)),
    }
}

fn wrap_next(agent: &mut Agent, c: Call) -> Res<Value> {
    let (i, n) = wrapped(agent, c.this, "next")?;
    if !agent.is_callable(n) {
        return Err(agent.not_callable(n));
    }
    agent.call_value(n, i, &[])
}

fn wrap_return(agent: &mut Agent, c: Call) -> Res<Value> {
    let (i, _) = wrapped(agent, c.this, "return")?;
    let key = agent.realm().return_key;
    let m = agent.get_method(i, key)?;
    if m.is_undefined() {
        return agent.iter_result(Value::UNDEFINED, true);
    }
    agent.call_value(m, i, &[])
}

/// `Iterator.concat` (the iterator sequencing proposal): each item an
/// object with an iterator method, read now and called as it is
/// reached.
fn concat(agent: &mut Agent, c: Call) -> Res<Value> {
    let items = agent.array_create(0.0, None)?;
    agent.vm.temps.push(items);
    let r = (|| {
        let key = agent.well_known_key(Wk::Iterator);
        for k in 0..c.argc {
            let item = agent.arg(c, k);
            if item.as_object().is_none() {
                return Err(agent.type_error("Iterator.concat called on non-object"));
            }
            let m = agent.get_value(item, key)?;
            if m.is_nullish() {
                let text = agent.display(item);
                return Err(agent.type_error(messages::not_iterable(&text)));
            }
            if !agent.is_callable(m) {
                let (a, t) = (agent.display(m), agent.display(item));
                return Err(agent.type_error(format!(
                    "'{a}' returned for property 'Symbol(Symbol.iterator)' of object '{t}' is not a function"
                )));
            }
            agent.array_push(items, item)?;
            agent.array_push(items, m)?;
        }
        agent.new_helper(Op::Concat, items, Value::UNDEFINED, Value::UNDEFINED)
    })();
    agent.vm.temps.pop();
    r
}

impl Agent {
    /// IteratorCloseAll over `zip`'s open iterators after the abrupt
    /// completion `e`: what goes on.
    fn zip_abort(&mut self, h: Value, e: Abrupt) -> Abrupt {
        match self.zip_close_all(h, Some(e)) {
            Err(e) => e,
            Ok(()) => Abrupt::Throw,
        }
    }

    /// IteratorCloseAll over `zip`'s open iterators, from the last; `e`
    /// the abrupt completion that closes them, or `None` for a normal
    /// one (whose closing errors then go on).
    fn zip_close_all(&mut self, h: Value, e: Option<Abrupt>) -> Res<()> {
        let iters = self.hslot(h, S_ITER);
        let n = self.array_len(iters);
        let mut err = e;
        for k in (0..n).rev() {
            let it = self.get_value(iters, PropertyKey::Index(k))?;
            if it.is_undefined() || it.is_null() {
                continue;
            }
            self.set_value_at(iters, k, Value::NULL)?;
            match err {
                Some(e) => err = Some(self.close_after(it, e)),
                None => {
                    if let Err(e) = self.iterator_close(it) {
                        err = Some(e);
                    }
                }
            }
        }
        match err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    fn set_value_at(&mut self, a: Value, k: u32, v: Value) -> Res<()> {
        self.define_value(a, PropertyKey::Index(k), v)
    }

    /// One step of `zip`: each open iterator's next value (the padding
    /// for one done in "longest"), as an array or, for `zipKeyed`, an
    /// object of null prototype.
    fn zip_step(&mut self, h: Value) -> Res<Option<Value>> {
        let iters = self.hslot(h, S_ITER);
        let nexts = self.hslot(h, S_NEXT);
        let padding = self.hslot(h, S_ARG);
        let mode = self.hslot(h, S_MODE).as_int().unwrap_or(SHORTEST);
        let n = self.array_len(iters);
        if n == 0 {
            return Ok(None);
        }
        let results = self.array_create(0.0, None)?;
        self.vm.temps.push(results);
        let r = (|| {
            for k in 0..n {
                let it = self.get_value(iters, PropertyKey::Index(k))?;
                let v = if it.is_null() {
                    self.get_value(padding, PropertyKey::Index(k))?
                } else {
                    let next = self.get_value(nexts, PropertyKey::Index(k))?;
                    let stepped = self.iterator_step(it, next);
                    match stepped {
                        Err(e) => {
                            self.set_value_at(iters, k, Value::NULL)?;
                            return Err(self.zip_abort(h, e));
                        }
                        Ok(Some(v)) => v,
                        Ok(None) => {
                            self.set_value_at(iters, k, Value::NULL)?;
                            match mode {
                                SHORTEST => {
                                    self.zip_close_all(h, None)?;
                                    return Ok(None);
                                }
                                STRICT => {
                                    if k != 0 {
                                        let e = self.type_error(ZIP_UNEVEN);
                                        return Err(self.zip_abort(h, e));
                                    }
                                    // The rest must be done too.
                                    for j in 1..n {
                                        let other = self.get_value(iters, PropertyKey::Index(j))?;
                                        if other.is_null() {
                                            continue;
                                        }
                                        let next = self.get_value(nexts, PropertyKey::Index(j))?;
                                        match self.iterator_step_done(other, next) {
                                            Err(e) => {
                                                self.set_value_at(iters, j, Value::NULL)?;
                                                return Err(self.zip_abort(h, e));
                                            }
                                            Ok(true) => self.set_value_at(iters, j, Value::NULL)?,
                                            Ok(false) => {
                                                let e = self.type_error(ZIP_UNEVEN);
                                                return Err(self.zip_abort(h, e));
                                            }
                                        }
                                    }
                                    return Ok(None);
                                }
                                _ => {
                                    let open = (0..n).any(|j| {
                                        self.get_value(iters, PropertyKey::Index(j))
                                            .is_ok_and(|v| !v.is_null())
                                    });
                                    if !open {
                                        return Ok(None);
                                    }
                                    self.get_value(padding, PropertyKey::Index(k))?
                                }
                            }
                        }
                    }
                };
                self.array_push(results, v)?;
            }
            let keys = self.hslot(h, S_KEYS);
            if keys.is_undefined() {
                return Ok(Some(results));
            }
            // zipKeyed: an object of null prototype.
            let o = self.alloc_object(ObjectCell::new(Value::NULL, ObjectKind::Ordinary))?;
            self.vm.temps.push(o);
            let filled = (|| {
                for k in 0..n {
                    let key = self.get_value(keys, PropertyKey::Index(k))?;
                    let key = self.property_key(key)?;
                    let v = self.get_value(results, PropertyKey::Index(k))?;
                    self.create_data_property(o, key, v);
                }
                Ok(())
            })();
            self.vm.temps.pop();
            filled.map(|()| Some(o))
        })();
        self.vm.temps.pop();
        r
    }

    /// IteratorStep's doneness alone.
    fn iterator_step_done(&mut self, it: Value, next: Value) -> Res<bool> {
        Ok(self.iterator_step(it, next)?.is_none())
    }
}

const ZIP_UNEVEN: &str =
    "Iterators passed to Iterator.zip in 'strict' mode must have the same length";

impl Agent {
    /// `zip`'s and `zipKeyed`'s options: the mode, and the padding
    /// option for "longest".
    fn zip_options(&mut self, options: Value, method: &str) -> Res<(i32, Value)> {
        if options.is_undefined() {
            return Ok((SHORTEST, Value::UNDEFINED));
        }
        if options.as_object().is_none() {
            return Err(self.type_error("options called on non-object"));
        }
        let key = self.name_key(Name::Str("mode"));
        let m = self.get_value(options, key)?;
        let mode = if m.is_undefined() {
            SHORTEST
        } else {
            let text = if m.as_string().is_some() {
                String::from_utf16_lossy(&self.text_units(m)?)
            } else {
                String::new()
            };
            match text.as_str() {
                "shortest" => SHORTEST,
                "longest" => LONGEST,
                "strict" => STRICT,
                _ => {
                    return Err(self.type_error(format!(
                        "{method} mode must be 'shortest', 'longest', or 'strict'"
                    )));
                }
            }
        };
        let mut padding = Value::UNDEFINED;
        if mode == LONGEST {
            let key = self.name_key(Name::Str("padding"));
            padding = self.get_value(options, key)?;
            if !padding.is_undefined() && padding.as_object().is_none() {
                return Err(self.type_error("padding called on non-object"));
            }
        }
        Ok((mode, padding))
    }

    /// A `zip` helper over the iterators and nexts in `iters` and
    /// `nexts`, with `padding` (an array) and `keys` (`zipKeyed`'s).
    fn new_zip(
        &mut self,
        iters: Value,
        nexts: Value,
        mode: i32,
        padding: Value,
        keys: Value,
    ) -> Res<Value> {
        let h = self.new_helper(Op::Zip, iters, nexts, padding)?;
        self.set_hslot(h, S_MODE, Value::int(mode));
        self.set_hslot(h, S_KEYS, keys);
        Ok(h)
    }

    /// Closes every iterator in `iters` after `e`, from the last.
    fn close_list_after(&mut self, iters: Value, e: Abrupt) -> Abrupt {
        let n = self.array_len(iters);
        let mut e = e;
        for k in (0..n).rev() {
            if let Ok(it) = self.get_value(iters, PropertyKey::Index(k)) {
                e = self.close_after(it, e);
            }
        }
        e
    }
}

/// `Iterator.zip` (the joint iteration proposal).
fn zip(agent: &mut Agent, c: Call) -> Res<Value> {
    const M: &str = "Iterator.zip";
    let iterables = agent.arg(c, 0);
    if iterables.as_object().is_none() {
        return Err(agent.type_error(format!("{M} called on non-object")));
    }
    let (mode, padding_option) = agent.zip_options(agent.arg(c, 1), M)?;
    let base = agent.vm.temps.len();
    let r = (|| {
        let iters = agent.array_create(0.0, None)?;
        agent.vm.temps.push(iters);
        let nexts = agent.array_create(0.0, None)?;
        agent.vm.temps.push(nexts);
        let (input, input_next) = agent.get_iterator_typed(iterables)?;
        agent.vm.temps.extend([input, input_next]);
        loop {
            let v = match agent.iterator_step(input, input_next) {
                Ok(Some(v)) => v,
                Ok(None) => break,
                Err(e) => return Err(agent.close_list_after(iters, e)),
            };
            agent.vm.temps.push(v);
            let flat = agent.flattenable(v, false, M);
            agent.vm.temps.pop();
            match flat {
                Ok((i, n)) => {
                    agent.vm.temps.extend([i, n]);
                    let pushed = agent
                        .array_push(iters, i)
                        .and_then(|()| agent.array_push(nexts, n));
                    agent.vm.temps.truncate(agent.vm.temps.len() - 2);
                    pushed?;
                }
                Err(e) => {
                    // The iterators from the last, the iterables' last.
                    let e = agent.close_list_after(iters, e);
                    return Err(agent.close_after(input, e));
                }
            }
        }
        let count = agent.array_len(iters);
        let padding = agent.zip_padding(iters, padding_option, count, None)?;
        // Rooted while the helper is made (the temps are cut after).
        agent.vm.temps.push(padding);
        agent.new_zip(iters, nexts, mode, padding, Value::UNDEFINED)
    })();
    agent.vm.temps.truncate(base);
    r
}

impl Agent {
    /// The padding for "longest": undefined for each, or the padding
    /// option's values (an iterable for `zip`, the keys' values of an
    /// object for `zipKeyed`), the iterators closed if it throws.
    fn zip_padding(
        &mut self,
        iters: Value,
        option: Value,
        count: u32,
        keys: Option<Value>,
    ) -> Res<Value> {
        let padding = self.array_create(0.0, None)?;
        self.vm.temps.push(padding);
        let r = (|| {
            if option.is_undefined() {
                for _ in 0..count {
                    self.array_push(padding, Value::UNDEFINED)?;
                }
                return Ok(padding);
            }
            if let Some(keys) = keys {
                for k in 0..count {
                    let key = self.get_value(keys, PropertyKey::Index(k))?;
                    let key = self.property_key(key)?;
                    match self.get_value(option, key) {
                        Ok(v) => self.array_push(padding, v)?,
                        Err(e) => return Err(self.close_list_after(iters, e)),
                    }
                }
                return Ok(padding);
            }
            let (pi, pn) = match self.get_iterator_typed(option) {
                Ok(p) => p,
                Err(e) => return Err(self.close_list_after(iters, e)),
            };
            self.vm.temps.extend([pi, pn]);
            let mut using = true;
            for _ in 0..count {
                let v = if using {
                    match self.iterator_step(pi, pn) {
                        Ok(Some(v)) => v,
                        Ok(None) => {
                            using = false;
                            Value::UNDEFINED
                        }
                        Err(e) => return Err(self.close_list_after(iters, e)),
                    }
                } else {
                    Value::UNDEFINED
                };
                self.array_push(padding, v)?;
            }
            if using && let Err(e) = self.iterator_close(pi) {
                return Err(self.close_list_after(iters, e));
            }
            Ok(padding)
        })();
        self.vm.temps.pop();
        r
    }
}

/// `Iterator.zipKeyed` (the joint iteration proposal).
fn zip_keyed(agent: &mut Agent, c: Call) -> Res<Value> {
    const M: &str = "Iterator.zipKeyed";
    let iterables = agent.arg(c, 0);
    if iterables.as_object().is_none() {
        return Err(agent.type_error(format!("{M} called on non-object")));
    }
    let (mode, padding_option) = agent.zip_options(agent.arg(c, 1), M)?;
    let base = agent.vm.temps.len();
    let r = (|| {
        let iters = agent.array_create(0.0, None)?;
        agent.vm.temps.push(iters);
        let nexts = agent.array_create(0.0, None)?;
        agent.vm.temps.push(nexts);
        let keys = agent.array_create(0.0, None)?;
        agent.vm.temps.push(keys);
        let own = agent.own_property_keys(iterables)?;
        for key in own {
            let enumerable = match agent.own_property(iterables, key) {
                Ok(p) => p.is_some_and(|p| p.enumerable()),
                Err(e) => return Err(agent.close_list_after(iters, e)),
            };
            if !enumerable {
                continue;
            }
            let v = match agent.get_value(iterables, key) {
                Ok(v) => v,
                Err(e) => return Err(agent.close_list_after(iters, e)),
            };
            if v.is_undefined() {
                continue;
            }
            agent.vm.temps.push(v);
            let flat = agent.flattenable(v, false, M);
            agent.vm.temps.pop();
            match flat {
                Ok((i, n)) => {
                    // The new iterator rooted while the key is made.
                    agent.vm.temps.extend([i, n]);
                    let pushed = (|| {
                        let kv = agent.key_value(key)?;
                        agent.array_push(keys, kv)?;
                        agent.array_push(iters, i)?;
                        agent.array_push(nexts, n)
                    })();
                    agent.vm.temps.truncate(agent.vm.temps.len() - 2);
                    pushed?;
                }
                Err(e) => return Err(agent.close_list_after(iters, e)),
            }
        }
        let count = agent.array_len(iters);
        let padding = agent.zip_padding(iters, padding_option, count, Some(keys))?;
        agent.vm.temps.push(padding);
        agent.new_zip(iters, nexts, mode, padding, keys)
    })();
    agent.vm.temps.truncate(base);
    r
}
