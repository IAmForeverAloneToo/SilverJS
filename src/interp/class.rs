//! Classes (ECMA-262 section 15.7): a class's constructor and prototype
//! made from its heritage, its members defined with their home objects,
//! `super.x` read and written through the home object, and `super(...)`
//! constructing the parent in a derived class's constructor.
//!
//! A function's home object is its first internal slot (a class's
//! methods', accessors' and constructor's, and an object literal's
//! methods'). A derived constructor's `this` starts uninitialized and is
//! a binding of its own, which `super()` initializes once.

use super::messages;
use super::{Abrupt, ErrorType, Res};
use crate::agent::Agent;
use crate::compiler::FunctionKind;
use crate::heap::ObjectCell;
use crate::object::{Descriptor, Got, ObjectKind, SetOutcome};
use crate::string::PropertyKey;
use crate::value::Value;

/// How a call of `super(...)` went on.
pub(crate) enum SuperCall {
    /// The parent's construction, done (a native's).
    Value(Value),
    /// A frame pushed: the loop runs it, its object to come.
    Frame,
}

impl Agent {
    /// A function's home object, if it has one.
    #[inline(never)]
    pub(crate) fn home_object(&self, f: Value) -> Value {
        f.as_object()
            .and_then(|o| self.heap.objects.get(o))
            .filter(|c| c.kind() == ObjectKind::Function)
            .and_then(|c| c.internal().first().copied())
            .unwrap_or(Value::UNDEFINED)
    }

    /// Makes `home` the home object of the function `f` (a code
    /// function's internal slots are its own).
    #[inline(never)]
    pub(crate) fn set_home_object(&mut self, f: Value, home: Value) {
        if let Some(o) = f.as_object() {
            self.heap
                .with_internal_slots(o, |slots| match slots.first_mut() {
                    Some(slot) => *slot = home,
                    None => slots.push(home),
                });
        }
    }

    /// The kind of the code function `f` is, if it is one.
    pub(crate) fn code_kind(&self, unit: u32, func: u32) -> Option<FunctionKind> {
        self.vm
            .units
            .get(unit as usize)
            .and_then(|u| u.code.functions.get(func as usize))
            .map(|f| f.kind)
    }

    /// The `this` a construction of function `func` of `unit` starts
    /// with: an object of `new_target`'s prototype, or for a derived
    /// class's constructor none yet (`super()` makes it).
    pub(crate) fn construct_this_for(
        &mut self,
        unit: u32,
        func: u32,
        new_target: Value,
    ) -> Res<Value> {
        if self.code_kind(unit, func) == Some(FunctionKind::DerivedConstructor) {
            return Ok(Value::UNINITIALIZED);
        }
        self.construct_this(new_target)
    }

    /// `CreateClass`: ClassDefinitionEvaluation's making of the prototype
    /// and the constructor (section 15.7.14, steps 5 to 18) from
    /// `parent` (`heritage` false: none), the constructor being function
    /// `func` of `unit` over `env`. Gives back the constructor and the
    /// prototype.
    #[inline(never)]
    #[allow(clippy::too_many_arguments)]
    pub(super) fn create_class(
        &mut self,
        heritage: bool,
        parent: Value,
        unit: u32,
        func: u32,
        env: Value,
    ) -> Res<(Value, Value)> {
        let (proto_parent, ctor_parent) = if !heritage {
            let r = self.realm();
            (r.object_prototype, r.function_prototype)
        } else if parent.is_null() {
            (Value::NULL, self.realm().function_prototype)
        } else if !self.is_constructor_value(parent) {
            let text = self.heritage_text(parent);
            return Err(self.throw_error(
                ErrorType::TypeError,
                messages::extends_not_constructor(&text),
            ));
        } else {
            let key = self.realm().prototype_key;
            let pp = self.get_value(parent, key)?;
            if pp.as_object().is_none() && !pp.is_null() {
                let text = self.display(pp);
                return Err(
                    self.throw_error(ErrorType::TypeError, messages::extends_bad_prototype(&text))
                );
            }
            (pp, parent)
        };
        let base = self.vm.temps.len();
        self.vm.temps.extend([parent, proto_parent]);
        let made = (|| {
            let proto = self.alloc_object(ObjectCell::new(proto_parent, ObjectKind::Ordinary))?;
            self.vm.temps.push(proto);
            let f = self.make_closure(unit, func, env)?;
            self.vm.temps.push(f);
            if let Some(o) = f.as_object() {
                self.heap.set_prototype_of(o, ctor_parent);
                let key = self.realm().prototype_key;
                self.heap.define_own_property(
                    o,
                    key,
                    Descriptor {
                        value: Some(proto),
                        writable: Some(false),
                        enumerable: Some(false),
                        configurable: Some(false),
                        ..Descriptor::default()
                    },
                );
            }
            if let Some(p) = proto.as_object() {
                let key = self.realm().constructor_key;
                self.heap.define_own_property(
                    p,
                    key,
                    Descriptor {
                        value: Some(f),
                        writable: Some(true),
                        enumerable: Some(false),
                        configurable: Some(true),
                        ..Descriptor::default()
                    },
                );
            }
            self.set_home_object(f, proto);
            Ok((f, proto))
        })();
        self.vm.temps.truncate(base);
        made
    }

