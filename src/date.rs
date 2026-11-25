//! Time values (ECMA-262 section 21.4.1): the days and fields of a time
//! value, MakeTime, MakeDay, MakeDate and TimeClip, local time through
//! the embedder's [`TimeZone`] (UTC without one), the string forms
//! `Date` gives, and `Date.parse`'s reading: the Date Time String Format
//! (section 21.4.1.32), and past it the other forms the reference
//! browser reads (the numbers probe, Chrome 158, in two time zones).

use crate::agent::TimeZone;

pub(crate) const MS_PER_DAY: f64 = 86_400_000.0;
const MS_PER_HOUR: f64 = 3_600_000.0;
const MS_PER_MINUTE: f64 = 60_000.0;
const MS_PER_SECOND: f64 = 1000.0;
/// The largest time value's magnitude (section 21.4.1.31).
const MAX_TIME: f64 = 8.64e15;
/// The years MakeDay works within: past them no time value is near.
const MAX_YEAR: f64 = 1_000_000.0;

const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// Day (section 21.4.1.3).
pub(crate) fn day(t: f64) -> f64 {
    (t / MS_PER_DAY).floor()
}

/// The days from 1970-01-01 to the proleptic Gregorian `y`-`m`-`d`
/// (`m` from 1), by whole 400-year eras.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The year, month (from 1) and day of day number `z`.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// A finite time value's fields (sections 21.4.1.4 to 21.4.1.14).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Fields {
    pub year: i64,
    /// From 0.
    pub month: i64,
    /// From 1.
    pub date: i64,
    pub hour: i64,
    pub minute: i64,
    pub second: i64,
    pub ms: i64,
    /// 0 for Sunday.
    pub weekday: i64,
}

pub(crate) fn fields(t: f64) -> Fields {
    let d = day(t);
    let within = (t - d * MS_PER_DAY) as i64;
    let d = d as i64;
    let (year, m, date) = civil_from_days(d);
    Fields {
        year,
        month: m - 1,
        date,
        hour: within / 3_600_000,
        minute: within / 60_000 % 60,
        second: within / 1000 % 60,
        ms: within % 1000,
        weekday: (d + 4).rem_euclid(7),
    }
}

/// MakeTime (section 21.4.1.27).
pub(crate) fn make_time(hour: f64, min: f64, sec: f64, ms: f64) -> f64 {
    if !(hour.is_finite() && min.is_finite() && sec.is_finite() && ms.is_finite()) {
        return f64::NAN;
    }
    hour.trunc() * MS_PER_HOUR
        + min.trunc() * MS_PER_MINUTE
        + sec.trunc() * MS_PER_SECOND
        + ms.trunc()
}

/// MakeDay (section 21.4.1.28): NaN for a year no time value is near.
pub(crate) fn make_day(year: f64, month: f64, date: f64) -> f64 {
    if !(year.is_finite() && month.is_finite() && date.is_finite()) {
        return f64::NAN;
    }
    let (y, m, dt) = (year.trunc(), month.trunc(), date.trunc());
    let ym = y + (m / 12.0).floor();
    if ym.abs() > MAX_YEAR {
        return f64::NAN;
    }
    let mn = m.rem_euclid(12.0);
    days_from_civil(ym as i64, mn as i64 + 1, 1) as f64 + dt - 1.0
}

/// MakeDate (section 21.4.1.29).
pub(crate) fn make_date(day: f64, time: f64) -> f64 {
    let tv = day * MS_PER_DAY + time;
    if tv.is_finite() { tv } else { f64::NAN }
}

/// TimeClip (section 21.4.1.31).
pub(crate) fn time_clip(t: f64) -> f64 {
    if !t.is_finite() || t.abs() > MAX_TIME {
        return f64::NAN;
    }
    t.trunc() + 0.0
}

/// The local time zone's offset at time value `t`, in milliseconds.
pub(crate) fn offset(tz: Option<&dyn TimeZone>, t: f64) -> f64 {
    tz.map_or(0.0, |z| z.offset(t))
}

/// LocalTime (section 21.4.1.25).
pub(crate) fn local_time(tz: Option<&dyn TimeZone>, t: f64) -> f64 {
    t + offset(tz, t)
}

