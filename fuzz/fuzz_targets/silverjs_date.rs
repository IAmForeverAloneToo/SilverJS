//! `Date.parse` on any text: the bytes, as text, are parsed in UTC, in a
//! fixed zone and in one with a daylight change. The result must be NaN
//! or an integer time value, found in bounded time and without a panic;
//! a time value's `toISOString` must read back to it, and its
//! `toUTCString` and `toString` to its whole second (for years 100
//! to 9999: a smaller year reads as 19xx or 20xx, as it does on the
//! reference browser). Once a run, every string of the recorded table
//! (the dates probe's corpus, symbols and spaces, Chrome 158 in UTC)
//! must parse to what the reference browser gave, and so must an input
//! that is one (its ASCII strings are this target's seeds).

#![no_main]

use std::fmt::Write;
use std::sync::Once;
use std::time::{Duration, Instant};

use libfuzzer_sys::fuzz_target;
use silverjs::agent::{Agent, AgentOptions, Outcome, TimeZone};

include!("silverjs_date_table.rs");

/// The text a run takes at most.
const MAX_CHARS: usize = 300;

/// The longest a run may take.
const LIMIT: Duration = Duration::from_secs(2);

/// A fixed zone: UTC+05:30.
struct Fixed;

impl TimeZone for Fixed {
    fn offset(&self, _: f64) -> f64 {
        5.5 * 3_600_000.0
    }

    fn name(&self, _: f64) -> String {
        "India Standard Time".into()
    }
}

/// A zone an hour ahead of UTC from April to September (by the UTC
/// month), with a gap and a repeated hour at each change.
struct Summer;

impl Summer {
    fn summer(t: f64) -> bool {
        let day = (t / 86_400_000.0).floor();
        // The day of the year, near enough for a test zone.
        let in_year = (day + 719_468.0).rem_euclid(365.2425);
        (90.0..273.0).contains(&in_year)
    }
}

impl TimeZone for Summer {
    fn offset(&self, t: f64) -> f64 {
        if Summer::summer(t) { 3_600_000.0 } else { 0.0 }
    }

    fn name(&self, t: f64) -> String {
        if Summer::summer(t) {
            "Summer Time".into()
        } else {
            "Winter Time".into()
        }
    }
}

/// `s` as a JavaScript string literal.
fn literal(s: &str) -> String {
    let mut out = String::from("'");
    for unit in s.encode_utf16() {
        let _ = write!(out, "\\u{unit:04x}");
    }
    out.push('\'');
    out
}

/// The checks, in script: "ok", or what failed.
const CHECK: &str = r#"
var t = Date.parse(s), bad = [];
if (typeof t !== 'number') bad.push('type');
if (t === t) {
  if (t !== Math.trunc(t) || Math.abs(t) > 8.64e15 || Object.is(t, -0)) bad.push('range ' + t);
  var d = new Date(t), sec = t - ((t % 1000) + 1000) % 1000, y = d.getUTCFullYear();
  if (Date.parse(d.toISOString()) !== t) bad.push('iso ' + d.toISOString());
  if (y >= 100 && y <= 9999 && d.getFullYear() >= 100 && d.getFullYear() <= 9999) {
    if (Date.parse(d.toUTCString()) !== sec) bad.push('utc ' + d.toUTCString());
    if (Date.parse(d.toString()) !== sec) bad.push('string ' + d.toString());
  }
}
bad.length ? bad.join('; ') : 'ok ' + t
"#;

fn run(source: &str, zone: Option<Box<dyn TimeZone>>) -> String {
    let mut agent = Agent::new(AgentOptions::default());
    agent.set_clock(Some(Box::new(|| 1_795_435_200_000.0)));
    agent.set_time_zone(zone);
    agent.scope(|scope| match scope.run_script(source) {
        Ok(Outcome::Value(v)) => scope.to_text(v).unwrap_or_default(),
        Ok(Outcome::Threw(v)) => format!("threw {}", scope.to_text(v).unwrap_or_default()),
        Ok(Outcome::Terminated) => "terminated".to_owned(),
        Err(e) => format!("did not compile: {e}"),
    })
}

/// A time value as `String` gives it.
fn shown(t: f64) -> String {
    if t.is_nan() {
        "NaN".to_owned()
    } else {
        format!("{t}")
    }
}

static TABLE_CHECKED: Once = Once::new();

/// Every string of the recorded table, parsed in UTC.
fn check_table() {
    let mut source = String::from("[");
    for (s, _) in TABLE {
        source.push_str(&literal(s));
        source.push(',');
    }
    source.push_str("].map(function (s) { return String(Date.parse(s)); }).join('|')");
    let got = run(&source, None);
    let got: Vec<&str> = got.split('|').collect();
    assert_eq!(got.len(), TABLE.len(), "{got:?}");
    for ((s, want), got) in TABLE.iter().zip(got) {
        assert_eq!(got, shown(*want), "{s:?}");
    }
}

fuzz_target!(|data: &[u8]| {
    TABLE_CHECKED.call_once(check_table);
    let text: String = String::from_utf8_lossy(data)
        .chars()
        .take(MAX_CHARS)
        .collect();
    let source = format!("var s = {};{CHECK}", literal(&text));
    for zone in 0..3 {
        let tz: Option<Box<dyn TimeZone>> = match zone {
            0 => None,
            1 => Some(Box::new(Fixed)),
            _ => Some(Box::new(Summer)),
        };
        let start = Instant::now();
        let got = run(&source, tz);
        assert!(
            start.elapsed() < LIMIT,
            "slow: {:?}\n{text:?}",
            start.elapsed()
        );
        assert!(got.starts_with("ok "), "{got}\n{text:?}");
        if zone == 0
            && let Some(&(_, want)) = TABLE.iter().find(|(s, _)| *s == text)
        {
            assert_eq!(got.trim_start_matches("ok "), shown(want), "{text:?}");
        }
    }
});
