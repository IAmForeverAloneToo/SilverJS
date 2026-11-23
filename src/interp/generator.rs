//! Generators (ECMA-262 section 27.5): a generator's frame suspended into
//! its object and pushed back to resume it.
//!
//! A suspended generator keeps, in its internal slots, its state, the
//! frame's fields and the frame's stretch of the register file (its
//! arguments and registers), copied as they were. `next`, `return` and
//! `throw` are not run as natives: the call pushes the frame back, on
//! the loop that called them, so that a generator resumes in the same
//! loop as a script call, never in a nested one. Its return completes it
//! with `{value, done: true}`; a throw out of it completes it too.

use super::messages;
use super::{Abrupt, ErrorType, Frame, MAX_FRAMES, NativeFrame, Res};
use crate::agent::Agent;
use crate::heap::ObjectCell;
use crate::object::ObjectKind;
use crate::value::Value;

/// A generator's state (its first internal slot).
pub(crate) const SUSPENDED_START: i32 = 0;
pub(crate) const SUSPENDED_YIELD: i32 = 1;
pub(crate) const EXECUTING: i32 = 2;
pub(crate) const COMPLETED: i32 = 3;
/// An async generator's, awaiting a return's value once completed.
pub(crate) const AWAITING_RETURN: i32 = 4;

/// How `yield*`'s call of the inner iterator went on.
pub(crate) enum DelegateCall {
    /// Its result (a native's), or the value received for a return the
    /// iterator had no method for.
    Value(Value),
    /// A frame pushed: the loop runs it, its result to come.
    Frame,
}

/// How a generator is resumed (and the value a yield's mode register
/// gets).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Resume {
    Next = 0,
    Throw = 1,
    Return = 2,
}

// The internal slots: the state, then the frame's fields, then its
// stretch of the register file.
const STATE: usize = 0;
const CLOSURE: usize = 1;
const THIS: usize = 2;
const NEW_TARGET: usize = 3;
const CONTEXT: usize = 4;
const FN_CONTEXT: usize = 5;
const UNIT: usize = 6;
const FUNC: usize = 7;
const RESUME: usize = 8;
const BASE: usize = 9;
const MODE: usize = 10;
/// An async function's promise; an async generator's queue, an object
/// whose internal slots are its requests, three values each (how, the
/// value sent, the promise to settle); undefined for a generator.
const OWNER: usize = 11;
const SAVED: usize = 12;

fn int(v: Option<&Value>) -> i64 {
    v.and_then(|v| v.as_number()).map_or(-1, |n| n as i64)
}

impl Agent {
    /// The generator's state, if `g` is one.
    pub(crate) fn generator_state(&self, g: Value) -> Option<i32> {
        let cell = self.heap.objects.get(g.as_object()?)?;
        (cell.kind() == ObjectKind::Generator).then(|| int(cell.internal().get(STATE)) as i32)
    }

    /// An async generator's state, if `g` is one.
    pub(crate) fn async_gen_state(&self, g: Value) -> Option<i32> {
        let cell = self.heap.objects.get(g.as_object()?)?;
        (cell.kind() == ObjectKind::AsyncGenerator).then(|| int(cell.internal().get(STATE)) as i32)
    }

    /// Whether `g` is an async generator.
    pub(crate) fn is_async_generator(&self, g: Value) -> bool {
        self.async_gen_state(g).is_some()
    }

    pub(crate) fn set_generator_state(&mut self, g: Value, state: i32) {
        if let Some(o) = g.as_object() {
            self.heap.with_internal_slots(o, |slots| {
                if state == COMPLETED {
                    // Its frame is no longer kept.
                    slots.truncate(SAVED);
                }
                if let Some(s) = slots.get_mut(STATE) {
                    *s = Value::int(state);
                }
            });
        }
    }

    /// The generator function whose call made `g` (its frame's closure;
    /// a stack frame of its `next` is named after it).
    pub(crate) fn generator_function(&self, g: Value) -> Option<Value> {
        let cell = self.heap.objects.get(g.as_object()?)?;
        matches!(
            cell.kind(),
            ObjectKind::Generator | ObjectKind::AsyncGenerator
        )
        .then(|| cell.internal().get(CLOSURE).copied())
        .flatten()
    }

