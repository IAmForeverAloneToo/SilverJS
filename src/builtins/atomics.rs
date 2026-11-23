//! `Atomics` (ECMA-262 section 25.4) over the integer typed arrays. An
//! agent runs alone (no other agent shares its memory yet), so every
//! operation is a plain read and write. `Atomics.wait` cannot block: as
//! on the reference browser's main thread, it throws once its arguments
//! are checked. `Atomics.waitAsync` keeps its waiters, which `notify`
//! wakes; one with a finite timeout times out only when the host gives
//! the agent timers (J5).

use super::{CONFIGURABLE, Call, Entry, Name, What, Wk, entry, method};
use crate::agent::Agent;
use crate::buffer::Elem;
use crate::interp::Res;
use crate::interp::ops::to_int32;
use crate::value::Value;

/// `Atomics`.
pub(crate) const ATOMICS: &[Entry] = &[
    method("load", 2, load),
    method("store", 3, store),
    method("add", 3, add),
    method("sub", 3, sub),
    method("and", 3, and),
    method("or", 3, or),
    method("xor", 3, xor),
    method("exchange", 3, exchange),
    method("compareExchange", 4, compare_exchange),
    method("isLockFree", 1, is_lock_free),
    method("wait", 4, wait),
    method("waitAsync", 4, wait_async),
    method("notify", 3, notify),
    method("pause", 0, pause),
    entry(
        Name::Sym(Wk::ToStringTag),
        What::Str("Atomics"),
        CONFIGURABLE,
    ),
];

/// One `Atomics.waitAsync` waiting: the shared buffer, the byte index in
/// it, and the promise `notify` resolves.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Waiter {
    pub buffer: Value,
    pub at: usize,
    pub promise: Value,
}

impl Agent {
    /// ValidateIntegerTypedArray (section 25.4.3.1): the first argument
    /// as a typed array in bounds of an element type `Atomics` works on
    /// (`Int32Array` alone when `waitable`), and its length.
    fn integer_view(&mut self, c: Call, waitable: bool, method: &str) -> Res<(u32, usize)> {
        let ta = self.arg(c, 0);
        let what = if waitable {
            "an int32 or BigInt64"
        } else {
            "an integer"
        };
        let Some(o) = ta.as_object().filter(|&o| self.heap.view(o).is_some()) else {
            let text = self.display(ta);
            return Err(self.type_error(format!("{text} is not {what} typed array.")));
        };
        let Some(len) = self.heap.view_length(o) else {
            return Err(self.out_of_bounds(method));
        };
        let elem = self.heap.view(o).map_or(Elem::Float64, |v| v.elem);
        let fits = if waitable {
            elem == Elem::Int32
        } else {
            matches!(
                elem,
                Elem::Int8 | Elem::Uint8 | Elem::Int16 | Elem::Uint16 | Elem::Int32 | Elem::Uint32
            )
        };
        if !fits {
            let text = self.display(ta);
            return Err(self.type_error(format!("{text} is not {what} typed array.")));
        }
        Ok((o, len))
    }

    /// ValidateAtomicAccess (section 25.4.3.2): the byte index in its
    /// buffer of the element the second argument names.
    fn atomic_index(&mut self, c: Call, o: u32, len: usize) -> Res<usize> {
        let Some(i) = self.index_value(self.arg(c, 1))?.filter(|&i| i < len) else {
            return Err(self.range_error("Invalid atomic access index"));
        };
        Ok(self
            .heap
            .view(o)
            .map_or(0, |v| v.offset + i * v.elem.size()))
    }

    /// RevalidateAtomicAccess (section 25.4.3.4), after a conversion
    /// that may have shrunk or detached the buffer. The reference
    /// browser throws its out-of-bounds TypeError when the element is
    /// gone, even where its view is still in bounds.
    fn revalidate(&mut self, o: u32, at: usize, method: &str) -> Res<(u32, Elem)> {
        let v = self.heap.view(o);
        let fits = self.heap.view_length(o).is_some()
            && v.is_some_and(|v| {
                self.heap
                    .buffer(v.buffer)
                    .is_some_and(|b| at + v.elem.size() <= b.bytes.len())
            });
        match v {
            Some(v) if fits => Ok((v.buffer, v.elem)),
            _ => Err(self.out_of_bounds(method)),
        }
    }

