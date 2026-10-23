//! `Date`, `RegExp`, `Map`, `Set`, `Promise`, `console`, timers and the loose
//! global functions (`parseInt`, `encodeURIComponent`, …).

use std::cell::RefCell;
use std::rc::Rc;

use super::builtins::{arg, def};
use super::jobs::is_promise;
use super::runtime::{Interp, JsResult};
use super::value::{Key, Kind, Obj, ObjRef, Prop, Value};

// ---------------------------------------------------------------------- Date

/// Days since the epoch → (year, month, day) — Howard Hinnant's algorithm.
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let mp = if m > 2 { m - 3 } else { m + 9 } as u64;
    let doy = (153 * mp + 2) / 5 + d as u64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe as i64 - 719468
}

/// ISO-8601 (`2024-07-19T08:00:00.000Z`).
pub fn date_to_iso(ms: f64) -> String {
    if !ms.is_finite() {
        return "Invalid Date".into();
    }
    let secs = (ms / 1000.0).floor();
    let millis = (ms - secs * 1000.0).round() as u32;
    let days = (secs / 86400.0).floor();
    let rem = (secs - days * 86400.0) as i64;
    let (y, m, d) = civil_from_days(days as i64);
    let (h, mi, s) = (
        (rem / 3600) as u32,
        ((rem % 3600) / 60) as u32,
        (rem % 60) as u32,
    );
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}.{millis:03}Z")
}

/// Parse the date strings real pages use (ISO, RFC-1123, "19.07.2024").
pub fn parse_date(s: &str) -> Option<f64> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    // pure number → milliseconds
    if let Ok(n) = t.parse::<f64>() {
        return Some(n);
    }
    // ISO: 2024-07-19[T08:00[:00][.000]][Z|+02:00]
    let iso: String = t.chars().collect();
    let (date_part, time_part, tz) = split_date_time(&iso);
    let (y, mo, d) = if date_part.contains('-') {
        let mut p = date_part.split('-');
        let y = p.next()?.get(0..4)?.parse::<i64>().ok()?;
        let mo = p.next()?.parse::<u32>().ok()?;
        let d = p
            .next()?
            .get(0..2)
            .unwrap_or(p.next().unwrap_or("1"))
            .parse::<u32>()
            .ok()?;
        (y, mo, d)
    } else if date_part.contains('.') {
        // German style 19.07.2024
        let p: Vec<&str> = date_part.split('.').collect();
        if p.len() != 3 {
            return None;
        }
        (p[2].parse().ok()?, p[1].parse().ok()?, p[0].parse().ok()?)
    } else {
        return None;
    };
    let (h, mi, sec, ms) = match time_part {
        "" => (0, 0, 0, 0),
        tp => {
            let mut p = tp.split(':');
            let h = p.next()?.parse::<u32>().ok()?;
            let mi = p.next().unwrap_or("0").parse::<u32>().ok()?;
            let s = p.next().unwrap_or("0");
            let (sec, ms) = match s.split_once('.') {
                Some((a, b)) => (
                    a.parse::<u32>().unwrap_or(0),
                    b.get(0..3).unwrap_or("0").parse::<u32>().unwrap_or(0),
                ),
                None => (s.parse::<u32>().unwrap_or(0), 0),
            };
            (h, mi, sec, ms)
        }
    };
    let days = days_from_civil(y, mo, d);
    let mut secs = days as f64 * 86400.0 + h as f64 * 3600.0 + mi as f64 * 60.0 + sec as f64;
    secs += ms as f64 / 1000.0;
    // timezone: `Z` means UTC, `+02:00` shifts
    let offset_secs = match tz {
        "" if time_part.is_empty() => 0.0,
        "" => local_offset_secs(),
        "Z" | "z" => 0.0,
        o => {
            let sign = if o.starts_with('-') { 1.0 } else { -1.0 };
            let digits: String = o
                .chars()
                .filter(|c| c.is_ascii_digit() || *c == ':')
                .collect();
            let mut p = digits.split(':');
            let hh: f64 = p.next().unwrap_or("0").parse().unwrap_or(0.0);
            let mm: f64 = p.next().unwrap_or("0").parse().unwrap_or(0.0);
            sign * (hh * 3600.0 + mm * 60.0)
        }
    };
    secs += offset_secs;
    Some(secs * 1000.0)
}

fn split_date_time(s: &str) -> (&str, &str, &str) {
    // find the `T` or a space separating date and time
    let sep = s.find(['T', 't', ' ']).unwrap_or(s.len());
    let (date, rest) = s.split_at(sep);
    let rest = rest.trim_start_matches(['T', 't', ' ']);
    // trailing timezone marker
    let tz_start = rest.find(['Z', 'z', '+']).unwrap_or_else(|| {
        // `-hh:mm` after a space is a timezone, but "-07-19" in the date is not
        rest.rfind('-').filter(|i| *i > 5).unwrap_or(rest.len())
    });
    let time = &rest[..tz_start];
    let tz = rest[tz_start..].trim_start();
    (date, time, tz)
}

/// Whole-hour local offset (the engine has no tz database; this keeps
/// `new Date("…")` without a zone consistent across a run).
pub fn local_offset_secs() -> f64 {
    0.0
}

