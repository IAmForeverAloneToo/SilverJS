//! The realm (ECMA-262 section 9.3): its intrinsics, its global object
//! and its global lexical bindings, and the natives J2.4 needs to run
//! the micro-benches. The rest of the built-ins are J2.5's: they join the
//! [`NATIVES`] table and the intrinsics made here.

use std::collections::HashMap;

use super::Res;
use super::messages::{self, ErrorType};
use super::ops::hidden;
use crate::agent::Agent;
use crate::heap::ObjectCell;
use crate::object::{Callable, Descriptor, ObjectKind};
use crate::string::PropertyKey;
use crate::value::Value;

/// A script's top-level `let`, `const` or class binding.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Lexical {
    pub value: Value,
    pub constant: bool,
}

/// Strings the engine uses often, made once.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Strings {
    pub empty: Value,
    pub undefined: Value,
    pub null: Value,
    pub true_: Value,
    pub false_: Value,
    pub number: Value,
    pub string: Value,
    pub boolean: Value,
    pub symbol: Value,
    pub bigint: Value,
    pub function: Value,
    pub object: Value,
}

pub(crate) struct Realm {
    pub global: Value,
    pub object_prototype: Value,
    pub function_prototype: Value,
    pub array_prototype: Value,
    pub string_prototype: Value,
    pub number_prototype: Value,
    pub boolean_prototype: Value,
    pub symbol_prototype: Value,
    /// The error prototypes, by [`ErrorType`].
    pub errors: [Value; 7],
    pub lexical: HashMap<PropertyKey, Lexical>,
    pub strings: Strings,
    pub prototype_key: PropertyKey,
    pub constructor_key: PropertyKey,
    pub name_key: PropertyKey,
    pub message_key: PropertyKey,
    pub to_string_key: PropertyKey,
    pub value_of_key: PropertyKey,
}

impl Realm {
    /// The prototype of the errors of type `t`.
    pub(crate) fn error_proto(&self, t: ErrorType) -> Value {
        self.errors.get(t as usize).copied().unwrap_or(Value::NULL)
    }

    pub(crate) fn trace(&self, out: &mut Vec<Value>) {
        out.extend([
            self.global,
            self.object_prototype,
            self.function_prototype,
            self.array_prototype,
            self.string_prototype,
            self.number_prototype,
            self.boolean_prototype,
            self.symbol_prototype,
        ]);
        out.extend_from_slice(&self.errors);
        out.extend(self.lexical.values().map(|b| b.value));
        let s = &self.strings;
        out.extend([
            s.empty,
            s.undefined,
            s.null,
            s.true_,
            s.false_,
            s.number,
            s.string,
            s.boolean,
            s.symbol,
            s.bigint,
            s.function,
            s.object,
        ]);
    }
}

/// A native: `this`, and its arguments, `argc` values from `start` on
/// the register file.
type Native = fn(&mut Agent, Value, usize, usize) -> Res<Value>;

/// The natives, by id: name, `length`, the function, and whether `new`
/// may call it.
const NATIVES: &[(&str, u32, Native, bool)] = &[
    ("", 0, function_prototype, false),
    ("toString", 0, object_to_string, false),
    ("valueOf", 0, object_value_of, false),
    ("push", 1, array_push, false),
    ("toString", 0, error_to_string, false),
];

const FUNCTION_PROTOTYPE: u32 = 0;
const OBJECT_TO_STRING: u32 = 1;
const OBJECT_VALUE_OF: u32 = 2;
const ARRAY_PUSH: u32 = 3;
const ERROR_TO_STRING: u32 = 4;

pub(crate) fn call_native(
    agent: &mut Agent,
    id: u32,
    this: Value,
    start: usize,
    argc: usize,
) -> Res<Value> {
    match NATIVES.get(id as usize) {
        Some((_, _, f, _)) => f(agent, this, start, argc),
        None => Err(agent.throw_error(ErrorType::TypeError, messages::not_a_function("native"))),
    }
}

pub(crate) fn native_constructs(id: u32) -> bool {
    NATIVES.get(id as usize).is_some_and(|n| n.3)
}

fn arg(agent: &Agent, start: usize, argc: usize, i: usize) -> Value {
    if i < argc {
        agent
            .vm
            .stack
            .get(start + i)
            .copied()
            .unwrap_or(Value::UNDEFINED)
    } else {
        Value::UNDEFINED
    }
}

/// `Function.prototype` itself: callable, returning undefined.
fn function_prototype(_: &mut Agent, _: Value, _: usize, _: usize) -> Res<Value> {
    Ok(Value::UNDEFINED)
}

