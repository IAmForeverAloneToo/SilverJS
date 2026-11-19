//! The parser's tests: sources that must parse, and sources each early
//! error must refuse, in scripts and strict code.

use crate::ast::{Expr, Stmt};
use crate::parser::{Goal, parse, parse_with_stack};

fn ok(source: &str) {
    if let Err(e) = parse(source, Goal::Script) {
        panic!("{source:?} should parse: {e} at {}", e.start);
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
fn what_later_steps_bring_is_refused_for_now() {
    err("class C {}");
    err("(class {});");
    err("this.#a;");
    assert!(parse("1;", Goal::Module).is_err());
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