pub fn date_call(it: &mut Interp, this: Value, args: Vec<Value>) -> JsResult<Value> {
    let ms = match args.as_slice() {
        [] => it.clock,
        [one] if one.is_number() => one.to_number(),
        [one] => parse_date(&it.to_string_val(one)?).unwrap_or(f64::NAN),
        many => {
            let y = many.first().map(|v| v.to_number()).unwrap_or(1970.0);
            let mo = many.get(1).map(|v| v.to_number()).unwrap_or(0.0) + 1.0;
            let d = many.get(2).map(|v| v.to_number()).unwrap_or(1.0);
            let h = many.get(3).map(|v| v.to_number()).unwrap_or(0.0);
            let mi = many.get(4).map(|v| v.to_number()).unwrap_or(0.0);
            let s = many.get(5).map(|v| v.to_number()).unwrap_or(0.0);
            let ms = many.get(6).map(|v| v.to_number()).unwrap_or(0.0);
            let days = days_from_civil(y as i64, mo as u32, d as u32);
            days as f64 * 86_400_000.0 + h * 3_600_000.0 + mi * 60_000.0 + s * 1_000.0 + ms
        }
    };
    if let Some(o) = this.as_object() {
        o.borrow_mut().kind = Kind::Date(ms);
    }
    Ok(this)
}

fn date_ms(v: &Value) -> f64 {
    match v.as_object().map(|o| o.borrow().kind.clone()) {
        Some(Kind::Date(ms)) => ms,
        _ => f64::NAN,
    }
}

/// `Kind::Date` needs a cheap clone; add it via a helper enum copy.
impl Clone for Kind {
    fn clone(&self) -> Kind {
        match self {
            Kind::Plain => Kind::Plain,
            Kind::Array { items } => Kind::Array {
                items: items.clone(),
            },
            Kind::Fn(f) => Kind::Fn(f.clone()),
            Kind::Date(d) => Kind::Date(*d),
            Kind::RegExp(re) => Kind::RegExp(re.clone()),
            Kind::Error => Kind::Error,
            Kind::Wrapper(v) => Kind::Wrapper(v.clone()),
        }
    }
}

impl Interp {
    /// The current timestamp used by `Date.now()` (the virtual clock).
    pub fn now_ms(&self) -> f64 {
        self.clock
    }
}

pub fn install_date(it: &mut Interp, p: &ObjRef, ctor: &Value) {
    def(it, p, "getTime", 0, |_it, this, _args| {
        Ok(Value::Number(date_ms(&this)))
    });
    def(it, p, "valueOf", 0, |_it, this, _args| {
        Ok(Value::Number(date_ms(&this)))
    });
    def(it, p, "toISOString", 0, |_it, this, _args| {
        Ok(Value::string(date_to_iso(date_ms(&this))))
    });
    def(it, p, "toJSON", 1, |_it, this, _args| {
        let ms = date_ms(&this);
        if ms.is_nan() {
            return Ok(Value::Null);
        }
        Ok(Value::string(date_to_iso(ms)))
    });
    def(it, p, "toString", 0, |_it, this, _args| {
        let ms = date_ms(&this);
        if ms.is_nan() {
            return Ok(Value::string("Invalid Date"));
        }
        Ok(Value::string(
            date_to_iso(ms).replace('T', " ").replace('Z', " GMT"),
        ))
    });
    def(it, p, "toUTCString", 0, |_it, this, _args| {
        Ok(Value::string(date_to_iso(date_ms(&this)).replace('T', " ")))
    });
    def(it, p, "toDateString", 0, |_it, this, _args| {
        Ok(Value::string(
            date_to_iso(date_ms(&this))
                .chars()
                .take(10)
                .collect::<String>(),
        ))
    });
    def(it, p, "toLocaleDateString", 0, |_it, this, _args| {
        let iso = date_to_iso(date_ms(&this));
        // German ordering, which is what taz uses
        let head: String = iso.chars().take(10).collect();
        let ymd: Vec<&str> = head.split('-').collect();
        Ok(Value::string(if ymd.len() == 3 {
            format!("{}.{}.{}", ymd[2], ymd[1], ymd[0])
        } else {
            iso
        }))
    });
    def(it, p, "toLocaleTimeString", 0, |_it, this, _args| {
        Ok(Value::string(
            date_to_iso(date_ms(&this))
                .chars()
                .skip(11)
                .take(5)
                .collect::<String>(),
        ))
    });
    def(it, p, "toLocaleString", 0, |_it, this, _args| {
        Ok(Value::string(
            date_to_iso(date_ms(&this))
                .replace('T', " ")
                .replace('Z', ""),
        ))
    });
    let parts: Vec<(&str, DatePart)> = vec![
        ("getFullYear", DatePart::Year),
        ("getUTCFullYear", DatePart::Year),
        ("getMonth", DatePart::Month),
        ("getUTCMonth", DatePart::Month),
        ("getDate", DatePart::Day),
        ("getUTCDate", DatePart::Day),
        ("getDay", DatePart::Weekday),
        ("getUTCDay", DatePart::Weekday),
        ("getHours", DatePart::Hours),
        ("getUTCHours", DatePart::Hours),
        ("getMinutes", DatePart::Minutes),
        ("getUTCMinutes", DatePart::Minutes),
        ("getSeconds", DatePart::Seconds),
        ("getUTCSeconds", DatePart::Seconds),
        ("getMilliseconds", DatePart::Millis),
        ("getUTCMilliseconds", DatePart::Millis),
        ("getTimezoneOffset", DatePart::TzOffset),
    ];
    for (name, part) in parts {
        let v = it.native_fn(name, 0, move |_it, this, _args| {
            let ms = date_ms(&this);
            if ms.is_nan() {
                return Ok(Value::Number(f64::NAN));
            }
            Ok(Value::Number(part.of(ms)))
        });
        p.borrow_mut().define_internal(name, v);
    }
    def(it, p, "setTime", 1, |_it, this, args| {
        let ms = arg(&args, 0).to_number();
        if let Some(o) = this.as_object() {
            o.borrow_mut().kind = Kind::Date(ms);
        }
        Ok(Value::Number(ms))
    });
    def(it, p, "setMilliseconds", 1, |_it, this, args| {
        let ms = date_ms(&this);
        let new = (ms - ms % 1000.0) + arg(&args, 0).to_integer();
        if let Some(o) = this.as_object() {
            o.borrow_mut().kind = Kind::Date(new);
        }
        Ok(Value::Number(new))
    });

    if let Some(c) = ctor.as_object() {
        let v = it.native_fn("now", 0, |it, _t, _args| Ok(Value::Number(it.now_ms())));
        c.borrow_mut().define_internal("now", v);
        let v = it.native_fn("parse", 1, |_it, _t, args| {
            let s = arg(&args, 0).to_string();
            Ok(match parse_date(&s) {
                Some(ms) => Value::Number(ms),
                None => Value::Number(f64::NAN),
            })
        });
        c.borrow_mut().define_internal("parse", v);
        let v = it.native_fn("UTC", 7, |_it, _t, args| {
            let y = arg(&args, 0).to_number();
            let mo = arg(&args, 1).to_number() + 1.0;
            let d = if args.len() > 2 {
                arg(&args, 2).to_number()
            } else {
                1.0
            };
            let days = days_from_civil(y as i64, mo as u32, d as u32);
            let extra = arg(&args, 3).to_number() * 3_600_000.0
                + arg(&args, 4).to_number() * 60_000.0
                + arg(&args, 5).to_number() * 1_000.0
                + arg(&args, 6).to_number();
            Ok(Value::Number(days as f64 * 86_400_000.0 + extra))
        });
        c.borrow_mut().define_internal("UTC", v);
    }
}

