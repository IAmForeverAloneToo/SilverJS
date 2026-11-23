//! The realm's making: the well-known symbols, the constructors and the
//! global object, in the reference browser's order; and the global
//! functions (`eval`, `isNaN`, `isFinite`, `parseInt`, `parseFloat`).

use super::{Call, Entry, FIXED, HIDDEN, Name, What, ctor, entry, id_of, method, table};
use crate::agent::Agent;
use crate::heap::ObjectCell;
use crate::interp::messages::ErrorType;
use crate::interp::ops::string_to_number;
use crate::interp::{Abrupt, Completion, Res};
use crate::object::{Descriptor, ObjectKind};
use crate::value::Value;

/// The well-known symbols (section 6.1.5.1), in the order the reference
/// browser's `Symbol` lists them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Wk {
    AsyncIterator,
    HasInstance,
    IsConcatSpreadable,
    Iterator,
    Match,
    MatchAll,
    Replace,
    Search,
    Species,
    Split,
    ToPrimitive,
    ToStringTag,
    Unscopables,
    Dispose,
    AsyncDispose,
}

impl Wk {
    pub(crate) const ALL: [Wk; 15] = [
        Wk::AsyncIterator,
        Wk::HasInstance,
        Wk::IsConcatSpreadable,
        Wk::Iterator,
        Wk::Match,
        Wk::MatchAll,
        Wk::Replace,
        Wk::Search,
        Wk::Species,
        Wk::Split,
        Wk::ToPrimitive,
        Wk::ToStringTag,
        Wk::Unscopables,
        Wk::Dispose,
        Wk::AsyncDispose,
    ];

    /// Its property name on `Symbol`.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Wk::AsyncIterator => "asyncIterator",
            Wk::HasInstance => "hasInstance",
            Wk::IsConcatSpreadable => "isConcatSpreadable",
            Wk::Iterator => "iterator",
            Wk::Match => "match",
            Wk::MatchAll => "matchAll",
            Wk::Replace => "replace",
            Wk::Search => "search",
            Wk::Species => "species",
            Wk::Split => "split",
            Wk::ToPrimitive => "toPrimitive",
            Wk::ToStringTag => "toStringTag",
            Wk::Unscopables => "unscopables",
            Wk::Dispose => "dispose",
            Wk::AsyncDispose => "asyncDispose",
        }
    }

    /// Its `[[Description]]`: `Symbol.iterator`.
    pub(crate) fn description(self) -> String {
        format!("Symbol.{}", self.name())
    }
}

/// The realm's intrinsics beyond the prototypes.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Intrinsics {
    pub object: Value,
    pub function: Value,
    pub array: Value,
    pub string: Value,
    pub number: Value,
    pub boolean: Value,
    pub symbol: Value,
    /// The error constructors, by [`ErrorType`].
    pub errors: [Value; 8],
    pub throw_type_error: Value,
    /// Every error's `stack` accessor pair.
    pub stack_get: Value,
    pub stack_set: Value,
    pub call_site_prototype: Value,
    pub math: Value,
    pub reflect: Value,
    pub eval: Value,
    pub parse_int: Value,
    pub parse_float: Value,
    pub iterator_prototype: Value,
    pub array_iterator_prototype: Value,
    pub string_iterator_prototype: Value,
    /// `Array.prototype.values`, an arguments object's `@@iterator`.
    pub array_values: Value,
    pub generator_prototype: Value,
    pub generator_function_prototype: Value,
    pub generator_function: Value,
    pub promise: Value,
    pub promise_prototype: Value,
    pub async_function_prototype: Value,
    pub async_function: Value,
    pub async_iterator_prototype: Value,
    pub async_generator_prototype: Value,
    pub async_generator_function_prototype: Value,
    pub async_generator_function: Value,
    pub async_from_sync_iterator_prototype: Value,
}

