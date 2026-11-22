//! Generators (J3.2): frames suspended into their objects and resumed,
//! `next`, `return` and `throw` with what they send, `yield*` with every
//! completion forwarded, and the surface the reference browser shows
//! (the generator probe, Chrome 157).

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

/// `r(x)`: an iterator result as `value/done`.
const R: &str = "function r(x) { return String(x.value) + '/' + x.done; }\n";

fn results(body: &str) -> String {
    run(&format!("{R}{body}"))
}

#[test]
fn a_generator_runs_between_its_yields() {
    // Its parameters are bound at the call; its body waits for `next`.
    assert_eq!(
        results(
            "var log = []; function* g(a = log.push('p')) { log.push('body'); var x = yield a; log.push('x' + x); return x * 2; } \
                 var it = g(); log.push('made'); \
                 [r(it.next('ignored')), r(it.next(21)), r(it.next()), log.join()].join(' ')"
        ),
        "1/false 42/true undefined/true p,made,body,x21"
    );
    // Fresh locals per generator; a yield in an expression.
    assert_eq!(
        results(
            "function* c() { var n = 0; while (true) n += (yield n) || 1; } \
                 var a = c(), b = c(); a.next(); b.next(); \
                 [a.next().value, a.next(5).value, b.next().value].join()"
        ),
        "1,6,1"
    );
}

#[test]
fn return_and_throw_end_or_unwind_it() {
    // Before it starts: no frame runs.
    assert_eq!(
        results(
            "var ran = 0; function* g() { ran++; yield 1; } var it = g(); \
                 [r(it.return(5)), r(it.next()), ran].join(' ')"
        ),
        "5/true undefined/true 0"
    );
    assert_eq!(
        results(
            "function* g() { yield 1; } var it = g(); \
                 try { it.throw(7); } catch (e) { [e, r(it.next())].join(' ') }"
        ),
        "7 undefined/true"
    );
    // Suspended: a return runs its finally blocks, which may yield.
    assert_eq!(
        results(
            "function* g() { try { yield 1; } finally { yield 'f'; } } var it = g(); it.next(); \
                 [r(it.return(9)), r(it.next()), r(it.next())].join(' ')"
        ),
        "f/false 9/true undefined/true"
    );
    // A throw is caught where it yielded.
    assert_eq!(
        results(
            "function* g() { try { yield 1; } catch (e) { yield 'c' + e; } } var it = g(); it.next(); \
                 [r(it.throw('x')), r(it.next())].join(' ')"
        ),
        "cx/false undefined/true"
    );
    // A throw out of it completes it.
    assert_eq!(
        results(
            "function* g() { yield 1; throw 'out'; } var it = g(); it.next(); \
                 try { it.next(); } catch (e) { [e, r(it.next())].join(' ') }"
        ),
        "out undefined/true"
    );
}

#[test]
fn a_return_closes_the_loops_it_leaves() {
    assert_eq!(
        run(
            "var closed = 0, src = {}; src[Symbol.iterator] = function () { return { \
             next: function () { return { value: 1, done: false }; }, \
             return: function () { closed++; return {}; } }; }; \
             function* g() { for (var x of src) yield x; } \
             var it = g(); it.next(); it.return(); closed"
        ),
        "1"
    );
}

#[test]
fn yield_star_forwards_every_completion() {
    assert_eq!(
        results(
            "function* inner() { var got = yield 'i1'; yield 'i2:' + got; return 'ret'; } \
                 function* outer() { var v = yield* inner(); yield 'o:' + v; } \
                 var o = outer(); [r(o.next()), r(o.next('s')), r(o.next()), r(o.next())].join(' ')"
        ),
        "i1/false i2:s/false o:ret/false undefined/true"
    );
    // Over any iterable; a throw goes in, a return goes in and out.
    assert_eq!(
        run("[...(function* () { yield* [1, 2]; yield* 'ab'; })()].join()"),
        "1,2,a,b"
    );
    assert_eq!(
        results(
            "function* inner() { try { yield 1; } catch (e) { yield 'in:' + e; } } \
                 function* outer() { yield* inner(); } \
                 var o = outer(); o.next(); r(o.throw('t'))"
        ),
        "in:t/false"
    );
    assert_eq!(
        results(
            "var log = []; function* inner() { try { yield 1; } finally { log.push('inner'); } } \
                 function* outer() { try { yield* inner(); } finally { log.push('outer'); } } \
                 var o = outer(); o.next(); [r(o.return(4)), log.join()].join(' ')"
        ),
        "4/true inner,outer"
    );
    // A return sent to an inner iterator with no return of its own: the
    // delegation returns the value sent.
    assert_eq!(
        results(
            "var src = {}; src[Symbol.iterator] = function () { return { \
                 next: function () { return { value: 1, done: false }; } }; }; \
                 function* g() { yield* src; yield 'not reached'; } var it = g(); it.next(); \
                 [r(it.return(5)), r(it.next())].join(' ')"
        ),
        "5/true undefined/true"
    );
    // An inner iterator with no throw: closed, then a TypeError.
    assert_eq!(
        run(
            "var closed = 0, src = {}; src[Symbol.iterator] = function () { return { \
             next: function () { return { value: 1, done: false }; }, \
             return: function () { closed++; return {}; } }; }; \
             function* g() { yield* src; } var it = g(); it.next(); \
             try { it.throw(new Error('t')); } catch (e) { closed + ':' + e.message }"
        ),
        "1:The iterator does not provide a 'throw' method."
    );
}

