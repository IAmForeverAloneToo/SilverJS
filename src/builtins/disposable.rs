//! Explicit resource management (the proposal ECMA-262 has taken in,
//! sections 27.3, 27.4 and 9.1.1.1): `DisposableStack`,
//! `AsyncDisposableStack`, and the dispose capability a block with a
//! `using` declaration keeps. All three keep their resources the same
//! way, in internal slots: the stack's state, DisposeResources' two
//! flags, the throw completion so far, then each resource as its value,
//! its dispose method and its hint. DisposeResources is a step at a
//! time ([`Agent::dispose_step`]), so that a block's code and
//! `disposeAsync` can await between steps. The messages are the
//! reference browser's (the J4.4 probe, Chrome 158).

use super::{CONFIGURABLE, Call, Entry, HIDDEN, Name, What, Wk, entry, getter, method};
use crate::agent::Agent;
use crate::heap::ObjectCell;
use crate::interp::ops::hidden;
use crate::interp::{Abrupt, ErrorType, Res};
use crate::object::ObjectKind;
use crate::value::Value;

/// `DisposableStack.prototype`, in the reference browser's order.
pub(crate) const DISPOSABLE_STACK_PROTO: &[Entry] = &[
    super::CONSTRUCTOR,
    method("use", 1, use_),
    method("dispose", 0, dispose),
    method("adopt", 2, adopt),
    method("defer", 1, defer),
    method("move", 0, move_),
    getter("disposed", disposed),
    entry(Name::Sym(Wk::Dispose), What::Alias("dispose"), HIDDEN),
    entry(
        Name::Sym(Wk::ToStringTag),
        What::Str("DisposableStack"),
        CONFIGURABLE,
    ),
];

/// `AsyncDisposableStack.prototype`, in the reference browser's order.
pub(crate) const ASYNC_DISPOSABLE_STACK_PROTO: &[Entry] = &[
    super::CONSTRUCTOR,
    method("use", 1, async_use),
    method("disposeAsync", 0, dispose_async),
    method("adopt", 2, async_adopt),
    method("defer", 1, async_defer),
    method("move", 0, async_move),
    getter("disposed", async_disposed),
    entry(
        Name::Sym(Wk::AsyncDispose),
        What::Alias("disposeAsync"),
        HIDDEN,
    ),
    entry(
        Name::Sym(Wk::ToStringTag),
        What::Str("AsyncDisposableStack"),
        CONFIGURABLE,
    ),
];

/// The natives `disposeAsync` makes for itself, installed nowhere: its
/// steps' reactions to what it awaits.
pub(crate) const DISPOSABLE_INTERNAL: &[Entry] = &[
    method("disposeFulfilled", 1, dispose_fulfilled),
    method("disposeRejected", 1, dispose_rejected),
    method("", 1, return_undefined),
];

const FULFILLED_FN: u32 = 0;
const REJECTED_FN: u32 = 1;
const UNDEFINED_FN: u32 = 2;

// A dispose capability's internal slots.
const S_STATE: usize = 0;
const S_NEEDS_AWAIT: usize = 1;
const S_HAS_AWAITED: usize = 2;
const S_THREW: usize = 3;
const S_ERROR: usize = 4;
/// The resources from here, three slots each: value, method, hint.
const S_RESOURCES: usize = 5;

const PENDING: i32 = 0;
const DISPOSED: i32 = 1;

/// A resource's hint: async-dispose, and how its method is called.
const ASYNC: i32 = 1;
/// `adopt`'s: the method called with the value as its argument.
const ADOPT: i32 = 2;
/// An async-dispose resource whose method is its sync `[Symbol.dispose]`
/// (GetDisposeMethod's closure): called, its result dropped, a promise.
const FROM_SYNC: i32 = 4;

/// The disposal message, the reference browser's.
const SUPPRESSED: &str = "An error was suppressed during disposal";
const NOT_OBJECT: &str = "An object is expected with `using` declarations";

/// What a step of DisposeResources leaves to its caller.
pub(crate) enum Step {
    /// Nothing left: the completion is in the capability.
    Done,
    /// A step taken, nothing to await.
    Next,
    /// A step taken whose value is to be awaited.
    Await(Value),
}

