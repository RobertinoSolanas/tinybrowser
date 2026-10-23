//! `window` and its satellites: `location`, `navigator`, `history`, `screen`,
//! storage, media queries, `getComputedStyle`, `fetch`, observers, `performance`.
//!
//! `window` **is** `globalThis` (`tb_js::Interp::globals`): a `var x = 1` in a
//! classic script lands on the window like in a browser, and `this` at top
//! level is the window as well.

use std::cell::RefCell;
use std::rc::Rc;

use tb_js::value::{Key, Obj, ObjRef, Prop, Value};
use tb_js::{Interp, JsResult};

use crate::events;
use crate::host::{
    def_native, merge_accessor, node_of, rw_this_prop, this_method, EvtTarget, Host, HostRef,
    NavRequest,
};
use crate::style_api;

/// Install the window and everything hanging off it.
pub fn install(it: &mut Interp, host: &HostRef) {
    let globals = it.globals.clone();
    let me = Value::Object(globals.clone());
    {
        let mut b = globals.borrow_mut();
        b.define_internal("@@win", Value::Boolean(true));
        b.define_internal("@@class", Value::string("Window"));
        for name in ["window", "self", "top", "parent", "frames", "globalThis"] {
            b.define_internal(name, me.clone());
        }
        b.define_internal("frameElement", Value::Null);
        b.define_internal("opener", Value::Null);
        b.define_internal("closed", Value::Boolean(false));
        b.define_internal("name", Value::string(""));
        b.define_internal("isSecureContext", Value::Boolean(true));
    }
    // `this` in a classic script is the window
    it.env.borrow_mut().this = Some(me.clone());

    // ------------------------------------------------------------- viewport --
    num_get(it, host, &globals, "innerWidth", |h| {
        f64::from(h.ctx.config.viewport.width)
    });
    num_get(it, host, &globals, "innerHeight", |h| {
        f64::from(h.ctx.config.viewport.height)
    });
    num_get(it, host, &globals, "outerWidth", |h| {
        f64::from(h.ctx.config.viewport.width)
    });
    num_get(it, host, &globals, "outerHeight", |h| {
        f64::from(h.ctx.config.viewport.height)
    });
    num_get(it, host, &globals, "devicePixelRatio", |h| {
        f64::from(h.ctx.config.scale)
    });
    num_get(it, host, &globals, "scrollX", |h| h.scroll.0);
    num_get(it, host, &globals, "pageXOffset", |h| h.scroll.0);
    num_get(it, host, &globals, "scrollY", |h| h.scroll.1);
    num_get(it, host, &globals, "pageYOffset", |h| h.scroll.1);
    num_get(it, host, &globals, "length", |_| 0.0);

    // --------------------------------------------------------------- objects --
    let screen = screen_object(it, host);
    globals.borrow_mut().define_internal("screen", screen);
    let navigator = navigator_object(it, host);
    globals.borrow_mut().define_internal("navigator", navigator);
    let history = history_object(it, host);
    globals.borrow_mut().define_internal("history", history);
    let location = location_object(it, host);
    globals
        .borrow_mut()
        .define_internal("@@location", location.clone());
    // `window.location = "…"` navigates; `window.location.href = "…"` is handled
    // by the Location object itself
    let loc_for_get = location.clone();
    let hs = host.clone();
    let setter = it.native_fn(
        "location",
        1,
        move |it: &mut Interp, _this: Value, a: Vec<Value>| {
            let url = a
                .first()
                .map(|v| it.to_string_val(v).unwrap_or_default())
                .unwrap_or_default();
            let mut h = hs
                .try_borrow_mut()
                .map_err(|_| it.type_error("location: DOM is busy"))?;
            let full = resolve_or_keep(&h, &url);
            h.request(NavRequest::Load(full));
            Ok(Value::Undefined)
        },
    );
    let getter = it.native_fn("location", 0, move |_it: &mut Interp, _this: Value, _| {
        let _ = &loc_for_get;
        Ok(location.clone())
    });
    merge_accessor(
        &globals,
        "location",
        Some(tb_js::builtins::obj_of(getter)),
        Some(tb_js::builtins::obj_of(setter)),
    );

    // ---------------------------------------------------------------- scroll -
    for name in ["scrollTo", "scroll", "scrollBy", "moveTo", "moveBy"] {
        let relative = name.ends_with("By");
        this_method(it, host, &globals, name, 2, move |it, h, _this, args| {
            let (x, y) = scroll_args(it, &args)?;
            let (base_x, base_y) = if relative { h.scroll } else { (0.0, 0.0) };
            h.scroll = ((base_x + x).max(0.0), (base_y + y).max(0.0));
            h.request(NavRequest::Scroll(h.scroll.0, h.scroll.1));
            Ok(Value::Undefined)
        });
    }

    // --------------------------------------------------------------- dialogs --
    for (name, answer) in [("alert", None), ("confirm", Some(false)), ("prompt", None)] {
        let h = host.clone();
        let default_answer = answer;
        let f = it.native_fn(
            name,
            1,
            move |it: &mut Interp, _this: Value, a: Vec<Value>| {
                let msg = a
                    .first()
                    .map(|v| it.to_string_val(v).unwrap_or_default())
                    .unwrap_or_default();
                it.log("log", format!("[{name}] {msg}"));
                if let Ok(mut h) = h.try_borrow_mut() {
                    h.log_problem(format!("{name}() was called and answered automatically"));
                }
                Ok(match default_answer {
                    Some(b) => Value::Boolean(b),
                    None => Value::Undefined,
                })
            },
        );
        globals.borrow_mut().define_internal(name, f);
    }

    // --------------------------------------------------------- animation -----
    let raf = it.native_fn(
        "requestAnimationFrame",
        1,
        |it: &mut Interp, _t: Value, a: Vec<Value>| {
            let cb = a.first().cloned().unwrap_or(Value::Undefined);
            Ok(Value::Number(it.schedule(16.0, None, cb, Vec::new()) as f64))
        },
    );
    let caf = it.native_fn(
        "cancelAnimationFrame",
        1,
        |it: &mut Interp, _t: Value, a: Vec<Value>| {
            it.clear_timer(a.first().map(|v| v.to_number()).unwrap_or(0.0));
            Ok(Value::Undefined)
        },
    );
    let ric = it.native_fn(
        "requestIdleCallback",
        1,
        |it: &mut Interp, _t: Value, a: Vec<Value>| {
            let cb = a.first().cloned().unwrap_or(Value::Undefined);
            Ok(Value::Number(it.schedule(1.0, None, cb, Vec::new()) as f64))
        },
    );
    {
        let mut b = globals.borrow_mut();
        b.define_internal("requestAnimationFrame", raf.clone());
        b.define_internal("webkitRequestAnimationFrame", raf);
        b.define_internal("cancelAnimationFrame", caf.clone());
        b.define_internal("webkitCancelAnimationFrame", caf);
        b.define_internal("requestIdleCallback", ric);
    }

    // ------------------------------------------------ media / style / storage -
    install_match_media(it, host, &globals);
    install_computed_style(it, host, &globals);
    install_storage(it, host, &globals);
    install_event_ctors(it, host, &globals);
    install_fetch(it, host, &globals);
    install_extras(it, host, &globals);
    install_base64(it, host, &globals);
    // `document.cookie` is the real thing; `window.cookie` mirrors it
    rw_this_prop(
        it,
        host,
        &globals,
        "cookie",
        |_it, h, _this, _| Ok(Value::string(h.cookie_header())),
        |it, h, _this, v| {
            let raw = it.to_string_val(&v)?;
            h.set_cookie(&raw);
            Ok(())
        },
    );
    events::install_handler_props(it, host, &globals);

    // methods a page calls and a console browser can ignore
    this_method(it, host, &globals, "open", 0, |_it, _h, _this, _args| {
        Ok(Value::Null)
    });
    this_method(it, host, &globals, "close", 0, |_it, _h, _this, _args| {
        Ok(Value::Undefined)
    });
    this_method(it, host, &globals, "focus", 0, |_it, _h, _this, _args| {
        Ok(Value::Undefined)
    });
    this_method(it, host, &globals, "blur", 0, |_it, _h, _this, _args| {
        Ok(Value::Undefined)
    });
    this_method(it, host, &globals, "print", 0, |_it, _h, _this, _args| {
        Ok(Value::Undefined)
    });
    this_method(it, host, &globals, "stop", 0, |_it, _h, _this, _args| {
        Ok(Value::Undefined)
    });
    this_method(
        it,
        host,
        &globals,
        "resizeTo",
        2,
        |_it, _h, _this, _args| Ok(Value::Undefined),
    );
    this_method(
        it,
        host,
        &globals,
        "postMessage",
        1,
        |_it, _h, _this, _args| Ok(Value::Undefined),
    );
    // `window.addEventListener('scroll' | 'resize' | 'hashchange' | …)`
    events::install_event_target(it, host, &globals);

    // Identifiers resolve through the global scope's *variables* first, so every
    // data property `window` gained has to be registered there too — otherwise a
    // bare `navigator`, `alert(…)`, `scrollBy(…)` or `performance.now()` would
    // still hit the engine's own stub instead of ours. Accessors (`scrollY`,
    // `location`, …) stay out: they are live and resolve through the object
    // anyway.
    let bound: Vec<(String, Value)> = globals
        .borrow()
        .props
        .iter()
        .filter_map(|(k, p)| {
            let Key::Name(n) = k else {
                return None;
            };
            if n.starts_with("@@") {
                return None;
            }
            p.value().cloned().map(|v| (n.to_string(), v))
        })
        .collect();
    for (name, value) in bound {
        tb_js::env::set_global(&it.env, &name, value);
    }
}

