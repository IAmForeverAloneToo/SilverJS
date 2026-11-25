//! `Array.fromAsync` (ECMA-262 section 23.1.2.2): the async function of
//! the spec as a native object stepped by its awaits' reactions. Its
//! state, in its internal slots: the promise it settles, the array it
//! fills, what it reads (an async iterator, or an array-like and its
//! length), the map function and its `this`, the count and what the
//! await it waits on is for. The messages are the reference browser's
//! (the J4.4c probe, Chrome 158).

use super::{Call, Entry, Name, Wk, method};
use crate::agent::Agent;
use crate::heap::ObjectCell;
use crate::interp::{Abrupt, ErrorType, Res};
use crate::object::ObjectKind;
use crate::value::Value;

/// The natives `Array.fromAsync` makes for itself, installed nowhere:
/// its awaits' two reactions.
pub(crate) const FROM_ASYNC_INTERNAL: &[Entry] = &[
    method("fromAsyncFulfilled", 1, fulfilled),
    method("fromAsyncRejected", 1, rejected),
];

const FULFILLED_FN: u32 = 0;
const REJECTED_FN: u32 = 1;

// A run's internal slots.
const S_PROMISE: usize = 0;
const S_ARRAY: usize = 1;
const S_SOURCE: usize = 2;
const S_NEXT: usize = 3;
const S_MAPFN: usize = 4;
const S_THIS: usize = 5;
const S_K: usize = 6;
const S_LEN: usize = 7;
const S_PHASE: usize = 8;
const S_ERROR: usize = 9;

// What an await is for.
/// An iterator result.
const NEXT: i32 = 0;
/// A mapped value, from an iterator.
const MAPPED: i32 = 1;
/// An array-like's value.
const VALUE: i32 = 2;
/// A mapped value, from an array-like.
const LIKE_MAPPED: i32 = 3;
/// What `return()` gave, closing on a throw (in `S_ERROR`).
const CLOSING: i32 = 4;

/// 2^53 - 1.
const MAX_SAFE: f64 = 9_007_199_254_740_991.0;

impl Agent {
    fn fslot(&self, o: Value, i: usize) -> Value {
        o.as_object()
            .and_then(|o| self.heap.objects.get(o))
            .and_then(|c| c.internal().get(i).copied())
            .unwrap_or(Value::UNDEFINED)
    }

    fn set_fslot(&mut self, o: Value, i: usize, v: Value) {
        if let Some(cell) = o.as_object().and_then(|o| self.heap.objects.get_mut(o)) {
            cell.set_internal(i, v);
        }
    }

    /// The thrown value just caught.
    fn caught(&mut self) -> Value {
        std::mem::replace(&mut self.vm.exception, Value::UNDEFINED)
    }

    /// The run's promise rejected with `e`.
    fn fa_reject(&mut self, run: Value, e: Value) -> Res<()> {
        let p = self.fslot(run, S_PROMISE);
        self.reject_promise(p, e)
    }

    /// `r`, or the run's promise rejected with what it threw.
    fn fa_or_reject(&mut self, run: Value, r: Res<()>) -> Res<()> {
        match r {
            Err(Abrupt::Throw) => {
                let e = self.caught();
                self.fa_reject(run, e)
            }
            other => other,
        }
    }

    /// Await(v), for `phase`: on in its reactions.
    fn fa_await(&mut self, run: Value, v: Value, phase: i32) -> Res<()> {
        self.set_fslot(run, S_PHASE, Value::int(phase));
        let ctor = self.realm().intrinsics.promise;
        let p = match self.promise_resolve(ctor, v) {
            Ok(p) => p,
            Err(Abrupt::Throw) => {
                let e = self.caught();
                return self.fa_settled(run, e, true);
            }
            Err(a) => return Err(a),
        };
        self.vm.temps.push(p);
        let mut made = Vec::new();
        let mut failed = None;
        for i in [FULFILLED_FN, REJECTED_FN] {
            let id = super::native_id(super::table::FROM_ASYNC_INTERNAL, i);
            match self.native_function(id, 1, "") {
                Ok(f) => {
                    if let Some(o) = f.as_object() {
                        self.heap.with_internal_slots(o, |s| *s = vec![run]);
                    }
                    self.vm.temps.push(f);
                    made.push(f);
                }
                Err(a) => {
                    failed = Some(a);
                    break;
                }
            }
        }
        let n = self.vm.temps.len();
        self.vm.temps.truncate(n.saturating_sub(made.len() + 1));
        if let Some(a) = failed {
            return Err(a);
        }
        if let [f, r] = made.as_slice() {
            self.perform_then(p, *f, *r, None);
        }
        Ok(())
    }

