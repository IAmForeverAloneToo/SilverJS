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
| A generator's frame is followed by its resumption, a native named after the generator function: `at thrower (url:l:c)`, `at thrower.next (<anonymous>)`; a frame waiting in `yield*` stands at its operand (Chrome 157, the generator probe) | J3.2 | `generator_tests::a_generators_frames_show_on_the_stack` |
| A class's frames: `new S` for its constructor (a derived class's `super()` at `super`), `S.m` and `S.st` for its methods and static methods, `get acc` for an accessor, `new <anonymous>` for an anonymous class (Chrome 157, the class probe) | J3.3a | `class_tests::class_frames_show_on_the_stack` |
| A field initializer's frame: `<instance_members_initializer>`, called from `new S` (a default constructor's at its class, a derived one's at its `super()`); a static field's or block's: `<static_initializer>`; a private method's or accessor's: its name alone (`#pm`, `get #pg`), its call standing at its `(` (Chrome 157, the second class probe) | J3.3b | `class_tests::initializer_frames_show_on_the_stack` |
| Below an error's frames, each async function awaiting the one at the bottom, at its `await`: `at async outer (url:l:c)` (Chrome 157, the async probe) | J3.4a | `async_tests::async_frames_show_on_the_stack` |
| An async generator resumed by `next` shows it (`at thrower.next (<anonymous>)`); the async function awaiting one through `for await` stands at the loop's binding (`at async consume (url:l:c)`, the column of `e` in `for await (var e of ...)`) (Chrome 157, the async generator probe) | J3.4b | `async_tests::async_generator_frames_show_on_the_stack` |
| A static initializer's caller frame stands at the previous statement's start (`at url:57:1` for a class at line 58) | J3.3b | not followed: SilverJS shows the class's start |

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
| `1 is not iterable`, `undefined is not iterable`, `{} is not iterable`: the iterable's expression text as written (`for-of`, array spread); an array pattern's value names itself where it has no source (Chrome 157, the iterator probe) | J3.1 | `iteration_tests::iteration_errors_read_as_the_reference_browsers` |
| A call's spread of a non-iterable: `Spread syntax requires ...iterable[Symbol.iterator] to be a function` (Chrome 157) | J3.1 | `iteration_tests::iteration_errors_read_as_the_reference_browsers` |
| `Cannot destructure property 'a' of 'null' as it is null.` (Chrome 157); of a member's value, its text (`'o.x'`), and of a parameter's, the value (`'undefined'`) | J3.1 | `iteration_tests::iteration_errors_read_as_the_reference_browsers` |
| `Iterator result 1 is not an object` (from `next` and from `return`), `Result of the Symbol.iterator method is not an object` (Chrome 157) | J3.1 | `iteration_tests::iteration_errors_read_as_the_reference_browsers` |
| `Method Array Iterator.prototype.next called on incompatible receiver #<Object>`, and String Iterator's (Chrome 157) | J3.1 | `iteration_tests::iteration_errors_read_as_the_reference_browsers` |
| `Generator is already running`, `Method [Generator].prototype.next called on incompatible receiver #<Object>` (and `return`, `throw`), `The iterator does not provide a 'throw' method.`, a `yield*`'s value `number 1 is not iterable (cannot read property Symbol(Symbol.iterator))` (Chrome 157, the generator probe) | J3.2 | `generator_tests::generator_errors_read_as_the_reference_browsers` |
| `Class constructor A cannot be invoked without 'new'`, `Class extends value 1 is not a constructor or null` (a function by its source text), `Class extends value does not have valid prototype property 1`, `Must call super constructor in derived class before accessing 'this' or returning from derived constructor`, `Super constructor may only be called once`, `Derived constructors may only return object or undefined`, `Super constructor max of X is not a constructor`, `Cannot read properties of null (reading 'm')` for `super.m` with no prototype (Chrome 157, the class probe) | J3.3a | `class_tests::classes_throw_chromes_errors`, `class_tests::a_derived_constructors_this_waits_for_super` |
| `delete super.x`: `Unsupported reference to 'super'` (a ReferenceError); `new` of a class extending null: `Super constructor null of X is not a constructor` (`of anonymous class` for one with no name) (Chrome 157, the second class probe) | J3.3a | `class_tests::classes_throw_chromes_errors` |
| `Cannot read private member #p from an object whose class did not declare it` (and `Cannot write ... to`), `Receiver must be an instance of class X` (`Receiver must be class X` for a static method), `'#g' was defined without a setter`, `'#s' was defined without a getter`, `Private method '#m' is not writable`, `Cannot initialize #p twice on the same object`, `Cannot initialize private methods of class X twice on the same object`, `Cannot use 'in' operator to search for '#p' in 1`, `Cannot define property f, object is not extensible` (Chrome 157, the class probes) | J3.3b | `class_tests::private_names_reach_what_their_class_declared`, `class_tests::fields_are_defined_in_order` |
| A private method of an anonymous class reached through the wrong receiver: `Receiver must be an instance of class anonymous` | J3.3b | SilverJS's choice, not probed |
| `Promise constructor cannot be invoked without 'new'`, `Promise resolver 1 is not a function`, `Method Promise.prototype.then called on incompatible receiver #<Object>`, `PromiseResolve called on non-object`, `Chaining cycle detected for promise #<Promise>`, `All promises were rejected` (an AggregateError), a combinator's `number 1 is not iterable (cannot read property Symbol(Symbol.iterator))`; `await` outside an async function: `await is only valid in async functions and the top level bodies of modules` (Chrome 157, the async probe) | J3.4a | `async_tests::promises_throw_and_reject_with_chromes_messages` |
| A function or class written in place and constructed: `(intermediate value) is not a constructor` (Chrome 157, the async probe) | J3.4a | `async_tests::promises_throw_and_reject_with_chromes_messages` |
| `Method [AsyncGenerator].prototype.next called on incompatible receiver #<Object>` (a rejection), `1 is not async iterable` (a `for await`'s value, by its source), an async generator's `yield* 1`: `undefined is not a function`, `Iterator result 1 is not an object` (Chrome 157, the async generator probe) | J3.4b | `async_tests::async_generators_have_chromes_shape_and_messages` |

## Function.prototype.toString

| Observed | Step | Test |
|---|---|---|
| Natives, getters and the embedder's methods: `function name() { [native code] }` | J2.5 | `builtins_tests::natives_and_bound_functions_print_as_the_reference_browser_shows_them`, `builtins_tests::the_embedders_natives_run_and_throw_through_the_api` |
| Bound functions: `function () { [native code] }` (not `function bound f()`) | J2.5 | `builtins_tests::natives_and_bound_functions_print_as_the_reference_browser_shows_them` |
| Classes: their source text, `class A { m() {} }`, a default constructor's too (`class D extends C {}`); a class's methods and accessors theirs, from their name, past `static` (`m() {}`, `sm() {}`, `get g() {}`, `#m() {}`) (Chrome 157, the class probe) | J3.3a | `class_tests::a_class_has_chromes_shape` |
| `new Function` code: `function anonymous(a,b\n) {\nbody\n}`, and with no arguments `function anonymous(\n) {\n\n}` (Chrome 157) | J2.5 | `builtins_tests::natives_and_bound_functions_print_as_the_reference_browser_shows_them` |

## Function.prototype.caller and arguments

| Observed (Chrome 157) | Step | Test |
|---|---|---|
| Each is an accessor of its own, getter and setter apart (`get caller`, `set caller`), not `%ThrowTypeError%` | J2.5 | `builtins_tests::legacy_caller_answers_null_for_sloppy_functions_and_throws_for_strict` |
| A sloppy function's `caller` is null; a strict function's throws a TypeError | J2.5 | `builtins_tests::legacy_caller_answers_null_for_sloppy_functions_and_throws_for_strict` |

ECMA-262 has `%ThrowTypeError%` as both accessors; Test262's three tests
of that identity, which QuickJS-ng passes, are recorded failures for
this reason.

## DataView and Atomics out of bounds

| Observed (Chrome 157, the binary data probes) | Step | Test |
|---|---|---|
| A `DataView`'s `byteLength` and `byteOffset` throw `Cannot perform get DataView.prototype.byteLength on a detached or out-of-bounds ArrayBuffer` once its buffer is detached or shrunk past it | J4.2b | `typed_tests::views_atomics_and_base64_read_as_the_reference_browser_gives_them` |
| An `Atomics` operation whose element a conversion shrank away throws `Cannot perform Atomics.add on a detached or out-of-bounds ArrayBuffer` (a TypeError), even where its view is still in bounds | J4.2b | `typed_tests::views_atomics_and_base64_read_as_the_reference_browser_gives_them` |

Test262's six tests that want 0 from a detached or out-of-bounds
`DataView`'s accessors are recorded failures for this reason; ECMA-262
has a RangeError where the `Atomics` row has a TypeError.

## Own-key order of built-ins

| Observed | Step | Test |
|---|---|---|
| `Error.prototype`: `constructor,name,message,toString` | J2.5 | `builtins_tests::own_keys_are_in_the_reference_browsers_order` |
| `Function.prototype` has no `fileName`, `lineNumber` or `columnNumber` | J2.5 | `builtins_tests::own_keys_are_in_the_reference_browsers_order` |
| The global object's ECMAScript names, `Object,Function,Array,Number,parseFloat,parseInt,Infinity,NaN,undefined,Boolean,String,Symbol,Date,...` (its web APIs after) (Chrome 157) | J2.5 | `builtins_tests::the_global_objects_built_ins_are_in_the_reference_browsers_order` |
| `Object`, `Object.prototype`, `Function`, `Function.prototype`, `Error` and the native errors, `Array`, `Array.prototype` and its `@@unscopables`, `String`, `String.prototype`, `Number`, `Number.prototype`, `Boolean`, `Boolean.prototype`, `Symbol`, `Symbol.prototype`, `Math`, `Reflect` (Chrome 157) | J2.5 | `builtins_tests::own_keys_are_in_the_reference_browsers_order` |
| `%GeneratorPrototype%`: `constructor,next,return,throw,@@toStringTag` (its methods of length 1); `%GeneratorFunction.prototype%`: `prototype,constructor,@@toStringTag`; `%GeneratorFunction%`: `length,name,prototype`; a generator function's and method's own keys `length,name,prototype`, its `prototype` writable only and without keys of its own (Chrome 157, the generator probe) | J3.2 | `builtins_tests::the_generator_objects_keys_are_in_the_reference_browsers_order` |
| A class: `length,name,prototype`, then its static members in order; its `prototype` neither writable, enumerable nor configurable; the prototype's keys: integer keys, `constructor`, then its methods and accessors in order, not enumerable; a method's own keys `length,name`, with no `prototype` (Chrome 157, the class probe) | J3.3a | `class_tests::a_class_has_chromes_shape` |
| An instance's fields: data properties, writable, enumerable and configurable, defined (not set) in order, integer keys first; a static field likewise on the class after `length,name,prototype`; private fields and methods are no own keys at all, and are added to a frozen object (Chrome 157, the second class probe) | J3.3b | `class_tests::fields_are_defined_in_order` |
| `Promise`: `length,name,prototype,all,allSettled,any,race,resolve,reject,withResolvers,try,@@species`; `Promise.prototype`: `constructor,then,catch,finally,@@toStringTag`; `%AsyncFunction.prototype%`: `constructor` (not writable), `@@toStringTag`; an async function's own keys `length,name` (no `prototype`); resolving functions named `""` (Chrome 157, the async probe) | J3.4a | `async_tests::promise_has_chromes_shape` |
| The order jobs run in: a promise's reactions in the order they were added, an await of a native promise one job, of a thenable two, an async function's return of a promise three (Chrome 157, the async probe) | J3.4a | `async_tests::jobs_run_in_the_reference_browsers_order` |
| `%AsyncGeneratorFunction%`: `length,name,prototype`; its prototype: `prototype,constructor,@@toStringTag`, none writable; `%AsyncGeneratorPrototype%`: `constructor` (an object, not writable), `next,return,throw` (length 1), `@@toStringTag`; `%AsyncIteratorPrototype%`: `@@asyncIterator`, `@@asyncDispose`; an async generator function's own keys `length,name,prototype`, its `prototype` writable only and without keys of its own (Chrome 157, the async generator probe) | J3.4b | `async_tests::async_generators_have_chromes_shape_and_messages` |
| The order an async generator's requests settle in: a yield awaits its operand, then the next waiting request goes on at once (`start, got b, tick1, ...`) (Chrome 157, the async generator probe) | J3.4b | `async_tests::async_generators_settle_their_requests_in_the_reference_browsers_order` |
| `%ArrayIteratorPrototype%` and `%StringIteratorPrototype%`: `next,@@toStringTag`; `Array.prototype[@@iterator]` and an arguments object's are `Array.prototype.values`; `%IteratorPrototype%` is `constructor,reduce,toArray,forEach,some,every,find,map,filter,take,drop,flatMap,join,includes,@@iterator,@@toStringTag,@@dispose` (the iterator probe, Chrome 157), of which J3.1 brings `@@iterator` and J4's iterator helpers the rest | J3.1 | `builtins_tests::own_keys_are_in_the_reference_browsers_order` |
| A function's own keys `length,name,prototype`, an arrow's and a bound function's `length,name`, an arguments object's `0,length,callee,@@iterator`, an error's `stack,message` (Chrome 157) | J2.5 | `builtins_tests::own_keys_are_in_the_reference_browsers_order` (the error's with the stack) |

