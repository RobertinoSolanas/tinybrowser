//! `Array` — prototype methods and constructor statics.
//!
//! Arrays store their elements in `Kind::Array { items }`; `length` and index
//! access are handled by the runtime, so `a.length = 2` truncates for real.

use std::rc::Rc;

use super::builtins::{arg, def};
use super::runtime::{Interp, JsResult};
use super::value::{Key, Kind, Obj, Value};

/// `new Array(...)`: one numeric argument sets the length.
pub fn array_call(it: &mut Interp, this: Value, args: Vec<Value>) -> JsResult<Value> {
    let obj = match this.as_object() {
        Some(o) => o.clone(),
        None => return Ok(this),
    };
    if !obj.borrow().is_array() {
        obj.borrow_mut().kind = Kind::Array { items: Vec::new() };
    }
    if args.len() == 1 {
        if let Value::Number(n) = args[0] {
            if n == n.trunc() && (0.0..=20_000_000.0).contains(&n) {
                if let Kind::Array { items } = &mut obj.borrow_mut().kind {
                    *items = vec![Value::Undefined; n as usize];
                }
                return Ok(this);
            }
            return Err(it.range_error("Invalid array length"));
        }
    }
    if let Kind::Array { items } = &mut obj.borrow_mut().kind {
        *items = args;
    }
    Ok(this)
}

/// Items of an array value (copies; the caller writes back with [`set_items`]).
pub fn items(v: &Value) -> Vec<Value> {
    v.as_object()
        .and_then(|o| o.borrow().items().cloned())
        .unwrap_or_default()
}

pub fn set_items(it: &Interp, v: &Value, list: Vec<Value>) {
    if let Some(o) = v.as_object() {
        let mut b = o.borrow_mut();
        if !b.is_array() {
            b.kind = Kind::Array { items: Vec::new() };
        }
        if let Kind::Array { items } = &mut b.kind {
            *items = list;
        }
    }
    let _ = it;
}

/// Normalise a possibly-negative / missing index against `len`.
pub fn rel_index(v: &Value, len: usize, default: usize) -> usize {
    if v.is_undefined() {
        return default;
    }
    let n = v.to_integer();
    if n < 0.0 {
        (len as f64 + n).max(0.0) as usize
    } else {
        (n as usize).min(len)
    }
}