impl Agent {
    fn dslot(&self, o: Value, i: usize) -> Value {
        o.as_object()
            .and_then(|o| self.heap.objects.get(o))
            .and_then(|c| c.internal().get(i).copied())
            .unwrap_or(Value::UNDEFINED)
    }

    fn set_dslot(&mut self, o: Value, i: usize, v: Value) {
        if let Some(cell) = o.as_object().and_then(|o| self.heap.objects.get_mut(o)) {
            cell.set_internal(i, v);
        }
    }

    /// A new dispose capability of `kind`, its prototype `proto`.
    pub(crate) fn new_dispose_capability(&mut self, proto: Value, kind: ObjectKind) -> Res<Value> {
        self.alloc_object(ObjectCell::with_internal(
            proto,
            kind,
            vec![
                Value::int(PENDING),
                Value::FALSE,
                Value::FALSE,
                Value::FALSE,
                Value::UNDEFINED,
            ],
        ))
    }

    fn push_resource(&mut self, cap: Value, v: Value, m: Value, hint: i32) {
        if let Some(o) = cap.as_object() {
            self.heap
                .with_internal_slots(o, |s| s.extend([v, m, Value::int(hint)]));
        }
    }

    /// AddDisposableResource (section 7.5.x) with no method given: `v`'s
    /// own, by GetDisposeMethod; a null or undefined value skipped (but
    /// awaited at the end, for `await using`).
    pub(crate) fn add_disposable(&mut self, cap: Value, v: Value, is_async: bool) -> Res<()> {
        if v.is_nullish() {
            if is_async {
                self.push_resource(cap, Value::UNDEFINED, Value::UNDEFINED, ASYNC);
            }
            return Ok(());
        }
        if v.as_object().is_none() {
            return Err(self.type_error(NOT_OBJECT));
        }
        let mut hint = if is_async { ASYNC } else { 0 };
        let mut m = Value::UNDEFINED;
        if is_async {
            m = self.dispose_method(v, Wk::AsyncDispose)?;
        }
        if m.is_undefined() {
            m = self.dispose_method(v, Wk::Dispose)?;
            if is_async {
                hint |= FROM_SYNC;
            }
        }
        if m.is_undefined() {
            return Err(self.type_error("Symbol(Symbol.dispose) is not a function"));
        }
        self.push_resource(cap, v, m, hint);
        Ok(())
    }

    /// GetMethod of `v`'s `key`, the reference browser's message naming
    /// the key, not the value.
    fn dispose_method(&mut self, v: Value, key: Wk) -> Res<Value> {
        let k = self.well_known_key(key);
        let m = self.get_value(v, k)?;
        if m.is_nullish() {
            return Ok(Value::UNDEFINED);
        }
        if !self.is_callable(m) {
            let name = match key {
                Wk::AsyncDispose => "Symbol(Symbol.asyncDispose)",
                _ => "Symbol(Symbol.dispose)",
            };
            return Err(self.type_error(format!("{name} is not a function")));
        }
        Ok(m)
    }

    /// A throw completion `e` met while disposing: kept, or, if one is
    /// kept already, the error of a new `SuppressedError` over it.
    pub(crate) fn dispose_throw(&mut self, cap: Value, e: Value) -> Res<()> {
        let kept = if self.dslot(cap, S_THREW) == Value::TRUE {
            let earlier = self.dslot(cap, S_ERROR);
            self.vm.temps.extend([cap, e, earlier]);
            let made = self.suppressed_error(e, earlier);
            let n = self.vm.temps.len();
            self.vm.temps.truncate(n.saturating_sub(3));
            made?
        } else {
            e
        };
        self.set_dslot(cap, S_THREW, Value::TRUE);
        self.set_dslot(cap, S_ERROR, kept);
        Ok(())
    }

    /// The thrown value just caught, as a throw completion of `cap`.
    fn dispose_caught(&mut self, cap: Value, a: Abrupt) -> Res<()> {
        match a {
            Abrupt::Throw => {
                let e = std::mem::replace(&mut self.vm.exception, Value::UNDEFINED);
                self.dispose_throw(cap, e)
            }
            other => Err(other),
        }
    }