/// UTC (section 21.4.1.26): the time value of local time `t`. Of two
/// (a repeated hour) the earlier; in a skipped hour, the offset from
/// before the transition.
pub(crate) fn utc(tz: Option<&dyn TimeZone>, t: f64) -> f64 {
    if !t.is_finite() {
        return f64::NAN;
    }
    let Some(z) = tz else {
        return t;
    };
    let before = z.offset(t - MS_PER_DAY);
    let after = z.offset(t + MS_PER_DAY);
    let (a, b) = (t - before, t - after);
    let fits = |u: f64| u + z.offset(u) == t;
    match (fits(a), fits(b)) {
        (true, true) => a.min(b),
        (true, false) => a,
        (false, true) => b,
        (false, false) => a,
    }
}

/// A year as the string forms write it: four digits at least, and a
/// minus sign before a negative one.
fn year_text(y: i64) -> String {
    if y >= 0 {
        format!("{y:04}")
    } else {
        format!("-{:04}", -y)
    }
}

/// DateString (section 21.4.4.41.2): "Sat Jul 04 2020".
pub(crate) fn date_string(f: &Fields) -> String {
    format!(
        "{} {} {:02} {}",
        DAYS.get(f.weekday as usize).copied().unwrap_or(""),
        MONTHS.get(f.month as usize).copied().unwrap_or(""),
        f.date,
        year_text(f.year)
    )
}

/// TimeString (section 21.4.4.41.1): "12:30:45".
pub(crate) fn time_string(f: &Fields) -> String {
    format!("{:02}:{:02}:{:02}", f.hour, f.minute, f.second)
}

/// TimeZoneString (section 21.4.4.41.3): "GMT-0400 (Eastern Daylight
/// Time)", the offset's whole minutes.
pub(crate) fn zone_string(offset: f64, name: &str) -> String {
    let sign = if offset >= 0.0 { '+' } else { '-' };
    let minutes = (offset.abs() / MS_PER_MINUTE).floor() as i64;
    format!("GMT{sign}{:02}{:02} ({name})", minutes / 60, minutes % 60)
}

/// `toISOString`'s form of a finite time value (section 21.4.4.36).
pub(crate) fn iso_string(t: f64) -> String {
    let f = fields(t);
    let year = if (0..=9999).contains(&f.year) {
        format!("{:04}", f.year)
    } else if f.year < 0 {
        format!("-{:06}", -f.year)
    } else {
        format!("+{:06}", f.year)
    };
    format!(
        "{year}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        f.month + 1,
        f.date,
        f.hour,
        f.minute,
        f.second,
        f.ms
    )
}

/// `toUTCString`'s form (section 21.4.4.43): "Sat, 04 Jul 2020 12:30:45
/// GMT".
pub(crate) fn utc_string(t: f64) -> String {
    let f = fields(t);
    format!(
        "{}, {:02} {} {} {} GMT",
        DAYS.get(f.weekday as usize).copied().unwrap_or(""),
        f.date,
        MONTHS.get(f.month as usize).copied().unwrap_or(""),
        year_text(f.year),
        time_string(&f)
    )
}

/// The locale forms, en-US's until `Intl` comes: "7/4/2020" and
/// "12:30:45 PM".
pub(crate) fn locale_date(f: &Fields) -> String {
    // No era is written: year 0 is 1 (before Christ), -1 is 2.
    let year = if f.year <= 0 { 1 - f.year } else { f.year };
    format!("{}/{}/{}", f.month + 1, f.date, year)
}

pub(crate) fn locale_time(f: &Fields) -> String {
    let h = match f.hour % 12 {
        0 => 12,
        h => h,
    };
    let half = if f.hour < 12 { "AM" } else { "PM" };
    format!("{h}:{:02}:{:02} {half}", f.minute, f.second)
}

/// `Date.parse` (section 21.4.3.2): the time value `s` names, or NaN.
pub(crate) fn parse(s: &[u16], tz: Option<&dyn TimeZone>) -> f64 {
    match iso(s, tz) {
        Iso::Value(t) => t,
        Iso::Not(from) => legacy(s.get(from..).unwrap_or_default(), tz),
    }
}

