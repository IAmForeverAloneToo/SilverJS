//! The parser (ECMA-262 sections 13 to 16) and its early errors
//! (section 17's SyntaxErrors, found while parsing): a script or a
//! module into a [`Program`].
//!
//! Recursive descent, one token of lookahead held and more read by
//! cloning the lexer. Binary operators are parsed by precedence in a
//! loop, and left-deep chains (operators, member accesses, calls) in
//! loops, so the Rust stack grows only with real nesting (brackets,
//! functions, right-associative operators), which [`MAX_DEPTH`] bounds:
//! past it the source is refused with a `SyntaxError`, never a stack
//! overflow.
//!
//! Cover grammars (section 13.2.5's CoverInitializedName, section
//! 13.2.1's CoverParenthesizedExpressionAndArrowParameterList, section
//! 15.8's CoverCallExpressionAndAsyncArrowHead) are parsed as
//! expressions and turned into patterns or parameters when a `=` or
//! `=>` says they are; what only one reading allows is remembered by
//! position in a [`Cover`] until the reading is known.
//!
//! Declarations are checked against each other as they are parsed, in a
//! stack of scopes ([`scope`]): a name declared twice where the
//! standard forbids it, a `var` hoisted across a `let` of its name.

mod expr;
mod func;
mod module;
mod pattern;
mod scope;
mod stmt;

use std::collections::{HashMap, HashSet};

use crate::SyntaxError;
use crate::ast::{Ast, ExprId, Ident, Program, Span, keyword_text};
use crate::lexer::{Keyword, Lexer, Punct, Token, TokenKind, TokenValue};
pub use crate::stack::DEFAULT_STACK_BUDGET;
use crate::stack::StackGuard;

use scope::Scopes;

/// How deep brackets, functions and right-associative operators may
/// nest before the source is refused ("Maximum nesting depth exceeded"),
/// so that parsing never overflows the Rust stack.
pub const MAX_DEPTH: u32 = 1000;

/// What to parse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Goal {
    #[default]
    Script,
    Module,
}

/// Parses `source` as a script or a module, within
/// [`DEFAULT_STACK_BUDGET`] of the caller's stack.
pub fn parse(source: &str, goal: Goal) -> Result<Program, SyntaxError> {
    parse_with_stack(source, goal, DEFAULT_STACK_BUDGET)
}

/// Parses with `stack_budget` bytes of the caller's stack: a source that
/// nests deeper is refused ("Maximum call stack size exceeded") before
/// the stack overflows. An embedder on a thread with a larger stack may
/// allow more.
pub fn parse_with_stack(
    source: &str,
    goal: Goal,
    stack_budget: usize,
) -> Result<Program, SyntaxError> {
    parse_with_stats(source, goal, stack_budget).0
}

/// What a parse used: the deepest nesting it reached (levels of
/// brackets, functions, statements and right-associative operators) and
/// the most stack it took past its entry, in bytes. An embedder sizes its
/// stack budget from these.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ParseStats {
    pub max_depth: u32,
    pub peak_stack: usize,
}

/// Parses as [`parse_with_stack`] does, and says what the parse used.
pub fn parse_with_stats(
    source: &str,
    goal: Goal,
    stack_budget: usize,
) -> (Result<Program, SyntaxError>, ParseStats) {
    let mut parser = match Parser::new(source, goal) {
        Ok(parser) => parser,
        Err(e) => return (Err(e), ParseStats::default()),
    };
    parser.stack = StackGuard::new(stack_budget);
    let result = parser.program();
    (result, parser.stats)
}

/// Positions where a cover grammar was read the way that only one of
/// its readings allows, kept until the reading is known.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct Cover {
    /// `{a = 1}`: only a pattern may have it.
    pub init: Option<Span>,
    /// `__proto__` twice in one object literal: only a pattern may.
    pub double_proto: Option<Span>,
}

/// What the code being parsed may use, set by the functions and classes
/// around it.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Context {
    pub strict: bool,
    /// In a generator: `yield` is an operator.
    pub yield_: bool,
    /// In an async function or a module's top level: `await` is one.
    pub await_: bool,
    /// In a function's body, not a static block: `return` is allowed.
    pub return_: bool,
    /// `super(...)` is allowed: a derived class's constructor.
    pub super_call: bool,
    /// `super.x` is allowed: a method, accessor, field or static block.
    pub super_prop: bool,
    /// `new.target` is allowed: any function but an arrow at the top.
    pub new_target: bool,
    /// In a class field's initializer or a static block: `arguments` is
    /// refused.
    pub class_field: bool,
    /// In a static block: `await` is reserved, and a nested function
    /// clears it.
    pub static_block: bool,
    /// The `in` operator is allowed (not in a `for` statement's head).
    pub in_: bool,
}

