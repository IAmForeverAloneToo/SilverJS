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
        "class C {} | class D extends C {} | class {} | m() { return 7; } | sm() {} | get g() { return 1; }"
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

/// `pkeys(o)`: own keys in order, with their attributes.
const PKEYS: &str = "function pkeys(o) { return Reflect.ownKeys(o).map(function (k) { \
    var d = Object.getOwnPropertyDescriptor(o, k); \
    return String(k) + ' ' + (d.writable ? 'w' : '-') + (d.enumerable ? 'e' : '-') + (d.configurable ? 'c' : '-'); \
    }).join(', '); }\n";

fn fields(body: &str) -> String {
    run(&format!("{ERR}{PKEYS}{body}"))
}

#[test]
fn fields_are_defined_in_order() {
    // Chrome 157, the second class probe: data properties, integer keys
    // first; private ones show nowhere.
    assert_eq!(
        fields(
            "class X { b = 1; a = 2; #p = 0; static z = 1; ['c'] = 3; 1 = 4; static #q = 0; } \
             pkeys(new X()) + ' / ' + pkeys(X) + ' / ' + Object.getOwnPropertySymbols(new X()).length"
        ),
        "1 wec, b wec, a wec, c wec / length --c, name --c, prototype ---, z wec / 0"
    );
    // Keys are evaluated with the class, values at each construction;
    // static elements run in order once the class is made.
    assert_eq!(
        fields(
            "var log = []; class X { [(log.push('k1'), 'a')] = log.push('v1'); [(log.push('k2'), 'b')] = log.push('v2'); } \
             log.push('made'); new X(); var a = log.join(); log = []; \
             class Y { a = log.push('ia'); static b = log.push('sb'); static { log.push('blk'); } static c = log.push('sc'); } \
             log.push('made'); new Y(); a + ' / ' + log.join()"
        ),
        "k1,k2,made,v1,v2 / sb,blk,sc,made,ia"
    );
    // Defined, not set: a setter on the parent is not called. A frozen
    // object takes no public field, but private ones.
    assert_eq!(
        fields(
            "class P { set f(v) { throw new Error('setter'); } } class X extends P { f = 1; } \
             class Q { constructor() { return Object.freeze({}); } } \
             class Y extends Q { f = 1; } class Z extends Q { #f = 1; static has(o) { return #f in o; } } \
             [pkeys(new X()), err(function () { new Y(); }), Z.has(new Z())].join('\\n')"
        ),
        "f wec\nTypeError: Cannot define property f, object is not extensible\ntrue"
    );
    // An initializer's this, arrows in it, super, new.target; a static
    // one's this is the class; names given to anonymous functions.
    assert_eq!(
        fields(
            "class P { m() { return 'pm'; } constructor() { this.seen = this.f; } } \
             class X extends P { f = 1; g = () => this; h = super.m(); nt = new.target; } \
             var x = new X(); \
             class S { static a = 1; static b = this.a + 1; static { this.k = 'blk' + S.b; } } \
             class N { f = function () {}; g = () => {}; #h = function () {}; static s = class {}; getH() { return this.#h; } } \
             var n = new N(); \
             class SP { static sm() { return 'sm'; } } \
             class SC extends SP { static t = super.sm(); static { this.u = super.sm(); } } \
             [x.seen, x.g() === x, x.h, x.nt, S.b, S.k, n.f.name, n.g.name, n.getH().name, N.s.name, SC.t + SC.u].join()"
        ),
        ",true,pm,,2,blk2,f,g,#h,s,smsm"
    );
    assert_eq!(
        fields(
            "class X { f = (() => { throw new Error('in field'); })(); } err(function () { new X(); })"
        ),
        "Error: in field"
    );
}