/// What the Date Time String Format made of a string.
enum Iso {
    /// Its time value (NaN for one it refused outright).
    Value(f64),
    /// Not that format: the other forms are read from this index (past
    /// "-000000", a year the format forbids and the reference browser
    /// skips).
    Not(usize),
}

/// A cursor over a string's code units.
struct Cursor<'a> {
    s: &'a [u16],
    at: usize,
}

impl Cursor<'_> {
    fn peek(&self) -> Option<u16> {
        self.s.get(self.at).copied()
    }

    fn eat(&mut self, c: u8) -> bool {
        if self.peek() == Some(u16::from(c)) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    /// Exactly `n` digits' value.
    fn digits(&mut self, n: usize) -> Option<i64> {
        let mut v = 0;
        for _ in 0..n {
            let d = self.peek().filter(|c| (0x30..=0x39).contains(c))?;
            v = v * 10 + i64::from(d - 0x30);
            self.at += 1;
        }
        Some(v)
    }

    fn done(&self) -> bool {
        self.at >= self.s.len()
    }
}

/// The Date Time String Format (section 21.4.1.32): a date alone is UTC,
/// a date and time without an offset local. A broken offset is refused
/// outright; anything else that is not the format is the other forms'.
fn iso(s: &[u16], tz: Option<&dyn TimeZone>) -> Iso {
    let mut c = Cursor { s, at: 0 };
    let year = if c.peek() == Some(u16::from(b'+')) || c.peek() == Some(u16::from(b'-')) {
        let negative = c.eat(b'-');
        if !negative {
            c.at += 1;
        }
        let Some(y) = c.digits(6) else {
            return Iso::Not(0);
        };
        if negative && y == 0 {
            return Iso::Not(7);
        }
        if negative { -y } else { y }
    } else {
        match c.digits(4) {
            Some(y) => y,
            None => return Iso::Not(0),
        }
    };
    let (mut month, mut date) = (1, 1);
    if c.eat(b'-') {
        match c.digits(2) {
            Some(m) if (1..=12).contains(&m) => month = m,
            _ => return Iso::Not(0),
        }
        if c.eat(b'-') {
            match c.digits(2) {
                Some(d) if (1..=31).contains(&d) => date = d,
                _ => return Iso::Not(0),
            }
        }
    }
    let day = make_day(year as f64, (month - 1) as f64, date as f64);
    if c.done() {
        return Iso::Value(time_clip(make_date(day, 0.0)));
    }
    if !(c.eat(b'T') || c.eat(b't')) {
        return Iso::Not(0);
    }
    // From the time on, what is not the format is refused.
    let refused = Iso::Value(f64::NAN);
    let Some(hour) = c.digits(2) else {
        return refused;
    };
    if !c.eat(b':') {
        return refused;
    }
    let Some(minute) = c.digits(2) else {
        return refused;
    };
    let (mut second, mut ms) = (0, 0);
    if c.eat(b':') {
        let Some(sec) = c.digits(2) else {
            return refused;
        };
        second = sec;
        if c.eat(b'.') {
            let start = c.at;
            while c.peek().is_some_and(|d| (0x30..=0x39).contains(&d)) {
                if c.at - start < 3 {
                    ms = ms * 10 + i64::from(c.peek().unwrap_or(0x30) - 0x30);
                }
                c.at += 1;
            }
            match c.at - start {
                0 => return refused,
                1 => ms *= 100,
                2 => ms *= 10,
                _ => {}
            }
        }
    }
    if minute > 59 || second > 59 || hour > 24 || (hour == 24 && (minute, second, ms) != (0, 0, 0))
    {
        return refused;
    }
    let time = make_time(hour as f64, minute as f64, second as f64, ms as f64);
    let local = make_date(day, time);
    if c.done() {
        return Iso::Value(time_clip(utc(tz, local)));
    }
    if (c.eat(b'Z') || c.eat(b'z')) && c.done() {
        return Iso::Value(time_clip(local));
    }
    let sign = match c.peek() {
        Some(0x2b) => 1.0,
        Some(0x2d) => -1.0,
        _ => return refused,
    };
    c.at += 1;
    let Some(oh) = c.digits(2) else {
        return refused;
    };
    c.eat(b':');
    let Some(om) = c.digits(2) else {
        return refused;
    };
    if !c.done() || oh > 23 || om > 59 {
        return refused;
    }
    let off = sign * (oh as f64 * MS_PER_HOUR + om as f64 * MS_PER_MINUTE);
    Iso::Value(time_clip(local - off))
}

