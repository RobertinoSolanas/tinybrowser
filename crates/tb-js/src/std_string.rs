//! `String`, `Number` and `Boolean` prototypes.

use std::rc::Rc;

use super::builtins::{arg, arg_or, def};
use super::regexp::replace_with;
use super::runtime::{Interp, JsResult};
use super::value::Value;

/// Character vector of a `this` string (or the wrapped primitive).
fn chars(v: &Value) -> Vec<char> {
    match v {
        Value::Str(s) => s.chars().collect(),
        Value::Object(o) => match &o.borrow().kind {
            super::value::Kind::Wrapper(inner) => inner.to_string().chars().collect(),
            _ => o.borrow().default_string().chars().collect(),
        },
        other => other.to_string().chars().collect(),
    }
}

pub fn string_call(it: &mut Interp, this: Value, args: Vec<Value>) -> JsResult<Value> {
    let s = match args.first() {
        None => String::new(),
        Some(v) => it.to_string_val(v)?,
    };
    // `String(x)` as a function returns a primitive, `new String(x)` a wrapper
    let len = s.chars().count();
    if let Some(o) = this.as_object() {
        o.borrow_mut().kind = super::value::Kind::Wrapper(Value::string(s));
        o.borrow_mut()
            .define_internal("length", Value::Number(len as f64));
        return Ok(this);
    }
    Ok(Value::string(s))
}

pub fn number_call(it: &mut Interp, this: Value, args: Vec<Value>) -> JsResult<Value> {
    let n = match args.first() {
        None => 0.0,
        Some(v) => it.to_number_val(v)?,
    };
    if let Some(o) = this.as_object() {
        o.borrow_mut().kind = super::value::Kind::Wrapper(Value::Number(n));
        return Ok(this);
    }
    Ok(Value::Number(n))
}

pub fn bool_call(_it: &mut Interp, this: Value, args: Vec<Value>) -> JsResult<Value> {
    let b = arg(&args, 0).to_boolean();
    if let Some(o) = this.as_object() {
        o.borrow_mut().kind = super::value::Kind::Wrapper(Value::Boolean(b));
        return Ok(this);
    }
    Ok(Value::Boolean(b))
}

