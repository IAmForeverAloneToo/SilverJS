//! Promises (ECMA-262 section 27.2) and the agent's job queue (section
//! 9.5): `Promise`, its prototype's `then`, `catch` and `finally`, its
//! combinators, and the jobs that run reactions and resolve thenables.
//!
//! A promise keeps, in its internal slots, its state, its result, whether
//! it was ever handled, then its reactions, five values each: what to
//! call on fulfillment and on rejection (a function, undefined for the
//! default, or an async function's driver to resume), and the capability
//! the reaction settles (its promise, resolve and reject; a native
//! `Promise`'s capability has no functions: it is settled directly).
//! Jobs run when the embedder asks, and after each script.

use std::collections::VecDeque;

use super::{CONFIGURABLE, Call, Entry, HIDDEN, Name, What, Wk, entry, method};
use crate::agent::Agent;
use crate::heap::ObjectCell;
use crate::interp::{Abrupt, ErrorType, Res, messages};
use crate::object::ObjectKind;
use crate::string::PropertyKey;
use crate::value::Value;

// A promise's internal slots.
const STATE: usize = 0;
const RESULT: usize = 1;
const HANDLED: usize = 2;
const REACTIONS: usize = 3;
const REACTION_LEN: usize = 5;

/// A promise's state.
pub(crate) const PENDING: i32 = 0;
pub(crate) const FULFILLED: i32 = 1;
pub(crate) const REJECTED: i32 = 2;

/// A job of the agent's queue.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Job {
    /// PromiseReactionJob (section 27.2.2.1): `handler` called with
    /// `argument`, its outcome settling the capability.
    Reaction {
        handler: Value,
        argument: Value,
        rejected: bool,
        promise: Value,
        resolve: Value,
        reject: Value,
    },
    /// PromiseResolveThenableJob (section 27.2.2.2): `then` called on
    /// `thenable` with `promise`'s resolving functions.
    Thenable {
        promise: Value,
        thenable: Value,
        then: Value,
    },
    /// A finalization registry's cleanup (section 9.13): its table.
    Cleanup { table: Value },
}

impl Job {
    /// The values a job holds, for the collector.
    pub(crate) fn values(&self) -> [Value; 6] {
        match *self {
            Job::Reaction {
                handler,
                argument,
                promise,
                resolve,
                reject,
                ..
            } => [
                handler,
                argument,
                promise,
                resolve,
                reject,
                Value::UNDEFINED,
            ],
            Job::Thenable {
                promise,
                thenable,
                then,
            } => [
                promise,
                thenable,
                then,
                Value::UNDEFINED,
                Value::UNDEFINED,
                Value::UNDEFINED,
            ],
            Job::Cleanup { table } => [
                table,
                Value::UNDEFINED,
                Value::UNDEFINED,
                Value::UNDEFINED,
                Value::UNDEFINED,
                Value::UNDEFINED,
            ],
        }
    }
}

/// The agent's queue of jobs.
pub(crate) type Jobs = VecDeque<Job>;

/// `Promise`.
pub(crate) const PROMISE: &[Entry] = &[
    method("all", 1, all),
    method("allSettled", 1, all_settled),
    method("any", 1, any),
    method("race", 1, race),
    method("resolve", 1, resolve_static),
    method("reject", 1, reject_static),
    method("withResolvers", 0, with_resolvers),
    method("try", 1, try_),
    entry(Name::Sym(Wk::Species), What::Getter(species), CONFIGURABLE),
];

/// `Promise.prototype`.
pub(crate) const PROMISE_PROTO: &[Entry] = &[
    entry(Name::Str("constructor"), What::Given(0), HIDDEN),
    method("then", 2, then),
    method("catch", 1, catch),
    method("finally", 1, finally),
    entry(
        Name::Sym(Wk::ToStringTag),
        What::Str("Promise"),
        CONFIGURABLE,
    ),
];

/// The natives promises make for themselves, installed nowhere: the
/// resolving functions, NewPromiseCapability's executor, the
/// combinators' element functions, and `finally`'s.
pub(crate) const PROMISE_INTERNAL: &[Entry] = &[
    method("resolve", 1, resolving_resolve),
    method("reject", 1, resolving_reject),
    method("executor", 2, capability_executor),
    method("allResolve", 1, all_resolve_element),
    method("allSettledResolve", 1, all_settled_resolve_element),
    method("allSettledReject", 1, all_settled_reject_element),
    method("anyReject", 1, any_reject_element),
    method("thenFinally", 1, then_finally),
    method("catchFinally", 1, catch_finally),
    method("valueThunk", 0, value_thunk),
    method("thrower", 0, thrower),
    method("asyncGenReturnFulfilled", 1, async_gen_return_fulfilled),
    method("asyncGenReturnRejected", 1, async_gen_return_rejected),
    method("asyncFromSyncUnwrap", 1, async_from_sync_unwrap),
    method("asyncFromSyncCloseReject", 1, async_from_sync_close_reject),
];

/// `%AsyncFromSyncIteratorPrototype%` (section 27.1.6.2), which no script
/// reaches.
pub(crate) const ASYNC_FROM_SYNC_PROTO: &[Entry] = &[
    method("next", 1, async_from_sync_next),
    method("return", 1, async_from_sync_return),
    method("throw", 1, async_from_sync_throw),
];

// Indices in PROMISE_INTERNAL.
const RESOLVE: u32 = 0;
const REJECT: u32 = 1;
const EXECUTOR: u32 = 2;
const ALL_RESOLVE: u32 = 3;
const ALL_SETTLED_RESOLVE: u32 = 4;
const ALL_SETTLED_REJECT: u32 = 5;
const ANY_REJECT: u32 = 6;
const THEN_FINALLY: u32 = 7;
const CATCH_FINALLY: u32 = 8;
const VALUE_THUNK: u32 = 9;
const THROWER: u32 = 10;
const ASYNC_GEN_RETURN_FULFILLED: u32 = 11;
const ASYNC_GEN_RETURN_REJECTED: u32 = 12;
const ASYNC_FROM_SYNC_UNWRAP: u32 = 13;
const ASYNC_FROM_SYNC_CLOSE_REJECT: u32 = 14;

/// A capability: a promise and the functions that settle it (undefined
/// for a native `Promise`'s, settled directly).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Capability {
    pub promise: Value,
    pub resolve: Value,
    pub reject: Value,
}

fn internal_id(i: u32) -> u32 {
    super::native_id(super::table::PROMISE_INTERNAL, i)
}

impl Agent {
    // Promises.

