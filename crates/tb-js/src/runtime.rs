//! Interpreter core: engine state, property access, conversions, calls.
//!
//! Statements are in [`crate::stmts`], expressions in [`crate::exprs`],
//! promises & timers in [`crate::jobs`], standard library in [`crate::builtins`].
//! They are all `impl Interp` blocks, so engine state has exactly one owner.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use super::ast as js;
use super::env::{self, Env, EnvRef};
use super::value::{Key, Kind, Obj, ObjRef, PrimitiveHint, Prop, Value};

/// A native (Rust) function: `(interp, this, args) -> Result<Value, thrown>`.
pub type NativeFn = dyn Fn(&mut Interp, Value, Vec<Value>) -> Result<Value, Value>;

/// What a function object really is.
pub enum FnKind {
    Native(Rc<NativeFn>),
    /// callable *and* `new`-able (`Array`, `Date`, `Promise`, …)
    NativeCtor {
        call: Option<Rc<NativeFn>>,
        /// instances get this prototype
        proto: ObjRef,
    },
    User {
        f: Rc<js::Function>,
        closure: EnvRef,
        /// object the method was defined on (`super.x` looks there)
        home: Option<Value>,
    },
    Class {
        decl: Rc<js::ClassDecl>,
        closure: EnvRef,
    },
    Bound {
        target: Value,
        this: Value,
        args: Vec<Value>,
    },
}

pub struct FnData {
    pub name: Rc<str>,
    pub length: u32,
    pub kind: FnKind,
}

impl std::fmt::Debug for FnData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Fn({})", self.name)
    }
}

/// Control flow out of statement execution.
#[derive(Debug, Clone)]
pub enum Flow {
    Normal,
    Break(Option<Rc<str>>),
    Continue(Option<Rc<str>>),
    Return(Value),
}

/// `Err(value)` means "a value was thrown" (usually an `Error` object).
pub type JsResult<T> = Result<T, Value>;

pub struct Timer {
    pub id: u32,
    pub due: f64,
    /// `Some(ms)` for `setInterval`
    pub every: Option<f64>,
    pub cb: Value,
    pub args: Vec<Value>,
}

/// A queued microtask.
pub type Job = Rc<dyn Fn(&mut Interp)>;

/// Prototypes and constructors that built-ins and the runtime need.
#[derive(Default, Clone)]
pub struct Natives {
    pub object_proto: Option<ObjRef>,
    pub array_proto: Option<ObjRef>,
    pub fn_proto: Option<ObjRef>,
    pub string_proto: Option<ObjRef>,
    pub number_proto: Option<ObjRef>,
    pub bool_proto: Option<ObjRef>,
    pub error_proto: Option<ObjRef>,
    pub date_proto: Option<ObjRef>,
    pub regexp_proto: Option<ObjRef>,
    pub promise_proto: Option<ObjRef>,
    pub map_proto: Option<ObjRef>,
    pub set_proto: Option<ObjRef>,
    pub iterator_proto: Option<ObjRef>,
    pub object_ctor: Option<ObjRef>,
    pub array_ctor: Option<ObjRef>,
    pub promise_ctor: Option<ObjRef>,
    pub error_ctor: Option<ObjRef>,
    pub type_error_ctor: Option<ObjRef>,
    pub range_error_ctor: Option<ObjRef>,
    pub syntax_error_ctor: Option<ObjRef>,
    pub uri_error_ctor: Option<ObjRef>,
    pub eval_error_ctor: Option<ObjRef>,
    pub date_ctor: Option<ObjRef>,
    pub regexp_ctor: Option<ObjRef>,
    pub map_ctor: Option<ObjRef>,
    pub set_ctor: Option<ObjRef>,
    pub fn_ctor: Option<ObjRef>,
}

/// One JavaScript engine instance (one per browsing context).
pub struct Interp {
    /// `globalThis`; host bindings install themselves here
    pub globals: ObjRef,
    /// scope chain of the running script
    pub env: EnvRef,
    pub natives: Natives,
    /// microtasks (`Promise` jobs, `queueMicrotask`)
    pub queue: VecDeque<Job>,
    /// `setTimeout` / `setInterval`
    pub timers: Vec<Timer>,
    pub next_timer: u32,
    /// virtual millisecond clock — deterministic, hosts advance it
    pub clock: f64,
    /// captured `console.*` output, oldest first
    pub console: Vec<(String, String)>,
    /// echo console output to stdout as well
    pub echo_console: bool,
    /// things worth telling the user about (unsupported features, throws)
    pub problems: Vec<String>,
    depth: u32,
    pub steps: u64,
    /// budget so a hostile page cannot wedge the browser
    pub step_limit: u64,
    /// set while a derived constructor still owes a `super()` call
    pub awaiting_super: bool,
    /// cycle guard for `JSON.stringify` / `structuredClone`
    pub(crate) marked: Rc<RefCell<Vec<ObjRef>>>,
    /// deterministic PRNG state for `Math.random`
    pub rand_state: u64,
    /// label of the loop that is about to run (set by `Labeled`, claimed by the
    /// loop itself) — needed because `continue outer` has to be answered *inside*
    /// the loop, not when the flow bubbles out of it
    pub(crate) loop_label: Option<std::rc::Rc<str>>,
    /// value of the last expression statement that ran — the *completion value*
    /// a script (and `try`/`if`/block statements inside it) evaluates to
    pub last_value: Value,
}

