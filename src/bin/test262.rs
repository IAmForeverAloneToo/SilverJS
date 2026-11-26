//! SilverJS's Test262 runner (docs/design.md section 1.7), built on the
//! crate's public API alone.
//!
//!     cargo run --release --bin test262 -- <test262 dir> [options]
//!
//! By default it runs the parse phase (J1): every test whose front
//! matter expects a parse-time SyntaxError must fail to parse, and every
//! other test must parse, in each mode its flags ask for (sloppy and
//! strict by default, `onlyStrict`, `noStrict`, `raw`, `module`). With
//! `--run` it runs the tests (J2.5): the harness's `assert.js` and
//! `sta.js` and the test's `includes`, then the test, in a fresh agent
//! for each mode, with `print` and `$262` (`global`, `evalScript`, `gc`,
//! `createRealm`) made through the embedding API as any embedder would;
//! a negative runtime test must throw an error of its type, an `async`
//! test must print its completion. Tests of features the reference
//! browser does not ship are counted as not applicable, as are those of
//! tail calls, which it does not implement.
//!
//! Options:
//!   --run                  run the tests, not only parse them
//!   --expectations <file>  the expected failures (default
//!                          test262/expected-parse-failures.txt, or
//!                          test262/expected-run-failures.txt with --run)
//!   --update               rewrite the expectations file from this run
//!   --all                  print every new failure, not the first 200
//!   --threads <n>          worker threads (default: the machine's)
//!   <prefix>...            only tests whose path starts with one of these
//!                          (relative to the checkout, e.g.
//!                          test/language/expressions)
//!
//! A run fails (exit code 1) when a test fails that the expectations do
//! not list, or passes that they do: progress is a shrinking file, and a
//! regression cannot pass unseen.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use silverjs::agent::{
    Agent, AgentOptions, Local, Outcome as RunOutcome, PropertyDescriptor, Scope,
};
use silverjs::compiler::CompileError;
use silverjs::parser::{Goal, parse_with_stack};
use silverjs::{ErrorKind, JsResult, Thrown};

/// The Test262 revision the expectations are measured on.
const REVISION: &str = "2e58f87d8350ad51224617a041c6e041d52f8444";

/// The directories measured (design section 1.7).
const ROOTS: &[&str] = &["test/language", "test/built-ins", "test/annexB"];

/// Features the reference browser does not ship: their tests are not
/// applicable, neither passes nor failures. `SharedArrayBuffer` is a
/// cross-origin isolated page's alone, which Silver does not make
/// (design section 1.7); `Atomics` every page has.
const NOT_APPLICABLE: &[&str] = &[
    "SharedArrayBuffer",
    "decorators",
    "import-defer",
    "source-phase-imports",
    "source-phase-imports-module-source",
    "ShadowRealm",
    // A proposal the reference browser does not ship: its
    // `ArrayBuffer.prototype` has no `transferToImmutable` (the binary
    // data probe, J4.2a).
    "immutable-arraybuffer",
    // Chrome 158 doesn't ship it: `Iterator.prototype` has no `chunks`
    // or `windows`, `Promise` no `allKeyed` or `allSettledKeyed` (the
    // J4.4 probe). SilverJS ships what the reference browser ships; at
    // each new major version these are probed again.
    "iterator-chunking",
    "await-dictionary",
    // Chrome 158 doesn't ship it: a private name is added to an object
    // that is not extensible (sealed, frozen, an array, a proxy of
    // one) without a TypeError (the J4.6 private probe, and these
    // tests run there with their harness).
    "nonextensible-applies-to-private",
];

/// Features the reference browser does not implement, though ECMA-262
/// has them: not applicable when tests are run.
/// So are the reference browser's own choices where a proposal differs:
/// its `stack` is an own property of each error, not an accessor of
/// `Error.prototype` (design note, section 1.11).
const NOT_RUN: &[&str] = &["tail-call-optimization", "error-stack-accessor"];

