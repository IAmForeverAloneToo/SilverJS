//! Destructuring (ECMA-262 sections 8.6.2, 13.15.5 and 14.3.3): binding
//! and assignment patterns, bound from the accumulator. An array pattern
//! runs the iterator protocol and closes its iterator if it stops early
//! or anything in it throws; an object pattern reads its properties and
//! copies the rest.

use super::expr::static_key;
use super::{Gen, R, units, unsupported};
use crate::ast::{Expr, MemberProp, Pat, PatElem, PatId, PatProp, PropKey, Span};
use crate::compiler::Handler;
use crate::compiler::bytecode::Op;

/// How a pattern's names are bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Bind {
    /// A destructuring assignment: names assigned, members set.
    Assign,
    /// A `var` declaration's: names assigned (they exist already).
    Var,
    /// A `let`, `const`, parameter or catch binding: names initialized.
    Init,
}

/// A member target evaluated before the value it is given (section
/// 13.15.5.6: the reference first, then the value).
enum Target {
    /// A name or a nested pattern: nothing evaluated before.
    Later,
    Named {
        object: u32,
        name: i64,
    },
    Keyed {
        object: u32,
        key: u32,
    },
}

impl Gen<'_, '_> {
    /// Binds the accumulator to `pat`.
    pub(crate) fn bind_pattern(&mut self, pat: PatId, mode: Bind) -> R {
        let node = self.ast.pat(pat);
        // A name or member target leaves the source to nothing.
        if matches!(node.kind, Pat::Ident(_) | Pat::Expr(_)) {
            self.pattern_source = None;
        }
        match &node.kind {
            Pat::Ident(name) => self.store_name(&name.name, node.span, mode == Bind::Init),
            Pat::Expr(e) => self.store_to_expr(*e),
            Pat::Object { props, rest } => self.object_pattern(props, *rest, mode, node.span),
            Pat::Array { elems, rest } => self.array_pattern(elems, *rest, mode, node.span),
        }
    }

    /// Evaluates a member target's object and key, if `pat` is one.
    fn prepare_target(&mut self, pat: PatId) -> R<Target> {
        let Pat::Expr(e) = &self.ast.pat(pat).kind else {
            return Ok(Target::Later);
        };
        let e = self.unparen(*e);
        let node = self.ast.expr(e);
        let Expr::Member {
            object,
            prop,
            optional: false,
        } = &node.kind
        else {
            return Ok(Target::Later);
        };
        self.expr(*object)?;
        let o = self.alloc();
        self.reg_op(Op::Star, o);
        match prop {
            MemberProp::Ident(i) => {
                let name = self.name(&i.name);
                Ok(Target::Named { object: o, name })
            }
            MemberProp::Computed(k) => {
                self.expr(*k)?;
                let key = self.alloc();
                self.reg_op(Op::Star, key);
                Ok(Target::Keyed { object: o, key })
            }
            MemberProp::Private(i) => Err(unsupported("private names", i.span)),
        }
    }

    /// Gives the accumulator to a target prepared by `prepare_target`.
    fn store_target(&mut self, pat: PatId, target: Target, mode: Bind) -> R {
        let span = self.ast.pat(pat).span;
        match target {
            Target::Later => self.bind_pattern(pat, mode),
            Target::Named { object, name } => {
                let ic = self.ic();
                self.pos(span);
                self.emit(Op::SetNamed, &[i64::from(object), name, ic]);
                self.free(object);
                Ok(())
            }
            Target::Keyed { object, key } => {
                let ic = self.ic();
                self.pos(span);
                self.emit(Op::SetKeyed, &[i64::from(object), i64::from(key), ic]);
                self.free(key);
                self.free(object);
                Ok(())
            }
        }
    }

    /// The accumulator's default when undefined: `default` evaluated,
    /// named after the binding if it is an anonymous function.
    fn pattern_default(&mut self, elem: &PatElem) -> R {
        let Some(d) = elem.default else {
            return Ok(());
        };
        let skip = self.new_label();
        self.jump(Op::JumpIfNotUndefined, skip);
        let name = match &self.ast.pat(elem.target).kind {
            Pat::Ident(i) => Some(units(&i.name)),
            _ => None,
        };
        self.expr_named(d, name.as_deref())?;
        self.bind(skip);
        Ok(())
    }