#[test]
fn private_names_reach_what_their_class_declared() {
    assert_eq!(
        fields(
            "class X { #p = 1; #m() { return this.#p; } get #g() { return this.#p * 10; } set #g(v) { this.#p = v; } \
             static #s = 's'; static #sm() { return X.#s; } \
             run() { this.#g = 4; this.#p++; this.#p += 1; return [this.#m(), this.#g, X.#sm(), #p in this, #m in this, #g in {}].join(); } } \
             new X().run()"
        ),
        "6,60,s,true,true,false"
    );
    // Each evaluation of a class makes new names.
    assert_eq!(
        fields(
            "function make() { return class { #p = 1; static has(o) { return #p in o; } }; } \
             var A = make(), B = make(); [A.has(new A()), A.has(new B())].join()"
        ),
        "true,false"
    );
    // Chrome 157's messages, from the class probes.
    assert_eq!(
        fields(
            "class Base { constructor(o) { return o; } } \
             [err(function () { class X { #p = 1; static read(o) { return o.#p; } } X.read({}); }), \
             err(function () { class X { #p = 1; static write(o) { o.#p = 2; } } X.write({}); }), \
             err(function () { class X { #m() {} static call(o) { o.#m(); } } X.call({}); }), \
             err(function () { class X { get #g() { return 1; } static w(o) { o.#g = 2; } } X.w(new X()); }), \
             err(function () { class X { set #s(v) {} static r(o) { return o.#s; } } X.r(new X()); }), \
             err(function () { class X { #m() {} static w(o) { o.#m = 1; } } X.w(new X()); }), \
             err(function () { class X extends Base { #p = 1; } var o = {}; new X(o); new X(o); }), \
             err(function () { class X extends Base { #m() {} } var o = {}; new X(o); new X(o); }), \
             err(function () { class X { #p; static has(o) { return #p in o; } } X.has(1); }), \
             err(function () { class X { static #p = 1; static r(o) { return o.#p; } } class Y extends X {} return Y.r(Y); }), \
             err(function () { class X { static #m() {} static c(o) { o.#m(); } } X.c({}); }), \
             err(function () { class X { get #a() { return 1; } static r(o) { return o.#a; } } X.r({}); }), \
             err(function () { class X { static set #s(v) {} static r() { return X.#s; } } X.r(); }), \
             err(function () { class X { set #s(v) {} static w(o) { o.#s = 1; } } X.w({}); })].join('\\n')"
        ),
        "TypeError: Cannot read private member #p from an object whose class did not declare it\n\
         TypeError: Cannot write private member #p to an object whose class did not declare it\n\
         TypeError: Receiver must be an instance of class X\n\
         TypeError: '#g' was defined without a setter\n\
         TypeError: '#s' was defined without a getter\n\
         TypeError: Private method '#m' is not writable\n\
         TypeError: Cannot initialize #p twice on the same object\n\
         TypeError: Cannot initialize private methods of class X twice on the same object\n\
         TypeError: Cannot use 'in' operator to search for '#p' in 1\n\
         TypeError: Cannot read private member #p from an object whose class did not declare it\n\
         TypeError: Receiver must be class X\n\
         TypeError: Receiver must be an instance of class X\n\
         TypeError: '#s' was defined without a getter\n\
         TypeError: Receiver must be an instance of class X"
    );
    // A private method's name and source text.
    assert_eq!(
        fields(
            "class X { #m() {} get #g() { return 1; } static #sm() {} \
             static n(o) { return [o.#m.name, String(o.#m), X.#sm.name].join(); } } X.n(new X())"
        ),
        "#m,#m() {},#sm"
    );
}

#[test]
fn initializer_frames_show_on_the_stack() {
    // Chrome 157: `<instance_members_initializer>` called from the
    // constructor (a default one's at its class, a derived one's at its
    // super()), `<static_initializer>`, and private methods by their
    // names alone.
    let src = "function top(e) { return e.stack.split('\\n').slice(0, 3).join(' | '); }\n\
               class S {\n\
               f = new Error('field');\n\
               static sf = new Error('static field');\n\
               static { S.blk = new Error('block'); }\n\
               #pm() { return new Error('privm'); }\n\
               static #spm() { return new Error('sprivm'); }\n\
               get #pg() { return new Error('privg'); }\n\
               callPm() { return this.#pm(); }\n\
               static callSpm() { return S.#spm(); }\n\
               callPg() { return this.#pg; }\n\
               }\n\
               class T extends S { g = new Error('derived field'); constructor() { super(); } }\n\
               var s = new S();\n\
               [top(s.f), top(S.sf).split(' | ')[1], top(S.blk).split(' | ')[1], top(s.callPm()), \
               top(S.callSpm()), top(s.callPg()), top(new T().g)].join('\\n')";
    assert_eq!(
        run(src),
        "Error: field |     at <instance_members_initializer> (u.js:3:5) |     at new S (u.js:2:1)\n    \
         at <static_initializer> (u.js:4:13)\n    \
         at <static_initializer> (u.js:5:18)\n\
         Error: privm |     at #pm (u.js:6:16) |     at S.callPm (u.js:9:27)\n\
         Error: sprivm |     at #spm (u.js:7:24) |     at S.callSpm (u.js:10:33)\n\
         Error: privg |     at get #pg (u.js:8:20) |     at S.callPg (u.js:11:24)\n\
         Error: derived field |     at <instance_members_initializer> (u.js:13:25) |     at new T (u.js:13:69)"
    );
}

