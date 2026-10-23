//! JavaScript values: the dynamic type system (pure data).
//!
//! [`Value`] is the union of JS's value types; objects are `Rc<RefCell<Obj>>`
//! with an **insertion-ordered** property list (so `Object.keys` and `for-in`
//! match source order), a prototype pointer and a kind tag.
//!
//! Everything that would *call back into JS* (`toString`, getters, setters,
//! `Symbol.toPrimitive`) lives in [`crate::runtime::Interp`], never here: that
//! keeps `RefCell` borrows short and re-entrancy impossible to trip over.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use super::runtime::FnData;

/// Reference to a heap object.
pub type ObjRef = Rc<RefCell<Obj>>;

/// A JS value.
#[derive(Clone)]
pub enum Value {
    Undefined,
    Null,
    Boolean(bool),
    Number(f64),
    Str(Rc<str>),
    Object(ObjRef),
}

impl Value {
    pub fn string(s: impl AsRef<str>) -> Value {
        Value::Str(Rc::from(s.as_ref()))
    }

    pub fn is_undefined(&self) -> bool {
        matches!(self, Value::Undefined)
    }
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }
    pub fn is_nullish(&self) -> bool {
        matches!(self, Value::Undefined | Value::Null)
    }
    pub fn is_number(&self) -> bool {
        matches!(self, Value::Number(_))
    }
    pub fn is_string(&self) -> bool {
        matches!(self, Value::Str(_))
    }
    pub fn is_object(&self) -> bool {
        matches!(self, Value::Object(_))
    }
    pub fn as_number(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(*n),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_object(&self) -> Option<&ObjRef> {
        match self {
            Value::Object(o) => Some(o),
            _ => None,
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Boolean(b) => Some(*b),
            _ => None,
        }
    }

    /// `typeof v`
    pub fn type_of(&self) -> &'static str {
        match self {
            Value::Undefined => "undefined",
            Value::Null => "object",
            Value::Boolean(_) => "boolean",
            Value::Number(_) => "number",
            Value::Str(_) => "string",
            Value::Object(o) => match &o.borrow().kind {
                Kind::Fn(_) => "function",
                _ => "object",
            },
        }
    }

    /// `ToBoolean`
    pub fn to_boolean(&self) -> bool {
        match self {
            Value::Undefined | Value::Null => false,
            Value::Boolean(b) => *b,
            Value::Number(n) => *n != 0.0 && !n.is_nan(),
            Value::Str(s) => !s.is_empty(),
            Value::Object(_) => true,
        }
    }

    /// `ToNumber` for primitives; objects become NaN here (the interpreter has
    /// the value-aware version).
    pub fn to_number(&self) -> f64 {
        match self {
            Value::Undefined => f64::NAN,
            Value::Null => 0.0,
            Value::Boolean(b) => {
                if *b {
                    1.0
                } else {
                    0.0
                }
            }
            Value::Number(n) => *n,
            Value::Str(s) => string_to_number(s),
            Value::Object(_) => f64::NAN,
        }
    }

    pub fn to_integer(&self) -> f64 {
        let n = self.to_number();
        if n.is_nan() {
            0.0
        } else if n.is_infinite() {
            n
        } else {
            n.trunc()
        }
    }

    pub fn to_int32(&self) -> i32 {
        to_int32(self.to_number())
    }

    pub fn to_uint32(&self) -> u32 {
        to_uint32(self.to_number())
    }

    /// `ToString` for primitives; for objects the interpreter has the version
    /// that calls `toString`/`valueOf`. The fallbacks here match what JS prints
    /// for the built-in object kinds we model — it is also the `Display` of the
    /// type, so `value.to_string()` and `format!("{value}")` say the same thing.
    pub fn js_string(&self) -> String {
        match self {
            Value::Undefined => "undefined".into(),
            Value::Null => "null".into(),
            Value::Boolean(b) => (if *b { "true" } else { "false" }).into(),
            Value::Number(n) => number_to_string(*n),
            Value::Str(s) => s.to_string(),
            Value::Object(o) => o.borrow().default_string(),
        }
    }

    /// `ToPropertyKey` — used for `obj[x]`.
    pub fn to_key(&self) -> Key {
        match self {
            Value::Str(s) => Key::parse(s),
            Value::Number(n) => Key::Num(*n),
            other => Key::Name(Rc::from(other.to_string())),
        }
    }

    /// Abstract equality (`==`).
    pub fn loose_eq(&self, other: &Value) -> bool {
        use Value::*;
        match (self, other) {
            (Undefined, Undefined) | (Null, Null) => true,
            (Undefined, Null) | (Null, Undefined) => true,
            (Number(a), Number(b)) => a == b,
            (Boolean(a), Boolean(b)) => a == b,
            (Str(a), Str(b)) => a == b,
            (Number(a), Str(b)) => string_to_number(b) == *a,
            (Str(b), Number(a)) => string_to_number(b) == *a,
            (Boolean(a), other) => Value::Number(if *a { 1.0 } else { 0.0 }).loose_eq(other),
            (other, Boolean(a)) => Value::Number(if *a { 1.0 } else { 0.0 }).loose_eq(other),
            // objects fall back to their primitive form
            (Str(a), Object(o)) | (Object(o), Str(a)) => {
                a.as_ref() == o.borrow().default_string().as_str()
            }
            (Number(a), Object(o)) | (Object(o), Number(a)) => {
                string_to_number(&o.borrow().default_string()) == *a
            }
            (Object(a), Object(b)) => Rc::ptr_eq(a, b),
            _ => false,
        }
    }

    /// Strict equality (`===`).
    pub fn strict_eq(&self, other: &Value) -> bool {
        use Value::*;
        match (self, other) {
            (Undefined, Undefined) => true,
            (Null, Null) => true,
            (Boolean(a), Boolean(b)) => a == b,
            (Number(a), Number(b)) => a == b,
            (Str(a), Str(b)) => a == b,
            (Object(a), Object(b)) => Rc::ptr_eq(a, b),
            _ => false,
        }
    }

    /// How `console.log` prints a value.
    pub fn inspect(&self) -> String {
        match self {
            Value::Str(s) => format!("\"{s}\""),
            Value::Object(o) => o.borrow().inspect(0),
            other => other.to_string(),
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.js_string())
    }
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Undefined => write!(f, "undefined"),
            Value::Null => write!(f, "null"),
            Value::Boolean(b) => write!(f, "{b}"),
            Value::Number(n) => write!(f, "{n}"),
            Value::Str(s) => write!(f, "{s:?}"),
            Value::Object(o) => {
                let b = o.borrow();
                write!(f, "{}({})", b.class_name(), b.len_hint())
            }
        }
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Value {
        Value::Str(Rc::from(s))
    }
}
impl From<String> for Value {
    fn from(s: String) -> Value {
        Value::Str(Rc::from(s))
    }
}
impl From<f64> for Value {
    fn from(n: f64) -> Value {
        Value::Number(n)
    }
}
impl From<i32> for Value {
    fn from(n: i32) -> Value {
        Value::Number(n as f64)
    }
}
impl From<usize> for Value {
    fn from(n: usize) -> Value {
        Value::Number(n as f64)
    }
}
impl From<bool> for Value {
    fn from(b: bool) -> Value {
        Value::Boolean(b)
    }
}

