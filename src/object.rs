//! Objects (docs/design.md section 1.3), in dictionary mode: shapes and
//! inline caches are J6's. The ordinary internal methods (ECMA-262
//! section 10.1) and the Array exotic object's `length` (section 10.4.2).
//!
//! An object cell is small: its prototype, a vector of named properties
//! (key, value, attributes: 24 bytes each), and a box of what few objects
//! need (indexed elements, accessor pairs, a hash index once the
//! properties are many, an array's length). Indexed properties live in
//! the elements: packed, holey (the hole marks a missing one) or sparse
//! (a sorted map, for arrays mostly holes and for elements with other
//! than the default attributes).
//!
//! `[[Get]]` and `[[Set]]` do not call accessors: they return the getter
//! or setter for the caller (the interpreter, J2.4) to call, so that
//! nothing here runs script or collects.

use std::collections::{BTreeMap, HashMap};

use crate::heap::{Heap, Trace};
use crate::internal::InternalCell;
use crate::string::PropertyKey;
use crate::value::{RefKind, Value};

pub(crate) const WRITABLE: u8 = 1;
pub(crate) const ENUMERABLE: u8 = 2;
pub(crate) const CONFIGURABLE: u8 = 4;
/// The entry's value is an index into the object's accessor pairs.
const ACCESSOR: u8 = 8;
/// Writable, enumerable and configurable: what a plain assignment makes.
pub(crate) const DEFAULT: u8 = WRITABLE | ENUMERABLE | CONFIGURABLE;

/// Past this many named properties an object keeps a hash index.
const INDEXED_PAST: usize = 8;
/// A holey array whose gap would pass this many holes goes sparse.
const SPARSE_GAP: usize = 1024;

#[derive(Debug, Clone, Copy)]
struct Entry {
    key: PropertyKey,
    value: Value,
    flags: u8,
}

#[derive(Debug, Clone, Copy)]
struct Accessor {
    get: Value,
    set: Value,
}

/// Indexed properties.
#[derive(Debug, Clone)]
enum Elements {
    /// No holes, every element writable, enumerable and configurable.
    Packed(Vec<Value>),
    /// The same with holes ([`Value::HOLE`]).
    Holey(Vec<Value>),
    /// Index to value (or accessor pair index) and attributes.
    Sparse(BTreeMap<u32, (Value, u8)>),
}

/// What a function object runs when called.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Callable {
    /// Function `func` of the agent's installed unit `unit`, over the
    /// environment it was made in (undefined: the global one).
    Code { unit: u32, func: u32, env: Value },
    /// The native `id` of the built-ins' tables ([`crate::builtins`]),
    /// made in the agent's realm `realm`.
    Native { id: u32, realm: u32 },
    /// A bound function (section 10.4.1): its target, `this` and leading
    /// arguments are the object's internal slots.
    Bound,
}

#[derive(Debug, Clone)]
struct Extra {
    elements: Elements,
    accessors: Vec<Accessor>,
    index: Option<HashMap<PropertyKey, u32>>,
    /// An array's length, and whether `length` is writable.
    length: u32,
    length_writable: bool,
    /// A function's code.
    function: Option<Callable>,
    /// Internal slots, by kind: a wrapper's primitive; a bound
    /// function's target, `this` and arguments; a mapped arguments
    /// object's environment and, per index, the slot it is mapped to
    /// (an int) or a hole.
    internal: Vec<Value>,
}

impl Default for Extra {
    fn default() -> Extra {
        Extra {
            elements: Elements::Packed(Vec::new()),
            accessors: Vec::new(),
            index: None,
            length: 0,
            length_writable: true,
            function: None,
            internal: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ObjectKind {
    Ordinary,
    Array,
    Function,
    /// Made by an error constructor (`[[ErrorData]]`).
    Error,
    /// An arguments object: mapped (section 10.4.4) when it has an
    /// environment in its internal slots.
    Arguments,
    /// The wrappers of primitives (`[[BooleanData]]` and the like), the
    /// primitive the first internal slot; a `String` one is exotic
    /// (section 10.4.3).
    Boolean,
    Number,
    String,
    Symbol,
    /// An array iterator: the array-like (undefined once done), the next
    /// index and what it gives, in its internal slots.
    ArrayIterator,
    /// A string iterator: the string (undefined once done) and the next
    /// position.
    StringIterator,
}

/// An object cell.
#[derive(Debug, Clone)]
pub(crate) struct ObjectCell {
    proto: Value,
    props: Vec<Entry>,
    extra: Option<Box<Extra>>,
    kind: ObjectKind,
    extensible: bool,
}

/// A property as its object holds it (section 6.2.6's fully populated
/// Property Descriptor).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OwnProperty {
    Data {
        value: Value,
        writable: bool,
        enumerable: bool,
        configurable: bool,
    },
    Accessor {
        get: Value,
        set: Value,
        enumerable: bool,
        configurable: bool,
    },
}

impl OwnProperty {
    fn enumerable(&self) -> bool {
        match *self {
            OwnProperty::Data { enumerable, .. } | OwnProperty::Accessor { enumerable, .. } => {
                enumerable
            }
        }
    }

    fn configurable(&self) -> bool {
        match *self {
            OwnProperty::Data { configurable, .. } | OwnProperty::Accessor { configurable, .. } => {
                configurable
            }
        }
    }
}

/// A Property Descriptor as `[[DefineOwnProperty]]` takes it: any field
/// may be absent.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Descriptor {
    pub value: Option<Value>,
    pub get: Option<Value>,
    pub set: Option<Value>,
    pub writable: Option<bool>,
    pub enumerable: Option<bool>,
    pub configurable: Option<bool>,
}

impl Descriptor {
    /// A data property writable, enumerable and configurable.
    pub fn data(value: Value) -> Descriptor {
        Descriptor {
            value: Some(value),
            writable: Some(true),
            enumerable: Some(true),
            configurable: Some(true),
            ..Descriptor::default()
        }
    }