    /// The top frame's slots for a generator, the stretch of the register
    /// file it owns copied after them; it resumes at `resume` with its
    /// mode in register `mode` (-1: none, its start).
    fn saved_frame(&self, state: i32, resume: u32, mode: i64) -> Vec<Value> {
        let Some(f) = self.vm.frames.last() else {
            return Vec::new();
        };
        let owner = f
            .generator
            .as_object()
            .and_then(|o| self.heap.objects.get(o))
            .and_then(|c| c.internal().get(OWNER).copied())
            .unwrap_or(Value::UNDEFINED);
        let mut slots = vec![
            Value::int(state),
            f.closure,
            f.this,
            f.new_target,
            f.context,
            f.fn_context,
            Value::number(f64::from(f.unit)),
            Value::number(f64::from(f.func)),
            Value::number(f64::from(resume)),
            Value::number(f64::from(f.base - f.args)),
            Value::number(mode as f64),
            owner,
        ];
        slots.extend_from_slice(self.vm.stack.get(f.args as usize..).unwrap_or(&[]));
        slots
    }

    /// Pops the top frame, as a return does, after its state is kept;
    /// gives it back (whether it was its run's entry).
    fn leave_generator_frame(&mut self) -> Option<Frame> {
        let frame = self.vm.frames.pop()?;
        self.vm.current = frame.caller_realm;
        self.vm.stack.truncate(frame.args as usize);
        self.pop_resume_frame();
        Some(frame)
    }

    /// The `next`, `return` or `throw` shown on the stack while the
    /// generator ran, taken off with it.
    pub(super) fn pop_resume_frame(&mut self) {
        let frames = u32::try_from(self.vm.frames.len()).unwrap_or(u32::MAX);
        if self
            .vm
            .natives
            .last()
            .is_some_and(|n| n.below == frames && n.this.as_object().is_some())
            && self
                .vm
                .natives
                .last()
                .is_some_and(|n| self.generator_state(n.this).is_some())
        {
            self.vm.natives.pop();
        }
    }

    /// `GeneratorStart`: the generator made from the running frame,
    /// which is suspended at `resume` in it; gives back the generator and
    /// the frame.
    #[inline(never)]
    pub(super) fn generator_start(&mut self, resume: u32) -> Res<(Value, Frame)> {
        let Some(top) = self.vm.frames.last().copied() else {
            return Err(Abrupt::Throw);
        };
        let kind = self
            .callable(top.closure)
            .and_then(|c| match c {
                super::Callable::Code { unit, func, .. } => self.code_kind(unit, func),
                _ => None,
            })
            .unwrap_or(crate::compiler::FunctionKind::Generator);
        let is_async = kind == crate::compiler::FunctionKind::AsyncGenerator;
        // OrdinaryCreateFromConstructor(F, "%GeneratorPrototype%"), or
        // "%AsyncGeneratorPrototype%".
        let proto = if is_async {
            self.prototype_from(top.closure, |r| r.intrinsics.async_generator_prototype)?
        } else {
            self.prototype_from(top.closure, |r| r.intrinsics.generator_prototype)?
        };
        self.vm.temps.push(proto);
        let mut slots = self.saved_frame(SUSPENDED_START, resume, -1);
        let queue = if is_async {
            let op = self.realm().object_prototype;
            self.alloc_object(ObjectCell::new(op, ObjectKind::Ordinary))
        } else {
            Ok(Value::UNDEFINED)
        };
        let queue = match queue {
            Ok(q) => q,
            Err(e) => {
                self.vm.temps.pop();
                return Err(e);
            }
        };
        if let Some(s) = slots.get_mut(OWNER) {
            *s = queue;
        }
        // The prototype and the queue rooted while the generator is made.
        self.vm.temps.push(queue);
        let g = self.alloc_object(ObjectCell::with_internal(
            proto,
            if is_async {
                ObjectKind::AsyncGenerator
            } else {
                ObjectKind::Generator
            },
            slots,
        ));
        self.vm
            .temps
            .truncate(self.vm.temps.len().saturating_sub(2));
        let g = g?;
        let frame = self.leave_generator_frame().ok_or(Abrupt::Throw)?;
        Ok((g, frame))
    }

