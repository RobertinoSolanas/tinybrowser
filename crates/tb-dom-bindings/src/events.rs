//! Events: `EventTarget`, `Event`/`MouseEvent`/…, `on*` handler properties and
//! the capture → target → bubble dispatch algorithm.
//!
//! Event state lives on the event object itself as non-enumerable `"@…"`
//! properties. That has two nice consequences: our `Event` wrapper needs no
//! side table, and `el.dispatchEvent({type: 'ping'})` — a *plain object*, very
//! common in the wild — works because the dispatcher installs whatever the
//! object does not have yet.

use std::cell::RefCell;
use std::rc::Rc;

use tb_core::Phase;
use tb_js::builtins::obj_of;
use tb_js::value::{Key, Obj, ObjRef, Prop, Value};
use tb_js::{Interp, JsResult};

use crate::host::{evt_target_of, host_mut, EvtTarget, Host, HostRef};

/// Internal (invisible to scripts) event state.
pub const F_TYPE: &str = "@type";
pub const F_TARGET: &str = "@target";
pub const F_CURRENT: &str = "@current";
pub const F_BUBBLES: &str = "@bubbles";
pub const F_CANCELABLE: &str = "@cancelable";
pub const F_PREVENTED: &str = "@prevented";
pub const F_STOPPED: &str = "@stopped";
pub const F_IMMEDIATE: &str = "@immediate";
pub const F_DETAIL: &str = "@detail";
pub const F_TIME: &str = "@time";
pub const F_PHASE: &str = "@phase";

// ---------------------------------------------------------------------------
// field helpers (they never call into JS, so they are safe under a host borrow)
// ---------------------------------------------------------------------------

pub fn field(v: &Value, key: &str) -> Value {
    v.as_object()
        .and_then(|o| {
            o.borrow()
                .get_own(&Key::Name(Rc::from(key)))
                .and_then(|p| p.value().cloned())
        })
        .unwrap_or(Value::Undefined)
}

pub fn set_field(it: &mut Interp, v: &Value, key: &str, val: Value) {
    if let Some(o) = v.as_object() {
        o.borrow_mut().define_internal(key, val.clone());
    }
    // also keep the visible property in sync (plain-object events)
    let _ = it.set(v, &Key::Name(Rc::from(&key[1..])), val);
}

pub fn flag(v: &Value, key: &str) -> bool {
    field(v, key).to_boolean()
}

pub fn set_flag(v: &Value, key: &str, val: bool) {
    if let Some(o) = v.as_object() {
        o.borrow_mut().define_internal(key, Value::Boolean(val));
    }
}

/// `options.bubbles` & friends, `false` when absent.
fn bool_opt(it: &mut Interp, obj: &Value, name: &str) -> bool {
    it.get(obj, &Key::Name(Rc::from(name)))
        .map(|v| v.to_boolean())
        .unwrap_or(false)
}

/// `event.type` etc. — accessors over the internal fields.
fn ev_prop<F>(it: &mut Interp, proto: &ObjRef, name: &str, f: F)
where
    F: Fn(&mut Interp, Value) -> JsResult<Value> + 'static,
{
    let get = it.native_fn(name, 0, move |it: &mut Interp, this: Value, _| f(it, this));
    proto
        .borrow_mut()
        .define(name, Prop::accessor(Some(obj_of(get)), None));
}

// ---------------------------------------------------------------------------
// EventTarget
// ---------------------------------------------------------------------------

