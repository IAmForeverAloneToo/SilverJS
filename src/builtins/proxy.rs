//! Proxies (ECMA-262 sections 10.5 and 28.2) and the objects' internal
//! methods as the agent runs them: an ordinary object's from the heap, a
//! proxy's through its handler's trap (or its target's), its answer held
//! to the invariants the target's own properties impose.
//!
//! A proxy is an object of its own kind whose internal slots are its
//! target and its handler (both null once revoked); one whose target is
//! callable is callable, and a constructor if the target is one.

use super::{Call, Entry, Name, method};
use crate::agent::Agent;
use crate::heap::ObjectCell;
use crate::interp::{Res, messages};
use crate::object::{Callable, Descriptor, Got, ObjectKind, OwnProperty, SetOutcome};
use crate::string::PropertyKey;
use crate::value::Value;

/// `Proxy`.
pub(crate) const PROXY: &[Entry] = &[method("revocable", 2, revocable)];

/// The natives proxies make: `revocable`'s revoke function.
pub(crate) const PROXY_INTERNAL: &[Entry] = &[method("revoke", 0, revoke)];

/// `Proxy(target, handler)` (section 28.2.1.1).
pub(crate) fn construct(agent: &mut Agent, c: Call) -> Res<Value> {
    if c.new_target.is_undefined() {
        return Err(agent.type_error(messages::PROXY_WITHOUT_NEW));
    }
    let (t, h) = (agent.arg(c, 0), agent.arg(c, 1));
    agent.new_proxy(t, h)
}

/// `Proxy.revocable(target, handler)` (section 28.2.2.1).
fn revocable(agent: &mut Agent, c: Call) -> Res<Value> {
    let (t, h) = (agent.arg(c, 0), agent.arg(c, 1));
    let p = agent.new_proxy(t, h)?;
    agent.vm.temps.push(p);
    let r = (|| {
        let id = super::native_id(super::table::PROXY_INTERNAL, 0);
        let revoke = agent.native_function(id, 0, "")?;
        if let Some(o) = revoke.as_object() {
            agent.heap.with_internal_slots(o, |s| *s = vec![p]);
        }
        agent.vm.temps.push(revoke);
        let proto = agent.realm().object_prototype;
        let result = agent.alloc_object(ObjectCell::new(proto, ObjectKind::Ordinary));
        agent.vm.temps.pop();
        let result = result?;
        let k = agent.name_key(Name::Str("proxy"));
        agent.create_data_property(result, k, p);
        let k = agent.name_key(Name::Str("revoke"));
        agent.create_data_property(result, k, revoke);
        Ok(result)
    })();
    agent.vm.temps.pop();
    r
}

fn revoke(agent: &mut Agent, c: Call) -> Res<Value> {
    let slots = agent.slots_of_value(c.callee);
    if let Some(p) = slots.first().copied().filter(|p| !p.is_null())
        && let Some(o) = p.as_object()
    {
        agent
            .heap
            .with_internal_slots(o, |s| *s = vec![Value::NULL, Value::NULL]);
        if let Some(f) = c.callee.as_object() {
            agent
                .heap
                .with_internal_slots(f, |s| *s = vec![Value::NULL]);
        }
    }
    Ok(Value::UNDEFINED)
}

impl Agent {
    /// ProxyCreate (section 10.5.14).
    pub(crate) fn new_proxy(&mut self, target: Value, handler: Value) -> Res<Value> {
        if target.as_object().is_none() || handler.as_object().is_none() {
            return Err(self.type_error(messages::PROXY_NON_OBJECT));
        }
        let callable = self.callable(target).is_some();
        let mut cell =
            ObjectCell::with_internal(Value::NULL, ObjectKind::Proxy, vec![target, handler]);
        if callable {
            cell.set_callable(Callable::Proxy);
        }
        self.vm.temps.extend([target, handler]);
        let p = self.alloc_object(cell);
        let n = self.vm.temps.len();
        self.vm.temps.truncate(n.saturating_sub(2));
        p
    }