    fn object_pattern(
        &mut self,
        props: &[PatProp],
        rest: Option<PatId>,
        mode: Bind,
        span: Span,
    ) -> R {
        // RequireObjectCoercible, its message naming the first property.
        let first = props.first().and_then(|p| static_key(&p.key));
        let n = self.string(first.as_deref().unwrap_or(&[]));
        let plain = self.pattern_position(span);
        self.emit(Op::RequireObjectCoercible, &[n, plain]);
        let v = self.alloc();
        self.reg_op(Op::Star, v);
        // The keys the rest leaves out.
        let keys = if rest.is_some() {
            self.op(Op::CreateArray);
            let k = self.alloc();
            self.reg_op(Op::Star, k);
            Some(k)
        } else {
            None
        };
        for p in props {
            match static_key(&p.key) {
                Some(key) => {
                    if let Some(k) = keys {
                        let c = self.string(&key);
                        self.emit(Op::LdaConst, &[c]);
                        self.reg_op(Op::ArrayPush, k);
                    }
                    let target = self.prepare_target(p.value.target)?;
                    let name = self.string(&key);
                    let ic = self.ic();
                    self.pos(p.span);
                    self.emit(Op::GetNamed, &[i64::from(v), name, ic]);
                    self.pattern_default(&p.value)?;
                    self.store_target(p.value.target, target, mode)?;
                }
                None => {
                    let PropKey::Computed(key) = p.key else {
                        return Err(unsupported("private names", p.span));
                    };
                    self.expr(key)?;
                    self.op(Op::ToPropertyKey);
                    let kr = self.alloc();
                    self.reg_op(Op::Star, kr);
                    if let Some(k) = keys {
                        self.reg_op(Op::ArrayPush, k);
                    }
                    let target = self.prepare_target(p.value.target)?;
                    // The key again, past the target's evaluation.
                    self.reg_op(Op::Ldar, kr);
                    let ic = self.ic();
                    self.pos(p.span);
                    self.emit(Op::GetKeyed, &[i64::from(v), ic]);
                    self.pattern_default(&p.value)?;
                    self.store_target(p.value.target, target, mode)?;
                    self.free(kr);
                }
            }
        }
        if let (Some(r), Some(k)) = (rest, keys) {
            let target = self.prepare_target(r)?;
            self.reg_op(Op::Ldar, v);
            self.reg_op(Op::CopyRest, k);
            self.store_target(r, target, mode)?;
            self.free(k);
        }
        self.free(v);
        Ok(())
    }

    fn array_pattern(
        &mut self,
        elems: &[Option<PatElem>],
        rest: Option<PatId>,
        mode: Bind,
        span: Span,
    ) -> R {
        let it = self.alloc();
        let next = self.alloc();
        let done = self.alloc();
        let plain = self.pattern_position(span);
        self.emit(Op::GetIterator, &[i64::from(it), i64::from(next), plain]);
        self.op(Op::LdaFalse);
        self.reg_op(Op::Star, done);
        let context = self.handler_context();
        let start = self.pc();
        for el in elems {
            match el {
                // An elision: a step, its value dropped.
                None => {
                    let after = self.new_label();
                    self.iterator_next(it, next, done, after);
                    self.bind(after);
                }
                Some(el) => {
                    let target = self.prepare_target(el.target)?;
                    let got = self.new_label();
                    // Undefined once the iterator is done.
                    self.op(Op::LdaUndefined);
                    self.iterator_next(it, next, done, got);
                    self.bind(got);
                    self.pattern_default(el)?;
                    self.store_target(el.target, target, mode)?;
                }
            }
        }
        if let Some(r) = rest {
            let target = self.prepare_target(r)?;
            self.op(Op::CreateArray);
            let a = self.alloc();
            self.reg_op(Op::Star, a);
            let (head, end) = (self.new_label(), self.new_label());
            self.bind(head);
            self.iterator_next(it, next, done, end);
            self.reg_op(Op::ArrayPush, a);
            self.jump(Op::JumpLoop, head);
            self.bind(end);
            self.reg_op(Op::Ldar, a);
            self.free(a);
            self.store_target(r, target, mode)?;
        }
        let end = self.pc();
        // Done with it before it was: closed.
        self.emit(Op::IteratorClose, &[i64::from(it), i64::from(done)]);
        let past = self.new_label();
        self.jump(Op::Jump, past);
        let target = self.pc();
        self.handler(Handler {
            start,
            end,
            target,
            context,
        });
        self.emit(Op::IteratorCloseQuiet, &[i64::from(it), i64::from(done)]);
        self.op(Op::Throw);
        self.bind(past);
        if let Some(c) = context {
            self.free(c);
        }
        self.free(done);
        self.free(next);
        self.free(it);
        Ok(())
    }

    /// The position a pattern's errors quote: its value's source if the
    /// pattern is bound from one (1 if not: the value itself is named).
    fn pattern_position(&mut self, span: Span) -> i64 {
        match self.pattern_source.take() {
            Some(source) => {
                self.pos(source);
                0
            }
            None => {
                self.pos(span);
                1
            }
        }
    }

    /// `IteratorNext` to `label` once the iterator is done.
    pub(crate) fn iterator_next(&mut self, it: u32, next: u32, done: u32, label: super::Label) {
        self.jump_with(
            Op::IteratorNext,
            &[i64::from(it), i64::from(next), i64::from(done)],
            label,
        );
    }
}
