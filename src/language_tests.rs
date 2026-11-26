//! J3.5a: tagged templates and their per-site template objects; Annex
//! B.3.2's functions in sloppy blocks, also a `var` of their function.

use crate::agent::{Agent, AgentOptions, Outcome};

fn run(src: &str) -> String {
    let mut agent = Agent::new(AgentOptions::default());
    agent.scope(|scope| match scope.run_script_named(src, "u.js") {
        Ok(Outcome::Value(v)) => scope.to_text(v).unwrap_or_default(),
        Ok(Outcome::Threw(e)) => format!("throw {}", scope.to_text(e).unwrap_or_default()),
        Ok(Outcome::Terminated) => "terminated".into(),
        Err(e) => format!("compile error: {e}"),
    })
}

#[test]
fn a_tagged_template_gets_its_sites_frozen_strings() {
    assert_eq!(
        run(
            "function tag(s) { return s; } function f() { return tag`a${1}b`; } \
             var a = f(), b = f(), c = tag`a${1}b`; \
             var d = Object.getOwnPropertyDescriptor(a, 'raw'); \
             var bad = tag`\\unicode and \\u{55}`; \
             var o = { m: function (s, x) { return this.k + s[0] + x; }, k: 'K' }; \
             [a === b, a === c, Object.isFrozen(a), Object.isFrozen(a.raw), a.length, a[0], a[1], \
             d.writable, d.enumerable, d.configurable, bad[0] === undefined, bad.raw[0], o.m`L${2}`, \
             (function (s, ...v) { return v.join('|') + ':' + s.join('|'); })`x${1}y${2}z`].join()"
        ),
        "true,false,true,true,2,a,b,false,false,false,true,\\unicode and \\u{55},KL2,1|2:x|y|z"
    );
}

#[test]
fn a_sloppy_blocks_function_is_also_a_var() {
    assert_eq!(
        run(
            "var out = [typeof f]; { function f() { return 'inner'; } } out.push(typeof f, f()); \
             function g() { var r = [typeof h]; if (true) { function h() {} } r.push(typeof h); return r.join('/'); } \
             function k() { let q = 1; { function q() {} } return typeof q; } \
             switch (1) { case 1: function sw() { return 'sw'; } } \
             function m(p) { { function p() {} } return typeof p; } \
             (function () { 'use strict'; { function s() {} } out.push(typeof s); })(); \
             (function () { { function t() { 'use strict'; } } out.push(typeof t); })(); \
             out.push(g(), k(), sw(), m(5)); out.join()"
        ),
        "undefined,function,inner,undefined,function,undefined/function,number,sw,number"
    );
}

/// J4.6: a lone surrogate in source made from a JavaScript string (`eval`,
/// `Function`) stays one in a string literal, a template (cooked and
/// raw), a regexp literal and `toString`'s text, beside a real U+FFFD or
/// a sixteenth-plane character; as an identifier it is refused (the J4.6
/// probe, Chrome 158).
#[test]
fn lone_surrogates_survive_eval_and_function() {
    assert_eq!(
        run(
            r#"var d = String.fromCharCode(0xD800), e = String.fromCharCode(0xDC00), r = String.fromCharCode(0xFFFD);
var out = [];
out.push(eval("'" + d + "'").charCodeAt(0).toString(16));
out.push((0, eval)("'a" + e + "b'").length + ',' + (0, eval)("'a" + e + "b'").charCodeAt(1).toString(16));
out.push(eval("`" + d + "`").charCodeAt(0).toString(16));
out.push(eval("String.raw`" + d + "`").charCodeAt(0).toString(16));
out.push(eval("/" + d + "/").source.charCodeAt(0).toString(16));
out.push(eval("/" + d + "/").test(d));
out.push(Function("return '" + e + "'")().charCodeAt(0).toString(16));
out.push(Function("return '" + e + "'").toString().indexOf(e) >= 0);
out.push(eval("'" + r + d + "'").charCodeAt(0).toString(16) + ',' + eval("'" + r + d + "'").charCodeAt(1).toString(16));
out.push(eval("'" + String.fromCodePoint(0x10FFFF) + d + "'").codePointAt(0).toString(16));
try { eval("var " + d + " = 1"); out.push('no error'); } catch (x) { out.push(x.name + ": " + x.message); }
out.join(' | ')"#
        ),
        r#"d800 | 3,dc00 | d800 | d800 | d800 | true | dc00 | true | fffd,d800 | 10ffff | SyntaxError: Invalid or unexpected token"#
    );
}