    /// Whether `v` is a promise; its state and result if it is.
    pub(crate) fn promise_state(&self, v: Value) -> Option<(i32, Value)> {
        let cell = self.heap.objects.get(v.as_object()?)?;
        if cell.kind() != ObjectKind::Promise {
            return None;
        }
        let s = cell.internal();
        let state = s.get(STATE).and_then(|v| v.as_int()).unwrap_or(PENDING);
        Some((state, s.get(RESULT).copied().unwrap_or(Value::UNDEFINED)))
    }

    /// An async function's driver a pending promise's reactions would
    /// resume (the first such), if any: who awaits it.
    pub(crate) fn promise_awaiter(&self, promise: Value) -> Option<Value> {
        let cell = self.heap.objects.get(promise.as_object()?)?;
        if cell.kind() != ObjectKind::Promise {
            return None;
        }
        cell.internal()
            .get(REACTIONS..)?
            .chunks(REACTION_LEN)
            .filter_map(|r| r.first().copied())
            .find(|h| self.is_async_driver(*h))
    }

    /// A new pending promise of prototype `proto`.
    pub(crate) fn new_promise_with(&mut self, proto: Value) -> Res<Value> {
        self.alloc_object(ObjectCell::with_internal(
            proto,
            ObjectKind::Promise,
            vec![Value::int(PENDING), Value::UNDEFINED, Value::FALSE],
        ))
    }

    /// A new pending `%Promise%` of the current realm.
    pub(crate) fn new_promise(&mut self) -> Res<Value> {
        let proto = self.realm().intrinsics.promise_prototype;
        self.new_promise_with(proto)
    }

    /// CreateResolvingFunctions (section 27.2.1.3): `resolve` and
    /// `reject`, each holding the promise and the other, so that the
    /// first call of either disarms both.
    pub(crate) fn resolving_functions(&mut self, promise: Value) -> Res<(Value, Value)> {
        self.vm.temps.push(promise);
        let made = (|| {
            let resolve = self.native_function(internal_id(RESOLVE), 1, "")?;
            self.vm.temps.push(resolve);
            let reject = self.native_function(internal_id(REJECT), 1, "");
            self.vm.temps.pop();
            let reject = reject?;
            for (f, other) in [(resolve, reject), (reject, resolve)] {
                if let Some(o) = f.as_object() {
                    self.heap
                        .with_internal_slots(o, |s| *s = vec![promise, other]);
                }
            }
            Ok((resolve, reject))
        })();
        self.vm.temps.pop();
        made
    }

    /// Disarms a pair of resolving functions; the promise they held, if
    /// they were armed.
    fn take_resolving(&mut self, f: Value) -> Option<Value> {
        let slots = self.slots_of(f);
        let promise = slots.first().copied().filter(|p| !p.is_undefined())?;
        let other = slots.get(1).copied().unwrap_or(Value::UNDEFINED);
        for g in [f, other] {
            if let Some(o) = g.as_object() {
                self.heap.with_internal_slots(o, |s| {
                    if let Some(p) = s.first_mut() {
                        *p = Value::UNDEFINED;
                    }
                });
            }
        }
        Some(promise)
    }

    /// An object's internal slots, copied.
    fn slots_of(&self, v: Value) -> Vec<Value> {
        v.as_object()
            .and_then(|o| self.heap.objects.get(o))
            .map(|c| c.internal().to_vec())
            .unwrap_or_default()
    }

    /// The resolving functions' resolve (section 27.2.1.3.2): a thenable
    /// is followed through a job; anything else fulfills.
    pub(crate) fn resolve_promise(&mut self, promise: Value, resolution: Value) -> Res<()> {
        if resolution == promise {
            let e = self.make_error(ErrorType::TypeError, &messages::promise_cycle())?;
            return self.reject_promise(promise, e);
        }
        if resolution.as_object().is_none() {
            return self.settle(promise, FULFILLED, resolution);
        }
        let key = self.then_key();
        let then = match self.get_value(resolution, key) {
            Ok(t) => t,
            Err(Abrupt::Throw) => {
                let e = std::mem::replace(&mut self.vm.exception, Value::UNDEFINED);
                return self.reject_promise(promise, e);
            }
            Err(e) => return Err(e),
        };
        if !self.is_callable(then) {
            return self.settle(promise, FULFILLED, resolution);
        }
        self.vm.jobs.push_back(Job::Thenable {
            promise,
            thenable: resolution,
            then,
        });
        Ok(())
    }

    /// RejectPromise (section 27.2.1.7).
    pub(crate) fn reject_promise(&mut self, promise: Value, reason: Value) -> Res<()> {
        self.settle(promise, REJECTED, reason)
    }

    /// FulfillPromise and RejectPromise: the state and result set, and a
    /// job queued for each reaction.
    fn settle(&mut self, promise: Value, state: i32, value: Value) -> Res<()> {
        let Some(o) = promise.as_object() else {
            return Ok(());
        };
        let reactions = self
            .heap
            .with_internal_slots(o, |s| {
                if s.len() < REACTIONS {
                    s.resize(REACTIONS, Value::UNDEFINED);
                }
                let taken: Vec<Value> = s.drain(REACTIONS..).collect();
                if let Some(v) = s.get_mut(STATE) {
                    *v = Value::int(state);
                }
                if let Some(v) = s.get_mut(RESULT) {
                    *v = value;
                }
                taken
            })
            .unwrap_or_default();
        for r in reactions.chunks(REACTION_LEN) {
            let at = |i: usize| r.get(i).copied().unwrap_or(Value::UNDEFINED);
            let handler = if state == FULFILLED { at(0) } else { at(1) };
            self.vm.jobs.push_back(Job::Reaction {
                handler,
                argument: value,
                rejected: state == REJECTED,
                promise: at(2),
                resolve: at(3),
                reject: at(4),
            });
        }
        Ok(())
    }

    /// PerformPromiseThen (section 27.2.5.4.1): the reaction kept, or
    /// queued at once if the promise is settled. `on_fulfilled` and
    /// `on_rejected` are functions, undefined, or an async function's
    /// driver.
    pub(crate) fn perform_then(
        &mut self,
        promise: Value,
        on_fulfilled: Value,
        on_rejected: Value,
        cap: Option<Capability>,
    ) {
        let keep = |agent: &Agent, f: Value| {
            if agent.is_callable(f) || agent.is_async_driver(f) {
                f
            } else {
                Value::UNDEFINED
            }
        };
        let (f, r) = (keep(self, on_fulfilled), keep(self, on_rejected));
        let cap = cap.unwrap_or(Capability {
            promise: Value::UNDEFINED,
            resolve: Value::UNDEFINED,
            reject: Value::UNDEFINED,
        });
        let Some((state, result)) = self.promise_state(promise) else {
            return;
        };
        let Some(o) = promise.as_object() else {
            return;
        };
        if state == PENDING {
            self.heap.with_internal_slots(o, |s| {
                if s.len() < REACTIONS {
                    s.resize(REACTIONS, Value::UNDEFINED);
                }
                s.extend([f, r, cap.promise, cap.resolve, cap.reject]);
                if let Some(h) = s.get_mut(HANDLED) {
                    *h = Value::TRUE;
                }
            });
            return;
        }
        self.heap.with_internal_slots(o, |s| {
            if let Some(h) = s.get_mut(HANDLED) {
                *h = Value::TRUE;
            }
        });
        self.vm.jobs.push_back(Job::Reaction {
            handler: if state == FULFILLED { f } else { r },
            argument: result,
            rejected: state == REJECTED,
            promise: cap.promise,
            resolve: cap.resolve,
            reject: cap.reject,
        });
    }