/// `addEventListener(type, cb, options)` — options is `true` (capture) or
/// `{capture, once, passive}`.
pub fn add_listener(
    it: &mut Interp,
    host: &HostRef,
    this: Value,
    args: Vec<Value>,
) -> JsResult<Value> {
    let Some(target) = evt_target_of(&this) else {
        return Err(it.type_error("addEventListener: `this` is not an event target"));
    };
    let kind = args
        .first()
        .map(|v| it.to_string_val(v).unwrap_or_default())
        .unwrap_or_default();
    if kind.is_empty() {
        return Ok(Value::Undefined);
    }
    let Some(cb) = args.get(1).cloned() else {
        return Ok(Value::Undefined);
    };
    if cb.is_nullish() {
        return Ok(Value::Undefined);
    }
    let callable = cb
        .as_object()
        .map(|o| o.borrow().is_function())
        .unwrap_or(false);
    if !callable {
        return Err(it.type_error("addEventListener: listener is not a function"));
    }
    let opts = args.get(2).cloned().unwrap_or(Value::Undefined);
    let (capture, once) = if let Some(b) = opts.as_bool() {
        (b, false)
    } else if let Some(o) = opts.as_object() {
        let capture = it
            .get(&opts, &Key::Name(Rc::from("capture")))
            .map(|v| v.to_boolean())
            .unwrap_or(false);
        let once = it
            .get(&opts, &Key::Name(Rc::from("once")))
            .map(|v| v.to_boolean())
            .unwrap_or(false);
        let _ = o;
        (capture, once)
    } else {
        (false, false)
    };
    let mut h = host_mut(host, it)?;
    // the spec dedupes (target, type, capture, callback) — so do we
    if h.listeners.iter().any(|l| {
        l.target == target && l.kind == kind && l.capture == capture && l.callback.strict_eq(&cb)
    }) {
        return Ok(Value::Undefined);
    }
    h.listeners.push(crate::host::Listener {
        target,
        kind,
        capture,
        once,
        callback: cb,
    });
    Ok(Value::Undefined)
}

pub fn remove_listener(
    it: &mut Interp,
    host: &HostRef,
    this: Value,
    args: Vec<Value>,
) -> JsResult<Value> {
    let Some(target) = evt_target_of(&this) else {
        return Ok(Value::Undefined);
    };
    let kind = args
        .first()
        .map(|v| it.to_string_val(v).unwrap_or_default())
        .unwrap_or_default();
    let cb = args.get(1).cloned().unwrap_or(Value::Undefined);
    let capture = match args.get(2) {
        Some(v) if v.as_bool().is_some() => v.to_boolean(),
        Some(v) if v.is_object() => it
            .get(v, &Key::Name(Rc::from("capture")))
            .map(|x| x.to_boolean())
            .unwrap_or(false),
        _ => false,
    };
    let mut h = host_mut(host, it)?;
    if let Some(i) = h.listeners.iter().position(|l| {
        l.target == target && l.kind == kind && l.capture == capture && l.callback.strict_eq(&cb)
    }) {
        h.listeners.remove(i);
    }
    Ok(Value::Undefined)
}

/// Install `addEventListener`/`removeEventListener`/`dispatchEvent` on a proto.
pub fn install_event_target(it: &mut Interp, host: &HostRef, proto: &ObjRef) {
    let h_add = host.clone();
    let add = it.native_fn(
        "addEventListener",
        2,
        move |it: &mut Interp, this: Value, args: Vec<Value>| add_listener(it, &h_add, this, args),
    );
    let h_rem = host.clone();
    let rem = it.native_fn(
        "removeEventListener",
        2,
        move |it: &mut Interp, this: Value, args: Vec<Value>| {
            remove_listener(it, &h_rem, this, args)
        },
    );
    {
        let mut b = proto.borrow_mut();
        b.define_internal("addEventListener", add);
        b.define_internal("removeEventListener", rem);
    }
    // dispatch runs script → no host borrow while it runs
    crate::host::this_method_unlocked(
        it,
        host,
        proto,
        "dispatchEvent",
        1,
        |it, href, this, args| {
            let Some(target) = evt_target_of(&this) else {
                return Err(it.type_error("dispatchEvent: `this` is not an event target"));
            };
            let ev = args.first().cloned().unwrap_or(Value::Undefined);
            if !ev.is_object() {
                return Err(it.type_error("dispatchEvent: an event object is required"));
            }
            dispatch(href, it, target, ev)
        },
    );
}

/// A `onclick="…"` attribute value becomes a function that runs the source.
/// (Real browsers compile it with `event` as a parameter; we run it as a
/// script, which covers the patterns pages actually use.)
pub fn handler_from_source(it: &mut Interp, src: &str) -> Value {
    let src = src.to_string();
    it.native_fn(
        "onhandler",
        1,
        move |it: &mut Interp, _this: Value, _a: Vec<Value>| match it.run_script(&src) {
            Ok(v) => Ok(v),
            Err(_) => Ok(Value::Undefined),
        },
    )
}