    /// `Yield` and `YieldDelegate`: the running generator suspended at
    /// `resume`, its mode to come in register `mode`; gives back what its
    /// caller gets (`{value, done: false}`, or for a delegation the inner
    /// result as it is) and the frame.
    #[inline(never)]
    pub(super) fn generator_yield(
        &mut self,
        resume: u32,
        mode: usize,
        delegate: bool,
    ) -> Res<(Value, Frame)> {
        let Some(top) = self.vm.frames.last().copied() else {
            return Err(Abrupt::Throw);
        };
        let result = if delegate {
            self.vm.acc
        } else {
            let v = self.vm.acc;
            self.iter_result(v, false)?
        };
        let slots = self.saved_frame(SUSPENDED_YIELD, resume, mode as i64);
        if let Some(o) = top.generator.as_object() {
            self.heap.with_internal_slots(o, |s| *s = slots);
        }
        let frame = self.leave_generator_frame().ok_or(Abrupt::Throw)?;
        Ok((result, frame))
    }

    /// A generator's return: completed, its caller given `{value, done:
    /// true}`.
    #[inline(never)]
    pub(super) fn generator_return(&mut self, g: Value, value: Value) -> Res<Value> {
        let result = self.iter_result(value, true)?;
        self.set_generator_state(g, COMPLETED);
        self.pop_resume_frame_under_top();
        Ok(result)
    }

    /// The resume's stack frame, under the generator's own (about to be
    /// popped by the return).
    fn pop_resume_frame_under_top(&mut self) {
        let below = u32::try_from(self.vm.frames.len().saturating_sub(1)).unwrap_or(u32::MAX);
        if self
            .vm
            .natives
            .last()
            .is_some_and(|n| n.below == below && self.generator_state(n.this).is_some())
        {
            self.vm.natives.pop();
        }
    }

    /// `%GeneratorPrototype%.next`, `return` and `throw` (sections
    /// 27.5.3.2 to 27.5.3.4) on `g`: a value at once (`Some`), or the
    /// generator's frame pushed for the caller's loop to run (`None`).
    #[inline(never)]
    pub(crate) fn resume_generator(
        &mut self,
        g: Value,
        how: Resume,
        sent: Value,
        entry: bool,
        callee: Value,
    ) -> Res<Option<Value>> {
        let Some(state) = self.generator_state(g) else {
            let name = match how {
                Resume::Next => "next",
                Resume::Throw => "throw",
                Resume::Return => "return",
            };
            let text = self.display(g);
            return Err(self.throw_error(
                ErrorType::TypeError,
                messages::generator_incompatible(name, &text),
            ));
        };
        if state == EXECUTING {
            return Err(self.throw_error(ErrorType::TypeError, messages::GENERATOR_RUNNING));
        }
        // Done, or never started and told to stop: no frame to run.
        if state == COMPLETED || (state == SUSPENDED_START && how != Resume::Next) {
            self.set_generator_state(g, COMPLETED);
            return match how {
                Resume::Next => self.iter_result(Value::UNDEFINED, true).map(Some),
                Resume::Return => self.iter_result(sent, true).map(Some),
                Resume::Throw => {
                    self.vm.exception = sent;
                    Err(Abrupt::Throw)
                }
            };
        }
        if self.vm.frames.len() >= MAX_FRAMES {
            return Err(self.throw_error(ErrorType::RangeError, messages::STACK_OVERFLOW));
        }
        // The stack shows the resuming method under the generator's frame.
        self.vm.natives.push(NativeFrame {
            below: u32::try_from(self.vm.frames.len()).unwrap_or(u32::MAX),
            callee,
            this: g,
            construct: false,
        });
        self.push_suspended(g, how, sent, entry)?;
        Ok(None)
    }