impl Interp {
    pub fn new() -> Interp {
        let mut it = Interp {
            globals: Rc::new(RefCell::new(Obj::plain())),
            env: Env::global(),
            natives: Natives::default(),
            queue: VecDeque::new(),
            timers: Vec::new(),
            next_timer: 1,
            clock: 0.0,
            console: Vec::new(),
            echo_console: false,
            problems: Vec::new(),
            depth: 0,
            steps: 0,
            step_limit: 30_000_000,
            awaiting_super: false,
            marked: Rc::new(RefCell::new(Vec::new())),
            rand_state: 0x2545F4914F6CDD1D,
            loop_label: None,
            last_value: Value::Undefined,
        };
        super::builtins::install(&mut it);
        it
    }

    // ---------------------------------------------------------------- budget

    pub fn tick(&mut self) -> JsResult<()> {
        self.steps += 1;
        if self.steps > self.step_limit {
            self.steps = 0;
            return Err(self.type_error("Script exceeded its execution budget"));
        }
        if self.depth > 500 {
            return Err(self.range_error("Maximum call stack size exceeded"));
        }
        Ok(())
    }

    pub fn note(&mut self, msg: impl Into<String>) {
        let m = msg.into();
        if !self.problems.contains(&m) && self.problems.len() < 200 {
            self.problems.push(m);
        }
    }

    // ------------------------------------------------------------ construction

    pub fn plain_object(&mut self) -> Value {
        let mut o = Obj::plain();
        o.proto = self.natives.object_proto.clone();
        Value::Object(Rc::new(RefCell::new(o)))
    }

    /// A fresh plain object, as a direct reference (no `Value` unwrapping).
    pub fn plain_ref(&mut self) -> ObjRef {
        let mut o = Obj::plain();
        o.proto = self.natives.object_proto.clone();
        Rc::new(RefCell::new(o))
    }

    pub fn object_with(&mut self, proto: Option<ObjRef>) -> Value {
        let mut o = Obj::plain();
        o.proto = proto;
        Value::Object(Rc::new(RefCell::new(o)))
    }

    pub fn array(&mut self, items: Vec<Value>) -> Value {
        let mut o = Obj::array();
        o.proto = self.natives.array_proto.clone();
        if let Kind::Array { items: dst } = &mut o.kind {
            *dst = items;
        }
        Value::Object(Rc::new(RefCell::new(o)))
    }

    /// Wrap a Rust closure as a JS function.
    pub fn native_fn(
        &mut self,
        name: &str,
        length: u32,
        f: impl Fn(&mut Interp, Value, Vec<Value>) -> Result<Value, Value> + 'static,
    ) -> Value {
        self.native_fn_rc(name, length, Rc::new(f))
    }

    pub fn native_fn_rc(&mut self, name: &str, length: u32, f: Rc<NativeFn>) -> Value {
        let mut o = Obj::plain();
        o.proto = self.natives.fn_proto.clone();
        o.kind = Kind::Fn(Rc::new(FnData {
            name: Rc::from(name),
            length,
            kind: FnKind::Native(f),
        }));
        let obj = Rc::new(RefCell::new(o));
        obj.borrow_mut()
            .define_internal("name", Value::string(name));
        obj.borrow_mut()
            .define_internal("length", Value::Number(length as f64));
        Value::Object(obj)
    }

    /// A constructor function whose `new` produces `proto` instances.
    pub fn native_ctor(&mut self, name: &str, proto: &ObjRef, call: Option<Rc<NativeFn>>) -> Value {
        let mut o = Obj::plain();
        o.proto = self.natives.fn_proto.clone();
        o.kind = Kind::Fn(Rc::new(FnData {
            name: Rc::from(name),
            length: 1,
            kind: FnKind::NativeCtor {
                call,
                proto: proto.clone(),
            },
        }));
        let obj = Rc::new(RefCell::new(o));
        obj.borrow_mut()
            .define_internal("name", Value::string(name));
        obj.borrow_mut()
            .define_internal("prototype", Value::Object(proto.clone()));
        proto
            .borrow_mut()
            .define_internal("constructor", Value::Object(obj.clone()));
        Value::Object(obj)
    }

