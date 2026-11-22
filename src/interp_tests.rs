//! The interpreter: scripts run end to end through the embedding API,
//! their results and errors checked, the errors' messages against the
//! reference browser's observed wording (docs/chrome-surface.md).

use crate::agent::{Agent, AgentOptions, GetResult, Outcome, Scope};

/// What a script came to, as text: a number, a string in quotes, a
/// literal, or `throw Name: message`.
fn describe(scope: &mut Scope<'_>, outcome: Outcome<'_>) -> String {
    match outcome {
        Outcome::Value(v) => {
            if let Some(n) = scope.as_number(v) {
                return crate::number::number_to_string(n);
            }
            if let Some(s) = scope.string_text(v) {
                return format!("{s:?}");
            }
            let u = scope.undefined();
            if scope.same(v, u) {
                return "undefined".into();
            }
            format!("{:?}", scope.value_for_test(v))
        }
        Outcome::Threw(e) => {
            let name = scope.key("name");
            let message = scope.key("message");
            let read = |scope: &mut Scope<'_>, k| match scope.get(e, k) {
                Some(GetResult::Value(v)) => scope.string_text(v).unwrap_or_default(),
                _ => String::new(),
            };
            let n = read(scope, name);
            let m = read(scope, message);
            format!("throw {n}: {m}")
        }
        Outcome::Terminated => "terminated".into(),
    }
}

fn run_in(agent: &mut Agent, src: &str) -> String {
    agent.scope(|scope| match scope.run_script(src) {
        Ok(outcome) => describe(scope, outcome),
        Err(e) => format!("compile error: {e}"),
    })
}

fn run(src: &str) -> String {
    let mut agent = Agent::new(AgentOptions::default());
    run_in(&mut agent, src)
}

#[test]
fn expressions_and_bindings() {
    assert_eq!(run("1 + 2 * 3"), "7");
    assert_eq!(run("let a = 5; a * 2 - 1"), "9");
    assert_eq!(run("0.1 + 0.2"), "0.30000000000000004");
    assert_eq!(run("2 ** 10"), "1024");
    assert_eq!(run("7 % 3 + (-7 % 3)"), "0");
    assert_eq!(run("(1 << 31) >>> 0"), "2147483648");
    assert_eq!(run("~5 | 2"), "-6");
    assert_eq!(run("2147483647 + 1"), "2147483648");
    assert_eq!(run("'a' + 1 + 2"), "\"a12\"");
    assert_eq!(run("1 + 2 + 'a'"), "\"3a\"");
    assert_eq!(run("'5' * '2'"), "10");
    assert_eq!(run("+'  0x10  ' + +'1e3' + +''"), "1016");
    assert_eq!(
        run("notAGlobal"),
        "throw ReferenceError: notAGlobal is not defined"
    );
    assert_eq!(
        run("typeof 1 + typeof 'x' + typeof undefined + typeof null + typeof function () {}"),
        "\"numberstringundefinedobjectfunction\""
    );
    assert_eq!(run("typeof notDeclared"), "\"undefined\"");
    assert_eq!(run("var x = 1; x += 2; x++; x"), "4");
    assert_eq!(run("1 < 2 && 'a' < 'b' && !(2 < 1) && 'b' > 'a'"), "true");
    assert_eq!(
        run("null == undefined && 1 == '1' && 0 == false && !(null == 0)"),
        run("true")
    );
    assert_eq!(run("NaN === NaN || 0 === -0"), run("true"));
    assert_eq!(run("`a${1 + 1}b${'c'}`"), "\"a2bc\"");
    assert_eq!(run("let o = null; o ?? 'd'"), "\"d\"");
    assert_eq!(run("void 0"), "undefined");
}