pub fn install_array(it: &mut Interp, p: &super::value::ObjRef, ctor: &Value) {
    // ----------------------------------------------------------- mutations
    def(it, p, "push", 1, |_it, this, args| {
        if let Some(o) = this.as_object() {
            let mut b = o.borrow_mut();
            if !b.is_array() {
                b.kind = Kind::Array { items: Vec::new() };
            }
            if let Kind::Array { items } = &mut b.kind {
                items.extend(args);
                return Ok(Value::Number(items.len() as f64));
            }
        }
        Ok(Value::Number(0.0))
    });
    def(it, p, "pop", 0, |_it, this, _args| {
        if let Some(o) = this.as_object() {
            let mut b = o.borrow_mut();
            if let Kind::Array { items } = &mut b.kind {
                return Ok(items.pop().unwrap_or(Value::Undefined));
            }
        }
        Ok(Value::Undefined)
    });
    def(it, p, "shift", 0, |_it, this, _args| {
        if let Some(o) = this.as_object() {
            let mut b = o.borrow_mut();
            if let Kind::Array { items } = &mut b.kind {
                return Ok(if items.is_empty() {
                    Value::Undefined
                } else {
                    items.remove(0)
                });
            }
        }
        Ok(Value::Undefined)
    });
    def(it, p, "unshift", 1, |_it, this, args| {
        if let Some(o) = this.as_object() {
            let mut b = o.borrow_mut();
            if let Kind::Array { items } = &mut b.kind {
                let mut out = args;
                out.extend(items.iter().cloned());
                *items = out;
                return Ok(Value::Number(items.len() as f64));
            }
        }
        Ok(Value::Number(0.0))
    });
    def(it, p, "splice", 2, |_it, this, args| {
        let list = items(&this);
        let len = list.len();
        let start = rel_index(&arg(&args, 0), len, 0);
        let count = match arg(&args, 1) {
            Value::Undefined => len - start,
            v => (v.to_integer().max(0.0) as usize).min(len - start),
        };
        let mut removed = Vec::new();
        let mut out = Vec::new();
        out.extend(list.iter().take(start).cloned());
        for v in list.iter().skip(start).take(count) {
            removed.push(v.clone());
        }
        out.extend(list.iter().skip(start + count).cloned());
        let mut inserted = Vec::new();
        for a in args.iter().skip(2) {
            inserted.push(a.clone());
        }
        out.splice(start..start, inserted);
        set_items(_it, &this, out);
        Ok(_it.array(removed))
    });
    def(it, p, "sort", 1, |it, this, args| {
        let list = items(&this);
        let cmp = arg(&args, 0);
        let result = if cmp.is_object() {
            it.sort_with(&list, &cmp)?
        } else {
            // default sort compares string forms; holes stay at the end
            let mut defined: Vec<Value> = list.into_iter().filter(|v| !v.is_undefined()).collect();
            defined.sort_by_key(|a| a.to_string());
            defined
        };
        set_items(it, &this, result);
        Ok(this)
    });
    def(it, p, "reverse", 0, |_it, this, _args| {
        let mut list = items(&this);
        list.reverse();
        set_items(_it, &this, list);
        Ok(this)
    });
    def(it, p, "fill", 1, |it, this, args| {
        let mut list = items(&this);
        let len = list.len();
        let v = arg(&args, 0);
        let start = rel_index(&arg(&args, 1), len, 0);
        let end = rel_index(&arg(&args, 2), len, len);
        for slot in list[start..end.min(len)].iter_mut() {
            *slot = v.clone();
        }
        set_items(it, &this, list);
        Ok(this)
    });

    // ------------------------------------------------------------ extraction
    def(it, p, "slice", 2, |it, this, args| {
        let list = items(&this);
        let len = list.len();
        let start = rel_index(&arg(&args, 0), len, 0);
        let end = rel_index(&arg(&args, 1), len, len);
        let out: Vec<Value> = if start < end {
            list[start..end].to_vec()
        } else {
            Vec::new()
        };
        Ok(it.array(out))
    });
    def(it, p, "concat", 1, |it, this, args| {
        let mut out = items(&this);
        for a in args {
            if a.as_object()
                .map(|o| o.borrow().is_array())
                .unwrap_or(false)
            {
                out.extend(items(&a));
            } else {
                out.push(a);
            }
        }
        Ok(it.array(out))
    });
    def(it, p, "join", 1, |it, this, args| {
        let sep = match arg(&args, 0) {
            Value::Undefined => ",".to_string(),
            v => it.to_string_val(&v)?,
        };
        let list = items(&this);
        let mut parts = Vec::with_capacity(list.len());
        for v in &list {
            match v {
                Value::Undefined | Value::Null => parts.push(String::new()),
                other => parts.push(it.to_string_val(other)?),
            }
        }
        Ok(Value::string(parts.join(&sep)))
    });
    def(it, p, "at", 1, |_it, this, args| {
        let list = items(&this);
        let n = arg(&args, 0).to_integer();
        let i = if n < 0.0 { list.len() as f64 + n } else { n };
        Ok(if i < 0.0 || i as usize >= list.len() {
            Value::Undefined
        } else {
            list[i as usize].clone()
        })
    });
    def(it, p, "flat", 0, |it, this, args| {
        let depth = match arg(&args, 0) {
            Value::Undefined => 1.0,
            v => v.to_number(),
        };
        let mut out = Vec::new();
        flatten(&items(&this), depth as i32, &mut out)?;
        Ok(it.array(out))
    });
    def(it, p, "flatMap", 1, |it, this, args| {
        let f = arg(&args, 0);
        let this_arg = arg(&args, 1);
        let mut out = Vec::new();
        for (i, v) in items(&this).into_iter().enumerate() {
            let r = it.call_value(
                &f,
                this_arg.clone(),
                vec![v, Value::Number(i as f64), this.clone()],
            )?;
            if r.as_object()
                .map(|o| o.borrow().is_array())
                .unwrap_or(false)
            {
                out.extend(items(&r));
            } else {
                out.push(r);
            }
        }
        Ok(it.array(out))
    });

    // -------------------------------------------------------------- searching
    def(it, p, "indexOf", 1, |_it, this, args| {
        let want = arg(&args, 0);
        let list = items(&this);
        let from = match arg(&args, 1) {
            Value::Undefined => 0,
            v => v.to_integer().max(0.0) as usize,
        };
        for (i, v) in list.iter().enumerate().skip(from) {
            if v.strict_eq(&want) {
                return Ok(Value::Number(i as f64));
            }
        }
        Ok(Value::Number(-1.0))
    });
    def(it, p, "lastIndexOf", 1, |_it, this, args| {
        let want = arg(&args, 0);
        let list = items(&this);
        for i in (0..list.len()).rev() {
            if list[i].strict_eq(&want) {
                return Ok(Value::Number(i as f64));
            }
        }
        Ok(Value::Number(-1.0))
    });
    def(it, p, "includes", 1, |_it, this, args| {
        let want = arg(&args, 0);
        for v in items(&this) {
            // includes uses SameValueZero: NaN is found, -0 equals 0
            if v.strict_eq(&want)
                || (v.is_number()
                    && want.is_number()
                    && v.to_number().is_nan()
                    && want.to_number().is_nan())
            {
                return Ok(Value::Boolean(true));
            }
        }
        Ok(Value::Boolean(false))
    });
    def(it, p, "find", 1, |it, this, args| {
        let f = arg(&args, 0);
        let this_arg = arg(&args, 1);
        for (i, v) in items(&this).into_iter().enumerate() {
            if it
                .call_value(
                    &f,
                    this_arg.clone(),
                    vec![v.clone(), Value::Number(i as f64), this.clone()],
                )?
                .to_boolean()
            {
                return Ok(v);
            }
        }
        Ok(Value::Undefined)
    });
    def(it, p, "findIndex", 1, |it, this, args| {
        let f = arg(&args, 0);
        let this_arg = arg(&args, 1);
        for (i, v) in items(&this).into_iter().enumerate() {
            if it
                .call_value(
                    &f,
                    this_arg.clone(),
                    vec![v, Value::Number(i as f64), this.clone()],
                )?
                .to_boolean()
            {
                return Ok(Value::Number(i as f64));
            }
        }
        Ok(Value::Number(-1.0))
    });
    def(it, p, "findLast", 1, |it, this, args| {
        let f = arg(&args, 0);
        let list = items(&this);
        for i in (0..list.len()).rev() {
            if it
                .call_value(
                    &f,
                    arg(&args, 1),
                    vec![list[i].clone(), Value::Number(i as f64), this.clone()],
                )?
                .to_boolean()
            {
                return Ok(list[i].clone());
            }
        }
        Ok(Value::Undefined)
    });

    // -------------------------------------------------------------- iteration
    def(it, p, "forEach", 1, move |it, this, args| {
        let f = arg(&args, 0);
        let this_arg = arg(&args, 1);
        for (i, v) in items(&this).into_iter().enumerate() {
            it.call_value(
                &f,
                this_arg.clone(),
                vec![v, Value::Number(i as f64), this.clone()],
            )?;
        }
        Ok(Value::Undefined)
    });
    def(it, p, "map", 1, |it, this, args| {
        let f = arg(&args, 0);
        let this_arg = arg(&args, 1);
        let mut out = Vec::new();
        for (i, v) in items(&this).into_iter().enumerate() {
            out.push(it.call_value(
                &f,
                this_arg.clone(),
                vec![v, Value::Number(i as f64), this.clone()],
            )?);
        }
        Ok(it.array(out))
    });
    def(it, p, "filter", 1, |it, this, args| {
        let f = arg(&args, 0);
        let this_arg = arg(&args, 1);
        let mut out = Vec::new();
        for (i, v) in items(&this).into_iter().enumerate() {
            if it
                .call_value(
                    &f,
                    this_arg.clone(),
                    vec![v.clone(), Value::Number(i as f64), this.clone()],
                )?
                .to_boolean()
            {
                out.push(v);
            }
        }
        Ok(it.array(out))
    });
    def(it, p, "some", 1, |it, this, args| {
        let f = arg(&args, 0);
        let this_arg = arg(&args, 1);
        for (i, v) in items(&this).into_iter().enumerate() {
            if it
                .call_value(
                    &f,
                    this_arg.clone(),
                    vec![v, Value::Number(i as f64), this.clone()],
                )?
                .to_boolean()
            {
                return Ok(Value::Boolean(true));
            }
        }
        Ok(Value::Boolean(false))
    });
    def(it, p, "every", 1, |it, this, args| {
        let f = arg(&args, 0);
        let this_arg = arg(&args, 1);
        for (i, v) in items(&this).into_iter().enumerate() {
            if !it
                .call_value(
                    &f,
                    this_arg.clone(),
                    vec![v, Value::Number(i as f64), this.clone()],
                )?
                .to_boolean()
            {
                return Ok(Value::Boolean(false));
            }
        }
        Ok(Value::Boolean(true))
    });
    def(it, p, "reduce", 2, |it, this, args| {
        let f = arg(&args, 0);
        let list = items(&this);
        let mut idx = 0usize;
        let mut acc = match args.get(1) {
            Some(v) => v.clone(),
            None => {
                if list.is_empty() {
                    return Err(it.type_error("Reduce of empty array with no initial value"));
                }
                idx = 1;
                list[0].clone()
            }
        };
        for (i, v) in list.iter().enumerate().skip(idx) {
            acc = it.call_value(
                &f,
                Value::Undefined,
                vec![acc, v.clone(), Value::Number(i as f64), this.clone()],
            )?;
        }
        Ok(acc)
    });
    def(it, p, "reduceRight", 2, |it, this, args| {
        let f = arg(&args, 0);
        let list = items(&this);
        if list.is_empty() && args.len() < 2 {
            return Err(it.type_error("Reduce of empty array with no initial value"));
        }
        let mut iter = list.iter().rev();
        let mut acc = match args.get(1) {
            Some(v) => v.clone(),
            None => iter.next().cloned().unwrap_or(Value::Undefined),
        };
        for v in iter {
            acc = it.call_value(
                &f,
                Value::Undefined,
                vec![acc, v.clone(), Value::Number(0.0), this.clone()],
            )?;
        }
        Ok(acc)
    });
    def(it, p, "keys", 0, |it, this, _args| {
        let list = items(&this);
        Ok(it.array((0..list.len()).map(|i| Value::Number(i as f64)).collect()))
    });
    def(it, p, "values", 0, |it, this, _args| {
        Ok(it.array(items(&this)))
    });
    def(it, p, "entries", 0, |it, this, _args| {
        let list = items(&this);
        let mut pairs = Vec::with_capacity(list.len());
        for (i, v) in list.into_iter().enumerate() {
            pairs.push(it.array(vec![Value::Number(i as f64), v]));
        }
        Ok(it.array(pairs))
    });
    // the iteration protocol marker: `Array.prototype[Symbol.iterator]`
    let iter_key = it
        .get(&it.globals_value(), &Key::Name(Rc::from("Symbol")))
        .ok()
        .and_then(|sym| it.get(&sym, &Key::Name(Rc::from("iterator"))).ok())
        .and_then(|v| v.as_str().map(|s| s.to_string()));
    if let Some(name) = iter_key {
        let values = it
            .get(&Value::Object(p.clone()), &Key::Name(Rc::from("values")))
            .ok();
        if let Some(v) = values {
            p.borrow_mut().define_internal(&name, v);
        }
    }
    def(it, p, "toString", 0, |it, this, _args| {
        match it.call_method(&this, "join", vec![Value::string(",")])? {
            Some(v) => Ok(v),
            None => Ok(Value::string("")),
        }
    });

    // --------------------------------------------------------- ctor statics
    if let Some(ctor) = ctor.as_object() {
        def(it, ctor, "isArray", 1, |_it, _t, args| {
            Ok(Value::Boolean(
                arg(&args, 0)
                    .as_object()
                    .map(|o| o.borrow().is_array())
                    .unwrap_or(false),
            ))
        });
        def(it, ctor, "of", 0, |it, _t, args| Ok(it.array(args)));
        def(it, ctor, "from", 1, |it, _t, args| {
            let src = arg(&args, 0);
            let map = args.get(1).cloned();
            let mut list = if src.is_string() {
                src.to_string()
                    .chars()
                    .map(|c| Value::string(c.to_string()))
                    .collect()
            } else {
                it.iterate(&src).unwrap_or_else(|_| {
                    // array-like: { length, 0:…, 1:… }
                    let len = it
                        .get(&src, &Key::Name(Rc::from("length")))
                        .map(|v| v.to_integer() as usize)
                        .unwrap_or(0);
                    (0..len)
                        .map(|i| {
                            it.get(&src, &Key::Num(i as f64))
                                .unwrap_or(Value::Undefined)
                        })
                        .collect()
                })
            };
            if let Some(m) = map {
                if m.is_object() {
                    let mut out = Vec::new();
                    for (i, v) in list.into_iter().enumerate() {
                        out.push(
                            it.call_value(&m, Value::Undefined, vec![v, Value::Number(i as f64)])
                                .unwrap_or(Value::Undefined),
                        );
                    }
                    list = out;
                }
            }
            Ok(it.array(list))
        });
        def(it, ctor, "fromAsync", 1, |it, _t, args| {
            let src = arg(&args, 0);
            let list = it.iterate(&src).unwrap_or_default();
            let arr = it.array(list);
            Ok(it.resolved_promise(&arr))
        });
    }
}