    fn is_accessor(&self) -> bool {
        self.get.is_some() || self.set.is_some()
    }

    fn is_data(&self) -> bool {
        self.value.is_some() || self.writable.is_some()
    }

    fn is_empty(&self) -> bool {
        *self == Descriptor::default()
    }
}

/// What `[[Get]]` found.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Got {
    Value(Value),
    /// An accessor: call this getter (undefined: the result is
    /// undefined) with the receiver.
    Getter(Value),
}

/// What `[[Set]]` did.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SetOutcome {
    /// Done; false where strict code throws.
    Done(bool),
    /// An accessor: call this setter with the receiver and the value.
    Setter(Value),
}

impl ObjectCell {
    pub(crate) fn new(proto: Value, kind: ObjectKind) -> ObjectCell {
        ObjectCell {
            proto,
            props: Vec::new(),
            extra: (kind == ObjectKind::Array).then(Box::default),
            kind,
            extensible: true,
        }
    }

    /// A function object: `callable` is what calling it runs.
    pub(crate) fn function(proto: Value, callable: Callable) -> ObjectCell {
        let mut cell = ObjectCell::new(proto, ObjectKind::Function);
        cell.extra_mut().function = Some(callable);
        cell
    }

    /// A bound function: its target, `this` and leading arguments.
    pub(crate) fn bound(proto: Value, internal: Vec<Value>) -> ObjectCell {
        let mut cell = ObjectCell::function(proto, Callable::Bound);
        cell.extra_mut().internal = internal;
        cell
    }

    /// An object of `kind` with internal slots.
    pub(crate) fn with_internal(
        proto: Value,
        kind: ObjectKind,
        internal: Vec<Value>,
    ) -> ObjectCell {
        let mut cell = ObjectCell::new(proto, kind);
        cell.extra_mut().internal = internal;
        cell
    }

    /// Its internal slots.
    pub(crate) fn internal(&self) -> &[Value] {
        self.extra.as_ref().map_or(&[], |e| &e.internal)
    }

    /// Sets internal slot `i`, if it has one.
    pub(crate) fn set_internal(&mut self, i: usize, v: Value) {
        if let Some(slot) = self.extra.as_mut().and_then(|e| e.internal.get_mut(i)) {
            *slot = v;
        }
    }

    pub(crate) fn kind(&self) -> ObjectKind {
        self.kind
    }

    /// What calling the object runs, if it is a function.
    pub(crate) fn callable(&self) -> Option<Callable> {
        self.extra.as_ref()?.function
    }

    fn extra_mut(&mut self) -> &mut Extra {
        self.extra.get_or_insert_with(Box::default)
    }

    fn find(&self, key: PropertyKey) -> Option<usize> {
        if let Some(index) = self.extra.as_ref().and_then(|e| e.index.as_ref()) {
            return index.get(&key).map(|&i| i as usize);
        }
        self.props.iter().position(|e| e.key == key)
    }

    fn reindex(&mut self) {
        if self.props.len() > INDEXED_PAST {
            let index = self
                .props
                .iter()
                .enumerate()
                .map(|(i, e)| (e.key, u32::try_from(i).unwrap_or(u32::MAX)))
                .collect();
            self.extra_mut().index = Some(index);
        } else if let Some(extra) = self.extra.as_mut() {
            extra.index = None;
        }
    }

    fn accessor(&self, slot: Value) -> Accessor {
        let i = slot.as_int().unwrap_or(0) as usize;
        self.extra
            .as_ref()
            .and_then(|e| e.accessors.get(i))
            .copied()
            .unwrap_or(Accessor {
                get: Value::UNDEFINED,
                set: Value::UNDEFINED,
            })
    }

    fn add_accessor(&mut self, pair: Accessor) -> Value {
        let accessors = &mut self.extra_mut().accessors;
        accessors.push(pair);
        Value::int(i32::try_from(accessors.len() - 1).unwrap_or(i32::MAX))
    }