// ---------------------------------------------------------------------------
// small accessors
// ---------------------------------------------------------------------------

fn borrow_guard<'a>(
    h: &'a HostRef,
    it: &mut Interp,
    what: &str,
) -> Result<std::cell::Ref<'a, Host>, Value> {
    h.try_borrow()
        .map_err(|_| it.type_error(&format!("{what}: DOM is busy")))
}

fn num_get(
    it: &mut Interp,
    host: &HostRef,
    obj: &ObjRef,
    name: &'static str,
    get: fn(&Host) -> f64,
) {
    let h = host.clone();
    let g = it.native_fn(name, 0, move |it: &mut Interp, _this: Value, _| {
        let h = borrow_guard(&h, it, name)?;
        Ok(Value::Number(get(&h)))
    });
    obj.borrow_mut()
        .define(name, Prop::accessor(Some(tb_js::builtins::obj_of(g)), None));
}

fn str_get(
    it: &mut Interp,
    host: &HostRef,
    obj: &ObjRef,
    name: &'static str,
    get: fn(&Host) -> String,
) {
    let h = host.clone();
    let g = it.native_fn(name, 0, move |it: &mut Interp, _this: Value, _| {
        let h = borrow_guard(&h, it, name)?;
        Ok(Value::string(get(&h)))
    });
    obj.borrow_mut()
        .define(name, Prop::accessor(Some(tb_js::builtins::obj_of(g)), None));
}

/// A relative reference becomes absolute against the current document.
pub fn resolve_or_keep(h: &Host, url: &str) -> String {
    h.ctx
        .resolve(url)
        .map(|u| u.to_string())
        .unwrap_or_else(|| url.to_string())
}

// ---------------------------------------------------------------------------
// location
// ---------------------------------------------------------------------------