fn flatten(list: &[Value], depth: i32, out: &mut Vec<Value>) -> JsResult<()> {
    for v in list {
        let is_arr = v
            .as_object()
            .map(|o| o.borrow().is_array())
            .unwrap_or(false);
        if is_arr && depth > 0 {
            flatten(&items(v), depth - 1, out)?;
        } else {
            out.push(v.clone());
        }
    }
    Ok(())
}

/// Comparator-driven sort (kept on Interp so it can call JS and report throws).
impl Interp {
    pub fn sort_with(&mut self, list: &[Value], cmp: &Value) -> JsResult<Vec<Value>> {
        let mut v: Vec<Value> = list.to_vec();
        let mut err: Option<Value> = None;
        // insertion sort keeps the comparator call order predictable
        for i in 1..v.len() {
            let mut j = i;
            while j > 0 {
                if err.is_some() {
                    break;
                }
                let r =
                    self.call_value(cmp, Value::Undefined, vec![v[j - 1].clone(), v[j].clone()]);
                match r {
                    Ok(x) => {
                        let n = x.to_number();
                        if n.is_nan() {
                            break;
                        }
                        if n <= 0.0 {
                            break;
                        }
                        v.swap(j - 1, j);
                        j -= 1;
                    }
                    Err(e) => {
                        err = Some(e);
                        break;
                    }
                }
            }
        }
        match err {
            Some(e) => Err(e),
            None => Ok(v),
        }
    }
}

/// Unused-import guard.
pub(crate) fn _keep(_: Obj, _: Kind) {}
