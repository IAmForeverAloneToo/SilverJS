//! The iterator protocol (ECMA-262 section 7.4): GetIterator,
//! IteratorStepValue and IteratorClose, as the iteration instructions
//! (`for-of`, spread, array destructuring) and the built-ins run them.

use super::messages;
use super::{Abrupt, ErrorType, Res};
use crate::agent::Agent;
use crate::builtins::Wk;
use crate::compiler::Op;
use crate::string::PropertyKey;
use crate::value::Value;

/// How a spread call went on.
pub(crate) enum SpreadCall {
    /// A native's value, or an eval's argument that was no string.
    Value(Value),
    /// A frame pushed (a script callee, a direct eval): the loop runs it.
    Frame,
}

impl Agent {
    /// GetIterator (section 7.4.3), sync: `iterable`'s iterator and its
    /// `next` method. `text` names the iterable in the error if it has no
    /// iterator.
    pub(crate) fn get_iterator(&mut self, iterable: Value, text: &str) -> Res<(Value, Value)> {
        self.get_iterator_or(iterable, || messages::not_iterable(text))
    }

    /// GetIterator, the error naming the value by type and text (`number
    /// 1 is not iterable (cannot read property Symbol(Symbol.iterator))`).
    pub(crate) fn get_iterator_typed(&mut self, iterable: Value) -> Res<(Value, Value)> {
        let text = self.typed_display(iterable);
        self.get_iterator_or(iterable, || messages::not_iterable_typed(&text))
    }

    /// GetIterator, `message` the error if `iterable` has no iterator.
    fn get_iterator_or(
        &mut self,
        iterable: Value,
        message: impl FnOnce() -> String,
    ) -> Res<(Value, Value)> {
        let key = self.well_known_key(Wk::Iterator);
        let method = if iterable.is_nullish() {
            Value::UNDEFINED
        } else {
            self.get_method(iterable, key)?
        };
        if method.is_undefined() {
            return Err(self.throw_error(ErrorType::TypeError, message()));
        }
        self.iterator_from_method(iterable, method)
    }

    /// GetIteratorFromMethod (section 7.4.2).
    pub(crate) fn iterator_from_method(
        &mut self,
        iterable: Value,
        method: Value,
    ) -> Res<(Value, Value)> {
        let iterator = self.call_value(method, iterable, &[])?;
        if iterator.as_object().is_none() {
            return Err(self.throw_error(ErrorType::TypeError, messages::ITERATOR_NOT_OBJECT));
        }
        self.vm.temps.push(iterator);
        let key = self.realm().next_key;
        let next = self.get_value(iterator, key);
        self.vm.temps.pop();
        Ok((iterator, next?))
    }

    /// IteratorStepValue (section 7.4.8): the next value, or `None` once
    /// the iterator is done.
    pub(crate) fn iterator_step(&mut self, iterator: Value, next: Value) -> Res<Option<Value>> {
        let result = self.call_value(next, iterator, &[])?;
        if result.as_object().is_none() {
            let text = self.display(result);
            return Err(self.throw_error(
                ErrorType::TypeError,
                messages::iterator_result_not_object(&text),
            ));
        }
        self.vm.temps.push(result);
        let stepped = (|| {
            let key = self.realm().done_key;
            let done = self.get_value(result, key)?;
            if self.to_boolean(done) {
                return Ok(None);
            }
            let key = self.realm().value_key;
            self.get_value(result, key).map(Some)
        })();
        self.vm.temps.pop();
        stepped
    }

    /// IteratorClose (section 7.4.11) on a normal completion: `return`
    /// called, its result an object.
    pub(crate) fn iterator_close(&mut self, iterator: Value) -> Res<()> {
        let key = self.realm().return_key;
        let method = self.get_method(iterator, key)?;
        if method.is_undefined() {
            return Ok(());
        }
        let result = self.call_value(method, iterator, &[])?;
        if result.as_object().is_none() {
            let text = self.display(result);
            return Err(self.throw_error(
                ErrorType::TypeError,
                messages::iterator_result_not_object(&text),
            ));
        }
        Ok(())
    }

    /// IteratorClose on a throw completion: `return` called, whatever it
    /// throws or gives ignored (the first exception is the one that goes
    /// on); only a termination goes on instead.
    pub(crate) fn iterator_close_quiet(&mut self, iterator: Value) -> Res<()> {
        let saved = self.vm.exception;
        let key = self.realm().return_key;
        let closed = self.get_method(iterator, key).and_then(|method| {
            if method.is_undefined() {
                Ok(())
            } else {
                self.call_value(method, iterator, &[]).map(|_| ())
            }
        });
        self.vm.exception = saved;
        match closed {
            Err(super::Abrupt::Terminate) => Err(super::Abrupt::Terminate),
            _ => Ok(()),
        }
    }

