//! The lexer (ECMA-262 section 12, and Annex B.1's comments): a source
//! into tokens, one at a time, as the parser asks.
//!
//! The standard's lexical grammar has several goal symbols, chosen by
//! the syntactic context: whether a `/` starts a regular expression or
//! divides, and whether a `}` ends a block or a template's substitution.
//! The lexer always reads the dividing, block-ending way, and the parser,
//! which knows the context, asks it to read a `/`, `/=` or `}` token
//! again the other way ([`Lexer::rescan_regexp`],
//! [`Lexer::rescan_template`]).
//!
//! What depends on more than the token is left to the parser and marked
//! on the token: a reserved word spelled with escapes
//! ([`Token::escaped`]), a legacy octal literal or escape that strict
//! code refuses ([`Token::strict_error`]), and a template's invalid
//! escape, an error unless the template is tagged
//! ([`Token::invalid_escape`]).
//!
//! Nothing here recurses, and every position is a byte offset into the
//! source as a `u32`.

use std::borrow::Cow;

use crate::unicode::{LS, PS, is_id_continue, is_id_start, is_line_terminator, is_white_space};
use crate::{SyntaxError, clamp};

/// The longest source a lexer reads, in bytes: positions are `u32`.
pub const MAX_SOURCE_BYTES: usize = u32::MAX as usize;

/// The reserved words that are never identifiers (section 12.7.2), and
/// the literals `true`, `false` and `null`. `await`, `yield`, `let`,
/// `static` and the strict-mode reserved words are identifiers here,
/// which the parser refuses where the context makes them reserved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Keyword {
    Break,
    Case,
    Catch,
    Class,
    Const,
    Continue,
    Debugger,
    Default,
    Delete,
    Do,
    Else,
    Enum,
    Export,
    Extends,
    False,
    Finally,
    For,
    Function,
    If,
    Import,
    In,
    Instanceof,
    New,
    Null,
    Return,
    Super,
    Switch,
    This,
    Throw,
    True,
    Try,
    Typeof,
    Var,
    Void,
    While,
    With,
}

impl Keyword {
    /// The keyword `name` spells, if any.
    pub fn of(name: &str) -> Option<Keyword> {
        Some(match name {
            "break" => Keyword::Break,
            "case" => Keyword::Case,
            "catch" => Keyword::Catch,
            "class" => Keyword::Class,
            "const" => Keyword::Const,
            "continue" => Keyword::Continue,
            "debugger" => Keyword::Debugger,
            "default" => Keyword::Default,
            "delete" => Keyword::Delete,
            "do" => Keyword::Do,
            "else" => Keyword::Else,
            "enum" => Keyword::Enum,
            "export" => Keyword::Export,
            "extends" => Keyword::Extends,
            "false" => Keyword::False,
            "finally" => Keyword::Finally,
            "for" => Keyword::For,
            "function" => Keyword::Function,
            "if" => Keyword::If,
            "import" => Keyword::Import,
            "in" => Keyword::In,
            "instanceof" => Keyword::Instanceof,
            "new" => Keyword::New,
            "null" => Keyword::Null,
            "return" => Keyword::Return,
            "super" => Keyword::Super,
            "switch" => Keyword::Switch,
            "this" => Keyword::This,
            "throw" => Keyword::Throw,
            "true" => Keyword::True,
            "try" => Keyword::Try,
            "typeof" => Keyword::Typeof,
            "var" => Keyword::Var,
            "void" => Keyword::Void,
            "while" => Keyword::While,
            "with" => Keyword::With,
            _ => return None,
        })
    }
}

/// A punctuator (section 12.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Punct {
    LBrace,
    RBrace,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Dot,
    Ellipsis,
    Semicolon,
    Comma,
    Lt,
    Gt,
    LtEq,
    GtEq,
    EqEq,
    NotEq,
    EqEqEq,
    NotEqEq,
    Plus,
    Minus,
    Star,
    Percent,
    StarStar,
    PlusPlus,
    MinusMinus,
    Shl,
    Shr,
    UShr,
    Amp,
    Pipe,
    Caret,
    Bang,
    Tilde,
    AmpAmp,
    PipePipe,
    QuestionQuestion,
    Question,
    QuestionDot,
    Colon,
    Eq,
    PlusEq,
    MinusEq,
    StarEq,
    PercentEq,
    StarStarEq,
    ShlEq,
    ShrEq,
    UShrEq,
    AmpEq,
    PipeEq,
    CaretEq,
    AmpAmpEq,
    PipePipeEq,
    QuestionQuestionEq,
    Arrow,
    Slash,
    SlashEq,
}

/// What a token is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    /// An IdentifierName that is no [`Keyword`], or one spelled with an
    /// escape ([`Token::escaped`]): its name is [`Token::name`].
    Identifier,
    /// A reserved word spelled without escapes.
    Keyword(Keyword),
    /// A PrivateIdentifier, `#name`: [`Token::name`] is without the `#`.
    PrivateName,
    Punct(Punct),
    /// A NumericLiteral: [`TokenValue::Number`].
    Number,
    /// A NumericLiteral with the BigInt suffix: [`TokenValue::BigInt`].
    BigInt,
    /// A StringLiteral: [`TokenValue::String`].
    String,
    /// A template's part (section 12.9.6): `head` when it begins with the
    /// backtick (not after a substitution), `tail` when it ends with one
    /// (not with `${`). [`TokenValue::Template`].
    Template {
        head: bool,
        tail: bool,
    },
    /// A RegularExpressionLiteral, made only by
    /// [`Lexer::rescan_regexp`]: [`TokenValue::RegExp`].
    RegExp,
    /// The end of the source.
    Eof,
}