/// Tests that fail because SilverJS shows what the reference browser
/// shows where ECMA-262 says otherwise (docs/chrome-surface.md): its own
/// `caller` and `arguments` accessors on `Function.prototype`, not
/// `%ThrowTypeError%`; a `DataView`'s `byteLength` and `byteOffset`
/// throwing once it is detached or out of bounds, where these tests
/// want 0; RegExp's legacy statics with a setter each and no check of
/// their receiver, and `compile` on a subclass's or another realm's
/// regexp, where these tests (the legacy-features proposal) want a
/// TypeError or no setter; and the statics' accessors' source text
/// (`function get $&() { [native code] }`), which NativeFunction's
/// grammar refuses; and a reference's evaluation in an assignment, a
/// compound assignment and an update: its binding looked up again for
/// the write (in a `with` object, or past an eval's new `var`), a
/// computed key converted again, `@@unscopables` and a proxy
/// environment's `has` asked once for each lookup, and a binding a
/// strict write finds gone written anyway (each test run in Chrome 158
/// with its harness, the J4.6c Test262 page, failing with SilverJS's
/// message).
const CHROME_CHOICES: &[&str] = &[
    "test/built-ins/Function/prototype/arguments/prop-desc.js",
    "test/built-ins/Function/prototype/caller-arguments/accessor-properties.js",
    "test/built-ins/Function/prototype/caller/prop-desc.js",
    "test/built-ins/ThrowTypeError/unique-per-realm-function-proto.js",
    "test/built-ins/DataView/prototype/byteLength/detached-buffer.js",
    "test/built-ins/DataView/prototype/byteLength/resizable-array-buffer-auto.js",
    "test/built-ins/DataView/prototype/byteLength/resizable-array-buffer-fixed.js",
    "test/built-ins/DataView/prototype/byteOffset/detached-buffer.js",
    "test/built-ins/DataView/prototype/byteOffset/resizable-array-buffer-auto.js",
    "test/built-ins/DataView/prototype/byteOffset/resizable-array-buffer-fixed.js",
    "test/annexB/built-ins/RegExp/legacy-accessors/index/prop-desc.js",
    "test/annexB/built-ins/RegExp/legacy-accessors/index/this-cross-realm-constructor.js",
    "test/annexB/built-ins/RegExp/legacy-accessors/index/this-not-regexp-constructor.js",
    "test/annexB/built-ins/RegExp/legacy-accessors/index/this-subclass-constructor.js",
    "test/annexB/built-ins/RegExp/legacy-accessors/input/this-cross-realm-constructor.js",
    "test/annexB/built-ins/RegExp/legacy-accessors/input/this-not-regexp-constructor.js",
    "test/annexB/built-ins/RegExp/legacy-accessors/input/this-subclass-constructor.js",
    "test/annexB/built-ins/RegExp/legacy-accessors/lastMatch/prop-desc.js",
    "test/annexB/built-ins/RegExp/legacy-accessors/lastMatch/this-cross-realm-constructor.js",
    "test/annexB/built-ins/RegExp/legacy-accessors/lastMatch/this-not-regexp-constructor.js",
    "test/annexB/built-ins/RegExp/legacy-accessors/lastMatch/this-subclass-constructor.js",
    "test/annexB/built-ins/RegExp/legacy-accessors/lastParen/prop-desc.js",
    "test/annexB/built-ins/RegExp/legacy-accessors/lastParen/this-cross-realm-constructor.js",
    "test/annexB/built-ins/RegExp/legacy-accessors/lastParen/this-not-regexp-constructor.js",
    "test/annexB/built-ins/RegExp/legacy-accessors/lastParen/this-subclass-constructor.js",
    "test/annexB/built-ins/RegExp/legacy-accessors/leftContext/prop-desc.js",
    "test/annexB/built-ins/RegExp/legacy-accessors/leftContext/this-cross-realm-constructor.js",
    "test/annexB/built-ins/RegExp/legacy-accessors/leftContext/this-not-regexp-constructor.js",
    "test/annexB/built-ins/RegExp/legacy-accessors/leftContext/this-subclass-constructor.js",
    "test/annexB/built-ins/RegExp/legacy-accessors/rightContext/prop-desc.js",
    "test/annexB/built-ins/RegExp/legacy-accessors/rightContext/this-cross-realm-constructor.js",
    "test/annexB/built-ins/RegExp/legacy-accessors/rightContext/this-not-regexp-constructor.js",
    "test/annexB/built-ins/RegExp/legacy-accessors/rightContext/this-subclass-constructor.js",
    "test/annexB/built-ins/RegExp/prototype/compile/this-cross-realm-instance.js",
    "test/annexB/built-ins/RegExp/prototype/compile/this-subclass-instance.js",
    "test/built-ins/Function/prototype/toString/built-in-function-object.js",
    "test/language/expressions/assignment/S11.13.1_A5_T1.js",
    "test/language/expressions/assignment/S11.13.1_A5_T2.js",
    "test/language/expressions/assignment/S11.13.1_A5_T3.js",
    "test/language/expressions/assignment/S11.13.1_A6_T1.js",
    "test/language/expressions/assignment/S11.13.1_A6_T2.js",
    "test/language/expressions/assignment/S11.13.1_A6_T3.js",
    "test/language/expressions/compound-assignment/S11.13.2_A5.10_T1.js",
    "test/language/expressions/compound-assignment/S11.13.2_A5.10_T2.js",
    "test/language/expressions/compound-assignment/S11.13.2_A5.10_T3.js",
    "test/language/expressions/compound-assignment/S11.13.2_A5.11_T1.js",
    "test/language/expressions/compound-assignment/S11.13.2_A5.11_T2.js",
    "test/language/expressions/compound-assignment/S11.13.2_A5.11_T3.js",
    "test/language/expressions/compound-assignment/S11.13.2_A5.1_T1.js",
    "test/language/expressions/compound-assignment/S11.13.2_A5.1_T2.js",
    "test/language/expressions/compound-assignment/S11.13.2_A5.1_T3.js",
    "test/language/expressions/compound-assignment/S11.13.2_A5.2_T1.js",
    "test/language/expressions/compound-assignment/S11.13.2_A5.2_T2.js",
    "test/language/expressions/compound-assignment/S11.13.2_A5.2_T3.js",
    "test/language/expressions/compound-assignment/S11.13.2_A5.3_T1.js",
    "test/language/expressions/compound-assignment/S11.13.2_A5.3_T2.js",
    "test/language/expressions/compound-assignment/S11.13.2_A5.3_T3.js",
    "test/language/expressions/compound-assignment/S11.13.2_A5.4_T1.js",
    "test/language/expressions/compound-assignment/S11.13.2_A5.4_T2.js",
    "test/language/expressions/compound-assignment/S11.13.2_A5.4_T3.js",
    "test/language/expressions/compound-assignment/S11.13.2_A5.5_T1.js",
    "test/language/expressions/compound-assignment/S11.13.2_A5.5_T2.js",
    "test/language/expressions/compound-assignment/S11.13.2_A5.5_T3.js",
    "test/language/expressions/compound-assignment/S11.13.2_A5.6_T1.js",
    "test/language/expressions/compound-assignment/S11.13.2_A5.6_T2.js",
    "test/language/expressions/compound-assignment/S11.13.2_A5.6_T3.js",
    "test/language/expressions/compound-assignment/S11.13.2_A5.7_T1.js",
    "test/language/expressions/compound-assignment/S11.13.2_A5.7_T2.js",
    "test/language/expressions/compound-assignment/S11.13.2_A5.7_T3.js",
    "test/language/expressions/compound-assignment/S11.13.2_A5.8_T1.js",
    "test/language/expressions/compound-assignment/S11.13.2_A5.8_T2.js",
    "test/language/expressions/compound-assignment/S11.13.2_A5.8_T3.js",
    "test/language/expressions/compound-assignment/S11.13.2_A5.9_T1.js",
    "test/language/expressions/compound-assignment/S11.13.2_A5.9_T2.js",
    "test/language/expressions/compound-assignment/S11.13.2_A5.9_T3.js",
    "test/language/expressions/compound-assignment/S11.13.2_A6.10_T1.js",
    "test/language/expressions/compound-assignment/S11.13.2_A6.11_T1.js",
    "test/language/expressions/compound-assignment/S11.13.2_A6.1_T1.js",
    "test/language/expressions/compound-assignment/S11.13.2_A6.2_T1.js",
    "test/language/expressions/compound-assignment/S11.13.2_A6.3_T1.js",
    "test/language/expressions/compound-assignment/S11.13.2_A6.4_T1.js",
    "test/language/expressions/compound-assignment/S11.13.2_A6.5_T1.js",
    "test/language/expressions/compound-assignment/S11.13.2_A6.6_T1.js",
    "test/language/expressions/compound-assignment/S11.13.2_A6.7_T1.js",
    "test/language/expressions/compound-assignment/S11.13.2_A6.8_T1.js",
    "test/language/expressions/compound-assignment/S11.13.2_A6.9_T1.js",
    "test/language/expressions/compound-assignment/left-hand-side-property-reference-topropertykey-once-add.js",
    "test/language/expressions/compound-assignment/left-hand-side-property-reference-topropertykey-once-bitand.js",
    "test/language/expressions/compound-assignment/left-hand-side-property-reference-topropertykey-once-bitor.js",
    "test/language/expressions/compound-assignment/left-hand-side-property-reference-topropertykey-once-bitxor.js",
    "test/language/expressions/compound-assignment/left-hand-side-property-reference-topropertykey-once-div.js",
    "test/language/expressions/compound-assignment/left-hand-side-property-reference-topropertykey-once-exp.js",
    "test/language/expressions/compound-assignment/left-hand-side-property-reference-topropertykey-once-lshift.js",
    "test/language/expressions/compound-assignment/left-hand-side-property-reference-topropertykey-once-mod.js",
    "test/language/expressions/compound-assignment/left-hand-side-property-reference-topropertykey-once-mult.js",
    "test/language/expressions/compound-assignment/left-hand-side-property-reference-topropertykey-once-rshift.js",
    "test/language/expressions/compound-assignment/left-hand-side-property-reference-topropertykey-once-srshift.js",
    "test/language/expressions/compound-assignment/left-hand-side-property-reference-topropertykey-once-sub.js",
    "test/language/expressions/postfix-decrement/S11.3.2_A5_T1.js",
    "test/language/expressions/postfix-decrement/S11.3.2_A5_T2.js",
    "test/language/expressions/postfix-decrement/S11.3.2_A5_T3.js",
    "test/language/expressions/postfix-decrement/S11.3.2_A6_T3.js",
    "test/language/expressions/postfix-increment/S11.3.1_A5_T1.js",
    "test/language/expressions/postfix-increment/S11.3.1_A5_T2.js",
    "test/language/expressions/postfix-increment/S11.3.1_A5_T3.js",
    "test/language/expressions/postfix-increment/S11.3.1_A6_T3.js",
    "test/language/expressions/prefix-decrement/S11.4.5_A5_T1.js",
    "test/language/expressions/prefix-decrement/S11.4.5_A5_T2.js",
    "test/language/expressions/prefix-decrement/S11.4.5_A5_T3.js",
    "test/language/expressions/prefix-decrement/S11.4.5_A6_T3.js",
    "test/language/expressions/prefix-increment/S11.4.4_A5_T1.js",
    "test/language/expressions/prefix-increment/S11.4.4_A5_T2.js",
    "test/language/expressions/prefix-increment/S11.4.4_A5_T3.js",
    "test/language/expressions/prefix-increment/S11.4.4_A6_T3.js",
    "test/language/statements/with/get-binding-value-call-with-proxy-env.js",
    "test/language/statements/with/get-binding-value-idref-with-proxy-env.js",
    "test/language/statements/with/get-mutable-binding-binding-deleted-in-get-unscopables-strict-mode.js",
    "test/language/statements/with/set-mutable-binding-binding-deleted-in-get-unscopables-strict-mode.js",
    "test/language/statements/with/set-mutable-binding-binding-deleted-with-typed-array-in-proto-chain-strict-mode.js",
    "test/language/statements/with/set-mutable-binding-idref-compound-assign-with-proxy-env.js",
    "test/language/statements/with/set-mutable-binding-idref-with-proxy-env.js",
    "test/language/statements/with/unscopables-inc-dec.js",
];

