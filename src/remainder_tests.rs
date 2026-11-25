//! The built-ins' remainders (J4.4c): `Array.fromAsync`,
//! `Object.groupBy`, `Promise.try`'s promise as it is, the immutable
//! `Object.prototype`, as the reference browser gives them (the J4.4c
//! probe, Chrome 158).

use crate::agent::{Agent, AgentOptions, Outcome};

fn run_in(agent: &mut Agent, src: &str) -> String {
    agent.scope(|scope| match scope.run_script_named(src, "r.js") {
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
fn array_methods_reach_a_proxys_traps() {
    // CreateDataPropertyOrThrow and DeletePropertyOrThrow on a proxy run
    // its traps; IsArray throws for a revoked proxy; Object.groupBy's
    // keys and groups.
    assert_eq!(
        run("(function () { var r = [], calls = []; \
             function P() { return new Proxy({}, { \
               defineProperty(t, k, d) { calls.push('dp ' + String(k)); return Reflect.defineProperty(t, k, d); }, \
               deleteProperty(t, k) { calls.push('del ' + String(k)); return Reflect.deleteProperty(t, k); } }); } \
             var a = Array.from.call(P, [1, 2]); r.push(calls.join('/'), a[1]); \
             var o = P(); o.length = 3; o[0] = 'x'; calls = []; \
             Array.prototype.copyWithin.call(o, 0, 1, 2); r.push(calls.join('/')); \
             var rev = Proxy.revocable([], {}); rev.revoke(); \
             try { Array.isArray(rev.proxy); } catch (e) { r.push(e.message); } \
             r.push(JSON.stringify(Object.groupBy('abca', function (c) { return c; }))); \
             var g = Object.groupBy([1], function () { return { toString() { return 'k'; } }; }); \
             r.push(Object.keys(g).join()); \
             return r.join(' | '); })()"),
        "dp 0/dp 1/dp length | 2 | del 0 | Cannot perform 'IsArray' on a proxy that has been revoked | \
         {\"a\":[\"a\",\"a\"],\"b\":[\"b\"],\"c\":[\"c\"]} | k"
    );
}

#[test]
fn object_prototypes_prototype_stays_null() {
    // An immutable prototype exotic object (section 10.4.7): a prototype
    // that makes no cycle is refused too.
    assert_eq!(
        run(
            "var n = Object.create(null), r = [Reflect.setPrototypeOf(Object.prototype, n)]; \
             try { Object.setPrototypeOf(Object.prototype, n); } catch (e) { r.push(e.constructor.name); } \
             r.push(Object.getPrototypeOf(Object.prototype) === null); r.join()"
        ),
        "false,TypeError,true"
    );
}

#[test]
fn from_async_survives_collection() {
    let mut agent = Agent::new(AgentOptions::default());
    agent.set_stress(true);
    run_in(
        &mut agent,
        "var sum = -1, likes = -1, groups = ''; \
         var items = []; for (var i = 0; i < 30; i++) items.push({ v: i }); \
         Array.fromAsync(items, async function (o, k) { return { w: o.v + k }; }) \
           .then(function (a) { sum = a.reduce(function (s, o) { return s + o.w; }, 0); }); \
         var like = { length: 20 }; for (var j = 0; j < 20; j++) like[j] = Promise.resolve({ v: j }); \
         Array.fromAsync(like).then(function (a) { likes = a.length + a[19].v; }); \
         var g = Object.groupBy(items, function (o) { return o.v % 3 ? 'n' + (o.v % 3) : 'z'; }); \
         groups = [g.z.length, g.n1.length, g.n2[9].v].join();",
    );
    assert_eq!(
        run_in(&mut agent, "[sum, likes, groups].join(' ')"),
        "870 39 10,10,29"
    );
}

/// The J4.4c probe's synchronous parts.
const PROBE: &str = r#"var out = { errors: [], facts: [] };
function err(label, fn) {
  try { var v = fn(); out.errors.push(label + ': no throw ' + String(v)); }
  catch (e) { out.errors.push(label + ': ' + e.constructor.name + ': ' + e.message); }
}
function fact(label, fn) {
  try { out.facts.push(label + ' ' + String(fn())); }
  catch (e) { out.facts.push(label + ' threw ' + e.constructor.name + ': ' + e.message); }
}
err('groupBy null', function () { Object.groupBy(null, function () {}); });
err('groupBy callback', function () { Object.groupBy([], 1); });
err('groupBy non-iterable', function () { Object.groupBy(1, function () {}); });
err('setPrototypeOf Object.prototype', function () { Object.setPrototypeOf(Object.prototype, {}); });
err('__proto__ Object.prototype', function () { Object.prototype.__proto__ = {}; });
fact('setPrototypeOf Object.prototype same', function () { return Object.setPrototypeOf(Object.prototype, null) === Object.prototype; });
fact('Reflect.setPrototypeOf Object.prototype', function () { return Reflect.setPrototypeOf(Object.prototype, {}) + ' ' + Reflect.setPrototypeOf(Object.prototype, null); });
fact('groupBy', function () {
  var g = Object.groupBy([1, 2, 3, 4, 5], function (x, i) { return x % 2 ? 'odd' : i; });
  return Object.getPrototypeOf(g) + ' ' + Reflect.ownKeys(g).join('/') + ' ' + JSON.stringify(g);
});
fact('Promise.try same', function () {
  var p = Promise.resolve();
  return Promise.try(function () { return p; }) === p;
});
fact('Promise.try thenable', function () {
  var t = { then: function (r) { r(1); } };
  return Promise.try(function () { return t; }) instanceof Promise;
});
fact('fromAsync', function () { return typeof Array.fromAsync + ' ' + Array.fromAsync.length + ' ' + Array.fromAsync.name; });
out.errors.concat(out.facts).join('\n')"#;

#[test]
fn remainders_read_as_the_reference_browser_gives_them() {
    let got = run(PROBE);
    let want = r#"groupBy null: TypeError: Object.groupBy called on null or undefined
groupBy callback: TypeError: 1 is not a function
groupBy non-iterable: TypeError: number 1 is not iterable (cannot read property Symbol(Symbol.iterator))
setPrototypeOf Object.prototype: TypeError: Immutable prototype object 'Object.prototype' cannot have their prototype set
__proto__ Object.prototype: TypeError: Immutable prototype object 'Object.prototype' cannot have their prototype set
setPrototypeOf Object.prototype same true
Reflect.setPrototypeOf Object.prototype false true
groupBy null 1/3/odd {"1":[2],"3":[4],"odd":[1,3,5]}
Promise.try same true
Promise.try thenable true
fromAsync function 1 fromAsync"#;
    for (g, w) in got.lines().zip(want.lines()) {
        assert_eq!(g, w);
    }
    assert_eq!(got.lines().count(), want.lines().count(), "{got}");
}

/// The J4.4c probes' `show`, which logs how a promise settles, and
/// `syncSource`, a sync iterable whose first value is a rejected promise.
const SHOW: &str = r#"function show(log) {
  return function (p) {
    p.then(function (a) { log.push('done ' + JSON.stringify(a) + ' ' + Array.isArray(a)); },
           function (e) { log.push('rejected ' + e.constructor.name + ': ' + e.message); });
  };
}
function syncSource(log) {
  return { [Symbol.iterator]: function () { var i = 0; return { next: function () { i++; return i === 1 ? { value: Promise.reject(new Error('r')), done: false } : { done: true }; }, return: function () { log.push('return'); return {}; } }; } };
}
"#;

/// The J4.4c probes' ticks: `Array.fromAsync` between ten chained
/// reactions (label, body, Chrome 158's log).
const TICKS: &[(&str, &str, &str)] = &[
    (
        r#"fromAsync array"#,
        r#"show(log)(Array.fromAsync([1, Promise.resolve(2), 3]));"#,
        r#"t1,t2,t3,t4,t5,t6,t7,t8,t9,done [1,2,3] true,t10"#,
    ),
    (
        r#"fromAsync array map"#,
        r#"show(log)(Array.fromAsync([1, 2], function (x, i) { log.push('map ' + x + ' ' + i); return x * 10; }));"#,
        r#"t1,t2,map 1 0,t3,t4,t5,map 2 1,t6,t7,t8,t9,done [10,20] true,t10"#,
    ),
    (
        r#"fromAsync array-like"#,
        r#"show(log)(Array.fromAsync({ length: 2, 0: 'a', 1: Promise.resolve('b') }));"#,
        r#"t1,t2,t3,done ["a","b"] true,t4,t5,t6,t7,t8,t9,t10"#,
    ),
    (
        r#"fromAsync async iter"#,
        r#"show(log)(Array.fromAsync((async function* () { yield 1; yield 2; })()));"#,
        r#"t1,t2,t3,t4,t5,t6,done [1,2] true,t7,t8,t9,t10"#,
    ),
    (
        r#"fromAsync null"#,
        r#"show(log)(Array.fromAsync(null));"#,
        r#"t1,rejected TypeError: Cannot read properties of null (reading 'Symbol(Symbol.asyncIterator)'),t2,t3,t4,t5,t6,t7,t8,t9,t10"#,
    ),
    (
        r#"fromAsync mapfn"#,
        r#"show(log)(Array.fromAsync([], 1));"#,
        r#"t1,rejected TypeError: 1 is not a function,t2,t3,t4,t5,t6,t7,t8,t9,t10"#,
    ),
    (
        r#"fromAsync bad result"#,
        r#"show(log)(Array.fromAsync({ [Symbol.asyncIterator]: function () { return { next: function () { return 1; } }; } }));"#,
        r#"t1,t2,rejected TypeError: Iterator result Array.fromAsync is not an object,t3,t4,t5,t6,t7,t8,t9,t10"#,
    ),
    (
        r#"fromAsync bad iterator"#,
        r#"show(log)(Array.fromAsync({ [Symbol.asyncIterator]: function () { return 1; } }));"#,
        r#"t1,rejected TypeError: Result of the Symbol.iterator method is not an object,t2,t3,t4,t5,t6,t7,t8,t9,t10"#,
    ),
    (
        r#"fromAsync bad sync iterator"#,
        r#"show(log)(Array.fromAsync({ [Symbol.iterator]: function () { return 1; } }));"#,
        r#"t1,rejected TypeError: Result of the Symbol.iterator method is not an object,t2,t3,t4,t5,t6,t7,t8,t9,t10"#,
    ),
    (
        r#"fromAsync map throws closes"#,
        r#"var it = { [Symbol.asyncIterator]: function () { return { next: function () { return { value: 1, done: false }; }, return: function () { log.push('return'); return {}; } }; } };
  show(log)(Array.fromAsync(it, function () { throw new Error('m'); }));"#,
        r#"t1,return,t2,t3,rejected Error: m,t4,t5,t6,t7,t8,t9,t10"#,
    ),
    (
        r#"fromAsync this"#,
        r#"function C() { log.push('C ' + arguments.length); }
  show(log)(Array.fromAsync.call(C, [1]));"#,
        r#"C 0,t1,t2,t3,t4,t5,done {"0":1,"length":1} false,t6,t7,t8,t9,t10"#,
    ),
    (
        r#"fromAsync sync rejection"#,
        r#"var it = { [Symbol.iterator]: function () { var i = 0; return { next: function () { i++; return i === 1 ? { value: Promise.reject(new Error('r')), done: false } : { done: true }; }, return: function () { log.push('return'); return {}; } }; } };
  show(log)(Array.fromAsync(it));"#,
        r#"t1,return,t2,return,t3,t4,t5,rejected Error: r,t6,t7,t8,t9,t10"#,
    ),
    (
        r#"for await sync rejection"#,
        r#"(async function () { try { for await (var x of syncSource(log)) {} } catch (e) { log.push('caught ' + e.message); } })();"#,
        r#"t1,return,t2,caught r,t3,t4,t5,t6,t7,t8,t9,t10"#,
    ),
    (
        r#"fromAsync async next rejects"#,
        r#"var it = { [Symbol.asyncIterator]: function () { return { next: function () { return Promise.reject(new Error('n')); }, return: function () { log.push('return'); return {}; } }; } };
  Array.fromAsync(it).then(null, function (e) { log.push('rejected ' + e.message); });"#,
        r#"t1,return,t2,t3,rejected n,t4,t5,t6,t7,t8,t9,t10"#,
    ),
    (
        r#"fromAsync async next throws"#,
        r#"var it = { [Symbol.asyncIterator]: function () { return { next: function () { throw new Error('n'); }, return: function () { log.push('return'); return {}; } }; } };
  Array.fromAsync(it).then(null, function (e) { log.push('rejected ' + e.message); });"#,
        r#"return,t1,t2,rejected n,t3,t4,t5,t6,t7,t8,t9,t10"#,
    ),
    (
        r#"fromAsync async value rejected"#,
        r#"var it = { [Symbol.asyncIterator]: function () { var i = 0; return { next: function () { i++; return i === 1 ? { value: Promise.reject(new Error('v')), done: false } : { done: true }; }, return: function () { log.push('return'); return {}; } }; } };
  Array.fromAsync(it).then(function (a) { log.push('done ' + a.length); }, function (e) { log.push('rejected ' + e.message); });"#,
        r#"t1,t2,t3,done 1,t4,t5,t6,t7,t8,t9,t10"#,
    ),
    (
        r#"fromAsync sync rejection no return"#,
        r#"var it = { [Symbol.iterator]: function () { var i = 0; return { next: function () { i++; return i === 1 ? { value: Promise.reject(new Error('r')), done: false } : { done: true }; } }; } };
  Array.fromAsync(it).then(null, function (e) { log.push('rejected ' + e.message); });"#,
        r#"t1,t2,t3,t4,rejected r,t5,t6,t7,t8,t9,t10"#,
    ),
];

#[test]
fn from_async_takes_the_reference_browsers_ticks() {
    for (label, body, want) in TICKS {
        let mut agent = Agent::new(AgentOptions::default());
        let src = format!(
            "{SHOW}var p = Promise.resolve(); \
             for (var i = 1; i <= 10; i++) (function (i) {{ p = p.then(function () {{ log.push('t' + i); }}); }})(i); \
             try {{ (function (log) {{ {body} }})(log); }} \
             catch (e) {{ log.push('sync ' + e.constructor.name + ': ' + e.message); }}"
        );
        assert_eq!(logged(&mut agent, &src), *want, "{label}");
    }
}

/// The J4.4c for-in probe: a proxy's traps in a for-in, and when.
const FOR_IN: &str = r#"// SilverJS's J4.4c probe of for-in over proxies: which traps run, when.
var out = { facts: [] };
function logged(target, log) {
  return new Proxy(target, {
    ownKeys: function (t) { log.push('ownKeys'); return Reflect.ownKeys(t); },
    getOwnPropertyDescriptor: function (t, k) { log.push('gopd ' + String(k)); return Reflect.getOwnPropertyDescriptor(t, k); },
    getPrototypeOf: function (t) { log.push('getPrototypeOf'); return Reflect.getPrototypeOf(t); },
    has: function (t, k) { log.push('has ' + String(k)); return Reflect.has(t, k); },
    get: function (t, k, r) { log.push('get ' + String(k)); return Reflect.get(t, k, r); }
  });
}
function fact(label, fn) {
  try { out.facts.push(label + ' ' + String(fn())); }
  catch (e) { out.facts.push(label + ' threw ' + e.constructor.name + ': ' + e.message); }
}
fact('own', function () {
  var log = [];
  for (var k in logged({ a: 1, b: 2 }, log)) log.push('body ' + k);
  return log.join(', ');
});
fact('proto', function () {
  var log = [];
  var o = Object.create(logged({ c: 3 }, log));
  o.d = 4;
  for (var k in o) log.push('body ' + k);
  return log.join(', ');
});
fact('deleted', function () {
  var log = [];
  var t = { a: 1, b: 2, c: 3 };
  for (var k in logged(t, log)) { log.push('body ' + k); delete t.b; }
  return log.join(', ');
});
fact('deleted proto', function () {
  var log = [];
  var t = { c: 3, e: 5 };
  var o = Object.create(logged(t, log));
  o.d = 4;
  for (var k in o) { log.push('body ' + k); delete t.e; }
  return log.join(', ');
});
fact('non-enumerable', function () {
  var log = [];
  var t = Object.defineProperty({ a: 1 }, 'h', { value: 2, enumerable: false });
  for (var k in logged(t, log)) log.push('body ' + k);
  return log.join(', ');
});
fact('array', function () {
  var log = [];
  for (var k in logged([7, 8], log)) log.push('body ' + k);
  return log.join(', ');
});
out.facts.join('\n')"#;

#[test]
fn for_in_reads_a_proxy_as_the_reference_browser_does() {
    assert_eq!(
        run(FOR_IN),
        r#"own ownKeys, getPrototypeOf, gopd a, body a, gopd b, body b
proto ownKeys, getPrototypeOf, body d, gopd c, body c
deleted ownKeys, getPrototypeOf, gopd a, body a, gopd b, getPrototypeOf, gopd c, body c
deleted proto ownKeys, getPrototypeOf, body d, gopd c, body c, gopd e, getPrototypeOf
non-enumerable ownKeys, getPrototypeOf, gopd a, body a, gopd h
array ownKeys, getPrototypeOf, gopd 0, body 0, gopd 1, body 1, gopd length"#
    );
}
