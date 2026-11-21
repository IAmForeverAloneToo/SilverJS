//! `Error.prototype.stack` and its kin as the reference browser shows
//! them (docs/chrome-surface.md; Chrome 157's answers to SilverJS's
//! stack probe), and direct `eval`.

use crate::agent::{Agent, AgentOptions, Outcome};

/// What a script named `u.js` came to: its value's ToString, or
/// `throw ...`.
fn run(src: &str) -> String {
    let mut agent = Agent::new(AgentOptions::default());
    agent.scope(|scope| match scope.run_script_named(src, "u.js") {
        Ok(Outcome::Value(v)) => scope
            .to_text(v)
            .unwrap_or_else(|_| "(ToString threw)".into()),
        Ok(Outcome::Threw(e)) => format!("throw {}", scope.to_text(e).unwrap_or_default()),
        Ok(Outcome::Terminated) => "terminated".into(),
        Err(e) => format!("compile error: {e}"),
    })
}

/// The stack of the error the expression on the script's last line
/// evaluates to; `lines` come before it, numbered from 1.
fn stack(lines: &[&str]) -> String {
    run(&lines.join("\n"))
}

#[test]
fn frames_name_functions_methods_and_constructions() {
    // Each column is where the frame stopped: a call's callee name (the
    // property's, for a method), a construction's `new`.
    assert_eq!(
        stack(&["new Error('top').stack"]),
        "Error: top\n    at u.js:1:1"
    );
    // Columns count UTF-16 units, as the reference browser's do.
    assert_eq!(
        stack(&["var s = '\u{e9}\u{1f600}'; new Error('w').stack"]),
        "Error: w\n    at u.js:1:16"
    );
    assert_eq!(
        stack(&[
            "function named() { return new Error('named'); }",
            "named().stack",
        ]),
        "Error: named\n    at named (u.js:1:27)\n    at u.js:2:1"
    );
    assert_eq!(
        stack(&[
            "var obj = { m: function () { return new Error('m'); }, s() { return new Error('s'); } };",
            "obj.m().stack + '|' + obj.s().stack",
        ]),
        "Error: m\n    at Object.m (u.js:1:37)\n    at u.js:2:5|Error: s\n    at Object.s (u.js:1:69)\n    at u.js:2:27"
    );
    assert_eq!(
        stack(&[
            "function Ctor() { this.e = new Error('ctor'); }",
            "Ctor.prototype.pm = function () { return new Error('pm'); };",
            "Ctor.st = function () { return new Error('st'); };",
            "[new Ctor().e.stack, new Ctor().pm().stack, Ctor.st().stack].join('|')",
        ]),
        "Error: ctor\n    at new Ctor (u.js:1:28)\n    at u.js:4:2|Error: pm\n    at Ctor.pm (u.js:2:42)\n    at u.js:4:33|Error: st\n    at Ctor.st (u.js:3:32)\n    at u.js:4:50"
    );
    // An anonymous function shows its location alone; an arrow, its
    // inferred name; a getter, `get g` without a type.
    assert_eq!(
        stack(&[
            "var a = () => new Error('arrow');",
            "var o = { get g() { return new Error('getter'); } };",
            "[(function () { return new Error('iife'); })().stack, a().stack, o.g.stack].join('|')",
        ]),
        "Error: iife\n    at u.js:3:24\n    at u.js:3:45|Error: arrow\n    at a (u.js:1:15)\n    at u.js:3:55|Error: getter\n    at get g (u.js:2:28)\n    at u.js:3:68"
    );
}