impl Intrinsics {
    pub(crate) const NONE: Intrinsics = Intrinsics {
        object: Value::UNDEFINED,
        function: Value::UNDEFINED,
        array: Value::UNDEFINED,
        string: Value::UNDEFINED,
        number: Value::UNDEFINED,
        boolean: Value::UNDEFINED,
        symbol: Value::UNDEFINED,
        errors: [Value::UNDEFINED; 8],
        throw_type_error: Value::UNDEFINED,
        stack_get: Value::UNDEFINED,
        stack_set: Value::UNDEFINED,
        call_site_prototype: Value::UNDEFINED,
        math: Value::UNDEFINED,
        reflect: Value::UNDEFINED,
        eval: Value::UNDEFINED,
        parse_int: Value::UNDEFINED,
        parse_float: Value::UNDEFINED,
        iterator_prototype: Value::UNDEFINED,
        array_iterator_prototype: Value::UNDEFINED,
        string_iterator_prototype: Value::UNDEFINED,
        array_values: Value::UNDEFINED,
        generator_prototype: Value::UNDEFINED,
        generator_function_prototype: Value::UNDEFINED,
        generator_function: Value::UNDEFINED,
        promise: Value::UNDEFINED,
        promise_prototype: Value::UNDEFINED,
        async_function_prototype: Value::UNDEFINED,
        async_function: Value::UNDEFINED,
        async_iterator_prototype: Value::UNDEFINED,
        async_generator_prototype: Value::UNDEFINED,
        async_generator_function_prototype: Value::UNDEFINED,
        async_generator_function: Value::UNDEFINED,
        async_from_sync_iterator_prototype: Value::UNDEFINED,
    };

    pub(crate) fn trace(&self, out: &mut Vec<Value>) {
        out.extend([
            self.object,
            self.function,
            self.array,
            self.string,
            self.number,
            self.boolean,
            self.symbol,
            self.throw_type_error,
            self.stack_get,
            self.stack_set,
            self.call_site_prototype,
            self.math,
            self.reflect,
            self.eval,
            self.parse_int,
            self.parse_float,
            self.iterator_prototype,
            self.array_iterator_prototype,
            self.string_iterator_prototype,
            self.array_values,
            self.generator_prototype,
            self.generator_function_prototype,
            self.generator_function,
            self.promise,
            self.promise_prototype,
            self.async_function_prototype,
            self.async_function,
            self.async_iterator_prototype,
            self.async_generator_prototype,
            self.async_generator_function_prototype,
            self.async_generator_function,
            self.async_from_sync_iterator_prototype,
        ]);
        out.extend_from_slice(&self.errors);
    }
}

/// The constructors, and the functions the realm holds on its own
/// (`Function.prototype` itself, `%ThrowTypeError%`): made by name.
pub(crate) const CTORS: &[Entry] = &[
    ctor("Object", 1, super::object::construct),
    ctor("Function", 1, super::function::construct),
    ctor("GeneratorFunction", 1, super::function::construct_generator),
    ctor("AsyncFunction", 1, super::function::construct_async),
    ctor(
        "AsyncGeneratorFunction",
        1,
        super::function::construct_async_generator,
    ),
    ctor("Array", 1, super::array::construct),
    ctor("Number", 1, super::number::construct),
    ctor("Boolean", 1, super::primitive::construct_boolean),
    ctor("String", 1, super::string::construct),
    ctor("Symbol", 0, super::primitive::construct_symbol),
    ctor("Error", 1, super::error::construct_error),
    ctor("EvalError", 1, super::error::construct_eval_error),
    ctor("RangeError", 1, super::error::construct_range_error),
    ctor("ReferenceError", 1, super::error::construct_reference_error),
    ctor("SyntaxError", 1, super::error::construct_syntax_error),
    ctor("TypeError", 1, super::error::construct_type_error),
    ctor("URIError", 1, super::error::construct_uri_error),
    ctor("AggregateError", 2, super::error::construct_aggregate_error),
    ctor("Promise", 1, super::promise::construct),
    method("FunctionPrototype", 0, super::function::function_prototype),
    method("ThrowTypeError", 0, super::function::throw_type_error),
    method("parseInt", 2, parse_int),
    method("parseFloat", 1, parse_float),
];

