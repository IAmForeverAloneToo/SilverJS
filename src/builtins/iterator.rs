//! The iterators of J3.1 (ECMA-262 sections 27.1.2, 23.1.5 and 22.1.5):
//! `%IteratorPrototype%`, the array iterators `values`, `keys` and
//! `entries` give, and the string iterator. Iterator helpers
//! (`Iterator.prototype.map` and the rest) are J4's.

use super::{CONFIGURABLE, Call, Entry, HIDDEN, Name, What, Wk, entry, method};
use crate::agent::Agent;
use crate::heap::ObjectCell;
use crate::interp::Res;
use crate::object::ObjectKind;
use crate::string::PropertyKey;
use crate::value::Value;

/// `%IteratorPrototype%`.
pub(crate) const ITERATOR_PROTO: &[Entry] = &[entry(
    Name::Sym(Wk::Iterator),
    What::Method(0, iterator_self),
    HIDDEN,
)];

/// `%ArrayIteratorPrototype%`.
pub(crate) const ARRAY_ITERATOR_PROTO: &[Entry] = &[
    method("next", 0, array_iterator_next),
    entry(
        Name::Sym(Wk::ToStringTag),
        What::Str("Array Iterator"),
        CONFIGURABLE,
    ),
];

/// `%StringIteratorPrototype%`.
pub(crate) const STRING_ITERATOR_PROTO: &[Entry] = &[
    method("next", 0, string_iterator_next),
    entry(
        Name::Sym(Wk::ToStringTag),
        What::Str("String Iterator"),
        CONFIGURABLE,
    ),
];

/// What an array iterator gives (its kind, an internal slot).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ArrayIterKind {
    Keys = 0,
    Values = 1,
    Entries = 2,
}

fn iterator_self(_: &mut Agent, c: Call) -> Res<Value> {
    Ok(c.this)
}

impl Agent {
    /// CreateIterResultObject (section 7.4.14).
    pub(crate) fn iter_result(&mut self, value: Value, done: bool) -> Res<Value> {
        let proto = self.realm().object_prototype;
        // The value rooted while the result is made.
        self.vm.temps.push(value);
        let made = self.alloc_object(ObjectCell::new(proto, ObjectKind::Ordinary));
        self.vm.temps.pop();
        let o = made?;
        let (vk, dk) = (self.realm().value_key, self.realm().done_key);
        self.define_value(o, vk, value)?;
        self.define_value(o, dk, Value::bool(done))?;
        Ok(o)
    }

    /// CreateArrayIterator (section 23.1.5.1).
    pub(crate) fn array_iterator(&mut self, array: Value, kind: ArrayIterKind) -> Res<Value> {
        let proto = self.realm().intrinsics.array_iterator_prototype;
        self.alloc_object(ObjectCell::with_internal(
            proto,
            ObjectKind::ArrayIterator,
            vec![array, Value::int(0), Value::int(kind as i32)],
        ))
    }

    fn set_internal(&mut self, o: Value, i: usize, v: Value) {
        if let Some(cell) = o.as_object().and_then(|o| self.heap.objects.get_mut(o)) {
            cell.set_internal(i, v);
        }
    }

    fn internal_of(&self, o: Value, kind: ObjectKind) -> Option<Vec<Value>> {
        let cell = self.heap.objects.get(o.as_object()?)?;
        (cell.kind() == kind).then(|| cell.internal().to_vec())
    }
}

/// `Array.prototype.values`, `keys` and `entries` (sections 23.1.3.38,
/// 23.1.3.19, 23.1.3.7).
fn make_array_iterator(agent: &mut Agent, c: Call, kind: ArrayIterKind) -> Res<Value> {
    let o = agent.object_of(c.this)?;
    agent.array_iterator(o, kind)
}

pub(crate) fn values(agent: &mut Agent, c: Call) -> Res<Value> {
    make_array_iterator(agent, c, ArrayIterKind::Values)
}

pub(crate) fn keys(agent: &mut Agent, c: Call) -> Res<Value> {
    make_array_iterator(agent, c, ArrayIterKind::Keys)
}

pub(crate) fn entries(agent: &mut Agent, c: Call) -> Res<Value> {
    make_array_iterator(agent, c, ArrayIterKind::Entries)
}