    /// Pushes the frame suspended in `g` (a generator or an async
    /// function's driver) back, resumed `how` with `sent`; `entry` if it
    /// is its run's.
    fn push_suspended(&mut self, g: Value, how: Resume, sent: Value, entry: bool) -> Res<()> {
        let slots = self
            .heap
            .objects
            .get(g.as_object().unwrap_or(0))
            .map(|c| c.internal().to_vec())
            .unwrap_or_default();
        let field = |i: usize| slots.get(i).copied().unwrap_or(Value::UNDEFINED);
        let unit = u32::try_from(int(slots.get(UNIT))).unwrap_or(0);
        let func = u32::try_from(int(slots.get(FUNC))).unwrap_or(0);
        let resume = u32::try_from(int(slots.get(RESUME))).unwrap_or(0);
        let base_offset = u32::try_from(int(slots.get(BASE))).unwrap_or(0);
        let mode = int(slots.get(MODE));
        let args = self.vm.stack.len();
        self.vm
            .stack
            .extend_from_slice(slots.get(SAVED..).unwrap_or(&[]));
        let args32 = u32::try_from(args).unwrap_or(u32::MAX);
        let realm = self
            .vm
            .units
            .get(unit as usize)
            .map_or(self.vm.current, |u| u.realm);
        let caller_realm = self.enter_realm(realm);
        self.vm.frames.push(Frame {
            unit,
            func,
            pc: resume,
            resume,
            args: args32,
            base: args32 + base_offset,
            closure: field(CLOSURE),
            this: field(THIS),
            new_target: field(NEW_TARGET),
            context: field(CONTEXT),
            fn_context: field(FN_CONTEXT),
            construct: false,
            entry,
            accessor: false,
            caller_realm,
            generator: g,
        });
        // Only its state: its saved frame is in the frame now.
        if let Some(o) = g.as_object() {
            self.heap.with_internal_slots(o, |s| {
                *s = slots;
                s.truncate(SAVED);
                if let Some(state) = s.get_mut(STATE) {
                    *state = Value::int(EXECUTING);
                }
            });
        }
        if mode >= 0 {
            let base = args32 + base_offset;
            if let Some(slot) = self.vm.stack.get_mut(base as usize + mode as usize) {
                *slot = Value::int(how as i32);
            }
            self.vm.acc = sent;
        } else {
            self.vm.acc = Value::UNDEFINED;
        }
        self.tick()
    }

    // Async functions (section 27.7).

    /// Whether `v` is an async function's driver, or an async
    /// generator: what an await's reaction resumes.
    pub(crate) fn is_async_driver(&self, v: Value) -> bool {
        v.as_object()
            .and_then(|o| self.heap.objects.get(o))
            .is_some_and(|c| {
                matches!(
                    c.kind(),
                    ObjectKind::AsyncDriver | ObjectKind::AsyncGenerator
                )
            })
    }

    // Async generators (section 27.6).

    /// The queue of an async generator: its holder object.
    fn async_gen_queue(&self, g: Value) -> Value {
        g.as_object()
            .and_then(|o| self.heap.objects.get(o))
            .and_then(|c| c.internal().get(OWNER).copied())
            .unwrap_or(Value::UNDEFINED)
    }

    /// The first request of an async generator's queue: how, the value
    /// sent and the promise.
    fn async_gen_front(&self, g: Value) -> Option<(Resume, Value, Value)> {
        let q = self.async_gen_queue(g);
        let cell = self.heap.objects.get(q.as_object()?)?;
        let s = cell.internal();
        let how = match int(s.first()) {
            1 => Resume::Throw,
            2 => Resume::Return,
            _ => Resume::Next,
        };
        Some((how, s.get(1).copied()?, s.get(2).copied()?))
    }