const fn intrinsic(name: &'static str) -> Entry {
    entry(Name::Str(name), What::Intrinsic(name), HIDDEN)
}

/// The global object's own properties, in the reference browser's order
/// (its web APIs, the embedder's, come after).
pub(crate) const GLOBAL: &[Entry] = &[
    intrinsic("Object"),
    intrinsic("Function"),
    intrinsic("Array"),
    intrinsic("Number"),
    intrinsic("parseFloat"),
    intrinsic("parseInt"),
    entry(Name::Str("Infinity"), What::Number(f64::INFINITY), FIXED),
    entry(Name::Str("NaN"), What::Number(f64::NAN), FIXED),
    entry(Name::Str("undefined"), What::Intrinsic("undefined"), FIXED),
    intrinsic("Boolean"),
    intrinsic("String"),
    intrinsic("Symbol"),
    intrinsic("Promise"),
    intrinsic("Error"),
    intrinsic("AggregateError"),
    intrinsic("EvalError"),
    intrinsic("RangeError"),
    intrinsic("ReferenceError"),
    intrinsic("SyntaxError"),
    intrinsic("TypeError"),
    intrinsic("URIError"),
    intrinsic("globalThis"),
    intrinsic("Math"),
    intrinsic("Reflect"),
    method("decodeURI", 1, super::uri::decode_uri),
    method("decodeURIComponent", 1, super::uri::decode_uri_component),
    method("encodeURI", 1, super::uri::encode_uri),
    method("encodeURIComponent", 1, super::uri::encode_uri_component),
    method("escape", 1, super::uri::escape),
    method("unescape", 1, super::uri::unescape),
    method("eval", 1, eval),
    method("isFinite", 1, is_finite),
    method("isNaN", 1, is_nan),
];

impl Agent {
    /// The intrinsic the global object's table names.
    pub(crate) fn intrinsic_named(&self, name: &str) -> Value {
        let r = self.realm();
        let i = &r.intrinsics;
        match name {
            "Object" => i.object,
            "Function" => i.function,
            "Array" => i.array,
            "Number" => i.number,
            "Boolean" => i.boolean,
            "String" => i.string,
            "Symbol" => i.symbol,
            "Promise" => i.promise,
            "Math" => i.math,
            "Reflect" => i.reflect,
            "parseInt" => i.parse_int,
            "parseFloat" => i.parse_float,
            "globalThis" => r.global,
            _ => ErrorType::ALL
                .iter()
                .find(|t| t.name() == name)
                .and_then(|t| i.errors.get(*t as usize))
                .copied()
                .unwrap_or(Value::UNDEFINED),
        }
    }

    fn set_intrinsics(&mut self, f: impl FnOnce(&mut Intrinsics)) {
        if let Some(r) = self.realm_mut() {
            f(&mut r.intrinsics);
        }
    }

    /// Makes the agent's well-known symbols, and its own hidden one,
    /// once.
    fn ensure_well_known(&mut self) -> Res<()> {
        if !self.vm.well_known.is_empty() {
            return Ok(());
        }
        let hidden = self.new_symbol(Value::UNDEFINED)?;
        self.vm.stack_symbol = hidden;
        if let Some((_, i)) = hidden.as_reference() {
            self.heap.hidden_symbol = Some(i);
        }
        for w in Wk::ALL {
            let d = self.new_string(&w.description())?;
            self.vm.temps.push(d);
            let s = self.new_symbol(d);
            self.vm.temps.pop();
            self.vm.well_known.push(s?);
        }
        Ok(())
    }