    fn atomic_read(&self, buffer: u32, elem: Elem, at: usize) -> (Vec<u8>, f64) {
        let bytes = self
            .heap
            .buffer_bytes(buffer, at, elem.size())
            .unwrap_or_default();
        let n = elem.read(&bytes);
        (bytes, n)
    }

    fn atomic_write(&mut self, buffer: u32, elem: Elem, at: usize, n: f64) {
        let mut bytes = vec![0; elem.size()];
        elem.write(&mut bytes, n);
        self.heap.store_buffer_bytes(buffer, at, &bytes);
    }

    /// ValidateAtomicAccessOnIntegerTypedArray, the value converted, and
    /// the access revalidated: the buffer, element type, byte index and
    /// value.
    fn atomic_operands(&mut self, c: Call, method: &str) -> Res<(u32, Elem, usize, f64)> {
        let (o, len) = self.integer_view(c, false, method)?;
        let at = self.atomic_index(c, o, len)?;
        let v = self.integer(self.arg(c, 2))?;
        let (buffer, elem) = self.revalidate(o, at, method)?;
        Ok((buffer, elem, at, v))
    }

    /// AtomicReadModifyWrite (section 25.4.3.17): the old value.
    fn modify(&mut self, c: Call, method: &str, op: fn(u32, u32) -> u32) -> Res<Value> {
        let (buffer, elem, at, v) = self.atomic_operands(c, method)?;
        let (_, old) = self.atomic_read(buffer, elem, at);
        let new = op(to_int32(old) as u32, to_int32(v) as u32);
        self.atomic_write(buffer, elem, at, f64::from(new));
        Ok(Value::number(old))
    }

    /// The `Int32Array` and byte index `Atomics.wait` and `waitAsync`
    /// wait on, their value and timeout (DoWait's first steps, section
    /// 25.4.3.14).
    fn wait_operands(&mut self, c: Call, method: &str) -> Res<(u32, usize, i32, f64)> {
        let (o, len) = self.integer_view(c, true, method)?;
        let buffer = self.heap.view(o).map_or(0, |v| v.buffer);
        if !self.heap.is_shared(buffer) {
            let text = self.display(self.arg(c, 0));
            return Err(self.type_error(format!("{text} is not a shared typed array.")));
        }
        let at = self.atomic_index(c, o, len)?;
        let v = to_int32(self.number(self.arg(c, 2))?);
        let q = self.number(self.arg(c, 3))?;
        let t = if q.is_nan() {
            f64::INFINITY
        } else {
            q.max(0.0)
        };
        Ok((buffer, at, v, t))
    }

    /// `{ async, value }`, a waitAsync result.
    fn wait_result(&mut self, is_async: bool, value: Value) -> Res<Value> {
        self.vm.temps.push(value);
        let r = (|| {
            let o = self.plain_object()?;
            let key = self.name_key(Name::Str("async"));
            self.create_data_property(o, key, Value::bool(is_async));
            let key = self.name_key(Name::Str("value"));
            self.create_data_property(o, key, value);
            Ok(o)
        })();
        self.vm.temps.pop();
        r
    }
}

fn load(agent: &mut Agent, c: Call) -> Res<Value> {
    const M: &str = "Atomics.load";
    let (o, len) = agent.integer_view(c, false, M)?;
    let at = agent.atomic_index(c, o, len)?;
    let (buffer, elem) = agent.revalidate(o, at, M)?;
    Ok(Value::number(agent.atomic_read(buffer, elem, at).1))
}

fn store(agent: &mut Agent, c: Call) -> Res<Value> {
    let (buffer, elem, at, v) = agent.atomic_operands(c, "Atomics.store")?;
    agent.atomic_write(buffer, elem, at, v);
    Ok(Value::number(v))
}

fn add(agent: &mut Agent, c: Call) -> Res<Value> {
    agent.modify(c, "Atomics.add", u32::wrapping_add)
}

fn sub(agent: &mut Agent, c: Call) -> Res<Value> {
    agent.modify(c, "Atomics.sub", u32::wrapping_sub)
}

fn and(agent: &mut Agent, c: Call) -> Res<Value> {
    agent.modify(c, "Atomics.and", |a, b| a & b)
}

fn or(agent: &mut Agent, c: Call) -> Res<Value> {
    agent.modify(c, "Atomics.or", |a, b| a | b)
}

fn xor(agent: &mut Agent, c: Call) -> Res<Value> {
    agent.modify(c, "Atomics.xor", |a, b| a ^ b)
}