#[test]
fn functions_closures_and_recursion() {
    // A sloppy call with no receiver gets the global object.
    assert_eq!(
        run(
            "function f() { return this; } (f() === globalThis) + ',' + (f.call(null) === globalThis)"
        ),
        "\"true,true\""
    );
    assert_eq!(
        run("function fib(n) { return n < 2 ? n : fib(n - 1) + fib(n - 2); } fib(20)"),
        "6765"
    );
    assert_eq!(
        run(
            "function counter() { let n = 0; return () => ++n; } const c = counter(); c(); c(); c()"
        ),
        "3"
    );
    // Each iteration's `let` is a binding of its own.
    assert_eq!(
        run(
            "const fs = []; for (let i = 0; i < 3; i++) fs.push(() => i); fs[0]() + fs[1]() * 10 + fs[2]() * 100"
        ),
        "210"
    );
    assert_eq!(
        run(
            "function f(a, b = a + 1, ...rest) { return a + b + rest.length; } f(1) + f(1, 5, 0, 0)"
        ),
        "11"
    );
    assert_eq!(
        run("function f() { return arguments.length + arguments[1]; } f(1, 2, 3)"),
        "5"
    );
    assert_eq!(
        run("(function g(n) { return n ? g(n - 1) + 1 : 0; })(5)"),
        "5"
    );
    assert_eq!(run("var o = { k: 2, m() { return this.k; } }; o.m()"), "2");
    assert_eq!(
        run(
            "function P(x) { this.x = x; } P.prototype.get = function () { return this.x * 2; }; new P(4).get()"
        ),
        "8"
    );
    assert_eq!(
        run("function P() {} var p = new P(); p instanceof P && !({} instanceof P)"),
        run("true")
    );
    assert_eq!(run("function F() { return { y: 1 }; } new F().y"), "1");
    assert_eq!(
        run("({ get x() { return 7; }, set x(v) { this.y = v; } }).x"),
        "7"
    );
    assert_eq!(
        run("var o = { set x(v) { this.y = v * 2; } }; o.x = 4; o.y"),
        "8"
    );
    assert_eq!(
        run("(function () { 'use strict'; return this; })()"),
        "undefined"
    );
    assert_eq!(run("(function () { return typeof this; })()"), "\"object\"");
    assert_eq!(
        run("var f = function () {}; f.name + (() => 1).name.length"),
        "\"f0\""
    );
}

#[test]
fn objects_arrays_and_strings() {
    assert_eq!(
        run("var a = []; a.push(1, 2); a.push(3); a.length + a[2]"),
        "6"
    );
    assert_eq!(run("var a = [1, , 3]; a.length + (1 in a ? 10 : 0)"), "3");
    assert_eq!(
        run("var o = { a: 1, b: 2 }; o.c = 3; delete o.a; var s = ''; for (var k in o) s += k; s"),
        "\"bc\""
    );
    assert_eq!(
        run("var o = { x: 1 }; 'x' in o && !('y' in o)"),
        run("true")
    );
    assert_eq!(run("'abc'.length + 'abc'[1]"), "\"3b\"");
    assert_eq!(run("var o = { ['a' + 1]: 5 }; o.a1"), "5");
    assert_eq!(run("var p = { k: 1 }; var o = { __proto__: p }; o.k"), "1");
    assert_eq!(
        run("var o = { ...{ a: 1, b: 2 }, c: 3 }; o.a + o.b + o.c"),
        "6"
    );
    assert_eq!(run("String({})"), "\"[object Object]\"");
    assert_eq!(run("'' + {}"), "\"[object Object]\"");
    assert_eq!(run("var o = { valueOf() { return 41; } }; o + 1"), "42");
    assert_eq!(run("var o; with ({ z: 9 }) { o = z; } o"), "9");
    assert_eq!(run("let x = 1; { let x = 2; } x"), "1");
    assert_eq!(
        run("switch (3) { case 1: 'a'; break; case 3: 'c'; default: 'd'; }"),
        "\"d\""
    );
}

#[test]
fn exceptions_unwind_through_handlers() {
    assert_eq!(
        run(
            "var s = ''; try { s += 'a'; throw 1; } catch (e) { s += 'b' + e; } finally { s += 'c'; } s"
        ),
        "\"ab1c\""
    );
    assert_eq!(
        run("function f() { try { return 1; } finally { g = 2; } } var g; f() + g"),
        "3"
    );
    assert_eq!(
        run(
            "function f() { throw 5; } function h() { try { f(); } catch (e) { return e * 2; } } h()"
        ),
        "10"
    );
    assert_eq!(
        run(
            "var log = ''; for (var i = 0; i < 3; i++) { try { if (i == 1) continue; log += i; } finally { log += 'f'; } } log"
        ),
        "\"0ff2f\""
    );
    assert_eq!(run("try { null.x } catch (e) { typeof e }"), "\"object\"");
    assert_eq!(run("throw 'raw'"), "throw : ");
    assert_eq!(
        run("var e = null; try { undefined.x; } catch (x) { e = x; } '' + e"),
        "\"TypeError: Cannot read properties of undefined (reading 'x')\""
    );
    // A finally block that completes normally keeps the try's
    // completion value (section 14.15.3); a break in it does not.
    assert_eq!(run("1; try { 2; } finally { 3; }"), "2");
    assert_eq!(
        run("try { throw 0; } catch (e) { 'c'; } finally { 'f'; }"),
        "\"c\""
    );
    assert_eq!(
        run("do { try { 2; } finally { 3; break; } } while (false)"),
        "3"
    );
}

