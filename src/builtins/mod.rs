//! The core built-ins (docs/j2.md, J2.5): natives, the ordered tables
//! they are installed from, and the abstract operations they share.
//!
//! A native is a Rust function given the agent and its [`Call`]: its
//! receiver, its arguments on the agent's register file (where the
//! collector sees them), and `new.target`. Natives are named by an id:
//! the table that holds them and their place in it, so that a function
//! object holds a number, never a pointer.
//!
//! Each intrinsic's own properties are a table of [`Entry`]s, installed
//! in the table's order: the reference browser's (docs/design.md section
//! 1.11; the observed lists are `docs/chrome-surface.md`'s), so that key
//! order is data, not an accident of the code.

mod array;
pub(crate) mod atomics;
mod base64;
pub(crate) mod bigint;
mod binary;
mod collection;
pub(crate) mod date;
mod disposable;
pub(crate) use disposable::Step;
mod error;
mod from_async;
mod function;
mod generator;
mod global;
mod iter_helpers;
mod iterator;
mod json;
mod math;
mod number;
mod object;
mod primitive;
pub(crate) mod promise;
pub(crate) mod proxy;
mod reflect;
mod stack;
mod string;
mod typed;
mod uri;

use crate::agent::Agent;
use crate::heap::ObjectCell;
use crate::interp::messages::{self, ErrorType};
use crate::interp::{Abrupt, Res};
use crate::object::{CONFIGURABLE, Callable, Descriptor, ENUMERABLE, ObjectKind, WRITABLE};
use crate::string::PropertyKey;
use crate::value::{RefKind, Value};

pub(crate) use global::{Intrinsics, Wk, function_prototype_id};
pub(crate) use math::random_state;

/// A native's call: `this`, the `argc` arguments from `args` on the
/// register file, `new.target` (undefined for a call) and the function
/// called.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Call {
    pub this: Value,
    pub args: usize,
    pub argc: usize,
    pub new_target: Value,
    pub callee: Value,
}

pub(crate) type Native = fn(&mut Agent, Call) -> Res<Value>;

/// A property's name in a table.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Name {
    Str(&'static str),
    Sym(Wk),
}

