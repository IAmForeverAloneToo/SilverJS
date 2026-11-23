//! Promises, the job queue and async functions (J3.4a): the order jobs
//! run in, the combinators, `await` suspending a frame that a job
//! resumes, async stack traces, and the surface the reference browser
//! shows (the async probe, Chrome 157).

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

/// Runs `body`, whose jobs push to `log`, then reads `log` from a second
/// script: the jobs of the first ran when it ended.
fn logged(body: &str) -> String {
    let mut agent = Agent::new(AgentOptions::default());
    run_in(&mut agent, &format!("var log = [];\n{body}"));
    run_in(&mut agent, "log.join()")
}

/// `keys(o)`: own keys in order, with kinds and attributes.
const KEYS: &str = "function keys(o) { return Reflect.ownKeys(o).map(function (k) { \
    var d = Object.getOwnPropertyDescriptor(o, k); \
    var kind = 'get' in d ? 'accessor' : typeof d.value; \
    var flags = (d.writable ? 'w' : '-') + (d.enumerable ? 'e' : '-') + (d.configurable ? 'c' : '-'); \
    var extra = ''; \
    if (typeof d.value === 'function') extra = ' len=' + d.value.length + ' name=' + d.value.name; \
    if (d.get) extra = ' get=' + d.get.name; \
    return String(k) + ' [' + kind + ' ' + flags + extra + ']'; }).join(', '); }\n";

#[test]
fn jobs_run_in_the_reference_browsers_order() {
    // Chrome 157, the async probe.
    assert_eq!(
        logged(
            "Promise.resolve().then(function () { log.push('t1'); }).then(function () { log.push('t2'); }); \
             (async function () { log.push('a0'); await null; log.push('a1'); await null; log.push('a2'); })(); \
             Promise.resolve().then(function () { log.push('t3'); }); \
             var thenable = { then: function (r) { log.push('thenable.then'); r('tv'); } }; \
             Promise.resolve(thenable).then(function (v) { log.push('thenable ' + v); }); \
             (async function () { var v = await thenable; log.push('await thenable ' + v); })(); \
             (async function () { return Promise.resolve('rp'); })().then(function (v) { log.push('async return promise ' + v); }); \
             new Promise(function (r) { r(Promise.resolve('nested')); }).then(function (v) { log.push('resolve promise ' + v); }); \
             log.push('sync end');"
        ),
        "a0,sync end,t1,a1,t3,thenable.then,thenable.then,t2,a2,thenable tv,\
         await thenable tv,async return promise rp,resolve promise nested"
    );
}

#[test]
fn promise_has_chromes_shape() {
    assert_eq!(
        run(&format!("{KEYS}keys(Promise)")),
        "length [number --c], name [string --c], prototype [object ---], \
         all [function w-c len=1 name=all], allSettled [function w-c len=1 name=allSettled], \
         any [function w-c len=1 name=any], race [function w-c len=1 name=race], \
         resolve [function w-c len=1 name=resolve], reject [function w-c len=1 name=reject], \
         withResolvers [function w-c len=0 name=withResolvers], try [function w-c len=1 name=try], \
         Symbol(Symbol.species) [accessor --c get=get [Symbol.species]]"
    );
    assert_eq!(
        run(&format!("{KEYS}keys(Promise.prototype)")),
        "constructor [function w-c len=1 name=Promise], then [function w-c len=2 name=then], \
         catch [function w-c len=1 name=catch], finally [function w-c len=1 name=finally], \
         Symbol(Symbol.toStringTag) [string --c]"
    );
    assert_eq!(
        run(&format!(
            "{KEYS}var AF = Object.getPrototypeOf(async function () {{}}).constructor; \
             var r; new Promise(function (res, rej) {{ r = [res, rej]; }}); \
             [keys(AF), keys(AF.prototype), keys(async function af(a, b) {{}}), keys(async (a) => a), \
             keys(r[0]), keys(Promise.withResolvers())].join(' / ')"
        )),
        "length [number --c], name [string --c], prototype [object ---] / \
         constructor [function --c len=1 name=AsyncFunction], Symbol(Symbol.toStringTag) [string --c] / \
         length [number --c], name [string --c] / length [number --c], name [string --c] / \
         length [number --c], name [string --c] / \
         promise [object wec], resolve [function wec len=1 name=], reject [function wec len=1 name=]"
    );
    assert_eq!(
        run(
            "var AF = Object.getPrototypeOf(async function () {}).constructor; \
             [String(async function af() {}), String(async () => 1), String({ async m() {} }.m), \
             Object.prototype.toString.call(Promise.resolve()), Object.prototype.toString.call(async function () {}), \
             AF.name, 'prototype' in async function () {}, new AF('a', 'return a').toString()].join(' | ')"
        ),
        "async function af() {} | async () => 1 | async m() {} | [object Promise] | \
         [object AsyncFunction] | AsyncFunction | false | async function anonymous(a\n) {\nreturn a\n}"
    );
}

