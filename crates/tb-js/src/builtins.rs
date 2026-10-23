//! Standard library installation.
//!
//! Split across modules: [`crate::std_array`] (Array), [`crate::std_string`]
//! (String/Number/Boolean), [`crate::std_misc`] (Date, RegExp, Map/Set,
//! Promise). This module wires the realm together: prototypes, constructors,
//! `Object`, `Function`, the `Error` family, `Math`, `JSON`, `console` and the
//! timer functions.

use std::cell::RefCell;
use std::rc::Rc;

use super::runtime::{Interp, JsResult};
use super::std_array::array_call;
use super::std_misc::{date_call, map_call, promise_call, regexp_call, set_call};
use super::std_string::{bool_call, number_call, string_call};
use super::value::{Key, Obj, ObjRef, Prop, Value};

/// The object behind a value (builtins always pass function/object values).
pub fn obj_of(v: Value) -> ObjRef {
    v.as_object()
        .expect("builtin passed a non-object value")
        .clone()
}

/// `args[i]` or `undefined`.
pub fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).cloned().unwrap_or(Value::Undefined)
}

pub fn arg_or<'a>(args: &'a [Value], i: usize, default: &'a str) -> String {
    match args.get(i) {
        None | Some(Value::Undefined) => default.to_string(),
        Some(v) => v.to_string(),
    }
}

/// Define a method on a prototype object.
pub fn def(
    it: &mut Interp,
    proto: &ObjRef,
    name: &str,
    len: u32,
    f: impl Fn(&mut Interp, Value, Vec<Value>) -> JsResult<Value> + 'static,
) {
    let v = it.native_fn(name, len, f);
    proto.borrow_mut().define_internal(name, v);
}

/// Define a value on a prototype object.
pub fn def_val(it: &mut Interp, proto: &ObjRef, name: &str, v: Value) {
    let _ = it;
    proto.borrow_mut().define_internal(name, v);
}

/// Make a bare prototype object (its `__proto__` is set later by `link`).
fn proto_obj(it: &mut Interp) -> ObjRef {
    let mut o = Obj::plain();
    o.proto = it.natives.object_proto.clone();
    Rc::new(RefCell::new(o))
}

