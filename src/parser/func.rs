//! Functions, arrows, methods and classes (section 15), and their early
//! errors: parameters against the body's strictness, `super`,
//! constructors and private names.

use super::scope::ScopeKind;
use super::{ClassScope, Parser, PrivateKind, is_strict_reserved};
use crate::SyntaxError;
use crate::ast::{
    Body, Class, ClassId, ClassMember, Expr, ExprId, FuncId, FuncKind, Function, Ident, MemberKind,
    Pat, PatElem, PatId, PropKey, Span, Stmt, StmtId,
};
use crate::lexer::{Keyword, Punct, TokenKind};

/// A function's parameter list as parsed.
struct Params {
    params: Vec<PatElem>,
    rest: Option<PatId>,
    simple: bool,
    /// The first name bound twice, an error unless the function allows it.
    duplicate: Option<Ident>,
}

impl Parser<'_> {
    /// `function name(...) {...}` (or `async function`, generators) as a
    /// declaration. `annex_b` for a sloppy `if` body or label, which only
    /// a plain function may be; `name_optional` for `export default`.
    pub(super) fn function_declaration(
        &mut self,
        is_async: bool,
        annex_b: bool,
    ) -> Result<StmtId, SyntaxError> {
        self.function_declaration_named(is_async, annex_b, false)
    }

    pub(super) fn function_declaration_named(
        &mut self,
        is_async: bool,
        annex_b: bool,
        name_optional: bool,
    ) -> Result<StmtId, SyntaxError> {
        let start = self.tok.start;
        if is_async {
            self.advance()?;
        }
        self.expect_keyword(Keyword::Function)?;
        let is_generator = self.eat(Punct::Star)?;
        if annex_b && (is_generator || is_async) {
            return Err(self.error(
                "In non-strict mode code, functions can only be declared at top level, inside a block, or as the body of an if statement.",
                Span::new(start, self.prev_end),
            ));
        }
        let name = if self.at_identifier() {
            // A declaration's name is bound in the enclosing scope, so the
            // enclosing context judges it.
            let name = self.identifier(true)?;
            let sloppy_plain = !self.cx.strict && !is_async && !is_generator;
            self.scopes
                .declare_function(&name.name, name.span, sloppy_plain)?;
            Some(name)
        } else if name_optional {
            None
        } else {
            return Err(self.unexpected());
        };
        let f = self.function_rest(FuncKind::Normal, is_async, is_generator, name, start)?;
        Ok(self
            .ast
            .add_stmt(Stmt::Function(f), Span::new(start, self.prev_end)))
    }

    /// A function expression from `function` (its `async` already read).
    pub(super) fn function_expression(
        &mut self,
        is_async: bool,
        start: u32,
    ) -> Result<ExprId, SyntaxError> {
        self.enter()?;
        let result = self.function_expression_inner(is_async, start);
        self.leave();
        result
    }

    fn function_expression_inner(
        &mut self,
        is_async: bool,
        start: u32,
    ) -> Result<ExprId, SyntaxError> {
        self.expect_keyword(Keyword::Function)?;
        let is_generator = self.eat(Punct::Star)?;
        let name = if self.at_identifier() {
            // An expression's name is bound in its own scope, so its own
            // context judges it.
            Some(self.with_context(
                |cx| {
                    cx.yield_ = is_generator;
                    cx.await_ = is_async;
                    cx.static_block = false;
                },
                |p| p.identifier(true),
            )?)
        } else {
            None
        };
        let f = self.function_rest(FuncKind::Normal, is_async, is_generator, name, start)?;
        Ok(self
            .ast
            .add_expr(Expr::Function(f), Span::new(start, self.prev_end)))
    }

    /// A method's parameters and body, from its `(`, as an expression.
    pub(super) fn method(
        &mut self,
        kind: FuncKind,
        is_async: bool,
        is_generator: bool,
        start: u32,
        _class: bool,
    ) -> Result<ExprId, SyntaxError> {
        self.enter()?;
        let f = self.function_rest(kind, is_async, is_generator, None, start);
        self.leave();
        let f = f?;
        Ok(self
            .ast
            .add_expr(Expr::Function(f), Span::new(start, self.prev_end)))
    }

    /// Parameters and body of any function but an arrow, in a context and
    /// a scope of its own.
    fn function_rest(
        &mut self,
        kind: FuncKind,
        is_async: bool,
        is_generator: bool,
        name: Option<Ident>,
        start: u32,
    ) -> Result<FuncId, SyntaxError> {
        let saved_cx = self.cx;
        let saved_labels = std::mem::take(&mut self.labels);
        let saved_jumps = (self.in_loop, self.in_breakable);
        let saved_marks = (self.yield_at, self.await_at, self.await_ident_at);
        self.cx.yield_ = is_generator;
        self.cx.await_ = is_async;
        self.cx.return_ = true;
        self.cx.new_target = true;
        self.cx.super_call = matches!(kind, FuncKind::Constructor { derived: true });
        self.cx.super_prop = matches!(
            kind,
            FuncKind::Method | FuncKind::Getter | FuncKind::Setter | FuncKind::Constructor { .. }
        );
        self.cx.class_field = false;
        self.cx.static_block = false;
        self.cx.in_ = true;
        self.in_loop = false;
        self.in_breakable = false;
        self.yield_at = None;
        self.await_at = None;
        self.await_ident_at = None;
        self.scopes.push(ScopeKind::Function);
        let result = self.function_parts(kind, is_async, is_generator, name, start);
        self.scopes.pop();
        self.cx = saved_cx;
        self.labels = saved_labels;
        (self.in_loop, self.in_breakable) = saved_jumps;
        (self.yield_at, self.await_at, self.await_ident_at) = saved_marks;
        result
    }

    fn function_parts(
        &mut self,
        kind: FuncKind,
        is_async: bool,
        is_generator: bool,
        name: Option<Ident>,
        start: u32,
    ) -> Result<FuncId, SyntaxError> {
        let params = self.formal_parameters()?;
        if let Some(span) = self.yield_at {
            return Err(self.error("Yield expression not allowed in formal parameter", span));
        }
        if let Some(span) = self.await_at {
            return Err(self.error(
                "Illegal await-expression in formal parameters of async function",
                span,
            ));
        }
        match kind {
            FuncKind::Getter if !params.params.is_empty() || params.rest.is_some() => {
                return Err(self.error(
                    "Getter must not have any formal parameters.",
                    Span::new(start, self.prev_end),
                ));
            }
            FuncKind::Setter if params.rest.is_some() => {
                return Err(self.error(
                    "Setter function argument must not be a rest parameter",
                    Span::new(start, self.prev_end),
                ));
            }
            FuncKind::Setter if params.params.len() != 1 => {
                return Err(self.error(
                    "Setter must have exactly one formal parameter.",
                    Span::new(start, self.prev_end),
                ));
            }
            _ => {}
        }
        let was_strict = self.cx.strict;
        self.expect(Punct::LBrace)?;
        let (body, strict, use_strict) =
            self.directives_and_body(|p| p.at(Punct::RBrace) || p.at_eof(), false)?;
        self.expect(Punct::RBrace)?;
        self.cx.strict = strict;
        self.check_params(&params, name.as_ref(), kind, was_strict, use_strict, start)?;
        Ok(self.ast.add_func(Function {
            kind,
            name,
            params: params.params,
            rest: params.rest,
            body: Body::Block(body),
            is_async,
            is_generator,
            strict,
            simple_params: params.simple,
            span: Span::new(start, self.prev_end),
        }))
    }

    /// The rules a function's parameters and name answer to once its
    /// body's strictness is known (sections 15.2.1 and 15.1.1).
    fn check_params(
        &self,
        params: &Params,
        name: Option<&Ident>,
        kind: FuncKind,
        was_strict: bool,
        use_strict: bool,
        start: u32,
    ) -> Result<(), SyntaxError> {
        let strict = self.cx.strict;
        if use_strict && !params.simple {
            return Err(self.error(
                "Illegal 'use strict' directive in function with non-simple parameter list",
                Span::new(start, self.prev_end),
            ));
        }
        let duplicates_allowed = !strict && params.simple && kind == FuncKind::Normal;
        if let Some(dup) = &params.duplicate
            && !duplicates_allowed
        {
            return Err(self.error(
                "Duplicate parameter name not allowed in this context",
                dup.span,
            ));
        }
        if strict && !was_strict {
            // Names judged before the directive made the code strict.
            if let Some(name) = name {
                self.check_strict_binding(name)?;
            }
            for p in params.params.iter().map(|p| p.target).chain(params.rest) {
                for n in self.bound_names(p) {
                    self.check_strict_binding(&n)?;
                }
            }
        }
        Ok(())
    }

    fn check_strict_binding(&self, name: &Ident) -> Result<(), SyntaxError> {
        if &*name.name == "eval" || &*name.name == "arguments" {
            return Err(self.error("Unexpected eval or arguments in strict mode", name.span));
        }
        if is_strict_reserved(&name.name) {
            return Err(self.error("Unexpected strict mode reserved word", name.span));
        }
        Ok(())
    }

    /// `( FormalParameters )`, each name declared in the function's scope.
    fn formal_parameters(&mut self) -> Result<Params, SyntaxError> {
        self.expect(Punct::LParen)?;
        let mut params = Vec::new();
        let mut rest = None;
        while !self.at(Punct::RParen) {
            if self.at(Punct::Ellipsis) {
                let span = self.tok_span();
                self.advance()?;
                rest = Some(self.binding_target()?);
                if !self.at(Punct::RParen) {
                    return Err(self.error("Rest parameter must be last formal parameter", span));
                }
                break;
            }
            params.push(self.binding_element()?);
            if !self.at(Punct::RParen) {
                self.expect(Punct::Comma)?;
            }
        }
        self.expect(Punct::RParen)?;
        self.declare_params(params, rest)
    }

    fn declare_params(
        &mut self,
        params: Vec<PatElem>,
        rest: Option<PatId>,
    ) -> Result<Params, SyntaxError> {
        let simple = rest.is_none()
            && params.iter().all(|p| {
                p.default.is_none() && matches!(self.ast.pat(p.target).kind, Pat::Ident(_))
            });
        let mut duplicate = None;
        for p in params.iter().map(|p| p.target).chain(rest) {
            for name in self.bound_names(p) {
                if !self.scopes.declare_param(&name.name) && duplicate.is_none() {
                    duplicate = Some(name);
                }
            }
        }
        Ok(Params {
            params,
            rest,
            simple,
            duplicate,
        })
    }

    /// `x => body` (or `async x => body`, its `async` read), from the
    /// parameter's token.
    pub(super) fn identifier_arrow(
        &mut self,
        start: u32,
        is_async: bool,
    ) -> Result<ExprId, SyntaxError> {
        let ident = self.identifier(true)?;
        if is_async && &*ident.name == "await" {
            return Err(self.error(
                "Illegal await-expression in formal parameters of async function",
                ident.span,
            ));
        }
        let span = ident.span;
        let target = self.ast.add_pat(Pat::Ident(ident), span);
        self.expect(Punct::Arrow)?;
        self.arrow_body(
            start,
            vec![PatElem {
                target,
                default: None,
            }],
            None,
            is_async,
        )
    }

    /// An arrow from a parenthesized cover, or `async(...)`'s: its items
    /// read again as parameters, then `=>` and the body.
    pub(super) fn cover_arrow(
        &mut self,
        start: u32,
        items: Vec<ExprId>,
        rest: Option<(PatId, Span)>,
        is_async: bool,
        _unused: Option<()>,
    ) -> Result<ExprId, SyntaxError> {
        let mut params = Vec::new();
        for item in items {
            params.push(self.param_of(item)?);
        }
        let rest = match rest {
            Some((p, _)) => {
                self.validate_binding(p)?;
                Some(p)
            }
            None => None,
        };
        self.expect(Punct::Arrow)?;
        self.arrow_body(start, params, rest, is_async)
    }

    fn arrow_body(
        &mut self,
        start: u32,
        params: Vec<PatElem>,
        rest: Option<PatId>,
        is_async: bool,
    ) -> Result<ExprId, SyntaxError> {
        let saved_cx = self.cx;
        let saved_labels = std::mem::take(&mut self.labels);
        let saved_jumps = (self.in_loop, self.in_breakable);
        let saved_marks = (self.yield_at, self.await_at, self.await_ident_at);
        self.cx.yield_ = false;
        self.cx.await_ = is_async;
        self.cx.return_ = true;
        self.cx.static_block = false;
        self.in_loop = false;
        self.in_breakable = false;
        self.scopes.push(ScopeKind::Function);
        let result = self.arrow_parts(start, params, rest, is_async);
        self.scopes.pop();
        self.cx = saved_cx;
        self.labels = saved_labels;
        (self.in_loop, self.in_breakable) = saved_jumps;
        (self.yield_at, self.await_at, self.await_ident_at) = saved_marks;
        result
    }

    fn arrow_parts(
        &mut self,
        start: u32,
        params: Vec<PatElem>,
        rest: Option<PatId>,
        is_async: bool,
    ) -> Result<ExprId, SyntaxError> {
        let params = self.declare_params(params, rest)?;
        let was_strict = self.cx.strict;
        let mut use_strict = false;
        let body = if self.at(Punct::LBrace) {
            self.advance()?;
            let in_ = self.cx.in_;
            self.cx.in_ = true;
            let (body, strict, has_use_strict) =
                self.directives_and_body(|p| p.at(Punct::RBrace) || p.at_eof(), false)?;
            use_strict = has_use_strict;
            self.expect(Punct::RBrace)?;
            self.cx.in_ = in_;
            self.cx.strict = strict;
            Body::Block(body)
        } else {
            Body::Expr(self.assignment()?)
        };
        self.check_params(
            &params,
            None,
            FuncKind::Arrow,
            was_strict,
            use_strict,
            start,
        )?;
        let f = self.ast.add_func(Function {
            kind: FuncKind::Arrow,
            name: None,
            params: params.params,
            rest: params.rest,
            body,
            is_async,
            is_generator: false,
            strict: self.cx.strict,
            simple_params: params.simple,
            span: Span::new(start, self.prev_end),
        });
        Ok(self
            .ast
            .add_expr(Expr::Arrow(f), Span::new(start, self.prev_end)))
    }

    /// `class Name ...` as a declaration (`name_optional` for `export
    /// default`).
    pub(super) fn class_declaration(&mut self, name_optional: bool) -> Result<StmtId, SyntaxError> {
        let start = self.tok.start;
        self.advance()?;
        let name = if self.at_identifier() {
            let name = self.with_context(|cx| cx.strict = true, |p| p.identifier(true))?;
            self.scopes.declare_lexical(&name.name, name.span)?;
            Some(name)
        } else if name_optional {
            None
        } else {
            return Err(self.unexpected());
        };
        let class = self.class_rest(start, name)?;
        Ok(self
            .ast
            .add_stmt(Stmt::Class(class), Span::new(start, self.prev_end)))
    }

    pub(super) fn class_expression(&mut self) -> Result<ExprId, SyntaxError> {
        let start = self.tok.start;
        self.enter()?;
        let result = (|| {
            self.advance()?;
            let name = if self.at_identifier() {
                Some(self.with_context(|cx| cx.strict = true, |p| p.identifier(true))?)
            } else {
                None
            };
            self.class_rest(start, name)
        })();
        self.leave();
        let class = result?;
        Ok(self
            .ast
            .add_expr(Expr::Class(class), Span::new(start, self.prev_end)))
    }

    /// A class's heritage and body, all strict code (section 15.7).
    fn class_rest(&mut self, start: u32, name: Option<Ident>) -> Result<ClassId, SyntaxError> {
        let saved = self.cx;
        self.cx.strict = true;
        let result = self.class_parts(start, name);
        self.cx = saved;
        result
    }

    fn class_parts(&mut self, start: u32, name: Option<Ident>) -> Result<ClassId, SyntaxError> {
        let extends = if self.eat_keyword(Keyword::Extends)? {
            let mut cover = super::Cover::default();
            let e = self.left_hand_side(&mut cover)?;
            self.check_cover(&cover)?;
            if self.is_bare_arrow(e) {
                let span = self.ast.expr(e).span;
                return Err(self.error("Malformed arrow function parameter list", span));
            }
            Some(e)
        } else {
            None
        };
        self.expect(Punct::LBrace)?;
        self.classes.push(ClassScope::default());
        let result = self.class_body(extends.is_some());
        let scope = self.classes.pop().unwrap_or_default();
        let (constructor, members) = result?;
        self.resolve_privates(scope)?;
        Ok(self.ast.add_class(Class {
            name,
            extends,
            constructor,
            members,
            span: Span::new(start, self.prev_end),
        }))
    }

    /// Hands the private names a class used but did not declare to the
    /// class around it, or refuses them at the outermost.
    fn resolve_privates(&mut self, scope: ClassScope) -> Result<(), SyntaxError> {
        for used in scope.used {
            if scope.declared.contains_key(&used.name) {
                continue;
            }
            match self.classes.last_mut() {
                Some(outer) => outer.used.push(used),
                None => {
                    return Err(self.error(
                        format!(
                            "Private field '#{}' must be declared in an enclosing class",
                            used.name
                        ),
                        used.span,
                    ));
                }
            }
        }
        Ok(())
    }

    fn class_body(
        &mut self,
        derived: bool,
    ) -> Result<(Option<FuncId>, Vec<ClassMember>), SyntaxError> {
        let mut constructor = None;
        let mut members = Vec::new();
        while !self.eat(Punct::RBrace)? {
            if self.eat(Punct::Semicolon)? {
                continue;
            }
            if self.at_eof() {
                return Err(self.unexpected());
            }
            let start = self.tok.start;
            let mut is_static = false;
            if self.at_contextual("static") {
                let next = self.peek()?;
                if next.is(Punct::LBrace) {
                    self.advance()?;
                    let block = self.static_block(start)?;
                    members.push(ClassMember {
                        kind: MemberKind::StaticBlock,
                        is_static: true,
                        key: PropKey::Ident(Ident {
                            name: "".into(),
                            span: Span::new(start, start),
                        }),
                        value: Some(block),
                        span: Span::new(start, self.prev_end),
                    });
                    continue;
                }
                if !matches!(
                    next.kind,
                    TokenKind::Punct(Punct::LParen | Punct::Eq | Punct::Semicolon | Punct::RBrace)
                        | TokenKind::Eof
                ) {
                    is_static = true;
                    self.advance()?;
                }
            }
            let (is_async, is_generator, accessor) = self.method_modifiers()?;
            let key_span = self.tok_span();
            let key = self.property_key(true)?;
            let key_name = static_key_name(&key);
            if let PropKey::Private(p) = &key
                && &*p.name == "constructor"
            {
                return Err(self.error(
                    "Classes may not have a private field named '#constructor'",
                    key_span,
                ));
            }
            if self.at(Punct::LParen) {
                let is_constructor = !is_static && key_name == Some("constructor");
                if is_constructor {
                    let what = if accessor.is_some() {
                        Some("Class constructor may not be an accessor")
                    } else if is_generator {
                        Some("Class constructor may not be a generator")
                    } else if is_async {
                        Some("Class constructor may not be an async method")
                    } else {
                        None
                    };
                    if let Some(what) = what {
                        return Err(self.error(what, key_span));
                    }
                    if constructor.is_some() {
                        return Err(self.error("A class may only have one constructor", key_span));
                    }
                }
                if is_static && key_name == Some("prototype") {
                    return Err(self.error(
                        "Classes may not have a static property named 'prototype'",
                        key_span,
                    ));
                }
                let (func_kind, member_kind, private_kind) = if is_constructor {
                    (
                        FuncKind::Constructor { derived },
                        MemberKind::Method,
                        PrivateKind::Method,
                    )
                } else {
                    match accessor {
                        Some(true) => (
                            FuncKind::Getter,
                            MemberKind::Getter,
                            PrivateKind::Getter { is_static },
                        ),
                        Some(false) => (
                            FuncKind::Setter,
                            MemberKind::Setter,
                            PrivateKind::Setter { is_static },
                        ),
                        None => (FuncKind::Method, MemberKind::Method, PrivateKind::Method),
                    }
                };
                if let PropKey::Private(p) = &key {
                    self.declare_private(p, private_kind)?;
                }
                self.enter()?;
                let f = self.function_rest(func_kind, is_async, is_generator, None, start);
                self.leave();
                let f = f?;
                if is_constructor {
                    constructor = Some(f);
                } else {
                    members.push(ClassMember {
                        kind: member_kind,
                        is_static,
                        key,
                        value: Some(f),
                        span: Span::new(start, self.prev_end),
                    });
                }
                continue;
            }
            // A field.
            if is_async || is_generator || accessor.is_some() {
                return Err(self.unexpected());
            }
            if key_name == Some("constructor") {
                return Err(
                    self.error("Classes may not have a field named 'constructor'", key_span)
                );
            }
            if is_static && key_name == Some("prototype") {
                return Err(self.error(
                    "Classes may not have a static property named 'prototype'",
                    key_span,
                ));
            }
            if let PropKey::Private(p) = &key {
                self.declare_private(p, PrivateKind::Field)?;
            }
            let value = if self.eat(Punct::Eq)? {
                Some(self.field_initializer(start)?)
            } else {
                None
            };
            self.semicolon()?;
            members.push(ClassMember {
                kind: MemberKind::Field,
                is_static,
                key,
                value,
                span: Span::new(start, self.prev_end),
            });
        }
        Ok((constructor, members))
    }

    /// A private name declared in the class being parsed: twice only as a
    /// getter and a setter of the same placement.
    fn declare_private(&mut self, name: &Ident, kind: PrivateKind) -> Result<(), SyntaxError> {
        let Some(class) = self.classes.last_mut() else {
            return Ok(());
        };
        if let Some(entry) = class.declared.get_mut(&name.name) {
            let pair = match (*entry, kind) {
                (PrivateKind::Getter { is_static: a }, PrivateKind::Setter { is_static: b })
                | (PrivateKind::Setter { is_static: a }, PrivateKind::Getter { is_static: b }) => {
                    a == b
                }
                _ => false,
            };
            if !pair {
                return Err(SyntaxError {
                    message: format!("Identifier '#{}' has already been declared", name.name),
                    start: name.span.start,
                    end: name.span.end,
                });
            }
            *entry = PrivateKind::Both;
            return Ok(());
        }
        class.declared.insert(name.name.clone(), kind);
        Ok(())
    }

    /// The context of a field's initializer or a static block: a method
    /// of the class, where `arguments` is refused.
    fn class_function_context(&mut self, static_block: bool) {
        self.cx.yield_ = false;
        self.cx.await_ = false;
        self.cx.return_ = false;
        self.cx.super_call = false;
        self.cx.super_prop = true;
        self.cx.new_target = true;
        self.cx.class_field = true;
        self.cx.static_block = static_block;
        self.cx.in_ = true;
    }

    fn field_initializer(&mut self, start: u32) -> Result<FuncId, SyntaxError> {
        let saved_cx = self.cx;
        let saved_labels = std::mem::take(&mut self.labels);
        let saved_marks = (self.yield_at, self.await_at, self.await_ident_at);
        self.class_function_context(false);
        self.scopes.push(ScopeKind::Function);
        let result = self.assignment();
        self.scopes.pop();
        self.cx = saved_cx;
        self.labels = saved_labels;
        (self.yield_at, self.await_at, self.await_ident_at) = saved_marks;
        let e = result?;
        Ok(self.ast.add_func(Function {
            kind: FuncKind::ClassField,
            name: None,
            params: Vec::new(),
            rest: None,
            body: Body::Expr(e),
            is_async: false,
            is_generator: false,
            strict: true,
            simple_params: true,
            span: Span::new(start, self.prev_end),
        }))
    }

    /// `static { ... }`, from its `{`.
    fn static_block(&mut self, start: u32) -> Result<FuncId, SyntaxError> {
        let saved_cx = self.cx;
        let saved_labels = std::mem::take(&mut self.labels);
        let saved_jumps = (self.in_loop, self.in_breakable);
        let saved_marks = (self.yield_at, self.await_at, self.await_ident_at);
        self.class_function_context(true);
        self.in_loop = false;
        self.in_breakable = false;
        self.scopes.push(ScopeKind::Function);
        self.enter()?;
        let result = self.expect(Punct::LBrace).and_then(|_| self.block_rest());
        self.leave();
        self.scopes.pop();
        self.cx = saved_cx;
        self.labels = saved_labels;
        (self.in_loop, self.in_breakable) = saved_jumps;
        (self.yield_at, self.await_at, self.await_ident_at) = saved_marks;
        let body = result?;
        Ok(self.ast.add_func(Function {
            kind: FuncKind::StaticBlock,
            name: None,
            params: Vec::new(),
            rest: None,
            body: Body::Block(body),
            is_async: false,
            is_generator: false,
            strict: true,
            simple_params: true,
            span: Span::new(start, self.prev_end),
        }))
    }
}

/// A class element's name when it is spelled literally (an identifier or
/// a string, not computed): what the constructor and `prototype` rules
/// look at (section 15.7.1's PropName).
fn static_key_name(key: &PropKey) -> Option<&str> {
    match key {
        PropKey::Ident(i) => Some(&i.name),
        PropKey::String(s) => {
            if s.iter().copied().eq("constructor".encode_utf16()) {
                Some("constructor")
            } else if s.iter().copied().eq("prototype".encode_utf16()) {
                Some("prototype")
            } else {
                None
            }
        }
        _ => None,
    }
}