pub fn install_string(it: &mut Interp, p: &super::value::ObjRef) {
    def(it, p, "toString", 0, |_it, this, _args| {
        Ok(Value::string(chars(&this).iter().collect::<String>()))
    });
    def(it, p, "valueOf", 0, |_it, this, _args| {
        Ok(Value::string(chars(&this).iter().collect::<String>()))
    });
    def(it, p, "charAt", 1, |_it, this, args| {
        let c = chars(&this);
        let i = arg(&args, 0).to_integer();
        let i = if i < 0.0 { usize::MAX } else { i as usize };
        Ok(match c.get(i) {
            Some(ch) => Value::string(ch.to_string()),
            None => Value::string(""),
        })
    });
    def(it, p, "charCodeAt", 1, |_it, this, args| {
        let c = chars(&this);
        let i = arg(&args, 0).to_integer() as usize;
        Ok(match c.get(i) {
            Some(ch) => Value::Number(*ch as u32 as f64),
            None => Value::Number(f64::NAN),
        })
    });
    def(it, p, "codePointAt", 1, |_it, this, args| {
        let c = chars(&this);
        let i = arg(&args, 0).to_integer() as usize;
        Ok(match c.get(i) {
            Some(ch) => Value::Number(*ch as u32 as f64),
            None => Value::Undefined,
        })
    });
    def(it, p, "at", 1, |_it, this, args| {
        let c = chars(&this);
        let n = arg(&args, 0).to_integer();
        let i = if n < 0.0 { c.len() as f64 + n } else { n };
        Ok(if i < 0.0 {
            Value::Undefined
        } else {
            match c.get(i as usize) {
                Some(ch) => Value::string(ch.to_string()),
                None => Value::Undefined,
            }
        })
    });
    def(it, p, "indexOf", 1, |it, this, args| {
        let hay = chars(&this).iter().collect::<String>();
        let needle = it.to_string_val(&arg(&args, 0))?;
        let from = arg(&args, 1).to_integer().max(0.0) as usize;
        let pre: String = hay.chars().skip(from).collect();
        let off = hay[..hay.len().min(pre.len().min(hay.len()))].len();
        let _ = off;
        match pre.find(&needle) {
            Some(rel) => {
                let idx = hay[..hay.find(&pre).unwrap_or(0) + 0].chars().count();
                let _ = idx;
                let chars_before: usize = hay.chars().take(byte_to_char(&hay, from)).count();
                let _ = chars_before;
                let needle_chars = pre[..rel].chars().count();
                Ok(Value::Number((from + needle_chars) as f64))
            }
            None => Ok(Value::Number(-1.0)),
        }
    });
    def(it, p, "lastIndexOf", 1, |it, this, args| {
        let hay = chars(&this).iter().collect::<String>();
        let needle = it.to_string_val(&arg(&args, 0))?;
        Ok(Value::Number(
            match hay.rfind(&needle) {
                Some(b) => hay[..b].chars().count(),
                None => usize::MAX,
            }
            .min(usize::MAX - 1) as f64
                - if hay.rfind(&needle).is_none() {
                    usize::MAX as f64 - 1.0
                } else {
                    0.0
                },
        ))
    });
    def(it, p, "includes", 1, |it, this, args| {
        let hay = chars(&this).iter().collect::<String>();
        let needle = it.to_string_val(&arg(&args, 0))?;
        Ok(Value::Boolean(hay.contains(&needle)))
    });
    def(it, p, "startsWith", 1, |it, this, args| {
        let hay = chars(&this).iter().collect::<String>();
        let needle = it.to_string_val(&arg(&args, 0))?;
        let from = arg(&args, 1).to_integer().max(0.0) as usize;
        let s: String = hay.chars().skip(from).collect();
        Ok(Value::Boolean(s.starts_with(&needle)))
    });
    def(it, p, "endsWith", 1, |it, this, args| {
        let hay = chars(&this).iter().collect::<String>();
        let needle = it.to_string_val(&arg(&args, 0))?;
        let end = match arg(&args, 1) {
            Value::Undefined => hay.chars().count(),
            v => v.to_integer().max(0.0) as usize,
        };
        let s: String = hay.chars().take(end).collect();
        Ok(Value::Boolean(s.ends_with(&needle)))
    });
    def(it, p, "slice", 2, |_it, this, args| {
        let c = chars(&this);
        let len = c.len() as f64;
        let mut a = arg(&args, 0).to_integer();
        let b = match arg(&args, 1) {
            Value::Undefined => len,
            v => v.to_integer(),
        };
        if a < 0.0 {
            a = (len + a).max(0.0);
        }
        let b = if b < 0.0 {
            (len + b).max(0.0)
        } else {
            b.min(len)
        };
        let out: String = if a < b {
            c[a as usize..b as usize].iter().collect()
        } else {
            String::new()
        };
        Ok(Value::string(out))
    });
    def(it, p, "substring", 2, |_it, this, args| {
        let c = chars(&this);
        let len = c.len() as f64;
        let mut a = arg(&args, 0).to_integer().clamp(0.0, len);
        let mut b = match arg(&args, 1) {
            Value::Undefined => len,
            v => v.to_integer().clamp(0.0, len),
        };
        if a > b {
            std::mem::swap(&mut a, &mut b);
        }
        Ok(Value::string(
            c[a as usize..b as usize].iter().collect::<String>(),
        ))
    });
    def(it, p, "substr", 2, |_it, this, args| {
        let c = chars(&this);
        let len = c.len() as f64;
        let mut a = arg(&args, 0).to_integer();
        if a < 0.0 {
            a = (len + a).max(0.0);
        }
        let n = match arg(&args, 1) {
            Value::Undefined => len - a,
            v => v.to_integer(),
        };
        let end = (a + n).clamp(0.0, len) as usize;
        let start = a as usize;
        Ok(Value::string(if start < end {
            c[start..end].iter().collect::<String>()
        } else {
            String::new()
        }))
    });
    def(it, p, "toUpperCase", 0, |_it, this, _args| {
        Ok(Value::string(
            chars(&this)
                .iter()
                .flat_map(|c| c.to_uppercase())
                .collect::<String>(),
        ))
    });
    def(it, p, "toLowerCase", 0, |_it, this, _args| {
        Ok(Value::string(
            chars(&this)
                .iter()
                .flat_map(|c| c.to_lowercase())
                .collect::<String>(),
        ))
    });
    def(it, p, "toLocaleUpperCase", 0, |_it, this, _args| {
        Ok(Value::string(
            chars(&this)
                .iter()
                .flat_map(|c| c.to_uppercase())
                .collect::<String>(),
        ))
    });
    def(it, p, "toLocaleLowerCase", 0, |_it, this, _args| {
        Ok(Value::string(
            chars(&this)
                .iter()
                .flat_map(|c| c.to_lowercase())
                .collect::<String>(),
        ))
    });
    def(it, p, "trim", 0, |_it, this, _args| {
        Ok(Value::string(
            chars(&this).iter().collect::<String>().trim().to_string(),
        ))
    });
    def(it, p, "trimStart", 0, |_it, this, _args| {
        Ok(Value::string(
            chars(&this)
                .iter()
                .collect::<String>()
                .trim_start()
                .to_string(),
        ))
    });
    def(it, p, "trimEnd", 0, |_it, this, _args| {
        Ok(Value::string(
            chars(&this)
                .iter()
                .collect::<String>()
                .trim_end()
                .to_string(),
        ))
    });
    def(it, p, "trimLeft", 0, |_it, this, _args| {
        Ok(Value::string(
            chars(&this)
                .iter()
                .collect::<String>()
                .trim_start()
                .to_string(),
        ))
    });
    def(it, p, "trimRight", 0, |_it, this, _args| {
        Ok(Value::string(
            chars(&this)
                .iter()
                .collect::<String>()
                .trim_end()
                .to_string(),
        ))
    });
    def(it, p, "concat", 1, |it, this, args| {
        let mut out = chars(&this).iter().collect::<String>();
        for a in args {
            out.push_str(&it.to_string_val(&a)?);
        }
        Ok(Value::string(out))
    });
    def(it, p, "repeat", 1, |it, this, args| {
        let n = it.to_number_val(&arg(&args, 0))? as usize;
        let s: String = chars(&this).iter().collect();
        if n > 1_000_000 {
            return Err(it.range_error("repeat count must be less than 2**28"));
        }
        Ok(Value::string(s.repeat(n)))
    });
    def(it, p, "padStart", 2, |_it, this, args| {
        let mut c = chars(&this);
        let target = arg(&args, 0).to_integer() as usize;
        let pad = match arg_or(&args, 1, " ") {
            s if s.is_empty() => " ".to_string(),
            s => s,
        };
        let mut i = 0;
        while c.len() < target {
            c.insert(0, pad.chars().nth(i % pad.chars().count()).unwrap());
            i += 1;
        }
        Ok(Value::string(c.iter().collect::<String>()))
    });
    def(it, p, "padEnd", 2, |_it, this, args| {
        let mut c = chars(&this);
        let target = arg(&args, 0).to_integer() as usize;
        let pad = match arg_or(&args, 1, " ") {
            s if s.is_empty() => " ".to_string(),
            s => s,
        };
        let mut i = 0;
        while c.len() < target {
            c.push(pad.chars().nth(i % pad.chars().count()).unwrap());
            i += 1;
        }
        Ok(Value::string(c.iter().collect::<String>()))
    });
    def(it, p, "split", 2, |it, this, args| {
        let s: String = chars(&this).iter().collect();
        let sep = arg(&args, 0);
        let limit = match arg(&args, 1) {
            Value::Undefined => usize::MAX,
            v => v.to_integer().max(0.0) as usize,
        };
        let parts: Vec<Value> = if sep.is_undefined() {
            vec![Value::string(s.clone())]
        } else if let Some(re) = regexp_of(&sep) {
            // splitting with a regular expression
            let mut out: Vec<Value> = Vec::new();
            let mut last = 0usize;
            for m in re.find_all(&s) {
                if m.end == m.start && last == m.start {
                    continue;
                }
                out.push(Value::string(
                    s.chars()
                        .skip(last)
                        .take(m.start - last)
                        .collect::<String>(),
                ));
                last = m.end;
                if out.len() >= limit {
                    break;
                }
            }
            if out.len() < limit {
                out.push(Value::string(s.chars().skip(last).collect::<String>()));
            }
            out
        } else if let Value::Str(sep) = &sep {
            if sep.is_empty() {
                s.chars().map(|c| Value::string(c.to_string())).collect()
            } else {
                s.split(&**sep).map(Value::string).collect::<Vec<Value>>()
            }
        } else {
            let sep = it.to_string_val(&sep)?;
            if sep.is_empty() {
                s.chars().map(|c| Value::string(c.to_string())).collect()
            } else {
                s.split(&sep).map(Value::string).collect::<Vec<Value>>()
            }
        };
        Ok(it.array(parts.into_iter().take(limit).collect()))
    });
    def(it, p, "replace", 2, |it, this, args| {
        let s: String = chars(&this).iter().collect();
        let pattern = arg(&args, 0);
        let repl = arg(&args, 1);
        if let Some(re) = regexp_of(&pattern) {
            let flags = re.flags_struct();
            let out = if repl.is_object() {
                let mut results: Vec<String> = Vec::new();
                for m in re.find_all(&s) {
                    let mut a: Vec<Value> = vec![Value::string(m.text.clone())];
                    for g in m.groups.iter().skip(1) {
                        a.push(
                            g.as_ref()
                                .map(|v| Value::string(v.clone()))
                                .unwrap_or(Value::Undefined),
                        );
                    }
                    a.push(Value::Number(m.start as f64));
                    a.push(Value::string(s.clone()));
                    let v = repl.call_value_shim(it, a)?;
                    results.push(v);
                }
                apply_matches(&s, &re.find_all(&s), results)
            } else {
                let rs = it.to_string_val(&repl)?;
                replace_with(&re, &s, |m| Value::string(expand_replacement(&rs, m, &s)))
            };
            let _ = flags;
            return Ok(Value::string(out));
        }
        let pat = it.to_string_val(&pattern)?;
        let rs = it.to_string_val(&repl)?;
        // only the first occurrence, with $& $1 … support
        match s.find(&pat) {
            Some(b) => {
                let pre: String = s[..b].chars().collect();
                let post: String = s[b + pat.len()..].chars().collect();
                let mid = expand_replacement(&rs, &simple_match(&pat, b, &s), &s);
                Ok(Value::string(format!("{pre}{mid}{post}")))
            }
            None => Ok(Value::string(s)),
        }
    });
    def(it, p, "replaceAll", 2, |it, this, args| {
        let s: String = chars(&this).iter().collect();
        let pattern = arg(&args, 0);
        let repl = arg(&args, 1);
        if let Some(re) = regexp_of(&pattern) {
            let re = re;
            if !re.flags_struct().global {
                return Err(it.type_error("replaceAll must be called with a global RegExp"));
            }
            let rs = it.to_string_val(&repl)?;
            let out = replace_with(&re, &s, |m| Value::string(expand_replacement(&rs, m, &s)));
            return Ok(Value::string(out));
        }
        let pat = it.to_string_val(&pattern)?;
        let rs = it.to_string_val(&repl)?;
        if pat.is_empty() {
            return Ok(Value::string(s));
        }
        let parts: Vec<&str> = s.split(pat.as_str()).collect();
        Ok(Value::string(parts.join(&rs)))
    });
    def(it, p, "match", 1, |it, this, args| {
        let s: String = chars(&this).iter().collect();
        let pat = arg(&args, 0);
        let re = match regexp_of(&pat) {
            Some(r) => r,
            None => {
                let ps = it.to_string_val(&pat)?;
                super::regexp::RegExp::compile(&escape_regexp(&ps), "")
                    .map_err(|e| it.syntax_error(&e.0))?
            }
        };
        if re.flags_struct().global {
            let all = re.find_all(&s);
            if all.is_empty() {
                return Ok(Value::Null);
            }
            return Ok(it.array(all.into_iter().map(|m| Value::string(m.text)).collect()));
        }
        match re.find(&s, 0) {
            None => Ok(Value::Null),
            Some(m) => Ok(it.match_result(&m, &s)),
        }
    });
    def(it, p, "matchAll", 1, |it, this, args| {
        let s: String = chars(&this).iter().collect();
        let pat = arg(&args, 0);
        let re = match regexp_of(&pat) {
            Some(r) => r,
            None => {
                let ps = it.to_string_val(&pat)?;
                super::regexp::RegExp::compile(&escape_regexp(&ps), "")
                    .map_err(|e| it.syntax_error(&e.0))?
            }
        };
        let all = re.find_all(&s);
        let arr: Vec<Value> = all.into_iter().map(|m| it.match_result(&m, &s)).collect();
        Ok(it.array(arr))
    });
    def(it, p, "search", 1, |it, this, args| {
        let s: String = chars(&this).iter().collect();
        let pat = arg(&args, 0);
        let re = match regexp_of(&pat) {
            Some(r) => r,
            None => {
                let ps = it.to_string_val(&pat)?;
                super::regexp::RegExp::compile(&escape_regexp(&ps), "")
                    .map_err(|e| it.syntax_error(&e.0))?
            }
        };
        Ok(match re.find(&s, 0) {
            Some(m) => Value::Number(m.start as f64),
            None => Value::Number(-1.0),
        })
    });
    def(it, p, "localeCompare", 1, |it, this, args| {
        let a: String = chars(&this).iter().collect();
        let b = it.to_string_val(&arg(&args, 0))?;
        Ok(Value::Number(match a.cmp(&b) {
            std::cmp::Ordering::Less => -1.0,
            std::cmp::Ordering::Equal => 0.0,
            std::cmp::Ordering::Greater => 1.0,
        }))
    });
    def(it, p, "normalize", 0, |_it, this, _args| {
        Ok(Value::string(chars(&this).iter().collect::<String>()))
    });
}

