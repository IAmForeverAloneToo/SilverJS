# Chrome's observed surface

What the reference browser shows a page where ECMA-262 leaves the
engine free (design note, section 1.11). Every row was observed in
Chrome 156 by Silver Browser's web-engine session (its probe page,
forwarded by the lead on 2026-11-20). None comes from another engine's
source.

A second probe, on Chrome 157.0.8103.80 (headless, a throwaway
profile, the page loaded from a file), was asked of the same session by
SilverJS on 2026-11-21: the built-ins' own-key orders, and the rows
marked "Chrome 157" below.

Each row is pinned by a SilverJS test when the step that makes it
observable lands (J2.4 for thrown errors, J2.5 for the rest); the last
column names that test, and a row without one is not done. A row that
changes when Chrome is probed again is changed here first, then in its
test.

## Error.prototype.stack

| Observed | Step | Test |
|---|---|---|
| The first line is `Name: message` | J2.5 | |
| A script's own top-level frame: `at url:l:c` | J2.5 | |
| A named function's frame: `at f (url:l:c)` | J2.5 | |
| Code from strings: `at eval (eval at <anonymous> (url:l:c), <anonymous>:3:8)` | J2.5 | |
| A method's frame is named `Object.m` style | J2.5 | |
| A built-in's frame: `at <anonymous>` | J2.5 | |
| A constructor call's column is at its `new` | J2.5 | |
| `Error.stackTraceLimit` is 10 by default: a data property, writable, enumerable and configurable (Chrome 157) | J2.5 | `builtins_tests::errors_and_globals_follow_the_spec` (the value) |
| With `Error.stackTraceLimit = 0`, `stack` is the heading alone (`Error: l`) | J2.5 | |
| `stack` is an own accessor of each error object, not on `Error.prototype`: a getter and a setter, not enumerable, configurable (Chrome 157) | J2.5 | |
| `Error.captureStackTrace` exists | J2.5 | `builtins_tests::own_keys_are_in_the_reference_browsers_order` |
| A page's own `Error.prepareStackTrace` is called | J2.5 | |
| `prepareStackTrace` is not an own property of `Error` by default (Chrome 157) | J2.5 | `builtins_tests::own_keys_are_in_the_reference_browsers_order` |

## Error messages

| Observed | Step | Test |
|---|---|---|
| `Cannot read properties of null (reading 'length')` | J2.4 | `interp_tests::errors_are_worded_as_chrome_words_them` |
| `Cannot read properties of undefined (reading 'length')` | J2.4 | `interp_tests::errors_are_worded_as_chrome_words_them` |
| `Cannot set properties of null (setting 'x')`, and of `undefined` (the tail confirmed by Chrome 157) | J2.4 | `interp_tests::errors_are_worded_as_chrome_words_them` |
| `o.f is not a function`: the callee's expression text as written | J2.4 | `interp_tests::errors_are_worded_as_chrome_words_them` |
| `... is not a constructor`: the expression text as written | J2.4 | `builtins_tests::bound_functions_call_and_construct_their_target` |
| `Assignment to constant variable.` | J2.4 | `interp_tests::errors_are_worded_as_chrome_words_them` |
| `Cannot convert a Symbol value to a string` | J2.4 | `interp_tests::errors_are_worded_as_chrome_words_them` |
| `Cannot mix BigInt and other types, use explicit conversions` | J4 (BigInt) | |

## Function.prototype.toString

| Observed | Step | Test |
|---|---|---|
| Natives, getters and the embedder's methods: `function name() { [native code] }` | J2.5 | `builtins_tests::natives_and_bound_functions_print_as_the_reference_browser_shows_them`, `builtins_tests::the_embedders_natives_run_and_throw_through_the_api` |
| Bound functions: `function () { [native code] }` (not `function bound f()`) | J2.5 | `builtins_tests::natives_and_bound_functions_print_as_the_reference_browser_shows_them` |
| Classes: their source text, `class A { m() {} }` (Chrome 157) | J3 | |
| `new Function` code: `function anonymous(a,b\n) {\nbody\n}` (expected; to be confirmed by the next probe, with the stack's rows) | J2.5 | `builtins_tests::natives_and_bound_functions_print_as_the_reference_browser_shows_them` |

## Function.prototype.caller and arguments

| Observed (Chrome 157) | Step | Test |
|---|---|---|
| Each is an accessor of its own, getter and setter apart (`get caller`, `set caller`), not `%ThrowTypeError%` | J2.5 | `builtins_tests::legacy_caller_answers_null_for_sloppy_functions_and_throws_for_strict` |
| A sloppy function's `caller` is null; a strict function's throws a TypeError | J2.5 | `builtins_tests::legacy_caller_answers_null_for_sloppy_functions_and_throws_for_strict` |

ECMA-262 has `%ThrowTypeError%` as both accessors; Test262's three tests
of that identity, which QuickJS-ng passes, are recorded failures for
this reason.

## Own-key order of built-ins

| Observed | Step | Test |
|---|---|---|
| `Error.prototype`: `constructor,name,message,toString` | J2.5 | `builtins_tests::own_keys_are_in_the_reference_browsers_order` |
| `Function.prototype` has no `fileName`, `lineNumber` or `columnNumber` | J2.5 | `builtins_tests::own_keys_are_in_the_reference_browsers_order` |
| The global object's ECMAScript names, `Object,Function,Array,Number,parseFloat,parseInt,Infinity,NaN,undefined,Boolean,String,Symbol,Date,...` (its web APIs after) (Chrome 157) | J2.5 | `builtins_tests::the_global_objects_built_ins_are_in_the_reference_browsers_order` |
| `Object`, `Object.prototype`, `Function`, `Function.prototype`, `Error` and the native errors, `Array`, `Array.prototype` and its `@@unscopables`, `String`, `String.prototype`, `Number`, `Number.prototype`, `Boolean`, `Boolean.prototype`, `Symbol`, `Symbol.prototype`, `Math`, `Reflect` (Chrome 157) | J2.5 | `builtins_tests::own_keys_are_in_the_reference_browsers_order` |
| A function's own keys `length,name,prototype`, an arrow's and a bound function's `length,name`, an arguments object's `0,length,callee,@@iterator`, an error's `stack,message` (Chrome 157) | J2.5 | `builtins_tests::own_keys_are_in_the_reference_browsers_order` (the error's with the stack) |

The probes' lists are in that test, each with the names a later step
brings (`JSON` is `parse,stringify,rawJSON,isRawJSON,@@toStringTag`,
for J4).
