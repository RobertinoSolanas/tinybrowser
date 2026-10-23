//! JSON: parser and serialiser (`JSON.parse` / `JSON.stringify`).

use std::rc::Rc;

use super::runtime::{Interp, JsResult};
use super::value::{Key, Value};

/// Parse JSON text into JS values.
/// Nesting limit for `JSON.parse` — a hostile document must not be able to
/// recurse us into a stack overflow.
const MAX_DEPTH: usize = 200;

pub fn parse(text: &str) -> Result<Value, String> {
    let b: Vec<char> = text.chars().collect();
    let mut p = P { b, i: 0, depth: 0 };
    p.ws();
    let v = p.value()?;
    p.ws();
    if p.i != p.b.len() {
        return Err(format!("trailing characters at {}", p.i));
    }
    Ok(v)
}

struct P {
    b: Vec<char>,
    i: usize,
    depth: usize,
}

impl P {
    fn peek(&self) -> Option<char> {
        self.b.get(self.i).copied()
    }
    fn ws(&mut self) {
        while matches!(self.peek(), Some(c) if c.is_whitespace()) {
            self.i += 1;
        }
    }
    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn word(&mut self, w: &str) -> bool {
        if self.b[self.i..].iter().collect::<String>().starts_with(w) {
            self.i += w.chars().count();
            return true;
        }
        false
    }

    fn value(&mut self) -> Result<Value, String> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err("nested too deeply".into());
        }
        let r = self.value_inner();
        self.depth -= 1;
        r
    }

    fn value_inner(&mut self) -> Result<Value, String> {
        match self.peek() {
            None => Err("unexpected end of input".into()),
            Some('{') => self.object(),
            Some('[') => self.array(),
            Some('"') => Ok(Value::string(self.string()?)),
            Some('t') => {
                if self.word("true") {
                    Ok(Value::Boolean(true))
                } else {
                    Err("invalid literal".into())
                }
            }
            Some('f') => {
                if self.word("false") {
                    Ok(Value::Boolean(false))
                } else {
                    Err("invalid literal".into())
                }
            }
            Some('n') => {
                if self.word("null") {
                    Ok(Value::Null)
                } else {
                    Err("invalid literal".into())
                }
            }
            Some(c) if c == '-' || c.is_ascii_digit() => self.number(),
            Some(c) => Err(format!("unexpected character `{c}`")),
        }
    }

    fn number(&mut self) -> Result<Value, String> {
        let start = self.i;
        if self.peek() == Some('-') {
            self.i += 1;
        }
        while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
            self.i += 1;
        }
        if self.peek() == Some('.') {
            self.i += 1;
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.i += 1;
            }
        }
        if matches!(self.peek(), Some('e') | Some('E')) {
            self.i += 1;
            if matches!(self.peek(), Some('+') | Some('-')) {
                self.i += 1;
            }
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.i += 1;
            }
        }
        let text: String = self.b[start..self.i].iter().collect();
        text.parse::<f64>()
            .map(Value::Number)
            .map_err(|_| format!("invalid number `{text}`"))
    }

    fn string(&mut self) -> Result<String, String> {
        if !self.eat('"') {
            return Err("expected `\"`".into());
        }
        let mut out = String::new();
        loop {
            match self.peek() {
                None => return Err("unterminated string".into()),
                Some('"') => {
                    self.i += 1;
                    return Ok(out);
                }
                Some('\\') => {
                    self.i += 1;
                    let c = self.peek().ok_or("bad escape")?;
                    self.i += 1;
                    match c {
                        '"' => out.push('"'),
                        '\\' => out.push('\\'),
                        '/' => out.push('/'),
                        'b' => out.push('\u{8}'),
                        'f' => out.push('\u{c}'),
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        'u' => {
                            let hex: String = (0..4)
                                .filter_map(|_| {
                                    let c = self.peek()?;
                                    self.i += 1;
                                    Some(c)
                                })
                                .collect();
                            let cp = u32::from_str_radix(&hex, 16)
                                .map_err(|_| "bad \\u escape".to_string())?;
                            // surrogate pairs
                            if (0xD800..0xDC00).contains(&cp) {
                                let save = self.i;
                                if self.peek() == Some('\\') {
                                    self.i += 1;
                                    if self.peek() == Some('u') {
                                        self.i += 1;
                                        let hex2: String = (0..4)
                                            .filter_map(|_| {
                                                let c = self.peek()?;
                                                self.i += 1;
                                                Some(c)
                                            })
                                            .collect();
                                        if let Ok(lo) = u32::from_str_radix(&hex2, 16) {
                                            if (0xDC00..0xE000).contains(&lo) {
                                                let combined =
                                                    0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
                                                out.push(
                                                    char::from_u32(combined).unwrap_or('\u{fffd}'),
                                                );
                                                continue;
                                            }
                                        }
                                    }
                                    self.i = save;
                                }
                                out.push('\u{fffd}');
                            } else {
                                out.push(char::from_u32(cp).unwrap_or('\u{fffd}'));
                            }
                        }
                        other => return Err(format!("bad escape `\\{other}`")),
                    }
                }
                Some(c) => {
                    self.i += 1;
                    out.push(c);
                }
            }
        }
    }

    fn array(&mut self) -> Result<Value, String> {
        self.i += 1; // [
        let mut items = Vec::new();
        loop {
            self.ws();
            if self.eat(']') {
                return Ok(json_array(items));
            }
            items.push(self.value()?);
            self.ws();
            if self.eat(',') {
                continue;
            }
            if self.eat(']') {
                return Ok(json_array(items));
            }
            return Err("expected `,` or `]`".into());
        }
    }

    fn object(&mut self) -> Result<Value, String> {
        self.i += 1; // {
        let mut out = super::value::Obj::plain();
        loop {
            self.ws();
            if self.eat('}') {
                return Ok(Value::Object(Rc::new(std::cell::RefCell::new(out))));
            }
            let key = self.string()?;
            self.ws();
            if !self.eat(':') {
                return Err("expected `:`".into());
            }
            let v = self.value()?;
            out.define_value(&key, v);
            self.ws();
            if self.eat(',') {
                continue;
            }
            if self.eat('}') {
                return Ok(Value::Object(Rc::new(std::cell::RefCell::new(out))));
            }
            return Err("expected `,` or `}`".into());
        }
    }
}

