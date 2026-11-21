//! The `Error` family (ECMA-262 sections 20.5.1 to 20.5.6). The stack,
//! `Error.captureStackTrace` and `Error.prepareStackTrace` follow the
//! reference browser (design note, section 1.11).

use super::{
    CONFIGURABLE, CONSTRUCTOR, Call, ENUMERABLE, Entry, HIDDEN, Name, WRITABLE, What, entry, method,
};
use crate::agent::Agent;
use crate::heap::ObjectCell;
use crate::interp::ops::hidden;
use crate::interp::{ErrorType, Res};
use crate::object::ObjectKind;
use crate::value::Value;

pub(crate) const STATICS: &[Entry] = &[
    method("captureStackTrace", 2, capture_stack_trace),
    method("isError", 1, is_error),
    entry(
        Name::Str("stackTraceLimit"),
        What::Number(10.0),
        WRITABLE | ENUMERABLE | CONFIGURABLE,
    ),
];

pub(crate) const PROTO: &[Entry] = &[
    CONSTRUCTOR,
    entry(Name::Str("name"), What::Given(0), HIDDEN),
    entry(Name::Str("message"), What::Given(1), HIDDEN),
    method("toString", 0, to_string),
];

pub(crate) const NATIVE_PROTO: &[Entry] = &[
    CONSTRUCTOR,
    entry(Name::Str("name"), What::Given(0), HIDDEN),
    entry(Name::Str("message"), What::Given(1), HIDDEN),
];

/// The error constructors' common body (section 20.5.1.1): an object of
/// the prototype `new.target` gives, its `message` and `cause`.
fn construct(agent: &mut Agent, c: Call, t: ErrorType) -> Res<Value> {
    let nt = if c.new_target.is_undefined() {
        c.callee
    } else {
        c.new_target
    };
    let default = agent.realm().error_proto(t);
    let proto = agent.prototype_from(nt, default)?;
    let o = agent.alloc_object(ObjectCell::new(proto, ObjectKind::Error))?;
    agent.vm.temps.push(o);
    let result = (|| {
        let message = agent.arg(c, 0);
        if !message.is_undefined() {
            let m = agent.string_value(message)?;
            let key = agent.realm().message_key;
            if let Some(i) = o.as_object() {
                agent.heap.define_own_property(i, key, hidden(m));
            }
        }
        let options = agent.arg(c, 1);
        let cause = agent.name_key(Name::Str("cause"));
        if options.as_object().is_some() && agent.has_property(options, cause) {
            let v = agent.get_value(options, cause)?;
            if let Some(i) = o.as_object() {
                agent.heap.define_own_property(i, cause, hidden(v));
            }
        }
        Ok(o)
    })();
    agent.vm.temps.pop();
    result
}

pub(crate) fn construct_error(agent: &mut Agent, c: Call) -> Res<Value> {
    construct(agent, c, ErrorType::Error)
}

pub(crate) fn construct_eval_error(agent: &mut Agent, c: Call) -> Res<Value> {
    construct(agent, c, ErrorType::EvalError)
}

pub(crate) fn construct_range_error(agent: &mut Agent, c: Call) -> Res<Value> {
    construct(agent, c, ErrorType::RangeError)
}

pub(crate) fn construct_reference_error(agent: &mut Agent, c: Call) -> Res<Value> {
    construct(agent, c, ErrorType::ReferenceError)
}

pub(crate) fn construct_syntax_error(agent: &mut Agent, c: Call) -> Res<Value> {
    construct(agent, c, ErrorType::SyntaxError)
}

pub(crate) fn construct_type_error(agent: &mut Agent, c: Call) -> Res<Value> {
    construct(agent, c, ErrorType::TypeError)
}

pub(crate) fn construct_uri_error(agent: &mut Agent, c: Call) -> Res<Value> {
    construct(agent, c, ErrorType::UriError)
}

/// `Error.captureStackTrace(object)`: gives `object` a `stack`.
fn capture_stack_trace(agent: &mut Agent, c: Call) -> Res<Value> {
    let o = agent.arg(c, 0);
    if o.as_object().is_none() {
        let text = agent.display(o);
        return Err(agent.type_error(format!("Invalid argument: {text}")));
    }
    let heading = error_heading(agent, o)?;
    let key = agent.name_key(Name::Str("stack"));
    agent.define_or_throw(o, key, hidden(heading))?;
    Ok(Value::UNDEFINED)
}

fn is_error(agent: &mut Agent, c: Call) -> Res<Value> {
    Ok(Value::bool(
        agent.kind_of(agent.arg(c, 0)) == Some(ObjectKind::Error),
    ))
}

/// `Error.prototype.toString`'s text (section 20.5.3.4).
fn error_heading(agent: &mut Agent, o: Value) -> Res<Value> {
    let (name_key, message_key) = (agent.realm().name_key, agent.realm().message_key);
    let name = agent.get_value(o, name_key)?;
    let name = if name.is_undefined() {
        "Error".encode_utf16().collect()
    } else {
        agent.text_units(name)?
    };
    let message = agent.get_value(o, message_key)?;
    let message = if message.is_undefined() {
        Vec::new()
    } else {
        agent.text_units(message)?
    };
    let mut text = name.clone();
    if name.is_empty() {
        text = message;
    } else if !message.is_empty() {
        text.extend(": ".encode_utf16());
        text.extend(message);
    }
    agent.new_string_units(&text)
}

fn to_string(agent: &mut Agent, c: Call) -> Res<Value> {
    if c.this.as_object().is_none() {
        return Err(agent.type_error("Error.prototype.toString requires that 'this' be an Object"));
    }
    error_heading(agent, c.this)
}
