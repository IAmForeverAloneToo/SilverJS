//! `ArrayBuffer` and the typed arrays (ECMA-262 sections 25.1 and 23.2):
//! the buffer's constructor and methods, `%TypedArray%` with its
//! prototype's methods, and the ten constructors of the number types
//! ([`crate::buffer::Elem`]). The elements themselves are read and
//! written by the heap ([`crate::buffer`]); a buffer's object names its
//! data block, an internal cell, in its first internal slot.

use super::iterator::ArrayIterKind;
use super::{CONFIGURABLE, Call, Entry, FIXED, HIDDEN, Name, What, Wk, entry, getter, method};
use crate::agent::Agent;
use crate::buffer::{Buffer, Elem};
use crate::heap::{InternalCell, ObjectCell};
use crate::interp::messages;
use crate::interp::realm::Realm;
use crate::interp::{Abrupt, Res};
use crate::object::ObjectKind;
use crate::string::PropertyKey;
use crate::value::{RefKind, Value};

/// The largest buffer made: past it a length is refused outright. The
/// reference browser refuses 10^12 bytes as a length and attempts 2^34
/// (failing for want of memory); this bound lies between.
pub(super) const MAX_BYTES: usize = 1 << 35;
pub(super) const MAX_SAFE: f64 = 9_007_199_254_740_991.0;

/// `ArrayBuffer`.
pub(crate) const ARRAY_BUFFER: &[Entry] = &[
    method("isView", 1, is_view),
    entry(Name::Sym(Wk::Species), What::Getter(species), CONFIGURABLE),
];

/// `ArrayBuffer.prototype`.
pub(crate) const ARRAY_BUFFER_PROTO: &[Entry] = &[
    entry(Name::Str("constructor"), What::Constructor, HIDDEN),
    getter("byteLength", byte_length),
    method("slice", 2, slice_buffer),
    getter("maxByteLength", max_byte_length),
    getter("resizable", resizable),
    method("resize", 1, resize),
    method("transfer", 0, transfer),
    method("transferToFixedLength", 0, transfer_to_fixed_length),
    getter("detached", detached),
    entry(
        Name::Sym(Wk::ToStringTag),
        What::Str("ArrayBuffer"),
        CONFIGURABLE,
    ),
];

/// `%TypedArray%`.
pub(crate) const TYPED_ARRAY: &[Entry] = &[
    method("of", 0, of),
    method("from", 1, from),
    entry(Name::Sym(Wk::Species), What::Getter(species), CONFIGURABLE),
];

/// `%TypedArray%.prototype`; its `toString` is `Array.prototype`'s,
/// given.
pub(crate) const TYPED_ARRAY_PROTO: &[Entry] = &[
    entry(Name::Str("constructor"), What::Constructor, HIDDEN),
    getter("buffer", buffer),
    getter("byteLength", view_byte_length),
    getter("byteOffset", byte_offset),
    getter("length", length),
    method("entries", 0, entries),
    method("keys", 0, keys),
    method("values", 0, values),
    method("at", 1, at),
    method("copyWithin", 2, copy_within),
    method("every", 1, every),
    method("fill", 1, fill),
    method("filter", 1, filter),
    method("find", 1, find),
    method("findIndex", 1, find_index),
    method("findLast", 1, find_last),
    method("findLastIndex", 1, find_last_index),
    method("forEach", 1, for_each),
    method("includes", 1, includes),
    method("indexOf", 1, index_of),
    method("join", 1, join),
    method("lastIndexOf", 1, last_index_of),
    method("map", 1, map),
    method("reverse", 0, reverse),
    method("reduce", 1, reduce),
    method("reduceRight", 1, reduce_right),
    method("set", 1, set),
    method("slice", 2, slice),
    method("some", 1, some),
    method("sort", 1, sort),
    method("subarray", 2, subarray),
    method("toReversed", 0, to_reversed),
    method("toSorted", 1, to_sorted),
    method("with", 2, with),
    method("toLocaleString", 0, to_locale_string),
    entry(Name::Str("toString"), What::Given(0), HIDDEN),
    entry(Name::Sym(Wk::ToStringTag), What::Getter(tag), CONFIGURABLE),
    entry(Name::Sym(Wk::Iterator), What::Alias("values"), HIDDEN),
];

/// Each typed array constructor: its `BYTES_PER_ELEMENT`, given.
pub(crate) const TYPED_CTOR: &[Entry] =
    &[entry(Name::Str("BYTES_PER_ELEMENT"), What::Given(0), FIXED)];

/// Each typed array constructor's prototype.
pub(crate) const TYPED_PROTO: &[Entry] = &[
    entry(Name::Str("constructor"), What::Constructor, HIDDEN),
    entry(Name::Str("BYTES_PER_ELEMENT"), What::Given(0), FIXED),
];

fn species(_: &mut Agent, c: Call) -> Res<Value> {
    Ok(c.this)
}

fn object(o: u32) -> Value {
    Value::reference(RefKind::Object, o)
}

fn number(n: Option<f64>) -> Value {
    n.map_or(Value::UNDEFINED, Value::number)
}

impl Agent {
    /// A typed array's element written with a value whose conversion may
    /// run script (TypedArraySetElement, section 10.4.5.16).
    pub(crate) fn typed_set(&mut self, t: u32, key: PropertyKey, value: Value) -> Res<()> {
        let n = self.number(value)?;
        if let Some(Some(i)) = self.heap.typed_key(t, key) {
            self.heap.view_set(t, i, n);
        }
        Ok(())
    }

    /// ToIndex (section 7.1.22): `None` when out of range.
    pub(super) fn index_value(&mut self, v: Value) -> Res<Option<usize>> {
        let n = self.integer(v)?;
        Ok((0.0..=MAX_SAFE).contains(&n).then_some(n as usize))
    }

    /// AllocateArrayBuffer's data block and object (section 25.1.3.1):
    /// `len` zero bytes, resizable up to `max`, with prototype `proto`.
    pub(crate) fn new_buffer(
        &mut self,
        proto: Value,
        len: usize,
        max: Option<usize>,
    ) -> Res<Value> {
        self.new_buffer_of(proto, len, max, ObjectKind::ArrayBuffer)
    }

    /// An `ArrayBuffer` or, by `kind`, a `SharedArrayBuffer`.
    pub(super) fn new_buffer_of(
        &mut self,
        proto: Value,
        len: usize,
        max: Option<usize>,
        kind: ObjectKind,
    ) -> Res<Value> {
        if len > MAX_BYTES {
            return Err(self.range_error("Invalid array buffer length"));
        }
        let mut bytes = Vec::new();
        if self.reserve(len + 128).is_err() || bytes.try_reserve_exact(len).is_err() {
            return Err(self.range_error("Array buffer allocation failed"));
        }
        bytes.resize(len, 0);
        self.vm.temps.push(proto);
        let cell = self.alloc_internal(InternalCell::Buffer(Box::new(Buffer {
            bytes,
            max,
            detached: false,
        })));
        let made = cell.and_then(|cell| {
            self.vm.temps.push(cell);
            let o = self.alloc_object(ObjectCell::with_internal(proto, kind, vec![cell]));
            self.vm.temps.pop();
            o
        });
        self.vm.temps.pop();
        made
    }