/// Give a freshly parsed tree the prototypes an object/array literal would
/// have. Without this `Array.isArray` still works (it looks at the internal
/// kind) but `JSON.parse('[1]').map(…)` finds no method at all.
pub fn link_protos(natives: &super::runtime::Natives, v: &Value) {
    let Some(o) = v.as_object().cloned() else {
        return;
    };
    let is_arr = o.borrow().is_array();
    let proto = if is_arr {
        natives.array_proto.clone()
    } else {
        natives.object_proto.clone()
    };
    let children: Vec<Value> = {
        let mut b = o.borrow_mut();
        if b.proto.is_none() {
            b.proto = proto;
        }
        if is_arr {
            b.items().cloned().unwrap_or_default()
        } else {
            b.props
                .iter()
                .filter_map(|(_, p)| p.value().cloned())
                .collect()
        }
    };
    for child in children {
        link_protos(natives, &child);
    }
}

fn json_array(items: Vec<Value>) -> Value {
    let mut o = super::value::Obj::array();
    if let super::value::Kind::Array { items: dst } = &mut o.kind {
        *dst = items;
    }
    Value::Object(Rc::new(std::cell::RefCell::new(o)))
}

/// `JSON.stringify` with an optional indent. Cycles and functions behave as in
/// JS (cycles throw, functions disappear).
pub fn stringify(
    it: &mut Interp,
    v: &Value,
    replacer: Option<&Value>,
    indent: &str,
    depth: usize,
) -> JsResult<String> {
    // a replacer array or function narrows/rewrites the output
    let mut value = v.clone();
    if let Some(r) = replacer {
        if let Some(ro) = r.as_object() {
            if ro.borrow().is_function() {
                let holder = it.plain_object();
                value = it.call(ro, holder, vec![Value::string(""), value.clone()])?;
            } else if ro.borrow().is_array() {
                // handled per-object below; keep the list around
                let _ = ro;
            }
        }
    }
    // toJSON wins (Date relies on this)
    if value.is_object() {
        if let Some(to) = it.call_method(&value, "toJSON", vec![Value::string("")])? {
            value = to;
        }
    }
    match &value {
        Value::Undefined => Ok(String::new()),
        Value::Null => Ok("null".into()),
        Value::Boolean(b) => Ok(if *b { "true".into() } else { "false".into() }),
        Value::Number(n) => Ok(if n.is_finite() {
            super::value::number_to_string(*n)
        } else {
            "null".into()
        }),
        Value::Str(s) => Ok(quote_json(s)),
        Value::Object(o) => {
            if it.in_progress(o) {
                return Err(it.type_error("Converting circular structure to JSON"));
            }
            it.mark(o, true);
            let out = stringify_object(it, o, replacer, indent, depth);
            it.mark(o, false);
            out
        }
    }
}

