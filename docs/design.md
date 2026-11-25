# Design note: SilverJS, the Silver Suite's own JavaScript engine

Status: approved 2026-11-20 with the lead's amendments (stale handles
fail loudly, section 1.5; SilverJS alone first and J0 after G90 phase
3, section 3; builds under the quiet-machine lock, section 3; benchmark
workloads' licences recorded, section 1.8). The engine replaces
QuickJS-ng in Silver Browser only after Silver's beta (the user's wish
and the lead's order, 2026-11-20). The beta work comes first
throughout.

Written as "argent", a crate of Silver Browser's; the same day the
engine was given a repository of its own,
github.com/IAmForeverAloneToo/SilverJS, under the Mozilla Public
License 2.0, and renamed SilverJS. This copy says so where the move
changes a path, the licence or how Silver Browser takes the engine in
(sections 1.1, 1.5, 1.10, 3 and 4); every decision is as approved.
"Silver" below is Silver Browser, the first embedder, and `crates/...`
paths are Silver Browser's.

Hard constraint (the user's): the engine is Silver's own. No code from,
and no adoption of, V8, SpiderMonkey, JavaScriptCore, QuickJS, Boa,
Hermes or any other JavaScript engine. What it is built from: ECMA-262
(2026, the 17th edition, the version `scripting.md` targets) and
ECMA-402 for the parts that touch it, Test262, the Unicode Character
Database, and published papers, cited where a technique comes from
one. QuickJS-ng is used only as a black box: the backend it replaces,
and the oracle the differential runs compare against.

## 0. Where it stands

`js` (M24, D1) is a leaf crate whose public face is the engine's own:
`ScriptEngine`, `Realm`, `ScriptingHost`, `Task`, `Limits`,
`ScriptError` and the plain data around them (`crates/js/src/lib.rs`).
Behind it, `quickjs/` reaches QuickJS-ng 0.16 through `rquickjs`
0.14:

- an `Agent` (G90) owns the runtime: one heap under `Limits::heap`
  (256 MiB for a top-level page), one job queue, one interrupt handler
  that counts steps (`MAX_SCRIPT_STEPS`, each ~10,000 operations) and
  reads a worker's `stop` flag, the promise rejection tracker, the
  promise hook and the module loader;
- a realm is a context of that runtime, with its own global, its own
  bootstrap (`bootstrap.js`, ~34k lines, plus seven chunks read at
  first use, D64) and ~390 natives (`native.*`) that turn arguments
  into a `Call` of the host and its `Answer` back into values;
- the host is moved into a slot for the length of a run (`Slot<H>`),
  and lent to the agent's other realms (G90 phase 2);
- WebAssembly runs in `wasmi`, beside the realm; a memory's bytes are
  mirrored into a script's `ArrayBuffer` and copied back (G79 is the
  zero-copy fix, beta step 4);
- `Intl` is ICU4X under natives (`intl*.rs`), not QuickJS's;
- the sampler (`js-sampler`, a tool) reads script URLs off the stack
  through raw QuickJS calls.

What QuickJS-ng costs Silver, from the repo's own record:

- *Unsafe and C.* Four of the js crate's five modules that allow
  `unsafe` exist because of QuickJS (`bytecode.rs`, `sampler.rs`,
  `wasm.rs`, `context.rs`), and `rquickjs-sys` brings 6,098 extern
  items of C that every page's script reaches (`security.md` 5).
- *The native stack.* A script's call is a C frame of ~3.5 KiB, so
  recursion depth is the thread's stack: G253's 16 MiB script stack,
  `NATIVE_STACK` under it, the realm refused on a thread without both,
  and `MAX_MODULE_DEPTH` because linking recurses.
- *Errors as text.* Stacks are parsed back out of strings
  (`first_frame`, `parse_stack`).
- *Copies at every edge.* Bytes cross as base64 (`wasm.rs`), and a
  wasm memory is a mirror, because `rquickjs` lends no buffer safely.
- *Realm cost.* The bootstrap is compiled per realm (30 ms) unless read
  from QuickJS bytecode (5 ms, M35.12), whose reader trusts its input.
- *Reference counting.* Cycles through the DOM's wrappers need D70's
  sweep of tree groups beside the cycle collector.

The new engine is designed against that list.

## 1. Decisions

### 1.1 Where it lives, and how it runs beside QuickJS-ng

**A crate in a repository of its own: `silverjs`, at the root of the
SilverJS repository.** It depends on no crate of Silver's and, to
start, on nothing outside `std` (Unicode tables are generated, below).
Silver Browser takes it in as an external git dependency of its `js`
crate, pinned to a commit or a tag of this repository (never a branch),
and its layer list in `Cargo.toml` gains one line: `js -> silverjs (an
external leaf: the JavaScript engine)`. Nothing but `js` depends on it:
`engine` keeps talking to `js`'s traits only. A new engine version
reaches Silver Browser when the pin is moved, in a commit of Silver
Browser's that runs its gate.

Why a crate of its own rather than a module of `js`: the standalone
rule (section 1.10) needs the boundary to be a crate boundary, which
the compiler enforces, and now a repository boundary, which nothing
crosses by accident; and `js` stays the place where everything
browser-shaped lives, for both backends.

**The repository's layout.** The engine crate is the repository's root
and its own workspace, so that `cargo test`, `cargo run --bin test262`
and `cargo bench` at the root are the whole engine's: `src/`, the
Test262 runner as a binary of the crate (`src/bin/test262.rs`, its
expectations under `test262/`), `tests/`, `benches/`, `examples/` and
`tools/` (the Unicode generator, the sabotage scripts). The fuzz
targets are `fuzz/`, cargo-fuzz's crate on the nightly toolchain,
excluded from the workspace. A second crate, if one is ever wanted,
makes the root a workspace with members, in a commit of its own.

**Two backends behind `js`'s existing API.**

- `crates/js/src/quickjs/` stays as it is.
- `crates/js/src/silverjs/` (new) is the second backend: `SilverJsAgent`,
  `SilverJsRealm`, and the natives written against SilverJS's embedding
  API (section 1.10). It implements the same `ScriptEngine` and
  `Realm` traits.
- The bootstrap and its chunks move to `crates/js/src/bindings/`: they
  are JavaScript and engine-neutral (they use `__proto__`,
  `Error.stackTraceLimit`, `Error.prepareStackTrace`,
  `Uint8Array.prototype.toBase64` and `fromBase64`, all of which SilverJS
  implements). The move is done in phase J5, at a quiet point agreed
  with the web-engine session, since it touches files that session
  edits.
- The ~390 natives are ported one by one into `silverjs/`. The logic
  under them (`dom.rs`'s `Call`/`Answer`, ICU4X's formatting, `wasmi`'s
  store) is already backend-neutral Rust and is shared; only the glue
  that reads arguments and makes values is written twice, and only
  until QuickJS-ng is removed (J8).

**The switch, per build and per run.**

- Per build: `js` features `quickjs` and `silverjs`. Both on in debug
  and test builds from J5; the release build keeps `quickjs` alone
  until J7, then both, then `silverjs` alone after J8.
- Per run: `SILVER_JS_ENGINE=quickjs|silverjs`, read once per process,
  and a `--js-engine` flag the browser passes to its renderers and
  worker processes (env would inherit, but the flag makes the choice
  visible in the process list and crash reports). One process runs one
  backend: realms of one agent share a heap, so a tab cannot mix them.