    /// Makes the realm's intrinsics and global object (the realm's
    /// prototypes were made empty, the strings too): each value stored
    /// into the realm as soon as it is made, so that it is rooted before
    /// the next allocation.
    pub(crate) fn make_intrinsics(&mut self) -> Res<()> {
        self.ensure_well_known()?;
        let op = self.realm().object_prototype;
        let fp = self.realm().function_prototype;
        // %ThrowTypeError% (section 10.2.4.1): frozen, its `length` and
        // `name` not configurable.
        let id = id_of(table::CTORS, "ThrowTypeError").unwrap_or(0);
        let t = self.native_function(id, 0, "")?;
        self.set_intrinsics(|i| i.throw_type_error = t);
        if let Some(o) = t.as_object() {
            for key in [self.heap.length_key, self.realm().name_key] {
                self.heap.define_own_property(
                    o,
                    key,
                    Descriptor {
                        configurable: Some(false),
                        ..Descriptor::default()
                    },
                );
            }
            self.heap.prevent_extensions(o);
        }
        // Every error's `stack` accessor pair.
        for (i, length) in [(0, 0), (1, 1)] {
            let f = self.native_function(super::native_id(table::STACK, i), length, "")?;
            self.set_intrinsics(|n| {
                if i == 0 {
                    n.stack_get = f;
                } else {
                    n.stack_set = f;
                }
            });
        }
        // `CallSite.prototype`, which `Error.prepareStackTrace` sees.
        let sp = self.alloc_object(ObjectCell::new(op, ObjectKind::Ordinary))?;
        self.set_intrinsics(|n| n.call_site_prototype = sp);
        let id = super::native_id(table::CALL_SITE_CTOR, 0);
        let site_ctor = self.native_function(id, 0, "CallSite")?;
        self.vm.temps.push(site_ctor);
        if let Some(o) = site_ctor.as_object() {
            let key = self.realm().prototype_key;
            self.heap.define_own_property(
                o,
                key,
                Descriptor {
                    value: Some(sp),
                    writable: Some(false),
                    enumerable: Some(false),
                    configurable: Some(false),
                    ..Descriptor::default()
                },
            );
        }
        let done = self.install_table(sp, table::CALL_SITE, site_ctor, &[]);
        self.vm.temps.pop();
        done?;
        // The constructors.
        let r = self.realm();
        let protos = [
            ("Object", r.object_prototype),
            ("Function", fp),
            ("Array", r.array_prototype),
            ("Number", r.number_prototype),
            ("Boolean", r.boolean_prototype),
            ("String", r.string_prototype),
            ("Symbol", r.symbol_prototype),
        ];
        for (name, proto) in protos {
            let c = self.make_ctor(name, proto)?;
            self.set_intrinsics(|i| match name {
                "Object" => i.object = c,
                "Function" => i.function = c,
                "Array" => i.array = c,
                "Number" => i.number = c,
                "Boolean" => i.boolean = c,
                "String" => i.string = c,
                _ => i.symbol = c,
            });
        }
        let mut error_ctor = Value::UNDEFINED;
        for t in ErrorType::ALL {
            let proto = self.realm().error_proto(t);
            let c = self.make_ctor(t.name(), proto)?;
            self.set_intrinsics(|i| {
                if let Some(slot) = i.errors.get_mut(t as usize) {
                    *slot = c;
                }
            });
            if t == ErrorType::Error {
                error_ctor = c;
            } else if let Some(o) = c.as_object() {
                // A native error's constructor inherits from `Error`.
                self.heap.set_prototype_of(o, error_ctor);
            }
        }
        for (name, length) in [("parseInt", 2), ("parseFloat", 1)] {
            let id = id_of(table::CTORS, name).unwrap_or(0);
            let f = self.native_function(id, length, name)?;
            self.set_intrinsics(|i| {
                if name == "parseInt" {
                    i.parse_int = f;
                } else {
                    i.parse_float = f;
                }
            });
        }
        // The iterator prototypes: %IteratorPrototype% and those that
        // inherit from it.
        let ip = self.alloc_object(ObjectCell::new(op, ObjectKind::Ordinary))?;
        self.set_intrinsics(|i| i.iterator_prototype = ip);
        let aip = self.alloc_object(ObjectCell::new(ip, ObjectKind::Ordinary))?;
        self.set_intrinsics(|i| i.array_iterator_prototype = aip);
        let sip = self.alloc_object(ObjectCell::new(ip, ObjectKind::Ordinary))?;
        self.set_intrinsics(|i| i.string_iterator_prototype = sip);
        // The generators': %GeneratorPrototype% inherits from
        // %IteratorPrototype%, %GeneratorFunction.prototype% from
        // Function.prototype, %GeneratorFunction% from Function.
        let gp = self.alloc_object(ObjectCell::new(ip, ObjectKind::Ordinary))?;
        self.set_intrinsics(|i| i.generator_prototype = gp);
        let gfp = self.alloc_object(ObjectCell::new(fp, ObjectKind::Ordinary))?;
        self.set_intrinsics(|i| i.generator_function_prototype = gfp);
        let gf = self.make_ctor("GeneratorFunction", gfp)?;
        self.set_intrinsics(|i| i.generator_function = gf);
        let function = self.realm().intrinsics.function;
        if let Some(o) = gf.as_object() {
            self.heap.set_prototype_of(o, function);
        }
        // %AsyncFunction.prototype% inherits from Function.prototype,
        // %AsyncFunction% from Function.
        let afp = self.alloc_object(ObjectCell::new(fp, ObjectKind::Ordinary))?;
        self.set_intrinsics(|i| i.async_function_prototype = afp);
        let af = self.make_ctor("AsyncFunction", afp)?;
        self.set_intrinsics(|i| i.async_function = af);
        if let Some(o) = af.as_object() {
            self.heap.set_prototype_of(o, function);
        }
        // The async generators': %AsyncIteratorPrototype% inherits from
        // Object.prototype, %AsyncGeneratorPrototype% from it,
        // %AsyncGeneratorFunction.prototype% from Function.prototype.
        let aip = self.alloc_object(ObjectCell::new(op, ObjectKind::Ordinary))?;
        self.set_intrinsics(|i| i.async_iterator_prototype = aip);
        let agp = self.alloc_object(ObjectCell::new(aip, ObjectKind::Ordinary))?;
        self.set_intrinsics(|i| i.async_generator_prototype = agp);
        let afs = self.alloc_object(ObjectCell::new(aip, ObjectKind::Ordinary))?;
        self.set_intrinsics(|i| i.async_from_sync_iterator_prototype = afs);
        let agfp = self.alloc_object(ObjectCell::new(fp, ObjectKind::Ordinary))?;
        self.set_intrinsics(|i| i.async_generator_function_prototype = agfp);
        let agf = self.make_ctor("AsyncGeneratorFunction", agfp)?;
        self.set_intrinsics(|i| i.async_generator_function = agf);
        if let Some(o) = agf.as_object() {
            self.heap.set_prototype_of(o, function);
        }
        // Promise and its prototype.
        let pp = self.alloc_object(ObjectCell::new(op, ObjectKind::Ordinary))?;
        self.set_intrinsics(|i| i.promise_prototype = pp);
        let promise = self.make_ctor("Promise", pp)?;
        self.set_intrinsics(|i| i.promise = promise);
        let math = self.alloc_object(ObjectCell::new(op, ObjectKind::Ordinary))?;
        self.set_intrinsics(|i| i.math = math);
        let reflect = self.alloc_object(ObjectCell::new(op, ObjectKind::Ordinary))?;
        self.set_intrinsics(|i| i.reflect = reflect);
        // The tables.
        let i = self.realm().intrinsics;
        let r = self.realm();
        let (ap, sp, np, bp, yp) = (
            r.array_prototype,
            r.string_prototype,
            r.number_prototype,
            r.boolean_prototype,
            r.symbol_prototype,
        );
        self.install_table(i.object, table::OBJECT, Value::UNDEFINED, &[])?;
        self.install_table(op, table::OBJECT_PROTO, i.object, &[])?;
        self.install_table(fp, table::FUNCTION_PROTO, i.function, &[])?;
        self.install_table(i.array, table::ARRAY, Value::UNDEFINED, &[])?;
        let unscopables = super::array::make_unscopables(self)?;
        self.vm.temps.push(unscopables);
        let done = self.install_table(ap, table::ARRAY_PROTO, i.array, &[unscopables]);
        self.vm.temps.pop();
        done?;
        let key = self.name_key(super::Name::Str("values"));
        let values = match ap
            .as_object()
            .and_then(|o| self.heap.get_own_property(o, key))
        {
            Some(crate::object::OwnProperty::Data { value, .. }) => value,
            _ => Value::UNDEFINED,
        };
        self.set_intrinsics(|n| n.array_values = values);
        self.install_table(
            i.iterator_prototype,
            table::ITERATOR_PROTO,
            Value::UNDEFINED,
            &[],
        )?;
        self.install_table(
            i.array_iterator_prototype,
            table::ARRAY_ITERATOR_PROTO,
            Value::UNDEFINED,
            &[],
        )?;
        self.install_table(
            i.string_iterator_prototype,
            table::STRING_ITERATOR_PROTO,
            Value::UNDEFINED,
            &[],
        )?;
        self.install_table(
            i.generator_prototype,
            table::GENERATOR_PROTO,
            Value::UNDEFINED,
            &[i.generator_function_prototype],
        )?;
        self.install_table(
            i.generator_function_prototype,
            table::GENERATOR_FUNCTION_PROTO,
            Value::UNDEFINED,
            &[i.generator_prototype, i.generator_function],
        )?;
        self.install_table(
            i.async_function_prototype,
            table::ASYNC_FUNCTION_PROTO,
            Value::UNDEFINED,
            &[i.async_function],
        )?;
        self.install_table(
            i.async_iterator_prototype,
            table::ASYNC_ITERATOR_PROTO,
            Value::UNDEFINED,
            &[],
        )?;
        self.install_table(
            i.async_from_sync_iterator_prototype,
            table::ASYNC_FROM_SYNC_PROTO,
            Value::UNDEFINED,
            &[],
        )?;
        self.install_table(
            i.async_generator_prototype,
            table::ASYNC_GENERATOR_PROTO,
            Value::UNDEFINED,
            &[i.async_generator_function_prototype],
        )?;
        self.install_table(
            i.async_generator_function_prototype,
            table::ASYNC_GENERATOR_FUNCTION_PROTO,
            Value::UNDEFINED,
            &[i.async_generator_prototype, i.async_generator_function],
        )?;
        self.install_table(i.promise, table::PROMISE, Value::UNDEFINED, &[])?;
        self.install_table(
            i.promise_prototype,
            table::PROMISE_PROTO,
            Value::UNDEFINED,
            &[i.promise],
        )?;
        self.install_table(i.string, table::STRING, Value::UNDEFINED, &[])?;
        self.install_table(sp, table::STRING_PROTO, i.string, &[])?;
        self.install_table(i.number, table::NUMBER, Value::UNDEFINED, &[])?;
        self.install_table(np, table::NUMBER_PROTO, i.number, &[])?;
        self.install_table(bp, table::BOOLEAN_PROTO, i.boolean, &[])?;
        self.install_table(i.symbol, table::SYMBOL, Value::UNDEFINED, &[])?;
        self.install_table(yp, table::SYMBOL_PROTO, i.symbol, &[])?;
        self.install_table(math, table::MATH, Value::UNDEFINED, &[])?;
        self.install_table(reflect, table::REFLECT, Value::UNDEFINED, &[])?;
        let empty = self.realm().strings.empty;
        for t in ErrorType::ALL {
            let proto = self.realm().error_proto(t);
            let c = i
                .errors
                .get(t as usize)
                .copied()
                .unwrap_or(Value::UNDEFINED);
            let name = self.new_string(t.name())?;
            self.vm.temps.push(name);
            let done = if t == ErrorType::Error {
                self.install_table(c, table::ERROR, Value::UNDEFINED, &[])
                    .and_then(|()| self.install_table(proto, table::ERROR_PROTO, c, &[name, empty]))
            } else {
                self.install_table(proto, table::NATIVE_ERROR_PROTO, c, &[name, empty])
            };
            self.vm.temps.pop();
            done?;
        }
        let g = self.realm().global;
        self.install_table(g, table::GLOBAL, Value::UNDEFINED, &[])?;
        let key = self.name_key(Name::Str("eval"));
        let eval = match g.as_object().map(|o| self.heap.get(o, key)) {
            Some(crate::object::Got::Value(v)) => v,
            _ => Value::UNDEFINED,
        };
        self.set_intrinsics(|i| i.eval = eval);
        Ok(())
    }