/// A label in scope, and whether it labels a loop (for `continue`).
#[derive(Debug, Clone)]
struct Label {
    name: Box<str>,
    is_loop: bool,
}

/// A class body being parsed, for its private names (section 15.7.1:
/// every `#x` used must be declared by an enclosing class).
#[derive(Debug, Default)]
struct ClassScope {
    /// Declared names and what declared them, to refuse duplicates (a
    /// map: a class may declare thousands).
    declared: HashMap<Box<str>, PrivateKind>,
    /// Names used and not yet declared in this class, with where.
    used: Vec<Ident>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrivateKind {
    Field,
    Method,
    Getter { is_static: bool },
    Setter { is_static: bool },
    Both,
}

pub(crate) struct Parser<'s> {
    source: &'s str,
    lexer: Lexer<'s>,
    /// The token being looked at.
    tok: Token,
    /// The end of the token before it.
    prev_end: u32,
    ast: Ast,
    module: bool,
    cx: Context,
    depth: u32,
    scopes: Scopes,
    labels: Vec<Label>,
    /// Whether a `break` without a label has a target, and a `continue`.
    in_breakable: bool,
    in_loop: bool,
    classes: Vec<ClassScope>,
    /// The first `yield` or `await` expression, and `await` read as an
    /// identifier, since the last reset: arrow parameters may have none
    /// of them (sections 15.3.1 and 15.9.1).
    yield_at: Option<Span>,
    await_at: Option<Span>,
    await_ident_at: Option<Span>,
    /// Spread elements followed by more of their literal (`[...a, b]`,
    /// `{...a,}`): fine in an expression, refused as a pattern's rest.
    rest_not_last: HashSet<ExprId>,
    /// The stack used since the parse began, checked at each level.
    stack: StackGuard,
    stats: ParseStats,
}

const UNEXPECTED_END: &str = "Unexpected end of input";

impl<'s> Parser<'s> {
    pub(crate) fn new(source: &'s str, goal: Goal) -> Result<Parser<'s>, SyntaxError> {
        let module = goal == Goal::Module;
        let mut lexer = Lexer::new(source, module)?;
        let tok = lexer.next_token()?;
        let cx = Context {
            strict: module,
            await_: module,
            in_: true,
            ..Context::default()
        };
        Ok(Parser {
            source,
            lexer,
            tok,
            prev_end: 0,
            ast: Ast::default(),
            module,
            cx,
            depth: 0,
            scopes: Scopes::new(module),
            labels: Vec::new(),
            in_breakable: false,
            in_loop: false,
            classes: Vec::new(),
            yield_at: None,
            await_at: None,
            await_ident_at: None,
            rest_not_last: HashSet::new(),
            stack: StackGuard::new(DEFAULT_STACK_BUDGET),
            stats: ParseStats::default(),
        })
    }

    pub(crate) fn program(&mut self) -> Result<Program, SyntaxError> {
        let start = self.tok.start;
        let body = if self.module {
            self.module_body()?
        } else {
            let (body, strict, _) = self.directives_and_body(|p| p.at_eof(), true)?;
            self.cx.strict = strict;
            body
        };
        self.scopes.finish_top()?;
        Ok(Program {
            ast: std::mem::take(&mut self.ast),
            body,
            module: self.module,
            strict: self.cx.strict,
            span: Span::new(start, self.prev_end.max(start)),
        })
    }

    // The tokens.

    fn advance(&mut self) -> Result<Token, SyntaxError> {
        let next = self.lexer.next_token()?;
        self.prev_end = self.tok.end;
        Ok(std::mem::replace(&mut self.tok, next))
    }

    /// The token after the current one, read without moving.
    fn peek(&self) -> Result<Token, SyntaxError> {
        self.lexer.clone().next_token()
    }

    /// The token after the current one and the one after that.
    fn peek2(&self) -> Result<(Token, Token), SyntaxError> {
        let mut lexer = self.lexer.clone();
        let a = lexer.next_token()?;
        let b = lexer.next_token()?;
        Ok((a, b))
    }

    fn at(&self, p: Punct) -> bool {
        self.tok.is(p)
    }

    fn at_keyword(&self, k: Keyword) -> bool {
        self.tok.kind == TokenKind::Keyword(k)
    }

    fn at_eof(&self) -> bool {
        self.tok.kind == TokenKind::Eof
    }

    /// Whether the current token is the contextual keyword `name`
    /// (`async`, `of`, `let`...), spelled without escapes.
    fn at_contextual(&self, name: &str) -> bool {
        self.tok.kind == TokenKind::Identifier && !self.tok.escaped && self.tok_text() == name
    }

    fn tok_text(&self) -> &'s str {
        self.source
            .get(self.tok.start as usize..self.tok.end as usize)
            .unwrap_or("")
    }