- In the code: `js::Agent<H>` and `js::WeakAgent<H>` become enums over
  the two backends' agents; `js::engine()` returns the chosen
  `ScriptEngine`, and the three places `engine` names `js::QuickJs`
  (`scripts/mod.rs`, `lib.rs`'s `agent_for`, the `scripts` bench) call
  it instead. That small change (J0) lands with QuickJS as the only
  variant, so the later switch touches no `engine` code; it waits until
  G90 phase 3 closes, since G90 is changing `js` now (section 3).
- The switch lives in Silver Browser's `js` crate alone: SilverJS
  knows nothing of QuickJS-ng or of the switch. The `silverjs` feature
  of `js` is what adds the pinned dependency, so a build with `quickjs`
  alone neither fetches nor compiles SilverJS.

**Differential testing** runs through the same switch: the js crate's
tests (about 8,000 lines over `tests.rs`, `dom_tests.rs`,
`event_tests.rs`, `module_tests.rs`, `wasm_tests.rs`, `intl_tests.rs`,
...) are parameterized by backend (`for_each_backend!`), and a
`differential` test mode runs one script on both with a recording host
and compares section 1.7's outputs.

### 1.2 Execution: a register bytecode interpreter first

**Pipeline.** Source -> lexer -> parser -> AST (in an arena, freed
after compiling) -> scope analysis -> bytecode. The compiler's output
is **independent of any heap**: bytecode, a constant pool of literal
strings and numbers, scope descriptors and source positions, with no
heap references. It is `Send`, so compiling can move off the main
thread later (a script parsed as it streams in), and it serializes, so
the bindings' compiled form is cached without trusting a reader
(section 1.5). A compiled unit is *instantiated* into an agent's heap
in one step that interns its strings.

**Lazy compiling.** Inner functions are pre-parsed (syntax and early
errors checked, their range and free names recorded) and compiled at
their first call. Pages ship megabytes of script of which a fraction
runs at load; the bootstrap is 34k lines of which a page uses some.

**A register machine with an accumulator, not a stack machine.**

- Instructions name their operands: registers are slots of the call's
  frame (locals, temporaries, arguments), and one implicit accumulator
  holds the result of the last operation. `a = b + c` is two
  instructions (`Load b; Add c`, then `Store a`) where a stack machine
  takes four (`push b; push c; add; pop a`).
- Shi, Gregg, Beatty and Ertl ("Virtual machine showdown: stack versus
  registers", VEE 2005, and ACM TACO 2008) measured register
  bytecode executing about 47% fewer instructions for about 25% more
  code; in an interpreter written as a Rust `match` loop, where each
  dispatch is an indirect branch and Rust has no computed goto, fewer
  dispatches is what matters most.
- The accumulator keeps most operands implicit, which takes back most
  of the code size: operands are 8 bits by default, with `Wide` and
  `ExtraWide` prefixes for 16 and 32.
- Every instruction that reads a property, calls or makes an object
  carries an inline-cache slot index (section 1.3), which a register
  form gives a natural place for.
- A register frame is what a baseline JIT wants: each register is a
  fixed slot, so compiled and interpreted frames have one layout and a
  loop can move between tiers at its header without translation.

**Frames live on the agent's own stack, not Rust's.** A JavaScript
call pushes a frame onto a `Vec`-backed register file and loops; it
does not recurse in Rust. Recursion depth becomes a number the engine
chooses (a `RangeError: Maximum call stack size exceeded` past it, the
reference browser's wording and roughly its depth, ~10,000 frames),
not a property of the thread. Native re-entry (JavaScript calls a
native that calls JavaScript: `sort`'s comparator, a getter, the
host) does use the Rust stack; it is bounded by a re-entry count
(default 256) the embedder can lower and an optional embedder callback
for the stack left (Silver passes `js::remaining_stack`). The parser is
bounded by a nesting limit (a `SyntaxError` past it), and the GC marks
with an explicit stack. **Rule: no algorithm in SilverJS recurses on the
Rust stack without a bound.** This retires G253's 16 MiB script stack
and `MAX_MODULE_DEPTH`'s reason.

**The budget.** A counter is charged at backward jumps, calls, and
inside long natives (per element in `sort`, per backtrack step in a
regexp, per chunk of a string or array operation); every 10,000 the
embedder's interrupt hook is called, and its `true` terminates the run
with an uncatchable termination that unwinds without running any
`catch` or `finally` (no page code runs after a stop). `Limits::steps`
keeps its meaning; the numbers differ between backends, so
differential runs never compare budgets.

**A later baseline JIT** (after J8, a note of its own): a template
compiler from bytecode to x86-64, one machine sequence per
instruction, reading the same IC slots, the same frame layout. It needs
writable-then-executable memory in the renderer, which G112's planned
"dynamic code" mitigation forbids; so the JIT waits for the sandbox
decision, is off in any process under that mitigation, and is where
SilverJS's first real `unsafe` lives (section 1.5). The interpreter is
designed so that adding it changes no data structure.

### 1.3 The object model

**Values: NaN-boxed, 64 bits.** A value is a `u64`. Every bit pattern
that is not one of the reserved quiet NaNs is a double; the reserved
NaN space carries a tag and a 32-bit payload: int32, the singletons
(undefined, null, true, false, the array hole, the uninitialized
binding), and references (object, string, symbol, BigInt) as 32-bit
heap indices (below). Reasons:

- doubles are never allocated: canvas math, layout reads, animation
  timing and `Math` stay off the heap, which an interpreter without
  escape analysis cannot otherwise avoid (Gudeman, "Representing type
  information in dynamically typed languages", 1993, surveys the
  options);
- int32 has its own tag, so bitwise operations, array indices and loop
  counters skip float conversion; `-0` is always a double;
- one word per value keeps objects and arrays dense, and arrays need no
  separate unboxed-double representation.

Pointer tagging with boxed doubles (31-bit small integers, everything
else on the heap) was rejected: it allocates for every non-integer
number, which only a JIT recovers. Pointer compression is had anyway:
references are 32-bit indices.

**NaN canonicalization is a safety rule.** Every double that enters
from bytes (`Float64Array`, `DataView`, a wasm `f64`, the host, the
deserializer) is canonicalized before it becomes a value, so no page
can forge a tagged reference. With the heap below, a forged reference
would still not be memory-unsafe, but it would break invariants; the
fuzzers check it.

**The heap is index-based.** Objects, strings, shapes, environments,
compiled functions and the rest live in typed arenas, segmented
vectors of fixed-size cells, and are named by `u32` indices, not by
pointers. This is the decision that decides section 1.5: the GC, the
value representation and the object model are written in safe Rust. A
reference is an index checked against its arena's length; a stale
index (a rooting bug) reads the wrong cell of the right type, which is
a correctness bug the debug build catches (each cell carries a
generation, and every handle checks it), not memory corruption. Also:
indices are 4 bytes where pointers are 8, cells never move so natives
need no relocation, and a heap of indices serializes without
relocation, which makes realm snapshots (J6) straightforward. The cost
is a bounds check per access, a predictable branch; J6's benches
measure it.

**Shapes (hidden classes).** An ordinary object has a shape: its
prototype, its property keys in order, each key's attributes and slot
number. Adding a property follows a transition from the current shape
to a child shape, shared by every object built the same way, so that
objects of one constructor share one shape chain (Chambers, Ungar and
Lee, "An efficient implementation of SELF", OOPSLA 1989). Shapes are
per agent, shared by its realms; since a shape holds its prototype, the
realms' different `Object.prototype`s give them separate trees without
a realm field. An object leaves the shared tree for **dictionary mode**
(a hash map from key to slot and attributes, in the object) when it is
used as a map: more than 64 named properties, a `delete` of a property
that is not the last added, or many attribute changes. The global
object is always a dictionary of **property cells** (one cell per
global binding, which ICs hold directly).

**Property storage.** Four inline slots in the object's cell (the size
most objects never outgrow), then an out-of-line slot vector in a
size-classed value space, grown by doubling. Indexed properties are
kept apart from named ones, in **elements**:

- *packed*: a dense vector of values, no holes;
- *holey*: a dense vector where the hole value marks absent elements
  (a read of one walks the prototype chain);
- *sparse*: a sorted map from index to value and attributes, entered
  when an array is mostly holes (more than 1,024 elements and under a
  quarter of them present), when an element gets non-default
  attributes, or for `length` near 2^32.

An array moves only from packed toward sparse, never back, so a site's
ICs see few kinds. Typed arrays read their buffer directly; there is
no double-elements kind because a NaN-boxed value already is a double.

**Inline caches.** Each property read, write, `in`, call and
constructor site has an IC slot in its function's feedback table
(Deutsch and Schiffman, "Efficient implementation of the Smalltalk-80
system", POPL 1984; Hoelzle, Chambers and Ungar, "Optimizing
dynamically-typed object-oriented languages with polymorphic inline
caches", ECOOP 1991):

- uninitialized -> monomorphic (one shape: the slot, or the holder's
  shape for a prototype hit) -> polymorphic (up to four shapes) ->
  megamorphic (the agent's global cache keyed by shape and key);
- a hit on a prototype is guarded by a *validity cell* on the
  prototype chain's shapes, invalidated when any prototype on it
  changes shape, so a hit checks one shape and one cell;
- global reads cache the property cell; `length` of arrays and
  strings, and typed-array elements, have fixed fast paths;
- IC slots are what the baseline JIT reads later, and their counts are
  what J6's benches report.

**Exotic and host objects.** Arrays, strings, arguments (mapped and
unmapped), typed arrays, module namespaces, bound functions and
`Proxy` follow the spec's exotic behaviour as variants of the object
cell's kind. The embedder gets: classes with a Rust payload, natives,
and **native interceptors** (get, set, has, delete, own keys, define,
get-own-descriptor, in Rust), which a WindowProxy, a named-properties
object or `HTMLCollection` can use instead of a `Proxy` in the
bootstrap. `document.all`'s `[[IsHTMLDDA]]` (Annex B) is a flag a
class can set.

**Strings.** Immutable, with WTF-16 semantics (lone surrogates are
values, as G78 needs):

- *flat Latin-1* (one byte per unit) when every unit is under 256,
  which is most of the web's text and halves its memory; *flat UTF-16*
  otherwise;
- *shared buffers* for building strings (J2.4b, the lead's ruling of
  2026-11-20, in place of ropes there): a string of 64 units or more
  made by `a + b` is a window on a buffer it shares with the strings it
  was built from, which keeps the range its windows have used and room
  at both ends, grown by doubling. `a + b` writes `b` in place when
  `a`'s window ends exactly at the used range's end, or `a` when `b`'s
  starts at its start (the shorter written where both could), and
  copies otherwise. Appending, prepending, both alternating, and
  `a + (b + (c + ...))` copy each unit a constant number of times,
  amortized, measured by bytes copied in the tests; every string stays
  flat to read, so there is no depth and nothing to flatten. A buffer is
  counted once, by its newest window; the collector copies out the live
  windows of a buffer they cover under a quarter of, so a short string
  never keeps a long one's buffer (tested at 10 MB);
- *ropes* (Boehm, Atkinson and Plass, "Ropes: an alternative to
  strings", 1995) remain J6's, for what buffers do not cover: branching
  off one prefix (`a = p + "x"; b = p + "y"`, in a loop) copies `p`
  each time and stays quadratic until then, a known gap the micro-bench
  `string_branch` measures (51x QuickJS-ng's time at J2.4b);
- no substring slices sharing a parent's buffer: they keep large
  strings alive by a small piece, a memory leak pages make easily; a
  measured need in J6 may reopen it;
- property keys are **atoms**: interned strings with an agent-wide
  `u32` id, so shapes and ICs compare integers; integer-like keys are
  canonical indices, never atoms;
- hashes are cached in the string cell.

**Numbers' text.** `Number.prototype.toString` and `parseFloat` are
built on Rust's `core`, whose `f64` formatting gives the shortest
round-tripping digits and whose parser is correctly rounded (`core` is
the Rust language's library, not an engine); the spec's formatting
rules are SilverJS's own. `toFixed`, `toPrecision`, `toExponential` and
radix conversion are SilverJS's, exact by big-integer arithmetic.
BigInt is SilverJS's own arbitrary-precision integer (schoolbook, then
Karatsuba past a measured size).

### 1.4 The garbage collector

**Precise, tracing, non-moving mark-sweep, per agent.** Roots are
known exactly: the register file, the handle stack (below), global
handles, the job queue, the realms' records, the compiled code's
constants, and the embedder's roots callback. No conservative
scanning, no reference counting. Cycles, including those through the
DOM's wrappers, are collected as any other garbage, which is D70's
problem dissolving rather than needing a sweep of its own.

- *Marking* is iterative with an explicit mark stack, a bitmap per
  arena segment.
- *Sweeping* is lazy: each arena's free cells are found when it next
  needs one, so a collection's pause is the marking.
- *Non-moving*, because indices are stable names for natives, for
  embedder payloads and for a later JIT. Fragmentation is held by
  fixed-size cells per arena and size classes for slot vectors and
  string bytes; segments wholly free are returned. A freed cell's
  generation is bumped, so every handle to its old occupant is stale
  from then on (section 1.5).
- *A write barrier from the first day.* Every store of a reference
  goes through a handful of setters (safe Rust makes it hard to do
  otherwise); in J2 the barrier is empty but for debug checks. J6
  turns it on for **incremental marking** (snapshot-at-the-beginning,
  so marking runs in slices with the host's idle deadline) and for a
  **generational** mode without moving, by sticky mark bits (Demers,
  Weiser, Hayes, Boehm, Bobrow and Shenker, "Combining generational
  and conservative garbage collection", POPL 1990): a minor
  collection marks from the remembered set and the roots and sweeps
  only cells allocated since the last one.
- *Scheduling*: a collection when the heap has grown by a factor
  (1.5x, tuned in J6) since the last; when the embedder asks
  (`collect()`, as `Realm::collect` and `sweep` do now); and in idle
  slices when the embedder gives a deadline.

**Rooting for natives: scopes and handles, checked by the borrow
checker.** Native code never holds a raw value across anything that
can collect.

- An embedder enters an agent with `agent.enter(realm, |scope| ...)`.
  A `Scope<'s>` owns a frame of the agent's **handle stack**; a
  `Local<'s, T>` is an index into it, `Copy`, valid while the scope
  lives, and scanned by the GC. A native is given a scope for its
  call. Nested scopes free their handles on exit; `scope.escape(x)`
  moves one out.
- A `Global<T>` is a strong handle in the agent's root table, held by
  Rust structures across runs (what `rquickjs::Persistent` is today);
  a `Weak<T>` is the same without keeping the target.
- Anything that may collect takes `&mut` on the scope, so a raw value
  read before it cannot be used after it: the compiler refuses.
- An embedder payload that holds engine references implements a
  `Trace` trait (a visitor over its `Member<T>` fields), which the GC
  calls; payloads of plain data (a node's `Handle`, a number) need
  nothing.
- Handles that outlive a scope (`Global`, `Weak`, and the `Member`s of
  embedder payloads) carry the cell's generation and their agent's id,
  checked on every use, in release builds too (section 1.5).
- A **GC-stress** build (`gc-stress` feature) collects at every
  allocation and checks every `Local` too, so a rooting bug fails the
  first test that reaches it.

**Weak references and finalization.**

- `WeakMap` and `WeakSet` are ephemerons: after the strong marking,
  entries whose key is marked mark their value, to a fixed point; the
  rest are cleared.
- `WeakRef`: a target read by `deref()` is added to the agent's
  kept-alive set until the embedder calls `clear_kept_objects()` at
  the end of its microtask checkpoint (the spec's ClearKeptObjects,
  which HTML calls there).
- `FinalizationRegistry`: cells whose targets died are queued after a
  collection, and the embedder is told through the spec's host hook
  (HostEnqueueFinalizationRegistryCleanupJob); Silver queues a task,
  as HTML says, and the cleanup runs as a job of the realm's.
- The embedder gets the same machinery: weak handles, and a
  finalizer per class payload (run after the sweep, with no access to
  the heap), by which Silver learns that a node's wrapper died.
- **Host groups**: an object can be tied to a group key; the GC asks
  the embedder, during marking, which groups are live (a DOM tree with
  a node reachable from script). This is what keeps a detached
  subtree's other wrappers alive while any one of them is held, the
  case D70 handles today by hand.

**Heap limits per agent.** The agent counts its arena segments, slot
and string storage, and the external bytes it owns (`ArrayBuffer`
stores it allocated). An allocation that would pass the limit collects
first; still past it, it throws a catchable
`RangeError: Out of memory` (QuickJS's behaviour today, which `js`
relies on, `Death::Heap` decided above it by the same 3/4 rule).
Backing stores the embedder owns (a wasm memory, below) are the
embedder's to bound, as `MAX_WASM_MEMORY` bounds them now. Stats:
used, committed, per arena, external, and per realm by owner when a
debug option asks (for the "where did Reddit's memory go" question,
beta step 8, on the new engine).

**Realms share one heap.** An agent is one heap, one atom table, one
shape tree, one job queue, one budget, one interrupt hook. A realm is a
record in it: its intrinsics (one `Array`, one `Object.prototype`,
... per realm, so `frames[0].Array !== Array`), its global object, its
global environment and its embedder data. A function carries its
realm, which decides the intrinsics its errors and objects use; an
object of one realm passed to another is the same object. The API
gives the embedder the current realm and lets it run code in any realm
of the agent, which is all G90's WindowProxy, entry settings and
lending need. An agent is `!Send`: it lives on one thread, and a worker
has an agent of its own on its own. `SharedArrayBuffer` stays absent
from Silver's pages: SilverJS has it (J4.2b) but puts it on a global
object only when the agent's options ask, as a cross-origin isolated
page has it, and Silver does not ask; its bytes are an agent's own
until the backing-store design's `Arc`ed shared store is taken up with
cross-origin isolation. `Atomics` every page has.

**WebAssembly: zero-copy by construction.** An `ArrayBuffer`'s bytes
are either SilverJS's own or an **external store** given by the
embedder: a trait object that lends `&[u8]` and `&mut [u8]` on
request, and that the embedder can detach (memory growth detaches the
old buffer, as the spec says). Silver's backend makes a wasm memory's
buffer an external store over `wasmi`'s memory: typed arrays read and
write the module's bytes in place, with no mirror and no base64. The
store is not borrowed while JavaScript runs because a module calls a
script's function by `wasmi`'s resumable call, as now. G79 is done by
QuickJS's own means before beta; on SilverJS it needs nothing more.

### 1.5 Unsafe: none in the engine to start, and a fence for what comes

**Policy.** The crate's root says `#![forbid(unsafe_code)]`, and no
module allows it in J1 to J8. The note first said `deny`, so that a
reviewed module could allow it as `js` does; the lead accepted
`forbid`, which no module can loosen: the first module to need
`unsafe` (the JIT, after J8) turns the root to `deny` in a commit of
its own, with the fence below. The interpreter, the GC and the value
representation are safe Rust because of the index-based heap (section
1.3): NaN-boxing is bit manipulation of a `u64` and needs no `unsafe`
when its payloads are indices, not pointers. This is a deliberate
trade: a bounds check per heap access for an engine whose every
memory-safety bug would be a renderer compromise.

**Stale handles fail loudly: the defence against logic-level
use-after-free.** Safe Rust stops memory corruption, but an index kept
past its cell's death and resolved after the cell is reused would read
the wrong object of the right type, silently: a confusion bug, the
logic-level twin of a use-after-free. So:

- every cell carries a generation, bumped when it is freed;
- every handle that outlives a scope (`Global<T>`, `Weak<T>`, and the
  `Member<T>` references in embedder payloads) carries the generation
  of the cell it names and the id of the agent it belongs to. Each use
  checks both, **in release builds too**, and a mismatch panics (a
  `Weak` whose target died reads as empty, which is its meaning, not a
  mismatch). The panic ends the renderer through the crash path; a
  wrong object never reaches a page;
- `Local<'s, T>` handles skip the check outside `gc-stress` builds:
  the borrow checker already ties them to a scope the GC scans, so a
  stale `Local` needs a bug in the engine's own rooting, which
  `gc-stress` exists to find;
- raw values inside the heap (slots, elements, environments) are not
  handles and carry no generation: they are reached only by marking,
  so a reachable one names a live cell by the GC's own invariant, which
  the heap verifier checks after every collection in debug and
  `gc-stress` builds;
- the fuzz targets assert that no handle is ever resolved against
  another agent's heap: each target runs at least two agents and the
  check counts any cross-agent resolution as a failure, never a pass.

The check costs a load and a compare per use of a long-lived handle,
which natives use far less than `Local`s; J6's benches report it.

**Where unsafe may come later, and on what terms.**

1. *The baseline JIT* (after J8): executable memory, writing code into
   it, flipping its protection, and calling into it. Unavoidable; a
   module of its own (`jit/memory.rs`, `jit/entry.rs`), behind a
   feature off by default.
2. *Unchecked indexing in the interpreter's hottest paths*, only if
   J6's benches show more than 10% on the engine's suite for a named
   site, each one a module of its own whose `// SAFETY:` argument
   states the invariant that makes the index in bounds and where it is
   established.
3. *Nothing else*: no `unsafe` for strings, for the GC, for natives or
   for the API. A shared-memory `SharedArrayBuffer` would be the next
   candidate (atomics on shared bytes), argued when it is taken up.

**The fence**, the same as the workspace's:

- every `unsafe` block has its `// SAFETY:` argument just above it;
- a test in SilverJS itself (`tests/unsafe_fence.rs`, so it travels with
  the crate) lists the modules allowed and their block counts, and
  fails on any other; the shell's
  `unsafe_is_written_in_the_platform_module_alone_each_block_argued`
  gains SilverJS's modules the same way, and `security.md` section 5 a
  paragraph per module;
- a change to an allowed module gets the extended local check the lead
  set on 2026-11-13 (gate, quick benches, fuzz crate check, cross-target
  check) before it is pushed.

**Miri, ASan and fuzzing.**

- *Miri*: SilverJS's unit tests and a Test262 smoke subset (some 2,000
  tests, chosen to touch every arena and the GC) run under Miri on the
  nightly toolchain the fuzz crate already uses: once per phase while
  SilverJS has no `unsafe` (it still catches undefined behaviour in
  `std` uses and integer overflow panics), and on every change to an
  allowed module once it has any.
- *ASan*: the fuzz targets build with the address sanitizer, as all
  `fuzz/` targets do; with no `unsafe` in SilverJS it guards the
  embedding glue and `std`.
- *Fuzz targets* (those needing no QuickJS in this repository's
  `fuzz/`; those comparing with QuickJS-ng in Silver Browser's `fuzz/`,
  after J0):
  - `silverjs_parse`: any bytes as a script and as a module; no panic,
    bounded time, every result a value or a `SyntaxError`;
  - `silverjs_run`: any bytes run under small limits, as `script.rs`
    does for QuickJS: no panic, the run ends by completing, throwing,
    the budget or the heap limit, and the heap stays under its limit;
  - `silverjs_gen`: a structure-aware generator of programs (written for
    SilverJS, in the fuzz crate) that makes valid, deterministic
    programs, run on both backends through `js`'s switch, outputs
    compared (section 1.7);
  - `silverjs_regexp`: pattern, flags and input; no panic, budget-bound,
    and compared with QuickJS-ng's result;
  - `silverjs_gc`: any bytes run twice, in two agents, once as usual
    and once collecting at every allocation, or every 2nd, 8th or 64th
    as the first byte chooses (`Agent::set_stress_every`: most runs
    fast, a quarter exhaustive), the heap verifier after
    every collection (fuzz builds keep debug assertions): a collection
    is never observable, so the two must end the same (`Math.random`
    seeded alike). Generated programs join the bytes when
    `silverjs_gen` is written, and with them the check that every
    handle resolves in its own agent and a stale one panics rather
    than resolves;
  - `silverjs_buffers`: the bytes made a script of typed arrays (the
    BigInt arrays among them, from J4.3c) and
    data views over resizable, fixed and shared buffers whose
    methods, `Atomics` and the base64 decoders run while values'
    `valueOf`, getters and callbacks resize, detach and grow the
    buffers; no panic, only the spec's TypeErrors and RangeErrors (and
    a decoder's SyntaxErrors), a detached view empty, no view past
    its buffer (J4.2, the lead's condition);
  - `silverjs_bigint`: differential, the bytes made pairs of BigInts
    (both signs, zero, a byte, around a power of two, many limbs) and
    an operator each, every result checked against num-bigint, a
    test-only oracle: the arithmetic, bitwise and shift operators,
    `asIntN` and `asUintN` at the edge widths, comparisons, radix
    conversion both ways (J4.3b, the lead's condition);
  - `silverjs_date`: any text to `Date.parse` in UTC, a fixed zone and
    one with a daylight change: NaN or an integral time value, in
    bounded time, and the string forms reading back to the time; a
    string of the recorded table (the dates probe's corpus, symbols and
    spaces, Chrome 158) must give what the reference browser gave
    (J4.3d, the lead's condition);
  - `silverjs_bytecode`: any bytes to the compiled-unit reader, which
    must refuse what it did not write (a checksum and a structural
    validation), never panic, never run it.
- *Panics are bugs.* A panic in SilverJS ends the renderer, which the
  crash path already handles, so it is not an exploit; the fuzzers hold
  every target to no panic, and clippy's `indexing_slicing` and
  `unwrap_used` are denied outside tests so that each index is a
  checked `get` with an argued fallback or a documented invariant.

**Bytecode caching becomes safe.** M35.12's QuickJS reader trusts its
input, which is why reading the bindings' bytecode is `unsafe` today.
SilverJS's reader validates what it reads (a malformed unit is refused,
and even an accepted wrong unit cannot corrupt memory), so the
bindings' compiled form, and later a page's cached scripts, need no
exception to the policy.

### 1.6 RegExp: SilverJS's own engine

**A backtracking compiler and VM.** ECMAScript's RegExp semantics are
defined by backtracking (ordered alternation, backreferences,
lookbehind matched backwards, lookahead, lazy quantifiers), so a
backtracking matcher is the direct implementation. A pattern is
parsed (with the `u` and `v` flags' grammars, named and duplicate named
groups, modifiers like `(?i:...)`, and Annex B's legacy syntax for
non-Unicode patterns), compiled to regexp bytecode, and run on a VM
whose backtrack stack is a heap vector, not Rust's stack. Case folding
and the property escapes (`\p{Script=Greek}`, the `v` flag's string
properties) use tables generated from the Unicode Character Database.

**The ReDoS stance: bounded by the budget, made linear where it can
be, never failed for being slow.**

- Every VM step is charged to the agent's step budget, so a
  catastrophic pattern on a long input is stopped by the same
  interrupt that stops an endless loop, with the same error, and a
  worker's `terminate()` stops it. This is what protects the browser.
- Patterns with no backreferences (almost all of them) get **selective
  memoization**: after a pattern has backtracked a set number of times
  on one input (100,000), the VM records the (instruction, position)
  pairs that failed at the loop and alternation points and never
  retries them, which bounds the match to the pattern's size times the
  input's (Davis, Servant and Lee, "Using selective memoization to
  defeat regular expression denial of service (ReDoS)", IEEE S&P 2021).
  Normal patterns never reach the threshold and pay nothing.
- No backtrack limit that throws: a page whose correct but slow regexp
  finishes in Chrome must finish in Silver; only the budget, which
  stops any script, stops it.
- Patterns are compiled once per source and flags and cached per agent.

A non-backtracking (automaton) matcher for backreference-free patterns
is a later option if J6's benches show the VM slow on real pages; the
memoization already removes the security reason for one.

### 1.7 Conformance

**A Test262 runner in SilverJS** (`cargo run --release --bin test262 --
<test262 dir>`), working from the SilverJS crate alone:

- it reads each test's front matter (includes, flags `onlyStrict`,
  `noStrict`, `module`, `raw`, `async`, `CanBlockIsFalse`, negative
  phase and type, features), runs it in a fresh agent with the harness
  includes, and provides `$262` (`createRealm`, `evalScript`,
  `detachArrayBuffer`, `gc`, `global`, `IsHTMLDDA`; not `agent`, which
  needs shared memory);
- tests run in parallel, one agent per thread, under a time limit
  each;
- the Test262 revision is pinned (a commit id in SilverJS), fetched like
  WPT under the standing download approval, never vendored;
- **an expectations file** (`test262/expected-failures.txt`)
  lists every failing test with a one-word reason; a run that fails a
  test not in it, or passes one in it, fails. Progress is a shrinking
  file and regressions are impossible to miss;
- the measured set is `test/language`, `test/built-ins` and
  `test/annexB`, minus `intl402` (ECMA-402 is Silver's, section 1.10),
  `staging`, and features the reference browser does not ship
  (`SharedArrayBuffer`, which a page has only when cross-origin
  isolated, and with it `Atomics.wait`; unshipped proposals), which
  are counted as not applicable, not as passes. `Atomics` itself every
  page has, and its tests count (J4.2b);
- a QuickJS-ng adapter in `js` (a test binary there, since QuickJS is
  Silver's side) runs the same set through the same front-matter
  reader (exported by SilverJS under its `test262` feature), so
  QuickJS-ng's own rate is the bar SilverJS is measured against, measured
  on the pinned revision as soon as J0 lands (section 3). Until the
  adapter exists (J5, which brings the dependency), a harness of
  silver-work's tooling measures it on the same revision and set
  (2026-11-20): 42,494 of 47,561, 89.35%; Temporal is 4,616 of its
  failures, and without Temporal it passes 42,494 of 42,945, 98.95%.
  The tests it passes are kept in `test262/quickjs-ng-passes.txt`, a
  fixed list;
- SilverJS's runner provides `$262.createRealm` (realms of one agent are
  the browser's frames), so the tests that use it, 185 of QuickJS-ng's
  failures for want of it there, count in SilverJS's set.

**Pass-rate targets** (of the measured set) per milestone in section
3: J1 99% of the parse-phase tests (every negative syntax test and the
parse of every positive one); J2 35%; J3 65%; J4 90% with Temporal
set aside (the lead, 2026-11-23: 38,619 of 42,910, the module tests
counted); J5 93%; J6 no regression (the expectations file only
shrinks). J7, before the
replacement is allowed (the lead, 2026-11-20), on the measured set with
Temporal set aside: at least QuickJS-ng's 98.95% in aggregate, and no
test of `test262/quickjs-ng-passes.txt` failing unless its failure is
recorded with a reason (QuickJS-ng passing by accident, or a deviation
chosen), so that the comparison is mechanical. Temporal has a milestone
of its own after J7 (J9, section 3).

**Differential runs against QuickJS-ng**, on the same scripts, through
`js`'s backend switch:

- *Outputs compared*: the completion value as `js::Completion`, every
  `ConsoleRecord`'s text, every `ScriptError`'s kind and message
  (normalized only where QuickJS's wording differs from the reference
  browser's, which SilverJS follows), the host `Call`s in order, and
  thrown or not.
- *Determinism*: in differential mode the host fixes `Date.now`,
  `Math.random`'s seed and the time zone, through SilverJS's host hooks
  and a QuickJS shim in the test host.
- *Corpora*: the js crate's test scripts (both backends must pass
  them), the fuzzers' generated programs, Test262 (rates compared per
  directory, so a directory where SilverJS trails is visible), and in J7
  the scripts the sweep keeps from the user's daily sites, run as
  pages through the whole engine with each backend, comparing the
  sweep's counters, console errors and screenshots.
- *Where they disagree*, the reference browser decides: a case where
  QuickJS-ng is wrong and SilverJS right becomes a test of SilverJS's that
  records Chrome's answer, as G90's tests record Chrome's.

**Web compatibility beyond ECMA-262**, because the reference browser
is the bar: error messages worded as the reference browser words them
(sites read `e.message`); `Error.prototype.stack` in the reference
browser's format, `Error.captureStackTrace` and `Error.stackTraceLimit`
(the bootstrap and many sites use them); the stack given to the
embedder as structured frames, so `js` no longer parses text; Annex B
in full; `Date.parse` accepting what the reference browser accepts.
Each is checked against Chrome's answer, never another engine's code.
Messages, stacks, `toString` and key order are section 1.11's, from
J2.4 on.

### 1.8 How memory and speed are measured against QuickJS-ng

The rules are the repo's: release builds, medians of at least ten
alternating runs (never two), a quiet machine, booked with the
web-engine session before any run, as its sweeps and benches need the
machine too.

- **SilverJS's own benches** (in SilverJS, standalone): property reads and
  writes (monomorphic, polymorphic, megamorphic), calls and closures,
  object and array allocation, string building and searching, regexp,
  JSON parse and stringify, `Map` and `Set`, promises and `async`
  churn, typed arrays, and the bootstrap's compile. Each reports time,
  allocations, peak heap and, from J6, IC hit rates and GC pauses.
- **The same workloads on both backends**: `engine/benches/scripts.rs`
  (a realm made, two million additions, a thousand console calls, the
  1,000-paragraph page) parameterized by backend, plus a new
  `js_engines` bench running SilverJS's bench programs through `js` on
  each. Reported as a ratio per row, as `perf-budgets.md` reports a
  milestone against the one before.
- **Memory compared at the process**, not by each engine's own count:
  QuickJS counts its `malloc`s and SilverJS its arenas, which are not the
  same quantity. The comparison is the renderer's private bytes
  (`K32GetProcessMemoryInfo`, as M35.1 reads it) for the same page,
  idle after load, with each backend; each engine's own heap stats are
  reported beside it for attribution.
- **Real pages**: the site sweep run with `--js-engine` each way on
  the user's daily sites: load time, renderer memory at idle, the
  sampler's script time per URL (SilverJS's sampler API, section 1.10),
  `idleCounters`, errors.
- **Realm creation**, which the bootstrap dominates: 5.3 ms today with
  QuickJS bytecode. Targets: at most that at J5 (SilverJS's compiled
  bindings), under 2 ms at J6 with a realm snapshot (the bootstrap's
  heap state after it runs, serialized once per process and copied
  into each new realm).
- **Published benchmark programs** (JetStream's workloads and the
  like) may be run as workloads, fetched at bench time and never
  vendored; they are inputs, not engine code (the lead's reading of
  the user's constraint: it is about engines, not test programs). Each
  workload's licence is recorded beside its fetch URL in the bench's
  list of workloads.
- **Replacement bar** (J7): on the daily sites, page load within 10%
  of QuickJS-ng's by median and renderer memory no higher; SilverJS's
  bench suite at least at parity, by geometric mean.

### 1.9 What the engine implements, and what it leaves to the embedder

SilverJS implements ECMA-262 2026 with Annex B, and the reference
browser's shipped stage-4 features as they come. It leaves to the
embedder, through hooks with defaults: ECMA-402 (`Intl`, and the
locale-sensitive methods' locale behaviour), the WebAssembly JS API,
the time zone and clock, `HostEnsureCanCompileStrings` (CSP's
`unsafe-eval`, Silver's `Call::CodeAllowed`), module loading, job
scheduling, and everything of the web platform.

### 1.10 Standalone use

The engine may move to a repository of its own after beta, for others
to use apart from Silver (the user's addition, 2026-11-20). So, from
the first commit:

**Boundaries.**

- `silverjs` depends on no Silver crate, and on nothing outside `std`
  until a dependency is argued in its own commit. It lives in a
  repository of its own (since 2026-11-20), so nothing of Silver
  Browser's can reach into it, and it builds and tests with nothing
  else checked out.
- Everything browser-shaped is on Silver's side, in `crates/js`,
  behind SilverJS's public embedding API: the DOM natives and their
  `Call`/`Answer` glue, the bootstrap and its chunks, the
  realms-per-agent policy (which documents share an agent, G90), the
  WindowProxy and lending, the budget's constants and `Limits`,
  `ScriptingHost`, `Task`, the console's formatting, structured clone,
  CSP's check on code from strings, the module map and fetching, Intl
  over ICU4X, WebAssembly over `wasmi`, the sampler (`js-sampler`), and
  the QuickJS-ng backend.
- SilverJS's tests, its Test262 runner and expectations, its benches,
  its unsafe fence test, its Unicode table generator, and the fuzz
  targets that need no QuickJS all live in this repository and run
  from it alone: `cargo test`, `cargo run --bin test262` and `cargo
  bench` at its root need nothing else. The differential runs, the
  QuickJS adapter and the site sweep are Silver's.
- The licence is the Mozilla Public License 2.0 (the lead's choice for
  the standalone repository, 2026-11-20): an embedder may be under any
  licence, open or closed, and changes to SilverJS's own files are
  shared under the same terms. Silver Browser (AGPL-3.0-only) takes it
  in as an MPL-2.0 dependency, which its `cargo deny` allows.

**What a non-Silver embedder calls.** The whole of it, in outline:

```rust
use silverjs::{Agent, AgentOptions, HostHooks, RealmOptions};

struct Hooks; // every hook has a default: no modules, UTC, no limits
impl HostHooks for Hooks {}

let mut agent = Agent::new(AgentOptions { heap_limit: 64 << 20, ..Default::default() }, Hooks)?;
let realm = agent.new_realm(RealmOptions::default())?;
agent.enter(realm, |scope| {
    let print = scope.native_function("print", 1, |cx| {
        println!("{}", cx.arg(0).to_string(cx)?);
        Ok(cx.undefined())
    })?;
    scope.global().set(scope, "print", print)?;
    let script = scope.compile_script("print(6 * 7)", "main.js")?;
    script.run(scope)?;
    Ok(())
})?;
agent.run_jobs()?; // the microtask checkpoint, when the embedder chooses
```

- `Agent`: `new(options, hooks)`, `new_realm`, `enter(realm, f)`,
  `run_jobs`, `clear_kept_objects`, `collect`, `collect_idle(deadline)`,
  `heap_stats`, `terminate_handle()` (an `Arc<AtomicBool>`-like handle
  another thread may set, a worker's `terminate()`), `set_limits`.
- `heap_stats()` reads in constant time, cheap enough for every call
  (the lead, 2026-11-20, by J5): `bytes`, the heap in use (live cells and
  what they own), and `committed`, what the heap has taken from the
  allocator (arenas at capacity, cells dead but not yet swept, the
  register file), from counts kept as cells come and go, never a walk.
  Silver's `performance.memory` reads them live and unrounded, as
  Chrome's does; QuickJS-ng can answer only with a heap walk, so the
  browser waits for SilverJS here. A test holds a read on a heap of
  300,000 objects to the time of one on an empty heap.
- `Scope`: values in and out (numbers, strings, objects, arrays,
  `ArrayBuffer`s with external stores), property get, set, define and
  delete, calls and `new`, `compile_script`, the module API, natives
  and classes, exceptions (throw, catch, and an exception's structured
  stack frames), `escape`.
- Natives and classes: `native_function(name, length, f)`; a `Class`
  builder with a Rust payload (`T: Trace`), methods, accessors,
  interceptors, a finalizer and the `IsHTMLDDA` flag.
- Handles: `Local<'s, T>`, `Global<T>`, `Weak<T>`.
- Modules: parse a source-text module (its requests come back as
  specifiers with import attributes), link with the loader hook giving
  modules asynchronously, evaluate (with top-level await, a promise),
  `import.meta` filled by a hook, `import()` through the same loader,
  and synthetic modules with given exports (Silver's CSS and text
  modules).
- `HostHooks`: the spec's host hooks (`HostLoadImportedModule`,
  `HostPromiseRejectionTracker`, `HostEnqueuePromiseJob` when the
  embedder wants to own scheduling, `HostMakeJobCallback` and
  `HostCallJobCallback` for incumbent and active-script tracking,
  `HostGetImportMetaProperties`, `HostEnsureCanCompileStrings`,
  `HostEnqueueFinalizationRegistryCleanupJob`,
  `HostHasSourceTextAvailable`, which Silver uses so a binding's
  `toString` says native code), plus the interrupt hook, the clock, the
  local time zone offset, the random seed, and the native-stack check.
- Tools: `serialize_compiled` and `load_compiled` (validated), realm
  snapshots (J6), and `sample_stack(&mut Vec<FrameInfo>)` (script URL,
  function name, line and column, read without anything a page can
  observe), which Silver's sampler calls from its interrupt hook.

A small command-line shell, `examples/shell.rs`, is the
reference embedder: the Test262 runner is built on the same API and
nothing private, which keeps the API sufficient for others.

### 1.11 Chrome-compatible surface

Added 2026-11-20 at the lead's request, from what a site's bot check
taught the web-engine session. Silver presents itself as Chrome, so
what its engine shows a page is part of compatibility: bot checks
compare it with what Chrome's engine shows. SilverJS therefore matches
the reference browser, from J2.4 and J2.5 on (not J7, as section 1.7
first had it), on:

- *error messages*: the texts of the errors SilverJS throws follow the
  reference browser's wording, for example `Cannot read properties of
  null (reading 'length')`;
- *`Error.prototype.stack`*: its format, frame by frame, including the
  frames of `eval` and `new Function` code (`at eval (eval at f
  (url:1:2), <anonymous>:3:41)`);
- *`Error.captureStackTrace`, `Error.prepareStackTrace` and
  `Error.stackTraceLimit`*: present, and behaving as they do there;
- *`Function.prototype.toString`*: its output for natives, bound
  functions and classes;
- *the own-key order of built-ins*: each intrinsic's properties are
  defined in the order the reference browser's appear in
  (`Object.getOwnPropertyNames(Array.prototype)` and the like).

ECMA-262 leaves all of these implementation-defined, so matching costs
nothing in conformance. There is no switch: an embedder other than
Silver gets the same behaviour.

How it is done, within the hard constraint: every text, format and
order is taken from the reference browser's observed output (the
web-engine session's probe table, forwarded by the lead), never from
another engine's source. Each matched output is pinned by a SilverJS
test that records Chrome's answer, as section 1.7's differential rule
says; `docs/chrome-surface.md` lists what was observed and the test
that pins each row. Messages live in one table in SilverJS, so the wording is
reviewed in one place; built-ins are installed from ordered tables, so
their key order is data, not an accident of the code.

## 2. Measures

**Memory.** Expected lower than QuickJS-ng per object: 4-byte
references inside the heap, four inline slots, Latin-1 strings and
shared shapes against QuickJS's 8-byte pointers and refcount headers.
Not claimed until J6 measures it at the process (section 1.8); the
index heap's segment granularity adds a fixed cost per agent that
small frames' agents feel.

**Speed.** The interpreter alone (J5) is expected near QuickJS-ng,
which is a well-tuned interpreter without inline caches; J6's shapes,
ICs, ropes and lazy compiling are where SilverJS should pass it on
property-heavy code. A JIT is not part of the replacement bar.

**Size.** Roughly 80,000 to 120,000 lines of Rust for J1 to J7,
SilverJS and its tests together, plus the second backend's glue in `js`
(about the size of `quickjs/` without `intl*`, ~5,000 lines).

## 3. Phases, estimates and tests

Each phase ends with its tests green, its Test262 expectations file
updated, and the gate green. Estimates are in weeks of one session's
work, in parallel with the beta and below it in priority; they are
rough.

**Order and coupling.** G90 is changing `crates/js` heavily now, so
SilverJS starts in its own repository and touches nothing of Silver
Browser's: J1 to J4 are this repository alone, and change no file of
`js`, `engine` or Silver Browser's `fuzz/`.
J0, the backend switch in `js` and the three call sites in `engine`,
waits until G90 phase 3 closes, which the lead announces; it then
lands in whatever gap the web-engine session's pushes leave, before or
during J2 to J4. Everything that needs both backends (the QuickJS
adapter's Test262 bar, the differential mode, the `silverjs_gen` and
`silverjs_regexp` comparisons with QuickJS-ng) comes after J0. Until
then SilverJS's targets are absolute rates, and the QuickJS bar is
measured as soon as J0 lands.

**Machine time.** The machine is shared with the web-engine session,
whose benches and sweeps need it quiet. The quiet-machine lock exists
(silver-work's `quiet.sh`, 2026-11-20): every SilverJS build first
runs `quiet.sh wait`, its gate runs under `quiet.sh run`, and SilverJS
never takes
the lock for a long run (a full Test262 run, a fuzz campaign, a bench)
while that session's gate or sweep holds it. A docs-only commit runs
no gate while that session is benching; its next gated push takes it.

- **J1. Syntax** (3 to 4 weeks, SilverJS alone): the crate,
  lexer, parser, early errors, scope analysis, the Unicode table
  generator, the Test262 runner with front matter and the expectations
  file. Target: 99% of parse-phase tests. Fuzz: `silverjs_parse`, in
  this repository's `fuzz/`.
- **J0. The switch, once G90 phase 3 closes** (2 to 3 days, in a gap
  between the web-engine session's pushes): `js::engine()`,
  `js::Agent<H>` as an enum with QuickJS its only variant, the three
  call sites in `engine` moved to it, the `quickjs`/`silverjs` features
  and the env and flag plumbing with only `quickjs` accepted; then the
  QuickJS adapter for Test262, which measures the bar. Silver Browser
  takes SilverJS in as a git dependency pinned to a commit or tag
  (section 1.1), behind the `silverjs` feature, from J5. Tests:
  everything passes unchanged; an unknown engine name is refused.
- **J2. Runs Test262's basics** (5 to 6 weeks): values, the arenas
  and mark-sweep, handles and scopes, the register bytecode compiler
  and interpreter, functions and closures, objects in dictionary mode,
  `Object`, `Function`, `Error`, basic `Array`, `String`, `Number`,
  `Boolean`, `Symbol`, exceptions, strict mode, the budget and
  termination, the heap limit, generations and agent ids on handles.
  Target: 35%. Fuzz: `silverjs_run`, `silverjs_gc` (SilverJS's own targets,
  as J1's).
- **J3. The whole language** (5 to 6 weeks): generators, `async` and
  `await`, classes with private names and static blocks,
  destructuring, spread, iterators, getters and setters, tagged
  templates, `Proxy` and `Reflect`, direct and indirect `eval`, `with`,
  labels, Annex B's syntax and semantics. Target: 65%.
- **J4. The built-ins** (7 to 9 weeks, of which RegExp 3): all of
  `String`, `Number`'s formatting, `Math`, `Date` with parsing, `JSON`,
  `Map`, `Set`, `WeakMap`, `WeakSet`, `WeakRef`,
  `FinalizationRegistry`, `Promise` and the job queue, `ArrayBuffer`
  (resizable, external stores), typed arrays, `DataView`, BigInt, the
  RegExp engine with memoization, iterator helpers, `Set` methods,
  `Array` in full, `Uint8Array` base64. Target: 90%. Fuzz:
  `silverjs_regexp`, against QuickJS-ng once J0 has landed.
- **J5. Embedded in Silver** (5 to 6 weeks): modules with top-level
  await and synthetic modules, the full embedding API, the bindings
  moved to `js/src/bindings/`, the SilverJS backend in `js` with every
  native ported, Intl and wasm (zero-copy external stores) on it,
  compiled bindings cached, the differential mode, `silverjs_gen` and
  `silverjs_bytecode`. Targets: Test262 93%; every js crate test passing
  on both backends; a realm made in at most QuickJS's 5.3 ms.
- **J6. Fast and small** (6 to 8 weeks): shapes and transitions,
  inline caches, elements kinds, ropes and atoms tuned, lazy
  compiling, the write barrier turned on with incremental marking and
  the sticky-mark-bit generational mode, realm snapshots, and SilverJS's
  bench suite. Targets: no Test262 regression; SilverJS's suite at
  parity or better than QuickJS-ng by geometric mean; a realm under 2
  ms; renderer memory no higher on the sweep's pages.
- **J7. The user's daily sites** (3 to 4 weeks): the sweep each way
  on the daily list, the gaps left in Chrome-worded messages and stacks
  (matched from J2.4 on, section 1.11), Annex B and
  `Date.parse` gaps, WPT's JavaScript-facing directories at QuickJS's
  pass counts or better, an hour's dogfood session on SilverJS without a
  crash. Targets: Test262 as section 1.7 sets it (Temporal aside, at
  least 98.95%, and no test of QuickJS-ng's pass list failing without a
  recorded reason); section 1.8's replacement bar met.
- **J8. The replacement, after beta** (1 to 2 weeks, then a week of
  dogfood): SilverJS becomes the default; QuickJS-ng stays selectable by
  `--js-engine quickjs` for one dogfood week as the fallback, then
  `quickjs/`, `rquickjs` and the four `unsafe` modules that exist for
  it are removed, and `security.md` section 5 loses QuickJS's C.
- **J9. Temporal** (after J7; the lead, 2026-11-20): Chrome ships it
  and Silver presents itself as Chrome, so SilverJS implements it;
  QuickJS-ng has none of it, so it does not gate the switch. Its
  calendars and time zones come from the embedder's ICU4X through hooks,
  as `Intl`'s do. Target: at least 95% of `test/built-ins/Temporal`
  and `test/intl402/Temporal` with Silver's ICU4X hooks, every Temporal
  test of Chrome's shipped behaviour that the reference browser passes
  passing too.

The order, then: J1, J2, J3, J4 in SilverJS alone, J0 slotted in once
G90 phase 3 closes, then J5 (which needs J0), J6, J7, then J9 and,
after beta, J8. About 35 to 45 weeks of one session for J1 to J8, J0 a few days.
Nothing here changes the beta's order: J1 to J7 proceed while the beta
work does, at a lower priority, and J8 waits for the beta.

## 4. Not in this note

- The baseline JIT, and any optimizing tier: a note of its own after
  J8, gated on the sandbox's dynamic-code decision (G112).
- `SharedArrayBuffer`, `Atomics.wait` and cross-origin isolation.
- Temporal's design: a note of its own when J9 starts (its milestone
  and target are in section 3).
- A debugger protocol for DevTools on SilverJS (breakpoints, stepping);
  source positions and structured stacks are designed in now so that
  it can be added.
- Compiling scripts off the main thread as they stream: the heap-free
  compiler output allows it; it is a J6+ option if load benches show
  compiling on the critical path.
- Publication on crates.io, and the name there: the user's decision.