/// Everything that has to run for one node in the path: registered listeners
/// plus the `on<kind>` property and attribute.
fn callbacks_for(
    it: &mut Interp,
    host: &HostRef,
    target: EvtTarget,
    kind: &str,
    want_capture: Option<bool>,
) -> (Vec<(Value, bool)>, Value) {
    let mut h = match host.try_borrow_mut() {
        Ok(h) => h,
        Err(_) => return (Vec::new(), Value::Undefined),
    };
    let this_val = h.wrap_target(target);
    let mut out: Vec<(Value, bool)> = Vec::new();
    for l in h.listeners.iter() {
        if l.target != target || l.kind != kind {
            continue;
        }
        match want_capture {
            // the target itself runs capture and bubble listeners in order
            Some(c) if l.capture != c => continue,
            None => {}
            _ => {}
        }
        out.push((l.callback.clone(), l.once));
    }
    if let Some(map) = h.handlers.get(&target) {
        if let Some(f) = map.get(kind) {
            out.push((f.clone(), false));
        }
    }
    if let EvtTarget::Node(node) = target {
        if let Some(attr) = h.dom.attr(node.id(), &format!("on{kind}")) {
            if !attr.trim().is_empty() {
                let f = handler_from_source(it, &attr);
                out.push((f, false));
            }
        }
    }
    (out, this_val)
}

fn drop_once(host: &HostRef, target: EvtTarget, kind: &str, cb: &Value) {
    if let Ok(mut h) = host.try_borrow_mut() {
        if let Some(i) = h
            .listeners
            .iter()
            .position(|l| l.target == target && l.kind == kind && l.callback.strict_eq(cb))
        {
            h.listeners.remove(i);
        }
    }
}

/// Event path: target → ancestors → window.
fn path_for(host: &Host, target: EvtTarget) -> Vec<EvtTarget> {
    let mut out = Vec::new();
    match target {
        EvtTarget::Window => out.push(EvtTarget::Window),
        EvtTarget::Node(h) => {
            let mut cur = Some(h.id());
            while let Some(c) = cur {
                out.push(EvtTarget::node(c));
                cur = host.dom.parent(c);
                if out.len() > 4096 {
                    break;
                }
            }
            out.push(EvtTarget::Window);
        }
    }
    out
}

/// Run the whole dispatch. Returns what `dispatchEvent` returns: `false` when
/// a cancelable event had `preventDefault()` called.
pub fn dispatch(host: &HostRef, it: &mut Interp, target: EvtTarget, ev: Value) -> JsResult<Value> {
    let kind = field(&ev, F_TYPE)
        .is_undefined()
        .then(|| it.get(&ev, &Key::Name(Rc::from("type"))).ok())
        .flatten()
        .or_else(|| Some(field(&ev, F_TYPE)))
        .map(|v| it.to_string_val(&v).unwrap_or_default())
        .unwrap_or_default();
    if kind.is_empty() {
        return Err(it.type_error("dispatchEvent: event has no type"));
    }
    // make sure every field is where we need it, whatever the event object was
    set_field(it, &ev, F_TYPE, Value::string(&kind));
    let bubbles = it
        .get(&ev, &Key::Name(Rc::from("bubbles")))
        .map(|v| v.to_boolean())
        .unwrap_or_else(|_| flag(&ev, F_BUBBLES));
    let target_val = {
        let mut h = host_mut(host, it)?;
        h.wrap_target(target)
    };
    set_field(it, &ev, F_TARGET, target_val.clone());
    // a plain object needs the three methods; our Event has them already
    for (name, key) in [
        ("preventDefault", F_PREVENTED),
        ("stopPropagation", F_STOPPED),
        ("stopImmediatePropagation", F_IMMEDIATE),
    ] {
        let have = it
            .get(&ev, &Key::Name(Rc::from(name)))
            .map(|v| {
                v.as_object()
                    .map(|o| o.borrow().is_function())
                    .unwrap_or(false)
            })
            .unwrap_or(false);
        if !have {
            let k = key;
            let f = it.native_fn(name, 0, move |it: &mut Interp, this: Value, _| {
                set_flag(&this, k, true);
                if k == F_PREVENTED {
                    let _ = it.set(
                        &this,
                        &Key::Name(Rc::from("defaultPrevented")),
                        Value::Boolean(true),
                    );
                    let _ = it.set(
                        &this,
                        &Key::Name(Rc::from("returnValue")),
                        Value::Boolean(false),
                    );
                }
                Ok(Value::Undefined)
            });
            if let Some(o) = ev.as_object() {
                o.borrow_mut().define_internal(name, f);
            }
        }
    }

    let path = {
        let h = match host.try_borrow() {
            Ok(h) => h,
            Err(_) => return Err(it.type_error("dispatchEvent: DOM is busy")),
        };
        path_for(&h, target)
    };

    // 1. capture: window first, down to the parent of the target
    for t in path.iter().rev().take(path.len().saturating_sub(1)) {
        if flag(&ev, F_STOPPED) {
            break;
        }
        run_listeners(it, host, *t, &kind, Some(true), &ev, 1.0);
    }
    // 2. at target: capture and bubble listeners alike, in registration order
    if !flag(&ev, F_STOPPED) && !path.is_empty() {
        run_listeners(it, host, path[0], &kind, None, &ev, 2.0);
    }
    // 3. bubble: parent of the target up to the window
    if bubbles {
        for t in path.iter().skip(1) {
            if flag(&ev, F_STOPPED) {
                break;
            }
            run_listeners(it, host, *t, &kind, Some(false), &ev, 3.0);
        }
    }
    set_field(it, &ev, F_PHASE, Value::Number(0.0));
    if let Ok(mut h) = host.try_borrow_mut() {
        h.flush_mirrors();
    }
    Ok(Value::Boolean(!flag(&ev, F_PREVENTED)))
}