/// Build a `MatchInfo`-shaped array result (`[full, g1, …]` + `index`, `groups`).
impl Interp {
    pub fn match_result(&mut self, m: &super::regexp::MatchInfo, input: &str) -> Value {
        let mut parts = vec![Value::string(m.text.clone())];
        let mut named = Vec::new();
        for (i, g) in m.groups.iter().enumerate().skip(1) {
            parts.push(
                g.as_ref()
                    .map(|v| Value::string(v.clone()))
                    .unwrap_or(Value::Undefined),
            );
            named.push((i, g.clone()));
        }
        let arr = self.array(parts);
        if let Some(o) = arr.as_object() {
            o.borrow_mut()
                .define_internal("index", Value::Number(m.start as f64));
            o.borrow_mut()
                .define_internal("input", Value::string(input));
        }
        let _ = named;
        arr
    }
}

fn byte_to_char(s: &str, byte: usize) -> usize {
    s[..byte.min(s.len())].chars().count()
}

/// Shared `$1`, `$&`, `` $` ``, `$'` replacement syntax.
fn expand_replacement(tpl: &str, m: &super::regexp::MatchInfo, input: &str) -> String {
    let mut out = String::new();
    let mut it = tpl.chars().peekable();
    while let Some(c) = it.next() {
        if c != '$' {
            out.push(c);
            continue;
        }
        match it.peek() {
            Some('$') => {
                out.push('$');
                it.next();
            }
            Some('&') => {
                out.push_str(&m.text);
                it.next();
            }
            Some('`') => {
                out.push_str(&input.chars().take(m.start).collect::<String>());
                it.next();
            }
            Some('\'') => {
                out.push_str(&input.chars().skip(m.end).collect::<String>());
                it.next();
            }
            Some(d) if d.is_ascii_digit() => {
                let mut num = String::new();
                while matches!(it.peek(), Some(x) if x.is_ascii_digit()) {
                    num.push(it.next().unwrap());
                }
                let idx: usize = num.parse().unwrap_or(0);
                if idx == 0 {
                    // `$0` is not a capture reference — it stays as written
                    out.push('$');
                    out.push('0');
                } else {
                    match m.groups.get(idx) {
                        Some(Some(g)) => out.push_str(g),
                        _ => out.push_str(&format!("${num}")),
                    }
                }
            }
            _ => out.push('$'),
        }
    }
    out
}

