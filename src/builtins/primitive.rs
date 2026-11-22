//! `Boolean` (ECMA-262 section 20.3) and `Symbol` (section 20.4).

use super::{CONFIGURABLE, CONSTRUCTOR, Call, Entry, FIXED, Name, What, Wk, entry, getter, method};
use crate::agent::Agent;
use crate::interp::Res;
use crate::object::ObjectKind;
use crate::value::{RefKind, Value};

pub(crate) const BOOLEAN_PROTO: &[Entry] = &[
    CONSTRUCTOR,
    method("toString", 0, boolean_to_string),
    method("valueOf", 0, boolean_value_of),
];

const fn well_known(w: Wk) -> Entry {
    entry(Name::Str(w.name()), What::Symbol(w), FIXED)
}

pub(crate) const SYMBOL_STATICS: &[Entry] = &[
    method("for", 1, symbol_for),
    method("keyFor", 1, key_for),
    well_known(Wk::AsyncIterator),
    well_known(Wk::HasInstance),
    well_known(Wk::IsConcatSpreadable),
    well_known(Wk::Iterator),
    well_known(Wk::Match),
    well_known(Wk::MatchAll),
    well_known(Wk::Replace),
    well_known(Wk::Search),
    well_known(Wk::Species),
    well_known(Wk::Split),
    well_known(Wk::ToPrimitive),
    well_known(Wk::ToStringTag),
    well_known(Wk::Unscopables),
    well_known(Wk::Dispose),
    well_known(Wk::AsyncDispose),
];

pub(crate) const SYMBOL_PROTO: &[Entry] = &[
    CONSTRUCTOR,
    method("toString", 0, symbol_to_string),
    method("valueOf", 0, symbol_value_of),
    getter("description", description),
    entry(
        Name::Sym(Wk::ToStringTag),
        What::Str("Symbol"),
        CONFIGURABLE,
    ),
    entry(
        Name::Sym(Wk::ToPrimitive),
        What::Method(1, symbol_to_primitive),
        CONFIGURABLE,
    ),
];

pub(crate) fn construct_boolean(agent: &mut Agent, c: Call) -> Res<Value> {
    let b = Value::bool(agent.to_boolean(agent.arg(c, 0)));
    if c.new_target.is_undefined() {
        return Ok(b);
    }
    let proto = agent.prototype_from(c.new_target, |r| r.boolean_prototype)?;
    agent.wrap(b, Some(proto))
}

fn this_boolean(agent: &mut Agent, v: Value, method: &str) -> Res<bool> {
    match agent
        .this_primitive(v, ObjectKind::Boolean)
        .and_then(Value::as_bool)
    {
        Some(b) => Ok(b),
        None => Err(agent.type_error(format!(
            "Boolean.prototype.{method} requires that 'this' be a Boolean"
        ))),
    }
}

fn boolean_to_string(agent: &mut Agent, c: Call) -> Res<Value> {
    let b = this_boolean(agent, c.this, "toString")?;
    let s = &agent.realm().strings;
    Ok(if b { s.true_ } else { s.false_ })
}

fn boolean_value_of(agent: &mut Agent, c: Call) -> Res<Value> {
    this_boolean(agent, c.this, "valueOf").map(Value::bool)
}

pub(crate) fn construct_symbol(agent: &mut Agent, c: Call) -> Res<Value> {
    if !c.new_target.is_undefined() {
        return Err(agent.type_error("Symbol is not a constructor"));
    }
    let d = agent.arg(c, 0);
    let d = if d.is_undefined() {
        d
    } else {
        agent.string_value(d)?
    };
    agent.vm.temps.push(d);
    let s = agent.new_symbol(d);
    agent.vm.temps.pop();
    s
}

fn symbol_for(agent: &mut Agent, c: Call) -> Res<Value> {
    let key = agent.string_value(agent.arg(c, 0))?;
    let units = agent.string_units(key).unwrap_or_default();
    if let Some(&s) = agent.vm.registry.get(&units) {
        return Ok(s);
    }
    agent.vm.temps.push(key);
    let s = agent.new_symbol(key);
    agent.vm.temps.pop();
    let s = s?;
    agent.vm.registry.insert(units, s);
    Ok(s)
}

fn key_for(agent: &mut Agent, c: Call) -> Res<Value> {
    let s = agent.arg(c, 0);
    if !matches!(s.as_reference(), Some((RefKind::Symbol, _))) {
        let text = agent.display(s);
        return Err(agent.type_error(format!("{text} is not a symbol")));
    }
    if agent.vm.registry.values().any(|&v| v == s) {
        return Ok(symbol_description(agent, s));
    }
    Ok(Value::UNDEFINED)
}

fn symbol_description(agent: &Agent, s: Value) -> Value {
    s.as_reference()
        .and_then(|(_, i)| agent.heap.symbols.get(i))
        .map_or(Value::UNDEFINED, |c| c.description)
}

fn this_symbol(agent: &mut Agent, v: Value, method: &str) -> Res<Value> {
    match agent.this_primitive(v, ObjectKind::Symbol) {
        Some(s) => Ok(s),
        None => Err(agent.type_error(format!(
            "Symbol.prototype.{method} requires that 'this' be a Symbol"
        ))),
    }
}

impl Agent {
    /// SymbolDescriptiveString (section 20.4.3.3.1).
    pub(crate) fn symbol_text(&self, s: Value) -> Vec<u16> {
        let d = symbol_description(self, s);
        let mut out: Vec<u16> = "Symbol(".encode_utf16().collect();
        out.extend(self.string_units(d).unwrap_or_default());
        out.push(u16::from(b')'));
        out
    }
}

fn symbol_to_string(agent: &mut Agent, c: Call) -> Res<Value> {
    let s = this_symbol(agent, c.this, "toString")?;
    let text = agent.symbol_text(s);
    agent.new_string_units(&text)
}

fn symbol_value_of(agent: &mut Agent, c: Call) -> Res<Value> {
    this_symbol(agent, c.this, "valueOf")
}

fn description(agent: &mut Agent, c: Call) -> Res<Value> {
    let s = this_symbol(agent, c.this, "description")?;
    Ok(symbol_description(agent, s))
}

fn symbol_to_primitive(agent: &mut Agent, c: Call) -> Res<Value> {
    this_symbol(agent, c.this, "[Symbol.toPrimitive]")
}