    /// An object's internal slots, copied.
    pub(crate) fn slots_of_value(&self, v: Value) -> Vec<Value> {
        v.as_object()
            .and_then(|o| self.heap.objects.get(o))
            .map(|c| c.internal().to_vec())
            .unwrap_or_default()
    }

    /// Whether `o` is a proxy.
    pub(crate) fn is_proxy(&self, o: u32) -> bool {
        self.heap
            .objects
            .get(o)
            .is_some_and(|c| c.kind() == ObjectKind::Proxy)
    }

    /// A proxy's target and handler, or the TypeError of a revoked one
    /// asked `what`.
    fn proxy_parts(&mut self, o: u32, what: &str) -> Res<(Value, Value)> {
        let s = self
            .heap
            .objects
            .get(o)
            .map(|c| c.internal().to_vec())
            .unwrap_or_default();
        let target = s.first().copied().unwrap_or(Value::NULL);
        let handler = s.get(1).copied().unwrap_or(Value::NULL);
        if handler.is_null() {
            return Err(self.type_error(messages::proxy_revoked(what)));
        }
        Ok((target, handler))
    }

    /// A proxy's trap `name` (GetMethod of its handler), with its target
    /// and handler.
    fn proxy_trap(&mut self, o: u32, name: &str) -> Res<(Value, Value, Value)> {
        let (target, handler) = self.proxy_parts(o, name)?;
        let key = self.name_key(Name::Str(leak_name(name)));
        let trap = self.get_value(handler, key)?;
        if trap.is_nullish() {
            return Ok((target, handler, Value::UNDEFINED));
        }
        if !self.is_callable(trap) {
            let text = self.display(trap);
            let owner = self.display(handler);
            return Err(self.type_error(messages::proxy_trap_not_function(name, &text, &owner)));
        }
        Ok((target, handler, trap))
    }

    fn key_text(&mut self, key: PropertyKey) -> String {
        self.key_display(key)
    }

    // The internal methods (section 6.1.7.2), for any object.

    /// `[[GetPrototypeOf]]`.
    pub(crate) fn o_get_prototype_of(&mut self, o: u32) -> Res<Value> {
        if !self.is_proxy(o) {
            return Ok(self.heap.get_prototype_of(o));
        }
        let (target, handler, trap) = self.proxy_trap(o, "getPrototypeOf")?;
        let t = target.as_object().unwrap_or(0);
        if trap.is_undefined() {
            return self.o_get_prototype_of(t);
        }
        let proto = self.call_value(trap, handler, &[target])?;
        if proto.as_object().is_none() && !proto.is_null() {
            return Err(self.type_error(messages::PROXY_PROTO_BAD));
        }
        if self.o_is_extensible(t)? {
            return Ok(proto);
        }
        if proto != self.o_get_prototype_of(t)? {
            return Err(self.type_error(messages::PROXY_PROTO_INVARIANT));
        }
        Ok(proto)
    }

    /// `[[SetPrototypeOf]]`.
    pub(crate) fn o_set_prototype_of(&mut self, o: u32, proto: Value) -> Res<bool> {
        if !self.is_proxy(o) {
            return Ok(self.heap.set_prototype_of(o, proto));
        }
        let (target, handler, trap) = self.proxy_trap(o, "setPrototypeOf")?;
        let t = target.as_object().unwrap_or(0);
        if trap.is_undefined() {
            return self.o_set_prototype_of(t, proto);
        }
        let r = self.call_value(trap, handler, &[target, proto])?;
        if !self.to_boolean(r) {
            return Ok(false);
        }
        if self.o_is_extensible(t)? {
            return Ok(true);
        }
        if proto != self.o_get_prototype_of(t)? {
            return Err(self.type_error(messages::PROXY_SET_PROTO_INVARIANT));
        }
        Ok(true)
    }

