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
use super::{Abrupt, ErrorType, Reader, Res};
use crate::agent::Agent;
use crate::compiler::FunctionKind;
use crate::compiler::Op;
use crate::heap::{ObjectCell, PrivateKind, PrivateName};
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
        flags: usize,
        parent: Value,
        unit: u32,
        func: u32,
        env: Value,
    ) -> Res<(Value, Value)> {
        let (heritage, branded) = (flags & 1 != 0, flags & 2 != 0);
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
            // Its home object, the prototype; its brand, if it has
            // private methods: a private name its instances hold,
            // described by the class's name.
            let brand = if branded {
                let name = self.function_name(f).unwrap_or_default();
                let d = self.new_string(&name)?;
                self.vm.temps.push(d);
                let b = self.new_private_name(d);
                self.vm.temps.pop();
                b?
            } else {
                Value::UNDEFINED
            };
            if let Some(o) = f.as_object() {
                self.heap.with_internal_slots(o, |s| {
                    s.clear();
                    s.push(proto);
                    s.push(brand);
                });
            }
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

    /// `super[key]` read (section 13.3.7.1): `base`'s property (the home
    /// object's prototype, read when the reference was made), with `this`
    /// as the receiver; the key converted now (GetValue).
    #[inline(never)]
    pub(super) fn get_super(&mut self, base: Value, this: Value, key: Value) -> Res<Value> {
        self.check_super_base(base, key)?;
        let key = self.property_key(key)?;
        self.get_value_with(base, key, this)
    }

    /// `super[key] = value`, with `this` as the receiver; in strict code
    /// a refusal throws.
    #[inline(never)]
    pub(super) fn set_super(
        &mut self,
        base: Value,
        this: Value,
        key: Value,
        value: Value,
        strict: bool,
    ) -> Res<()> {
        self.check_super_base(base, key)?;
        let key = self.property_key(key)?;
        self.set_value_with(base, key, value, this, strict)
    }

    /// A super reference's base, null when the home object has no
    /// prototype: then a TypeError, before the key is converted
    /// (GetValue's ToObject).
    fn check_super_base(&mut self, base: Value, key: Value) -> Res<()> {
        if base.is_nullish() {
            let text = self.display(base);
            let k = match key.as_object() {
                Some(_) => self.display(key),
                None => {
                    let k = self.property_key(key)?;
                    self.key_display(k)
                }
            };
            return Err(
                self.throw_error(ErrorType::TypeError, messages::read_properties(&text, &k))
            );
        }
        Ok(())
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

    /// A private name's record and its text (`#x`), if `name` is one.
    fn private_of(&self, name: Value) -> Option<(PropertyKey, PrivateName)> {
        let (_, i) = name.as_reference()?;
        let p = *self.heap.symbols.get(i)?.private.as_deref()?;
        Some((PropertyKey::Symbol(i), p))
    }

    /// The text of a symbol's description (a private name's `#x`, a
    /// brand's class name).
    fn description_text(&self, name: Value) -> String {
        let d = name
            .as_reference()
            .and_then(|(_, i)| self.heap.symbols.get(i))
            .map_or(Value::UNDEFINED, |c| c.description);
        self.string_units(d)
            .map(|u| String::from_utf16_lossy(&u))
            .unwrap_or_default()
    }

    /// `DefinePrivateMethod`: the private name `name` made a method (how
    /// 0), getter (1) or setter (2) of `class` (how & 4: static), `f`.
    #[inline(never)]
    pub(super) fn define_private_method(
        &mut self,
        name: Value,
        class: Value,
        f: Value,
        how: usize,
    ) {
        let is_static = how & 4 != 0;
        let brand = if is_static {
            class
        } else {
            self.class_slot(class, 1)
        };
        let Some(cell) = name
            .as_reference()
            .and_then(|(_, i)| self.heap.symbols.get_mut(i))
        else {
            return;
        };
        let p = cell
            .private
            .get_or_insert_with(|| Box::new(PrivateName::field()));
        p.brand = brand;
        p.is_static = is_static;
        match how & 3 {
            0 => {
                p.kind = PrivateKind::Method;
                p.get = f;
            }
            1 => {
                p.kind = PrivateKind::Accessor;
                p.get = f;
            }
            _ => {
                p.kind = PrivateKind::Accessor;
                p.set = f;
            }
        }
    }

    /// A class constructor's internal slot `i`: its home object (0), its
    /// brand (1), then its instance fields' keys and initializers.
    fn class_slot(&self, class: Value, i: usize) -> Value {
        class
            .as_object()
            .and_then(|o| self.heap.objects.get(o))
            .and_then(|c| c.internal().get(i).copied())
            .unwrap_or(Value::UNDEFINED)
    }

    /// `AddClassField`: an instance field of `class`, its key and its
    /// initializer (undefined for none), kept for its constructions.
    #[inline(never)]
    pub(super) fn add_class_field(&mut self, class: Value, key: Value, init: Value) {
        if let Some(o) = class.as_object() {
            self.heap.with_internal_slots(o, |s| {
                if s.len() < 2 {
                    s.resize(2, Value::UNDEFINED);
                }
                s.extend([key, init]);
            });
        }
    }

    /// `GetClassField`: instance field `i`'s key (`which` 0) or
    /// initializer (1).
    #[inline(never)]
    pub(super) fn class_field(&self, class: Value, i: usize, which: usize) -> Value {
        self.class_slot(class, 2 + 2 * i + which.min(1))
    }

    /// `AddPrivateBrand` (PrivateMethodOrAccessorAdd, section 7.3.32, for
    /// all of a class's at once): `obj` given `class`'s brand.
    #[inline(never)]
    pub(super) fn add_private_brand(&mut self, obj: Value, class: Value) -> Res<()> {
        let brand = self.class_slot(class, 1);
        let (Some(o), Some((_, i))) = (obj.as_object(), brand.as_reference()) else {
            return Ok(());
        };
        if !self
            .heap
            .private_add(o, PropertyKey::Symbol(i), Value::TRUE)
        {
            let name = self.description_text(brand);
            return Err(self.throw_error(ErrorType::TypeError, messages::brand_twice(&name)));
        }
        Ok(())
    }

    /// `DefineFieldKey` (DefineField, section 7.3.33): a private field
    /// added, or a public one defined as CreateDataPropertyOrThrow does.
    #[inline(never)]
    pub(super) fn define_field_key(&mut self, obj: Value, key: Value, value: Value) -> Res<()> {
        let Some(o) = obj.as_object() else {
            return Ok(());
        };
        if let Some((k, _)) = self.private_of(key) {
            if !self.heap.private_add(o, k, value) {
                let name = self.description_text(key);
                return Err(self.throw_error(ErrorType::TypeError, messages::private_twice(&name)));
            }
            return Ok(());
        }
        let k = self.property_key(key)?;
        let desc = Descriptor {
            value: Some(value),
            writable: Some(true),
            enumerable: Some(true),
            configurable: Some(true),
            ..Descriptor::default()
        };
        if !self.heap.define_own_property(o, k, desc) {
            let text = self.key_display(k);
            let message = if self.heap.objects.get(o).is_some_and(|c| !c.is_extensible()) {
                messages::field_not_extensible(&text)
            } else {
                messages::cannot_redefine(&text)
            };
            return Err(self.throw_error(ErrorType::TypeError, message));
        }
        Ok(())
    }

    /// Whether `obj` reaches the private method or accessor `p`: the
    /// class itself for a static one, else an instance with its brand.
    fn has_brand(&self, obj: Value, p: &PrivateName) -> bool {
        if p.is_static {
            return obj == p.brand;
        }
        match (obj.as_object(), p.brand.as_reference()) {
            (Some(o), Some((_, b))) => self.heap.private_get(o, PropertyKey::Symbol(b)).is_some(),
            _ => false,
        }
    }

    /// The error of a private method or accessor reached through what
    /// has not its brand.
    fn receiver_error(&mut self, p: &PrivateName) -> Abrupt {
        let class = if p.is_static {
            self.function_name(p.brand).unwrap_or_default()
        } else {
            self.description_text(p.brand)
        };
        let class = if class.is_empty() {
            "anonymous".to_owned()
        } else {
            class
        };
        self.throw_error(
            ErrorType::TypeError,
            messages::private_receiver(&class, p.is_static),
        )
    }

    /// `GetPrivate` (PrivateGet, section 7.3.30): `obj.#x`.
    #[inline(never)]
    pub(super) fn get_private(&mut self, obj: Value, name: Value) -> Res<Value> {
        let Some((k, p)) = self.private_of(name) else {
            return Ok(Value::UNDEFINED);
        };
        match p.kind {
            PrivateKind::Field => match obj.as_object().and_then(|o| self.heap.private_get(o, k)) {
                Some(v) => Ok(v),
                None => {
                    let text = self.description_text(name);
                    Err(self.throw_error(ErrorType::TypeError, messages::private_read(&text)))
                }
            },
            _ if !self.has_brand(obj, &p) => Err(self.receiver_error(&p)),
            PrivateKind::Method => Ok(p.get),
            PrivateKind::Accessor => {
                if p.get.is_undefined() {
                    let text = self.description_text(name);
                    return Err(
                        self.throw_error(ErrorType::TypeError, messages::private_no_getter(&text))
                    );
                }
                self.call_value(p.get, obj, &[])
            }
        }
    }

    /// `SetPrivate` (PrivateSet, section 7.3.31): `obj.#x = value`.
    #[inline(never)]
    pub(super) fn set_private(&mut self, obj: Value, name: Value, value: Value) -> Res<()> {
        let Some((k, p)) = self.private_of(name) else {
            return Ok(());
        };
        match p.kind {
            PrivateKind::Field => match obj.as_object() {
                Some(o) if self.heap.private_get(o, k).is_some() => {
                    self.heap.private_set(o, k, value);
                    Ok(())
                }
                _ => {
                    let text = self.description_text(name);
                    Err(self.throw_error(ErrorType::TypeError, messages::private_write(&text)))
                }
            },
            _ if !self.has_brand(obj, &p) => Err(self.receiver_error(&p)),
            PrivateKind::Method => {
                let text = self.description_text(name);
                Err(self.throw_error(ErrorType::TypeError, messages::private_method_write(&text)))
            }
            PrivateKind::Accessor => {
                if p.set.is_undefined() {
                    let text = self.description_text(name);
                    return Err(
                        self.throw_error(ErrorType::TypeError, messages::private_no_setter(&text))
                    );
                }
                self.call_value(p.set, obj, &[value]).map(|_| ())
            }
        }
    }

    /// `HasPrivate`: `#x in obj` (section 13.10.1).
    #[inline(never)]
    pub(super) fn has_private(&mut self, obj: Value, name: Value) -> Res<Value> {
        let Some((k, p)) = self.private_of(name) else {
            return Ok(Value::FALSE);
        };
        let Some(o) = obj.as_object() else {
            let text = self.description_text(name);
            let what = self.display(obj);
            return Err(self.throw_error(ErrorType::TypeError, messages::private_in(&text, &what)));
        };
        let has = match p.kind {
            PrivateKind::Field => self.heap.private_get(o, k).is_some(),
            _ => self.has_brand(obj, &p),
        };
        Ok(Value::bool(has))
    }

    /// The class instructions that neither call nor jump, out of the
    /// interpreter's loop: in it they would cost the arithmetic its own
    /// registers (the micro-benches' numeric loop, 11% slower).
    #[inline(never)]
    pub(super) fn class_op(
        &mut self,
        op: Op,
        r: &mut Reader<'_>,
        base: u32,
        u: u32,
        fi: u32,
    ) -> Res<()> {
        match op {
            Op::CreateClass => {
                let (p, q, f, heritage) = (r.u(), r.u(), r.u(), r.u());
                let parent = self.reg(base, p);
                let env = self.top().context;
                let made = self.create_class(heritage, parent, u, f as u32, env);
                let (ctor, proto) = made?;
                self.set_reg(base, q, proto);
                self.vm.acc = ctor;
            }
            Op::SetHomeObject => {
                let h = r.u();
                let home = self.reg(base, h);
                let f = self.vm.acc;
                self.set_home_object(f, home);
            }
            Op::DefineMethod => {
                let (o, k, how) = (r.u(), r.u(), r.u());
                let (target, key, f) = (self.reg(base, o), self.reg(base, k), self.vm.acc);
                self.define_class_member(target, key, f, how)?;
            }
            Op::LdaHomeObject => {
                let f = self.top().closure;
                self.vm.acc = self.home_object(f);
            }
            Op::GetSuperBase => {
                let h = r.u();
                let home = self.reg(base, h);
                self.vm.acc = home
                    .as_object()
                    .map_or(Value::NULL, |o| self.heap.get_prototype_of(o));
            }
            Op::GetSuper => {
                let (h, t) = (r.u(), r.u());
                let (home, this, key) = (self.reg(base, h), self.reg(base, t), self.vm.acc);
                self.vm.acc = self.get_super(home, this, key)?;
            }
            Op::SetSuper => {
                let (h, t, k, strict) = (r.u(), r.u(), r.u(), r.u());
                let (home, this) = (self.reg(base, h), self.reg(base, t));
                let (key, v) = (self.reg(base, k), self.vm.acc);
                self.set_super(home, this, key, v, strict == 1)?;
            }
            Op::CheckSuperOnce => {
                if self.vm.acc != Value::UNINITIALIZED {
                    let e = self.throw_error(ErrorType::ReferenceError, messages::SUPER_TWICE);
                    return Err(e);
                }
            }
            Op::CreatePrivateName => {
                let k = r.u();
                let d = self.constant(u, fi, k)?;
                self.vm.acc = self.new_private_name(d)?;
            }
            Op::DefinePrivateMethod => {
                let (n, c, how) = (r.u(), r.u(), r.u());
                let (name, class, f) = (self.reg(base, n), self.reg(base, c), self.vm.acc);
                self.define_private_method(name, class, f, how);
            }
            Op::AddClassField => {
                let (c, k) = (r.u(), r.u());
                let (class, key, init) = (self.reg(base, c), self.reg(base, k), self.vm.acc);
                self.add_class_field(class, key, init);
            }
            Op::GetClassField => {
                let (c, i, which) = (r.u(), r.u(), r.u());
                let class = self.reg(base, c);
                self.vm.acc = self.class_field(class, i, which);
            }
            Op::AddPrivateBrand => {
                let (o, c) = (r.u(), r.u());
                let (obj, class) = (self.reg(base, o), self.reg(base, c));
                self.add_private_brand(obj, class)?;
            }
            Op::DefineFieldKey => {
                let (o, k) = (r.u(), r.u());
                let (obj, key, v) = (self.reg(base, o), self.reg(base, k), self.vm.acc);
                self.define_field_key(obj, key, v)?;
            }
            Op::GetPrivate => {
                let o = r.u();
                let (obj, name) = (self.reg(base, o), self.vm.acc);
                self.vm.acc = self.get_private(obj, name)?;
            }
            Op::SetPrivate => {
                let (o, n) = (r.u(), r.u());
                let (obj, name, v) = (self.reg(base, o), self.reg(base, n), self.vm.acc);
                self.set_private(obj, name, v)?;
            }
            Op::HasPrivate => {
                let n = r.u();
                let (name, obj) = (self.reg(base, n), self.vm.acc);
                self.vm.acc = self.has_private(obj, name)?;
            }
            Op::ThrowSuperDelete => {
                let e = self.throw_error(ErrorType::ReferenceError, messages::SUPER_DELETE);
                return Err(e);
            }
            Op::ReturnDerived => {
                let v = r.u();
                let (value, this) = (self.reg(base, v), self.vm.acc);
                self.vm.acc = self.derived_return(value, this)?;
            }
            _ => {}
        }
        Ok(())
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
            Got::Proxy(p) => self.proxy_get(p, key, receiver),
            Got::Element(o, i) => self.element(o, i),
        }
    }

    /// `base[key] = value` with `receiver` as the object set (and a
    /// setter's `this`); in strict code a refusal throws.
    fn set_value_with(
        &mut self,
        base: Value,
        key: PropertyKey,
        value: Value,
        receiver: Value,
        strict: bool,
    ) -> Res<()> {
        let Some(o) = base.as_object() else {
            return self.set_value(base, key, value, strict);
        };
        match self.heap.set(o, key, value, receiver) {
            SetOutcome::Done(true) => Ok(()),
            SetOutcome::Receiver(r) => {
                if self.set_on_receiver(r, key, value)? || !strict {
                    Ok(())
                } else {
                    let k = self.key_display(key);
                    Err(self.throw_error(ErrorType::TypeError, messages::proxy_set_false(&k)))
                }
            }
            SetOutcome::Typed(t) => self.typed_set(t, key, value),
            SetOutcome::Proxy(p) => {
                if self.proxy_set(p, key, value, receiver)? || !strict {
                    Ok(())
                } else {
                    let k = self.key_display(key);
                    Err(self.throw_error(ErrorType::TypeError, messages::proxy_set_false(&k)))
                }
            }
            // Sloppy code ignores a refusal.
            SetOutcome::Done(false) if !strict => Ok(()),
            SetOutcome::Setter(s) if s.is_undefined() && !strict => Ok(()),
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