    /// Where the native now running was called from: the frame below it,
    /// and where that frame stands.
    pub(crate) fn caller_position(&self) -> Option<(u32, u32, u32)> {
        let frames = u32::try_from(self.vm.frames.len()).unwrap_or(u32::MAX);
        let top = self.vm.frames.last()?;
        let pc = if self.vm.natives.last().is_some_and(|n| n.below == frames) {
            top.pc
        } else {
            self.vm.pc
        };
        Some((top.unit, top.func, pc))
    }

    /// Runs `source` as a script in the realm, from a native: indirect
    /// `eval` (section 19.2.1.1), the `Function` constructor's code and
    /// the embedder's `evalScript`, each with its origin.
    pub(crate) fn eval_script(
        &mut self,
        source: &str,
        origin: crate::interp::Origin,
    ) -> Res<Value> {
        let unit = match crate::compiler::compile(source, crate::parser::Goal::Script) {
            Ok(u) => u,
            Err(crate::compiler::CompileError::Syntax(e)) => {
                return Err(self.throw_error(ErrorType::SyntaxError, e.message));
            }
            Err(e) => return Err(self.throw_error(ErrorType::SyntaxError, e.to_string())),
        };
        if self.vm.reentry >= crate::interp::MAX_REENTRY {
            return Err(self.range_error(crate::interp::messages::STACK_OVERFLOW));
        }
        self.vm.reentry += 1;
        let done = self.run_compiled(unit, origin);
        self.vm.reentry -= 1;
        match done {
            Completion::Normal(v) => Ok(v),
            Completion::Threw(e) => {
                self.vm.exception = e;
                Err(Abrupt::Throw)
            }
            Completion::Terminated => Err(Abrupt::Terminate),
        }
    }
}

