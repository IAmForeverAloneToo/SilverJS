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

/// `AggregateError(errors, message, options)` (section 20.5.7.1): the
/// common body from the second argument, then `errors`, a list of the
/// first's values.
pub(crate) fn construct_aggregate_error(agent: &mut Agent, c: Call) -> Res<Value> {
    let rest = Call {
        args: c.args + 1,
        argc: c.argc.saturating_sub(1),
        ..c
    };
    let o = construct(agent, rest, ErrorType::AggregateError)?;
    agent.vm.temps.push(o);
    let r = (|| {
        let errors = agent.arg(c, 0);
        let (iterator, next) = agent.get_iterator_typed(errors)?;
        agent.vm.temps.extend([iterator, next]);
        let mut list = Vec::new();
        let r = loop {
            match agent.iterator_step(iterator, next) {
                Ok(Some(v)) => {
                    agent.vm.temps.push(v);
                    list.push(v);
                }
                Ok(None) => break Ok(()),
                Err(e) => break Err(e),
            }
        };
        let array = r.and_then(|()| agent.array_of(&list));
        let n = agent.vm.temps.len();
        agent.vm.temps.truncate(n.saturating_sub(2 + list.len()));
        let array = array?;
        let key = agent.name_key(Name::Str("errors"));
        if let Some(i) = o.as_object() {
            agent.heap.define_own_property(i, key, hidden(array));
        }
        Ok(o)
    })();
    agent.vm.temps.pop();
    r
}

/// `SuppressedError(error, suppressed, message)` (section 20.5.8.1 of
/// the explicit resource management proposal): `message` if given, then
/// `error` and `suppressed`; no options.
pub(crate) fn construct_suppressed_error(agent: &mut Agent, c: Call) -> Res<Value> {
    let nt = if c.new_target.is_undefined() {
        c.callee
    } else {
        c.new_target
    };
    let proto = agent.prototype_from(nt, |r| r.error_proto(ErrorType::SuppressedError))?;
    let o = agent.alloc_object(ObjectCell::new(proto, ObjectKind::Error))?;
    agent.vm.temps.push(o);
    let result = (|| {
        let message = agent.arg(c, 2);
        let m = if message.is_undefined() {
            Value::UNDEFINED
        } else {
            agent.string_value(message)?
        };
        agent.vm.temps.push(m);
        let captured = agent.capture_stack(o, m, true, Value::UNDEFINED);
        agent.vm.temps.pop();
        captured?;
        let Some(i) = o.as_object() else {
            return Ok(o);
        };
        if !m.is_undefined() {
            let key = agent.realm().message_key;
            agent.heap.define_own_property(i, key, hidden(m));
        }
        for (k, name) in [(0, "error"), (1, "suppressed")] {
            let v = agent.arg(c, k);
            let key = agent.name_key(Name::Str(name));
            agent.heap.define_own_property(i, key, hidden(v));
        }
        Ok(o)
    })();
    agent.vm.temps.pop();
    result
}

/// The error constructors' common body (section 20.5.1.1): an object of
/// the prototype `new.target` gives, its `message` and `cause`.
fn construct(agent: &mut Agent, c: Call, t: ErrorType) -> Res<Value> {
    let nt = if c.new_target.is_undefined() {
        c.callee
    } else {
        c.new_target
    };
    let proto = agent.prototype_from(nt, |r| r.error_proto(t))?;
    let o = agent.alloc_object(ObjectCell::new(proto, ObjectKind::Error))?;
    agent.vm.temps.push(o);
    let result = (|| {
        let message = agent.arg(c, 0);
        let m = if message.is_undefined() {
            Value::UNDEFINED
        } else {
            agent.string_value(message)?
        };
        // `stack` first, then `message`: the reference browser's order.
        agent.vm.temps.push(m);
        let captured = agent.capture_stack(o, m, true, Value::UNDEFINED);
        agent.vm.temps.pop();
        captured?;
        if !m.is_undefined() {
            let key = agent.realm().message_key;
            if let Some(i) = o.as_object() {
                agent.heap.define_own_property(i, key, hidden(m));
            }
        }
        let options = agent.arg(c, 1);
        let cause = agent.name_key(Name::Str("cause"));
        if options.as_object().is_some() && agent.has_property(options, cause)? {
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

/// `Error.captureStackTrace(object, fn)`: gives `object` a `stack` of
/// the frames below this call (below `fn`'s topmost, when given), headed
/// by its `name` and its `message` as it is now.
fn capture_stack_trace(agent: &mut Agent, c: Call) -> Res<Value> {
    let o = agent.arg(c, 0);
    if o.as_object().is_none() {
        let text = agent.display(o);
        return Err(agent.type_error(format!("Invalid argument: {text}")));
    }
    let key = agent.realm().message_key;
    let message = agent.get_value(o, key)?;
    let message = if message.is_undefined() {
        message
    } else {
        agent.string_value(message)?
    };
    let until = agent.arg(c, 1);
    let until = if agent.is_callable(until) {
        until
    } else {
        Value::UNDEFINED
    };
    agent.vm.temps.push(message);
    let r = agent.capture_stack(o, message, true, until);
    agent.vm.temps.pop();
    r?;
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
