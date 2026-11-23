//! The realm (ECMA-262 section 9.3): its intrinsics, its global object
//! and its global lexical bindings. The prototypes are made here, empty;
//! the built-ins fill them from their tables ([`crate::builtins`]).

use std::collections::HashMap;

use super::messages::ErrorType;
use super::{RealmIndex, Res};
use crate::agent::Agent;
use crate::builtins::Intrinsics;
use crate::heap::ObjectCell;
use crate::object::{Callable, ObjectKind};
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
    pub errors: [Value; 8],
    pub intrinsics: Intrinsics,
    pub lexical: HashMap<PropertyKey, Lexical>,
    /// The template objects of the tagged templates run in this realm,
    /// by their site: unit, function and constant.
    pub templates: HashMap<(u32, u32, u32), Value>,
    pub strings: Strings,
    pub prototype_key: PropertyKey,
    pub constructor_key: PropertyKey,
    pub name_key: PropertyKey,
    pub message_key: PropertyKey,
    pub to_string_key: PropertyKey,
    pub value_of_key: PropertyKey,
    pub stack_trace_limit_key: PropertyKey,
    /// The iterator protocol's names.
    pub next_key: PropertyKey,
    pub done_key: PropertyKey,
    pub value_key: PropertyKey,
    pub return_key: PropertyKey,
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
        self.intrinsics.trace(out);
        out.extend(self.lexical.values().map(|b| b.value));
        out.extend(self.templates.values().copied());
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

impl Agent {
    fn key_named(&mut self, name: &str) -> PropertyKey {
        let units: Vec<u16> = name.encode_utf16().collect();
        self.heap.atoms.key(&units)
    }

    /// Makes the agent's first realm on first use.
    pub(crate) fn ensure_realm(&mut self) -> Res<()> {
        if self.vm.realms.is_empty() {
            self.create_realm()?;
        }
        Ok(())
    }

    /// CreateRealm and SetDefaultGlobalBindings (sections 9.3.1 and
    /// 9.3.3): a new realm of the agent's, its intrinsics made while it
    /// is current. The current realm is the same after.
    pub(crate) fn create_realm(&mut self) -> Res<RealmIndex> {
        let index = RealmIndex::try_from(self.vm.realms.len()).unwrap_or(RealmIndex::MAX);
        let fresh = self.new_realm_record();
        self.vm.realms.push(fresh);
        let caller = self.enter_realm(index);
        let made = self.fill_realm();
        // The first realm stays current.
        self.vm.current = if self.vm.realms.len() == 1 {
            index
        } else {
            caller
        };
        made.map(|()| index)
    }

    /// A realm with nothing made yet.
    fn new_realm_record(&mut self) -> Realm {
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
        Realm {
            global: u,
            object_prototype: u,
            function_prototype: u,
            array_prototype: u,
            string_prototype: u,
            number_prototype: u,
            boolean_prototype: u,
            symbol_prototype: u,
            errors: [u; 8],
            intrinsics: Intrinsics::NONE,
            lexical: HashMap::new(),
            templates: HashMap::new(),
            strings,
            prototype_key: self.key_named("prototype"),
            constructor_key: self.key_named("constructor"),
            name_key: self.key_named("name"),
            message_key: self.key_named("message"),
            to_string_key: self.key_named("toString"),
            value_of_key: self.key_named("valueOf"),
            stack_trace_limit_key: self.key_named("stackTraceLimit"),
            next_key: self.key_named("next"),
            done_key: self.key_named("done"),
            value_key: self.key_named("value"),
            return_key: self.key_named("return"),
        }
    }

    /// Makes the current realm's objects. Each value is stored into the
    /// realm as soon as it is made, so that it is rooted before the next
    /// allocation.
    fn fill_realm(&mut self) -> Res<()> {
        macro_rules! set {
            ($field:ident, $v:expr) => {{
                let v = $v;
                if let Some(r) = self.realm_mut() {
                    r.$field = v;
                }
                v
            }};
        }
        let names = [
            "",
            "undefined",
            "null",
            "true",
            "false",
            "number",
            "string",
            "boolean",
            "symbol",
            "bigint",
            "function",
            "object",
        ];
        for (i, text) in names.into_iter().enumerate() {
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
        // `Function.prototype` is itself a function, returning undefined.
        let fp = set!(
            function_prototype,
            self.alloc_object(ObjectCell::function(
                op,
                Callable::Native {
                    id: crate::builtins::function_prototype_id(),
                    realm: self.vm.current,
                }
            ))?
        );
        self.finish_function(fp, 0, &[], false)?;
        set!(
            array_prototype,
            self.alloc_object(ObjectCell::new(op, ObjectKind::Array))?
        );
        // The prototypes of the primitives are wrappers of a primitive
        // (`String.prototype` is the empty string's).
        let empty = self.realm().strings.empty;
        set!(
            string_prototype,
            self.alloc_object(ObjectCell::with_internal(
                op,
                ObjectKind::String,
                vec![empty]
            ))?
        );
        set!(
            number_prototype,
            self.alloc_object(ObjectCell::with_internal(
                op,
                ObjectKind::Number,
                vec![Value::int(0)]
            ))?
        );
        set!(
            boolean_prototype,
            self.alloc_object(ObjectCell::with_internal(
                op,
                ObjectKind::Boolean,
                vec![Value::FALSE]
            ))?
        );
        set!(
            symbol_prototype,
            self.alloc_object(ObjectCell::new(op, ObjectKind::Ordinary))?
        );
        // The error prototypes: Error.prototype, and the native errors'
        // inheriting from it.
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
        }
        set!(
            global,
            self.alloc_object(ObjectCell::new(op, ObjectKind::Ordinary))?
        );
        self.make_intrinsics()
    }
}