/// How long one test may take before it is reported as a failure
/// ("timeout") and left behind: a parser that loops must not hang the run.
const TIME_LIMIT: Duration = Duration::from_secs(10);

/// Tests that take [`LONG_TIME_LIMIT`] instead, each with its reason
/// (the lead, 2026-11-26): ones near the limit by the work they do,
/// which a loaded host can push past it.
const LONG_LIMIT: &[(&str, &str)] = &[(
    "test/language/literals/regexp/S7.8.5_A1.1_T2.js",
    "65k evals; host load",
)];

const LONG_TIME_LIMIT: Duration = Duration::from_secs(30);

/// Test `test`'s time limit.
fn time_limit(test: &str) -> Duration {
    if LONG_LIMIT.iter().any(|(t, _)| *t == test) {
        LONG_TIME_LIMIT
    } else {
        TIME_LIMIT
    }
}

/// A stack for the parsing threads: the parser bounds its own nesting,
/// and debug builds use large frames.
const STACK: usize = 64 << 20;

#[derive(Debug, Default)]
struct FrontMatter {
    flags: Vec<String>,
    features: Vec<String>,
    negative_phase: Option<String>,
    negative_type: Option<String>,
    includes: Vec<String>,
}

/// The YAML between `/*---` and `---*/`, as much of it as the runner
/// reads: `flags`, `features` and `negative`'s `phase` and `type`, as
/// inline (`[a, b]`) or block (`- a`) lists.
fn front_matter(source: &str) -> FrontMatter {
    let mut fm = FrontMatter::default();
    let Some(start) = source.find("/*---") else {
        return fm;
    };
    let Some(len) = source.get(start..).and_then(|s| s.find("---*/")) else {
        return fm;
    };
    let yaml = source.get(start + 5..start + len).unwrap_or("");
    let mut key = String::new();
    for line in yaml.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let indented = line.starts_with(' ') || line.starts_with('\t');
        if !indented {
            if let Some((k, rest)) = trimmed.split_once(':') {
                key = k.trim().to_owned();
                let rest = rest.trim();
                if let Some(list) = rest.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
                    let items = list
                        .split(',')
                        .map(|s| s.trim().to_owned())
                        .filter(|s| !s.is_empty());
                    match key.as_str() {
                        "flags" => fm.flags.extend(items),
                        "features" => fm.features.extend(items),
                        "includes" => fm.includes.extend(items),
                        _ => {}
                    }
                }
            }
            continue;
        }
        if let Some(item) = trimmed.strip_prefix("- ") {
            match key.as_str() {
                "flags" => fm.flags.push(item.trim().to_owned()),
                "features" => fm.features.push(item.trim().to_owned()),
                "includes" => fm.includes.push(item.trim().to_owned()),
                _ => {}
            }
        } else if key == "negative"
            && let Some((k, v)) = trimmed.split_once(':')
        {
            match k.trim() {
                "phase" => fm.negative_phase = Some(v.trim().to_owned()),
                "type" => fm.negative_type = Some(v.trim().to_owned()),
                _ => {}
            }
        }
    }
    fm
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Pass,
    Fail,
    NotApplicable,
}

