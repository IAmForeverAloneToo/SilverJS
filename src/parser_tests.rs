//! The parser's tests: sources that must parse, and sources each early
//! error must refuse, in scripts, strict code and modules.

use crate::ast::{Expr, Stmt};
use crate::parser::{Goal, parse, parse_with_stack};

fn ok(source: &str) {
    if let Err(e) = parse(source, Goal::Script) {
        panic!("{source:?} should parse: {e} at {}", e.start);
    }
}

fn ok_module(source: &str) {
    if let Err(e) = parse(source, Goal::Module) {
        panic!("{source:?} should parse as a module: {e} at {}", e.start);
    }
}

fn err_module(source: &str) {
    if parse(source, Goal::Module).is_ok() {
        panic!("{source:?} should be a SyntaxError in a module");
    }
}

fn err(source: &str) {
    if parse(source, Goal::Script).is_ok() {
        panic!("{source:?} should be a SyntaxError");
    }
}

fn err_strict(source: &str) {
    ok(source);
    err(&format!("'use strict'; {source}"));
}

#[test]
fn expressions_and_precedence() {
    ok("a + b * c - d / e % f ** g ** h;");
    ok("a ? b : c ? d : e;");
    ok("a = b = c += d;");
    ok("a ?? b; (a || b) ?? c; a ?? (b && c);");
    err("a ?? b || c;");
    err("a || b ?? c;");
    err("a && b ?? c;");
    ok("(-a) ** b; a ** -b;");
    err("-a ** b;");
    err("typeof a ** b;");
    ok("x = a instanceof b in c;");
    ok("new new a()(); new a.b.c; new (a())();");
    ok("a?.b?.[c]?.(d); a?.b.c();");
    err("a?.b`t`;");
    err("new a?.b();");
    err("a?.b = 1;");
    ok("`a${b}c${`d${e}`}f`; tag`\\unicode`;");
    err("`\\unicode`;");
    ok("x = /a[/]b/g; y = a / b / c;");
    err("x = /a/gg;");
    ok("a\n++b");
    err("a++\n++");
    ok("import('m'); import('m', { with: {} });");
    err("import.meta;");
    err("import a from 'm';");
}

#[test]
fn long_chains_do_not_recurse() {
    let sum = format!("x = {};", vec!["a"; 100_000].join(" + "));
    ok(&sum);
    let member = format!("x{};", ".a".repeat(100_000));
    ok(&member);
}

#[test]
fn arrows_and_their_parameters() {
    ok("x => x; (a, b) => a; () => {}; (a, ...b) => b; ([a], {b}) => 0;");
    ok("async x => x; async (a, b) => a; async () => {};");
    ok("(a = 1, {b = 2} = {}) => 0;");
    err("(a, a) => 0;");
    err("(a, b,) => 0; (...a,) => 0;");
    ok("(a, b,) => 0;");
    err("(...a, b) => 0;");
    err("x\n=> x;");
    err("() => {} + 1;");
    err("!() => 1;");
    ok("async(a, b);");
    err("async (await) => 0;");
    err("function* g() { (a = yield) => 0; }");
    err("async function f() { (a = await 1) => 0; }");
    err("(a.b) => 0;");
    err("((a)) => 0;");
    err("'use strict'; (eval) => 0;");
    err("() => { 'use strict'; }; (a = 1) => { 'use strict'; };");
}

#[test]
fn cover_grammars() {
    ok("({a = 1} = b);");
    ok("[{a = 1}] = b;");
    ok("({a = 1}) => a;");
    err("({a = 1});");
    err("({a = 1}).x;");
    err("({a = 1}.x = 1);");
    ok("({__proto__: a, __proto__: b} = c);");
    err("({__proto__: a, __proto__: b});");
    ok("({__proto__: a, ['__proto__']: b, __proto__() {}});");
    ok("[a, ...b] = c; ({a, ...b} = c);");
    err("[...a, b] = c;");
    err("[...a,] = c;");
    err("({...a,} = c);");
    err("({...{a}} = c);");
    ok("[...[a]] = c;");
    err("([a]) = 1;");
    ok("(a) = 1; (a.b) = 1;");
    err("(a, b) = 1;");
    err("[a + b] = c;");
    ok("f() = 1;");
    err("'use strict'; f() = 1;");
    err("f() &&= 1;");
    err("[f()] = 1;");
}

