//! Classes (ECMA-262 section 15.7): ClassDefinitionEvaluation, and the
//! `super` forms: `super[key]` read, written and called as a method, and
//! `super(...)` in a derived class's constructor, which binds its `this`.
//! Fields with their initializers, private names (`#x`: a private name
//! made per evaluation, its methods and accessors kept on it), static
//! fields and blocks.

use super::expr::Args;
use super::expr::static_key;
use super::{Gen, R, units, unsupported};
use crate::ast::{ClassId, FuncId, MemberKind, MemberProp, PropKey, Span, Spreadable};
use crate::compiler::FunctionKind;
use crate::compiler::bytecode::Op;
use crate::compiler::scope::{Place, ScopeKey, ScopeKind};

/// A static field (its key's register) or static block, run once its
/// class is made.
struct Static {
    key: Option<u32>,
    value: Option<FuncId>,
    name: Option<Box<[u16]>>,
}

impl Gen<'_, '_> {
    /// The class `id` into the accumulator; `inferred` names it if it has
    /// no name of its own (`var C = class {}`).
    pub(super) fn class(&mut self, id: ClassId, inferred: Option<&[u16]>) -> R {
        let class = self.ast.class(id);
        let span = class.span;
        let save = self.enter_scope(ScopeKey::Class(id))?;
        // Its private names, new for each evaluation, made first.
        let mut declared: Vec<&str> = Vec::new();
        for m in &class.members {
            if let PropKey::Private(p) = &m.key
                && !declared.contains(&&*p.name)
            {
                declared.push(&p.name);
                let name = format!("#{}", p.name);
                let c = self.string(&units(&name));
                self.emit(Op::CreatePrivateName, &[c]);
                self.store_name(&name, p.span, true)?;
            }
        }
        // Its heritage, with the class's own name in its TDZ.
        let parent = self.alloc();
        let heritage = match class.extends {
            Some(e) => {
                self.expr(e)?;
                self.pos(self.ast.expr(e).span);
                1
            }
            None => {
                self.op(Op::LdaUndefined);
                0
            }
        };
        self.reg_op(Op::Star, parent);
        let at = class.extends.map_or(span, |e| self.ast.expr(e).span);
        let Some(ctor) = class.constructor else {
            return Err(unsupported("classes", span));
        };
        let index = self.a.func_index.get(&ctor).copied().unwrap_or(0);
        let name = match (&class.name, inferred) {
            (Some(n), _) => units(&n.name),
            (None, Some(n)) => n.into(),
            (None, None) => Box::default(),
        };
        let kind = if heritage == 1 {
            FunctionKind::DerivedConstructor
        } else {
            FunctionKind::ClassConstructor
        };
        self.name_function(index, name, kind);
        let proto = self.alloc();
        self.pos(at);
        let branded = i64::from(self.has_instance_private_methods(id)) * 2;
        self.emit(
            Op::CreateClass,
            &[
                i64::from(parent),
                i64::from(proto),
                i64::from(index),
                heritage | branded,
            ],
        );
        let f = self.alloc();
        self.reg_op(Op::Star, f);
        // Static fields and blocks run once the class is made, in order:
        // a static field's key is evaluated where it stands.
        let mut statics: Vec<Static> = Vec::new();
        for m in &class.members {
            let how = match m.kind {
                MemberKind::Method => 0,
                MemberKind::Getter => 1,
                MemberKind::Setter => 2,
                MemberKind::StaticBlock => {
                    statics.push(Static {
                        key: None,
                        value: m.value,
                        name: None,
                    });
                    continue;
                }
                MemberKind::Field => {
                    let name = self.field_key(&m.key)?;
                    if m.is_static {
                        let k = self.alloc();
                        self.reg_op(Op::Star, k);
                        statics.push(Static {
                            key: Some(k),
                            value: m.value,
                            name,
                        });
                        continue;
                    }
                    let k = self.alloc();
                    self.reg_op(Op::Star, k);
                    match m.value {
                        Some(init) => {
                            self.initializer(init, false, name)?;
                            self.reg_op(Op::SetHomeObject, proto);
                        }
                        None => self.op(Op::LdaUndefined),
                    }
                    self.emit(Op::AddClassField, &[i64::from(f), i64::from(k)]);
                    self.free(k);
                    continue;
                }
                MemberKind::Accessor => return Err(unsupported("auto-accessors", m.span)),
            };
            let Some(value) = m.value else {
                continue;
            };
            let target = if m.is_static { f } else { proto };
            if let PropKey::Private(p) = &m.key {
                // A private method or accessor: kept on its name.
                let pname = format!("#{}", p.name);
                let mut name = match how {
                    1 => units("get ").into_vec(),
                    2 => units("set ").into_vec(),
                    _ => Vec::new(),
                };
                name.extend_from_slice(&units(&pname));
                let n = self.alloc();
                self.load_name(&pname, p.span, false)?;
                self.reg_op(Op::Star, n);
                self.closure(value, Some(&name))?;
                self.reg_op(Op::SetHomeObject, target);
                let flags = how | (i64::from(m.is_static) * 4);
                self.emit(
                    Op::DefinePrivateMethod,
                    &[i64::from(n), i64::from(f), flags],
                );
                self.free(n);
                continue;
            }
            let k = self.alloc();
            match static_key(&m.key) {
                Some(key) => {
                    let c = self.string(&key);
                    self.emit(Op::LdaConst, &[c]);
                    self.reg_op(Op::Star, k);
                    let mut name = match how {
                        1 => units("get ").into_vec(),
                        2 => units("set ").into_vec(),
                        _ => Vec::new(),
                    };
                    name.extend_from_slice(&key);
                    self.closure(value, Some(&name))?;
                }
                None => {
                    let PropKey::Computed(e) = m.key else {
                        return Err(unsupported("private names", m.span));
                    };
                    self.expr(e)?;
                    self.op(Op::ToPropertyKey);
                    self.reg_op(Op::Star, k);
                    self.closure(value, None)?;
                    self.emit(Op::SetFunctionName, &[i64::from(k), how]);
                }
            }
            self.reg_op(Op::SetHomeObject, target);
            self.pos(m.span);
            self.emit(Op::DefineMethod, &[i64::from(target), i64::from(k), how]);
            self.free(k);
        }
        // The class's own name inside it, now initialized: its static
        // elements see it.
        if let Some(n) = &class.name {
            self.reg_op(Op::Ldar, f);
            self.store_name(&n.name, n.span, true)?;
        }
        for Static { key, value, name } in &statics {
            match value {
                Some(init) => {
                    self.initializer(*init, true, name.clone())?;
                    self.reg_op(Op::SetHomeObject, f);
                    let c = self.alloc();
                    self.reg_op(Op::Star, c);
                    let ic = self.ic();
                    self.pos_at(span, span.start);
                    self.emit(Op::Call, &[i64::from(c), i64::from(f), i64::from(c), 0, ic]);
                    self.free(c);
                }
                None => self.op(Op::LdaUndefined),
            }
            if let Some(k) = key {
                self.pos(span);
                self.emit(Op::DefineFieldKey, &[i64::from(f), i64::from(*k)]);
            }
        }
        for Static { key, .. } in statics.iter().rev() {
            if let Some(k) = key {
                self.free(*k);
            }
        }
        self.reg_op(Op::Ldar, f);
        self.free(f);
        self.free(proto);
        self.free(parent);
        self.leave_scope(save);
        Ok(())
    }