    /// `[[IsExtensible]]`.
    pub(crate) fn o_is_extensible(&mut self, o: u32) -> Res<bool> {
        if !self.is_proxy(o) {
            return Ok(self.heap.is_extensible(o));
        }
        let (target, handler, trap) = self.proxy_trap(o, "isExtensible")?;
        let t = target.as_object().unwrap_or(0);
        if trap.is_undefined() {
            return self.o_is_extensible(t);
        }
        let r = self.call_value(trap, handler, &[target])?;
        let r = self.to_boolean(r);
        let expected = self.o_is_extensible(t)?;
        if r != expected {
            return Err(self.type_error(messages::proxy_extensible_invariant(expected)));
        }
        Ok(r)
    }

    /// `[[PreventExtensions]]`.
    pub(crate) fn o_prevent_extensions(&mut self, o: u32) -> Res<bool> {
        if !self.is_proxy(o) {
            return Ok(self.heap.prevent_extensions(o));
        }
        let (target, handler, trap) = self.proxy_trap(o, "preventExtensions")?;
        let t = target.as_object().unwrap_or(0);
        if trap.is_undefined() {
            return self.o_prevent_extensions(t);
        }
        let r = self.call_value(trap, handler, &[target])?;
        let r = self.to_boolean(r);
        if r && self.o_is_extensible(t)? {
            return Err(self.type_error(messages::PROXY_PREVENT_INVARIANT));
        }
        Ok(r)
    }

    /// `[[GetOwnProperty]]`.
    pub(crate) fn o_get_own_property(
        &mut self,
        o: u32,
        key: PropertyKey,
    ) -> Res<Option<OwnProperty>> {
        if !self.is_proxy(o) {
            return Ok(self.heap.get_own_property(o, key));
        }
        let (target, handler, trap) = self.proxy_trap(o, "getOwnPropertyDescriptor")?;
        let t = target.as_object().unwrap_or(0);
        if trap.is_undefined() {
            return self.o_get_own_property(t, key);
        }
        let k = self.key_value(key)?;
        let r = self.call_value(trap, handler, &[target, k])?;
        if r.as_object().is_none() && !r.is_undefined() {
            let text = self.key_text(key);
            return Err(self.type_error(messages::proxy_gopd_bad(&text)));
        }
        let current = self.o_get_own_property(t, key)?;
        if r.is_undefined() {
            let Some(current) = current else {
                return Ok(None);
            };
            if !current.configurable() {
                let text = self.key_text(key);
                return Err(self.type_error(messages::proxy_gopd_hide_nonconfigurable(&text)));
            }
            if !self.o_is_extensible(t)? {
                let text = self.key_text(key);
                return Err(self.type_error(messages::proxy_gopd_hide_nonextensible(&text)));
            }
            return Ok(None);
        }
        let extensible = self.o_is_extensible(t)?;
        let desc = self.descriptor_of(r)?;
        let result = complete(desc);
        if !compatible(self, extensible, &desc, current) {
            let text = self.key_text(key);
            return Err(self.type_error(messages::proxy_gopd_incompatible(&text)));
        }
        if !result.configurable() {
            let non_configurable_current = current.is_some_and(|c| !c.configurable());
            if !non_configurable_current {
                let text = self.key_text(key);
                return Err(self.type_error(messages::proxy_gopd_nonconfigurable(&text)));
            }
            if let (
                OwnProperty::Data {
                    writable: false, ..
                },
                Some(OwnProperty::Data { writable: true, .. }),
            ) = (result, current)
            {
                let text = self.key_text(key);
                return Err(self.type_error(messages::proxy_gopd_nonconfigurable(&text)));
            }
        }
        Ok(Some(result))
    }