#[test]
fn declarations_and_scopes() {
    ok("var a; var a; function f() {} var f;");
    err("let a; let a;");
    err("let a; var a;");
    err("var a; let a;");
    err("{ var a; } let a;");
    err("let a; { var a; }");
    ok("let a; { let a; }");
    err("const a;");
    ok("let a;");
    err("let [a];");
    err("let let = 1;");
    err("let [a, a] = b;");
    err("function f(a) { let a; }");
    ok("function f(a) { var a; }");
    err("try {} catch (e) { let e; }");
    ok("try {} catch (e) { var e; }");
    err("try {} catch ([e]) { var e; }");
    ok("try {} catch (e) { for (var e of []); }");
    ok("try {} catch (e) { for (var e in {}); }");
    err("try {} catch ([e, e]) {}");
    ok("{ function f() {} function f() {} }");
    err("'use strict'; { function f() {} function f() {} }");
    err("{ function f() {} async function f() {} }");
    err("{ let f; function f() {} }");
    err("switch (0) { case 1: let a; case 2: let a; }");
    err("for (let a of b) { var a; }");
    ok("for (let a of b) { let a; }");
    err("label: let a;");
    err("if (1) let a;");
    ok("if (1) let\na;");
}

#[test]
fn functions_and_strictness() {
    ok("function f(a, a) {}");
    err("'use strict'; function f(a, a) {}");
    err("function f(a, a) { 'use strict'; }");
    err("function f(a = 1, a) {}");
    err("function f(a = 1) { 'use strict'; }");
    err("function eval() { 'use strict'; }");
    err("function f(eval) { 'use strict'; }");
    err("function f() { '\\01'; 'use strict'; }");
    err("function f() { 'use strict'; 010; }");
    ok("function f() { 010; }");
    err_strict("with (a) {}");
    err_strict("delete a;");
    ok("'use strict'; delete a.b;");
    err_strict("var yield;");
    err_strict("var implements;");
    err_strict("eval = 1;");
    err_strict("arguments++;");
    err("function* g() { var yield; }");
    err("async function f() { var await; }");
    ok("function* g() { yield; yield* a; yield\na; }");
    err("function* g(a = yield) {}");
    err("async function f(a = await 1) {}");
    err("return;");
    ok("function f() { return; }");
    ok("({ get a() {}, set a(v) {} });");
    err("({ get a(x) {} });");
    err("({ set a() {} });");
    err("({ set a(...v) {} });");
    err("new.target;");
    ok("function f() { new.target; () => new.target; }");
    err("function f() { super.x; }");
    ok("({ m() { super.x; } });");
    err("({ m() { super(); } });");
}

#[test]
fn statements_and_labels() {
    ok("a: for (;;) { break a; continue a; }");
    err("a: { continue a; }");
    ok("a: { break a; }");
    err("break;");
    err("continue;");
    ok("while (1) break; switch (0) { case 0: break; }");
    err("switch (0) { case 0: continue; }");
    err("a: a: ;");
    err("a: { b: { break c; } }");
    err("a: while (1) { function f() { break a; } }");
    ok("a: function f() {}");
    err("'use strict'; a: function f() {}");
    err("while (1) a: function f() {}");
    ok("if (1) function f() {}");
    err("if (1) function* f() {}");
    err("'use strict'; if (1) function f() {}");
    err("while (1) function f() {}");
    err("throw\na;");
    err("try {}");
    err("switch (0) { default: default: }");
    ok("do ; while (0) a;");
    ok("for (var a = 1 in b);");
    err("'use strict'; for (var a = 1 in b);");
    err("for (let a = 1 in b);");
    err("for (var a, b of c);");
    err("for (let of x);");
    err("for (async of x);");
    ok("async function f() { for await (async of x); }");
    err("for (a + b of c);");
    ok("for ([a, b] of c); for ({a} in b);");
    ok("for (let in x);");
    err("'use strict'; for (let in x);");
    ok("for (x in y in z);");
}

#[test]
fn the_tree_keeps_what_was_read() {
    let program = parse("a = 1 + 2;", Goal::Script).unwrap();
    let [stmt] = program.body.as_slice() else {
        panic!("one statement");
    };
    let Stmt::Expr(e) = &program.ast.stmt(*stmt).kind else {
        panic!("an expression statement");
    };
    let Expr::Assign { value, .. } = &program.ast.expr(*e).kind else {
        panic!("an assignment");
    };
    assert!(matches!(program.ast.expr(*value).kind, Expr::Binary { .. }));
    assert!(parse("'use strict';", Goal::Script).unwrap().strict);
    assert!(!parse("'use\\x20strict';", Goal::Script).unwrap().strict);
}

#[test]
fn rules_test262_held_us_to() {
    // "use strict" with non-simple parameters, even in strict code.
    err("'use strict'; function f(a = 1) { 'use strict'; }");
    err("'use strict'; ({ m([a]) { 'use strict'; } });");
    err("'use strict'; (...a) => { 'use strict'; };");
    // An async arrow's rest has no trailing comma.
    err("async (...a,) => 0;");
    // `using` in a for head, `using of` included; not in a case clause.
    ok("function f() { for (using x = null;;) break; for (using of = null;;) break; }");
    err("function f() { switch (0) { case 0: using x = null; } }");
    err("async function f() { switch (0) { default: await using x = null; } }");
    // `-->` on the first line is a comment (Annex B.1.1).
    ok("--> a comment");
}