    fn to_property(&self, value: Value, flags: u8) -> OwnProperty {
        if flags & ACCESSOR != 0 {
            let pair = self.accessor(value);
            OwnProperty::Accessor {
                get: pair.get,
                set: pair.set,
                enumerable: flags & ENUMERABLE != 0,
                configurable: flags & CONFIGURABLE != 0,
            }
        } else {
            OwnProperty::Data {
                value,
                writable: flags & WRITABLE != 0,
                enumerable: flags & ENUMERABLE != 0,
                configurable: flags & CONFIGURABLE != 0,
            }
        }
    }

    fn element(&self, i: u32) -> Option<(Value, u8)> {
        match &self.extra.as_ref()?.elements {
            Elements::Packed(v) | Elements::Holey(v) => v
                .get(i as usize)
                .filter(|v| **v != Value::HOLE)
                .map(|&v| (v, DEFAULT)),
            Elements::Sparse(map) => map.get(&i).copied(),
        }
    }

    /// Writes `value` over an own writable data property, if `key` names
    /// one: what `[[Set]]` comes to when the receiver holds the property
    /// itself, without its general path. False, and nothing written,
    /// otherwise.
    fn write_own(&mut self, key: PropertyKey, value: Value) -> bool {
        match key {
            PropertyKey::Index(i) => match self.extra.as_mut().map(|e| &mut e.elements) {
                Some(Elements::Packed(v) | Elements::Holey(v)) => match v.get_mut(i as usize) {
                    Some(slot) if *slot != Value::HOLE => {
                        *slot = value;
                        true
                    }
                    _ => false,
                },
                Some(Elements::Sparse(map)) => match map.get_mut(&i) {
                    Some((v, flags)) if *flags & (WRITABLE | ACCESSOR) == WRITABLE => {
                        *v = value;
                        true
                    }
                    _ => false,
                },
                None => false,
            },
            _ => match self.find(key).and_then(|at| self.props.get_mut(at)) {
                Some(e) if e.flags & (WRITABLE | ACCESSOR) == WRITABLE => {
                    e.value = value;
                    true
                }
                _ => false,
            },
        }
    }

    /// `[[GetOwnProperty]]` (section 10.1.5).
    pub(crate) fn own_property(&self, key: PropertyKey) -> Option<OwnProperty> {
        match key {
            PropertyKey::Index(i) => {
                let (value, flags) = self.element(i)?;
                Some(self.to_property(value, flags))
            }
            _ => {
                let entry = self.props.get(self.find(key)?)?;
                Some(self.to_property(entry.value, entry.flags))
            }
        }
    }

    /// An array's own `length` property.
    pub(crate) fn array_length(&self) -> Option<(u32, bool)> {
        let extra = self.extra.as_ref()?;
        (self.kind == ObjectKind::Array).then_some((extra.length, extra.length_writable))
    }

    /// Writes an element of default attributes, keeping the elements
    /// packed or holey while they stay dense.
    fn put_element(&mut self, i: u32, value: Value) {
        let extra = self.extra_mut();
        let index = i as usize;
        match &mut extra.elements {
            Elements::Packed(v) if index < v.len() => {
                if let Some(slot) = v.get_mut(index) {
                    *slot = value;
                }
            }
            Elements::Packed(v) if index == v.len() => v.push(value),
            Elements::Holey(v) if index < v.len() => {
                if let Some(slot) = v.get_mut(index) {
                    *slot = value;
                }
            }
            Elements::Packed(v) | Elements::Holey(v) if index - v.len() <= SPARSE_GAP => {
                v.resize(index, Value::HOLE);
                v.push(value);
                if let Elements::Packed(v) = &mut extra.elements {
                    let taken = std::mem::take(v);
                    extra.elements = Elements::Holey(taken);
                }
            }
            Elements::Sparse(map) => {
                map.insert(i, (value, DEFAULT));
            }
            _ => {
                Self::make_sparse(extra);
                if let Elements::Sparse(map) = &mut extra.elements {
                    map.insert(i, (value, DEFAULT));
                }
            }
        }
    }

    fn make_sparse(extra: &mut Extra) {
        if let Elements::Packed(v) | Elements::Holey(v) = &mut extra.elements {
            let map = v
                .iter()
                .enumerate()
                .filter(|(_, v)| **v != Value::HOLE)
                .map(|(i, &v)| (u32::try_from(i).unwrap_or(u32::MAX), (v, DEFAULT)))
                .collect();
            extra.elements = Elements::Sparse(map);
        }
    }

    fn put_element_flags(&mut self, i: u32, value: Value, flags: u8) {
        if flags == DEFAULT {
            self.put_element(i, value);
            return;
        }
        let extra = self.extra_mut();
        Self::make_sparse(extra);
        if let Elements::Sparse(map) = &mut extra.elements {
            map.insert(i, (value, flags));
        }
    }

