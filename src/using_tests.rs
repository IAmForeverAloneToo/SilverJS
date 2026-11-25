//! Explicit resource management (J4.4b): `using` and `await using`,
//! `DisposableStack`, `AsyncDisposableStack` and `SuppressedError`, as
//! the reference browser gives them (the J4.4 and J4.4b probes, Chrome
//! 158).

use crate::agent::{Agent, AgentOptions, Outcome};

fn run_in(agent: &mut Agent, src: &str) -> String {
    agent.scope(|scope| match scope.run_script_named(src, "u.js") {
        Ok(Outcome::Value(v)) => scope.to_text(v).unwrap_or_default(),
        Ok(Outcome::Threw(e)) => format!("throw {}", scope.to_text(e).unwrap_or_default()),
        Ok(Outcome::Terminated) => "terminated".into(),
        Err(e) => format!("compile error: {e}"),
    })
}

fn run(src: &str) -> String {
    let mut agent = Agent::new(AgentOptions::default());
    run_in(&mut agent, src)
}

/// Runs `body`, whose jobs push to `log`, then reads `log` from a second
/// script: the jobs of the first ran when it ended.
fn logged(agent: &mut Agent, body: &str) -> String {
    run_in(agent, &format!("var log = [];\n{body}"));
    run_in(agent, "log.join()")
}

#[test]
fn using_disposes_as_the_spec_says() {
    // The last resource first; on a return, a continue, a break past two
    // scopes, a generator's return; each for-of iteration's at its end,
    // a for head's once at the loop's end; null and undefined skipped;
    // the method read at the declaration; a throw kept over the body's
    // (and over an earlier one) in a SuppressedError; the resource as
    // `this`; a stack moved and disposed.
    assert_eq!(
        run("(function () { \
             var log = []; \
             function r(n) { return { [Symbol.dispose]() { log.push(n); } }; } \
             { using a = r('a'), b = r('b'); log.push('body'); } \
             function f() { using c = r('c'); return 'ret'; } log.push(f()); \
             for (var i = 0; i < 2; i++) { using d = r('d' + i); if (i == 0) continue; } \
             for (using e of [r('e1'), r('e2')]) log.push('it'); \
             outer: { using g = r('g'); { using h = r('h'); break outer; } } \
             function* gen() { using k = r('k'); yield 1; } var it = gen(); it.next(); it.return(); \
             { using n = null, u = undefined; } \
             var m = { [Symbol.dispose]() { log.push('first'); } }; \
             { using x = m; m[Symbol.dispose] = function () { log.push('second'); }; } \
             try { using t = { [Symbol.dispose]() { throw 'e1'; } }; throw 'body'; } \
             catch (e) { log.push(e.constructor.name + ':' + e.error + ':' + e.suppressed); } \
             try { { using a = { [Symbol.dispose]() { throw 'a'; } }, b = { [Symbol.dispose]() { throw 'b'; } }; } } \
             catch (e) { log.push(e.error + '/' + e.suppressed); } \
             function g2() { using z = { [Symbol.dispose]() { throw 'z'; } }; return 1; } \
             try { g2(); } catch (e) { log.push('g2 ' + e); } \
             var self, o = { [Symbol.dispose]() { self = this; } }; { using q = o; } log.push(self === o); \
             var cnt = 0; for (using w = r('w'); cnt < 2; cnt++) log.push('c' + cnt); \
             var s = new DisposableStack(); s.use(r('su')); s.defer(function () { log.push('sd'); }); \
             var m2 = s.move(); log.push(s.disposed); m2.dispose(); log.push(m2.disposed); \
             var se = new SuppressedError('x', 'y', 'm'); log.push(se.error + se.suppressed + se.message);              return log.join(); })()"),
        "body,b,a,c,ret,d0,d1,it,e1,it,e2,h,g,k,first,SuppressedError:e1:body,a/b,g2 z,true,c0,c1,w,true,sd,su,true,xym"
    );
}