/// Runs one test's parse phase in each of its modes; the first failure's
/// description if any.
fn run_test(source: &str) -> (Outcome, Option<String>) {
    let fm = front_matter(source);
    if fm
        .features
        .iter()
        .any(|f| NOT_APPLICABLE.contains(&f.as_str()))
    {
        return (Outcome::NotApplicable, None);
    }
    let has = |flag: &str| fm.flags.iter().any(|f| f == flag);
    let expect_error = fm.negative_phase.as_deref() == Some("parse");
    let mut modes: Vec<(Goal, bool)> = Vec::new();
    if has("module") {
        modes.push((Goal::Module, false));
    } else if has("raw") {
        modes.push((Goal::Script, false));
    } else {
        if !has("onlyStrict") {
            modes.push((Goal::Script, false));
        }
        if !has("noStrict") {
            modes.push((Goal::Script, true));
        }
    }
    for (goal, strict) in modes {
        let text;
        let src = if strict {
            text = format!("\"use strict\";\n{source}");
            text.as_str()
        } else {
            source
        };
        let mode = match (goal, strict) {
            (Goal::Module, _) => "module",
            (_, true) => "strict",
            _ => "sloppy",
        };
        match parse_with_stack(src, goal, STACK / 2) {
            Ok(_) if expect_error => {
                let want = fm.negative_type.as_deref().unwrap_or("SyntaxError");
                return (
                    Outcome::Fail,
                    Some(format!("{mode}: parsed, wanted {want}")),
                );
            }
            Err(e) if !expect_error => {
                return (
                    Outcome::Fail,
                    Some(format!("{mode}: {} at {}", e.message, e.start)),
                );
            }
            _ => {}
        }
    }
    (Outcome::Pass, None)
}

