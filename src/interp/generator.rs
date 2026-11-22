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
const SAVED: usize = 11;

fn int(v: Option<&Value>) -> i64 {
    v.and_then(|v| v.as_number()).map_or(-1, |n| n as i64)
}

impl Agent {
    /// The generator's state, if `g` is one.
    pub(crate) fn generator_state(&self, g: Value) -> Option<i32> {
        let cell = self.heap.objects.get(g.as_object()?)?;
        (cell.kind() == ObjectKind::Generator).then(|| int(cell.internal().get(STATE)) as i32)
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
        (cell.kind() == ObjectKind::Generator)
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
        // OrdinaryCreateFromConstructor(F, "%GeneratorPrototype%").
        let proto = self.prototype_from(top.closure, |r| r.intrinsics.generator_prototype)?;
        let slots = self.saved_frame(SUSPENDED_START, resume, -1);
        let g = self.alloc_object(ObjectCell::with_internal(
            proto,
            ObjectKind::Generator,
            slots,
        ))?;
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
        // The stack shows the resuming method under the generator's frame.
        self.vm.natives.push(NativeFrame {
            below: u32::try_from(self.vm.frames.len()).unwrap_or(u32::MAX),
            callee,
            this: g,
            construct: false,
        });
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
        self.tick()?;
        Ok(None)
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