fn exchange(agent: &mut Agent, c: Call) -> Res<Value> {
    agent.modify(c, "Atomics.exchange", |_, b| b)
}

/// `Atomics.compareExchange` (section 25.4.6): the replacement written
/// where the element's bytes are the expected value's.
fn compare_exchange(agent: &mut Agent, c: Call) -> Res<Value> {
    const M: &str = "Atomics.compareExchange";
    let (o, len) = agent.integer_view(c, false, M)?;
    let at = agent.atomic_index(c, o, len)?;
    let expected = agent.integer(agent.arg(c, 2))?;
    let replacement = agent.integer(agent.arg(c, 3))?;
    let (buffer, elem) = agent.revalidate(o, at, M)?;
    let (bytes, old) = agent.atomic_read(buffer, elem, at);
    let mut want = vec![0; elem.size()];
    elem.write(&mut want, expected);
    if bytes == want {
        agent.atomic_write(buffer, elem, at, replacement);
    }
    Ok(Value::number(old))
}

/// `Atomics.isLockFree` (section 25.4.10): every size the reference
/// browser's atomics take.
fn is_lock_free(agent: &mut Agent, c: Call) -> Res<Value> {
    let n = agent.integer(agent.arg(c, 0))?;
    Ok(Value::bool([1.0, 2.0, 4.0, 8.0].contains(&n)))
}

/// `Atomics.wait` (section 25.4.13): the agent cannot block.
fn wait(agent: &mut Agent, c: Call) -> Res<Value> {
    agent.wait_operands(c, "Atomics.wait")?;
    Err(agent.type_error("Atomics.wait cannot be called in this context"))
}

/// `Atomics.waitAsync` (section 25.4.14).
fn wait_async(agent: &mut Agent, c: Call) -> Res<Value> {
    let (buffer, at, v, t) = agent.wait_operands(c, "Atomics.waitAsync")?;
    let (_, w) = agent.atomic_read(buffer, Elem::Int32, at);
    if f64::from(v) != w {
        let s = agent.new_string("not-equal")?;
        return agent.wait_result(false, s);
    }
    if t == 0.0 {
        let s = agent.new_string("timed-out")?;
        return agent.wait_result(false, s);
    }
    let promise = agent.new_promise()?;
    agent.vm.waiters.push(Waiter {
        buffer: crate::value::Value::reference(crate::value::RefKind::Object, buffer),
        at,
        promise,
    });
    agent.wait_result(true, promise)
}

/// `Atomics.notify` (section 25.4.15): the waiters on the element woken,
/// first come first, as many as `count`; how many.
fn notify(agent: &mut Agent, c: Call) -> Res<Value> {
    const M: &str = "Atomics.notify";
    let (o, len) = agent.integer_view(c, true, M)?;
    let at = agent.atomic_index(c, o, len)?;
    let count = agent.arg(c, 2);
    let count = if count.is_undefined() {
        f64::INFINITY
    } else {
        agent.integer(count)?.max(0.0)
    };
    let buffer = agent.heap.view(o).map_or(0, |v| v.buffer);
    if !agent.heap.is_shared(buffer) {
        return Ok(Value::number(0.0));
    }
    let buffer = crate::value::Value::reference(crate::value::RefKind::Object, buffer);
    let mut woken = Vec::new();
    let mut kept = Vec::new();
    for w in std::mem::take(&mut agent.vm.waiters) {
        if (woken.len() as f64) < count && w.buffer == buffer && w.at == at {
            woken.push(w.promise);
        } else {
            kept.push(w);
        }
    }
    agent.vm.waiters = kept;
    let n = woken.len();
    let base = agent.vm.temps.len();
    agent.vm.temps.extend_from_slice(&woken);
    let r = (|| {
        let ok = agent.new_string("ok")?;
        agent.vm.temps.push(ok);
        for p in woken {
            agent.resolve_promise(p, ok)?;
        }
        Ok(())
    })();
    agent.vm.temps.truncate(base);
    r?;
    Ok(Value::number(n as f64))
}

/// `Atomics.pause` (section 25.4.11): a hint, and nothing to do.
fn pause(agent: &mut Agent, c: Call) -> Res<Value> {
    let n = agent.arg(c, 0);
    if !n.is_undefined()
        && !n
            .as_number()
            .is_some_and(|n| n.is_finite() && n.trunc() == n)
    {
        return Err(agent.type_error("Atomics.pause argument must be undefined or an integer"));
    }
    Ok(Value::UNDEFINED)
}