    /// What the run's await settled with: `v`, or a throw of it.
    fn fa_settled(&mut self, run: Value, v: Value, threw: bool) -> Res<()> {
        self.vm.temps.extend([run, v]);
        let phase = self.fslot(run, S_PHASE).as_int().unwrap_or(NEXT);
        let r = match (phase, threw) {
            (CLOSING, _) => {
                let e = self.fslot(run, S_ERROR);
                self.fa_reject(run, e)
            }
            // A rejected step closes the iterator, as the reference
            // browser does (the spec rejects without closing).
            (NEXT | MAPPED, true) => self.fa_close(run, v),
            (VALUE | LIKE_MAPPED, true) => self.fa_reject(run, v),
            (NEXT, false) => {
                let r = self.fa_result(run, v);
                self.fa_or_reject(run, r)
            }
            (MAPPED, false) => self.fa_define(run, v),
            (VALUE, false) => {
                let r = self.fa_like_value(run, v);
                self.fa_or_reject(run, r)
            }
            _ => {
                let r = self.fa_like_define(run, v);
                self.fa_or_reject(run, r)
            }
        };
        let n = self.vm.temps.len();
        self.vm.temps.truncate(n.saturating_sub(2));
        r
    }

    /// The next step of an iterator's run: `next()` called and awaited.
    fn fa_step(&mut self, run: Value) -> Res<()> {
        let k = self.fslot(run, S_K).as_number().unwrap_or(0.0);
        if k >= MAX_SAFE {
            let e = self.make_error(ErrorType::TypeError, "Invalid array length")?;
            return self.fa_close(run, e);
        }
        let (it, next) = (self.fslot(run, S_SOURCE), self.fslot(run, S_NEXT));
        match self.call_value(next, it, &[]) {
            Ok(r) => self.fa_await(run, r, NEXT),
            // A throwing `next()` closes the iterator too, as the
            // reference browser has it.
            Err(Abrupt::Throw) => {
                let e = self.caught();
                self.fa_close(run, e)
            }
            Err(a) => Err(a),
        }
    }

    /// An awaited iterator result: the end, or its value mapped.
    fn fa_result(&mut self, run: Value, r: Value) -> Res<()> {
        if r.as_object().is_none() {
            // The reference browser names the function, not the value.
            return Err(self.type_error("Iterator result Array.fromAsync is not an object"));
        }
        let key = self.name_key(Name::Str("done"));
        let done = self.get_value(r, key)?;
        let k = self.fslot(run, S_K).as_number().unwrap_or(0.0);
        if self.to_boolean(done) {
            let a = self.fslot(run, S_ARRAY);
            self.set_length(a, k)?;
            let p = self.fslot(run, S_PROMISE);
            return self.resolve_promise(p, a);
        }
        let key = self.name_key(Name::Str("value"));
        let v = self.get_value(r, key)?;
        let mapfn = self.fslot(run, S_MAPFN);
        if mapfn.is_undefined() {
            return self.fa_define(run, v);
        }
        let this = self.fslot(run, S_THIS);
        match self.call_value(mapfn, this, &[v, Value::number(k)]) {
            Ok(m) => self.fa_await(run, m, MAPPED),
            Err(Abrupt::Throw) => {
                let e = self.caught();
                self.fa_close(run, e)
            }
            Err(a) => Err(a),
        }
    }

    /// A value defined at the count, then the next step; a failure
    /// closes the iterator.
    fn fa_define(&mut self, run: Value, v: Value) -> Res<()> {
        let a = self.fslot(run, S_ARRAY);
        let k = self.fslot(run, S_K).as_number().unwrap_or(0.0);
        match self.create_index(a, k, v) {
            Ok(()) => {
                self.set_fslot(run, S_K, Value::number(k + 1.0));
                self.fa_step(run)
            }
            Err(Abrupt::Throw) => {
                let e = self.caught();
                self.fa_close(run, e)
            }
            Err(a) => Err(a),
        }
    }

    /// AsyncIteratorClose with a throw of `e`: `return()` called and
    /// awaited, then the run rejected with `e` whatever it did.
    fn fa_close(&mut self, run: Value, e: Value) -> Res<()> {
        self.set_fslot(run, S_ERROR, e);
        let it = self.fslot(run, S_SOURCE);
        let key = self.name_key(Name::Str("return"));
        let ret = match self.get_method(it, key) {
            Ok(r) => r,
            Err(Abrupt::Throw) => {
                self.caught();
                return self.fa_reject(run, e);
            }
            Err(a) => return Err(a),
        };
        if ret.is_undefined() {
            return self.fa_reject(run, e);
        }
        match self.call_value(ret, it, &[]) {
            Ok(r) => self.fa_await(run, r, CLOSING),
            Err(Abrupt::Throw) => {
                self.caught();
                self.fa_reject(run, e)
            }
            Err(a) => Err(a),
        }
    }

    /// The next step of an array-like's run: its value read and awaited,
    /// or the end.
    fn fa_like_step(&mut self, run: Value) -> Res<()> {
        let k = self.fslot(run, S_K).as_number().unwrap_or(0.0);
        let len = self.fslot(run, S_LEN).as_number().unwrap_or(0.0);
        if k >= len {
            let a = self.fslot(run, S_ARRAY);
            self.set_length(a, len)?;
            let p = self.fslot(run, S_PROMISE);
            return self.resolve_promise(p, a);
        }
        let source = self.fslot(run, S_SOURCE);
        let v = self.get_index(source, k)?;
        self.fa_await(run, v, VALUE)
    }