    /// A field's key into the accumulator: a name, a computed key, or a
    /// private name. Gives back the name an anonymous function it holds
    /// takes (none for a computed key).
    fn field_key(&mut self, key: &PropKey) -> R<Option<Box<[u16]>>> {
        if let Some(k) = static_key(key) {
            let c = self.string(&k);
            self.emit(Op::LdaConst, &[c]);
            return Ok(Some(k));
        }
        match key {
            PropKey::Computed(e) => {
                self.expr(*e)?;
                self.op(Op::ToPropertyKey);
                Ok(None)
            }
            PropKey::Private(p) => {
                let name = format!("#{}", p.name);
                self.load_name(&name, p.span, false)?;
                Ok(Some(units(&name)))
            }
            _ => Ok(None),
        }
    }

    /// A field's initializer or a static block, made into the
    /// accumulator: its frame is the class's `<instance_members_initializer>`
    /// or `<static_initializer>`; `name` names an anonymous function its
    /// value is.
    fn initializer(&mut self, fid: FuncId, is_static: bool, name: Option<Box<[u16]>>) -> R {
        let index = self.a.func_index.get(&fid).copied().unwrap_or(0);
        let frame = if is_static {
            "<static_initializer>"
        } else {
            "<instance_members_initializer>"
        };
        self.name_function(index, units(frame), FunctionKind::Initializer);
        if let Some(n) = name {
            self.sh.field_names.insert(index, n);
        }
        self.emit(Op::CreateClosure, &[i64::from(index)]);
        Ok(())
    }