    /// Appends `v` to the array `array`, at its length.
    #[inline(never)]
    pub(crate) fn array_push(&mut self, array: Value, v: Value) -> Res<()> {
        let len = self.array_length_of(array);
        self.define_value(array, crate::string::PropertyKey::Index(len), v)
    }

    /// Lengthens the array `array` by one, a hole.
    #[inline(never)]
    pub(crate) fn array_push_hole(&mut self, array: Value) -> Res<()> {
        let len = self.array_length_of(array);
        let key = self.heap.length_key;
        self.put(array, key, Value::number(f64::from(len) + 1.0))
    }

    /// The length of an array the instructions build (never past 2^32 - 2
    /// before running out of memory).
    fn array_length_of(&self, array: Value) -> u32 {
        array
            .as_object()
            .and_then(|o| self.heap.objects.get(o))
            .and_then(|c| c.array_length())
            .map_or(0, |(len, _)| len)
    }

    /// Appends every value `iterable` gives to the array `array`; `call`
    /// for a call's arguments, whose missing iterator is told otherwise.
    pub(crate) fn spread_into(
        &mut self,
        array: Value,
        iterable: Value,
        text: &str,
        call: bool,
    ) -> Res<()> {
        let (iterator, next) = if call {
            let key = self.well_known_key(Wk::Iterator);
            let method = if iterable.is_nullish() {
                Value::UNDEFINED
            } else {
                self.get_method(iterable, key)?
            };
            if self.callable(method).is_none() {
                return Err(self.throw_error(ErrorType::TypeError, messages::SPREAD_NOT_ITERABLE));
            }
            self.iterator_from_method(iterable, method)?
        } else {
            self.get_iterator(iterable, text)?
        };
        self.vm.temps.extend([iterator, next]);
        let result = (|| {
            // (Defining an element allocates nothing the collector
            // could take `v` in.)
            while let Some(v) = self.iterator_step(iterator, next)? {
                self.array_push(array, v)?;
            }
            Ok(())
        })();
        self.vm.temps.truncate(self.vm.temps.len() - 2);
        result
    }

    // The iteration instructions' bodies, apart from the interpreter's
    // loop so that its own code stays as it was.

    #[inline(never)]
    #[allow(clippy::too_many_arguments)]
    pub(super) fn op_get_iterator(
        &mut self,
        base: u32,
        it: usize,
        nx: usize,
        how: usize,
        unit: u32,
        func: u32,
        pc: u32,
    ) -> Res<()> {
        let v = self.vm.acc;
        // How the error names the value: its source (0), itself (1), or
        // itself with its type, as `yield*`'s does (2).
        let (i, n) = match how {
            0 => {
                let text = self.text_at(unit, func, pc);
                self.get_iterator(v, &text)?
            }
            1 => {
                let text = self.display(v);
                self.get_iterator(v, &text)?
            }
            // `for await` (its source in the error) and an async
            // generator's `yield*` (V8's call of the missing method).
            3 => {
                let text = self.text_at(unit, func, pc);
                self.get_async_iterator(v, messages::not_async_iterable(&text))?
            }
            4 => self.get_async_iterator(v, messages::not_a_function("undefined"))?,
            _ => {
                let text = self.typed_display(v);
                self.get_iterator_or(v, || messages::not_iterable_typed(&text))?
            }
        };
        self.set_reg(base, it, i);
        self.set_reg(base, nx, n);
        Ok(())
    }

    /// GetIterator(obj, async) (section 7.4.3): `@@asyncIterator`'s
    /// iterator, else the sync one wrapped (CreateAsyncFromSyncIterator);
    /// `message` the error if there is neither.
    pub(crate) fn get_async_iterator(&mut self, v: Value, message: String) -> Res<(Value, Value)> {
        let key = self.well_known_key(crate::builtins::Wk::AsyncIterator);
        let method = self.get_method_value(v, key)?;
        if method.is_undefined() {
            let key = self.well_known_key(crate::builtins::Wk::Iterator);
            let sync = self.get_method_value(v, key)?;
            if sync.is_undefined() {
                return Err(self.throw_error(crate::interp::ErrorType::TypeError, message));
            }
            let (i, n) = self.get_iterator_or(v, || message)?;
            return self.async_from_sync_iterator(i, n);
        }
        let iterator = self.call_value(method, v, &[])?;
        if iterator.as_object().is_none() {
            return Err(self.throw_error(
                crate::interp::ErrorType::TypeError,
                messages::ASYNC_ITERATOR_NOT_OBJECT,
            ));
        }
        let key = self.name_key(crate::builtins::Name::Str("next"));
        self.vm.temps.push(iterator);
        let next = self.get_value(iterator, key);
        self.vm.temps.pop();
        Ok((iterator, next?))
    }