    /// A `SuppressedError` of the disposal: `error` thrown while
    /// `suppressed` was.
    fn suppressed_error(&mut self, error: Value, suppressed: Value) -> Res<Value> {
        let o = self.make_error(ErrorType::SuppressedError, SUPPRESSED)?;
        if let Some(i) = o.as_object() {
            for (name, v) in [("error", error), ("suppressed", suppressed)] {
                let key = self.name_key(Name::Str(name));
                self.heap.define_own_property(i, key, hidden(v));
            }
        }
        Ok(o)
    }

    /// DisposeResources (section 7.5.x), one step: the last resource
    /// disposed, or an `Await(undefined)` it calls for first. A throw of
    /// the dispose method comes back as it is; the caller keeps it with
    /// [`Agent::dispose_throw`] and steps on.
    pub(crate) fn dispose_step(&mut self, cap: Value) -> Res<Step> {
        let n = cap
            .as_object()
            .and_then(|o| self.heap.objects.get(o))
            .map_or(0, |c| c.internal().len());
        let needs_await = self.dslot(cap, S_NEEDS_AWAIT) == Value::TRUE;
        let has_awaited = self.dslot(cap, S_HAS_AWAITED) == Value::TRUE;
        if n < S_RESOURCES + 3 {
            if needs_await && !has_awaited {
                self.set_dslot(cap, S_NEEDS_AWAIT, Value::FALSE);
                return Ok(Step::Await(Value::UNDEFINED));
            }
            return Ok(Step::Done);
        }
        let at = n - 3;
        let hint = self.dslot(cap, at + 2).as_int().unwrap_or(0);
        if hint & ASYNC == 0 && needs_await && !has_awaited {
            self.set_dslot(cap, S_NEEDS_AWAIT, Value::FALSE);
            return Ok(Step::Await(Value::UNDEFINED));
        }
        let (v, m) = (self.dslot(cap, at), self.dslot(cap, at + 1));
        if let Some(o) = cap.as_object() {
            self.heap.with_internal_slots(o, |s| s.truncate(at));
        }
        if m.is_undefined() {
            self.set_dslot(cap, S_NEEDS_AWAIT, Value::TRUE);
            return Ok(Step::Next);
        }
        self.vm.temps.extend([cap, v, m]);
        let called = if hint & ADOPT != 0 {
            self.call_value(m, Value::UNDEFINED, &[v])
        } else {
            self.call_value(m, v, &[])
        };
        let result = if hint & FROM_SYNC != 0 {
            // The closure's promise: undefined, or what the call threw.
            match called {
                Ok(_) => self.new_promise().and_then(|p| {
                    self.vm.temps.push(p);
                    let r = self.resolve_promise(p, Value::UNDEFINED);
                    self.vm.temps.pop();
                    r.map(|()| p)
                }),
                Err(Abrupt::Throw) => {
                    let e = std::mem::replace(&mut self.vm.exception, Value::UNDEFINED);
                    self.vm.temps.push(e);
                    let p = self.new_promise().and_then(|p| {
                        self.vm.temps.push(p);
                        let r = self.reject_promise(p, e);
                        self.vm.temps.pop();
                        r.map(|()| p)
                    });
                    self.vm.temps.pop();
                    p
                }
                Err(a) => Err(a),
            }
        } else {
            called
        };
        let n = self.vm.temps.len();
        self.vm.temps.truncate(n.saturating_sub(3));
        let result = result?;
        if hint & ASYNC != 0 {
            self.set_dslot(cap, S_HAS_AWAITED, Value::TRUE);
            return Ok(Step::Await(result));
        }
        Ok(Step::Next)
    }

    /// DisposeResources' end: the throw completion kept, thrown.
    pub(crate) fn dispose_end(&mut self, cap: Value) -> Res<()> {
        if self.dslot(cap, S_THREW) != Value::TRUE {
            return Ok(());
        }
        let e = self.dslot(cap, S_ERROR);
        self.set_dslot(cap, S_THREW, Value::FALSE);
        self.set_dslot(cap, S_ERROR, Value::UNDEFINED);
        self.vm.exception = e;
        Err(Abrupt::Throw)
    }