    // ---------------------------------------------------------------- errors

    pub fn error_with(&mut self, kind: &str, message: &str) -> Value {
        let ctor_proto = match kind {
            "TypeError" => self.proto_of_ctor(self.natives.type_error_ctor.clone()),
            "RangeError" => self.proto_of_ctor(self.natives.range_error_ctor.clone()),
            "SyntaxError" => self.proto_of_ctor(self.natives.syntax_error_ctor.clone()),
            "URIError" => self.proto_of_ctor(self.natives.uri_error_ctor.clone()),
            "EvalError" => self.proto_of_ctor(self.natives.eval_error_ctor.clone()),
            _ => self.natives.error_proto.clone(),
        };
        let mut o = Obj::plain();
        o.kind = Kind::Error;
        o.proto = ctor_proto.or_else(|| self.natives.error_proto.clone());
        let obj = Rc::new(RefCell::new(o));
        obj.borrow_mut()
            .define_internal("name", Value::string(kind));
        obj.borrow_mut()
            .define_internal("message", Value::string(message));
        obj.borrow_mut().define_internal(
            "stack",
            Value::string(format!("{kind}: {message}\n    at <eval>")),
        );
        Value::Object(obj)
    }

    fn proto_of_ctor(&self, ctor: Option<ObjRef>) -> Option<ObjRef> {
        let c = ctor?;
        let v = c
            .borrow()
            .get_own(&Key::Name(Rc::from("prototype")))
            .and_then(|p| p.value().cloned())?;
        v.as_object().cloned()
    }

    pub fn type_error(&mut self, msg: &str) -> Value {
        self.error_with("TypeError", msg)
    }
    pub fn range_error(&mut self, msg: &str) -> Value {
        self.error_with("RangeError", msg)
    }
    pub fn syntax_error(&mut self, msg: &str) -> Value {
        self.error_with("SyntaxError", msg)
    }
    pub fn reference_error(&mut self, msg: &str) -> Value {
        self.error_with("ReferenceError", msg)
    }

    /// Wrap a compiled regular expression as a JS RegExp object.
    pub fn regexp_value(&mut self, re: super::regexp::RegExp) -> Value {
        let mut o = Obj::plain();
        o.kind = Kind::RegExp(Rc::new(re));
        o.proto = self.natives.regexp_proto.clone();
        let obj = Rc::new(RefCell::new(o));
        let (src, flags) = {
            let b = obj.borrow();
            (b.kind.source_of(), b.kind.flags_of())
        };
        let last = if flags.contains('g') { 0.0 } else { -1.0 };
        {
            let mut b = obj.borrow_mut();
            b.define_internal("source", Value::string(src));
            b.define_internal("flags", Value::string(flags));
            b.define_internal("lastIndex", Value::Number(last));
        }
        Value::Object(obj)
    }

    // ------------------------------------------------------- property access

    /// Walk `obj`'s chain looking for `key`.
    fn chain_get(&mut self, start: &ObjRef, key: &Key, this: &Value) -> JsResult<Value> {
        let mut cur = Some(start.clone());
        while let Some(o) = cur {
            // read everything we need, then drop the borrow before any call
            let (own, next, arr) = {
                let b = o.borrow();
                let arr = if b.is_array() {
                    match key {
                        Key::Name(n) if &**n == "length" => {
                            b.items().map(|v| Value::Number(v.len() as f64))
                        }
                        Key::Num(n) => {
                            if let Some(items) = b.items() {
                                match items.get(*n as usize) {
                                    // an explicit hole behaves like undefined
                                    Some(v) => Some(v.clone()),
                                    None => Some(Value::Undefined),
                                }
                            } else {
                                None
                            }
                        }
                        _ => None,
                    }
                } else {
                    None
                };
                (b.get_own(key).cloned(), b.proto.clone(), arr)
            };
            if let Some(prop) = own {
                return match prop.kind {
                    super::value::PropKind::Data(v) => Ok(v),
                    super::value::PropKind::Access { getter, .. } => match getter {
                        Some(g) => self.call(&g, this.clone(), Vec::new()),
                        None => Ok(Value::Undefined),
                    },
                };
            }
            if let Some(v) = arr {
                return Ok(v);
            }
            cur = next;
        }
        Ok(Value::Undefined)
    }