/// `%ArrayIteratorPrototype%.next` (section 23.1.5.2.1).
fn array_iterator_next(agent: &mut Agent, c: Call) -> Res<Value> {
    let Some(slots) = agent.internal_of(c.this, ObjectKind::ArrayIterator) else {
        let text = agent.display(c.this);
        return Err(agent.type_error(format!(
            "Method Array Iterator.prototype.next called on incompatible receiver {text}"
        )));
    };
    let (array, index, kind) = match slots.as_slice() {
        [a, i, k] => (
            *a,
            i.as_number().unwrap_or(0.0),
            k.as_number().unwrap_or(1.0),
        ),
        _ => return agent.iter_result(Value::UNDEFINED, true),
    };
    if array.is_undefined() {
        return agent.iter_result(Value::UNDEFINED, true);
    }
    // A typed array's length is its own (section 23.1.5.2.1), a TypeError
    // once it is out of bounds.
    let view = array.as_object().filter(|&o| agent.heap.view(o).is_some());
    let len = match view {
        Some(o) => match agent.heap.view_length(o) {
            Some(n) => n as f64,
            None => {
                return Err(agent.type_error(
                    "Cannot perform Array Iterator.prototype.next on a detached or out-of-bounds ArrayBuffer",
                ));
            }
        },
        None => agent.length_of(array)?,
    };
    if index >= len {
        agent.set_internal(c.this, 0, Value::UNDEFINED);
        return agent.iter_result(Value::UNDEFINED, true);
    }
    agent.set_internal(c.this, 1, Value::number(index + 1.0));
    let i = Value::number(index);
    if kind == ArrayIterKind::Keys as i32 as f64 {
        return agent.iter_result(i, false);
    }
    let k = agent.index_key(index);
    let v = agent.get_value(array, k)?;
    if kind == ArrayIterKind::Values as i32 as f64 {
        return agent.iter_result(v, false);
    }
    agent.vm.temps.push(v);
    let pair = agent.array_create(0.0, None);
    agent.vm.temps.pop();
    let pair = pair?;
    agent.vm.temps.push(pair);
    let filled = agent
        .define_value(pair, PropertyKey::Index(0), i)
        .and_then(|()| agent.define_value(pair, PropertyKey::Index(1), v));
    let result = filled.and_then(|()| agent.iter_result(pair, false));
    agent.vm.temps.pop();
    result
}

/// `String.prototype[@@iterator]` (section 22.1.3.36).
pub(crate) fn string_iterator(agent: &mut Agent, c: Call) -> Res<Value> {
    if c.this.is_nullish() {
        return Err(
            agent.type_error("String.prototype[Symbol.iterator] called on null or undefined")
        );
    }
    let s = agent.string_value(c.this)?;
    agent.vm.temps.push(s);
    let proto = agent.realm().intrinsics.string_iterator_prototype;
    let it = agent.alloc_object(ObjectCell::with_internal(
        proto,
        ObjectKind::StringIterator,
        vec![s, Value::int(0)],
    ));
    agent.vm.temps.pop();
    it
}

/// `%StringIteratorPrototype%.next` (section 22.1.5.1.1): a code point
/// at a time.
fn string_iterator_next(agent: &mut Agent, c: Call) -> Res<Value> {
    let Some(slots) = agent.internal_of(c.this, ObjectKind::StringIterator) else {
        let text = agent.display(c.this);
        return Err(agent.type_error(format!(
            "Method String Iterator.prototype.next called on incompatible receiver {text}"
        )));
    };
    let (s, pos) = match slots.as_slice() {
        [s, p] => (*s, p.as_number().unwrap_or(0.0) as usize),
        _ => return agent.iter_result(Value::UNDEFINED, true),
    };
    let units = match agent.string_units(s) {
        Some(u) if pos < u.len() => u,
        _ => {
            agent.set_internal(c.this, 0, Value::UNDEFINED);
            return agent.iter_result(Value::UNDEFINED, true);
        }
    };
    let first = units.get(pos).copied().unwrap_or(0);
    let pair = (0xd800..0xdc00).contains(&first)
        && units
            .get(pos + 1)
            .is_some_and(|u| (0xdc00..0xe000).contains(u));
    let end = if pair { pos + 2 } else { pos + 1 };
    agent.set_internal(c.this, 1, Value::number(end as f64));
    let piece = agent.new_string_units(units.get(pos..end).unwrap_or(&[]))?;
    agent.iter_result(piece, false)
}
