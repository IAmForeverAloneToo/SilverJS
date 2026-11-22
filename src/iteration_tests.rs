//! Iteration (J3.1): the iterator protocol as `for-of`, spread and
//! destructuring run it, the array, string and `arguments` iterators,
//! and the order things happen in, which Test262 alone would not pin
//! where its tests need later steps' syntax.

use crate::agent::{Agent, AgentOptions, Outcome};

/// What a script came to: its value's ToString, or `throw Name: message`.
fn run(src: &str) -> String {
    let mut agent = Agent::new(AgentOptions::default());
    run_in(&mut agent, src)
}

fn run_in(agent: &mut Agent, src: &str) -> String {
    agent.scope(|scope| match scope.run_script(src) {
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

/// A script prefix: `iter(n, tag)` gives an iterable of `0..n` whose
/// steps and closes are logged in `log`.
const ITER: &str = "var log = [];
function iter(n, tag, ret) {
  var i = 0, it = {};
  it[Symbol.iterator] = function () {
    log.push(tag + 'i');
    return {
      next: function () { log.push(tag + i); return i < n ? { value: i++, done: false } : { value: 'end', done: true }; },
      return: function () { log.push(tag + 'r'); return ret === undefined ? {} : ret; }
    };
  };
  return it;
}
";

fn logged(body: &str) -> String {
    run(&format!("{ITER}{body}; log.join()"))
}

#[test]
fn for_of_walks_arrays_strings_and_arguments() {
    assert_eq!(
        run("var out = []; for (var x of [1, 2, 3]) out.push(x); \
             for (let c of 'a' + String.fromCharCode(0xd83d, 0xde00) + 'b') out.push(c.length); \
             (function () { for (const a of arguments) out.push(a); })(7, 8); out.join()"),
        "1,2,3,1,2,1,7,8"
    );
    // An array grown while it is walked: the iterator reads its length
    // at each step.
    assert_eq!(
        run(
            "var a = [1], out = []; for (var x of a) { out.push(x); if (a.length < 3) a.push(x + 1); } out.join()"
        ),
        "1,2,3"
    );
}

#[test]
fn for_of_closes_its_iterator_when_left_early() {
    // Exhausted: not closed. Broken out of, returned from, thrown out of:
    // closed. Continued: not closed; an outer continue closes the inner.
    assert_eq!(logged("for (var x of iter(2, 'A'));"), "Ai,A0,A1,A2");
    assert_eq!(
        logged("for (var x of iter(3, 'B')) { if (x === 1) break; }"),
        "Bi,B0,B1,Br"
    );
    assert_eq!(
        logged("(function () { for (var x of iter(3, 'C')) return x; })()"),
        "Ci,C0,Cr"
    );
    assert_eq!(
        logged("try { for (var x of iter(3, 'D')) throw 1; } catch (e) {}"),
        "Di,D0,Dr"
    );
    assert_eq!(
        logged("for (var x of iter(2, 'E')) { continue; }"),
        "Ei,E0,E1,E2"
    );
    assert_eq!(
        logged(
            "outer: for (var a of iter(2, 'F')) { for (var b of iter(2, 'G')) continue outer; }"
        ),
        "Fi,F0,Gi,G0,Gr,F1,Gi,G0,Gr,F2"
    );
}

#[test]
fn a_failing_next_is_not_closed_and_a_failing_return_is_ignored_on_a_throw() {
    // next throws: the iterator is not closed.
    assert_eq!(
        run(
            "var closed = 0, it = {}; it[Symbol.iterator] = function () { return { \
             next: function () { throw 'n'; }, return: function () { closed++; return {}; } }; }; \
             try { for (var x of it); } catch (e) {} closed"
        ),
        "0"
    );
    // The body throws and return throws too: the body's exception wins.
    assert_eq!(
        run("var it = {}; it[Symbol.iterator] = function () { return { \
             next: function () { return { done: false }; }, return: function () { throw 'r'; } }; }; \
             try { for (var x of it) throw 'body'; } catch (e) { e }"),
        "body"
    );
    // A break, and return gives a primitive: a TypeError.
    assert_eq!(
        run("var it = {}; it[Symbol.iterator] = function () { return { \
             next: function () { return { done: false }; }, return: function () { return 1; } }; }; \
             try { for (var x of it) break; 'none' } catch (e) { e instanceof TypeError }"),
        "true"
    );
}

#[test]
fn what_is_not_iterable_throws() {
    assert_eq!(
        run("try { for (var x of 1); } catch (e) { e instanceof TypeError }"),
        "true"
    );
    assert_eq!(
        run("try { [...{}]; } catch (e) { e instanceof TypeError }"),
        "true"
    );
    assert_eq!(
        run(
            "var o = {}; o[Symbol.iterator] = function () { return 1; }; \
             try { for (var x of o); } catch (e) { e.message }"
        ),
        "Result of the Symbol.iterator method is not an object"
    );
    assert_eq!(
        run(
            "var o = {}; o[Symbol.iterator] = function () { return { next: function () { return 2; } }; }; \
             try { for (var x of o); } catch (e) { e.message }"
        ),
        "Iterator result 2 is not an object"
    );
}

#[test]
fn array_patterns_step_as_far_as_they_need_and_close() {
    assert_eq!(
        run("var [a, , c = 9, ...d] = [1, 2, undefined, 4, 5]; [a, c, d.join('+')].join()"),
        "1,9,4+5"
    );
    // Fewer values than targets: undefined, the iterator not closed.
    assert_eq!(
        logged("var [a, b, c] = iter(1, 'A'); log.push(String(b))"),
        "Ai,A0,A1,undefined"
    );
    // Fewer targets than values: closed.
    assert_eq!(logged("var [a] = iter(3, 'B')"), "Bi,B0,Br");
    // A rest takes the rest: not closed.
    assert_eq!(logged("var [a, ...r] = iter(2, 'C')"), "Ci,C0,C1,C2");
    // A default that throws: closed, its exception goes on.
    assert_eq!(
        logged(
            "try { var [a, b = (function () { throw 1; })()] = iter(1, 'D'); } catch (e) { log.push('e' + e); }"
        ),
        "Di,D0,D1,e1"
    );
    // A target that throws, the iterator not done: closed.
    assert_eq!(
        logged(
            "var o = { set x(v) { throw 1; } }; try { [o.x] = iter(5, 'E'); } catch (e) { log.push('e' + e); }"
        ),
        "Ei,E0,Er,e1"
    );
    // A default that is not needed is not evaluated.
    assert_eq!(
        logged("var [a, b = log.push('no')] = iter(5, 'F')"),
        "Fi,F0,F1,Fr"
    );
}

#[test]
fn an_assignment_target_is_evaluated_before_its_value() {
    // Section 13.15.5.6: the reference, then the step.
    assert_eq!(
        logged("var o = {}; [(log.push('t'), o).x] = iter(1, 'A')"),
        "Ai,t,A0,Ar"
    );
    assert_eq!(
        run("var order = [], o = {}; \
             ({ [(order.push('k'), 'a')]: (order.push('t'), o).a } = { get a() { order.push('v'); return 1; } }); \
             order.join() + ':' + o.a"),
        "k,t,v:1"
    );
}

#[test]
fn object_patterns_take_their_keys_and_leave_the_rest() {
    assert_eq!(
        run(
            "var k = 'q', { a, b: { c = 3 }, [k]: q, ...rest } = { a: 1, b: {}, q: 7, x: 8, y: 9 }; \
             [a, c, q, Object.keys(rest).join('+')].join()"
        ),
        "1,3,7,x+y"
    );
    // A default function takes the binding's name.
    assert_eq!(
        run("var { f = function () {}, g = () => 0 } = {}; f.name + g.name"),
        "fg"
    );
    // Strings destructure as objects, their indices included.
    assert_eq!(
        run(
            "var { length, 0: first, ...more } = 'ab'; [length, first, Object.keys(more).join()].join()"
        ),
        "2,a,1"
    );
    assert_eq!(
        run("try { var { a } = null; } catch (e) { e instanceof TypeError }"),
        "true"
    );
    // An empty pattern still refuses undefined.
    assert_eq!(
        run("try { var {} = undefined; 'none' } catch (e) { e instanceof TypeError }"),
        "true"
    );
}

#[test]
fn patterns_bind_parameters_catch_clauses_and_assignments() {
    assert_eq!(
        run(
            "function f(a, [b, c] = [7, 8], { d } = { d: 9 }, ...[e, ...g]) { return [a, b, c, d, e, g.length].join(); } \
             f(1) + '|' + f(1, [2, 3], { d: 4 }, 5, 6, 7)"
        ),
        "1,7,8,9,,0|1,2,3,4,5,2"
    );
    assert_eq!(
        run("try { throw { m: 'x', n: [1, 2] }; } catch ({ m, n: [, two] }) { m + two }"),
        "x2"
    );
    // An assignment pattern's value is the right side's.
    assert_eq!(
        run("var a, b, r = ([a, b] = [1, 2]); [a, b, Array.isArray(r)].join()"),
        "1,2,true"
    );
    // A `let` head in for-of: a new binding each iteration.
    assert_eq!(
        run(
            "var fs = []; for (let [i] of [[1], [2]]) fs.push(function () { return i; }); \
             fs[0]() + fs[1]()"
        ),
        "3"
    );
}

#[test]
fn spread_fills_arrays_calls_and_constructions() {
    assert_eq!(
        run("var a = [0, ...[1, 2], , ...'xy']; [a.length, a.join('|'), 3 in a].join()"),
        "6,0|1|2||x|y,false"
    );
    // Holes before the first spread count too.
    assert_eq!(
        run("var a = [, , ...[1]]; [a.length, 2 in a].join()"),
        "3,true"
    );
    assert_eq!(
        run(
            "Math.max(...[1, 5, 3]) + ',' + new Array(...[3]).length + ',' + \
             [].concat(...[[1], [2]]).join('')"
        ),
        "5,3,12"
    );
    // Through a bound function, and with a receiver.
    assert_eq!(
        run(
            "function f() { return this.k + [].join.call(arguments, ''); } \
             var g = f.bind({ k: 'b' }, 1); g(...[2, 3]) + ({ k: 'o', f: f }).f(...'xy')"
        ),
        "b123oxy"
    );
    // A spread call is a frame of the loop: it recurses as deep as plain
    // calls do, not as native re-entry allows.
    assert_eq!(
        run("function r(n) { return n === 0 ? 'deep' : r(...[n - 1]); } r(5000)"),
        "deep"
    );
    assert_eq!(
        run("try { (0, undefined)(...[]); } catch (e) { e instanceof TypeError }"),
        "true"
    );
}

#[test]
fn loops_and_conditionals_complete_with_undefined_when_their_body_gives_nothing() {
    assert_eq!(run("1; for (var x of []);"), "undefined");
    assert_eq!(run("1; for (var x of [5]) x;"), "5");
    assert_eq!(run("1; while (false);"), "undefined");
    assert_eq!(run("1; if (false) 2;"), "undefined");
    assert_eq!(run("1; {}"), "1");
    assert_eq!(run("1; var y = 2;"), "1");
}

#[test]
fn the_iterators_built_ins() {
    assert_eq!(
        run(
            "[[...['a', 'b'].keys()], [...['a', 'b'].entries()].join('/'), \
             Array.prototype[Symbol.iterator] === Array.prototype.values, \
             Object.prototype.toString.call([].values()), \
             Object.prototype.toString.call(''[Symbol.iterator]()), \
             (function () { return arguments[Symbol.iterator] === [].values; })(), \
             Object.getPrototypeOf(Object.getPrototypeOf([].values()))[Symbol.iterator].call(5)].join(' ')"
        ),
        "0,1 0,a/1,b true [object Array Iterator] [object String Iterator] true 5"
    );
    // A finished iterator stays finished, the array grown or not.
    assert_eq!(
        run("var a = [1], it = a.values(); it.next(); it.next(); a.push(2); it.next().done"),
        "true"
    );
    assert_eq!(
        run("try { [].values().next.call({}); } catch (e) { e instanceof TypeError }"),
        "true"
    );
    // Object.fromEntries closes the iterator on an entry that is not one.
    assert_eq!(
        logged(
            "var e = iter(3, 'A'); var it2 = e[Symbol.iterator](); \
                var o = {}; o[Symbol.iterator] = function () { return { next: function () { return { value: 1, done: false }; }, return: function () { log.push('closed'); return {}; } }; }; \
                try { Object.fromEntries(o); } catch (x) { log.push(x instanceof TypeError); }"
        ),
        "Ai,closed,true"
    );
    assert_eq!(
        run("var o = Object.fromEntries([['a', 1], ['b', 2]]); o.a + o.b"),
        "3"
    );
}

#[test]
fn iteration_holds_under_a_collection_at_every_allocation() {
    let mut agent = Agent::new(AgentOptions::default());
    agent.set_stress(true);
    assert_eq!(
        run_in(
            &mut agent,
            "var out = []; for (var [k, v] of Object.entries({ a: 1, b: 2 })) out.push(k + v); \
             var { x, ...r } = { x: 1, y: 2, z: 3 }; var [p, ...q] = 'abc'; \
             out.concat(Object.keys(r), q, [...'de']).join()"
        ),
        "a1,b2,y,z,b,c,d,e"
    );
}

#[test]
fn defaults_take_undefined_only_and_lexical_patterns_initialize() {
    assert_eq!(
        run("var [a = 1, b = 2] = [null]; var { c = 3 } = { c: null }; [a, b, c].join()"),
        ",2,"
    );
    assert_eq!(
        run(
            "let [x, { y }] = [1, { y: 2 }]; const { z = 3, ...w } = { q: 4 }; [x, y, z, w.q].join()"
        ),
        "1,2,3,4"
    );
    // A `const` pattern's names are constant.
    assert_eq!(
        run("const [k] = [1]; try { k = 2; } catch (e) { e instanceof TypeError }"),
        "true"
    );
}

#[test]
fn a_spread_call_of_eval_is_still_direct() {
    assert_eq!(
        run(
            "var x = 'global'; (function () { var x = 'local'; return eval(...['x']) + (0, eval)(...['x']); })()"
        ),
        "localglobal"
    );
}

#[test]
fn iteration_errors_read_as_the_reference_browsers() {
    // Chrome 157's words for each (the iterator probe of J3.1); a
    // parameter's pattern and a member's value follow the probed
    // destructuring message's form, not probed themselves.
    let cases = [
        ("for (var x of 1);", "1 is not iterable"),
        ("for (var x of undefined);", "undefined is not iterable"),
        ("for (var x of {});", "{} is not iterable"),
        ("[...1];", "1 is not iterable"),
        (
            "Math.max(...{});",
            "Spread syntax requires ...iterable[Symbol.iterator] to be a function",
        ),
        ("var [a] = undefined;", "undefined is not iterable"),
        (
            "var { a } = null;",
            "Cannot destructure property 'a' of 'null' as it is null.",
        ),
        (
            "(function ({ b }) {})();",
            "Cannot destructure property 'b' of 'undefined' as it is undefined.",
        ),
        (
            "var o = {}; var { c } = o.missing;",
            "Cannot destructure property 'c' of 'o.missing' as it is undefined.",
        ),
        (
            "[].values().next.call({});",
            "Method Array Iterator.prototype.next called on incompatible receiver #<Object>",
        ),
        (
            "''[Symbol.iterator]().next.call({});",
            "Method String Iterator.prototype.next called on incompatible receiver #<Object>",
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