    /// DisposeResources with nothing to await: a `DisposableStack`'s.
    fn dispose_all(&mut self, cap: Value) -> Res<()> {
        loop {
            match self.dispose_step(cap) {
                Ok(Step::Done) => break,
                Ok(Step::Next | Step::Await(_)) => {}
                Err(a) => self.dispose_caught(cap, a)?,
            }
        }
        self.dispose_end(cap)
    }

    /// `disposeAsync`'s DisposeResources from where it stands: steps
    /// until one is to be awaited (the rest then run by its reactions),
    /// or to the end, which settles `promise`.
    fn dispose_async_run(&mut self, cap: Value, promise: Value) -> Res<()> {
        loop {
            match self.dispose_step(cap) {
                Ok(Step::Done) => {
                    if self.dslot(cap, S_THREW) == Value::TRUE {
                        let e = self.dslot(cap, S_ERROR);
                        return self.reject_promise(promise, e);
                    }
                    return self.resolve_promise(promise, Value::UNDEFINED);
                }
                Ok(Step::Next) => {}
                Ok(Step::Await(v)) => {
                    // Await(v): PromiseResolve(%Promise%, v), then on.
                    let ctor = self.realm().intrinsics.promise;
                    let p = match self.promise_resolve(ctor, v) {
                        Ok(p) => p,
                        Err(a) => {
                            self.dispose_caught(cap, a)?;
                            continue;
                        }
                    };
                    self.vm.temps.push(p);
                    let made = self.dispose_reactions(cap, promise);
                    self.vm.temps.pop();
                    let (f, r) = made?;
                    self.perform_then(p, f, r, None);
                    return Ok(());
                }
                Err(a) => self.dispose_caught(cap, a)?,
            }
        }
    }

    /// The two reactions of `disposeAsync`'s await, holding the
    /// capability and the promise it settles.
    fn dispose_reactions(&mut self, cap: Value, promise: Value) -> Res<(Value, Value)> {
        let mut made = Vec::new();
        for (i, length) in [(FULFILLED_FN, 1), (REJECTED_FN, 1)] {
            let id = super::native_id(super::table::DISPOSABLE_INTERNAL, i);
            match self.native_function(id, length, "") {
                Ok(f) => {
                    if let Some(o) = f.as_object() {
                        self.heap
                            .with_internal_slots(o, |s| *s = vec![cap, promise]);
                    }
                    self.vm.temps.push(f);
                    made.push(f);
                }
                Err(e) => {
                    let n = self.vm.temps.len();
                    self.vm.temps.truncate(n.saturating_sub(made.len()));
                    return Err(e);
                }
            }
        }
        let n = self.vm.temps.len();
        self.vm.temps.truncate(n.saturating_sub(2));
        match made.as_slice() {
            [f, r] => Ok((*f, *r)),
            _ => Ok((Value::UNDEFINED, Value::UNDEFINED)),
        }
    }

    /// A function that returns undefined (`%AsyncIteratorPrototype%`
    /// `[Symbol.asyncDispose]`'s unwrap).
    pub(crate) fn undefined_thunk(&mut self) -> Res<Value> {
        let id = super::native_id(super::table::DISPOSABLE_INTERNAL, UNDEFINED_FN);
        self.native_function(id, 1, "")
    }

    /// `this` as a stack of `kind`, or `method`'s incompatible receiver.
    fn this_stack(&mut self, this: Value, kind: ObjectKind, method: &str) -> Res<Value> {
        if self.kind_of(this) == Some(kind) {
            Ok(this)
        } else {
            Err(self.incompatible(method, this))
        }
    }

    /// The stack `this` of `kind`, pending, or `method`'s errors.
    fn pending_stack(
        &mut self,
        this: Value,
        kind: ObjectKind,
        name: &str,
        method: &str,
    ) -> Res<Value> {
        let full = format!("{name}.prototype.{method}");
        let s = self.this_stack(this, kind, &full)?;
        if self.dslot(s, S_STATE).as_int() == Some(DISPOSED) {
            return Err(self.throw_error(
                ErrorType::ReferenceError,
                format!("Cannot call {full} on an already-disposed DisposableStack"),
            ));
        }
        Ok(s)
    }