    fn remove_element(&mut self, i: u32) {
        let Some(extra) = self.extra.as_mut() else {
            return;
        };
        match &mut extra.elements {
            Elements::Packed(v) | Elements::Holey(v) => {
                let index = i as usize;
                if index + 1 == v.len() {
                    v.pop();
                } else if let Some(slot) = v.get_mut(index) {
                    *slot = Value::HOLE;
                    if let Elements::Packed(v) = &mut extra.elements {
                        let taken = std::mem::take(v);
                        extra.elements = Elements::Holey(taken);
                    }
                }
            }
            Elements::Sparse(map) => {
                map.remove(&i);
            }
        }
    }

    /// Writes `prop` as the own property `key`, replacing any.
    fn put(&mut self, key: PropertyKey, prop: OwnProperty) {
        let (value, flags) = match prop {
            OwnProperty::Data {
                value,
                writable,
                enumerable,
                configurable,
            } => (
                value,
                (u8::from(writable) * WRITABLE)
                    | (u8::from(enumerable) * ENUMERABLE)
                    | (u8::from(configurable) * CONFIGURABLE),
            ),
            OwnProperty::Accessor {
                get,
                set,
                enumerable,
                configurable,
            } => {
                // An accessor redefined keeps its pair's slot.
                let existing = match key {
                    PropertyKey::Index(i) => self.element(i),
                    _ => self
                        .find(key)
                        .and_then(|at| self.props.get(at))
                        .map(|e| (e.value, e.flags)),
                };
                let slot = match existing {
                    Some((slot, flags)) if flags & ACCESSOR != 0 => {
                        let i = slot.as_int().unwrap_or(0) as usize;
                        if let Some(pair) = self.extra_mut().accessors.get_mut(i) {
                            *pair = Accessor { get, set };
                        }
                        slot
                    }
                    _ => self.add_accessor(Accessor { get, set }),
                };
                (
                    slot,
                    ACCESSOR
                        | (u8::from(enumerable) * ENUMERABLE)
                        | (u8::from(configurable) * CONFIGURABLE),
                )
            }
        };
        match key {
            PropertyKey::Index(i) => self.put_element_flags(i, value, flags),
            _ => match self.find(key) {
                Some(at) => {
                    if let Some(entry) = self.props.get_mut(at) {
                        entry.value = value;
                        entry.flags = flags;
                    }
                }
                None => {
                    // Capacity 1, 2, 4, 8...: a Vec's own first growth
                    // reserves four entries (96 bytes), more than most
                    // objects' two or three properties want.
                    if self.props.len() == self.props.capacity() {
                        self.props.reserve_exact(self.props.len().max(1));
                    }
                    self.props.push(Entry { key, value, flags });
                    if let Some(index) = self.extra.as_mut().and_then(|e| e.index.as_mut()) {
                        index.insert(key, u32::try_from(self.props.len() - 1).unwrap_or(0));
                    } else if self.props.len() > INDEXED_PAST {
                        self.reindex();
                    }
                }
            },
        }
    }

    /// Removes the own property `key`.
    fn remove(&mut self, key: PropertyKey) {
        match key {
            PropertyKey::Index(i) => self.remove_element(i),
            _ => {
                if let Some(at) = self.find(key) {
                    self.props.remove(at);
                    self.reindex();
                }
            }
        }
    }

    /// `[[OwnPropertyKeys]]` (section 10.1.11): indices ascending, then
    /// names and then symbols, each in creation order; and an array's
    /// `length`, as `length_key`.
    pub(crate) fn own_keys(&self, length_key: PropertyKey) -> Vec<PropertyKey> {
        let mut keys = Vec::new();
        if let Some(extra) = &self.extra {
            match &extra.elements {
                Elements::Packed(v) | Elements::Holey(v) => keys.extend(
                    v.iter()
                        .enumerate()
                        .filter(|(_, v)| **v != Value::HOLE)
                        .map(|(i, _)| PropertyKey::Index(u32::try_from(i).unwrap_or(0))),
                ),
                Elements::Sparse(map) => keys.extend(map.keys().map(|&i| PropertyKey::Index(i))),
            }
        }
        if self.kind == ObjectKind::Array {
            keys.push(length_key);
        }
        keys.extend(
            self.props
                .iter()
                .filter(|e| matches!(e.key, PropertyKey::Atom(_)))
                .map(|e| e.key),
        );
        keys.extend(
            self.props
                .iter()
                .filter(|e| matches!(e.key, PropertyKey::Symbol(_)))
                .map(|e| e.key),
        );
        keys
    }

    pub(crate) fn proto(&self) -> Value {
        self.proto
    }

    /// The highest index below `limit` the elements hold (a hole of a
    /// holey vector counts: removing it is nothing).
    fn highest_index_below(&self, limit: u32) -> Option<u32> {
        match &self.extra.as_ref()?.elements {
            Elements::Packed(v) | Elements::Holey(v) => {
                let len = u32::try_from(v.len()).unwrap_or(u32::MAX).min(limit);
                len.checked_sub(1)
            }
            Elements::Sparse(map) => map.range(..limit).next_back().map(|(&i, _)| i),
        }
    }

    pub(crate) fn is_extensible(&self) -> bool {
        self.extensible
    }
}