/// What a token holds beyond its kind.
#[derive(Debug, Clone, PartialEq)]
pub enum TokenValue {
    None,
    /// An escaped identifier's or a private name's name, as its escapes
    /// mean it.
    Name(Box<str>),
    Number(f64),
    /// A BigInt's digits in `radix`, without separators or prefix.
    BigInt {
        radix: u32,
        digits: Box<str>,
    },
    /// A string's value in UTF-16 code units, lone surrogates and all.
    String(Box<[u16]>),
    /// A template part's cooked value (`None` when an escape is invalid,
    /// [`Token::invalid_escape`]) and its raw text, line terminators made
    /// LF (section 13.2.8.6).
    Template {
        cooked: Option<Box<[u16]>>,
        raw: Box<str>,
    },
    /// A regular expression's body and flags as written, checked by the
    /// RegExp parser, not here.
    RegExp {
        body: Box<str>,
        flags: Box<str>,
    },
}

/// A token.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub value: TokenValue,
    /// Its bytes in the source, `start..end`.
    pub start: u32,
    pub end: u32,
    /// Whether a line terminator came between it and the token before
    /// (in a comment or not), which automatic semicolon insertion and the
    /// restricted productions read.
    pub newline_before: bool,
    /// Whether an identifier's name is spelled with a Unicode escape: one
    /// that spells a reserved word is neither that word nor an
    /// identifier (section 12.7.2's early errors).
    pub escaped: bool,
    /// What strict mode code refuses in this token, and where: a legacy
    /// octal literal (`017`), a decimal with a leading zero (`08`), a
    /// legacy octal escape (`"\01"`) or `\8` and `\9`. The parser raises
    /// it once it knows the code is strict, which a directive after the
    /// token may decide.
    pub strict_error: Option<SyntaxError>,
    /// A template part's first invalid escape: an error unless the
    /// template is tagged (section 13.2.8.1), where its cooked value is
    /// undefined instead.
    pub invalid_escape: Option<SyntaxError>,
}

impl Token {
    /// An identifier's or private name's name, as its escapes mean it.
    pub fn name<'s>(&'s self, source: &'s str) -> &'s str {
        match &self.value {
            TokenValue::Name(name) => name,
            _ => source
                .get(self.start as usize..self.end as usize)
                .unwrap_or(""),
        }
    }

    /// Whether it is punctuator `p`.
    pub fn is(&self, p: Punct) -> bool {
        self.kind == TokenKind::Punct(p)
    }
}

/// "Invalid or unexpected token": the reference browser's words for a
/// character or literal that starts no token.
const UNEXPECTED: &str = "Invalid or unexpected token";
const UNTERMINATED_TEMPLATE: &str = "Unterminated template literal";
const UNTERMINATED_REGEXP: &str = "Invalid regular expression: missing /";
const BAD_HEX_ESCAPE: &str = "Invalid hexadecimal escape sequence";
const BAD_UNICODE_ESCAPE: &str = "Invalid Unicode escape sequence";
const BAD_CODE_POINT: &str = "Undefined Unicode code-point";
const OCTAL_LITERAL: &str = "Octal literals are not allowed in strict mode.";
const LEADING_ZERO: &str = "Decimals with leading zeros are not allowed in strict mode.";
const OCTAL_ESCAPE: &str = "Octal escape sequences are not allowed in strict mode.";
const EIGHT_NINE_ESCAPE: &str = "\\8 and \\9 are not allowed in strict mode.";
const OCTAL_TEMPLATE: &str = "Octal escape sequences are not allowed in template strings";
const EIGHT_NINE_TEMPLATE: &str = "\\8 and \\9 are not allowed in template strings";
const SEPARATOR_AFTER_ZERO: &str = "Numeric separator can not be used after leading 0.";
const SEPARATOR_DOUBLED: &str = "Only one underscore is allowed as numeric separator";
const SEPARATOR_AT_END: &str = "Numeric separators are not allowed at the end of numeric literals";
const SEPARATOR_MISPLACED: &str = "Numeric separators are not allowed here";
const BAD_REGEXP_FLAGS: &str = "Invalid regular expression flags";

/// A lexer over one source.
#[derive(Debug, Clone)]
pub struct Lexer<'s> {
    source: &'s str,
    bytes: &'s [u8],
    pos: usize,
    /// A module's source, where Annex B's HTML-like comments are not
    /// comments (section B.1.1).
    module: bool,
}

impl<'s> Lexer<'s> {
    /// A lexer at the start of `source`, past its hashbang comment if it
    /// has one (section 12.5). `module` for a module's source.
    pub fn new(source: &'s str, module: bool) -> Result<Lexer<'s>, SyntaxError> {
        if source.len() > MAX_SOURCE_BYTES {
            return Err(SyntaxError::new("Source is too long", 0, 0));
        }
        let mut lexer = Lexer {
            source,
            bytes: source.as_bytes(),
            pos: 0,
            module,
        };
        if source.starts_with("#!") {
            lexer.pos = 2;
            lexer.skip_line();
        }
        Ok(lexer)
    }

    /// Where the lexer is: the byte after the last token read.
    pub fn position(&self) -> u32 {
        clamp(self.pos)
    }

    /// Moves the lexer back to `position`, the end of a token it read,
    /// for a parser that reads ahead and returns.
    pub fn reset(&mut self, position: u32) {
        self.pos = (position as usize).min(self.bytes.len());
    }

    fn byte(&self, at: usize) -> Option<u8> {
        self.bytes.get(at).copied()
    }

    fn peek(&self) -> Option<u8> {
        self.byte(self.pos)
    }

    fn peek_at(&self, ahead: usize) -> Option<u8> {
        self.byte(self.pos.saturating_add(ahead))
    }

    /// The character at `at`, if one starts there.
    fn char_at(&self, at: usize) -> Option<char> {
        self.source.get(at..).and_then(|rest| rest.chars().next())
    }

    fn slice(&self, start: usize, end: usize) -> &'s str {
        self.source.get(start..end).unwrap_or("")
    }

