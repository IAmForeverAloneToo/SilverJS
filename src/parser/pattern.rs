//! Patterns (sections 8.6's BoundNames, 13.15.5 and 14.3.3): binding
//! patterns parsed as such, and assignment patterns and arrow
//! parameters turned from the expressions their cover grammars read.

use super::Parser;
use crate::SyntaxError;
use crate::ast::{
    AssignOp, AssignTarget, Expr, ExprId, Ident, Pat, PatElem, PatId, PatProp, PropKey, PropKind,
    Span, Spreadable,
};
use crate::lexer::{Punct, TokenKind};

const INVALID_TARGET: &str = "Invalid destructuring assignment target";
const REST_LAST: &str = "Rest element must be last element";

impl Parser<'_> {
    /// A BindingIdentifier or BindingPattern (section 14.3.3).
    pub(super) fn binding_target(&mut self) -> Result<PatId, SyntaxError> {
        match self.tok.kind {
            TokenKind::Identifier => {
                let ident = self.identifier(true)?;
                let span = ident.span;
                Ok(self.ast.add_pat(Pat::Ident(ident), span))
            }
            TokenKind::Punct(Punct::LBracket) => {
                self.enter()?;
                let result = self.array_binding();
                self.leave();
                result
            }
            TokenKind::Punct(Punct::LBrace) => {
                self.enter()?;
                let result = self.object_binding();
                self.leave();
                result
            }
            _ => Err(self.unexpected()),
        }
    }

    /// A binding element: a target and its default.
    pub(super) fn binding_element(&mut self) -> Result<PatElem, SyntaxError> {
        let target = self.binding_target()?;
        let default = if self.eat(Punct::Eq)? {
            Some(self.with_context(|cx| cx.in_ = true, |p| p.assignment())?)
        } else {
            None
        };
        Ok(PatElem { target, default })
    }

    fn array_binding(&mut self) -> Result<PatId, SyntaxError> {
        let start = self.tok.start;
        self.bump()?;
        let mut elems = Vec::new();
        let mut rest = None;
        loop {
            if self.at(Punct::RBracket) {
                break;
            }
            if self.eat(Punct::Comma)? {
                elems.push(None);
                continue;
            }
            if self.at(Punct::Ellipsis) {
                let span = self.tok_span();
                self.bump()?;
                rest = Some(self.binding_target()?);
                if !self.at(Punct::RBracket) {
                    return Err(self.error(REST_LAST, span));
                }
                break;
            }
            elems.push(Some(self.binding_element()?));
            if !self.at(Punct::RBracket) {
                self.expect(Punct::Comma)?;
            }
        }
        self.expect(Punct::RBracket)?;
        Ok(self
            .ast
            .add_pat(Pat::Array { elems, rest }, Span::new(start, self.prev_end)))
    }

    fn object_binding(&mut self) -> Result<PatId, SyntaxError> {
        let start = self.tok.start;
        self.bump()?;
        let mut props = Vec::new();
        let mut rest = None;
        while !self.at(Punct::RBrace) {
            let prop_start = self.tok.start;
            if self.at(Punct::Ellipsis) {
                let span = self.tok_span();
                self.bump()?;
                let ident = self.identifier(true)?;
                let ident_span = ident.span;
                rest = Some(self.ast.add_pat(Pat::Ident(ident), ident_span));
                if !self.at(Punct::RBrace) {
                    return Err(self.error(REST_LAST, span));
                }
                break;
            }
            let key_tok = self.tok.clone();
            let key = self.property_key(false)?;
            let (value, shorthand) = if self.eat(Punct::Colon)? {
                (self.binding_element()?, false)
            } else {
                // `{ name }` or `{ name = default }`: the key is the
                // binding, so it must be a BindingIdentifier.
                let PropKey::Ident(ident) = &key else {
                    return Err(self.unexpected());
                };
                if key_tok.kind != TokenKind::Identifier {
                    return Err(self.unexpected_token(&key_tok));
                }
                self.check_identifier(&ident.name, ident.span, true)?;
                let target = self.ast.add_pat(Pat::Ident(ident.clone()), ident.span);
                let default = if self.eat(Punct::Eq)? {
                    Some(self.with_context(|cx| cx.in_ = true, |p| p.assignment())?)
                } else {
                    None
                };
                (PatElem { target, default }, true)
            };
            props.push(PatProp {
                key,
                value,
                shorthand,
                span: Span::new(prop_start, self.prev_end),
            });
            if !self.at(Punct::RBrace) {
                self.expect(Punct::Comma)?;
            }
        }
        self.expect(Punct::RBrace)?;
        Ok(self
            .ast
            .add_pat(Pat::Object { props, rest }, Span::new(start, self.prev_end)))
    }

    /// The names a pattern binds, in order (section 8.6.2's BoundNames),
    /// found with a stack of its own.
    pub(super) fn bound_names(&self, pat: PatId) -> Vec<Ident> {
        let mut names = Vec::new();
        let mut stack = vec![pat];
        while let Some(p) = stack.pop() {
            match &self.ast.pat(p).kind {
                Pat::Ident(i) => names.push(i.clone()),
                Pat::Expr(_) => {}
                Pat::Object { props, rest } => {
                    if let Some(r) = rest {
                        stack.push(*r);
                    }
                    for prop in props.iter().rev() {
                        stack.push(prop.value.target);
                    }
                }
                Pat::Array { elems, rest } => {
                    if let Some(r) = rest {
                        stack.push(*r);
                    }
                    for elem in elems.iter().rev().flatten() {
                        stack.push(elem.target);
                    }
                }
            }
        }
        names
    }

    /// The target of `=` or of a `for-in` / `for-of` head: a simple target
    /// or, from an object or array literal, an assignment pattern.
    pub(super) fn assign_target_of(
        &mut self,
        e: ExprId,
        message: &'static str,
    ) -> Result<AssignTarget, SyntaxError> {
        match self.ast.expr(e).kind {
            Expr::Object(_) | Expr::Array(_) => {
                Ok(AssignTarget::Pattern(self.assign_pattern_of(e)?))
            }
            _ => self.simple_target(e, false, || message),
        }
    }

    /// An object or array literal read again as an assignment pattern
    /// (section 13.15.5). Recursion follows the literal's own nesting,
    /// which parsing it already bounded.
    fn assign_pattern_of(&mut self, e: ExprId) -> Result<PatId, SyntaxError> {
        let node = self.ast.expr(e).clone();
        let kind = match node.kind {
            Expr::Object(props) => {
                let mut out = Vec::new();
                let mut rest = None;
                let count = props.len();
                for (i, prop) in props.into_iter().enumerate() {
                    match prop.kind {
                        PropKind::Init => {
                            let value = self.assign_elem_of(prop.value)?;
                            out.push(PatProp {
                                key: prop.key,
                                value,
                                shorthand: false,
                                span: prop.span,
                            });
                        }
                        PropKind::Shorthand => {
                            let value = self.assign_elem_of(prop.value)?;
                            out.push(PatProp {
                                key: prop.key,
                                value,
                                shorthand: true,
                                span: prop.span,
                            });
                        }
                        PropKind::Spread => {
                            if i + 1 != count || self.rest_not_last.contains(&prop.value) {
                                return Err(self.error(REST_LAST, prop.span));
                            }
                            rest = Some(self.rest_target_of(prop.value, false)?);
                        }
                        PropKind::Method | PropKind::Getter | PropKind::Setter => {
                            return Err(self.error(INVALID_TARGET, prop.span));
                        }
                    }
                }
                Pat::Object { props: out, rest }
            }
            Expr::Array(elems) => {
                let mut out = Vec::new();
                let mut rest = None;
                let count = elems.len();
                for (i, elem) in elems.into_iter().enumerate() {
                    match elem {
                        None => out.push(None),
                        Some(Spreadable::Expr(x)) => out.push(Some(self.assign_elem_of(x)?)),
                        Some(Spreadable::Spread(x)) => {
                            if i + 1 != count || self.rest_not_last.contains(&x) {
                                return Err(self.error(REST_LAST, self.ast.expr(x).span));
                            }
                            rest = Some(self.rest_target_of(x, true)?);
                        }
                    }
                }
                Pat::Array { elems: out, rest }
            }
            _ => return Err(self.error(INVALID_TARGET, node.span)),
        };
        Ok(self.ast.add_pat(kind, node.span))
    }

    /// A rest element's target: a simple target, or in an array a
    /// nested pattern; never with a default.
    fn rest_target_of(&mut self, e: ExprId, array: bool) -> Result<PatId, SyntaxError> {
        let span = self.ast.expr(e).span;
        match self.ast.expr(e).kind {
            Expr::Assign { .. } => {
                Err(self.error("Rest elements cannot have a default value", span))
            }
            Expr::Object(_) | Expr::Array(_) if array => self.assign_pattern_of(e),
            Expr::Object(_) | Expr::Array(_) => Err(self.error(
                "`...` must be followed by an assignable reference in assignment contexts",
                span,
            )),
            _ => self.assign_leaf_of(e),
        }
    }

    /// An element of an assignment pattern, `target = default` or
    /// `target`.
    fn assign_elem_of(&mut self, e: ExprId) -> Result<PatElem, SyntaxError> {
        if let Expr::Assign {
            op: AssignOp::Assign,
            target,
            value,
        } = self.ast.expr(e).kind
        {
            let target = match target {
                AssignTarget::Pattern(p) => p,
                AssignTarget::Simple(x) => self.assign_leaf_of(x)?,
            };
            return Ok(PatElem {
                target,
                default: Some(value),
            });
        }
        Ok(PatElem {
            target: self.assign_leaf_of(e)?,
            default: None,
        })
    }

    /// A DestructuringAssignmentTarget: a nested pattern or a simple
    /// target (no call: section 13.15.5.1).
    fn assign_leaf_of(&mut self, e: ExprId) -> Result<PatId, SyntaxError> {
        let node = self.ast.expr(e);
        let span = node.span;
        match &node.kind {
            Expr::Object(_) | Expr::Array(_) => self.assign_pattern_of(e),
            Expr::Ident(ident) => {
                let ident = ident.clone();
                self.simple_target(e, true, || INVALID_TARGET)?;
                Ok(self.ast.add_pat(Pat::Ident(ident), span))
            }
            _ => {
                self.simple_target(e, true, || INVALID_TARGET)?;
                Ok(self.ast.add_pat(Pat::Expr(e), span))
            }
        }
    }

    /// An arrow's parameter read from the cover expression: a binding
    /// target and its default.
    pub(super) fn param_of(&mut self, e: ExprId) -> Result<PatElem, SyntaxError> {
        if let Expr::Assign {
            op: AssignOp::Assign,
            target,
            value,
        } = self.ast.expr(e).kind
        {
            let target = match target {
                AssignTarget::Pattern(p) => {
                    self.validate_binding(p)?;
                    p
                }
                AssignTarget::Simple(x) => self.binding_pattern_of(x)?,
            };
            return Ok(PatElem {
                target,
                default: Some(value),
            });
        }
        Ok(PatElem {
            target: self.binding_pattern_of(e)?,
            default: None,
        })
    }

    /// An expression read again as a binding pattern: an arrow's
    /// parameter.
    pub(super) fn binding_pattern_of(&mut self, e: ExprId) -> Result<PatId, SyntaxError> {
        let node = self.ast.expr(e);
        let span = node.span;
        match &node.kind {
            Expr::Ident(ident) => {
                let ident = ident.clone();
                self.check_identifier(&ident.name, ident.span, true)?;
                Ok(self.ast.add_pat(Pat::Ident(ident), span))
            }
            Expr::Object(_) | Expr::Array(_) => {
                let p = self.assign_pattern_of(e)?;
                self.validate_binding(p)?;
                Ok(p)
            }
            _ => Err(self.error("Invalid destructuring assignment target", span)),
        }
    }

    /// Holds a pattern made as an assignment pattern to a binding
    /// pattern's rules: every leaf a name that may be bound here.
    pub(super) fn validate_binding(&self, pat: PatId) -> Result<(), SyntaxError> {
        let mut stack = vec![pat];
        while let Some(p) = stack.pop() {
            let node = self.ast.pat(p);
            match &node.kind {
                Pat::Ident(i) => self.check_identifier(&i.name, i.span, true)?,
                Pat::Expr(e) => {
                    // A name that was parenthesized, or a member access.
                    return Err(self.error(INVALID_TARGET, self.ast.expr(*e).span));
                }
                Pat::Object { props, rest } => {
                    if let Some(r) = rest {
                        if !matches!(self.ast.pat(*r).kind, Pat::Ident(_)) {
                            return Err(self.error(INVALID_TARGET, self.ast.pat(*r).span));
                        }
                        stack.push(*r);
                    }
                    stack.extend(props.iter().map(|p| p.value.target));
                }
                Pat::Array { elems, rest } => {
                    stack.extend(rest.iter().copied());
                    stack.extend(elems.iter().flatten().map(|e| e.target));
                }
            }
        }
        Ok(())
    }
}