#[test]
fn disposables_survive_collection() {
    let mut agent = Agent::new(AgentOptions::default());
    agent.set_stress(true);
    run_in(
        &mut agent,
        "var total = 0, total2 = 0, done = -1, chain = ''; \
         (function () { \
           for (var i = 0; i < 40; i++) { \
             using a = { v: { n: i }, [Symbol.dispose]() { total += this.v.n; } }; \
             var s = new DisposableStack(); \
             s.adopt({ n: i }, function (o) { total += o.n; }); \
             s.defer(function () { total += 1; }); \
             s.dispose(); \
           } \
           try { \
             using x = { [Symbol.dispose]() { throw { m: 'x' }; } }, y = { [Symbol.dispose]() { throw { m: 'y' }; } }; \
             throw { m: 'body' }; \
           } catch (e) { chain = e.error.m + e.suppressed.error.m + e.suppressed.suppressed.m; } \
           var as = new AsyncDisposableStack(); \
           for (var j = 0; j < 20; j++) as.use({ [Symbol.asyncDispose]() { total2 += 1; return Promise.resolve({ big: [1, 2, 3] }); } }); \
           as.disposeAsync().then(function (v) { done = v === undefined ? 1 : 0; }); \
           (async function () { for (var k = 0; k < 10; k++) { await using b = { o: { k: k }, [Symbol.asyncDispose]() { total2 += this.o.k; } }; } })(); \
         })();",
    );
    assert_eq!(
        run_in(&mut agent, "[total, chain, done, total2].join()"),
        "1600,xybody,1,65"
    );
}

/// The J4.4b probe's synchronous parts: the messages, the keys of a
/// SuppressedError, the order of disposal.
const PROBE: &str = r#"var out = { errors: [], facts: [] };
function err(label, fn) {
  try { var v = fn(); out.errors.push(label + ': no throw ' + String(v)); }
  catch (e) { out.errors.push(label + ': ' + e.constructor.name + ': ' + e.message); }
}
function fact(label, fn) {
  try { out.facts.push(label + ' ' + String(fn())); }
  catch (e) { out.facts.push(label + ' threw ' + e.constructor.name + ': ' + e.message); }
}
err('use number', function () { new DisposableStack().use(1); });
err('use string', function () { new DisposableStack().use('x'); });
err('use non-callable', function () { new DisposableStack().use({ [Symbol.dispose]: 1 }); });
err('adopt non-callable', function () { new DisposableStack().adopt(1, 2); });
err('use receiver', function () { DisposableStack.prototype.use.call({}, null); });
err('move receiver', function () { DisposableStack.prototype.move.call({}); });
err('disposed receiver', function () { Object.getOwnPropertyDescriptor(DisposableStack.prototype, 'disposed').get.call({}); });
err('adopt after dispose', function () { var s = new DisposableStack(); s.dispose(); s.adopt(1, function () {}); });
err('defer after dispose', function () { var s = new DisposableStack(); s.dispose(); s.defer(function () {}); });
err('async call', function () { AsyncDisposableStack(); });
err('async use number', function () { new AsyncDisposableStack().use(1); });
err('async use non-callable', function () { new AsyncDisposableStack().use({ [Symbol.asyncDispose]: 1 }); });
err('async use after dispose', function () { var s = new AsyncDisposableStack(); s.disposeAsync(); s.use(null); });
err('async move after dispose', function () { var s = new AsyncDisposableStack(); s.disposeAsync(); s.move(); });
err('async defer non-callable', function () { new AsyncDisposableStack().defer(1); });
err('async use receiver', function () { AsyncDisposableStack.prototype.use.call({}, null); });
err('async disposed receiver', function () { Object.getOwnPropertyDescriptor(AsyncDisposableStack.prototype, 'disposed').get.call({}); });
err('using number', function () { (new Function('{ using a = 1; }'))(); });
err('using non-callable', function () { (new Function('{ using a = { [Symbol.dispose]: 1 }; }'))(); });
err('using string', function () { (new Function('{ using a = "s"; }'))(); });
err('using two throws', function () { (new Function('{ using a = { [Symbol.dispose]() { throw new Error("a"); } }, b = { [Symbol.dispose]() { throw new Error("b"); } }; throw new Error("body"); }'))(); });
fact('suppressed keys', function () {
  var s = new DisposableStack();
  s.defer(function () { throw 2; });
  s.defer(function () { throw 1; });
  try { s.dispose(); } catch (e) { return Object.getOwnPropertyNames(e).join('/') + ' ' + e.error + ' ' + e.suppressed; }
});
fact('suppressed ctor keys', function () { return Object.getOwnPropertyNames(new SuppressedError(1, 2, 'm')).join('/') + ' ' + Object.getOwnPropertyNames(new SuppressedError(1, 2)).join('/'); });
fact('suppressed toString', function () { return String(new SuppressedError(1, 2, 'm')) + ' ' + String(new SuppressedError()); });
fact('order', function () {
  var log = [];
  var s = new DisposableStack();
  s.use({ [Symbol.dispose]: function () { log.push('use'); } });
  s.adopt('v', function (v) { log.push('adopt ' + v + ' ' + (this === undefined)); });
  s.defer(function () { log.push('defer ' + arguments.length); });
  var m = s.move();
  log.push(s.disposed, m.disposed);
  m.dispose(); m.dispose();
  return log.join(',');
});
fact('using order', function () {
  var log = [];
  (new Function('log', 'for (using x of [{ [Symbol.dispose]() { log.push("d1"); } }, { [Symbol.dispose]() { log.push("d2"); } }]) { log.push("body"); } ' +
    'outer: { using a = { [Symbol.dispose]() { log.push("a"); } }; { using b = { [Symbol.dispose]() { log.push("b"); } }; break outer; } } ' +
    'function f() { using c = { [Symbol.dispose]() { log.push("c"); } }; return "r"; } log.push(f()); ' +
    'function* g() { using d = { [Symbol.dispose]() { log.push("d"); } }; yield 1; yield 2; } var it = g(); it.next(); it.return(); ' +
    'var i = 0; for (using e = { [Symbol.dispose]() { log.push("e"); } }; i < 2; i++) log.push("i" + i);'))(log);
  return log.join(',');
});
out.errors.concat(out.facts).join('\n')"#;