/// Object property key.
#[derive(Debug, Clone, PartialEq)]
pub enum Key {
    Name(Rc<str>),
    Num(f64),
}

/// `"7"` → `Some(7)`, but `"07"`, `"1.5"`, `"-1"` and `""` are not indices.
fn array_index(s: &str) -> Option<u32> {
    if s.is_empty() || (s.len() > 1 && s.starts_with('0')) {
        return None;
    }
    match s.parse::<u32>() {
        // 2^32-1 is the largest array length, so it is never an index
        Ok(u32::MAX) | Err(_) => None,
        Ok(n) => Some(n),
    }
}

/// Stable reorder: integer indices ascending, remaining keys keep their order.
fn order_keys(keys: &mut Vec<String>) {
    let mut numbered: Vec<(u32, usize)> = Vec::new();
    for (i, k) in keys.iter().enumerate() {
        if let Some(n) = array_index(k) {
            numbered.push((n, i));
        }
    }
    if numbered.is_empty() {
        return;
    }
    numbered.sort_unstable_by_key(|(n, _)| *n);
    let mut moved = vec![false; keys.len()];
    let mut ordered: Vec<String> = Vec::with_capacity(keys.len());
    for (_, i) in &numbered {
        moved[*i] = true;
        ordered.push(keys[*i].clone());
    }
    for (i, k) in keys.iter().enumerate() {
        if !moved[i] {
            ordered.push(k.clone());
        }
    }
    *keys = ordered;
}