#[test]
fn promises_throw_and_reject_with_chromes_messages() {
    assert_eq!(
        run(
            "function err(f) { try { return 'no throw ' + f(); } catch (e) { return e.constructor.name + ': ' + e.message; } } \
             [err(function () { Promise(); }), err(function () { new Promise(1); }), \
             err(function () { Promise.prototype.then.call({}, function () {}); }), \
             err(function () { Promise.resolve.call(1, 2); }), \
             err(function () { new (async function () {})(); }), err(function () { new (async () => {})(); }), \
             err(function () { eval('function f() { await 1; }'); })].join('\\n')"
        ),
        "TypeError: Promise constructor cannot be invoked without 'new'\n\
         TypeError: Promise resolver 1 is not a function\n\
         TypeError: Method Promise.prototype.then called on incompatible receiver #<Object>\n\
         TypeError: PromiseResolve called on non-object\n\
         TypeError: (intermediate value) is not a constructor\n\
         TypeError: (intermediate value) is not a constructor\n\
         SyntaxError: await is only valid in async functions and the top level bodies of modules"
    );
    assert_eq!(
        logged(
            "function settle(label, p) { p.then(function (v) { log.push(label + ': fulfilled ' + v); }, \
             function (e) { log.push(label + ': rejected ' + e.constructor.name + ': ' + e.message); }); } \
             settle('all', Promise.all(1)); \
             settle('cycle', (function () { var res; var p = new Promise(function (r) { res = r; }); res(p); return p; })()); \
             settle('any', Promise.any([])); \
             settle('throws', (async function () { throw new TypeError('boom'); })());"
        ),
        "all: rejected TypeError: number 1 is not iterable (cannot read property Symbol(Symbol.iterator)),\
         cycle: rejected TypeError: Chaining cycle detected for promise #<Promise>,\
         any: rejected AggregateError: All promises were rejected,\
         throws: rejected TypeError: boom"
    );
}

#[test]
fn the_combinators_settle_as_the_spec_says() {
    assert_eq!(
        logged(
            "Promise.all([1, Promise.resolve(2), { then: function (r) { r(3); } }]).then(function (v) { log.push('all ' + v.join('|')); }); \
             Promise.allSettled([1, Promise.reject(2)]).then(function (r) { log.push('settled ' + r.map(function (x) { return x.status + ':' + (x.value || x.reason); }).join('|')); }); \
             Promise.any([Promise.reject(1), Promise.resolve(2)]).then(function (v) { log.push('any ' + v); }); \
             Promise.any([Promise.reject(1), Promise.reject(2)]).catch(function (e) { log.push('anyerr ' + e.errors.join('|')); }); \
             Promise.race([new Promise(function () {}), Promise.resolve('r')]).then(function (v) { log.push('race ' + v); }); \
             Promise.resolve(5).finally(function () { return 9; }).then(function (v) { log.push('finally ' + v); }); \
             Promise.reject(6).finally(function () {}).catch(function (e) { log.push('finally reject ' + e); }); \
             Promise.try(function (a) { return a * 2; }, 21).then(function (v) { log.push('try ' + v); }); \
             var w = Promise.withResolvers(); w.promise.then(function (v) { log.push('with ' + v); }); w.resolve('wr'); w.resolve('again'); \
             class MyPromise extends Promise {} var mp = MyPromise.resolve(1).then(function (v) { return v; }); \
             log.push('sub ' + (mp instanceof MyPromise)); \
             Promise.all([]).then(function (v) { log.push('empty ' + v.length); });"
        ),
        "sub true,try 42,with wr,empty 0,settled fulfilled:1|rejected:2,any 2,anyerr 1|2,race r,all 1|2|3,finally 5,finally reject 6"
    );
}