    /// `obj[key]` — strings, numbers and booleans get their prototype methods.
    pub fn get(&mut self, obj: &Value, key: &Key) -> JsResult<Value> {
        match obj {
            Value::Object(o) => self.chain_get(o, key, obj),
            Value::Str(s) => match key {
                Key::Num(n) => Ok(s
                    .chars()
                    .nth(*n as usize)
                    .map(|c| Value::string(c.to_string()))
                    .unwrap_or(Value::Undefined)),
                Key::Name(name) if &**name == "length" => {
                    Ok(Value::Number(s.chars().count() as f64))
                }
                _ => match self.natives.string_proto.clone() {
                    Some(p) => self.chain_get(&p, key, obj),
                    None => Ok(Value::Undefined),
                },
            },
            Value::Number(_) => match self.natives.number_proto.clone() {
                Some(p) => self.chain_get(&p, key, obj),
                None => Ok(Value::Undefined),
            },
            Value::Boolean(_) => match self.natives.bool_proto.clone() {
                Some(p) => self.chain_get(&p, key, obj),
                None => Ok(Value::Undefined),
            },
            Value::Undefined | Value::Null => Err(self.type_error(&format!(
                "Cannot read properties of {} (reading '{}')",
                obj.type_of(),
                key.name()
            ))),
        }
    }

    /// `obj[key] = value`, honouring setters, read-only props and arrays.
    pub fn set(&mut self, obj: &Value, key: &Key, v: Value) -> JsResult<()> {
        let o = match obj.as_object() {
            Some(o) => o.clone(),
            None => {
                return Err(self.type_error(&format!(
                    "Cannot set properties of {} (setting '{}')",
                    obj.type_of(),
                    key.name()
                )))
            }
        };
        // arrays: index and length are stored in Kind::Array
        let is_arr = o.borrow().is_array();
        if is_arr {
            match key {
                Key::Name(n) if &**n == "length" => {
                    let new_len = v.to_integer().max(0.0) as usize;
                    if let Kind::Array { items } = &mut o.borrow_mut().kind {
                        items.resize(new_len, Value::Undefined);
                    }
                    return Ok(());
                }
                Key::Num(n) => {
                    let i = *n as usize;
                    if i <= 5_000_000 {
                        let mut b = o.borrow_mut();
                        if let Kind::Array { items } = &mut b.kind {
                            if i < items.len() {
                                items[i] = v;
                            } else {
                                while items.len() < i {
                                    items.push(Value::Undefined);
                                }
                                items.push(v);
                            }
                        }
                        return Ok(());
                    }
                }
                _ => {}
            }
        }
        let own = o.borrow().get_own(key).cloned();
        if let Some(prop) = own {
            return match prop.kind {
                super::value::PropKind::Access { setter, .. } => {
                    if let Some(s) = setter {
                        self.call(&s, obj.clone(), vec![v])?;
                    }
                    Ok(())
                }
                super::value::PropKind::Data(_) => {
                    if !prop.writable {
                        // sloppy mode ignores failed writes
                        return Ok(());
                    }
                    let mut b = o.borrow_mut();
                    if let Some(i) = b.find(key) {
                        b.props[i].1 = Prop {
                            kind: super::value::PropKind::Data(v),
                            enumerable: b.props[i].1.enumerable,
                            writable: b.props[i].1.writable,
                        };
                    }
                    Ok(())
                }
            };
        }
        // an accessor on the prototype chain wins over shadowing
        let mut cur = o.borrow().proto.clone();
        while let Some(p) = cur {
            let hit = p.borrow().get_own(key).cloned();
            if let Some(prop) = hit {
                match prop.kind {
                    super::value::PropKind::Access { setter, .. } => {
                        if let Some(s) = setter {
                            self.call(&s, obj.clone(), vec![v])?;
                        }
                        return Ok(());
                    }
                    super::value::PropKind::Data(_) => {
                        if !prop.writable {
                            return Err(self.type_error(&format!(
                                "Cannot assign to read only property '{}' of {}",
                                key.name(),
                                o.borrow().class_name()
                            )));
                        }
                        break;
                    }
                }
            }
            let next = p.borrow().proto.clone();
            cur = next;
        }
        if o.borrow().extensible {
            o.borrow_mut().props.push((key.clone(), Prop::data(v)));
        }
        Ok(())
    }

    /// `key in obj`
    pub fn has_prop(&mut self, obj: &Value, key: &Key) -> bool {
        let mut cur = obj.as_object().cloned();
        while let Some(o) = cur {
            let (found, next, arr_hit) = {
                let b = o.borrow();
                let arr_hit = if b.is_array() {
                    match key {
                        Key::Name(n) => &**n == "length",
                        Key::Num(n) => b.items().map(|v| (*n as usize) < v.len()).unwrap_or(false),
                    }
                } else {
                    false
                };
                (b.get_own(key).is_some(), b.proto.clone(), arr_hit)
            };
            if found || arr_hit {
                return true;
            }
            cur = next;
        }
        // strings support `"0" in "abc"`
        if let Value::Str(s) = obj {
            let n = s.chars().count();
            match key {
                Key::Name(k) => {
                    if &**k == "length" {
                        return true;
                    }
                    if let Ok(i) = k.parse::<usize>() {
                        return i < n;
                    }
                }
                Key::Num(i) => return (*i as usize) < n,
            }
        }
        false
    }