impl Key {
    /// Numeric strings become index keys (arrays depend on that).
    ///
    /// Not `FromStr`: an index key is not a lossless parse of a string.
    pub fn parse(s: &str) -> Key {
        if let Ok(i) = s.parse::<u32>() {
            Key::Num(i as f64)
        } else {
            Key::Name(Rc::from(s))
        }
    }

    pub fn name(&self) -> String {
        match self {
            Key::Name(n) => n.to_string(),
            Key::Num(n) => number_to_string(*n),
        }
    }

    pub fn value(&self) -> Value {
        match self {
            Key::Name(n) => Value::Str(n.clone()),
            Key::Num(n) => Value::Number(*n),
        }
    }
}

/// Own property.
#[derive(Debug, Clone)]
pub struct Prop {
    pub kind: PropKind,
    pub enumerable: bool,
    pub writable: bool,
}

#[derive(Debug, Clone)]
pub enum PropKind {
    Data(Value),
    Access {
        getter: Option<ObjRef>,
        setter: Option<ObjRef>,
    },
}

impl Prop {
    pub fn data(v: Value) -> Prop {
        Prop {
            kind: PropKind::Data(v),
            enumerable: true,
            writable: true,
        }
    }
    /// Built-in / internal property: present but not enumerated.
    pub fn internal(v: Value) -> Prop {
        Prop {
            kind: PropKind::Data(v),
            enumerable: false,
            writable: true,
        }
    }
    pub fn read_only(v: Value) -> Prop {
        Prop {
            kind: PropKind::Data(v),
            enumerable: false,
            writable: false,
        }
    }
    pub fn accessor(getter: Option<ObjRef>, setter: Option<ObjRef>) -> Prop {
        Prop {
            kind: PropKind::Access { getter, setter },
            enumerable: true,
            writable: true,
        }
    }
    pub fn value(&self) -> Option<&Value> {
        match &self.kind {
            PropKind::Data(v) => Some(v),
            _ => None,
        }
    }
}

/// What kind of object this is.
pub enum Kind {
    Plain,
    Array {
        items: Vec<Value>,
    },
    Fn(Rc<FnData>),
    Date(f64),
    RegExp(Rc<super::regexp::RegExp>),
    Error,
    /// `new String(…)`, `new Number(…)`, `new Boolean(…)`
    Wrapper(Value),
}