fn location_object(it: &mut Interp, host: &HostRef) -> Value {
    let proto = host
        .try_borrow()
        .ok()
        .and_then(|h| h.protos.location.clone());
    let mut o = Obj::plain();
    o.proto = proto;
    let obj = Rc::new(RefCell::new(o));
    let v = Value::Object(obj.clone());

    str_get(it, host, &obj, "href", |h| h.url_string());
    str_get(it, host, &obj, "protocol", |h| {
        h.ctx
            .url
            .as_ref()
            .map(|u| format!("{}:", u.scheme))
            .unwrap_or_default()
    });
    str_get(it, host, &obj, "host", |h| {
        h.ctx
            .url
            .as_ref()
            .map(|u| match u.port {
                Some(p) => format!("{}:{}", u.host, p),
                None => u.host.clone(),
            })
            .unwrap_or_default()
    });
    str_get(it, host, &obj, "hostname", |h| {
        h.ctx
            .url
            .as_ref()
            .map(|u| u.host.clone())
            .unwrap_or_default()
    });
    str_get(it, host, &obj, "port", |h| {
        h.ctx
            .url
            .as_ref()
            .and_then(|u| u.port.map(|p| p.to_string()))
            .unwrap_or_default()
    });
    str_get(it, host, &obj, "pathname", |h| {
        h.ctx
            .url
            .as_ref()
            .map(|u| u.path.clone())
            .unwrap_or_default()
    });
    str_get(it, host, &obj, "search", |h| {
        h.ctx
            .url
            .as_ref()
            .and_then(|u| u.query.clone())
            .filter(|q| !q.is_empty())
            .map(|q| format!("?{q}"))
            .unwrap_or_default()
    });
    str_get(it, host, &obj, "hash", |h| {
        h.ctx
            .url
            .as_ref()
            .and_then(|u| u.fragment.clone())
            .map(|f| format!("#{f}"))
            .unwrap_or_default()
    });
    str_get(it, host, &obj, "origin", |h| {
        h.ctx.url.as_ref().map(|u| u.origin()).unwrap_or_default()
    });

    // setters: `href` navigates, the other parts rewrite the URL and navigate
    for name in ["href", "pathname", "search", "hash", "hostname", "protocol"] {
        let h = host.clone();
        let field = name.to_string();
        let s = it.native_fn(
            name,
            1,
            move |it: &mut Interp, _this: Value, a: Vec<Value>| {
                let value = a
                    .first()
                    .map(|v| it.to_string_val(v).unwrap_or_default())
                    .unwrap_or_default();
                let mut h = h
                    .try_borrow_mut()
                    .map_err(|_| it.type_error("location: DOM is busy"))?;
                if field == "href" {
                    let full = resolve_or_keep(&h, &value);
                    h.request(NavRequest::Load(full));
                    return Ok(Value::Undefined);
                }
                let Some(current) = h.ctx.url.clone() else {
                    return Ok(Value::Undefined);
                };
                let mut url = current;
                match field.as_str() {
                    "pathname" => url.path = value,
                    "search" => url.query = Some(value.trim_start_matches('?').to_string()),
                    "hash" => url.fragment = Some(value.trim_start_matches('#').to_string()),
                    "hostname" => url.host = value.split(':').next().unwrap_or("").to_string(),
                    "protocol" => url.scheme = value.trim_end_matches(':').to_ascii_lowercase(),
                    _ => {}
                }
                h.request(NavRequest::Load(url.to_string()));
                Ok(Value::Undefined)
            },
        );
        merge_accessor(&obj, name, None, Some(tb_js::builtins::obj_of(s)));
    }

    let h = host.clone();
    let assign = it.native_fn(
        "assign",
        1,
        move |it: &mut Interp, _this: Value, a: Vec<Value>| {
            let url = a
                .first()
                .map(|v| it.to_string_val(v).unwrap_or_default())
                .unwrap_or_default();
            let mut h = h
                .try_borrow_mut()
                .map_err(|_| it.type_error("location: DOM is busy"))?;
            let full = resolve_or_keep(&h, &url);
            h.request(NavRequest::Load(full));
            Ok(Value::Undefined)
        },
    );
    let h = host.clone();
    let replace = it.native_fn(
        "replace",
        1,
        move |it: &mut Interp, _this: Value, a: Vec<Value>| {
            let url = a
                .first()
                .map(|v| it.to_string_val(v).unwrap_or_default())
                .unwrap_or_default();
            let mut h = h
                .try_borrow_mut()
                .map_err(|_| it.type_error("location: DOM is busy"))?;
            let full = resolve_or_keep(&h, &url);
            h.request(NavRequest::Replace(full));
            Ok(Value::Undefined)
        },
    );
    let h = host.clone();
    let reload = it.native_fn("reload", 0, move |_it: &mut Interp, _this: Value, _| {
        if let Ok(mut h) = h.try_borrow_mut() {
            h.request(NavRequest::Reload);
        }
        Ok(Value::Undefined)
    });
    let h = host.clone();
    let to_string = it.native_fn("toString", 0, move |_it: &mut Interp, _this: Value, _| {
        let h = h
            .try_borrow()
            .map_err(|_| "location: DOM is busy".to_string())?;
        Ok(Value::string(h.url_string()))
    });
    {
        let mut b = obj.borrow_mut();
        b.define_internal("assign", assign);
        b.define_internal("replace", replace);
        b.define_internal("reload", reload);
        b.define_internal("toString", to_string);
    }
    v
}

// ---------------------------------------------------------------------------
// navigator / screen / history
// ---------------------------------------------------------------------------

fn plain_with_proto(_it: &mut Interp, host: &HostRef, which: WhichProto) -> Value {
    let proto = host.try_borrow().ok().and_then(|h| match which {
        WhichProto::Navigator => h.protos.navigator.clone(),
        WhichProto::Screen => h.protos.screen.clone(),
        WhichProto::History => h.protos.history.clone(),
    });
    let mut o = Obj::plain();
    o.proto = proto;
    Value::Object(Rc::new(RefCell::new(o)))
}

enum WhichProto {
    Navigator,
    Screen,
    History,
}

/// `Object.prototype`, for the handful of host objects with no class of their own.
fn fallback_proto(it: &mut Interp) -> ObjRef {
    it.natives
        .object_proto
        .clone()
        .unwrap_or_else(|| Rc::new(RefCell::new(Obj::plain())))
}