    fn tok_name(&self) -> &str {
        self.tok.name(self.source)
    }

    fn tok_span(&self) -> Span {
        Span::new(self.tok.start, self.tok.end)
    }

    fn eat(&mut self, p: Punct) -> Result<bool, SyntaxError> {
        if self.at(p) {
            self.advance()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn eat_keyword(&mut self, k: Keyword) -> Result<bool, SyntaxError> {
        if self.at_keyword(k) {
            self.advance()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn expect(&mut self, p: Punct) -> Result<Token, SyntaxError> {
        if self.at(p) {
            self.advance()
        } else {
            Err(self.unexpected())
        }
    }

    fn expect_keyword(&mut self, k: Keyword) -> Result<Token, SyntaxError> {
        if self.at_keyword(k) {
            self.advance()
        } else {
            Err(self.unexpected())
        }
    }

    /// A `;`, or one inserted (section 12.10): before `}`, at the end, or
    /// after a line terminator.
    fn semicolon(&mut self) -> Result<(), SyntaxError> {
        if self.eat(Punct::Semicolon)? {
            return Ok(());
        }
        if self.at(Punct::RBrace) || self.at_eof() || self.tok.newline_before {
            return Ok(());
        }
        Err(self.unexpected())
    }

    // Errors.

    fn error(&self, message: impl Into<String>, span: Span) -> SyntaxError {
        SyntaxError {
            message: message.into(),
            start: span.start,
            end: span.end,
        }
    }

    /// The reference browser's words for the current token where it was
    /// not expected.
    fn unexpected(&self) -> SyntaxError {
        self.unexpected_token(&self.tok)
    }

    fn unexpected_token(&self, tok: &Token) -> SyntaxError {
        let span = Span::new(tok.start, tok.end);
        let text = self
            .source
            .get(tok.start as usize..tok.end as usize)
            .unwrap_or("");
        let message = match tok.kind {
            TokenKind::Eof => UNEXPECTED_END.to_owned(),
            TokenKind::Identifier => {
                let name = tok.name(self.source);
                if is_strict_reserved(name) && self.cx.strict {
                    "Unexpected strict mode reserved word".to_owned()
                } else {
                    format!("Unexpected identifier '{name}'")
                }
            }
            TokenKind::Keyword(Keyword::True | Keyword::False | Keyword::Null) => {
                format!("Unexpected token '{text}'")
            }
            TokenKind::Keyword(k) => format!("Unexpected token '{}'", keyword_text(k)),
            TokenKind::Number | TokenKind::BigInt => "Unexpected number".to_owned(),
            TokenKind::String => "Unexpected string".to_owned(),
            TokenKind::Template { .. } => "Unexpected template string".to_owned(),
            TokenKind::PrivateName => format!("Unexpected identifier '{text}'"),
            TokenKind::Punct(_) | TokenKind::RegExp => format!("Unexpected token '{text}'"),
        };
        self.error(message, span)
    }

    // Nesting.

    /// Counts one level of nesting, refusing past [`MAX_DEPTH`]; paired
    /// with [`Parser::leave`].
    fn enter(&mut self) -> Result<(), SyntaxError> {
        self.depth += 1;
        let used = self.stack.used();
        self.stats.max_depth = self.stats.max_depth.max(self.depth);
        self.stats.peak_stack = self.stats.peak_stack.max(used);
        if self.depth > MAX_DEPTH || self.stack.exceeded() {
            return Err(self.error("Maximum call stack size exceeded", self.tok_span()));
        }
        Ok(())
    }

    /// How many scopes are open, the top one included.
    fn scopes_depth(&self) -> usize {
        self.scopes.depth()
    }

    fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    // Identifiers.

    /// Whether `name` may be an IdentifierReference or a binding here
    /// (section 13.1.1); the error if not.
    fn check_identifier(&self, name: &str, span: Span, binding: bool) -> Result<(), SyntaxError> {
        if Keyword::of(name).is_some() {
            // Only an escaped spelling reaches here as an identifier.
            return Err(self.error("Keyword must not contain escaped characters", span));
        }
        match name {
            "yield" => {
                if self.cx.yield_ || self.cx.strict {
                    return Err(self.reserved_error(name, span));
                }
            }
            "await" => {
                if self.cx.await_ || self.module || self.cx.static_block {
                    return Err(self.reserved_error(name, span));
                }
            }
            "arguments" | "eval" if binding && self.cx.strict => {
                return Err(self.error("Unexpected eval or arguments in strict mode", span));
            }
            "arguments" if !binding && self.cx.class_field => {
                return Err(self.error(
                    "'arguments' is not allowed in class field initializer or static initialization block",
                    span,
                ));
            }
            _ if self.cx.strict && is_strict_reserved(name) => {
                return Err(self.error("Unexpected strict mode reserved word", span));
            }
            _ => {}
        }
        Ok(())
    }

    fn reserved_error(&self, name: &str, span: Span) -> SyntaxError {
        if self.cx.strict && name == "yield" {
            self.error("Unexpected strict mode reserved word", span)
        } else {
            self.error("Unexpected reserved word", span)
        }
    }

    /// The current token as an identifier (reference or binding), checked
    /// against the context, and moved past.
    fn identifier(&mut self, binding: bool) -> Result<Ident, SyntaxError> {
        if self.tok.kind != TokenKind::Identifier {
            return Err(self.unexpected());
        }
        let span = self.tok_span();
        let name: Box<str> = self.tok_name().into();
        self.check_identifier(&name, span, binding)?;
        if &*name == "await" && self.await_ident_at.is_none() {
            self.await_ident_at = Some(span);
        }
        self.advance()?;
        Ok(Ident { name, span })
    }

    /// Whether the current token can start an identifier reference here.
    fn at_identifier(&self) -> bool {
        self.tok.kind == TokenKind::Identifier
    }

    /// An IdentifierName (any word, reserved or not): a property's name.
    fn identifier_name(&mut self) -> Result<Ident, SyntaxError> {
        let span = self.tok_span();
        let name: Box<str> = match self.tok.kind {
            TokenKind::Identifier => self.tok_name().into(),
            TokenKind::Keyword(k) => keyword_text(k).into(),
            _ => return Err(self.unexpected()),
        };
        self.advance()?;
        Ok(Ident { name, span })
    }

    fn at_identifier_name(&self) -> bool {
        matches!(self.tok.kind, TokenKind::Identifier | TokenKind::Keyword(_))
    }

    /// Raises a token's strict-mode error if the code is strict.
    fn check_strict_token(&self, tok: &Token) -> Result<(), SyntaxError> {
        if self.cx.strict
            && let Some(error) = &tok.strict_error
        {
            return Err(error.clone());
        }
        Ok(())
    }

    /// Runs `f` with the context changed by `change`, then puts it back.
    fn with_context<T>(
        &mut self,
        change: impl FnOnce(&mut Context),
        f: impl FnOnce(&mut Self) -> Result<T, SyntaxError>,
    ) -> Result<T, SyntaxError> {
        let saved = self.cx;
        change(&mut self.cx);
        let result = f(self);
        self.cx = saved;
        result
    }

    fn string_value(tok: &Token) -> Box<[u16]> {
        match &tok.value {
            TokenValue::String(s) => s.clone(),
            _ => Box::default(),
        }
    }
}

/// The words reserved in strict mode code only (section 12.7.2), beside
/// `yield`, which has rules of its own.
pub(crate) fn is_strict_reserved(name: &str) -> bool {
    matches!(
        name,
        "implements"
            | "interface"
            | "let"
            | "package"
            | "private"
            | "protected"
            | "public"
            | "static"
            | "yield"
    )
}

/// A UTF-16 string as Rust text, lone surrogates replaced.
pub(crate) fn utf16_text(units: &[u16]) -> String {
    String::from_utf16_lossy(units)
}
