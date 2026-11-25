//! `Date` (ECMA-262 section 21.4, Annex B.2.3): the constructor, `now`,
//! `parse` and `UTC`, and the prototype's getters, setters and string
//! forms. A date's time value is in its internal slot; the arithmetic,
//! the forms and the parser are [`crate::date`]'s, the clock and the
//! local time zone the embedder's ([`Agent::set_clock`],
//! [`Agent::set_time_zone`]). `toTemporalInstant` comes with Temporal
//! (J9).

use super::{CONFIGURABLE, CONSTRUCTOR, Call, Entry, Name, What, Wk, entry, method};
use crate::agent::{Agent, TimeZone};
use crate::date::{
    self, Fields, date_string, fields, local_time, make_date, make_day, make_time, time_clip,
    time_string, utc, zone_string,
};
use crate::heap::ObjectCell;
use crate::interp::Res;
use crate::object::ObjectKind;
use crate::value::Value;

/// `Date`.
pub(crate) const DATE: &[Entry] = &[
    method("now", 0, now),
    method("parse", 1, parse),
    method("UTC", 7, utc_static),
];

/// `Date.prototype`, in the reference browser's order.
pub(crate) const DATE_PROTO: &[Entry] = &[
    CONSTRUCTOR,
    method("toString", 0, to_string),
    method("toDateString", 0, to_date_string),
    method("toTimeString", 0, to_time_string),
    method("toISOString", 0, to_iso_string),
    method("toUTCString", 0, to_utc_string),
    entry(
        Name::Str("toGMTString"),
        What::Alias("toUTCString"),
        super::HIDDEN,
    ),
    method("getDate", 0, get_date),
    method("setDate", 1, set_date),
    method("getDay", 0, get_day),
    method("getFullYear", 0, get_full_year),
    method("setFullYear", 3, set_full_year),
    method("getHours", 0, get_hours),
    method("setHours", 4, set_hours),
    method("getMilliseconds", 0, get_milliseconds),
    method("setMilliseconds", 1, set_milliseconds),
    method("getMinutes", 0, get_minutes),
    method("setMinutes", 3, set_minutes),
    method("getMonth", 0, get_month),
    method("setMonth", 2, set_month),
    method("getSeconds", 0, get_seconds),
    method("setSeconds", 2, set_seconds),
    method("getTime", 0, get_time),
    method("setTime", 1, set_time),
    method("getTimezoneOffset", 0, get_timezone_offset),
    method("getUTCDate", 0, get_utc_date),
    method("setUTCDate", 1, set_utc_date),
    method("getUTCDay", 0, get_utc_day),
    method("getUTCFullYear", 0, get_utc_full_year),
    method("setUTCFullYear", 3, set_utc_full_year),
    method("getUTCHours", 0, get_utc_hours),
    method("setUTCHours", 4, set_utc_hours),
    method("getUTCMilliseconds", 0, get_utc_milliseconds),
    method("setUTCMilliseconds", 1, set_utc_milliseconds),
    method("getUTCMinutes", 0, get_utc_minutes),
    method("setUTCMinutes", 3, set_utc_minutes),
    method("getUTCMonth", 0, get_utc_month),
    method("setUTCMonth", 2, set_utc_month),
    method("getUTCSeconds", 0, get_utc_seconds),
    method("setUTCSeconds", 2, set_utc_seconds),
    method("valueOf", 0, get_time),
    method("getYear", 0, get_year),
    method("setYear", 1, set_year),
    method("toJSON", 1, to_json),
    method("toLocaleString", 0, to_locale_string),
    method("toLocaleDateString", 0, to_locale_date_string),
    method("toLocaleTimeString", 0, to_locale_time_string),
    entry(
        Name::Sym(Wk::ToPrimitive),
        What::Method(1, to_primitive),
        CONFIGURABLE,
    ),
];

const NOT_A_DATE: &str = "this is not a Date object.";
const INVALID: &str = "Invalid Date";

/// The field indices the setters take: year, month, date, hours,
/// minutes, seconds, milliseconds.
const YEAR: usize = 0;
const MONTH: usize = 1;
const DATE_FIELD: usize = 2;
const HOURS: usize = 3;
const MINUTES: usize = 4;
const SECONDS: usize = 5;
const MILLIS: usize = 6;