fn navigator_object(it: &mut Interp, host: &HostRef) -> Value {
    let obj = plain_with_proto(it, host, WhichProto::Navigator);
    let target = obj.as_object().unwrap().clone();
    target
        .borrow_mut()
        .define_internal("@@class", Value::string("Navigator"));
    str_get(it, host, &target, "userAgent", |h| h.ctx.user_agent.clone());
    str_get(it, host, &target, "appVersion", |h| {
        h.ctx.user_agent.clone()
    });
    str_get(it, host, &target, "platform", |h| h.ctx.platform.clone());
    str_get(it, host, &target, "language", |h| h.ctx.language.clone());
    str_get(it, host, &target, "vendor", |_| String::new());
    str_get(it, host, &target, "product", |_| "Gecko".into());
    str_get(it, host, &target, "productSub", |_| "20030107".into());

    let h = host.clone();
    let langs = it.native_fn("languages", 0, move |it: &mut Interp, _this: Value, _| {
        let h = borrow_guard(&h, it, "navigator")?;
        Ok(it.array(vec![Value::string(h.ctx.language.clone())]))
    });
    let beacon = it.native_fn("sendBeacon", 2, |_it: &mut Interp, _t: Value, _| {
        Ok(Value::Boolean(true))
    });
    let java = it.native_fn("javaEnabled", 0, |_it: &mut Interp, _t: Value, _| {
        Ok(Value::Boolean(false))
    });
    let permissions = it.plain_object();
    def_native(
        it,
        &permissions,
        "query",
        1,
        |it: &mut Interp, _t: Value, _| Ok(it.resolved_promise(&Value::Undefined)),
    );
    let clipboard = it.plain_object();
    def_native(
        it,
        &clipboard,
        "writeText",
        1,
        |it: &mut Interp, _t: Value, a: Vec<Value>| {
            let text = a
                .first()
                .map(|v| it.to_string_val(v).unwrap_or_default())
                .unwrap_or_default();
            it.log("log", format!("[clipboard] {text}"));
            Ok(it.resolved_promise(&Value::Undefined))
        },
    );
    {
        let mut b = target.borrow_mut();
        b.define_internal("languages", langs);
        b.define_internal("sendBeacon", beacon);
        b.define_internal("javaEnabled", java);
        b.define_internal("permissions", permissions);
        b.define_internal("clipboard", clipboard);
        b.define_internal("onLine", Value::Boolean(true));
        b.define_internal("cookieEnabled", Value::Boolean(true));
        b.define_internal("doNotTrack", Value::Null);
        b.define_internal("webdriver", Value::Boolean(true));
        b.define_internal("hardwareConcurrency", Value::Number(4.0));
        b.define_internal("maxTouchPoints", Value::Number(0.0));
    }
    obj
}

fn screen_object(it: &mut Interp, host: &HostRef) -> Value {
    let obj = plain_with_proto(it, host, WhichProto::Screen);
    let target = obj.as_object().unwrap().clone();
    target
        .borrow_mut()
        .define_internal("@@class", Value::string("Screen"));
    num_get(it, host, &target, "width", |h| {
        f64::from(h.ctx.config.viewport.width)
    });
    num_get(it, host, &target, "height", |h| {
        f64::from(h.ctx.config.viewport.height)
    });
    num_get(it, host, &target, "availWidth", |h| {
        f64::from(h.ctx.config.viewport.width)
    });
    num_get(it, host, &target, "availHeight", |h| {
        f64::from(h.ctx.config.viewport.height)
    });
    let orientation = it.plain_object();
    let _ = it.set(
        &orientation,
        &Key::Name(Rc::from("type")),
        Value::string("landscape-primary"),
    );
    let _ = it.set(
        &orientation,
        &Key::Name(Rc::from("angle")),
        Value::Number(0.0),
    );
    def_native(
        it,
        &orientation,
        "addEventListener",
        2,
        |_it: &mut Interp, _t: Value, _| Ok(Value::Undefined),
    );
    {
        let mut b = target.borrow_mut();
        b.define_internal("orientation", orientation);
        b.define_internal("colorDepth", Value::Number(24.0));
        b.define_internal("pixelDepth", Value::Number(24.0));
    }
    obj
}

fn history_object(it: &mut Interp, host: &HostRef) -> Value {
    let obj = plain_with_proto(it, host, WhichProto::History);
    let target = obj.as_object().unwrap().clone();
    target
        .borrow_mut()
        .define_internal("@@class", Value::string("History"));

    let h = host.clone();
    let length = it.native_fn("length", 0, move |it: &mut Interp, _this: Value, _| {
        let h = borrow_guard(&h, it, "history")?;
        Ok(Value::Number(h.history.len().max(1) as f64))
    });
    target.borrow_mut().define(
        "length",
        Prop::accessor(Some(tb_js::builtins::obj_of(length)), None),
    );
    let h = host.clone();
    let state = it.native_fn("state", 0, move |it: &mut Interp, _this: Value, _| {
        let h = borrow_guard(&h, it, "history")?;
        Ok(h.states.get(h.history_pos).cloned().unwrap_or(Value::Null))
    });
    target.borrow_mut().define(
        "state",
        Prop::accessor(Some(tb_js::builtins::obj_of(state)), None),
    );

    this_method(it, host, &target, "pushState", 2, |it, h, _this, args| {
        let data = args.first().cloned().unwrap_or(Value::Null);
        let url = args
            .get(2)
            .filter(|v| !v.is_nullish())
            .map(|v| it.to_string_val(v).unwrap_or_default());
        h.states.truncate(h.history_pos + 1);
        h.states.push(data);
        h.history.truncate(h.history_pos + 1);
        h.history.push(match url {
            Some(u) => resolve_or_keep(h, &u),
            None => h.url_string(),
        });
        h.history_pos = h.history.len() - 1;
        Ok(Value::Undefined)
    });
    this_method(
        it,
        host,
        &target,
        "replaceState",
        2,
        |it, h, _this, args| {
            let data = args.first().cloned().unwrap_or(Value::Null);
            let url = args
                .get(2)
                .filter(|v| !v.is_nullish())
                .map(|v| it.to_string_val(v).unwrap_or_default());
            while h.history_pos >= h.states.len() {
                h.states.push(Value::Null);
            }
            h.states[h.history_pos] = data;
            if let Some(u) = url {
                let full = resolve_or_keep(h, &u);
                if let Some(slot) = h.history.get_mut(h.history_pos) {
                    *slot = full;
                }
            }
            Ok(Value::Undefined)
        },
    );
    this_method(it, host, &target, "back", 0, |_it, h, _this, _args| {
        h.request(NavRequest::Back(-1));
        Ok(Value::Undefined)
    });
    this_method(it, host, &target, "forward", 0, |_it, h, _this, _args| {
        h.request(NavRequest::Back(1));
        Ok(Value::Undefined)
    });
    this_method(it, host, &target, "go", 1, |_it, h, _this, args| {
        let delta = args.first().map(|v| v.to_integer() as i32).unwrap_or(0);
        h.request(NavRequest::Back(delta));
        Ok(Value::Undefined)
    });
    target
        .borrow_mut()
        .define_internal("scrollRestoration", Value::string("auto"));
    Value::Object(target)
}