#[derive(Clone, Copy)]
enum DatePart {
    Year,
    Month,
    Day,
    Weekday,
    Hours,
    Minutes,
    Seconds,
    Millis,
    TzOffset,
}

impl DatePart {
    fn of(self, ms: f64) -> f64 {
        let secs = (ms / 1000.0).floor();
        let days = (secs / 86400.0).floor();
        let rem = secs - days * 86400.0;
        match self {
            DatePart::Year => civil_from_days(days as i64).0 as f64,
            DatePart::Month => civil_from_days(days as i64).1 as f64 - 1.0,
            DatePart::Day => civil_from_days(days as i64).2 as f64,
            DatePart::Weekday => (((days as i64 % 7) + 11) % 7) as f64,
            DatePart::Hours => (rem / 3600.0).floor(),
            DatePart::Minutes => ((rem % 3600.0) / 60.0).floor(),
            DatePart::Seconds => (rem % 60.0).floor(),
            DatePart::Millis => (ms - secs * 1000.0).round(),
            DatePart::TzOffset => -local_offset_secs() / 60.0,
        }
    }
}

// -------------------------------------------------------------------- RegExp

pub fn regexp_call(it: &mut Interp, this: Value, args: Vec<Value>) -> JsResult<Value> {
    let (src, flags) = match (&arg(&args, 0), arg(&args, 1)) {
        (Value::Object(o), f) if matches!(o.borrow().kind, Kind::RegExp(_)) => {
            let (s, _fl) = (o.borrow().kind.source_of(), o.borrow().kind.flags_of());
            (s, f.to_string())
        }
        (s, f) => (
            it.to_string_val(s).unwrap_or_default(),
            if f.is_undefined() {
                String::new()
            } else {
                f.to_string()
            },
        ),
    };
    let re = super::regexp::RegExp::compile(&src, &flags).map_err(|e| it.syntax_error(&e.0))?;
    if let Some(o) = this.as_object() {
        o.borrow_mut().kind = Kind::RegExp(Rc::new(re));
        o.borrow_mut().define_internal("source", Value::string(src));
        o.borrow_mut()
            .define_internal("flags", Value::string(flags));
        return Ok(this);
    }
    Ok(it.regexp_value(re))
}

pub fn install_regexp(it: &mut Interp, p: &ObjRef, _ctor: &Value) {
    def(it, p, "toString", 0, |_it, this, _args| {
        let kind = this.as_object().map(|o| o.borrow().kind.clone());
        Ok(Value::string(match kind {
            Some(Kind::RegExp(re)) => format!("/{}/{}", re.source, re.flags),
            _ => "/(?:)/".into(),
        }))
    });
    def(it, p, "test", 1, |it, this, args| {
        let re = match this.as_object().map(|o| o.borrow().kind.clone()) {
            Some(Kind::RegExp(re)) => re,
            _ => return Ok(Value::Boolean(false)),
        };
        let s = it.to_string_val(&arg(&args, 0))?;
        let from = if re.flags_struct().global {
            match it.get(&this, &Key::Name(Rc::from("lastIndex"))) {
                Ok(v) if v.is_number() => v.to_integer().max(0.0) as usize,
                _ => 0,
            }
        } else {
            0
        };
        match re.find(&s, from) {
            Some(m) => {
                if re.flags_struct().global {
                    it.set(
                        &this,
                        &Key::Name(Rc::from("lastIndex")),
                        Value::Number(m.end as f64),
                    )?;
                }
                Ok(Value::Boolean(true))
            }
            None => {
                if re.flags_struct().global {
                    it.set(&this, &Key::Name(Rc::from("lastIndex")), Value::Number(0.0))?;
                }
                Ok(Value::Boolean(false))
            }
        }
    });
    def(it, p, "exec", 1, |it, this, args| {
        let re = match this.as_object().map(|o| o.borrow().kind.clone()) {
            Some(Kind::RegExp(re)) => re,
            _ => return Ok(Value::Null),
        };
        let s = it.to_string_val(&arg(&args, 0))?;
        let from = if re.flags_struct().global {
            match it.get(&this, &Key::Name(Rc::from("lastIndex"))) {
                Ok(v) if v.is_number() => v.to_integer().max(0.0) as usize,
                _ => 0,
            }
        } else {
            0
        };
        match re.find(&s, from) {
            Some(m) => {
                if re.flags_struct().global {
                    it.set(
                        &this,
                        &Key::Name(Rc::from("lastIndex")),
                        Value::Number(m.end as f64),
                    )?;
                }
                Ok(it.match_result(&m, &s))
            }
            None => {
                if re.flags_struct().global {
                    it.set(&this, &Key::Name(Rc::from("lastIndex")), Value::Number(0.0))?;
                }
                Ok(Value::Null)
            }
        }
    });
}

