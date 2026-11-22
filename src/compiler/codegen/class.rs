//! Classes (ECMA-262 section 15.7): ClassDefinitionEvaluation, and the
//! `super` forms: `super[key]` read, written and called as a method, and
//! `super(...)` in a derived class's constructor, which binds its `this`.
//! Fields, private names and static blocks are J3.3b's.

use super::expr::Args;
use super::expr::static_key;
use super::{Gen, R, units, unsupported};
use crate::ast::{ClassId, MemberKind, MemberProp, PropKey, Span, Spreadable};
use crate::compiler::FunctionKind;
use crate::compiler::bytecode::Op;
use crate::compiler::scope::{Place, ScopeKey};

impl Gen<'_, '_> {
    /// The class `id` into the accumulator; `inferred` names it if it has
    /// no name of its own (`var C = class {}`).
    pub(super) fn class(&mut self, id: ClassId, inferred: Option<&[u16]>) -> R {
        let class = self.ast.class(id);
        let span = class.span;
        let save = self.enter_scope(ScopeKey::Class(id))?;
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
        self.emit(
            Op::CreateClass,
            &[
                i64::from(parent),
                i64::from(proto),
                i64::from(index),
                heritage,
            ],
        );
        let f = self.alloc();
        self.reg_op(Op::Star, f);
        for m in &class.members {
            let how = match m.kind {
                MemberKind::Method => 0,
                MemberKind::Getter => 1,
                MemberKind::Setter => 2,
                MemberKind::Field | MemberKind::Accessor | MemberKind::StaticBlock => {
                    return Err(unsupported("class fields", m.span));
                }
            };
            let Some(value) = m.value else {
                continue;
            };
            let target = if m.is_static { f } else { proto };
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
        // The class's own name inside it, now initialized.
        if let Some(n) = &class.name {
            self.reg_op(Op::Ldar, f);
            self.store_name(&n.name, n.span, true)?;
        }
        self.reg_op(Op::Ldar, f);
        self.free(f);
        self.free(proto);
        self.free(parent);
        self.leave_scope(save);
        Ok(())
    }

    /// `super[key]` and `super.name` read: the key into the accumulator
    /// first, then the read with `this` as the receiver.
    pub(super) fn super_get(&mut self, prop: &MemberProp, span: Span) -> R {
        let (home, this) = self.super_refs(span)?;
        self.super_key(prop)?;
        self.pos(span);
        self.emit(Op::GetSuper, &[i64::from(home), i64::from(this)]);
        self.free(this);
        self.free(home);
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
        let (home, this) = self.super_refs(callee_span)?;
        self.super_key(prop)?;
        self.pos(callee_span);
        self.emit(Op::GetSuper, &[i64::from(home), i64::from(this)]);
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
        self.free(this);
        self.free(home);
        Ok(())
    }

    /// A `super` member as a reference, for a write: its home object,
    /// `this` and key in three registers, `this` read before the key is
    /// evaluated (section 13.3.7.1).
    pub(super) fn super_ref(&mut self, prop: &MemberProp, span: Span) -> R<[u32; 3]> {
        let (home, this) = self.super_refs(span)?;
        self.super_key(prop)?;
        let k = self.alloc();
        self.reg_op(Op::Star, k);
        Ok([home, this, k])
    }

    /// A `super_ref` reference's value into the accumulator.
    pub(super) fn super_ref_get(&mut self, r: [u32; 3], span: Span) {
        let [home, this, k] = r;
        self.reg_op(Op::Ldar, k);
        self.pos(span);
        self.emit(Op::GetSuper, &[i64::from(home), i64::from(this)]);
    }

    /// The accumulator into a `super_ref` reference; it stays there.
    pub(super) fn super_ref_set(&mut self, r: [u32; 3], span: Span) {
        let [home, this, k] = r;
        self.pos(span);
        self.emit(
            Op::SetSuper,
            &[i64::from(home), i64::from(this), i64::from(k)],
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
            MemberProp::Computed(e) => {
                self.expr(*e)?;
                self.op(Op::ToPropertyKey);
            }
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