    /// An array-like's awaited value, mapped and awaited, or defined.
    fn fa_like_value(&mut self, run: Value, v: Value) -> Res<()> {
        let mapfn = self.fslot(run, S_MAPFN);
        if mapfn.is_undefined() {
            return self.fa_like_define(run, v);
        }
        let k = self.fslot(run, S_K).as_number().unwrap_or(0.0);
        let this = self.fslot(run, S_THIS);
        let m = self.call_value(mapfn, this, &[v, Value::number(k)])?;
        self.fa_await(run, m, LIKE_MAPPED)
    }

    fn fa_like_define(&mut self, run: Value, v: Value) -> Res<()> {
        let a = self.fslot(run, S_ARRAY);
        let k = self.fslot(run, S_K).as_number().unwrap_or(0.0);
        self.create_index(a, k, v)?;
        self.set_fslot(run, S_K, Value::number(k + 1.0));
        self.fa_like_step(run)
    }

    /// The run's start, up to its first await: the map function checked,
    /// the iterator (or the array-like) and the array made.
    fn fa_start(&mut self, run: Value, c: Call) -> Res<()> {
        let (items, mapfn) = (self.arg(c, 0), self.arg(c, 1));
        if !mapfn.is_undefined() && !self.is_callable(mapfn) {
            let text = self.display(mapfn);
            return Err(self.type_error(format!("{text} is not a function")));
        }
        self.set_fslot(run, S_MAPFN, mapfn);
        self.set_fslot(run, S_THIS, self.arg(c, 2));
        if items.is_nullish() {
            let text = self.display(items);
            return Err(self.type_error(format!(
                "Cannot read properties of {text} (reading 'Symbol(Symbol.asyncIterator)')"
            )));
        }
        let key = self.well_known_key(Wk::AsyncIterator);
        let using_async = self.get_method(items, key)?;
        let using_sync = if using_async.is_undefined() {
            let key = self.well_known_key(Wk::Iterator);
            self.get_method(items, key)?
        } else {
            Value::UNDEFINED
        };
        if using_async.is_undefined() && using_sync.is_undefined() {
            let source = self.object_of(items)?;
            self.set_fslot(run, S_SOURCE, source);
            let len = self.length_of(source)?;
            self.set_fslot(run, S_LEN, Value::number(len));
            let a = if self.is_constructor_value(c.this) {
                self.construct_value(c.this, &[Value::number(len)], c.this)?
            } else {
                self.array_create(len, None)?
            };
            self.set_fslot(run, S_ARRAY, a);
            return self.fa_like_step(run);
        }
        let method = if using_async.is_undefined() {
            using_sync
        } else {
            using_async
        };
        let it = self.call_value(method, items, &[])?;
        if it.as_object().is_none() {
            // Symbol.iterator for both, as the reference browser has it.
            return Err(self.type_error("Result of the Symbol.iterator method is not an object"));
        }
        self.vm.temps.push(it);
        let key = self.name_key(Name::Str("next"));
        let next = self.get_value(it, key);
        self.vm.temps.pop();
        let next = next?;
        let (it, next) = if using_async.is_undefined() {
            self.async_from_sync_iterator(it, next)?
        } else {
            (it, next)
        };
        self.set_fslot(run, S_SOURCE, it);
        self.set_fslot(run, S_NEXT, next);
        let a = if self.is_constructor_value(c.this) {
            self.construct_value(c.this, &[], c.this)?
        } else {
            self.array_create(0.0, None)?
        };
        self.set_fslot(run, S_ARRAY, a);
        self.fa_step(run)
    }
}

/// `Array.fromAsync(items, mapfn, thisArg)`: a promise of the array.
pub(crate) fn from_async(agent: &mut Agent, c: Call) -> Res<Value> {
    let promise = agent.new_promise()?;
    agent.vm.temps.push(promise);
    let run = agent.alloc_object(ObjectCell::with_internal(
        Value::NULL,
        ObjectKind::FromAsync,
        vec![
            promise,
            Value::UNDEFINED,
            Value::UNDEFINED,
            Value::UNDEFINED,
            Value::UNDEFINED,
            Value::UNDEFINED,
            Value::number(0.0),
            Value::number(0.0),
            Value::int(NEXT),
            Value::UNDEFINED,
        ],
    ));
    let r = run.and_then(|run| {
        agent.vm.temps.push(run);
        let started = agent.fa_start(run, c);
        let r = agent.fa_or_reject(run, started);
        agent.vm.temps.pop();
        r
    });
    agent.vm.temps.pop();
    r.map(|()| promise)
}

fn fulfilled(agent: &mut Agent, c: Call) -> Res<Value> {
    let run = agent.fslot(c.callee, 0);
    agent.fa_settled(run, agent.arg(c, 0), false)?;
    Ok(Value::UNDEFINED)
}

fn rejected(agent: &mut Agent, c: Call) -> Res<Value> {
    let run = agent.fslot(c.callee, 0);
    agent.fa_settled(run, agent.arg(c, 0), true)?;
    Ok(Value::UNDEFINED)
}