impl Trace for ObjectCell {
    fn trace(&self, out: &mut Vec<Value>) {
        out.push(self.proto);
        for e in &self.props {
            if e.flags & ACCESSOR == 0 {
                out.push(e.value);
            }
            if let PropertyKey::Symbol(s) = e.key {
                out.push(Value::reference(RefKind::Symbol, s));
            }
        }
        if let Some(extra) = &self.extra {
            if let Some(Callable::Code { env, .. }) = extra.function {
                out.push(env);
            }
            out.extend(extra.internal.iter().filter(|v| **v != Value::HOLE));
            for pair in &extra.accessors {
                out.push(pair.get);
                out.push(pair.set);
            }
            match &extra.elements {
                Elements::Packed(v) | Elements::Holey(v) => out.extend_from_slice(v),
                Elements::Sparse(map) => out.extend(
                    map.values()
                        .filter(|(_, flags)| flags & ACCESSOR == 0)
                        .map(|(v, _)| *v),
                ),
            }
        }
    }

    fn extra_bytes(&self) -> usize {
        let props = self.props.capacity() * std::mem::size_of::<Entry>();
        let extra = self.extra.as_ref().map_or(0, |e| {
            let elements = match &e.elements {
                Elements::Packed(v) | Elements::Holey(v) => v.capacity() * 8,
                Elements::Sparse(map) => map.len() * 48,
            };
            std::mem::size_of::<Extra>()
                + elements
                + e.accessors.capacity() * std::mem::size_of::<Accessor>()
                + e.index.as_ref().map_or(0, |m| m.capacity() * 24)
                + e.internal.capacity() * 8
        });
        props + extra
    }
}

/// SameValue (section 7.2.10): numbers by value, NaN equal to itself and
/// the zeros told apart; strings by their units; the rest by identity.
pub(crate) fn same_value(heap: &Heap, a: Value, b: Value) -> bool {
    if let (Some(x), Some(y)) = (a.as_number(), b.as_number()) {
        if x.is_nan() && y.is_nan() {
            return true;
        }
        return x == y && x.is_sign_negative() == y.is_sign_negative();
    }
    if let (Some(x), Some(y)) = (a.as_string(), b.as_string()) {
        return match (heap.strings.get(x), heap.strings.get(y)) {
            (Some(x), Some(y)) => x.same_units(y),
            _ => false,
        };
    }
    a == b
}

/// Whether defining `desc` over the non-configurable data property
/// `current` would be allowed, with nothing to change
/// (ValidateAndApplyPropertyDescriptor's checks, section 10.1.6.3).
fn compatible(heap: &Heap, current: OwnProperty, desc: &Descriptor) -> bool {
    let OwnProperty::Data {
        value,
        writable,
        enumerable,
        ..
    } = current
    else {
        return false;
    };
    !(desc.configurable == Some(true)
        || desc.enumerable.is_some_and(|e| e != enumerable)
        || desc.is_accessor()
        || (!writable && desc.writable == Some(true))
        || desc.value.is_some_and(|v| !same_value(heap, v, value)))
}

/// The object operations, over the heap's objects named by index.
impl Heap {
    fn object(&self, o: u32) -> Option<&ObjectCell> {
        self.objects.get(o)
    }

    /// Runs `f` on the object `o` and keeps the heap's count of its
    /// bytes right.
    fn with_object<R>(&mut self, o: u32, f: impl FnOnce(&mut ObjectCell) -> R) -> Option<R> {
        let cell = self.objects.get_mut(o)?;
        let before = cell.extra_bytes();
        let result = f(cell);
        let after = cell.extra_bytes();
        self.objects.update_extra(before, after);
        Some(result)
    }

    /// `[[GetOwnProperty]]`, an array's `length` included, a `String`
    /// wrapper's indices and a mapped arguments object's too.
    pub(crate) fn get_own_property(&self, o: u32, key: PropertyKey) -> Option<OwnProperty> {
        let cell = self.object(o)?;
        match cell.kind {
            ObjectKind::Array if key == self.length_key => {
                let (length, writable) = cell.array_length()?;
                return Some(OwnProperty::Data {
                    value: Value::number(f64::from(length)),
                    writable,
                    enumerable: false,
                    configurable: false,
                });
            }
            ObjectKind::String => {
                if let Some(p) = self.string_own(cell, key) {
                    return Some(p);
                }
            }
            ObjectKind::Arguments => {
                let mut p = cell.own_property(key)?;
                if let (Some(v), OwnProperty::Data { value, .. }) = (self.mapped(cell, key), &mut p)
                {
                    *value = v;
                }
                return Some(p);
            }
            _ => {}
        }
        cell.own_property(key)
    }

