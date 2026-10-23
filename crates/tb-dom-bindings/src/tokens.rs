//! `DOMTokenList` — `element.classList`.
//!
//! The list is cached per element (`el.classList === el.classList`), its
//! `length`/`value` are live accessors and `add/remove/toggle` write straight
//! back into the `class` attribute. The numeric indices (`list[0]`) are a
//! snapshot taken whenever `el.classList` is evaluated — documented deviation,
//! real code uses `item(i)` or `contains`.

use std::rc::Rc;

use tb_dom::{Dom, NodeId};
use tb_js::value::{Key, ObjRef, Value};
use tb_js::{Interp, JsResult};

use crate::host::{method, ro_prop, rw_prop, Host, HostRef};

/// Live token list for `el.classList`.
pub fn token_list(it: &mut Interp, h: &mut Host, id: NodeId) -> Value {
    let _ = it;
    let proto = h.protos.token_list.clone();
    let obj = h.aux(id, "DOMTokenList", proto);
    refresh(&obj, &h.dom, id);
    obj
}

/// Re-read the class attribute into `0..n` (see module docs).
fn refresh(obj: &Value, dom: &Dom, id: NodeId) {
    let Some(o) = obj.as_object() else { return };
    let tokens: Vec<String> = dom
        .element(id)
        .map(|e| e.classes().iter().map(|c| c.to_string()).collect())
        .unwrap_or_default();
    let previous = o
        .borrow()
        .get_own(&Key::Name(Rc::from("@@n")))
        .and_then(|p| p.value().cloned())
        .map(|v| v.to_integer() as i64)
        .unwrap_or(0);
    {
        let mut b = o.borrow_mut();
        for i in 0..previous.max(0) {
            b.delete(&Key::Num(i as f64));
        }
        b.define_internal("@@n", Value::Number(tokens.len() as f64));
        for (i, t) in tokens.iter().enumerate() {
            b.define_value(&i.to_string(), Value::string(t));
        }
    }
}

fn tokens_of(dom: &Dom, id: NodeId) -> Vec<String> {
    dom.element(id)
        .map(|e| e.classes().iter().map(|c| c.to_string()).collect())
        .unwrap_or_default()
}

fn write_tokens(h: &mut Host, id: NodeId, tokens: &[String]) {
    let joined = tokens.join(" ");
    if joined.is_empty() {
        h.dom.remove_attr(id, "class");
    } else {
        h.dom.set_attr(id, "class", joined);
    }
    h.touch();
}

/// Normalise the arguments of add/remove (`"a  b"`, `"a", "b"` — both fine).
fn token_args(it: &mut Interp, args: &[Value]) -> JsResult<Vec<String>> {
    let mut out = Vec::new();
    for a in args {
        let s = it.to_string_val(a)?;
        for part in s.split_whitespace() {
            if part.is_empty() {
                return Err(it.type_error("token must not be empty"));
            }
            if !out.iter().any(|t: &String| t == part) {
                out.push(part.to_string());
            }
        }
    }
    Ok(out)
}