// ------------------------------------------------------------------ Map / Set

/// Map/Set entries live in a non-enumerable internal array.
fn entries_of(this: &Value) -> Vec<(Value, Value)> {
    let Some(o) = this.as_object() else {
        return Vec::new();
    };
    let slot = o
        .borrow()
        .get_own(&Key::Name(Rc::from("@@entries")))
        .and_then(|p| p.value().cloned());
    match slot.and_then(|v| v.as_object().cloned()) {
        Some(a) => a
            .borrow()
            .items()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|pair| {
                let mut it = match pair.as_object().and_then(|o| o.borrow().items().cloned()) {
                    Some(v) => v.into_iter(),
                    None => Vec::new().into_iter(),
                };
                (
                    it.next().unwrap_or(Value::Undefined),
                    it.next().unwrap_or(Value::Undefined),
                )
            })
            .collect(),
        None => Vec::new(),
    }
}

fn store_entries(it: &mut Interp, this: &Value, list: Vec<(Value, Value)>) {
    let mut pairs = Vec::with_capacity(list.len());
    for (k, v) in list {
        pairs.push(it.array(vec![k, v]));
    }
    let arr = it.array(pairs);
    if let Some(o) = this.as_object() {
        o.borrow_mut().define_internal("@@entries", arr);
    }
}

fn find_index(list: &[(Value, Value)], k: &Value) -> Option<usize> {
    // Map keys use SameValueZero
    list.iter().position(|(key, _)| {
        key.strict_eq(k)
            || (key.is_number()
                && k.is_number()
                && key.to_number().is_nan()
                && k.to_number().is_nan())
    })
}

pub fn map_call(it: &mut Interp, this: Value, args: Vec<Value>) -> JsResult<Value> {
    store_entries(it, &this, Vec::new());
    if let Some(src) = args.first() {
        if !src.is_nullish() {
            let items = it.iterate(src).unwrap_or_default();
            let mut list = Vec::new();
            for pair in items {
                let k = it.get(&pair, &Key::Num(0.0))?;
                let v = it.get(&pair, &Key::Num(1.0))?;
                list.push((k, v));
            }
            store_entries(it, &this, list);
        }
    }
    Ok(this)
}

pub fn set_call(it: &mut Interp, this: Value, args: Vec<Value>) -> JsResult<Value> {
    store_entries(it, &this, Vec::new());
    if let Some(src) = args.first() {
        if !src.is_nullish() {
            let items = it.iterate(src).unwrap_or_default();
            let mut list: Vec<(Value, Value)> = Vec::new();
            for v in items {
                if find_index(&list, &v).is_none() {
                    list.push((v.clone(), v));
                }
            }
            store_entries(it, &this, list);
        }
    }
    Ok(this)
}

