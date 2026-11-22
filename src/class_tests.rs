//! Classes (J3.3a): ClassDefinitionEvaluation, methods and accessors with
//! their home objects, `super` reads, writes and calls, derived
//! constructors' `this`, and the surface the reference browser shows (the
//! class probe, Chrome 157). Fields, private names and static blocks are
//! J3.3b's.

use crate::agent::{Agent, AgentOptions, Outcome};

fn run(src: &str) -> String {
    let mut agent = Agent::new(AgentOptions::default());
    run_in(&mut agent, src)
}

fn run_in(agent: &mut Agent, src: &str) -> String {
    agent.scope(|scope| match scope.run_script_named(src, "u.js") {
        Ok(Outcome::Value(v)) => scope
            .to_text(v)
            .unwrap_or_else(|_| "(ToString threw)".into()),
        Ok(Outcome::Threw(e)) => {
            let text = scope.to_text(e).unwrap_or_default();
            format!("throw {text}")
        }
        Ok(Outcome::Terminated) => "terminated".into(),
        Err(e) => format!("compile error: {e}"),
    })
}

/// `keys(o)`: own keys in order, with their kinds and attributes, as the
/// probe lists them.
const KEYS: &str = "function keys(o) { return Reflect.ownKeys(o).map(function (k) { \
    var d = Object.getOwnPropertyDescriptor(o, k); \
    var kind = 'get' in d ? 'accessor' : typeof d.value; \
    var flags = (d.writable ? 'w' : '-') + (d.enumerable ? 'e' : '-') + (d.configurable ? 'c' : '-'); \
    var extra = ''; \
    if (typeof d.value === 'function') extra = ' len=' + d.value.length + ' name=' + d.value.name; \
    if (d.get) extra = ' get=' + d.get.name + (d.set ? ' set=' + d.set.name : ''); \
    return String(k) + ' [' + kind + ' ' + flags + extra + ']'; }).join(', '); }\n";

const A: &str = "class A { constructor(a, b) { this.y = a; } m() { return 7; } \
    get g() { return 1; } set g(v) {} static sm() {} [Symbol.iterator]() {} \
    'quoted'() {} 1() {} }\n";

fn with_a(body: &str) -> String {
    run(&format!("{KEYS}{A}{body}"))
}

/// `err(f)`: what `f` throws, as `Type: message`.
const ERR: &str = "function err(f) { try { return 'no throw ' + String(f()); } \
    catch (e) { return e.constructor.name + ': ' + e.message; } }\n";

fn error(body: &str) -> String {
    run(&format!("{ERR}class C {{}}\n{body}"))
}

#[test]
fn a_class_has_chromes_shape() {
    // The constructor's keys: length, name, prototype, then its static
    // members in order; the prototype's: integer keys first, then
    // constructor and the methods in order, symbols last.
    assert_eq!(
        with_a("keys(A)"),
        "length [number --c], name [string --c], prototype [object ---], \
         sm [function w-c len=0 name=sm]"
    );
    assert_eq!(
        with_a("keys(A.prototype)"),
        "1 [function w-c len=0 name=1], constructor [function w-c len=2 name=A], \
         m [function w-c len=0 name=m], g [accessor --c get=get g set=set g], \
         quoted [function w-c len=0 name=quoted], \
         Symbol(Symbol.iterator) [function w-c len=0 name=[Symbol.iterator]]"
    );
    assert_eq!(with_a("keys(new A(5))"), "y [number wec]");
    // A method has no prototype; it is no constructor.
    assert_eq!(
        with_a("keys(A.prototype.m)"),
        "length [number --c], name [string --c]"
    );
    assert_eq!(
        with_a("class B extends A { constructor() { super(1); } m() {} } keys(B.prototype)"),
        "constructor [function w-c len=0 name=B], m [function w-c len=0 name=m]"
    );
    // A default constructor, named after its class or its binding.
    assert_eq!(
        with_a(
            "var anon = class {}; class D extends A {} [keys(anon), keys(D), anon.name, D.length].join(' / ')"
        ),
        "length [number --c], name [string --c], prototype [object ---] / \
         length [number --c], name [string --c], prototype [object ---] / anon / 0"
    );
    // Its source text, and its methods'.
    assert_eq!(
        with_a(
            "class C {} class D extends C {} \
             [String(C), String(D), String(class {}), String(A.prototype.m), String(A.sm), \
             String(Object.getOwnPropertyDescriptor(A.prototype, 'g').get)].join(' | ')"
        ),
        "class C {} | class D extends C {} | class {} | m() { return 7; } | static sm() {} | get g() { return 1; }"
    );
}