    /// A `String` wrapper's own `length` and indices (section 10.4.3.5),
    /// the one-unit strings read from the heap's table of them (which
    /// the wrapper filled when it was made).
    fn string_own(&self, cell: &ObjectCell, key: PropertyKey) -> Option<OwnProperty> {
        let s = self.strings.get(cell.internal().first()?.as_string()?)?;
        let value = if key == self.length_key {
            Value::int(i32::try_from(s.len()).unwrap_or(i32::MAX))
        } else {
            let PropertyKey::Index(i) = key else {
                return None;
            };
            let unit = s.unit(i as usize)?;
            *self.unit_strings.get(&unit)?
        };
        Some(OwnProperty::Data {
            value,
            writable: false,
            enumerable: key != self.length_key,
            configurable: false,
        })
    }

    /// The value a mapped arguments object's index reads from its
    /// environment, if the index is mapped.
    fn mapped(&self, cell: &ObjectCell, key: PropertyKey) -> Option<Value> {
        let (env, slot) = Self::mapping(cell, key)?;
        match self.internals.get(env)? {
            InternalCell::Env(e) => e.slots.get(slot).copied(),
            InternalCell::ForIn(_) | InternalCell::Stack(_) => None,
        }
    }

    fn mapping(cell: &ObjectCell, key: PropertyKey) -> Option<(u32, usize)> {
        let PropertyKey::Index(i) = key else {
            return None;
        };
        let internal = cell.internal();
        let (_, env) = internal.first()?.as_reference()?;
        let slot = internal.get(1 + i as usize)?.as_int()?;
        Some((env, usize::try_from(slot).ok()?))
    }

    /// Writes a mapped index's environment slot.
    fn set_mapped(&mut self, o: u32, key: PropertyKey, v: Value) {
        let Some((env, slot)) = self.object(o).and_then(|c| Self::mapping(c, key)) else {
            return;
        };
        if let Some(InternalCell::Env(e)) = self.internals.get_mut(env)
            && let Some(s) = e.slots.get_mut(slot)
        {
            *s = v;
        }
    }

    /// Ends the mapping of an arguments object's index.
    fn unmap(&mut self, o: u32, key: PropertyKey) {
        if let PropertyKey::Index(i) = key {
            self.with_object(o, |c| c.set_internal(1 + i as usize, Value::HOLE));
        }
    }

    /// Whether `o` is a mapped arguments object mapping `key`.
    fn is_mapped(&self, o: u32, key: PropertyKey) -> bool {
        self.object(o)
            .is_some_and(|c| c.kind == ObjectKind::Arguments && Self::mapping(c, key).is_some())
    }

    /// `[[DefineOwnProperty]]` (sections 10.1.6 and 10.4.2.1).
    pub(crate) fn define_own_property(
        &mut self,
        o: u32,
        key: PropertyKey,
        desc: Descriptor,
    ) -> bool {
        let Some(kind) = self.object(o).map(ObjectCell::kind) else {
            return false;
        };
        if kind == ObjectKind::String
            && let Some(cell) = self.object(o)
            && let Some(current) = self.string_own(cell, key)
        {
            // Never written: whether `desc` agrees with what is there.
            return compatible(self, current, &desc);
        }
        if kind == ObjectKind::Arguments && self.is_mapped(o, key) {
            return self.arguments_define(o, key, desc);
        }
        if kind == ObjectKind::Array {
            if key == self.length_key {
                return self.array_set_length(o, desc);
            }
            if let PropertyKey::Index(i) = key {
                let Some((length, length_writable)) =
                    self.object(o).and_then(ObjectCell::array_length)
                else {
                    return false;
                };
                if i >= length && !length_writable {
                    return false;
                }
                if !self.ordinary_define(o, key, desc) {
                    return false;
                }
                if i >= length {
                    self.with_object(o, |c| c.extra_mut().length = i + 1);
                }
                return true;
            }
        }
        self.ordinary_define(o, key, desc)
    }

    /// A mapped arguments object's `[[DefineOwnProperty]]` of a mapped
    /// index (section 10.4.4.2).
    fn arguments_define(&mut self, o: u32, key: PropertyKey, desc: Descriptor) -> bool {
        let mut arg_desc = desc;
        if desc.is_data() && desc.value.is_none() && desc.writable == Some(false) {
            arg_desc.value = self.object(o).and_then(|c| self.mapped(c, key));
        }
        if !self.ordinary_define(o, key, arg_desc) {
            return false;
        }
        if desc.is_accessor() {
            self.unmap(o, key);
        } else {
            if let Some(v) = desc.value {
                self.set_mapped(o, key, v);
            }
            if desc.writable == Some(false) {
                self.unmap(o, key);
            }
        }
        true
    }

