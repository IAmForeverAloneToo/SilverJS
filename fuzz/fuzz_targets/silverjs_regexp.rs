//! RegExp matching against an independent model: the bytes make small
//! patterns (literals, `.`, classes, groups, alternation, greedy and lazy
//! quantifiers, anchors, `\b`, lookarounds, backreferences) with flags
//! and subjects, the script runs each with `exec` under the `d` flag, and
//! where the regex crate (a test-only oracle) means the same thing every
//! capture's span must be what it gives. That is a pattern with no
//! lookaround, no backreference and no quantified body that can match
//! empty: the regex crate's leftmost-first matching is a backtracker's,
//! but JavaScript rejects an empty iteration and resets a quantified
//! body's captures, and the regex crate does neither. Every case must
//! run without a throw (each pattern is valid) and without a panic, to a
//! result or the end of the step budget (the backtracking bound).

#![no_main]

use std::cell::Cell;
use std::fmt::Write;
use std::rc::Rc;

use libfuzzer_sys::fuzz_target;
use silverjs::agent::{Agent, AgentOptions, Outcome};

/// The cases one input may make.
const MAX_CASES: usize = 8;
/// The interrupt hook's calls a run may take (each 10,000 steps).
const BUDGET: u32 = 300;
/// How deep a pattern nests.
const MAX_DEPTH: u32 = 4;

struct Bytes<'a> {
    data: &'a [u8],
    at: usize,
}

impl Bytes<'_> {
    fn next(&mut self) -> u8 {
        let b = self.data.get(self.at).copied().unwrap_or(0);
        self.at += 1;
        b
    }

    fn done(&self) -> bool {
        self.at >= self.data.len()
    }
}

/// A pattern being written out, and what it holds.
#[derive(Default)]
struct Pattern {
    text: String,
    groups: u32,
    /// Groups closed so far, which a backreference may name.
    closed: Vec<u32>,
    lookaround: bool,
    backref: bool,
    /// A quantified body that can match empty, or a capture inside a
    /// quantified body.
    loose_loop: bool,
}

const LETTERS: &[u8] = b"abcAB";

/// Writes an atom or a composite into `p`; whether it can match empty.
fn node(b: &mut Bytes, p: &mut Pattern, depth: u32, quantified: bool) -> bool {
    let kinds = if depth >= MAX_DEPTH { 6 } else { 13 };
    match b.next() % kinds {
        0..=2 => {
            p.text
                .push(char::from(LETTERS[usize::from(b.next()) % LETTERS.len()]));
            false
        }
        3 => {
            p.text.push('.');
            false
        }
        4 => {
            p.text.push('[');
            if b.next() % 3 == 0 {
                p.text.push('^');
            }
            for _ in 0..=b.next() % 3 {
                p.text.push(char::from(b"abc"[usize::from(b.next()) % 3]));
            }
            if b.next() % 4 == 0 {
                p.text.push_str("a-c");
            }
            p.text.push(']');
            false
        }
        5 => {
            p.text
                .push_str(["^", "$", r"\b", r"\B"][usize::from(b.next()) % 4]);
            true
        }
        6 | 7 => {
            let capture = b.next() % 2 == 0;
            p.text.push_str(if capture { "(" } else { "(?:" });
            let index = if capture {
                p.groups += 1;
                if quantified {
                    p.loose_loop = true;
                }
                Some(p.groups)
            } else {
                None
            };
            let empty = node(b, p, depth + 1, quantified);
            p.text.push(')');
            p.closed.extend(index);
            empty
        }
        8 => {
            let mut empty = false;
            for i in 0..2 + b.next() % 2 {
                if i > 0 {
                    p.text.push('|');
                }
                empty |= node(b, p, depth + 1, quantified);
            }
            empty
        }
        9 => {
            let mut empty = true;
            for _ in 0..2 + b.next() % 3 {
                empty &= node(b, p, depth + 1, quantified);
            }
            empty
        }
        10 | 11 => {
            // A quantified body, always a group so that the quantifier
            // reads as one.
            p.text.push_str("(?:");
            let body_empty = node(b, p, depth + 1, true);
            p.text.push(')');
            if body_empty {
                p.loose_loop = true;
            }
            let (m, n) = (u32::from(b.next() % 3), u32::from(b.next() % 3));
            let min = match b.next() % 6 {
                0 => {
                    p.text.push('*');
                    0
                }
                1 => {
                    p.text.push('+');
                    1
                }
                2 => {
                    p.text.push('?');
                    0
                }
                3 => {
                    let _ = write!(p.text, "{{{m}}}");
                    m
                }
                4 => {
                    let _ = write!(p.text, "{{{m},}}");
                    m
                }
                _ => {
                    let _ = write!(p.text, "{{{m},{}}}", m + n);
                    m
                }
            };
            if b.next() % 3 == 0 {
                p.text.push('?');
            }
            min == 0 || body_empty
        }
        _ => {
            if b.next() % 3 == 0
                && let Some(&g) = p.closed.get(usize::from(b.next()) % p.closed.len().max(1))
            {
                p.backref = true;
                let _ = write!(p.text, r"\{g}");
                return true;
            }
            p.lookaround = true;
            p.text
                .push_str(["(?=", "(?!", "(?<=", "(?<!"][usize::from(b.next()) % 4]);
            node(b, p, depth + 1, quantified);
            p.text.push(')');
            true
        }
    }
}