    pub fn delete_prop(&mut self, obj: &Value, key: &Key) -> bool {
        match obj.as_object() {
            Some(o) => o.borrow_mut().delete(key),
            None => true,
        }
    }

    /// Own+inherited enumerable keys, prototype chain included (`for-in`).
    pub fn enumerable_keys(&mut self, obj: &Value) -> Vec<String> {
        let mut out = Vec::new();
        let mut seen: Vec<String> = Vec::new();
        let mut cur = obj.as_object().cloned();
        while let Some(o) = cur {
            let (keys, next) = {
                let b = o.borrow();
                (b.own_keys(), b.proto.clone())
            };
            for k in keys {
                if !seen.contains(&k) {
                    seen.push(k.clone());
                    out.push(k);
                }
            }
            cur = next;
        }
        out
    }

    // ------------------------------------------------------------ conversions

    /// `ToPrimitive` via `valueOf` / `toString`.
    pub fn to_primitive(&mut self, v: &Value, hint: PrimitiveHint) -> JsResult<Value> {
        let obj = match v.as_object() {
            Some(o) => o.clone(),
            None => return Ok(v.clone()),
        };
        // arrays stringify by joining, and we own Array.prototype.join anyway
        if obj.borrow().is_array() && hint == PrimitiveHint::String {
            let items: Vec<Value> = obj.borrow().items().cloned().unwrap_or_default();
            let mut parts = Vec::with_capacity(items.len());
            for it in &items {
                match it {
                    Value::Undefined | Value::Null => parts.push(String::new()),
                    other => parts.push(self.to_string_val(other)?),
                }
            }
            return Ok(Value::string(parts.join(",")));
        }
        let order: [&str; 2] = match hint {
            PrimitiveHint::String => ["toString", "valueOf"],
            PrimitiveHint::Number => ["valueOf", "toString"],
        };
        for name in order {
            let f = self.get(v, &Key::Name(Rc::from(name)))?;
            if let Some(fo) = f.as_object() {
                if fo.borrow().is_function() {
                    let out = self.call(fo, v.clone(), Vec::new())?;
                    if !out.is_object() {
                        return Ok(out);
                    }
                }
            }
        }
        let s = obj.borrow().default_string();
        Ok(Value::string(s))
    }

    pub fn to_string_val(&mut self, v: &Value) -> JsResult<String> {
        match v {
            Value::Str(s) => Ok(s.to_string()),
            Value::Object(_) => match self.to_primitive(v, PrimitiveHint::String)? {
                Value::Str(s) => Ok(s.to_string()),
                other => Ok(other.to_string()),
            },
            other => Ok(other.to_string()),
        }
    }

    pub fn to_number_val(&mut self, v: &Value) -> JsResult<f64> {
        match v {
            Value::Number(n) => Ok(*n),
            Value::Object(_) => {
                let p = self.to_primitive(v, PrimitiveHint::Number)?;
                Ok(p.to_number())
            }
            other => Ok(other.to_number()),
        }
    }

    pub fn to_key(&mut self, v: &Value) -> JsResult<Key> {
        match v {
            Value::Str(s) => Ok(Key::from_str(s)),
            Value::Number(n) => Ok(Key::Num(*n)),
            other => Ok(Key::Name(Rc::from(self.to_string_val(other)?))),
        }
    }

    /// `RelationalComparison` for `<`, `>`, `<=`, `>=`.
    pub fn compare(&mut self, a: &Value, b: &Value) -> JsResult<bool> {
        let pa = self.to_primitive(a, PrimitiveHint::Number)?;
        let pb = self.to_primitive(b, PrimitiveHint::Number)?;
        if pa.is_string() && pb.is_string() {
            return Ok(pa.to_string() < pb.to_string());
        }
        let x = pa.to_number();
        let y = pb.to_number();
        if x.is_nan() || y.is_nan() {
            return Ok(false);
        }
        if x == y {
            return Ok(false);
        }
        Ok(x < y)
    }

    /// `instanceof`
    pub fn instance_of(&mut self, value: &Value, ctor: &Value) -> JsResult<bool> {
        let c = match ctor.as_object() {
            Some(c) => c.clone(),
            None => return Err(self.type_error("Right-hand side of 'instanceof' is not callable")),
        };
        let proto = c
            .borrow()
            .get_own(&Key::Name(Rc::from("prototype")))
            .and_then(|p| p.value().cloned());
        let target = match proto.and_then(|v| v.as_object().cloned()) {
            Some(p) => p,
            None => return Ok(false),
        };
        let mut cur = match value.as_object() {
            Some(o) => o.borrow().proto.clone(),
            None => return Ok(false),
        };
        while let Some(o) = cur {
            if Rc::ptr_eq(&o, &target) {
                return Ok(true);
            }
            let next = o.borrow().proto.clone();
            cur = next;
        }
        Ok(false)
    }

