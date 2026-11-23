//! `SharedArrayBuffer` and `DataView` (ECMA-262 sections 25.2 and 25.3).
//! A shared buffer's data block is an `ArrayBuffer`'s (an agent has no
//! other agent to share it with yet); it is never detached and grows
//! but never shrinks. A `DataView` reads and writes its buffer's bytes
//! as any element type, in either byte order, at any offset.

use super::typed::{MAX_SAFE, buffer_length_and_max};
use super::{CONFIGURABLE, Call, Entry, HIDDEN, Name, What, Wk, entry, getter, method};
use crate::agent::Agent;
use crate::buffer::Elem;
use crate::heap::ObjectCell;
use crate::interp::Res;
use crate::number::number_to_string;
use crate::object::ObjectKind;
use crate::value::{RefKind, Value};

/// `SharedArrayBuffer`.
pub(crate) const SHARED_ARRAY_BUFFER: &[Entry] = &[entry(
    Name::Sym(Wk::Species),
    What::Getter(species),
    CONFIGURABLE,
)];

/// `SharedArrayBuffer.prototype`.
pub(crate) const SHARED_ARRAY_BUFFER_PROTO: &[Entry] = &[
    entry(Name::Str("constructor"), What::Constructor, HIDDEN),
    getter("byteLength", shared_byte_length),
    method("slice", 2, shared_slice),
    getter("maxByteLength", shared_max_byte_length),
    getter("growable", growable),
    method("grow", 1, grow),
    entry(
        Name::Sym(Wk::ToStringTag),
        What::Str("SharedArrayBuffer"),
        CONFIGURABLE,
    ),
];

/// `DataView.prototype`. The BigInt accessors come with `BigInt`.
pub(crate) const DATA_VIEW_PROTO: &[Entry] = &[
    entry(Name::Str("constructor"), What::Constructor, HIDDEN),
    getter("buffer", view_buffer),
    getter("byteLength", view_byte_length),
    getter("byteOffset", view_byte_offset),
    method("getInt8", 1, get_int8),
    method("setInt8", 2, set_int8),
    method("getUint8", 1, get_uint8),
    method("setUint8", 2, set_uint8),
    method("getInt16", 1, get_int16),
    method("setInt16", 2, set_int16),
    method("getUint16", 1, get_uint16),
    method("setUint16", 2, set_uint16),
    method("getInt32", 1, get_int32),
    method("setInt32", 2, set_int32),
    method("getUint32", 1, get_uint32),
    method("setUint32", 2, set_uint32),
    method("getFloat16", 1, get_float16),
    method("setFloat16", 2, set_float16),
    method("getFloat32", 1, get_float32),
    method("setFloat32", 2, set_float32),
    method("getFloat64", 1, get_float64),
    method("setFloat64", 2, set_float64),
    entry(
        Name::Sym(Wk::ToStringTag),
        What::Str("DataView"),
        CONFIGURABLE,
    ),
];

fn species(_: &mut Agent, c: Call) -> Res<Value> {
    Ok(c.this)
}

fn object(o: u32) -> Value {
    Value::reference(RefKind::Object, o)
}

const OFFSET_OUTSIDE: &str = "Offset is outside the bounds of the DataView";

impl Agent {
    /// `this` as a `SharedArrayBuffer`, or the TypeError of `method`.
    fn this_shared(&mut self, this: Value, method: &str) -> Res<u32> {
        match this.as_object().filter(|&o| self.heap.is_shared(o)) {
            Some(o) => Ok(o),
            None => Err(self.incompatible(method, this)),
        }
    }

    /// `this` as a `DataView`, or the TypeError of `method`.
    fn this_data_view(&mut self, this: Value, method: &str) -> Res<u32> {
        match this
            .as_object()
            .filter(|&o| self.heap.data_view(o).is_some())
        {
            Some(o) => Ok(o),
            None => Err(self.incompatible(method, this)),
        }
    }

    fn buffer_len(&self, b: u32) -> usize {
        self.heap.buffer(b).map_or(0, |b| b.bytes.len())
    }

    /// GetViewValue (section 25.3.1.5).
    fn view_value(&mut self, c: Call, elem: Elem, method: &str) -> Res<Value> {
        let o = self.this_data_view(c.this, method)?;
        let Some(i) = self.index_value(self.arg(c, 0))? else {
            return Err(self.range_error(OFFSET_OUTSIDE));
        };
        let little = self.to_boolean(self.arg(c, 1));
        let at = self.view_index(o, i, elem, method)?;
        let mut bytes = self
            .heap
            .data_view(o)
            .and_then(|v| self.heap.buffer_bytes(v.buffer, at, elem.size()))
            .unwrap_or_default();
        if !little {
            bytes.reverse();
        }
        Ok(Value::number(elem.read(&bytes)))
    }