#[test]
fn fields_and_private_names_survive_collection() {
    let mut agent = Agent::new(AgentOptions::default());
    agent.set_stress(true);
    assert_eq!(
        run_in(
            &mut agent,
            "var out = []; for (var i = 0; i < 20; i++) { \
             var K = class { #n = i; tag = 't' + i; #m() { return this.#n * 2; } get v() { return this.#m() + this.tag.length; } \
             static #count = 0; static inc() { return ++K.#count; } }; \
             out.push(new K().v + K.inc()); } out.join()"
        ),
        "3,5,7,9,11,13,15,17,19,21,24,26,28,30,32,34,36,38,40,42"
    );
}

#[test]
fn a_super_reference_reads_its_base_before_converting_its_key() {
    // The base is the home object's prototype when the reference is
    // made; the key is converted when it is read or written, once for a
    // compound form (section 13.3.7, Test262's getsuperbase tests).
    assert_eq!(
        run(
            "var log = []; var proto = { p: 'ok', set q(v) { log.push('ok' + v); } }; \
             var proto2 = { p: 'bad', set q(v) { log.push('bad' + v); } }; \
             var obj = { __proto__: proto, m() { return super[key]; }, w() { super[wkey] = 1; } }; \
             var key = { toString() { Object.setPrototypeOf(obj, proto2); return 'p'; } }; \
             var wkey = { toString() { Object.setPrototypeOf(obj, proto2); return 'q'; } }; \
             var r = obj.m(); Object.setPrototypeOf(obj, proto); obj.w(); \
             var n = 0; var once = { toString() { n++; return 'c'; } }; \
             var o2 = { __proto__: { c: 1 }, m() { super[once] += 1; super[once]++; return this.c; } }; \
             [r, log.join(), o2.m(), n].join()"
        ),
        "ok,ok1,2,2"
    );
    // A sloppy method's write that the target refuses is ignored; a
    // strict one's throws.
    assert_eq!(
        run(
            "var proto = {}; Object.defineProperty(proto, 'x', { value: 1, writable: false }); \
             var sloppy = { __proto__: proto, m() { super.x = 2; return this.x; } }; \
             var strict = { __proto__: proto, m() { 'use strict'; super.x = 2; } }; \
             var r; try { strict.m(); r = 'no throw'; } catch (e) { r = e.constructor.name; } \
             [sloppy.m(), r].join()"
        ),
        "1,TypeError"
    );
}

#[test]
fn eval_in_class_code_sees_super_and_private_names() {
    // Chrome 157, the second class probe.
    assert_eq!(
        fields(
            "class P { get q() { return 'Q'; } } \
             class X extends P { #p = 'P'; f = eval('super.q'); g = eval('new.target'); m() { return eval('this.#p + super.q'); } \
             n() { return (() => eval('super.q'))(); } } \
             var x = new X(); \
             [x.f, x.g, x.m(), x.n(), \
             err(function () { class Y { #a; m() { return eval('this.#q'); } } return new Y().m(); }), \
             err(function () { return eval('(class { f = () => eval(\"arguments\"); })'); }), \
             err(function () { class Z { f = eval('arguments'); } new Z(); }), \
             err(function () { function plain() { return eval('super.x'); } plain(); })].join('\\n')"
        ),
        "Q\n\nPQ\nQ\n\
         SyntaxError: Private field '#q' must be declared in an enclosing class\n\
         no throw class { f = () => eval(\"arguments\"); }\n\
         SyntaxError: 'arguments' is not allowed in class field initializer or static initialization block\n\
         SyntaxError: 'super' keyword unexpected here"
    );
}