    /// Whether class `id` has private methods or accessors that are not
    /// static: its instances carry its brand.
    fn has_instance_private_methods(&self, id: ClassId) -> bool {
        self.ast.class(id).members.iter().any(|m| {
            !m.is_static
                && matches!(
                    m.kind,
                    MemberKind::Method | MemberKind::Getter | MemberKind::Setter
                )
                && matches!(m.key, PropKey::Private(_))
        })
    }

    /// The class whose constructor the code being compiled is in (an
    /// arrow's, through its enclosing functions), if it is one's.
    fn enclosing_class(&self) -> Option<ClassId> {
        let mut at = Some(self.scope);
        while let Some(s) = at {
            let scope = self.a.scope(s);
            if scope.kind == (ScopeKind::Function { arrow: false }) {
                let fid = self.a.function(scope.func).func?;
                return self.a.ctor_class.get(&fid).copied();
            }
            at = scope.parent;
        }
        None
    }

    /// InitializeInstanceElements (section 7.3.34) of class `id`, its
    /// constructor in register `f`, on the object in register `this`:
    /// the brand of its private methods, then its fields in order, each
    /// initializer called as a method of the object. The calls stand at
    /// `at` (a base constructor's start, a `super(...)`).
    pub(super) fn init_instance(
        &mut self,
        id: ClassId,
        f: u32,
        this: u32,
        span: Span,
        at: u32,
    ) -> R {
        let class = self.ast.class(id);
        if self.has_instance_private_methods(id) {
            self.pos_at(span, at);
            self.emit(Op::AddPrivateBrand, &[i64::from(this), i64::from(f)]);
        }
        let fields = class
            .members
            .iter()
            .filter(|m| !m.is_static && m.kind == MemberKind::Field);
        for (i, m) in fields.enumerate() {
            let i = i64::try_from(i).unwrap_or(0);
            if m.value.is_some() {
                self.emit(Op::GetClassField, &[i64::from(f), i, 1]);
                let c = self.alloc();
                self.reg_op(Op::Star, c);
                let ic = self.ic();
                self.pos_at(span, at);
                self.emit(
                    Op::Call,
                    &[i64::from(c), i64::from(this), i64::from(c), 0, ic],
                );
                self.free(c);
            } else {
                self.op(Op::LdaUndefined);
            }
            let v = self.alloc();
            self.reg_op(Op::Star, v);
            self.emit(Op::GetClassField, &[i64::from(f), i, 0]);
            let k = self.alloc();
            self.reg_op(Op::Star, k);
            self.reg_op(Op::Ldar, v);
            self.pos_at(span, at);
            self.emit(Op::DefineFieldKey, &[i64::from(this), i64::from(k)]);
            self.free(k);
            self.free(v);
        }
        Ok(())
    }

    /// Whether class `id` has instance elements to initialize.
    pub(super) fn has_instance_elements(&self, id: ClassId) -> bool {
        self.has_instance_private_methods(id)
            || self
                .ast
                .class(id)
                .members
                .iter()
                .any(|m| !m.is_static && m.kind == MemberKind::Field)
    }