pub fn install_map_set(
    it: &mut Interp,
    map_p: &ObjRef,
    _map_c: &Value,
    set_p: &ObjRef,
    _set_c: &Value,
) {
    // ---------------------------------------------------------------- Map
    def(it, map_p, "get", 1, |_it, this, args| {
        let list = entries_of(&this);
        Ok(match find_index(&list, &arg(&args, 0)) {
            Some(i) => list[i].1.clone(),
            None => Value::Undefined,
        })
    });
    def(it, map_p, "set", 2, |it, this, args| {
        let mut list = entries_of(&this);
        let k = arg(&args, 0);
        let v = arg(&args, 1);
        match find_index(&list, &k) {
            Some(i) => list[i].1 = v,
            None => list.push((k, v)),
        }
        store_entries(it, &this, list);
        Ok(this)
    });
    def(it, map_p, "has", 1, |_it, this, args| {
        Ok(Value::Boolean(
            find_index(&entries_of(&this), &arg(&args, 0)).is_some(),
        ))
    });
    def(it, map_p, "delete", 1, |it, this, args| {
        let mut list = entries_of(&this);
        match find_index(&list, &arg(&args, 0)) {
            Some(i) => {
                list.remove(i);
                store_entries(it, &this, list);
                Ok(Value::Boolean(true))
            }
            None => Ok(Value::Boolean(false)),
        }
    });
    def(it, map_p, "clear", 0, |it, this, _args| {
        store_entries(it, &this, Vec::new());
        Ok(Value::Undefined)
    });
    def(it, map_p, "forEach", 1, |it, this, args| {
        let f = arg(&args, 0);
        for (k, v) in entries_of(&this) {
            it.call_value(&f, arg(&args, 1), vec![v, k, this.clone()])?;
        }
        Ok(Value::Undefined)
    });
    def(it, map_p, "keys", 0, |it, this, _args| {
        Ok(it.array(entries_of(&this).into_iter().map(|(k, _)| k).collect()))
    });
    def(it, map_p, "values", 0, |it, this, _args| {
        Ok(it.array(entries_of(&this).into_iter().map(|(_, v)| v).collect()))
    });
    def(it, map_p, "entries", 0, |it, this, _args| {
        let mut pairs = Vec::new();
        for (k, v) in entries_of(&this) {
            pairs.push(it.array(vec![k, v]));
        }
        Ok(it.array(pairs))
    });
    // size is a getter on the prototype in real JS
    let size = it.native_fn("size getter", 0, |_it, this, _args| {
        Ok(Value::Number(entries_of(&this).len() as f64))
    });
    map_p
        .borrow_mut()
        .define("size", Prop::accessor(size.as_object().cloned(), None));

    // ---------------------------------------------------------------- Set
    def(it, set_p, "add", 1, |it, this, args| {
        let mut list = entries_of(&this);
        let v = arg(&args, 0);
        if find_index(&list, &v).is_none() {
            list.push((v.clone(), v));
        }
        store_entries(it, &this, list);
        Ok(this)
    });
    def(it, set_p, "has", 1, |_it, this, args| {
        Ok(Value::Boolean(
            find_index(&entries_of(&this), &arg(&args, 0)).is_some(),
        ))
    });
    def(it, set_p, "delete", 1, |it, this, args| {
        let mut list = entries_of(&this);
        match find_index(&list, &arg(&args, 0)) {
            Some(i) => {
                list.remove(i);
                store_entries(it, &this, list);
                Ok(Value::Boolean(true))
            }
            None => Ok(Value::Boolean(false)),
        }
    });
    def(it, set_p, "clear", 0, |it, this, _args| {
        store_entries(it, &this, Vec::new());
        Ok(Value::Undefined)
    });
    def(it, set_p, "forEach", 1, |it, this, args| {
        let f = arg(&args, 0);
        for (v, _) in entries_of(&this) {
            it.call_value(&f, arg(&args, 1), vec![v.clone(), v, this.clone()])?;
        }
        Ok(Value::Undefined)
    });
    def(it, set_p, "values", 0, |it, this, _args| {
        Ok(it.array(entries_of(&this).into_iter().map(|(v, _)| v).collect()))
    });
    def(it, set_p, "keys", 0, |it, this, _args| {
        Ok(it.array(entries_of(&this).into_iter().map(|(v, _)| v).collect()))
    });
    def(it, set_p, "entries", 0, |it, this, _args| {
        let mut pairs = Vec::new();
        for (v, _) in entries_of(&this) {
            pairs.push(it.array(vec![v.clone(), v]));
        }
        Ok(it.array(pairs))
    });
    let size = it.native_fn("size getter", 0, |_it, this, _args| {
        Ok(Value::Number(entries_of(&this).len() as f64))
    });
    set_p
        .borrow_mut()
        .define("size", Prop::accessor(size.as_object().cloned(), None));
}

// ------------------------------------------------------------------- Promise

pub fn promise_call(it: &mut Interp, this: Value, args: Vec<Value>) -> JsResult<Value> {
    let executor = arg(&args, 0);
    let Some(obj) = this.as_object().cloned() else {
        // called without new: hand back a fresh resolved promise
        let v = arg(&args, 0);
        return Ok(it.resolved_promise(&v));
    };
    // the constructed object *is* the promise: `resolve`/`reject` settle it
    Interp::init_promise(&obj);
    let (res, rej) = it.settle_hooks(&obj);
    if let Some(ex) = executor.as_object() {
        if ex.borrow().is_function() {
            let r = Value::Object(res);
            let j = Value::Object(rej);
            if let Err(e) = it.call(ex, Value::Undefined, vec![r, j]) {
                it.reject(&obj, &e);
            }
        }
    }
    Ok(this)
}

