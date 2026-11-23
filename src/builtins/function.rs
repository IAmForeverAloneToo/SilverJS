//! `Function` (ECMA-262 section 20.2) and bound functions (section
//! 10.4.1).

use super::{CONFIGURABLE, CONSTRUCTOR, Call, Entry, FIXED, Name, What, Wk, entry, method};
use crate::agent::Agent;
use crate::ast::{Expr, Stmt};
use crate::heap::ObjectCell;
use crate::interp::Res;
use crate::interp::ops::hidden_readonly;
use crate::object::Callable;
use crate::parser::Goal;
use crate::value::Value;

pub(crate) const PROTO: &[Entry] = &[
    CONSTRUCTOR,
    method("apply", 2, apply),
    method("bind", 1, bind),
    method("call", 1, call),
    method("toString", 0, to_string),
    entry(
        Name::Str("arguments"),
        What::Accessor(legacy_get, legacy_set),
        CONFIGURABLE,
    ),
    entry(
        Name::Str("caller"),
        What::Accessor(legacy_get, legacy_set),
        CONFIGURABLE,
    ),
    entry(
        Name::Sym(Wk::HasInstance),
        What::Method(1, has_instance),
        FIXED,
    ),
];

/// `Function.prototype` itself: returns undefined.
pub(crate) fn function_prototype(_: &mut Agent, _: Call) -> Res<Value> {
    Ok(Value::UNDEFINED)
}

/// `%ThrowTypeError%` (section 10.2.4.1).
pub(crate) fn throw_type_error(agent: &mut Agent, _: Call) -> Res<Value> {
    Err(agent.type_error(
        "'caller', 'callee', and 'arguments' properties may not be accessed on strict mode functions or the arguments objects for calls to them",
    ))
}

/// Whether `f` is a sloppy ordinary function, whose legacy `caller` and
/// `arguments` the reference browser answers rather than throws for.
fn is_sloppy_function(agent: &Agent, f: Value) -> bool {
    match agent.callable(f) {
        Some(Callable::Code { unit, func, .. }) => agent
            .vm
            .units
            .get(unit as usize)
            .and_then(|u| u.code.functions.get(func as usize))
            .is_some_and(|f| !f.strict && f.kind == crate::compiler::FunctionKind::Normal),
        _ => false,
    }
}

/// `Function.prototype.caller` and `arguments`, as the reference browser
/// has them (docs/chrome-surface.md): accessors of their own (`get
/// caller` and the like, not `%ThrowTypeError%`), answering null for a
/// sloppy function (no caller is exposed) and throwing for the rest.
fn legacy_get(agent: &mut Agent, c: Call) -> Res<Value> {
    if is_sloppy_function(agent, c.this) {
        Ok(Value::NULL)
    } else {
        throw_type_error(agent, c)
    }
}

/// Their setters: nothing for a sloppy function.
fn legacy_set(agent: &mut Agent, c: Call) -> Res<Value> {
    if is_sloppy_function(agent, c.this) {
        Ok(Value::UNDEFINED)
    } else {
        throw_type_error(agent, c)
    }
}

/// Whether `source` is one parenthesized function expression and
/// nothing else, so that a `Function` constructor's parameters or body
/// cannot close the function and go on.
fn one_function(source: &str) -> bool {
    let Ok(program) = crate::parser::parse(source, Goal::Script) else {
        return false;
    };
    let [s] = program.body.as_slice() else {
        return false;
    };
    let stmt = program.ast.stmt(*s);
    let Stmt::Expr(e) = stmt.kind else {
        return false;
    };
    let mut e = program.ast.expr(e);
    if let Expr::Paren(inner) = e.kind {
        e = program.ast.expr(inner);
    }
    matches!(e.kind, Expr::Function(_)) && stmt.span.end as usize >= source.trim_end().len()
}

/// `Function(...)`: CreateDynamicFunction (section 20.2.1.1.1) for an
/// ordinary function, the source text the reference browser gives it
/// `function anonymous(a,b\n) {\nbody\n}`.
pub(crate) fn construct(agent: &mut Agent, c: Call) -> Res<Value> {
    dynamic_function(agent, c, "function", |r| r.function_prototype)
}

/// `GeneratorFunction(...)`: the same for a generator,
/// `function* anonymous(...`.
pub(crate) fn construct_generator(agent: &mut Agent, c: Call) -> Res<Value> {
    dynamic_function(agent, c, "function*", |r| {
        r.intrinsics.generator_function_prototype
    })
}