    /// `super[key]` and `super.name` read: the key into the accumulator
    /// first, then the read with `this` as the receiver.
    pub(super) fn super_get(&mut self, prop: &MemberProp, span: Span) -> R {
        let r = self.super_ref(prop, span, false)?;
        self.super_ref_get(r, span);
        self.super_ref_free(r);
        Ok(())
    }

    /// `super.m(args)`: the method read from the home object's prototype,
    /// called with `this`.
    pub(super) fn super_method_call(
        &mut self,
        prop: &MemberProp,
        args: &[Spreadable],
        optional: bool,
        callee_span: Span,
    ) -> R {
        let r = self.super_ref(prop, callee_span, false)?;
        self.super_ref_get(r, callee_span);
        let this = r[1];
        let f = self.alloc();
        self.reg_op(Op::Star, f);
        let skip = optional.then(|| {
            let l = self.new_label();
            self.jump(Op::JumpIfNullish, l);
            l
        });
        let list = self.args(args)?;
        let ic = self.ic();
        self.pos_at(callee_span, self.super_prop_at(prop, callee_span));
        match list {
            Args::Regs(first, n) => self.emit(
                Op::Call,
                &[
                    i64::from(f),
                    i64::from(this),
                    i64::from(first),
                    i64::from(n),
                    ic,
                ],
            ),
            Args::Array(a) => self.emit(
                Op::CallSpread,
                &[i64::from(f), i64::from(this), i64::from(a), 0],
            ),
        }
        self.free_args(list);
        if let Some(l) = skip {
            let done = self.new_label();
            self.jump(Op::Jump, done);
            self.bind(l);
            self.op(Op::LdaUndefined);
            self.bind(done);
        }
        self.free(f);
        self.super_ref_free(r);
        Ok(())
    }

    /// A `super` member as a reference (section 13.3.7): its base (the
    /// home object's prototype), `this` and key in three registers.
    /// `this` is read first, then the key evaluated, then the base read
    /// (MakeSuperPropertyReference); the key is made a property key when
    /// the reference is read or written, or now if `convert` (a compound
    /// form's, read and written with one conversion).
    pub(super) fn super_ref(
        &mut self,
        prop: &MemberProp,
        span: Span,
        convert: bool,
    ) -> R<[u32; 3]> {
        let (home, this) = self.super_refs(span)?;
        self.super_key(prop)?;
        let k = self.alloc();
        self.reg_op(Op::Star, k);
        self.reg_op(Op::GetSuperBase, home);
        self.reg_op(Op::Star, home);
        if convert {
            self.reg_op(Op::Ldar, k);
            self.pos(span);
            self.op(Op::ToPropertyKey);
            self.reg_op(Op::Star, k);
        }
        Ok([home, this, k])
    }

    /// A `super_ref` reference's value into the accumulator.
    pub(super) fn super_ref_get(&mut self, r: [u32; 3], span: Span) {
        let [base, this, k] = r;
        self.reg_op(Op::Ldar, k);
        self.pos(span);
        self.emit(Op::GetSuper, &[i64::from(base), i64::from(this)]);
    }

    /// The accumulator into a `super_ref` reference; it stays there.
    pub(super) fn super_ref_set(&mut self, r: [u32; 3], span: Span) {
        let [base, this, k] = r;
        self.pos(span);
        self.emit(
            Op::SetSuper,
            &[
                i64::from(base),
                i64::from(this),
                i64::from(k),
                i64::from(self.strict),
            ],
        );
    }

    /// Gives back a `super_ref` reference's registers.
    pub(super) fn super_ref_free(&mut self, r: [u32; 3]) {
        let [home, this, k] = r;
        self.free(k);
        self.free(this);
        self.free(home);
    }

    /// The home object and `this` into two registers.
    fn super_refs(&mut self, span: Span) -> R<(u32, u32)> {
        let home = self.alloc();
        self.load_name("%home", span, false)?;
        self.reg_op(Op::Star, home);
        let this = self.alloc();
        self.load_name("this", span, false)?;
        self.reg_op(Op::Star, this);
        Ok((home, this))
    }

