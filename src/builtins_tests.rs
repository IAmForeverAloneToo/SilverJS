//! The core built-ins (J2.5): their own keys in the reference browser's
//! order (docs/chrome-surface.md), their behaviour where Test262 alone
//! would not catch a slip, and the embedder's natives.

use crate::agent::{Agent, AgentOptions, Outcome, Scope};
use crate::embed::{ErrorKind, Thrown};

/// What a script came to, as text: its value's ToString, or
/// `throw Name: message`.
fn text_in(agent: &mut Agent, src: &str) -> String {
    agent.scope(|scope| match scope.run_script(src) {
        Ok(Outcome::Value(v)) => scope
            .to_text(v)
            .unwrap_or_else(|_| "(ToString threw)".into()),
        Ok(Outcome::Threw(e)) => thrown(scope, e),
        Ok(Outcome::Terminated) => "terminated".into(),
        Err(e) => format!("compile error: {e}"),
    })
}

fn thrown<'s>(scope: &mut Scope<'s>, e: crate::agent::Local<'s>) -> String {
    let key = scope.key("name");
    let name = scope
        .get_property(e, key)
        .ok()
        .and_then(|n| scope.to_text(n).ok())
        .unwrap_or_default();
    let key = scope.key("message");
    let message = scope
        .get_property(e, key)
        .ok()
        .and_then(|m| scope.to_text(m).ok())
        .unwrap_or_default();
    format!("throw {name}: {message}")
}

fn text(src: &str) -> String {
    let mut agent = Agent::new(AgentOptions::default());
    text_in(&mut agent, src)
}

/// Chrome 157's own keys of `expr` (the web-engine session's probe),
/// less `missing`, the names a later step brings: SilverJS's must be
/// those, in that order.
fn keys_as_chrome(expr: &str, chrome: &[&str], missing: &[&str]) {
    let want: Vec<&str> = chrome
        .iter()
        .copied()
        .filter(|k| !missing.contains(k))
        .collect();
    let got = text(&format!(
        "Reflect.ownKeys({expr}).map(function (k) {{ return String(k); }}).join()"
    ));
    assert_eq!(got, want.join(","), "own keys of {expr}");
}

