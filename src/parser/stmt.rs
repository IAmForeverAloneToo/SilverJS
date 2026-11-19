//! Statements and declarations (section 14), the directive prologue
//! (section 11.2.1), labels, and the early errors of each.

use super::scope::ScopeKind;
use super::{Label, Parser};
use crate::SyntaxError;
use crate::ast::{
    Case, Catch, Declarator, Expr, ForHead, ForInit, Ident, Span, Stmt, StmtId, VarDecl, VarKind,
};
use crate::lexer::{Keyword, Punct, TokenKind};

/// Where a statement stands, which decides the declarations it may be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Place {
    /// In a statement list: any declaration.
    List,
    /// The body of `if` (sloppy code may put a plain function there,
    /// Annex B.3.3).
    IfBody,
    /// The body of a loop or `with`, or a labelled statement inside one.
    Body,
    /// A labelled statement's body in a statement list (sloppy code may
    /// label a plain function, Annex B.3.1).
    Labelled,
}

impl Parser<'_> {
    /// A directive prologue and the statements after it, up to where
    /// `end` says: the statements, whether the code is strict after the
    /// prologue, and whether the prologue has "use strict" (section
    /// 11.2.1's ContainsUseStrict, whatever the code around was).
    pub(super) fn directives_and_body(
        &mut self,
        end: impl Fn(&Self) -> bool,
        top: bool,
    ) -> Result<(Vec<StmtId>, bool, bool), SyntaxError> {
        let mut body = Vec::new();
        let mut strict = self.cx.strict;
        let mut use_strict = false;
        // The prologue's strings that strict code refuses, raised if a
        // later directive makes the code strict.
        let mut pending: Option<SyntaxError> = None;
        while self.tok.kind == TokenKind::String && !end(self) {
            let tok = self.tok.clone();
            let stmt = self.statement_list_item(top)?;
            body.push(stmt);
            let is_directive = match &self.ast.stmt(stmt).kind {
                Stmt::Expr(e) => {
                    let e = self.ast.expr(*e);
                    matches!(e.kind, Expr::String(_)) && e.span == Span::new(tok.start, tok.end)
                }
                _ => false,
            };
            if !is_directive {
                break;
            }
            let raw = self
                .source
                .get(tok.start as usize + 1..(tok.end as usize).saturating_sub(1))
                .unwrap_or("");
            if raw == "use strict" {
                strict = true;
                use_strict = true;
                self.cx.strict = true;
                if let Some(error) = pending.take() {
                    return Err(error);
                }
            }
            if pending.is_none() {
                pending = tok.strict_error.clone();
            }
        }
        while !end(self) {
            body.push(self.statement_list_item(top)?);
        }
        Ok((body, strict, use_strict))
    }

    /// A StatementListItem (section 14): a statement or a declaration.
    pub(super) fn statement_list_item(&mut self, top: bool) -> Result<StmtId, SyntaxError> {
        let _ = top;
        match self.tok.kind {
            TokenKind::Keyword(Keyword::Function) => self.function_declaration(false, false),
            TokenKind::Keyword(Keyword::Class) => self.class_declaration(false),
            TokenKind::Keyword(Keyword::Const) => self.lexical_declaration(VarKind::Const),
            TokenKind::Keyword(Keyword::Import)
                if self.module && self.import_is_declaration()? =>
            {
                Err(self.error(
                    "Cannot use import statement outside the top level of a module",
                    self.tok_span(),
                ))
            }
            TokenKind::Keyword(Keyword::Export) => {
                Err(self.error("Unexpected token 'export'", self.tok_span()))
            }
            TokenKind::Identifier if self.at_contextual("let") && self.let_declares()? => {
                self.lexical_declaration(VarKind::Let)
            }
            TokenKind::Identifier
                if self.at_contextual("async") && self.async_function_next()? =>
            {
                self.function_declaration(true, false)
            }
            TokenKind::Identifier if self.at_contextual("using") && self.using_declares()? => {
                self.lexical_declaration(VarKind::Using)
            }
            TokenKind::Identifier
                if self.at_contextual("await") && self.await_using_declares()? =>
            {
                self.lexical_declaration(VarKind::AwaitUsing)
            }
            _ => self.statement(Place::List),
        }
    }

    /// Whether `let` here begins a declaration (section 14.3.1): it is
    /// followed by a binding identifier, `[` or `{`. Elsewhere it is an
    /// identifier (sloppy code only).
    pub(super) fn let_declares(&self) -> Result<bool, SyntaxError> {
        let next = self.peek()?;
        Ok(match next.kind {
            TokenKind::Punct(Punct::LBracket | Punct::LBrace) => true,
            TokenKind::Identifier => true,
            // `let yield`, `let await`, `let let`: a declaration, which
            // the binding's own checks refuse where they must.
            TokenKind::Keyword(_) => false,
            _ => false,
        })
    }

    /// `async function` with no line terminator between.
    pub(super) fn async_function_next(&self) -> Result<bool, SyntaxError> {
        let next = self.peek()?;
        Ok(next.kind == TokenKind::Keyword(Keyword::Function) && !next.newline_before)
    }

    /// `using x` (Explicit Resource Management): a declaration when an
    /// identifier follows on the same line.
    fn using_declares(&self) -> Result<bool, SyntaxError> {
        let next = self.peek()?;
        Ok(next.kind == TokenKind::Identifier && !next.newline_before)
    }

    /// `await using x`, in an async function or a module.
    fn await_using_declares(&self) -> Result<bool, SyntaxError> {
        if !self.cx.await_ {
            return Ok(false);
        }
        let (a, b) = self.peek2()?;
        let using = a.kind == TokenKind::Identifier
            && !a.escaped
            && !a.newline_before
            && a.name(self.source) == "using";
        Ok(using && b.kind == TokenKind::Identifier && !b.newline_before)
    }

    /// Whether `import` begins a declaration rather than `import(...)` or
    /// `import.meta`.
    pub(super) fn import_is_declaration(&self) -> Result<bool, SyntaxError> {
        let next = self.peek()?;
        Ok(!matches!(
            next.kind,
            TokenKind::Punct(Punct::LParen | Punct::Dot)
        ))
    }

    /// A Statement (section 14), where `place` says what declarations it
    /// may be.
    pub(super) fn statement(&mut self, place: Place) -> Result<StmtId, SyntaxError> {
        self.enter()?;
        let result = self.statement_inner(place);
        self.leave();
        result
    }

    fn statement_inner(&mut self, place: Place) -> Result<StmtId, SyntaxError> {
        let start = self.tok.start;
        match self.tok.kind {
            TokenKind::Punct(Punct::LBrace) => self.block_statement(),
            TokenKind::Punct(Punct::Semicolon) => {
                self.advance()?;
                Ok(self
                    .ast
                    .add_stmt(Stmt::Empty, Span::new(start, self.prev_end)))
            }
            TokenKind::Keyword(Keyword::Var) => {
                self.advance()?;
                let decl = self.var_declarations(VarKind::Var, false)?;
                self.semicolon()?;
                Ok(self
                    .ast
                    .add_stmt(Stmt::Var(decl), Span::new(start, self.prev_end)))
            }
            TokenKind::Keyword(Keyword::If) => self.if_statement(),
            TokenKind::Keyword(Keyword::For) => self.for_statement(),
            TokenKind::Keyword(Keyword::While) => {
                self.advance()?;
                let test = self.paren_expression()?;
                let body = self.loop_body()?;
                Ok(self
                    .ast
                    .add_stmt(Stmt::While { test, body }, Span::new(start, self.prev_end)))
            }
            TokenKind::Keyword(Keyword::Do) => {
                self.advance()?;
                let body = self.loop_body()?;
                self.expect_keyword(Keyword::While)?;
                let test = self.paren_expression()?;
                // A `;` after do-while is inserted even on the same line
                // (section 12.10.1, rule 1's addition).
                self.eat(Punct::Semicolon)?;
                Ok(self.ast.add_stmt(
                    Stmt::DoWhile { body, test },
                    Span::new(start, self.prev_end),
                ))
            }
            TokenKind::Keyword(Keyword::Continue) => self.continue_statement(),
            TokenKind::Keyword(Keyword::Break) => self.break_statement(),
            TokenKind::Keyword(Keyword::Return) => {
                if !self.cx.return_ {
                    return Err(self.error("Illegal return statement", self.tok_span()));
                }
                self.advance()?;
                let arg = if self.at(Punct::Semicolon)
                    || self.at(Punct::RBrace)
                    || self.at_eof()
                    || self.tok.newline_before
                {
                    None
                } else {
                    Some(self.expression()?)
                };
                self.semicolon()?;
                Ok(self
                    .ast
                    .add_stmt(Stmt::Return(arg), Span::new(start, self.prev_end)))
            }
            TokenKind::Keyword(Keyword::With) => {
                if self.cx.strict {
                    return Err(self.error(
                        "Strict mode code may not include a with statement",
                        self.tok_span(),
                    ));
                }
                self.advance()?;
                let object = self.paren_expression()?;
                let body = self.statement(Place::Body)?;
                Ok(self
                    .ast
                    .add_stmt(Stmt::With { object, body }, Span::new(start, self.prev_end)))
            }
            TokenKind::Keyword(Keyword::Switch) => self.switch_statement(),
            TokenKind::Keyword(Keyword::Throw) => {
                self.advance()?;
                if self.tok.newline_before {
                    return Err(self.error("Illegal newline after throw", self.tok_span()));
                }
                let arg = self.expression()?;
                self.semicolon()?;
                Ok(self
                    .ast
                    .add_stmt(Stmt::Throw(arg), Span::new(start, self.prev_end)))
            }
            TokenKind::Keyword(Keyword::Try) => self.try_statement(),
            TokenKind::Keyword(Keyword::Debugger) => {
                self.advance()?;
                self.semicolon()?;
                Ok(self
                    .ast
                    .add_stmt(Stmt::Debugger, Span::new(start, self.prev_end)))
            }
            TokenKind::Keyword(Keyword::Function) => self.function_in_statement(place),
            TokenKind::Keyword(Keyword::Class | Keyword::Const) => Err(self.unexpected()),
            TokenKind::Keyword(Keyword::Import) if self.import_is_declaration()? => Err(self
                .error(
                    "Cannot use import statement outside a module",
                    self.tok_span(),
                )),
            TokenKind::Identifier => self.identifier_statement(place),
            _ => self.expression_statement(),
        }
    }

    /// A function declaration where only a statement may be: refused but
    /// for Annex B's sloppy `if (x) function f() {}` and labelled
    /// function.
    fn function_in_statement(&mut self, place: Place) -> Result<StmtId, SyntaxError> {
        let sloppy = !self.cx.strict;
        match place {
            Place::IfBody if sloppy => {
                // As if in a block of its own (Annex B.3.3).
                let start = self.tok.start;
                self.scopes.push(ScopeKind::Block);
                let result = self.function_declaration(false, true);
                self.scopes.pop();
                let f = result?;
                Ok(self
                    .ast
                    .add_stmt(Stmt::Block(vec![f]), Span::new(start, self.prev_end)))
            }
            Place::Labelled if sloppy => self.function_declaration(false, true),
            Place::List => self.function_declaration(false, false),
            _ if self.cx.strict => Err(self.error(
                "In strict mode code, functions can only be declared at top level or inside a block.",
                self.tok_span(),
            )),
            _ => Err(self.error(
                "In non-strict mode code, functions can only be declared at top level, inside a block, or as the body of an if statement.",
                self.tok_span(),
            )),
        }
    }

    /// A statement that begins with an identifier: a label, `let`
    /// misplaced, `async function`, or an expression.
    fn identifier_statement(&mut self, place: Place) -> Result<StmtId, SyntaxError> {
        let next = self.peek()?;
        if next.is(Punct::Colon) {
            return self.labelled_statement(place);
        }
        if self.at_contextual("let") && next.is(Punct::LBracket) {
            // `let [` begins no expression statement (section 14.5).
            return Err(self.error(
                "Lexical declaration cannot appear in a single-statement context",
                self.tok_span(),
            ));
        }
        if self.at_contextual("let")
            && place != Place::List
            && matches!(
                next.kind,
                TokenKind::Identifier | TokenKind::Punct(Punct::LBrace)
            )
            && !next.newline_before
        {
            return Err(self.error(
                "Lexical declaration cannot appear in a single-statement context",
                self.tok_span(),
            ));
        }
        if self.at_contextual("async")
            && next.kind == TokenKind::Keyword(Keyword::Function)
            && !next.newline_before
        {
            return Err(self.error(
                "Async functions can only be declared at the top level or inside a block.",
                self.tok_span(),
            ));
        }
        self.expression_statement()
    }

    fn expression_statement(&mut self) -> Result<StmtId, SyntaxError> {
        let start = self.tok.start;
        let expr = self.expression()?;
        self.semicolon()?;
        Ok(self
            .ast
            .add_stmt(Stmt::Expr(expr), Span::new(start, self.prev_end)))
    }

    /// `( Expression )`.
    pub(super) fn paren_expression(&mut self) -> Result<crate::ast::ExprId, SyntaxError> {
        self.expect(Punct::LParen)?;
        let e = self.with_context(|cx| cx.in_ = true, |p| p.expression())?;
        self.expect(Punct::RParen)?;
        Ok(e)
    }

    /// `{ StatementList }` in a scope of its own.
    pub(super) fn block_statement(&mut self) -> Result<StmtId, SyntaxError> {
        let start = self.tok.start;
        let body = self.block(ScopeKind::Block)?;
        Ok(self
            .ast
            .add_stmt(Stmt::Block(body), Span::new(start, self.prev_end)))
    }

    /// `{ StatementList }` in a new scope of `kind`.
    pub(super) fn block(&mut self, kind: ScopeKind) -> Result<Vec<StmtId>, SyntaxError> {
        self.expect(Punct::LBrace)?;
        self.scopes.push(kind);
        let result = self.block_rest();
        self.scopes.pop();
        result
    }

    /// The statements of a block whose `{` and scope are done, and its `}`.
    pub(super) fn block_rest(&mut self) -> Result<Vec<StmtId>, SyntaxError> {
        let mut body = Vec::new();
        while !self.at(Punct::RBrace) {
            if self.at_eof() {
                return Err(self.unexpected());
            }
            body.push(self.statement_list_item(false)?);
        }
        self.advance()?;
        Ok(body)
    }

    fn if_statement(&mut self) -> Result<StmtId, SyntaxError> {
        let start = self.tok.start;
        self.advance()?;
        let test = self.paren_expression()?;
        let consequent = self.statement(Place::IfBody)?;
        let alternate = if self.eat_keyword(Keyword::Else)? {
            Some(self.statement(Place::IfBody)?)
        } else {
            None
        };
        Ok(self.ast.add_stmt(
            Stmt::If {
                test,
                consequent,
                alternate,
            },
            Span::new(start, self.prev_end),
        ))
    }

    /// A loop's body, where `break` and `continue` have a target.
    fn loop_body(&mut self) -> Result<StmtId, SyntaxError> {
        let (b, l) = (self.in_breakable, self.in_loop);
        self.in_breakable = true;
        self.in_loop = true;
        let body = self.statement(Place::Body);
        self.in_breakable = b;
        self.in_loop = l;
        body
    }

    fn continue_statement(&mut self) -> Result<StmtId, SyntaxError> {
        let start = self.tok.start;
        let keyword = self.tok_span();
        self.advance()?;
        let label = self.jump_label()?;
        match &label {
            Some(label) => match self.labels.iter().rev().find(|l| l.name == label.name) {
                None => {
                    return Err(self.error(format!("Undefined label '{}'", label.name), label.span));
                }
                Some(l) if !l.is_loop => {
                    return Err(self.error(
                        format!(
                            "Illegal continue statement: '{}' does not denote an iteration statement",
                            label.name
                        ),
                        label.span,
                    ));
                }
                Some(_) => {}
            },
            None if !self.in_loop => {
                return Err(self.error(
                    "Illegal continue statement: no surrounding iteration statement",
                    keyword,
                ));
            }
            None => {}
        }
        self.semicolon()?;
        Ok(self
            .ast
            .add_stmt(Stmt::Continue(label), Span::new(start, self.prev_end)))
    }

    fn break_statement(&mut self) -> Result<StmtId, SyntaxError> {
        let start = self.tok.start;
        let keyword = self.tok_span();
        self.advance()?;
        let label = self.jump_label()?;
        match &label {
            Some(label) => {
                if !self.labels.iter().any(|l| l.name == label.name) {
                    return Err(self.error(format!("Undefined label '{}'", label.name), label.span));
                }
            }
            None if !self.in_breakable => {
                return Err(self.error("Illegal break statement", keyword));
            }
            None => {}
        }
        self.semicolon()?;
        Ok(self
            .ast
            .add_stmt(Stmt::Break(label), Span::new(start, self.prev_end)))
    }

    /// The label after `break` or `continue`, on the same line.
    fn jump_label(&mut self) -> Result<Option<Ident>, SyntaxError> {
        if self.tok.kind == TokenKind::Identifier && !self.tok.newline_before {
            Ok(Some(self.label_identifier()?))
        } else {
            Ok(None)
        }
    }

    /// A LabelIdentifier: checked as a reference, but `arguments` is no
    /// reference to the arguments object, so a class field may use it.
    fn label_identifier(&mut self) -> Result<Ident, SyntaxError> {
        self.with_context(|cx| cx.class_field = false, |p| p.identifier(false))
    }

    fn labelled_statement(&mut self, place: Place) -> Result<StmtId, SyntaxError> {
        let start = self.tok.start;
        let label = self.label_identifier()?;
        self.expect(Punct::Colon)?;
        if self.labels.iter().any(|l| l.name == label.name) {
            return Err(self.error(
                format!("Label '{}' has already been declared", label.name),
                label.span,
            ));
        }
        let is_loop = self.labels_a_loop()?;
        self.labels.push(Label {
            name: label.name.clone(),
            is_loop,
        });
        let inner = match place {
            Place::List | Place::Labelled => Place::Labelled,
            Place::IfBody | Place::Body => Place::Body,
        };
        let body = self.statement(inner);
        self.labels.pop();
        let body = body?;
        Ok(self.ast.add_stmt(
            Stmt::Labeled { label, body },
            Span::new(start, self.prev_end),
        ))
    }

    /// Whether the statement after a label's `:` (past any more labels)
    /// is a loop.
    fn labels_a_loop(&self) -> Result<bool, SyntaxError> {
        let mut lexer = self.lexer.clone();
        let mut tok = self.tok.clone();
        loop {
            match tok.kind {
                TokenKind::Keyword(Keyword::For | Keyword::While | Keyword::Do) => return Ok(true),
                TokenKind::Identifier => {
                    let next = lexer.next_token()?;
                    if !next.is(Punct::Colon) {
                        return Ok(false);
                    }
                    tok = lexer.next_token()?;
                }
                _ => return Ok(false),
            }
        }
    }

    fn switch_statement(&mut self) -> Result<StmtId, SyntaxError> {
        let start = self.tok.start;
        self.advance()?;
        let discriminant = self.paren_expression()?;
        self.expect(Punct::LBrace)?;
        self.scopes.push(ScopeKind::Block);
        let saved = self.in_breakable;
        self.in_breakable = true;
        let cases = self.switch_cases();
        self.in_breakable = saved;
        self.scopes.pop();
        let cases = cases?;
        Ok(self.ast.add_stmt(
            Stmt::Switch {
                discriminant,
                cases,
            },
            Span::new(start, self.prev_end),
        ))
    }

    fn switch_cases(&mut self) -> Result<Vec<Case>, SyntaxError> {
        let mut cases = Vec::new();
        let mut has_default = false;
        while !self.eat(Punct::RBrace)? {
            let start = self.tok.start;
            let test = if self.eat_keyword(Keyword::Case)? {
                Some(self.with_context(|cx| cx.in_ = true, |p| p.expression())?)
            } else if self.at_keyword(Keyword::Default) {
                if has_default {
                    return Err(self.error(
                        "More than one default clause in switch statement",
                        self.tok_span(),
                    ));
                }
                has_default = true;
                self.advance()?;
                None
            } else {
                return Err(self.unexpected());
            };
            self.expect(Punct::Colon)?;
            let mut body = Vec::new();
            while !(self.at(Punct::RBrace)
                || self.at_keyword(Keyword::Case)
                || self.at_keyword(Keyword::Default))
            {
                if self.at_eof() {
                    return Err(self.unexpected());
                }
                if (self.at_contextual("using") && self.using_declares()?)
                    || (self.at_contextual("await") && self.await_using_declares()?)
                {
                    return Err(self.error(
                        "Using declarations are not allowed directly in a case clause",
                        self.tok_span(),
                    ));
                }
                body.push(self.statement_list_item(false)?);
            }
            cases.push(Case {
                test,
                body,
                span: Span::new(start, self.prev_end),
            });
        }
        Ok(cases)
    }

    fn try_statement(&mut self) -> Result<StmtId, SyntaxError> {
        let start = self.tok.start;
        self.advance()?;
        let block = self.block(ScopeKind::Block)?;
        let handler = if self.at_keyword(Keyword::Catch) {
            let catch_start = self.tok.start;
            self.advance()?;
            Some(self.catch_clause(catch_start)?)
        } else {
            None
        };
        let finalizer = if self.eat_keyword(Keyword::Finally)? {
            Some(self.block(ScopeKind::Block)?)
        } else {
            None
        };
        if handler.is_none() && finalizer.is_none() {
            return Err(self.error("Missing catch or finally after try", self.tok_span()));
        }
        Ok(self.ast.add_stmt(
            Stmt::Try {
                block,
                handler,
                finalizer,
            },
            Span::new(start, self.prev_end),
        ))
    }

    fn catch_clause(&mut self, start: u32) -> Result<Catch, SyntaxError> {
        if !self.eat(Punct::LParen)? {
            let body = self.block(ScopeKind::Block)?;
            return Ok(Catch {
                param: None,
                body,
                span: Span::new(start, self.prev_end),
            });
        }
        let simple = self.tok.kind == TokenKind::Identifier;
        self.scopes.push(ScopeKind::Catch { simple });
        let result = self.catch_rest(start);
        self.scopes.pop();
        result
    }

    fn catch_rest(&mut self, start: u32) -> Result<Catch, SyntaxError> {
        let param = self.binding_target()?;
        for name in self.bound_names(param) {
            if !self.scopes.declare_param(&name.name) {
                return Err(self.error(
                    format!("Identifier '{}' has already been declared", name.name),
                    name.span,
                ));
            }
        }
        self.expect(Punct::RParen)?;
        // The block shares the parameter's scope: its lexical names may
        // not repeat the parameter's (section 14.15.1).
        self.expect(Punct::LBrace)?;
        let body = self.block_rest()?;
        Ok(Catch {
            param: Some(param),
            body,
            span: Span::new(start, self.prev_end),
        })
    }

    /// `let`, `const`, `using` or `await using` declarations, and the
    /// `;` after them.
    fn lexical_declaration(&mut self, kind: VarKind) -> Result<StmtId, SyntaxError> {
        let start = self.tok.start;
        if kind == VarKind::AwaitUsing {
            self.advance()?;
        }
        if matches!(kind, VarKind::Using | VarKind::AwaitUsing) && self.scopes_at_script_top() {
            return Err(self.error(
                "Using declarations are not allowed at the top level of a script",
                self.tok_span(),
            ));
        }
        self.advance()?;
        let decl = self.var_declarations(kind, false)?;
        self.semicolon()?;
        Ok(self
            .ast
            .add_stmt(Stmt::Var(decl), Span::new(start, self.prev_end)))
    }

    fn scopes_at_script_top(&self) -> bool {
        !self.module && self.scopes_depth() == 1
    }

    /// The declarators after `var`, `let`, `const` or `using`, declared in
    /// the current scopes. In a `for` head (`in_for`), a declarator may
    /// lack the initializer its kind wants, which the caller checks once
    /// it knows whether `in` or `of` follows.
    pub(super) fn var_declarations(
        &mut self,
        kind: VarKind,
        in_for: bool,
    ) -> Result<VarDecl, SyntaxError> {
        let mut decls = Vec::new();
        loop {
            let target_start = self.tok_span();
            let target = if matches!(kind, VarKind::Using | VarKind::AwaitUsing) {
                let ident = self.identifier(true)?;
                let span = ident.span;
                self.ast.add_pat(crate::ast::Pat::Ident(ident), span)
            } else {
                self.binding_target()?
            };
            for name in self.bound_names(target) {
                self.declare_binding(kind, &name)?;
            }
            let init = if self.eat(Punct::Eq)? {
                Some(self.assignment()?)
            } else {
                None
            };
            if init.is_none() && !in_for {
                let is_pattern = !matches!(self.ast.pat(target).kind, crate::ast::Pat::Ident(_));
                if is_pattern {
                    return Err(self.error(
                        "Missing initializer in destructuring declaration",
                        target_start,
                    ));
                }
                if kind != VarKind::Var && kind != VarKind::Let {
                    let what = if kind == VarKind::Const {
                        "const"
                    } else {
                        "using"
                    };
                    return Err(self.error(
                        format!("Missing initializer in {what} declaration"),
                        self.tok_span(),
                    ));
                }
            }
            decls.push(Declarator { target, init });
            if !self.eat(Punct::Comma)? {
                break;
            }
        }
        Ok(VarDecl { kind, decls })
    }

    /// Declares one bound name of a `var` or lexical declaration.
    pub(super) fn declare_binding(
        &mut self,
        kind: VarKind,
        name: &Ident,
    ) -> Result<(), SyntaxError> {
        if kind == VarKind::Var {
            self.scopes.declare_var(&name.name, name.span)
        } else {
            if &*name.name == "let" {
                return Err(self.error("let is disallowed as a lexically bound name", name.span));
            }
            self.scopes.declare_lexical(&name.name, name.span)
        }
    }

    fn for_statement(&mut self) -> Result<StmtId, SyntaxError> {
        let start = self.tok.start;
        self.advance()?;
        let is_await = if self.at_contextual("await") && self.cx.await_ {
            self.advance()?;
            true
        } else {
            false
        };
        self.expect(Punct::LParen)?;
        // A scope for the head's lexical declarations, around the body.
        self.scopes.push(ScopeKind::Block);
        let result = self.for_rest(start, is_await);
        self.scopes.pop();
        result
    }

    fn for_rest(&mut self, start: u32, is_await: bool) -> Result<StmtId, SyntaxError> {
        let head_start = self.tok_span();
        // A declaration head.
        let kind = match self.tok.kind {
            TokenKind::Keyword(Keyword::Var) => Some(VarKind::Var),
            TokenKind::Keyword(Keyword::Const) => Some(VarKind::Const),
            TokenKind::Identifier if self.at_contextual("let") && self.let_declares()? => {
                Some(VarKind::Let)
            }
            TokenKind::Identifier
                if self.at_contextual("using") && self.for_using_declares()? =>
            {
                Some(VarKind::Using)
            }
            TokenKind::Identifier
                if self.at_contextual("await") && self.await_using_declares()? =>
            {
                Some(VarKind::AwaitUsing)
            }
            _ => None,
        };
        if let Some(kind) = kind {
            if kind == VarKind::AwaitUsing {
                self.advance()?;
            }
            self.advance()?;
            let decl =
                self.with_context(|cx| cx.in_ = false, |p| p.var_declarations(kind, true))?;
            let is_of = self.at_contextual("of");
            if self.at_keyword(Keyword::In) || is_of {
                return self.for_in_of_declaration(start, decl, is_of, is_await, head_start);
            }
            if is_await {
                return Err(self.unexpected());
            }
            // An ordinary `for`: every declarator must have what its kind
            // wants.
            for d in &decl.decls {
                if d.init.is_none() {
                    let is_pattern =
                        !matches!(self.ast.pat(d.target).kind, crate::ast::Pat::Ident(_));
                    if is_pattern {
                        return Err(self.error(
                            "Missing initializer in destructuring declaration",
                            head_start,
                        ));
                    }
                    if !matches!(kind, VarKind::Var | VarKind::Let) {
                        return Err(
                            self.error("Missing initializer in const declaration", head_start)
                        );
                    }
                }
            }
            return self.for_rest_ordinary(start, Some(ForInit::Var(decl)));
        }
        if self.at(Punct::Semicolon) {
            if is_await {
                return Err(self.unexpected());
            }
            return self.for_rest_ordinary(start, None);
        }
        // An expression, or the target of `in` / `of`.
        let starts_with_let = self.at_contextual("let");
        let starts_with_async = self.at_contextual("async");
        let mut cover = super::Cover::default();
        let expr_start = self.tok.start;
        let init = self.with_context(|cx| cx.in_ = false, |p| p.lhs_or_expression(&mut cover))?;
        let is_of = self.at_contextual("of");
        if self.at_keyword(Keyword::In) || is_of {
            if is_of && starts_with_let {
                return Err(self.error(
                    "The left-hand side of a for-of loop may not be 'let'.",
                    head_start,
                ));
            }
            if is_of && starts_with_async && !is_await {
                let e = self.ast.expr(init);
                if matches!(&e.kind, Expr::Ident(i) if &*i.name == "async")
                    && e.span == Span::new(expr_start, expr_start + 5)
                {
                    return Err(self.error(
                        "The left-hand side of a for-of loop may not be 'async'.",
                        head_start,
                    ));
                }
            }
            let target = self.assign_target_of(init, "Invalid left-hand side in for-loop")?;
            let left = match target {
                crate::ast::AssignTarget::Pattern(p) => ForHead::Pattern(p),
                crate::ast::AssignTarget::Simple(e) => {
                    let span = self.ast.expr(e).span;
                    ForHead::Pattern(self.ast.add_pat(crate::ast::Pat::Expr(e), span))
                }
            };
            return self.for_in_of_rest(start, left, is_of, is_await);
        }
        self.check_cover(&cover)?;
        if is_await {
            return Err(self.unexpected());
        }
        let init = self.expression_rest(init)?;
        self.for_rest_ordinary(start, Some(ForInit::Expr(init)))
    }

    /// `using x of` in a `for` head (but `using of` is an identifier).
    fn for_using_declares(&self) -> Result<bool, SyntaxError> {
        let (next, after) = self.peek2()?;
        let using_of = next.name(self.source) == "of" && !next.escaped;
        // `using of` begins a declaration only as `using of = ...` (the
        // for-of lookahead restriction, section 14.7.5).
        Ok(next.kind == TokenKind::Identifier
            && !next.newline_before
            && (!using_of || after.is(Punct::Eq)))
    }

    fn for_in_of_declaration(
        &mut self,
        start: u32,
        decl: VarDecl,
        is_of: bool,
        is_await: bool,
        head: Span,
    ) -> Result<StmtId, SyntaxError> {
        let [d] = decl.decls.as_slice() else {
            return Err(self.error(
                if is_of {
                    "Invalid left-hand side in for-of loop: Must have a single binding."
                } else {
                    "Invalid left-hand side in for-in loop: Must have a single binding."
                },
                head,
            ));
        };
        if let Some(init) = d.init {
            // Annex B.3.5: sloppy `for (var x = 1 in o)`, a plain name.
            let simple = matches!(self.ast.pat(d.target).kind, crate::ast::Pat::Ident(_));
            if is_of || self.cx.strict || decl.kind != VarKind::Var || !simple {
                let span = self.ast.expr(init).span;
                return Err(self.error(
                    if is_of {
                        "for-of loop variable declaration may not have an initializer."
                    } else {
                        "for-in loop variable declaration may not have an initializer."
                    },
                    span,
                ));
            }
        }
        if matches!(decl.kind, VarKind::Using | VarKind::AwaitUsing) && !is_of {
            return Err(self.error("Invalid left-hand side in for-in loop", head));
        }
        self.for_in_of_rest(start, ForHead::Var(decl), is_of, is_await)
    }

    fn for_in_of_rest(
        &mut self,
        start: u32,
        left: ForHead,
        is_of: bool,
        is_await: bool,
    ) -> Result<StmtId, SyntaxError> {
        if is_await && !is_of {
            return Err(self.unexpected());
        }
        self.advance()?;
        let right = self.with_context(
            |cx| cx.in_ = true,
            |p| {
                if is_of {
                    p.assignment()
                } else {
                    p.expression()
                }
            },
        )?;
        self.expect(Punct::RParen)?;
        let body = self.loop_body()?;
        let span = Span::new(start, self.prev_end);
        Ok(if is_of {
            self.ast.add_stmt(
                Stmt::ForOf {
                    left,
                    right,
                    body,
                    is_await,
                },
                span,
            )
        } else {
            self.ast.add_stmt(Stmt::ForIn { left, right, body }, span)
        })
    }

    fn for_rest_ordinary(
        &mut self,
        start: u32,
        init: Option<ForInit>,
    ) -> Result<StmtId, SyntaxError> {
        self.expect(Punct::Semicolon)?;
        let test = if self.at(Punct::Semicolon) {
            None
        } else {
            Some(self.with_context(|cx| cx.in_ = true, |p| p.expression())?)
        };
        self.expect(Punct::Semicolon)?;
        let update = if self.at(Punct::RParen) {
            None
        } else {
            Some(self.with_context(|cx| cx.in_ = true, |p| p.expression())?)
        };
        self.expect(Punct::RParen)?;
        let body = self.loop_body()?;
        Ok(self.ast.add_stmt(
            Stmt::For {
                init,
                test,
                update,
                body,
            },
            Span::new(start, self.prev_end),
        ))
    }
}