    // ------------------------------------------------------------------ calls

    /// Invoke a function value with an explicit `this`.
    pub fn call(&mut self, f: &ObjRef, this: Value, args: Vec<Value>) -> JsResult<Value> {
        self.tick()?;
        let data = f.borrow().fn_data().cloned();
        let Some(data) = data else {
            return Err(self.type_error(&format!(
                "{} is not a function",
                super::value::Value::Object(f.clone()).inspect()
            )));
        };
        self.depth += 1;
        // a function body has its own completion value: the caller's must not
        // leak in and the callee's must not leak out
        let saved = std::mem::replace(&mut self.last_value, Value::Undefined);
        let out = self.dispatch(&data, this, args);
        self.last_value = saved;
        self.depth -= 1;
        out
    }

    pub fn call_value(&mut self, f: &Value, this: Value, args: Vec<Value>) -> JsResult<Value> {
        match f.as_object() {
            Some(o) => self.call(o, this, args),
            None => Err(self.type_error(&format!("{} is not a function", f.inspect()))),
        }
    }

    /// `obj[name](args)`; `Ok(None)` when the property is missing or not callable.
    pub fn call_method(
        &mut self,
        obj: &Value,
        name: &str,
        args: Vec<Value>,
    ) -> JsResult<Option<Value>> {
        let f = self.get(obj, &Key::Name(Rc::from(name)))?;
        match f.as_object() {
            Some(o) if o.borrow().is_function() => Ok(Some(self.call(o, obj.clone(), args)?)),
            _ => Ok(None),
        }
    }

    fn dispatch(&mut self, data: &FnData, this: Value, args: Vec<Value>) -> JsResult<Value> {
        match &data.kind {
            FnKind::Native(f) => f(self, this, args),
            FnKind::NativeCtor { call, .. } => match call {
                Some(f) => f(self, this, args),
                None => {
                    Err(self.type_error(&format!("{} cannot be called without new", data.name)))
                }
            },
            FnKind::Bound {
                target,
                this: bound,
                args: pre,
            } => {
                let mut all = pre.clone();
                all.extend(args);
                let t = if bound.is_undefined() {
                    this
                } else {
                    bound.clone()
                };
                self.call_value(target, t, all)
            }
            FnKind::User { f, closure, home } => {
                let f = f.clone();
                let env = Env::new(env::ScopeKind::Function, Some(closure.clone()));
                if !f.is_arrow {
                    env.borrow_mut().this = Some(if this.is_undefined() {
                        self.globals_value()
                    } else {
                        this
                    });
                }
                env.borrow_mut().home = home.clone();
                env.borrow_mut().is_async = f.is_async;
                self.bind_params(&f, &env, &args, false)?;
                if f.is_generator {
                    self.note("generators are not supported; running the body eagerly");
                }
                if f.is_async {
                    // The body runs now; `await` inside it drains the job queue,
                    // and the promise we return settles with the body's result.
                    let (p, _resolve, _reject) = self.new_promise();
                    let out = self.run_body(&f, &env);
                    match out {
                        Ok(v) => self.resolve(&p, &v),
                        Err(e) => self.reject(&p, &e),
                    }
                    self.run_jobs(5_000);
                    Ok(Value::Object(p))
                } else {
                    self.run_body(&f, &env)
                }
            }
            FnKind::Class { .. } => Err(self.type_error(&format!(
                "Class constructor {} cannot be invoked without 'new'",
                data.name
            ))),
        }
    }

    /// `new f(...args)`
    pub fn construct(&mut self, f: &ObjRef, args: Vec<Value>) -> JsResult<Value> {
        self.tick()?;
        let data = f.borrow().fn_data().cloned();
        let Some(data) = data else {
            return Err(
                self.type_error(&format!("{} is not a constructor", f.borrow().class_name()))
            );
        };
        self.depth += 1;
        let saved = std::mem::replace(&mut self.last_value, Value::Undefined);
        let out = self.construct_inner(f, &data, args);
        self.last_value = saved;
        self.depth -= 1;
        out
    }

    pub fn construct_value(&mut self, f: &Value, args: Vec<Value>) -> JsResult<Value> {
        match f.as_object() {
            Some(o) => self.construct(o, args),
            None => Err(self.type_error(&format!("{} is not a constructor", f.inspect()))),
        }
    }