fn simple_match(pat: &str, byte: usize, input: &str) -> super::regexp::MatchInfo {
    let start = input[..byte].chars().count();
    let plen = pat.chars().count();
    super::regexp::MatchInfo {
        text: pat.to_string(),
        start,
        end: start + plen,
        groups: vec![Some(pat.to_string())],
    }
}

pub fn install_number(it: &mut Interp, p: &super::value::ObjRef, ctor: &Value) {
    def(it, p, "toString", 1, |it, this, args| {
        let n = match this.to_number().to_string().parse::<f64>() {
            Ok(v) => v,
            Err(_) => it.to_number_val(&this)?,
        };
        let radix = match arg(&args, 0) {
            Value::Undefined => 10,
            v => v.to_integer() as u32,
        };
        if radix == 10 || !n.is_finite() {
            return Ok(Value::string(super::value::number_to_string(n)));
        }
        if !(2..=36).contains(&radix) {
            return Err(it.range_error("toString() radix must be between 2 and 36"));
        }
        Ok(Value::string(to_radix(n, radix)))
    });
    def(it, p, "valueOf", 0, |_it, this, _args| {
        Ok(Value::Number(this.to_number()))
    });
    def(it, p, "toFixed", 1, |it, this, args| {
        let n = it.to_number_val(&this)?;
        let digits = arg(&args, 0).to_integer().clamp(0.0, 100.0) as usize;
        if !n.is_finite() {
            return Ok(Value::string(super::value::number_to_string(n)));
        }
        Ok(Value::string(format!("{n:.digits$}")))
    });
    def(it, p, "toPrecision", 1, |it, this, args| {
        let n = it.to_number_val(&this)?;
        match arg(&args, 0) {
            Value::Undefined => Ok(Value::string(super::value::number_to_string(n))),
            v => {
                let p = v.to_integer() as usize;
                Ok(Value::string(format!("{n:.p$}")))
            }
        }
    });
    def(it, p, "toExponential", 1, |it, this, args| {
        let n = it.to_number_val(&this)?;
        let digits = match arg(&args, 0) {
            Value::Undefined => 6,
            v => v.to_integer().clamp(0.0, 100.0) as usize,
        };
        Ok(Value::string(format!(
            "{:e}",
            format!("{n:.digits$}").parse::<f64>().unwrap_or(0.0)
        )))
    });
    def(it, p, "toLocaleString", 1, |it, this, _args| {
        let n = it.to_number_val(&this)?;
        // German-style grouping is what taz (and most EU sites) expects
        if n.fract() == 0.0 && n.abs() < 1e15 {
            let i = n as i64;
            let s = i.abs().to_string();
            let mut out = String::new();
            for (k, ch) in s.chars().enumerate() {
                if k > 0 && (s.len() - k) % 3 == 0 {
                    out.push('.');
                }
                out.push(ch);
            }
            return Ok(Value::string(if i < 0 { format!("-{out}") } else { out }));
        }
        Ok(Value::string(super::value::number_to_string(n)))
    });

    if let Some(c) = ctor.as_object() {
        for (n, v) in [
            ("MAX_VALUE", f64::MAX),
            ("MIN_VALUE", f64::MIN_POSITIVE),
            ("NaN", f64::NAN),
            ("POSITIVE_INFINITY", f64::INFINITY),
            ("NEGATIVE_INFINITY", f64::NEG_INFINITY),
            ("MAX_SAFE_INTEGER", 9007199254740991.0),
            ("MIN_SAFE_INTEGER", -9007199254740991.0),
            ("EPSILON", f64::EPSILON),
        ] {
            c.borrow_mut().define_internal(n, Value::Number(v));
        }
        def(it, c, "isInteger", 1, |_it, _t, args| {
            let v = arg(&args, 0);
            Ok(Value::Boolean(
                v.is_number() && v.to_number().fract() == 0.0 && v.to_number().is_finite(),
            ))
        });
        def(it, c, "isSafeInteger", 1, |_it, _t, args| {
            let v = arg(&args, 0);
            Ok(Value::Boolean(
                v.is_number()
                    && v.to_number().fract() == 0.0
                    && v.to_number().abs() <= 9007199254740991.0,
            ))
        });
        def(it, c, "isFinite", 1, |_it, _t, args| {
            let v = arg(&args, 0);
            Ok(Value::Boolean(v.is_number() && v.to_number().is_finite()))
        });
        def(it, c, "isNaN", 1, |_it, _t, args| {
            let v = arg(&args, 0);
            Ok(Value::Boolean(v.is_number() && v.to_number().is_nan()))
        });
    }
}