    /// OrdinaryDefineOwnProperty with ValidateAndApplyPropertyDescriptor
    /// (section 10.1.6.3).
    fn ordinary_define(&mut self, o: u32, key: PropertyKey, desc: Descriptor) -> bool {
        let current = self.get_own_property(o, key);
        let Some(cell) = self.object(o) else {
            return false;
        };
        let Some(current) = current else {
            if !cell.is_extensible() {
                return false;
            }
            let prop = if desc.is_accessor() {
                OwnProperty::Accessor {
                    get: desc.get.unwrap_or(Value::UNDEFINED),
                    set: desc.set.unwrap_or(Value::UNDEFINED),
                    enumerable: desc.enumerable.unwrap_or(false),
                    configurable: desc.configurable.unwrap_or(false),
                }
            } else {
                OwnProperty::Data {
                    value: desc.value.unwrap_or(Value::UNDEFINED),
                    writable: desc.writable.unwrap_or(false),
                    enumerable: desc.enumerable.unwrap_or(false),
                    configurable: desc.configurable.unwrap_or(false),
                }
            };
            self.with_object(o, |c| c.put(key, prop));
            return true;
        };
        if desc.is_empty() {
            return true;
        }
        if !current.configurable() {
            if desc.configurable == Some(true) {
                return false;
            }
            if desc.enumerable.is_some_and(|e| e != current.enumerable()) {
                return false;
            }
            let current_accessor = matches!(current, OwnProperty::Accessor { .. });
            if (desc.is_accessor() || desc.is_data()) && desc.is_accessor() != current_accessor {
                return false;
            }
            match current {
                OwnProperty::Accessor { get, set, .. } => {
                    if desc.get.is_some_and(|g| !same_value(self, g, get))
                        || desc.set.is_some_and(|s| !same_value(self, s, set))
                    {
                        return false;
                    }
                }
                OwnProperty::Data {
                    value, writable, ..
                } => {
                    if !writable
                        && (desc.writable == Some(true)
                            || desc.value.is_some_and(|v| !same_value(self, v, value)))
                    {
                        return false;
                    }
                }
            }
        }
        let enumerable = desc.enumerable.unwrap_or(current.enumerable());
        let configurable = desc.configurable.unwrap_or(current.configurable());
        let next = match current {
            OwnProperty::Data {
                value, writable, ..
            } if !desc.is_accessor() => OwnProperty::Data {
                value: desc.value.unwrap_or(value),
                writable: desc.writable.unwrap_or(writable),
                enumerable,
                configurable,
            },
            OwnProperty::Data { .. } => OwnProperty::Accessor {
                get: desc.get.unwrap_or(Value::UNDEFINED),
                set: desc.set.unwrap_or(Value::UNDEFINED),
                enumerable,
                configurable,
            },
            OwnProperty::Accessor { get, set, .. } if !desc.is_data() => OwnProperty::Accessor {
                get: desc.get.unwrap_or(get),
                set: desc.set.unwrap_or(set),
                enumerable,
                configurable,
            },
            OwnProperty::Accessor { .. } => OwnProperty::Data {
                value: desc.value.unwrap_or(Value::UNDEFINED),
                writable: desc.writable.unwrap_or(false),
                enumerable,
                configurable,
            },
        };
        self.with_object(o, |c| c.put(key, next));
        true
    }

    /// ArraySetLength (section 10.4.2.4): a smaller length deletes the
    /// elements past it, from the last, stopping at a non-configurable
    /// one.
    fn array_set_length(&mut self, o: u32, desc: Descriptor) -> bool {
        let Some((old, writable)) = self.object(o).and_then(ObjectCell::array_length) else {
            return false;
        };
        if desc.configurable == Some(true) || desc.enumerable == Some(true) || desc.is_accessor() {
            return false;
        }
        let Some(value) = desc.value else {
            if desc.writable == Some(true) && !writable {
                return false;
            }
            if desc.writable == Some(false) {
                self.with_object(o, |c| c.extra_mut().length_writable = false);
            }
            return true;
        };
        let Some(n) = value.as_number() else {
            return false;
        };
        if n < 0.0 || n.fract() != 0.0 || n >= 4_294_967_296.0 {
            return false;
        }
        let new = n as u32;
        if new != old && !writable {
            return false;
        }
        // Down from the highest element present, never one index at a
        // time: `a.length = 1e9; a.length = 0` must not loop a billion
        // times.
        while let Some(i) = self.object(o).and_then(|c| c.highest_index_below(old)) {
            if i < new {
                break;
            }
            let removable = self
                .object(o)
                .and_then(|c| c.own_property(PropertyKey::Index(i)))
                .is_none_or(|p| p.configurable());
            if !removable {
                self.with_object(o, |c| {
                    let extra = c.extra_mut();
                    extra.length = i + 1;
                    if desc.writable == Some(false) {
                        extra.length_writable = false;
                    }
                });
                return false;
            }
            self.with_object(o, |c| c.remove_element(i));
        }
        self.with_object(o, |c| {
            let extra = c.extra_mut();
            extra.length = new;
            if desc.writable == Some(false) {
                extra.length_writable = false;
            }
        });
        true
    }

    /// `[[HasProperty]]` (section 10.1.7), along the prototype chain.
    pub(crate) fn has_property(&self, o: u32, key: PropertyKey) -> bool {
        let mut at = Some(o);
        while let Some(o) = at {
            if self.get_own_property(o, key).is_some() {
                return true;
            }
            at = self.object(o).and_then(|c| c.proto().as_object());
        }
        false
    }