impl Kind {
    pub fn source_of(&self) -> String {
        match self {
            Kind::RegExp(re) => re.source.clone(),
            _ => String::new(),
        }
    }
    pub fn flags_of(&self) -> String {
        match self {
            Kind::RegExp(re) => re.flags.clone(),
            _ => String::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrimitiveHint {
    Number,
    String,
}

/// A JS object.
pub struct Obj {
    pub kind: Kind,
    /// insertion order matters (Object.keys / for-in / JSON.stringify)
    pub props: Vec<(Key, Prop)>,
    pub proto: Option<ObjRef>,
    pub extensible: bool,
}

impl Obj {
    pub fn plain() -> Obj {
        Obj {
            kind: Kind::Plain,
            props: Vec::new(),
            proto: None,
            extensible: true,
        }
    }

    pub fn array() -> Obj {
        Obj {
            kind: Kind::Array { items: Vec::new() },
            props: Vec::new(),
            proto: None,
            extensible: true,
        }
    }

    pub fn with_proto(proto: Option<ObjRef>) -> Obj {
        let mut o = Obj::plain();
        o.proto = proto;
        o
    }

    pub fn class_name(&self) -> &'static str {
        match &self.kind {
            Kind::Plain => "Object",
            Kind::Array { .. } => "Array",
            Kind::Fn(_) => "Function",
            Kind::Date(_) => "Date",
            Kind::RegExp(_) => "RegExp",
            Kind::Error => "Error",
            Kind::Wrapper(Value::Str(_)) => "String",
            Kind::Wrapper(Value::Number(_)) => "Number",
            Kind::Wrapper(Value::Boolean(_)) => "Boolean",
            Kind::Wrapper(_) => "Object",
        }
    }

    pub fn is_function(&self) -> bool {
        matches!(self.kind, Kind::Fn(_))
    }

    pub fn is_array(&self) -> bool {
        matches!(self.kind, Kind::Array { .. })
    }

    pub fn fn_data(&self) -> Option<&Rc<FnData>> {
        match &self.kind {
            Kind::Fn(f) => Some(f),
            _ => None,
        }
    }

    pub fn items(&self) -> Option<&Vec<Value>> {
        match &self.kind {
            Kind::Array { items } => Some(items),
            _ => None,
        }
    }

    pub fn find(&self, key: &Key) -> Option<usize> {
        self.props.iter().position(|(k, _)| k == key)
    }

    pub fn get_own(&self, key: &Key) -> Option<&Prop> {
        self.props.iter().find(|(k, _)| k == key).map(|(_, p)| p)
    }

    /// Define (or replace) an own property.
    pub fn define(&mut self, key: &str, prop: Prop) {
        let key = Key::parse(key);
        match self.find(&key) {
            Some(i) => self.props[i].1 = prop,
            None => self.props.push((key, prop)),
        }
    }

    pub fn define_value(&mut self, key: &str, v: Value) {
        self.define(key, Prop::data(v));
    }

    pub fn define_internal(&mut self, key: &str, v: Value) {
        self.define(key, Prop::internal(v));
    }

    /// Own enumerable keys, ordered the way `Object.keys`/`for-in`/JSON are:
    /// integer indices ascending, then everything else in insertion order.
    pub fn own_keys(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Kind::Array { items } = &self.kind {
            for (i, v) in items.iter().enumerate() {
                if !v.is_undefined() {
                    out.push(i.to_string());
                }
            }
        }
        for (k, p) in &self.props {
            if !p.enumerable {
                continue;
            }
            let name = k.name();
            if !out.contains(&name) {
                out.push(name);
            }
        }
        order_keys(&mut out);
        out
    }

    pub fn delete(&mut self, key: &Key) -> bool {
        if let Some(i) = self.find(key) {
            if !self.props[i].1.writable {
                return false;
            }
            self.props.remove(i);
            return true;
        }
        if let Kind::Array { items } = &mut self.kind {
            if let Key::Num(n) = key {
                let i = *n as usize;
                if i < items.len() {
                    items[i] = Value::Undefined;
                }
            }
        }
        true
    }

    /// `String(obj)` without calling user code (the interpreter has the
    /// spec-correct version that uses `toString`/`valueOf`).
    pub fn default_string(&self) -> String {
        // host objects (DOM nodes, events, …) name their class in `@@class`,
        // which is what `String(div)` and `console.log(div)` must print
        if let Some(class) = self.host_class() {
            return format!("[object {class}]");
        }
        match &self.kind {
            Kind::Array { items } => {
                let parts: Vec<String> = items
                    .iter()
                    .map(|v| match v {
                        Value::Undefined | Value::Null => String::new(),
                        other => other.to_string(),
                    })
                    .collect();
                parts.join(",")
            }
            Kind::Fn(f) => format!("function {}() {{ [code] }}", f.name),
            Kind::Date(ms) => super::std_misc::date_to_iso(*ms),
            Kind::RegExp(re) => format!("/{}/{}", re.source, re.flags),
            Kind::Error => {
                let name = self
                    .get_own(&Key::Name(Rc::from("name")))
                    .and_then(|p| p.value())
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "Error".into());
                let msg = self
                    .get_own(&Key::Name(Rc::from("message")))
                    .and_then(|p| p.value())
                    .map(|v| v.to_string())
                    .unwrap_or_default();
                if msg.is_empty() {
                    name
                } else {
                    format!("{name}: {msg}")
                }
            }
            Kind::Wrapper(inner) => inner.to_string(),
            Kind::Plain => "[object Object]".into(),
        }
    }

    /// The class a host binding declared for this object (`@@class`), if any.
    pub fn host_class(&self) -> Option<String> {
        self.get_own(&Key::Name(Rc::from("@@class")))
            .and_then(|p| p.value())
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }

    pub fn len_hint(&self) -> usize {
        match &self.kind {
            Kind::Array { items } => items.len(),
            _ => self.props.len(),
        }
    }

    /// `console.log` formatting.
    pub fn inspect(&self, depth: usize) -> String {
        match &self.kind {
            Kind::Fn(f) => format!("[Function: {}]", f.name),
            Kind::Array { items } => {
                if depth > 2 {
                    return "[ … ]".into();
                }
                let parts: Vec<String> = items
                    .iter()
                    .take(24)
                    .map(|v| inspect_value(v, depth + 1))
                    .collect();
                let more = if items.len() > 24 {
                    format!(", … +{}", items.len() - 24)
                } else {
                    String::new()
                };
                format!("[ {}{more} ]", parts.join(", "))
            }
            Kind::Date(_) | Kind::RegExp(_) | Kind::Error => self.default_string(),
            Kind::Wrapper(inner) => inspect_value(inner, depth),
            Kind::Plain => {
                if let Some(class) = self.host_class() {
                    return format!("[object {class}]");
                }
                if depth > 2 {
                    return "{ … }".into();
                }
                let mut parts: Vec<String> = Vec::new();
                for (k, p) in self.props.iter().filter(|(_, p)| p.enumerable).take(24) {
                    let v = match &p.kind {
                        PropKind::Data(v) => inspect_value(v, depth + 1),
                        PropKind::Access { .. } => "[Getter/Setter]".into(),
                    };
                    parts.push(format!("{}: {v}", k.name()));
                }
                // an object that only inherits shows that, which is very helpful
                // when debugging why a built-in method is missing
                if let Some(proto) = &self.proto {
                    if parts.is_empty() {
                        return format!("[[Prototype]] {}", proto.borrow().class_name());
                    }
                }
                if parts.is_empty() {
                    "{}".into()
                } else {
                    format!("{{ {} }}", parts.join(", "))
                }
            }
        }
    }
}

impl fmt::Debug for Obj {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}({})", self.class_name(), self.len_hint())
    }
}