    /// Takes the first request off the queue.
    fn async_gen_dequeue(&mut self, g: Value) {
        if let Some(q) = self.async_gen_queue(g).as_object() {
            self.heap.with_internal_slots(q, |s| {
                if s.len() >= 3 {
                    s.drain(..3);
                }
            });
        }
    }

    fn set_async_gen_state(&mut self, g: Value, state: i32) {
        if let Some(o) = g.as_object() {
            self.heap.with_internal_slots(o, |s| {
                if state == COMPLETED {
                    s.truncate(SAVED);
                }
                if let Some(v) = s.get_mut(STATE) {
                    *v = Value::int(state);
                }
            });
        }
    }

    /// AsyncGeneratorEnqueue (section 27.6.3.4), then the generator
    /// resumed if it is not running: what `next`, `return` and `throw`
    /// do, their promise given back.
    pub(crate) fn async_gen_enqueue(&mut self, g: Value, how: Resume, value: Value) -> Res<Value> {
        let promise = self.new_promise()?;
        if let Some(q) = self.async_gen_queue(g).as_object() {
            self.heap
                .with_internal_slots(q, |s| s.extend([Value::int(how as i32), value, promise]));
        }
        self.vm.temps.push(promise);
        let r = self.async_gen_resume_next(g);
        self.vm.temps.pop();
        r.map(|()| promise)
    }

    /// AsyncGeneratorCompleteStep (section 27.6.3.5): the first request
    /// settled, fulfilled with `{value, done}` or rejected with `value`,
    /// and taken off.
    fn async_gen_complete(
        &mut self,
        g: Value,
        value: Value,
        done: bool,
        rejected: bool,
    ) -> Res<()> {
        let Some((_, _, promise)) = self.async_gen_front(g) else {
            return Ok(());
        };
        self.async_gen_dequeue(g);
        self.vm.temps.push(promise);
        let r = if rejected {
            self.reject_promise(promise, value)
        } else {
            self.iter_result(value, done)
                .and_then(|result| self.resolve_promise(promise, result))
        };
        self.vm.temps.pop();
        r
    }

    /// AsyncGeneratorResumeNext, AsyncGeneratorDrainQueue and
    /// AsyncGeneratorAwaitReturn (sections 27.6.3.6 to 27.6.3.10): the
    /// requests answered while the generator is done, else its frame
    /// resumed with the first, run to its next stop.
    pub(crate) fn async_gen_resume_next(&mut self, g: Value) -> Res<()> {
        loop {
            let Some(state) = self.async_gen_state(g) else {
                return Ok(());
            };
            if state == EXECUTING || state == AWAITING_RETURN {
                return Ok(());
            }
            let Some((how, value, _)) = self.async_gen_front(g) else {
                return Ok(());
            };
            let mut state = state;
            if how != Resume::Next && state == SUSPENDED_START {
                self.set_async_gen_state(g, COMPLETED);
                state = COMPLETED;
            }
            if state == COMPLETED {
                match how {
                    Resume::Next => self.async_gen_complete(g, Value::UNDEFINED, true, false)?,
                    Resume::Throw => self.async_gen_complete(g, value, true, true)?,
                    Resume::Return => {
                        self.set_async_gen_state(g, AWAITING_RETURN);
                        let c = self.realm().intrinsics.promise;
                        match self.promise_resolve(c, value) {
                            Ok(p) => {
                                self.vm.temps.push(p);
                                let r = self.await_return_handlers(g);
                                self.vm.temps.pop();
                                let (on_f, on_r) = r?;
                                self.perform_then(p, on_f, on_r, None);
                                return Ok(());
                            }
                            Err(Abrupt::Throw) => {
                                let e = std::mem::replace(&mut self.vm.exception, Value::UNDEFINED);
                                self.set_async_gen_state(g, COMPLETED);
                                self.async_gen_complete(g, e, true, true)?;
                            }
                            Err(e) => return Err(e),
                        }
                    }
                }
                continue;
            }
            // Suspended: resumed with the request, run to its next stop.
            if self.vm.frames.len() >= MAX_FRAMES {
                return Err(self.throw_error(ErrorType::RangeError, messages::STACK_OVERFLOW));
            }
            self.push_suspended(g, how, value, true)?;
            self.run_frames()?;
            return Ok(());
        }
    }

