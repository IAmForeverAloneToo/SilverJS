//! Proxies (J3.5b): every internal method through its trap or the
//! target's, the invariants, revocation, call and construct, and the
//! surface the reference browser shows (the proxy probe, Chrome 157).

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
    run_in(&mut Agent::new(AgentOptions::default()), src)
}

/// A handler logging each trap, then doing what the target would.
const LOGGING: &str = "var log = []; var handler = {}; \
    ['get', 'set', 'has', 'deleteProperty', 'ownKeys', 'getOwnPropertyDescriptor', 'defineProperty', \
     'getPrototypeOf', 'setPrototypeOf', 'isExtensible', 'preventExtensions', 'apply', 'construct'].forEach(function (t) { \
      handler[t] = function () { log.push(t + (typeof arguments[1] === 'string' ? ':' + arguments[1] : '')); \
        return Reflect[t].apply(null, arguments); }; });\n";

#[test]
fn every_operation_goes_through_its_trap() {
    assert_eq!(
        run(&format!(
            "{LOGGING}var p = new Proxy({{ a: 1, b: 2 }}, handler); \
             p.a; p.c = 3; 'a' in p; delete p.c; log.push('|'); Object.keys(p); log.push('|'); \
             ({{ ...p }}); log.push('|'); for (var k in p) {{}} log.push('|'); Object.freeze(p); \
             var f = new Proxy(function (x) {{ return x; }}, handler); log.push('|'); f(1); new f(1); log.join()"
        )),
        "get:a,set:c,getOwnPropertyDescriptor:c,defineProperty:c,has:a,deleteProperty:c,|,\
         ownKeys,getOwnPropertyDescriptor:a,getOwnPropertyDescriptor:b,|,\
         ownKeys,getOwnPropertyDescriptor:a,get:a,getOwnPropertyDescriptor:b,get:b,|,\
         ownKeys,getPrototypeOf,getOwnPropertyDescriptor:a,getOwnPropertyDescriptor:b,|,\
         preventExtensions,ownKeys,getOwnPropertyDescriptor:a,defineProperty:a,getOwnPropertyDescriptor:b,defineProperty:b,|,\
         apply,construct,get:prototype"
    );
}

#[test]
fn a_proxy_without_traps_is_its_target_and_revoked_is_nothing() {
    assert_eq!(
        run("var t = [1, 2]; var p = new Proxy(t, {}); p.push(3); \
             var r = Proxy.revocable({}, {}); r.revoke(); r.revoke(); \
             var e; try { r.proxy.x; } catch (x) { e = x.message; } \
             var f = new Proxy(function () { return this; }, {}); \
             [t.join(), Array.isArray(p), typeof p, typeof f, typeof new Proxy({}, {}), \
              'prototype' in Proxy, Object.getPrototypeOf(p) === Array.prototype, e].join()"),
        "1,2,3,true,object,function,object,false,true,Cannot perform 'get' on a proxy that has been revoked"
    );
}

#[test]
fn species_spreading_and_flattening_see_through_a_proxy() {
    assert_eq!(
        run(
            "class A extends Array {} var p = new Proxy(A.of(1, 2), {}); \
             [[].concat(p).length, [p].flat().length, \
              Array.prototype.map.call(p, function (x) { return x; }) instanceof A].join()"
        ),
        "2,2,true"
    );
}

#[test]
fn a_proxys_answers_are_held_to_the_invariants() {
    assert_eq!(
        run(
            "function err(f) { try { return 'no throw ' + f(); } catch (e) { return e.constructor.name; } } \
             var frozen = Object.freeze({ x: 1 }); \
             [err(function () { return new Proxy(frozen, { get: function () { return 2; } }).x; }), \
              err(function () { return 'x' in new Proxy(frozen, { has: function () { return false; } }); }), \
              err(function () { return Object.keys(new Proxy(frozen, { ownKeys: function () { return []; } })); }), \
              err(function () { return Reflect.ownKeys(new Proxy({}, { ownKeys: function () { return ['a', 'a']; } })); }), \
              err(function () { return Object.getPrototypeOf(new Proxy(Object.preventExtensions({}), { getPrototypeOf: function () { return Array.prototype; } })); }), \
              err(function () { return Object.isExtensible(new Proxy({}, { isExtensible: function () { return false; } })); }), \
              err(function () { return new (new Proxy(function () {}, { construct: function () { return 1; } }))(); }), \
              err(function () { return new Proxy({ y: 1 }, { get: function () { return 9; } }).y; })].join()"
        ),
        "TypeError,TypeError,TypeError,TypeError,TypeError,TypeError,TypeError,no throw 9"
    );
}