thread_local! {
    /// The harness files a worker has read.
    static HARNESS: RefCell<HashMap<String, Rc<str>>> = RefCell::new(HashMap::new());
}

fn harness(root: &Path, name: &str) -> Option<Rc<str>> {
    HARNESS.with(|h| {
        if let Some(text) = h.borrow().get(name) {
            return Some(Rc::clone(text));
        }
        let text: Rc<str> = std::fs::read_to_string(root.join("harness").join(name))
            .ok()?
            .into();
        h.borrow_mut().insert(name.to_owned(), Rc::clone(&text));
        Some(text)
    })
}

/// A value's text for a failure's description.
fn describe<'s>(scope: &mut Scope<'s>, v: Local<'s>) -> String {
    match scope.to_text(v) {
        Ok(t) => t,
        Err(_) => "(a value whose ToString throws)".to_owned(),
    }
}

/// The `name` of a thrown value's constructor, if it has one.
fn constructor_name<'s>(scope: &mut Scope<'s>, v: Local<'s>) -> Option<String> {
    let key = scope.key("constructor");
    let c = scope.get_property(v, key).ok()?;
    let key = scope.key("name");
    let n = scope.get_property(c, key).ok()?;
    scope.to_text(n).ok()
}

/// Defines a non-enumerable property, as built-ins are.
fn define<'s>(scope: &mut Scope<'s>, on: Local<'s>, name: &str, value: Local<'s>) {
    let key = scope.key(name);
    scope.define_property(
        on,
        key,
        PropertyDescriptor {
            value: Some(value),
            writable: Some(true),
            enumerable: Some(false),
            configurable: Some(true),
            ..PropertyDescriptor::default()
        },
    );
}

/// `print` and `$262` on the current realm's global object; gives back
/// that `$262`.
fn install_host<'s>(
    scope: &mut Scope<'s>,
    printed: &Rc<RefCell<Vec<String>>>,
) -> JsResult<'s, Local<'s>> {
    let Ok(global) = scope.global_object() else {
        return Err(Thrown::Terminated);
    };
    let out = Rc::clone(printed);
    let print = scope.native_function("print", 1, move |scope, _, args| {
        let text = match args.first() {
            Some(&v) => scope.to_text(v)?,
            None => String::new(),
        };
        out.borrow_mut().push(text);
        Ok(scope.undefined())
    })?;
    define(scope, global, "print", print);
    let key = scope.key("Object");
    let object = scope.get_property(global, key)?;
    let key = scope.key("prototype");
    let proto = scope.get_property(object, key)?;
    let Ok(host) = scope.object(Some(proto)) else {
        return Err(Thrown::Terminated);
    };
    define(scope, host, "global", global);
    let eval_script = scope.native_function("evalScript", 1, |scope, _, args| {
        let source = match args.first() {
            Some(&v) => scope.to_text(v)?,
            None => String::new(),
        };
        scope.eval_script(&source)
    })?;
    define(scope, host, "evalScript", eval_script);
    let gc = scope.native_function("gc", 0, |scope, _, _| {
        scope.collect();
        Ok(scope.undefined())
    })?;
    define(scope, host, "gc", gc);
    let detach = scope.native_function("detachArrayBuffer", 1, |scope, _, args| {
        let detached = args.first().is_some_and(|&b| scope.detach_array_buffer(b));
        if detached {
            return Ok(scope.undefined());
        }
        let e = scope.new_error(ErrorKind::TypeError, "not an ArrayBuffer")?;
        Err(Thrown::Value(e))
    })?;
    define(scope, host, "detachArrayBuffer", detach);
    // A new realm, with its own `print` and `$262`: the new `$262`.
    let out = Rc::clone(printed);
    let create_realm = scope.native_function("createRealm", 0, move |scope, _, _| {
        let realm = scope.new_realm()?;
        scope.in_realm(realm, |scope| install_host(scope, &out))
    })?;
    define(scope, host, "createRealm", create_realm);
    define(scope, global, "$262", host);
    Ok(host)
}

