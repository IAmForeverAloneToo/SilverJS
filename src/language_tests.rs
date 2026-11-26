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

/// J4.6a (the lead's check): a source with a lone surrogate and a
/// character in each of the 32 stand-in windows is refused with a
/// SyntaxError, never read with a real character in the surrogate's
/// place; with one window left free, every character keeps its value.
#[test]
fn a_source_using_every_stand_in_window_is_a_syntax_error() {
    assert_eq!(
        run(r#"var d = String.fromCharCode(0xD800), out = [];
function chars(n) { var s = ''; for (var i = 0; i < n; i++) s += String.fromCodePoint(0x100000 + i * 0x800 + 0x7FF); return s; }
function tryit(f) { try { return f(); } catch (x) { return x.name + ': ' + x.message; } }
out.push(tryit(function () { return eval("'" + chars(32) + d + "'").length; }));
out.push(tryit(function () { return (0, eval)("'" + d + chars(32) + "'").length; }));
out.push(tryit(function () { return Function("return '" + chars(32) + d + "'")().length; }));
out.push(tryit(function () { return Function("a", "return a + '" + d + "'")(chars(32)).length; }));
var s = eval("'" + chars(31) + d + "'");
out.push(s.length + ',' + (s.slice(0, 62) === chars(31)) + ',' + s.charCodeAt(62).toString(16));
out.push(eval("'" + chars(32) + "'") === chars(32));
out.join(' | ')"#),
        "SyntaxError: Lone surrogates in this source have no free stand-in window | \
         SyntaxError: Lone surrogates in this source have no free stand-in window | \
         SyntaxError: Lone surrogates in this source have no free stand-in window | \
         65 | 63,true,d800 | true"
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

/// J4.6c (the J4.6 refs probe, Chrome 158): the order in which an
/// assignment, a compound assignment and an update evaluate their
/// reference, in and out of a `with` scope, as the reference browser
/// does it rather than as the specification says.
const REFS_PROBE: &str = r#"// SilverJS's J4.6c probe: the order of a reference's evaluation in an
// assignment, a compound assignment and an update, in and out of a
// `with` scope. Each is [label, function]; the page and SilverJS's shell
// run the same list. Sloppy script code, as `with` needs.
function err(f) { try { return 'no throw: ' + String(f()); } catch (e) { return e.constructor.name + ': ' + e.message; } }
function show(v) { return v === undefined ? 'undefined' : JSON.stringify(v); }
var cases = [
  ['with getter deletes, compound', function () {
    var x = 0;
    var scope = { get x() { delete this.x; return 2; } };
    with (scope) { x *= 3; }
    return show(scope.x) + ' ' + x;
  }],
  ['with delete in rhs, simple', function () {
    var x = 0;
    var scope = { x: 1 };
    with (scope) { x = (delete scope.x, 2); }
    return show(scope.x) + ' ' + x;
  }],
  ['with delete in rhs, compound', function () {
    var x = 0;
    var scope = { x: 1 };
    with (scope) { x += (delete scope.x, 2); }
    return show(scope.x) + ' ' + x;
  }],
  ['with delete in rhs, strict function', function () {
    var scope = { x: 1 };
    var r;
    with (scope) { r = err(function () { 'use strict'; x = (delete scope.x, 2); }); }
    return r + ' ' + show(scope.x);
  }],
  ['eval var in rhs, compound', function () {
    var x = 3;
    var inner = (function () { x *= (eval('var x = 2;'), 4); return x; })();
    return inner + ' ' + x;
  }],
  ['eval var in rhs, simple', function () {
    var x = 3;
    var inner = (function () { x = (eval('var x = 2;'), 4); return x; })();
    return inner + ' ' + x;
  }],
  ['eval var in rhs, update', function () {
    var log = [];
    var x = 3;
    var inner = (function () { x += (eval('var x = 2;'), 1); log.push(x); x++; return x; })();
    return inner + ' ' + x + ' ' + log;
  }],
  ['null base key throws first', function () {
    var base = null;
    return err(function () { base[(function () { throw new RangeError('key'); })()] *= (function () { throw new Error('rhs'); })(); });
  }],
  ['null base key not converted', function () {
    var base = null;
    var log = [];
    var r = err(function () { base[{ toString: function () { log.push('key'); return 'p'; } }] *= (function () { log.push('rhs'); return 1; })(); });
    return r + ' [' + log + ']';
  }],
  ['null base simple assignment', function () {
    var base = null;
    var log = [];
    var r = err(function () { base[{ toString: function () { log.push('key'); return 'p'; } }] = (function () { log.push('rhs'); return 1; })(); });
    return r + ' [' + log + ']';
  }],
  ['key converted once, compound', function () {
    var n = 0;
    var base = { p: 2 };
    base[{ toString: function () { n++; return 'p'; } }] += 3;
    return n + ' ' + base.p;
  }],
  ['key converted once, update', function () {
    var n = 0;
    var base = { p: 2 };
    base[{ toString: function () { n++; return 'p'; } }]++;
    return n + ' ' + base.p;
  }],
  ['key converted once, logical', function () {
    var n = 0;
    var base = { p: 0 };
    base[{ toString: function () { n++; return 'p'; } }] ||= 3;
    return n + ' ' + base.p;
  }],
  ['key order against rhs, simple', function () {
    var log = [];
    var base = {};
    base[{ toString: function () { log.push('key'); return 'p'; } }] = (log.push('rhs'), 1);
    return log.join();
  }],
  ['key order against rhs, compound', function () {
    var log = [];
    var base = { p: 1 };
    base[{ toString: function () { log.push('key'); return 'p'; } }] += (log.push('rhs'), 1);
    return log.join();
  }],
  ['unscopables once, update', function () {
    var n = 0, flag = true, a, b;
    with (a = { x: 7 }) {
      with (b = { x: 4, get [Symbol.unscopables]() { n++; return { x: flag = !flag }; } }) { x++; }
    }
    return n + ' ' + a.x + ' ' + b.x;
  }],
  ['unscopables once, compound', function () {
    var n = 0, flag = true, a, b;
    with (a = { x: 7 }) {
      with (b = { x: 4, get [Symbol.unscopables]() { n++; return { x: flag = !flag }; } }) { x += 1; }
    }
    return n + ' ' + a.x + ' ' + b.x;
  }],
  ['unscopables getter deletes, strict set', function () {
    var n = 0;
    var env = { binding: 0, get [Symbol.unscopables]() { n++; delete env.binding; return null; } };
    var r;
    with (env) { r = err(function () { 'use strict'; binding = 123; }); }
    return r + ' ' + n + ' ' + show(Object.getOwnPropertyDescriptor(env, 'binding'));
  }],
  ['unscopables getter deletes, sloppy set', function () {
    var n = 0;
    var env = { binding: 0, get [Symbol.unscopables]() { n++; delete env.binding; return null; } };
    with (env) { binding = 123; }
    return n + ' ' + show(env.binding);
  }],
  ['unscopables getter deletes, get', function () {
    var n = 0;
    var env = { binding: 5, get [Symbol.unscopables]() { n++; delete env.binding; return null; } };
    var r;
    with (env) { r = err(function () { return binding; }); }
    return r + ' ' + n;
  }],
  ['unscopables getter throws', function () {
    var env = { x: 1, get [Symbol.unscopables]() { throw new RangeError('u'); } };
    with (env) { return err(function () { return x; }); }
  }],
  ['with call this', function () {
    var o = { f: function () { return this === o; } };
    with (o) { return f() + ' ' + (0, f)(); }
  }],
  ['with call this, nested function', function () {
    var o = { f: function () { return this === o; } };
    with (o) { return (function () { return f(); })(); }
  }],
  ['with getter receiver', function () {
    var o = { get g() { return this === o; } };
    with (o) { return g; }
  }],
  ['unscopables blocks', function () {
    var x = 'outer';
    var o = { x: 'inner', [Symbol.unscopables]: { x: true } };
    with (o) { return x; }
  }],
  ['unscopables not an object', function () {
    var x = 'outer';
    var o = { x: 'inner', [Symbol.unscopables]: true };
    with (o) { return x; }
  }],
  ['unscopables from prototype', function () {
    var keys = 'outer';
    with ([]) { return keys; }
  }],
  ['unscopables truthy value', function () {
    var x = 'outer';
    var o = { x: 'inner', [Symbol.unscopables]: { x: 'yes' } };
    with (o) { x = 'set'; }
    return x + ' ' + o.x;
  }],
  ['unscopables typeof and delete', function () {
    var o = { x: 1, [Symbol.unscopables]: { x: true } };
    with (o) { return typeof x + ' ' + delete x + ' ' + o.x; }
  }],
  ['null base string key read', function () { var b = null; return err(function () { return b['p']; }); }],
  ['null base number key read', function () { var b = null; return err(function () { return b[1]; }); }],
  ['null base symbol key read', function () { var b = null; return err(function () { return b[Symbol('s')]; }); }],
  ['null base object key read', function () { var b = null; var n = 0; return err(function () { return b[{ toString: function () { n++; return 'p'; } }]; }) + ' ' + n; }],
  ['undefined base object key read', function () { var b; var n = 0; return err(function () { return b[{ toString: function () { n++; return 'p'; } }]; }) + ' ' + n; }],
  ['null base string key set', function () { var b = null; return err(function () { b['p'] = 1; }); }],
  ['null base string key compound', function () { var b = null; return err(function () { b['p'] += 1; }); }],
  ['null base object key update', function () { var b = null; var n = 0; return err(function () { b[{ toString: function () { n++; return 'p'; } }]++; }) + ' ' + n; }],
  ['null base object key call', function () { var b = null; var n = 0; return err(function () { b[{ toString: function () { n++; return 'p'; } }](); }) + ' ' + n; }],
  ['null base dot read', function () { var b = null; return err(function () { return b.p; }); }],
  ['covered name gives no name', function () {
    var f, g, h;
    (f) = function () {};
    g = function () {};
    (h) ||= function () {};
    return JSON.stringify([f.name, g.name, h.name]);
  }],
  ['proxy env traps, read', function () {
    var log = [];
    var p = new Proxy({ x: 1 }, trap(log));
    with (p) { x; }
    return log.join();
  }],
  ['proxy env traps, missing read', function () {
    var log = [];
    var p = new Proxy({}, trap(log));
    with (p) { Object; }
    return log.join();
  }],
  ['proxy env traps, assign', function () {
    var log = [];
    var p = new Proxy({ x: 1 }, trap(log));
    with (p) { x = 2; }
    return log.join();
  }],
  ['proxy env traps, compound', function () {
    var log = [];
    var p = new Proxy({ x: 1 }, trap(log));
    with (p) { x += 2; }
    return log.join();
  }],
  ['proxy env traps, update', function () {
    var log = [];
    var p = new Proxy({ x: 1 }, trap(log));
    with (p) { x++; }
    return log.join();
  }],
  ['proxy env traps, call', function () {
    var log = [];
    var p = new Proxy({ f: function () { return this === p; } }, trap(log));
    var r;
    with (p) { r = f(); }
    return r + ' ' + log.join();
  }],
  ['proxy env traps, typeof', function () {
    var log = [];
    var p = new Proxy({ x: 1 }, trap(log));
    with (p) { typeof x; }
    return log.join();
  }],
  ['proxy env traps, strict assign', function () {
    var log = [];
    var p = new Proxy({ x: 1 }, trap(log));
    with (p) { (function () { 'use strict'; x = 2; })(); }
    return log.join();
  }],
  ['proxy env traps, delete in has', function () {
    var log = [];
    var t = { x: 1 };
    var h = trap(log);
    var p = new Proxy(t, h);
    var r;
    with (p) { r = err(function () { 'use strict'; x = (delete t.x, 2); return t.x; }); }
    return r + ' ' + log.join();
  }]
];
function trap(log) {
  return {
    has: function (t, k) { log.push('has:' + String(k)); return Reflect.has(t, k); },
    get: function (t, k, r) { log.push('get:' + String(k)); return Reflect.get(t, k, r); },
    set: function (t, k, v, r) { log.push('set:' + String(k)); return Reflect.set(t, k, v, r); },
    getOwnPropertyDescriptor: function (t, k) { log.push('gopd:' + String(k)); return Reflect.getOwnPropertyDescriptor(t, k); },
    defineProperty: function (t, k, d) { log.push('define:' + String(k)); return Reflect.defineProperty(t, k, d); }
  };
}
function run() { return cases.map(function (c) { var r; try { r = c[1](); } catch (e) { r = 'threw ' + e; } return c[0] + ': ' + r; }); }
run().join('\n')"#;

#[test]
fn references_read_as_the_reference_browser_gives_them() {
    let want = [
        r#"with getter deletes, compound: undefined 6"#,
        r#"with delete in rhs, simple: undefined 2"#,
        r#"with delete in rhs, compound: undefined 3"#,
        r#"with delete in rhs, strict function: ReferenceError: x is not defined undefined"#,
        r#"eval var in rhs, compound: 12 3"#,
        r#"eval var in rhs, simple: 4 3"#,
        r#"eval var in rhs, update: 5 3 4"#,
        r#"null base key throws first: RangeError: key"#,
        r#"null base key not converted: TypeError: Cannot read properties of null []"#,
        r#"null base simple assignment: TypeError: Cannot set properties of null [rhs]"#,
        r#"key converted once, compound: 2 5"#,
        r#"key converted once, update: 2 3"#,
        r#"key converted once, logical: 2 3"#,
        r#"key order against rhs, simple: rhs,key"#,
        r#"key order against rhs, compound: key,rhs,key"#,
        r#"unscopables once, update: 2 5 4"#,
        r#"unscopables once, compound: 2 5 4"#,
        r#"unscopables getter deletes, strict set: no throw: undefined 1 {"value":123,"writable":true,"enumerable":true,"configurable":true}"#,
        r#"unscopables getter deletes, sloppy set: 1 123"#,
        r#"unscopables getter deletes, get: no throw: undefined 1"#,
        r#"unscopables getter throws: RangeError: u"#,
        r#"with call this: true false"#,
        r#"with call this, nested function: true"#,
        r#"with getter receiver: true"#,
        r#"unscopables blocks: outer"#,
        r#"unscopables not an object: inner"#,
        r#"unscopables from prototype: outer"#,
        r#"unscopables truthy value: set inner"#,
        r#"unscopables typeof and delete: undefined true 1"#,
        r#"null base string key read: TypeError: Cannot read properties of null (reading 'p')"#,
        r#"null base number key read: TypeError: Cannot read properties of null (reading '1')"#,
        r#"null base symbol key read: TypeError: Cannot read properties of null (reading 'Symbol(s)')"#,
        r#"null base object key read: TypeError: Cannot read properties of null 0"#,
        r#"undefined base object key read: TypeError: Cannot read properties of undefined 0"#,
        r#"null base string key set: TypeError: Cannot set properties of null (setting 'p')"#,
        r#"null base string key compound: TypeError: Cannot read properties of null (reading 'p')"#,
        r#"null base object key update: TypeError: Cannot read properties of null 0"#,
        r#"null base object key call: TypeError: Cannot read properties of null 0"#,
        r#"null base dot read: TypeError: Cannot read properties of null (reading 'p')"#,
        r#"covered name gives no name: ["","g",""]"#,
        r#"proxy env traps, read: has:x,get:Symbol(Symbol.unscopables),get:x"#,
        r#"proxy env traps, missing read: has:Object"#,
        r#"proxy env traps, assign: has:x,get:Symbol(Symbol.unscopables),set:x,gopd:x,define:x"#,
        r#"proxy env traps, compound: has:x,get:Symbol(Symbol.unscopables),get:x,has:x,get:Symbol(Symbol.unscopables),set:x,gopd:x,define:x"#,
        r#"proxy env traps, update: has:x,get:Symbol(Symbol.unscopables),get:x,has:x,get:Symbol(Symbol.unscopables),set:x,gopd:x,define:x"#,
        r#"proxy env traps, call: true has:f,get:Symbol(Symbol.unscopables),get:f,has:r"#,
        r#"proxy env traps, typeof: has:x,get:Symbol(Symbol.unscopables),get:x"#,
        r#"proxy env traps, strict assign: has:x,get:Symbol(Symbol.unscopables),set:x,gopd:x,define:x"#,
        r#"proxy env traps, delete in has: ReferenceError: x is not defined has:err,has:t,has:x,has:r"#,
    ];
    let got = run(REFS_PROBE);
    let got: Vec<&str> = got.split('\n').collect();
    assert_eq!(got, want);
}
