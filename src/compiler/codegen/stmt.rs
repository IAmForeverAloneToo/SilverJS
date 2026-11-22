//! Statements.

use super::pattern::Bind;
use super::{BreakKind, Completion, Control, Gen, Label, R, units, unsupported};
use crate::ast::{
    Case, Catch, ExprId, ForHead, ForInit, Ident, Pat, Span, Stmt, StmtId, VarDecl, VarKind,
};
use crate::compiler::Handler;
use crate::compiler::bytecode::Op;
use crate::compiler::scope::ScopeKey;

/// A `finally` block's tokens: how it was entered.
const FALL_THROUGH: i64 = 0;
const THREW: i64 = 1;
const FIRST_COMPLETION: i64 = 2;

impl Gen<'_, '_> {
    pub(super) fn stmts(&mut self, stmts: &[StmtId]) -> R {
        for s in stmts {
            self.stmt(*s)?;
        }
        Ok(())
    }

    fn block(&mut self, key: ScopeKey, stmts: &[StmtId]) -> R {
        let save = self.enter_scope(key)?;
        self.stmts(stmts)?;
        self.leave_scope(save);
        Ok(())
    }

    pub(super) fn stmt(&mut self, id: StmtId) -> R {
        let node = self.ast.stmt(id);
        let span = node.span;
        self.check(span)?;
        if !matches!(node.kind, Stmt::Block(_) | Stmt::Empty | Stmt::Function(_)) {
            self.pos(span);
        }
        let labels = std::mem::take(&mut self.pending_labels);
        // A script's completion value: these statements' own is
        // undefined if their body gives none (UpdateEmpty(_, undefined)).
        if let Some(c) = self.completion
            && matches!(
                node.kind,
                Stmt::If { .. }
                    | Stmt::For { .. }
                    | Stmt::ForIn { .. }
                    | Stmt::ForOf { .. }
                    | Stmt::While { .. }
                    | Stmt::DoWhile { .. }
                    | Stmt::Switch { .. }
                    | Stmt::Try { .. }
                    | Stmt::With { .. }
            )
        {
            self.op(Op::LdaUndefined);
            self.reg_op(Op::Star, c);
        }
        match &node.kind {
            Stmt::Expr(e) => {
                self.expr(*e)?;
                if let Some(c) = self.completion {
                    self.reg_op(Op::Star, c);
                }
            }
            Stmt::Var(decl) => self.var_decl(decl, span)?,
            // Made at their scope's entry.
            Stmt::Function(_) | Stmt::Empty => {}
            Stmt::Class(_) => return Err(unsupported("classes", span)),
            Stmt::Block(stmts) => self.block(ScopeKey::Stmt(id), stmts)?,
            Stmt::Debugger => self.op(Op::Debugger),
            Stmt::If {
                test,
                consequent,
                alternate,
            } => self.if_stmt(*test, *consequent, *alternate)?,
            Stmt::For {
                init,
                test,
                update,
                body,
            } => self.for_stmt(id, init.as_ref(), *test, *update, *body, labels, span)?,
            Stmt::ForIn { left, right, body } => {
                self.for_in(id, left, *right, *body, labels, span)?
            }
            Stmt::ForOf { is_await: true, .. } => return Err(unsupported("async functions", span)),
            Stmt::ForOf {
                left, right, body, ..
            } => self.for_of(id, left, *right, *body, labels)?,
            Stmt::While { test, body } => self.while_stmt(*test, *body, labels)?,
            Stmt::DoWhile { body, test } => self.do_while(*body, *test, labels)?,
            Stmt::Return(e) => {
                match e {
                    Some(e) => self.expr(*e)?,
                    None => self.op(Op::LdaUndefined),
                }
                self.jump_out(Completion::Return);
            }
            Stmt::Break(label) => {
                let target = self.find_target(label.as_ref().map(|l| &*l.name), false);
                if let Some(i) = target {
                    self.jump_out(Completion::Break(i));
                }
            }
            Stmt::Continue(label) => {
                let target = self.find_target(label.as_ref().map(|l| &*l.name), true);
                if let Some(i) = target {
                    self.jump_out(Completion::Continue(i));
                }
            }
            Stmt::Throw(e) => {
                self.expr(*e)?;
                self.pos(span);
                self.op(Op::Throw);
            }
            Stmt::Try {
                block,
                handler,
                finalizer,
            } => self.try_stmt(id, block, handler.as_ref(), finalizer.as_deref())?,
            Stmt::Switch {
                discriminant,
                cases,
            } => self.switch(id, *discriminant, cases, labels)?,
            Stmt::Labeled { label, body } => self.labeled(label, *body, labels)?,
            Stmt::With { object, body } => self.with_stmt(id, *object, *body)?,
            Stmt::Import(_) | Stmt::Export(_) => return Err(unsupported("modules", span)),
        }
        Ok(())
    }

