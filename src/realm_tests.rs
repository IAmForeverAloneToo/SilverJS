//! Several realms in one agent (ECMA-262 section 9.3), made through the
//! embedding API as Test262's `$262.createRealm` makes them: each its
//! own global object and built-ins, the running code's realm switched
//! at every call into a function of another.

use crate::agent::{Agent, AgentOptions, Outcome, Scope};
use crate::{JsResult, Thrown};

/// `createRealm()` on the current realm's global: a new realm's global
/// object, which has a `createRealm` of its own.
fn install<'s>(scope: &mut Scope<'s>) -> JsResult<'s, ()> {
    let Ok(global) = scope.global_object() else {
        return Err(Thrown::Terminated);
    };
    let create = scope.native_function("createRealm", 0, |scope, _, _| {
        let realm = scope.new_realm()?;
        scope.in_realm(realm, |scope| {
            install(scope)?;
            scope.global_object().map_err(|_| Thrown::Terminated)
        })
    })?;
    let key = scope.key("createRealm");
    scope.set_property(global, key, create)
}

/// What a script came to, `createRealm` given: its value's ToString, or
/// `throw ...`.
fn run(src: &str) -> String {
    let mut agent = Agent::new(AgentOptions::default());
    agent.scope(|scope| {
        if install(scope).is_err() {
            return "no host".into();
        }
        match scope.run_script(src) {
            Ok(Outcome::Value(v)) => scope
                .to_text(v)
                .unwrap_or_else(|_| "(ToString threw)".into()),
            Ok(Outcome::Threw(e)) => format!("throw {}", scope.to_text(e).unwrap_or_default()),
            Ok(Outcome::Terminated) => "terminated".into(),
            Err(e) => format!("compile error: {e}"),
        }
    })
}

#[test]
fn each_realm_has_its_own_globals_and_built_ins() {
    assert_eq!(
        run(
            "var other = createRealm(); [other === this, other.Array === Array, \
             other.Object.prototype === Object.prototype, \
             Object.getPrototypeOf(other) === other.Object.prototype, \
             other.Symbol.iterator === Symbol.iterator, \
             other.Symbol.for('k') === Symbol.for('k')].join()"
        ),
        "false,false,false,true,true,true"
    );
    // A realm of a realm is a third.
    assert_eq!(
        run("var a = createRealm(), b = a.createRealm(); \
             [b === a, b.Array === a.Array, b.Array === Array].join()"),
        "false,false,false"
    );
}

#[test]
fn code_runs_in_its_own_realm() {
    // A function of another realm sees that realm's globals, its sloppy
    // `this` that realm's global object; back here, this one's.
    assert_eq!(
        run("var x = 'here'; var other = createRealm(); \
             other.eval(\"var x = 'there'; function f() { return x + ':' + (this === globalThis); }\"); \
             [other.f(), x, other.x, (function () { return this === globalThis; })()].join()"),
        "there:true,here,there,true"
    );
    // Literals and errors a function makes are its realm's.
    assert_eq!(
        run("var other = createRealm(); \
             var mk = other.eval('(function () { return [[], {}, function () {}, /*e*/ (function () { try { null.x; } catch (e) { return e; } })()]; })'); \
             var r = mk(); \
             [r[0] instanceof other.Array, r[0] instanceof Array, \
              Object.getPrototypeOf(r[1]) === other.Object.prototype, \
              r[2] instanceof other.Function, r[3] instanceof other.TypeError, \
              r[3] instanceof TypeError].join()"),
        "true,false,true,true,true,false"
    );
}

#[test]
fn natives_throw_their_own_realms_errors() {
    assert_eq!(
        run("var other = createRealm(); \
             try { other.Function.prototype.call.call(1); 'none'; } \
             catch (e) { [e instanceof other.TypeError, e instanceof TypeError].join(); }"),
        "true,false"
    );
    // After that throw, this realm is current again.
    assert_eq!(
        run(
            "var other = createRealm(), home = globalThis, HomeArray = Array; \
             var HomeTypeError = TypeError; \
             try { other.Function.prototype.call.call(1); } catch (e) {} \
             try { other.eval('null.x'); } catch (e) {} \
             var g = other.eval('(function () { null.z; })'); \
             try { g(); } catch (e) {} \
             [globalThis === home, [] instanceof HomeArray, {} instanceof home.Object, \
              (function () { try { null.y; } catch (e) { return e instanceof HomeTypeError; } })()].join()"
        ),
        "true,true,true,true"
    );
    // The `Function` constructor compiles in its own realm.
    assert_eq!(
        run("var other = createRealm(); other.v = 'o'; var v = 'h'; \
             [other.Function('return v')(), Function('return v')()].join()"),
        "o,h"
    );
}

