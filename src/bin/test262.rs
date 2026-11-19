//! SilverJS's Test262 runner (docs/design.md section 1.7), built on the
//! crate's public API alone.
//!
//!     cargo run --release --bin test262 -- <test262 dir> [options]
//!
//! J1 runs the parse phase: every test whose front matter expects a
//! parse-time SyntaxError must fail to parse, and every other test must
//! parse, in each mode its flags ask for (sloppy and strict by default,
//! `onlyStrict`, `noStrict`, `raw`, `module`). Tests of features the
//! reference browser does not ship are counted as not applicable.
//!
//! Options:
//!   --expectations <file>  the expected failures (default
//!                          test262/expected-parse-failures.txt)
//!   --update               rewrite the expectations file from this run
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

use silverjs::parser::{Goal, parse_with_stack};

/// The Test262 revision the expectations are measured on.
const REVISION: &str = "2e58f87d8350ad51224617a041c6e041d52f8444";

/// The directories measured (design section 1.7).
const ROOTS: &[&str] = &["test/language", "test/built-ins", "test/annexB"];

/// Features the reference browser does not ship: their tests are not
/// applicable, neither passes nor failures.
const NOT_APPLICABLE: &[&str] = &[
    "SharedArrayBuffer",
    "Atomics",
    "Atomics.waitAsync",
    "Atomics.pause",
    "decorators",
    "import-defer",
    "source-phase-imports",
    "source-phase-imports-module-source",
    "ShadowRealm",
];

/// How long one test may take before it is reported as a failure
/// ("timeout") and left behind: a parser that loops must not hang the run.
const TIME_LIMIT: Duration = Duration::from_secs(10);

/// A stack for the parsing threads: the parser bounds its own nesting,
/// and debug builds use large frames.
const STACK: usize = 64 << 20;

#[derive(Debug, Default)]
struct FrontMatter {
    flags: Vec<String>,
    features: Vec<String>,
    negative_phase: Option<String>,
    negative_type: Option<String>,
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
}

/// Runs every test on `threads` workers. A test past [`TIME_LIMIT`] is
/// a failure, and its worker is abandoned (the process exits when the
/// rest are done).
fn run_all(root: &Path, tests: Vec<String>, threads: usize) -> Vec<TestResult> {
    let threads = threads.max(1);
    let total = tests.len();
    let shared = Arc::new(Shared {
        root: root.to_owned(),
        tests,
        next: AtomicUsize::new(0),
        done: AtomicUsize::new(0),
        results: Mutex::new(Vec::new()),
        running: Mutex::new(vec![None; threads]),
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
                if began.elapsed() > TIME_LIMIT && !timed_out.contains(i) {
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
fn reason_of(detail: &str) -> &'static str {
    if detail.contains("Classes are not supported yet") || detail.contains("Private field") {
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
    let mut expectations = PathBuf::from("test262/expected-parse-failures.txt");
    let mut update = false;
    let mut threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let mut prefixes = Vec::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--expectations" => {
                if let Some(p) = args.next() {
                    expectations = PathBuf::from(p);
                }
            }
            "--update" => update = true,
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
            "usage: test262 <test262 dir> [--expectations f] [--update] [--threads n] [prefix...]"
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
    let results = run_all(&root, tests, threads);
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
                let reason = expected
                    .get(test)
                    .cloned()
                    .unwrap_or_else(|| reason_of(detail.as_deref().unwrap_or("")).to_owned());
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
    println!(
        "test262 (parse phase): {pass} passed, {fail} failed, {na} not applicable; {rate:.2}% of {measured}"
    );
    for (test, detail) in new_failures.iter().take(200) {
        println!("NEW FAILURE {test}: {detail}");
    }
    if new_failures.len() > 200 {
        println!("... and {} more new failures", new_failures.len() - 200);
    }
    for test in new_passes.iter().take(200) {
        println!("NEW PASS {test}");
    }
    if update {
        let mut text = String::from(
            "# Test262 tests SilverJS fails to parse as they want, one a line with\n# a one-word reason; written by `test262 --update`, never by hand.\n",
        );
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
