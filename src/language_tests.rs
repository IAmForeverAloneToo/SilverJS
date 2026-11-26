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