// ---------------------------------------------------------------------------
// matchMedia, getComputedStyle, storage, constructors, fetch, extras
// ---------------------------------------------------------------------------

fn install_match_media(it: &mut Interp, host: &HostRef, globals: &ObjRef) {
    let h = host.clone();
    let match_media = it.native_fn(
        "matchMedia",
        1,
        move |it: &mut Interp, _t: Value, a: Vec<Value>| {
            let query = a
                .first()
                .map(|v| it.to_string_val(v).unwrap_or_default())
                .unwrap_or_default();
            let matches = {
                let h = borrow_guard(&h, it, "matchMedia")?;
                let vp = h.ctx.viewport();
                tb_css::media::parse_media_list(&query)
                    .iter()
                    .any(|q| q.matches(&vp))
            };
            let out = it.plain_object();
            let _ = it.set(&out, &Key::Name(Rc::from("media")), Value::string(query));
            let _ = it.set(
                &out,
                &Key::Name(Rc::from("matches")),
                Value::Boolean(matches),
            );
            let _ = it.set(&out, &Key::Name(Rc::from("onchange")), Value::Null);
            for m in [
                "addEventListener",
                "removeEventListener",
                "addListener",
                "removeListener",
            ] {
                def_native(it, &out, m, 2, |_it: &mut Interp, _t: Value, _| {
                    Ok(Value::Undefined)
                });
            }
            Ok(out)
        },
    );
    globals
        .borrow_mut()
        .define_internal("matchMedia", match_media);

    let css = it.plain_object();
    def_native(it, &css, "supports", 2, |_it: &mut Interp, _t: Value, _| {
        // answering "yes" keeps feature-detection code on the happy path
        Ok(Value::Boolean(true))
    });
    globals.borrow_mut().define_internal("CSS", css);
}

fn install_computed_style(it: &mut Interp, host: &HostRef, globals: &ObjRef) {
    let h = host.clone();
    let computed = it.native_fn(
        "getComputedStyle",
        1,
        move |it: &mut Interp, _t: Value, a: Vec<Value>| {
            let Some(id) = a.first().and_then(node_of) else {
                return Err(it.type_error("getComputedStyle: argument is not an element"));
            };
            let mut h = h
                .try_borrow_mut()
                .map_err(|_| it.type_error("getComputedStyle: DOM is busy"))?;
            if !h.dom.exists(id) {
                return Err(it.type_error("getComputedStyle: detached node"));
            }
            Ok(style_api::computed_object(it, &mut h, id))
        },
    );
    globals
        .borrow_mut()
        .define_internal("getComputedStyle", computed);
}

#[derive(Clone, Copy)]
enum Store {
    Local,
    Session,
}

impl Store {
    fn label(self) -> &'static str {
        match self {
            Store::Local => "localStorage",
            Store::Session => "sessionStorage",
        }
    }
    fn idx(self) -> usize {
        match self {
            Store::Local => 0,
            Store::Session => 1,
        }
    }
    fn of(self, h: &Host) -> &std::collections::HashMap<String, String> {
        match self {
            Store::Local => &h.local,
            Store::Session => &h.session,
        }
    }
    fn of_mut(self, h: &mut Host) -> &mut std::collections::HashMap<String, String> {
        match self {
            Store::Local => &mut h.local,
            Store::Session => &mut h.session,
        }
    }
}