pub fn install(it: &mut Interp) {
    // ---------------------------------------------------------------- roots
    let object_proto = Rc::new(RefCell::new(Obj::plain()));
    let fn_proto = Rc::new(RefCell::new(Obj::plain()));
    fn_proto.borrow_mut().proto = Some(object_proto.clone());
    // Object.prototype.__proto__ === null
    it.globals.borrow_mut().proto = None;

    it.natives.object_proto = Some(object_proto.clone());
    it.natives.fn_proto = Some(fn_proto.clone());

    // Object.prototype
    install_object_proto(it, &object_proto);

    // Function.prototype must exist before any function is built
    install_fn_proto(it, &fn_proto);

    // ------------------------------------------------------- value prototypes
    let array_proto = proto_obj(it);
    let string_proto = proto_obj(it);
    let number_proto = proto_obj(it);
    let bool_proto = proto_obj(it);
    let date_proto = proto_obj(it);
    let regexp_proto = proto_obj(it);
    let promise_proto = proto_obj(it);
    let map_proto = proto_obj(it);
    let set_proto = proto_obj(it);
    let error_proto = proto_obj(it);
    let iterator_proto = proto_obj(it);

    it.natives.array_proto = Some(array_proto.clone());
    it.natives.string_proto = Some(string_proto.clone());
    it.natives.number_proto = Some(number_proto.clone());
    it.natives.bool_proto = Some(bool_proto.clone());
    it.natives.date_proto = Some(date_proto.clone());
    it.natives.regexp_proto = Some(regexp_proto.clone());
    it.natives.promise_proto = Some(promise_proto.clone());
    it.natives.map_proto = Some(map_proto.clone());
    it.natives.set_proto = Some(set_proto.clone());
    it.natives.error_proto = Some(error_proto.clone());
    it.natives.iterator_proto = Some(iterator_proto.clone());

    // errors first: `new TypeError()` needs `Error.prototype`
    error_proto
        .borrow_mut()
        .define_internal("name", Value::string("Error"));
    error_proto
        .borrow_mut()
        .define_internal("message", Value::string(""));
    let error_ctor = it.native_ctor(
        "Error",
        &error_proto,
        Some(Rc::new(error_call) as Rc<super::runtime::NativeFn>),
    );
    it.natives.error_ctor = Some(obj_of(error_ctor.clone()));
    install_error_subclass(it, &error_ctor, "TypeError");
    it.natives.type_error_ctor = Some(global_ctor(it, "TypeError"));
    install_error_subclass(it, &error_ctor, "RangeError");
    it.natives.range_error_ctor = Some(global_ctor(it, "RangeError"));
    install_error_subclass(it, &error_ctor, "SyntaxError");
    it.natives.syntax_error_ctor = Some(global_ctor(it, "SyntaxError"));
    install_error_subclass(it, &error_ctor, "ReferenceError");
    install_uri_and_eval(it, &error_ctor);
    it.natives.uri_error_ctor = Some(global_ctor(it, "URIError"));
    it.natives.eval_error_ctor = Some(global_ctor(it, "EvalError"));

    // ----------------------------------------------------------- constructors
    let array_ctor = it.native_ctor(
        "Array",
        &array_proto,
        Some(Rc::new(array_call) as Rc<super::runtime::NativeFn>),
    );
    it.natives.array_ctor = Some(obj_of(array_ctor.clone()));
    let promise_ctor = it.native_ctor(
        "Promise",
        &promise_proto,
        Some(Rc::new(promise_call) as Rc<super::runtime::NativeFn>),
    );
    it.natives.promise_ctor = Some(obj_of(promise_ctor.clone()));
    let date_ctor = it.native_ctor(
        "Date",
        &date_proto,
        Some(Rc::new(date_call) as Rc<super::runtime::NativeFn>),
    );
    it.natives.date_ctor = Some(obj_of(date_ctor.clone()));
    let regexp_ctor = it.native_ctor(
        "RegExp",
        &regexp_proto,
        Some(Rc::new(regexp_call) as Rc<super::runtime::NativeFn>),
    );
    it.natives.regexp_ctor = Some(obj_of(regexp_ctor.clone()));
    let map_ctor = it.native_ctor(
        "Map",
        &map_proto,
        Some(Rc::new(map_call) as Rc<super::runtime::NativeFn>),
    );
    it.natives.map_ctor = Some(obj_of(map_ctor.clone()));
    let set_ctor = it.native_ctor(
        "Set",
        &set_proto,
        Some(Rc::new(set_call) as Rc<super::runtime::NativeFn>),
    );
    it.natives.set_ctor = Some(obj_of(set_ctor.clone()));

    // Object as a constructor
    let object_ctor = {
        let v = it.native_ctor(
            "Object",
            &object_proto.clone(),
            Some(Rc::new(object_call) as Rc<super::runtime::NativeFn>),
        );
        v
    };
    it.natives.object_ctor = Some(obj_of(object_ctor.clone()));

    // primitive wrapper constructors
    let string_ctor = it.native_ctor(
        "String",
        &string_proto.clone(),
        Some(Rc::new(string_call) as Rc<super::runtime::NativeFn>),
    );
    let number_ctor = it.native_ctor(
        "Number",
        &number_proto.clone(),
        Some(Rc::new(number_call) as Rc<super::runtime::NativeFn>),
    );
    let bool_ctor = it.native_ctor(
        "Boolean",
        &bool_proto.clone(),
        Some(Rc::new(bool_call) as Rc<super::runtime::NativeFn>),
    );

    // --------------------------------------------- loose globals & prototypes
    // `Symbol` must exist before Array wires up `Symbol.iterator`
    super::std_misc::install_global_functions(it);
    super::std_misc::install_console(it);
    super::std_misc::install_timers(it);
    super::std_array::install_array(it, &array_proto, &array_ctor);
    super::std_string::install_string(it, &string_proto);
    super::std_string::install_number(it, &number_proto, &number_ctor);
    super::std_string::install_boolean(it, &bool_proto);
    super::std_misc::install_date(it, &date_proto, &date_ctor);
    super::std_misc::install_regexp(it, &regexp_proto, &regexp_ctor);
    super::std_misc::install_map_set(it, &map_proto, &map_ctor, &set_proto, &set_ctor);
    super::std_misc::install_promise(it, &promise_proto, &promise_ctor);
    install_object_ctor_statics(it, &obj_of(object_ctor.clone()));
    install_math(it);
    install_json(it);

    // -------------------------------------------------------------- globals
    let g = &it.globals.clone();
    for (name, val) in [
        ("Object", object_ctor),
        ("Function", Value::Object(fn_proto.clone())),
        ("Array", array_ctor),
        ("String", string_ctor),
        ("Number", number_ctor),
        ("Boolean", bool_ctor),
        ("Date", date_ctor),
        ("RegExp", regexp_ctor),
        ("Map", map_ctor),
        ("Set", set_ctor),
        ("Promise", promise_ctor),
        ("Error", error_ctor),
    ] {
        g.borrow_mut().define_internal(name, val);
    }
    // Function.prototype is not itself callable; give globals a real one
    g.borrow_mut().define_internal(
        "Function",
        it.native_fn("Function", 1, |it, _t, a| {
            it.note("Function constructor is not supported");
            let _ = a;
            Ok(it.native_fn("anonymous", 0, |_it, _t, _a| Ok(Value::Undefined)))
        }),
    );

    it.globals
        .borrow_mut()
        .define_internal("globalThis", it.globals_value());
    it.globals
        .borrow_mut()
        .define_internal("undefined", Value::Undefined);
    it.globals
        .borrow_mut()
        .define_internal("NaN", Value::Number(f64::NAN));
    it.globals
        .borrow_mut()
        .define_internal("Infinity", Value::Number(f64::INFINITY));
    // the top-level scope shares the global object
    let env_vars = it.globals.borrow().props.clone();
    for (k, p) in env_vars {
        if let Some(v) = p.value() {
            it.env
                .borrow_mut()
                .vars
                .insert(Rc::from(k.name()), v.clone());
        }
    }
}