#[test]
fn errors_are_worded_as_chrome_words_them() {
    // docs/chrome-surface.md, the rows J2.4 makes observable.
    assert_eq!(
        run("var o = null; o.length"),
        "throw TypeError: Cannot read properties of null (reading 'length')"
    );
    assert_eq!(
        run("var o; o.length"),
        "throw TypeError: Cannot read properties of undefined (reading 'length')"
    );
    assert_eq!(
        run("var o = null; o.x = 1"),
        "throw TypeError: Cannot set properties of null (setting 'x')"
    );
    assert_eq!(
        run("var o; o.x = 1"),
        "throw TypeError: Cannot set properties of undefined (setting 'x')"
    );
    assert_eq!(
        run("var o = {}; o.f()"),
        "throw TypeError: o.f is not a function"
    );
    assert_eq!(
        run("var o = { f: 1 }; new o.f()"),
        "throw TypeError: o.f is not a constructor"
    );
    assert_eq!(
        run("const c = 1; function f() { c = 2; } f()"),
        "throw TypeError: Assignment to constant variable."
    );
    assert_eq!(
        run("function f() { const c = 1; c = 2; } f()"),
        "throw TypeError: Assignment to constant variable."
    );
    assert_eq!(
        run("function f() { x; let x; } f()"),
        "throw ReferenceError: Cannot access 'x' before initialization"
    );
    assert_eq!(run("y"), "throw ReferenceError: y is not defined");
    assert_eq!(
        run("function f() { 'use strict'; undeclared = 1; } f()"),
        "throw ReferenceError: undeclared is not defined"
    );
    assert_eq!(
        run("var g = () => {}; new g()"),
        "throw TypeError: g is not a constructor"
    );
}

#[test]
fn deep_recursion_is_a_range_error() {
    assert_eq!(
        run("function f() { return f(); } f()"),
        "throw RangeError: Maximum call stack size exceeded"
    );
    // And it is catchable.
    assert_eq!(
        run("function f() { return f(); } try { f(); } catch (e) { 'caught' }"),
        "\"caught\""
    );
}

#[test]
fn the_interrupt_hook_terminates_past_every_finally() {
    let mut agent = Agent::new(AgentOptions::default());
    let mut calls = 0;
    agent.set_interrupt(Some(Box::new(move || {
        calls += 1;
        calls > 3
    })));
    assert_eq!(
        run_in(
            &mut agent,
            "var f = 0; try { while (true) {} } finally { f = 1; }"
        ),
        "terminated"
    );
    agent.set_interrupt(None);
    // The finally did not run, and the agent runs on.
    assert_eq!(run_in(&mut agent, "f"), "0");
}

#[test]
fn scripts_share_the_global_scope() {
    let mut agent = Agent::new(AgentOptions::default());
    assert_eq!(
        run_in(
            &mut agent,
            "let a = 1; const b = 2; var c = 3; function d() { return a + b + c; }"
        ),
        "undefined"
    );
    assert_eq!(run_in(&mut agent, "d() + globalThis.c"), "9");
    assert_eq!(
        run_in(&mut agent, "b = 5"),
        "throw TypeError: Assignment to constant variable."
    );
    assert_eq!(
        run_in(&mut agent, "let a = 2"),
        "throw SyntaxError: Identifier 'a' has already been declared"
    );
    assert_eq!(run_in(&mut agent, "typeof globalThis.a"), "\"undefined\"");
}