#[test]
fn modules() {
    ok_module("import a, { b as c, 'd' as e } from 'm'; import * as ns from 'm'; import 'm';");
    ok_module(
        "export var a; export { a as b, a as 'c' }; export * from 'm'; export * as d from 'm';",
    );
    ok_module("export default function () {} export function f() {}");
    ok_module("export default class {}");
    ok_module("export default 1 + 2;");
    ok_module("import x from 'm' with { type: 'json' };");
    err_module("import x from 'm' with { type: 'json', type: 'json' };");
    err_module("export { a };");
    err_module("var a; export { a, a };");
    err_module("var a; export { a as b }; export { a as b };");
    err_module("export { 'a' };");
    ok_module("export { 'a' } from 'm';");
    // A string or keyword names no local binding to export.
    let message = |s: &str| parse(s, Goal::Module).map(|_| ()).unwrap_err().message;
    assert_eq!(message("var a; export { 'a' };"), "Unexpected string");
    assert_eq!(message("export { if };"), "Unexpected token 'if'");
    err_module("export * as '\\uD800' from 'm';");
    ok_module("export * as '\\uD83D\\uDE00' from 'm';");
    err_module("import { 'a' } from 'm';");
    ok_module("import { 'a' as b } from 'm';");
    err_module("{ import a from 'm'; }");
    err_module("await: ;");
    ok_module("await 1;");
    ok_module("import.meta;");
    err("import.meta;");
    err("import a from 'm';");
    ok("import('m'); import('m', { with: {} });");
    err_module("import { a } from 'm'; let a;");
    err_module("function f() {} var f;");
    err_module("010;");
    err_module("<!-- a");
}

#[test]
fn regexp_literals() {
    ok("/(?<a>x)|(?<a>y)/;");
    err("/(?<a>x)(?<a>y)/;");
    ok("/\\k<a>(?<a>x)/;");
    err("/(?<a>x)\\k<b>/;");
    ok("/\\k/;");
    err("/\\k/u;");
    ok("/{/; /a{/; /a{1/;");
    err("/{/u;");
    err("/a{2,1}/;");
    err("/+/;");
    ok("/(?=a)*/;");
    err("/(?=a)*/u;");
    err("/(?<=a)*/;");
    ok("/[a-b]/;");
    err("/[b-a]/;");
    // Outside Unicode mode an astral character is two code units: it
    // reads (it once looped), and a range across two is out of order.
    ok("/\u{20BB7}/; /\\\u{20BB7}/; /[\u{1F600}-\u{1F603}]/u;");
    err("/[\u{1F600}-\u{1F603}]/;");
    ok("/[\\d-a]/;");
    err("/[\\d-a]/u;");
    ok("/\\p{L}\\p{Script=Greek}\\P{ASCII}/u;");
    err("/\\p{Lx}/u;");
    err("/\\p{Script=Nope}/u;");
    ok("/\\p{L}/;");
    ok("/[\\p{L}--\\p{Lu}]/v; /[[a-z]&&[aeiou]]/v; /[\\q{abc|d}]/v;");
    err("/[^\\q{abc}]/v;");
    err("/[a&&&b]/v;");
    err("/[(]/v;");
    ok("/\\p{RGI_Emoji}/v;");
    err("/\\P{RGI_Emoji}/v;");
    err("/\\p{RGI_Emoji}/u;");
    ok("/(?i:a)(?-s:b)(?m-i:c)/;");
    err("/(?ii:a)/;");
    err("/(?-:a)/;");
    ok("/\\1(a)/; /\\8/;");
    err("/\\2(a)/u;");
    ok("/a/dgimsy; /a/v;");
    err("/a/uv;");
    // Unicode mode escapes only syntax characters and `/`, and has no
    // lone `]`; `v` mode refuses a doubled punctuator in a class.
    ok("/\\a/; /]/;");
    err("/\\a/u;");
    err("/]/u;");
    ok("/[a!b]/v;");
    err("/[a!!b]/v;");
}

#[test]
fn classes_and_private_names() {
    ok(
        "class C extends B { constructor() { super(); } #a; get #b() {} set #b(v) {} m() { this.#a; #a in this; } static { this; } }",
    );
    err("class C { constructor() { super(); } }");
    err("class C { constructor() {} constructor() {} }");
    err("class C { get constructor() {} }");
    err("class C { *constructor() {} }");
    err("class C { async constructor() {} }");
    ok("class C { static constructor() {} }");
    ok("class C { constructor() {} static constructor() {} static get constructor() {} }");
    err("class C { static prototype() {} }");
    err("class C { constructor = 1; }");
    err("class C { #constructor; }");
    err("class C { #a; #a; }");
    err("class C { get #a() {} static set #a(v) {} }");
    err("class C { m() { this.#b; } }");
    ok("class C { #a; m() { class D { n() { this.#a; } } } }");
    err("this.#a;");
    err("class C { #a; m() { delete this.#a; } }");
    err("class C { x = arguments; }");
    err("class C { static { arguments; } }");
    err("class C { static { await; } }");
    err("class C { m() { with (a) {} } }");
    err("class let {}");
    ok("class C { static async *m() {} get() {} set() {} static() {} async() {} }");
    ok("class C { a\n b }");
    err("class C { a b }");
    err("class C { #a; m() { 1 + #a in this; } }");
}