    fn if_stmt(&mut self, test: ExprId, consequent: StmtId, alternate: Option<StmtId>) -> R {
        let other = self.new_label();
        self.expr(test)?;
        self.jump(Op::JumpIfFalse, other);
        self.stmt(consequent)?;
        match alternate {
            Some(a) => {
                let end = self.new_label();
                self.jump(Op::Jump, end);
                self.bind(other);
                self.stmt(a)?;
                self.bind(end);
            }
            None => self.bind(other),
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn for_stmt(
        &mut self,
        id: StmtId,
        init: Option<&ForInit>,
        test: Option<ExprId>,
        update: Option<ExprId>,
        body: StmtId,
        labels: Vec<crate::ast::Name>,
        span: Span,
    ) -> R {
        let save = self.enter_scope(ScopeKey::Stmt(id))?;
        match init {
            Some(ForInit::Var(decl)) => self.var_decl(decl, span)?,
            Some(ForInit::Expr(e)) => self.expr(*e)?,
            None => {}
        }
        // A `let` captured by a closure is a new binding each
        // iteration: the environment is copied.
        let per_iteration = save.save.is_some();
        if per_iteration {
            self.op(Op::CopyContext);
        }
        let (head, end, cont) = (self.new_label(), self.new_label(), self.new_label());
        self.loop_control(labels, end, cont);
        self.bind(head);
        if let Some(t) = test {
            self.expr(t)?;
            self.jump(Op::JumpIfFalse, end);
        }
        self.stmt(body)?;
        self.bind(cont);
        if per_iteration {
            self.op(Op::CopyContext);
        }
        if let Some(u) = update {
            self.expr(u)?;
        }
        self.jump(Op::JumpLoop, head);
        self.control.pop();
        self.bind(end);
        self.leave_scope(save);
        Ok(())
    }

    fn while_stmt(&mut self, test: ExprId, body: StmtId, labels: Vec<crate::ast::Name>) -> R {
        let (head, end) = (self.new_label(), self.new_label());
        self.loop_control(labels, end, head);
        self.bind(head);
        self.expr(test)?;
        self.jump(Op::JumpIfFalse, end);
        self.stmt(body)?;
        self.jump(Op::JumpLoop, head);
        self.control.pop();
        self.bind(end);
        Ok(())
    }

    fn do_while(&mut self, body: StmtId, test: ExprId, labels: Vec<crate::ast::Name>) -> R {
        let (head, end, cont) = (self.new_label(), self.new_label(), self.new_label());
        self.loop_control(labels, end, cont);
        self.bind(head);
        self.stmt(body)?;
        self.bind(cont);
        self.expr(test)?;
        self.jump(Op::JumpIfFalse, end);
        self.jump(Op::JumpLoop, head);
        self.control.pop();
        self.bind(end);
        Ok(())
    }

    fn labeled(&mut self, label: &Ident, body: StmtId, labels: Vec<crate::ast::Name>) -> R {
        let mut labels = labels;
        labels.push(label.name.clone());
        let inner = self.ast.stmt(body);
        let is_loop = matches!(
            inner.kind,
            Stmt::For { .. }
                | Stmt::ForIn { .. }
                | Stmt::ForOf { .. }
                | Stmt::While { .. }
                | Stmt::DoWhile { .. }
                | Stmt::Labeled { .. }
        );
        if is_loop {
            self.pending_labels = labels;
            self.stmt(body)?;
        } else {
            let end = self.new_label();
            self.control.push(Control::Breakable {
                labels,
                kind: BreakKind::Block,
                break_to: end,
                continue_to: None,
                iterator: None,
            });
            self.stmt(body)?;
            self.control.pop();
            self.bind(end);
        }
        Ok(())
    }

    fn with_stmt(&mut self, id: StmtId, object: ExprId, body: StmtId) -> R {
        self.expr(object)?;
        let save = self.alloc();
        self.reg_op(Op::PushWithContext, save);
        self.control.push(Control::Context { save });
        let outer = self.scope;
        if let Some(s) = self.a.key(ScopeKey::Stmt(id)) {
            self.scope = s;
        }
        self.stmt(body)?;
        self.scope = outer;
        self.reg_op(Op::PopContext, save);
        self.control.pop();
        self.free(save);
        Ok(())
    }

    fn loop_control(&mut self, labels: Vec<crate::ast::Name>, end: Label, cont: Label) {
        self.control.push(Control::Breakable {
            labels,
            kind: BreakKind::Loop,
            break_to: end,
            continue_to: Some(cont),
            iterator: None,
        });
    }

    /// The control entry a `break` or `continue` goes to.
    fn find_target(&self, label: Option<&str>, is_continue: bool) -> Option<usize> {
        self.control.iter().rposition(|c| match c {
            Control::Breakable { labels, kind, .. } => match label {
                Some(l) => {
                    labels.iter().any(|x| &**x == l) && (!is_continue || *kind == BreakKind::Loop)
                }
                None if is_continue => *kind == BreakKind::Loop,
                None => matches!(kind, BreakKind::Loop | BreakKind::Switch),
            },
            _ => false,
        })
    }

    /// Jumps out to `target`, popping the environments it leaves and
    /// running the `finally` blocks it passes (a return's value in the
    /// accumulator).
    pub(super) fn jump_out(&mut self, target: Completion) {
        let goal = match target {
            Completion::Break(i) | Completion::Continue(i) => Some(i),
            Completion::Return => None,
        };
        let mut pop: Option<u32> = None;
        let mut level = self.control.len();
        while level > 0 {
            level -= 1;
            if goal == Some(level) {
                if let (
                    Some(Control::Breakable {
                        iterator: Some((it, done)),
                        ..
                    }),
                    Completion::Break(_),
                ) = (self.control.get(level), target)
                {
                    let (it, done) = (*it, *done);
                    self.emit(Op::IteratorClose, &[i64::from(it), i64::from(done)]);
                }
                if let Some(r) = pop {
                    self.reg_op(Op::PopContext, r);
                }
                let label = match (self.control.get(level), target) {
                    (
                        Some(Control::Breakable {
                            continue_to: Some(c),
                            ..
                        }),
                        Completion::Continue(_),
                    ) => Some(*c),
                    (Some(Control::Breakable { break_to, .. }), _) => Some(*break_to),
                    _ => None,
                };
                if let Some(l) = label {
                    self.jump(Op::Jump, l);
                }
                return;
            }
            match self.control.get_mut(level) {
                Some(Control::Context { save }) => pop = Some(*save),
                Some(Control::Finally {
                    token,
                    value,
                    entry,
                    completions,
                }) => {
                    let k = match completions.iter().position(|c| *c == target) {
                        Some(k) => k,
                        None => {
                            completions.push(target);
                            completions.len() - 1
                        }
                    };
                    let (token, value, entry) = (*token, *value, *entry);
                    if let Some(r) = pop {
                        // Not the accumulator: a return's value is in it.
                        self.reg_op(Op::PopContext, r);
                    }
                    if target == Completion::Return {
                        self.reg_op(Op::Star, value);
                    }
                    self.emit(Op::LdaSmi, &[FIRST_COMPLETION + k as i64]);
                    self.reg_op(Op::Star, token);
                    self.jump(Op::Jump, entry);
                    return;
                }
                Some(Control::Breakable {
                    iterator: Some((it, done)),
                    ..
                }) => {
                    let (it, done) = (*it, *done);
                    // A return's value stays in the accumulator.
                    self.emit(Op::IteratorClose, &[i64::from(it), i64::from(done)]);
                }
                Some(Control::Breakable { .. }) | None => {}
            }
        }
        // Past every entry: a return.
        self.op(Op::Return);
    }

    /// The register a handler restores the environment from: the one
    /// current now, if a block pushed one; `None` for the function's.
    pub(super) fn handler_context(&mut self) -> Option<u32> {
        if self
            .control
            .iter()
            .any(|c| matches!(c, Control::Context { .. }))
        {
            let r = self.alloc();
            self.reg_op(Op::StoreContext, r);
            Some(r)
        } else {
            None
        }
    }

    fn try_stmt(
        &mut self,
        id: StmtId,
        block: &[StmtId],
        handler: Option<&Catch>,
        finalizer: Option<&[StmtId]>,
    ) -> R {
        let Some(fin) = finalizer else {
            return self.try_catch(id, block, handler);
        };
        let token = self.alloc();
        let value = self.alloc();
        let entry = self.new_label();
        self.control.push(Control::Finally {
            token,
            value,
            entry,
            completions: Vec::new(),
        });
        let context = self.handler_context();
        let start = self.pc();
        self.try_catch(id, block, handler)?;
        let end = self.pc();
        self.emit(Op::LdaSmi, &[FALL_THROUGH]);
        self.reg_op(Op::Star, token);
        self.jump(Op::Jump, entry);
        let target = self.pc();
        self.handler(Handler {
            start,
            end,
            target,
            context,
        });
        self.reg_op(Op::Star, value);
        self.emit(Op::LdaSmi, &[THREW]);
        self.reg_op(Op::Star, token);
        self.bind(entry);
        let completions = match self.control.pop() {
            Some(Control::Finally { completions, .. }) => completions,
            _ => Vec::new(),
        };
        if let Some(r) = context {
            self.free(r);
        }
        self.block(ScopeKey::Finally(id), fin)?;
        // Then on as it was entered.
        for (k, c) in completions.iter().enumerate() {
            let next = self.new_label();
            self.emit(Op::LdaSmi, &[FIRST_COMPLETION + k as i64]);
            self.reg_op(Op::TestStrictEq, token);
            self.jump(Op::JumpIfFalse, next);
            if *c == Completion::Return {
                self.reg_op(Op::Ldar, value);
            }
            self.jump_out(*c);
            self.bind(next);
        }
        let done = self.new_label();
        self.emit(Op::LdaSmi, &[THREW]);
        self.reg_op(Op::TestStrictEq, token);
        self.jump(Op::JumpIfFalse, done);
        self.reg_op(Op::Ldar, value);
        self.op(Op::Throw);
        self.bind(done);
        self.free(value);
        self.free(token);
        Ok(())
    }

    fn try_catch(&mut self, id: StmtId, block: &[StmtId], handler: Option<&Catch>) -> R {
        let Some(catch) = handler else {
            return self.block(ScopeKey::TryBlock(id), block);
        };
        let context = self.handler_context();
        let start = self.pc();
        self.block(ScopeKey::TryBlock(id), block)?;
        let end = self.pc();
        let done = self.new_label();
        self.jump(Op::Jump, done);
        let target = self.pc();
        self.handler(Handler {
            start,
            end,
            target,
            context,
        });
        let exception = self.alloc();
        self.reg_op(Op::Star, exception);
        let save = self.enter_scope(ScopeKey::Catch(id))?;
        if let Some(p) = catch.param {
            self.reg_op(Op::Ldar, exception);
            self.bind_pattern(p, Bind::Init)?;
        }
        self.stmts(&catch.body)?;
        self.leave_scope(save);
        self.free(exception);
        self.bind(done);
        if let Some(r) = context {
            self.free(r);
        }
        Ok(())
    }

    fn switch(
        &mut self,
        id: StmtId,
        discriminant: crate::ast::ExprId,
        cases: &[Case],
        labels: Vec<crate::ast::Name>,
    ) -> R {
        self.expr(discriminant)?;
        let d = self.alloc();
        self.reg_op(Op::Star, d);
        let save = self.enter_scope(ScopeKey::Stmt(id))?;
        let end = self.new_label();
        let bodies: Vec<Label> = cases.iter().map(|_| self.new_label()).collect();
        let mut default = None;
        for (c, l) in cases.iter().zip(&bodies) {
            match c.test {
                Some(t) => {
                    self.expr(t)?;
                    self.reg_op(Op::TestStrictEq, d);
                    self.jump(Op::JumpIfTrue, *l);
                }
                None => default = Some(*l),
            }
        }
        self.jump(Op::Jump, default.unwrap_or(end));
        self.control.push(Control::Breakable {
            labels,
            kind: BreakKind::Switch,
            break_to: end,
            continue_to: None,
            iterator: None,
        });
        for (c, l) in cases.iter().zip(&bodies) {
            self.bind(*l);
            self.stmts(&c.body)?;
        }
        self.control.pop();
        self.bind(end);
        self.leave_scope(save);
        self.free(d);
        Ok(())
    }

    fn for_in(
        &mut self,
        id: StmtId,
        left: &ForHead,
        right: crate::ast::ExprId,
        body: StmtId,
        labels: Vec<crate::ast::Name>,
        span: Span,
    ) -> R {
        let lexical = matches!(left, ForHead::Var(d) if d.kind != VarKind::Var);
        // Annex B's `for (var x = 1 in o)`.
        if let ForHead::Var(decl) = left
            && decl.kind == VarKind::Var
            && decl.decls.first().is_some_and(|d| d.init.is_some())
        {
            self.var_decl(decl, span)?;
        }
        // The object is evaluated with the head's names in their TDZ.
        if lexical {
            let save = self.enter_scope(ScopeKey::Stmt(id))?;
            self.expr(right)?;
            self.leave_scope(save);
        } else {
            self.expr(right)?;
        }
        self.op(Op::ForInEnumerate);
        let it = self.alloc();
        self.reg_op(Op::Star, it);
        let (head, end, cont) = (self.new_label(), self.new_label(), self.new_label());
        self.loop_control(labels, end, cont);
        self.bind(head);
        self.reg_op(Op::ForInNext, it);
        self.jump(Op::JumpIfUndefined, end);
        // Each iteration a new environment for a `let` or `const` head.
        let save = if lexical {
            let v = self.alloc();
            self.reg_op(Op::Star, v);
            let save = self.enter_scope(ScopeKey::Stmt(id))?;
            self.reg_op(Op::Ldar, v);
            Some((v, save))
        } else {
            None
        };
        self.bind_head(left)?;
        self.stmt(body)?;
        if let Some((v, save)) = save {
            self.leave_scope(save);
            self.free(v);
        }
        self.bind(cont);
        self.jump(Op::JumpLoop, head);
        self.control.pop();
        self.bind(end);
        self.free(it);
        Ok(())
    }

    /// Stores the accumulator into a member expression (a for-in head).
    /// `for (left of right) body` (section 14.7.5): the iterator closed
    /// when the loop is left early, by a jump or a throw.
    fn for_of(
        &mut self,
        id: StmtId,
        left: &ForHead,
        right: crate::ast::ExprId,
        body: StmtId,
        labels: Vec<crate::ast::Name>,
    ) -> R {
        let lexical = matches!(left, ForHead::Var(d) if d.kind != VarKind::Var);
        // The iterable is evaluated with the head's names in their TDZ.
        if lexical {
            let save = self.enter_scope(ScopeKey::Stmt(id))?;
            self.expr(right)?;
            self.leave_scope(save);
        } else {
            self.expr(right)?;
        }
        let (it, next, done) = (self.alloc(), self.alloc(), self.alloc());
        self.pos(self.ast.expr(right).span);
        self.emit(Op::GetIterator, &[i64::from(it), i64::from(next), 0]);
        self.op(Op::LdaFalse);
        self.reg_op(Op::Star, done);
        let (head, end, cont) = (self.new_label(), self.new_label(), self.new_label());
        let context = self.handler_context();
        self.control.push(Control::Breakable {
            labels,
            kind: BreakKind::Loop,
            break_to: end,
            continue_to: Some(cont),
            iterator: Some((it, done)),
        });
        let start = self.pc();
        self.bind(head);
        self.iterator_next(it, next, done, end);
        // Each iteration a new environment for a `let` or `const` head.
        let save = if lexical {
            let v = self.alloc();
            self.reg_op(Op::Star, v);
            let save = self.enter_scope(ScopeKey::Stmt(id))?;
            self.reg_op(Op::Ldar, v);
            Some((v, save))
        } else {
            None
        };
        self.bind_head(left)?;
        self.stmt(body)?;
        if let Some((v, save)) = save {
            self.leave_scope(save);
            self.free(v);
        }
        self.bind(cont);
        self.jump(Op::JumpLoop, head);
        let stop = self.pc();
        self.control.pop();
        // A throw out of the loop (not out of `next`) closes it.
        let target = self.pc();
        self.handler(Handler {
            start,
            end: stop,
            target,
            context,
        });
        self.emit(Op::IteratorCloseQuiet, &[i64::from(it), i64::from(done)]);
        self.op(Op::Throw);
        self.bind(end);
        if let Some(c) = context {
            self.free(c);
        }
        self.free(done);
        self.free(next);
        self.free(it);
        Ok(())
    }

    /// Binds the accumulator to a `for-in` or `for-of` head.
    fn bind_head(&mut self, left: &ForHead) -> R {
        match left {
            ForHead::Var(decl) => {
                let Some(d) = decl.decls.first() else {
                    return Ok(());
                };
                let mode = if decl.kind == VarKind::Var {
                    Bind::Var
                } else {
                    Bind::Init
                };
                self.bind_pattern(d.target, mode)
            }
            ForHead::Pattern(p) => self.bind_pattern(*p, Bind::Assign),
        }
    }

    pub(super) fn store_to_expr(&mut self, e: crate::ast::ExprId) -> R {
        let node = self.ast.expr(e);
        // A name (`for (x in o)`), parenthesized or not.
        match &node.kind {
            crate::ast::Expr::Ident(name) => {
                return self.store_name(&name.name, node.span, false);
            }
            crate::ast::Expr::Paren(inner) => return self.store_to_expr(*inner),
            _ => {}
        }
        let crate::ast::Expr::Member {
            object,
            prop,
            optional: false,
        } = &node.kind
        else {
            self.pos(node.span);
            self.op(Op::ThrowInvalidLhs);
            return Ok(());
        };
        let v = self.alloc();
        self.reg_op(Op::Star, v);
        self.expr(*object)?;
        let o = self.alloc();
        self.reg_op(Op::Star, o);
        match prop {
            crate::ast::MemberProp::Ident(i) => {
                let n = self.name(&i.name);
                let ic = self.ic();
                self.reg_op(Op::Ldar, v);
                self.pos(node.span);
                self.emit(Op::SetNamed, &[i64::from(o), n, ic]);
            }
            crate::ast::MemberProp::Computed(k) => {
                self.expr(*k)?;
                let kr = self.alloc();
                self.reg_op(Op::Star, kr);
                let ic = self.ic();
                self.reg_op(Op::Ldar, v);
                self.pos(node.span);
                self.emit(Op::SetKeyed, &[i64::from(o), i64::from(kr), ic]);
                self.free(kr);
            }
            crate::ast::MemberProp::Private(i) => return Err(unsupported("private names", i.span)),
        }
        self.free(o);
        self.free(v);
        Ok(())
    }

    fn var_decl(&mut self, decl: &VarDecl, span: Span) -> R {
        if matches!(decl.kind, VarKind::Using | VarKind::AwaitUsing) {
            return Err(unsupported("using declarations", span));
        }
        let lexical = decl.kind != VarKind::Var;
        for d in &decl.decls {
            let node = self.ast.pat(d.target);
            let Pat::Ident(name) = &node.kind else {
                // A pattern, which always has an initializer.
                if let Some(init) = d.init {
                    self.expr(init)?;
                    self.pattern_source = Some(self.ast.expr(init).span);
                }
                let mode = if lexical { Bind::Init } else { Bind::Var };
                self.bind_pattern(d.target, mode)?;
                continue;
            };
            match d.init {
                Some(init) => {
                    let inferred = units(&name.name);
                    self.expr_named(init, Some(&inferred))?;
                }
                None if lexical => self.op(Op::LdaUndefined),
                None => continue,
            }
            self.store_name(&name.name, node.span, lexical)?;
        }
        Ok(())
    }
}