pub fn install(it: &mut Interp, host: &HostRef, proto: &ObjRef) {
    ro_prop(it, host, proto, "length", |it, h, id| {
        let _ = it;
        Ok(Value::Number(tokens_of(&h.dom, id).len() as f64))
    });
    rw_prop(
        it,
        host,
        proto,
        "value",
        |it, h, id, _| {
            let _ = it;
            Ok(Value::string(h.dom.attr(id, "class").unwrap_or_default()))
        },
        |it, h, id, v| {
            let s = it.to_string_val(&v)?;
            h.dom.set_attr(id, "class", s);
            h.touch();
            Ok(())
        },
    );
    method(it, host, proto, "item", 1, |it, h, id, args| {
        let _ = it;
        let i = args.first().map(|v| v.to_integer()).unwrap_or(-1.0);
        let tokens = tokens_of(&h.dom, id);
        if i < 0.0 || (i as usize) >= tokens.len() {
            return Ok(Value::Null);
        }
        Ok(Value::string(&tokens[i as usize]))
    });
    method(it, host, proto, "contains", 1, |it, h, id, args| {
        let want = args
            .first()
            .map(|v| it.to_string_val(v).unwrap_or_default())
            .unwrap_or_default();
        Ok(Value::Boolean(
            h.dom
                .element(id)
                .map(|e| e.has_class(&want))
                .unwrap_or(false),
        ))
    });
    method(it, host, proto, "add", 1, |it, h, id, args| {
        let add = token_args(it, &args)?;
        let mut tokens = tokens_of(&h.dom, id);
        for t in add {
            if !tokens.iter().any(|e| e == &t) {
                tokens.push(t);
            }
        }
        write_tokens(h, id, &tokens);
        Ok(Value::Undefined)
    });
    method(it, host, proto, "remove", 1, |it, h, id, args| {
        let drop = token_args(it, &args)?;
        let mut tokens = tokens_of(&h.dom, id);
        tokens.retain(|t| !drop.iter().any(|d| d == t));
        write_tokens(h, id, &tokens);
        Ok(Value::Undefined)
    });
    method(it, host, proto, "toggle", 2, |it, h, id, args| {
        let token = args
            .first()
            .map(|v| it.to_string_val(v).unwrap_or_default())
            .unwrap_or_default();
        if token.is_empty() || token.split_whitespace().count() > 1 {
            return Err(it.type_error("toggle: token must be a single token"));
        }
        let force = args
            .get(1)
            .filter(|v| !v.is_undefined())
            .map(|v| v.to_boolean());
        let mut tokens = tokens_of(&h.dom, id);
        let has = tokens.iter().any(|t| t == &token);
        let on = force.unwrap_or(!has);
        if on {
            if !has {
                tokens.push(token);
            }
        } else {
            tokens.retain(|t| t != &token);
        }
        write_tokens(h, id, &tokens);
        Ok(Value::Boolean(on))
    });
    method(it, host, proto, "replace", 2, |it, h, id, args| {
        let old = args
            .first()
            .map(|v| it.to_string_val(v).unwrap_or_default())
            .unwrap_or_default();
        let new = args
            .get(1)
            .map(|v| it.to_string_val(v).unwrap_or_default())
            .unwrap_or_default();
        let mut tokens = tokens_of(&h.dom, id);
        let mut hit = false;
        for t in tokens.iter_mut() {
            if *t == old {
                *t = new.clone();
                hit = true;
            }
        }
        if hit {
            tokens.dedup();
            write_tokens(h, id, &tokens);
        }
        Ok(Value::Boolean(hit))
    });
    // forEach runs script, so it must not hold the host borrow
    crate::host::method_unlocked(it, host, proto, "forEach", 1, |it, href, id, args| {
        let cb = args.first().cloned().unwrap_or(Value::Undefined);
        if !cb
            .as_object()
            .map(|o| o.borrow().is_function())
            .unwrap_or(false)
        {
            return Ok(Value::Undefined);
        }
        let (tokens, list) = {
            let mut h = href
                .try_borrow_mut()
                .map_err(|_| it.type_error("classList.forEach: DOM is busy"))?;
            let t = tokens_of(&h.dom, id);
            let proto = h.protos.token_list.clone();
            let l = h.aux(id, "DOMTokenList", proto);
            (t, l)
        };
        for (i, t) in tokens.iter().enumerate() {
            it.call_value(
                &cb,
                list.clone(),
                vec![Value::string(t), Value::Number(i as f64), list.clone()],
            )?;
        }
        Ok(Value::Undefined)
    });
    method(it, host, proto, "toString", 0, |it, h, id, _args| {
        let _ = it;
        Ok(Value::string(h.dom.attr(id, "class").unwrap_or_default()))
    });
}