#[test]
fn own_keys_are_in_the_reference_browsers_order() {
    keys_as_chrome(
        "Object",
        &[
            "length",
            "name",
            "prototype",
            "assign",
            "getOwnPropertyDescriptor",
            "getOwnPropertyDescriptors",
            "getOwnPropertyNames",
            "getOwnPropertySymbols",
            "hasOwn",
            "is",
            "preventExtensions",
            "seal",
            "create",
            "defineProperties",
            "defineProperty",
            "freeze",
            "getPrototypeOf",
            "setPrototypeOf",
            "isExtensible",
            "isFrozen",
            "isSealed",
            "keys",
            "entries",
            "fromEntries",
            "values",
            "groupBy",
        ],
        &[],
    );
    keys_as_chrome(
        "Object.prototype",
        &[
            "constructor",
            "__defineGetter__",
            "__defineSetter__",
            "hasOwnProperty",
            "__lookupGetter__",
            "__lookupSetter__",
            "isPrototypeOf",
            "propertyIsEnumerable",
            "toString",
            "valueOf",
            "__proto__",
            "toLocaleString",
        ],
        &[],
    );
    keys_as_chrome("Function", &["length", "name", "prototype"], &[]);
    keys_as_chrome(
        "Function.prototype",
        &[
            "length",
            "name",
            "constructor",
            "apply",
            "bind",
            "call",
            "toString",
            "arguments",
            "caller",
            "Symbol(Symbol.hasInstance)",
        ],
        &[],
    );
    keys_as_chrome(
        "Error",
        &[
            "length",
            "name",
            "prototype",
            "captureStackTrace",
            "isError",
            "stackTraceLimit",
        ],
        &[],
    );
    keys_as_chrome(
        "Error.prototype",
        &["constructor", "name", "message", "toString"],
        &[],
    );
    keys_as_chrome("TypeError", &["length", "name", "prototype"], &[]);
    keys_as_chrome(
        "TypeError.prototype",
        &["constructor", "name", "message"],
        &[],
    );
    keys_as_chrome(
        "Array",
        &[
            "length",
            "name",
            "prototype",
            "isArray",
            "from",
            "fromAsync",
            "of",
            "Symbol(Symbol.species)",
        ],
        &[],
    );
    keys_as_chrome(
        "Array.prototype",
        &[
            "length",
            "constructor",
            "at",
            "concat",
            "copyWithin",
            "fill",
            "find",
            "findIndex",
            "findLast",
            "findLastIndex",
            "lastIndexOf",
            "pop",
            "push",
            "reverse",
            "shift",
            "unshift",
            "slice",
            "sort",
            "splice",
            "includes",
            "indexOf",
            "join",
            "keys",
            "entries",
            "values",
            "forEach",
            "filter",
            "flat",
            "flatMap",
            "map",
            "every",
            "some",
            "reduce",
            "reduceRight",
            "toReversed",
            "toSorted",
            "toSpliced",
            "with",
            "toLocaleString",
            "toString",
            "Symbol(Symbol.iterator)",
            "Symbol(Symbol.unscopables)",
        ],
        &[],
    );
    keys_as_chrome(
        "String",
        &[
            "length",
            "name",
            "prototype",
            "fromCharCode",
            "fromCodePoint",
            "raw",
        ],
        &[],
    );
    keys_as_chrome(
        "String.prototype",
        &[
            "length",
            "constructor",
            "anchor",
            "at",
            "big",
            "blink",
            "bold",
            "charAt",
            "charCodeAt",
            "codePointAt",
            "concat",
            "endsWith",
            "fontcolor",
            "fontsize",
            "fixed",
            "includes",
            "indexOf",
            "isWellFormed",
            "italics",
            "lastIndexOf",
            "link",
            "localeCompare",
            "match",
            "matchAll",
            "normalize",
            "padEnd",
            "padStart",
            "repeat",
            "replace",
            "replaceAll",
            "search",
            "slice",
            "small",
            "split",
            "strike",
            "sub",
            "substr",
            "substring",
            "sup",
            "startsWith",
            "toString",
            "toWellFormed",
            "trim",
            "trimStart",
            "trimLeft",
            "trimEnd",
            "trimRight",
            "toLocaleLowerCase",
            "toLocaleUpperCase",
            "toLowerCase",
            "toUpperCase",
            "valueOf",
            "Symbol(Symbol.iterator)",
        ],
        &["match", "matchAll", "normalize", "search"],
    );
    keys_as_chrome(
        "Number",
        &[
            "length",
            "name",
            "prototype",
            "isFinite",
            "isInteger",
            "isNaN",
            "isSafeInteger",
            "parseFloat",
            "parseInt",
            "MAX_VALUE",
            "MIN_VALUE",
            "NaN",
            "NEGATIVE_INFINITY",
            "POSITIVE_INFINITY",
            "MAX_SAFE_INTEGER",
            "MIN_SAFE_INTEGER",
            "EPSILON",
        ],
        &[],
    );
    keys_as_chrome(
        "Number.prototype",
        &[
            "constructor",
            "toExponential",
            "toFixed",
            "toPrecision",
            "toString",
            "valueOf",
            "toLocaleString",
        ],
        &[],
    );
    keys_as_chrome("Boolean", &["length", "name", "prototype"], &[]);
    keys_as_chrome(
        "Boolean.prototype",
        &["constructor", "toString", "valueOf"],
        &[],
    );
    keys_as_chrome(
        "Symbol",
        &[
            "length",
            "name",
            "prototype",
            "for",
            "keyFor",
            "asyncIterator",
            "hasInstance",
            "isConcatSpreadable",
            "iterator",
            "match",
            "matchAll",
            "replace",
            "search",
            "species",
            "split",
            "toPrimitive",
            "toStringTag",
            "unscopables",
            "dispose",
            "asyncDispose",
        ],
        &[],
    );
    keys_as_chrome(
        "Symbol.prototype",
        &[
            "constructor",
            "toString",
            "valueOf",
            "description",
            "Symbol(Symbol.toStringTag)",
            "Symbol(Symbol.toPrimitive)",
        ],
        &[],
    );
    keys_as_chrome(
        "Math",
        &[
            "abs",
            "acos",
            "acosh",
            "asin",
            "asinh",
            "atan",
            "atanh",
            "atan2",
            "ceil",
            "cbrt",
            "expm1",
            "clz32",
            "cos",
            "cosh",
            "exp",
            "floor",
            "f16round",
            "fround",
            "hypot",
            "imul",
            "log",
            "log1p",
            "log2",
            "log10",
            "max",
            "min",
            "pow",
            "random",
            "round",
            "sign",
            "sin",
            "sinh",
            "sqrt",
            "sumPrecise",
            "tan",
            "tanh",
            "trunc",
            "E",
            "LN10",
            "LN2",
            "LOG10E",
            "LOG2E",
            "PI",
            "SQRT1_2",
            "SQRT2",
            "Symbol(Symbol.toStringTag)",
        ],
        &[],
    );
    keys_as_chrome(
        "Reflect",
        &[
            "defineProperty",
            "deleteProperty",
            "apply",
            "construct",
            "get",
            "getOwnPropertyDescriptor",
            "getPrototypeOf",
            "has",
            "isExtensible",
            "ownKeys",
            "preventExtensions",
            "set",
            "setPrototypeOf",
            "Symbol(Symbol.toStringTag)",
        ],
        &[],
    );
    keys_as_chrome(
        "Array.prototype[Symbol.unscopables]",
        &[
            "at",
            "copyWithin",
            "entries",
            "fill",
            "find",
            "findIndex",
            "findLast",
            "findLastIndex",
            "flat",
            "flatMap",
            "includes",
            "keys",
            "toReversed",
            "toSorted",
            "toSpliced",
            "values",
        ],
        &[],
    );
    keys_as_chrome("(function f() {})", &["length", "name", "prototype"], &[]);
    keys_as_chrome("(() => {})", &["length", "name"], &[]);
    keys_as_chrome("[]", &["length"], &[]);
    keys_as_chrome("new String('ab')", &["0", "1", "length"], &[]);
    keys_as_chrome("(function f() {}).bind(null)", &["length", "name"], &[]);
    keys_as_chrome(
        "(function () { return arguments; })(1)",
        &["0", "length", "callee", "Symbol(Symbol.iterator)"],
        &[],
    );
}