/// A subject: a few letters, or a long run with an ending that may fail.
fn subject(b: &mut Bytes) -> String {
    if b.next() % 8 == 0 {
        let n = 200 + usize::from(b.next()) * 8;
        let mut s = "a".repeat(n);
        s.push(char::from(LETTERS[usize::from(b.next()) % LETTERS.len()]));
        return s;
    }
    (0..b.next() % 16)
        .map(|_| char::from(LETTERS[usize::from(b.next()) % LETTERS.len()]))
        .collect()
}

/// What the regex crate gives: each group's span as `JSON.stringify`
/// writes the `indices` array, or `-`.
fn oracle(pattern: &str, ignore_case: bool, s: &str) -> Option<String> {
    let re = regex::Regex::new(&format!(
        "{}{pattern}",
        if ignore_case { "(?i)" } else { "" }
    ))
    .ok()?;
    let Some(caps) = re.captures(s) else {
        return Some("-".into());
    };
    let spans: Vec<String> = caps
        .iter()
        .map(|m| m.map_or_else(|| "null".into(), |m| format!("[{},{}]", m.start(), m.end())))
        .collect();
    Some(format!("[{}]", spans.join(",")))
}

fuzz_target!(|data: &[u8]| {
    let mut b = Bytes { data, at: 0 };
    let mut script = String::from(
        "var out = [];\n\
         function f(re, s) { try { var m = re.exec(s); return m ? JSON.stringify(m.indices) : '-'; } \
         catch (e) { return 'throw ' + e.name + ': ' + e.message; } }\n",
    );
    let mut cases = Vec::new();
    while cases.len() < MAX_CASES && !b.done() {
        let mut p = Pattern::default();
        node(&mut b, &mut p, 0, false);
        let flags = b.next();
        let ignore_case = flags & 1 != 0;
        let unicode = flags & 2 != 0;
        let s = subject(&mut b);
        let js_flags = format!(
            "d{}{}",
            if ignore_case { "i" } else { "" },
            if unicode { "u" } else { "" }
        );
        let _ = writeln!(
            script,
            "out.push(f(new RegExp({:?}, {js_flags:?}), {s:?}));",
            p.text
        );
        let expect = if p.lookaround || p.backref || p.loose_loop {
            None
        } else {
            oracle(&p.text, ignore_case, &s)
        };
        cases.push((p.text, js_flags, s, expect));
    }
    if cases.is_empty() {
        return;
    }
    script.push_str("out.join('\\n')");

    let mut agent = Agent::new(AgentOptions::default());
    let calls = Rc::new(Cell::new(0u32));
    let hook = Rc::clone(&calls);
    agent.set_interrupt(Some(Box::new(move || {
        hook.set(hook.get() + 1);
        hook.get() > BUDGET
    })));
    let got = agent.scope(|scope| match scope.run_script(&script) {
        Ok(Outcome::Value(v)) => Some(scope.to_text(v).unwrap_or_default()),
        Ok(Outcome::Threw(e)) => {
            panic!("the script threw {}", scope.to_text(e).unwrap_or_default())
        }
        Ok(Outcome::Terminated) => None,
        Err(e) => panic!("the script did not compile: {e}\n{script}"),
    });
    // The end of the budget is the bound working, not an answer.
    let Some(got) = got else { return };
    let lines: Vec<&str> = got.split('\n').collect();
    assert_eq!(lines.len(), cases.len(), "{got}");
    for ((pattern, flags, s, expect), line) in cases.iter().zip(lines) {
        assert!(
            !line.starts_with("throw"),
            "/{pattern}/{flags} on {s:?}: {line}"
        );
        if let Some(want) = expect {
            assert_eq!(line, want, "/{pattern}/{flags} on {s:?}");
        }
    }
});