pub(crate) fn construct_async(agent: &mut Agent, c: Call) -> Res<Value> {
    dynamic_function(agent, c, "async function", |r| {
        r.intrinsics.async_function_prototype
    })
}

pub(crate) fn construct_async_generator(agent: &mut Agent, c: Call) -> Res<Value> {
    dynamic_function(agent, c, "async function*", |r| {
        r.intrinsics.async_generator_function_prototype
    })
}

fn dynamic_function(
    agent: &mut Agent,
    c: Call,
    keyword: &str,
    default_proto: fn(&crate::interp::Realm) -> Value,
) -> Res<Value> {
    let mut params = Vec::new();
    for i in 0..c.argc.saturating_sub(1) {
        let v = agent.arg(c, i);
        params.push(String::from_utf16_lossy(&agent.text_units(v)?));
    }
    let body = if c.argc == 0 {
        String::new()
    } else {
        let v = agent.arg(c, c.argc - 1);
        String::from_utf16_lossy(&agent.text_units(v)?)
    };
    let params = params.join(",");
    let syntax = |agent: &mut Agent| {
        agent.throw_error(
            crate::interp::ErrorType::SyntaxError,
            "Unexpected token in function source",
        )
    };
    // Each part alone, as the spec parses them: neither can close the
    // function and go on, so the whole is the one function they make
    // (its own early errors, a repeated parameter under "use strict",
    // are the compiler's).
    if !one_function(&format!("({keyword} anonymous({params}\n) {{\n}})"))
        || !one_function(&format!("({keyword} anonymous(\n) {{\n{body}\n}})"))
    {
        return Err(syntax(agent));
    }
    let source = format!("({keyword} anonymous({params}\n) {{\n{body}\n}})");
    let origin = crate::interp::Origin::Eval {
        caller: agent.caller_position(),
        caller_name: "<anonymous>".into(),
        function: true,
    };
    let f = agent.eval_script(&source, origin)?;
    if !c.new_target.is_undefined()
        && c.new_target != c.callee
        && let Some(o) = f.as_object()
    {
        agent.vm.temps.push(f);
        let proto = agent.prototype_from(c.new_target, default_proto);
        agent.vm.temps.pop();
        agent.heap.set_prototype_of(o, proto?);
    }
    Ok(f)
}

fn not_a_function(agent: &mut Agent, v: Value) -> crate::interp::Abrupt {
    let text = agent.display(v);
    agent.type_error(format!("{text} is not a function"))
}

fn apply(agent: &mut Agent, c: Call) -> Res<Value> {
    if !agent.is_callable(c.this) {
        return Err(not_a_function(agent, c.this));
    }
    let this = agent.arg(c, 0);
    let list = agent.arg(c, 1);
    if list.is_nullish() {
        return agent.call_value(c.this, this, &[]);
    }
    let args = agent.list_from_array_like(list)?;
    agent.call_value(c.this, this, &args)
}

fn call(agent: &mut Agent, c: Call) -> Res<Value> {
    if !agent.is_callable(c.this) {
        return Err(not_a_function(agent, c.this));
    }
    let this = agent.arg(c, 0);
    let args: Vec<Value> = (1..c.argc).map(|i| agent.arg(c, i)).collect();
    agent.call_value(c.this, this, &args)
}

fn bind(agent: &mut Agent, c: Call) -> Res<Value> {
    let target = c.this;
    if !agent.is_callable(target) {
        return Err(agent.type_error("Bind must be called on a function"));
    }
    let Some(t) = target.as_object() else {
        return Err(agent.type_error("Bind must be called on a function"));
    };
    let proto = agent.heap.get_prototype_of(t);
    let mut internal = vec![target, agent.arg(c, 0)];
    internal.extend((1..c.argc).map(|i| agent.arg(c, i)));
    let bound_args = c.argc.saturating_sub(1) as f64;
    let f = agent.alloc_object(ObjectCell::bound(proto, internal))?;
    agent.vm.temps.push(f);
    let result = (|| {
        let length_key = agent.heap.length_key;
        let mut length = 0.0;
        if agent.has_own(target, length_key) {
            let l = agent.get_value(target, length_key)?;
            if let Some(n) = l.as_number() {
                length = if n == f64::INFINITY {
                    f64::INFINITY
                } else if n == f64::NEG_INFINITY {
                    0.0
                } else {
                    let n = if n.is_nan() { 0.0 } else { n.trunc() };
                    (n - bound_args).max(0.0)
                };
            }
        }
        let name_key = agent.realm().name_key;
        let name = agent.get_value(target, name_key)?;
        let mut units: Vec<u16> = "bound ".encode_utf16().collect();
        if name.as_string().is_some() {
            units.extend(agent.string_units(name).unwrap_or_default());
        }
        let name = agent.new_string_units(&units)?;
        if let Some(o) = f.as_object() {
            agent
                .heap
                .define_own_property(o, length_key, hidden_readonly(Value::number(length)));
            agent
                .heap
                .define_own_property(o, name_key, hidden_readonly(name));
        }
        Ok(f)
    })();
    agent.vm.temps.pop();
    result
}