    /// A `super` member's key into the accumulator.
    fn super_key(&mut self, prop: &MemberProp) -> R {
        match prop {
            MemberProp::Ident(i) => {
                let c = self.string(&units(&i.name));
                self.emit(Op::LdaConst, &[c]);
            }
            MemberProp::Computed(e) => self.expr(*e)?,
            MemberProp::Private(i) => return Err(unsupported("private names", i.span)),
        }
        Ok(())
    }

    fn super_prop_at(&self, prop: &MemberProp, span: Span) -> u32 {
        match prop {
            MemberProp::Ident(i) => i.span.start,
            MemberProp::Computed(e) => self.ast.expr(*e).span.start,
            MemberProp::Private(i) => i.span.start,
        }
        .max(span.start)
    }

    /// `super(...)`: the parent constructed with the class's `new.target`,
    /// then `this` bound to what it made, once (section 13.3.7.1). `args`
    /// `None` passes the frame's own arguments on (a default
    /// constructor's).
    pub(super) fn super_call(&mut self, args: Option<&[Spreadable]>, span: Span) -> R {
        let f = self.alloc();
        self.load_name("%fn", span, false)?;
        self.reg_op(Op::Star, f);
        let nt = self.alloc();
        self.load_name("new.target", span, false)?;
        self.reg_op(Op::Star, nt);
        match args {
            Some(args) => {
                let list = self.args(args)?;
                self.pos_at(span, span.start);
                match list {
                    Args::Regs(first, n) => self.emit(
                        Op::SuperCall,
                        &[i64::from(f), i64::from(nt), i64::from(first), i64::from(n)],
                    ),
                    Args::Array(a) => self.emit(
                        Op::SuperCallSpread,
                        &[i64::from(f), i64::from(nt), i64::from(a)],
                    ),
                }
                self.free_args(list);
            }
            None => {
                self.pos_at(span, span.start);
                self.emit(Op::SuperCallForward, &[i64::from(f), i64::from(nt)]);
            }
        }
        // Bound once: the binding must still be uninitialized.
        let v = self.alloc();
        self.reg_op(Op::Star, v);
        self.load_raw("this")?;
        self.pos(span);
        self.op(Op::CheckSuperOnce);
        self.reg_op(Op::Ldar, v);
        self.store_raw("this")?;
        // Then its class's instance elements, on what super() made.
        if let Some(id) = self.enclosing_class()
            && self.has_instance_elements(id)
        {
            self.init_instance(id, f, v, span, span.start)?;
        }
        self.reg_op(Op::Ldar, v);
        self.free(v);
        self.free(nt);
        self.free(f);
        Ok(())
    }

    /// A derived constructor's return of the accumulator: an object as
    /// it is, else its `this`, checked (section 10.2.2).
    pub(super) fn derived_return(&mut self) -> R {
        let v = self.alloc();
        self.reg_op(Op::Star, v);
        let inner = self.scope;
        self.scope = self.a.function(self.index).scope;
        let r = self.load_raw("this");
        self.scope = inner;
        r?;
        self.reg_op(Op::ReturnDerived, v);
        self.free(v);
        Ok(())
    }

    /// Whether the function being compiled is a derived constructor.
    pub(super) fn is_derived(&self) -> bool {
        self.a.function(self.index).derived
    }

    /// A binding read as it is, uninitialized or not.
    fn load_raw(&mut self, name: &str) -> R {
        match self.resolve(name).place {
            Place::Register(r) => self.reg_op(Op::Ldar, r),
            Place::Slot { depth, slot } => {
                self.emit(Op::LdaSlot, &[i64::from(depth), i64::from(slot)]);
            }
            _ => self.op(Op::LdaThis),
        }
        Ok(())
    }

    /// A binding written as it is, its TDZ and constness aside.
    fn store_raw(&mut self, name: &str) -> R {
        match self.resolve(name).place {
            Place::Register(r) => self.reg_op(Op::Star, r),
            Place::Slot { depth, slot } => {
                self.emit(Op::StaSlot, &[i64::from(depth), i64::from(slot)]);
            }
            _ => {}
        }
        Ok(())
    }
}