#[test]
fn classes_link_their_parents() {
    assert_eq!(
        with_a(
            "class B extends A { constructor() { super(1); this.z = 3; } m() { return super.m() + 1; } } \
             var named = class Inner { who() { return Inner; } }; \
             [Object.getPrototypeOf(B) === A, Object.getPrototypeOf(B.prototype) === A.prototype, \
             Object.getPrototypeOf(class extends null {}.prototype) === null, \
             Object.getPrototypeOf(class extends null {}) === Function.prototype, typeof A, \
             named.name, new B().z, new B().y, new B().m(), named.prototype.who() === named, \
             'prototype' in A.prototype.m, new (class { constructor() { return { r: 1 }; } })().r, \
             Object.prototype.toString.call(new A(1)), new B() instanceof A].join()"
        ),
        "true,true,true,true,function,Inner,3,1,8,true,false,1,[object Object],true"
    );
    // A built-in parent makes the instance: an array, with its length.
    assert_eq!(
        run("class E extends Array {} var e = new E(); e.push(1, 2); \
             [e.length, e instanceof E, Array.isArray(e), Object.getPrototypeOf(e) === E.prototype].join()"),
        "2,true,true,true"
    );
    // new.target is the class newed, through every super().
    assert_eq!(
        run(
            "class P { constructor() { this.nt = new.target; } } class Q extends P {} class R extends Q {} \
             [new R().nt === R, new P().nt === P].join()"
        ),
        "true,true"
    );
}

#[test]
fn a_default_derived_constructor_passes_its_arguments_on() {
    // As they came: not through the array iterator (section 15.7.15).
    assert_eq!(
        run(
            "class P { constructor(...a) { this.a = a; } } class X extends P {} \
             var saved = Array.prototype[Symbol.iterator]; \
             Array.prototype[Symbol.iterator] = function () { throw new Error('iterated'); }; \
             try { new X(1, 2, 3).a.length } finally { Array.prototype[Symbol.iterator] = saved; }"
        ),
        "3"
    );
    assert_eq!(
        run("class P { constructor(...a) { this.a = a; } } \
             class X extends P { constructor(...a) { super(...a, 9); } } \
             class Y extends P { constructor(a, b) { super(b, a); } } \
             [new X(1, 2).a.join(), new Y(1, 2).a.join()].join(' ')"),
        "1,2,9 2,1"
    );
}