/// One node of the path. The host borrow is dropped before any callback runs.
fn run_listeners(
    it: &mut Interp,
    host: &HostRef,
    target: EvtTarget,
    kind: &str,
    capture: Option<bool>,
    ev: &Value,
    phase: f64,
) {
    let (callbacks, this_val) = callbacks_for(it, host, target, kind, capture);
    if callbacks.is_empty() {
        return;
    }
    set_field(it, ev, F_CURRENT, this_val.clone());
    // 1 capturing, 2 at target, 3 bubbling
    set_field(it, ev, F_PHASE, Value::Number(phase));
    for (cb, once) in callbacks {
        if flag(ev, F_IMMEDIATE) {
            break;
        }
        let out = it.call_value(&cb, this_val.clone(), vec![ev.clone()]);
        if once {
            drop_once(host, target, kind, &cb);
        }
        // the handler's `dataset` writes land in the attributes before the next
        // handler runs, so listeners agree on what the DOM says
        if let Ok(mut h) = host.try_borrow_mut() {
            h.flush_mirrors();
        }
        if let Err(e) = out {
            let msg = it.format_thrown(&e);
            it.log("error", format!("[{kind}] {msg}"));
            if let Ok(mut h) = host.try_borrow_mut() {
                h.problems.warn_ctx(
                    Phase::Script,
                    format!("uncaught in {kind} handler: {msg}"),
                    "",
                );
            }
        }
    }
}

/// Fire a plain bubbling event on a target (`click`, `change`, `focus`, …).
pub fn fire(
    host: &HostRef,
    it: &mut Interp,
    target: EvtTarget,
    kind: &str,
    cancelable: bool,
) -> JsResult<Value> {
    let proto = host.try_borrow().ok().and_then(|h| h.protos.event.clone());
    let o = Rc::new(std::cell::RefCell::new(Obj::plain()));
    o.borrow_mut().define_value("bubbles", Value::Boolean(true));
    o.borrow_mut()
        .define_value("cancelable", Value::Boolean(cancelable));
    let ev = make_event(it, kind, Some(Value::Object(o)), proto);
    dispatch(host, it, target, ev)
}

// ---------------------------------------------------------------------------
// Event objects
// ---------------------------------------------------------------------------