#[test]
fn the_micro_benches_compute_what_they_should() {
    // The bench programs (silver-work's silverjs-bench), smaller.
    let alloc = "function run(n) { let sum = 0; for (let i = 0; i < n; i++) { const p = { x: i, y: i + 1 }; sum += p.x + p.y; } return sum; } run(1000)";
    assert_eq!(run(alloc), "1000000");
    let push = "function run(n) { const a = []; for (let i = 0; i < n; i++) a.push(i); let s = 0; for (let i = 0; i < a.length; i++) s += a[i]; return s; } run(1000)";
    assert_eq!(run(push), "499500");
    let closures = "function make(k) { return function (x) { return x + k; }; } function run(n) { let sum = 0; for (let i = 0; i < n; i++) { const f = make(i & 7); sum = f(sum) | 0; } return sum; } run(1000)";
    assert_eq!(run(closures), "3500");
    let strings = "function run(n) { let t = 0; for (let j = 0; j < 3; j++) { let s = ''; for (let i = 0; i < n; i++) s += 'ab'; t += s.length; } return t; } run(100)";
    assert_eq!(run(strings), "600");
    let poly = "function run(n) { const os = [{ a: 1, x: 1 }, { b: 2, x: 2 }, { c: 3, d: 4, x: 3 }, { x: 4, e: 5 }]; let s = 0; for (let i = 0; i < n; i++) s += os[i & 3].x; return s; } run(8)";
    assert_eq!(run(poly), "20");
}

#[test]
fn collections_during_a_run_keep_what_the_frames_hold() {
    let mut agent = Agent::new(AgentOptions::default());
    agent.set_stress(true);
    let src = "function make(k) { let o = { k }; return () => o.k + k; } var fs = []; for (let i = 0; i < 40; i++) fs.push(make(i)); var s = ''; try { null.x; } catch (e) { s = '' + e; } fs[39]() + fs[1]() + s.length";
    assert_eq!(run_in(&mut agent, src), "135");
}

#[test]
fn edge_cases_the_sabotage_pass_named() {
    // A handler restores the environment of its try: a throw from a
    // block nested deeper leaves its environment behind.
    assert_eq!(
        run(
            "function f() { let x = 1; const h = () => x; { let w = 5; const g = () => w; try { { let z = 3; const q = () => z; throw q() + g(); } } catch (e) { return x + h() + w + e; } } } f()"
        ),
        "15"
    );
    assert_eq!(run("-1 >>> 0"), "4294967295");
    assert_eq!(run("NaN === NaN"), "false");
    assert_eq!(run("0 === -0"), "true");
    assert_eq!(run("1 / -0"), "-Infinity");
    // A key deleted while for-in walks is not visited.
    assert_eq!(
        run(
            "var o = { a: 1, b: 2, c: 3 }; var s = ''; for (var k in o) { if (k == 'a') delete o.b; s += k; } s"
        ),
        "\"ac\""
    );
    // A read-only property: ignored in sloppy code, a TypeError in strict.
    assert_eq!(run("function f() {} f.name = 'x'; f.name"), "\"f\"");
    assert_eq!(
        run("'use strict'; function f() {} f.name = 'x';"),
        "throw TypeError: Cannot assign to read only property 'name' of object '#<Object>'"
    );
    assert_eq!(run("var a = [1, 2]; a[1] = 5; a[0] + a[1] + a.length"), "8");
    assert_eq!(run("var a = []; a.push(1, 2); a[0] * 10 + a[1]"), "12");
}

#[test]
fn for_in_under_collection() {
    let mut agent = Agent::new(AgentOptions::default());
    agent.set_stress(true);
    let src = "var o = { a: 1, b: 2 }; var p = Object; var s = ''; for (var k in o) { var t = { k }; s += t.k; } s";
    assert_eq!(run_in(&mut agent, src), "\"ab\"");
    let src =
        "var o = { a: 1, b: 2, c: 3 }; var s = ''; for (var k in o) { var t = { k }; s += t.k; } s";
    assert_eq!(run_in(&mut agent, src), "\"abc\"");
}

/// The bytes of string units copied while `src` runs.
fn copies(agent: &mut Agent, src: &str) -> (String, usize) {
    let before = agent.heap.string_copies;
    let out = run_in(agent, src);
    (out, agent.heap.string_copies - before)
}