pub fn install_promise(it: &mut Interp, p: &ObjRef, ctor: &Value) {
    def(it, p, "then", 2, |it, this, args| {
        let on_ok = arg(&args, 0);
        let on_err = arg(&args, 1);
        match this.as_object() {
            Some(o) => it.promise_then(&Value::Object(o.clone()), on_ok, on_err),
            None => Err(it.type_error("then called on a non-promise")),
        }
    });
    def(it, p, "catch", 1, |it, this, args| {
        let on_err = arg(&args, 0);
        match this.as_object() {
            Some(o) => it.promise_then(&Value::Object(o.clone()), Value::Undefined, on_err),
            None => Err(it.type_error("catch called on a non-promise")),
        }
    });
    def(it, p, "finally", 1, |it, this, args| {
        let f = arg(&args, 0);
        let f2 = f.clone();
        let passthrough_ok = it.native_fn("onFinally", 1, move |it, _t, a| {
            if f.is_object() {
                it.call_value(&f, Value::Undefined, Vec::new())?;
            }
            Ok(arg(&a, 0))
        });
        let passthrough_err = it.native_fn("onFinallyErr", 1, move |it, _t, a| {
            if f2.is_object() {
                it.call_value(&f2, Value::Undefined, Vec::new())?;
            }
            Err(arg(&a, 0))
        });
        match this.as_object() {
            Some(o) => it.promise_then(&Value::Object(o.clone()), passthrough_ok, passthrough_err),
            None => Err(it.type_error("finally called on a non-promise")),
        }
    });

    if let Some(c) = ctor.as_object() {
        def(it, c, "resolve", 1, |it, _t, args| {
            let v = arg(&args, 0);
            Ok(it.resolved_promise(&v))
        });
        def(it, c, "reject", 1, |it, _t, args| {
            let v = arg(&args, 0);
            Ok(it.rejected_promise(&v))
        });
        def(it, c, "all", 1, |it, _t, args| {
            let items = it.iterate(&arg(&args, 0))?;
            let (p, _res, _rej) = it.new_promise();
            let n = items.len();
            if n == 0 {
                let empty = it.array(Vec::new());
                it.resolve(&p, &empty);
                return Ok(Value::Object(p));
            }
            // shared (results, completed) state for this aggregate
            let state = Rc::new(RefCell::new((vec![Value::Undefined; n], 0usize, false)));
            let mut already = 0usize;
            // count how many entries are plain values: those are known right now
            for (i, v) in items.iter().enumerate() {
                let promise = v.as_object().map(is_promise);
                if promise == Some(true) || v.is_object() && it.is_thenable(v) {
                    continue;
                }
                state.borrow_mut().0[i] = v.clone();
                already += 1;
            }
            // the synchronously known entries count towards the total
            state.borrow_mut().1 = already;
            if already == n {
                let out = it.array(state.borrow().0.clone());
                it.resolve(&p, &out);
                return Ok(Value::Object(p));
            }
            for (i, v) in items.into_iter().enumerate() {
                let st = state.clone();
                let st2 = state.clone();
                let target = p.clone();
                let target2 = p.clone();
                let ok = it.native_fn("allOk", 1, move |it, _t2, a2| {
                    let val = a2.first().cloned().unwrap_or(Value::Undefined);
                    let finished;
                    {
                        let mut b = st.borrow_mut();
                        if b.2 {
                            return Ok(Value::Undefined);
                        }
                        b.0[i] = val;
                        b.1 += 1;
                        finished = b.1 >= n;
                        if finished {
                            b.2 = true;
                        }
                    }
                    if finished {
                        let out = it.array(st.borrow().0.clone());
                        it.resolve(&target, &out);
                    }
                    Ok(Value::Undefined)
                });
                let err = it.native_fn("allErr", 1, move |it, _t2, a2| {
                    let val = a2.first().cloned().unwrap_or(Value::Undefined);
                    if !st2.borrow().2 {
                        st2.borrow_mut().2 = true;
                        it.reject(&target2, &val);
                    }
                    Ok(Value::Undefined)
                });
                if let Some(o) = v.as_object() {
                    if is_promise(o) {
                        it.then_of(o, &ok, &err);
                    } else {
                        // a thenable: call its `then`
                        it.resolve_thenable(&v.clone(), &ok, &err);
                    }
                }
                // plain values were stored up front
            }
            Ok(Value::Object(p))
        });
        def(it, c, "allSettled", 1, |it, _t, args| {
            let items = it.iterate(&arg(&args, 0))?;
            let mut out = Vec::new();
            for v in items {
                let o = it.plain_object();
                it.set(
                    &o,
                    &Key::Name(Rc::from("status")),
                    Value::string("fulfilled"),
                )?;
                it.set(&o, &Key::Name(Rc::from("value")), v)?;
                out.push(o);
            }
            let arr = it.array(out);
            Ok(it.resolved_promise(&arr))
        });
        def(it, c, "race", 1, |it, _t, args| {
            let items = it.iterate(&arg(&args, 0))?;
            let (p, _res, _rej) = it.new_promise();
            let done = Rc::new(RefCell::new(false));
            for v in items {
                let d = done.clone();
                let d2 = done.clone();
                let target = p.clone();
                let target2 = p.clone();
                let ok = it.native_fn("raceOk", 1, move |it, _t2, a2| {
                    if *d.borrow() {
                        return Ok(Value::Undefined);
                    }
                    *d.borrow_mut() = true;
                    it.resolve(&target, a2.first().unwrap_or(&Value::Undefined));
                    Ok(Value::Undefined)
                });
                let err = it.native_fn("raceErr", 1, move |it, _t2, a2| {
                    if *d2.borrow() {
                        return Ok(Value::Undefined);
                    }
                    *d2.borrow_mut() = true;
                    it.reject(&target2, a2.first().unwrap_or(&Value::Undefined));
                    Ok(Value::Undefined)
                });
                if let Some(o) = v.as_object() {
                    if is_promise(o) {
                        it.then_of(o, &ok, &err);
                    }
                }
            }
            Ok(Value::Object(p))
        });
        def(it, c, "withResolvers", 0, |it, _t, _args| {
            let (p, res, rej) = it.new_promise();
            let o = it.plain_object();
            it.set(&o, &Key::Name(Rc::from("promise")), Value::Object(p))?;
            it.set(&o, &Key::Name(Rc::from("resolve")), Value::Object(res))?;
            it.set(&o, &Key::Name(Rc::from("reject")), Value::Object(rej))?;
            Ok(o)
        });
    }
}

impl Interp {
    /// Attach native handlers to a promise (used by `Promise.all`).
    pub fn then_of(&mut self, p: &ObjRef, ok: &Value, err: &Value) {
        let status = p
            .borrow()
            .get_own(&Key::Name(Rc::from("@@status")))
            .and_then(|pr| pr.value().cloned())
            .map(|v| v.to_string())
            .unwrap_or_else(|| "pending".into());
        if status == "pending" {
            let mut r = Obj::plain();
            r.define_internal("ok", ok.clone());
            r.define_internal("err", err.clone());
            // no derived promise: settle side effects only
            let target = Value::Undefined;
            r.define_internal("target", target);
            let reaction = Rc::new(RefCell::new(r));
            super::jobs::push_reaction_public(p, reaction);
        } else {
            let value = p
                .borrow()
                .get_own(&Key::Name(Rc::from("@@value")))
                .and_then(|pr| pr.value().cloned())
                .unwrap_or(Value::Undefined);
            let handler = if status == "fulfilled" {
                ok.clone()
            } else {
                err.clone()
            };
            let job = move |it: &mut Interp| {
                if let Some(h) = handler.as_object() {
                    it.call(h, Value::Undefined, vec![value.clone()]).ok();
                }
            };
            self.queue.push_back(Rc::new(job));
        }
    }
}

// ------------------------------------------------------------------- console