#[test]
fn the_global_objects_built_ins_are_in_the_reference_browsers_order() {
    // Chrome 157's global object, its ECMAScript names (its web APIs
    // follow), less those later steps bring.
    let chrome = [
        "Object",
        "Function",
        "Array",
        "Number",
        "parseFloat",
        "parseInt",
        "Infinity",
        "NaN",
        "undefined",
        "Boolean",
        "String",
        "Symbol",
        "Date",
        "Promise",
        "RegExp",
        "Error",
        "AggregateError",
        "EvalError",
        "RangeError",
        "ReferenceError",
        "SyntaxError",
        "TypeError",
        "URIError",
        "SuppressedError",
        "globalThis",
        "JSON",
        "Math",
        "Intl",
        "ArrayBuffer",
        "Atomics",
        "Uint8Array",
        "Int8Array",
        "Uint16Array",
        "Int16Array",
        "Uint32Array",
        "Int32Array",
        "BigUint64Array",
        "BigInt64Array",
        "Uint8ClampedArray",
        "Float32Array",
        "Float64Array",
        "Float16Array",
        "DataView",
        "Map",
        "BigInt",
        "Set",
        "Iterator",
        "WeakMap",
        "WeakSet",
        "Proxy",
        "Reflect",
        "FinalizationRegistry",
        "WeakRef",
        "DisposableStack",
        "AsyncDisposableStack",
        "decodeURI",
        "decodeURIComponent",
        "encodeURI",
        "encodeURIComponent",
        "escape",
        "unescape",
        "eval",
        "isFinite",
        "isNaN",
    ];
    let got = text("Reflect.ownKeys(globalThis).join()");
    let got: Vec<&str> = got.split(',').collect();
    let ours: Vec<&str> = chrome.iter().copied().filter(|k| got.contains(k)).collect();
    assert_eq!(got, ours, "the global object's keys, in Chrome's order");
    for k in [
        "Object",
        "Function",
        "Array",
        "Error",
        "Math",
        "Reflect",
        "globalThis",
        "eval",
    ] {
        assert!(got.contains(&k), "the global object has {k}");
    }
}

#[test]
fn natives_and_bound_functions_print_as_the_reference_browser_shows_them() {
    assert_eq!(
        text("Object.prototype.hasOwnProperty.toString()"),
        "function hasOwnProperty() { [native code] }"
    );
    assert_eq!(
        text("Object.getOwnPropertyDescriptor(Object.prototype, '__proto__').get.toString()"),
        "function get __proto__() { [native code] }"
    );
    assert_eq!(
        text("Object.getOwnPropertyDescriptor(Symbol.prototype, 'description').get.name"),
        "get description"
    );
    assert_eq!(
        text("(function f(a) { return a; }).bind(null).toString()"),
        "function () { [native code] }"
    );
    assert_eq!(
        text("Function.prototype[Symbol.hasInstance].toString()"),
        "function [Symbol.hasInstance]() { [native code] }"
    );
    assert_eq!(
        text("(function f(a) { return a; }).toString()"),
        "function f(a) { return a; }"
    );
    // `new Function`'s source text, as the reference browser writes it.
    assert_eq!(
        text("new Function('a', 'b', 'return a + b').toString()"),
        "function anonymous(a,b\n) {\nreturn a + b\n}"
    );
}