/// What a token of the other forms is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// A number's value (saturating) and digits.
    Number {
        value: u64,
        digits: usize,
    },
    /// A word: its letters' index range.
    Word(usize, usize),
    Colon,
    Dot,
    /// `+` (false) or `-` (true).
    Sign(bool),
    /// A `)` outside a comment.
    Close,
}

/// A token, whether a digit or letter came straight before it, and the
/// index past it.
#[derive(Debug, Clone, Copy)]
struct Token {
    kind: Kind,
    glued: bool,
    end: usize,
}

fn is_digit(c: u16) -> bool {
    (0x30..=0x39).contains(&c)
}

fn is_letter(c: u16) -> bool {
    (u16::from(b'a')..=u16::from(b'z')).contains(&(c | 0x20))
}

/// Space, as the forms take it: ASCII's, no-break space, the BOM and
/// the space separators (the line separators are not).
fn is_space(c: u16) -> bool {
    matches!(
        c,
        0x09..=0x0d | 0x20 | 0xa0 | 0x1680 | 0x202f | 0x205f | 0x3000 | 0xfeff
    ) || (0x2000..=0x200a).contains(&c)
}

/// Part of a word: a letter, `[\]^_` and the backquote, `{|}~`, DEL,
/// and every character past ASCII but space (the symbols probe).
fn is_word(c: u16) -> bool {
    is_letter(c)
        || (0x5b..=0x60).contains(&c)
        || (0x7b..=0x7f).contains(&c)
        || (c >= 0x80 && !is_space(c))
}

/// The tokens of `s`, to a NUL if it has one. The other symbols and
/// control characters are skipped; a comment runs to its `)` or the end.
fn tokens(s: &[u16]) -> Vec<Token> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(&c) = s.get(i).filter(|&&c| c != 0) {
        let start = i;
        i += 1;
        let kind = if is_digit(c) {
            let mut value: u64 = 0;
            i = start;
            while let Some(&d) = s.get(i).filter(|&&d| is_digit(d)) {
                value = value.saturating_mul(10).saturating_add(u64::from(d - 0x30));
                i += 1;
            }
            Kind::Number {
                value,
                digits: i - start,
            }
        } else if is_word(c) {
            while s.get(i).is_some_and(|&d| is_word(d)) {
                i += 1;
            }
            Kind::Word(start, i)
        } else if is_space(c) {
            continue;
        } else {
            match c {
                0x3a => Kind::Colon,
                0x2e => Kind::Dot,
                0x2b => Kind::Sign(false),
                0x2d => Kind::Sign(true),
                0x29 => Kind::Close,
                0x28 => {
                    let mut depth = 1u32;
                    while let Some(&d) = s.get(i) {
                        i += 1;
                        if d == 0x28 {
                            depth += 1;
                        } else if d == 0x29 {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                    }
                    continue;
                }
                _ => continue,
            }
        };
        let glued = start
            .checked_sub(1)
            .and_then(|p| s.get(p))
            .is_some_and(|&p| is_digit(p) || is_letter(p));
        out.push(Token {
            kind,
            glued,
            end: i,
        });
    }
    out
}

/// A month's name: one whose first three letters are its.
fn month_named(w: &str) -> Option<i64> {
    const NAMES: [&str; 12] = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    let prefix = w.get(..3)?;
    NAMES.iter().position(|n| *n == prefix).map(|m| m as i64)
}

