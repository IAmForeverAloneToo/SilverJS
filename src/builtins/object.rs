//! `Object` (ECMA-262 section 20.1). `Object.fromEntries` and
//! `Object.groupBy` take iterables, and come with J3's iterators.

use super::{CONSTRUCTOR, Call, Entry, Name, What, Wk, entry, method};
use crate::agent::Agent;
use crate::heap::ObjectCell;
use crate::interp::Res;
use crate::interp::messages;
use crate::object::{CONFIGURABLE, Descriptor, ObjectKind, OwnProperty};
use crate::string::PropertyKey;
use crate::value::Value;

pub(crate) const STATICS: &[Entry] = &[
    method("assign", 2, assign),
    method("getOwnPropertyDescriptor", 2, get_own_property_descriptor),
    method("getOwnPropertyDescriptors", 1, get_own_property_descriptors),
    method("getOwnPropertyNames", 1, get_own_property_names),
    method("getOwnPropertySymbols", 1, get_own_property_symbols),
    method("hasOwn", 2, has_own),
    method("is", 2, is),
    method("preventExtensions", 1, prevent_extensions),
    method("seal", 1, seal),
    method("create", 2, create),
    method("defineProperties", 2, define_properties),
    method("defineProperty", 3, define_property),
    method("freeze", 1, freeze),
    method("getPrototypeOf", 1, get_prototype_of),
    method("setPrototypeOf", 2, set_prototype_of),
    method("isExtensible", 1, is_extensible),
    method("isFrozen", 1, is_frozen),
    method("isSealed", 1, is_sealed),
    method("keys", 1, keys),
    method("entries", 1, entries),
    method("fromEntries", 1, from_entries),
    method("values", 1, values),
    method("groupBy", 2, group_by),
];

/// `Object.fromEntries(iterable)` (section 20.1.2.7): each entry an
/// object whose `0` is a key and `1` a value; the iterator closed if an
/// entry fails.
fn from_entries(agent: &mut Agent, c: Call) -> Res<Value> {
    let iterable = agent.arg(c, 0);
    if iterable.is_nullish() {
        let text = agent.display(iterable);
        return Err(agent.type_error(messages::not_iterable(&text)));
    }
    let proto = agent.realm().object_prototype;
    let o = agent.alloc_object(ObjectCell::new(proto, ObjectKind::Ordinary))?;
    let base = agent.vm.temps.len();
    agent.vm.temps.push(o);
    let result = (|| {
        let text = agent.display(iterable);
        let (iterator, next) = agent.get_iterator(iterable, &text)?;
        agent.vm.temps.extend([iterator, next]);
        while let Some(entry) = agent.iterator_step(iterator, next)? {
            let added = (|| {
                if entry.as_object().is_none() {
                    let text = agent.display(entry);
                    return Err(
                        agent.type_error(format!("Iterator value {text} is not an entry object"))
                    );
                }
                agent.vm.temps.push(entry);
                let k = agent.get_value(entry, PropertyKey::Index(0))?;
                agent.vm.temps.push(k);
                let v = agent.get_value(entry, PropertyKey::Index(1))?;
                agent.vm.temps.push(v);
                let key = agent.property_key(k)?;
                agent.define_value(o, key, v)
            })();
            if let Err(e) = added {
                agent.iterator_close_quiet(iterator)?;
                return Err(e);
            }
        }
        Ok(o)
    })();
    agent.vm.temps.truncate(base);
    result
}

/// `Object.groupBy(items, callback)` (section 20.1.2.12): GroupBy with
/// property keys, the groups the properties of an object of no
/// prototype, each made where its key first comes.
fn group_by(agent: &mut Agent, c: Call) -> Res<Value> {
    let (items, callback) = (agent.arg(c, 0), agent.arg(c, 1));
    if items.is_nullish() {
        return Err(agent.type_error("Object.groupBy called on null or undefined"));
    }
    if !agent.is_callable(callback) {
        let text = agent.display(callback);
        return Err(agent.type_error(format!("{text} is not a function")));
    }
    let o = agent.alloc_object(ObjectCell::new(Value::NULL, ObjectKind::Ordinary))?;
    let base = agent.vm.temps.len();
    agent.vm.temps.push(o);
    let result = (|| {
        let (it, next) = agent.get_iterator_typed(items)?;
        agent.vm.temps.extend([it, next]);
        let mut k = 0.0;
        while let Some(v) = agent.iterator_step(it, next)? {
            let mark = agent.vm.temps.len();
            let step = (|| {
                agent.vm.temps.push(v);
                let key = agent.call_value(callback, Value::UNDEFINED, &[v, Value::number(k)])?;
                agent.vm.temps.push(key);
                let key = agent.property_key(key)?;
                let mut group = agent.get_value(o, key)?;
                if group.is_undefined() {
                    group = agent.array_of(&[])?;
                    agent.define_value(o, key, group)?;
                }
                agent.array_push(group, v)
            })();
            agent.vm.temps.truncate(mark);
            if let Err(e) = step {
                if matches!(e, crate::interp::Abrupt::Throw) {
                    agent.iterator_close_quiet(it)?;
                }
                return Err(e);
            }
            k += 1.0;
        }
        Ok(o)
    })();
    agent.vm.temps.truncate(base);
    result
}