#[test]
fn a_prototype_not_an_object_falls_back_to_the_constructors_realm() {
    // GetPrototypeFromConstructor takes the default from new.target's
    // realm, a bound function's from its target's.
    assert_eq!(
        run(
            "var r1 = createRealm(), r2 = createRealm(), r3 = createRealm(); \
             var nt = new r1.Function(); nt.prototype = 'str'; \
             var bound = r2.Function.prototype.bind.call(nt); \
             var a = Reflect.construct(r3.Array, [], bound); \
             var e = Reflect.construct(r3.RangeError, [], nt); \
             var o = Reflect.construct(r3.Object, [], nt); \
             var s = Reflect.construct(r3.String, [], nt); \
             var plain = Reflect.construct(r3.Function('this.k = 1'), [], nt); \
             [Object.getPrototypeOf(a) === r1.Array.prototype, \
              Object.getPrototypeOf(e) === r1.RangeError.prototype, \
              Object.getPrototypeOf(o) === r1.Object.prototype, \
              Object.getPrototypeOf(s) === r1.String.prototype, \
              Object.getPrototypeOf(plain) === r1.Object.prototype].join()"
        ),
        "true,true,true,true,true"
    );
}

#[test]
fn another_realms_array_constructor_makes_this_realms_arrays() {
    // ArraySpeciesCreate: an array whose constructor is another realm's
    // `Array` gives an array of the running realm's, its @@species not
    // read.
    assert_eq!(
        run("var OArray = createRealm().Array, reads = 0; \
             var d = { get: function () { reads++; } }; \
             Object.defineProperty(Array, Symbol.species, d); \
             Object.defineProperty(OArray, Symbol.species, d); \
             var a = []; a.constructor = OArray; \
             var rs = [a.concat(), a.map(function (x) { return x; }), a.filter(Boolean), a.slice(), a.splice(0)]; \
             rs.every(function (r) { return Object.getPrototypeOf(r) === Array.prototype; }) + ':' + reads"),
        "true:0"
    );
    // Another realm's constructor that is not its `Array` is asked.
    assert_eq!(
        run("var F = createRealm().Function(''); \
             function Mine(n) { this.n = n; } \
             F[Symbol.species] = Mine; \
             var a = [1, 2]; a.constructor = F; \
             var m = a.map(function (x) { return x; }); \
             [m instanceof Mine, m[1]].join()"),
        "true,2"
    );
}

#[test]
fn eval_is_direct_only_for_the_callers_realms_eval() {
    assert_eq!(
        run("var other = createRealm(); other.x = 'o'; \
             (function () { var x = 'local'; return [eval('x'), other.eval('x')].join(); })()"),
        "local,o"
    );
    // Another realm's eval run indirectly makes its vars on that realm's
    // global object.
    assert_eq!(
        run("var other = createRealm(); other.eval('var made = 1'); \
             [typeof made, other.made].join()"),
        "undefined,1"
    );
}

#[test]
fn realms_survive_collection() {
    let mut agent = Agent::new(AgentOptions::default());
    agent.set_stress(true);
    let out = agent.scope(|scope| {
        if install(scope).is_err() {
            return "no host".to_owned();
        }
        match scope.run_script(
            "var other = createRealm(); other.eval('var kept = [1, 2]'); \
             createRealm(); \
             [other.kept.join(), other.Array.isArray(other.kept), other.kept instanceof other.Array].join()",
        ) {
            Ok(Outcome::Value(v)) => scope.to_text(v).unwrap_or_default(),
            _ => "failed".to_owned(),
        }
    });
    assert_eq!(out, "1,2,true,true");
}

#[test]
fn the_embedder_picks_the_realm_code_runs_in() {
    let mut agent = Agent::new(AgentOptions::default());
    agent.scope(|scope| {
        let first = scope.current_realm().ok();
        let Ok(second) = scope.new_realm() else {
            panic!("no realm");
        };
        // Making a realm leaves the current one current.
        assert_eq!(scope.current_realm().ok(), first);
        assert_ne!(Some(second), first);
        let inside = scope.in_realm(second, |scope| scope.current_realm().ok());
        assert_eq!(inside, Some(second));
        // Each realm's own global lexical bindings.
        let _ = ran(scope, "let where = 'first'");
        let got = scope.in_realm(second, |scope| {
            let _ = ran(scope, "let where = 'second'");
            ran(scope, "where")
        });
        assert_eq!(got, "second");
        assert_eq!(ran(scope, "where"), "first");
        // A realm id no realm of this agent's has runs in the current.
        let mut other = Agent::new(AgentOptions::default());
        let foreign = other.scope(|s| {
            let _ = s.new_realm();
            s.new_realm().ok()
        });
        if let Some(foreign) = foreign {
            assert_eq!(
                scope.in_realm(foreign, |scope| ran(scope, "where")),
                "first"
            );
        }
    });
}

/// A script's completion value's ToString, run in the current realm.
fn ran(scope: &mut Scope<'_>, src: &str) -> String {
    match scope.run_script(src) {
        Ok(Outcome::Value(v)) => scope.to_text(v).unwrap_or_default(),
        _ => "none".to_owned(),
    }
}