impl Agent {
    /// The time value now: the embedder's clock, or the system's.
    fn now_value(&mut self) -> f64 {
        let t = match self.vm.clock.as_mut() {
            Some(clock) => clock(),
            None => std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0.0, |d| d.as_millis() as f64),
        };
        time_clip(t)
    }

    fn zone(&self) -> Option<&dyn TimeZone> {
        self.vm.time_zone.as_deref()
    }

    /// The local time zone's name at `t`.
    fn zone_name(&self, t: f64) -> String {
        self.zone()
            .map_or_else(|| "Coordinated Universal Time".to_owned(), |z| z.name(t))
    }

    /// `v` as a date and its time value, if it is one.
    fn date_value(&self, v: Value) -> Option<(u32, f64)> {
        let o = v.as_object()?;
        let cell = self.heap.objects.get(o)?;
        (cell.kind() == ObjectKind::Date).then(|| {
            let t = cell.internal().first().and_then(|v| v.as_number());
            (o, t.unwrap_or(f64::NAN))
        })
    }

    /// `this` as a date and its time value, or the TypeError.
    fn this_time(&mut self, this: Value) -> Res<(u32, f64)> {
        match self.date_value(this) {
            Some(found) => Ok(found),
            None => Err(self.type_error(NOT_A_DATE)),
        }
    }

    /// Sets date `o`'s time value.
    fn set_time_value(&mut self, o: u32, t: f64) -> Value {
        let v = Value::number(t);
        if let Some(cell) = self.heap.objects.get_mut(o) {
            cell.set_internal(0, v);
        }
        v
    }

    /// The local fields of time value `t`.
    fn local_fields(&self, t: f64) -> Fields {
        fields(local_time(self.zone(), t))
    }

    /// ToDateString (section 21.4.4.41.4): `toString`'s form.
    pub(crate) fn date_text(&self, t: f64) -> String {
        if t.is_nan() {
            return INVALID.to_owned();
        }
        let f = self.local_fields(t);
        let off = date::offset(self.zone(), t);
        format!(
            "{} {} {}",
            date_string(&f),
            time_string(&f),
            zone_string(off, &self.zone_name(t))
        )
    }

    /// A year argument as the constructor and `UTC` take it: 0 to 99
    /// are 1900 to 1999.
    fn full_year(&mut self, y: f64) -> f64 {
        if y.is_nan() {
            return y;
        }
        let yi = y.trunc();
        if (0.0..=99.0).contains(&yi) {
            1900.0 + yi
        } else {
            y
        }
    }

    /// The constructor's and `UTC`'s fields from the arguments: year,
    /// month, then date (1), hours, minutes, seconds and milliseconds.
    fn date_from_args(&mut self, c: Call) -> Res<f64> {
        let mut v = [f64::NAN, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0];
        for (i, slot) in v.iter_mut().enumerate().take(c.argc.max(1)) {
            *slot = self.number(self.arg(c, i))?;
        }
        let [y, m, dt, h, min, s, ms] = v;
        let y = self.full_year(y);
        Ok(make_date(make_day(y, m, dt), make_time(h, min, s, ms)))
    }

    /// A setter: the fields from `first` on given by the arguments (at
    /// most `count` of them, the first always), the rest kept, in local
    /// time or UTC (sections 21.4.4.20 to 21.4.4.31).
    fn set_fields(&mut self, c: Call, local: bool, first: usize, count: usize) -> Res<Value> {
        let (o, t) = self.this_time(c.this)?;
        let mut given = [None; 7];
        for i in 0..count.min(c.argc.max(1)) {
            let n = self.number(self.arg(c, i))?;
            if let Some(slot) = given.get_mut(first + i) {
                *slot = Some(n);
            }
        }
        let t = if t.is_nan() {
            // setFullYear alone starts a date with no time value afresh.
            if first != YEAR {
                return Ok(Value::number(f64::NAN));
            }
            0.0
        } else if local {
            local_time(self.zone(), t)
        } else {
            t
        };
        let f = fields(t);
        let now = [f.year, f.month, f.date, f.hour, f.minute, f.second, f.ms];
        let pick = |i: usize| {
            given
                .get(i)
                .copied()
                .flatten()
                .unwrap_or_else(|| now.get(i).copied().unwrap_or(0) as f64)
        };
        let day = make_day(pick(YEAR), pick(MONTH), pick(DATE_FIELD));
        let time = make_time(pick(HOURS), pick(MINUTES), pick(SECONDS), pick(MILLIS));
        let d = make_date(day, time);
        let u = time_clip(if local { utc(self.zone(), d) } else { d });
        Ok(self.set_time_value(o, u))
    }

    /// A getter of the local or UTC fields.
    fn get_field(&mut self, c: Call, local: bool, pick: fn(&Fields) -> i64) -> Res<Value> {
        let (_, t) = self.this_time(c.this)?;
        if t.is_nan() {
            return Ok(Value::number(f64::NAN));
        }
        let f = if local {
            self.local_fields(t)
        } else {
            fields(t)
        };
        Ok(Value::number(pick(&f) as f64))
    }

    /// A string form of a valid time value, "Invalid Date" for NaN.
    fn date_form(&mut self, c: Call, form: fn(&Agent, f64) -> String) -> Res<Value> {
        let (_, t) = self.this_time(c.this)?;
        if t.is_nan() {
            return self.new_string(INVALID);
        }
        let text = form(self, t);
        self.new_string(&text)
    }
}