pub fn install_console(it: &mut Interp) {
    let c = it.plain_ref();
    for (level, name) in [
        ("log", "log"),
        ("info", "info"),
        ("warn", "warn"),
        ("error", "error"),
        ("debug", "debug"),
        ("trace", "trace"),
        ("dir", "dir"),
    ] {
        let lvl = level;
        let v = it.native_fn(name, 1, move |it, _t, args| {
            let mut parts = Vec::new();
            for a in &args {
                parts.push(match a {
                    Value::Str(s) => s.to_string(),
                    other => other.inspect(),
                });
            }
            it.log(lvl, parts.join(" "));
            Ok(Value::Undefined)
        });
        c.borrow_mut().define_internal(name, v);
    }
    for name in [
        "group",
        "groupCollapsed",
        "groupEnd",
        "time",
        "timeEnd",
        "table",
        "count",
        "assert",
        "clear",
    ] {
        let v = it.native_fn(name, 1, move |it, _t, args| {
            if name == "assert" && !arg(&args, 0).to_boolean() {
                let msg = if args.len() > 1 {
                    it.to_string_val(&args[1])?
                } else {
                    "Assertion failed".into()
                };
                it.log("error", msg);
            }
            Ok(Value::Undefined)
        });
        c.borrow_mut().define_internal(name, v);
    }
    it.globals
        .borrow_mut()
        .define_internal("console", Value::Object(c));
}

// -------------------------------------------------------------------- timers

pub fn install_timers(it: &mut Interp) {
    let g = it.globals.clone();
    let v = it.native_fn("setTimeout", 1, |it, _t, mut args| {
        let cb = if args.is_empty() {
            Value::Undefined
        } else {
            args.remove(0)
        };
        let ms = arg(&args, 0).to_number();
        let ms = if ms.is_nan() { 0.0 } else { ms };
        let id = it.schedule(ms, None, cb, args[1..].to_vec());
        Ok(Value::Number(id as f64))
    });
    g.borrow_mut().define_internal("setTimeout", v);
    let v = it.native_fn("setInterval", 1, |it, _t, mut args| {
        let cb = if args.is_empty() {
            Value::Undefined
        } else {
            args.remove(0)
        };
        let ms = arg(&args, 0).to_number();
        let ms = if ms.is_nan() || ms <= 0.0 { 1.0 } else { ms };
        let id = it.schedule(ms, Some(ms), cb, args[1..].to_vec());
        Ok(Value::Number(id as f64))
    });
    g.borrow_mut().define_internal("setInterval", v);
    let v = it.native_fn("clearTimeout", 1, |it, _t, args| {
        it.clear_timer(arg(&args, 0).to_number());
        Ok(Value::Undefined)
    });
    g.borrow_mut().define_internal("clearTimeout", v);
    let v = it.native_fn("clearInterval", 1, |it, _t, args| {
        it.clear_timer(arg(&args, 0).to_number());
        Ok(Value::Undefined)
    });
    g.borrow_mut().define_internal("clearInterval", v);
    let v = it.native_fn("queueMicrotask", 1, |it, _t, args| {
        let f = arg(&args, 0);
        if let Some(o) = f.as_object() {
            let o = o.clone();
            it.queue.push_back(Rc::new(move |it| {
                if let Err(e) = it.call(&o, Value::Undefined, Vec::new()) {
                    let msg = it.format_thrown(&e);
                    it.log("error", format!("Uncaught {msg}"));
                }
            }));
        }
        Ok(Value::Undefined)
    });
    g.borrow_mut().define_internal("queueMicrotask", v);
    // animation frames are just 16 ms timers
    let v = it.native_fn("requestAnimationFrame", 1, |it, _t, args| {
        let cb = arg(&args, 0);
        let id = it.schedule(16.0, None, cb, vec![Value::Number(it.clock)]);
        Ok(Value::Number(id as f64))
    });
    g.borrow_mut().define_internal("requestAnimationFrame", v);
    let v = it.native_fn("cancelAnimationFrame", 1, |it, _t, args| {
        it.clear_timer(arg(&args, 0).to_number());
        Ok(Value::Undefined)
    });
    g.borrow_mut().define_internal("cancelAnimationFrame", v);
}

// ------------------------------------------------------- loose global things