    /// `[[DefineOwnProperty]]`.
    pub(crate) fn o_define_own_property(
        &mut self,
        o: u32,
        key: PropertyKey,
        desc: Descriptor,
    ) -> Res<bool> {
        if !self.is_proxy(o) {
            return Ok(self.heap.define_own_property(o, key, desc));
        }
        let (target, handler, trap) = self.proxy_trap(o, "defineProperty")?;
        let t = target.as_object().unwrap_or(0);
        if trap.is_undefined() {
            return self.o_define_own_property(t, key, desc);
        }
        let k = self.key_value(key)?;
        self.vm.temps.push(k);
        let d = self.descriptor_from(&desc);
        self.vm.temps.pop();
        let d = d?;
        let r = self.call_value(trap, handler, &[target, k, d])?;
        if !self.to_boolean(r) {
            return Ok(false);
        }
        let current = self.o_get_own_property(t, key)?;
        let extensible = self.o_is_extensible(t)?;
        let setting_non_configurable = desc.configurable == Some(false);
        match current {
            None => {
                if !extensible {
                    let text = self.key_text(key);
                    return Err(self.type_error(messages::proxy_define_nonextensible(&text)));
                }
                if setting_non_configurable {
                    let text = self.key_text(key);
                    return Err(self.type_error(messages::proxy_define_nonconfigurable(&text)));
                }
            }
            Some(c) => {
                if !compatible(self, extensible, &desc, Some(c)) {
                    let text = self.key_text(key);
                    return Err(self.type_error(messages::proxy_define_incompatible(&text)));
                }
                if setting_non_configurable && c.configurable() {
                    let text = self.key_text(key);
                    return Err(self.type_error(messages::proxy_define_nonconfigurable(&text)));
                }
                if let OwnProperty::Data {
                    writable: true,
                    configurable: false,
                    ..
                } = c
                    && desc.writable == Some(false)
                {
                    let text = self.key_text(key);
                    return Err(self.type_error(messages::proxy_define_nonconfigurable(&text)));
                }
            }
        }
        Ok(true)
    }

    /// `[[HasProperty]]`, along the prototype chain.
    pub(crate) fn o_has_property(&mut self, o: u32, key: PropertyKey) -> Res<bool> {
        let mut at = o;
        loop {
            if self.is_proxy(at) {
                return self.proxy_has(at, key);
            }
            if self.heap.get_own_property(at, key).is_some() {
                return Ok(true);
            }
            match self.heap.get_prototype_of(at).as_object() {
                Some(p) => at = p,
                None => return Ok(false),
            }
        }
    }

    fn proxy_has(&mut self, o: u32, key: PropertyKey) -> Res<bool> {
        let (target, handler, trap) = self.proxy_trap(o, "has")?;
        let t = target.as_object().unwrap_or(0);
        if trap.is_undefined() {
            return self.o_has_property(t, key);
        }
        let k = self.key_value(key)?;
        let r = self.call_value(trap, handler, &[target, k])?;
        let r = self.to_boolean(r);
        if !r && let Some(c) = self.o_get_own_property(t, key)? {
            if !c.configurable() {
                let text = self.key_text(key);
                return Err(self.type_error(messages::proxy_has_nonconfigurable(&text)));
            }
            if !self.o_is_extensible(t)? {
                let text = self.key_text(key);
                return Err(self.type_error(messages::proxy_has_nonextensible(&text)));
            }
        }
        Ok(r)
    }

    /// `[[Get]]` of `o` with `receiver`, along the prototype chain.
    pub(crate) fn o_get(&mut self, o: u32, key: PropertyKey, receiver: Value) -> Res<Value> {
        match self.heap.get(o, key) {
            Got::Value(v) => Ok(v),
            Got::Getter(g) if g.is_undefined() => Ok(Value::UNDEFINED),
            Got::Getter(g) => {
                self.vm.accessor_call = true;
                self.call_value(g, receiver, &[])
            }
            Got::Proxy(p) => self.proxy_get(p, key, receiver),
        }
    }