    /// A stack's resources moved to a new stack of `proto` (`move`).
    fn move_stack(&mut self, s: Value, proto: Value, kind: ObjectKind) -> Res<Value> {
        let n = self.new_dispose_capability(proto, kind)?;
        let taken = s
            .as_object()
            .map(|o| {
                let mut taken = Vec::new();
                self.heap.with_internal_slots(o, |v| {
                    taken = v.split_off(S_RESOURCES.min(v.len()));
                });
                taken
            })
            .unwrap_or_default();
        if let Some(o) = n.as_object() {
            self.heap.with_internal_slots(o, |v| v.extend(taken));
        }
        self.set_dslot(s, S_STATE, Value::int(DISPOSED));
        Ok(n)
    }
}

/// `DisposableStack()` (section 27.3.1.1).
pub(crate) fn construct(agent: &mut Agent, c: Call) -> Res<Value> {
    if c.new_target.is_undefined() {
        return Err(agent.type_error("Constructor DisposableStack requires 'new'"));
    }
    let proto = agent.prototype_from(c.new_target, |r| r.intrinsics.disposable_stack_prototype)?;
    agent.new_dispose_capability(proto, ObjectKind::DisposableStack)
}

/// `AsyncDisposableStack()` (section 27.4.1.1).
pub(crate) fn construct_async(agent: &mut Agent, c: Call) -> Res<Value> {
    if c.new_target.is_undefined() {
        return Err(agent.type_error("Constructor AsyncDisposableStack requires 'new'"));
    }
    let proto = agent.prototype_from(c.new_target, |r| {
        r.intrinsics.async_disposable_stack_prototype
    })?;
    agent.new_dispose_capability(proto, ObjectKind::AsyncDisposableStack)
}

const SYNC_NAME: &str = "DisposableStack";
const ASYNC_NAME: &str = "AsyncDisposableStack";

fn use_(agent: &mut Agent, c: Call) -> Res<Value> {
    let s = agent.pending_stack(c.this, ObjectKind::DisposableStack, SYNC_NAME, "use")?;
    let v = agent.arg(c, 0);
    agent.add_disposable(s, v, false)?;
    Ok(v)
}

fn async_use(agent: &mut Agent, c: Call) -> Res<Value> {
    let s = agent.pending_stack(c.this, ObjectKind::AsyncDisposableStack, ASYNC_NAME, "use")?;
    let v = agent.arg(c, 0);
    agent.add_disposable(s, v, true)?;
    Ok(v)
}

/// `adopt(value, onDispose)`: `onDispose(value)` at disposal.
fn adopt_with(agent: &mut Agent, c: Call, kind: ObjectKind, name: &str, hint: i32) -> Res<Value> {
    let s = agent.pending_stack(c.this, kind, name, "adopt")?;
    let (v, f) = (agent.arg(c, 0), agent.arg(c, 1));
    if !agent.is_callable(f) {
        let text = agent.display(f);
        return Err(agent.type_error(format!("{text} is not a function")));
    }
    agent.push_resource(s, v, f, hint | ADOPT);
    Ok(v)
}

fn adopt(agent: &mut Agent, c: Call) -> Res<Value> {
    adopt_with(agent, c, ObjectKind::DisposableStack, SYNC_NAME, 0)
}

fn async_adopt(agent: &mut Agent, c: Call) -> Res<Value> {
    adopt_with(
        agent,
        c,
        ObjectKind::AsyncDisposableStack,
        ASYNC_NAME,
        ASYNC,
    )
}

/// `defer(onDispose)`: `onDispose()` at disposal.
fn defer_with(agent: &mut Agent, c: Call, kind: ObjectKind, name: &str, hint: i32) -> Res<Value> {
    let s = agent.pending_stack(c.this, kind, name, "defer")?;
    let f = agent.arg(c, 0);
    if !agent.is_callable(f) {
        let text = agent.display(f);
        return Err(agent.type_error(format!("{text} is not a function")));
    }
    agent.push_resource(s, Value::UNDEFINED, f, hint);
    Ok(Value::UNDEFINED)
}

fn defer(agent: &mut Agent, c: Call) -> Res<Value> {
    defer_with(agent, c, ObjectKind::DisposableStack, SYNC_NAME, 0)
}

