# Chrome's observed surface

What the reference browser shows a page where ECMA-262 leaves the
engine free (design note, section 1.11). Every row was observed in
Chrome 156 by Silver Browser's web-engine session (its probe page,
forwarded by the lead on 2026-11-20). None comes from another engine's
source.

Later probes, on Chrome 157.0.8103.80 (headless, a throwaway profile,
the page loaded from a file), were asked of the same session by
SilverJS on 2026-11-21: the built-ins' own-key orders, the
`Function.prototype` accessors, and 46 stack cases on a page SilverJS
wrote (one case a line, so that every line and column maps back); the
rows marked "Chrome 157" below are theirs.

Each row is pinned by a SilverJS test when the step that makes it
observable lands (J2.4 for thrown errors, J2.5 for the rest); the last
column names that test, and a row without one is not done. A row that
changes when Chrome is probed again is changed here first, then in its
test.

## Error.prototype.stack

| Observed | Step | Test |
|---|---|---|
| The first line is `Name: message`: the name read when `stack` is first read, the message as the error was made (`Changed: m`) (Chrome 157) | J2.5 | `stack_tests::the_limit_the_heading_and_the_accessor_are_the_reference_browsers` |
| A script's own top-level frame: `at url:l:c` | J2.5 | `stack_tests::frames_name_functions_methods_and_constructions` |
| A named function's frame: `at f (url:l:c)`; an anonymous one's, the location alone; an arrow's, its inferred name; a getter's, `get g` without a type (Chrome 157) | J2.5 | `stack_tests::frames_name_functions_methods_and_constructions` |
| Code from strings: `at eval (eval at <anonymous> (url:l:c), <anonymous>:3:8)`; a direct eval names its caller (`eval at ev (...)`), an indirect one shows the `eval` native's frame too (Chrome 157) | J2.5 | `stack_tests::code_from_strings_shows_where_it_was_made` |
| A method's frame is named `Object.m` style: its receiver's constructor's name (a function receiver's own name, `Ctor.st`), the property it is held under standing in for no name (`Ctor.pm`) (Chrome 157) | J2.5 | `stack_tests::frames_name_functions_methods_and_constructions` |
| A built-in's frame: `at Array.map (<anonymous>)`; `Function.prototype.call` and `apply` show none (Chrome 157) | J2.5 | `stack_tests::natives_show_as_anonymous_and_call_and_apply_not_at_all` |
| A frame's column: a construction's at its `new`, a call's at the name called (the property's, for a method), else at its arguments' `(`; a property read's at the property's name; in UTF-16 units (Chrome 157) | J2.5 | `stack_tests::frames_name_functions_methods_and_constructions` |
| `Error.stackTraceLimit` is 10 by default: a data property, writable, enumerable and configurable (Chrome 157); not a number (or deleted), no stack: the accessor answers undefined (an accessor `stackTraceLimit` counts as not a number: SilverJS's choice, not probed) | J2.5 | `builtins_tests::errors_and_globals_follow_the_spec`, `stack_tests::the_limit_the_heading_and_the_accessor_are_the_reference_browsers` |
| With `Error.stackTraceLimit = 0`, `stack` is the heading alone (`Error: l`) | J2.5 | `stack_tests::the_limit_the_heading_and_the_accessor_are_the_reference_browsers` |
| `stack` is an own accessor of each error object, not on `Error.prototype`: a getter and a setter, not enumerable, configurable, both named `""` and shared by every error; setting it replaces the text and keeps the accessor; own keys `stack,message,cause` (Chrome 157) | J2.5 | `stack_tests::the_limit_the_heading_and_the_accessor_are_the_reference_browsers` |
| `Error.captureStackTrace(o, fn)` gives `o` the same accessor, the frames below `fn`'s topmost, headed by `o`'s name and message; it returns undefined (Chrome 157) | J2.5 | `builtins_tests::own_keys_are_in_the_reference_browsers_order`, `stack_tests::capture_stack_trace_gives_any_object_a_stack` |
| A page's own `Error.prepareStackTrace` is called at the first read of `stack` (not at the error's making), with `Error` as `this`, the error and its CallSite objects; its value, of any type, is kept; what it throws, `stack` throws. CallSite's methods, in order: `constructor,getColumnNumber,...,toString` (Chrome 157) | J2.5 | `stack_tests::prepare_stack_trace_is_called_at_the_first_read_with_call_sites` |
| `prepareStackTrace` is not an own property of `Error` by default (Chrome 157) | J2.5 | `builtins_tests::own_keys_are_in_the_reference_browsers_order` |
| A stack overflow's top frame stands at the callee's entry (`at r (url:54:57)`, its parameters' `(`) | J2.5 | not followed: SilverJS shows the call that overflowed (`at r (url:54:67)`), the frame count the same |

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
| `new Function` code: `function anonymous(a,b\n) {\nbody\n}`, and with no arguments `function anonymous(\n) {\n\n}` (Chrome 157) | J2.5 | `builtins_tests::natives_and_bound_functions_print_as_the_reference_browser_shows_them` |

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