#[test]
fn natives_show_as_anonymous_and_call_and_apply_not_at_all() {
    assert_eq!(
        stack(&[
            "var s; [1].map(function cb() { s = new Error('map').stack; });",
            "var t; Array.prototype.forEach.call([1], function cb() { t = new Error('fe').stack; });",
            "s + '|' + t",
        ]),
        "Error: map\n    at cb (u.js:1:36)\n    at Array.map (<anonymous>)\n    at u.js:1:12|Error: fe\n    at cb (u.js:2:62)\n    at Array.forEach (<anonymous>)\n    at u.js:2:32"
    );
    // The engine's own errors carry a stack too.
    assert_eq!(
        stack(&["try { null.x; } catch (e) { e.stack }"]),
        "TypeError: Cannot read properties of null (reading 'x')\n    at u.js:1:12"
    );
}

#[test]
fn code_from_strings_shows_where_it_was_made() {
    assert_eq!(
        stack(&[
            "function ev() { return eval(\"new Error('ev')\"); }",
            "ev().stack",
        ]),
        "Error: ev\n    at eval (eval at ev (u.js:1:24), <anonymous>:1:1)\n    at ev (u.js:1:24)\n    at u.js:2:1"
    );
    assert_eq!(
        stack(&[
            "function ev2() { return eval('1;\\n2;\\n  new Error(\"x\")'); }",
            "ev2().stack"
        ]),
        "Error: x\n    at eval (eval at ev2 (u.js:1:25), <anonymous>:3:3)\n    at ev2 (u.js:1:25)\n    at u.js:2:1"
    );
    assert_eq!(
        stack(&["(0, eval)(\"new Error('ind')\").stack"]),
        "Error: ind\n    at eval (eval at <anonymous> (u.js:1:10), <anonymous>:1:1)\n    at eval (<anonymous>)\n    at u.js:1:10"
    );
    assert_eq!(
        stack(&["new Function('a', \"return new Error('nf')\")().stack"]),
        "Error: nf\n    at eval (eval at <anonymous> (u.js:1:1), <anonymous>:3:8)\n    at u.js:1:44"
    );
}

#[test]
fn the_limit_the_heading_and_the_accessor_are_the_reference_browsers() {
    let named = "function named() { return new Error('n'); }";
    assert_eq!(
        stack(&["Error.stackTraceLimit = 0; new Error('l').stack"]),
        "Error: l"
    );
    assert_eq!(
        stack(&[named, "Error.stackTraceLimit = 1; named().stack"]),
        "Error: n\n    at named (u.js:1:27)"
    );
    // Not a number: no stack, though the accessor is there.
    assert_eq!(
        run(
            "Error.stackTraceLimit = 'x'; var e = new Error('ls'); typeof e.stack + '|' + Object.prototype.hasOwnProperty.call(e, 'stack')"
        ),
        "undefined|true"
    );
    assert_eq!(
        run("delete Error.stackTraceLimit; typeof new Error('d').stack"),
        "undefined"
    );
    // SilverJS's own choice (not probed): the limit is read without
    // calling anything, so an accessor is not a number either.
    assert_eq!(
        run(
            "Object.defineProperty(Error, 'stackTraceLimit', { get: function () { return 5; } }); typeof new Error('a').stack"
        ),
        "undefined"
    );
    // The name is read when the stack is first read, the message as it
    // was made.
    assert_eq!(
        run(
            "var e = new Error('m'); e.name = 'Changed'; e.message = 'changed'; e.stack.split('\\n')[0]"
        ),
        "Changed: m"
    );
    assert_eq!(run("new Error().stack.split('\\n')[0]"), "Error");
    assert_eq!(
        run(
            "Reflect.ownKeys(new Error('x', { cause: 1 })).join() + '|' + Reflect.ownKeys(new Error()).join()"
        ),
        "stack,message,cause|stack"
    );
    assert_eq!(
        run("try { null.x; } catch (e) { Reflect.ownKeys(e).join() }"),
        "stack,message"
    );
    // One getter and setter, nameless, shared by every error; setting
    // replaces the text and keeps the accessor.
    assert_eq!(
        run(
            "var d = Object.getOwnPropertyDescriptor(new Error('a'), 'stack'); var d2 = Object.getOwnPropertyDescriptor(new Error('b'), 'stack'); [d.get.name, d.set.name, d.get === d2.get, d.get.length, d.set.length, String(d.get), d.enumerable, d.configurable].join('|')"
        ),
        "||true|0|1|function () { [native code] }|false|true"
    );
    assert_eq!(
        run(
            "var e = new Error('s'); e.stack = 'replaced'; e.stack + '|' + typeof Object.getOwnPropertyDescriptor(e, 'stack').get"
        ),
        "replaced|function"
    );
    assert_eq!(
        run(
            "var d = Object.getOwnPropertyDescriptor(new Error('a'), 'stack'); String(d.get.call({})) + '|' + d.get.call(new Error('other')).split('\\n')[0]"
        ),
        "undefined|Error: other"
    );
    // Deep recursion: the heading and ten frames.
    assert_eq!(
        run(
            "function r() { r(); } try { r(); } catch (e) { e.stack.split('\\n').length + '|' + e.constructor.name }"
        ),
        "11|RangeError"
    );
}

