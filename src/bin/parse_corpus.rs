//! Parses every script of a directory and reports what SilverJS made of
//! them: real pages' bundles, beside Test262's tests of the standard.
//!
//!     cargo run --release --bin parse_corpus -- <dir> [--top <n>] [--context] [--compile]
//!
//! Where the directory has an `index.tsv` (lines of file name, `script`
//! or `module`, and the URL the page ran it from), it says each file's
//! goal, and files are named by the URL's host. Otherwise files named
//! `*.mjs` or `*.module.js` are modules, other `*.js` and `*.cjs` files
//! scripts (a script that fails is tried as a module too).
//!
//! Each file parses on a thread with a large stack and no budget to
//! speak of, so that the deepest nesting and the stack it takes are
//! measured rather than refused. The report gives:
//! - how many files, and how many fail, each failure with its message and
//!   byte offset (line and column);
//! - the deepest nesting met, the most stack a parse took, and the stack
//!   per level they imply, beside the default budget
//!   (`DEFAULT_STACK_BUDGET`) and the files that would not fit it;
//! - parse time for the largest files, a median of seven parses;
//! - with `--compile`, what the compiler made of every script that
//!   parsed: how many compiled into a unit that validates, how many it
//!   refused and for what (what J2 does not run yet), and any other
//!   failure, which is a bug; then the compile time of the largest that
//!   compiled, beside their parse time.
//!
//! The corpus is third-party code: it is read where it lies and never
//! copied into the repository, and a failure is reported by host and
//! byte offset alone, unless `--context` asks for the source around it
//! (for local triage only).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use silverjs::compiler::{CompileError, compile_with_stack};
use silverjs::lexer::LineIndex;
use silverjs::parser::{DEFAULT_STACK_BUDGET, Goal, ParseStats, parse_with_stats};

/// The measuring thread's stack, and the budget that leaves it.
const STACK: usize = 512 << 20;
const BUDGET: usize = 480 << 20;

/// What `index.tsv` says of a file: its goal and the host it came from.
struct Entry {
    goal: Goal,
    host: String,
}

fn read_index(dir: &Path) -> HashMap<String, Entry> {
    let text = std::fs::read_to_string(dir.join("index.tsv")).unwrap_or_default();
    parse_index(&text)
}

/// The lines of an `index.tsv`: file name, goal, URL; any later columns
/// (the dump may say inline, external or worker) are ignored.
fn parse_index(text: &str) -> HashMap<String, Entry> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let name = fields.next()?.trim();
            let goal = match fields.next()?.trim() {
                "module" => Goal::Module,
                _ => Goal::Script,
            };
            let url = fields.next().unwrap_or("").trim();
            let host = url
                .split("://")
                .nth(1)
                .and_then(|rest| rest.split(['/', '?', '#']).next())
                .unwrap_or("inline")
                .to_owned();
            Some((name.to_owned(), Entry { goal, host }))
        })
        .collect()
}

struct Outcome {
    path: PathBuf,
    /// Where the file came from: its URL's host and file name, or its
    /// path.
    name: String,
    bytes: usize,
    goal: Goal,
    error: Option<String>,
    stats: ParseStats,
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().map(|e| e.path()).collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect(&path, out);
        } else if path
            .extension()
            .is_some_and(|e| e == "js" || e == "mjs" || e == "cjs")
        {
            out.push(path);
        }
    }
}

fn goal_of(path: &Path) -> Goal {
    let name = path.to_string_lossy();
    if name.ends_with(".mjs") || name.ends_with(".module.js") {
        Goal::Module
    } else {
        Goal::Script
    }
}

/// A one-line view of the source around `offset`.
fn context(source: &str, offset: usize) -> String {
    let mut start = offset.saturating_sub(60).min(source.len());
    while !source.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = (offset + 60).min(source.len());
    while !source.is_char_boundary(end) {
        end += 1;
    }
    source
        .get(start..end)
        .unwrap_or("")
        .replace(['\n', '\r'], " ")
        .chars()
        .map(|c| if c.is_ascii() { c } else { '?' })
        .collect()
}