pub(crate) const PROTO: &[Entry] = &[
    CONSTRUCTOR,
    method("__defineGetter__", 2, define_getter),
    method("__defineSetter__", 2, define_setter),
    method("hasOwnProperty", 1, has_own_property),
    method("__lookupGetter__", 1, lookup_getter),
    method("__lookupSetter__", 1, lookup_setter),
    method("isPrototypeOf", 1, is_prototype_of),
    method("propertyIsEnumerable", 1, property_is_enumerable),
    method("toString", 0, to_string),
    method("valueOf", 0, value_of),
    entry(
        Name::Str("__proto__"),
        What::Accessor(get_proto, set_proto),
        CONFIGURABLE,
    ),
    method("toLocaleString", 0, to_locale_string),
];

/// `Object(value)` (section 20.1.1.1).
pub(crate) fn construct(agent: &mut Agent, c: Call) -> Res<Value> {
    if !c.new_target.is_undefined() && c.new_target != c.callee {
        let proto = agent.prototype_from(c.new_target, |r| r.object_prototype)?;
        return agent.alloc_object(ObjectCell::new(proto, ObjectKind::Ordinary));
    }
    let v = agent.arg(c, 0);
    if v.is_nullish() {
        return agent.plain_object();
    }
    agent.object_of(v)
}

impl Agent {
    /// FromPropertyDescriptor (section 6.2.6.4).
    pub(crate) fn descriptor_object(&mut self, p: OwnProperty) -> Res<Value> {
        // The property's values rooted until the object holds them: a
        // BigInt element's value is new, held by nothing else.
        let base = self.vm.temps.len();
        match p {
            OwnProperty::Data { value, .. } => self.vm.temps.push(value),
            OwnProperty::Accessor { get, set, .. } => self.vm.temps.extend([get, set]),
        }
        let o = match self.plain_object() {
            Ok(o) => o,
            Err(e) => {
                self.vm.temps.truncate(base);
                return Err(e);
            }
        };
        self.vm.temps.push(o);
        let fields: Vec<(&str, Value)> = match p {
            OwnProperty::Data {
                value,
                writable,
                enumerable,
                configurable,
            } => vec![
                ("value", value),
                ("writable", Value::bool(writable)),
                ("enumerable", Value::bool(enumerable)),
                ("configurable", Value::bool(configurable)),
            ],
            OwnProperty::Accessor {
                get,
                set,
                enumerable,
                configurable,
            } => vec![
                ("get", get),
                ("set", set),
                ("enumerable", Value::bool(enumerable)),
                ("configurable", Value::bool(configurable)),
            ],
        };
        for (name, v) in fields {
            let key = self.name_key(Name::Str(name));
            self.create_data_property(o, key, v);
        }
        self.vm.temps.truncate(base);
        Ok(o)
    }