    /// `this` as an `ArrayBuffer`, or the TypeError of `method` called on
    /// something else.
    fn this_buffer(&mut self, this: Value, method: &str) -> Res<u32> {
        let found = this.as_object().filter(|&o| {
            self.heap
                .objects
                .get(o)
                .is_some_and(|c| c.kind() == ObjectKind::ArrayBuffer)
        });
        match found {
            Some(o) => Ok(o),
            None => Err(self.incompatible(method, this)),
        }
    }

    pub(super) fn incompatible(&mut self, method: &str, this: Value) -> Abrupt {
        let text = self.display(this);
        self.type_error(format!(
            "Method {method} called on incompatible receiver {text}"
        ))
    }

    pub(super) fn is_detached(&self, b: u32) -> bool {
        self.heap.buffer(b).is_none_or(|b| b.detached)
    }

    pub(super) fn detached_error(&mut self, method: &str) -> Abrupt {
        self.type_error(format!("Cannot perform {method} on a detached ArrayBuffer"))
    }

    pub(super) fn out_of_bounds(&mut self, method: &str) -> Abrupt {
        self.type_error(format!(
            "Cannot perform {method} on a detached or out-of-bounds ArrayBuffer"
        ))
    }

    /// A typed array over `buffer`.
    fn new_view(
        &mut self,
        proto: Value,
        buffer: Value,
        elem: Elem,
        offset: usize,
        length: Option<usize>,
    ) -> Res<Value> {
        let slots = vec![
            buffer,
            Value::int(elem as i32),
            Value::number(offset as f64),
            length.map_or(Value::UNDEFINED, |n| Value::number(n as f64)),
        ];
        self.alloc_object(ObjectCell::with_internal(
            proto,
            ObjectKind::TypedArray,
            slots,
        ))
    }

    /// AllocateTypedArray with a length (section 23.2.5.1.1): a new
    /// buffer of its own.
    pub(super) fn view_with_length(&mut self, proto: Value, elem: Elem, len: usize) -> Res<Value> {
        let Some(bytes) = len.checked_mul(elem.size()).filter(|&b| b <= MAX_BYTES) else {
            return Err(self.range_error(format!("Invalid typed array length: {len}")));
        };
        let ab = self.realm().intrinsics.array_buffer_prototype;
        self.vm.temps.push(proto);
        let buf = self.new_buffer(ab, bytes, None);
        let made = buf.and_then(|buf| {
            self.vm.temps.push(buf);
            let o = self.new_view(proto, buf, elem, 0, Some(len));
            self.vm.temps.pop();
            o
        });
        self.vm.temps.pop();
        made
    }

    /// TypedArrayCreateSameType (section 23.2.4.3): a new typed array of
    /// `o`'s type, this realm's.
    fn view_same_type(&mut self, o: u32, len: usize) -> Res<u32> {
        let elem = self.heap.view(o).map_or(Elem::Uint8, |v| v.elem);
        let proto = self.realm().intrinsics.typed_proto(elem);
        let a = self.view_with_length(proto, elem, len)?;
        Ok(a.as_object().unwrap_or(0))
    }

    /// ValidateTypedArray (section 23.2.4.4): `this` as a typed array in
    /// bounds and its length, or the TypeError `method` throws.
    pub(super) fn this_view(&mut self, this: Value, method: &str) -> Res<(u32, usize)> {
        let Some(o) = this.as_object().filter(|&o| self.heap.view(o).is_some()) else {
            return Err(self.type_error("this is not a typed array."));
        };
        match self.heap.view_length(o) {
            Some(n) => Ok((o, n)),
            None => Err(self.out_of_bounds(method)),
        }
    }

    /// `this` as a typed array, in bounds or not.
    pub(super) fn any_view(&mut self, this: Value, method: &str) -> Res<u32> {
        match this.as_object().filter(|&o| self.heap.view(o).is_some()) {
            Some(o) => Ok(o),
            None => Err(self.incompatible(method, this)),
        }
    }