#[test]
fn bound_functions_call_and_construct_their_target() {
    assert_eq!(
        text(
            "function f(a, b, c) { return [this.x, a, b, c].join(); } var g = f.bind({ x: 1 }, 2); g(3, 4)"
        ),
        "1,2,3,4"
    );
    assert_eq!(
        text("function f(a, b) {} var g = f.bind(null, 1); [g.name, g.length].join()"),
        "bound f,1"
    );
    assert_eq!(
        text(
            "function P(a, b) { this.s = a + b; } var B = P.bind(null, 1); var o = new B(2); [o.s, o instanceof P, o instanceof B].join()"
        ),
        "3,true,true"
    );
    assert_eq!(text("var b = Math.max.bind(null, 5); b(1, 9)"), "9");
    assert_eq!(
        text("new (Math.max.bind(null))()"),
        "throw TypeError: (Math.max.bind(null)) is not a constructor"
    );
}

#[test]
fn the_function_constructor_refuses_text_that_leaves_its_part() {
    // Parameters that close the function would run code at once.
    assert_eq!(
        text(
            "var leaked = 0; try { Function('a = 1) {}; leaked = 1; (function (b', ''); } catch (e) {} leaked"
        ),
        "0"
    );
    assert_eq!(
        text("try { Function('a, a', '\"use strict\";'); 'made' } catch (e) { e.name }"),
        "SyntaxError"
    );
    assert_eq!(
        text(
            "try { Function('a) { return 1 }, function (b', 'return 2'); 'made' } catch (e) { e.name }"
        ),
        "SyntaxError"
    );
    assert_eq!(
        text("try { Function('/*', '*/){'); 'made' } catch (e) { e.name }"),
        "SyntaxError"
    );
    assert_eq!(
        text("try { Function('}), (function () {'); 'made' } catch (e) { e.name }"),
        "SyntaxError"
    );
    assert_eq!(text("Function('a', 'b', 'return a * b')(6, 7)"), "42");
}

#[test]
fn arguments_objects_map_to_sloppy_parameters_only() {
    assert_eq!(
        text("function f(a) { arguments[0] = 9; return a; } f(1)"),
        "9"
    );
    assert_eq!(
        text("function f(a) { a = 9; return arguments[0]; } f(1)"),
        "9"
    );
    assert_eq!(
        text("function f(a) { 'use strict'; arguments[0] = 9; return a; } f(1)"),
        "1"
    );
    // A repeated name maps its last parameter only.
    assert_eq!(
        text("function f(a, a) { a = 9; return [arguments[0], arguments[1]].join(); } f(1, 2)"),
        "1,9"
    );
    // Deleting, or defining as not writable, ends the mapping.
    assert_eq!(
        text("function f(a) { delete arguments[0]; arguments[0] = 5; return a; } f(1)"),
        "1"
    );
    assert_eq!(
        text(
            "function f(a) { Object.defineProperty(arguments, '0', { writable: false }); a = 3; return arguments[0]; } f(1)"
        ),
        "1"
    );
    assert_eq!(
        text("function f() { return Object.prototype.toString.call(arguments); } f()"),
        "[object Arguments]"
    );
    assert_eq!(
        text("function f() { return arguments.callee === f; } f()"),
        "true"
    );
    assert_eq!(
        text(
            "function f() { 'use strict'; try { arguments.callee; } catch (e) { return e.name; } } f()"
        ),
        "TypeError"
    );
    // Arguments past the parameters are not mapped.
    assert_eq!(
        text("function f(a) { arguments[1] = 4; return arguments.length + ':' + a; } f(1, 2)"),
        "2:1"
    );
}

#[test]
fn primitives_are_wrapped_where_the_spec_wraps_them() {
    assert_eq!(
        text("function f() { return typeof this; } f.call(1) + ',' + f.call('s')"),
        "object,object"
    );
    assert_eq!(
        text("function f() { 'use strict'; return typeof this; } f.call(1)"),
        "number"
    );
    assert_eq!(
        text("var s = new String('ab'); [s.length, s[1], s instanceof String, typeof s].join()"),
        "2,b,true,object"
    );
    assert_eq!(
        text("var s = new String('ab'); s[0] = 'z'; s.x = 1; [s[0], s.x, Object.keys(s)].join()"),
        "a,1,0,1,x"
    );
    assert_eq!(
        text("'use strict'; var s = new String('ab'); try { s[0] = 'z'; } catch (e) { e.name }"),
        "TypeError"
    );
    assert_eq!(text("Object(1) + Object(2)"), "3");
    assert_eq!(
        text("new Boolean(false) ? 'object is truthy' : ''"),
        "object is truthy"
    );
    assert_eq!(text("typeof Object(Symbol('x'))"), "object");
    assert_eq!(text("Object('ab').hasOwnProperty('1')"), "true");
}