    /// ToPropertyDescriptor (section 6.2.6.5).
    pub(crate) fn descriptor_of(&mut self, v: Value) -> Res<Descriptor> {
        if v.as_object().is_none() {
            let text = self.display(v);
            return Err(self.type_error(format!("Property description must be an object: {text}")));
        }
        let mut d = Descriptor::default();
        for name in [
            "enumerable",
            "configurable",
            "value",
            "writable",
            "get",
            "set",
        ] {
            let key = self.name_key(Name::Str(name));
            if !self.has_property(v, key)? {
                continue;
            }
            let x = self.get_value(v, key)?;
            match name {
                "enumerable" => d.enumerable = Some(self.to_boolean(x)),
                "configurable" => d.configurable = Some(self.to_boolean(x)),
                "value" => d.value = Some(x),
                "writable" => d.writable = Some(self.to_boolean(x)),
                _ => {
                    if !x.is_undefined() && !self.is_callable(x) {
                        let text = self.display(x);
                        let what = if name == "get" { "Getter" } else { "Setter" };
                        return Err(self.type_error(format!("{what} must be a function: {text}")));
                    }
                    if name == "get" {
                        d.get = Some(x);
                    } else {
                        d.set = Some(x);
                    }
                }
            }
        }
        if (d.get.is_some() || d.set.is_some()) && (d.value.is_some() || d.writable.is_some()) {
            return Err(self.type_error(
                "Invalid property descriptor. Cannot both specify accessors and a value or writable attribute",
            ));
        }
        Ok(d)
    }

    /// The own keys of `o`, strings or symbols.
    pub(crate) fn own_property_keys(&mut self, o: Value) -> Res<Vec<PropertyKey>> {
        match o.as_object() {
            Some(i) => self.o_own_keys(i),
            None => Ok(Vec::new()),
        }
    }

    /// `[[GetOwnProperty]]` of `v`, if it is an object.
    pub(crate) fn own_property(&mut self, v: Value, key: PropertyKey) -> Res<Option<OwnProperty>> {
        match v.as_object() {
            Some(i) => self.o_get_own_property(i, key),
            None => Ok(None),
        }
    }

    /// EnumerableOwnProperties (section 7.3.24): keys, values or
    /// entries, as an array.
    fn enumerable_own(&mut self, o: Value, what: u8) -> Res<Value> {
        let keys = self.own_property_keys(o)?;
        let out = self.array_of(&[])?;
        self.vm.temps.push(out);
        let result = (|| {
            let mut n = 0u32;
            for key in keys {
                if matches!(key, PropertyKey::Symbol(_)) {
                    continue;
                }
                let Some(p) = self.own_property(o, key)? else {
                    continue;
                };
                let enumerable = match p {
                    OwnProperty::Data { enumerable, .. }
                    | OwnProperty::Accessor { enumerable, .. } => enumerable,
                };
                if !enumerable {
                    continue;
                }
                let item = match what {
                    0 => self.key_value(key)?,
                    1 => self.get_value(o, key)?,
                    _ => {
                        let k = self.key_value(key)?;
                        self.vm.temps.push(k);
                        let v = self.get_value(o, key);
                        let pair = v.and_then(|v| self.array_of(&[k, v]));
                        self.vm.temps.pop();
                        pair?
                    }
                };
                self.create_data_property(out, PropertyKey::Index(n), item);
                n += 1;
            }
            Ok(out)
        })();
        self.vm.temps.pop();
        result
    }

    /// An object frozen (SetIntegrityLevel), as the engine freezes what
    /// it makes.
    pub(crate) fn freeze_value(&mut self, o: Value) -> Res<()> {
        self.set_integrity(o, true).map(|_| ())
    }

    /// SetIntegrityLevel (section 7.3.16): sealed, or frozen.
    fn set_integrity(&mut self, o: Value, frozen: bool) -> Res<bool> {
        let Some(i) = o.as_object() else {
            return Ok(true);
        };
        if !self.o_prevent_extensions(i)? {
            return Ok(false);
        }
        // A typed array's elements stay writable and configurable: the
        // first one refuses (section 10.4.5.3), in the reference
        // browser's words.
        if self.heap.view_length(i).is_some_and(|n| n > 0) {
            let what = if frozen { "freeze" } else { "seal" };
            return Err(self.type_error(format!("Cannot {what} array buffer views with elements")));
        }
        for key in self.o_own_keys(i)? {
            let d = match self.o_get_own_property(i, key)? {
                Some(OwnProperty::Data { .. }) if frozen => Descriptor {
                    configurable: Some(false),
                    writable: Some(false),
                    ..Descriptor::default()
                },
                Some(_) => Descriptor {
                    configurable: Some(false),
                    ..Descriptor::default()
                },
                None => continue,
            };
            self.define_or_throw(o, key, d)?;
        }
        Ok(true)
    }