    fn error(&self, message: &str, start: usize) -> SyntaxError {
        SyntaxError::new(message, start, self.pos.max(start))
    }

    /// Skips to the next line terminator, which it leaves.
    fn skip_line(&mut self) {
        while let Some(b) = self.peek() {
            match b {
                b'\n' | b'\r' => return,
                _ if b < 0x80 => self.pos += 1,
                _ => match self.char_at(self.pos) {
                    Some(c) if c == LS || c == PS => return,
                    Some(c) => self.pos += c.len_utf8(),
                    None => self.pos += 1,
                },
            }
        }
    }

    /// Skips white space, line terminators and comments; whether a line
    /// terminator was among them.
    fn skip_trivia(&mut self) -> Result<bool, SyntaxError> {
        let mut newline = false;
        // Whether only white space and comments on one line have come
        // since a line terminator: where `-->` begins a comment (Annex
        // B.1.1's SingleLineHTMLCloseComment). Not at the source's start.
        let mut line_start = false;
        while let Some(b) = self.peek() {
            match b {
                b' ' | b'\t' | 0x0B | 0x0C => self.pos += 1,
                b'\n' | b'\r' => {
                    self.pos += 1;
                    newline = true;
                    line_start = true;
                }
                b'/' if self.peek_at(1) == Some(b'/') => self.skip_line(),
                b'/' if self.peek_at(1) == Some(b'*') => {
                    let start = self.pos;
                    self.pos += 2;
                    let mut lines = false;
                    loop {
                        match self.peek() {
                            None => return Err(self.error(UNEXPECTED, start)),
                            Some(b'*') if self.peek_at(1) == Some(b'/') => {
                                self.pos += 2;
                                break;
                            }
                            Some(b'\n' | b'\r') => {
                                lines = true;
                                self.pos += 1;
                            }
                            Some(b) if b < 0x80 => self.pos += 1,
                            Some(_) => match self.char_at(self.pos) {
                                Some(c) => {
                                    lines |= c == LS || c == PS;
                                    self.pos += c.len_utf8();
                                }
                                None => self.pos += 1,
                            },
                        }
                    }
                    if lines {
                        newline = true;
                        line_start = true;
                    }
                }
                b'<' if !self.module
                    && self
                        .bytes
                        .get(self.pos..)
                        .is_some_and(|r| r.starts_with(b"<!--")) =>
                {
                    self.skip_line();
                }
                b'-' if !self.module
                    && line_start
                    && self
                        .bytes
                        .get(self.pos..)
                        .is_some_and(|r| r.starts_with(b"-->")) =>
                {
                    self.skip_line();
                }
                _ if b < 0x80 => break,
                _ => match self.char_at(self.pos) {
                    Some(c) if is_line_terminator(c) => {
                        self.pos += c.len_utf8();
                        newline = true;
                        line_start = true;
                    }
                    Some(c) if is_white_space(c) => self.pos += c.len_utf8(),
                    _ => break,
                },
            }
        }
        Ok(newline)
    }

    fn token(&self, kind: TokenKind, value: TokenValue, start: usize, newline: bool) -> Token {
        Token {
            kind,
            value,
            start: clamp(start),
            end: clamp(self.pos),
            newline_before: newline,
            escaped: false,
            strict_error: None,
            invalid_escape: None,
        }
    }