/// The `Date` constructor (section 21.4.2.1).
pub(crate) fn construct(agent: &mut Agent, c: Call) -> Res<Value> {
    if c.new_target.is_undefined() {
        let now = agent.now_value();
        let text = agent.date_text(now);
        return agent.new_string(&text);
    }
    let tv = match c.argc {
        0 => agent.now_value(),
        1 => {
            let value = agent.arg(c, 0);
            match agent.date_value(value) {
                Some((_, t)) => t,
                None => {
                    let v = agent.primitive(value, None)?;
                    if v.as_string().is_some() {
                        let units = agent.text_units(v)?;
                        date::parse(&units, agent.zone())
                    } else {
                        time_clip(agent.number(v)?)
                    }
                }
            }
        }
        _ => {
            let d = agent.date_from_args(c)?;
            time_clip(utc(agent.zone(), d))
        }
    };
    let proto = agent.prototype_from(c.new_target, |r| r.intrinsics.date_prototype)?;
    agent.alloc_object(ObjectCell::with_internal(
        proto,
        ObjectKind::Date,
        vec![Value::number(tv)],
    ))
}

fn now(agent: &mut Agent, _: Call) -> Res<Value> {
    Ok(Value::number(agent.now_value()))
}

fn parse(agent: &mut Agent, c: Call) -> Res<Value> {
    let units = agent.text_units(agent.arg(c, 0))?;
    Ok(Value::number(date::parse(&units, agent.zone())))
}

/// `Date.UTC` (section 21.4.3.4).
fn utc_static(agent: &mut Agent, c: Call) -> Res<Value> {
    let d = agent.date_from_args(c)?;
    Ok(Value::number(time_clip(d)))
}

macro_rules! getters {
    ($($f:ident, $local:expr, $pick:expr;)*) => {
        $(
            fn $f(agent: &mut Agent, c: Call) -> Res<Value> {
                agent.get_field(c, $local, $pick)
            }
        )*
    };
}

getters! {
    get_date, true, |f| f.date;
    get_day, true, |f| f.weekday;
    get_full_year, true, |f| f.year;
    get_hours, true, |f| f.hour;
    get_milliseconds, true, |f| f.ms;
    get_minutes, true, |f| f.minute;
    get_month, true, |f| f.month;
    get_seconds, true, |f| f.second;
    get_utc_date, false, |f| f.date;
    get_utc_day, false, |f| f.weekday;
    get_utc_full_year, false, |f| f.year;
    get_utc_hours, false, |f| f.hour;
    get_utc_milliseconds, false, |f| f.ms;
    get_utc_minutes, false, |f| f.minute;
    get_utc_month, false, |f| f.month;
    get_utc_seconds, false, |f| f.second;
    get_year, true, |f| f.year - 1900;
}

macro_rules! setters {
    ($($f:ident, $local:expr, $first:expr, $count:expr;)*) => {
        $(
            fn $f(agent: &mut Agent, c: Call) -> Res<Value> {
                agent.set_fields(c, $local, $first, $count)
            }
        )*
    };
}

setters! {
    set_date, true, DATE_FIELD, 1;
    set_full_year, true, YEAR, 3;
    set_hours, true, HOURS, 4;
    set_milliseconds, true, MILLIS, 1;
    set_minutes, true, MINUTES, 3;
    set_month, true, MONTH, 2;
    set_seconds, true, SECONDS, 2;
    set_utc_date, false, DATE_FIELD, 1;
    set_utc_full_year, false, YEAR, 3;
    set_utc_hours, false, HOURS, 4;
    set_utc_milliseconds, false, MILLIS, 1;
    set_utc_minutes, false, MINUTES, 3;
    set_utc_month, false, MONTH, 2;
    set_utc_seconds, false, SECONDS, 2;
}

fn get_time(agent: &mut Agent, c: Call) -> Res<Value> {
    let (_, t) = agent.this_time(c.this)?;
    Ok(Value::number(t))
}

/// `setTime` (section 21.4.4.27), whose receiver check is the reference
/// browser's generic one.
fn set_time(agent: &mut Agent, c: Call) -> Res<Value> {
    let Some((o, _)) = agent.date_value(c.this) else {
        return Err(agent.incompatible("Date.prototype.setTime", c.this));
    };
    let t = agent.number(agent.arg(c, 0))?;
    Ok(agent.set_time_value(o, time_clip(t)))
}