    /// TestIntegrityLevel (section 7.3.17).
    fn test_integrity(&mut self, o: Value, frozen: bool) -> Res<bool> {
        let Some(i) = o.as_object() else {
            return Ok(true);
        };
        if self.o_is_extensible(i)? {
            return Ok(false);
        }
        for key in self.o_own_keys(i)? {
            let sealed = match self.o_get_own_property(i, key)? {
                Some(OwnProperty::Data {
                    configurable,
                    writable,
                    ..
                }) => !configurable && !(frozen && writable),
                Some(OwnProperty::Accessor { configurable, .. }) => !configurable,
                None => true,
            };
            if !sealed {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// ObjectDefineProperties (section 20.1.2.3.1).
    fn define_properties_from(&mut self, o: Value, props: Value) -> Res<()> {
        let props = self.object_of(props)?;
        self.vm.temps.push(props);
        let result = (|| {
            let mut descs = Vec::new();
            for key in self.own_property_keys(props)? {
                let enumerable = self.own_property(props, key)?.is_some_and(|p| match p {
                    OwnProperty::Data { enumerable, .. }
                    | OwnProperty::Accessor { enumerable, .. } => enumerable,
                });
                if enumerable {
                    let v = self.get_value(props, key)?;
                    let d = self.descriptor_of(v)?;
                    // Rooted while the rest are read.
                    for x in [d.value, d.get, d.set].into_iter().flatten() {
                        self.vm.temps.push(x);
                    }
                    descs.push((key, d));
                }
            }
            let rooted: usize = descs
                .iter()
                .map(|(_, d)| [d.value, d.get, d.set].iter().flatten().count())
                .sum();
            let mut result = Ok(());
            for (key, d) in descs {
                result = self.define_or_throw(o, key, d);
                if result.is_err() {
                    break;
                }
            }
            let len = self.vm.temps.len();
            self.vm.temps.truncate(len - rooted);
            result
        })();
        self.vm.temps.pop();
        result
    }

    /// The `[[Class]]`-like tag `Object.prototype.toString` begins from.
    fn builtin_tag(&mut self, o: Value) -> Res<&'static str> {
        if self.is_array_checked(o)? {
            return Ok("Array");
        }
        Ok(match self.kind_of(o) {
            Some(ObjectKind::Arguments) => "Arguments",
            Some(ObjectKind::Error) => "Error",
            Some(ObjectKind::Boolean) => "Boolean",
            Some(ObjectKind::Number) => "Number",
            Some(ObjectKind::String) => "String",
            Some(ObjectKind::Date) => "Date",
            _ if self.is_callable(o) => "Function",
            _ => "Object",
        })
    }
}

fn assign(agent: &mut Agent, c: Call) -> Res<Value> {
    let to = agent.object_of(agent.arg(c, 0))?;
    agent.vm.temps.push(to);
    let result = (|| {
        for i in 1..c.argc {
            let from = agent.arg(c, i);
            if from.is_nullish() {
                continue;
            }
            let from = agent.object_of(from)?;
            agent.vm.temps.push(from);
            let r = (|| {
                for key in agent.own_property_keys(from)? {
                    let enumerable = agent.own_property(from, key)?.is_some_and(|p| match p {
                        OwnProperty::Data { enumerable, .. }
                        | OwnProperty::Accessor { enumerable, .. } => enumerable,
                    });
                    if enumerable {
                        let v = agent.get_value(from, key)?;
                        agent.put(to, key, v)?;
                    }
                }
                Ok(())
            })();
            agent.vm.temps.pop();
            r?;
        }
        Ok(to)
    })();
    agent.vm.temps.pop();
    result
}

fn get_own_property_descriptor(agent: &mut Agent, c: Call) -> Res<Value> {
    let o = agent.object_of(agent.arg(c, 0))?;
    agent.vm.temps.push(o);
    let key = agent.property_key(agent.arg(c, 1));
    agent.vm.temps.pop();
    let key = key?;
    match agent.own_property(o, key)? {
        Some(p) => agent.descriptor_object(p),
        None => Ok(Value::UNDEFINED),
    }
}

fn get_own_property_descriptors(agent: &mut Agent, c: Call) -> Res<Value> {
    let o = agent.object_of(agent.arg(c, 0))?;
    agent.vm.temps.push(o);
    let result = (|| {
        let out = agent.plain_object()?;
        agent.vm.temps.push(out);
        let r = (|| {
            for key in agent.own_property_keys(o)? {
                if let Some(p) = agent.own_property(o, key)? {
                    let d = agent.descriptor_object(p)?;
                    agent.create_data_property(out, key, d);
                }
            }
            Ok(out)
        })();
        agent.vm.temps.pop();
        r
    })();
    agent.vm.temps.pop();
    result
}

/// GetOwnPropertyKeys (section 20.1.2.11.1): the strings, or the symbols.
fn own_keys_of(agent: &mut Agent, v: Value, symbols: bool) -> Res<Value> {
    let o = agent.object_of(v)?;
    agent.vm.temps.push(o);
    let keys: Vec<PropertyKey> = agent
        .own_property_keys(o)?
        .into_iter()
        .filter(|k| matches!(k, PropertyKey::Symbol(_)) == symbols)
        .collect();
    let out = agent.array_of(&[]);
    let result = out.and_then(|out| {
        agent.vm.temps.push(out);
        let mut r = Ok(out);
        for (n, key) in keys.into_iter().enumerate() {
            match agent.key_value(key) {
                Ok(k) => {
                    agent.create_data_property(out, PropertyKey::Index(n as u32), k);
                }
                Err(e) => {
                    r = Err(e);
                    break;
                }
            }
        }
        agent.vm.temps.pop();
        r
    });
    agent.vm.temps.pop();
    result
}

fn get_own_property_names(agent: &mut Agent, c: Call) -> Res<Value> {
    own_keys_of(agent, agent.arg(c, 0), false)
}

fn get_own_property_symbols(agent: &mut Agent, c: Call) -> Res<Value> {
    own_keys_of(agent, agent.arg(c, 0), true)
}

fn has_own(agent: &mut Agent, c: Call) -> Res<Value> {
    let o = agent.object_of(agent.arg(c, 0))?;
    agent.vm.temps.push(o);
    let key = agent.property_key(agent.arg(c, 1));
    agent.vm.temps.pop();
    let key = key?;
    Ok(Value::bool(agent.own_property(o, key)?.is_some()))
}

fn is(agent: &mut Agent, c: Call) -> Res<Value> {
    Ok(Value::bool(crate::object::same_value(
        &agent.heap,
        agent.arg(c, 0),
        agent.arg(c, 1),
    )))
}

fn prevent_extensions(agent: &mut Agent, c: Call) -> Res<Value> {
    let o = agent.arg(c, 0);
    if let Some(i) = o.as_object()
        && !agent.o_prevent_extensions(i)?
    {
        let text = if agent.is_proxy(i) {
            messages::PROXY_PREVENT_FALSE
        } else {
            messages::CANNOT_PREVENT_EXTENSIONS
        };
        return Err(agent.type_error(text));
    }
    Ok(o)
}

fn seal(agent: &mut Agent, c: Call) -> Res<Value> {
    let o = agent.arg(c, 0);
    if !agent.set_integrity(o, false)? {
        return Err(agent.type_error("Cannot seal"));
    }
    Ok(o)
}

fn freeze(agent: &mut Agent, c: Call) -> Res<Value> {
    let o = agent.arg(c, 0);
    if !agent.set_integrity(o, true)? {
        return Err(agent.type_error("Cannot freeze"));
    }
    Ok(o)
}

fn create(agent: &mut Agent, c: Call) -> Res<Value> {
    let proto = agent.arg(c, 0);
    if proto.as_object().is_none() && !proto.is_null() {
        let text = agent.display(proto);
        return Err(agent.type_error(format!(
            "Object prototype may only be an Object or null: {text}"
        )));
    }
    let o = agent.alloc_object(ObjectCell::new(proto, ObjectKind::Ordinary))?;
    let props = agent.arg(c, 1);
    if !props.is_undefined() {
        agent.vm.temps.push(o);
        let r = agent.define_properties_from(o, props);
        agent.vm.temps.pop();
        r?;
    }
    Ok(o)
}

fn define_properties(agent: &mut Agent, c: Call) -> Res<Value> {
    let o = agent.arg(c, 0);
    if o.as_object().is_none() {
        return Err(agent.type_error("Object.defineProperties called on non-object"));
    }
    agent.define_properties_from(o, agent.arg(c, 1))?;
    Ok(o)
}

fn define_property(agent: &mut Agent, c: Call) -> Res<Value> {
    let o = agent.arg(c, 0);
    if o.as_object().is_none() {
        return Err(agent.type_error("Object.defineProperty called on non-object"));
    }
    let key = agent.property_key(agent.arg(c, 1))?;
    let d = agent.descriptor_of(agent.arg(c, 2))?;
    agent.define_or_throw(o, key, d)?;
    Ok(o)
}

fn get_prototype_of(agent: &mut Agent, c: Call) -> Res<Value> {
    let o = agent.object_of(agent.arg(c, 0))?;
    match o.as_object() {
        Some(i) => agent.o_get_prototype_of(i),
        None => Ok(Value::NULL),
    }
}

fn set_prototype_of(agent: &mut Agent, c: Call) -> Res<Value> {
    let o = agent.arg(c, 0);
    if o.is_nullish() {
        return Err(agent.type_error("Object.setPrototypeOf called on null or undefined"));
    }
    let proto = agent.arg(c, 1);
    if proto.as_object().is_none() && !proto.is_null() {
        let text = agent.display(proto);
        return Err(agent.type_error(format!(
            "Object prototype may only be an Object or null: {text}"
        )));
    }
    if let Some(i) = o.as_object()
        && !agent.o_set_prototype_of(i, proto)?
    {
        if agent.is_immutable_prototype(i) {
            return Err(agent.type_error(messages::IMMUTABLE_PROTOTYPE));
        }
        if agent.is_proxy(i) {
            return Err(agent.type_error(messages::PROXY_SET_PROTO_FALSE));
        }
        if agent.heap.is_extensible(i) {
            return Err(agent.type_error("Cyclic __proto__ value"));
        }
        let text = agent.display(o);
        return Err(agent.type_error(format!("{text} is not extensible")));
    }
    Ok(o)
}

fn is_extensible(agent: &mut Agent, c: Call) -> Res<Value> {
    let o = agent.arg(c, 0);
    match o.as_object() {
        Some(i) => agent.o_is_extensible(i).map(Value::bool),
        None => Ok(Value::FALSE),
    }
}

fn is_frozen(agent: &mut Agent, c: Call) -> Res<Value> {
    agent.test_integrity(agent.arg(c, 0), true).map(Value::bool)
}

fn is_sealed(agent: &mut Agent, c: Call) -> Res<Value> {
    agent
        .test_integrity(agent.arg(c, 0), false)
        .map(Value::bool)
}

fn keys(agent: &mut Agent, c: Call) -> Res<Value> {
    let o = agent.object_of(agent.arg(c, 0))?;
    agent.vm.temps.push(o);
    let r = agent.enumerable_own(o, 0);
    agent.vm.temps.pop();
    r
}

fn values(agent: &mut Agent, c: Call) -> Res<Value> {
    let o = agent.object_of(agent.arg(c, 0))?;
    agent.vm.temps.push(o);
    let r = agent.enumerable_own(o, 1);
    agent.vm.temps.pop();
    r
}

fn entries(agent: &mut Agent, c: Call) -> Res<Value> {
    let o = agent.object_of(agent.arg(c, 0))?;
    agent.vm.temps.push(o);
    let r = agent.enumerable_own(o, 2);
    agent.vm.temps.pop();
    r
}

fn define_accessor(agent: &mut Agent, c: Call, getter: bool) -> Res<Value> {
    let o = agent.object_of(c.this)?;
    let f = agent.arg(c, 1);
    if !agent.is_callable(f) {
        let which = if getter { "getter" } else { "setter" };
        return Err(agent.type_error(format!(
            "Object.prototype.__define{}__: Expecting function",
            if which == "getter" {
                "Getter"
            } else {
                "Setter"
            }
        )));
    }
    agent.vm.temps.push(o);
    let key = agent.property_key(agent.arg(c, 0));
    agent.vm.temps.pop();
    let key = key?;
    let d = Descriptor {
        get: getter.then_some(f),
        set: (!getter).then_some(f),
        enumerable: Some(true),
        configurable: Some(true),
        ..Descriptor::default()
    };
    agent.define_or_throw(o, key, d)?;
    Ok(Value::UNDEFINED)
}

fn define_getter(agent: &mut Agent, c: Call) -> Res<Value> {
    define_accessor(agent, c, true)
}

fn define_setter(agent: &mut Agent, c: Call) -> Res<Value> {
    define_accessor(agent, c, false)
}

fn lookup_accessor(agent: &mut Agent, c: Call, getter: bool) -> Res<Value> {
    let o = agent.object_of(c.this)?;
    agent.vm.temps.push(o);
    let key = agent.property_key(agent.arg(c, 0));
    agent.vm.temps.pop();
    let key = key?;
    let mut at = o.as_object();
    while let Some(i) = at {
        match agent.o_get_own_property(i, key)? {
            Some(OwnProperty::Accessor { get, set, .. }) => {
                return Ok(if getter { get } else { set });
            }
            Some(OwnProperty::Data { .. }) => return Ok(Value::UNDEFINED),
            None => at = agent.o_get_prototype_of(i)?.as_object(),
        }
    }
    Ok(Value::UNDEFINED)
}

fn lookup_getter(agent: &mut Agent, c: Call) -> Res<Value> {
    lookup_accessor(agent, c, true)
}

fn lookup_setter(agent: &mut Agent, c: Call) -> Res<Value> {
    lookup_accessor(agent, c, false)
}

fn has_own_property(agent: &mut Agent, c: Call) -> Res<Value> {
    let key = agent.property_key(agent.arg(c, 0))?;
    let o = agent.object_of(c.this)?;
    Ok(Value::bool(agent.own_property(o, key)?.is_some()))
}

fn is_prototype_of(agent: &mut Agent, c: Call) -> Res<Value> {
    let v = agent.arg(c, 0);
    let Some(mut at) = v.as_object() else {
        return Ok(Value::FALSE);
    };
    let o = agent.object_of(c.this)?;
    loop {
        let p = agent.o_get_prototype_of(at)?;
        if p.is_null() || p.as_object().is_none() {
            return Ok(Value::FALSE);
        }
        if p == o {
            return Ok(Value::TRUE);
        }
        match p.as_object() {
            Some(next) => at = next,
            None => return Ok(Value::FALSE),
        }
    }
}

fn property_is_enumerable(agent: &mut Agent, c: Call) -> Res<Value> {
    let key = agent.property_key(agent.arg(c, 0))?;
    let o = agent.object_of(c.this)?;
    let p = agent.own_property(o, key)?;
    Ok(Value::bool(p.is_some_and(|p| match p {
        OwnProperty::Data { enumerable, .. } | OwnProperty::Accessor { enumerable, .. } => {
            enumerable
        }
    })))
}

/// `Object.prototype.toString` (section 20.1.3.6).
pub(crate) fn to_string(agent: &mut Agent, c: Call) -> Res<Value> {
    let this = c.this;
    let builtin = match this {
        Value::UNDEFINED => return agent.new_string("[object Undefined]"),
        Value::NULL => return agent.new_string("[object Null]"),
        _ => {
            let o = agent.object_of(this)?;
            agent.vm.temps.push(o);
            let builtin = agent.builtin_tag(o)?;
            let key = agent.well_known_key(Wk::ToStringTag);
            let tag = agent.get_value(o, key);
            agent.vm.temps.pop();
            let tag = tag?;
            if tag.as_string().is_some() {
                let text = agent.text(tag);
                return agent.new_string(&format!("[object {text}]"));
            }
            builtin
        }
    };
    agent.new_string(&format!("[object {builtin}]"))
}

fn value_of(agent: &mut Agent, c: Call) -> Res<Value> {
    agent.object_of(c.this)
}

fn to_locale_string(agent: &mut Agent, c: Call) -> Res<Value> {
    let key = agent.realm().to_string_key;
    let f = agent.get_value(c.this, key)?;
    agent.call_value(f, c.this, &[])
}

fn get_proto(agent: &mut Agent, c: Call) -> Res<Value> {
    let o = agent.object_of(c.this)?;
    match o.as_object() {
        Some(i) => agent.o_get_prototype_of(i),
        None => Ok(Value::NULL),
    }
}

fn set_proto(agent: &mut Agent, c: Call) -> Res<Value> {
    if c.this.is_nullish() {
        return Err(agent.type_error(messages::NULLISH_TO_OBJECT));
    }
    let proto = agent.arg(c, 0);
    if proto.as_object().is_none() && !proto.is_null() {
        return Ok(Value::UNDEFINED);
    }
    let Some(i) = c.this.as_object() else {
        return Ok(Value::UNDEFINED);
    };
    if !agent.o_set_prototype_of(i, proto)? {
        if agent.is_immutable_prototype(i) {
            return Err(agent.type_error(messages::IMMUTABLE_PROTOTYPE));
        }
        if agent.heap.is_extensible(i) {
            return Err(agent.type_error("Cyclic __proto__ value"));
        }
        let text = agent.display(c.this);
        return Err(agent.type_error(format!("{text} is not extensible")));
    }
    Ok(Value::UNDEFINED)
}
