# Chrome's observed surface

What the reference browser shows a page where ECMA-262 leaves the
engine free (design note, section 1.11). Every row was observed in
Chrome 156 by Silver Browser's web-engine session (its probe page,
forwarded by the lead on 2026-11-20). None comes from another engine's
source.

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
| `Error.stackTraceLimit` is 10 by default | J2.5 | |
| With `Error.stackTraceLimit = 0`, `stack` is the heading alone (`Error: l`) | J2.5 | |
| `stack` is an own accessor of each error object, not on `Error.prototype` | J2.5 | |
| `Error.captureStackTrace` exists | J2.5 | |
| A page's own `Error.prepareStackTrace` is called | J2.5 | |
| Whether `prepareStackTrace` is an own property of `Error` by default | J2.5 | still being probed |

## Error messages

| Observed | Step | Test |
|---|---|---|
| `Cannot read properties of null (reading 'length')` | J2.4 | |
| `Cannot read properties of undefined (reading 'length')` | J2.4 | |
| `Cannot set properties of null (setting ...)`, and of `undefined` | J2.4 | |
| `o.f is not a function`: the callee's expression text as written | J2.4 | |
| `... is not a constructor`: the expression text as written | J2.4 | |
| `Assignment to constant variable.` | J2.4 | |
| `Cannot convert a Symbol value to a string` | J2.4 | |
| `Cannot mix BigInt and other types, use explicit conversions` | J4 (BigInt) | |

The setter message's tail after `setting` is to be confirmed by the
probe before its test is written.

## Function.prototype.toString

| Observed | Step | Test |
|---|---|---|
| Natives, getters and the embedder's methods: `function name() { [native code] }` | J2.5 | |
| Bound functions: `function () { [native code] }` (not `function bound f()`) | J2.5 | |

## Own-key order of built-ins

| Observed | Step | Test |
|---|---|---|
| `Error.prototype`: `constructor,name,message,toString` | J2.5 | |
| `Function.prototype` has no `fileName`, `lineNumber` or `columnNumber` | J2.5 | |

`Object.prototype`, `Math` and the global object differ from other
engines' too. Their lists are recorded here when J2.5 reaches them,
probed on the same page (a copy asked of the web-engine session).