    pub(crate) fn proxy_get(&mut self, o: u32, key: PropertyKey, receiver: Value) -> Res<Value> {
        let (target, handler, trap) = self.proxy_trap(o, "get")?;
        let t = target.as_object().unwrap_or(0);
        if trap.is_undefined() {
            return self.o_get(t, key, receiver);
        }
        let k = self.key_value(key)?;
        let v = self.call_value(trap, handler, &[target, k, receiver])?;
        match self.o_get_own_property(t, key)? {
            Some(OwnProperty::Data {
                value,
                writable: false,
                configurable: false,
                ..
            }) if !crate::object::same_value(&self.heap, v, value) => {
                let (text, want, got) = (self.key_text(key), self.display(value), self.display(v));
                Err(self.type_error(messages::proxy_get_data(&text, &want, &got)))
            }
            Some(OwnProperty::Accessor {
                get,
                configurable: false,
                ..
            }) if get.is_undefined() && !v.is_undefined() => {
                let (text, got) = (self.key_text(key), self.display(v));
                Err(self.type_error(messages::proxy_get_accessor(&text, &got)))
            }
            _ => Ok(v),
        }
    }

    /// `[[Set]]` of `o` with `receiver`: whether it was done.
    pub(crate) fn o_set(
        &mut self,
        o: u32,
        key: PropertyKey,
        value: Value,
        receiver: Value,
    ) -> Res<bool> {
        match self.heap.set(o, key, value, receiver) {
            SetOutcome::Done(done) => Ok(done),
            SetOutcome::Setter(s) => {
                self.call_value(s, receiver, &[value])?;
                Ok(true)
            }
            SetOutcome::Proxy(p) => self.proxy_set(p, key, value, receiver),
            SetOutcome::Receiver(r) => self.set_on_receiver(r, key, value),
        }
    }

    /// OrdinarySetWithOwnDescriptor's last steps (section 10.1.9.2) on a
    /// receiver that is a proxy: its own property written, or made.
    pub(crate) fn set_on_receiver(&mut self, r: u32, key: PropertyKey, value: Value) -> Res<bool> {
        match self.o_get_own_property(r, key)? {
            Some(OwnProperty::Accessor { .. })
            | Some(OwnProperty::Data {
                writable: false, ..
            }) => Ok(false),
            Some(OwnProperty::Data { .. }) => self.o_define_own_property(
                r,
                key,
                Descriptor {
                    value: Some(value),
                    ..Descriptor::default()
                },
            ),
            None => self.o_define_own_property(r, key, Descriptor::data(value)),
        }
    }

    pub(crate) fn proxy_set(
        &mut self,
        o: u32,
        key: PropertyKey,
        value: Value,
        receiver: Value,
    ) -> Res<bool> {
        let (target, handler, trap) = self.proxy_trap(o, "set")?;
        let t = target.as_object().unwrap_or(0);
        if trap.is_undefined() {
            return self.o_set(t, key, value, receiver);
        }
        let k = self.key_value(key)?;
        let r = self.call_value(trap, handler, &[target, k, value, receiver])?;
        if !self.to_boolean(r) {
            return Ok(false);
        }
        match self.o_get_own_property(t, key)? {
            Some(OwnProperty::Data {
                value: current,
                writable: false,
                configurable: false,
                ..
            }) if !crate::object::same_value(&self.heap, value, current) => {
                let text = self.key_text(key);
                Err(self.type_error(messages::proxy_set_data(&text)))
            }
            Some(OwnProperty::Accessor {
                set,
                configurable: false,
                ..
            }) if set.is_undefined() => {
                let text = self.key_text(key);
                Err(self.type_error(messages::proxy_set_accessor(&text)))
            }
            _ => Ok(true),
        }
    }

    /// `[[Delete]]`: whether it was done.
    pub(crate) fn o_delete(&mut self, o: u32, key: PropertyKey) -> Res<bool> {
        if !self.is_proxy(o) {
            return Ok(self.heap.delete(o, key));
        }
        let (target, handler, trap) = self.proxy_trap(o, "deleteProperty")?;
        let t = target.as_object().unwrap_or(0);
        if trap.is_undefined() {
            return self.o_delete(t, key);
        }
        let k = self.key_value(key)?;
        let r = self.call_value(trap, handler, &[target, k])?;
        if !self.to_boolean(r) {
            return Ok(false);
        }
        if let Some(c) = self.o_get_own_property(t, key)? {
            if !c.configurable() {
                let text = self.key_text(key);
                return Err(self.type_error(messages::proxy_delete_nonconfigurable(&text)));
            }
            if !self.o_is_extensible(t)? {
                let text = self.key_text(key);
                return Err(self.type_error(messages::proxy_delete_nonextensible(&text)));
            }
        }
        Ok(true)
    }