    /// `DefineMethod` of a class: `f` defined on `target` under `key` as
    /// a method (0), a getter (1) or a setter (2), not enumerable.
    #[inline(never)]
    pub(super) fn define_class_member(
        &mut self,
        target: Value,
        key: Value,
        f: Value,
        how: usize,
    ) -> Res<()> {
        let key = self.property_key(key)?;
        let Some(o) = target.as_object() else {
            return Ok(());
        };
        let desc = match how {
            1 => Descriptor {
                get: Some(f),
                enumerable: Some(false),
                configurable: Some(true),
                ..Descriptor::default()
            },
            2 => Descriptor {
                set: Some(f),
                enumerable: Some(false),
                configurable: Some(true),
                ..Descriptor::default()
            },
            _ => Descriptor {
                value: Some(f),
                writable: Some(true),
                enumerable: Some(false),
                configurable: Some(true),
                ..Descriptor::default()
            },
        };
        if !self.heap.define_own_property(o, key, desc) {
            let k = self.key_display(key);
            return Err(self.throw_error(ErrorType::TypeError, messages::cannot_redefine(&k)));
        }
        Ok(())
    }

    /// `super[key]` (section 13.3.7.1): the home object's prototype's
    /// property, read with `this` as the receiver.
    #[inline(never)]
    pub(super) fn get_super(&mut self, home: Value, this: Value, key: Value) -> Res<Value> {
        let key = self.property_key(key)?;
        let base = self.super_base(home, key)?;
        self.get_value_with(base, key, this)
    }

    /// `super[key] = value`, with `this` as the receiver; strict, as class
    /// code is, a refusal throws.
    #[inline(never)]
    pub(super) fn set_super(
        &mut self,
        home: Value,
        this: Value,
        key: Value,
        value: Value,
    ) -> Res<()> {
        let key = self.property_key(key)?;
        let base = self.super_base(home, key)?;
        self.set_value_with(base, key, value, this)
    }

    /// The object `super.x` reads from: the home object's prototype.
    fn super_base(&mut self, home: Value, key: PropertyKey) -> Res<Value> {
        let base = home
            .as_object()
            .map_or(Value::NULL, |o| self.heap.get_prototype_of(o));
        if base.is_nullish() {
            let text = self.display(base);
            let k = self.key_display(key);
            return Err(
                self.throw_error(ErrorType::TypeError, messages::read_properties(&text, &k))
            );
        }
        Ok(base)
    }

    /// A heritage that is no constructor, as the reference browser names
    /// it in the error: its ToString (a function's source text).
    fn heritage_text(&mut self, v: Value) -> String {
        if self.callable(v).is_some() {
            return self.function_source(v).unwrap_or_else(|| self.display(v));
        }
        self.display(v)
    }