fn inspect_value(v: &Value, depth: usize) -> String {
    match v {
        Value::Str(s) => format!("\"{s}\""),
        Value::Object(o) => o.borrow().inspect(depth),
        other => other.to_string(),
    }
}

// ---------------------------------------------------------------------------
// numeric helpers with JS semantics
// ---------------------------------------------------------------------------

/// `Number(s)`: empty → 0, hex allowed, `Infinity` allowed, junk → NaN.
pub fn string_to_number(s: &str) -> f64 {
    let t = s.trim();
    if t.is_empty() {
        return 0.0;
    }
    if t.eq_ignore_ascii_case("infinity") || t == "+Infinity" {
        return f64::INFINITY;
    }
    if t.eq_ignore_ascii_case("-infinity") {
        return f64::NEG_INFINITY;
    }
    let (sign, rest) = if let Some(r) = t.strip_prefix('-') {
        (-1.0, r)
    } else if let Some(r) = t.strip_prefix('+') {
        (1.0, r)
    } else {
        (1.0, t)
    };
    if let Some(hex) = rest.strip_prefix("0x").or(rest.strip_prefix("0X")) {
        return u64::from_str_radix(hex, 16)
            .map(|v| sign * v as f64)
            .unwrap_or(f64::NAN);
    }
    if let Some(bin) = rest.strip_prefix("0b").or(rest.strip_prefix("0B")) {
        return u64::from_str_radix(bin, 2)
            .map(|v| sign * v as f64)
            .unwrap_or(f64::NAN);
    }
    match rest.parse::<f64>() {
        Ok(v) => sign * v,
        Err(_) => f64::NAN,
    }
}

/// JS number formatting: `1` not `1.0`, shortest round-trip otherwise.
pub fn number_to_string(n: f64) -> String {
    if n.is_nan() {
        return "NaN".into();
    }
    if n == f64::INFINITY {
        return "Infinity".into();
    }
    if n == f64::NEG_INFINITY {
        return "-Infinity".into();
    }
    if n == 0.0 {
        return if n.is_sign_negative() {
            "-0".into()
        } else {
            "0".into()
        };
    }
    if n.fract() == 0.0 && n.abs() < 1e21 {
        return format!("{}", n as i64);
    }
    let s = format!("{n}");
    s
}