fn parse_one(path: &Path, entry: Option<&Entry>, show_context: bool) -> Outcome {
    let bytes = std::fs::read(path).unwrap_or_default();
    let source = String::from_utf8_lossy(&bytes);
    let goal = entry.map_or_else(|| goal_of(path), |e| e.goal);
    let (result, stats) = parse_with_stats(&source, goal, BUDGET);
    let (goal, result, stats) = match (&result, goal) {
        (Err(_), Goal::Script) if entry.is_none() => {
            let (again, module_stats) = parse_with_stats(&source, Goal::Module, BUDGET);
            if again.is_ok() {
                (Goal::Module, again, module_stats)
            } else {
                (goal, result, stats)
            }
        }
        _ => (goal, result, stats),
    };
    let error = result.err().map(|e| {
        let (line, column) = LineIndex::new(&source).location(&source, e.start);
        let mut text = format!("{} at byte {} ({line}:{column})", e.message, e.start);
        if show_context {
            text.push_str("\n      ");
            text.push_str(&context(&source, e.start as usize));
        }
        text
    });
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name = match entry {
        Some(e) => format!("{} ({file_name})", e.host),
        None => path.display().to_string(),
    };
    Outcome {
        path: path.to_owned(),
        name,
        bytes: bytes.len(),
        goal,
        error,
        stats,
    }
}

/// The median time of `runs` parses of `path`, in seconds.
fn time_parse(path: &Path, goal: Goal, runs: usize) -> f64 {
    let bytes = std::fs::read(path).unwrap_or_default();
    let source = String::from_utf8_lossy(&bytes);
    let mut times: Vec<f64> = (0..runs)
        .map(|_| {
            let start = Instant::now();
            let (result, _) = parse_with_stats(&source, goal, BUDGET);
            std::hint::black_box(&result);
            start.elapsed().as_secs_f64()
        })
        .collect();
    times.sort_by(f64::total_cmp);
    times.get(times.len() / 2).copied().unwrap_or(0.0)
}

fn goal_name(goal: Goal) -> &'static str {
    match goal {
        Goal::Module => "module",
        Goal::Script => "script",
    }
}

fn run(dir: PathBuf, top: usize, show_context: bool, compile: bool) {
    let mut files = Vec::new();
    collect(&dir, &mut files);
    let index = read_index(&dir);
    let outcomes: Vec<Outcome> = files
        .iter()
        .map(|p| {
            let key = p.file_name().map(|n| n.to_string_lossy().into_owned());
            let entry = key.and_then(|k| index.get(&k));
            parse_one(p, entry, show_context)
        })
        .collect();
    let total_bytes: usize = outcomes.iter().map(|o| o.bytes).sum();
    let failed: Vec<&Outcome> = outcomes.iter().filter(|o| o.error.is_some()).collect();
    let modules = outcomes.iter().filter(|o| o.goal == Goal::Module).count();
    println!(
        "{} files ({} modules), {:.1} MB; {} fail",
        outcomes.len(),
        modules,
        total_bytes as f64 / 1e6,
        failed.len()
    );
    for o in &failed {
        println!(
            "FAIL {} ({} bytes, {}): {}",
            o.name,
            o.bytes,
            goal_name(o.goal),
            o.error.as_deref().unwrap_or("")
        );
    }

    let mut by_depth: Vec<&Outcome> = outcomes.iter().collect();
    by_depth.sort_by_key(|o| std::cmp::Reverse(o.stats.max_depth));
    println!("\ndeepest nesting (levels, peak stack, stack per level):");
    for o in by_depth.iter().take(top) {
        let per_level = o.stats.peak_stack / o.stats.max_depth.max(1) as usize;
        println!(
            "  {:>5} levels {:>9} bytes {:>6} B/level  {}",
            o.stats.max_depth, o.stats.peak_stack, per_level, o.name
        );
    }
    let peak = outcomes
        .iter()
        .map(|o| o.stats.peak_stack)
        .max()
        .unwrap_or(0);
    let over: Vec<&Outcome> = outcomes
        .iter()
        .filter(|o| o.stats.peak_stack > DEFAULT_STACK_BUDGET)
        .collect();
    println!(
        "most stack a parse took: {peak} bytes; default budget {DEFAULT_STACK_BUDGET} bytes; {} file(s) over it",
        over.len()
    );
    for o in &over {
        println!("  over: {} ({} bytes)", o.name, o.stats.peak_stack);
    }

    let mut by_size: Vec<&Outcome> = outcomes.iter().filter(|o| o.error.is_none()).collect();
    by_size.sort_by_key(|o| std::cmp::Reverse(o.bytes));
    println!("\nparse time of the largest files (median of 7):");
    for o in by_size.iter().take(top) {
        let seconds = time_parse(&o.path, o.goal, 7);
        println!(
            "  {:>9} bytes {:>8.2} ms {:>7.1} MB/s  {}",
            o.bytes,
            seconds * 1e3,
            o.bytes as f64 / 1e6 / seconds.max(1e-9),
            o.name
        );
    }
    if compile {
        compile_all(&outcomes, top);
    }
}

