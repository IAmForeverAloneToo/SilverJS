//! The lexer's tests: each case is what ECMA-262 section 12 (and Annex
//! B.1.1) says the source is, token by token.

use crate::lexer::{Keyword, Lexer, LineIndex, Punct, Token, TokenKind, TokenValue};
use crate::unicode::ZWNJ;

/// Every token of `source` read the dividing way, to the end.
fn tokens(source: &str) -> Vec<Token> {
    tokens_of(source, false)
}

fn tokens_of(source: &str, module: bool) -> Vec<Token> {
    let mut lexer = Lexer::new(source, module).expect("a lexer");
    let mut out = Vec::new();
    loop {
        let token = lexer
            .next_token()
            .unwrap_or_else(|e| panic!("{source:?}: {e}"));
        if token.kind == TokenKind::Eof {
            return out;
        }
        out.push(token);
    }
}

fn kinds(source: &str) -> Vec<TokenKind> {
    tokens(source).into_iter().map(|t| t.kind).collect()
}

/// The first error lexing `source` reaches.
fn error(source: &str) -> String {
    let mut lexer = Lexer::new(source, false).expect("a lexer");
    loop {
        match lexer.next_token() {
            Ok(token) if token.kind == TokenKind::Eof => panic!("{source:?} lexed"),
            Ok(_) => {}
            Err(e) => return e.message,
        }
    }
}

fn one(source: &str) -> Token {
    let mut all = tokens(source);
    assert_eq!(all.len(), 1, "{source:?}: {all:?}");
    all.remove(0)
}

fn number(source: &str) -> f64 {
    match one(source).value {
        TokenValue::Number(n) => n,
        other => panic!("{source:?}: {other:?}"),
    }
}

fn string(source: &str) -> Vec<u16> {
    match one(source).value {
        TokenValue::String(s) => s.into_vec(),
        other => panic!("{source:?}: {other:?}"),
    }
}