/// Pull a constructor back out of the global object after defining it.
fn global_ctor(it: &mut Interp, name: &str) -> ObjRef {
    it.globals
        .borrow()
        .get_own(&Key::Name(Rc::from(name)))
        .and_then(|p| p.value().cloned())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_else(|| {
            let o = Rc::new(RefCell::new(Obj::plain()));
            o.borrow_mut().define_internal("name", Value::string(name));
            o
        })
}

fn install_uri_and_eval(it: &mut Interp, base: &Value) {
    for name in ["URIError", "EvalError"] {
        install_error_subclass(it, base, name);
        let v = global_ctor(it, name);
        let _ = base;
        let _ = v;
    }
}

/// `class TypeError extends Error` in the shape the runtime expects.
fn install_error_subclass(it: &mut Interp, base: &Value, name: &str) {
    let proto = proto_obj(it);
    let base_proto = base
        .as_object()
        .and_then(|o| {
            o.borrow()
                .get_own(&Key::Name(Rc::from("prototype")))
                .and_then(|p| p.value().cloned())
        })
        .and_then(|v| v.as_object().cloned());
    proto.borrow_mut().proto = base_proto;
    proto
        .borrow_mut()
        .define_internal("name", Value::string(name));
    proto
        .borrow_mut()
        .define_internal("message", Value::string(""));
    let ctor = it.native_ctor(
        name,
        &proto,
        Some(Rc::new(error_call) as Rc<super::runtime::NativeFn>),
    );
    let g = it.globals.clone();
    g.borrow_mut().define_internal(name, ctor);
}

/// `new Error("msg")` — also used for the subclasses.
fn error_call(it: &mut Interp, this: Value, args: Vec<Value>) -> JsResult<Value> {
    if let Some(o) = this.as_object() {
        if let Some(name) = o
            .borrow()
            .get_own(&Key::Name(Rc::from("name")))
            .and_then(|p| p.value().cloned())
        {
            let _ = name;
        }
        let msg = match args.get(0) {
            None | Some(Value::Undefined) => String::new(),
            Some(v) => it.to_string_val(v).unwrap_or_default(),
        };
        o.borrow_mut()
            .define_internal("message", Value::string(msg.clone()));
        let stack = format!(
            "{}: {msg}\n    at <eval>",
            o.borrow()
                .get_own(&Key::Name(Rc::from("name")))
                .and_then(|p| p.value().cloned())
                .map(|v| v.to_string())
                .unwrap_or_else(|| "Error".into())
        );
        o.borrow_mut()
            .define_internal("stack", Value::string(stack));
        o.borrow_mut().kind = super::value::Kind::Error;
    }
    Ok(this)
}

// ---------------------------------------------------------------------- Object