/// What a table's entry defines.
#[derive(Clone, Copy)]
pub(crate) enum What {
    /// A method of this `length`.
    Method(u32, Native),
    /// A constructor (callable with `new`) of this `length`.
    Ctor(u32, Native),
    /// An accessor with a getter only, or a getter and a setter.
    Getter(Native),
    Accessor(Native, Native),
    Number(f64),
    Str(&'static str),
    Symbol(Wk),
    /// `constructor`: the constructor the prototype belongs to.
    Constructor,
    /// The intrinsic of this name (on the global object).
    Intrinsic(&'static str),
    /// A value the installer is given.
    Given(usize),
    /// The same function as the holder's own property of this name,
    /// installed before (`trimLeft` is `trimStart`).
    Alias(&'static str),
}

#[derive(Clone, Copy)]
pub(crate) struct Entry {
    pub name: Name,
    pub what: What,
    pub attrs: u8,
}

/// Writable and configurable, not enumerable: a built-in method.
pub(crate) const HIDDEN: u8 = WRITABLE | CONFIGURABLE;
/// Fixed: a constant (`Math.PI`).
pub(crate) const FIXED: u8 = 0;

pub(crate) const fn method(name: &'static str, length: u32, f: Native) -> Entry {
    Entry {
        name: Name::Str(name),
        what: What::Method(length, f),
        attrs: HIDDEN,
    }
}

pub(crate) const fn ctor(name: &'static str, length: u32, f: Native) -> Entry {
    Entry {
        name: Name::Str(name),
        what: What::Ctor(length, f),
        attrs: HIDDEN,
    }
}

pub(crate) const fn getter(name: &'static str, f: Native) -> Entry {
    Entry {
        name: Name::Str(name),
        what: What::Getter(f),
        attrs: CONFIGURABLE,
    }
}

pub(crate) const fn constant(name: &'static str, n: f64) -> Entry {
    Entry {
        name: Name::Str(name),
        what: What::Number(n),
        attrs: FIXED,
    }
}

pub(crate) const fn entry(name: Name, what: What, attrs: u8) -> Entry {
    Entry { name, what, attrs }
}

/// `constructor`, first on most prototypes.
pub(crate) const CONSTRUCTOR: Entry = Entry {
    name: Name::Str("constructor"),
    what: What::Constructor,
    attrs: HIDDEN,
};

/// Every table, by its index: a native's id is its table's index
/// shifted left by 16, or'd with its index in the table, and with
/// [`SETTER`] for an accessor's setter.
const TABLES: &[&[Entry]] = &[
    global::CTORS,
    global::GLOBAL,
    object::STATICS,
    object::PROTO,
    function::PROTO,
    error::STATICS,
    error::PROTO,
    error::NATIVE_PROTO,
    primitive::BOOLEAN_PROTO,
    primitive::SYMBOL_STATICS,
    primitive::SYMBOL_PROTO,
    number::STATICS,
    number::PROTO,
    string::STATICS,
    string::PROTO,
    array::STATICS,
    array::PROTO,
    math::MATH,
    reflect::REFLECT,
    stack::ACCESSORS,
    stack::CALL_SITE,
    stack::CALL_SITE_CTOR,
    iter_helpers::ITERATOR_PROTO,
    iterator::ARRAY_ITERATOR_PROTO,
    iterator::STRING_ITERATOR_PROTO,
    generator::GENERATOR_PROTO,
    generator::GENERATOR_FUNCTION_PROTO,
    promise::PROMISE,
    promise::PROMISE_PROTO,
    promise::PROMISE_INTERNAL,
    generator::ASYNC_FUNCTION_PROTO,
    generator::ASYNC_GENERATOR_PROTO,
    generator::ASYNC_GENERATOR_FUNCTION_PROTO,
    generator::ASYNC_ITERATOR_PROTO,
    promise::ASYNC_FROM_SYNC_PROTO,
    proxy::PROXY,
    proxy::PROXY_INTERNAL,
    collection::MAP,
    collection::MAP_PROTO,
    collection::SET,
    collection::SET_PROTO,
    collection::WEAK_MAP_PROTO,
    collection::WEAK_SET_PROTO,
    collection::WEAK_REF_PROTO,
    collection::REGISTRY_PROTO,
    collection::MAP_ITERATOR_PROTO,
    collection::SET_ITERATOR_PROTO,
    typed::ARRAY_BUFFER,
    typed::ARRAY_BUFFER_PROTO,
    typed::TYPED_ARRAY,
    typed::TYPED_ARRAY_PROTO,
    typed::TYPED_CTOR,
    typed::TYPED_PROTO,
    binary::SHARED_ARRAY_BUFFER,
    binary::SHARED_ARRAY_BUFFER_PROTO,
    binary::DATA_VIEW_PROTO,
    atomics::ATOMICS,
    base64::UINT8_ARRAY,
    base64::UINT8_ARRAY_PROTO,
    global::SHARED_GLOBAL,
    json::JSON,
    bigint::BIGINT,
    bigint::BIGINT_PROTO,
    date::DATE,
    date::DATE_PROTO,
    iter_helpers::ITERATOR,
    iter_helpers::ITERATOR_HELPER_PROTO,
    iter_helpers::WRAP_PROTO,
    disposable::DISPOSABLE_STACK_PROTO,
    disposable::ASYNC_DISPOSABLE_STACK_PROTO,
    disposable::DISPOSABLE_INTERNAL,
    from_async::FROM_ASYNC_INTERNAL,
];

/// The tables' indices.
pub(crate) mod table {
    pub const CTORS: u32 = 0;
    pub const GLOBAL: u32 = 1;
    pub const OBJECT: u32 = 2;
    pub const OBJECT_PROTO: u32 = 3;
    pub const FUNCTION_PROTO: u32 = 4;
    pub const ERROR: u32 = 5;
    pub const ERROR_PROTO: u32 = 6;
    pub const NATIVE_ERROR_PROTO: u32 = 7;
    pub const BOOLEAN_PROTO: u32 = 8;
    pub const SYMBOL: u32 = 9;
    pub const SYMBOL_PROTO: u32 = 10;
    pub const NUMBER: u32 = 11;
    pub const NUMBER_PROTO: u32 = 12;
    pub const STRING: u32 = 13;
    pub const STRING_PROTO: u32 = 14;
    pub const ARRAY: u32 = 15;
    pub const ARRAY_PROTO: u32 = 16;
    pub const MATH: u32 = 17;
    pub const REFLECT: u32 = 18;
    pub const STACK: u32 = 19;
    pub const CALL_SITE: u32 = 20;
    pub const CALL_SITE_CTOR: u32 = 21;
    pub const ITERATOR_PROTO: u32 = 22;
    pub const ARRAY_ITERATOR_PROTO: u32 = 23;
    pub const STRING_ITERATOR_PROTO: u32 = 24;
    pub const GENERATOR_PROTO: u32 = 25;
    pub const GENERATOR_FUNCTION_PROTO: u32 = 26;
    pub const PROMISE: u32 = 27;
    pub const PROMISE_PROTO: u32 = 28;
    pub const PROMISE_INTERNAL: u32 = 29;
    pub const ASYNC_FUNCTION_PROTO: u32 = 30;
    pub const ASYNC_GENERATOR_PROTO: u32 = 31;
    pub const ASYNC_GENERATOR_FUNCTION_PROTO: u32 = 32;
    pub const ASYNC_ITERATOR_PROTO: u32 = 33;
    pub const ASYNC_FROM_SYNC_PROTO: u32 = 34;
    pub const PROXY: u32 = 35;
    pub const PROXY_INTERNAL: u32 = 36;
    pub const MAP: u32 = 37;
    pub const MAP_PROTO: u32 = 38;
    pub const SET: u32 = 39;
    pub const SET_PROTO: u32 = 40;
    pub const WEAK_MAP_PROTO: u32 = 41;
    pub const WEAK_SET_PROTO: u32 = 42;
    pub const WEAK_REF_PROTO: u32 = 43;
    pub const REGISTRY_PROTO: u32 = 44;
    pub const MAP_ITERATOR_PROTO: u32 = 45;
    pub const SET_ITERATOR_PROTO: u32 = 46;
    pub const ARRAY_BUFFER: u32 = 47;
    pub const ARRAY_BUFFER_PROTO: u32 = 48;
    pub const TYPED_ARRAY: u32 = 49;
    pub const TYPED_ARRAY_PROTO: u32 = 50;
    pub const TYPED_CTOR: u32 = 51;
    pub const TYPED_PROTO: u32 = 52;
    pub const SHARED_ARRAY_BUFFER: u32 = 53;
    pub const SHARED_ARRAY_BUFFER_PROTO: u32 = 54;
    pub const DATA_VIEW_PROTO: u32 = 55;
    pub const ATOMICS: u32 = 56;
    pub const UINT8_ARRAY: u32 = 57;
    pub const UINT8_ARRAY_PROTO: u32 = 58;
    pub const SHARED_GLOBAL: u32 = 59;
    pub const JSON: u32 = 60;
    pub const BIGINT: u32 = 61;
    pub const BIGINT_PROTO: u32 = 62;
    pub const DATE: u32 = 63;
    pub const DATE_PROTO: u32 = 64;
    pub const ITERATOR: u32 = 65;
    pub const ITERATOR_HELPER_PROTO: u32 = 66;
    pub const WRAP_PROTO: u32 = 67;
    pub const DISPOSABLE_STACK_PROTO: u32 = 68;
    pub const ASYNC_DISPOSABLE_STACK_PROTO: u32 = 69;
    pub const DISPOSABLE_INTERNAL: u32 = 70;
    pub const FROM_ASYNC_INTERNAL: u32 = 71;
}

/// How native `id` resumes a generator, if it is `%GeneratorPrototype%`'s
/// `next`, `return` or `throw`.
pub(crate) fn generator_resume(id: u32) -> Option<crate::interp::generator::Resume> {
    if id >> 16 == table::GENERATOR_PROTO {
        generator::resume_of(id & 0x7fff)
    } else {
        None
    }
}

const SETTER: u32 = 0x8000;

fn lookup(id: u32) -> Option<&'static Entry> {
    TABLES
        .get((id >> 16) as usize)
        .and_then(|t| t.get((id & 0x7fff) as usize))
}

/// Runs native `id`.
pub(crate) fn call_native(agent: &mut Agent, id: u32, call: Call) -> Res<Value> {
    if crate::embed::is_host(id) {
        let args = agent.args(call);
        return crate::embed::call_host(agent, id - crate::embed::HOST_BASE, call.this, &args);
    }
    let f = match lookup(id).map(|e| e.what) {
        Some(What::Method(_, f) | What::Ctor(_, f) | What::Getter(f)) => f,
        Some(What::Accessor(get, set)) => {
            if id & SETTER != 0 {
                set
            } else {
                get
            }
        }
        _ => return Err(agent.throw_error(ErrorType::TypeError, "Illegal invocation")),
    };
    f(agent, call)
}

/// Whether native `id` may be called with `new`.
pub(crate) fn native_constructs(id: u32) -> bool {
    id & SETTER == 0 && matches!(lookup(id).map(|e| e.what), Some(What::Ctor(..)))
}

/// The name a native function was made with, for
/// `Function.prototype.toString`.
pub(crate) fn native_name(id: u32) -> String {
    let Some(e) = lookup(id) else {
        return String::new();
    };
    let base = match e.name {
        Name::Str(s) => s.to_owned(),
        Name::Sym(w) => format!("[{}]", w.description()),
    };
    match e.what {
        What::Getter(_) => format!("get {base}"),
        What::Accessor(..) if id & SETTER != 0 => format!("set {base}"),
        What::Accessor(..) => format!("get {base}"),
        _ => base,
    }
}

pub(crate) const fn native_id(table: u32, index: u32) -> u32 {
    table << 16 | index
}

/// The id of the entry named `name` in `table`, for the natives the
/// engine itself names (`%ThrowTypeError%`, the constructors).
pub(crate) fn id_of(table: u32, name: &str) -> Option<u32> {
    let t = TABLES.get(table as usize)?;
    t.iter()
        .position(|e| matches!(e.name, Name::Str(s) if s == name))
        .map(|i| native_id(table, i as u32))
}

impl Agent {
    // Arguments.

    /// Argument `i` of a native's call, undefined past the last.
    pub(crate) fn arg(&self, c: Call, i: usize) -> Value {
        if i < c.argc {
            self.vm
                .stack
                .get(c.args + i)
                .copied()
                .unwrap_or(Value::UNDEFINED)
        } else {
            Value::UNDEFINED
        }
    }

    /// The arguments of a call, copied out.
    pub(crate) fn args(&self, c: Call) -> Vec<Value> {
        (0..c.argc).map(|i| self.arg(c, i)).collect()
    }

    pub(crate) fn type_error(&mut self, message: impl AsRef<str>) -> Abrupt {
        self.throw_error(ErrorType::TypeError, message)
    }

    pub(crate) fn range_error(&mut self, message: impl AsRef<str>) -> Abrupt {
        self.throw_error(ErrorType::RangeError, message)
    }

    pub(crate) fn syntax_error(&mut self, message: impl AsRef<str>) -> Abrupt {
        self.throw_error(ErrorType::SyntaxError, message)
    }

    // Making natives and intrinsics.

    /// A function object running native `id`, with its `length` and
    /// `name`.
    pub(crate) fn native_function(&mut self, id: u32, length: u32, name: &str) -> Res<Value> {
        let fp = self.realm().function_prototype;
        let f = self.alloc_object(ObjectCell::function(
            fp,
            Callable::Native {
                id,
                realm: self.vm.current,
            },
        ))?;
        self.vm.temps.push(f);
        let units: Vec<u16> = name.encode_utf16().collect();
        let done = self.finish_function(f, length, &units, false);
        self.vm.temps.pop();
        done.map(|()| f)
    }

    /// The key a table's name makes.
    pub(crate) fn name_key(&mut self, name: Name) -> PropertyKey {
        match name {
            Name::Str(s) => {
                let units: Vec<u16> = s.encode_utf16().collect();
                self.heap.atoms.key(&units)
            }
            Name::Sym(w) => self.well_known_key(w),
        }
    }

    /// Installs table `table` on `holder` in its order: `constructor`
    /// is `ctor`, and `What::Given(i)` is `given[i]`.
    pub(crate) fn install_table(
        &mut self,
        holder: Value,
        table: u32,
        ctor: Value,
        given: &[Value],
    ) -> Res<()> {
        let Some(entries) = TABLES.get(table as usize) else {
            return Ok(());
        };
        let Some(o) = holder.as_object() else {
            return Ok(());
        };
        for (i, e) in entries.iter().enumerate() {
            let id = native_id(table, i as u32);
            let key = self.name_key(e.name);
            let fname = match e.name {
                Name::Str(s) => s.to_owned(),
                Name::Sym(w) => format!("[{}]", w.description()),
            };
            let attrs = |value: Option<Value>, get: Option<Value>, set: Option<Value>| Descriptor {
                value,
                get,
                set,
                writable: value.map(|_| e.attrs & WRITABLE != 0),
                enumerable: Some(e.attrs & ENUMERABLE != 0),
                configurable: Some(e.attrs & CONFIGURABLE != 0),
            };
            let desc = match e.what {
                What::Method(length, _) | What::Ctor(length, _) => {
                    let f = self.native_function(id, length, &fname)?;
                    attrs(Some(f), None, None)
                }
                What::Getter(_) => {
                    let g = self.native_function(id, 0, &format!("get {fname}"))?;
                    attrs(None, Some(g), Some(Value::UNDEFINED))
                }
                What::Accessor(..) => {
                    let g = self.native_function(id, 0, &format!("get {fname}"))?;
                    self.vm.temps.push(g);
                    let s = self.native_function(id | SETTER, 1, &format!("set {fname}"));
                    self.vm.temps.pop();
                    attrs(None, Some(g), Some(s?))
                }
                What::Number(n) => attrs(Some(Value::number(n)), None, None),
                What::Str(s) => {
                    let v = self.new_string(s)?;
                    attrs(Some(v), None, None)
                }
                What::Symbol(w) => {
                    let s = self.well_known(w);
                    attrs(Some(s), None, None)
                }
                What::Constructor => attrs(Some(ctor), None, None),
                What::Intrinsic(name) => {
                    let v = self.intrinsic_named(name);
                    attrs(Some(v), None, None)
                }
                What::Alias(name) => {
                    let k = self.name_key(Name::Str(name));
                    let v = match self.heap.get_own_property(o, k) {
                        Some(crate::object::OwnProperty::Data { value, .. }) => value,
                        _ => Value::UNDEFINED,
                    };
                    attrs(Some(v), None, None)
                }
                What::Given(i) => attrs(
                    Some(given.get(i).copied().unwrap_or(Value::UNDEFINED)),
                    None,
                    None,
                ),
            };
            self.heap.define_own_property(o, key, desc);
        }
        Ok(())
    }

    /// A constructor from the constructors' table: its `length`, `name`
    /// and a `prototype` neither writable, enumerable nor configurable.
    pub(crate) fn make_ctor(&mut self, name: &str, proto: Value) -> Res<Value> {
        let Some(id) = id_of(table::CTORS, name) else {
            return Ok(Value::UNDEFINED);
        };
        let length = match lookup(id).map(|e| e.what) {
            Some(What::Ctor(l, _) | What::Method(l, _)) => l,
            _ => 0,
        };
        let f = self.native_function(id, length, name)?;
        if let Some(o) = f.as_object() {
            let key = self.realm().prototype_key;
            self.heap.define_own_property(
                o,
                key,
                Descriptor {
                    value: Some(proto),
                    writable: Some(false),
                    enumerable: Some(false),
                    configurable: Some(false),
                    ..Descriptor::default()
                },
            );
        }
        Ok(f)
    }

    // Symbols.

    /// The well-known symbol `w` (shared by the agent's realms).
    pub(crate) fn well_known(&self, w: Wk) -> Value {
        self.vm
            .well_known
            .get(w as usize)
            .copied()
            .unwrap_or(Value::UNDEFINED)
    }

    pub(crate) fn well_known_key(&self, w: Wk) -> PropertyKey {
        match self.well_known(w).as_reference() {
            Some((RefKind::Symbol, s)) => PropertyKey::Symbol(s),
            _ => PropertyKey::Index(u32::MAX),
        }
    }

    pub(crate) fn new_symbol(&mut self, description: Value) -> Res<Value> {
        if self.reserve(32).is_err() {
            return Err(self.throw_error(ErrorType::RangeError, messages::OUT_OF_MEMORY));
        }
        let (i, _) = self
            .heap
            .symbols
            .alloc(crate::heap::SymbolCell::new(description));
        Ok(Value::reference(RefKind::Symbol, i))
    }

    /// A new private name described `#name` (a field's until a method
    /// or accessor is defined under it).
    #[inline(never)]
    pub(crate) fn new_private_name(&mut self, description: Value) -> Res<Value> {
        let s = self.new_symbol(description)?;
        if let Some(cell) = s
            .as_reference()
            .and_then(|(_, i)| self.heap.symbols.get_mut(i))
        {
            cell.private = Some(Box::new(crate::heap::PrivateName::field()));
        }
        Ok(s)
    }

    // Conversions.

    /// The one-unit string of `unit`, made once.
    pub(crate) fn unit_string(&mut self, unit: u16) -> Res<Value> {
        if let Some(&s) = self.heap.unit_strings.get(&unit) {
            return Ok(s);
        }
        let s = self.new_string_units(&[unit])?;
        self.heap.unit_strings.insert(unit, s);
        Ok(s)
    }

    /// ToObject (section 7.1.18).
    pub(crate) fn object_of(&mut self, v: Value) -> Res<Value> {
        if v.as_object().is_some() {
            return Ok(v);
        }
        if v.is_nullish() {
            return Err(self.type_error(messages::NULLISH_TO_OBJECT));
        }
        self.wrap(v, None)
    }

    /// A wrapper object of the primitive `v`, with prototype `proto` or
    /// the realm's.
    pub(crate) fn wrap(&mut self, v: Value, proto: Option<Value>) -> Res<Value> {
        let r = self.realm();
        let (kind, default) = if v.is_number() {
            (ObjectKind::Number, r.number_prototype)
        } else if v.as_bool().is_some() {
            (ObjectKind::Boolean, r.boolean_prototype)
        } else if v.as_string().is_some() {
            (ObjectKind::String, r.string_prototype)
        } else if matches!(v.as_reference(), Some((RefKind::BigInt, _))) {
            (ObjectKind::BigInt, r.intrinsics.bigint_prototype)
        } else {
            (ObjectKind::Symbol, r.symbol_prototype)
        };
        if kind == ObjectKind::String {
            // The table of one-unit strings holds every unit of it, which
            // its own indices read.
            self.vm.temps.push(v);
            let units = self.string_units(v).unwrap_or_default();
            let mut done = Ok(Value::UNDEFINED);
            for u in units {
                if !self.heap.unit_strings.contains_key(&u) {
                    done = self.unit_string(u);
                    if done.is_err() {
                        break;
                    }
                }
            }
            self.vm.temps.pop();
            done?;
        }
        let proto = proto.unwrap_or(default);
        self.alloc_object(ObjectCell::with_internal(proto, kind, vec![v]))
    }

    /// The primitive a wrapper of `kind` holds, or `v` itself if it is
    /// such a primitive (thisBooleanValue and the like).
    pub(crate) fn this_primitive(&self, v: Value, kind: ObjectKind) -> Option<Value> {
        let is = |p: Value| match kind {
            ObjectKind::Number => p.is_number(),
            ObjectKind::Boolean => p.as_bool().is_some(),
            ObjectKind::String => p.as_string().is_some(),
            ObjectKind::BigInt => matches!(p.as_reference(), Some((RefKind::BigInt, _))),
            _ => matches!(p.as_reference(), Some((RefKind::Symbol, _))),
        };
        if is(v) {
            return Some(v);
        }
        let cell = self.heap.objects.get(v.as_object()?)?;
        if cell.kind() == kind {
            cell.internal().first().copied()
        } else {
            None
        }
    }

    /// ToIntegerOrInfinity (section 7.1.5).
    pub(crate) fn integer(&mut self, v: Value) -> Res<f64> {
        if let Some(i) = v.as_int() {
            return Ok(f64::from(i));
        }
        let n = self.number(v)?;
        Ok(if n.is_nan() { 0.0 } else { n.trunc() + 0.0 })
    }

    /// ToLength (section 7.1.20).
    pub(crate) fn length_value(&mut self, v: Value) -> Res<f64> {
        let n = self.integer(v)?;
        Ok(n.clamp(0.0, 9_007_199_254_740_991.0))
    }

    /// LengthOfArrayLike (section 7.3.18).
    pub(crate) fn length_of(&mut self, o: Value) -> Res<f64> {
        if let Some((len, _)) = o
            .as_object()
            .and_then(|i| self.heap.objects.get(i))
            .and_then(ObjectCell::array_length)
        {
            return Ok(f64::from(len));
        }
        let key = self.heap.length_key;
        let len = self.get_value(o, key)?;
        self.length_value(len)
    }

    /// The text of a value's ToString, for natives working on text.
    pub(crate) fn text_units(&mut self, v: Value) -> Res<Vec<u16>> {
        let s = self.string_value(v)?;
        Ok(self.string_units(s).unwrap_or_default())
    }

    /// A relative index (`slice`'s, `at`'s): ToIntegerOrInfinity of
    /// `v`, from the end if negative, clamped to `0..=len`; `default`
    /// when undefined.
    pub(crate) fn relative(&mut self, v: Value, len: f64, default: f64) -> Res<f64> {
        if v.is_undefined() {
            return Ok(default);
        }
        let n = self.integer(v)?;
        Ok(if n < 0.0 {
            (len + n).max(0.0)
        } else {
            n.min(len)
        })
    }

    /// The key of index `i`, which may be past an array index.
    pub(crate) fn index_key(&mut self, i: f64) -> PropertyKey {
        if (0.0..4_294_967_295.0).contains(&i) {
            PropertyKey::Index(i as u32)
        } else {
            let text = crate::number::number_to_string(i);
            let units: Vec<u16> = text.encode_utf16().collect();
            self.heap.atoms.key(&units)
        }
    }

    /// A property key as a value: a string, or the symbol.
    pub(crate) fn key_value(&mut self, key: PropertyKey) -> Res<Value> {
        match key {
            PropertyKey::Index(i) => self.new_string(&i.to_string()),
            PropertyKey::Atom(a) => {
                let units = self.heap.atoms.units(a).to_vec();
                self.new_string_units(&units)
            }
            PropertyKey::Symbol(s) => Ok(Value::reference(RefKind::Symbol, s)),
        }
    }

    // Objects.

    /// CreateDataProperty (section 7.3.5).
    pub(crate) fn create_data_property(&mut self, o: Value, key: PropertyKey, v: Value) -> bool {
        o.as_object()
            .is_some_and(|i| self.heap.define_own_property(i, key, Descriptor::data(v)))
    }

    /// CreateDataPropertyOrThrow (section 7.3.7).
    pub(crate) fn create_data_property_or_throw(
        &mut self,
        o: Value,
        key: PropertyKey,
        v: Value,
    ) -> Res<()> {
        // A proxy's [[DefineOwnProperty]] is its trap's.
        if o.as_object().is_some_and(|i| self.is_proxy(i)) {
            return self.define_or_throw(o, key, Descriptor::data(v));
        }
        if self.create_data_property(o, key, v) {
            Ok(())
        } else {
            let k = self.key_display(key);
            Err(self.type_error(format!("Cannot add property {k}, object is not extensible")))
        }
    }

    /// DefinePropertyOrThrow (section 7.3.8).
    pub(crate) fn define_or_throw(&mut self, o: Value, key: PropertyKey, d: Descriptor) -> Res<()> {
        let mut d = d;
        if key == self.heap.length_key
            && self.is_array(o)
            && let Some(v) = d.value
        {
            d.value = Some(self.array_length_value(v)?);
        }
        let done = match o.as_object() {
            Some(i) => self.o_define_own_property(i, key, d)?,
            None => false,
        };
        if done {
            Ok(())
        } else if o.as_object().is_some_and(|i| self.is_proxy(i)) {
            let k = self.key_display(key);
            Err(self.type_error(messages::proxy_define_false(&k)))
        } else if let Some(i) = o.as_object()
            && let Some(index) = self.heap.typed_key(i, key)
            && !self.heap.view_valid(i, index)
        {
            Err(self.type_error("Invalid typed array index"))
        } else {
            let k = self.key_display(key);
            Err(self.type_error(format!("Cannot redefine property: {k}")))
        }
    }

    /// DeletePropertyOrThrow (section 7.3.10).
    pub(crate) fn delete_or_throw(&mut self, o: Value, key: PropertyKey) -> Res<()> {
        let done = match o.as_object() {
            Some(i) => self.o_delete(i, key)?,
            None => true,
        };
        if done {
            Ok(())
        } else if o.as_object().is_some_and(|i| self.is_proxy(i)) {
            let k = self.key_display(key);
            Err(self.type_error(messages::proxy_delete_false(&k)))
        } else {
            let k = self.key_display(key);
            let text = self.display(o);
            Err(self.type_error(messages::cannot_delete(&k, &text)))
        }
    }

    /// Set (section 7.3.4) with Throw true.
    pub(crate) fn put(&mut self, o: Value, key: PropertyKey, v: Value) -> Res<()> {
        self.set_value(o, key, v, true)
    }

    pub(crate) fn has_own(&self, o: Value, key: PropertyKey) -> bool {
        o.as_object()
            .is_some_and(|i| self.heap.get_own_property(i, key).is_some())
    }

    pub(crate) fn has_property(&mut self, o: Value, key: PropertyKey) -> Res<bool> {
        match o.as_object() {
            Some(i) => self.o_has_property(i, key),
            None => Ok(false),
        }
    }

    /// A data property of `o` or its prototypes, read with no side
    /// effects: none through an accessor or a proxy.
    #[inline(never)]
    fn data_property(&self, o: u32, key: PropertyKey) -> Option<Value> {
        let mut at = o;
        for _ in 0..10_000 {
            let cell = self.heap.objects.get(at)?;
            if cell.kind() == ObjectKind::Proxy {
                return None;
            }
            match self.heap.get_own_property(at, key) {
                Some(crate::object::OwnProperty::Data { value, .. }) => return Some(value),
                Some(crate::object::OwnProperty::Accessor { .. }) => return None,
                None => at = self.heap.get_prototype_of(at).as_object()?,
            }
        }
        None
    }

    /// An object as the reference browser names it in a message (its
    /// NoSideEffectsToString): an error as `Name: message`; one whose
    /// `toString` is `Object.prototype.toString` by its constructor's
    /// name (`#<Map>`); the rest as `[object Tag]`.
    #[inline(never)]
    pub(crate) fn object_display(&self, o: u32) -> String {
        let r = self.realm();
        let text = |v: Option<Value>| {
            v.and_then(|v| self.string_units(v))
                .map(|u| String::from_utf16_lossy(&u))
        };
        let kind = self.heap.objects.get(o).map(|c| c.kind());
        if kind == Some(ObjectKind::Error) {
            let name = text(self.data_property(o, r.name_key)).unwrap_or_else(|| "Error".into());
            let message = text(self.data_property(o, r.message_key)).unwrap_or_default();
            return match (name.is_empty(), message.is_empty()) {
                (_, true) => name,
                (true, false) => message,
                (false, false) => format!("{name}: {message}"),
            };
        }
        let object_to_string = r
            .object_prototype
            .as_object()
            .and_then(|p| self.data_property(p, r.to_string_key));
        if object_to_string.is_some() && self.data_property(o, r.to_string_key) == object_to_string
        {
            let ctor = self
                .data_property(o, r.constructor_key)
                .filter(|c| self.callable(*c).is_some());
            if let Some(name) = text(
                ctor.and_then(|c| c.as_object())
                    .and_then(|c| self.heap.get_own_property(c, r.name_key))
                    .and_then(|p| match p {
                        crate::object::OwnProperty::Data { value, .. } => Some(value),
                        _ => None,
                    }),
            ) && !name.is_empty()
            {
                return format!("#<{name}>");
            }
        }
        let tag =
            text(self.data_property(o, self.well_known_key(crate::builtins::Wk::ToStringTag)));
        let builtin = match kind {
            Some(ObjectKind::Array) => "Array",
            Some(ObjectKind::Arguments) => "Arguments",
            Some(ObjectKind::Boolean) => "Boolean",
            Some(ObjectKind::Number) => "Number",
            Some(ObjectKind::String) => "String",
            Some(ObjectKind::TypedArray) => self.heap.view(o).map_or("Object", |v| v.elem.name()),
            _ => "Object",
        };
        format!("[object {}]", tag.unwrap_or_else(|| builtin.into()))
    }

    /// A new array of `values`.
    pub(crate) fn array_of(&mut self, values: &[Value]) -> Res<Value> {
        let start = self.vm.stack.len();
        self.vm.stack.extend_from_slice(values);
        let a = self.make_array_from_stack(start, values.len());
        self.vm.stack.truncate(start);
        a
    }

    /// An object with the realm's `Object.prototype`.
    pub(crate) fn plain_object(&mut self) -> Res<Value> {
        let proto = self.realm().object_prototype;
        self.alloc_object(ObjectCell::new(proto, ObjectKind::Ordinary))
    }

    /// GetMethod (section 7.3.11): undefined for undefined or null, else
    /// a function or a TypeError.
    pub(crate) fn get_method(&mut self, v: Value, key: PropertyKey) -> Res<Value> {
        let f = self.get_value(v, key)?;
        if f.is_nullish() {
            return Ok(Value::UNDEFINED);
        }
        if self.callable(f).is_none() {
            let text = self.display(f);
            return Err(self.type_error(format!("{text} is not a function")));
        }
        Ok(f)
    }

    pub(crate) fn is_callable(&self, v: Value) -> bool {
        self.callable(v).is_some()
    }

    /// IsArray (section 7.2.2).
    pub(crate) fn is_array(&self, v: Value) -> bool {
        v.as_object()
            .and_then(|o| self.heap.objects.get(o))
            .is_some_and(|c| c.kind() == ObjectKind::Array)
    }

    /// IsArray (section 7.2.2), throwing for a revoked proxy.
    pub(crate) fn is_array_checked(&mut self, v: Value) -> Res<bool> {
        let mut at = v;
        loop {
            let Some(c) = at.as_object().and_then(|o| self.heap.objects.get(o)) else {
                return Ok(false);
            };
            match c.kind() {
                ObjectKind::Array => return Ok(true),
                ObjectKind::Proxy => {
                    let target = c.internal().first().copied().unwrap_or(Value::NULL);
                    if target.is_null() {
                        return Err(self.type_error(
                            "Cannot perform 'IsArray' on a proxy that has been revoked",
                        ));
                    }
                    at = target;
                }
                _ => return Ok(false),
            }
        }
    }

    pub(crate) fn kind_of(&self, v: Value) -> Option<ObjectKind> {
        Some(self.heap.objects.get(v.as_object()?)?.kind())
    }

    /// The text of a string value, lossily, for messages.
    pub(crate) fn text(&self, v: Value) -> String {
        self.string_units(v)
            .map(|u| String::from_utf16_lossy(&u))
            .unwrap_or_default()
    }
}