/// String building copies each unit a constant number of times: at two
/// sizes four times apart, the bytes copied stay under the same multiple
/// of the length built (J2.4b, the lead's conditions).
#[test]
fn building_a_string_copies_linearly() {
    let shapes = [
        (
            "append",
            "var s = ''; for (var i = 0; i < N; i++) s += 'ab'; s.length",
        ),
        (
            "prepend",
            "var s = ''; for (var i = 0; i < N; i++) s = 'ab' + s; s.length",
        ),
        (
            "alternating",
            "var s = ''; for (var i = 0; i < N; i++) { if (i & 1) s += 'ab'; else s = 'ab' + s; } s.length",
        ),
        (
            "right-leaning",
            "function r(i) { return i == 0 ? '' : 'ab' + r(i - 1); } var s = ''; for (var k = 0; k < N / 4000; k++) s = r(4000) + s; s.length",
        ),
    ];
    for (name, src) in shapes {
        for n in [20_000usize, 80_000] {
            let mut agent = Agent::new(AgentOptions::default());
            let (out, copied) = copies(&mut agent, &src.replace('N', &n.to_string()));
            assert_eq!(out, (2 * n).to_string(), "{name}");
            // Each unit a byte (Latin-1): at most 6 bytes copied per unit
            // built, whatever the size.
            assert!(
                copied <= 6 * 2 * n,
                "{name} at {n}: {copied} bytes copied for {} units",
                2 * n
            );
            // A buffer counted once, by its newest window: the heap does
            // not seem to grow by the whole string at every step.
            // (gc-stress builds collect at every allocation.)
            let collections = agent.heap_stats().collections;
            if !cfg!(feature = "gc-stress") {
                assert!(
                    collections < 100,
                    "{name} at {n}: {collections} collections"
                );
            }
        }
    }
}

#[test]
fn a_prefix_extended_twice_copies_the_second_time() {
    let mut agent = Agent::new(AgentOptions::default());
    let src = "var p = ''; for (var i = 0; i < 100; i++) p += 'ab'; var a = p + 'x'; var b = p + 'y'; \
               a[a.length - 1] + b[b.length - 1] + (a.length === b.length) + (p.length) + a.slice";
    let out = run_in(&mut agent, src);
    // Neither sees the other's unit; p sees neither.
    assert!(out.starts_with("\"xytrue200"), "{out}");
    let (out, copied) = copies(&mut agent, "var c = p + 'z'; c[c.length - 1]");
    assert_eq!(out, "\"z\"");
    // p no longer ends where its buffer is used to: c is a copy.
    assert!(copied >= 200, "{copied}");
}

#[test]
fn a_short_string_does_not_pin_a_long_buffer() {
    for stress in [false, true] {
        let mut agent = Agent::new(AgentOptions::default());
        agent.set_stress(stress);
        // 10 MB without stress; 100 KB under it, which collects at every
        // allocation.
        let doublings = if stress { 11 } else { 18 };
        let src = format!(
            "var base = ''; for (var i = 0; i < 40; i++) base += 'ab'; var keep = base + 'x'; \
             var big = keep; for (var i = 0; i < {doublings}; i++) big += big; big.length"
        );
        let out = run_in(&mut agent, &src);
        let n: usize = out.parse().unwrap();
        assert!(n >= 81 << doublings, "{out}");
        // While it lives, the heap counts it.
        agent.collect();
        let held = agent.heap_stats().bytes;
        assert!(
            held >= n,
            "stress {stress}: {held} bytes counted for {n} units held"
        );
        assert_eq!(run_in(&mut agent, "big = null; 0"), "0");
        agent.collect();
        agent.collect();
        let bytes = agent.heap_stats().bytes;
        assert!(
            bytes < 1 << 20,
            "stress {stress}: {bytes} bytes after dropping {n} units"
        );
        assert_eq!(run_in(&mut agent, "keep.length + keep[80]"), "\"81x\"");
    }
}

#[test]
fn building_under_collection_keeps_every_unit() {
    let mut agent = Agent::new(AgentOptions::default());
    agent.set_stress(true);
    let src = "var s = ''; var t = ''; for (var i = 0; i < 300; i++) { s += 'ab'; t = 'c' + t; if (i % 50 == 0) t = s + t; } \
               s.length + t.length + s[599] + t[0] + t[t.length - 1]";
    let expected = {
        let mut a = Agent::new(AgentOptions::default());
        a.set_stress(false);
        run_in(&mut a, src)
    };
    assert_eq!(run_in(&mut agent, src), expected);
}