#[test]
fn super_reads_and_writes_go_through_the_home_object() {
    // Read on the home object's prototype, with this as the receiver.
    assert_eq!(
        run(
            "class P { get v() { return this.t; } m() { return 'p' + this.t; } } \
             class X extends P { constructor() { super(); this.t = 'T'; } \
             m() { return super.m() + super.v + super['m'].call({ t: 'o' }); } } \
             new X().m()"
        ),
        "pTTpo"
    );
    // Written: a setter on the prototype runs with this; else the
    // property lands on this. A compound write reads the prototype's.
    assert_eq!(
        run(
            "var log = []; class P { set s(v) { log.push(this.tag + v); } } \
             class X extends P { constructor() { super(); this.tag = 'x'; } \
             m() { super.s = 4; super.q = 5; super.n = 1; super.n++; super.n += 2; return this.q + ',' + this.n; } } \
             P.prototype.n = 100; var r = new X().m(); r + ',' + log.join()"
        ),
        "5,102,x4"
    );
    // A constructor's home object is its prototype; a super call's value
    // goes on along a chain.
    assert_eq!(
        run("class P { m() { return 'pm' + this.tag; } } \
             class X extends P { constructor() { super(); this.tag = 'T'; this.r = super.m(); } \
             len() { return super.m().length + super.m().toUpperCase(); } } \
             var x = new X(); x.r + x.len()"),
        "pmT3PMT"
    );
    // Static methods: the home object is the class.
    assert_eq!(
        run("class P { static who() { return 'P' + this.name; } } \
             class X extends P { static who() { return super.who() + '!'; } } X.who()"),
        "PX!"
    );
    // Object literals' methods have their home objects too.
    assert_eq!(
        run(
            "var proto = { hi() { return 'proto ' + this.n; }, get g() { return 'G'; } }; \
             var o = { __proto__: proto, n: 1, hi() { return super.hi() + '!'; }, \
             get g() { return super.g + 'g'; }, ['c' + 1]() { return super.hi(); } }; \
             [o.hi(), o.g, o.c1()].join()"
        ),
        "proto 1!,Gg,proto 1"
    );
    // The home object is fixed: a method borrowed keeps it.
    assert_eq!(
        run(
            "class P { m() { return 'P'; } } class X extends P { m() { return super.m(); } } \
             var other = { m: X.prototype.m }; other.m()"
        ),
        "P"
    );
    // `super.m?.()` short-circuits; generators and arrows see super.
    assert_eq!(
        run("class P { v() { return 'p'; } } \
             class X extends P { m() { return String(super.nope?.()); } \
             *g() { yield super.v(); } a() { return (() => super.v())(); } } \
             var x = new X(); [x.m(), x.g().next().value, x.a()].join()"),
        "undefined,p,p"
    );
}

#[test]
fn a_derived_constructors_this_waits_for_super() {
    assert_eq!(
        error(
            "[err(function () { class X extends C { constructor() { this.a = 1; super(); } } new X(); }), \
             err(function () { class X extends C { constructor() { super(); super(); } } new X(); }), \
             err(function () { class X extends C { constructor() {} } new X(); }), \
             err(function () { class X extends C { constructor() { super(); return 1; } } new X(); }), \
             err(function () { class X extends C { constructor() { var f = () => this; f(); super(); } } new X(); })].join('\\n')"
        ),
        "ReferenceError: Must call super constructor in derived class before accessing 'this' or returning from derived constructor\n\
         ReferenceError: Super constructor may only be called once\n\
         ReferenceError: Must call super constructor in derived class before accessing 'this' or returning from derived constructor\n\
         TypeError: Derived constructors may only return object or undefined\n\
         ReferenceError: Must call super constructor in derived class before accessing 'this' or returning from derived constructor"
    );
    // An object returned replaces this; undefined needs it bound.
    assert_eq!(
        error(
            "[err(function () { class X extends C { constructor() { return { a: 1 }; } } return new X().a; }), \
             err(function () { class X extends C { constructor() { super(); return undefined; } } return new X() instanceof X; }), \
             err(function () { class X extends C { constructor() { return undefined; } } new X(); })].join('\\n')"
        ),
        "no throw 1\nno throw true\n\
         ReferenceError: Must call super constructor in derived class before accessing 'this' or returning from derived constructor"
    );
    // super() in an arrow binds the constructor's this, once; a return is
    // checked after its finally blocks run.
    assert_eq!(
        error(
            "[err(function () { class X extends C { constructor() { var f = () => super(); f(); this.k = 1; } } return new X().k; }), \
             err(function () { class X extends C { constructor() { var f = () => super(); f(); f(); } } new X(); }), \
             err(function () { class X extends C { constructor() { try { return; } finally { super(); } } } return new X() instanceof X; }), \
             err(function () { class X extends C { constructor() { try { return 1; } finally { super(); } } } new X(); })].join('\\n')"
        ),
        "no throw 1\nReferenceError: Super constructor may only be called once\nno throw true\n\
         TypeError: Derived constructors may only return object or undefined"
    );
}