    /// `[[OwnPropertyKeys]]`.
    pub(crate) fn o_own_keys(&mut self, o: u32) -> Res<Vec<PropertyKey>> {
        if !self.is_proxy(o) {
            return Ok(self.heap.own_keys(o));
        }
        let (target, handler, trap) = self.proxy_trap(o, "ownKeys")?;
        let t = target.as_object().unwrap_or(0);
        if trap.is_undefined() {
            return self.o_own_keys(t);
        }
        let r = self.call_value(trap, handler, &[target])?;
        if r.as_object().is_none() {
            return Err(self.type_error(messages::PROXY_OWN_KEYS_NOT_OBJECT));
        }
        let list = self.list_from_array_like(r)?;
        let mut keys = Vec::with_capacity(list.len());
        for v in list {
            let symbol = matches!(v.as_reference(), Some((crate::value::RefKind::Symbol, _)));
            if v.as_string().is_none() && !symbol {
                let text = self.display(v);
                return Err(self.type_error(messages::proxy_own_keys_bad(&text)));
            }
            let k = self.property_key(v)?;
            if keys.contains(&k) {
                return Err(self.type_error(messages::PROXY_OWN_KEYS_DUPLICATE));
            }
            keys.push(k);
        }
        let extensible = self.o_is_extensible(t)?;
        let target_keys = self.o_own_keys(t)?;
        let mut configurable = Vec::new();
        let mut non_configurable = Vec::new();
        for k in target_keys {
            match self.o_get_own_property(t, k)? {
                Some(p) if !p.configurable() => non_configurable.push(k),
                _ => configurable.push(k),
            }
        }
        if extensible && non_configurable.is_empty() {
            return Ok(keys);
        }
        let mut unchecked = keys.clone();
        for k in non_configurable {
            match unchecked.iter().position(|u| *u == k) {
                Some(i) => {
                    unchecked.remove(i);
                }
                None => {
                    let text = self.key_text(k);
                    return Err(self.type_error(messages::proxy_own_keys_missing(&text)));
                }
            }
        }
        if extensible {
            return Ok(keys);
        }
        for k in configurable {
            match unchecked.iter().position(|u| *u == k) {
                Some(i) => {
                    unchecked.remove(i);
                }
                None => {
                    let text = self.key_text(k);
                    return Err(
                        self.type_error(messages::proxy_own_keys_missing_nonextensible(&text))
                    );
                }
            }
        }
        if !unchecked.is_empty() {
            return Err(self.type_error(messages::PROXY_OWN_KEYS_EXTRA));
        }
        Ok(keys)
    }

    /// A callable proxy's `[[Call]]` (section 10.5.12).
    pub(crate) fn proxy_call(&mut self, o: u32, this: Value, args: &[Value]) -> Res<Value> {
        let (target, handler, trap) = self.proxy_trap(o, "apply")?;
        if trap.is_undefined() {
            return self.call_value(target, this, args);
        }
        let list = self.array_of(args)?;
        self.vm.temps.push(list);
        let r = self.call_value(trap, handler, &[target, this, list]);
        self.vm.temps.pop();
        r
    }

    /// A constructor proxy's `[[Construct]]` (section 10.5.13).
    pub(crate) fn proxy_construct(
        &mut self,
        o: u32,
        args: &[Value],
        new_target: Value,
    ) -> Res<Value> {
        let (target, handler, trap) = self.proxy_trap(o, "construct")?;
        if trap.is_undefined() {
            return self.construct_value(target, args, new_target);
        }
        let list = self.array_of(args)?;
        self.vm.temps.push(list);
        let r = self.call_value(trap, handler, &[target, list, new_target]);
        self.vm.temps.pop();
        let r = r?;
        if r.as_object().is_none() {
            let text = self.display(r);
            return Err(self.type_error(messages::proxy_construct_not_object(&text)));
        }
        Ok(r)
    }