/// `Object.prototype.toString` (section 20.1.3.6), without
/// `@@toStringTag` (J2.5).
fn object_to_string(agent: &mut Agent, this: Value, _: usize, _: usize) -> Res<Value> {
    let tag = match this {
        Value::UNDEFINED => "Undefined",
        Value::NULL => "Null",
        _ => match this.as_object().and_then(|o| agent.heap.objects.get(o)) {
            Some(c) if c.kind() == ObjectKind::Array => "Array",
            Some(c) if c.callable().is_some() => "Function",
            Some(_) => {
                let errors = agent.realm().errors;
                let mut at = this.as_object();
                let mut is_error = false;
                while let Some(o) = at {
                    let p = agent.heap.get_prototype_of(o);
                    if errors.contains(&p) {
                        is_error = true;
                        break;
                    }
                    at = p.as_object();
                }
                if is_error { "Error" } else { "Object" }
            }
            None if this.is_number() => "Number",
            None if this.as_bool().is_some() => "Boolean",
            None if this.as_string().is_some() => "String",
            None => "Symbol",
        },
    };
    agent.new_string(&format!("[object {tag}]"))
}

/// `Object.prototype.valueOf` (section 20.1.3.7). (A primitive `this`
/// is its own value until the wrapper objects of J2.5.)
fn object_value_of(agent: &mut Agent, this: Value, _: usize, _: usize) -> Res<Value> {
    if this.is_nullish() {
        return Err(agent.throw_error(ErrorType::TypeError, messages::NULLISH_TO_OBJECT));
    }
    Ok(this)
}

/// `Array.prototype.push` (section 23.1.3.23).
fn array_push(agent: &mut Agent, this: Value, start: usize, argc: usize) -> Res<Value> {
    if this.is_nullish() {
        return Err(agent.throw_error(
            ErrorType::TypeError,
            "Array.prototype.push called on null or undefined",
        ));
    }
    let length_key = agent.heap.length_key;
    let len = agent.get_value(this, length_key)?;
    let len = agent.number(len)?;
    let len = if len.is_nan() || len <= 0.0 {
        0.0
    } else {
        len.trunc().min(9_007_199_254_740_991.0)
    };
    if len + argc as f64 > 9_007_199_254_740_991.0 {
        return Err(agent.throw_error(
            ErrorType::TypeError,
            "Pushing elements past 2^53-1 length is disallowed",
        ));
    }
    for i in 0..argc {
        let v = arg(agent, start, argc, i);
        let index = len + i as f64;
        let key = if index < 4_294_967_295.0 {
            PropertyKey::Index(index as u32)
        } else {
            let text = crate::number::number_to_string(index);
            let units: Vec<u16> = text.encode_utf16().collect();
            agent.heap.atoms.key(&units)
        };
        agent.set_value(this, key, v, true)?;
    }
    let new_len = Value::number(len + argc as f64);
    agent.set_value(this, length_key, new_len, true)?;
    Ok(new_len)
}

/// `Error.prototype.toString` (section 20.5.3.4).
fn error_to_string(agent: &mut Agent, this: Value, _: usize, _: usize) -> Res<Value> {
    if this.as_object().is_none() {
        return Err(agent.throw_error(
            ErrorType::TypeError,
            "Error.prototype.toString requires that 'this' be an Object",
        ));
    }
    let (name_key, message_key) = (agent.realm().name_key, agent.realm().message_key);
    let name = agent.get_value(this, name_key)?;
    let name = if name.is_undefined() {
        "Error".to_owned()
    } else {
        let s = agent.string_value(name)?;
        String::from_utf16_lossy(&agent.string_units(s).unwrap_or_default())
    };
    let message = agent.get_value(this, message_key)?;
    let message = if message.is_undefined() {
        String::new()
    } else {
        let s = agent.string_value(message)?;
        String::from_utf16_lossy(&agent.string_units(s).unwrap_or_default())
    };
    let text = match (name.is_empty(), message.is_empty()) {
        (true, _) => message,
        (_, true) => name,
        _ => format!("{name}: {message}"),
    };
    agent.new_string(&text)
}

impl Agent {
    fn realm_mut(&mut self) -> Option<&mut Realm> {
        self.vm.realm.as_mut()
    }

    fn key_named(&mut self, name: &str) -> PropertyKey {
        let units: Vec<u16> = name.encode_utf16().collect();
        self.heap.atoms.key(&units)
    }