/// Runs a test in one mode: a fresh agent, the harness, then the test.
fn run_mode(
    root: &Path,
    fm: &FrontMatter,
    source: &str,
    strict: bool,
    limit: Duration,
) -> Result<(), String> {
    let has = |flag: &str| fm.flags.iter().any(|f| f == flag);
    let mut agent = Agent::new(AgentOptions::default());
    let deadline = Instant::now() + limit;
    agent.set_interrupt(Some(Box::new(move || Instant::now() > deadline)));
    let printed = Rc::new(RefCell::new(Vec::new()));
    let negative = fm.negative_phase.as_deref();
    let want = fm.negative_type.as_deref().unwrap_or("SyntaxError");
    agent.scope(|scope| {
        if install_host(scope, &printed).is_err() {
            return Err("the host's natives did not install".to_owned());
        }
        let mut scripts: Vec<(String, Rc<str>)> = Vec::new();
        if !has("raw") {
            for name in ["assert.js", "sta.js"] {
                scripts.push((name.to_owned(), harness(root, name).ok_or("no harness")?));
            }
            if has("async") {
                let name = "doneprintHandle.js";
                scripts.push((name.to_owned(), harness(root, name).ok_or("no harness")?));
            }
        }
        for name in &fm.includes {
            let text = harness(root, name).ok_or_else(|| format!("no harness file {name}"))?;
            scripts.push((name.clone(), text));
        }
        let test: Rc<str> = if strict {
            format!("\"use strict\";\n{source}").into()
        } else {
            source.into()
        };
        scripts.push(("test".to_owned(), test));
        let last = scripts.len() - 1;
        for (i, (name, text)) in scripts.iter().enumerate() {
            let is_test = i == last;
            let done = scope.nested(|scope| match scope.run_script(text) {
                Err(CompileError::Unsupported { feature, .. }) => {
                    Err(format!("unsupported: {feature}"))
                }
                Err(CompileError::Syntax(e)) => {
                    if is_test && matches!(negative, Some("parse" | "early")) {
                        Ok(true)
                    } else {
                        Err(format!("{name}: SyntaxError: {} at {}", e.message, e.start))
                    }
                }
                Ok(RunOutcome::Terminated) => Err("timeout".to_owned()),
                Ok(RunOutcome::Threw(e)) => {
                    let ctor = constructor_name(scope, e);
                    if is_test && negative == Some("runtime") && ctor.as_deref() == Some(want) {
                        Ok(true)
                    } else {
                        let text = describe(scope, e);
                        Err(format!("{name}: threw {text}"))
                    }
                }
                Ok(RunOutcome::Value(_)) => Ok(false),
            })?;
            if done {
                return Ok(());
            }
        }
        if let Some(phase) = negative {
            return Err(format!("completed, wanted a {phase} {want}"));
        }
        if has("async") {
            let printed = printed.borrow();
            if !printed.iter().any(|p| p == "Test262:AsyncTestComplete") {
                let first = printed.first().cloned().unwrap_or_default();
                return Err(format!("async: {first}"));
            }
        }
        Ok(())
    })
}

/// Runs a test, in each mode its flags ask for.
fn run_exec(root: &Path, source: &str, limit: Duration) -> (Outcome, Option<String>) {
    let fm = front_matter(source);
    if fm
        .features
        .iter()
        .any(|f| NOT_APPLICABLE.contains(&f.as_str()) || NOT_RUN.contains(&f.as_str()))
    {
        return (Outcome::NotApplicable, None);
    }
    let has = |flag: &str| fm.flags.iter().any(|f| f == flag);
    if has("module") {
        return (Outcome::Fail, Some("modules".to_owned()));
    }
    let mut modes = Vec::new();
    if has("raw") || !has("onlyStrict") {
        modes.push(false);
    }
    if !has("raw") && !has("noStrict") {
        modes.push(true);
    }
    for strict in modes {
        if let Err(e) = run_mode(root, &fm, source, strict, limit) {
            let mode = if strict { "strict" } else { "sloppy" };
            return (Outcome::Fail, Some(format!("{mode}: {e}")));
        }
    }
    (Outcome::Pass, None)
}

fn collect(dir: &Path, root: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, root, out);
        } else if path.extension().is_some_and(|e| e == "js")
            && !path.to_string_lossy().contains("_FIXTURE")
            && let Ok(rel) = path.strip_prefix(root)
        {
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
}

/// The checkout's commit, read from its `.git` without running git.
fn checkout_revision(root: &Path) -> Option<String> {
    let git = root.join(".git");
    let head = std::fs::read_to_string(git.join("HEAD")).ok()?;
    let head = head.trim();
    let Some(reference) = head.strip_prefix("ref: ") else {
        return Some(head.to_owned());
    };
    if let Ok(id) = std::fs::read_to_string(git.join(reference)) {
        return Some(id.trim().to_owned());
    }
    let packed = std::fs::read_to_string(git.join("packed-refs")).ok()?;
    packed
        .lines()
        .find(|l| l.ends_with(reference))
        .and_then(|l| l.split_whitespace().next())
        .map(str::to_owned)
}