/// `eval` called other than directly: a script in the global scope.
fn eval(agent: &mut Agent, c: Call) -> Res<Value> {
    let x = agent.arg(c, 0);
    if x.as_string().is_none() {
        return Ok(x);
    }
    agent.indirect_eval(x)
}

fn is_nan(agent: &mut Agent, c: Call) -> Res<Value> {
    let n = agent.number(agent.arg(c, 0))?;
    Ok(Value::bool(n.is_nan()))
}

fn is_finite(agent: &mut Agent, c: Call) -> Res<Value> {
    let n = agent.number(agent.arg(c, 0))?;
    Ok(Value::bool(n.is_finite()))
}

fn is_space(u: u16) -> bool {
    matches!(
        u,
        0x09 | 0x0a | 0x0b | 0x0c | 0x0d | 0x20 | 0xa0 | 0x1680 | 0x2000
            ..=0x200a | 0x2028 | 0x2029 | 0x202f | 0x205f | 0x3000 | 0xfeff
    )
}

/// The units of `s` without leading white space and line terminators.
pub(crate) fn trim_start(s: &[u16]) -> &[u16] {
    let start = s.iter().position(|&u| !is_space(u)).unwrap_or(s.len());
    s.get(start..).unwrap_or(&[])
}

pub(crate) fn trim_end(s: &[u16]) -> &[u16] {
    let end = s.iter().rposition(|&u| !is_space(u)).map_or(0, |i| i + 1);
    s.get(..end).unwrap_or(&[])
}