    /// The next token, read the dividing way: a `/` is division and a
    /// `}` ends a block (see the module's text).
    pub fn next_token(&mut self) -> Result<Token, SyntaxError> {
        let newline = self.skip_trivia()?;
        let start = self.pos;
        let Some(b) = self.peek() else {
            return Ok(self.token(TokenKind::Eof, TokenValue::None, start, newline));
        };
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'$' | b'_' | b'\\' => self.identifier(start, newline),
            b'0'..=b'9' => self.number(start, newline),
            b'.' if self.peek_at(1).is_some_and(|d| d.is_ascii_digit()) => {
                self.number(start, newline)
            }
            b'"' | b'\'' => self.string(start, newline),
            b'`' => {
                self.pos += 1;
                self.template(start, newline, true)
            }
            b'#' => self.private_name(start, newline),
            _ if b < 0x80 => {
                let punct = self.punct().ok_or_else(|| {
                    self.pos = start + 1;
                    self.error(UNEXPECTED, start)
                })?;
                Ok(self.token(TokenKind::Punct(punct), TokenValue::None, start, newline))
            }
            _ => match self.char_at(start) {
                Some(c) if is_id_start(c) => self.identifier(start, newline),
                Some(c) => {
                    self.pos = start + c.len_utf8();
                    Err(self.error(UNEXPECTED, start))
                }
                None => {
                    self.pos = start + 1;
                    Err(self.error(UNEXPECTED, start))
                }
            },
        }
    }

    /// Reads a punctuator at the lexer's position, longest first, and
    /// moves past it.
    fn punct(&mut self) -> Option<Punct> {
        let at = |n: usize| self.peek_at(n);
        let (punct, len) = match at(0)? {
            b'{' => (Punct::LBrace, 1),
            b'}' => (Punct::RBrace, 1),
            b'(' => (Punct::LParen, 1),
            b')' => (Punct::RParen, 1),
            b'[' => (Punct::LBracket, 1),
            b']' => (Punct::RBracket, 1),
            b';' => (Punct::Semicolon, 1),
            b',' => (Punct::Comma, 1),
            b':' => (Punct::Colon, 1),
            b'~' => (Punct::Tilde, 1),
            b'.' => match (at(1), at(2)) {
                (Some(b'.'), Some(b'.')) => (Punct::Ellipsis, 3),
                _ => (Punct::Dot, 1),
            },
            b'<' => match (at(1), at(2)) {
                (Some(b'<'), Some(b'=')) => (Punct::ShlEq, 3),
                (Some(b'<'), _) => (Punct::Shl, 2),
                (Some(b'='), _) => (Punct::LtEq, 2),
                _ => (Punct::Lt, 1),
            },
            b'>' => match (at(1), at(2), at(3)) {
                (Some(b'>'), Some(b'>'), Some(b'=')) => (Punct::UShrEq, 4),
                (Some(b'>'), Some(b'>'), _) => (Punct::UShr, 3),
                (Some(b'>'), Some(b'='), _) => (Punct::ShrEq, 3),
                (Some(b'>'), _, _) => (Punct::Shr, 2),
                (Some(b'='), _, _) => (Punct::GtEq, 2),
                _ => (Punct::Gt, 1),
            },
            b'=' => match (at(1), at(2)) {
                (Some(b'='), Some(b'=')) => (Punct::EqEqEq, 3),
                (Some(b'='), _) => (Punct::EqEq, 2),
                (Some(b'>'), _) => (Punct::Arrow, 2),
                _ => (Punct::Eq, 1),
            },
            b'!' => match (at(1), at(2)) {
                (Some(b'='), Some(b'=')) => (Punct::NotEqEq, 3),
                (Some(b'='), _) => (Punct::NotEq, 2),
                _ => (Punct::Bang, 1),
            },
            b'+' => match at(1) {
                Some(b'+') => (Punct::PlusPlus, 2),
                Some(b'=') => (Punct::PlusEq, 2),
                _ => (Punct::Plus, 1),
            },
            b'-' => match at(1) {
                Some(b'-') => (Punct::MinusMinus, 2),
                Some(b'=') => (Punct::MinusEq, 2),
                _ => (Punct::Minus, 1),
            },
            b'*' => match (at(1), at(2)) {
                (Some(b'*'), Some(b'=')) => (Punct::StarStarEq, 3),
                (Some(b'*'), _) => (Punct::StarStar, 2),
                (Some(b'='), _) => (Punct::StarEq, 2),
                _ => (Punct::Star, 1),
            },
            b'%' => match at(1) {
                Some(b'=') => (Punct::PercentEq, 2),
                _ => (Punct::Percent, 1),
            },
            b'&' => match (at(1), at(2)) {
                (Some(b'&'), Some(b'=')) => (Punct::AmpAmpEq, 3),
                (Some(b'&'), _) => (Punct::AmpAmp, 2),
                (Some(b'='), _) => (Punct::AmpEq, 2),
                _ => (Punct::Amp, 1),
            },
            b'|' => match (at(1), at(2)) {
                (Some(b'|'), Some(b'=')) => (Punct::PipePipeEq, 3),
                (Some(b'|'), _) => (Punct::PipePipe, 2),
                (Some(b'='), _) => (Punct::PipeEq, 2),
                _ => (Punct::Pipe, 1),
            },
            b'^' => match at(1) {
                Some(b'=') => (Punct::CaretEq, 2),
                _ => (Punct::Caret, 1),
            },
            b'?' => match (at(1), at(2)) {
                (Some(b'?'), Some(b'=')) => (Punct::QuestionQuestionEq, 3),
                (Some(b'?'), _) => (Punct::QuestionQuestion, 2),
                // `?.` but not before a digit: `a?.5:b` is a conditional
                // (section 12.8, OptionalChainingPunctuator).
                (Some(b'.'), d) if !d.is_some_and(|d| d.is_ascii_digit()) => {
                    (Punct::QuestionDot, 2)
                }
                _ => (Punct::Question, 1),
            },
            b'/' => match at(1) {
                Some(b'=') => (Punct::SlashEq, 2),
                _ => (Punct::Slash, 1),
            },
            _ => return None,
        };
        self.pos += len;
        Some(punct)
    }

    /// Reads `\u` and four hex digits or `\u{...}` at the lexer's
    /// position (at the backslash): the code point, which may be a lone
    /// surrogate, or why the escape is malformed, the lexer moved past
    /// what it read (never past a character that is no hex digit, `{` or
    /// `}`, so a template's closing backtick is never taken).
    fn unicode_escape(&mut self) -> Result<u32, &'static str> {
        if self.peek_at(1) != Some(b'u') {
            self.pos += 1;
            return Err(BAD_UNICODE_ESCAPE);
        }
        self.pos += 2;
        if self.peek() == Some(b'{') {
            self.pos += 1;
            let mut value: u32 = 0;
            let mut digits = 0;
            while let Some(d) = self.peek().and_then(|b| (b as char).to_digit(16)) {
                self.pos += 1;
                digits += 1;
                value = value.saturating_mul(16).saturating_add(d);
            }
            if digits == 0 || self.peek() != Some(b'}') {
                return Err(BAD_UNICODE_ESCAPE);
            }
            self.pos += 1;
            if value > 0x10FFFF {
                return Err(BAD_CODE_POINT);
            }
            Ok(value)
        } else {
            let mut value: u32 = 0;
            for _ in 0..4 {
                match self.peek().and_then(|b| (b as char).to_digit(16)) {
                    Some(d) => {
                        self.pos += 1;
                        value = value * 16 + d;
                    }
                    None => return Err(BAD_UNICODE_ESCAPE),
                }
            }
            Ok(value)
        }
    }

    /// Reads an IdentifierName from `start` (section 12.7): its name, as
    /// its escapes mean it when it has any (`Some`).
    fn identifier_name(&mut self, start: usize) -> Result<Option<String>, SyntaxError> {
        let mut cooked: Option<String> = None;
        let mut first = true;
        loop {
            let at = self.pos;
            let Some(b) = self.peek() else { break };
            let c = if b == b'\\' {
                let code = self
                    .unicode_escape()
                    .map_err(|message| self.error(message, at))?;
                let c = char::from_u32(code)
                    .filter(|&c| {
                        if first {
                            is_id_start(c)
                        } else {
                            is_id_continue(c)
                        }
                    })
                    .ok_or_else(|| self.error(BAD_UNICODE_ESCAPE, at))?;
                cooked
                    .get_or_insert_with(|| self.slice(start, at).to_owned())
                    .push(c);
                first = false;
                continue;
            } else if b < 0x80 {
                b as char
            } else {
                match self.char_at(at) {
                    Some(c) => c,
                    None => break,
                }
            };
            let fits = if first {
                is_id_start(c)
            } else {
                is_id_continue(c)
            };
            if !fits {
                break;
            }
            self.pos += c.len_utf8();
            if let Some(cooked) = &mut cooked {
                cooked.push(c);
            }
            first = false;
        }
        Ok(cooked)
    }

    fn identifier(&mut self, start: usize, newline: bool) -> Result<Token, SyntaxError> {
        let cooked = self.identifier_name(start)?;
        if self.pos == start {
            // A backslash that starts no escape was refused above; this is
            // a lone character that starts no identifier.
            self.pos = start + 1;
            return Err(self.error(UNEXPECTED, start));
        }
        Ok(match cooked {
            Some(name) => {
                let mut token = self.token(
                    TokenKind::Identifier,
                    TokenValue::Name(name.into()),
                    start,
                    newline,
                );
                token.escaped = true;
                token
            }
            None => match Keyword::of(self.slice(start, self.pos)) {
                Some(keyword) => self.token(
                    TokenKind::Keyword(keyword),
                    TokenValue::None,
                    start,
                    newline,
                ),
                None => self.token(TokenKind::Identifier, TokenValue::None, start, newline),
            },
        })
    }

    fn private_name(&mut self, start: usize, newline: bool) -> Result<Token, SyntaxError> {
        self.pos += 1;
        let cooked = self.identifier_name(self.pos)?;
        if self.pos == start + 1 {
            return Err(self.error(UNEXPECTED, start));
        }
        let escaped = cooked.is_some();
        let name = cooked.unwrap_or_else(|| self.slice(start + 1, self.pos).to_owned());
        let mut token = self.token(
            TokenKind::PrivateName,
            TokenValue::Name(name.into()),
            start,
            newline,
        );
        token.escaped = escaped;
        Ok(token)
    }

    /// Reads digits of `radix` into `out`, with numeric separators where
    /// `separators` allows them (section 12.9.3): how many digits.
    fn digits(
        &mut self,
        radix: u32,
        separators: bool,
        out: &mut String,
    ) -> Result<usize, SyntaxError> {
        let mut count = 0;
        let mut after_separator = false;
        while let Some(b) = self.peek() {
            if b == b'_' && separators {
                let at = self.pos;
                self.pos += 1;
                if after_separator {
                    return Err(self.error(SEPARATOR_DOUBLED, at));
                }
                if count == 0 {
                    return Err(self.error(SEPARATOR_MISPLACED, at));
                }
                after_separator = true;
                continue;
            }
            if b == b'_' {
                let at = self.pos;
                self.pos += 1;
                return Err(self.error(SEPARATOR_AFTER_ZERO, at));
            }
            if !(b as char).is_digit(radix) {
                break;
            }
            out.push(b as char);
            count += 1;
            after_separator = false;
            self.pos += 1;
        }
        if after_separator {
            return Err(self.error(SEPARATOR_AT_END, self.pos - 1));
        }
        Ok(count)
    }

    /// Reads a NumericLiteral from `start` (section 12.9.3, and Annex
    /// B.1.1's legacy octal and leading-zero decimals).
    fn number(&mut self, start: usize, newline: bool) -> Result<Token, SyntaxError> {
        let mut digits = String::new();
        if self.peek() == Some(b'0') {
            let radix = match self.peek_at(1) {
                Some(b'x' | b'X') => Some(16),
                Some(b'o' | b'O') => Some(8),
                Some(b'b' | b'B') => Some(2),
                _ => None,
            };
            if let Some(radix) = radix {
                self.pos += 2;
                if self.digits(radix, true, &mut digits)? == 0 {
                    return Err(self.error(UNEXPECTED, start));
                }
                let bits = match radix {
                    16 => 4,
                    8 => 3,
                    _ => 1,
                };
                return self.number_end(start, newline, radix, digits, bits, None);
            }
            match self.peek_at(1) {
                Some(b'0'..=b'9') => {
                    // `017`, legacy octal, or `08`/`019`, a decimal with a
                    // leading zero: no separators, no BigInt.
                    self.pos += 1;
                    self.digits(10, false, &mut digits)?;
                    if digits.bytes().all(|d| d < b'8') {
                        let error = SyntaxError::new(OCTAL_LITERAL, start, self.pos);
                        return self.number_end(start, newline, 8, digits, 3, Some(error));
                    }
                    let error = SyntaxError::new(LEADING_ZERO, start, self.pos);
                    // Its fraction and exponent are a decimal's.
                    return self.decimal_rest(start, newline, digits, Some(error), false);
                }
                Some(b'_') => {
                    self.pos += 1;
                    return Err(self.error(SEPARATOR_AFTER_ZERO, self.pos));
                }
                _ => {}
            }
        }
        if self.peek() != Some(b'.') {
            self.digits(10, true, &mut digits)?;
        }
        self.decimal_rest(start, newline, digits, None, true)
    }

    /// A decimal literal's fraction, exponent and BigInt suffix, after
    /// its integer digits `int` (`bigint` when the suffix may follow).
    fn decimal_rest(
        &mut self,
        start: usize,
        newline: bool,
        int: String,
        strict_error: Option<SyntaxError>,
        bigint: bool,
    ) -> Result<Token, SyntaxError> {
        let mut text = if int.is_empty() {
            "0".to_owned()
        } else {
            int.clone()
        };
        let mut integer = true;
        if self.peek() == Some(b'.') {
            self.pos += 1;
            integer = false;
            if self.peek() == Some(b'_') {
                return Err(self.error(SEPARATOR_MISPLACED, self.pos));
            }
            let mut fraction = String::new();
            if self.digits(10, true, &mut fraction)? > 0 {
                text.push('.');
                text.push_str(&fraction);
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            let at = self.pos;
            self.pos += 1;
            integer = false;
            text.push('e');
            if let Some(sign @ (b'+' | b'-')) = self.peek() {
                text.push(sign as char);
                self.pos += 1;
            }
            if self.peek() == Some(b'_') {
                return Err(self.error(SEPARATOR_MISPLACED, self.pos));
            }
            if self.digits(10, true, &mut text)? == 0 {
                return Err(self.error(UNEXPECTED, at));
            }
        }
        if self.peek() == Some(b'n') {
            if !(integer && bigint) {
                self.pos += 1;
                return Err(self.error(UNEXPECTED, start));
            }
            self.pos += 1;
            self.check_after_number(start)?;
            let mut token = self.token(
                TokenKind::BigInt,
                TokenValue::BigInt {
                    radix: 10,
                    digits: int.into(),
                },
                start,
                newline,
            );
            token.strict_error = strict_error;
            return Ok(token);
        }
        self.check_after_number(start)?;
        let value = text
            .parse::<f64>()
            .map_err(|_| self.error(UNEXPECTED, start))?;
        let mut token = self.token(TokenKind::Number, TokenValue::Number(value), start, newline);
        token.strict_error = strict_error;
        Ok(token)
    }

    /// A literal in a radix of a power of two (hex, octal, binary, legacy
    /// octal), its digits read: its BigInt suffix (none for legacy octal,
    /// which has a strict error), and its value.
    fn number_end(
        &mut self,
        start: usize,
        newline: bool,
        radix: u32,
        digits: String,
        bits: u32,
        strict_error: Option<SyntaxError>,
    ) -> Result<Token, SyntaxError> {
        if self.peek() == Some(b'n') {
            self.pos += 1;
            if strict_error.is_some() {
                return Err(self.error(UNEXPECTED, start));
            }
            self.check_after_number(start)?;
            return Ok(self.token(
                TokenKind::BigInt,
                TokenValue::BigInt {
                    radix,
                    digits: digits.into(),
                },
                start,
                newline,
            ));
        }
        self.check_after_number(start)?;
        let value = pow2_radix_value(&digits, bits);
        let mut token = self.token(TokenKind::Number, TokenValue::Number(value), start, newline);
        token.strict_error = strict_error;
        Ok(token)
    }

    /// The character after a numeric literal may not start an identifier
    /// or be a digit (section 12.9.3: `3in` is an error).
    fn check_after_number(&mut self, start: usize) -> Result<(), SyntaxError> {
        let bad = match self.peek() {
            Some(b) if b.is_ascii_digit() || b == b'\\' => true,
            Some(b) if b < 0x80 => is_id_start(b as char),
            Some(_) => self.char_at(self.pos).is_some_and(is_id_start),
            None => false,
        };
        if bad {
            self.pos += self.char_at(self.pos).map_or(1, char::len_utf8);
            return Err(self.error(UNEXPECTED, start));
        }
        Ok(())
    }

    /// Reads an escape at the lexer's position (at the backslash) of a
    /// string (`template` false) or a template, pushing its value to
    /// `out` (section 12.9.4). `Ok(Some(error))` for what strict code
    /// refuses (a string's legacy octal escapes); `Err` for what is
    /// malformed, which a template keeps as its invalid escape.
    fn escape(
        &mut self,
        out: &mut Vec<u16>,
        template: bool,
    ) -> Result<Option<SyntaxError>, SyntaxError> {
        let start = self.pos;
        let Some(b) = self.peek_at(1) else {
            self.pos += 1;
            return Err(self.error(UNEXPECTED, start));
        };
        let simple = |c: u8| -> Option<u16> {
            Some(match c {
                b'b' => 0x08,
                b't' => 0x09,
                b'n' => 0x0A,
                b'v' => 0x0B,
                b'f' => 0x0C,
                b'r' => 0x0D,
                _ => return None,
            })
        };
        if let Some(unit) = simple(b) {
            self.pos += 2;
            out.push(unit);
            return Ok(None);
        }
        match b {
            // A line continuation: nothing.
            b'\n' => self.pos += 2,
            b'\r' => {
                self.pos += 2;
                if self.peek() == Some(b'\n') {
                    self.pos += 1;
                }
            }
            b'0' if !self.peek_at(2).is_some_and(|d| d.is_ascii_digit()) => {
                self.pos += 2;
                out.push(0);
            }
            b'0'..=b'7' => {
                self.pos += 1;
                if template {
                    self.pos += 1;
                    return Err(self.error(OCTAL_TEMPLATE, start));
                }
                // LegacyOctalEscapeSequence (Annex B.1.2): up to \377.
                let max_digits = if b <= b'3' { 3 } else { 2 };
                let mut value: u16 = 0;
                let mut read = 0;
                while read < max_digits {
                    match self.peek() {
                        Some(d @ b'0'..=b'7') => {
                            value = value * 8 + u16::from(d - b'0');
                            self.pos += 1;
                            read += 1;
                        }
                        _ => break,
                    }
                }
                out.push(value);
                return Ok(Some(SyntaxError::new(OCTAL_ESCAPE, start, self.pos)));
            }
            b'8' | b'9' => {
                self.pos += 2;
                if template {
                    return Err(self.error(EIGHT_NINE_TEMPLATE, start));
                }
                out.push(u16::from(b));
                return Ok(Some(SyntaxError::new(EIGHT_NINE_ESCAPE, start, self.pos)));
            }
            b'x' => {
                self.pos += 2;
                let mut value: u16 = 0;
                for _ in 0..2 {
                    match self.peek().and_then(|b| (b as char).to_digit(16)) {
                        Some(d) => {
                            self.pos += 1;
                            value = value * 16 + d as u16;
                        }
                        None => return Err(self.error(BAD_HEX_ESCAPE, start)),
                    }
                }
                out.push(value);
            }
            b'u' => {
                let code = self
                    .unicode_escape()
                    .map_err(|message| self.error(message, start))?;
                push_code_point(out, code);
            }
            _ => {
                // An identity escape: the character itself, or a line
                // continuation of LS or PS.
                self.pos += 1;
                let c = self.char_at(self.pos).unwrap_or('\u{FFFD}');
                self.pos += c.len_utf8();
                if c != LS && c != PS {
                    push_code_point(out, u32::from(c));
                }
            }
        }
        Ok(None)
    }

    fn string(&mut self, start: usize, newline: bool) -> Result<Token, SyntaxError> {
        let quote = self.peek();
        self.pos += 1;
        let mut value = Vec::new();
        let mut strict_error = None;
        loop {
            match self.peek() {
                None | Some(b'\n' | b'\r') => return Err(self.error(UNEXPECTED, start)),
                Some(b) if Some(b) == quote => {
                    self.pos += 1;
                    break;
                }
                Some(b'\\') => {
                    if let Some(error) = self.escape(&mut value, false)? {
                        strict_error.get_or_insert(error);
                    }
                }
                Some(b) if b < 0x80 => {
                    value.push(u16::from(b));
                    self.pos += 1;
                }
                Some(_) => {
                    // LS and PS may stand in a string (section 12.9.4).
                    let c = self.char_at(self.pos).unwrap_or('\u{FFFD}');
                    self.pos += c.len_utf8();
                    push_code_point(&mut value, u32::from(c));
                }
            }
        }
        let mut token = self.token(
            TokenKind::String,
            TokenValue::String(value.into()),
            start,
            newline,
        );
        token.strict_error = strict_error;
        Ok(token)
    }

    /// Reads a template's part from the lexer's position, just past its
    /// opening backtick (`head`) or its substitution's `}`, to its
    /// closing backtick or `${` (section 12.9.6).
    fn template(&mut self, start: usize, newline: bool, head: bool) -> Result<Token, SyntaxError> {
        let raw_start = self.pos;
        let mut cooked = Vec::new();
        let mut invalid: Option<SyntaxError> = None;
        let (raw_end, tail) = loop {
            match self.peek() {
                None => return Err(self.error(UNTERMINATED_TEMPLATE, start)),
                Some(b'`') => {
                    let end = self.pos;
                    self.pos += 1;
                    break (end, true);
                }
                Some(b'$') if self.peek_at(1) == Some(b'{') => {
                    let end = self.pos;
                    self.pos += 2;
                    break (end, false);
                }
                Some(b'\\') => match self.escape(&mut cooked, true) {
                    Ok(_) => {}
                    Err(error) => {
                        invalid.get_or_insert(error);
                    }
                },
                Some(b'\r') => {
                    // CR and CRLF are LF in both values (section 12.9.6).
                    self.pos += 1;
                    if self.peek() == Some(b'\n') {
                        self.pos += 1;
                    }
                    cooked.push(u16::from(b'\n'));
                }
                Some(b) if b < 0x80 => {
                    cooked.push(u16::from(b));
                    self.pos += 1;
                }
                Some(_) => {
                    let c = self.char_at(self.pos).unwrap_or('\u{FFFD}');
                    self.pos += c.len_utf8();
                    push_code_point(&mut cooked, u32::from(c));
                }
            }
        };
        let raw = match Cow::from(self.slice(raw_start, raw_end)) {
            raw if raw.contains('\r') => raw.replace("\r\n", "\n").replace('\r', "\n"),
            raw => raw.into_owned(),
        };
        let cooked = if invalid.is_some() {
            None
        } else {
            Some(cooked.into())
        };
        let mut token = self.token(
            TokenKind::Template { head, tail },
            TokenValue::Template {
                cooked,
                raw: raw.into(),
            },
            start,
            newline,
        );
        token.invalid_escape = invalid;
        Ok(token)
    }

    /// Reads again, as a template's middle or tail, from a `}` token the
    /// lexer gave (section 12: the InputElementTemplateTail goal), where
    /// it ends a template's substitution.
    pub fn rescan_template(&mut self, brace: &Token) -> Result<Token, SyntaxError> {
        let start = brace.start as usize;
        self.pos = start + 1;
        self.template(start, brace.newline_before, false)
    }

    /// Reads again, as a regular expression literal (section 12.9.5), from
    /// a `/` or `/=` token the lexer gave, where the parser expects an
    /// expression. Its body and flags are checked by the RegExp parser.
    pub fn rescan_regexp(&mut self, slash: &Token) -> Result<Token, SyntaxError> {
        let start = slash.start as usize;
        self.pos = start + 1;
        let mut class = false;
        let body_end = loop {
            match self.peek() {
                None | Some(b'\n' | b'\r') => return Err(self.error(UNTERMINATED_REGEXP, start)),
                Some(b'\\') => {
                    self.pos += 1;
                    match self.char_at(self.pos) {
                        Some(c) if !is_line_terminator(c) => self.pos += c.len_utf8(),
                        _ => return Err(self.error(UNTERMINATED_REGEXP, start)),
                    }
                }
                Some(b'[') => {
                    class = true;
                    self.pos += 1;
                }
                Some(b']') => {
                    class = false;
                    self.pos += 1;
                }
                Some(b'/') if !class => {
                    let end = self.pos;
                    self.pos += 1;
                    break end;
                }
                Some(b) if b < 0x80 => self.pos += 1,
                Some(_) => match self.char_at(self.pos) {
                    Some(c) if is_line_terminator(c) => {
                        return Err(self.error(UNTERMINATED_REGEXP, start));
                    }
                    Some(c) => self.pos += c.len_utf8(),
                    None => self.pos += 1,
                },
            }
        };
        let flags_start = self.pos;
        while let Some(b) = self.peek() {
            if b == b'\\' {
                self.pos += 1;
                return Err(self.error(BAD_REGEXP_FLAGS, flags_start));
            }
            let c = if b < 0x80 {
                b as char
            } else {
                match self.char_at(self.pos) {
                    Some(c) => c,
                    None => break,
                }
            };
            if !is_id_continue(c) {
                break;
            }
            self.pos += c.len_utf8();
        }
        Ok(self.token(
            TokenKind::RegExp,
            TokenValue::RegExp {
                body: self.slice(start + 1, body_end).into(),
                flags: self.slice(flags_start, self.pos).into(),
            },
            start,
            slash.newline_before,
        ))
    }
}