#[test]
fn capture_stack_trace_gives_any_object_a_stack() {
    assert_eq!(
        stack(&["var o = {}; Error.captureStackTrace(o); o.stack"]),
        "Error\n    at u.js:1:19"
    );
    assert_eq!(
        stack(&["var o = { message: 'om', name: 'ON' }; Error.captureStackTrace(o); o.stack"]),
        "ON: om\n    at u.js:1:46"
    );
    assert_eq!(
        stack(&[
            "function outer() { return inner(); }",
            "function inner() { var o = {}; Error.captureStackTrace(o, inner); return o.stack; }",
            "outer()",
        ]),
        "Error\n    at outer (u.js:1:27)\n    at u.js:3:1"
    );
    assert_eq!(
        run(
            "var o = {}; var r = Error.captureStackTrace(o); var d = Object.getOwnPropertyDescriptor(o, 'stack'); [r, typeof d.get, d.enumerable, d.configurable, Object.keys(o).length, Reflect.ownKeys(o).join()].join('|')"
        ),
        "|function|false|true|0|stack"
    );
}

#[test]
fn prepare_stack_trace_is_called_at_the_first_read_with_call_sites() {
    assert_eq!(
        run(
            "var calls = 0; Error.prepareStackTrace = function (e, cs) { calls++; return 'P' + cs.length; }; var e = new Error('p'); var before = calls; var s = e.stack; var s2 = e.stack; [before, calls, s, s2].join('|')"
        ),
        "0|1|P1|P1"
    );
    assert_eq!(
        run(
            "var t; Error.prepareStackTrace = function () { t = this === Error; return 1; }; new Error().stack; t"
        ),
        "true"
    );
    assert_eq!(
        run("Error.prepareStackTrace = function () { return { k: 1 }; }; typeof new Error().stack"),
        "object"
    );
    assert_eq!(
        run(
            "Error.prepareStackTrace = function () { throw new Error('inside'); }; try { new Error().stack; } catch (e) { e.message }"
        ),
        "inside"
    );
    // CallSite: its methods, in the reference browser's order, and what
    // they answer.
    assert_eq!(
        run(
            "var out; Error.prepareStackTrace = function (e, cs) { out = Object.getOwnPropertyNames(Object.getPrototypeOf(cs[0])).join(); return ''; }; new Error().stack; out"
        ),
        "constructor,getColumnNumber,getEnclosingColumnNumber,getEnclosingLineNumber,getEvalOrigin,getFileName,getFunction,getFunctionName,getLineNumber,getMethodName,getPosition,getPromiseIndex,getScriptNameOrSourceURL,getScriptHash,getThis,getTypeName,isAsync,isConstructor,isEval,isNative,isPromiseAll,isToplevel,toString"
    );
    assert_eq!(
        stack(&[
            "function Ctor() { this.e = new Error('ctor'); }",
            "function rec(f) { return f(); }",
            "var out; Error.prepareStackTrace = function (e, cs) { out = cs.map(function (c) { return [c.getFunctionName(), c.getMethodName(), c.getTypeName(), c.getLineNumber(), c.getColumnNumber(), c.isToplevel(), c.isEval(), c.isNative(), c.isConstructor(), c.getFileName(), String(c)].join(';'); }).join(' // '); return ''; };",
            "rec(function () { return new Ctor().e.stack; }); out",
        ]),
        "Ctor;;;1;28;false;false;false;true;u.js;new Ctor (u.js:1:28) // ;;;4;26;true;false;false;false;u.js;u.js:4:26 // rec;rec;;2;26;true;false;false;false;u.js;rec (u.js:2:26) // ;;;4;1;true;false;false;false;u.js;u.js:4:1"
    );
}

