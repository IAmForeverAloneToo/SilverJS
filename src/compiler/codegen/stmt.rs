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
        match self.using_in(stmts) {
            Some(is_async) => self.disposing(is_async, |g| g.stmts(stmts))?,
            None => self.stmts(stmts)?,
        }
        self.leave_scope(save);
        Ok(())
    }

    /// Whether `stmts` declare with `using`: `Some(true)` if one is an
    /// `await using`.
    pub(super) fn using_in(&self, stmts: &[StmtId]) -> Option<bool> {
        let mut found = None;
        for s in stmts {
            if let Stmt::Var(d) = &self.ast.stmt(*s).kind {
                match d.kind {
                    VarKind::AwaitUsing => return Some(true),
                    VarKind::Using => found = Some(false),
                    _ => {}
                }
            }
        }
        found
    }

    /// `body` with a dispose capability, its `using` resources disposed
    /// however it is left (DisposeResources, section 7.5.x of the
    /// explicit resource management proposal): last added first, each
    /// one's throw kept (over an earlier one, in a SuppressedError), and
    /// with `await using` each async result awaited. Then on as it was
    /// left, or the kept throw thrown.
    pub(super) fn disposing(&mut self, is_async: bool, body: impl FnOnce(&mut Self) -> R) -> R {
        let cap = self.alloc();
        self.op(Op::DisposeNew);
        self.reg_op(Op::Star, cap);
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
        self.using_caps.push(cap);
        let r = body(self);
        self.using_caps.pop();
        r?;
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
        // A throw out of the body: the completion the disposal starts
        // from.
        let steps = self.new_label();
        self.emit(Op::LdaSmi, &[THREW]);
        self.reg_op(Op::TestStrictEq, token);
        self.jump(Op::JumpIfFalse, steps);
        self.reg_op(Op::Ldar, value);
        self.reg_op(Op::DisposeThrow, cap);
        self.bind(steps);
        // The steps, each one's throw kept.
        let (head, done) = (self.new_label(), self.new_label());
        let awaited = self.alloc();
        self.bind(head);
        let step_start = self.pc();
        self.jump_with(Op::DisposeStep, &[i64::from(cap), i64::from(awaited)], done);
        if is_async {
            self.jump(Op::JumpIfFalse, head);
            self.reg_op(Op::Ldar, awaited);
            self.await_acc(Span::new(0, 0));
        }
        let step_end = self.pc();
        self.jump(Op::Jump, head);
        let caught = self.pc();
        self.handler(Handler {
            start: step_start,
            end: step_end,
            target: caught,
            context,
        });
        self.reg_op(Op::DisposeThrow, cap);
        self.jump(Op::Jump, head);
        self.bind(done);
        self.reg_op(Op::DisposeEnd, cap);
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
        if let Some(r) = context {
            self.free(r);
        }
        self.free(awaited);
        self.free(value);
        self.free(token);
        self.free(cap);
        Ok(())
    }

    /// A `using` or `await using` declaration: each value added to the
    /// scope's resources, then bound.
    fn using_decl(&mut self, decl: &VarDecl, span: Span) -> R {
        let Some(cap) = self.using_caps.last().copied() else {
            return Err(unsupported("using declarations here", span));
        };
        let hint = i64::from(decl.kind == VarKind::AwaitUsing);
        for d in &decl.decls {
            let node = self.ast.pat(d.target);
            let Pat::Ident(name) = &node.kind else {
                return Err(unsupported("using patterns", node.span));
            };
            match d.init {
                Some(init) => {
                    let inferred = units(&name.name);
                    self.expr_named(init, Some(&inferred))?;
                }
                None => self.op(Op::LdaUndefined),
            }
            self.pos(node.span);
            self.emit(Op::DisposeAdd, &[i64::from(cap), hint]);
            self.store_name(&name.name, node.span, true)?;
        }
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
            // Made at their scope's entry; a sloppy block's, also a `var`
            // of its function, set where it stands (Annex B.3.2).
            Stmt::Function(f) if self.a.annex_b.contains(f) => self.annex_b_store(*f, span)?,
            Stmt::Function(_) | Stmt::Empty => {}
            Stmt::Class(c) => {
                self.class(*c, None)?;
                if let Some(n) = &self.ast.class(*c).name {
                    self.store_name(&n.name, n.span, true)?;
                }
            }
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
            Stmt::ForOf {
                left,
                right,
                body,
                is_await,
            } => self.for_of(id, left, *right, *body, labels, *is_await, span)?,
            Stmt::While { test, body } => self.while_stmt(*test, *body, labels)?,
            Stmt::DoWhile { body, test } => self.do_while(*body, *test, labels)?,
            Stmt::Return(e) => {
                match e {
                    Some(e) => {
                        self.expr(*e)?;
                        // An async generator's return awaits its value.
                        if self.is_async_generator() {
                            self.await_acc(span);
                        }
                    }
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
        if let Some(ForInit::Var(decl)) = init
            && matches!(decl.kind, VarKind::Using | VarKind::AwaitUsing)
        {
            let is_async = decl.kind == VarKind::AwaitUsing;
            let per_iteration = save.save.is_some();
            self.disposing(is_async, |g| {
                g.var_decl(decl, span)?;
                g.for_rest(test, update, body, labels, per_iteration)
            })?;
            self.leave_scope(save);
            return Ok(());
        }
        match init {
            Some(ForInit::Var(decl)) => self.var_decl(decl, span)?,
            Some(ForInit::Expr(e)) => self.expr(*e)?,
            None => {}
        }
        // A `let` captured by a closure is a new binding each
        // iteration: the environment is copied.
        self.for_rest(test, update, body, labels, save.save.is_some())?;
        self.leave_scope(save);
        Ok(())
    }

    /// A `for` statement's loop, after its head's declaration.
    fn for_rest(
        &mut self,
        test: Option<ExprId>,
        update: Option<ExprId>,
        body: StmtId,
        labels: Vec<crate::ast::Name>,
        per_iteration: bool,
    ) -> R {
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
                        iterator: Some((it, done, is_async)),
                        ..
                    }),
                    Completion::Break(_),
                ) = (self.control.get(level), target)
                {
                    let (it, done, is_async) = (*it, *done, *is_async);
                    self.close_iterator(it, done, is_async);
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
                    iterator: Some((it, done, is_async)),
                    ..
                }) => {
                    let (it, done, is_async) = (*it, *done, *is_async);
                    // A return's value stays in the accumulator.
                    self.close_iterator(it, done, is_async);
                }
                Some(Control::Destructuring { it, done }) => {
                    let (it, done) = (*it, *done);
                    // IteratorClose with the return: what `return()`
                    // throws, or a result that is no object, wins.
                    self.close_iterator(it, done, false);
                }
                Some(Control::Breakable { .. }) | None => {}
            }
        }
        // Past every entry: a return.
        if self.is_derived() {
            // Its value is checked after its `finally` blocks have run.
            let _ = self.derived_return();
        }
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
        // A finally block that completes normally leaves the script's
        // completion value as the try gave it (section 14.15.3).
        let saved = self.completion.map(|c| {
            let s = self.alloc();
            self.reg_op(Op::Ldar, c);
            self.reg_op(Op::Star, s);
            (c, s)
        });
        self.block(ScopeKey::Finally(id), fin)?;
        if let Some((c, s)) = saved {
            self.reg_op(Op::Ldar, s);
            self.reg_op(Op::Star, c);
            self.free(s);
        }
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
    /// Closes a loop's iterator, the accumulator kept: IteratorClose, or
    /// for `for await` AsyncIteratorClose (its `return()` awaited).
    pub(super) fn close_iterator(&mut self, it: u32, done: u32, is_async: bool) {
        if !is_async {
            self.emit(Op::IteratorClose, &[i64::from(it), i64::from(done)]);
            return;
        }
        let kept = self.alloc();
        self.reg_op(Op::Star, kept);
        let skip = self.new_label();
        self.jump_with(
            Op::IteratorReturnCall,
            &[i64::from(it), i64::from(done), 0],
            skip,
        );
        self.await_acc(Span::new(0, 0));
        self.op(Op::CheckIterResult);
        self.bind(skip);
        self.reg_op(Op::Ldar, kept);
        self.free(kept);
    }

    #[allow(clippy::too_many_arguments)]
    fn for_of(
        &mut self,
        id: StmtId,
        left: &ForHead,
        right: crate::ast::ExprId,
        body: StmtId,
        labels: Vec<crate::ast::Name>,
        is_await: bool,
        span: Span,
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
        // `for await`: the async iterator (a sync one wrapped).
        let how = if is_await { 3 } else { 0 };
        self.emit(Op::GetIterator, &[i64::from(it), i64::from(next), how]);
        self.op(Op::LdaFalse);
        self.reg_op(Op::Star, done);
        let (head, end, cont) = (self.new_label(), self.new_label(), self.new_label());
        let context = self.handler_context();
        self.control.push(Control::Breakable {
            labels,
            kind: BreakKind::Loop,
            break_to: end,
            continue_to: Some(cont),
            iterator: Some((it, done, is_await)),
        });
        let start = self.pc();
        self.bind(head);
        if is_await {
            // next() called (done meanwhile: a throw closes nothing),
            // its result awaited, then read. A frame waiting here stands
            // at the head's target, as Chrome 157 shows it.
            let target = match left {
                ForHead::Var(d) => d.decls.first().map(|d| d.target),
                ForHead::Pattern(p) => Some(*p),
            };
            let at = target.map_or(span, |t| self.ast.pat(t).span);
            self.op(Op::LdaTrue);
            self.reg_op(Op::Star, done);
            let ic = self.ic();
            self.pos(span);
            self.emit(
                Op::Call,
                &[i64::from(next), i64::from(it), i64::from(it), 0, ic],
            );
            self.await_acc(at);
            self.pos(span);
            self.jump_with(Op::IteratorResult, &[i64::from(done)], end);
        } else {
            self.iterator_next(it, next, done, end);
        }
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
        match left {
            ForHead::Var(d) if matches!(d.kind, VarKind::Using | VarKind::AwaitUsing) => {
                let is_async = d.kind == VarKind::AwaitUsing;
                // The value, kept while the capability is made.
                let kept = save.as_ref().map(|(v, _)| *v);
                self.disposing(is_async, |g| {
                    if let Some(v) = kept {
                        g.reg_op(Op::Ldar, v);
                    }
                    g.bind_head(left)?;
                    g.stmt(body)
                })?;
            }
            _ => {
                self.bind_head(left)?;
                self.stmt(body)?;
            }
        }
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
        if is_await {
            // AsyncIteratorClose with a throw: `return()` awaited, what it
            // throws or rejects with dropped, the throw going on.
            let e = self.alloc();
            self.reg_op(Op::Star, e);
            let after = self.new_label();
            let quiet_start = self.pc();
            self.jump_with(
                Op::IteratorReturnCall,
                &[i64::from(it), i64::from(done), 1],
                after,
            );
            self.await_acc(span);
            let quiet_end = self.pc();
            let quiet_target = self.pc();
            self.handler(Handler {
                start: quiet_start,
                end: quiet_end,
                target: quiet_target,
                context: None,
            });
            self.bind(after);
            self.reg_op(Op::Ldar, e);
            self.free(e);
        } else {
            self.emit(Op::IteratorCloseQuiet, &[i64::from(it), i64::from(done)]);
        }
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
                if let (Some(cap), VarKind::Using | VarKind::AwaitUsing) =
                    (self.using_caps.last().copied(), decl.kind)
                {
                    let hint = i64::from(decl.kind == VarKind::AwaitUsing);
                    self.emit(Op::DisposeAdd, &[i64::from(cap), hint]);
                }
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
            crate::ast::MemberProp::Private(i) => {
                self.load_name(&format!("#{}", i.name), i.span, false)?;
                let kr = self.alloc();
                self.reg_op(Op::Star, kr);
                self.reg_op(Op::Ldar, v);
                self.pos(node.span);
                self.emit(Op::SetPrivate, &[i64::from(o), i64::from(kr)]);
                self.free(kr);
            }
        }
        self.free(o);
        self.free(v);
        Ok(())
    }

    fn var_decl(&mut self, decl: &VarDecl, span: Span) -> R {
        if matches!(decl.kind, VarKind::Using | VarKind::AwaitUsing) {
            return self.using_decl(decl, span);
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