#[test]
fn class_rules_test262_held_us_to() {
    err("class C { static { return; } }");
    // An arrow is no class heritage.
    err("class C extends () => {} {}");
    // `super.x?.y` is fine; only `super?.` is not.
    ok("class C extends B { m() { super.a?.c; super.m?.(); } }");
    err("class C extends B { m() { super?.a; } }");
    // `get` on its own line before a generator is a field.
    ok("class C { get\n *a() {} static set\n *b() {} }");
    // A label named arguments in a class field's arrow.
    ok("class C { x = () => { arguments: for (;;) break arguments; }; }");
}

#[test]
fn operators_associate_as_the_grammar_says() {
    let shape = |source: &str| {
        let program = parse(source, Goal::Script).unwrap();
        let Some(Stmt::Expr(e)) = program.body.first().map(|s| &program.ast.stmt(*s).kind) else {
            panic!("an expression statement");
        };
        let Expr::Binary { left, right, .. } = &program.ast.expr(*e).kind else {
            panic!("a binary expression");
        };
        (
            matches!(program.ast.expr(*left).kind, Expr::Binary { .. }),
            matches!(program.ast.expr(*right).kind, Expr::Binary { .. }),
        )
    };
    // `**` groups to the right, `-` to the left.
    assert_eq!(shape("a ** b ** c;"), (false, true));
    assert_eq!(shape("a - b - c;"), (true, false));
}

#[test]
fn nesting_is_bounded_by_the_stack_budget() {
    // On a thread whose stack has room, so that a missing bound shows as
    // a parse that should have been refused, not as an overflow.
    let run = std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(|| {
            let deep = format!("{}1{}", "[".repeat(300), "]".repeat(300));
            // Far under the level count, past a small budget: refused.
            let small = parse_with_stack(&deep, Goal::Script, 16 << 10).is_err();
            // Within a budget the thread has room for: parsed.
            let roomy = parse_with_stack(&deep, Goal::Script, 32 << 20).is_ok();
            (small, roomy)
        })
        .unwrap();
    assert_eq!(run.join().unwrap(), (true, true));
}

#[test]
fn every_identifier_keeps_its_own_name() {
    // More names than the parser's cache of names has slots: some share
    // a slot, and each must still come out as written.
    let names: Vec<String> = (0..6000).map(|i| format!("n{i}")).collect();
    let source = format!("var {}; {};", names.join(", "), names.join(" + "));
    let program = parse(&source, Goal::Script).expect("parses");
    let declared: Vec<&str> = program
        .ast
        .pats
        .iter()
        .filter_map(|p| match &p.kind {
            crate::ast::Pat::Ident(i) => Some(&*i.name),
            _ => None,
        })
        .collect();
    let used: Vec<&str> = program
        .ast
        .exprs
        .iter()
        .filter_map(|e| match &e.kind {
            Expr::Ident(i) => Some(&*i.name),
            _ => None,
        })
        .collect();
    assert_eq!(declared, names);
    assert_eq!(used, names);
}

#[test]
fn a_template_after_a_name_is_read_again_past_its_brace() {
    // The name before `}` looks at the `}`; the template's next part is
    // read from that brace again, not from the token looked at.
    ok("`a${x}b${y}c${z}`");
    ok("f`a${x}b`; `${a}${b}${c}`");
    err("`a${x}b${y");
}

#[test]
fn nodes_span_their_source_to_their_last_token() {
    let source = "var a = 1;\nfoo(b, c)\nif (a) { b }\n(a + b);";
    let program = parse(source, Goal::Script).expect("parses");
    let spans: Vec<(u32, u32)> = program
        .body
        .iter()
        .map(|&s| {
            let span = program.ast.stmt(s).span;
            (span.start, span.end)
        })
        .collect();
    assert_eq!(spans, [(0, 10), (11, 20), (21, 33), (34, 42)]);
    let parens: Vec<(u32, u32)> = program
        .ast
        .exprs
        .iter()
        .filter(|e| matches!(e.kind, Expr::Paren(_)))
        .map(|e| (e.span.start, e.span.end))
        .collect();
    assert_eq!(parens, [(34, 41)]);
    assert_eq!(program.span.end, 42);
}