fn install_object_proto(it: &mut Interp, p: &ObjRef) {
    def(it, p, "toString", 0, |_it, this, _args| match &this {
        Value::Undefined => Ok(Value::string("[object Undefined]")),
        Value::Null => Ok(Value::string("[object Null]")),
        Value::Object(o) => Ok(Value::string(format!(
            "[object {}]",
            o.borrow().class_name()
        ))),
        other => Ok(Value::string(format!("[object {}]", cap(other.type_of())))),
    });
    def(it, p, "valueOf", 0, |_it, this, _args| Ok(this));
    def(it, p, "hasOwnProperty", 1, |it, this, args| {
        let key = it.to_key(&arg(&args, 0))?;
        let found = match this.as_object() {
            Some(o) => {
                let b = o.borrow();
                let own = b.get_own(&key).is_some();
                if !own && b.is_array() {
                    if let Key::Num(n) = &key {
                        return Ok(Value::Boolean(
                            b.items().map(|v| (*n as usize) < v.len()).unwrap_or(false),
                        ));
                    }
                    if let Key::Name(n) = &key {
                        if &**n == "length" {
                            return Ok(Value::Boolean(true));
                        }
                    }
                }
                own
            }
            None => false,
        };
        Ok(Value::Boolean(found))
    });
    def(it, p, "propertyIsEnumerable", 1, |it, this, args| {
        let key = it.to_key(&arg(&args, 0))?;
        Ok(Value::Boolean(
            this.as_object()
                .map(|o| {
                    o.borrow()
                        .get_own(&key)
                        .map(|p| p.enumerable)
                        .unwrap_or(false)
                })
                .unwrap_or(false),
        ))
    });
    def(it, p, "isPrototypeOf", 1, |_it, this, args| {
        let v = arg(&args, 0);
        let mut cur = v.as_object().and_then(|o| o.borrow().proto.clone());
        let Some(target) = this.as_object() else {
            return Ok(Value::Boolean(false));
        };
        while let Some(o) = cur {
            if Rc::ptr_eq(&o, &target) {
                return Ok(Value::Boolean(true));
            }
            let next = o.borrow().proto.clone();
            cur = next;
        }
        Ok(Value::Boolean(false))
    });
    // `__proto__` accessor, because feature detection for it is everywhere
    let getter = it.native_fn("__proto__ getter", 0, |_it, this, _args| {
        match this.as_object().and_then(|o| o.borrow().proto.clone()) {
            Some(p) => Ok(Value::Object(p)),
            None => Ok(Value::Null),
        }
    });
    let setter = it.native_fn("__proto__ setter", 1, |_it, this, args| {
        if let (Some(o), Some(p)) = (this.as_object(), arg(&args, 0).as_object().cloned()) {
            o.borrow_mut().proto = Some(p);
        }
        Ok(Value::Undefined)
    });
    let mut proto_prop = Prop::accessor(getter.as_object().cloned(), setter.as_object().cloned());
    // visible to `Object.getPrototypeOf`, never to `for-in`
    proto_prop.enumerable = false;
    p.borrow_mut().define("__proto__", proto_prop);
}