pub fn to_int32(n: f64) -> i32 {
    if !n.is_finite() || n == 0.0 {
        return 0;
    }
    let t = n.trunc();
    let m = t.rem_euclid(4294967296.0);
    if m >= 2147483648.0 {
        (m - 4294967296.0) as i32
    } else {
        m as i32
    }
}

pub fn to_uint32(n: f64) -> u32 {
    if !n.is_finite() || n == 0.0 {
        return 0;
    }
    let t = n.trunc();
    t.rem_euclid(4294967296.0) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truthiness() {
        for v in [
            Value::Undefined,
            Value::Null,
            Value::Number(0.0),
            Value::string(""),
            Value::Number(f64::NAN),
        ] {
            assert!(!v.to_boolean(), "{v:?} should be falsy");
        }
        for v in [
            Value::string("0"),
            Value::Number(-1.0),
            Value::Boolean(true),
        ] {
            assert!(v.to_boolean(), "{v:?} should be truthy");
        }
    }

    #[test]
    fn number_to_string_js_style() {
        assert_eq!(number_to_string(1.0), "1");
        assert_eq!(number_to_string(1.5), "1.5");
        assert_eq!(number_to_string(-0.0), "-0");
        assert_eq!(number_to_string(f64::NAN), "NaN");
        assert_eq!(number_to_string(f64::INFINITY), "Infinity");
        assert_eq!(number_to_string(1e21), "1000000000000000000000");
    }

    #[test]
    fn string_to_number_js_style() {
        assert_eq!(string_to_number(""), 0.0);
        assert_eq!(string_to_number("  42  "), 42.0);
        assert_eq!(string_to_number("0x10"), 16.0);
        assert_eq!(string_to_number("-3.5"), -3.5);
        assert!(string_to_number("abc").is_nan());
        assert!(string_to_number("12abc").is_nan());
        assert_eq!(string_to_number("Infinity"), f64::INFINITY);
    }

    #[test]
    fn equality_table() {
        assert!(Value::Undefined.loose_eq(&Value::Null));
        assert!(!Value::Undefined.strict_eq(&Value::Null));
        assert!(Value::string("1").loose_eq(&Value::Number(1.0)));
        assert!(!Value::string("1").strict_eq(&Value::Number(1.0)));
        assert!(Value::Boolean(true).loose_eq(&Value::string("1")));
        assert!(!Value::Number(f64::NAN).loose_eq(&Value::Number(f64::NAN)));
        let o = Rc::new(RefCell::new(Obj::plain()));
        assert!(Value::Object(o.clone()).strict_eq(&Value::Object(o)));
    }

    #[test]
    fn int32_wrapping() {
        assert_eq!(to_int32(1.0), 1);
        assert_eq!(to_int32(-1.0), -1);
        assert_eq!(to_uint32(-1.0), 4294967295);
        assert_eq!(to_int32(4294967296.0), 0);
        assert_eq!(to_int32(f64::NAN), 0);
    }

    #[test]
    fn object_keys_are_ordered_like_js() {
        // integer indices ascending, then insertion order (verified against V8)
        let mut o = Obj::plain();
        o.define_value("z", Value::Number(1.0));
        o.define_value("a", Value::Number(2.0));
        o.define_value("7", Value::Number(3.0));
        o.define_value("3", Value::Number(4.0));
        o.define_internal("hidden", Value::Number(5.0));
        assert_eq!(o.own_keys(), vec!["3", "7", "z", "a"]);
        // "0.5" and "-1" are ordinary string keys, not indices
        let mut p = Obj::plain();
        p.define_value("b", Value::Number(1.0));
        p.define_value("0.5", Value::Number(2.0));
        p.define_value("-1", Value::Number(3.0));
        p.define_value("3", Value::Number(4.0));
        assert_eq!(p.own_keys(), vec!["3", "b", "0.5", "-1"]);
    }

    #[test]
    fn array_stringification() {
        let mut o = Obj::array();
        if let Kind::Array { items } = &mut o.kind {
            items.push(Value::Number(1.0));
            items.push(Value::Null);
            items.push(Value::string("x"));
        }
        assert_eq!(Value::Object(Rc::new(RefCell::new(o))).to_string(), "1,,x");
    }
}