fn install_storage(it: &mut Interp, host: &HostRef, globals: &ObjRef) {
    for which in [Store::Local, Store::Session] {
        let label = which.label();
        let idx = which.idx();
        // One prototype *per storage*: the methods below close over `which`,
        // and sharing a prototype would make localStorage act like sessionStorage.
        let proto = Rc::new(RefCell::new(Obj::with_proto(Some(fallback_proto(it)))));
        proto
            .borrow_mut()
            .define_internal("@@class", Value::string("Storage"));

        // Methods on the *prototype*: the mirror object's own properties are
        // then exactly the stored keys, like in a browser.
        let h = host.clone();
        def_native(
            it,
            &Value::Object(proto.clone()),
            "getItem",
            1,
            move |it: &mut Interp, _t: Value, a: Vec<Value>| {
                let key = a
                    .first()
                    .map(|v| it.to_string_val(v).unwrap_or_default())
                    .unwrap_or_default();
                let mut h = h
                    .try_borrow_mut()
                    .map_err(|_| it.type_error(&format!("{label}: DOM is busy")))?;
                h.sync_storage(idx);
                Ok(match which.of(&h).get(&key) {
                    Some(v) => Value::string(v),
                    None => Value::Null,
                })
            },
        );
        let h = host.clone();
        def_native(
            it,
            &Value::Object(proto.clone()),
            "setItem",
            2,
            move |it: &mut Interp, _t: Value, a: Vec<Value>| {
                let key = a
                    .first()
                    .map(|v| it.to_string_val(v).unwrap_or_default())
                    .unwrap_or_default();
                let value = a
                    .get(1)
                    .map(|v| it.to_string_val(v).unwrap_or_default())
                    .unwrap_or_default();
                let mut h = h
                    .try_borrow_mut()
                    .map_err(|_| it.type_error(&format!("{label}: DOM is busy")))?;
                h.sync_storage(idx);
                which.of_mut(&mut h).insert(key, value);
                h.sync_storage(idx);
                Ok(Value::Undefined)
            },
        );
        let h = host.clone();
        def_native(
            it,
            &Value::Object(proto.clone()),
            "removeItem",
            1,
            move |it: &mut Interp, _t: Value, a: Vec<Value>| {
                let key = a
                    .first()
                    .map(|v| it.to_string_val(v).unwrap_or_default())
                    .unwrap_or_default();
                let mut h = h
                    .try_borrow_mut()
                    .map_err(|_| it.type_error(&format!("{label}: DOM is busy")))?;
                h.sync_storage(idx);
                which.of_mut(&mut h).remove(&key);
                h.sync_storage(idx);
                Ok(Value::Undefined)
            },
        );
        let h = host.clone();
        def_native(
            it,
            &Value::Object(proto.clone()),
            "clear",
            0,
            move |_it: &mut Interp, _t: Value, _| {
                if let Ok(mut h) = h.try_borrow_mut() {
                    h.sync_storage(idx);
                    which.of_mut(&mut h).clear();
                    h.sync_storage(idx);
                }
                Ok(Value::Undefined)
            },
        );
        let h = host.clone();
        def_native(
            it,
            &Value::Object(proto.clone()),
            "key",
            1,
            move |it: &mut Interp, _t: Value, a: Vec<Value>| {
                let i = a.first().map(|v| v.to_integer()).unwrap_or(-1.0);
                let mut h = h
                    .try_borrow_mut()
                    .map_err(|_| it.type_error(&format!("{label}: DOM is busy")))?;
                h.sync_storage(idx);
                let mut keys: Vec<String> = which.of(&h).keys().cloned().collect();
                keys.sort();
                if i < 0.0 || (i as usize) >= keys.len() {
                    return Ok(Value::Null);
                }
                Ok(Value::string(keys[i as usize].clone()))
            },
        );
        let h = host.clone();
        let len = it.native_fn("length", 0, move |it: &mut Interp, _t: Value, _| {
            let mut h = h
                .try_borrow_mut()
                .map_err(|_| it.type_error(&format!("{label}: DOM is busy")))?;
            h.sync_storage(idx);
            Ok(Value::Number(which.of(&h).len() as f64))
        });
        proto.borrow_mut().define(
            "length",
            Prop::accessor(Some(tb_js::builtins::obj_of(len)), None),
        );

        // one mirror object, handed out by a getter that keeps it in step
        let mut mirror = Obj::with_proto(Some(proto.clone()));
        mirror.define_internal("@@class", Value::string("Storage"));
        let mirror = Value::Object(Rc::new(RefCell::new(mirror)));
        if let Ok(mut h) = host.try_borrow_mut() {
            h.storage_mirrors[idx] = Some(mirror.as_object().unwrap().clone());
            h.sync_storage(idx);
        }
        let h = host.clone();
        let getter = it.native_fn(label, 0, move |it: &mut Interp, _t: Value, _| {
            let mut h = h
                .try_borrow_mut()
                .map_err(|_| it.type_error(&format!("{label}: DOM is busy")))?;
            h.sync_storage(idx);
            Ok(h.storage_mirrors[idx]
                .clone()
                .map(Value::Object)
                .unwrap_or(Value::Undefined))
        });
        merge_accessor(globals, label, Some(tb_js::builtins::obj_of(getter)), None);
    }
}

fn install_event_ctors(it: &mut Interp, host: &HostRef, globals: &ObjRef) {
    let event_proto = host
        .try_borrow()
        .ok()
        .and_then(|h| h.protos.event.clone())
        .unwrap_or_else(|| fallback_proto(it));
    for name in [
        "Event",
        "CustomEvent",
        "MouseEvent",
        "KeyboardEvent",
        "PointerEvent",
        "UIEvent",
        "FocusEvent",
        "ProgressEvent",
        "StorageEvent",
        "HashChangeEvent",
        "PopStateEvent",
        "InputEvent",
        "WheelEvent",
        "Touch",
    ] {
        let proto = event_proto.clone();
        let instance = event_proto.clone();
        let c = crate::host::ctor(
            it,
            name,
            &proto,
            move |it: &mut Interp, _this: Value, a: Vec<Value>| {
                let kind = a
                    .first()
                    .map(|v| it.to_string_val(v).unwrap_or_default())
                    .unwrap_or_default();
                Ok(events::make_event(
                    it,
                    &kind,
                    a.get(1).cloned(),
                    Some(instance.clone()),
                ))
            },
        );
        globals.borrow_mut().define_internal(name, c);
    }

    // `new Image()` / `new Audio()` create the element itself
    for (name, tag) in [("Image", "img"), ("Audio", "audio"), ("Video", "video")] {
        let h = host.clone();
        let t = tag;
        let proto = host
            .try_borrow()
            .ok()
            .and_then(|h| h.protos.html_element.clone())
            .unwrap_or_else(|| fallback_proto(it));
        let c = crate::host::ctor(
            it,
            name,
            &proto,
            move |it: &mut Interp, _this: Value, a: Vec<Value>| {
                let _ = it;
                let mut h = h
                    .try_borrow_mut()
                    .map_err(|_| format!("{name}: DOM is busy"))?;
                let el = h.dom.create_element(t);
                // `new Image(60, 40)` sizes it, `new Audio(src)` loads it
                if let Some(w) = a.first().filter(|v| v.is_number()) {
                    h.dom
                        .set_attr(el, "width", crate::host::num_string(w.to_number()));
                }
                if let Some(hh) = a.get(1).filter(|v| v.is_number()) {
                    h.dom
                        .set_attr(el, "height", crate::host::num_string(hh.to_number()));
                }
                if let Some(src) = a.first().filter(|v| v.is_string()) {
                    let src = src.to_string();
                    let full = resolve_or_keep(&h, &src);
                    h.dom.set_attr(el, "src", full);
                }
                h.touch();
                Ok(h.wrap(el))
            },
        );
        globals.borrow_mut().define_internal(name, c);
    }

    // observers: accepted, never fire (documented limitation)
    for name in [
        "MutationObserver",
        "IntersectionObserver",
        "ResizeObserver",
        "PerformanceObserver",
    ] {
        let proto = fallback_proto(it);
        let c = crate::host::ctor(
            it,
            name,
            &proto,
            move |it: &mut Interp, _t: Value, _a: Vec<Value>| {
                let o = it.plain_object();
                for m in ["observe", "disconnect", "unobserve", "takeRecords"] {
                    let records = m == "takeRecords";
                    def_native(it, &o, m, 1, move |it: &mut Interp, _t: Value, _| {
                        Ok(if records {
                            it.array(Vec::new())
                        } else {
                            Value::Undefined
                        })
                    });
                }
                Ok(o)
            },
        );
        globals.borrow_mut().define_internal(name, c);
    }

    // `new XMLHttpRequest()` — no network here (documented in NOTES), but the
    // object exists so a page's feature detection does not fall over
    let proto = fallback_proto(it);
    let xhr = crate::host::ctor(
        it,
        "XMLHttpRequest",
        &proto,
        |it: &mut Interp, _t: Value, _a: Vec<Value>| {
            let o = it.plain_object();
            for m in [
                "open",
                "send",
                "setRequestHeader",
                "abort",
                "getAllResponseHeaders",
            ] {
                let is_text = m == "getAllResponseHeaders";
                def_native(it, &o, m, 2, move |_it: &mut Interp, _t: Value, _| {
                    Ok(if is_text {
                        Value::string("")
                    } else {
                        Value::Undefined
                    })
                });
            }
            for (k, v) in [
                ("readyState", Value::Number(0.0)),
                ("status", Value::Number(0.0)),
                ("responseText", Value::string("")),
                ("response", Value::string("")),
                ("onload", Value::Null),
                ("onerror", Value::Null),
                ("withCredentials", Value::Boolean(false)),
            ] {
                let _ = it.set(&o, &Key::Name(Rc::from(k)), v);
            }
            Ok(o)
        },
    );
    globals.borrow_mut().define_internal("XMLHttpRequest", xhr);

    // Node constants
    let node_ctor = it.plain_object();
    for (k, v) in [
        ("ELEMENT_NODE", 1.0),
        ("TEXT_NODE", 3.0),
        ("COMMENT_NODE", 8.0),
        ("DOCUMENT_NODE", 9.0),
        ("DOCUMENT_TYPE_NODE", 10.0),
        ("DOCUMENT_FRAGMENT_NODE", 11.0),
    ] {
        let _ = it.set(&node_ctor, &Key::Name(Rc::from(k)), Value::Number(v));
    }
    globals.borrow_mut().define_internal("Node", node_ctor);
}