    fn construct_inner(&mut self, f: &ObjRef, data: &FnData, args: Vec<Value>) -> JsResult<Value> {
        match &data.kind {
            FnKind::NativeCtor { proto, call } => {
                let mut o = Obj::plain();
                o.proto = Some(proto.clone());
                let obj = Rc::new(RefCell::new(o));
                let this = Value::Object(obj.clone());
                match call {
                    Some(c) => {
                        let r = c(self, this.clone(), args)?;
                        if r.is_object() {
                            Ok(r)
                        } else {
                            Ok(this)
                        }
                    }
                    None => Ok(this),
                }
            }
            FnKind::User {
                f: fun,
                closure,
                home,
            } => {
                let fun = fun.clone();
                let this = self.new_instance(f);
                let env = Env::new(env::ScopeKind::Function, Some(closure.clone()));
                env.borrow_mut().this = Some(this.clone());
                env.borrow_mut().new_target = Some(Value::Object(f.clone()));
                env.borrow_mut().home = home.clone();
                self.bind_params(&fun, &env, &args, false)?;
                let out = self.run_body(&fun, &env);
                match out {
                    Ok(v) if v.is_object() => Ok(v),
                    Ok(_) => Ok(this),
                    Err(e) => Err(e),
                }
            }
            FnKind::Class { decl, closure } => {
                let decl = decl.clone();
                let this = self.new_instance(f);
                let env = Env::new(env::ScopeKind::Function, Some(closure.clone()));
                env.borrow_mut().this = Some(this.clone());
                env.borrow_mut().new_target = Some(Value::Object(f.clone()));
                env.borrow_mut().home = Some(Value::Object(f.clone()));
                env.borrow_mut().class_fields = Some(decl.clone());
                match decl.ctor() {
                    Some(ctor) => {
                        let ctor = ctor.clone();
                        self.bind_params(&ctor, &env, &args, false)?;
                        if decl.super_class.is_none() {
                            self.init_class_fields(&decl, &this, &env)?;
                        }
                        let out = self.run_body(&ctor, &env);
                        match out {
                            Ok(v) if v.is_object() => Ok(v),
                            Ok(_) => Ok(this),
                            Err(e) => Err(e),
                        }
                    }
                    None => {
                        // implicit `constructor(...a) { super(...a) }`
                        if let Some(sc) = &decl.super_class {
                            let parent = self.eval(sc, closure)?;
                            let new_target = Value::Object(f.clone());
                            let pthis = self.run_parent_ctor(
                                &parent,
                                &this,
                                args.clone(),
                                &new_target,
                                &env,
                            )?;
                            self.init_class_fields(&decl, &pthis, &env)?;
                            return Ok(pthis);
                        }
                        self.init_class_fields(&decl, &this, &env)?;
                        Ok(this)
                    }
                }
            }
            FnKind::Native(_) => {
                Err(self.type_error(&format!("{} is not a constructor", data.name)))
            }
            FnKind::Bound {
                target, args: pre, ..
            } => {
                let mut all = pre.clone();
                all.extend(args);
                self.construct_value(target, all)
            }
        }
    }

    /// A fresh instance whose prototype is `f.prototype` (falling back to Object).
    fn new_instance(&mut self, f: &ObjRef) -> Value {
        let proto = f
            .borrow()
            .get_own(&Key::Name(Rc::from("prototype")))
            .and_then(|p| p.value().cloned())
            .and_then(|v| v.as_object().cloned());
        match proto {
            Some(p) => self.object_with(Some(p)),
            None => self.plain_object(),
        }
    }

    /// Bind declared parameters, including defaults, rest and destructuring.
    pub fn bind_params(
        &mut self,
        f: &js::Function,
        env: &EnvRef,
        args: &[Value],
        lexical: bool,
    ) -> JsResult<()> {
        env.borrow_mut()
            .vars
            .insert(Rc::from("arguments"), self.array(args.to_vec()));
        let mut i = 0usize;
        for p in f.params.iter() {
            if p.rest {
                let rest = self.array(args.iter().skip(i).cloned().collect());
                self.bind_pattern(&p.pattern, &rest, env, lexical)?;
                i = usize::MAX;
                continue;
            }
            let val = args.get(i).cloned().unwrap_or(Value::Undefined);
            let val = if val.is_undefined() {
                match &p.default {
                    Some(d) => self.eval(d, env)?,
                    None => Value::Undefined,
                }
            } else {
                val
            };
            self.bind_pattern(&p.pattern, &val, env, lexical)?;
            i += 1;
        }
        Ok(())
    }

    /// Run a function body and flatten control flow into a value.
    pub fn run_body(&mut self, f: &js::Function, env: &EnvRef) -> JsResult<Value> {
        self.hoist(&f.body, env);
        self.depth += 1;
        let r = self.exec_block(&f.body, env, false);
        self.depth -= 1;
        match r? {
            Flow::Return(v) => Ok(v),
            _ => Ok(Value::Undefined),
        }
    }

