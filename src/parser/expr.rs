//! Expressions (section 13), with the cover grammars that turn into
//! patterns and arrow parameters, and their early errors.

use super::{Cover, Parser};
use crate::SyntaxError;
use crate::ast::{
    AssignOp, AssignTarget, BinaryOp, Expr, ExprId, FuncKind, Ident, LogicalOp, MemberProp, Prop,
    PropKey, PropKind, Quasi, Span, Spreadable, UnaryOp, UpdateOp,
};
use crate::lexer::{Keyword, Punct, TokenKind, TokenValue};

/// A binary operator as the precedence parser sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BinOp {
    Binary(BinaryOp),
    Logical(LogicalOp),
}

/// Relational operators' precedence.
const RELATIONAL: u8 = 8;

/// An operand of the precedence parser: an expression, or `#x` waiting
/// for its `in`.
#[derive(Debug, Clone)]
enum Operand {
    Expr(ExprId),
    Private(Ident),
}

impl Parser<'_> {
    /// An Expression: assignments separated by commas.
    pub(super) fn expression(&mut self) -> Result<ExprId, SyntaxError> {
        let first = self.assignment()?;
        self.expression_rest(first)
    }

    /// The rest of an Expression whose first assignment is parsed.
    pub(super) fn expression_rest(&mut self, first: ExprId) -> Result<ExprId, SyntaxError> {
        if !self.at(Punct::Comma) {
            return Ok(first);
        }
        let mut items = vec![first];
        while self.eat(Punct::Comma)? {
            items.push(self.assignment()?);
        }
        let span = self.span_since(first);
        Ok(self.ast.add_expr(Expr::Sequence(items), span))
    }

    fn span_since(&self, first: ExprId) -> Span {
        Span::new(self.ast.expr(first).span.start, self.prev_end)
    }

    /// An AssignmentExpression, its cover errors raised.
    pub(super) fn assignment(&mut self) -> Result<ExprId, SyntaxError> {
        let mut cover = Cover::default();
        let e = self.assignment_cover(&mut cover)?;
        self.check_cover(&cover)?;
        Ok(e)
    }

    /// Raises what a cover grammar allowed only as a pattern.
    pub(super) fn check_cover(&self, cover: &Cover) -> Result<(), SyntaxError> {
        if let Some(span) = cover.init {
            return Err(self.error("Invalid shorthand property initializer", span));
        }
        if let Some(span) = cover.double_proto {
            return Err(self.error(
                "Duplicate __proto__ fields are not allowed in object literals",
                span,
            ));
        }
        Ok(())
    }

    /// An AssignmentExpression whose pattern-only parts are left in
    /// `cover` for the caller to judge.
    pub(super) fn assignment_cover(&mut self, cover: &mut Cover) -> Result<ExprId, SyntaxError> {
        self.enter()?;
        let result = self.assignment_inner(cover);
        self.leave();
        result
    }

    /// The same, as a for-statement's head reads it.
    pub(super) fn lhs_or_expression(&mut self, cover: &mut Cover) -> Result<ExprId, SyntaxError> {
        self.assignment_cover(cover)
    }

    fn assignment_inner(&mut self, cover: &mut Cover) -> Result<ExprId, SyntaxError> {
        if self.cx.yield_ && self.at_contextual("yield") {
            return self.yield_expression();
        }
        if self.tok.kind == TokenKind::Identifier
            && self.tok.escaped
            && self.tok_name() == "yield"
            && self.cx.yield_
        {
            return Err(self.error(
                "Keyword must not contain escaped characters",
                self.tok_span(),
            ));
        }
        let start = self.tok.start;
        let left = self.conditional(cover)?;
        if self.is_bare_arrow(left) {
            return Ok(left);
        }
        let op = match self.tok.kind {
            TokenKind::Punct(p) => assign_op(p),
            _ => None,
        };
        let Some(op) = op else {
            return Ok(left);
        };
        let target = if op == AssignOp::Assign {
            let target = self.assign_target_of(left, "Invalid left-hand side in assignment")?;
            if let AssignTarget::Pattern(_) = target {
                // What the left allowed only as a pattern is now allowed.
                if cover.init.is_some_and(|s| s.start >= start) {
                    cover.init = None;
                }
                if cover.double_proto.is_some_and(|s| s.start >= start) {
                    cover.double_proto = None;
                }
            } else {
                self.check_cover(cover)?;
            }
            target
        } else {
            self.check_cover(cover)?;
            self.simple_target(
                left,
                matches!(op, AssignOp::Logical(_)),
                || "Invalid left-hand side in assignment",
            )?
        };
        self.bump()?;
        let value = self.assignment()?;
        let span = Span::new(start, self.prev_end);
        Ok(self.ast.add_expr(Expr::Assign { op, target, value }, span))
    }

    /// Whether `e` is an arrow function not in parentheses, which can only
    /// be a whole AssignmentExpression: no operator may follow it.
    pub(super) fn is_bare_arrow(&self, e: ExprId) -> bool {
        matches!(self.ast.expr(e).kind, Expr::Arrow(_))
    }

    fn yield_expression(&mut self) -> Result<ExprId, SyntaxError> {
        let start = self.tok.start;
        let span = self.tok_span();
        if self.yield_at.is_none() {
            self.yield_at = Some(span);
        }
        self.bump()?;
        let delegate = !self.tok.newline_before && self.at(Punct::Star);
        if delegate {
            self.bump()?;
        }
        let arg = if delegate || (!self.tok.newline_before && self.starts_expression()) {
            Some(self.assignment()?)
        } else {
            None
        };
        Ok(self.ast.add_expr(
            Expr::Yield { arg, delegate },
            Span::new(start, self.prev_end),
        ))
    }

    /// Whether the current token can begin an AssignmentExpression.
    fn starts_expression(&self) -> bool {
        match self.tok.kind {
            TokenKind::Identifier
            | TokenKind::Number
            | TokenKind::BigInt
            | TokenKind::String
            | TokenKind::Template { .. }
            | TokenKind::PrivateName => true,
            TokenKind::Keyword(k) => matches!(
                k,
                Keyword::This
                    | Keyword::Function
                    | Keyword::Class
                    | Keyword::New
                    | Keyword::Delete
                    | Keyword::Typeof
                    | Keyword::Void
                    | Keyword::True
                    | Keyword::False
                    | Keyword::Null
                    | Keyword::Super
                    | Keyword::Import
            ),
            TokenKind::Punct(p) => matches!(
                p,
                Punct::LParen
                    | Punct::LBracket
                    | Punct::LBrace
                    | Punct::Plus
                    | Punct::Minus
                    | Punct::Bang
                    | Punct::Tilde
                    | Punct::PlusPlus
                    | Punct::MinusMinus
                    | Punct::Slash
                    | Punct::SlashEq
            ),
            TokenKind::RegExp | TokenKind::Eof => false,
        }
    }

    fn conditional(&mut self, cover: &mut Cover) -> Result<ExprId, SyntaxError> {
        let start = self.tok.start;
        let test = self.binary(cover)?;
        if self.is_bare_arrow(test) || !self.at(Punct::Question) {
            return Ok(test);
        }
        self.check_cover(cover)?;
        self.bump()?;
        let consequent = self.with_context(|cx| cx.in_ = true, |p| p.assignment())?;
        self.expect(Punct::Colon)?;
        let alternate = self.assignment()?;
        Ok(self.ast.add_expr(
            Expr::Conditional {
                test,
                consequent,
                alternate,
            },
            Span::new(start, self.prev_end),
        ))
    }

    /// The binary operator at the current token and its precedence.
    fn binary_op(&self) -> Option<(BinOp, u8)> {
        use BinaryOp as B;
        let op = match self.tok.kind {
            TokenKind::Punct(p) => match p {
                Punct::QuestionQuestion => (BinOp::Logical(LogicalOp::Coalesce), 1),
                Punct::PipePipe => (BinOp::Logical(LogicalOp::Or), 2),
                Punct::AmpAmp => (BinOp::Logical(LogicalOp::And), 3),
                Punct::Pipe => (BinOp::Binary(B::BitOr), 4),
                Punct::Caret => (BinOp::Binary(B::BitXor), 5),
                Punct::Amp => (BinOp::Binary(B::BitAnd), 6),
                Punct::EqEq => (BinOp::Binary(B::Eq), 7),
                Punct::NotEq => (BinOp::Binary(B::NotEq), 7),
                Punct::EqEqEq => (BinOp::Binary(B::StrictEq), 7),
                Punct::NotEqEq => (BinOp::Binary(B::StrictNotEq), 7),
                Punct::Lt => (BinOp::Binary(B::Lt), RELATIONAL),
                Punct::Gt => (BinOp::Binary(B::Gt), RELATIONAL),
                Punct::LtEq => (BinOp::Binary(B::LtEq), RELATIONAL),
                Punct::GtEq => (BinOp::Binary(B::GtEq), RELATIONAL),
                Punct::Shl => (BinOp::Binary(B::Shl), 9),
                Punct::Shr => (BinOp::Binary(B::Shr), 9),
                Punct::UShr => (BinOp::Binary(B::UShr), 9),
                Punct::Plus => (BinOp::Binary(B::Add), 10),
                Punct::Minus => (BinOp::Binary(B::Sub), 10),
                Punct::Star => (BinOp::Binary(B::Mul), 11),
                Punct::Slash => (BinOp::Binary(B::Div), 11),
                Punct::Percent => (BinOp::Binary(B::Mod), 11),
                Punct::StarStar => (BinOp::Binary(B::Exp), 12),
                _ => return None,
            },
            TokenKind::Keyword(Keyword::Instanceof) => (BinOp::Binary(B::Instanceof), RELATIONAL),
            TokenKind::Keyword(Keyword::In) if self.cx.in_ => (BinOp::Binary(B::In), RELATIONAL),
            _ => return None,
        };
        Some(op)
    }

    /// Binary and logical operators by precedence, in a loop with stacks
    /// of its own: `a + b + c ...` of any length never recurses.
    fn binary(&mut self, cover: &mut Cover) -> Result<ExprId, SyntaxError> {
        let first = self.binary_operand(cover)?;
        if let Operand::Expr(e) = &first
            && (self.is_bare_arrow(*e) || self.binary_op().is_none())
        {
            return Ok(*e);
        }
        if let Operand::Expr(_) = &first {
            self.check_cover(cover)?;
        }
        let mut operands = vec![first];
        let mut ops: Vec<(BinOp, u8)> = Vec::new();
        while let Some((op, prec)) = self.binary_op() {
            while let Some(&(top, top_prec)) = ops.last() {
                // `**` is right-associative; the rest are left.
                let pops =
                    top_prec > prec || (top_prec == prec && op != BinOp::Binary(BinaryOp::Exp));
                if !pops {
                    break;
                }
                ops.pop();
                self.reduce(&mut operands, top)?;
            }
            if op == BinOp::Binary(BinaryOp::Exp)
                && let Some(Operand::Expr(left)) = operands.last()
                && matches!(
                    self.ast.expr(*left).kind,
                    Expr::Unary { .. } | Expr::Await(_)
                )
            {
                return Err(self.error(
                    "Unary operator used immediately before exponentiation expression. Parenthesis must be used to disambiguate operator precedence",
                    self.tok_span(),
                ));
            }
            self.bump()?;
            ops.push((op, prec));
            let mut inner = Cover::default();
            let operand = self.binary_operand(&mut inner)?;
            self.check_cover(&inner)?;
            if let Operand::Expr(e) = &operand
                && self.is_bare_arrow(*e)
            {
                let span = self.ast.expr(*e).span;
                return Err(self.error("Malformed arrow function parameter list", span));
            }
            operands.push(operand);
        }
        while let Some((op, _)) = ops.pop() {
            self.reduce(&mut operands, op)?;
        }
        match operands.pop() {
            Some(Operand::Expr(e)) if operands.is_empty() => Ok(e),
            Some(Operand::Private(name)) => {
                Err(self.error(format!("Unexpected identifier '#{}'", name.name), name.span))
            }
            _ => Err(self.unexpected()),
        }
    }

    /// One operand of the precedence parser: a unary expression, or a
    /// private name standing before `in`. Where an operator before it
    /// binds tighter, [`Parser::reduce`] refuses it as that operator's
    /// operand.
    fn binary_operand(&mut self, cover: &mut Cover) -> Result<Operand, SyntaxError> {
        if self.tok.kind == TokenKind::PrivateName {
            let next = self.lookahead()?;
            if next.kind == TokenKind::Keyword(Keyword::In) && self.cx.in_ {
                let name = Ident {
                    name: super::intern(&mut self.names, self.tok.name(self.source)),
                    span: self.tok_span(),
                };
                self.use_private(&name)?;
                self.bump()?;
                return Ok(Operand::Private(name));
            }
            return Err(self.unexpected());
        }
        Ok(Operand::Expr(self.unary(cover)?))
    }

    fn reduce(&mut self, operands: &mut Vec<Operand>, op: BinOp) -> Result<(), SyntaxError> {
        let (Some(right), Some(left)) = (operands.pop(), operands.pop()) else {
            return Err(self.unexpected());
        };
        let Operand::Expr(right) = right else {
            return Err(self.unexpected());
        };
        let right_span = self.ast.expr(right).span;
        let expr = match (left, op) {
            (Operand::Private(name), BinOp::Binary(BinaryOp::In)) => {
                let span = Span::new(name.span.start, right_span.end);
                self.ast.add_expr(Expr::PrivateIn { name, right }, span)
            }
            (Operand::Private(name), _) => {
                return Err(
                    self.error(format!("Unexpected identifier '#{}'", name.name), name.span)
                );
            }
            (Operand::Expr(left), BinOp::Binary(op)) => {
                let span = self.ast.expr(left).span.to(right_span);
                self.ast.add_expr(Expr::Binary { op, left, right }, span)
            }
            (Operand::Expr(left), BinOp::Logical(op)) => {
                // `??` mixes with `&&` and `||` only in parentheses
                // (section 13.13).
                let mixes = |p: &Self, e: ExprId| match p.ast.expr(e).kind {
                    Expr::Logical { op: other, .. } => {
                        (op == LogicalOp::Coalesce) != (other == LogicalOp::Coalesce)
                    }
                    _ => false,
                };
                if mixes(self, left) || mixes(self, right) {
                    return Err(self.error(
                        "Unexpected token '??'",
                        self.ast.expr(left).span.to(right_span),
                    ));
                }
                let span = self.ast.expr(left).span.to(right_span);
                self.ast.add_expr(Expr::Logical { op, left, right }, span)
            }
        };
        operands.push(Operand::Expr(expr));
        Ok(())
    }

    /// A UnaryExpression or UpdateExpression: prefix operators gathered in
    /// a loop, then the operand, then the operators applied inside out.
    fn unary(&mut self, cover: &mut Cover) -> Result<ExprId, SyntaxError> {
        #[derive(Clone, Copy)]
        enum Prefix {
            Unary(UnaryOp),
            Update(UpdateOp),
            Await,
        }
        let mut prefixes: Vec<(Prefix, u32)> = Vec::new();
        loop {
            let prefix = match self.tok.kind {
                TokenKind::Punct(Punct::Bang) => Prefix::Unary(UnaryOp::Not),
                TokenKind::Punct(Punct::Tilde) => Prefix::Unary(UnaryOp::BitNot),
                TokenKind::Punct(Punct::Plus) => Prefix::Unary(UnaryOp::Plus),
                TokenKind::Punct(Punct::Minus) => Prefix::Unary(UnaryOp::Minus),
                TokenKind::Punct(Punct::PlusPlus) => Prefix::Update(UpdateOp::Increment),
                TokenKind::Punct(Punct::MinusMinus) => Prefix::Update(UpdateOp::Decrement),
                TokenKind::Keyword(Keyword::Typeof) => Prefix::Unary(UnaryOp::Typeof),
                TokenKind::Keyword(Keyword::Void) => Prefix::Unary(UnaryOp::Void),
                TokenKind::Keyword(Keyword::Delete) => Prefix::Unary(UnaryOp::Delete),
                TokenKind::Identifier
                    if self.cx.await_
                        && self.at_contextual("await")
                        && !self.in_static_block_await() =>
                {
                    if self.await_at.is_none() {
                        self.await_at = Some(self.tok_span());
                    }
                    Prefix::Await
                }
                _ => break,
            };
            prefixes.push((prefix, self.tok.start));
            self.bump()?;
            if prefixes.len() > super::MAX_DEPTH as usize {
                return Err(self.error("Maximum call stack size exceeded", self.tok_span()));
            }
        }
        if self.tok.kind == TokenKind::Identifier
            && self.tok.escaped
            && self.tok_name() == "await"
            && self.cx.await_
        {
            return Err(self.error(
                "Keyword must not contain escaped characters",
                self.tok_span(),
            ));
        }
        let mut expr = self.postfix(cover)?;
        if !prefixes.is_empty() && self.is_bare_arrow(expr) {
            let span = self.ast.expr(expr).span;
            return Err(self.error("Malformed arrow function parameter list", span));
        }
        while let Some((prefix, start)) = prefixes.pop() {
            let span = Span::new(start, self.prev_end);
            expr = match prefix {
                Prefix::Unary(op) => {
                    if op == UnaryOp::Delete {
                        self.check_delete(expr)?;
                    }
                    self.ast.add_expr(Expr::Unary { op, arg: expr }, span)
                }
                Prefix::Update(op) => {
                    let target = self.update_target(
                        expr,
                        "Invalid left-hand side expression in prefix operation",
                    )?;
                    self.ast.add_expr(
                        Expr::Update {
                            op,
                            prefix: true,
                            target,
                        },
                        span,
                    )
                }
                Prefix::Await => self.ast.add_expr(Expr::Await(expr), span),
            };
        }
        Ok(expr)
    }

    /// `await` is never an operator in a static block's own code, which
    /// has no await context (section 15.7.1).
    fn in_static_block_await(&self) -> bool {
        false
    }

    /// Strict code may not delete a plain name, and no code a private
    /// field (section 13.5.1.1).
    fn check_delete(&self, arg: ExprId) -> Result<(), SyntaxError> {
        let mut e = arg;
        loop {
            match &self.ast.expr(e).kind {
                Expr::Paren(inner) => e = *inner,
                Expr::Ident(_) if self.cx.strict => {
                    return Err(self.error(
                        "Delete of an unqualified identifier in strict mode.",
                        self.ast.expr(arg).span,
                    ));
                }
                Expr::Member {
                    prop: MemberProp::Private(_),
                    ..
                } => {
                    return Err(
                        self.error("Private fields can not be deleted", self.ast.expr(arg).span)
                    );
                }
                Expr::OptionalChain(inner) => e = *inner,
                _ => return Ok(()),
            }
        }
    }

    /// The target of `++` or `--`: an identifier or a member access.
    fn update_target(&self, e: ExprId, message: &'static str) -> Result<ExprId, SyntaxError> {
        match self.simple_target(e, false, || message)? {
            AssignTarget::Simple(e) => Ok(e),
            AssignTarget::Pattern(_) => Err(self.error(message, self.ast.expr(e).span)),
        }
    }

    /// An expression whose AssignmentTargetType is simple (section
    /// 13.15.1): a name (not `eval` or `arguments` in strict code) or a
    /// member access, maybe parenthesized; and, for sloppy code, a call
    /// (web compatibility: its error is thrown when it runs) unless
    /// `logical` (`&&=` and the like never take one).
    pub(super) fn simple_target(
        &self,
        e: ExprId,
        logical: bool,
        message: impl Fn() -> &'static str,
    ) -> Result<AssignTarget, SyntaxError> {
        let span = self.ast.expr(e).span;
        let mut inner = e;
        loop {
            match &self.ast.expr(inner).kind {
                Expr::Paren(x) => inner = *x,
                Expr::Ident(id) => {
                    if self.cx.strict && (&*id.name == "eval" || &*id.name == "arguments") {
                        return Err(self.error("Unexpected eval or arguments in strict mode", span));
                    }
                    return Ok(AssignTarget::Simple(e));
                }
                Expr::Member {
                    optional: false, ..
                }
                | Expr::SuperMember(_) => return Ok(AssignTarget::Simple(e)),
                Expr::Call {
                    optional: false, ..
                } if !self.cx.strict && !logical => {
                    return Ok(AssignTarget::Simple(e));
                }
                _ => return Err(self.error(message(), span)),
            }
        }
    }

    /// A LeftHandSideExpression and a postfix `++` or `--` after it.
    fn postfix(&mut self, cover: &mut Cover) -> Result<ExprId, SyntaxError> {
        let start = self.tok.start;
        let expr = self.left_hand_side(cover)?;
        if self.is_bare_arrow(expr) {
            return Ok(expr);
        }
        let op = match self.tok.kind {
            TokenKind::Punct(Punct::PlusPlus) => UpdateOp::Increment,
            TokenKind::Punct(Punct::MinusMinus) => UpdateOp::Decrement,
            _ => return Ok(expr),
        };
        if self.tok.newline_before {
            return Ok(expr);
        }
        self.check_cover(cover)?;
        let target = self.update_target(
            expr,
            "Invalid left-hand side expression in postfix operation",
        )?;
        self.bump()?;
        Ok(self.ast.add_expr(
            Expr::Update {
                op,
                prefix: false,
                target,
            },
            Span::new(start, self.prev_end),
        ))
    }

    /// A LeftHandSideExpression (section 13.3): a primary expression or
    /// `new`, then member accesses, calls, templates and optional chains
    /// in a loop.
    pub(super) fn left_hand_side(&mut self, cover: &mut Cover) -> Result<ExprId, SyntaxError> {
        let start = self.tok.start;
        let base = if self.at_keyword(Keyword::New) {
            self.new_expression()?
        } else if self.at_keyword(Keyword::Super) {
            self.super_expression()?
        } else if self.at_keyword(Keyword::Import) {
            self.import_expression()?
        } else {
            self.primary(cover)?
        };
        if self.is_bare_arrow(base) {
            return Ok(base);
        }
        self.subscripts(base, start, true)
    }

    /// Member accesses, calls (`calls`), tagged templates and optional
    /// chains after `base`.
    fn subscripts(&mut self, base: ExprId, start: u32, calls: bool) -> Result<ExprId, SyntaxError> {
        let mut expr = base;
        let mut chain = false;
        loop {
            match self.tok.kind {
                TokenKind::Punct(Punct::Dot) => {
                    self.bump()?;
                    let prop = self.member_name()?;
                    expr = self.ast.add_expr(
                        Expr::Member {
                            object: expr,
                            prop,
                            optional: false,
                        },
                        Span::new(start, self.prev_end),
                    );
                }
                TokenKind::Punct(Punct::LBracket) => {
                    self.bump()?;
                    let index = self.with_context(|cx| cx.in_ = true, |p| p.expression())?;
                    self.expect(Punct::RBracket)?;
                    expr = self.ast.add_expr(
                        Expr::Member {
                            object: expr,
                            prop: MemberProp::Computed(index),
                            optional: false,
                        },
                        Span::new(start, self.prev_end),
                    );
                }
                TokenKind::Punct(Punct::LParen) if calls => {
                    let args = self.arguments()?;
                    expr = self.ast.add_expr(
                        Expr::Call {
                            callee: expr,
                            args,
                            optional: false,
                        },
                        Span::new(start, self.prev_end),
                    );
                }
                TokenKind::Punct(Punct::QuestionDot) if calls => {
                    self.bump()?;
                    chain = true;
                    expr = match self.tok.kind {
                        TokenKind::Punct(Punct::LParen) => {
                            let args = self.arguments()?;
                            self.ast.add_expr(
                                Expr::Call {
                                    callee: expr,
                                    args,
                                    optional: true,
                                },
                                Span::new(start, self.prev_end),
                            )
                        }
                        TokenKind::Punct(Punct::LBracket) => {
                            self.bump()?;
                            let index =
                                self.with_context(|cx| cx.in_ = true, |p| p.expression())?;
                            self.expect(Punct::RBracket)?;
                            self.ast.add_expr(
                                Expr::Member {
                                    object: expr,
                                    prop: MemberProp::Computed(index),
                                    optional: true,
                                },
                                Span::new(start, self.prev_end),
                            )
                        }
                        TokenKind::Template { .. } => {
                            return Err(self.error(
                                "Invalid tagged template on optional chain",
                                self.tok_span(),
                            ));
                        }
                        _ => {
                            let prop = self.member_name()?;
                            self.ast.add_expr(
                                Expr::Member {
                                    object: expr,
                                    prop,
                                    optional: true,
                                },
                                Span::new(start, self.prev_end),
                            )
                        }
                    };
                }
                TokenKind::Template { .. } => {
                    if chain {
                        return Err(self
                            .error("Invalid tagged template on optional chain", self.tok_span()));
                    }
                    let (quasis, exprs) = self.template(true)?;
                    expr = self.ast.add_expr(
                        Expr::TaggedTemplate {
                            tag: expr,
                            quasis,
                            exprs,
                        },
                        Span::new(start, self.prev_end),
                    );
                }
                _ => break,
            }
        }
        if chain {
            expr = self
                .ast
                .add_expr(Expr::OptionalChain(expr), Span::new(start, self.prev_end));
        }
        Ok(expr)
    }

    /// The name after `.` or `?.`: an IdentifierName or a private name.
    fn member_name(&mut self) -> Result<MemberProp, SyntaxError> {
        if self.tok.kind == TokenKind::PrivateName {
            let name = Ident {
                name: super::intern(&mut self.names, self.tok.name(self.source)),
                span: self.tok_span(),
            };
            self.use_private(&name)?;
            self.bump()?;
            return Ok(MemberProp::Private(name));
        }
        if !self.at_identifier_name() {
            return Err(self.unexpected());
        }
        Ok(MemberProp::Ident(self.identifier_name()?))
    }

    /// `( args )`, spread allowed, a trailing comma allowed.
    pub(super) fn arguments(&mut self) -> Result<Vec<Spreadable>, SyntaxError> {
        self.expect(Punct::LParen)?;
        let mut args = Vec::new();
        self.with_context(
            |cx| cx.in_ = true,
            |p| {
                while !p.at(Punct::RParen) {
                    if p.eat(Punct::Ellipsis)? {
                        args.push(Spreadable::Spread(p.assignment()?));
                    } else {
                        args.push(Spreadable::Expr(p.assignment()?));
                    }
                    if !p.eat(Punct::Comma)? {
                        break;
                    }
                }
                Ok(())
            },
        )?;
        self.expect(Punct::RParen)?;
        Ok(args)
    }

    /// `new X(args)`, `new X`, and `new.target`.
    fn new_expression(&mut self) -> Result<ExprId, SyntaxError> {
        self.enter()?;
        let result = self.new_inner();
        self.leave();
        result
    }

    fn new_inner(&mut self) -> Result<ExprId, SyntaxError> {
        let start = self.tok.start;
        let new_tok = self.advance()?;
        if self.eat(Punct::Dot)? {
            if self.at_contextual("target") {
                if !self.cx.new_target {
                    return Err(self.error(
                        "new.target expression is not allowed here",
                        Span::new(start, self.tok.end),
                    ));
                }
                self.bump()?;
                return Ok(self
                    .ast
                    .add_expr(Expr::NewTarget, Span::new(start, self.prev_end)));
            }
            return Err(self.unexpected());
        }
        let _ = new_tok;
        let callee_start = self.tok.start;
        let callee = if self.at_keyword(Keyword::New) {
            self.new_expression()?
        } else if self.at_keyword(Keyword::Super) {
            self.super_expression()?
        } else if self.at_keyword(Keyword::Import) {
            let next = self.lookahead()?;
            if next.is(Punct::LParen) {
                return Err(self.error("Cannot use new with import", self.tok_span()));
            }
            self.import_expression()?
        } else {
            let mut cover = Cover::default();
            let e = self.primary(&mut cover)?;
            self.check_cover(&cover)?;
            if self.is_bare_arrow(e) {
                let span = self.ast.expr(e).span;
                return Err(self.error("Malformed arrow function parameter list", span));
            }
            e
        };
        let callee = self.subscripts(callee, callee_start, false)?;
        if self.at(Punct::QuestionDot) {
            return Err(self.error(
                "Invalid optional chain from new expression",
                self.tok_span(),
            ));
        }
        let args = if self.at(Punct::LParen) {
            self.arguments()?
        } else {
            Vec::new()
        };
        Ok(self
            .ast
            .add_expr(Expr::New { callee, args }, Span::new(start, self.prev_end)))
    }

    /// `super(...)`, `super.x` or `super[x]`, where they are allowed.
    fn super_expression(&mut self) -> Result<ExprId, SyntaxError> {
        let start = self.tok.start;
        let span = self.tok_span();
        self.bump()?;
        match self.tok.kind {
            TokenKind::Punct(Punct::LParen) => {
                if !self.cx.super_call {
                    return Err(self.error("'super' keyword unexpected here", span));
                }
                let args = self.arguments()?;
                Ok(self
                    .ast
                    .add_expr(Expr::SuperCall(args), Span::new(start, self.prev_end)))
            }
            TokenKind::Punct(Punct::Dot | Punct::LBracket) => {
                if !self.cx.super_prop {
                    return Err(self.error("'super' keyword unexpected here", span));
                }
                let prop = if self.eat(Punct::Dot)? {
                    if self.tok.kind == TokenKind::PrivateName {
                        return Err(self.error("Unexpected private field", self.tok_span()));
                    }
                    if !self.at_identifier_name() {
                        return Err(self.unexpected());
                    }
                    MemberProp::Ident(self.identifier_name()?)
                } else {
                    self.bump()?;
                    let index = self.with_context(|cx| cx.in_ = true, |p| p.expression())?;
                    self.expect(Punct::RBracket)?;
                    MemberProp::Computed(index)
                };
                Ok(self
                    .ast
                    .add_expr(Expr::SuperMember(prop), Span::new(start, self.prev_end)))
            }
            _ => Err(self.error("'super' keyword unexpected here", span)),
        }
    }

    /// `import(...)` or `import.meta`.
    fn import_expression(&mut self) -> Result<ExprId, SyntaxError> {
        let start = self.tok.start;
        let span = self.tok_span();
        self.bump()?;
        if self.eat(Punct::Dot)? {
            if self.at_contextual("meta") {
                if !self.module {
                    return Err(self.error(
                        "Cannot use 'import.meta' outside a module",
                        Span::new(start, self.tok.end),
                    ));
                }
                self.bump()?;
                return Ok(self
                    .ast
                    .add_expr(Expr::ImportMeta, Span::new(start, self.prev_end)));
            }
            return Err(self.unexpected());
        }
        if !self.at(Punct::LParen) {
            return Err(self.error("Cannot use import statement outside a module", span));
        }
        self.bump()?;
        let (source, options) = self.with_context(
            |cx| cx.in_ = true,
            |p| {
                let source = p.assignment()?;
                let mut options = None;
                if p.eat(Punct::Comma)? && !p.at(Punct::RParen) {
                    options = Some(p.assignment()?);
                    p.eat(Punct::Comma)?;
                }
                Ok((source, options))
            },
        )?;
        self.expect(Punct::RParen)?;
        Ok(self.ast.add_expr(
            Expr::Import { source, options },
            Span::new(start, self.prev_end),
        ))
    }

    /// A PrimaryExpression (section 13.2), arrows included.
    pub(super) fn primary(&mut self, cover: &mut Cover) -> Result<ExprId, SyntaxError> {
        let start = self.tok.start;
        let span = self.tok_span();
        match self.tok.kind {
            TokenKind::Identifier => self.primary_identifier(cover),
            TokenKind::Keyword(Keyword::This) => {
                self.bump()?;
                Ok(self.ast.add_expr(Expr::This, span))
            }
            TokenKind::Keyword(Keyword::Null) => {
                self.bump()?;
                Ok(self.ast.add_expr(Expr::Null, span))
            }
            TokenKind::Keyword(Keyword::True) => {
                self.bump()?;
                Ok(self.ast.add_expr(Expr::Bool(true), span))
            }
            TokenKind::Keyword(Keyword::False) => {
                self.bump()?;
                Ok(self.ast.add_expr(Expr::Bool(false), span))
            }
            TokenKind::Keyword(Keyword::Function) => self.function_expression(false, start),
            TokenKind::Keyword(Keyword::Class) => self.class_expression(),
            TokenKind::Number
            | TokenKind::BigInt
            | TokenKind::String
            | TokenKind::Template { .. }
            | TokenKind::Punct(Punct::Slash | Punct::SlashEq) => self.literal(),
            TokenKind::Punct(Punct::LParen) => self.paren_or_arrow(cover),
            TokenKind::Punct(Punct::LBracket) => self.array_literal(cover),
            TokenKind::Punct(Punct::LBrace) => self.object_literal(cover),
            TokenKind::PrivateName => Err(self.unexpected()),
            _ => Err(self.unexpected()),
        }
    }

    /// A literal: a number, BigInt, string, template or regular
    /// expression. Apart from `primary` so that the tokens it holds take
    /// no room in the frame every nested expression goes through.
    #[inline(never)]
    fn literal(&mut self) -> Result<ExprId, SyntaxError> {
        let start = self.tok.start;
        let span = self.tok_span();
        match self.tok.kind {
            TokenKind::Number => {
                let tok = self.advance()?;
                self.check_strict_token(&tok)?;
                let value = match tok.value {
                    TokenValue::Number(n) => n,
                    _ => 0.0,
                };
                Ok(self.ast.add_expr(Expr::Number(value), span))
            }
            TokenKind::BigInt => {
                let tok = self.advance()?;
                let (radix, digits) = match tok.value {
                    TokenValue::BigInt { radix, digits } => (radix, digits),
                    _ => (10, "0".into()),
                };
                Ok(self.ast.add_expr(Expr::BigInt { radix, digits }, span))
            }
            TokenKind::String => {
                let tok = self.advance()?;
                self.check_strict_token(&tok)?;
                Ok(self
                    .ast
                    .add_expr(Expr::String(Self::string_value(&tok)), span))
            }
            TokenKind::Template { .. } => {
                let (quasis, exprs) = self.template(false)?;
                Ok(self.ast.add_expr(
                    Expr::Template { quasis, exprs },
                    Span::new(start, self.prev_end),
                ))
            }
            TokenKind::Punct(Punct::Slash | Punct::SlashEq) => {
                let slash = self.tok.clone();
                let tok = self.lexer.rescan_regexp(&slash)?;
                self.rescanned(tok);
                let tok = self.advance()?;
                let (body, flags) = match tok.value {
                    TokenValue::RegExp(r) => (r.body, r.flags),
                    _ => (Box::default(), Box::default()),
                };
                crate::regexp::check_literal(&body, &flags, self.stack)
                    .map_err(|m| self.error(m, Span::new(tok.start, tok.end)))?;
                Ok(self
                    .ast
                    .add_expr(Expr::RegExp { body, flags }, Span::new(tok.start, tok.end)))
            }
            _ => Err(self.unexpected()),
        }
    }

    /// An identifier in expression position: a reference, `async`'s
    /// forms, or an arrow's single parameter.
    fn primary_identifier(&mut self, cover: &mut Cover) -> Result<ExprId, SyntaxError> {
        let start = self.tok.start;
        if self.at_contextual("async") {
            let next = self.lookahead()?;
            if !next.newline_before {
                if next.kind == TokenKind::Keyword(Keyword::Function) {
                    self.bump()?;
                    return self.function_expression(true, start);
                }
                if next.kind == TokenKind::Identifier {
                    // `async x => ...`
                    let (_, after) = self.peek2()?;
                    if after.is(Punct::Arrow) && !after.newline_before {
                        self.bump()?;
                        return self.identifier_arrow(start, true);
                    }
                }
                if next.is(Punct::LParen) {
                    return self.async_call_or_arrow(cover);
                }
            }
        }
        // Every identifier looks at the token after it: kept, not lexed
        // again by the move past the identifier.
        let next = self.peek()?;
        if next.is(Punct::Arrow) {
            if next.newline_before {
                let next = next.clone();
                return Err(self.unexpected_token(&next));
            }
            return self.identifier_arrow(start, false);
        }
        let ident = self.identifier(false)?;
        let span = ident.span;
        Ok(self.ast.add_expr(Expr::Ident(ident), span))
    }

    /// A template literal from its first part; `tagged` lets an escape be
    /// invalid (its cooked value is then undefined).
    fn template(&mut self, tagged: bool) -> Result<(Vec<Quasi>, Vec<ExprId>), SyntaxError> {
        let mut quasis = Vec::new();
        let mut exprs = Vec::new();
        let mut tok = self.tok.clone();
        loop {
            if !tagged && let Some(error) = &tok.invalid_escape {
                return Err(SyntaxError::clone(error));
            }
            let (cooked, raw) = match &tok.value {
                TokenValue::Template(t) => (t.cooked.clone(), t.raw.clone()),
                _ => (None, Box::default()),
            };
            quasis.push(Quasi {
                cooked,
                raw,
                span: Span::new(tok.start, tok.end),
            });
            let tail = matches!(tok.kind, TokenKind::Template { tail: true, .. });
            self.bump()?;
            if tail {
                break;
            }
            let e = self.with_context(|cx| cx.in_ = true, |p| p.expression())?;
            exprs.push(e);
            if !self.at(Punct::RBrace) {
                return Err(self.unexpected());
            }
            let brace = self.tok.clone();
            tok = self.lexer.rescan_template(&brace)?;
            self.rescanned(tok.clone());
        }
        Ok((quasis, exprs))
    }

    /// `[ elements ]`; what only a pattern allows goes into `cover`.
    fn array_literal(&mut self, cover: &mut Cover) -> Result<ExprId, SyntaxError> {
        self.enter()?;
        let result = self.array_inner(cover);
        self.leave();
        result
    }

    fn array_inner(&mut self, cover: &mut Cover) -> Result<ExprId, SyntaxError> {
        let start = self.tok.start;
        self.bump()?;
        let mut elems = Vec::new();
        self.with_context(
            |cx| cx.in_ = true,
            |p| {
                loop {
                    if p.at(Punct::RBracket) {
                        break;
                    }
                    if p.eat(Punct::Comma)? {
                        elems.push(None);
                        continue;
                    }
                    let elem = if p.eat(Punct::Ellipsis)? {
                        let e = p.assignment_cover(cover)?;
                        if !p.at(Punct::RBracket) {
                            p.rest_not_last.insert(e);
                        }
                        Spreadable::Spread(e)
                    } else {
                        Spreadable::Expr(p.assignment_cover(cover)?)
                    };
                    elems.push(Some(elem));
                    if !p.at(Punct::RBracket) {
                        p.expect(Punct::Comma)?;
                    }
                }
                Ok(())
            },
        )?;
        self.expect(Punct::RBracket)?;
        Ok(self
            .ast
            .add_expr(Expr::Array(elems), Span::new(start, self.prev_end)))
    }

    /// `{ properties }`; what only a pattern allows goes into `cover`.
    fn object_literal(&mut self, cover: &mut Cover) -> Result<ExprId, SyntaxError> {
        self.enter()?;
        let result = self.object_inner(cover);
        self.leave();
        result
    }

    fn object_inner(&mut self, cover: &mut Cover) -> Result<ExprId, SyntaxError> {
        let start = self.tok.start;
        self.bump()?;
        let mut props = Vec::new();
        let mut proto: Option<Span> = None;
        self.with_context(
            |cx| cx.in_ = true,
            |p| {
                while !p.at(Punct::RBrace) {
                    let prop = p.property(cover)?;
                    if prop.kind == PropKind::Spread && !p.at(Punct::RBrace) {
                        p.rest_not_last.insert(prop.value);
                    }
                    if prop.kind == PropKind::Init && is_proto_key(&prop.key) {
                        if proto.is_some() && cover.double_proto.is_none() {
                            cover.double_proto = Some(prop.span);
                        }
                        proto = Some(prop.span);
                    }
                    props.push(prop);
                    if !p.at(Punct::RBrace) {
                        p.expect(Punct::Comma)?;
                    }
                }
                Ok(())
            },
        )?;
        self.expect(Punct::RBrace)?;
        Ok(self
            .ast
            .add_expr(Expr::Object(props), Span::new(start, self.prev_end)))
    }

    /// One property of an object literal (section 13.2.5).
    fn property(&mut self, cover: &mut Cover) -> Result<Prop, SyntaxError> {
        let start = self.tok.start;
        if self.eat(Punct::Ellipsis)? {
            let value = self.assignment_cover(cover)?;
            return Ok(Prop {
                key: PropKey::Ident(Ident {
                    name: "".into(),
                    span: Span::new(start, start),
                }),
                kind: PropKind::Spread,
                value,
                span: Span::new(start, self.prev_end),
            });
        }
        let (is_async, is_generator, accessor) = self.method_modifiers()?;
        let key_tok = self.tok.clone();
        let key = self.property_key(false)?;
        if is_async || is_generator || accessor.is_some() {
            let kind = match accessor {
                Some(true) => FuncKind::Getter,
                Some(false) => FuncKind::Setter,
                None => FuncKind::Method,
            };
            let value = self.method(kind, is_async, is_generator, start, false)?;
            let prop_kind = match kind {
                FuncKind::Getter => PropKind::Getter,
                FuncKind::Setter => PropKind::Setter,
                _ => PropKind::Method,
            };
            return Ok(Prop {
                key,
                kind: prop_kind,
                value,
                span: Span::new(start, self.prev_end),
            });
        }
        match self.tok.kind {
            TokenKind::Punct(Punct::Colon) => {
                self.bump()?;
                let value = self.assignment_cover(cover)?;
                Ok(Prop {
                    key,
                    kind: PropKind::Init,
                    value,
                    span: Span::new(start, self.prev_end),
                })
            }
            TokenKind::Punct(Punct::LParen) => {
                let value = self.method(FuncKind::Method, false, false, start, false)?;
                Ok(Prop {
                    key,
                    kind: PropKind::Method,
                    value,
                    span: Span::new(start, self.prev_end),
                })
            }
            _ => {
                // Shorthand: an IdentifierReference, maybe with the cover
                // initializer only a pattern allows.
                let PropKey::Ident(ident) = &key else {
                    return Err(self.unexpected());
                };
                if key_tok.kind != TokenKind::Identifier {
                    return Err(self.unexpected_token(&key_tok));
                }
                self.check_identifier(&ident.name, ident.span, false)?;
                if &*ident.name == "await" && self.await_ident_at.is_none() {
                    self.await_ident_at = Some(ident.span);
                }
                let reference = self.ast.add_expr(Expr::Ident(ident.clone()), ident.span);
                let value = if self.at(Punct::Eq) {
                    if cover.init.is_none() {
                        cover.init = Some(self.tok_span());
                    }
                    self.bump()?;
                    let default = self.assignment()?;
                    self.ast.add_expr(
                        Expr::Assign {
                            op: AssignOp::Assign,
                            target: AssignTarget::Simple(reference),
                            value: default,
                        },
                        Span::new(start, self.prev_end),
                    )
                } else {
                    reference
                };
                Ok(Prop {
                    key,
                    kind: PropKind::Shorthand,
                    value,
                    span: Span::new(start, self.prev_end),
                })
            }
        }
    }

    /// `async`, `*`, `get` and `set` before a method's key, when they are
    /// modifiers rather than the key: (async, generator, accessor), the
    /// accessor `Some(true)` for a getter.
    pub(super) fn method_modifiers(&mut self) -> Result<(bool, bool, Option<bool>), SyntaxError> {
        let mut is_async = false;
        let mut accessor = None;
        if self.tok.kind == TokenKind::Identifier && !self.tok.escaped {
            let name = self.tok_text();
            if matches!(name, "async" | "get" | "set") {
                let next = self.lookahead()?;
                let is_modifier = !matches!(
                    next.kind,
                    TokenKind::Punct(
                        Punct::LParen
                            | Punct::Comma
                            | Punct::Colon
                            | Punct::RBrace
                            | Punct::Eq
                            | Punct::Semicolon
                    ) | TokenKind::Eof
                ) && !(name == "async" && next.newline_before)
                    // An accessor is no generator: `get` then `*` is a field
                    // named get, ended by an inserted `;`.
                    && !(name != "async" && next.is(Punct::Star));
                if is_modifier {
                    match name {
                        "async" => is_async = true,
                        "get" => accessor = Some(true),
                        _ => accessor = Some(false),
                    }
                    self.bump()?;
                }
            }
        }
        let is_generator = accessor.is_none() && self.eat(Punct::Star)?;
        Ok((is_async, is_generator, accessor))
    }

    /// A property's key: a name, string, number or `[computed]`; and in
    /// a class (`class_body`), a private name.
    pub(super) fn property_key(&mut self, class_body: bool) -> Result<PropKey, SyntaxError> {
        let span = self.tok_span();
        match self.tok.kind {
            TokenKind::Identifier | TokenKind::Keyword(_) => {
                Ok(PropKey::Ident(self.identifier_name()?))
            }
            TokenKind::String => {
                let tok = self.advance()?;
                self.check_strict_token(&tok)?;
                Ok(PropKey::String(Self::string_value(&tok)))
            }
            TokenKind::Number => {
                let tok = self.advance()?;
                self.check_strict_token(&tok)?;
                Ok(PropKey::Number(match tok.value {
                    TokenValue::Number(n) => n,
                    _ => 0.0,
                }))
            }
            TokenKind::BigInt => {
                let tok = self.advance()?;
                let (radix, digits) = match tok.value {
                    TokenValue::BigInt { radix, digits } => (radix, digits),
                    _ => (10, "0".into()),
                };
                Ok(PropKey::BigInt { radix, digits })
            }
            TokenKind::Punct(Punct::LBracket) => {
                self.bump()?;
                let e = self.with_context(|cx| cx.in_ = true, |p| p.assignment())?;
                self.expect(Punct::RBracket)?;
                Ok(PropKey::Computed(e))
            }
            TokenKind::PrivateName if class_body => {
                let name = super::intern(&mut self.names, self.tok.name(self.source));
                self.bump()?;
                Ok(PropKey::Private(Ident { name, span }))
            }
            _ => Err(self.unexpected()),
        }
    }

    /// `( ... )`: a parenthesized expression, or an arrow's parameters.
    fn paren_or_arrow(&mut self, cover: &mut Cover) -> Result<ExprId, SyntaxError> {
        self.enter()?;
        let result = self.paren_inner(cover);
        self.leave();
        result
    }

    fn paren_inner(&mut self, outer: &mut Cover) -> Result<ExprId, SyntaxError> {
        let start = self.tok.start;
        self.bump()?;
        let saved = (self.yield_at, self.await_at, self.await_ident_at);
        self.yield_at = None;
        self.await_at = None;
        self.await_ident_at = None;
        let mut cover = Cover::default();
        let mut items = Vec::new();
        let mut rest = None;
        let mut trailing_comma = None;
        let parsed: Result<(), SyntaxError> = self.with_context(
            |cx| cx.in_ = true,
            |p| {
                while !p.at(Punct::RParen) {
                    if p.at(Punct::Ellipsis) {
                        let span = p.tok_span();
                        p.bump()?;
                        let target = p.binding_target()?;
                        rest = Some((target, span));
                        if !p.at(Punct::RParen) {
                            return Err(
                                p.error("Rest parameter must be last formal parameter", span)
                            );
                        }
                        break;
                    }
                    items.push(p.assignment_cover(&mut cover)?);
                    if p.at(Punct::Comma) {
                        let comma = p.tok_span();
                        p.bump()?;
                        if p.at(Punct::RParen) {
                            trailing_comma = Some(comma);
                        }
                    } else {
                        break;
                    }
                }
                Ok(())
            },
        );
        parsed?;
        if !self.at(Punct::RParen) {
            return Err(self.unexpected());
        }
        let close = self.advance()?;
        if self.at(Punct::Arrow) && !self.tok.newline_before {
            let inner = (self.yield_at, self.await_at, self.await_ident_at);
            self.yield_at = saved.0.or(inner.0);
            self.await_at = saved.1.or(inner.1);
            self.await_ident_at = saved.2.or(inner.2);
            if let Some(span) = inner.0 {
                return Err(self.error("Yield expression not allowed in formal parameter", span));
            }
            if let Some(span) = inner.1 {
                return Err(self.error(
                    "Illegal await-expression in formal parameters of async function",
                    span,
                ));
            }
            return self.cover_arrow(start, items, rest, false, None);
        }
        if self.at(Punct::Arrow) {
            return Err(self.unexpected());
        }
        self.yield_at = saved.0.or(self.yield_at);
        self.await_at = saved.1.or(self.await_at);
        self.await_ident_at = saved.2.or(self.await_ident_at);
        if let Some((_, span)) = rest {
            return Err(self.error("Unexpected token '...'", span));
        }
        if trailing_comma.is_some() || items.is_empty() {
            return Err(self.unexpected_token(&close));
        }
        self.check_cover(&cover)?;
        let _ = outer;
        let inner = if items.len() == 1 {
            items.pop().unwrap_or(ExprId(0))
        } else {
            let first = items.first().copied().unwrap_or(ExprId(0));
            let last = items.last().copied().unwrap_or(ExprId(0));
            let span = self.ast.expr(first).span.to(self.ast.expr(last).span);
            self.ast.add_expr(Expr::Sequence(items), span)
        };
        Ok(self
            .ast
            .add_expr(Expr::Paren(inner), Span::new(start, self.prev_end)))
    }

    /// `async(...)`: a call of a function named `async`, or an async
    /// arrow's parameters (section 15.9's CoverCallExpressionAndAsyncArrowHead).
    fn async_call_or_arrow(&mut self, cover: &mut Cover) -> Result<ExprId, SyntaxError> {
        let start = self.tok.start;
        let async_ident = self.identifier(false)?;
        let callee_span = async_ident.span;
        self.enter()?;
        let saved = (self.yield_at, self.await_at, self.await_ident_at);
        self.yield_at = None;
        self.await_at = None;
        self.await_ident_at = None;
        self.bump()?; // (
        let mut items = Vec::new();
        let mut spreads = Vec::new();
        let mut rest = None;
        let mut rest_comma = false;
        let mut inner_cover = Cover::default();
        let parsed: Result<(), SyntaxError> = self.with_context(
            |cx| cx.in_ = true,
            |p| {
                while !p.at(Punct::RParen) {
                    if p.at(Punct::Ellipsis) {
                        let span = p.tok_span();
                        p.bump()?;
                        let e = p.assignment_cover(&mut inner_cover)?;
                        spreads.push(items.len());
                        rest = Some(span);
                        items.push(e);
                        if p.at(Punct::Comma) {
                            rest_comma = true;
                        }
                    } else {
                        items.push(p.assignment_cover(&mut inner_cover)?);
                    }
                    if !p.eat(Punct::Comma)? {
                        break;
                    }
                }
                Ok(())
            },
        );
        let parsed = parsed.and_then(|()| self.expect(Punct::RParen).map(|_| ()));
        self.leave();
        parsed?;
        if self.at(Punct::Arrow) && !self.tok.newline_before {
            let inner = (self.yield_at, self.await_at, self.await_ident_at);
            self.yield_at = saved.0.or(inner.0);
            self.await_at = saved.1.or(inner.1);
            self.await_ident_at = saved.2;
            if let Some(span) = inner.0 {
                return Err(self.error("Yield expression not allowed in formal parameter", span));
            }
            if let Some(span) = inner.1.or(inner.2) {
                return Err(self.error(
                    "Illegal await-expression in formal parameters of async function",
                    span,
                ));
            }
            // A spread must be the last item, and becomes the rest.
            let mut rest_target = None;
            if let Some(&index) = spreads.first() {
                if spreads.len() > 1 || index + 1 != items.len() || rest_comma {
                    let span = rest.unwrap_or_default();
                    return Err(self.error("Rest parameter must be last formal parameter", span));
                }
                if let Some(e) = items.pop() {
                    let target = self.binding_pattern_of(e)?;
                    rest_target = Some((target, rest.unwrap_or_default()));
                }
            }
            return self.cover_arrow(start, items, rest_target, true, None);
        }
        self.yield_at = saved.0.or(self.yield_at);
        self.await_at = saved.1.or(self.await_at);
        self.await_ident_at = saved.2.or(self.await_ident_at);
        self.check_cover(&inner_cover)?;
        let _ = cover;
        let callee = self.ast.add_expr(Expr::Ident(async_ident), callee_span);
        let args = items
            .into_iter()
            .enumerate()
            .map(|(i, e)| {
                if spreads.contains(&i) {
                    Spreadable::Spread(e)
                } else {
                    Spreadable::Expr(e)
                }
            })
            .collect();
        Ok(self.ast.add_expr(
            Expr::Call {
                callee,
                args,
                optional: false,
            },
            Span::new(start, self.prev_end),
        ))
    }

    /// The private name `name` is used here: it must be declared by an
    /// enclosing class, which the class's end checks.
    pub(super) fn use_private(&mut self, name: &Ident) -> Result<(), SyntaxError> {
        match self.classes.last_mut() {
            Some(class) => {
                class.used.push(name.clone());
                Ok(())
            }
            None => Err(self.error(
                format!(
                    "Private field '#{}' must be declared in an enclosing class",
                    name.name
                ),
                name.span,
            )),
        }
    }
}