    /// SetViewValue (section 25.3.1.6).
    fn set_view_value(&mut self, c: Call, elem: Elem, method: &str) -> Res<Value> {
        let o = self.this_data_view(c.this, method)?;
        let Some(i) = self.index_value(self.arg(c, 0))? else {
            return Err(self.range_error(OFFSET_OUTSIDE));
        };
        let n = self.number(self.arg(c, 1))?;
        let little = self.to_boolean(self.arg(c, 2));
        let at = self.view_index(o, i, elem, method)?;
        let mut bytes = vec![0; elem.size()];
        elem.write(&mut bytes, n);
        if !little {
            bytes.reverse();
        }
        if let Some(v) = self.heap.data_view(o) {
            self.heap.store_buffer_bytes(v.buffer, at, &bytes);
        }
        Ok(Value::UNDEFINED)
    }

    /// The buffer index of byte `i` of view `o`, checked to have an
    /// element of `elem` there.
    fn view_index(&mut self, o: u32, i: usize, elem: Elem, method: &str) -> Res<usize> {
        let Some(size) = self.heap.data_view_length(o) else {
            return Err(self.out_of_bounds(method));
        };
        if i.checked_add(elem.size()).is_none_or(|end| end > size) {
            return Err(self.range_error(OFFSET_OUTSIDE));
        }
        Ok(self.heap.data_view(o).map_or(0, |v| v.offset) + i)
    }
}

// SharedArrayBuffer.

pub(crate) fn construct_shared_array_buffer(agent: &mut Agent, c: Call) -> Res<Value> {
    if c.new_target.is_undefined() {
        return Err(agent.type_error("Constructor SharedArrayBuffer requires 'new'"));
    }
    let (len, max) = buffer_length_and_max(agent, c)?;
    let proto =
        agent.prototype_from(c.new_target, |r| r.intrinsics.shared_array_buffer_prototype)?;
    agent.new_buffer_of(proto, len, max, ObjectKind::SharedArrayBuffer)
}

fn shared_byte_length(agent: &mut Agent, c: Call) -> Res<Value> {
    let b = agent.this_shared(c.this, "get SharedArrayBuffer.prototype.byteLength")?;
    Ok(Value::number(agent.buffer_len(b) as f64))
}

fn shared_max_byte_length(agent: &mut Agent, c: Call) -> Res<Value> {
    let b = agent.this_shared(c.this, "get SharedArrayBuffer.prototype.maxByteLength")?;
    let n = agent
        .heap
        .buffer(b)
        .map_or(0, |b| b.max.unwrap_or(b.bytes.len()));
    Ok(Value::number(n as f64))
}

fn growable(agent: &mut Agent, c: Call) -> Res<Value> {
    let b = agent.this_shared(c.this, "get SharedArrayBuffer.prototype.growable")?;
    Ok(Value::bool(
        agent.heap.buffer(b).is_some_and(|b| b.max.is_some()),
    ))
}

/// `SharedArrayBuffer.prototype.grow` (section 25.2.5.3).
fn grow(agent: &mut Agent, c: Call) -> Res<Value> {
    const M: &str = "SharedArrayBuffer.prototype.grow";
    let b = agent.this_shared(c.this, M)?;
    let Some(max) = agent.heap.buffer(b).and_then(|b| b.max) else {
        return Err(agent.incompatible(M, c.this));
    };
    let new_len = agent.index_value(agent.arg(c, 0))?;
    let old = agent.buffer_len(b);
    let Some(new_len) = new_len.filter(|&n| n <= max && n >= old) else {
        return Err(agent.range_error(format!("{M}: Invalid length parameter")));
    };
    if new_len > old && agent.reserve(new_len - old).is_err() {
        return Err(agent.range_error("Array buffer allocation failed"));
    }
    agent
        .heap
        .with_buffer(b, |buf| buf.bytes.resize(new_len, 0));
    Ok(Value::UNDEFINED)
}

/// `SharedArrayBuffer.prototype.slice` (section 25.2.5.6).
fn shared_slice(agent: &mut Agent, c: Call) -> Res<Value> {
    const M: &str = "SharedArrayBuffer.prototype.slice";
    let b = agent.this_shared(c.this, M)?;
    let len = agent.buffer_len(b) as f64;
    let first = agent.relative(agent.arg(c, 0), len, 0.0)?;
    let last = agent.relative(agent.arg(c, 1), len, len)?;
    let new_len = (last - first).max(0.0);
    let default = agent.realm().intrinsics.shared_array_buffer;
    let ctor = agent.species_constructor(c.this, default)?;
    let new = agent.construct_value(ctor, &[Value::number(new_len)], ctor)?;
    let Some(n) = new.as_object().filter(|&o| agent.heap.is_shared(o)) else {
        return Err(agent.incompatible(M, new));
    };
    if n == b {
        return Err(
            agent.type_error("SharedArrayBuffer subclass returned this from species constructor")
        );
    }
    let (first, new_len) = (first as usize, new_len as usize);
    if agent.buffer_len(n) < new_len {
        return Err(agent.type_error(
            "Derived SharedArrayBuffer constructor created a buffer which was too small",
        ));
    }
    let bytes = agent
        .heap
        .buffer_bytes(b, first, new_len)
        .unwrap_or_default();
    agent.heap.store_buffer_bytes(n, 0, &bytes);
    Ok(new)
}