/// The median time of `runs` parses and compiles of `source`, in seconds.
fn time_compile(source: &str, goal: Goal, runs: usize) -> f64 {
    let mut times: Vec<f64> = (0..runs)
        .map(|_| {
            let start = Instant::now();
            let result = compile_with_stack(source, goal, BUDGET);
            std::hint::black_box(&result);
            start.elapsed().as_secs_f64()
        })
        .collect();
    times.sort_by(f64::total_cmp);
    times.get(times.len() / 2).copied().unwrap_or(0.0)
}

fn compile_all(outcomes: &[Outcome], top: usize) {
    let mut compiled = Vec::new();
    let mut refused: HashMap<&'static str, usize> = HashMap::new();
    let mut broken = Vec::new();
    for o in outcomes.iter().filter(|o| o.error.is_none()) {
        let bytes = std::fs::read(&o.path).unwrap_or_default();
        let source = String::from_utf8_lossy(&bytes);
        match compile_with_stack(&source, o.goal, BUDGET) {
            Ok(unit) => match unit.validate() {
                Ok(()) => compiled.push(o),
                Err(e) => broken.push(format!("{}: an invalid unit: {e}", o.name)),
            },
            Err(CompileError::Unsupported { feature, .. }) => {
                *refused.entry(feature).or_default() += 1
            }
            Err(CompileError::Syntax(e)) => {
                broken.push(format!("{}: {} at byte {}", o.name, e.message, e.start));
            }
        }
    }
    let mut refused: Vec<_> = refused.into_iter().collect();
    refused.sort_by_key(|(f, n)| (std::cmp::Reverse(*n), *f));
    println!(
        "
compile: {} compiled into valid units, {} refused, {} broken",
        compiled.len(),
        refused.iter().map(|(_, n)| n).sum::<usize>(),
        broken.len()
    );
    for (feature, n) in &refused {
        println!("  refused for {feature}: {n}");
    }
    for b in &broken {
        println!("BROKEN {b}");
    }
    compiled.sort_by_key(|o| std::cmp::Reverse(o.bytes));
    println!(
        "
parse and compile time of the largest that compiled (median of 7):"
    );
    for o in compiled.iter().take(top) {
        let bytes = std::fs::read(&o.path).unwrap_or_default();
        let source = String::from_utf8_lossy(&bytes);
        let parse = time_parse(&o.path, o.goal, 7);
        let both = time_compile(&source, o.goal, 7);
        println!(
            "  {:>9} bytes parse {:>8.2} ms, parse and compile {:>8.2} ms ({:.2}x)  {}",
            o.bytes,
            parse * 1e3,
            both * 1e3,
            both / parse.max(1e-9),
            o.name
        );
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mut dir = None;
    let mut top = 10;
    let mut show_context = false;
    let mut compile = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--context" => show_context = true,
            "--compile" => compile = true,
            "--top" => top = args.next().and_then(|n| n.parse().ok()).unwrap_or(top),
            _ => dir = Some(PathBuf::from(arg)),
        }
    }
    let Some(dir) = dir else {
        eprintln!("usage: parse_corpus <dir> [--top <n>] [--context] [--compile]");
        std::process::exit(2);
    };
    let worker = std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(move || run(dir, top, show_context, compile));
    match worker.map(|w| w.join()) {
        Ok(Ok(())) => {}
        _ => {
            eprintln!("parse_corpus: the worker failed");
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_index_reads_three_columns_and_ignores_more() {
        let index = parse_index(concat!(
            "a.js\tscript\thttps://www.example.com/app.js?v=1\n",
            "b.js\tmodule\thttps://cdn.example.org:8443/m.js\tworker\textra\n",
            "c.js\tscript\n",
            "\n",
        ));
        let a = index.get("a.js").unwrap();
        assert_eq!((a.goal, a.host.as_str()), (Goal::Script, "www.example.com"));
        let b = index.get("b.js").unwrap();
        assert_eq!(
            (b.goal, b.host.as_str()),
            (Goal::Module, "cdn.example.org:8443")
        );
        let c = index.get("c.js").unwrap();
        assert_eq!((c.goal, c.host.as_str()), (Goal::Script, "inline"));
        assert_eq!(index.len(), 3);
    }
}