    /// GetMethod: `v[key]`, undefined for null and undefined.
    fn get_method_value(&mut self, v: Value, key: crate::string::PropertyKey) -> Res<Value> {
        if v.is_nullish() {
            return Ok(Value::UNDEFINED);
        }
        let m = self.get_value(v, key)?;
        Ok(if m.is_nullish() { Value::UNDEFINED } else { m })
    }

    /// `IteratorResult`: the awaited result in the accumulator read;
    /// whether it is done.
    #[inline(never)]
    pub(super) fn op_iterator_result(&mut self, base: u32, d: usize) -> Res<bool> {
        let result = self.vm.acc;
        if result.as_object().is_none() {
            let text = self.display(result);
            return Err(self.throw_error(
                crate::interp::ErrorType::TypeError,
                messages::iterator_result_not_object(&text),
            ));
        }
        let key = self.name_key(crate::builtins::Name::Str("done"));
        let done = self.get_value(result, key)?;
        if self.to_boolean(done) {
            return Ok(true);
        }
        let key = self.name_key(crate::builtins::Name::Str("value"));
        self.vm.acc = self.get_value(result, key)?;
        self.set_reg(base, d, Value::FALSE);
        Ok(false)
    }

    /// `IteratorReturnCall`: an async iterator's `return()` called, its
    /// result in the accumulator to await; false if there is none.
    #[inline(never)]
    pub(super) fn op_iterator_return_call(
        &mut self,
        base: u32,
        it: usize,
        d: usize,
        quiet: bool,
    ) -> Res<bool> {
        if self.reg(base, d) == Value::TRUE {
            return Ok(false);
        }
        self.set_reg(base, d, Value::TRUE);
        let i = self.reg(base, it);
        let called = (|| {
            let key = self.name_key(crate::builtins::Name::Str("return"));
            let ret = self.get_method_value(i, key)?;
            if ret.is_undefined() {
                return Ok(None);
            }
            self.call_value(ret, i, &[]).map(Some)
        })();
        match called {
            Ok(Some(v)) => {
                self.vm.acc = v;
                Ok(true)
            }
            Ok(None) => Ok(false),
            Err(Abrupt::Throw) if quiet => {
                self.vm.exception = Value::UNDEFINED;
                Ok(false)
            }
            Err(e) => Err(e),
        }
    }

    /// `CheckIterResult`.
    pub(super) fn check_iter_result(&mut self) -> Res<()> {
        let result = self.vm.acc;
        if result.as_object().is_none() {
            let text = self.display(result);
            return Err(self.throw_error(
                crate::interp::ErrorType::TypeError,
                messages::iterator_result_not_object(&text),
            ));
        }
        Ok(())
    }

    /// `AsyncDelegateMethod`.
    #[inline(never)]
    pub(super) fn op_async_delegate_method(
        &mut self,
        base: u32,
        it: usize,
        nx: usize,
        m: usize,
        o: usize,
    ) -> Res<()> {
        let how = self.reg(base, m).as_int().unwrap_or(0);
        let i = self.reg(base, it);
        let (method, outcome) = match how {
            0 => (self.reg(base, nx), 0),
            1 => {
                let key = self.name_key(crate::builtins::Name::Str("throw"));
                let t = self.get_method_value(i, key)?;
                (t, if t.is_undefined() { 4 } else { 0 })
            }
            _ => {
                let key = self.name_key(crate::builtins::Name::Str("return"));
                let r = self.get_method_value(i, key)?;
                (r, if r.is_undefined() { 2 } else { 0 })
            }
        };
        self.set_reg(base, o, Value::int(outcome));
        self.vm.acc = method;
        Ok(())
    }

    /// `AsyncDelegateResult`.
    #[inline(never)]
    pub(super) fn op_async_delegate_result(&mut self, base: u32, m: usize, o: usize) -> Res<()> {
        self.check_iter_result()?;
        let result = self.vm.acc;
        let key = self.name_key(crate::builtins::Name::Str("done"));
        let done = self.get_value(result, key)?;
        let done = self.to_boolean(done);
        let key = self.name_key(crate::builtins::Name::Str("value"));
        self.vm.acc = self.get_value(result, key)?;
        let how = self.reg(base, m).as_int().unwrap_or(0);
        let outcome = match (done, how) {
            (false, _) => 0,
            (true, 2) => 2,
            (true, _) => 1,
        };
        self.set_reg(base, o, Value::int(outcome));
        Ok(())
    }