The probes' lists are in that test, each with the names a later step
brings (`JSON` is `parse,stringify,rawJSON,isRawJSON,@@toStringTag`,
for J4).

## Date

| Observed | Step | Test |
|---|---|---|
| `Date.parse` past the Date Time String Format: month names by their first three letters (a later one replacing an earlier), the date's numbers in the order their values tell (past 31 or 0 a year; else month, day, year), a year below 50 in 20xx and below 100 in 19xx, none 2001; a time of colons whose parts past 59 go to the date; AM and PM after a time; zone names (UT, UTC, GMT, Z and the US zones) once a number is read; an offset after a time or a zone name replacing it; a `-` straight after a number or word separating; comments in parentheses (Chrome 158, the dates probe: 439 strings in UTC and in New York) | J4.3d | `date_tests::the_corpus_parses_as_the_reference_browser_parses_it_in_utc`, `..._in_new_york` |
| Which characters `Date.parse` skips (the other ASCII symbols and control characters), takes as part of a word (`[\]^_`, the backquote, `{\|}~`, DEL and every character past ASCII but space, so that a word of them refuses the string after a number), takes as space (ASCII's, no-break space, the BOM and the space separators, not the line separators) and ends at (NUL); a time must end at the end, at space, at a sign or at Z (Chrome 158, the symbols and spaces probes) | J4.3d | `date_tests::symbols_and_spaces_parse_as_the_reference_browser_parses_them` |
| A `-000000` year, which the format forbids, is skipped and the rest read as the other forms (`-000000-01-01` is 1 January 2001, local); with a time after it, refused (Chrome 158; Test262's `year-zero`) | J4.3d | `date_tests::dates_read_as_the_reference_browser_gives_them_in_utc` |
| `toString`'s zone offset in whole minutes (`GMT-0456` for mean time's -4:56:02), and a time before 1883 named by today's daylight rules though its offset is mean time; `getTimezoneOffset` gives the fraction (296.0333...) | J4.3d | `date_tests::the_corpus_parses_as_the_reference_browser_parses_it_in_new_york` |
| `toLocaleString`, `toLocaleDateString` and `toLocaleTimeString`: until `Intl`, the reference browser's en-US forms alone (`11/24/2026, 10:00:00 AM`), a year at or before 0 written without an era (`1 - year`); other locales and the options wait for `Intl` | J4.3d | `date_tests::the_corpus_parses_as_the_reference_browser_parses_it_in_utc` |
| Receivers: a getter's or string form's non-date `this` is `this is not a Date object.`; `setTime`'s and `[Symbol.toPrimitive]`'s are the generic incompatible-receiver message; `Invalid hint: x` (Chrome 158, the numbers probe) | J4.3d | `date_tests::dates_read_as_the_reference_browser_gives_them_in_utc` |

## Iterator helpers

| Observed | Step | Test |
|---|---|---|
| `Iterator.prototype` has `join` and `includes` and `Iterator` has `concat`, `zip` and `zipKeyed`, beyond ECMA-262; it has no `chunks` or `windows`, and `Promise` no `allKeyed` or `allSettledKeyed`, which Test262 tests (not applicable: Chrome 158 doesn't ship them) (Chrome 158, the J4.4 probe) | J4.4a | `iter_tests::iterators_read_as_the_reference_browser_gives_them` |
| The helpers' messages: `-1 must be positive` (and past 2^53 - 1, `9007199254740992 must be positive`) for `take`, `drop` and `includes`' count, `skippedElements argument must be undefined or an integer`, `Iterator.prototype.map called on non-object`, `Reduce of a done iterator with no initial value`, `Generator is already running`, `options called on non-object`, `Iterator.zip mode must be 'shortest', 'longest', or 'strict'`, `Iterators passed to Iterator.zip in 'strict' mode must have the same length`, a `zip` item's `1 is not iterable`, `concat`'s `'1' returned for property 'Symbol(Symbol.iterator)' of object '#<Object>' is not a function` (Chrome 158, the J4.4 messages probe) | J4.4a | `iter_tests::iterators_read_as_the_reference_browser_gives_them` |

## Explicit resource management

| Observed | Step | Test |
|---|---|---|
| The messages: `` An object is expected with `using` declarations `` (a primitive resource, also for the stacks' `use`), a resource's method named by its key (`Symbol(Symbol.dispose) is not a function`, `Symbol(Symbol.asyncDispose) is not a function`), `Cannot call AsyncDisposableStack.prototype.use on an already-disposed DisposableStack` (the async stack's too ends in `DisposableStack`), `Receiver is not an AsyncDisposableStack` (`disposeAsync`'s rejection), `'1' returned for property 'return' of object '#<Object>' is not a function` (`[Symbol.asyncDispose]`'s rejection), `An error was suppressed during disposal` (Chrome 158, the J4.4b probe) | J4.4b | `using_tests::disposables_read_as_the_reference_browser_gives_them` |
| A `SuppressedError`'s own properties are `stack`, `message`, `error`, `suppressed` in that order, also the constructor's (`message` only when given) (Chrome 158, the J4.4b probe) | J4.4b | `using_tests::disposables_read_as_the_reference_browser_gives_them` |
| The ticks between eight chained reactions of `await using` (an async resource, a sync one, null, null before a sync one, a rejection), of `disposeAsync` and of `%AsyncIteratorPrototype%[Symbol.asyncDispose]`, whose `return()` is called with no argument (Chrome 158, the J4.4b probe) | J4.4b | `using_tests::awaits_take_the_reference_browsers_ticks` |

## The built-ins' remainders

| Observed | Step | Test |
|---|---|---|
| `Promise.try` gives the callback's promise back as it is when its constructor is the receiver; `Object.groupBy(null)` throws `Object.groupBy called on null or undefined`; `Object.setPrototypeOf(Object.prototype, x)` and its `__proto__` throw `Immutable prototype object 'Object.prototype' cannot have their prototype set`, `Reflect.setPrototypeOf` gives false (Chrome 158, the J4.4c probe) | J4.4c | `remainder_tests::remainders_read_as_the_reference_browser_gives_them` |
| `Array.fromAsync`'s ticks between ten chained reactions, its rejections (`Cannot read properties of null (reading 'Symbol(Symbol.asyncIterator)')`, `Iterator result Array.fromAsync is not an object`, `Result of the Symbol.iterator method is not an object` for an async iterator too); unlike the spec, it closes its iterator when `next()` throws or its result rejects, so a sync iterator whose value rejects is closed twice, by the async-from-sync wrapper and by `fromAsync` (Chrome 158, the J4.4c probe and its close probe) | J4.4c | `remainder_tests::from_async_takes_the_reference_browsers_ticks` |
| A for-in over a proxy reads its `ownKeys` and `getPrototypeOf` first, then each key's descriptor as the key comes, before the body; a key with none is looked for through the proxy's prototype, with no `has` trap (Chrome 158, the J4.4c for-in probe) | J4.4c | `remainder_tests::for_in_reads_a_proxy_as_the_reference_browser_does` |

## Unicode

| Observed | Step | Test |
|---|---|---|
| Unicode 17.0: code points first assigned in 17.0 (Sidetic, Tolong Siki, Beria Erfe, Tai Yo) are letters of their scripts, identifier characters and case-mapped; those of 18.0 (Jurchen, the new Latin and Armenian letters) are none of these, and `\p{Script=Jurchen}` is a SyntaxError (Chrome 158, the J4.4d probe) | J4.4d | `normalize_tests::normalize_and_the_version_read_as_the_reference_browser_gives_them`, `normalize_tests::the_unicode_version_is_one_everywhere` |
| `normalize` keeps all 4,780,592 invariants of UCD 17.0.0's NormalizationTest.txt; its messages are `The normalization form should be one of NFC, NFD, NFKC, NFKD.` and `String.prototype.normalize called on null or undefined`; a lone surrogate stays as it is (Chrome 158, the J4.4d normalize probe) | J4.4d | `normalize_tests::normalize_keeps_the_conformance_files_invariants` |

## RegExp

| Observed | Step | Test |
|---|---|---|
| The messages: `Invalid regular expression: /(/: Unterminated group` (the pattern between slashes, then the reason), `Invalid flags supplied to RegExp constructor 'gg'`, `Method RegExp.prototype.exec called on incompatible receiver #<Object>`, `RegExp.prototype.flags getter called on non-object 1`, `RegExp.prototype.global getter called on non-RegExp object`, `String.prototype.matchAll called with a non-global RegExp argument` (and `replaceAll`'s), `Method %RegExpStringIterator%.prototype.next called on incompatible receiver #<Object>`, `Invalid named capture referenced`, `RegExp exec method returned something other than an Object or null`; a read-only `lastIndex` is `Cannot assign to read only property 'lastIndex' of object '[object RegExp]'` (Chrome 158, the J4.5 probes) | J4.5a | `regexp_tests::messages_are_chromes` |
| A u-mode regexp whose `lastIndex` falls inside a surrogate pair starts at the pair's lead (Chrome 158, the J4.5 probe) | J4.5a | `regexp_tests::last_index_moves_as_in_chrome` |
| `match`, `matchAll`, `replace`, `replaceAll`, `search` and `split` never look a symbol method up on a primitive argument: a getter on `Number.prototype[Symbol.match]` is not called (ecma262 PR 3009, after the pinned Test262) (Chrome 158, the j45 prim probe) | J4.5a | `regexp_tests::a_primitive_pattern_is_never_asked_for_its_method` |
| Catastrophic patterns, each in a Worker stopped at 10 s: `^(a+)+$` on 25, 30 and 40 a's and a `!` takes 0.3 s, 8.9 s and did not finish; `(a*)*b` on 30 and `^(a\|a?)+$` on 30 did not finish; `(a\|aa)+c` on 30 and 40 0.1 s and 7.6 s; an email-like pattern on 30 0.9 s; `^(a+)+\1$` on 25 and 28 0.4 s and 2.9 s; `a*b` on 100,000 a's at once (Chrome 158, the j45 redos probe). SilverJS memoizes a pattern without backreferences once it has backtracked enough, so each of these but the backreference's ends in milliseconds; the backreference's grows as Chrome's does, and every run stops within 100 ms of the agent's interrupt | J4.5a | `regexp_tests::a_catastrophic_pattern_stops_soon_after_the_interrupt`, `regexp::vm::tests::a_catastrophic_pattern_finishes` |