#[test]
fn await_suspends_and_a_job_resumes() {
    assert_eq!(
        logged(
            "async function add(a, b = (() => { throw new Error('param'); })()) { return a + b; } \
             add(1, 2).then(function (v) { log.push('add ' + v); }); \
             add(1).catch(function (e) { log.push('param ' + e.message); }); \
             var o = { async m() { return this.k + (await 1); }, k: 'K' }; \
             o.m().then(function (v) { log.push('method ' + v); }); \
             (async (x) => (await x) * 2)(Promise.resolve(21)).then(function (v) { log.push('arrow ' + v); }); \
             (async function () { try { await Promise.reject(new Error('r')); } catch (e) { return 'caught ' + e.message; } finally { log.push('fin'); } })() \
               .then(function (v) { log.push(v); }); \
             async function deep(n) { if (n === 0) return 0; return 1 + await deep(n - 1); } \
             deep(1000).then(function (v) { log.push('deep ' + v); }); \
             class C { async m() { return super.toString === Object.prototype.toString; } } \
             new C().m().then(function (v) { log.push('super ' + v); });"
        ),
        "add 3,param param,fin,super true,method K1,arrow 42,caught r,deep 1000"
    );
}

#[test]
fn async_frames_show_on_the_stack() {
    // Chrome 157: the frames running, then `at async f` for each async
    // function awaiting the one at the bottom, at its `await`.
    assert_eq!(
        logged(
            "function top(e) { return e.stack.split('\\n').slice(0, 4).join(' | '); }\n\
             async function inner() { await null; return new Error('after await'); }\n\
             async function outer() { return await inner(); }\n\
             async function middle() { var e = await outer(); return e; }\n\
             middle().then(function (e) { log.push(top(e)); });"
        ),
        "Error: after await |     at inner (u.js:3:45) |     at async outer (u.js:4:33) |     \
         at async middle (u.js:5:35)"
    );
}

#[test]
fn promises_and_async_functions_survive_collection() {
    let mut agent = Agent::new(AgentOptions::default());
    agent.set_stress(true);
    run_in(
        &mut agent,
        "var log = []; async function f(i) { var a = await Promise.resolve(i); var b = await { then: function (r) { r(a * 2); } }; return b + 1; } \
         for (var i = 0; i < 10; i++) f(i).then(function (v) { log.push(v); }); \
         Promise.all([f(100), Promise.allSettled([f(1)])]).then(function (v) { log.push('all ' + v[0]); });",
    );
    assert_eq!(
        run_in(&mut agent, "log.join()"),
        "1,3,5,7,9,11,13,15,17,19,all 201"
    );
}

#[test]
fn a_promise_settles_once_and_passes_what_it_cannot_handle_on() {
    // Resolving with a thenable disarms reject too; a reaction with no
    // handler passes the rejection on; an element function of a
    // combinator counts its first call only.
    assert_eq!(
        logged(
            "new Promise(function (res, rej) { res({ then: function (r) { r('t'); } }); rej('x'); }) \
               .then(function (v) { log.push('once ' + v); }, function (e) { log.push('rejected ' + e); }); \
             Promise.reject(7).then(function () { log.push('wrong'); }).catch(function (e) { log.push('passed ' + e); }); \
             Promise.all([{ then: function (r) { r(1); r(2); } }, new Promise(function () {})]) \
               .then(function (v) { log.push('all early ' + v); }); \
             Promise.all([{ then: function (r) { r(1); r(2); } }]).then(function (v) { log.push('all ' + v); }); \
             function C(executor) { return new Promise(executor); } \
             C.resolve = function (v) { return { then: function (f) { f(1); f(2); } }; }; \
             Promise.all.call(C, [0, 0]).then(function (v) { log.push('elements ' + v); });"
        ),
        "elements 1,1,once t,passed 7,all 1"
    );
}