    /// `AsyncYield`: the first request fulfilled with `{acc, false}`;
    /// then, if another waits, the frame goes on at once with it
    /// (`None`), else it is suspended at `resume` (its caller given
    /// undefined, and the frame).
    #[inline(never)]
    pub(super) fn async_gen_yield(
        &mut self,
        resume: u32,
        mode: usize,
    ) -> Res<Option<(Value, Frame)>> {
        let Some(top) = self.vm.frames.last().copied() else {
            return Err(Abrupt::Throw);
        };
        let g = top.generator;
        let v = self.vm.acc;
        self.async_gen_complete(g, v, false, false)?;
        if let Some((how, value, _)) = self.async_gen_front(g) {
            if let Some(slot) = self.vm.stack.get_mut(top.base as usize + mode) {
                *slot = Value::int(how as i32);
            }
            self.vm.acc = value;
            return Ok(None);
        }
        let slots = self.saved_frame(SUSPENDED_YIELD, resume, mode as i64);
        if let Some(o) = g.as_object() {
            self.heap.with_internal_slots(o, |s| *s = slots);
        }
        let frame = self.leave_generator_frame().ok_or(Abrupt::Throw)?;
        Ok(Some((Value::UNDEFINED, frame)))
    }

    /// An async generator's return: the first request fulfilled with
    /// `{value, done: true}`, the rest answered as a done generator's.
    #[inline(never)]
    pub(super) fn async_gen_return(&mut self, g: Value, value: Value) -> Res<Value> {
        self.set_async_gen_state(g, COMPLETED);
        self.async_gen_complete(g, value, true, false)?;
        self.async_gen_resume_next(g)?;
        Ok(Value::UNDEFINED)
    }

    /// A throw out of an async generator: the first request rejected,
    /// the rest answered as a done generator's.
    pub(super) fn async_gen_throw(&mut self, g: Value, reason: Value) -> Res<()> {
        self.set_async_gen_state(g, COMPLETED);
        self.async_gen_complete(g, reason, true, true)?;
        self.async_gen_resume_next(g)
    }

    /// AsyncGeneratorAwaitReturn's two reactions, holding `g`.
    fn await_return_handlers(&mut self, g: Value) -> Res<(Value, Value)> {
        let (f, r) = self.async_gen_return_functions()?;
        for h in [f, r] {
            if let Some(o) = h.as_object() {
                self.heap.with_internal_slots(o, |s| *s = vec![g]);
            }
        }
        Ok((f, r))
    }

    /// A done async generator's awaited return value settled: its first
    /// request answered, then the rest.
    pub(crate) fn async_gen_return_settled(
        &mut self,
        g: Value,
        value: Value,
        rejected: bool,
    ) -> Res<()> {
        self.set_async_gen_state(g, COMPLETED);
        self.async_gen_complete(g, value, true, rejected)?;
        self.async_gen_resume_next(g)
    }

    /// The async function awaiting the one driven by `d` (a reaction of
    /// its promise that resumes a driver), if one is.
    pub(crate) fn async_awaiter(&self, d: Value) -> Option<Value> {
        if !self.is_async_driver(d) {
            return None;
        }
        // An async generator is awaited through its first request's
        // promise; an async function, through its own.
        let promise = if self.is_async_generator(d) {
            self.async_gen_front(d)?.2
        } else {
            self.async_promise(d)
        };
        self.promise_awaiter(promise)
    }

    /// The stack frame of the async function suspended in `d`: where its
    /// `await` stands.
    pub(crate) fn suspended_frame(&self, d: Value) -> Option<crate::internal::StackFrame> {
        let cell = self.heap.objects.get(d.as_object()?)?;
        let s = cell.internal();
        if s.len() <= SAVED {
            return None;
        }
        let field = |i: usize| s.get(i).copied().unwrap_or(Value::UNDEFINED);
        Some(crate::internal::StackFrame {
            unit: u32::try_from(int(s.get(UNIT))).unwrap_or(0),
            func: u32::try_from(int(s.get(FUNC))).unwrap_or(0),
            pc: u32::try_from(int(s.get(RESUME))).unwrap_or(0),
            receiver: field(THIS),
            callee: field(CLOSURE),
            native: false,
            construct: false,
            accessor: false,
            is_async: true,
        })
    }