fn assign_op(p: Punct) -> Option<AssignOp> {
    use BinaryOp as B;
    Some(match p {
        Punct::Eq => AssignOp::Assign,
        Punct::PlusEq => AssignOp::Binary(B::Add),
        Punct::MinusEq => AssignOp::Binary(B::Sub),
        Punct::StarEq => AssignOp::Binary(B::Mul),
        Punct::SlashEq => AssignOp::Binary(B::Div),
        Punct::PercentEq => AssignOp::Binary(B::Mod),
        Punct::StarStarEq => AssignOp::Binary(B::Exp),
        Punct::ShlEq => AssignOp::Binary(B::Shl),
        Punct::ShrEq => AssignOp::Binary(B::Shr),
        Punct::UShrEq => AssignOp::Binary(B::UShr),
        Punct::AmpEq => AssignOp::Binary(B::BitAnd),
        Punct::PipeEq => AssignOp::Binary(B::BitOr),
        Punct::CaretEq => AssignOp::Binary(B::BitXor),
        Punct::AmpAmpEq => AssignOp::Logical(LogicalOp::And),
        Punct::PipePipeEq => AssignOp::Logical(LogicalOp::Or),
        Punct::QuestionQuestionEq => AssignOp::Logical(LogicalOp::Coalesce),
        _ => return None,
    })
}

/// `__proto__` as a key, by name or string, not computed (section
/// 13.2.5.1's duplicate rule).
fn is_proto_key(key: &PropKey) -> bool {
    match key {
        PropKey::Ident(i) => &*i.name == "__proto__",
        PropKey::String(s) => s.iter().copied().eq("__proto__".encode_utf16()),
        _ => false,
    }
}