fn async_defer(agent: &mut Agent, c: Call) -> Res<Value> {
    defer_with(
        agent,
        c,
        ObjectKind::AsyncDisposableStack,
        ASYNC_NAME,
        ASYNC,
    )
}

fn move_(agent: &mut Agent, c: Call) -> Res<Value> {
    let s = agent.pending_stack(c.this, ObjectKind::DisposableStack, SYNC_NAME, "move")?;
    let proto = agent.realm().intrinsics.disposable_stack_prototype;
    agent.move_stack(s, proto, ObjectKind::DisposableStack)
}

fn async_move(agent: &mut Agent, c: Call) -> Res<Value> {
    let s = agent.pending_stack(c.this, ObjectKind::AsyncDisposableStack, ASYNC_NAME, "move")?;
    let proto = agent.realm().intrinsics.async_disposable_stack_prototype;
    agent.move_stack(s, proto, ObjectKind::AsyncDisposableStack)
}

/// `DisposableStack.prototype.dispose` (section 27.3.3.2).
fn dispose(agent: &mut Agent, c: Call) -> Res<Value> {
    let s = agent.this_stack(
        c.this,
        ObjectKind::DisposableStack,
        "DisposableStack.prototype.dispose",
    )?;
    if agent.dslot(s, S_STATE).as_int() == Some(DISPOSED) {
        return Ok(Value::UNDEFINED);
    }
    agent.set_dslot(s, S_STATE, Value::int(DISPOSED));
    agent.dispose_all(s)?;
    Ok(Value::UNDEFINED)
}

/// `AsyncDisposableStack.prototype.disposeAsync` (section 27.4.3.2): a
/// promise, rejected at once for what is no such stack.
fn dispose_async(agent: &mut Agent, c: Call) -> Res<Value> {
    let promise = agent.new_promise()?;
    agent.vm.temps.push(promise);
    let r = (|| {
        if agent.kind_of(c.this) != Some(ObjectKind::AsyncDisposableStack) {
            let e = agent.make_error(
                ErrorType::TypeError,
                "Receiver is not an AsyncDisposableStack",
            )?;
            return agent.reject_promise(promise, e);
        }
        if agent.dslot(c.this, S_STATE).as_int() == Some(DISPOSED) {
            return agent.resolve_promise(promise, Value::UNDEFINED);
        }
        agent.set_dslot(c.this, S_STATE, Value::int(DISPOSED));
        agent.dispose_async_run(c.this, promise)
    })();
    agent.vm.temps.pop();
    r.map(|()| promise)
}

fn reaction_slots(agent: &Agent, f: Value) -> (Value, Value) {
    (agent.dslot(f, 0), agent.dslot(f, 1))
}

fn return_undefined(_agent: &mut Agent, _c: Call) -> Res<Value> {
    Ok(Value::UNDEFINED)
}

fn dispose_fulfilled(agent: &mut Agent, c: Call) -> Res<Value> {
    let (cap, promise) = reaction_slots(agent, c.callee);
    agent.dispose_async_run(cap, promise)?;
    Ok(Value::UNDEFINED)
}

fn dispose_rejected(agent: &mut Agent, c: Call) -> Res<Value> {
    let (cap, promise) = reaction_slots(agent, c.callee);
    let e = agent.arg(c, 0);
    agent.dispose_throw(cap, e)?;
    agent.dispose_async_run(cap, promise)?;
    Ok(Value::UNDEFINED)
}

fn disposed_of(agent: &mut Agent, c: Call, kind: ObjectKind, method: &str) -> Res<Value> {
    let s = agent.this_stack(c.this, kind, method)?;
    Ok(Value::bool(
        agent.dslot(s, S_STATE).as_int() == Some(DISPOSED),
    ))
}

fn disposed(agent: &mut Agent, c: Call) -> Res<Value> {
    disposed_of(
        agent,
        c,
        ObjectKind::DisposableStack,
        "get DisposableStack.prototype.disposed",
    )
}

fn async_disposed(agent: &mut Agent, c: Call) -> Res<Value> {
    disposed_of(
        agent,
        c,
        ObjectKind::AsyncDisposableStack,
        "get AsyncDisposableStack.prototype.disposed",
    )
}