    /// An async function's promise, kept in its driver.
    fn async_promise(&self, d: Value) -> Value {
        d.as_object()
            .and_then(|o| self.heap.objects.get(o))
            .and_then(|c| c.internal().get(OWNER).copied())
            .unwrap_or(Value::UNDEFINED)
    }

    /// `AsyncStart`: the running frame's promise and driver made; the
    /// frame goes on.
    #[inline(never)]
    pub(super) fn async_start(&mut self) -> Res<()> {
        let promise = self.new_promise()?;
        self.vm.temps.push(promise);
        let mut slots = vec![Value::UNDEFINED; SAVED];
        if let Some(s) = slots.get_mut(STATE) {
            *s = Value::int(EXECUTING);
        }
        if let Some(s) = slots.get_mut(OWNER) {
            *s = promise;
        }
        let d = self.alloc_object(ObjectCell::with_internal(
            Value::NULL,
            ObjectKind::AsyncDriver,
            slots,
        ));
        self.vm.temps.pop();
        let d = d?;
        if let Some(top) = self.vm.frames.last_mut() {
            top.generator = d;
        }
        Ok(())
    }

    /// `Await` (section 27.7.5.3): the running async function suspended
    /// at `resume` on `acc`, its mode to come in register `mode`; gives
    /// back its promise (what its caller gets) and the frame.
    #[inline(never)]
    pub(super) fn async_await(&mut self, resume: u32, mode: usize) -> Res<(Value, Frame)> {
        let Some(top) = self.vm.frames.last().copied() else {
            return Err(Abrupt::Throw);
        };
        let d = top.generator;
        let c = self.realm().intrinsics.promise;
        let v = self.vm.acc;
        let promise = self.promise_resolve(c, v)?;
        self.perform_then(promise, d, d, None);
        let owner = self.async_promise(d);
        // Still running, as far as requests go: an async generator's
        // `next` only queues while it awaits.
        let slots = self.saved_frame(EXECUTING, resume, mode as i64);
        if let Some(o) = d.as_object() {
            self.heap.with_internal_slots(o, |s| *s = slots);
        }
        let frame = self.leave_generator_frame().ok_or(Abrupt::Throw)?;
        Ok((owner, frame))
    }

    /// An async function's return: its promise resolved with `value`,
    /// what its caller gets.
    #[inline(never)]
    pub(super) fn async_return(&mut self, d: Value, value: Value) -> Res<Value> {
        let promise = self.async_promise(d);
        self.set_driver_state(d, COMPLETED);
        self.resolve_promise(promise, value)?;
        Ok(promise)
    }

    /// An async function's throw: its promise rejected with `reason`,
    /// what its caller gets instead.
    pub(super) fn async_throw(&mut self, d: Value, reason: Value) -> Res<Value> {
        let promise = self.async_promise(d);
        self.set_driver_state(d, COMPLETED);
        self.reject_promise(promise, reason)?;
        Ok(promise)
    }

    fn set_driver_state(&mut self, d: Value, state: i32) {
        if let Some(o) = d.as_object() {
            self.heap.with_internal_slots(o, |s| {
                if state == COMPLETED {
                    s.truncate(SAVED);
                }
                if let Some(v) = s.get_mut(STATE) {
                    *v = Value::int(state);
                }
            });
        }
    }

    /// An await's reaction job: the async function suspended in `d`
    /// resumed with `value` (thrown in if `rejected`) and run to its next
    /// stop.
    pub(crate) fn resume_async(&mut self, d: Value, value: Value, rejected: bool) -> Res<()> {
        if self.vm.frames.len() >= MAX_FRAMES {
            return Err(self.throw_error(ErrorType::RangeError, messages::STACK_OVERFLOW));
        }
        let how = if rejected {
            Resume::Throw
        } else {
            Resume::Next
        };
        self.push_suspended(d, how, value, true)?;
        self.run_frames().map(|_| ())
    }