/// Integer/fraction conversion to bases 2…36.
fn to_radix(n: f64, radix: u32) -> String {
    let digits = "0123456789abcdefghijklmnopqrstuvwxyz";
    let neg = n < 0.0;
    let abs = n.abs();
    let mut int_part = abs.trunc();
    let mut frac = abs.fract();
    let mut out = String::new();
    if int_part == 0.0 {
        out.push('0');
    }
    while int_part > 0.0 {
        let d = (int_part % radix as f64) as usize;
        out.push(digits.chars().nth(d).unwrap());
        int_part = (int_part / radix as f64).trunc();
    }
    if frac > 0.0 {
        out.push('.');
        for _ in 0..20 {
            frac *= radix as f64;
            let d = frac.trunc() as usize;
            out.push(digits.chars().nth(d).unwrap());
            frac -= frac.trunc();
            if frac == 0.0 {
                break;
            }
        }
    }
    let mut res: String = out.chars().rev().collect();
    // the integer part was reversed together with the fraction; redo cleanly
    if res.contains('.') {
        let (a, b) = res.split_once('.').unwrap();
        let ia: String = a.chars().rev().collect();
        res = format!("{ia}.{b}");
    }
    if neg {
        res.insert(0, '-');
    }
    res
}

pub fn install_boolean(it: &mut Interp, p: &super::value::ObjRef) {
    def(it, p, "toString", 0, |_it, this, _args| {
        let b = match &this {
            Value::Boolean(b) => *b,
            Value::Object(o) => match &o.borrow().kind {
                super::value::Kind::Wrapper(v) => v.to_boolean(),
                _ => true,
            },
            _ => true,
        };
        Ok(Value::string(if b { "true" } else { "false" }))
    });
    def(it, p, "valueOf", 0, |_it, this, _args| {
        Ok(Value::Boolean(match &this {
            Value::Boolean(b) => *b,
            Value::Object(o) => match &o.borrow().kind {
                super::value::Kind::Wrapper(v) => v.to_boolean(),
                _ => true,
            },
            _ => true,
        }))
    });
}