/// `parseInt` (section 19.2.5).
pub(crate) fn parse_int(agent: &mut Agent, c: Call) -> Res<Value> {
    let units = agent.text_units(agent.arg(c, 0))?;
    let radix = agent.arg(c, 1);
    let radix = crate::interp::ops::to_int32(agent.number(radix)?);
    let mut s = trim_start(&units);
    let mut sign = 1.0;
    if let Some(&u) = s.first()
        && (u == u16::from(b'-') || u == u16::from(b'+'))
    {
        if u == u16::from(b'-') {
            sign = -1.0;
        }
        s = s.get(1..).unwrap_or(&[]);
    }
    let mut r = radix;
    let mut strip = true;
    if r != 0 {
        if !(2..=36).contains(&r) {
            return Ok(Value::number(f64::NAN));
        }
        if r != 16 {
            strip = false;
        }
    } else {
        r = 10;
    }
    if strip
        && s.len() >= 2
        && s.first() == Some(&u16::from(b'0'))
        && matches!(s.get(1), Some(&x) if x == u16::from(b'x') || x == u16::from(b'X'))
    {
        s = s.get(2..).unwrap_or(&[]);
        r = 16;
    }
    let digit = |u: u16| -> Option<u32> {
        let d = char::from_u32(u32::from(u))?.to_digit(36)?;
        (d < r as u32).then_some(d)
    };
    let end = s
        .iter()
        .position(|&u| digit(u).is_none())
        .unwrap_or(s.len());
    let digits = s.get(..end).unwrap_or(&[]);
    if digits.is_empty() {
        return Ok(Value::number(f64::NAN));
    }
    let n = if r == 10 {
        // Decimal digits read as a decimal literal, correctly rounded.
        let text: String = digits.iter().map(|&u| char::from(u as u8)).collect();
        text.parse::<f64>().unwrap_or(f64::NAN)
    } else {
        let mut n = 0.0f64;
        for &u in digits {
            n = n * f64::from(r) + f64::from(digit(u).unwrap_or(0));
        }
        n
    };
    Ok(Value::number(sign * n))
}