    /// `yield*`'s step (section 15.5.5): the inner iterator's `next`,
    /// `throw` or `return`, as `how` says, called with the value received
    /// (`acc`) as a `Call` instruction calls: a script's (a generator's
    /// too) is a frame of the loop, its result to come in the
    /// accumulator. `outcome` is set to 3 (check the result), or to 2 at
    /// once for a return the iterator has no method for.
    #[inline(never)]
    #[allow(clippy::too_many_arguments)]
    pub(super) fn delegate_step(
        &mut self,
        base: u32,
        it: usize,
        nx: usize,
        how: i64,
        outcome: usize,
        start: u32,
        resume: u32,
    ) -> Res<DelegateCall> {
        let received = self.vm.acc;
        let iterator = self.reg(base, it);
        let method = match how {
            1 => {
                let key = self.throw_key();
                let throw = self.get_method(iterator, key)?;
                if throw.is_undefined() {
                    // The delegation closes the iterator, then fails: the
                    // iterator gave it no way to take the throw.
                    self.iterator_close(iterator)?;
                    return Err(self.throw_error(ErrorType::TypeError, messages::DELEGATE_NO_THROW));
                }
                throw
            }
            2 => {
                let key = self.realm().return_key;
                let ret = self.get_method(iterator, key)?;
                if ret.is_undefined() {
                    self.vm.acc = received;
                    self.set_reg(base, outcome, Value::int(2));
                    return Ok(DelegateCall::Value(received));
                }
                ret
            }
            _ => self.reg(base, nx),
        };
        self.set_reg(base, outcome, Value::int(3));
        if self.callable(method).is_none() {
            let text = self.display(method);
            return Err(self.throw_error(ErrorType::TypeError, messages::not_a_function(&text)));
        }
        if let Some(top) = self.vm.frames.last_mut() {
            top.pc = start;
            top.resume = resume;
        }
        let args = self.vm.stack.len();
        self.vm.stack.push(received);
        match self.begin_call_owned(method, iterator, args, None)? {
            Some(v) => Ok(DelegateCall::Value(v)),
            None => Ok(DelegateCall::Frame),
        }
    }

    /// `yield*`'s check of what the inner iterator gave (`acc`), unless
    /// the step decided already (`outcome` not 3): 0, yield it as it is;
    /// 1, the delegation's value (in the accumulator); 2, return it.
    #[inline(never)]
    pub(super) fn delegate_check(&mut self, base: u32, how: i64, outcome: usize) -> Res<()> {
        if self.reg(base, outcome) != Value::int(3) {
            return Ok(());
        }
        let result = self.vm.acc;
        if result.as_object().is_none() {
            let text = self.display(result);
            return Err(self.throw_error(
                ErrorType::TypeError,
                messages::iterator_result_not_object(&text),
            ));
        }
        self.vm.temps.push(result);
        let checked = self.iterator_complete(result).and_then(|done| {
            if !done {
                return Ok(0);
            }
            let key = self.realm().value_key;
            self.vm.acc = self.get_value(result, key)?;
            Ok(if how == 2 { 2 } else { 1 })
        });
        self.vm.temps.pop();
        let o = checked?;
        if o == 0 {
            self.vm.acc = result;
        }
        self.set_reg(base, outcome, Value::int(o));
        Ok(())
    }

    /// IteratorComplete (section 7.4.4).
    fn iterator_complete(&mut self, result: Value) -> Res<bool> {
        let key = self.realm().done_key;
        let done = self.get_value(result, key)?;
        Ok(self.to_boolean(done))
    }

    fn throw_key(&mut self) -> crate::string::PropertyKey {
        let units: Vec<u16> = "throw".encode_utf16().collect();
        self.heap.atoms.key(&units)
    }
}