    /// `IteratorNext`: whether a value came (into the accumulator); if
    /// not, the accumulator is as it was (a pattern's undefined).
    #[inline(never)]
    pub(super) fn op_iterator_next(
        &mut self,
        base: u32,
        it: usize,
        nx: usize,
        d: usize,
    ) -> Res<bool> {
        if self.reg(base, d) == Value::TRUE {
            return Ok(false);
        }
        // Done until a value comes: an iterator whose next throws is not
        // closed (section 7.4.8).
        self.set_reg(base, d, Value::TRUE);
        let (i, n) = (self.reg(base, it), self.reg(base, nx));
        let acc = self.vm.acc;
        self.vm.temps.push(acc);
        let stepped = self.iterator_step(i, n);
        self.vm.temps.pop();
        match stepped? {
            Some(v) => {
                self.vm.acc = v;
                self.set_reg(base, d, Value::FALSE);
                Ok(true)
            }
            None => {
                self.vm.acc = acc;
                Ok(false)
            }
        }
    }

    /// `IteratorClose`, normal or quiet, unless done: the accumulator (a
    /// return's value, a throw's) kept.
    #[inline(never)]
    pub(super) fn op_iterator_close(
        &mut self,
        base: u32,
        it: usize,
        d: usize,
        quiet: bool,
    ) -> Res<()> {
        if self.reg(base, d) == Value::TRUE {
            return Ok(());
        }
        self.set_reg(base, d, Value::TRUE);
        let i = self.reg(base, it);
        let acc = self.vm.acc;
        self.vm.temps.push(acc);
        let closed = if quiet {
            self.iterator_close_quiet(i)
        } else {
            self.iterator_close(i)
        };
        self.vm.temps.pop();
        self.vm.acc = acc;
        closed
    }

    #[inline(never)]
    pub(super) fn op_spread_into(
        &mut self,
        base: u32,
        o: usize,
        call: bool,
        unit: u32,
        func: u32,
        pc: u32,
    ) -> Res<()> {
        let (a, v) = (self.reg(base, o), self.vm.acc);
        let text = self.text_at(unit, func, pc);
        self.spread_into(a, v, &text, call)
    }

    /// An object pattern's value was null or undefined.
    #[cold]
    #[inline(never)]
    pub(super) fn destructure_error(
        &mut self,
        n: usize,
        plain: bool,
        unit: u32,
        func: u32,
        pc: u32,
    ) -> Abrupt {
        let what = if self.vm.acc.is_null() {
            "null"
        } else {
            "undefined"
        };
        let name = self.name_text(unit, func, n);
        let text = if plain {
            what.to_owned()
        } else {
            self.text_at(unit, func, pc)
        };
        let message = messages::cannot_destructure(&name, &text, what);
        self.throw_error(ErrorType::TypeError, message)
    }

    /// `CallSpread` and `NewSpread`: the array's elements onto the top of
    /// the register file, which the call takes as its own. `at` is the
    /// instruction's unit, function, pc and the pc after it.
    #[inline(never)]
    #[allow(clippy::too_many_arguments)]
    pub(super) fn op_call_spread(
        &mut self,
        op: Op,
        base: u32,
        callee: usize,
        recv: Option<usize>,
        array: usize,
        flags: usize,
        at: (u32, u32, u32, u32),
    ) -> Res<SpreadCall> {
        let (unit, func, start, resume) = at;
        let callee = self.reg(base, callee);
        let recv = recv.map_or(Value::UNDEFINED, |r| self.reg(base, r));
        let array = self.reg(base, array);
        let args = self.vm.stack.len();
        let n = array
            .as_object()
            .and_then(|o| self.heap.objects.get(o))
            .and_then(|c| c.array_length())
            .map_or(0, |(len, _)| len);
        for i in 0..n {
            match self.get_value(array, PropertyKey::Index(i)) {
                Ok(v) => self.vm.stack.push(v),
                Err(e) => {
                    self.vm.stack.truncate(args);
                    return Err(e);
                }
            }
        }
        if let Some(top) = self.vm.frames.last_mut() {
            top.pc = start;
            top.resume = resume;
        }
        if flags & 1 != 0 && n > 0 && callee == self.realm().intrinsics.eval {
            let x = self.vm.stack.get(args).copied().unwrap_or(Value::UNDEFINED);
            self.vm.stack.truncate(args);
            if x.as_string().is_none() {
                return Ok(SpreadCall::Value(x));
            }
            self.begin_direct_eval(x, start)?;
            return Ok(SpreadCall::Frame);
        }
        let construct = op == Op::NewSpread;
        let callable_ok = if construct {
            self.is_constructor_value(callee)
        } else {
            self.callable(callee).is_some()
        };
        if !callable_ok {
            self.vm.stack.truncate(args);
            let text = self.text_at(unit, func, start);
            let message = if construct {
                messages::not_a_constructor(&text)
            } else {
                messages::not_a_function(&text)
            };
            return Err(self.throw_error(ErrorType::TypeError, message));
        }
        match self.begin_call_owned(callee, recv, args, construct.then_some(callee))? {
            Some(v) => Ok(SpreadCall::Value(v)),
            None => Ok(SpreadCall::Frame),
        }
    }
}