/// `parseFloat` (section 19.2.4).
pub(crate) fn parse_float(agent: &mut Agent, c: Call) -> Res<Value> {
    let units = agent.text_units(agent.arg(c, 0))?;
    let s = trim_start(&units);
    // The longest prefix that is a StrDecimalLiteral.
    let at = |i: usize| s.get(i).copied().unwrap_or(0);
    let mut i = 0;
    if at(0) == u16::from(b'+') || at(0) == u16::from(b'-') {
        i = 1;
    }
    let inf: Vec<u16> = "Infinity".encode_utf16().collect();
    if s.get(i..i + inf.len()) == Some(&inf[..]) {
        let neg = at(0) == u16::from(b'-');
        return Ok(Value::number(if neg {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        }));
    }
    let is_digit = |u: u16| (u16::from(b'0')..=u16::from(b'9')).contains(&u);
    let int_start = i;
    while is_digit(at(i)) {
        i += 1;
    }
    let mut digits = i - int_start;
    if at(i) == u16::from(b'.') {
        let mut j = i + 1;
        while is_digit(at(j)) {
            j += 1;
        }
        digits += j - i - 1;
        if digits > 0 {
            i = j;
        }
    }
    if digits == 0 {
        return Ok(Value::number(f64::NAN));
    }
    if at(i) == u16::from(b'e') || at(i) == u16::from(b'E') {
        let mut j = i + 1;
        if at(j) == u16::from(b'+') || at(j) == u16::from(b'-') {
            j += 1;
        }
        if is_digit(at(j)) {
            while is_digit(at(j)) {
                j += 1;
            }
            i = j;
        }
    }
    Ok(Value::number(string_to_number(s.get(..i).unwrap_or(&[]))))
}

/// The realm's own function objects are made by `ensure_realm`, which
/// calls this for `Function.prototype` itself.
pub(crate) fn function_prototype_id() -> u32 {
    id_of(table::CTORS, "FunctionPrototype").unwrap_or(0)
}