#[test]
fn generators_iterate_and_are_made_as_the_spec_says() {
    assert_eq!(
        run(
            "function* fib() { var a = 0, b = 1; for (;;) { yield a; var t = a + b; a = b; b = t; } } \
             var ten = []; for (var n of fib()) { if (ten.length === 10) break; ten.push(n); } ten.join()"
        ),
        "0,1,1,2,3,5,8,13,21,34"
    );
    assert_eq!(
        run("var o = { k: 'K', *m() { yield this.k; } }; o.m().next().value"),
        "K"
    );
    assert_eq!(
        run(
            "function* g() {} var GP = Object.getPrototypeOf(g.prototype); \
             [Object.getPrototypeOf(g()) === g.prototype, \
              Object.getPrototypeOf(GP) === Object.getPrototypeOf(Object.getPrototypeOf([].values())), \
              Object.prototype.toString.call(g()), Object.prototype.toString.call(g), \
              Object.getPrototypeOf(g) === GP.constructor, g.prototype.hasOwnProperty('constructor')].join()"
        ),
        "true,true,[object Generator],[object GeneratorFunction],true,false"
    );
    // A prototype that is not an object: the realm's %GeneratorPrototype%.
    assert_eq!(
        run("function* g() {} g.prototype = 1; \
             Object.getPrototypeOf(g()) === Object.getPrototypeOf(function* () {}.prototype)"),
        "true"
    );
    let gf = "var GF = Object.getPrototypeOf(function* () {}).constructor;";
    assert_eq!(
        run(&format!(
            "{gf} var h = GF('a', 'yield a; yield a + 1'); [String(h), [...h(3)].join()].join(' ')"
        )),
        "function* anonymous(a\n) {\nyield a; yield a + 1\n} 3,4"
    );
    assert_eq!(
        run(&format!(
            "{gf} [Object.getPrototypeOf(GF) === Function, GF.name, GF.length].join()"
        )),
        "true,GeneratorFunction,1"
    );
}

#[test]
fn generator_errors_read_as_the_reference_browsers() {
    let cases = [
        ("function* g() {} new g();", "g is not a constructor"),
        (
            "var self; function* r() { self.next(); } self = r(); self.next();",
            "Generator is already running",
        ),
        (
            "function* g() {} Object.getPrototypeOf(g.prototype).next.call({});",
            "Method [Generator].prototype.next called on incompatible receiver #<Object>",
        ),
        (
            "function* g() {} Object.getPrototypeOf(g.prototype).throw.call({});",
            "Method [Generator].prototype.throw called on incompatible receiver #<Object>",
        ),
        (
            "function* o() { yield* 1; } o().next();",
            "number 1 is not iterable (cannot read property Symbol(Symbol.iterator))",
        ),
        (
            "var inner = {}; inner[Symbol.iterator] = function () { return { next: function () { return 3; } }; }; \
             function* o() { yield* inner; } o().next();",
            "Iterator result 3 is not an object",
        ),
    ];
    for (src, message) in cases {
        assert_eq!(
            run(&format!(
                "try {{ {src} 'none' }} catch (e) {{ e.constructor.name + ': ' + e.message }}"
            )),
            format!("TypeError: {message}"),
            "{src}"
        );
    }
}

#[test]
fn a_generators_frames_show_on_the_stack() {
    // Chrome 157: the generator's frame, then its `next`, named after it.
    assert_eq!(
        run(
            "function* thrower() { yield 1; throw new Error('in gen'); }\n\
             function* deleg() { yield* thrower(); }\n\
             var d = deleg(); d.next();\n\
             try { d.next(); } catch (e) { e.stack }"
        ),
        "Error: in gen\n    at thrower (u.js:1:38)\n    at thrower.next (<anonymous>)\n    \
         at deleg (u.js:2:28)\n    at deleg.next (<anonymous>)\n    at u.js:4:9"
    );
}

#[test]
fn generators_survive_collection_and_deep_resumes() {
    let mut agent = Agent::new(AgentOptions::default());
    agent.set_stress(true);
    assert_eq!(
        run_in(
            &mut agent,
            "function* g(s) { var parts = []; for (var c of s) parts.push(yield c + '!'); return parts.join(''); } \
             var it = g('abc'), out = [], x = it.next(); \
             while (!x.done) { out.push(x.value); x = it.next(x.value.toUpperCase()); } \
             out.join() + ':' + x.value"
        ),
        "a!,b!,c!:A!B!C!"
    );
    // A chain of delegating generators, deeper than natives may re-enter:
    // each resume is a frame of the loop, not a nested run.
    assert_eq!(
        run(
            "function* d(n) { if (n === 0) { yield 'bottom'; return; } yield* d(n - 1); } \
             d(1000).next().value"
        ),
        "bottom"
    );
}