#[test]
fn conversions_reach_the_well_known_symbols() {
    assert_eq!(
        text(
            "var o = { [Symbol.toPrimitive](hint) { return hint; } }; `${o}` + ',' + (o + '') + ',' + (+{ [Symbol.toPrimitive]() { return 7; } })"
        ),
        "string,default,7"
    );
    assert_eq!(
        text("var o = { [Symbol.toStringTag]: 'Thing' }; Object.prototype.toString.call(o)"),
        "[object Thing]"
    );
    assert_eq!(
        text(
            "var C = { [Symbol.hasInstance](v) { return v === 1; } }; (1 instanceof C) + ',' + (2 instanceof C)"
        ),
        "true,false"
    );
    assert_eq!(text("Symbol.for('k') === Symbol.for('k')"), "true");
    assert_eq!(
        text("Symbol.keyFor(Symbol.for('k')) + typeof Symbol.keyFor(Symbol('k'))"),
        "kundefined"
    );
    assert_eq!(text("Symbol('d').toString()"), "Symbol(d)");
    assert_eq!(
        text("try { new Symbol(); } catch (e) { e.message }"),
        "Symbol is not a constructor"
    );
}

#[test]
fn numbers_print_as_the_spec_says() {
    assert_eq!(
        text("(255).toString(16) + ',' + (0.5).toString(2)"),
        "ff,0.1"
    );
    assert_eq!(
        text("(1.005).toFixed(2) + ',' + (2.5).toFixed(0)"),
        "1.00,3"
    );
    assert_eq!(
        text("(123.456).toPrecision(4) + ',' + (0.00001).toPrecision(1)"),
        "123.5,0.00001"
    );
    assert_eq!(text("(123456).toExponential(2)"), "1.23e+5");
    assert_eq!(text("(1234567.891).toLocaleString()"), "1,234,567.891");
    assert_eq!(
        text("try { (1).toFixed(101) } catch (e) { e.message }"),
        "toFixed() digits argument must be between 0 and 100"
    );
    assert_eq!(
        text("parseInt('  0x1F') + parseInt('08') + parseFloat('3.5e1x')"),
        "74"
    );
    assert_eq!(
        text("Number.isSafeInteger(2 ** 53) + ',' + Number.isInteger(5.0)"),
        "false,true"
    );
    assert_eq!(
        text("Math.round(-0.5) === 0 && 1 / Math.round(-0.5)"),
        "-Infinity"
    );
    assert_eq!(
        text("Math.max() + ',' + Math.min() + ',' + Math.max(1, NaN, 3)"),
        "-Infinity,Infinity,NaN"
    );
    assert_eq!(text("Math.f16round(1.337)"), "1.3369140625");
}

#[test]
fn strings_methods_work_on_units() {
    assert_eq!(
        text("'abcabc'.indexOf('c', 3) + ',' + 'abcabc'.lastIndexOf('b')"),
        "5,4"
    );
    assert_eq!(text("'a-b-c'.split('-', 2).join('|')"), "a|b");
    assert_eq!(
        text("'aXbX'.replace('X', '[$&]') + 'aXbX'.replaceAll('X', '$$')"),
        "a[X]bXa$b$"
    );
    assert_eq!(
        text("'abc'.replace('b', function (m, i, s) { return m + i + s; })"),
        "ab1abcc"
    );
    assert_eq!(text("'Stra\u{df}e'.toUpperCase()"), "STRASSE");
    assert_eq!(text("'\u{3a3}A\u{3a3}'.toLowerCase()"), "\u{3c3}a\u{3c2}");
    assert_eq!(text("'abc'.padStart(6, '12') + 'x'.repeat(3)"), "121abcxxx");
    assert_eq!(text("'  x '.trim() + '|' + ' x'.trimLeft()"), "x|x");
    assert_eq!(
        text("String.prototype.trimLeft === String.prototype.trimStart"),
        "true"
    );
    assert_eq!(
        text("'\u{1f600}'.codePointAt(0) + ',' + String.fromCodePoint(128512).length"),
        "128512,2"
    );
    assert_eq!(
        text("'ab'.at(-1) + 'ab'.charAt(5) + 'ab'.substring(2, 0)"),
        "bab"
    );
    assert_eq!(text("'x'.anchor('a\"b')"), "<a name=\"a&quot;b\">x</a>");
    assert_eq!(text("String.raw({ raw: ['a', 'b', 'c'] }, 1, 2)"), "a1b2c");
    assert_eq!(
        text("try { 'x'.repeat(-1) } catch (e) { e.message }"),
        "Invalid count value: -1"
    );
}