type TestResult = (String, Outcome, Option<String>);

struct Shared {
    root: PathBuf,
    tests: Vec<String>,
    next: AtomicUsize,
    done: AtomicUsize,
    results: Mutex<Vec<TestResult>>,
    /// Each worker's test and when it began.
    running: Mutex<Vec<Option<(usize, Instant)>>>,
    /// Run the tests, not only parse them.
    execute: bool,
}

/// Runs every test on `threads` workers. A test past [`TIME_LIMIT`] is
/// a failure, and its worker is abandoned (the process exits when the
/// rest are done).
fn run_all(root: &Path, tests: Vec<String>, threads: usize, execute: bool) -> Vec<TestResult> {
    let threads = threads.max(1);
    let total = tests.len();
    let shared = Arc::new(Shared {
        root: root.to_owned(),
        tests,
        next: AtomicUsize::new(0),
        done: AtomicUsize::new(0),
        results: Mutex::new(Vec::new()),
        running: Mutex::new(vec![None; threads]),
        execute,
    });
    for worker in 0..threads {
        spawn_worker(&shared, worker);
    }
    let mut timed_out: Vec<usize> = Vec::new();
    loop {
        std::thread::sleep(Duration::from_millis(100));
        let mut replacements = 0;
        if let Ok(running) = shared.running.lock() {
            for (i, began) in running.iter().flatten() {
                // A run's own interrupt hook stops script at the limit; a
                // worker past twice it is stuck in Rust.
                let limit = shared.tests.get(*i).map_or(TIME_LIMIT, |t| time_limit(t));
                if began.elapsed() > limit * 2 && !timed_out.contains(i) {
                    timed_out.push(*i);
                    replacements += 1;
                    if let Some(test) = shared.tests.get(*i) {
                        eprintln!("test262: {test} passed the time limit");
                    }
                }
            }
        }
        // A worker stuck past the limit is left behind; another takes its
        // place, so that the run goes on.
        for _ in 0..replacements {
            let worker = match shared.running.lock() {
                Ok(mut running) => {
                    running.push(None);
                    running.len() - 1
                }
                Err(_) => break,
            };
            spawn_worker(&shared, worker);
        }
        if shared.done.load(Ordering::Relaxed) + timed_out.len() >= total {
            break;
        }
    }
    let mut results = shared.results.lock().map(|r| r.clone()).unwrap_or_default();
    for i in timed_out {
        if let Some(test) = shared.tests.get(i) {
            results.push((test.clone(), Outcome::Fail, Some("timeout".to_owned())));
        }
    }
    results
}

fn spawn_worker(shared: &Arc<Shared>, worker: usize) {
    let shared = Arc::clone(shared);
    let spawned = std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(move || work(&shared, worker));
    if let Err(e) = spawned {
        eprintln!("test262: cannot start a thread: {e}");
        std::process::exit(2);
    }
}

fn work(shared: &Shared, worker: usize) {
    loop {
        let i = shared.next.fetch_add(1, Ordering::Relaxed);
        let Some(test) = shared.tests.get(i) else {
            break;
        };
        if let Ok(mut running) = shared.running.lock()
            && let Some(slot) = running.get_mut(worker)
        {
            *slot = Some((i, Instant::now()));
        }
        let (outcome, detail) = match std::fs::read_to_string(shared.root.join(test)) {
            Ok(source) if shared.execute => run_exec(&shared.root, &source, time_limit(test)),
            Ok(source) => run_test(&source),
            Err(e) => (Outcome::Fail, Some(format!("unreadable: {e}"))),
        };
        if let Ok(mut running) = shared.running.lock()
            && let Some(slot) = running.get_mut(worker)
        {
            *slot = None;
        }
        if let Ok(mut r) = shared.results.lock() {
            r.push((test.clone(), outcome, detail));
        }
        shared.done.fetch_add(1, Ordering::Relaxed);
    }
}

/// A failure's one-word reason for the expectations file.
fn reason_of(detail: &str) -> String {
    if let Some(at) = detail.find("unsupported: ") {
        let feature = detail.get(at + 13..).unwrap_or("");
        return feature.trim().replace(' ', "-");
    }
    if detail.contains("async: ") {
        return "async".to_owned();
    }
    if detail.contains(": threw ") {
        return "throws".to_owned();
    }
    if detail.contains("completed, wanted") {
        return "completes".to_owned();
    }
    if detail.contains("SyntaxError: ") {
        return "syntax".to_owned();
    }
    reason_of_parse(detail).to_owned()
}

fn reason_of_parse(detail: &str) -> &'static str {
    if detail == "modules" {
        "modules"
    } else if detail.contains("Classes are not supported yet") || detail.contains("Private field") {
        "classes"
    } else if detail.contains("Modules are not supported yet") {
        "modules"
    } else if detail == "timeout" {
        "timeout"
    } else if detail.contains("parsed, wanted") {
        "accepts"
    } else {
        "refuses"
    }
}