    /// Makes the realm on first use. Each value is stored into the realm
    /// as soon as it is made, so that it is rooted before the next
    /// allocation.
    pub(crate) fn ensure_realm(&mut self) -> Res<()> {
        if self.vm.realm.is_some() {
            return Ok(());
        }
        let u = Value::UNDEFINED;
        let strings = Strings {
            empty: u,
            undefined: u,
            null: u,
            true_: u,
            false_: u,
            number: u,
            string: u,
            boolean: u,
            symbol: u,
            bigint: u,
            function: u,
            object: u,
        };
        let realm = Realm {
            global: u,
            object_prototype: u,
            function_prototype: u,
            array_prototype: u,
            string_prototype: u,
            number_prototype: u,
            boolean_prototype: u,
            symbol_prototype: u,
            errors: [u; 7],
            lexical: HashMap::new(),
            strings,
            prototype_key: self.key_named("prototype"),
            constructor_key: self.key_named("constructor"),
            name_key: self.key_named("name"),
            message_key: self.key_named("message"),
            to_string_key: self.key_named("toString"),
            value_of_key: self.key_named("valueOf"),
        };
        self.vm.realm = Some(realm);
        macro_rules! set {
            ($field:ident $(. $sub:ident)?, $v:expr) => {{
                let v = $v;
                if let Some(r) = self.realm_mut() {
                    r.$field $(.$sub)? = v;
                }
                v
            }};
        }
        let names = [
            ("", 0),
            ("undefined", 1),
            ("null", 2),
            ("true", 3),
            ("false", 4),
            ("number", 5),
            ("string", 6),
            ("boolean", 7),
            ("symbol", 8),
            ("bigint", 9),
            ("function", 10),
            ("object", 11),
        ];
        for (text, i) in names {
            let s = self.new_string(text)?;
            if let Some(r) = self.realm_mut() {
                let st = &mut r.strings;
                *match i {
                    0 => &mut st.empty,
                    1 => &mut st.undefined,
                    2 => &mut st.null,
                    3 => &mut st.true_,
                    4 => &mut st.false_,
                    5 => &mut st.number,
                    6 => &mut st.string,
                    7 => &mut st.boolean,
                    8 => &mut st.symbol,
                    9 => &mut st.bigint,
                    10 => &mut st.function,
                    _ => &mut st.object,
                } = s;
            }
        }
        let op = set!(
            object_prototype,
            self.alloc_object(ObjectCell::new(Value::NULL, ObjectKind::Ordinary))?
        );
        let fp = set!(
            function_prototype,
            self.alloc_object(ObjectCell::function(
                op,
                Callable::Native {
                    id: FUNCTION_PROTOTYPE
                }
            ))?
        );
        self.finish_function(fp, 0, &[], false)?;
        set!(
            array_prototype,
            self.alloc_object(ObjectCell::new(op, ObjectKind::Array))?
        );
        set!(
            string_prototype,
            self.alloc_object(ObjectCell::new(op, ObjectKind::Ordinary))?
        );
        set!(
            number_prototype,
            self.alloc_object(ObjectCell::new(op, ObjectKind::Ordinary))?
        );
        set!(
            boolean_prototype,
            self.alloc_object(ObjectCell::new(op, ObjectKind::Ordinary))?
        );
        set!(
            symbol_prototype,
            self.alloc_object(ObjectCell::new(op, ObjectKind::Ordinary))?
        );
        // The error prototypes: Error.prototype, and the native errors'
        // inheriting from it, each with its name and an empty message.
        let mut error_proto = Value::NULL;
        for t in ErrorType::ALL {
            let proto = if t == ErrorType::Error {
                op
            } else {
                error_proto
            };
            let e = self.alloc_object(ObjectCell::new(proto, ObjectKind::Ordinary))?;
            if let Some(slot) = self.realm_mut().and_then(|r| r.errors.get_mut(t as usize)) {
                *slot = e;
            }
            if t == ErrorType::Error {
                error_proto = e;
            }
            let name = self.new_string(t.name())?;
            let (name_key, message_key) = (self.realm().name_key, self.realm().message_key);
            let empty = self.realm().strings.empty;
            if let Some(o) = e.as_object() {
                self.heap.define_own_property(o, name_key, hidden(name));
                self.heap.define_own_property(o, message_key, hidden(empty));
            }
        }
        let g = set!(
            global,
            self.alloc_object(ObjectCell::new(op, ObjectKind::Ordinary))?
        );
        // The natives on their prototypes.
        let ap = self.realm().array_prototype;
        let ep = self.realm().error_proto(ErrorType::Error);
        for (holder, id) in [
            (op, OBJECT_TO_STRING),
            (op, OBJECT_VALUE_OF),
            (ap, ARRAY_PUSH),
            (ep, ERROR_TO_STRING),
        ] {
            self.install_native(holder, id)?;
        }
        // The global object's own: globalThis, and the constants.
        if let Some(o) = g.as_object() {
            let key = self.key_named("globalThis");
            self.heap.define_own_property(o, key, hidden(g));
            for (name, value) in [
                ("undefined", Value::UNDEFINED),
                ("NaN", Value::double(f64::NAN)),
                ("Infinity", Value::double(f64::INFINITY)),
            ] {
                let key = self.key_named(name);
                self.heap.define_own_property(
                    o,
                    key,
                    Descriptor {
                        value: Some(value),
                        writable: Some(false),
                        enumerable: Some(false),
                        configurable: Some(false),
                        ..Descriptor::default()
                    },
                );
            }
        }
        Ok(())
    }

    /// Defines native `id` on `holder` under its name.
    fn install_native(&mut self, holder: Value, id: u32) -> Res<()> {
        let Some(&(name, length, _, _)) = NATIVES.get(id as usize) else {
            return Ok(());
        };
        let fp = self.realm().function_prototype;
        let f = self.alloc_object(ObjectCell::function(fp, Callable::Native { id }))?;
        self.vm.temps.push(f);
        let units: Vec<u16> = name.encode_utf16().collect();
        let done = self.finish_function(f, length, &units, false);
        self.vm.temps.pop();
        done?;
        let key = self.key_named(name);
        if let Some(o) = holder.as_object() {
            self.heap.define_own_property(o, key, hidden(f));
        }
        Ok(())
    }
}