    /// Whether a proxy is a constructor: its target is.
    pub(crate) fn proxy_constructs(&self, o: u32) -> bool {
        let target = self
            .heap
            .objects
            .get(o)
            .and_then(|c| c.internal().first().copied())
            .unwrap_or(Value::NULL);
        self.is_constructor_value(target)
    }

    /// A descriptor as an object (FromPropertyDescriptor of a partial
    /// one): only the fields it has.
    fn descriptor_from(&mut self, d: &Descriptor) -> Res<Value> {
        let o = self.plain_object()?;
        self.vm.temps.push(o);
        let fields: [(&'static str, Option<Value>); 6] = [
            ("value", d.value),
            ("writable", d.writable.map(Value::bool)),
            ("get", d.get),
            ("set", d.set),
            ("enumerable", d.enumerable.map(Value::bool)),
            ("configurable", d.configurable.map(Value::bool)),
        ];
        for (name, v) in fields {
            if let Some(v) = v {
                let k = self.name_key(Name::Str(name));
                self.create_data_property(o, k, v);
            }
        }
        self.vm.temps.pop();
        Ok(o)
    }
}

/// The trap names, as `'static` strings for the table keys.
fn leak_name(name: &str) -> &'static str {
    match name {
        "get" => "get",
        "set" => "set",
        "has" => "has",
        "deleteProperty" => "deleteProperty",
        "ownKeys" => "ownKeys",
        "getOwnPropertyDescriptor" => "getOwnPropertyDescriptor",
        "defineProperty" => "defineProperty",
        "getPrototypeOf" => "getPrototypeOf",
        "setPrototypeOf" => "setPrototypeOf",
        "isExtensible" => "isExtensible",
        "preventExtensions" => "preventExtensions",
        "apply" => "apply",
        _ => "construct",
    }
}

/// CompletePropertyDescriptor (section 6.2.6.6).
fn complete(d: Descriptor) -> OwnProperty {
    if d.get.is_some() || d.set.is_some() {
        OwnProperty::Accessor {
            get: d.get.unwrap_or(Value::UNDEFINED),
            set: d.set.unwrap_or(Value::UNDEFINED),
            enumerable: d.enumerable.unwrap_or(false),
            configurable: d.configurable.unwrap_or(false),
        }
    } else {
        OwnProperty::Data {
            value: d.value.unwrap_or(Value::UNDEFINED),
            writable: d.writable.unwrap_or(false),
            enumerable: d.enumerable.unwrap_or(false),
            configurable: d.configurable.unwrap_or(false),
        }
    }
}

/// IsCompatiblePropertyDescriptor (section 10.1.6.2): whether `desc`
/// could be applied to an object with `current` (none: absent).
fn compatible(
    agent: &Agent,
    extensible: bool,
    desc: &Descriptor,
    current: Option<OwnProperty>,
) -> bool {
    let Some(current) = current else {
        return extensible;
    };
    if current.configurable() {
        return true;
    }
    if desc.configurable == Some(true) {
        return false;
    }
    if desc.enumerable.is_some_and(|e| e != current.enumerable()) {
        return false;
    }
    let is_accessor = desc.get.is_some() || desc.set.is_some();
    let is_data = desc.value.is_some() || desc.writable.is_some();
    match current {
        OwnProperty::Data {
            value, writable, ..
        } => {
            if is_accessor {
                return false;
            }
            if !writable {
                if desc.writable == Some(true) {
                    return false;
                }
                if desc
                    .value
                    .is_some_and(|v| !crate::object::same_value(&agent.heap, v, value))
                {
                    return false;
                }
            }
            true
        }
        OwnProperty::Accessor { get, set, .. } => {
            if is_data {
                return false;
            }
            !(desc
                .get
                .is_some_and(|g| !crate::object::same_value(&agent.heap, g, get))
                || desc
                    .set
                    .is_some_and(|s| !crate::object::same_value(&agent.heap, s, set)))
        }
    }
}