#[test]
fn classes_throw_chromes_errors() {
    assert_eq!(
        error(
            "var anon = class {}; \
             [err(function () { C(); }), err(function () { anon(); }), \
             err(function () { class X extends 1 {} }), \
             err(function () { class X extends Math.max {} }), \
             err(function () { function F() {} F.prototype = 1; class X extends F {} }), \
             err(function () { class X extends X {} }), \
             err(function () { class X { [X]() {} } }), \
             err(function () { class X { m() { return super.m(); } } Object.setPrototypeOf(X.prototype, null); new X().m(); }), \
             err(function () { class X extends C { constructor() { super(); } } Object.setPrototypeOf(X, Math.max); new X(); }), \
             err(function () { class X { m() { delete super.x; } } new X().m(); }), \
             err(function () { class X extends null {} new X(); }), \
             err(function () { new (class extends null {})(); }), \
             err(function () { class X { m() {} } new X.prototype.m(); }), \
             err(function () { class X { static ['prototype']() {} } }), \
             err(function () { var K = class Kk { m() { Kk = 1; } }; new K().m(); })].join('\\n')"
        ),
        "TypeError: Class constructor C cannot be invoked without 'new'\n\
         TypeError: Class constructor anon cannot be invoked without 'new'\n\
         TypeError: Class extends value 1 is not a constructor or null\n\
         TypeError: Class extends value function max() { [native code] } is not a constructor or null\n\
         TypeError: Class extends value does not have valid prototype property 1\n\
         ReferenceError: Cannot access 'X' before initialization\n\
         ReferenceError: Cannot access 'X' before initialization\n\
         TypeError: Cannot read properties of null (reading 'm')\n\
         TypeError: Super constructor max of X is not a constructor\n\
         ReferenceError: Unsupported reference to 'super'\n\
         TypeError: Super constructor null of X is not a constructor\n\
         TypeError: Super constructor null of anonymous class is not a constructor\n\
         TypeError: X.prototype.m is not a constructor\n\
         TypeError: Cannot redefine property: prototype\n\
         TypeError: Assignment to constant variable."
    );
    // Class code is strict.
    assert_eq!(
        run("class X { m() { return this; } } String(X.prototype.m.call(undefined))"),
        "undefined"
    );
}

#[test]
fn class_frames_show_on_the_stack() {
    // Chrome 157: `new S`, `S.m`, `S.st`, `get acc`; a derived
    // constructor's super() call at `super`; `new <anonymous>`.
    let src = "class S {\n\
               constructor() { this.e = new Error('ctor'); }\n\
               m() { return new Error('m'); }\n\
               static st() { return new Error('st'); }\n\
               get acc() { return new Error('acc'); }\n\
               }\n\
               class T extends S { constructor() { super(); this.f = new Error('derived'); } }\n\
               function top(e) { return e.stack.split('\\n').slice(0, 3).join(' | '); }\n\
               [top(new S().e), top(new S().m()), top(S.st()), top(new S().acc), top(new T().e), \
               top(new T().f), top(new (class { constructor() { this.e = new Error('anon'); } })().e)].join('\\n')";
    assert_eq!(
        run(src),
        "Error: ctor |     at new S (u.js:2:26) |     at u.js:9:6\n\
         Error: m |     at S.m (u.js:3:14) |     at u.js:9:30\n\
         Error: st |     at S.st (u.js:4:22) |     at u.js:9:42\n\
         Error: acc |     at get acc (u.js:5:20) |     at u.js:9:61\n\
         Error: ctor |     at new S (u.js:2:26) |     at new T (u.js:7:37)\n\
         Error: derived |     at new T (u.js:7:55) |     at u.js:9:87\n\
         Error: anon |     at new <anonymous> (u.js:9:141) |     at u.js:9:103"
    );
}

#[test]
fn classes_survive_collection() {
    let mut agent = Agent::new(AgentOptions::default());
    agent.set_stress(true);
    assert_eq!(
        run_in(
            &mut agent,
            "class P { constructor(n) { this.n = n; } get d() { return this.n * 2; } } \
             class X extends P { constructor(n) { super(n + 1); } m() { return super.d + this.n; } } \
             var out = []; for (var i = 0; i < 20; i++) { var K = class extends X {}; out.push(new K(i).m()); } \
             out.join()"
        ),
        "3,6,9,12,15,18,21,24,27,30,33,36,39,42,45,48,51,54,57,60"
    );
}