#[test]
fn array_methods_follow_the_spec() {
    assert_eq!(
        text("var a = [3, 1, 2]; a.sort(); a.join() + '|' + [10, 9, 1].sort().join()"),
        "1,2,3|1,10,9"
    );
    // Stable, and undefined last.
    assert_eq!(
        text(
            "[{k:1,v:'a'},{k:0,v:'b'},{k:1,v:'c'},{k:0,v:'d'}].sort(function (x, y) { return x.k - y.k; }).map(function (o) { return o.v; }).join()"
        ),
        "b,d,a,c"
    );
    assert_eq!(text("[undefined, 2, , 1].sort().length"), "4");
    assert_eq!(
        text("var a = [1, 2, 3, 4]; a.splice(1, 2, 'x').join() + '|' + a.join()"),
        "2,3|1,x,4"
    );
    assert_eq!(text("[1, [2, [3, [4]]]].flat(Infinity).join()"), "1,2,3,4");
    assert_eq!(
        text("[1, 2].flatMap(function (x) { return [x, [x]]; }).length"),
        "4"
    );
    assert_eq!(
        text("[1, 2, 3].reduce(function (a, b) { return a + b; })"),
        "6"
    );
    assert_eq!(
        text("try { [].reduce(function () {}); } catch (e) { e.message }"),
        "Reduce of empty array with no initial value"
    );
    assert_eq!(
        text("[NaN].includes(NaN) + ',' + [NaN].indexOf(NaN)"),
        "true,-1"
    );
    // A cycle joins as the empty string, as in the reference browser.
    assert_eq!(text("var a = [1]; a.push(a); a.join()"), "1,");
    assert_eq!(
        text("try { [].length = -1; } catch (e) { e.constructor.name + ': ' + e.message }"),
        "RangeError: Invalid array length"
    );
    assert_eq!(
        text("var a = [1, 2, 3]; a.length = { valueOf: function () { return 1; } }; a.join()"),
        "1"
    );
    assert_eq!(
        text("Array.from({ length: 3 }, function (v, i) { return i * 2; }).join()"),
        "0,2,4"
    );
    assert_eq!(text("Array.of(7).length + Array(3).length"), "4");
    assert_eq!(
        text("[1, 2, 3].with(-1, 9).join() + [3, 1, 2].toSorted().join()"),
        "1,2,91,2,3"
    );
    assert_eq!(
        text("Object.keys(Array.prototype[Symbol.unscopables]).length"),
        "16"
    );
}

#[test]
fn object_statics_follow_the_spec() {
    assert_eq!(
        text(
            "var o = Object.freeze({ a: 1 }); o.a = 2; [o.a, Object.isFrozen(o), Object.isSealed(o)].join()"
        ),
        "1,true,true"
    );
    assert_eq!(
        text(
            "var o = Object.create({ p: 1 }, { q: { value: 2, enumerable: true } }); [o.p, o.q, Object.keys(o)].join()"
        ),
        "1,2,q"
    );
    assert_eq!(
        text(
            "JSONless = Object.entries({ a: 1, b: 2 }).map(function (e) { return e.join('='); }).join('&')"
        ),
        "a=1&b=2"
    );
    assert_eq!(
        text(
            "var d = Object.getOwnPropertyDescriptor({ get x() { return 1; } }, 'x'); [typeof d.get, d.set, d.enumerable].join()"
        ),
        "function,,true"
    );
    assert_eq!(
        text("try { Object.defineProperty({}, 'x', { get: 1 }); } catch (e) { e.message }"),
        "Getter must be a function: 1"
    );
    assert_eq!(
        text("try { Object.setPrototypeOf(Object.prototype, {}); 'set' } catch (e) { e.name }"),
        "TypeError"
    );
    assert_eq!(
        text("Object.assign({}, { a: 1 }, null, 'xy').a + Object.assign({}, 'xy')[1]"),
        "1y"
    );
    assert_eq!(
        text("Object.is(NaN, NaN) + ',' + Object.is(0, -0)"),
        "true,false"
    );
    // Built-in methods and constants are not enumerable.
    assert_eq!(
        text(
            "(function () { var n = 0; for (var k in {}) n++; for (var k in []) n++; return n + Object.keys(Math).length + Object.keys(Array.prototype).length + Object.keys(globalThis).length; })()"
        ),
        "0"
    );
    assert_eq!(
        text(
            "var o = {}; o.__defineGetter__('g', function () { return 5; }); o.g + typeof o.__lookupGetter__('g')"
        ),
        "5function"
    );
    assert_eq!(
        text("Reflect.ownKeys({ b: 1, 1: 1, a: 1, [Symbol.iterator]: 1, 0: 1 }).length"),
        "5"
    );
    assert_eq!(
        text(
            "function C() {} var o = Reflect.construct(C, [], Object); Object.getPrototypeOf(o) === Object.prototype"
        ),
        "true"
    );
}