/// Build an event object. `options` may be `undefined` or
/// `{bubbles, cancelable, detail, clientX, key, …}`.
pub fn make_event(
    it: &mut Interp,
    kind: &str,
    options: Option<Value>,
    proto: Option<ObjRef>,
) -> Value {
    let mut o = Obj::plain();
    o.proto = proto;
    let ev = Rc::new(RefCell::new(o));
    {
        let mut b = ev.borrow_mut();
        b.define_internal(F_TYPE, Value::string(kind));
        b.define_internal(F_TIME, Value::Number(0.0));
        b.define_internal(F_DETAIL, Value::Null);
        b.define_internal(F_BUBBLES, Value::Boolean(false));
        b.define_internal(F_CANCELABLE, Value::Boolean(false));
        b.define_internal(F_PREVENTED, Value::Boolean(false));
        b.define_internal(F_STOPPED, Value::Boolean(false));
        b.define_internal(F_IMMEDIATE, Value::Boolean(false));
    }
    if let Some(opts) = options.filter(|v| v.is_object()) {
        let bubbles = bool_opt(it, &opts, "bubbles");
        let cancelable = bool_opt(it, &opts, "cancelable");
        let detail = it
            .get(&opts, &Key::Name(Rc::from("detail")))
            .unwrap_or(Value::Null);
        {
            let mut b = ev.borrow_mut();
            b.define_internal(F_BUBBLES, Value::Boolean(bubbles));
            b.define_internal(F_CANCELABLE, Value::Boolean(cancelable));
            if !detail.is_undefined() {
                b.define_internal(F_DETAIL, detail);
            }
        }
        // MouseEvent / KeyboardEvent payloads ride along as plain data
        for name in [
            "clientX",
            "clientY",
            "screenX",
            "screenY",
            "pageX",
            "pageY",
            "movementX",
            "movementY",
            "button",
            "buttons",
            "detail",
            "key",
            "code",
            "charCode",
            "keyCode",
            "ctrlKey",
            "shiftKey",
            "altKey",
            "metaKey",
            "relatedTarget",
            "view",
        ] {
            if let Ok(v) = it.get(&opts, &Key::Name(Rc::from(name))) {
                if !v.is_undefined() {
                    ev.borrow_mut().define_value(name, v);
                }
            }
        }
    }
    Value::Object(ev)
}

/// `Event.prototype` (+ the subclasses, which are the same shape).
pub fn install_event_proto(it: &mut Interp, proto: &ObjRef) {
    ev_prop(it, proto, "type", |_it, this| Ok(field(&this, F_TYPE)));
    ev_prop(it, proto, "target", |_it, this| Ok(field(&this, F_TARGET)));
    ev_prop(it, proto, "currentTarget", |_it, this| {
        Ok(field(&this, F_CURRENT))
    });
    ev_prop(it, proto, "srcElement", |_it, this| {
        Ok(field(&this, F_TARGET))
    });
    ev_prop(it, proto, "eventPhase", |_it, this| {
        Ok(field(&this, F_PHASE))
    });
    ev_prop(it, proto, "timeStamp", |_it, this| Ok(field(&this, F_TIME)));
    ev_prop(it, proto, "detail", |_it, this| Ok(field(&this, F_DETAIL)));
    ev_prop(it, proto, "isTrusted", |_it, _this| {
        Ok(Value::Boolean(false))
    });
    ev_prop(it, proto, "composed", |_it, _this| {
        Ok(Value::Boolean(false))
    });
    ev_prop(it, proto, "bubbles", |_it, this| {
        Ok(if field(&this, F_BUBBLES).is_undefined() {
            Value::Boolean(false)
        } else {
            field(&this, F_BUBBLES)
        })
    });
    ev_prop(it, proto, "cancelable", |_it, this| {
        Ok(if field(&this, F_CANCELABLE).is_undefined() {
            Value::Boolean(false)
        } else {
            field(&this, F_CANCELABLE)
        })
    });
    ev_prop(it, proto, "defaultPrevented", |_it, this| {
        Ok(Value::Boolean(flag(&this, F_PREVENTED)))
    });

    // preventDefault / stopPropagation need the interpreter to keep the visible
    // properties (`event.defaultPrevented`) in sync for plain-object events
    let pd = it.native_fn(
        "preventDefault",
        0,
        move |it: &mut Interp, this: Value, _| {
            if field(&this, F_CANCELABLE).to_boolean() {
                set_flag(&this, F_PREVENTED, true);
                let _ = it.set(
                    &this,
                    &Key::Name(Rc::from("defaultPrevented")),
                    Value::Boolean(true),
                );
                let _ = it.set(
                    &this,
                    &Key::Name(Rc::from("returnValue")),
                    Value::Boolean(false),
                );
            }
            Ok(Value::Undefined)
        },
    );
    let sp = it.native_fn("stopPropagation", 0, |_, this: Value, _| {
        set_flag(&this, F_STOPPED, true);
        Ok(Value::Undefined)
    });
    let si = it.native_fn("stopImmediatePropagation", 0, |_, this: Value, _| {
        set_flag(&this, F_STOPPED, true);
        set_flag(&this, F_IMMEDIATE, true);
        Ok(Value::Undefined)
    });
    let init = it.native_fn("initEvent", 2, |_, _this: Value, _| Ok(Value::Undefined));
    {
        let mut b = proto.borrow_mut();
        b.define_internal("preventDefault", pd);
        b.define_internal("stopPropagation", sp);
        b.define_internal("stopImmediatePropagation", si);
        b.define_internal("initEvent", init);
    }
}