/// J4.6b (the J4.6 params probe, Chrome 158): a parameter of a list that
/// is not simple is uninitialized until its own element is bound, read
/// from its function or from a closure; an array pattern left by a
/// generator's `return()` at a `yield` in it closes its iterator, after
/// which a `finally` runs.
const PARAMS_PROBE: &str = r#"// SilverJS's J4.6b probe: default parameters' TDZ and the iterator
// close of an array pattern left by a generator's return(). Each is
// [label, function]; the page and SilverJS's shell run the same list.
function err(f) { try { return 'no throw: ' + String(f()); } catch (e) { return e.constructor.name + ': ' + e.message; } }
var cases = [
  ['ref later', function () { return err(function () { function f(x = y, y) {} f(); }); }],
  ['ref self', function () { return err(function () { function f(x = x) {} f(); }); }],
  ['ref later given', function () { return err(function () { function f(x = y, y) { return x; } return f(1); }); }],
  ['ref earlier', function () { return err(function () { function f(a, b = a) { return b; } return f(7); }); }],
  ['closure called early', function () { return err(function () { function f(a = (function () { return b; })(), b) {} f(); }); }],
  ['closure called late', function () { return err(function () { function f(a = function () { return b; }, b = 2) { return a(); } return f(); }); }],
  ['assign later', function () { return err(function () { function f(a = (b = 3), b) {} f(); }); }],
  ['pattern ref later', function () { return err(function () { function f({ p = q }, q) {} f({}); }); }],
  ['rest after default', function () { return err(function () { function f(a = 1, ...r) { return a + r.length; } return f(undefined, 1, 2); }); }],
  ['arrow ref later', function () { return err(function () { var f = (x = y, y) => 0; f(); }); }],
  ['typeof later', function () { return err(function () { function f(x = typeof y, y) {} f(); }); }],
  ['close on return', function () {
    var log = [];
    var it = { [Symbol.iterator]() { return this; }, next() { log.push('next'); return { done: false }; }, return() { log.push('return:' + arguments.length); return {}; } };
    function* g() { var a; [a = yield] = it; log.push('after'); }
    var gen = g(); gen.next(); var r = gen.return(5);
    return log.join() + ' ' + JSON.stringify(r);
  }],
  ['close on return null', function () {
    var it = { [Symbol.iterator]() { return this; }, next() { return { done: false }; }, return() { return null; } };
    function* g() { [{}[yield]] = it; }
    var gen = g(); gen.next();
    return err(function () { return JSON.stringify(gen.return(5)); });
  }],
  ['close on return throws', function () {
    var it = { [Symbol.iterator]() { return this; }, next() { return { done: false }; }, return() { throw new RangeError('r'); } };
    function* g() { [...{}[yield]] = it; }
    var gen = g(); gen.next();
    return err(function () { return JSON.stringify(gen.return(5)); });
  }],
  ['done iterator not closed', function () {
    var n = 0;
    var it = { [Symbol.iterator]() { return this; }, next() { return { done: true }; }, return() { n++; return {}; } };
    function* g() { var a, b; [a, b = yield] = it; }
    var gen = g(); gen.next(); gen.return(1);
    return n;
  }],
  ['finally then close', function () {
    var log = [];
    var it = { [Symbol.iterator]() { return this; }, next() { return { done: false }; }, return() { log.push('close'); return {}; } };
    function* g() { try { [{}[yield]] = it; } finally { log.push('finally'); } }
    var gen = g(); gen.next(); gen.return(1);
    return log.join();
  }]
];
function run() { return cases.map(function (c) { var r; try { r = c[1](); } catch (e) { r = 'threw ' + e; } return c[0] + ': ' + r; }); }
run().join('\n')"#;

#[test]
fn parameters_and_patterns_read_as_the_reference_browser_gives_them() {
    let want = [
        r#"ref later: ReferenceError: Cannot access 'y' before initialization"#,
        r#"ref self: ReferenceError: Cannot access 'x' before initialization"#,
        r#"ref later given: no throw: 1"#,
        r#"ref earlier: no throw: 7"#,
        r#"closure called early: ReferenceError: Cannot access 'b' before initialization"#,
        r#"closure called late: no throw: 2"#,
        r#"assign later: ReferenceError: Cannot access 'b' before initialization"#,
        r#"pattern ref later: ReferenceError: Cannot access 'q' before initialization"#,
        r#"rest after default: no throw: 3"#,
        r#"arrow ref later: ReferenceError: Cannot access 'y' before initialization"#,
        r#"typeof later: ReferenceError: Cannot access 'y' before initialization"#,
        r#"close on return: next,return:0 {"value":5,"done":true}"#,
        r#"close on return null: TypeError: Iterator result null is not an object"#,
        r#"close on return throws: RangeError: r"#,
        r#"done iterator not closed: 0"#,
        r#"finally then close: close,finally"#,
    ];
    let got = run(PARAMS_PROBE);
    let got: Vec<&str> = got.split('\n').collect();
    assert_eq!(got, want);
}