#[test]
fn proxies_survive_collection() {
    let mut agent = Agent::new(AgentOptions::default());
    agent.set_stress(true);
    assert_eq!(
        run_in(
            &mut agent,
            "var out = []; for (var i = 0; i < 20; i++) { \
             var p = new Proxy({ n: i }, { get: function (t, k) { return k === 'twice' ? t.n * 2 : Reflect.get(t, k); } }); \
             out.push(p.twice + p.n); } out.join()"
        ),
        "0,3,6,9,12,15,18,21,24,27,30,33,36,39,42,45,48,51,54,57"
    );
}

/// The proxy probe's errors, each as the reference browser gave it.
const PROBE_ERRORS: &str = r#"var errors = [];
function err(label, fn) {
  try { var v = fn(); errors.push(label + ': no throw ' + String(v)); }
  catch (e) { errors.push(label + ': ' + e.constructor.name + ': ' + e.message); }
}
var frozen = Object.freeze({ x: 1 });
err('Proxy()', function () { Proxy({}, {}); });
err('new Proxy(1)', function () { new Proxy(1, {}); });
err('new Proxy handler 1', function () { new Proxy({}, 1); });
err('revoked get', function () { var r = Proxy.revocable({}, {}); r.revoke(); return r.proxy.x; });
err('revoked typeof', function () { var r = Proxy.revocable({}, {}); r.revoke(); return typeof r.proxy; });
err('trap not function', function () { return new Proxy({}, { get: 1 }).x; });
err('get invariant', function () { return new Proxy(frozen, { get: function () { return 2; } }).x; });
err('set invariant', function () { 'use strict'; new Proxy(frozen, { set: function () { return true; } }).x = 2; });
err('set false strict', function () { 'use strict'; new Proxy({}, { set: function () { return false; } }).x = 2; });
err('has invariant', function () { return 'x' in new Proxy(frozen, { has: function () { return false; } }); });
err('delete invariant', function () { return delete new Proxy(frozen, { deleteProperty: function () { return true; } }).x; });
err('delete false strict', function () { 'use strict'; return delete new Proxy({ y: 1 }, { deleteProperty: function () { return false; } }).y; });
err('ownKeys not object', function () { return Object.keys(new Proxy({}, { ownKeys: function () { return 1; } })); });
err('ownKeys missing', function () { return Object.keys(new Proxy(frozen, { ownKeys: function () { return []; } })); });
err('ownKeys duplicate', function () { return Reflect.ownKeys(new Proxy({}, { ownKeys: function () { return ['a', 'a']; } })); });
err('ownKeys bad type', function () { return Reflect.ownKeys(new Proxy({}, { ownKeys: function () { return [1]; } })); });
err('gopd invariant', function () { return Object.getOwnPropertyDescriptor(new Proxy(frozen, { getOwnPropertyDescriptor: function () { return undefined; } }), 'x'); });
err('gopd bad', function () { return Object.getOwnPropertyDescriptor(new Proxy({}, { getOwnPropertyDescriptor: function () { return 1; } }), 'x'); });
err('defineProperty false', function () { return Object.defineProperty(new Proxy({}, { defineProperty: function () { return false; } }), 'x', { value: 1 }); });
err('defineProperty invariant', function () { return Object.defineProperty(new Proxy(Object.preventExtensions({}), { defineProperty: function () { return true; } }), 'x', { value: 1 }); });
err('getPrototypeOf bad', function () { return Object.getPrototypeOf(new Proxy({}, { getPrototypeOf: function () { return 1; } })); });
err('getPrototypeOf invariant', function () { return Object.getPrototypeOf(new Proxy(Object.preventExtensions({}), { getPrototypeOf: function () { return Array.prototype; } })); });
err('setPrototypeOf false', function () { return Object.setPrototypeOf(new Proxy({}, { setPrototypeOf: function () { return false; } }), null); });
err('isExtensible invariant', function () { return Object.isExtensible(new Proxy({}, { isExtensible: function () { return false; } })); });
err('preventExtensions invariant', function () { return Object.preventExtensions(new Proxy({}, { preventExtensions: function () { return true; } })); });
err('preventExtensions false', function () { return Object.preventExtensions(new Proxy({}, { preventExtensions: function () { return false; } })); });
err('apply non-callable', function () { return new Proxy({}, {})(); });
err('construct non-object', function () { return new (new Proxy(function () {}, { construct: function () { return 1; } }))(); });
err('construct non-constructor', function () { return new (new Proxy(function () {}.bind(), {}))(); });
err('arrow construct', function () { return new (new Proxy(() => {}, {}))(); });
errors.join('\n')"#;