#[test]
fn errors_and_globals_follow_the_spec() {
    assert_eq!(
        text(
            "var e = new TypeError('m', { cause: 1 }); [e.name, e.message, e.cause, e instanceof Error, Error.isError(e)].join()"
        ),
        "TypeError,m,1,true,true"
    );
    assert_eq!(text("Error('x') instanceof Error"), "true");
    assert_eq!(text("Object.getPrototypeOf(RangeError) === Error"), "true");
    assert_eq!(
        text("Object.prototype.toString.call(new Error())"),
        "[object Error]"
    );
    assert_eq!(text("Error.stackTraceLimit"), "10");
    assert_eq!(
        text("encodeURIComponent('a b&\u{e9}') + encodeURI('/a b#')"),
        "a%20b%26%C3%A9/a%20b#"
    );
    assert_eq!(
        text("decodeURIComponent('%C3%A9%26') + decodeURI('%26%41')"),
        "\u{e9}&%26A"
    );
    assert_eq!(
        text("try { decodeURI('%E9'); } catch (e) { e.name + ': ' + e.message }"),
        "URIError: URI malformed"
    );
    assert_eq!(
        text("escape('a b\u{100}') + unescape('%u0100%41')"),
        "a%20b%u0100\u{100}A"
    );
    // Indirect eval runs in the global scope.
    assert_eq!(
        text("var x = 'global'; function f() { var x = 'local'; return (0, eval)('x'); } f()"),
        "global"
    );
    assert_eq!(
        text("try { (0, eval)('{'); } catch (e) { e.name }"),
        "SyntaxError"
    );
}

#[test]
fn legacy_caller_answers_null_for_sloppy_functions_and_throws_for_strict() {
    assert_eq!(text("String((function () {}).caller)"), "null");
    assert_eq!(
        text("try { (function () { 'use strict'; }).caller; } catch (e) { e.name }"),
        "TypeError"
    );
    assert_eq!(
        text(
            "var d = Object.getOwnPropertyDescriptor(Function.prototype, 'caller'); var a = Object.getOwnPropertyDescriptor(Function.prototype, 'arguments'); [d.get === d.set, d.get === a.get].join()"
        ),
        "false,false"
    );
    // The reference browser's accessors: four functions, named.
    assert_eq!(
        text(
            "var d = Object.getOwnPropertyDescriptor(Function.prototype, 'caller'); d.get.name + ',' + d.set.name"
        ),
        "get caller,set caller"
    );
}

#[test]
fn the_embedders_natives_run_and_throw_through_the_api() {
    let mut agent = Agent::new(AgentOptions::default());
    agent.scope(|scope| {
        let Ok(global) = scope.global_object() else {
            panic!("no global object");
        };
        let add = scope
            .native_function("add", 2, |scope, _, args| {
                let mut sum = 0.0;
                for &a in args {
                    match scope.as_number(a) {
                        Some(n) => sum += n,
                        None => {
                            let e = scope.new_error(ErrorKind::TypeError, "add wants numbers")?;
                            return Err(Thrown::Value(e));
                        }
                    }
                }
                Ok(scope.number(sum))
            })
            .unwrap_or_else(|_| panic!("native_function"));
        let key = scope.key("add");
        assert!(scope.define_property(global, key, crate::agent::PropertyDescriptor::data(add)));
    });
    assert_eq!(text_in(&mut agent, "add(1, 2, 3)"), "6");
    assert_eq!(
        text_in(
            &mut agent,
            "try { add(1, 'x'); } catch (e) { e.constructor.name + ': ' + e.message }"
        ),
        "TypeError: add wants numbers"
    );
    assert_eq!(
        text_in(&mut agent, "add.toString() + add.length + add.name"),
        "function add() { [native code] }2add"
    );
    // A native calling script calling the native, through `call`.
    agent.scope(|scope| {
        let Ok(global) = scope.global_object() else {
            panic!("no global object");
        };
        let twice = scope
            .native_function("twice", 1, |scope, _, args| {
                let f = args.first().copied().unwrap_or_else(|| scope.undefined());
                let u = scope.undefined();
                let a = scope.call(f, u, &[])?;
                let b = scope.call(f, u, &[])?;
                let a = scope.as_number(a).unwrap_or(0.0);
                let b = scope.as_number(b).unwrap_or(0.0);
                Ok(scope.number(a + b))
            })
            .unwrap_or_else(|_| panic!("native_function"));
        let key = scope.key("twice");
        assert!(scope.define_property(global, key, crate::agent::PropertyDescriptor::data(twice)));
    });
    assert_eq!(
        text_in(
            &mut agent,
            "var n = 0; twice(function () { return add(++n, 10); })"
        ),
        "23"
    );
    assert_eq!(
        text_in(
            &mut agent,
            "try { twice(function () { throw new RangeError('r'); }); } catch (e) { e.message }"
        ),
        "r"
    );
}