    /// Settles a capability with a value or a reason.
    fn settle_capability(&mut self, cap: Capability, value: Value, rejected: bool) -> Res<()> {
        let f = if rejected { cap.reject } else { cap.resolve };
        if !f.is_undefined() {
            return self.call_value(f, Value::UNDEFINED, &[value]).map(|_| ());
        }
        if cap.promise.is_undefined() {
            return Ok(());
        }
        if rejected {
            self.reject_promise(cap.promise, value)
        } else {
            self.resolve_promise(cap.promise, value)
        }
    }

    /// NewPromiseCapability (section 27.2.1.5): a native `Promise`'s
    /// directly, else through `c`'s constructor and an executor that
    /// keeps the functions it is given.
    pub(crate) fn new_capability(&mut self, c: Value) -> Res<Capability> {
        if self.is_intrinsic_promise(c) {
            let promise = self.new_promise_for(c)?;
            return Ok(Capability {
                promise,
                resolve: Value::UNDEFINED,
                reject: Value::UNDEFINED,
            });
        }
        if !self.is_constructor_value(c) {
            let text = self.display(c);
            return Err(self.type_error(messages::not_a_constructor(&text)));
        }
        let executor = self.native_function(internal_id(EXECUTOR), 2, "")?;
        if let Some(o) = executor.as_object() {
            self.heap
                .with_internal_slots(o, |s| *s = vec![Value::UNDEFINED, Value::UNDEFINED]);
        }
        self.vm.temps.push(executor);
        let made = self.construct_value(c, &[executor], c);
        self.vm.temps.pop();
        let promise = made?;
        let slots = self.slots_of(executor);
        let resolve = slots.first().copied().unwrap_or(Value::UNDEFINED);
        let reject = slots.get(1).copied().unwrap_or(Value::UNDEFINED);
        if !self.is_callable(resolve) {
            let text = self.display(resolve);
            return Err(self.type_error(messages::promise_resolve_not_function(&text)));
        }
        if !self.is_callable(reject) {
            let text = self.display(reject);
            return Err(self.type_error(messages::promise_reject_not_function(&text)));
        }
        Ok(Capability {
            promise,
            resolve,
            reject,
        })
    }

    /// Whether `c` is a realm's `%Promise%`: its capability needs no
    /// functions.
    fn is_intrinsic_promise(&self, c: Value) -> bool {
        !c.is_undefined() && self.vm.realms.iter().any(|r| r.intrinsics.promise == c)
    }

    /// A new pending promise made as `new c(...)` would: `c`'s realm's
    /// `%Promise.prototype%`.
    fn new_promise_for(&mut self, c: Value) -> Res<Value> {
        let proto = self.prototype_from(c, |r| r.intrinsics.promise_prototype)?;
        self.new_promise_with(proto)
    }

    /// PromiseResolve (section 27.2.4.7.1): `x` itself if it is a
    /// promise made by `c`, else a promise of `c` resolved with `x`.
    pub(crate) fn promise_resolve(&mut self, c: Value, x: Value) -> Res<Value> {
        if self.promise_state(x).is_some() {
            let key = self.realm().constructor_key;
            let ctor = self.get_value(x, key)?;
            if ctor == c {
                return Ok(x);
            }
        }
        let cap = self.new_capability(c)?;
        self.vm.temps.push(cap.promise);
        let done = self.settle_capability(cap, x, false);
        self.vm.temps.pop();
        done.map(|()| cap.promise)
    }

    /// SpeciesConstructor (section 7.3.22) of a promise.
    fn promise_species(&mut self, o: Value) -> Res<Value> {
        let default = self.realm().intrinsics.promise;
        let key = self.realm().constructor_key;
        let c = self.get_value(o, key)?;
        if c.is_undefined() {
            return Ok(default);
        }
        if c.as_object().is_none() {
            return Err(self.type_error(messages::CONSTRUCTOR_NOT_OBJECT));
        }
        let key = self.well_known_key(Wk::Species);
        let s = self.get_value(c, key)?;
        if s.is_nullish() {
            return Ok(default);
        }
        if self.is_constructor_value(s) {
            return Ok(s);
        }
        let text = self.display(s);
        Err(self.type_error(messages::not_a_constructor(&text)))
    }

    fn then_key(&mut self) -> PropertyKey {
        self.name_key(Name::Str("then"))
    }

    // Jobs.