#[test]
fn errors_read_as_the_reference_browser_gives_them() {
    assert_eq!(
        run(PROBE_ERRORS),
        r#"Proxy(): TypeError: Constructor Proxy requires 'new'
new Proxy(1): TypeError: Cannot create proxy with a non-object as target or handler
new Proxy handler 1: TypeError: Cannot create proxy with a non-object as target or handler
revoked get: TypeError: Cannot perform 'get' on a proxy that has been revoked
revoked typeof: no throw object
trap not function: TypeError: '1' returned for property 'get' of object '#<Object>' is not a function
get invariant: TypeError: 'get' on proxy: property 'x' is a read-only and non-configurable data property on the proxy target but the proxy did not return its actual value (expected '1' but got '2')
set invariant: TypeError: 'set' on proxy: trap returned truish for property 'x' which exists in the proxy target as a non-configurable and non-writable data property with a different value
set false strict: TypeError: 'set' on proxy: trap returned falsish for property 'x'
has invariant: TypeError: 'has' on proxy: trap returned falsish for property 'x' which exists in the proxy target as non-configurable
delete invariant: TypeError: 'deleteProperty' on proxy: trap returned truish for property 'x' which is non-configurable in the proxy target
delete false strict: TypeError: 'deleteProperty' on proxy: trap returned falsish for property 'y'
ownKeys not object: TypeError: CreateListFromArrayLike called on non-object
ownKeys missing: TypeError: 'ownKeys' on proxy: trap result did not include 'x'
ownKeys duplicate: TypeError: 'ownKeys' on proxy: trap returned duplicate entries
ownKeys bad type: TypeError: 1 is not a valid property name
gopd invariant: TypeError: 'getOwnPropertyDescriptor' on proxy: trap returned undefined for property 'x' which is non-configurable in the proxy target
gopd bad: TypeError: 'getOwnPropertyDescriptor' on proxy: trap returned neither object nor undefined for property 'x'
defineProperty false: TypeError: 'defineProperty' on proxy: trap returned falsish for property 'x'
defineProperty invariant: TypeError: 'defineProperty' on proxy: trap returned truish for adding property 'x'  to the non-extensible proxy target
getPrototypeOf bad: TypeError: 'getPrototypeOf' on proxy: trap returned neither object nor null
getPrototypeOf invariant: TypeError: 'getPrototypeOf' on proxy: proxy target is non-extensible but the trap did not return its actual prototype
setPrototypeOf false: TypeError: 'setPrototypeOf' on proxy: trap returned falsish for property 'undefined'
isExtensible invariant: TypeError: 'isExtensible' on proxy: trap result does not reflect extensibility of proxy target (which is 'true')
preventExtensions invariant: TypeError: 'preventExtensions' on proxy: trap returned truish but the proxy target is extensible
preventExtensions false: TypeError: 'preventExtensions' on proxy: trap returned falsish
apply non-callable: TypeError: (intermediate value) is not a function
construct non-object: TypeError: 'construct' on proxy: trap returned non-object ('1')
construct non-constructor: no throw [object Object]
arrow construct: TypeError: (intermediate value) is not a constructor"#
    );
}