/// Helper: call a replacement function value.
impl Value {
    fn call_value_shim(&self, it: &mut Interp, args: Vec<Value>) -> JsResult<String> {
        match self.as_object() {
            Some(o) => it.call(o, Value::Undefined, args).map(|v| v.to_string()),
            None => Ok(String::new()),
        }
    }
}

/// Splice pre-computed replacement strings into `s` at each match.
fn apply_matches(s: &str, matches: &[super::regexp::MatchInfo], reps: Vec<String>) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0usize;
    for (m, rep) in matches.iter().zip(reps.into_iter()) {
        while i < m.start.min(chars.len()) {
            out.push(chars[i]);
            i += 1;
        }
        out.push_str(&rep);
        i = m.end.max(i);
    }
    while i < chars.len() {
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Pull a compiled regex out of a RegExp object value.
pub fn regexp_of(v: &Value) -> Option<super::regexp::RegExp> {
    v.as_object().and_then(|o| match &o.borrow().kind {
        super::value::Kind::RegExp(re) => Some((**re).clone()),
        _ => None,
    })
}

/// Escape a literal string so it can be used as a pattern.
pub fn escape_regexp(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if "\\^$.|?*+()[]{}".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Unused-import guard.
pub(crate) fn _keep(_: Rc<str>) {}