pub fn install_global_functions(it: &mut Interp) {
    let g = it.globals.clone();
    let v = it.native_fn("parseInt", 2, |_it, _t, args| {
        let s = arg(&args, 0).to_string();
        let radix = match arg(&args, 1) {
            Value::Undefined => 10,
            v => v.to_integer() as u32,
        };
        Ok(Value::Number(parse_int(&s, radix)))
    });
    g.borrow_mut().define_internal("parseInt", v);
    let v = it.native_fn("parseFloat", 1, |_it, _t, args| {
        Ok(Value::Number(parse_float(&arg(&args, 0).to_string())))
    });
    g.borrow_mut().define_internal("parseFloat", v);
    let v = it.native_fn("isNaN", 1, |it, _t, args| {
        Ok(Value::Boolean(it.to_number_val(&arg(&args, 0))?.is_nan()))
    });
    g.borrow_mut().define_internal("isNaN", v);
    let v = it.native_fn("isFinite", 1, |it, _t, args| {
        Ok(Value::Boolean(
            it.to_number_val(&arg(&args, 0))?.is_finite(),
        ))
    });
    g.borrow_mut().define_internal("isFinite", v);

    for (name, keep, decode) in [
        ("encodeURIComponent", false, false),
        ("encodeURI", true, false),
        ("decodeURIComponent", false, true),
        ("decodeURI", true, true),
    ] {
        let keep_extra = keep;
        let is_decode = decode;
        let v = it.native_fn(name, 1, move |_it, _t, args| {
            let s = arg(&args, 0).to_string();
            Ok(Value::string(if is_decode {
                percent_decode(&s)
            } else {
                percent_encode(&s, keep_extra)
            }))
        });
        g.borrow_mut().define_internal(name, v);
    }

    // Symbol: unique string keys, enough for `Symbol.iterator` feature tests
    let counter = Rc::new(RefCell::new(0u64));
    let v = it.native_fn_rc(
        "Symbol",
        0,
        Rc::new(move |_it: &mut Interp, _t, args| {
            let mut n = counter.borrow_mut();
            *n += 1;
            let desc = arg(&args, 0).to_string();
            Ok(Value::string(format!("@@{desc}_{n}")))
        }),
    );
    g.borrow_mut().define_internal("Symbol", v.clone());
    if let Some(o) = v.as_object() {
        for name in [
            "iterator",
            "asyncIterator",
            "hasInstance",
            "toPrimitive",
            "toStringTag",
            "species",
        ] {
            o.borrow_mut()
                .define_internal(name, Value::string(format!("@@{name}")));
        }
    }

    // navigator / performance: enough for feature detection and UA sniffs
    let nav = it.plain_ref();
    for (k, v) in [
        ("userAgent", "tinybrowser/0.1 (console; rv:1.0)"),
        ("appName", "Netscape"),
        ("appVersion", "5.0"),
        ("platform", "linux x86_64"),
        ("language", "de-DE"),
        ("product", "Gecko"),
        ("vendor", ""),
    ] {
        nav.borrow_mut().define_internal(k, Value::string(v));
    }
    nav.borrow_mut()
        .define_internal("onLine", Value::Boolean(true));
    g.borrow_mut()
        .define_internal("navigator", Value::Object(nav));

    let perf = it.plain_ref();
    let v = it.native_fn("now", 0, |it, _t, _args| Ok(Value::Number(it.now_ms())));
    perf.borrow_mut().define_internal("now", v);
    let v = it.native_fn("timeOrigin", 0, |_it, _t, _args| Ok(Value::Number(0.0)));
    perf.borrow_mut().define_internal("timeOrigin", v);
    g.borrow_mut()
        .define_internal("performance", Value::Object(perf));

    // host hooks that are no-ops without a UI
    for name in [
        "alert", "confirm", "prompt", "scrollTo", "scrollBy", "stop", "close",
    ] {
        let v = it.native_fn(name, 0, move |it, _t, _args| {
            it.note(format!("{name}() is not supported (no UI)"));
            Ok(if name == "confirm" {
                Value::Boolean(false)
            } else {
                Value::Undefined
            })
        });
        g.borrow_mut().define_internal(name, v);
    }
    let v = it.native_fn("structuredClone", 1, |it, _t, args| {
        let text = super::json::stringify(it, &arg(&args, 0), None, "", 0)?;
        let v = super::json::parse(&text).unwrap_or(Value::Undefined);
        super::json::link_protos(&it.natives, &v);
        Ok(v)
    });
    g.borrow_mut().define_internal("structuredClone", v);
    let v = it.native_fn("eval", 1, |it, _t, args| {
        let src = it.to_string_val(&arg(&args, 0))?;
        match it.run_script(&src) {
            Ok(v) => Ok(v),
            Err(msg) => Err(it.syntax_error(&msg)),
        }
    });
    g.borrow_mut().define_internal("eval", v);
}

/// `parseInt` semantics: optional sign, radix prefix, longest numeric prefix.
pub fn parse_int(s: &str, radix: u32) -> f64 {
    let t = s.trim_start();
    let (neg, rest) = match t.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    let mut radix = radix;
    let mut body = rest;
    if radix == 0 || radix == 16 {
        if let Some(r) = body.strip_prefix("0x").or(body.strip_prefix("0X")) {
            radix = 16;
            body = r;
        } else if radix == 0 {
            radix = 10;
        }
    }
    if radix == 0 {
        radix = 10;
    }
    if !(2..=36).contains(&radix) {
        return f64::NAN;
    }
    let mut acc = 0.0f64;
    let mut any = false;
    for c in body.chars() {
        let d = c.to_digit(radix);
        match d {
            Some(d) => {
                acc = acc * radix as f64 + d as f64;
                any = true;
            }
            None => break,
        }
    }
    if !any {
        return f64::NAN;
    }
    if neg {
        -acc
    } else {
        acc
    }
}

/// `parseFloat` semantics.
pub fn parse_float(s: &str) -> f64 {
    let t = s.trim_start();
    let chars: Vec<char> = t.chars().collect();
    let mut end = 0;
    if matches!(chars.first(), Some('+') | Some('-')) {
        end = 1;
    }
    let mut dot = false;
    let mut exp = false;
    while end < chars.len() {
        let c = chars[end];
        if c.is_ascii_digit() {
            end += 1;
        } else if c == '.' && !dot && !exp {
            dot = true;
            end += 1;
        } else if (c == 'e' || c == 'E') && !exp && end > 0 {
            exp = true;
            end += 1;
            if matches!(chars.get(end), Some('+') | Some('-')) {
                end += 1;
            }
        } else {
            break;
        }
    }
    if end == 0 || (end == 1 && matches!(chars[0], '+' | '-')) {
        return f64::NAN;
    }
    let head: String = chars[..end].iter().collect();
    head.parse::<f64>().unwrap_or(f64::NAN)
}

const UNRESERVED: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_.!~*'()";
const URI_RESERVED: &str = ":/?#[]@!$&'()*+,;=";

pub fn percent_encode(s: &str, keep_extra: bool) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if UNRESERVED.contains(c) || (keep_extra && URI_RESERVED.contains(c)) {
            out.push(c);
        } else {
            let mut buf = [0u8; 4];
            for b in c.encode_utf8(&mut buf).as_bytes() {
                out.push_str(&format!("%{b:02X}"));
            }
        }
    }
    out
}

pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