impl Agent {
    /// A function's source text, as `Function.prototype.toString` gives
    /// it; `None` if `f` is no function.
    pub(crate) fn function_source(&self, f: Value) -> Option<String> {
        Some(match self.callable(f)? {
            Callable::Code { unit, func, .. } => {
                let code = self
                    .vm
                    .units
                    .get(unit as usize)
                    .map(|u| std::rc::Rc::clone(&u.code));
                code.and_then(|code| {
                    let f = code.functions.get(func as usize)?;
                    code.source
                        .get(f.span.0 as usize..f.span.1 as usize)
                        .map(str::to_owned)
                })
                .unwrap_or_default()
            }
            Callable::Native { id, .. } => {
                let name = match self.host_name(id) {
                    Some(n) => n.to_owned(),
                    None => super::native_name(id),
                };
                format!("function {name}() {{ [native code] }}")
            }
            Callable::Bound | Callable::Proxy => "function () { [native code] }".to_owned(),
        })
    }
}

/// `Function.prototype.toString` (section 20.2.3.5): a function's source
/// text; natives and bound functions as the reference browser shows
/// them.
fn to_string(agent: &mut Agent, c: Call) -> Res<Value> {
    let Some(text) = agent.function_source(c.this) else {
        return Err(
            agent.type_error("Function.prototype.toString requires that 'this' be a Function")
        );
    };
    agent.new_string(&text)
}

fn has_instance(agent: &mut Agent, c: Call) -> Res<Value> {
    let r = agent.ordinary_has_instance(c.this, agent.arg(c, 0))?;
    Ok(Value::bool(r))
}

impl Agent {
    /// CreateListFromArrayLike (section 7.3.19).
    pub(crate) fn list_from_array_like(&mut self, v: Value) -> Res<Vec<Value>> {
        if v.as_object().is_none() {
            return Err(self.type_error("CreateListFromArrayLike called on non-object"));
        }
        let len = self.length_of(v)?;
        // As many as the register file takes before the frame limit
        // would: past it, the reference browser's own RangeError.
        if len > f64::from(1u32 << 23) {
            return Err(self.range_error(crate::interp::messages::STACK_OVERFLOW));
        }
        let mut out = Vec::with_capacity(len as usize);
        let base = self.vm.temps.len();
        let mut result = Ok(());
        for i in 0..len as u32 {
            match self.get_value(v, crate::string::PropertyKey::Index(i)) {
                Ok(x) => {
                    self.vm.temps.push(x);
                    out.push(x);
                }
                Err(e) => {
                    result = Err(e);
                    break;
                }
            }
        }
        self.vm.temps.truncate(base);
        result.map(|()| out)
    }

    /// OrdinaryHasInstance (section 7.3.21).
    pub(crate) fn ordinary_has_instance(&mut self, c: Value, o: Value) -> Res<bool> {
        if !self.is_callable(c) {
            return Ok(false);
        }
        if let Some(Callable::Bound) = self.callable(c) {
            let target = c
                .as_object()
                .and_then(|i| self.heap.objects.get(i))
                .and_then(|cell| cell.internal().first().copied())
                .unwrap_or(Value::UNDEFINED);
            return self.instance_of(o, target);
        }
        let Some(mut at) = o.as_object() else {
            return Ok(false);
        };
        let key = self.realm().prototype_key;
        let proto = self.get_value(c, key)?;
        if proto.as_object().is_none() {
            let text = self.display(proto);
            return Err(self.type_error(format!(
                "Function has non-object prototype '{text}' in instanceof check"
            )));
        }
        loop {
            let p = self.o_get_prototype_of(at)?;
            if p == proto {
                return Ok(true);
            }
            match p.as_object() {
                Some(next) => at = next,
                None => return Ok(false),
            }
        }
    }
}