    /// `[[Get]]` (section 10.1.8): a value, or a getter to call with
    /// `receiver`.
    pub(crate) fn get(&self, o: u32, key: PropertyKey) -> Got {
        let mut at = Some(o);
        while let Some(o) = at {
            match self.get_own_property(o, key) {
                Some(OwnProperty::Data { value, .. }) => return Got::Value(value),
                Some(OwnProperty::Accessor { get, .. }) => return Got::Getter(get),
                None => at = self.object(o).and_then(|c| c.proto().as_object()),
            }
        }
        Got::Value(Value::UNDEFINED)
    }

    /// `[[Set]]` (section 10.1.9's OrdinarySet), `receiver` the value the
    /// assignment is to.
    pub(crate) fn set(
        &mut self,
        o: u32,
        key: PropertyKey,
        value: Value,
        receiver: Value,
    ) -> SetOutcome {
        // The receiver's own writable data property: written in place
        // (the count of its bytes is unchanged).
        if receiver.as_object() == Some(o)
            && self
                .objects
                .get_mut(o)
                .is_some_and(|c| c.kind != ObjectKind::Arguments && c.write_own(key, value))
        {
            return SetOutcome::Done(true);
        }
        let mut at = Some(o);
        let mut found = None;
        while let Some(o) = at {
            if let Some(p) = self.get_own_property(o, key) {
                found = Some(p);
                break;
            }
            at = self.object(o).and_then(|c| c.proto().as_object());
        }
        match found {
            Some(OwnProperty::Accessor { set, .. }) => {
                if set.is_undefined() {
                    SetOutcome::Done(false)
                } else {
                    SetOutcome::Setter(set)
                }
            }
            Some(OwnProperty::Data {
                writable: false, ..
            }) => SetOutcome::Done(false),
            _ => {
                let Some(r) = receiver.as_object() else {
                    return SetOutcome::Done(false);
                };
                let done = match self.get_own_property(r, key) {
                    Some(OwnProperty::Accessor { .. }) => false,
                    Some(OwnProperty::Data {
                        writable: false, ..
                    }) => false,
                    Some(OwnProperty::Data { .. }) => self.define_own_property(
                        r,
                        key,
                        Descriptor {
                            value: Some(value),
                            ..Descriptor::default()
                        },
                    ),
                    None => self.define_own_property(r, key, Descriptor::data(value)),
                };
                SetOutcome::Done(done)
            }
        }
    }

    /// `[[Delete]]` (section 10.1.10).
    pub(crate) fn delete(&mut self, o: u32, key: PropertyKey) -> bool {
        match self.get_own_property(o, key) {
            None => true,
            Some(p) if p.configurable() => {
                if self.is_mapped(o, key) {
                    self.unmap(o, key);
                }
                self.with_object(o, |c| c.remove(key));
                true
            }
            Some(_) => false,
        }
    }

    pub(crate) fn own_keys(&self, o: u32) -> Vec<PropertyKey> {
        let Some(cell) = self.object(o) else {
            return Vec::new();
        };
        let mut keys = cell.own_keys(self.length_key);
        if let Some(h) = self.hidden_symbol {
            keys.retain(|k| *k != PropertyKey::Symbol(h));
        }
        // A `String` wrapper's indices, then `length`, come first
        // (section 10.4.3.3); its other indices are past the string's.
        if cell.kind == ObjectKind::String
            && let Some(s) = cell
                .internal()
                .first()
                .and_then(|v| v.as_string())
                .and_then(|i| self.strings.get(i))
        {
            let n = u32::try_from(s.len()).unwrap_or(u32::MAX);
            let mut all: Vec<PropertyKey> = (0..n).map(PropertyKey::Index).collect();
            let split = keys
                .iter()
                .position(|k| !matches!(k, PropertyKey::Index(_)))
                .unwrap_or(keys.len());
            all.extend(keys.iter().take(split).copied());
            all.push(self.length_key);
            all.extend(keys.iter().skip(split).copied());
            keys = all;
        }
        keys
    }

    pub(crate) fn prevent_extensions(&mut self, o: u32) -> bool {
        self.with_object(o, |c| c.extensible = false).is_some()
    }

    pub(crate) fn is_extensible(&self, o: u32) -> bool {
        self.object(o).is_some_and(ObjectCell::is_extensible)
    }

    pub(crate) fn get_prototype_of(&self, o: u32) -> Value {
        self.object(o).map_or(Value::NULL, ObjectCell::proto)
    }

    /// `[[SetPrototypeOf]]` (section 10.1.2): refused when the object is
    /// not extensible or the chain would come back to it.
    pub(crate) fn set_prototype_of(&mut self, o: u32, proto: Value) -> bool {
        let Some(cell) = self.object(o) else {
            return false;
        };
        if cell.proto() == proto {
            return true;
        }
        if !cell.is_extensible() {
            return false;
        }
        let mut at = proto.as_object();
        while let Some(p) = at {
            if p == o {
                return false;
            }
            at = self.object(p).and_then(|c| c.proto().as_object());
        }
        self.with_object(o, |c| c.proto = proto).is_some()
    }
}