#[test]
fn the_realm_survives_a_collection_at_every_allocation() {
    let mut agent = Agent::new(AgentOptions::default());
    agent.set_stress(true);
    assert_eq!(
        text_in(
            &mut agent,
            "var a = [3, 1, 2].map(function (x) { return x * 2; }).sort(); a.join() + Object.keys({ p: 1 }) + 'xy'.toUpperCase() + new String('q')[0]"
        ),
        "2,4,6pXYq"
    );
    assert_eq!(
        text_in(
            &mut agent,
            "function f(a) { a = 2; return arguments[0]; } f(1) + String(Symbol.for('s').description)"
        ),
        "2s"
    );
}

#[test]
fn a_seeded_math_random_repeats() {
    let draw = |seed: u64| {
        let mut agent = Agent::new(AgentOptions::default());
        agent.set_random_seed(seed);
        text_in(
            &mut agent,
            "[Math.random(), Math.random(), Math.random()].every(function (x) { return x >= 0 && x < 1; }) + ':' + Math.random()",
        )
    };
    let a = draw(7);
    assert!(a.starts_with("true:0."), "{a}");
    assert_eq!(a, draw(7));
    assert_ne!(a, draw(8));
}

#[test]
fn the_iterator_prototypes_keys_are_in_the_reference_browsers_order() {
    // The iterator probe (Chrome 157), less what J4's iterator helpers
    // bring.
    keys_as_chrome(
        "Object.getPrototypeOf([].values())",
        &["next", "Symbol(Symbol.toStringTag)"],
        &[],
    );
    keys_as_chrome(
        "Object.getPrototypeOf(''[Symbol.iterator]())",
        &["next", "Symbol(Symbol.toStringTag)"],
        &[],
    );
    keys_as_chrome(
        "Object.getPrototypeOf(Object.getPrototypeOf([].values()))",
        &[
            "constructor",
            "reduce",
            "toArray",
            "forEach",
            "some",
            "every",
            "find",
            "map",
            "filter",
            "take",
            "drop",
            "flatMap",
            "join",
            "includes",
            "Symbol(Symbol.iterator)",
            "Symbol(Symbol.toStringTag)",
            "Symbol(Symbol.dispose)",
        ],
        &[],
    );
}

#[test]
fn the_generator_objects_keys_are_in_the_reference_browsers_order() {
    // The generator probe (Chrome 157).
    let gp = "Object.getPrototypeOf(function* () {}).prototype";
    keys_as_chrome(
        gp,
        &[
            "constructor",
            "next",
            "return",
            "throw",
            "Symbol(Symbol.toStringTag)",
        ],
        &[],
    );
    keys_as_chrome(
        "Object.getPrototypeOf(function* () {})",
        &["prototype", "constructor", "Symbol(Symbol.toStringTag)"],
        &[],
    );
    keys_as_chrome(
        "Object.getPrototypeOf(function* () {}).constructor",
        &["length", "name", "prototype"],
        &[],
    );
    keys_as_chrome(
        "(function* g(a, b) {})",
        &["length", "name", "prototype"],
        &[],
    );
    keys_as_chrome("(function* g() {}).prototype", &[], &[]);
    keys_as_chrome("(function* g() {})()", &[], &[]);
    keys_as_chrome("({ *m() {} }).m", &["length", "name", "prototype"], &[]);
    // Their descriptors: next, return and throw of length 1; a generator
    // function's prototype writable only.
    assert_eq!(
        text(&format!(
            "var p = {gp}; var d = Object.getOwnPropertyDescriptor(function* () {{}}, 'prototype'); \
             [p.next.length, p.return.length, p.throw.length, d.writable, d.enumerable, d.configurable].join()"
        )),
        "1,1,1,true,false,false"
    );
}