fn get_timezone_offset(agent: &mut Agent, c: Call) -> Res<Value> {
    let (_, t) = agent.this_time(c.this)?;
    if t.is_nan() {
        return Ok(Value::number(f64::NAN));
    }
    // (t - LocalTime(t)) / msPerMinute: +0 for UTC, not -0.
    Ok(Value::number((t - local_time(agent.zone(), t)) / 60_000.0))
}

/// `setYear` (Annex B.2.3.2).
fn set_year(agent: &mut Agent, c: Call) -> Res<Value> {
    let (o, t) = agent.this_time(c.this)?;
    let y = agent.number(agent.arg(c, 0))?;
    if y.is_nan() {
        return Ok(agent.set_time_value(o, f64::NAN));
    }
    let y = agent.full_year(y);
    let t = if t.is_nan() {
        0.0
    } else {
        local_time(agent.zone(), t)
    };
    let f = fields(t);
    let day = make_day(y, f.month as f64, f.date as f64);
    let time = make_time(f.hour as f64, f.minute as f64, f.second as f64, f.ms as f64);
    let u = time_clip(utc(agent.zone(), make_date(day, time)));
    Ok(agent.set_time_value(o, u))
}

fn to_string(agent: &mut Agent, c: Call) -> Res<Value> {
    let (_, t) = agent.this_time(c.this)?;
    let text = agent.date_text(t);
    agent.new_string(&text)
}

fn to_date_string(agent: &mut Agent, c: Call) -> Res<Value> {
    agent.date_form(c, |a, t| date_string(&a.local_fields(t)))
}

fn to_time_string(agent: &mut Agent, c: Call) -> Res<Value> {
    agent.date_form(c, |a, t| {
        let off = date::offset(a.zone(), t);
        format!(
            "{} {}",
            time_string(&a.local_fields(t)),
            zone_string(off, &a.zone_name(t))
        )
    })
}

fn to_utc_string(agent: &mut Agent, c: Call) -> Res<Value> {
    agent.date_form(c, |_, t| date::utc_string(t))
}

fn to_locale_string(agent: &mut Agent, c: Call) -> Res<Value> {
    agent.date_form(c, |a, t| {
        let f = a.local_fields(t);
        format!("{}, {}", date::locale_date(&f), date::locale_time(&f))
    })
}

fn to_locale_date_string(agent: &mut Agent, c: Call) -> Res<Value> {
    agent.date_form(c, |a, t| date::locale_date(&a.local_fields(t)))
}

fn to_locale_time_string(agent: &mut Agent, c: Call) -> Res<Value> {
    agent.date_form(c, |a, t| date::locale_time(&a.local_fields(t)))
}

/// `toISOString` (section 21.4.4.36).
fn to_iso_string(agent: &mut Agent, c: Call) -> Res<Value> {
    let (_, t) = agent.this_time(c.this)?;
    if t.is_nan() {
        return Err(agent.range_error("Invalid time value"));
    }
    agent.new_string(&date::iso_string(t))
}

/// `toJSON` (section 21.4.4.37), generic.
fn to_json(agent: &mut Agent, c: Call) -> Res<Value> {
    let o = agent.object_of(c.this)?;
    agent.vm.temps.push(o);
    let r = (|| {
        let tv = agent.primitive(o, Some(false))?;
        if tv.as_number().is_some_and(|n| !n.is_finite()) {
            return Ok(Value::NULL);
        }
        let key = agent.name_key(Name::Str("toISOString"));
        let f = agent.get_value(o, key)?;
        if !agent.is_callable(f) {
            return Err(agent.type_error("toISOString is not a function"));
        }
        agent.call_value(f, o, &[])
    })();
    agent.vm.temps.pop();
    r
}

/// `Date.prototype[@@toPrimitive]` (section 21.4.4.45).
fn to_primitive(agent: &mut Agent, c: Call) -> Res<Value> {
    if c.this.as_object().is_none() {
        return Err(agent.incompatible("Date.prototype [ @@toPrimitive ]", c.this));
    }
    let hint = agent.arg(c, 0);
    let text = if hint.as_string().is_some() {
        String::from_utf16_lossy(&agent.text_units(hint)?)
    } else {
        String::new()
    };
    let string_first = match text.as_str() {
        "string" | "default" => true,
        "number" => false,
        _ => {
            let shown = agent.display(hint);
            return Err(agent.type_error(format!("Invalid hint: {shown}")));
        }
    };
    agent.ordinary_primitive(c.this, string_first)
}