    /// Own enumerable property names (arrays report their indices).
    pub fn own_enumerable(&mut self, v: &Value) -> JsResult<Vec<String>> {
        match v {
            Value::Object(o) => Ok(o.borrow().own_keys()),
            Value::Str(s) => Ok((0..s.chars().count()).map(|i| i.to_string()).collect()),
            Value::Number(n) => Ok(super::value::number_to_string(*n)
                .chars()
                .enumerate()
                .map(|(i, _)| i.to_string())
                .collect()),
            _ => Ok(Vec::new()),
        }
    }

    /// Parse and run a script, returning its completion value.
    /// Syntax errors come back as `Err(String)` so the caller can log them.
    pub fn run_script(&mut self, src: &str) -> Result<Value, String> {
        let prog = match super::parser::Parser::parse_program(src) {
            Ok(p) => p,
            Err(e) => {
                let mut problems = Vec::new();
                let tolerant = super::parser::Parser::parse_tolerant(src, &mut problems);
                if tolerant.body.is_empty() {
                    return Err(format!("{e}"));
                }
                for msg in &problems {
                    self.note(format!("script: {msg}"));
                }
                tolerant
            }
        };
        match self.eval_program(&prog) {
            Ok(v) => Ok(v),
            Err(e) => {
                let msg = self.format_thrown(&e);
                self.note(format!("uncaught: {msg}"));
                Err(msg)
            }
        }
    }

    /// Run a parsed program in the global scope.
    pub fn eval_program(&mut self, prog: &js::Program) -> JsResult<Value> {
        self.hoist(
            &js::Block {
                body: prog.body.clone(),
            },
            &self.env.clone(),
        );
        self.last_value = Value::Undefined;
        for stmt in prog.body.clone() {
            // `exec` records expression statements in `last_value`, which is the
            // completion value this call returns
            self.exec(&stmt, &self.env.clone())?;
        }
        // keep the global object in sync with the global scope
        let vars = self.env.borrow().vars.clone();
        for (k, v) in vars {
            self.globals.borrow_mut().define_internal(&k, v);
        }
        Ok(self.last_value.clone())
    }

    pub fn globals_value(&self) -> Value {
        Value::Object(self.globals.clone())
    }

    /// Evaluate an expression in `env` (used by host bindings).
    pub fn eval_in(&mut self, expr: &Rc<js::Expr>, env: &EnvRef) -> JsResult<Value> {
        self.depth += 1;
        let r = self.eval(expr, env);
        self.depth -= 1;
        r
    }

    /// The iteration protocol: arrays, strings, Map/Set, `@@iterator`.
    pub fn iterate(&mut self, v: &Value) -> JsResult<Vec<Value>> {
        match v {
            Value::Str(s) => Ok(s.chars().map(|c| Value::string(c.to_string())).collect()),
            Value::Object(o) => {
                // arrays and array-likes
                let is_arr = o.borrow().is_array();
                if is_arr {
                    return Ok(o.borrow().items().cloned().unwrap_or_default());
                }
                // Map/Set and host iterables expose a "@@iterator" method
                for name in ["@@iterator", "entries", "values"] {
                    let f = self.get(v, &Key::Name(Rc::from(name)))?;
                    if let Some(fo) = f.as_object() {
                        if fo.borrow().is_function() {
                            let it = self.call(fo, v.clone(), Vec::new())?;
                            return self.drain_iterator(&it);
                        }
                    }
                }
                // plain array-likes: { length, 0: … }
                let len = self.get(v, &Key::Name(Rc::from("length")))?.to_integer();
                if len >= 0.0 && len < 1e7 {
                    let mut out = Vec::new();
                    for i in 0..len as usize {
                        out.push(self.get(v, &Key::Num(i as f64))?);
                    }
                    return Ok(out);
                }
                Err(self.type_error(&format!("{} is not iterable", o.borrow().class_name())))
            }
            _ => Err(self.type_error(&format!("{} is not iterable", v.type_of()))),
        }
    }

    fn drain_iterator(&mut self, it: &Value) -> JsResult<Vec<Value>> {
        let mut out = Vec::new();
        let next = self.get(it, &Key::Name(Rc::from("next")))?;
        if !next.is_object() {
            return Ok(out);
        }
        for _ in 0..1_000_000 {
            let step = self.call_value(&next, it.clone(), Vec::new())?;
            let done = self.get(&step, &Key::Name(Rc::from("done")))?.to_boolean();
            if done {
                break;
            }
            out.push(self.get(&step, &Key::Name(Rc::from("value")))?);
        }
        Ok(out)
    }
}

impl Default for Interp {
    fn default() -> Self {
        Interp::new()
    }
}