fn install_fetch(it: &mut Interp, host: &HostRef, globals: &ObjRef) {
    let h = host.clone();
    let fetch = it.native_fn(
        "fetch",
        1,
        move |it: &mut Interp, _t: Value, a: Vec<Value>| {
            let url = a
                .first()
                .map(|v| it.to_string_val(v).unwrap_or_default())
                .unwrap_or_default();
            let (hook, full) = {
                let h = borrow_guard(&h, it, "fetch")?;
                (h.fetch_hook.clone(), crate::html::resolve_display(&h, &url))
            };
            let Some(hook) = hook else {
                if let Ok(mut h) = h.try_borrow_mut() {
                    h.log_problem(format!("fetch() without a network hook: {full}"));
                }
                let err = it.error_with("TypeError", &format!("fetch blocked: {full}"));
                return Ok(it.rejected_promise(&err));
            };
            match hook(&full) {
                Some((status, body)) => {
                    let response = it.plain_object();
                    let _ = it.set(
                        &response,
                        &Key::Name(Rc::from("ok")),
                        Value::Boolean((200..300).contains(&status)),
                    );
                    let _ = it.set(
                        &response,
                        &Key::Name(Rc::from("status")),
                        Value::Number(status as f64),
                    );
                    let _ = it.set(
                        &response,
                        &Key::Name(Rc::from("statusText")),
                        Value::string(""),
                    );
                    let _ = it.set(&response, &Key::Name(Rc::from("url")), Value::string(&full));
                    let _ = it.set(
                        &response,
                        &Key::Name(Rc::from("redirected")),
                        Value::Boolean(false),
                    );
                    let _ = it.set(
                        &response,
                        &Key::Name(Rc::from("type")),
                        Value::string("basic"),
                    );
                    let text_body = body.clone();
                    def_native(
                        it,
                        &response,
                        "text",
                        0,
                        move |it: &mut Interp, _t: Value, _| {
                            Ok(it.resolved_promise(&Value::string(&text_body)))
                        },
                    );
                    let blob_body = body.clone();
                    def_native(
                        it,
                        &response,
                        "blob",
                        0,
                        move |it: &mut Interp, _t: Value, _| {
                            Ok(it.resolved_promise(&Value::string(&blob_body)))
                        },
                    );
                    let json_body = body;
                    def_native(
                        it,
                        &response,
                        "json",
                        0,
                        move |it: &mut Interp, _t: Value, _| match tb_js::json::parse(&json_body) {
                            Ok(v) => Ok(it.resolved_promise(&v)),
                            Err(e) => {
                                let err = it.error_with("SyntaxError", &e);
                                Ok(it.rejected_promise(&err))
                            }
                        },
                    );
                    let headers = it.plain_object();
                    def_native(it, &headers, "get", 1, |_it: &mut Interp, _t: Value, _| {
                        Ok(Value::Null)
                    });
                    def_native(it, &headers, "has", 1, |_it: &mut Interp, _t: Value, _| {
                        Ok(Value::Boolean(false))
                    });
                    let _ = it.set(&response, &Key::Name(Rc::from("headers")), headers);
                    Ok(it.resolved_promise(&response))
                }
                None => {
                    let msg = format!("fetch failed: {full}");
                    if let Ok(mut h) = h.try_borrow_mut() {
                        h.log_problem(msg.clone());
                    }
                    let err = it.error_with("TypeError", &msg);
                    Ok(it.rejected_promise(&err))
                }
            }
        },
    );
    globals.borrow_mut().define_internal("fetch", fetch);
}