#[test]
fn async_generators_settle_their_requests_in_the_reference_browsers_order() {
    // Chrome 157, the async generator probe: a yield awaits its operand,
    // then the generator goes on with a request already waiting.
    assert_eq!(
        logged(
            "async function* counter() { log.push('start'); var x = yield 1; log.push('got ' + x); yield 2; log.push('end'); } \
             var c = counter(); \
             c.next('a').then(function (r) { log.push('r1 ' + r.value + ' ' + r.done); }); \
             c.next('b').then(function (r) { log.push('r2 ' + r.value + ' ' + r.done); }); \
             c.next('c').then(function (r) { log.push('r3 ' + r.value + ' ' + r.done); }); \
             c.next('d').then(function (r) { log.push('r4 ' + r.value + ' ' + r.done); }); \
             Promise.resolve().then(function () { log.push('tick1'); }).then(function () { log.push('tick2'); }).then(function () { log.push('tick3'); }); \
             async function* ya() { yield Promise.resolve('yp'); return Promise.resolve('rp'); } \
             var y = ya(); \
             y.next().then(function (r) { log.push('yield promise ' + r.value); }); \
             y.next().then(function (r) { log.push('return promise ' + r.value + ' ' + r.done); }); \
             async function* fin() { try { yield 1; yield 2; } finally { log.push('finally ran'); } } \
             var f = fin(); \
             f.next().then(function () { return f.return('early'); }).then(function (r) { log.push('return ' + r.value + ' ' + r.done); }); \
             fin().return('fresh').then(function (r) { log.push('fresh return ' + r.value + ' ' + r.done); }); \
             fin().throw(new Error('fresh throw')).then(null, function (e) { log.push('fresh throw ' + e.message); });"
        ),
        "start,got b,tick1,fresh throw fresh throw,r1 1 false,end,tick2,yield promise yp,\
         fresh return fresh true,r2 2 false,r3 undefined true,r4 undefined true,tick3,\
         return promise rp true,finally ran,return early true"
    );
}

#[test]
fn for_await_and_async_yield_star_iterate_and_close() {
    assert_eq!(
        logged(
            "(async function () { \
               var s = []; for await (var v of [Promise.resolve('p1'), 'v2', Promise.resolve('p3')]) s.push(v); \
               log.push('sync ' + s.join('|')); \
               var t = []; for await (const w of (async function* () { yield 'g1'; yield 'g2'; })()) t.push(w); \
               log.push('gen ' + t.join('|')); \
               var closed = 0; \
               var it = { [Symbol.asyncIterator]() { return { next() { return Promise.resolve({ value: 1, done: false }); }, \
                 return() { closed++; return Promise.resolve({ done: true }); } }; } }; \
               for await (var z of it) break; \
               try { for await (var z of it) throw new Error('x'); } catch (e) { log.push('threw ' + e.message); } \
               log.push('closed ' + closed); \
               async function* inner() { var x = yield 'i1'; yield 'i2 ' + x; return 'iret'; } \
               async function* outer() { var v = yield* inner(); yield 'outer ' + v; yield* [Promise.resolve('sync1')]; } \
               var o = outer(), out = []; \
               out.push((await o.next()).value, (await o.next('sent')).value, (await o.next()).value, (await o.next()).value); \
               log.push('yield* ' + out.join('|'));                var o2 = outer(); await o2.next(); var rr = await o2.return('R'); log.push('ret ' + rr.value + ' ' + rr.done); \
             })();"
        ),
        "sync p1|v2|p3,gen g1|g2,threw x,closed 2,yield* i1|i2 sent|outer iret|sync1,ret R true"
    );
}