// DataView.

/// The `DataView` constructor (section 25.3.2.1).
pub(crate) fn construct_data_view(agent: &mut Agent, c: Call) -> Res<Value> {
    const M: &str = "DataView constructor";
    if c.new_target.is_undefined() {
        return Err(agent.type_error("Constructor DataView requires 'new'"));
    }
    let buffer = agent.arg(c, 0);
    let Some(b) = buffer.as_object().filter(|&o| agent.heap.is_buffer(o)) else {
        return Err(
            agent.type_error("First argument to DataView constructor must be an ArrayBuffer")
        );
    };
    let outside = |agent: &mut Agent, offset: f64| {
        agent.range_error(format!(
            "Start offset {} is outside the bounds of the buffer",
            number_to_string(offset)
        ))
    };
    let offset = agent.integer(agent.arg(c, 1))?;
    if !(0.0..=MAX_SAFE).contains(&offset) {
        return Err(outside(agent, offset));
    }
    if agent.is_detached(b) {
        return Err(agent.detached_error(M));
    }
    let size = agent.buffer_len(b) as f64;
    if offset > size {
        return Err(outside(agent, offset));
    }
    let fixed = agent.heap.buffer(b).is_some_and(|b| b.max.is_none());
    let length_arg = agent.arg(c, 2);
    let length = if length_arg.is_undefined() {
        fixed.then_some(size - offset)
    } else {
        let n = agent.integer(length_arg)?;
        if !(0.0..=MAX_SAFE).contains(&n) || offset + n > size {
            return Err(
                agent.range_error(format!("Invalid DataView length {}", number_to_string(n)))
            );
        }
        Some(n)
    };
    let proto = agent.prototype_from(c.new_target, |r| r.intrinsics.data_view_prototype)?;
    if agent.is_detached(b) {
        return Err(agent.detached_error(M));
    }
    let size = agent.buffer_len(b) as f64;
    if offset > size {
        return Err(outside(agent, offset));
    }
    if !length_arg.is_undefined() && length.is_some_and(|n| offset + n > size) {
        return Err(agent.range_error("Invalid DataView length undefined"));
    }
    agent.vm.temps.push(proto);
    let made = agent.alloc_object(ObjectCell::with_internal(
        proto,
        ObjectKind::DataView,
        vec![
            buffer,
            Value::number(offset),
            length.map_or(Value::UNDEFINED, Value::number),
        ],
    ));
    agent.vm.temps.pop();
    made
}

fn view_buffer(agent: &mut Agent, c: Call) -> Res<Value> {
    let o = agent.this_data_view(c.this, "get DataView.prototype.buffer")?;
    Ok(agent
        .heap
        .data_view(o)
        .map_or(Value::UNDEFINED, |v| object(v.buffer)))
}

fn view_byte_length(agent: &mut Agent, c: Call) -> Res<Value> {
    const M: &str = "get DataView.prototype.byteLength";
    let o = agent.this_data_view(c.this, M)?;
    match agent.heap.data_view_length(o) {
        Some(n) => Ok(Value::number(n as f64)),
        None => Err(agent.out_of_bounds(M)),
    }
}

fn view_byte_offset(agent: &mut Agent, c: Call) -> Res<Value> {
    const M: &str = "get DataView.prototype.byteOffset";
    let o = agent.this_data_view(c.this, M)?;
    if agent.heap.data_view_length(o).is_none() {
        return Err(agent.out_of_bounds(M));
    }
    Ok(Value::number(
        agent.heap.data_view(o).map_or(0, |v| v.offset) as f64,
    ))
}

macro_rules! view_accessors {
    ($($get:ident, $set:ident = $e:ident, $name:literal;)*) => {
        $(
            fn $get(agent: &mut Agent, c: Call) -> Res<Value> {
                agent.view_value(c, Elem::$e, concat!("DataView.prototype.get", $name))
            }

            fn $set(agent: &mut Agent, c: Call) -> Res<Value> {
                agent.set_view_value(c, Elem::$e, concat!("DataView.prototype.set", $name))
            }
        )*
    };
}

view_accessors! {
    get_int8, set_int8 = Int8, "Int8";
    get_uint8, set_uint8 = Uint8, "Uint8";
    get_int16, set_int16 = Int16, "Int16";
    get_uint16, set_uint16 = Uint16, "Uint16";
    get_int32, set_int32 = Int32, "Int32";
    get_uint32, set_uint32 = Uint32, "Uint32";
    get_float16, set_float16 = Float16, "Float16";
    get_float32, set_float32 = Float32, "Float32";
    get_float64, set_float64 = Float64, "Float64";
}