#[test]
fn disposables_read_as_the_reference_browser_gives_them() {
    let got = run(PROBE);
    let want = r#"use number: TypeError: An object is expected with `using` declarations
use string: TypeError: An object is expected with `using` declarations
use non-callable: TypeError: Symbol(Symbol.dispose) is not a function
adopt non-callable: TypeError: 2 is not a function
use receiver: TypeError: Method DisposableStack.prototype.use called on incompatible receiver #<Object>
move receiver: TypeError: Method DisposableStack.prototype.move called on incompatible receiver #<Object>
disposed receiver: TypeError: Method get DisposableStack.prototype.disposed called on incompatible receiver #<Object>
adopt after dispose: ReferenceError: Cannot call DisposableStack.prototype.adopt on an already-disposed DisposableStack
defer after dispose: ReferenceError: Cannot call DisposableStack.prototype.defer on an already-disposed DisposableStack
async call: TypeError: Constructor AsyncDisposableStack requires 'new'
async use number: TypeError: An object is expected with `using` declarations
async use non-callable: TypeError: Symbol(Symbol.asyncDispose) is not a function
async use after dispose: ReferenceError: Cannot call AsyncDisposableStack.prototype.use on an already-disposed DisposableStack
async move after dispose: ReferenceError: Cannot call AsyncDisposableStack.prototype.move on an already-disposed DisposableStack
async defer non-callable: TypeError: 1 is not a function
async use receiver: TypeError: Method AsyncDisposableStack.prototype.use called on incompatible receiver #<Object>
async disposed receiver: TypeError: Method get AsyncDisposableStack.prototype.disposed called on incompatible receiver #<Object>
using number: TypeError: An object is expected with `using` declarations
using non-callable: TypeError: Symbol(Symbol.dispose) is not a function
using string: TypeError: An object is expected with `using` declarations
using two throws: SuppressedError: An error was suppressed during disposal
suppressed keys stack/message/error/suppressed 2 1
suppressed ctor keys stack/message/error/suppressed stack/error/suppressed
suppressed toString SuppressedError: m SuppressedError
order true,false,defer 0,adopt v false,use
using order body,d1,body,d2,b,a,c,r,d,i0,i1,e"#;
    for (g, w) in got.lines().zip(want.lines()) {
        assert_eq!(g, w);
    }
    assert_eq!(got.lines().count(), want.lines().count(), "{got}");
}