#[test]
fn async_generators_have_chromes_shape_and_messages() {
    assert_eq!(
        run(&format!(
            "{KEYS}async function* ag(a) {{ yield a; }} \
             var AGF = Object.getPrototypeOf(ag).constructor; var AGP = Object.getPrototypeOf(ag.prototype); \
             var AIP = Object.getPrototypeOf(AGP); \
             [keys(AGF), keys(AGF.prototype), keys(AGP), keys(AIP), keys(ag), keys(ag.prototype), \
             String(ag), Object.prototype.toString.call(ag()), AGF.name, new AGF('a', 'yield a').toString()].join(' / ')"
        )),
        "length [number --c], name [string --c], prototype [object ---] / \
         prototype [object --c], constructor [function --c len=1 name=AsyncGeneratorFunction], \
         Symbol(Symbol.toStringTag) [string --c] / \
         constructor [object --c], next [function w-c len=1 name=next], return [function w-c len=1 name=return], \
         throw [function w-c len=1 name=throw], Symbol(Symbol.toStringTag) [string --c] / \
         Symbol(Symbol.asyncIterator) [function w-c len=0 name=[Symbol.asyncIterator]], \
         Symbol(Symbol.asyncDispose) [function w-c len=0 name=[Symbol.asyncDispose]] / \
         length [number --c], name [string --c], prototype [object w--] /  / \
         async function* ag(a) { yield a; } / [object AsyncGenerator] / AsyncGeneratorFunction / \
         async function* anonymous(a\n) {\nyield a\n}"
    );
    assert_eq!(
        logged(
            "async function* ag() {} \
             function settle(label, p) { p.then(function (v) { log.push(label + ': fulfilled ' + v); }, \
             function (e) { log.push(label + ': rejected ' + e.constructor.name + ': ' + e.message); }); } \
             try { new ag(); } catch (e) { log.push(e.message); } \
             settle('next', Object.getPrototypeOf(ag.prototype).next.call({})); \
             settle('for await', (async function () { for await (var q of 1) {} })()); \
             settle('yield*', (async function* () { yield* 1; })().next()); \
             settle('bad result', (async function () { for await (var q of { [Symbol.asyncIterator]() { return { next() { return 1; } }; } }) {} })());"
        ),
        "ag is not a constructor,\
         next: rejected TypeError: Method [AsyncGenerator].prototype.next called on incompatible receiver #<Object>,\
         for await: rejected TypeError: 1 is not async iterable,\
         yield*: rejected TypeError: undefined is not a function,\
         bad result: rejected TypeError: Iterator result 1 is not an object"
    );
}

#[test]
fn async_generator_frames_show_on_the_stack() {
    // Chrome 157: an async generator resumed by `next` shows it; one
    // awaited through `for await`, the async function awaiting it.
    assert_eq!(
        logged(
            "function top(e, n) { return e.stack.split('\\n').slice(0, n).join(' | '); }\n\
             async function* thrower() { yield 1; throw new Error('in agen'); }\n\
             var tg = thrower(); tg.next().then(function () { return tg.next(); }).then(null, function (e) { log.push(top(e, 3)); });\n\
             async function consume() { for await (var e of (async function* named() { await null; yield new Error('yielded'); })()) return e; }\n\
             consume().then(function (e) { log.push(top(e, 3)); });"
        ),
        "Error: in agen |     at thrower (u.js:3:44) |     at thrower.next (<anonymous>),\
         Error: yielded |     at named (u.js:5:93) |     at async consume (u.js:5:43)"
    );
}

#[test]
fn async_generators_survive_collection() {
    let mut agent = Agent::new(AgentOptions::default());
    agent.set_stress(true);
    run_in(
        &mut agent,
        "var log = []; async function* g(n) { for (var i = 0; i < n; i++) { yield await Promise.resolve(i * 2); } return 'end'; } \
         (async function () { var out = []; for await (var v of g(8)) out.push(v); \
           var h = g(2); out.push((await h.next()).value, (await h.return('r')).value, (await h.next()).done); \
           log.push(out.join()); })();",
    );
    assert_eq!(
        run_in(&mut agent, "log.join()"),
        "0,2,4,6,8,10,12,14,0,r,true"
    );
}

#[test]
fn a_done_async_generator_answers_in_turn_and_falls_back_to_its_prototype() {
    // A return's value awaited holds the requests after it; a function
    // whose `prototype` is no object makes generators of the realm's
    // %AsyncGeneratorPrototype%.
    assert_eq!(
        logged(
            "async function* fg() {} var g = fg(); \
             g.return({ then: function (r) { log.push('then'); r('x'); } }).then(function (r) { log.push('return ' + r.value + ' ' + r.done); }); \
             g.next().then(function (r) { log.push('next ' + r.value + ' ' + r.done); }); \
             var AGP = Object.getPrototypeOf(Object.getPrototypeOf((async function* () {})())); \
             async function* np() {} np.prototype = null; \
             log.push('fallback ' + (Object.getPrototypeOf(np()) === AGP));"
        ),
        "fallback true,then,return x true,next undefined true"
    );
}
