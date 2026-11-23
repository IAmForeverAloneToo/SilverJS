//! `Reflect` (ECMA-262 section 28.1).

use super::{CONFIGURABLE, Call, Entry, Name, What, Wk, entry, method};
use crate::agent::Agent;
use crate::interp::Res;
use crate::string::PropertyKey;
use crate::value::Value;

/// In the reference browser's order (docs/chrome-surface.md).
pub(crate) const REFLECT: &[Entry] = &[
    method("defineProperty", 3, define_property),
    method("deleteProperty", 2, delete_property),
    method("apply", 3, apply),
    method("construct", 2, construct),
    method("get", 2, get),
    method("getOwnPropertyDescriptor", 2, get_own_property_descriptor),
    method("getPrototypeOf", 1, get_prototype_of),
    method("has", 2, has),
    method("isExtensible", 1, is_extensible),
    method("ownKeys", 1, own_keys),
    method("preventExtensions", 1, prevent_extensions),
    method("set", 3, set),
    method("setPrototypeOf", 2, set_prototype_of),
    entry(
        Name::Sym(Wk::ToStringTag),
        What::Str("Reflect"),
        CONFIGURABLE,
    ),
];

/// The target, which must be an object.
fn target(agent: &mut Agent, c: Call, method: &str) -> Res<(Value, u32)> {
    let t = agent.arg(c, 0);
    match t.as_object() {
        Some(o) => Ok((t, o)),
        None => Err(agent.type_error(format!("Reflect.{method} called on non-object"))),
    }
}

fn key(agent: &mut Agent, c: Call) -> Res<PropertyKey> {
    agent.property_key(agent.arg(c, 1))
}

fn apply(agent: &mut Agent, c: Call) -> Res<Value> {
    let f = agent.arg(c, 0);
    if !agent.is_callable(f) {
        let text = agent.display(f);
        return Err(agent.type_error(format!(
            "Function.prototype.apply was called on {text}, which is not a function"
        )));
    }
    let args = agent.list_from_array_like(agent.arg(c, 2))?;
    agent.call_value(f, agent.arg(c, 1), &args)
}

fn construct(agent: &mut Agent, c: Call) -> Res<Value> {
    let f = agent.arg(c, 0);
    if !agent.is_constructor_value(f) {
        let text = agent.display(f);
        return Err(agent.type_error(format!("{text} is not a constructor")));
    }
    let nt = if c.argc > 2 { agent.arg(c, 2) } else { f };
    if !agent.is_constructor_value(nt) {
        let text = agent.display(nt);
        return Err(agent.type_error(format!("{text} is not a constructor")));
    }
    let args = agent.list_from_array_like(agent.arg(c, 1))?;
    agent.construct_value(f, &args, nt)
}

fn define_property(agent: &mut Agent, c: Call) -> Res<Value> {
    let (t, o) = target(agent, c, "defineProperty")?;
    let k = key(agent, c)?;
    let mut d = agent.descriptor_of(agent.arg(c, 2))?;
    if k == agent.heap.length_key
        && agent.is_array(t)
        && let Some(v) = d.value
    {
        d.value = Some(agent.array_length_value(v)?);
    }
    agent.o_define_own_property(o, k, d).map(Value::bool)
}

fn delete_property(agent: &mut Agent, c: Call) -> Res<Value> {
    let (_, o) = target(agent, c, "deleteProperty")?;
    let k = key(agent, c)?;
    agent.o_delete(o, k).map(Value::bool)
}

fn get(agent: &mut Agent, c: Call) -> Res<Value> {
    let (t, o) = target(agent, c, "get")?;
    let k = key(agent, c)?;
    let receiver = if c.argc > 2 { agent.arg(c, 2) } else { t };
    agent.o_get(o, k, receiver)
}

fn get_own_property_descriptor(agent: &mut Agent, c: Call) -> Res<Value> {
    let (_, o) = target(agent, c, "getOwnPropertyDescriptor")?;
    let k = key(agent, c)?;
    match agent.o_get_own_property(o, k)? {
        Some(p) => agent.descriptor_object(p),
        None => Ok(Value::UNDEFINED),
    }
}

fn get_prototype_of(agent: &mut Agent, c: Call) -> Res<Value> {
    let (_, o) = target(agent, c, "getPrototypeOf")?;
    agent.o_get_prototype_of(o)
}

fn has(agent: &mut Agent, c: Call) -> Res<Value> {
    let (_, o) = target(agent, c, "has")?;
    let k = key(agent, c)?;
    agent.o_has_property(o, k).map(Value::bool)
}

fn is_extensible(agent: &mut Agent, c: Call) -> Res<Value> {
    let (_, o) = target(agent, c, "isExtensible")?;
    agent.o_is_extensible(o).map(Value::bool)
}

fn own_keys(agent: &mut Agent, c: Call) -> Res<Value> {
    let (_, o) = target(agent, c, "ownKeys")?;
    let keys = agent.o_own_keys(o)?;
    let out = agent.array_of(&[])?;
    agent.vm.temps.push(out);
    let mut result = Ok(out);
    for (i, k) in keys.into_iter().enumerate() {
        match agent.key_value(k) {
            Ok(v) => {
                agent.create_data_property(out, PropertyKey::Index(i as u32), v);
            }
            Err(e) => {
                result = Err(e);
                break;
            }
        }
    }
    agent.vm.temps.pop();
    result
}

fn prevent_extensions(agent: &mut Agent, c: Call) -> Res<Value> {
    let (_, o) = target(agent, c, "preventExtensions")?;
    agent.o_prevent_extensions(o).map(Value::bool)
}

fn set(agent: &mut Agent, c: Call) -> Res<Value> {
    let (t, o) = target(agent, c, "set")?;
    let k = key(agent, c)?;
    let mut v = agent.arg(c, 2);
    let receiver = if c.argc > 3 { agent.arg(c, 3) } else { t };
    if receiver == t
        && k == agent.heap.length_key
        && let Some((_, true)) = agent
            .heap
            .objects
            .get(o)
            .and_then(crate::heap::ObjectCell::array_length)
    {
        v = agent.array_length_value(v)?;
    }
    agent.o_set(o, k, v, receiver).map(Value::bool)
}

fn set_prototype_of(agent: &mut Agent, c: Call) -> Res<Value> {
    let (_, o) = target(agent, c, "setPrototypeOf")?;
    let proto = agent.arg(c, 1);
    if proto.as_object().is_none() && !proto.is_null() {
        let text = agent.display(proto);
        return Err(agent.type_error(format!(
            "Object prototype may only be an Object or null: {text}"
        )));
    }
    agent.o_set_prototype_of(o, proto).map(Value::bool)
}