fn read_expectations(path: &Path) -> BTreeMap<String, String> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| match l.split_once(' ') {
            Some((t, r)) => (t.to_owned(), r.trim().to_owned()),
            None => (l.to_owned(), "unknown".to_owned()),
        })
        .collect()
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mut root: Option<PathBuf> = None;
    let mut expectations: Option<PathBuf> = None;
    let mut update = false;
    let mut execute = false;
    let mut all = false;
    let mut threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let mut prefixes = Vec::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--expectations" => {
                if let Some(p) = args.next() {
                    expectations = Some(PathBuf::from(p));
                }
            }
            "--update" => update = true,
            "--run" => execute = true,
            "--all" => all = true,
            "--threads" => {
                if let Some(n) = args.next().and_then(|n| n.parse().ok()) {
                    threads = n;
                }
            }
            _ if root.is_none() => root = Some(PathBuf::from(arg)),
            _ => prefixes.push(arg),
        }
    }
    let Some(root) = root else {
        eprintln!(
            "usage: test262 <test262 dir> [--run] [--expectations f] [--update] [--threads n] [prefix...]"
        );
        std::process::exit(2);
    };
    match checkout_revision(&root) {
        Some(rev) if rev == REVISION => {}
        Some(rev) => eprintln!("test262: the checkout is at {rev}, the pin is {REVISION}"),
        None => eprintln!("test262: cannot read the checkout's revision; the pin is {REVISION}"),
    }
    let mut tests = Vec::new();
    for r in ROOTS {
        collect(&root.join(r), &root, &mut tests);
    }
    if !prefixes.is_empty() {
        tests.retain(|t| prefixes.iter().any(|p| t.starts_with(p.as_str())));
    }
    let expectations = expectations.unwrap_or_else(|| {
        PathBuf::from(if execute {
            "test262/expected-run-failures.txt"
        } else {
            "test262/expected-parse-failures.txt"
        })
    });
    let results = run_all(&root, tests, threads, execute);
    let mut results = results;
    results.sort_by(|a, b| a.0.cmp(&b.0));
    let expected = read_expectations(&expectations);
    let (mut pass, mut fail, mut na) = (0usize, 0usize, 0usize);
    let mut new_failures = Vec::new();
    let mut new_passes = Vec::new();
    let mut failing = BTreeMap::new();
    for (test, outcome, detail) in &results {
        match outcome {
            Outcome::Pass => {
                pass += 1;
                if expected.contains_key(test) {
                    new_passes.push(test.clone());
                }
            }
            Outcome::Fail => {
                fail += 1;
                // The reason: what it fails of now, when the file is
                // rewritten (a test once refused for `for-of` may now
                // fail for a class in it); else the file's.
                let fresh = || reason_of(detail.as_deref().unwrap_or(""));
                let reason = if CHROME_CHOICES.contains(&test.as_str()) {
                    "chrome-surface".to_owned()
                } else if update {
                    fresh()
                } else {
                    expected.get(test).cloned().unwrap_or_else(fresh)
                };
                failing.insert(test.clone(), reason);
                if !expected.contains_key(test) {
                    new_failures.push((test.clone(), detail.clone().unwrap_or_default()));
                }
            }
            Outcome::NotApplicable => na += 1,
        }
    }
    let measured = pass + fail;
    let rate = if measured == 0 {
        0.0
    } else {
        pass as f64 * 100.0 / measured as f64
    };
    let phase = if execute { "run" } else { "parse phase" };
    // What is given up on purpose to show what the reference browser
    // shows (J7's report counts it).
    let chrome = failing.values().filter(|r| *r == "chrome-surface").count();
    println!(
        "test262 ({phase}): {pass} passed, {fail} failed ({chrome} chrome-surface), {na} not applicable; {rate:.2}% of {measured}"
    );
    let shown = if all { usize::MAX } else { 200 };
    for (test, detail) in new_failures.iter().take(shown) {
        println!("NEW FAILURE {test}: {detail}");
    }
    if new_failures.len() > shown {
        println!("... and {} more new failures", new_failures.len() - shown);
    }
    for test in new_passes.iter().take(200) {
        println!("NEW PASS {test}");
    }
    if update {
        let mut text = String::from(if execute {
            "# Test262 tests SilverJS fails to run as they want, one a line with\n# a one-word reason; written by `test262 --run --update`, never by hand.\n"
        } else {
            "# Test262 tests SilverJS fails to parse as they want, one a line with\n# a one-word reason; written by `test262 --update`, never by hand.\n"
        });
        text.push_str(&format!("# Test262 revision {REVISION}.\n"));
        for (test, reason) in &failing {
            text.push_str(&format!("{test} {reason}\n"));
        }
        if let Err(e) = std::fs::write(&expectations, text) {
            eprintln!("test262: cannot write {}: {e}", expectations.display());
            std::process::exit(2);
        }
        println!("expectations written: {} failures", failing.len());
        std::process::exit(0);
    }
    let code = i32::from(!new_failures.is_empty() || !new_passes.is_empty());
    // Exits rather than returns: a worker past the time limit is still
    // running, and returning would wait for it.
    std::process::exit(code);
}