/// A zone's name: its offset in hours.
fn zone_named(w: &str) -> Option<f64> {
    Some(match w {
        "ut" | "utc" | "gmt" | "z" => 0.0,
        "edt" => -4.0,
        "est" | "cdt" => -5.0,
        "cst" | "mdt" => -6.0,
        "mst" | "pdt" => -7.0,
        "pst" => -8.0,
        _ => return None,
    })
}

/// A time's fraction of a second, its first three digits in
/// milliseconds.
fn millis(value: u64, digits: usize) -> u64 {
    if digits <= 3 {
        value * 10u64.pow(3 - digits as u32)
    } else {
        let cut = u32::try_from(digits - 3).unwrap_or(u32::MAX);
        10u64.checked_pow(cut).map_or(0, |p| value / p)
    }
}

/// The other forms (the reference browser's own, as the probes show
/// them):
///
/// - Words: a month's name (a later one replacing an earlier), AM and PM
///   after a time, a zone's name once a number is read; any other word
///   is skipped before the first number and refuses the string after.
/// - A time: numbers joined by colons, hours first, then minutes and
///   seconds below 60 (60 or more is left to the date), a fourth part or
///   a fraction after a dot; it ends at the end, at space, at `Z` or at
///   a sign.
/// - An offset after a time or a zone's name: a sign and hours (two
///   digits at most, then `:mm`) or `hhmm`; it replaces the zone's.
/// - The date: up to three numbers, in the order their values tell. A
///   number past 31, or 0, is a year; with a month's name the others
///   are its day; without, month, day, then year. A year below 50 is
///   20xx, below 100 19xx; none is 2001.
/// - A `-` straight after a number or word separates; any other sign
///   after a number, a fourth date number, a stray colon or `)` after a
///   number refuses the string. Other characters are skipped.
///
/// Local time unless a zone is named.
fn legacy(s: &[u16], tz: Option<&dyn TimeZone>) -> f64 {
    let toks = tokens(s);
    let at = |k: usize| toks.get(k).map(|t| t.kind);
    let mut nums: Vec<u64> = Vec::new();
    let mut month = None;
    let (mut hour, mut minute, mut second, mut ms) = (None::<u64>, 0u64, 0u64, 0u64);
    let mut zone: Option<f64> = None;
    let mut pm = None;
    let mut read_number = false;
    let mut k = 0;
    while let Some(&tok) = toks.get(k) {
        k += 1;
        match tok.kind {
            Kind::Number { value, .. } => {
                read_number = true;
                // A colon straight after a number starts a time.
                if hour.is_none()
                    && toks
                        .get(k)
                        .is_some_and(|t| t.kind == Kind::Colon && t.glued)
                {
                    hour = Some(value);
                    k += 1;
                    // Minutes, then seconds ("h::s" has no minutes),
                    // each below 60: a number past 59 is left to the
                    // date. `last_number`: the time ended on a number.
                    let mut last_number = false;
                    let mut seconds_next = false;
                    if at(k) == Some(Kind::Colon) {
                        k += 1;
                        seconds_next = true;
                    } else if let Some(Kind::Number { value: v, .. }) = at(k)
                        && v < 60
                    {
                        minute = v;
                        k += 1;
                        last_number = true;
                        if at(k) == Some(Kind::Colon) {
                            k += 1;
                            last_number = false;
                            seconds_next = true;
                            // "h:m::" is refused, where "h::s" is not.
                            if at(k) == Some(Kind::Colon) {
                                return f64::NAN;
                            }
                        }
                    }
                    if seconds_next
                        && let Some(Kind::Number { value: v, .. }) = at(k)
                        && v < 60
                    {
                        second = v;
                        k += 1;
                        last_number = true;
                        // A fourth part, or a fraction after a dot.
                        if at(k) == Some(Kind::Colon)
                            && let Some(Kind::Number { value: v, .. }) = at(k + 1)
                        {
                            ms = v;
                            k += 2;
                        } else if at(k) == Some(Kind::Dot) {
                            let Some(Kind::Number { value: f, digits }) = at(k + 1) else {
                                return f64::NAN;
                            };
                            ms = millis(f, digits);
                            k += 2;
                        }
                    }
                    // A time ends at the end, at space, at a sign or at Z.
                    let end = toks.get(k - 1).map_or(0, |t| t.end);
                    let ends = match s.get(end) {
                        None | Some(0) => true,
                        Some(&c) => {
                            is_space(c)
                                || c == 0x2b
                                || c == 0x2d
                                || (c | 0x20 == u16::from(b'z')
                                    && !s.get(end + 1).is_some_and(|&d| d != 0 && is_word(d)))
                        }
                    };
                    if last_number && !ends {
                        return f64::NAN;
                    }
                    continue;
                }
                if nums.len() == 3 {
                    return f64::NAN;
                }
                nums.push(value);
            }
            Kind::Word(a, b) => {
                let w: String = char::decode_utf16(s.get(a..b).unwrap_or_default().iter().copied())
                    .map(|c| c.map_or('?', |c| c.to_ascii_lowercase()))
                    .collect();
                if let Some(m) = month_named(&w) {
                    month = Some(m);
                } else if (w == "am" || w == "pm") && hour.is_some() {
                    pm = Some(w == "pm");
                } else if let Some(h) = zone_named(&w).filter(|_| read_number) {
                    zone = Some(h * MS_PER_HOUR);
                } else if read_number {
                    return f64::NAN;
                }
            }
            Kind::Sign(negative) => {
                if hour.is_some() || zone.is_some() {
                    // An offset: hours (then `:mm`) or hhmm.
                    let Some(Kind::Number { value, digits }) = at(k) else {
                        return f64::NAN;
                    };
                    k += 1;
                    let minutes = match digits {
                        0..=2 => {
                            let mut m = value * 60;
                            if at(k) == Some(Kind::Colon)
                                && let Some(Kind::Number { value: mm, .. }) = at(k + 1)
                            {
                                m += mm;
                                k += 2;
                            }
                            m
                        }
                        3 | 4 => value / 100 * 60 + value % 100,
                        _ => return f64::NAN,
                    };
                    let off = minutes as f64 * MS_PER_MINUTE;
                    zone = Some(if negative { -off } else { off });
                    continue;
                }
                if negative && tok.glued {
                    continue;
                }
                if read_number {
                    return f64::NAN;
                }
            }
            Kind::Close if read_number => return f64::NAN,
            Kind::Close | Kind::Dot | Kind::Colon => {}
        }
    }
    let year_like = |v: u64| v > 31 || v == 0;
    let (year, mon, date) = match (month, nums.as_slice()) {
        (_, []) => return f64::NAN,
        (Some(m), &[a]) if year_like(a) => (Some(a), m + 1, 1),
        (Some(m), &[a]) => (None, m + 1, a),
        (Some(m), &[a, b, ..]) if year_like(a) => (Some(a), m + 1, b),
        (Some(m), &[a, b, ..]) => (Some(b), m + 1, a),
        (None, &[a]) if year_like(a) => (Some(a), 1, 1),
        (None, &[a]) => (None, a as i64, 1),
        (None, &[a, b]) if year_like(a) => (Some(a), b as i64, 1),
        (None, &[a, b]) => (None, a as i64, b),
        (None, &[a, b, c, ..]) if year_like(a) => (Some(a), b as i64, c),
        (None, &[a, b, c, ..]) => (Some(c), a as i64, b),
    };
    let year = match year {
        None => 2001.0,
        Some(v) if v < 50 => 2000.0 + v as f64,
        Some(v) if v < 100 => 1900.0 + v as f64,
        Some(v) => v as f64,
    };
    if !(1..=12).contains(&mon) || !(1..=31).contains(&date) {
        return f64::NAN;
    }
    let mut h = hour.unwrap_or(0);
    if let Some(pm) = pm {
        if h > 12 {
            return f64::NAN;
        }
        h = h % 12 + if pm { 12 } else { 0 };
    }
    if h > 24 || (h == 24 && (minute, second, ms) != (0, 0, 0)) {
        return f64::NAN;
    }
    let day = make_day(year, (mon - 1) as f64, date as f64);
    let local = make_date(
        day,
        make_time(h as f64, minute as f64, second as f64, ms as f64),
    );
    time_clip(match zone {
        Some(z) => local - z,
        None => utc(tz, local),
    })
}