    /// `super(...)` (section 13.3.7.1): the parent of the derived
    /// constructor `f` constructed with `new_target`, its arguments from
    /// `args` to the top of the register file (which the call takes).
    #[inline(never)]
    pub(super) fn super_call(
        &mut self,
        f: Value,
        new_target: Value,
        args: usize,
        start: u32,
        resume: u32,
    ) -> Res<SuperCall> {
        let parent = f
            .as_object()
            .map_or(Value::NULL, |o| self.heap.get_prototype_of(o));
        if !self.is_constructor_value(parent) {
            self.vm.stack.truncate(args);
            // A function with no name (`extends null`'s parent,
            // %Function.prototype%) shows as null (Chrome 157).
            let text = match self.function_name(parent) {
                Some(n) if !n.is_empty() => n,
                Some(_) => "null".to_owned(),
                None => self.display(parent),
            };
            let class = self
                .function_name(f)
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| "anonymous class".to_owned());
            return Err(self.throw_error(
                ErrorType::TypeError,
                messages::super_not_constructor(&text, &class),
            ));
        }
        if let Some(top) = self.vm.frames.last_mut() {
            top.pc = start;
            top.resume = resume;
        }
        match self.begin_call_owned(parent, Value::UNDEFINED, args, Some(new_target))? {
            Some(v) => Ok(SuperCall::Value(v)),
            None => Ok(SuperCall::Frame),
        }
    }

    /// The error of a derived constructor's `this` read, or of its
    /// return, before `super()` was called.
    #[inline(never)]
    pub(crate) fn this_before_super(&mut self) -> Abrupt {
        self.throw_error(ErrorType::ReferenceError, messages::THIS_BEFORE_SUPER)
    }

    /// `ReturnDerived`: what a derived constructor's [[Construct]] gives
    /// back (section 10.2.2, steps 9 to 12), its body having returned
    /// `value` with its `this` binding `this`.
    #[inline(never)]
    pub(super) fn derived_return(&mut self, value: Value, this: Value) -> Res<Value> {
        if value.as_object().is_some() {
            return Ok(value);
        }
        if !value.is_undefined() {
            return Err(self.throw_error(ErrorType::TypeError, messages::DERIVED_RETURN));
        }
        if this == Value::UNINITIALIZED {
            return Err(self.this_before_super());
        }
        Ok(this)
    }

    /// A class constructor called without `new`.
    #[inline(never)]
    pub(crate) fn class_call_error(&mut self, f: Value) -> Abrupt {
        let name = self.function_name(f).unwrap_or_default();
        self.throw_error(ErrorType::TypeError, messages::class_without_new(&name))
    }

    /// The elements of the array `array` onto the top of the register
    /// file; gives back where they start.
    #[inline(never)]
    pub(super) fn push_array_elements(&mut self, array: Value) -> Res<usize> {
        let at = self.vm.stack.len();
        let n = array
            .as_object()
            .and_then(|o| self.heap.objects.get(o))
            .and_then(|c| c.array_length())
            .map_or(0, |(len, _)| len);
        for i in 0..n {
            match self.get_value(array, PropertyKey::Index(i)) {
                Ok(v) => self.vm.stack.push(v),
                Err(e) => {
                    self.vm.stack.truncate(at);
                    return Err(e);
                }
            }
        }
        Ok(at)
    }

    /// Registers `first` to `first + n` of the frame at `base`, onto the
    /// top of the register file; gives back where they start.
    #[inline(never)]
    pub(super) fn push_registers(&mut self, base: u32, first: usize, n: usize) -> usize {
        let at = self.vm.stack.len();
        for i in 0..n {
            let v = self.reg(base, first + i);
            self.vm.stack.push(v);
        }
        at
    }

    /// The arguments of the running frame, onto the top of the register
    /// file (a default derived constructor's `super(...args)`).
    #[inline(never)]
    pub(super) fn push_own_arguments(&mut self) -> usize {
        let at = self.vm.stack.len();
        if let Some(f) = self.vm.frames.last() {
            let (from, to) = (f.args as usize, f.base as usize);
            let args: Vec<Value> = self.vm.stack.get(from..to).unwrap_or(&[]).to_vec();
            self.vm.stack.extend(args);
        }
        at
    }

    /// `base[key]` read with `receiver` as `this` for a getter.
    fn get_value_with(&mut self, base: Value, key: PropertyKey, receiver: Value) -> Res<Value> {
        let Some(o) = base.as_object() else {
            return self.get_value(base, key);
        };
        match self.heap.get(o, key) {
            Got::Value(v) => Ok(v),
            Got::Getter(g) if g.is_undefined() => Ok(Value::UNDEFINED),
            Got::Getter(g) => self.call_value(g, receiver, &[]),
        }
    }

    /// `base[key] = value` with `receiver` as the object set (and a
    /// setter's `this`); a refusal throws, as in strict code.
    fn set_value_with(
        &mut self,
        base: Value,
        key: PropertyKey,
        value: Value,
        receiver: Value,
    ) -> Res<()> {
        let Some(o) = base.as_object() else {
            return self.set_value(base, key, value, true);
        };
        match self.heap.set(o, key, value, receiver) {
            SetOutcome::Done(true) => Ok(()),
            SetOutcome::Done(false) => {
                let k = self.key_display(key);
                Err(self.throw_error(ErrorType::TypeError, messages::read_only(&k, "#<Object>")))
            }
            SetOutcome::Setter(s) if s.is_undefined() => {
                let k = self.key_display(key);
                Err(self.throw_error(ErrorType::TypeError, messages::read_only(&k, "#<Object>")))
            }
            SetOutcome::Setter(s) => self.call_value(s, receiver, &[value]).map(|_| ()),
        }
    }
}