fn cap(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

fn object_call(it: &mut Interp, this: Value, args: Vec<Value>) -> JsResult<Value> {
    let v = arg(&args, 0);
    if v.is_object() {
        return Ok(v);
    }
    if v.is_undefined() || v.is_null() {
        return Ok(this);
    }
    // Object(1) / Object("x") wrap primitives
    let proto = match &v {
        Value::Str(_) => it.natives.string_proto.clone(),
        Value::Number(_) => it.natives.number_proto.clone(),
        Value::Boolean(_) => it.natives.bool_proto.clone(),
        _ => it.natives.object_proto.clone(),
    };
    let out = it.object_with(proto);
    if let Some(o) = out.as_object() {
        o.borrow_mut().kind = super::value::Kind::Wrapper(v);
    }
    Ok(out)
}

fn install_object_ctor_statics(it: &mut Interp, ctor: &ObjRef) {
    def(it, ctor, "keys", 1, |it, _t, args| {
        let v = arg(&args, 0);
        let keys = it.own_enumerable(&v)?;
        Ok(it.array(keys.into_iter().map(Value::string).collect()))
    });
    def(it, ctor, "values", 1, |it, _t, args| {
        let v = arg(&args, 0);
        let keys = it.own_enumerable(&v)?;
        let mut out = Vec::new();
        for k in keys {
            out.push(it.get(&v, &Key::from_str(&k))?);
        }
        Ok(it.array(out))
    });
    def(it, ctor, "entries", 1, |it, _t, args| {
        let v = arg(&args, 0);
        let keys = it.own_enumerable(&v)?;
        let mut out = Vec::new();
        for k in keys {
            let val = it.get(&v, &Key::from_str(&k))?;
            out.push(it.array(vec![Value::string(k), val]));
        }
        Ok(it.array(out))
    });
    def(it, ctor, "assign", 2, |it, _t, args| {
        let target = arg(&args, 0);
        for src in args.iter().skip(1) {
            if src.is_nullish() {
                continue;
            }
            let keys = it.own_enumerable(src)?;
            for k in keys {
                let v = it.get(src, &Key::from_str(&k))?;
                it.set(&target, &Key::from_str(&k), v)?;
            }
        }
        Ok(target)
    });
    def(it, ctor, "create", 2, |it, _t, args| {
        let proto = arg(&args, 0);
        let obj = it.object_with(match proto.as_object() {
            Some(o) => Some(o.clone()),
            None => None,
        });
        if let Some(desc) = args.get(1) {
            apply_descriptors(it, &obj, desc)?;
        }
        Ok(obj)
    });
    def(it, ctor, "defineProperty", 3, |it, _t, args| {
        let target = arg(&args, 0);
        let key = it.to_key(&arg(&args, 1))?;
        let desc = arg(&args, 2);
        let value = it.get(&desc, &Key::Name(Rc::from("value")))?;
        let enumerable = it
            .get(&desc, &Key::Name(Rc::from("enumerable")))?
            .to_boolean();
        let writable = it
            .get(&desc, &Key::Name(Rc::from("writable")))?
            .to_boolean();
        let get_f = it.get(&desc, &Key::Name(Rc::from("get")))?;
        let set_f = it.get(&desc, &Key::Name(Rc::from("set")))?;
        if let Some(o) = target.as_object() {
            let prop = if get_f.is_object() || set_f.is_object() {
                Prop::accessor(get_f.as_object().cloned(), set_f.as_object().cloned())
            } else {
                Prop {
                    kind: super::value::PropKind::Data(value),
                    enumerable,
                    writable,
                }
            };
            o.borrow_mut().define(&key.name(), prop);
        }
        Ok(target)
    });
    def(it, ctor, "defineProperties", 2, |it, _t, args| {
        let target = arg(&args, 0);
        apply_descriptors(it, &target, &arg(&args, 1))?;
        Ok(target)
    });
    def(it, ctor, "getPrototypeOf", 1, |_it, _t, args| {
        Ok(
            match arg(&args, 0)
                .as_object()
                .and_then(|o| o.borrow().proto.clone())
            {
                Some(p) => Value::Object(p),
                None => Value::Null,
            },
        )
    });
    def(it, ctor, "setPrototypeOf", 2, |_it, _t, args| {
        if let (Some(o), Some(p)) = (
            arg(&args, 0).as_object(),
            arg(&args, 1).as_object().cloned(),
        ) {
            o.borrow_mut().proto = Some(p);
        }
        Ok(arg(&args, 0))
    });
    def(it, ctor, "freeze", 1, |_it, _t, args| {
        if let Some(o) = arg(&args, 0).as_object() {
            let mut b = o.borrow_mut();
            b.extensible = false;
            for (_, p) in b.props.iter_mut() {
                if let super::value::PropKind::Data(_) = p.kind {
                    p.writable = false;
                }
            }
        }
        Ok(arg(&args, 0))
    });
    def(it, ctor, "seal", 1, |_it, _t, args| {
        if let Some(o) = arg(&args, 0).as_object() {
            o.borrow_mut().extensible = false;
        }
        Ok(arg(&args, 0))
    });
    def(it, ctor, "preventExtensions", 1, |_it, _t, args| {
        if let Some(o) = arg(&args, 0).as_object() {
            o.borrow_mut().extensible = false;
        }
        Ok(arg(&args, 0))
    });
    def(it, ctor, "isFrozen", 1, |_it, _t, args| {
        Ok(Value::Boolean(
            arg(&args, 0)
                .as_object()
                .map(|o| !o.borrow().extensible)
                .unwrap_or(true),
        ))
    });
    def(it, ctor, "is", 2, |_it, _t, args| {
        let a = arg(&args, 0);
        let b = arg(&args, 1);
        // SameValue: NaN === NaN, +0 !== -0
        Ok(Value::Boolean(match (&a, &b) {
            (Value::Number(x), Value::Number(y)) => x.to_bits() == y.to_bits(),
            _ => a.strict_eq(&b),
        }))
    });
    def(it, ctor, "fromEntries", 1, |it, _t, args| {
        let v = arg(&args, 0);
        let out = it.plain_object();
        let items = it.iterate(&v).unwrap_or_default();
        for pair in items {
            let k = it.get(&pair, &Key::Num(0.0))?;
            let val = it.get(&pair, &Key::Num(1.0))?;
            let key = it.to_key(&k)?;
            it.set(&out, &key, val)?;
        }
        Ok(out)
    });
    def(it, ctor, "getOwnPropertyNames", 1, |it, _t, args| {
        let names = match arg(&args, 0).as_object() {
            Some(o) => {
                let b = o.borrow();
                let mut v: Vec<String> = b.props.iter().map(|(k, _)| k.name()).collect();
                if let Some(items) = b.items() {
                    for i in 0..items.len() {
                        v.push(i.to_string());
                    }
                }
                v
            }
            None => Vec::new(),
        };
        Ok(it.array(names.into_iter().map(Value::string).collect()))
    });
    def(it, ctor, "getOwnPropertyDescriptor", 2, |it, _t, args| {
        let target = arg(&args, 0);
        let key = it.to_key(&arg(&args, 1))?;
        let prop = target
            .as_object()
            .and_then(|o| o.borrow().get_own(&key).cloned());
        Ok(match prop {
            Some(p) => {
                let d = it.plain_object();
                match &p.kind {
                    super::value::PropKind::Data(v) => {
                        it.set(&d, &Key::Name(Rc::from("value")), v.clone())?;
                        it.set(
                            &d,
                            &Key::Name(Rc::from("writable")),
                            Value::Boolean(p.writable),
                        )?;
                    }
                    super::value::PropKind::Access { getter, setter } => {
                        it.set(
                            &d,
                            &Key::Name(Rc::from("get")),
                            getter
                                .as_ref()
                                .map(|g| Value::Object(g.clone()))
                                .unwrap_or(Value::Undefined),
                        )?;
                        it.set(
                            &d,
                            &Key::Name(Rc::from("set")),
                            setter
                                .as_ref()
                                .map(|s| Value::Object(s.clone()))
                                .unwrap_or(Value::Undefined),
                        )?;
                    }
                }
                it.set(
                    &d,
                    &Key::Name(Rc::from("enumerable")),
                    Value::Boolean(p.enumerable),
                )?;
                d
            }
            None => Value::Undefined,
        })
    });
    def(it, ctor, "groupBy", 2, |it, _t, args| {
        let items = it.iterate(&arg(&args, 0))?;
        let fnv = arg(&args, 1);
        let out = it.plain_object();
        // group first (pure Rust), then write the buckets out
        let mut groups: Vec<(String, Vec<Value>)> = Vec::new();
        for v in items {
            let k = it.call_value(&fnv, Value::Undefined, vec![v.clone()])?;
            let key = it.to_string_val(&k)?;
            match groups.iter_mut().find(|(name, _)| *name == key) {
                Some((_, list)) => list.push(v),
                None => groups.push((key, vec![v])),
            }
        }
        for (key, list) in groups {
            let arr = it.array(list);
            it.set(&out, &Key::from_str(&key), arr)?;
        }
        Ok(out)
    });
}

fn apply_descriptors(it: &mut Interp, target: &Value, desc: &Value) -> JsResult<()> {
    let keys = it.own_enumerable(desc)?;
    for k in keys {
        let d = it.get(desc, &Key::from_str(&k))?;
        let key = Key::from_str(&k);
        let value = it.get(&d, &Key::Name(Rc::from("value")))?;
        let enumerable = it.get(&d, &Key::Name(Rc::from("enumerable")))?.to_boolean();
        let writable = it.get(&d, &Key::Name(Rc::from("writable")))?.to_boolean();
        if let Some(o) = target.as_object() {
            o.borrow_mut().define(
                &key.name(),
                Prop {
                    kind: super::value::PropKind::Data(value),
                    enumerable,
                    writable,
                },
            );
        }
    }
    Ok(())
}

// ------------------------------------------------------------------ Function

fn install_fn_proto(it: &mut Interp, p: &ObjRef) {
    def(it, p, "call", 1, |it, this, args| {
        let mut rest = args;
        let this_arg = if rest.is_empty() {
            Value::Undefined
        } else {
            rest.remove(0)
        };
        match this.as_object() {
            Some(o) => it.call(o, this_arg, rest),
            None => Err(it.type_error("target is not a function")),
        }
    });
    def(it, p, "apply", 2, |it, this, args| {
        let this_arg = arg(&args, 0);
        let list = arg(&args, 1);
        let argv = if list.is_nullish() {
            Vec::new()
        } else {
            it.iterate(&list).unwrap_or_default()
        };
        match this.as_object() {
            Some(o) => it.call(o, this_arg, argv),
            None => Err(it.type_error("target is not a function")),
        }
    });
    def(it, p, "bind", 1, |it, this, args| {
        let mut rest = args;
        let this_arg = if rest.is_empty() {
            Value::Undefined
        } else {
            rest.remove(0)
        };
        let target = match this.as_object() {
            Some(o) => Value::Object(o.clone()),
            None => return Err(it.type_error("bind target is not a function")),
        };
        let name = this
            .as_object()
            .and_then(|o| o.borrow().fn_data().map(|d| d.name.clone()))
            .unwrap_or_else(|| Rc::from("bound"));
        let mut o = Obj::plain();
        o.proto = it.natives.fn_proto.clone();
        o.kind = super::value::Kind::Fn(Rc::new(super::runtime::FnData {
            name: name.clone(),
            length: 0,
            kind: super::runtime::FnKind::Bound {
                target,
                this: this_arg,
                args: rest,
            },
        }));
        let obj = Rc::new(RefCell::new(o));
        obj.borrow_mut()
            .define_internal("name", Value::string(format!("bound {}", name)));
        Ok(Value::Object(obj))
    });
    def(it, p, "toString", 0, |_it, this, _args| {
        Ok(Value::string(match this.as_object() {
            Some(o) => o.borrow().default_string(),
            None => "function () { }".into(),
        }))
    });
    def(it, p, "hasInstance", 1, |it, this, args| {
        let v = arg(&args, 0);
        Ok(Value::Boolean(it.instance_of(&v, &this)?))
    });
}

// ---------------------------------------------------------------------- Math

fn install_math(it: &mut Interp) {
    let m = it.plain_ref();
    let consts: [(&str, f64); 8] = [
        ("PI", std::f64::consts::PI),
        ("E", std::f64::consts::E),
        ("LN2", std::f64::consts::LN_2),
        ("LN10", std::f64::consts::LN_10),
        ("LOG2E", std::f64::consts::LOG2_E),
        ("LOG10E", std::f64::consts::LOG10_E),
        ("SQRT2", std::f64::consts::SQRT_2),
        ("SQRT1_2", std::f64::consts::FRAC_1_SQRT_2),
    ];
    for (n, v) in consts {
        m.borrow_mut().define_internal(n, Value::Number(v));
    }
    let unary: Vec<(&str, fn(f64) -> f64)> = vec![
        ("abs", |x| x.abs()),
        ("ceil", |x| x.ceil()),
        ("floor", |x| x.floor()),
        ("round", |x| js_round(x)),
        ("trunc", |x| x.trunc()),
        ("sqrt", |x| x.sqrt()),
        ("cbrt", |x| x.cbrt()),
        ("sin", |x| x.sin()),
        ("cos", |x| x.cos()),
        ("tan", |x| x.tan()),
        ("asin", |x| x.asin()),
        ("acos", |x| x.acos()),
        ("atan", |x| x.atan()),
        ("sinh", |x| x.sinh()),
        ("cosh", |x| x.cosh()),
        ("tanh", |x| x.tanh()),
        ("exp", |x| x.exp()),
        ("expm1", |x| x.exp_m1()),
        ("log", |x| x.ln()),
        ("log2", |x| x.log2()),
        ("log10", |x| x.log10()),
        ("log1p", |x| x.ln_1p()),
        ("sign", |x| {
            if x.is_nan() {
                f64::NAN
            } else if x > 0.0 {
                1.0
            } else if x < 0.0 {
                -1.0
            } else {
                x
            }
        }),
        ("fround", |x| x as f32 as f64),
        ("clz32", |x| {
            let n = super::value::to_uint32(x);
            n.leading_zeros() as f64
        }),
    ];
    for (name, f) in unary {
        let v = it.native_fn(name, 1, move |it, _t, args| {
            let x = it.to_number_val(&arg(&args, 0))?;
            Ok(Value::Number(f(x)))
        });
        m.borrow_mut().define_internal(name, v);
    }
    for name in ["min", "max"] {
        let is_min = name == "min";
        let v = it.native_fn(name, 2, move |it, _t, args| {
            let mut best = if is_min {
                f64::INFINITY
            } else {
                f64::NEG_INFINITY
            };
            for a in &args {
                let x = it.to_number_val(a)?;
                if x.is_nan() {
                    best = f64::NAN;
                    break;
                }
                best = if is_min { best.min(x) } else { best.max(x) };
            }
            if args.is_empty() {
                best = if is_min {
                    f64::INFINITY
                } else {
                    f64::NEG_INFINITY
                };
            }
            Ok(Value::Number(best))
        });
        m.borrow_mut().define_internal(name, v);
    }
    let v = it.native_fn("atan2", 2, |it, _t, args| {
        let y = it.to_number_val(&arg(&args, 0))?;
        let x = it.to_number_val(&arg(&args, 1))?;
        Ok(Value::Number(y.atan2(x)))
    });
    m.borrow_mut().define_internal("atan2", v);
    let v = it.native_fn("pow", 2, |it, _t, args| {
        let a = it.to_number_val(&arg(&args, 0))?;
        let b = it.to_number_val(&arg(&args, 1))?;
        Ok(Value::Number(a.powf(b)))
    });
    m.borrow_mut().define_internal("pow", v);
    let v = it.native_fn("hypot", 2, |it, _t, args| {
        let mut sum = 0.0;
        for a in &args {
            let x = it.to_number_val(a)?;
            sum += x * x;
        }
        Ok(Value::Number(sum.sqrt()))
    });
    m.borrow_mut().define_internal("hypot", v);
    // deterministic pseudo-random: reproducible renders, still uniform enough
    let v = it.native_fn("random", 0, |it, _t, _args| {
        it.rand_state = it
            .rand_state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let x = ((it.rand_state >> 33) as f64) / (u32::MAX as f64);
        Ok(Value::Number(x))
    });
    m.borrow_mut().define_internal("random", v);
    it.globals
        .borrow_mut()
        .define_internal("Math", Value::Object(m));
}

/// JS `Math.round`: -0.5 → -0, 0.5 → 1.
pub fn js_round(x: f64) -> f64 {
    if x.is_nan() || x.is_infinite() || x == 0.0 {
        return x;
    }
    (x + 0.5).floor()
}

// ---------------------------------------------------------------------- JSON

fn install_json(it: &mut Interp) {
    let j = it.plain_ref();
    let v = it.native_fn("stringify", 3, |it, _t, args| {
        let value = arg(&args, 0);
        let replacer = args.get(1).cloned();
        let space = args.get(2).cloned();
        let indent = match space {
            None | Some(Value::Undefined) | Some(Value::Null) => String::new(),
            Some(Value::Number(n)) => " ".repeat(n.max(0.0).min(10.0) as usize),
            Some(Value::Str(s)) => s.chars().take(10).collect(),
            Some(other) => other.to_string(),
        };
        match super::json::stringify(it, &value, replacer.as_ref(), &indent, 0) {
            Ok(s) => Ok(Value::string(s)),
            Err(e) => Err(e),
        }
    });
    j.borrow_mut().define_internal("stringify", v);
    let v = it.native_fn("parse", 2, |it, _t, args| {
        let text = it.to_string_val(&arg(&args, 0))?;
        match super::json::parse(&text) {
            Ok(v) => {
                super::json::link_protos(&it.natives, &v);
                if let Some(rev) = args.get(1).and_then(|v| v.as_object().cloned()) {
                    if rev.borrow().is_function() {
                        let holder = it.plain_object();
                        return it.revive(&rev, &holder, Value::string("root"), v);
                    }
                }
                Ok(v)
            }
            Err(msg) => Err(it.syntax_error(&format!("Unexpected token in JSON: {msg}"))),
        }
    });
    j.borrow_mut().define_internal("parse", v);
    it.globals
        .borrow_mut()
        .define_internal("JSON", Value::Object(j));
}