    /// SpeciesConstructor (section 7.3.22) of `o`, `default` when it
    /// names none.
    pub(crate) fn species_constructor(&mut self, o: Value, default: Value) -> Res<Value> {
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

    /// TypedArrayCreateFromConstructor (section 23.2.4.2): `c` constructed
    /// with `args`, checked to be a typed array in bounds, and long enough
    /// when `args` is a length.
    fn view_create(&mut self, c: Value, args: &[Value], method: &str) -> Res<u32> {
        let new = self.construct_value(c, args, c)?;
        let Some(o) = new.as_object().filter(|&o| self.heap.view(o).is_some()) else {
            return Err(self.incompatible(method, new));
        };
        let Some(len) = self.heap.view_length(o) else {
            return Err(self.out_of_bounds(method));
        };
        if let [n] = args
            && let Some(n) = n.as_number()
            && (len as f64) < n
        {
            return Err(self.type_error(
                "Derived TypedArray constructor created an array which was too small",
            ));
        }
        Ok(o)
    }

    /// TypedArraySpeciesCreate (section 23.2.4.1).
    fn species_view(&mut self, exemplar: u32, args: &[Value], method: &str) -> Res<u32> {
        let elem = self.heap.view(exemplar).map_or(Elem::Uint8, |v| v.elem);
        let default = self.realm().intrinsics.typed_ctor(elem);
        let c = self.species_constructor(object(exemplar), default)?;
        self.view_create(c, args, method)
    }

    /// Element `k` written with `v` converted (Set on a typed array).
    fn view_put(&mut self, o: u32, k: usize, v: Value) -> Res<()> {
        let n = self.number(v)?;
        self.heap.view_set(o, k, n);
        Ok(())
    }

    fn element(&self, o: u32, k: usize) -> Value {
        number(self.heap.view_get(o, k))
    }

    /// A typed array constructor (section 23.2.5.1).
    fn construct_typed(&mut self, c: Call, elem: Elem) -> Res<Value> {
        if c.new_target.is_undefined() {
            return Err(self.type_error(format!("Constructor {} requires 'new'", elem.name())));
        }
        let default = move |r: &Realm| r.intrinsics.typed_proto(elem);
        let first = self.arg(c, 0);
        let Some(source) = first.as_object() else {
            let n = self.integer(first)?;
            if !(0.0..=MAX_SAFE).contains(&n) {
                let text = crate::number::number_to_string(n);
                return Err(self.range_error(format!("Invalid typed array length: {text}")));
            }
            let proto = self.prototype_from(c.new_target, default)?;
            return self.view_with_length(proto, elem, n as usize);
        };
        let proto = self.prototype_from(c.new_target, default)?;
        self.vm.temps.push(proto);
        let r = (|| {
            if self.heap.view(source).is_some() {
                return self.view_from_view(proto, elem, source);
            }
            if self.heap.is_buffer(source) {
                return self.view_from_buffer(c, proto, elem, source);
            }
            self.view_from_object(proto, elem, first)
        })();
        self.vm.temps.pop();
        r
    }

    /// InitializeTypedArrayFromTypedArray (section 23.2.5.1.2).
    fn view_from_view(&mut self, proto: Value, elem: Elem, src: u32) -> Res<Value> {
        let Some(len) = self.heap.view_length(src) else {
            return Err(self.out_of_bounds("Construct"));
        };
        let a = self.view_with_length(proto, elem, len)?;
        let o = a.as_object().unwrap_or(0);
        if self.heap.view(src).is_some_and(|v| v.elem == elem) {
            let bytes = self.heap.view_bytes(src, 0, len);
            self.heap.store_view_bytes(o, 0, &bytes);
        } else {
            for k in 0..len {
                if let Some(n) = self.heap.view_get(src, k) {
                    self.heap.view_set(o, k, n);
                }
            }
        }
        Ok(a)
    }

    /// InitializeTypedArrayFromArrayBuffer (section 23.2.5.1.3).
    fn view_from_buffer(&mut self, c: Call, proto: Value, elem: Elem, b: u32) -> Res<Value> {
        let es = elem.size();
        let offset_arg = self.arg(c, 1);
        let Some(offset) = self.index_value(offset_arg)? else {
            let text = self.display(offset_arg);
            return Err(self.range_error(format!(
                "Start offset {text} is outside the bounds of the buffer"
            )));
        };
        if offset % es != 0 {
            return Err(self.range_error(format!(
                "start offset of {} should be a multiple of {es}",
                elem.name()
            )));
        }
        let length_arg = self.arg(c, 2);
        let new_length = if length_arg.is_undefined() {
            None
        } else {
            match self.index_value(length_arg)? {
                Some(n) => Some(n),
                None => {
                    let text = self.display(length_arg);
                    return Err(self.range_error(format!("Invalid typed array length: {text}")));
                }
            }
        };
        if self.is_detached(b) {
            return Err(self.detached_error("Construct"));
        }
        let (size, resizable) = self
            .heap
            .buffer(b)
            .map_or((0, false), |buf| (buf.bytes.len(), buf.max.is_some()));
        let outside = |agent: &mut Agent| {
            agent.range_error(format!(
                "Start offset {offset} is outside the bounds of the buffer"
            ))
        };
        let length = match new_length {
            None if resizable => {
                if offset > size {
                    return Err(outside(self));
                }
                None
            }
            None => {
                if size % es != 0 {
                    return Err(self.range_error(format!(
                        "byte length of {} should be a multiple of {es}",
                        elem.name()
                    )));
                }
                if offset > size {
                    return Err(outside(self));
                }
                Some((size - offset) / es)
            }
            Some(n) => {
                if n.checked_mul(es)
                    .and_then(|bytes| bytes.checked_add(offset))
                    .is_none_or(|end| end > size)
                {
                    return Err(self.range_error(format!("Invalid typed array length: {n}")));
                }
                Some(n)
            }
        };
        self.new_view(proto, object(b), elem, offset, length)
    }

    /// InitializeTypedArrayFromList and FromArrayLike (sections
    /// 23.2.5.1.4 and 23.2.5.1.5).
    fn view_from_object(&mut self, proto: Value, elem: Elem, source: Value) -> Res<Value> {
        let key = self.well_known_key(Wk::Iterator);
        let using = self.get_method(source, key)?;
        let base = self.vm.temps.len();
        let r = (|| {
            if !using.is_undefined() {
                let values = self.list_from_iterator(source, using)?;
                let a = self.view_with_length(proto, elem, values.len())?;
                self.vm.temps.push(a);
                let o = a.as_object().unwrap_or(0);
                for (k, v) in values.into_iter().enumerate() {
                    self.view_put(o, k, v)?;
                }
                return Ok(a);
            }
            let len = self.length_of(source)?;
            let a = self.view_with_length(proto, elem, len as usize)?;
            self.vm.temps.push(a);
            let o = a.as_object().unwrap_or(0);
            for k in 0..len as usize {
                let key = self.index_key(k as f64);
                let v = self.get_value(source, key)?;
                self.view_put(o, k, v)?;
            }
            Ok(a)
        })();
        self.vm.temps.truncate(base);
        r
    }

    /// IteratorToList of GetIteratorFromMethod (section 7.4.12): the
    /// values, rooted in `temps` until the caller truncates them.
    fn list_from_iterator(&mut self, iterable: Value, method: Value) -> Res<Vec<Value>> {
        let (it, next) = self.iterator_from_method(iterable, method)?;
        self.vm.temps.extend([it, next]);
        let mut values = Vec::new();
        while let Some(v) = self.iterator_step(it, next)? {
            self.vm.temps.push(v);
            values.push(v);
        }
        Ok(values)
    }

    /// The callback methods' loop over a typed array (sections 23.2.3.9
    /// and the like): `f(value, index, o)` for each index, `each` given
    /// the index, value and result; `true` from `each` stops it.
    fn view_each(
        &mut self,
        c: Call,
        method: &str,
        mut each: impl FnMut(&mut Agent, usize, Value, Value) -> Res<bool>,
    ) -> Res<()> {
        let (o, len) = self.this_view(c.this, method)?;
        let f = self.view_callback(self.arg(c, 0))?;
        let this_arg = self.arg(c, 1);
        for k in 0..len {
            let v = self.element(o, k);
            let r = self.call_value(f, this_arg, &[v, Value::number(k as f64), object(o)])?;
            if each(self, k, v, r)? {
                break;
            }
        }
        Ok(())
    }

    fn view_callback(&mut self, f: Value) -> Res<Value> {
        if self.is_callable(f) {
            Ok(f)
        } else {
            let text = self.display(f);
            Err(self.type_error(format!("{text} is not a function")))
        }
    }

    /// A stable merge sort of `values` by `f` (SortCompare of a typed
    /// array, section 23.2.4.7), whose calls may throw.
    fn sort_numbers(&mut self, values: &mut Vec<f64>, f: Value) -> Res<()> {
        let n = values.len();
        let mut buf = values.clone();
        let mut width = 1;
        while width < n {
            let mut i = 0;
            while i < n {
                let mid = (i + width).min(n);
                let hi = (i + 2 * width).min(n);
                let (mut a, mut b, mut out) = (i, mid, i);
                while a < mid && b < hi {
                    let x = values.get(a).copied().unwrap_or(0.0);
                    let y = values.get(b).copied().unwrap_or(0.0);
                    let take_b = self.compare_numbers(f, x, y)?;
                    if let Some(slot) = buf.get_mut(out) {
                        *slot = if take_b { y } else { x };
                    }
                    if take_b {
                        b += 1;
                    } else {
                        a += 1;
                    }
                    out += 1;
                }
                for j in (a..mid).chain(b..hi) {
                    if let (Some(slot), Some(&v)) = (buf.get_mut(out), values.get(j)) {
                        *slot = v;
                    }
                    out += 1;
                }
                i += 2 * width;
            }
            std::mem::swap(values, &mut buf);
            width *= 2;
        }
        Ok(())
    }

    /// Whether `y` sorts before `x`.
    fn compare_numbers(&mut self, f: Value, x: f64, y: f64) -> Res<bool> {
        if f.is_undefined() {
            return Ok(match (x.is_nan(), y.is_nan()) {
                (true, _) => !y.is_nan(),
                (false, true) => false,
                _ => {
                    x > y || (x == 0.0 && y == 0.0 && !x.is_sign_negative() && y.is_sign_negative())
                }
            });
        }
        self.tick()?;
        let r = self.call_value(f, Value::UNDEFINED, &[Value::number(x), Value::number(y)])?;
        let n = self.number(r)?;
        Ok(n > 0.0)
    }

    fn comparator_of(&mut self, f: Value) -> Res<Value> {
        if !f.is_undefined() && !self.is_callable(f) {
            let text = self.display(f);
            return Err(self.type_error(format!(
                "The comparison function must be either a function or undefined: {text}"
            )));
        }
        Ok(f)
    }

    /// Element `k` of `o` read and written as raw bytes into `a`: a
    /// copy of `count` elements of the same type.
    fn copy_elements(&mut self, from: u32, k: usize, to: u32, count: usize) {
        let bytes = self.heap.view_bytes(from, k, k + count);
        self.heap.store_view_bytes(to, 0, &bytes);
    }
}

// The constructors.

pub(crate) fn construct_array_buffer(agent: &mut Agent, c: Call) -> Res<Value> {
    if c.new_target.is_undefined() {
        return Err(agent.type_error("Constructor ArrayBuffer requires 'new'"));
    }
    let (len, max) = buffer_length_and_max(agent, c)?;
    let proto = agent.prototype_from(c.new_target, |r| r.intrinsics.array_buffer_prototype)?;
    agent.new_buffer(proto, len, max)
}

/// A buffer constructor's length and its options' `maxByteLength`
/// (sections 25.1.4.1 and 25.2.3.1).
pub(super) fn buffer_length_and_max(agent: &mut Agent, c: Call) -> Res<(usize, Option<usize>)> {
    let Some(len) = agent.index_value(agent.arg(c, 0))? else {
        return Err(agent.range_error("Invalid array buffer length"));
    };
    let options = agent.arg(c, 1);
    let mut max = None;
    if options.as_object().is_some() {
        let key = agent.name_key(Name::Str("maxByteLength"));
        let m = agent.get_value(options, key)?;
        if !m.is_undefined() {
            match agent.index_value(m)? {
                Some(m) if m <= MAX_BYTES && len <= m => max = Some(m),
                _ => return Err(agent.range_error("Invalid array buffer max length")),
            }
        }
    }
    Ok((len, max))
}

pub(crate) fn construct_abstract(agent: &mut Agent, _: Call) -> Res<Value> {
    Err(agent.type_error("Abstract class TypedArray not directly constructable"))
}

macro_rules! typed_constructors {
    ($($f:ident = $e:ident),* $(,)?) => {
        $(
            pub(crate) fn $f(agent: &mut Agent, c: Call) -> Res<Value> {
                agent.construct_typed(c, Elem::$e)
            }
        )*
    };
}

typed_constructors!(
    construct_uint8 = Uint8,
    construct_int8 = Int8,
    construct_uint16 = Uint16,
    construct_int16 = Int16,
    construct_uint32 = Uint32,
    construct_int32 = Int32,
    construct_uint8_clamped = Uint8Clamped,
    construct_float32 = Float32,
    construct_float64 = Float64,
    construct_float16 = Float16,
);

// ArrayBuffer.

fn is_view(agent: &mut Agent, c: Call) -> Res<Value> {
    let v = agent.arg(c, 0);
    let is = v
        .as_object()
        .and_then(|o| agent.heap.objects.get(o))
        .is_some_and(|c| matches!(c.kind(), ObjectKind::TypedArray | ObjectKind::DataView));
    Ok(Value::bool(is))
}

fn byte_length(agent: &mut Agent, c: Call) -> Res<Value> {
    let b = agent.this_buffer(c.this, "get ArrayBuffer.prototype.byteLength")?;
    let n = agent.heap.buffer(b).map_or(0, |b| b.bytes.len());
    Ok(Value::number(n as f64))
}

fn max_byte_length(agent: &mut Agent, c: Call) -> Res<Value> {
    let b = agent.this_buffer(c.this, "get ArrayBuffer.prototype.maxByteLength")?;
    let n = agent.heap.buffer(b).map_or(0, |b| {
        if b.detached {
            0
        } else {
            b.max.unwrap_or(b.bytes.len())
        }
    });
    Ok(Value::number(n as f64))
}

fn resizable(agent: &mut Agent, c: Call) -> Res<Value> {
    let b = agent.this_buffer(c.this, "get ArrayBuffer.prototype.resizable")?;
    Ok(Value::bool(
        agent.heap.buffer(b).is_some_and(|b| b.max.is_some()),
    ))
}

fn detached(agent: &mut Agent, c: Call) -> Res<Value> {
    let b = agent.this_buffer(c.this, "get ArrayBuffer.prototype.detached")?;
    Ok(Value::bool(agent.is_detached(b)))
}

/// `ArrayBuffer.prototype.slice` (section 25.1.6.7).
fn slice_buffer(agent: &mut Agent, c: Call) -> Res<Value> {
    const M: &str = "ArrayBuffer.prototype.slice";
    let b = agent.this_buffer(c.this, M)?;
    if agent.is_detached(b) {
        return Err(agent.detached_error(M));
    }
    let len = agent.heap.buffer(b).map_or(0, |b| b.bytes.len()) as f64;
    let first = agent.relative(agent.arg(c, 0), len, 0.0)?;
    let last = agent.relative(agent.arg(c, 1), len, len)?;
    let new_len = (last - first).max(0.0);
    let default = agent.realm().intrinsics.array_buffer;
    let ctor = agent.species_constructor(c.this, default)?;
    let new = agent.construct_value(ctor, &[Value::number(new_len)], ctor)?;
    let Some(n) = new.as_object().filter(|&o| {
        agent
            .heap
            .objects
            .get(o)
            .is_some_and(|c| c.kind() == ObjectKind::ArrayBuffer)
    }) else {
        return Err(agent.incompatible(M, new));
    };
    if agent.is_detached(n) {
        return Err(agent.detached_error(M));
    }
    if n == b {
        return Err(agent.type_error("ArrayBuffer subclass returned this from species constructor"));
    }
    if agent.heap.buffer(n).map_or(0, |b| b.bytes.len()) < new_len as usize {
        return Err(agent
            .type_error("Derived ArrayBuffer constructor created a buffer which was too small"));
    }
    if agent.is_detached(b) {
        return Err(agent.detached_error(M));
    }
    let (first, new_len) = (first as usize, new_len as usize);
    let bytes: Vec<u8> = agent.heap.buffer(b).map_or_else(Vec::new, |buf| {
        let end = (first + new_len).min(buf.bytes.len());
        buf.bytes
            .get(first.min(end)..end)
            .unwrap_or_default()
            .to_vec()
    });
    agent.heap.with_buffer(n, |buf| {
        if let Some(dst) = buf.bytes.get_mut(..bytes.len()) {
            dst.copy_from_slice(&bytes);
        }
    });
    Ok(new)
}

/// `ArrayBuffer.prototype.resize` (section 25.1.6.6).
fn resize(agent: &mut Agent, c: Call) -> Res<Value> {
    const M: &str = "ArrayBuffer.prototype.resize";
    let b = agent.this_buffer(c.this, M)?;
    let Some(max) = agent.heap.buffer(b).and_then(|b| b.max) else {
        return Err(agent.incompatible(M, c.this));
    };
    let new_len = agent.index_value(agent.arg(c, 0))?;
    if agent.is_detached(b) {
        return Err(agent.detached_error(M));
    }
    let Some(new_len) = new_len.filter(|&n| n <= max) else {
        return Err(agent.range_error(format!("{M}: Invalid length parameter")));
    };
    let old = agent.heap.buffer(b).map_or(0, |b| b.bytes.len());
    if new_len > old && agent.reserve(new_len - old).is_err() {
        return Err(agent.range_error("Array buffer allocation failed"));
    }
    agent.heap.with_buffer(b, |buf| {
        buf.bytes.resize(new_len, 0);
        buf.bytes.shrink_to(new_len.max(buf.bytes.capacity() / 2));
    });
    Ok(Value::UNDEFINED)
}

/// ArrayBufferCopyAndDetach (section 25.1.3.3): `transfer` and
/// `transferToFixedLength`.
fn transfer_with(agent: &mut Agent, c: Call, preserve: bool) -> Res<Value> {
    let m = if preserve {
        "ArrayBuffer.prototype.transfer"
    } else {
        "ArrayBuffer.prototype.transferToFixedLength"
    };
    let b = agent.this_buffer(c.this, m)?;
    let arg = agent.arg(c, 0);
    let new_len = if arg.is_undefined() {
        agent.heap.buffer(b).map_or(0, |b| b.bytes.len())
    } else {
        match agent.index_value(arg)? {
            Some(n) => n,
            None => return Err(agent.range_error("Invalid array buffer length")),
        }
    };
    if agent.is_detached(b) {
        return Err(agent.detached_error(m));
    }
    let max = if preserve {
        agent.heap.buffer(b).and_then(|b| b.max)
    } else {
        None
    };
    if max.is_some_and(|m| new_len > m) {
        return Err(agent.range_error(format!("{m}: Invalid length parameter")));
    }
    if new_len > MAX_BYTES {
        return Err(agent.range_error("Invalid array buffer length"));
    }
    let old = agent.heap.buffer(b).map_or(0, |b| b.bytes.len());
    if new_len > old && agent.reserve(new_len - old).is_err() {
        return Err(agent.range_error("Array buffer allocation failed"));
    }
    let proto = agent.realm().intrinsics.array_buffer_prototype;
    let new = agent.new_buffer(proto, 0, max)?;
    let mut bytes = agent
        .heap
        .with_buffer(b, |buf| {
            buf.detached = true;
            std::mem::take(&mut buf.bytes)
        })
        .unwrap_or_default();
    bytes.resize(new_len, 0);
    if let Some(n) = new.as_object() {
        agent.heap.with_buffer(n, |buf| buf.bytes = bytes);
    }
    Ok(new)
}

fn transfer(agent: &mut Agent, c: Call) -> Res<Value> {
    transfer_with(agent, c, true)
}

fn transfer_to_fixed_length(agent: &mut Agent, c: Call) -> Res<Value> {
    transfer_with(agent, c, false)
}

// %TypedArray%.

/// `%TypedArray%.from` (section 23.2.2.1).
fn from(agent: &mut Agent, c: Call) -> Res<Value> {
    const M: &str = "%TypedArray%.from";
    let ctor = c.this;
    if !agent.is_constructor_value(ctor) {
        let text = agent.display(ctor);
        return Err(agent.type_error(messages::not_a_constructor(&text)));
    }
    let (source, mapfn, this_arg) = (agent.arg(c, 0), agent.arg(c, 1), agent.arg(c, 2));
    let mapping = !mapfn.is_undefined();
    if mapping && !agent.is_callable(mapfn) {
        let text = agent.typed_display(mapfn);
        return Err(agent.type_error(format!("{text} is not a function")));
    }
    let key = agent.well_known_key(Wk::Iterator);
    let using = agent.get_method(source, key)?;
    let base = agent.vm.temps.len();
    let r = (|| {
        let (values, array_like) = if using.is_undefined() {
            let a = agent.object_of(source)?;
            agent.vm.temps.push(a);
            (None, a)
        } else {
            (
                Some(agent.list_from_iterator(source, using)?),
                Value::UNDEFINED,
            )
        };
        let len = match &values {
            Some(v) => v.len(),
            None => agent.length_of(array_like)? as usize,
        };
        let target = agent.view_create(ctor, &[Value::number(len as f64)], M)?;
        agent.vm.temps.push(object(target));
        for k in 0..len {
            let v = match &values {
                Some(v) => v.get(k).copied().unwrap_or(Value::UNDEFINED),
                None => {
                    let key = agent.index_key(k as f64);
                    agent.get_value(array_like, key)?
                }
            };
            let v = if mapping {
                agent.call_value(mapfn, this_arg, &[v, Value::number(k as f64)])?
            } else {
                v
            };
            let key = agent.index_key(k as f64);
            agent.put(object(target), key, v)?;
        }
        Ok(object(target))
    })();
    agent.vm.temps.truncate(base);
    r
}

/// `%TypedArray%.of` (section 23.2.2.2).
fn of(agent: &mut Agent, c: Call) -> Res<Value> {
    let ctor = c.this;
    if !agent.is_constructor_value(ctor) {
        let text = agent.display(ctor);
        return Err(agent.type_error(messages::not_a_constructor(&text)));
    }
    let o = agent.view_create(ctor, &[Value::number(c.argc as f64)], "%TypedArray%.of")?;
    agent.vm.temps.push(object(o));
    let r = (|| {
        for k in 0..c.argc {
            let key = agent.index_key(k as f64);
            agent.put(object(o), key, agent.arg(c, k))?;
        }
        Ok(object(o))
    })();
    agent.vm.temps.pop();
    r
}

// %TypedArray%.prototype's accessors.

fn buffer(agent: &mut Agent, c: Call) -> Res<Value> {
    let o = agent.any_view(c.this, "get %TypedArray%.prototype.buffer")?;
    Ok(agent
        .heap
        .view(o)
        .map_or(Value::UNDEFINED, |v| object(v.buffer)))
}

fn view_byte_length(agent: &mut Agent, c: Call) -> Res<Value> {
    let o = agent.any_view(c.this, "get %TypedArray%.prototype.byteLength")?;
    let n = match (agent.heap.view(o), agent.heap.view_length(o)) {
        (Some(v), Some(len)) => len * v.elem.size(),
        _ => 0,
    };
    Ok(Value::number(n as f64))
}

fn byte_offset(agent: &mut Agent, c: Call) -> Res<Value> {
    let o = agent.any_view(c.this, "get %TypedArray%.prototype.byteOffset")?;
    let n = match (agent.heap.view(o), agent.heap.view_length(o)) {
        (Some(v), Some(_)) => v.offset,
        _ => 0,
    };
    Ok(Value::number(n as f64))
}

fn length(agent: &mut Agent, c: Call) -> Res<Value> {
    let o = agent.any_view(c.this, "get TypedArray.prototype.length")?;
    Ok(Value::number(agent.heap.view_length(o).unwrap_or(0) as f64))
}

fn tag(agent: &mut Agent, c: Call) -> Res<Value> {
    match c.this.as_object().and_then(|o| agent.heap.view(o)) {
        Some(v) => agent.new_string(v.elem.name()),
        None => Ok(Value::UNDEFINED),
    }
}

// %TypedArray%.prototype's methods.

fn iterator_of(agent: &mut Agent, c: Call, kind: ArrayIterKind, method: &str) -> Res<Value> {
    let (o, _) = agent.this_view(c.this, method)?;
    agent.array_iterator(object(o), kind)
}

fn entries(agent: &mut Agent, c: Call) -> Res<Value> {
    iterator_of(
        agent,
        c,
        ArrayIterKind::Entries,
        "%TypedArray%.prototype.entries",
    )
}

fn keys(agent: &mut Agent, c: Call) -> Res<Value> {
    iterator_of(agent, c, ArrayIterKind::Keys, "%TypedArray%.prototype.keys")
}

fn values(agent: &mut Agent, c: Call) -> Res<Value> {
    iterator_of(
        agent,
        c,
        ArrayIterKind::Values,
        "%TypedArray%.prototype.values",
    )
}

fn at(agent: &mut Agent, c: Call) -> Res<Value> {
    let (o, len) = agent.this_view(c.this, "%TypedArray%.prototype.at")?;
    let rel = agent.integer(agent.arg(c, 0))?;
    let k = if rel >= 0.0 { rel } else { len as f64 + rel };
    if k < 0.0 || k >= len as f64 {
        return Ok(Value::UNDEFINED);
    }
    Ok(agent.element(o, k as usize))
}

/// `%TypedArray%.prototype.copyWithin` (section 23.2.3.6).
fn copy_within(agent: &mut Agent, c: Call) -> Res<Value> {
    const M: &str = "%TypedArray%.prototype.copyWithin";
    let (o, len) = agent.this_view(c.this, M)?;
    let lenf = len as f64;
    let to = agent.relative(agent.arg(c, 0), lenf, 0.0)?;
    let from = agent.relative(agent.arg(c, 1), lenf, 0.0)?;
    let end = agent.relative(agent.arg(c, 2), lenf, lenf)?;
    let count = (end - from).min(lenf - to);
    if count > 0.0 {
        let Some(len) = agent.heap.view_length(o) else {
            return Err(agent.out_of_bounds(M));
        };
        if let Some(v) = agent.heap.view(o) {
            let es = v.elem.size();
            let limit = len * es + v.offset;
            agent.heap.move_bytes(
                v.buffer,
                (from as usize * es + v.offset, to as usize * es + v.offset),
                count as usize * es,
                limit,
                true,
            );
        }
    }
    Ok(c.this)
}

fn every(agent: &mut Agent, c: Call) -> Res<Value> {
    let mut result = true;
    agent.view_each(c, "%TypedArray%.prototype.every", |agent, _, _, r| {
        result = agent.to_boolean(r);
        Ok(!result)
    })?;
    Ok(Value::bool(result))
}

fn some(agent: &mut Agent, c: Call) -> Res<Value> {
    let mut result = false;
    agent.view_each(c, "%TypedArray%.prototype.some", |agent, _, _, r| {
        result = agent.to_boolean(r);
        Ok(result)
    })?;
    Ok(Value::bool(result))
}

fn for_each(agent: &mut Agent, c: Call) -> Res<Value> {
    agent.view_each(c, "%TypedArray%.prototype.forEach", |_, _, _, _| Ok(false))?;
    Ok(Value::UNDEFINED)
}

/// `%TypedArray%.prototype.fill` (section 23.2.3.10).
fn fill(agent: &mut Agent, c: Call) -> Res<Value> {
    const M: &str = "%TypedArray%.prototype.fill";
    let (o, len) = agent.this_view(c.this, M)?;
    let n = agent.number(agent.arg(c, 0))?;
    let lenf = len as f64;
    let k = agent.relative(agent.arg(c, 1), lenf, 0.0)?;
    let end = agent.relative(agent.arg(c, 2), lenf, lenf)?;
    let Some(len) = agent.heap.view_length(o) else {
        return Err(agent.out_of_bounds(M));
    };
    for i in k as usize..(end as usize).min(len) {
        agent.heap.view_set(o, i, n);
    }
    Ok(c.this)
}

/// `%TypedArray%.prototype.filter` (section 23.2.3.11).
fn filter(agent: &mut Agent, c: Call) -> Res<Value> {
    const M: &str = "%TypedArray%.prototype.filter";
    let (o, _) = agent.this_view(c.this, M)?;
    let mut kept = Vec::new();
    agent.view_each(c, M, |agent, _, v, r| {
        if agent.to_boolean(r) {
            kept.push(v);
        }
        Ok(false)
    })?;
    let a = agent.species_view(o, &[Value::number(kept.len() as f64)], M)?;
    for (k, v) in kept.into_iter().enumerate() {
        agent.view_put(a, k, v)?;
    }
    Ok(object(a))
}

fn find_via(agent: &mut Agent, c: Call, last: bool, method: &str) -> Res<(f64, Value)> {
    let (o, len) = agent.this_view(c.this, method)?;
    let f = agent.view_callback(agent.arg(c, 0))?;
    let this_arg = agent.arg(c, 1);
    for i in 0..len {
        let k = if last { len - 1 - i } else { i };
        let v = agent.element(o, k);
        let r = agent.call_value(f, this_arg, &[v, Value::number(k as f64), object(o)])?;
        if agent.to_boolean(r) {
            return Ok((k as f64, v));
        }
    }
    Ok((-1.0, Value::UNDEFINED))
}

fn find(agent: &mut Agent, c: Call) -> Res<Value> {
    find_via(agent, c, false, "%TypedArray%.prototype.find").map(|(_, v)| v)
}

fn find_index(agent: &mut Agent, c: Call) -> Res<Value> {
    find_via(agent, c, false, "%TypedArray%.prototype.findIndex").map(|(k, _)| Value::number(k))
}

fn find_last(agent: &mut Agent, c: Call) -> Res<Value> {
    find_via(agent, c, true, "%TypedArray%.prototype.findLast").map(|(_, v)| v)
}

fn find_last_index(agent: &mut Agent, c: Call) -> Res<Value> {
    find_via(agent, c, true, "%TypedArray%.prototype.findLastIndex").map(|(k, _)| Value::number(k))
}

/// The start of `includes` and `indexOf`: the first index to look at,
/// or `None` when there is none.
fn search_start(agent: &mut Agent, c: Call, len: usize) -> Res<Option<usize>> {
    if len == 0 {
        return Ok(None);
    }
    let n = agent.integer(agent.arg(c, 1))?;
    if n == f64::INFINITY {
        return Ok(None);
    }
    let k = if n >= 0.0 {
        n
    } else {
        (len as f64 + n).max(0.0)
    };
    Ok((k < len as f64).then_some(k as usize))
}

fn includes(agent: &mut Agent, c: Call) -> Res<Value> {
    let (o, len) = agent.this_view(c.this, "%TypedArray%.prototype.includes")?;
    let Some(start) = search_start(agent, c, len)? else {
        return Ok(Value::bool(false));
    };
    let search = agent.arg(c, 0);
    let s = search.as_number();
    for k in start..len {
        let found = match (agent.heap.view_get(o, k), s) {
            (Some(x), Some(s)) => x == s || (x.is_nan() && s.is_nan()),
            (None, _) => search.is_undefined(),
            _ => false,
        };
        if found {
            return Ok(Value::bool(true));
        }
    }
    Ok(Value::bool(false))
}

fn index_of(agent: &mut Agent, c: Call) -> Res<Value> {
    let (o, len) = agent.this_view(c.this, "%TypedArray%.prototype.indexOf")?;
    let Some(start) = search_start(agent, c, len)? else {
        return Ok(Value::int(-1));
    };
    if let Some(s) = agent.arg(c, 0).as_number() {
        for k in start..len {
            if agent.heap.view_get(o, k) == Some(s) {
                return Ok(Value::number(k as f64));
            }
        }
    }
    Ok(Value::int(-1))
}

fn last_index_of(agent: &mut Agent, c: Call) -> Res<Value> {
    let (o, len) = agent.this_view(c.this, "%TypedArray%.prototype.lastIndexOf")?;
    if len == 0 {
        return Ok(Value::int(-1));
    }
    let n = if c.argc > 1 {
        agent.integer(agent.arg(c, 1))?
    } else {
        len as f64 - 1.0
    };
    if n == f64::NEG_INFINITY {
        return Ok(Value::int(-1));
    }
    let k = if n >= 0.0 {
        n.min(len as f64 - 1.0)
    } else {
        len as f64 + n
    };
    if k < 0.0 {
        return Ok(Value::int(-1));
    }
    if let Some(s) = agent.arg(c, 0).as_number() {
        for k in (0..=k as usize).rev() {
            if agent.heap.view_get(o, k) == Some(s) {
                return Ok(Value::number(k as f64));
            }
        }
    }
    Ok(Value::int(-1))
}

/// `join` and `toLocaleString` (sections 23.2.3.18, 23.2.3.31).
fn join_with(agent: &mut Agent, c: Call, locale: bool) -> Res<Value> {
    let m = if locale {
        "%TypedArray%.prototype.toLocaleString"
    } else {
        "%TypedArray%.prototype.join"
    };
    let (o, len) = agent.this_view(c.this, m)?;
    let sep = agent.arg(c, 0);
    let sep = if locale || sep.is_undefined() {
        vec![u16::from(b',')]
    } else {
        agent.text_units(sep)?
    };
    let mut out = Vec::new();
    for k in 0..len {
        agent.tick()?;
        if k > 0 {
            out.extend_from_slice(&sep);
        }
        let Some(x) = agent.heap.view_get(o, k) else {
            continue;
        };
        if locale {
            let v = Value::number(x);
            let key = agent.name_key(Name::Str("toLocaleString"));
            let f = agent.get_value(v, key)?;
            let r = agent.call_value(f, v, &[])?;
            out.extend(agent.text_units(r)?);
        } else {
            out.extend(crate::number::number_to_string(x).encode_utf16());
        }
        if out.len() > super::string::MAX_LENGTH {
            return Err(agent.range_error("Invalid string length"));
        }
    }
    agent.new_string_units(&out)
}

fn join(agent: &mut Agent, c: Call) -> Res<Value> {
    join_with(agent, c, false)
}

fn to_locale_string(agent: &mut Agent, c: Call) -> Res<Value> {
    join_with(agent, c, true)
}

/// `%TypedArray%.prototype.map` (section 23.2.3.22).
fn map(agent: &mut Agent, c: Call) -> Res<Value> {
    const M: &str = "%TypedArray%.prototype.map";
    let (o, len) = agent.this_view(c.this, M)?;
    let f = agent.view_callback(agent.arg(c, 0))?;
    let this_arg = agent.arg(c, 1);
    let a = agent.species_view(o, &[Value::number(len as f64)], M)?;
    agent.vm.temps.push(object(a));
    let r = (|| {
        for k in 0..len {
            let v = agent.element(o, k);
            let mapped = agent.call_value(f, this_arg, &[v, Value::number(k as f64), object(o)])?;
            let key = agent.index_key(k as f64);
            agent.put(object(a), key, mapped)?;
        }
        Ok(object(a))
    })();
    agent.vm.temps.pop();
    r
}

fn reduce_with(agent: &mut Agent, c: Call, right: bool) -> Res<Value> {
    let m = if right {
        "%TypedArray%.prototype.reduceRight"
    } else {
        "%TypedArray%.prototype.reduce"
    };
    let (o, len) = agent.this_view(c.this, m)?;
    let f = agent.view_callback(agent.arg(c, 0))?;
    let index = |i: usize| if right { len - 1 - i } else { i };
    let mut i = 0;
    let mut acc = if c.argc >= 2 {
        agent.arg(c, 1)
    } else {
        if len == 0 {
            return Err(agent.type_error("Reduce of empty array with no initial value"));
        }
        i = 1;
        agent.element(o, index(0))
    };
    let base = agent.vm.temps.len();
    let r = (|| {
        while i < len {
            let k = index(i);
            let v = agent.element(o, k);
            agent.vm.temps.push(acc);
            acc = agent.call_value(
                f,
                Value::UNDEFINED,
                &[acc, v, Value::number(k as f64), object(o)],
            )?;
            agent.vm.temps.pop();
            i += 1;
        }
        Ok(acc)
    })();
    agent.vm.temps.truncate(base);
    r
}

fn reduce(agent: &mut Agent, c: Call) -> Res<Value> {
    reduce_with(agent, c, false)
}

fn reduce_right(agent: &mut Agent, c: Call) -> Res<Value> {
    reduce_with(agent, c, true)
}

/// The elements' bytes, `es` at a time, reversed.
fn reversed_bytes(bytes: &[u8], es: usize) -> Vec<u8> {
    bytes.rchunks(es).flatten().copied().collect()
}

fn reverse(agent: &mut Agent, c: Call) -> Res<Value> {
    let (o, len) = agent.this_view(c.this, "%TypedArray%.prototype.reverse")?;
    let es = agent.heap.view(o).map_or(1, |v| v.elem.size());
    let bytes = reversed_bytes(&agent.heap.view_bytes(o, 0, len), es);
    agent.heap.store_view_bytes(o, 0, &bytes);
    Ok(c.this)
}

fn to_reversed(agent: &mut Agent, c: Call) -> Res<Value> {
    let (o, len) = agent.this_view(c.this, "%TypedArray%.prototype.toReversed")?;
    let a = agent.view_same_type(o, len)?;
    let es = agent.heap.view(o).map_or(1, |v| v.elem.size());
    let bytes = reversed_bytes(&agent.heap.view_bytes(o, 0, len), es);
    agent.heap.store_view_bytes(a, 0, &bytes);
    Ok(object(a))
}

/// `%TypedArray%.prototype.set` (section 23.2.3.26).
fn set(agent: &mut Agent, c: Call) -> Res<Value> {
    const M: &str = "%TypedArray%.prototype.set";
    let Some(target) = c.this.as_object().filter(|&o| agent.heap.view(o).is_some()) else {
        return Err(agent.type_error("this is not a typed array."));
    };
    let offset = agent.integer(agent.arg(c, 1))?;
    if offset < 0.0 {
        return Err(agent.range_error("offset is out of bounds"));
    }
    let Some(target_len) = agent.heap.view_length(target) else {
        return Err(agent.out_of_bounds(M));
    };
    let source = agent.arg(c, 0);
    if let Some(src) = source.as_object().filter(|&o| agent.heap.view(o).is_some()) {
        let Some(src_len) = agent.heap.view_length(src) else {
            return Err(agent.out_of_bounds(M));
        };
        if offset + src_len as f64 > target_len as f64 {
            return Err(agent.range_error("offset is out of bounds"));
        }
        let at = offset as usize;
        let same = agent.heap.view(src).map(|v| v.elem) == agent.heap.view(target).map(|v| v.elem);
        if same {
            let bytes = agent.heap.view_bytes(src, 0, src_len);
            agent.heap.store_view_bytes(target, at, &bytes);
        } else {
            let values: Vec<f64> = (0..src_len)
                .map(|k| agent.heap.view_get(src, k).unwrap_or(f64::NAN))
                .collect();
            for (k, n) in values.into_iter().enumerate() {
                agent.heap.view_set(target, at + k, n);
            }
        }
        return Ok(Value::UNDEFINED);
    }
    let src = agent.object_of(source)?;
    agent.vm.temps.push(src);
    let r = (|| {
        let src_len = agent.length_of(src)?;
        if offset + src_len > target_len as f64 {
            return Err(agent.range_error("offset is out of bounds"));
        }
        let at = offset as usize;
        for k in 0..src_len as usize {
            let key = agent.index_key(k as f64);
            let v = agent.get_value(src, key)?;
            agent.view_put(target, at + k, v)?;
        }
        Ok(Value::UNDEFINED)
    })();
    agent.vm.temps.pop();
    r
}

/// `%TypedArray%.prototype.slice` (section 23.2.3.27).
fn slice(agent: &mut Agent, c: Call) -> Res<Value> {
    const M: &str = "%TypedArray%.prototype.slice";
    let (o, len) = agent.this_view(c.this, M)?;
    let lenf = len as f64;
    let k = agent.relative(agent.arg(c, 0), lenf, 0.0)?;
    let end = agent.relative(agent.arg(c, 1), lenf, lenf)?;
    let count = (end - k).max(0.0);
    let a = agent.species_view(o, &[Value::number(count)], M)?;
    if count > 0.0 {
        let Some(len) = agent.heap.view_length(o) else {
            return Err(agent.out_of_bounds(M));
        };
        let (k, end) = (k as usize, (end as usize).min(len));
        let count = end.saturating_sub(k);
        let (src, dst) = (agent.heap.view(o), agent.heap.view(a));
        if let (Some(s), Some(d)) = (src, dst)
            && s.elem == d.elem
        {
            if s.buffer == d.buffer {
                let es = s.elem.size();
                agent.heap.move_bytes(
                    s.buffer,
                    (s.offset + k * es, d.offset),
                    count * es,
                    usize::MAX,
                    false,
                );
            } else {
                agent.copy_elements(o, k, a, count);
            }
        } else {
            for (n, i) in (k..end).enumerate() {
                if let Some(x) = agent.heap.view_get(o, i) {
                    agent.heap.view_set(a, n, x);
                }
            }
        }
    }
    Ok(object(a))
}

/// `%TypedArray%.prototype.sort` (section 23.2.3.29).
fn sort(agent: &mut Agent, c: Call) -> Res<Value> {
    let f = agent.comparator_of(agent.arg(c, 0))?;
    let (o, len) = agent.this_view(c.this, "%TypedArray%.prototype.sort")?;
    let mut values: Vec<f64> = (0..len)
        .map(|k| agent.heap.view_get(o, k).unwrap_or(f64::NAN))
        .collect();
    agent.sort_numbers(&mut values, f)?;
    for (k, n) in values.into_iter().enumerate() {
        agent.heap.view_set(o, k, n);
    }
    Ok(c.this)
}

fn to_sorted(agent: &mut Agent, c: Call) -> Res<Value> {
    let f = agent.comparator_of(agent.arg(c, 0))?;
    let (o, len) = agent.this_view(c.this, "%TypedArray%.prototype.toSorted")?;
    let a = agent.view_same_type(o, len)?;
    agent.vm.temps.push(object(a));
    let mut values: Vec<f64> = (0..len)
        .map(|k| agent.heap.view_get(o, k).unwrap_or(f64::NAN))
        .collect();
    let r = agent.sort_numbers(&mut values, f);
    agent.vm.temps.pop();
    r?;
    for (k, n) in values.into_iter().enumerate() {
        agent.heap.view_set(a, k, n);
    }
    Ok(object(a))
}

/// `%TypedArray%.prototype.subarray` (section 23.2.3.30).
fn subarray(agent: &mut Agent, c: Call) -> Res<Value> {
    const M: &str = "%TypedArray%.prototype.subarray";
    let Some(o) = c.this.as_object().filter(|&o| agent.heap.view(o).is_some()) else {
        return Err(agent.type_error("this is not a typed array."));
    };
    let Some(v) = agent.heap.view(o) else {
        return Err(agent.type_error("this is not a typed array."));
    };
    let src_len = agent.heap.view_length(o).unwrap_or(0) as f64;
    let begin = agent.relative(agent.arg(c, 0), src_len, 0.0)?;
    let end_arg = agent.arg(c, 1);
    let begin_byte = Value::number((v.offset + begin as usize * v.elem.size()) as f64);
    let buf = object(v.buffer);
    let a = if v.length.is_none() && end_arg.is_undefined() {
        agent.species_view(o, &[buf, begin_byte], M)?
    } else {
        let end = agent.relative(end_arg, src_len, src_len)?;
        let new_len = Value::number((end - begin).max(0.0));
        agent.species_view(o, &[buf, begin_byte, new_len], M)?
    };
    Ok(object(a))
}

/// `%TypedArray%.prototype.with` (section 23.2.3.36).
fn with(agent: &mut Agent, c: Call) -> Res<Value> {
    let (o, len) = agent.this_view(c.this, "%TypedArray%.prototype.with")?;
    let rel = agent.integer(agent.arg(c, 0))?;
    let actual = if rel >= 0.0 { rel } else { len as f64 + rel };
    let n = agent.number(agent.arg(c, 1))?;
    if actual < 0.0 || !agent.heap.view_valid(o, Some(actual as usize)) {
        return Err(agent.range_error("Invalid typed array index"));
    }
    let actual = actual as usize;
    let a = agent.view_same_type(o, len)?;
    for k in 0..len {
        let x = if k == actual {
            Some(n)
        } else {
            agent.heap.view_get(o, k)
        };
        agent.heap.view_set(a, k, x.unwrap_or(f64::NAN));
    }
    Ok(object(a))
}