fn install_base64(it: &mut Interp, _host: &HostRef, globals: &ObjRef) {
    let atob = it.native_fn(
        "atob",
        1,
        move |it: &mut Interp, _t: Value, a: Vec<Value>| {
            let input = a
                .first()
                .map(|v| it.to_string_val(v).unwrap_or_default())
                .unwrap_or_default();
            match tb_net::base64_decode(&input) {
                Ok(bytes) => Ok(Value::string(String::from_utf8_lossy(&bytes))),
                Err(_) => Ok(Value::string("")),
            }
        },
    );
    let btoa = it.native_fn(
        "btoa",
        1,
        move |it: &mut Interp, _t: Value, a: Vec<Value>| {
            let input = a
                .first()
                .map(|v| it.to_string_val(v).unwrap_or_default())
                .unwrap_or_default();
            Ok(Value::string(tb_net::base64_encode(input.as_bytes())))
        },
    );
    let mut b = globals.borrow_mut();
    b.define_internal("atob", atob);
    b.define_internal("btoa", btoa);
}

fn install_extras(it: &mut Interp, host: &HostRef, globals: &ObjRef) {
    // performance.now() runs on the interpreter's virtual clock — deterministic
    let performance = it.plain_object();
    let now = it.native_fn("now", 0, move |it: &mut Interp, _t: Value, _| {
        Ok(Value::Number(it.clock))
    });
    let entries = it.native_fn(
        "getEntriesByType",
        1,
        move |it: &mut Interp, _t: Value, _| Ok(it.array(Vec::new())),
    );
    {
        let target = performance.as_object().unwrap().clone();
        let mut b = target.borrow_mut();
        b.define_internal("now", now);
        b.define_internal("timeOrigin", Value::Number(0.0));
        b.define_internal("getEntriesByType", entries);
        b.define_internal("timing", Value::Object(Rc::new(RefCell::new(Obj::plain()))));
        b.define_internal(
            "navigation",
            Value::Object(Rc::new(RefCell::new(Obj::plain()))),
        );
    }
    for m in ["mark", "measure", "clearMarks", "clearMeasures"] {
        def_native(it, &performance, m, 1, |_it: &mut Interp, _t: Value, _| {
            Ok(Value::Undefined)
        });
    }
    globals
        .borrow_mut()
        .define_internal("performance", performance);

    // crypto: deterministic on purpose — a random seed is a test flake
    let crypto = it.plain_object();
    let uuid = it.native_fn("randomUUID", 0, move |it: &mut Interp, _t: Value, _| {
        let mut s = 0x9E37_79B9_7F4A_7C15u64 ^ it.steps;
        let mut bytes = [0u8; 16];
        for b in bytes.iter_mut() {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            *b = (s >> 24) as u8;
        }
        bytes[6] = (bytes[6] & 0x0f) | 0x40;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        Ok(Value::string(format!(
            "{}-{}-{}-{}-{}",
            &hex[0..8],
            &hex[8..12],
            &hex[12..16],
            &hex[16..20],
            &hex[20..32]
        )))
    });
    let get_random = it.native_fn(
        "getRandomValues",
        1,
        |_it: &mut Interp, _t: Value, a: Vec<Value>| {
            Ok(a.first().cloned().unwrap_or(Value::Undefined))
        },
    );
    {
        let target = crypto.as_object().unwrap().clone();
        let mut b = target.borrow_mut();
        b.define_internal("randomUUID", uuid);
        b.define_internal("getRandomValues", get_random);
    }
    globals.borrow_mut().define_internal("crypto", crypto);

    // DOMParser: gives back an element whose children are the parsed fragment
    let parser_proto = fallback_proto(it);
    let parser = crate::host::ctor(it, "DOMParser", &parser_proto, {
        let h = host.clone();
        move |it: &mut Interp, _t: Value, _| {
            let o = it.plain_object();
            let hp = h.clone();
            let parse = it.native_fn(
                "parseFromString",
                2,
                move |_it: &mut Interp, _t: Value, a: Vec<Value>| {
                    let markup = a.first().map(|v| v.to_string()).unwrap_or_default();
                    let mut h = hp
                        .try_borrow_mut()
                        .map_err(|_| "DOMParser: DOM is busy".to_string())?;
                    let root = h.dom.create_element("div");
                    tb_html::parse_fragment(&mut h.dom, root, &markup);
                    h.touch();
                    Ok(h.wrap(root))
                },
            );
            let target = o.as_object().unwrap().clone();
            target
                .borrow_mut()
                .define_internal("parseFromString", parse);
            Ok(o)
        }
    });
    globals.borrow_mut().define_internal("DOMParser", parser);

    // visualViewport: scripts read it for layout tricks
    let vv = it.plain_object();
    let vv_obj = vv.as_object().unwrap().clone();
    num_get(it, host, &vv_obj, "width", |h| {
        f64::from(h.ctx.config.viewport.width)
    });
    num_get(it, host, &vv_obj, "height", |h| {
        f64::from(h.ctx.config.viewport.height)
    });
    num_get(it, host, &vv_obj, "scale", |_| 1.0);
    num_get(it, host, &vv_obj, "offsetTop", |h| h.scroll.1);
    num_get(it, host, &vv_obj, "offsetLeft", |h| h.scroll.0);
    globals.borrow_mut().define_internal("visualViewport", vv);

    let _ = EvtTarget::Window;
}

/// `scrollTo(x, y)` and `scrollTo({left, top})`.
fn scroll_args(it: &mut Interp, args: &[Value]) -> JsResult<(f64, f64)> {
    match args.first() {
        Some(v) if v.is_object() => {
            let x = it
                .get(v, &Key::Name(Rc::from("left")))
                .map(|v| v.to_number())
                .unwrap_or(0.0);
            let y = it
                .get(v, &Key::Name(Rc::from("top")))
                .map(|v| v.to_number())
                .unwrap_or(0.0);
            Ok((x, y))
        }
        _ => {
            let x = args.first().map(|v| v.to_number()).unwrap_or(0.0);
            let y = args.get(1).map(|v| v.to_number()).unwrap_or(0.0);
            Ok((x, y))
        }
    }
}