fn utf16(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

#[test]
fn punctuators_are_read_longest_first() {
    use Punct::*;
    let expected = [
        UShrEq,
        UShr,
        ShrEq,
        Shr,
        GtEq,
        Gt,
        ShlEq,
        Shl,
        LtEq,
        Lt,
        EqEqEq,
        EqEq,
        Arrow,
        Eq,
        NotEqEq,
        NotEq,
        Bang,
        StarStarEq,
        StarStar,
        StarEq,
        Star,
        AmpAmpEq,
        AmpAmp,
        AmpEq,
        Amp,
        PipePipeEq,
        PipePipe,
        PipeEq,
        Pipe,
        QuestionQuestionEq,
        QuestionQuestion,
        QuestionDot,
        Question,
        Ellipsis,
        Dot,
        PlusPlus,
        PlusEq,
        Plus,
        MinusMinus,
        MinusEq,
        Minus,
        PercentEq,
        Percent,
        CaretEq,
        Caret,
        Tilde,
        Colon,
        Semicolon,
        Comma,
        LParen,
        RParen,
        LBracket,
        RBracket,
        LBrace,
        RBrace,
    ];
    let source = ">>>= >>> >>= >> >= > <<= << <= < === == => = !== != ! **= ** *= * \
                  &&= && &= & ||= || |= | ??= ?? ?. ? ... . ++ += + -- -= - %= % ^= ^ ~ : ; , \
                  ( ) [ ] { }";
    let got: Vec<TokenKind> = kinds(source);
    let want: Vec<TokenKind> = expected.iter().map(|&p| TokenKind::Punct(p)).collect();
    assert_eq!(got, want);
}

#[test]
fn question_dot_before_a_digit_is_a_conditional() {
    assert_eq!(
        kinds("a?.5:b"),
        vec![
            TokenKind::Identifier,
            TokenKind::Punct(Punct::Question),
            TokenKind::Number,
            TokenKind::Punct(Punct::Colon),
            TokenKind::Identifier,
        ]
    );
    assert_eq!(kinds("a?.b")[1], TokenKind::Punct(Punct::QuestionDot));
}

#[test]
fn a_slash_is_division_until_the_parser_rescans_it() {
    assert_eq!(kinds("a / b")[1], TokenKind::Punct(Punct::Slash));
    assert_eq!(kinds("a /= b")[1], TokenKind::Punct(Punct::SlashEq));
    let source = "x = /[/]\\/a/gu;";
    let mut lexer = Lexer::new(source, false).unwrap();
    lexer.next_token().unwrap();
    lexer.next_token().unwrap();
    let slash = lexer.next_token().unwrap();
    assert!(slash.is(Punct::Slash));
    let regexp = lexer.rescan_regexp(&slash).unwrap();
    assert_eq!(regexp.kind, TokenKind::RegExp);
    assert_eq!(
        regexp.value,
        TokenValue::RegExp {
            body: "[/]\\/a".into(),
            flags: "gu".into()
        }
    );
    assert!(lexer.next_token().unwrap().is(Punct::Semicolon));
    // `/=` rescanned keeps its `=` in the body.
    let mut lexer = Lexer::new("/=a/", false).unwrap();
    let slash = lexer.next_token().unwrap();
    let regexp = lexer.rescan_regexp(&slash).unwrap();
    assert_eq!(
        regexp.value,
        TokenValue::RegExp {
            body: "=a".into(),
            flags: "".into()
        }
    );
}

#[test]
fn a_regexp_ends_on_its_line() {
    for source in ["/a\n/", "/a", "/[/\n]/", "/a\\\n/", "/a\u{2028}/"] {
        let mut lexer = Lexer::new(source, false).unwrap();
        let slash = lexer.next_token().unwrap();
        let e = lexer.rescan_regexp(&slash).unwrap_err();
        assert_eq!(
            e.message, "Invalid regular expression: missing /",
            "{source:?}"
        );
    }
    let mut lexer = Lexer::new("/a/\\u0067", false).unwrap();
    let slash = lexer.next_token().unwrap();
    assert_eq!(
        lexer.rescan_regexp(&slash).unwrap_err().message,
        "Invalid regular expression flags"
    );
}

#[test]
fn keywords_identifiers_and_escapes() {
    let all = tokens("if iff \\u0069f \\u{69}ff $_a1 _ caf\\u00e9 await yield let");
    assert_eq!(all[0].kind, TokenKind::Keyword(Keyword::If));
    assert_eq!(all[1].kind, TokenKind::Identifier);
    // An escaped reserved word is an identifier token marked escaped: the
    // parser refuses it as either.
    assert_eq!(all[2].kind, TokenKind::Identifier);
    assert!(all[2].escaped);
    assert_eq!(all[2].value, TokenValue::Name("if".into()));
    assert_eq!(all[3].value, TokenValue::Name("iff".into()));
    assert!(!all[4].escaped);
    let source = "if iff \\u0069f \\u{69}ff $_a1 _ caf\\u00e9 await yield let";
    assert_eq!(all[4].name(source), "$_a1");
    assert_eq!(all[6].name(source), "caf\u{e9}");
    for t in &all[7..] {
        assert_eq!(t.kind, TokenKind::Identifier);
    }
}

#[test]
fn identifiers_are_unicodes_id_start_and_id_continue() {
    let source = format!("\u{3b1}\u{3b2} \u{4e00}x \u{1d4d0} a\u{301} a{}b", ZWNJ);
    let all = tokens(&source);
    assert_eq!(all.len(), 5);
    assert!(all.iter().all(|t| t.kind == TokenKind::Identifier));
    assert_eq!(all[4].name(&source), format!("a{}b", ZWNJ));
    // An escape must itself be an identifier's character.
    assert_eq!(error("\\u0031a"), "Invalid Unicode escape sequence");
    assert_eq!(error("a\\u002D"), "Invalid Unicode escape sequence");
    assert_eq!(error("\\u{110000}"), "Undefined Unicode code-point");
    assert_eq!(error("\\ud800"), "Invalid Unicode escape sequence");
    // A character that starts no token.
    assert_eq!(error("\u{1f600}"), "Invalid or unexpected token");
    assert_eq!(error("@"), "Invalid or unexpected token");
}

#[test]
fn private_names() {
    let source = "#x #\\u{61}b";
    let all = tokens(source);
    assert_eq!(all[0].kind, TokenKind::PrivateName);
    assert_eq!(all[0].name(source), "x");
    assert_eq!(all[1].name(source), "ab");
    assert!(all[1].escaped);
    assert_eq!(error("# x"), "Invalid or unexpected token");
}

#[test]
fn decimal_numbers() {
    assert_eq!(number("0"), 0.0);
    assert_eq!(number("42"), 42.0);
    assert_eq!(number("1.5"), 1.5);
    assert_eq!(number(".5"), 0.5);
    assert_eq!(number("5."), 5.0);
    assert_eq!(number("1e3"), 1000.0);
    assert_eq!(number("1.e3"), 1000.0);
    assert_eq!(number("2E-2"), 0.02);
    assert_eq!(number("1_000_000.000_1"), 1_000_000.000_1);
    assert_eq!(number("1e1_0"), 1e10);
    assert_eq!(number("0.1"), 0.1);
    assert_eq!(number("1e400"), f64::INFINITY);
    assert_eq!(number("9007199254740993"), 9007199254740992.0);
    // `1..x` is `1.` and `.x`.
    assert_eq!(kinds("1..x").len(), 3);
}

#[test]
fn radix_numbers_round_to_nearest_even() {
    assert_eq!(number("0x10"), 16.0);
    assert_eq!(number("0XfF"), 255.0);
    assert_eq!(number("0o17"), 15.0);
    assert_eq!(number("0b1010"), 10.0);
    assert_eq!(number("0xF_F"), 255.0);
    // 2^53 + 1 rounds down to even, 2^53 + 3 up.
    assert_eq!(number("0x20000000000001"), 9007199254740992.0);
    assert_eq!(number("0x20000000000003"), 9007199254740996.0);
    // A sticky bit far below the rounding bit breaks the tie upward.
    let tie_up = format!("0x20000000000001{}1", "0".repeat(40));
    let exact = 9007199254740994.0 * 16f64.powi(41);
    assert_eq!(number(&tie_up), exact);
    assert_eq!(number(&format!("0x1{}", "0".repeat(300))), f64::INFINITY);
    assert_eq!(number(&format!("0x{}", "0".repeat(300))), 0.0);
}

#[test]
fn legacy_octal_and_leading_zero_decimals_are_marked_for_strict_code() {
    let t = one("017");
    assert_eq!(t.value, TokenValue::Number(15.0));
    assert_eq!(
        t.strict_error.unwrap().message,
        "Octal literals are not allowed in strict mode."
    );
    let t = one("08");
    assert_eq!(t.value, TokenValue::Number(8.0));
    assert_eq!(
        t.strict_error.unwrap().message,
        "Decimals with leading zeros are not allowed in strict mode."
    );
    assert_eq!(number("09.5"), 9.5);
    assert!(one("0").strict_error.is_none());
    assert!(one("0.5").strict_error.is_none());
}

#[test]
fn bigints() {
    let t = one("123n");
    assert_eq!(t.kind, TokenKind::BigInt);
    assert_eq!(
        t.value,
        TokenValue::BigInt {
            radix: 10,
            digits: "123".into()
        }
    );
    assert_eq!(
        one("0x1_fn").value,
        TokenValue::BigInt {
            radix: 16,
            digits: "1f".into()
        }
    );
    assert_eq!(one("0n").kind, TokenKind::BigInt);
    for bad in ["1.5n", "1e3n", "017n", "08n", ".5n"] {
        assert_eq!(error(bad), "Invalid or unexpected token", "{bad}");
    }
}

#[test]
fn numeric_separators_stand_between_digits_only() {
    assert_eq!(
        error("1__0"),
        "Only one underscore is allowed as numeric separator"
    );
    assert_eq!(
        error("1_"),
        "Numeric separators are not allowed at the end of numeric literals"
    );
    assert_eq!(
        error("0_1"),
        "Numeric separator can not be used after leading 0."
    );
    assert_eq!(
        error("01_1"),
        "Numeric separator can not be used after leading 0."
    );
    for bad in ["0x_1", "1._5", "1e_5"] {
        assert_eq!(
            error(bad),
            "Numeric separators are not allowed here",
            "{bad}"
        );
    }
}

#[test]
fn a_number_is_not_followed_by_an_identifier_or_digit() {
    for bad in ["3in", "0x1g", "1\\u0061", "0b12", "5.toString"] {
        assert_eq!(error(bad), "Invalid or unexpected token", "{bad}");
    }
    assert_eq!(error("0x"), "Invalid or unexpected token");
    assert_eq!(error("1e"), "Invalid or unexpected token");
}

#[test]
fn string_escapes() {
    assert_eq!(
        string(r#""a\b\t\n\v\f\r\"\'\\z""#),
        utf16("a\u{8}\t\n\u{b}\u{c}\r\"'\\z")
    );
    assert_eq!(string(r"'\x41B\u{43}\u{1F600}'"), utf16("ABC\u{1F600}"));
    // A lone surrogate is a value of its own.
    assert_eq!(string(r"'\uD800'"), vec![0xD800]);
    // A line continuation is nothing; LS and PS may stand in a string.
    assert_eq!(string("'a\\\r\nb'"), utf16("ab"));
    assert_eq!(string("'a\\\u{2028}b'"), utf16("ab"));
    assert_eq!(string("'a\u{2029}b'"), utf16("a\u{2029}b"));
    assert_eq!(string(r"'\0'"), vec![0]);
    assert_eq!(string(r"'\q'"), utf16("q"));
    assert_eq!(error("'a\nb'"), "Invalid or unexpected token");
    assert_eq!(error("'abc"), "Invalid or unexpected token");
    assert_eq!(error(r"'\x4'"), "Invalid hexadecimal escape sequence");
    assert_eq!(error(r"'\u004'"), "Invalid Unicode escape sequence");
    assert_eq!(error(r"'\u{110000}'"), "Undefined Unicode code-point");
}

#[test]
fn legacy_octal_escapes_are_marked_for_strict_code() {
    let t = one(r"'\101\08\3777\8'");
    assert_eq!(
        t.value,
        TokenValue::String(
            vec![
                0o101,
                0,
                u16::from(b'8'),
                0o377,
                u16::from(b'7'),
                u16::from(b'8')
            ]
            .into()
        )
    );
    assert_eq!(
        t.strict_error.unwrap().message,
        "Octal escape sequences are not allowed in strict mode."
    );
    let t = one(r"'\9'");
    assert_eq!(
        t.strict_error.unwrap().message,
        "\\8 and \\9 are not allowed in strict mode."
    );
    assert!(one(r"'\0a'").strict_error.is_none());
}

#[test]
fn templates_cooked_and_raw() {
    let t = one("`a\\n${");
    assert_eq!(
        t.kind,
        TokenKind::Template {
            head: true,
            tail: false
        }
    );
    assert_eq!(
        t.value,
        TokenValue::Template {
            cooked: Some(utf16("a\n").into()),
            raw: "a\\n".into()
        }
    );
    // CR and CRLF are LF in both values.
    let t = one("`a\r\nb\rc`");
    assert_eq!(
        t.kind,
        TokenKind::Template {
            head: true,
            tail: true
        }
    );
    assert_eq!(
        t.value,
        TokenValue::Template {
            cooked: Some(utf16("a\nb\nc").into()),
            raw: "a\nb\nc".into()
        }
    );
    assert_eq!(error("`abc"), "Unterminated template literal");
}

#[test]
fn a_templates_invalid_escape_leaves_it_uncooked() {
    for (source, message) in [
        (
            "`\\01`",
            "Octal escape sequences are not allowed in template strings",
        ),
        ("`\\9`", "\\8 and \\9 are not allowed in template strings"),
        ("`\\xg`", "Invalid hexadecimal escape sequence"),
        ("`\\u{`", "Invalid Unicode escape sequence"),
        ("`\\u{110000}`", "Undefined Unicode code-point"),
    ] {
        let t = one(source);
        assert_eq!(
            t.kind,
            TokenKind::Template {
                head: true,
                tail: true
            },
            "{source:?}"
        );
        let TokenValue::Template { cooked, raw } = &t.value else {
            panic!("{source:?}");
        };
        assert!(cooked.is_none(), "{source:?}");
        assert_eq!(**raw, source[1..source.len() - 1], "{source:?}");
        assert_eq!(
            t.invalid_escape.as_ref().unwrap().message,
            message,
            "{source:?}"
        );
    }
    // `\0` alone is valid.
    assert!(one("`\\0`").invalid_escape.is_none());
}

#[test]
fn a_substitutions_brace_is_rescanned_as_the_templates_rest() {
    let source = "`a${x}b${y}c`";
    let mut lexer = Lexer::new(source, false).unwrap();
    let head = lexer.next_token().unwrap();
    assert_eq!(
        head.kind,
        TokenKind::Template {
            head: true,
            tail: false
        }
    );
    assert_eq!(lexer.next_token().unwrap().kind, TokenKind::Identifier);
    let brace = lexer.next_token().unwrap();
    assert!(brace.is(Punct::RBrace));
    let middle = lexer.rescan_template(&brace).unwrap();
    assert_eq!(
        middle.kind,
        TokenKind::Template {
            head: false,
            tail: false
        }
    );
    assert_eq!(
        middle.value,
        TokenValue::Template {
            cooked: Some(utf16("b").into()),
            raw: "b".into()
        }
    );
    lexer.next_token().unwrap();
    let brace = lexer.next_token().unwrap();
    let tail = lexer.rescan_template(&brace).unwrap();
    assert_eq!(
        tail.kind,
        TokenKind::Template {
            head: false,
            tail: true
        }
    );
    assert_eq!(lexer.next_token().unwrap().kind, TokenKind::Eof);
}

#[test]
fn newlines_before_tokens_in_and_out_of_comments() {
    let all = tokens("a\nb /* x */ c /*\n*/ d // e\nf \u{2028} g\r\nh");
    let newlines: Vec<bool> = all.iter().map(|t| t.newline_before).collect();
    assert_eq!(newlines, vec![false, true, false, true, true, true, true]);
    assert_eq!(error("a /* b"), "Invalid or unexpected token");
}

#[test]
fn white_space_includes_unicodes_space_separators_and_zwnbsp() {
    assert_eq!(kinds("a\u{a0}\u{3000}\u{feff}\u{b}\u{c}\tb").len(), 2);
}

#[test]
fn a_hashbang_is_a_comment_at_the_very_start_only() {
    assert_eq!(kinds("#!/usr/bin/env node\nx"), vec![TokenKind::Identifier]);
    assert!(tokens("#!x\ny")[0].newline_before);
    assert_eq!(error(" #!x"), "Invalid or unexpected token");
    assert_eq!(tokens_of("#!x\ny", true).len(), 1);
}

#[test]
fn html_like_comments_in_scripts_not_modules() {
    // `<!--` begins a comment anywhere in a script.
    assert_eq!(kinds("a <!-- b\nc").len(), 2);
    // `-->` only first on its line, after white space and comments of
    // one line.
    assert_eq!(kinds("a\n --> b\nc").len(), 2);
    assert_eq!(kinds("a\n/* x */ /* y */ --> b\nc").len(), 2);
    assert_eq!(kinds("a /*\n*/ --> b\nc").len(), 2);
    // Elsewhere it is `--` and `>`.
    assert_eq!(
        kinds("a --> b"),
        vec![
            TokenKind::Identifier,
            TokenKind::Punct(Punct::MinusMinus),
            TokenKind::Punct(Punct::Gt),
            TokenKind::Identifier,
        ]
    );
    // Not at the source's start (Annex B.1.1 asks a line terminator).
    assert_eq!(kinds("-->").len(), 2);
    // In a module neither is a comment.
    assert_eq!(tokens_of("a <!-- b", true).len(), 5);
    assert_eq!(tokens_of("a\n--> b", true).len(), 4);
}

#[test]
fn reset_reads_again_from_a_tokens_end() {
    let mut lexer = Lexer::new("a b c", false).unwrap();
    lexer.next_token().unwrap();
    let at = lexer.position();
    let b = lexer.next_token().unwrap();
    lexer.next_token().unwrap();
    lexer.reset(at);
    assert_eq!(lexer.next_token().unwrap(), b);
}

#[test]
fn line_and_column_count_utf16_units_from_one() {
    let source = "ab\n\u{1F600}x\r\ny\u{2028}z";
    let index = LineIndex::new(source);
    assert_eq!(index.location(source, 0), (1, 1));
    assert_eq!(index.location(source, 1), (1, 2));
    let x = source.find('x').unwrap() as u32;
    // The emoji is two UTF-16 units.
    assert_eq!(index.location(source, x), (2, 3));
    let y = source.find('y').unwrap() as u32;
    assert_eq!(index.location(source, y), (3, 1));
    let z = source.find('z').unwrap() as u32;
    assert_eq!(index.location(source, z), (4, 1));
}

#[test]
fn errors_name_their_bytes() {
    let mut lexer = Lexer::new("a 'bc", false).unwrap();
    lexer.next_token().unwrap();
    let e = lexer.next_token().unwrap_err();
    assert_eq!((e.start, e.end), (2, 5));
}