/// The J4.4b probe's ticks: what `await using`, `disposeAsync` and
/// `%AsyncIteratorPrototype%[Symbol.asyncDispose]` do between eight
/// chained reactions (label, body, Chrome 158's log).
const TICKS: &[(&str, &str, &str)] = &[
    (
        r#"await using async"#,
        r#"  (new Function('log', 'return (async function () { log.push("a"); { await using x = { [Symbol.asyncDispose]() { log.push("d"); return Promise.resolve(); } }; log.push("b"); } log.push("c"); })()'))(log);"#,
        r#"a,b,d,t1,c,t2,t3,t4,t5,t6,t7,t8"#,
    ),
    (
        r#"await using sync fallback"#,
        r#"  (new Function('log', 'return (async function () { { await using x = { [Symbol.dispose]() { log.push("d"); return 5; } }; log.push("b"); } log.push("c"); })()'))(log);"#,
        r#"b,d,t1,c,t2,t3,t4,t5,t6,t7,t8"#,
    ),
    (
        r#"await using null"#,
        r#"  (new Function('log', 'return (async function () { { await using x = null; log.push("b"); } log.push("c"); })()'))(log);"#,
        r#"b,t1,c,t2,t3,t4,t5,t6,t7,t8"#,
    ),
    (
        r#"await using null then sync"#,
        r#"  (new Function('log', 'return (async function () { { using s = { [Symbol.dispose]() { log.push("s"); } }; await using x = null; log.push("b"); } log.push("c"); })()'))(log);"#,
        r#"b,t1,s,c,t2,t3,t4,t5,t6,t7,t8"#,
    ),
    (
        r#"await using none"#,
        r#"  (new Function('log', 'return (async function () { { await using x = { [Symbol.asyncDispose]() { log.push("d"); } }; } log.push("c"); })()'))(log);"#,
        r#"d,t1,c,t2,t3,t4,t5,t6,t7,t8"#,
    ),
    (
        r#"await using reject"#,
        r#"  (new Function('log', 'return (async function () { try { await using x = { [Symbol.asyncDispose]() { return Promise.reject(new Error("r")); } }; } catch (e) { log.push("caught " + e.message); } })()'))(log);"#,
        r#"t1,caught r,t2,t3,t4,t5,t6,t7,t8"#,
    ),
    (
        r#"await using not disposable"#,
        r#"  (new Function('log', 'return (async function () { try { await using x = {}; } catch (e) { log.push(e.constructor.name + ": " + e.message); } })()'))(log);"#,
        r#"TypeError: Symbol(Symbol.dispose) is not a function,t1,t2,t3,t4,t5,t6,t7,t8"#,
    ),
    (
        r#"disposeAsync"#,
        r#"  var s = new AsyncDisposableStack();
  s.use({ [Symbol.asyncDispose]: function () { log.push('u'); return Promise.resolve(); } });
  s.defer(function () { log.push('df'); });
  s.use({ [Symbol.dispose]: function () { log.push('sync'); } });
  s.disposeAsync().then(function (v) { log.push('done ' + v); });"#,
        r#"sync,t1,df,t2,u,t3,t4,done undefined,t5,t6,t7,t8"#,
    ),
    (
        r#"disposeAsync errors"#,
        r#"  var s = new AsyncDisposableStack();
  s.defer(function () { throw new Error('one'); });
  s.defer(function () { return Promise.reject(new Error('two')); });
  s.disposeAsync().then(null, function (e) { log.push(e.constructor.name + ' ' + e.error.message + ' ' + e.suppressed.message); });"#,
        r#"t1,t2,SuppressedError one two,t3,t4,t5,t6,t7,t8"#,
    ),
    (
        r#"disposeAsync receiver"#,
        r#"  AsyncDisposableStack.prototype.disposeAsync.call({}).then(null, function (e) { log.push(e.constructor.name + ': ' + e.message); });"#,
        r#"t1,TypeError: Receiver is not an AsyncDisposableStack,t2,t3,t4,t5,t6,t7,t8"#,
    ),
    (
        r#"asyncIterator asyncDispose"#,
        r#"  var AIP = Object.getPrototypeOf(Object.getPrototypeOf(async function* () {}.prototype));
  AIP[Symbol.asyncDispose].call({ return: function () { log.push('ret ' + arguments.length + ' ' + arguments[0]); return Promise.resolve(7); } })
    .then(function (v) { log.push('done ' + v); });"#,
        r#"ret 0 undefined,t1,t2,done undefined,t3,t4,t5,t6,t7,t8"#,
    ),
    (
        r#"asyncIterator asyncDispose bad"#,
        r#"  var AIP = Object.getPrototypeOf(Object.getPrototypeOf(async function* () {}.prototype));
  AIP[Symbol.asyncDispose].call({ return: 1 }).then(null, function (e) { log.push(e.constructor.name + ': ' + e.message); });"#,
        r#"t1,TypeError: '1' returned for property 'return' of object '#<Object>' is not a function,t2,t3,t4,t5,t6,t7,t8"#,
    ),
];

#[test]
fn awaits_take_the_reference_browsers_ticks() {
    for (label, body, want) in TICKS {
        let mut agent = Agent::new(AgentOptions::default());
        let src = format!(
            "var p = Promise.resolve(); \
             for (var i = 1; i <= 8; i++) (function (i) {{ p = p.then(function () {{ log.push('t' + i); }}); }})(i); \
             (function (log) {{\n{body}\n}})(log);"
        );
        assert_eq!(logged(&mut agent, &src), *want, "{label}");
    }
}