/// `on<event>` handler *properties* (used on elements, document and window).
pub fn install_handler_props(it: &mut Interp, host: &HostRef, proto: &ObjRef) {
    for kind in EVENT_HANDLERS {
        let name = format!("on{kind}");
        let hg = host.clone();
        let hs = host.clone();
        let kind_g: String = kind.to_string();
        let kind_s: String = kind.to_string();
        let getter = it.native_fn(&name.clone(), 0, move |_it: &mut Interp, this: Value, _| {
            let Some(target) = evt_target_of(&this) else {
                return Ok(Value::Null);
            };
            let h = match hg.try_borrow() {
                Ok(h) => h,
                Err(_) => return Ok(Value::Null),
            };
            Ok(h.handlers
                .get(&target)
                .and_then(|m| m.get(&kind_g))
                .cloned()
                .unwrap_or(Value::Null))
        });
        let setter = it.native_fn(
            &name.clone(),
            1,
            move |it: &mut Interp, this: Value, a: Vec<Value>| {
                let Some(target) = evt_target_of(&this) else {
                    return Ok(Value::Undefined);
                };
                let v = a.first().cloned().unwrap_or(Value::Undefined);
                let mut h = host_mut(&hs, it)?;
                let map = h.handlers.entry(target).or_default();
                match &v {
                    Value::Null | Value::Undefined => {
                        map.remove(&kind_s);
                    }
                    // `el.onclick = "doStuff()"` — compile the string
                    Value::Str(s) => {
                        let f = handler_from_source(it, s);
                        map.insert(kind_s.clone(), f);
                    }
                    other => {
                        map.insert(kind_s.clone(), other.clone());
                    }
                }
                Ok(Value::Undefined)
            },
        );
        proto.borrow_mut().define(
            &name,
            Prop::accessor(Some(obj_of(getter)), Some(obj_of(setter))),
        );
    }
}

/// The `on*` properties we expose. Missing ones only cost a line here, not a
/// broken page — handlers that are set via `addEventListener` never touch this.
pub const EVENT_HANDLERS: [&str; 68] = [
    "abort",
    "afterprint",
    "animationend",
    "animationiteration",
    "animationstart",
    "beforeprint",
    "blur",
    "cancel",
    "canplay",
    "change",
    "click",
    "contextmenu",
    "copy",
    "cut",
    "dblclick",
    "drag",
    "dragend",
    "dragenter",
    "dragleave",
    "dragover",
    "drop",
    "ended",
    "error",
    "focus",
    "focusin",
    "focusout",
    "input",
    "invalid",
    "keydown",
    "keypress",
    "keyup",
    "load",
    "mousedown",
    "mouseenter",
    "mouseleave",
    "mousemove",
    "mouseout",
    "mouseover",
    "mouseup",
    "paste",
    "pause",
    "play",
    "playing",
    "pointercancel",
    "pointerdown",
    "pointerenter",
    "pointerleave",
    "pointermove",
    "pointerout",
    "pointerover",
    "pointerup",
    "popstate",
    "progress",
    "ratechange",
    "readystatechange",
    "reset",
    "resize",
    "scroll",
    "search",
    "seeked",
    "select",
    "show",
    "storage",
    "submit",
    "toggle",
    "touchend",
    "touchmove",
    "touchstart",
];