#[test]
fn direct_eval_runs_in_its_callers_scope() {
    assert_eq!(
        run(
            "function f(a) { var x = 1; eval('var y = x + a; x = 10'); return [x, y, typeof z].join(); } f(5)"
        ),
        "10,6,undefined"
    );
    assert_eq!(
        run("function g() { eval('function inner() { return 7 }'); return inner(); } g()"),
        "7"
    );
    assert_eq!(
        run("var o = { m: function () { return eval('this === o'); } }; o.m()"),
        "true"
    );
    // A strict eval keeps its `var`s; a sloppy one's are deletable.
    assert_eq!(
        run("function h() { 'use strict'; eval('var s = 1'); return typeof s; } h()"),
        "undefined"
    );
    assert_eq!(
        run("eval('var gv = 3'); [gv, delete gv, typeof gv].join()"),
        "3,true,undefined"
    );
    assert_eq!(
        run(
            "function k() { let q = 1; try { eval('var q = 2'); } catch (e) { return e.name + ': ' + e.message; } } k()"
        ),
        "SyntaxError: Identifier 'q' has already been declared"
    );
    // An arrow's `this` and `new.target` are its function's.
    assert_eq!(
        run(
            "var o = {}; var ar = function () { return (() => eval('this'))(); }; ar.call(o) === o"
        ),
        "true"
    );
    assert_eq!(
        run("function foo() { return eval('() => this'); } foo.call(7)() == 7"),
        "true"
    );
    assert_eq!(
        run(
            "function nt() { return eval('new.target'); } [new nt() !== undefined, typeof nt()].join()"
        ),
        "true,undefined"
    );
    assert_eq!(
        run("var f = () => eval('new.target'); try { f(); } catch (e) { e.name }"),
        "SyntaxError"
    );
    // Indirect, and optional, calls are not direct.
    assert_eq!(
        run(
            "var lv = 'global'; (function () { var lv = 'local'; var e = eval; return [e('lv'), eval('lv'), eval?.('lv')].join(); })()"
        ),
        "global,local,global"
    );
    assert_eq!(
        run("function af(a) { return eval('arguments.length + a'); } af(1, 2)"),
        "3"
    );
    assert_eq!(run("[eval('1;2;'), eval(), eval(42)].join()"), "2,,42");
    // A sloppy eval's `var` is its function's, not the global object's;
    // one in an eval inside an eval goes to the same function.
    assert_eq!(
        run("function f() { eval('var y = 1'); return y; } f() + typeof y"),
        "1undefined"
    );
    assert_eq!(
        run("function f() { eval(\"let l = 1; eval('var w = 2')\"); return typeof w; } f()"),
        "number"
    );
    // Assigning by name before the binding is initialized throws.
    assert_eq!(
        run("(function () { try { eval('l = 1'); } catch (e) { return e.name; } let l; })()"),
        "ReferenceError"
    );
    // A named function expression's name is not assignable by name.
    assert_eq!(
        run("var r = function B() { eval('B = 1'); return typeof B; }; r()"),
        "function"
    );
    assert_eq!(
        run(
            "(function () { const c = 1; try { eval('c = 2'); } catch (e) { return e.message; } })()"
        ),
        "Assignment to constant variable."
    );
}