/// Pushes code point `code` as UTF-16: a pair past U+FFFF, else one unit,
/// a lone surrogate as itself.
fn push_code_point(out: &mut Vec<u16>, code: u32) {
    if code > 0xFFFF {
        let code = code - 0x10000;
        out.push(0xD800 | (code >> 10) as u16);
        out.push(0xDC00 | (code & 0x3FF) as u16);
    } else {
        out.push(code as u16);
    }
}

/// The value of `digits` in a radix of `2^bits`, rounded to the nearest
/// double, ties to even (section 12.9.3.3's MV, rounded as section 6.1.6.1
/// says).
fn pow2_radix_value(digits: &str, bits: u32) -> f64 {
    // At least 124 significant bits are kept, far more than a double's
    // 53 and its rounding bit; what does not fit counts in the exponent
    // and, if any of it is nonzero, sets a sticky bit.
    let mut mantissa: u128 = 0;
    let mut exponent: i64 = 0;
    let mut sticky = false;
    for d in digits.chars().filter_map(|c| c.to_digit(1 << bits)) {
        if mantissa >> (128 - bits) == 0 {
            mantissa = (mantissa << bits) | u128::from(d);
        } else {
            exponent += i64::from(bits);
            sticky |= d != 0;
        }
    }
    if sticky {
        mantissa |= 1;
    }
    // `as` rounds to nearest, ties to even; with the sticky bit below
    // the rounding bit, that is the exact value's rounding.
    let value = mantissa as f64;
    if exponent == 0 || value == 0.0 {
        return value;
    }
    // Here the mantissa holds over 120 bits, so the value is past 2^120
    // and the scale is exact until it overflows to infinity.
    match i32::try_from(exponent) {
        Ok(exponent) if exponent <= 1023 => {
            value * f64::from_bits(((exponent + 1023) as u64) << 52)
        }
        _ => f64::INFINITY,
    }
}

/// Line starts, for the line and column of a byte offset (1-based, the
/// column in UTF-16 units as the reference browser counts them).
#[derive(Debug, Clone)]
pub struct LineIndex {
    starts: Vec<u32>,
}

impl LineIndex {
    pub fn new(source: &str) -> LineIndex {
        let mut starts = vec![0];
        let mut chars = source.char_indices().peekable();
        while let Some((at, c)) = chars.next() {
            let end = match c {
                '\r' => match chars.peek() {
                    Some(&(next, '\n')) => {
                        chars.next();
                        next + 1
                    }
                    _ => at + 1,
                },
                '\n' => at + 1,
                LS | PS => at + c.len_utf8(),
                _ => continue,
            };
            starts.push(clamp(end));
        }
        LineIndex { starts }
    }

    /// The line and column of byte `offset` of `source`, both from 1.
    pub fn location(&self, source: &str, offset: u32) -> (u32, u32) {
        let line = self.starts.partition_point(|&start| start <= offset).max(1);
        let start = self.starts.get(line - 1).copied().unwrap_or(0) as usize;
        let text = source.get(start..offset as usize).unwrap_or("");
        let column = text.chars().map(char::len_utf16).sum::<usize>() + 1;
        (clamp(line), clamp(column))
    }
}