    /// Runs the queued jobs until none is left, those they queue too;
    /// stops at a termination. The jobs' own throws settle promises; one
    /// that escapes (a capability's function throwing) is dropped, as a
    /// host reports it.
    pub(crate) fn run_jobs(&mut self) -> Res<()> {
        while let Some(job) = self.vm.jobs.pop_front() {
            match self.run_job(job) {
                Ok(()) => {}
                Err(Abrupt::Throw) => {
                    self.vm.exception = Value::UNDEFINED;
                }
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    fn run_job(&mut self, job: Job) -> Res<()> {
        let held = job.values();
        let at = self.vm.temps.len();
        self.vm.temps.extend(held);
        let r = self.run_job_rooted(job);
        self.vm.temps.truncate(at);
        r
    }

    fn run_job_rooted(&mut self, job: Job) -> Res<()> {
        match job {
            Job::Reaction {
                handler,
                argument,
                rejected,
                promise,
                resolve,
                reject,
            } => {
                let cap = Capability {
                    promise,
                    resolve,
                    reject,
                };
                if self.is_async_driver(handler) {
                    return self.resume_async(handler, argument, rejected);
                }
                if handler.is_undefined() {
                    return self.settle_capability(cap, argument, rejected);
                }
                match self.call_value(handler, Value::UNDEFINED, &[argument]) {
                    Ok(v) => self.settle_capability(cap, v, false),
                    Err(Abrupt::Throw) => {
                        let e = std::mem::replace(&mut self.vm.exception, Value::UNDEFINED);
                        self.settle_capability(cap, e, true)
                    }
                    Err(e) => Err(e),
                }
            }
            Job::Thenable {
                promise,
                thenable,
                then,
            } => {
                let (resolve, reject) = self.resolving_functions(promise)?;
                self.vm.temps.extend([resolve, reject]);
                let r = match self.call_value(then, thenable, &[resolve, reject]) {
                    Ok(_) => Ok(()),
                    Err(Abrupt::Throw) => {
                        let e = std::mem::replace(&mut self.vm.exception, Value::UNDEFINED);
                        self.call_value(reject, Value::UNDEFINED, &[e]).map(|_| ())
                    }
                    Err(e) => Err(e),
                };
                let n = self.vm.temps.len();
                self.vm.temps.truncate(n.saturating_sub(2));
                r
            }
            Job::Cleanup { table } => self.run_cleanup(table),
        }
    }

    /// A combinator's iteration (sections 27.2.4.1 to 27.2.4.5): `step`
    /// called for each value with its index; a throw before the
    /// iterator is done closes it. Gives back the count.
    fn each_promise(
        &mut self,
        iterable: Value,
        mut step: impl FnMut(&mut Agent, Value, u32) -> Res<()>,
    ) -> Res<u32> {
        let (iterator, next) = self.get_iterator_typed(iterable)?;
        self.vm.temps.extend([iterator, next]);
        let mut index = 0u32;
        let r = loop {
            let value = match self.iterator_step(iterator, next) {
                Ok(Some(v)) => v,
                Ok(None) => break Ok(index),
                Err(e) => break Err(e),
            };
            self.vm.temps.push(value);
            let done = step(self, value, index);
            self.vm.temps.pop();
            if let Err(e) = done {
                if matches!(e, Abrupt::Throw) {
                    let saved = self.vm.exception;
                    let _ = self.iterator_close_quiet(iterator);
                    self.vm.exception = saved;
                }
                break Err(e);
            }
            index += 1;
        };
        let n = self.vm.temps.len();
        self.vm.temps.truncate(n.saturating_sub(2));
        r
    }

    /// A combinator's rejection of its capability with the exception a
    /// step threw (IfAbruptRejectPromise).
    fn reject_abrupt(&mut self, cap: Capability, r: Res<()>) -> Res<Value> {
        match r {
            Ok(()) => Ok(cap.promise),
            Err(Abrupt::Throw) => {
                let e = std::mem::replace(&mut self.vm.exception, Value::UNDEFINED);
                self.settle_capability(cap, e, true)?;
                Ok(cap.promise)
            }
            Err(e) => Err(e),
        }
    }
}

/// `Promise(executor)` (section 27.2.3.1).
pub(crate) fn construct(agent: &mut Agent, c: Call) -> Res<Value> {
    if c.new_target.is_undefined() {
        return Err(agent.type_error(messages::PROMISE_WITHOUT_NEW));
    }
    let executor = agent.arg(c, 0);
    if !agent.is_callable(executor) {
        let text = agent.display(executor);
        return Err(agent.type_error(messages::promise_resolver(&text)));
    }
    let proto = agent.prototype_from(c.new_target, |r| r.intrinsics.promise_prototype)?;
    let promise = agent.new_promise_with(proto)?;
    agent.vm.temps.push(promise);
    let r = (|| {
        let (resolve, reject) = agent.resolving_functions(promise)?;
        agent.vm.temps.extend([resolve, reject]);
        let r = match agent.call_value(executor, Value::UNDEFINED, &[resolve, reject]) {
            Ok(_) => Ok(()),
            Err(Abrupt::Throw) => {
                let e = std::mem::replace(&mut agent.vm.exception, Value::UNDEFINED);
                agent.call_value(reject, Value::UNDEFINED, &[e]).map(|_| ())
            }
            Err(e) => Err(e),
        };
        let n = agent.vm.temps.len();
        agent.vm.temps.truncate(n.saturating_sub(2));
        r
    })();
    agent.vm.temps.pop();
    r.map(|()| promise)
}

fn resolving_resolve(agent: &mut Agent, c: Call) -> Res<Value> {
    let value = agent.arg(c, 0);
    if let Some(promise) = agent.take_resolving(c.callee) {
        agent.resolve_promise(promise, value)?;
    }
    Ok(Value::UNDEFINED)
}

fn resolving_reject(agent: &mut Agent, c: Call) -> Res<Value> {
    let reason = agent.arg(c, 0);
    if let Some(promise) = agent.take_resolving(c.callee) {
        agent.reject_promise(promise, reason)?;
    }
    Ok(Value::UNDEFINED)
}

/// GetCapabilitiesExecutor (section 27.2.1.5.1).
fn capability_executor(agent: &mut Agent, c: Call) -> Res<Value> {
    let slots = agent.slots_of(c.callee);
    let set = |v: Option<&Value>| v.is_some_and(|v| !v.is_undefined());
    if set(slots.first()) {
        return Err(agent.type_error(messages::PROMISE_EXECUTOR_TWICE));
    }
    if set(slots.get(1)) {
        return Err(agent.type_error(messages::PROMISE_EXECUTOR_TWICE));
    }
    let (resolve, reject) = (agent.arg(c, 0), agent.arg(c, 1));
    if let Some(o) = c.callee.as_object() {
        agent
            .heap
            .with_internal_slots(o, |s| *s = vec![resolve, reject]);
    }
    Ok(Value::UNDEFINED)
}

/// `this` as a promise, for `then`.
fn this_promise(agent: &mut Agent, c: Call, name: &str) -> Res<Value> {
    if agent.promise_state(c.this).is_some() {
        return Ok(c.this);
    }
    let text = agent.display(c.this);
    Err(agent.type_error(messages::incompatible_receiver(name, &text)))
}

/// `Promise.prototype.then` (section 27.2.5.4).
fn then(agent: &mut Agent, c: Call) -> Res<Value> {
    let promise = this_promise(agent, c, "Promise.prototype.then")?;
    let (on_fulfilled, on_rejected) = (agent.arg(c, 0), agent.arg(c, 1));
    let ctor = agent.promise_species(promise)?;
    let cap = agent.new_capability(ctor)?;
    agent.perform_then(promise, on_fulfilled, on_rejected, Some(cap));
    Ok(cap.promise)
}

/// `Promise.prototype.catch` (section 27.2.5.1): `this.then(undefined,
/// onRejected)`.
fn catch(agent: &mut Agent, c: Call) -> Res<Value> {
    let key = agent.then_key();
    let then = agent.get_value(c.this, key)?;
    let on_rejected = agent.arg(c, 0);
    agent.call_value(then, c.this, &[Value::UNDEFINED, on_rejected])
}

/// `Promise.prototype.finally` (section 27.2.5.3).
fn finally(agent: &mut Agent, c: Call) -> Res<Value> {
    if c.this.as_object().is_none() {
        let text = agent.display(c.this);
        return Err(agent.type_error(messages::incompatible_receiver(
            "Promise.prototype.finally",
            &text,
        )));
    }
    let on_finally = agent.arg(c, 0);
    let ctor = agent.promise_species(c.this)?;
    let (then_finally, catch_finally) = if agent.is_callable(on_finally) {
        let a = agent.native_function(internal_id(THEN_FINALLY), 1, "")?;
        agent.vm.temps.push(a);
        let b = agent.native_function(internal_id(CATCH_FINALLY), 1, "");
        agent.vm.temps.pop();
        let b = b?;
        for f in [a, b] {
            if let Some(o) = f.as_object() {
                agent
                    .heap
                    .with_internal_slots(o, |s| *s = vec![on_finally, ctor]);
            }
        }
        (a, b)
    } else {
        (on_finally, on_finally)
    };
    agent.vm.temps.extend([then_finally, catch_finally]);
    let key = agent.then_key();
    let r = agent
        .get_value(c.this, key)
        .and_then(|then| agent.call_value(then, c.this, &[then_finally, catch_finally]));
    let n = agent.vm.temps.len();
    agent.vm.temps.truncate(n.saturating_sub(2));
    r
}

/// `finally`'s callbacks: `onFinally()`, then `C.resolve(result)` and a
/// thunk giving the value back (or a thrower throwing the reason).
fn finally_step(agent: &mut Agent, c: Call, thrower_: bool) -> Res<Value> {
    let slots = agent.slots_of(c.callee);
    let on_finally = slots.first().copied().unwrap_or(Value::UNDEFINED);
    let ctor = slots.get(1).copied().unwrap_or(Value::UNDEFINED);
    let value = agent.arg(c, 0);
    let result = agent.call_value(on_finally, Value::UNDEFINED, &[])?;
    let promise = agent.promise_resolve(ctor, result)?;
    agent.vm.temps.push(promise);
    let thunk = agent.native_function(
        internal_id(if thrower_ { THROWER } else { VALUE_THUNK }),
        0,
        "",
    );
    agent.vm.temps.pop();
    let thunk = thunk?;
    if let Some(o) = thunk.as_object() {
        agent.heap.with_internal_slots(o, |s| *s = vec![value]);
    }
    agent.vm.temps.extend([promise, thunk]);
    let key = agent.then_key();
    let r = agent
        .get_value(promise, key)
        .and_then(|then| agent.call_value(then, promise, &[thunk]));
    let n = agent.vm.temps.len();
    agent.vm.temps.truncate(n.saturating_sub(2));
    r
}

fn then_finally(agent: &mut Agent, c: Call) -> Res<Value> {
    finally_step(agent, c, false)
}

fn catch_finally(agent: &mut Agent, c: Call) -> Res<Value> {
    finally_step(agent, c, true)
}

impl Agent {
    /// AsyncGeneratorAwaitReturn's reactions, with no generator yet.
    pub(crate) fn async_gen_return_functions(&mut self) -> Res<(Value, Value)> {
        let f = self.native_function(internal_id(ASYNC_GEN_RETURN_FULFILLED), 1, "")?;
        self.vm.temps.push(f);
        let r = self.native_function(internal_id(ASYNC_GEN_RETURN_REJECTED), 1, "");
        self.vm.temps.pop();
        Ok((f, r?))
    }
}

fn async_gen_return_fulfilled(agent: &mut Agent, c: Call) -> Res<Value> {
    let g = agent
        .slots_of(c.callee)
        .first()
        .copied()
        .unwrap_or(Value::UNDEFINED);
    let v = agent.arg(c, 0);
    agent.async_gen_return_settled(g, v, false)?;
    Ok(Value::UNDEFINED)
}

fn async_gen_return_rejected(agent: &mut Agent, c: Call) -> Res<Value> {
    let g = agent
        .slots_of(c.callee)
        .first()
        .copied()
        .unwrap_or(Value::UNDEFINED);
    let v = agent.arg(c, 0);
    agent.async_gen_return_settled(g, v, true)?;
    Ok(Value::UNDEFINED)
}

impl Agent {
    /// CreateAsyncFromSyncIterator (section 27.1.6.1): the wrapper and its
    /// `next`.
    pub(crate) fn async_from_sync_iterator(
        &mut self,
        sync: Value,
        next: Value,
    ) -> Res<(Value, Value)> {
        let proto = self.realm().intrinsics.async_from_sync_iterator_prototype;
        self.vm.temps.extend([sync, next]);
        let o = self.alloc_object(ObjectCell::with_internal(
            proto,
            ObjectKind::AsyncFromSyncIterator,
            vec![sync, next],
        ));
        let n = self.vm.temps.len();
        self.vm.temps.truncate(n.saturating_sub(2));
        let o = o?;
        self.vm.temps.push(o);
        let key = self.name_key(Name::Str("next"));
        let method = self.get_value(o, key);
        self.vm.temps.pop();
        Ok((o, method?))
    }

    /// AsyncFromSyncIteratorContinuation (section 27.1.6.4): `promise`
    /// settled from the sync result once its value is awaited; a
    /// rejected value closes the sync iterator if `close`.
    fn async_from_sync_continue(
        &mut self,
        result: Value,
        promise: Value,
        sync: Value,
        close: bool,
    ) -> Res<()> {
        let key = self.name_key(Name::Str("done"));
        let done = match self.get_value(result, key) {
            Ok(d) => self.to_boolean(d),
            Err(e) => return self.reject_abrupt_promise(promise, e),
        };
        let key = self.name_key(Name::Str("value"));
        let value = match self.get_value(result, key) {
            Ok(v) => v,
            Err(e) => return self.reject_abrupt_promise(promise, e),
        };
        let c = self.realm().intrinsics.promise;
        let wrapper = match self.promise_resolve(c, value) {
            Ok(w) => w,
            Err(Abrupt::Throw) if !done && close => {
                let e = std::mem::replace(&mut self.vm.exception, Value::UNDEFINED);
                let _ = self.iterator_close_quiet(sync);
                return self.reject_promise(promise, e);
            }
            Err(e) => return self.reject_abrupt_promise(promise, e),
        };
        self.vm.temps.push(wrapper);
        let r = (|| {
            let on_f = self.native_function(internal_id(ASYNC_FROM_SYNC_UNWRAP), 1, "")?;
            if let Some(o) = on_f.as_object() {
                self.heap
                    .with_internal_slots(o, |s| *s = vec![Value::bool(done)]);
            }
            self.vm.temps.push(on_f);
            let on_r = if done || !close {
                Ok(Value::UNDEFINED)
            } else {
                self.native_function(internal_id(ASYNC_FROM_SYNC_CLOSE_REJECT), 1, "")
                    .inspect(|f| {
                        if let Some(o) = f.as_object() {
                            self.heap.with_internal_slots(o, |s| *s = vec![sync]);
                        }
                    })
            };
            self.vm.temps.pop();
            let on_r = on_r?;
            self.perform_then(
                wrapper,
                on_f,
                on_r,
                Some(Capability {
                    promise,
                    resolve: Value::UNDEFINED,
                    reject: Value::UNDEFINED,
                }),
            );
            Ok(())
        })();
        self.vm.temps.pop();
        r
    }

    /// A promise rejected with what an abrupt step threw.
    fn reject_abrupt_promise(&mut self, promise: Value, e: Abrupt) -> Res<()> {
        match e {
            Abrupt::Throw => {
                let x = std::mem::replace(&mut self.vm.exception, Value::UNDEFINED);
                self.reject_promise(promise, x)
            }
            other => Err(other),
        }
    }
}

/// `this`'s sync iterator and its `next`.
fn sync_of(agent: &Agent, c: Call) -> (Value, Value) {
    let s = agent.slots_of(c.this);
    (
        s.first().copied().unwrap_or(Value::UNDEFINED),
        s.get(1).copied().unwrap_or(Value::UNDEFINED),
    )
}

/// The value a method passes on: its argument, if it was given one.
fn passed(agent: &Agent, c: Call) -> Vec<Value> {
    if c.argc > 0 {
        vec![agent.arg(c, 0)]
    } else {
        Vec::new()
    }
}

/// One of the methods, `step` giving the sync result (or `Ok(None)` when
/// it settled the promise itself), then the continuation.
fn async_from_sync(
    agent: &mut Agent,
    c: Call,
    close: bool,
    step: impl FnOnce(&mut Agent, Value, Value) -> Res<Option<Value>>,
) -> Res<Value> {
    let promise = agent.new_promise()?;
    agent.vm.temps.push(promise);
    let (sync, next) = sync_of(agent, c);
    let r = match step(agent, sync, promise) {
        Ok(Some(result)) => {
            if result.as_object().is_none() {
                let text = agent.display(result);
                agent
                    .make_error(
                        ErrorType::TypeError,
                        &messages::iterator_result_not_object(&text),
                    )
                    .and_then(|e| agent.reject_promise(promise, e))
            } else {
                agent.vm.temps.push(result);
                let r = agent.async_from_sync_continue(result, promise, sync, close);
                agent.vm.temps.pop();
                r
            }
        }
        Ok(None) => Ok(()),
        Err(e) => agent.reject_abrupt_promise(promise, e),
    };
    let _ = next;
    agent.vm.temps.pop();
    r.map(|()| promise)
}

fn async_from_sync_next(agent: &mut Agent, c: Call) -> Res<Value> {
    let args = passed(agent, c);
    let (_, next) = sync_of(agent, c);
    async_from_sync(agent, c, true, |agent, sync, _| {
        agent.call_value(next, sync, &args).map(Some)
    })
}

fn async_from_sync_return(agent: &mut Agent, c: Call) -> Res<Value> {
    let args = passed(agent, c);
    let value = agent.arg(c, 0);
    async_from_sync(agent, c, false, |agent, sync, promise| {
        let key = agent.name_key(Name::Str("return"));
        let ret = agent.get_value(sync, key)?;
        if ret.is_nullish() {
            let result = agent.iter_result(value, true)?;
            agent.resolve_promise(promise, result)?;
            return Ok(None);
        }
        agent.call_value(ret, sync, &args).map(Some)
    })
}

fn async_from_sync_throw(agent: &mut Agent, c: Call) -> Res<Value> {
    let args = passed(agent, c);
    async_from_sync(agent, c, true, |agent, sync, promise| {
        let key = agent.name_key(Name::Str("throw"));
        let thr = agent.get_value(sync, key)?;
        if thr.is_nullish() {
            // No throw method: the sync iterator closed, then a TypeError.
            agent.iterator_close(sync)?;
            let e = agent.make_error(ErrorType::TypeError, messages::DELEGATE_NO_THROW)?;
            agent.reject_promise(promise, e)?;
            return Ok(None);
        }
        agent.call_value(thr, sync, &args).map(Some)
    })
}

fn async_from_sync_unwrap(agent: &mut Agent, c: Call) -> Res<Value> {
    let done = agent.slots_of(c.callee).first().copied() == Some(Value::TRUE);
    let v = agent.arg(c, 0);
    agent.iter_result(v, done)
}

fn async_from_sync_close_reject(agent: &mut Agent, c: Call) -> Res<Value> {
    let sync = agent
        .slots_of(c.callee)
        .first()
        .copied()
        .unwrap_or(Value::UNDEFINED);
    let e = agent.arg(c, 0);
    let _ = agent.iterator_close_quiet(sync);
    agent.vm.exception = e;
    Err(Abrupt::Throw)
}

fn value_thunk(agent: &mut Agent, c: Call) -> Res<Value> {
    Ok(agent
        .slots_of(c.callee)
        .first()
        .copied()
        .unwrap_or(Value::UNDEFINED))
}

fn thrower(agent: &mut Agent, c: Call) -> Res<Value> {
    agent.vm.exception = agent
        .slots_of(c.callee)
        .first()
        .copied()
        .unwrap_or(Value::UNDEFINED);
    Err(Abrupt::Throw)
}

/// `this` as a constructor for the statics: an object (PromiseResolve's
/// check).
fn this_ctor(agent: &mut Agent, c: Call) -> Res<Value> {
    if c.this.as_object().is_none() {
        return Err(agent.type_error(messages::PROMISE_RESOLVE_NON_OBJECT));
    }
    Ok(c.this)
}

/// `Promise.resolve` (section 27.2.4.7).
fn resolve_static(agent: &mut Agent, c: Call) -> Res<Value> {
    let ctor = this_ctor(agent, c)?;
    let x = agent.arg(c, 0);
    agent.promise_resolve(ctor, x)
}

/// `Promise.reject` (section 27.2.4.6).
fn reject_static(agent: &mut Agent, c: Call) -> Res<Value> {
    let ctor = this_ctor(agent, c)?;
    let cap = agent.new_capability(ctor)?;
    let r = agent.arg(c, 0);
    agent.vm.temps.push(cap.promise);
    let done = agent.settle_capability(cap, r, true);
    agent.vm.temps.pop();
    done.map(|()| cap.promise)
}

/// `Promise.withResolvers` (section 27.2.4.8).
fn with_resolvers(agent: &mut Agent, c: Call) -> Res<Value> {
    let ctor = this_ctor(agent, c)?;
    let cap = agent.new_capability(ctor)?;
    let (resolve, reject) = if cap.resolve.is_undefined() {
        agent.vm.temps.push(cap.promise);
        let fns = agent.resolving_functions(cap.promise);
        agent.vm.temps.pop();
        fns?
    } else {
        (cap.resolve, cap.reject)
    };
    agent.vm.temps.extend([cap.promise, resolve, reject]);
    let r = (|| {
        let proto = agent.realm().object_prototype;
        let o = agent.alloc_object(ObjectCell::new(proto, ObjectKind::Ordinary))?;
        agent.vm.temps.push(o);
        for (name, v) in [
            ("promise", cap.promise),
            ("resolve", resolve),
            ("reject", reject),
        ] {
            let key = agent.name_key(Name::Str(name));
            agent.create_data_property(o, key, v);
        }
        agent.vm.temps.pop();
        Ok(o)
    })();
    let n = agent.vm.temps.len();
    agent.vm.temps.truncate(n.saturating_sub(3));
    r
}

/// `Promise.try` (section 27.2.4.8 of ES2025).
fn try_(agent: &mut Agent, c: Call) -> Res<Value> {
    // The callback first; a value given through PromiseResolve, so that
    // a promise of the receiver comes back as it is (the 2026 change,
    // which the reference browser follows), a throw rejected.
    let ctor = this_ctor(agent, c)?;
    let f = agent.arg(c, 0);
    let args: Vec<Value> = (1..c.argc).map(|i| agent.arg(c, i)).collect();
    match agent.call_value(f, Value::UNDEFINED, &args) {
        Ok(v) => {
            agent.vm.temps.push(v);
            let r = agent.promise_resolve(ctor, v);
            agent.vm.temps.pop();
            r
        }
        Err(Abrupt::Throw) => {
            let e = std::mem::replace(&mut agent.vm.exception, Value::UNDEFINED);
            agent.vm.temps.push(e);
            let r = agent.new_capability(ctor).and_then(|cap| {
                agent.vm.temps.push(cap.promise);
                let r = agent.settle_capability(cap, e, true);
                agent.vm.temps.pop();
                r.map(|()| cap.promise)
            });
            agent.vm.temps.pop();
            r
        }
        Err(e) => Err(e),
    }
}

/// `get Promise[@@species]`.
fn species(_: &mut Agent, c: Call) -> Res<Value> {
    Ok(c.this)
}

/// The shared record of a combinator: its values (an array), how many
/// are left to settle (one more than the count while iterating), and the
/// capability, in a holder object's internal slots.
fn combinator_record(agent: &mut Agent, cap: Capability) -> Res<Value> {
    let values = agent.array_of(&[])?;
    agent.vm.temps.push(values);
    let proto = agent.realm().object_prototype;
    let r = agent.alloc_object(ObjectCell::with_internal(
        proto,
        ObjectKind::Ordinary,
        vec![values, Value::int(1), cap.promise, cap.resolve, cap.reject],
    ));
    agent.vm.temps.pop();
    r
}

/// A combinator element function, holding its record, its index and
/// whether it was called.
fn element_function(agent: &mut Agent, which: u32, record: Value, index: u32) -> Res<Value> {
    let f = agent.native_function(internal_id(which), 1, "")?;
    if let Some(o) = f.as_object() {
        agent.heap.with_internal_slots(o, |s| {
            *s = vec![record, Value::number(f64::from(index)), Value::FALSE];
        });
    }
    Ok(f)
}

fn record_capability(agent: &Agent, record: Value) -> Capability {
    let s = agent.slots_of(record);
    let at = |i: usize| s.get(i).copied().unwrap_or(Value::UNDEFINED);
    Capability {
        promise: at(2),
        resolve: at(3),
        reject: at(4),
    }
}

/// One element settled: its value kept; when none is left, the
/// capability resolved with the values (or for `any`, rejected with an
/// AggregateError of them).
fn element_settled(agent: &mut Agent, c: Call, value: Value, aggregate: bool) -> Res<Value> {
    let slots = agent.slots_of(c.callee);
    if slots.get(2).copied() == Some(Value::TRUE) {
        return Ok(Value::UNDEFINED);
    }
    if let Some(o) = c.callee.as_object() {
        agent.heap.with_internal_slots(o, |s| {
            if let Some(v) = s.get_mut(2) {
                *v = Value::TRUE;
            }
        });
    }
    let record = slots.first().copied().unwrap_or(Value::UNDEFINED);
    let index = slots.get(1).and_then(|v| v.as_number()).unwrap_or(0.0) as u32;
    let values = agent
        .slots_of(record)
        .first()
        .copied()
        .unwrap_or(Value::UNDEFINED);
    agent.create_data_property(values, PropertyKey::Index(index), value);
    if agent.combinator_done(record) {
        let cap = record_capability(agent, record);
        if aggregate {
            let e = agent.aggregate_error(values)?;
            agent.settle_capability(cap, e, true)?;
        } else {
            agent.settle_capability(cap, values, false)?;
        }
    }
    Ok(Value::UNDEFINED)
}

impl Agent {
    /// One fewer left; whether none is.
    fn combinator_done(&mut self, record: Value) -> bool {
        let Some(o) = record.as_object() else {
            return false;
        };
        self.heap
            .with_internal_slots(o, |s| {
                let left = s.get(1).and_then(|v| v.as_int()).unwrap_or(1) - 1;
                if let Some(v) = s.get_mut(1) {
                    *v = Value::int(left);
                }
                left == 0
            })
            .unwrap_or(false)
    }

    /// One more left.
    fn combinator_add(&mut self, record: Value) {
        if let Some(o) = record.as_object() {
            self.heap.with_internal_slots(o, |s| {
                let left = s.get(1).and_then(|v| v.as_int()).unwrap_or(1) + 1;
                if let Some(v) = s.get_mut(1) {
                    *v = Value::int(left);
                }
            });
        }
    }

    /// An `AggregateError` of the array `errors`, `All promises were
    /// rejected`.
    fn aggregate_error(&mut self, errors: Value) -> Res<Value> {
        let e = self.make_error(ErrorType::AggregateError, messages::ALL_REJECTED)?;
        self.vm.temps.push(e);
        let key = self.name_key(Name::Str("errors"));
        let desc = crate::object::Descriptor {
            value: Some(errors),
            writable: Some(true),
            enumerable: Some(false),
            configurable: Some(true),
            ..crate::object::Descriptor::default()
        };
        if let Some(o) = e.as_object() {
            self.heap.define_own_property(o, key, desc);
        }
        self.vm.temps.pop();
        Ok(e)
    }
}

fn all_resolve_element(agent: &mut Agent, c: Call) -> Res<Value> {
    let v = agent.arg(c, 0);
    element_settled(agent, c, v, false)
}

fn settled_record(agent: &mut Agent, status: &str, key: &'static str, v: Value) -> Res<Value> {
    let proto = agent.realm().object_prototype;
    agent.vm.temps.push(v);
    let o = agent.alloc_object(ObjectCell::new(proto, ObjectKind::Ordinary));
    agent.vm.temps.pop();
    let o = o?;
    agent.vm.temps.push(o);
    let s = agent.new_string(status);
    let r = s.map(|s| {
        let k = agent.name_key(Name::Str("status"));
        agent.create_data_property(o, k, s);
        let k = agent.name_key(Name::Str(key));
        agent.create_data_property(o, k, v);
        o
    });
    agent.vm.temps.pop();
    r
}

fn all_settled_resolve_element(agent: &mut Agent, c: Call) -> Res<Value> {
    let v = agent.arg(c, 0);
    let o = settled_record(agent, "fulfilled", "value", v)?;
    agent.vm.temps.push(o);
    let r = element_settled(agent, c, o, false);
    agent.vm.temps.pop();
    r
}

fn all_settled_reject_element(agent: &mut Agent, c: Call) -> Res<Value> {
    let v = agent.arg(c, 0);
    let o = settled_record(agent, "rejected", "reason", v)?;
    agent.vm.temps.push(o);
    let r = element_settled(agent, c, o, false);
    agent.vm.temps.pop();
    r
}

fn any_reject_element(agent: &mut Agent, c: Call) -> Res<Value> {
    let v = agent.arg(c, 0);
    element_settled(agent, c, v, true)
}

/// Which combinator.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Combinator {
    All,
    AllSettled,
    Any,
    Race,
}

/// `Promise.all`, `allSettled`, `any` and `race` (sections 27.2.4.1 to
/// 27.2.4.5).
fn combine(agent: &mut Agent, c: Call, which: Combinator) -> Res<Value> {
    let ctor = c.this;
    let cap = agent.new_capability(ctor)?;
    agent.vm.temps.push(cap.promise);
    // A native capability's one pair of functions, which every element
    // shares: the promise is settled once.
    let cap = match agent.with_functions(cap) {
        Ok(cap) => cap,
        Err(e) => {
            agent.vm.temps.pop();
            return Err(e);
        }
    };
    agent.vm.temps.extend([cap.resolve, cap.reject]);
    let r = (|| {
        let key = agent.name_key(Name::Str("resolve"));
        let resolve = match agent.get_value(ctor, key) {
            Ok(r) if agent.is_callable(r) => r,
            Ok(r) => {
                let text = agent.display(r);
                let e = agent.type_error(messages::promise_resolve_not_function(&text));
                return agent.reject_abrupt(cap, Err(e));
            }
            Err(e) => return agent.reject_abrupt(cap, Err(e)),
        };
        agent.vm.temps.push(resolve);
        let record = if which == Combinator::Race {
            Value::UNDEFINED
        } else {
            match combinator_record(agent, cap) {
                Ok(r) => r,
                Err(e) => {
                    agent.vm.temps.pop();
                    return Err(e);
                }
            }
        };
        agent.vm.temps.push(record);
        let iterable = agent.arg(c, 0);
        let counted = agent.each_promise(iterable, |agent, value, index| {
            let next = agent.call_value(resolve, ctor, &[value])?;
            agent.vm.temps.push(next);
            let fns = match which {
                Combinator::Race => Ok((cap.resolve, cap.reject)),
                Combinator::All => {
                    element_function(agent, ALL_RESOLVE, record, index).map(|f| (f, cap.reject))
                }
                Combinator::AllSettled => {
                    element_function(agent, ALL_SETTLED_RESOLVE, record, index).and_then(|f| {
                        agent.vm.temps.push(f);
                        let g = element_function(agent, ALL_SETTLED_REJECT, record, index);
                        agent.vm.temps.pop();
                        g.map(|g| (f, g))
                    })
                }
                Combinator::Any => {
                    element_function(agent, ANY_REJECT, record, index).map(|f| (cap.resolve, f))
                }
            };
            let r = fns.and_then(|(on_f, on_r)| {
                agent.vm.temps.extend([on_f, on_r]);
                if which != Combinator::Race {
                    agent.combinator_add(record);
                    let values = agent.slots_of(record).first().copied();
                    if let Some(values) = values {
                        agent.create_data_property(
                            values,
                            PropertyKey::Index(index),
                            Value::UNDEFINED,
                        );
                    }
                }
                let key = agent.then_key();
                let r = agent
                    .get_value(next, key)
                    .and_then(|then| agent.call_value(then, next, &[on_f, on_r]))
                    .map(|_| ());
                let n = agent.vm.temps.len();
                agent.vm.temps.truncate(n.saturating_sub(2));
                r
            });
            agent.vm.temps.pop();
            r
        });
        let r = match counted {
            Ok(_) if which == Combinator::Race => Ok(()),
            Ok(_) => {
                if agent.combinator_done(record) {
                    let values = agent
                        .slots_of(record)
                        .first()
                        .copied()
                        .unwrap_or(Value::UNDEFINED);
                    if which == Combinator::Any {
                        agent
                            .aggregate_error(values)
                            .and_then(|e| agent.settle_capability(cap, e, true))
                    } else {
                        agent.settle_capability(cap, values, false)
                    }
                } else {
                    Ok(())
                }
            }
            Err(e) => Err(e),
        };
        agent.vm.temps.pop();
        agent.vm.temps.pop();
        agent.reject_abrupt(cap, r)
    })();
    let n = agent.vm.temps.len();
    agent.vm.temps.truncate(n.saturating_sub(3));
    r
}

impl Agent {
    /// `cap` with functions of its own: a native capability given a
    /// pair of resolving functions for its promise.
    fn with_functions(&mut self, cap: Capability) -> Res<Capability> {
        if !cap.resolve.is_undefined() {
            return Ok(cap);
        }
        let (resolve, reject) = self.resolving_functions(cap.promise)?;
        Ok(Capability {
            promise: cap.promise,
            resolve,
            reject,
        })
    }
}

fn all(agent: &mut Agent, c: Call) -> Res<Value> {
    combine(agent, c, Combinator::All)
}

fn all_settled(agent: &mut Agent, c: Call) -> Res<Value> {
    combine(agent, c, Combinator::AllSettled)
}

fn any(agent: &mut Agent, c: Call) -> Res<Value> {
    combine(agent, c, Combinator::Any)
}

fn race(agent: &mut Agent, c: Call) -> Res<Value> {
    combine(agent, c, Combinator::Race)
}