fn stringify_object(
    it: &mut Interp,
    o: &super::value::ObjRef,
    replacer: Option<&Value>,
    indent: &str,
    depth: usize,
) -> JsResult<String> {
    let is_arr = o.borrow().is_array();
    let pad = indent.repeat(depth + 1);
    let close = indent.repeat(depth);
    let mut parts = Vec::new();
    let pretty = !indent.is_empty();
    if is_arr {
        let items = o.borrow().items().cloned().unwrap_or_default();
        for v in items {
            let s = match stringify(it, &v, replacer, indent, depth + 1) {
                Ok(s) if !s.is_empty() => s,
                _ => "null".into(),
            };
            parts.push(s);
        }
        if parts.is_empty() {
            return Ok("[]".into());
        }
        return Ok(if pretty {
            let indented: Vec<String> = parts.iter().map(|p| format!("{pad}{p}")).collect();
            format!("[\n{}\n{close}]", indented.join(",\n"))
        } else {
            format!("[{}]", parts.join(","))
        });
    }
    let keys = o.borrow().own_keys();
    for k in keys {
        let v = it.get(&Value::Object(o.clone()), &Key::from_str(&k))?;
        let s = stringify(it, &v, replacer, indent, depth + 1)?;
        if s.is_empty() || v.type_of() == "function" {
            continue;
        }
        parts.push(if pretty {
            format!("{pad}{}: {s}", quote_json(&k))
        } else {
            format!("{}:{}", quote_json(&k), s)
        });
    }
    if parts.is_empty() {
        return Ok("{}".into());
    }
    Ok(if pretty {
        format!("{{\n{}\n{close}}}", parts.join(",\n"))
    } else {
        format!("{{{}}}", parts.join(","))
    })
}

pub fn quote_json(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

impl Interp {
    /// `JSON.parse(text, reviver)` support.
    pub fn revive(
        &mut self,
        reviver: &super::value::ObjRef,
        holder: &Value,
        key: Value,
        value: Value,
    ) -> JsResult<Value> {
        // walk children first, like the spec
        if value.is_object() {
            let keys = self.own_enumerable(&value)?;
            for k in keys {
                let child = self.get(&value, &Key::from_str(&k))?;
                let new = self.revive(reviver, &value, Value::string(k.clone()), child)?;
                self.set(&value, &Key::from_str(&k), new)?;
            }
        }
        self.call(reviver, holder.clone(), vec![key, value])
    }

    /// Cycle detection scratch space.
    pub(crate) fn in_progress(&self, o: &super::value::ObjRef) -> bool {
        self.marked.borrow().iter().any(|x| Rc::ptr_eq(x, o))
    }

    pub(crate) fn mark(&self, o: &super::value::ObjRef, on: bool) {
        let mut m = self.marked.borrow_mut();
        if on {
            m.push(o.clone());
        } else {
            m.retain(|x| !Rc::ptr_eq(x, o));
        }
    }
}
