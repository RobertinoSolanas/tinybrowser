//! `HTMLElement.prototype` — the tag specific surface: reflected attributes,
//! form values, links, `click()`/`focus()`.
//!
//! Everything is attribute backed and lives on the *one* HTML element
//! prototype: `input.value` on a `<div>` answers `""` instead of `undefined`.
//! Pages do not test for that; they test the tag.

use tb_dom::NodeId;
use tb_js::value::{ObjRef, Value};
use tb_js::{Interp, JsResult};

use crate::events;
use crate::host::{method, method_unlocked, node_list, ro_prop, rw_prop, EvtTarget, Host, HostRef};

/// `el.<prop>` ⇄ `el.getAttribute("<attr>")` with a default.
pub fn reflect_str(
    it: &mut Interp,
    host: &HostRef,
    proto: &ObjRef,
    prop: &str,
    attr: &str,
    default: &str,
) {
    let d = default.to_string();
    let attr_get = attr.to_string();
    let attr_set = attr.to_string();
    rw_prop(
        it,
        host,
        proto,
        prop,
        move |it, h, id, _| {
            let _ = it;
            Ok(Value::string(
                h.dom.attr(id, &attr_get).unwrap_or_else(|| d.clone()),
            ))
        },
        move |it, h, id, v| {
            let s = it.to_string_val(&v)?;
            h.dom.set_attr(id, &attr_set, s);
            h.touch();
            Ok(())
        },
    );
}

/// Presence-only attributes (`disabled`, `checked`, `hidden`, …).
pub fn reflect_bool(it: &mut Interp, host: &HostRef, proto: &ObjRef, prop: &str, attr: &str) {
    let attr_get = attr.to_string();
    let attr_set = attr.to_string();
    rw_prop(
        it,
        host,
        proto,
        prop,
        move |it, h, id, _| {
            let _ = it;
            Ok(Value::Boolean(h.dom.has_attr(id, &attr_get)))
        },
        move |_it, h, id, v| {
            if v.to_boolean() {
                h.dom.set_attr(id, &attr_set, "");
            } else {
                h.dom.remove_attr(id, &attr_set);
            }
            h.touch();
            Ok(())
        },
    );
}

/// Numeric attributes (`width`, `rows`, `tabIndex`, …).
pub fn reflect_num(
    it: &mut Interp,
    host: &HostRef,
    proto: &ObjRef,
    prop: &str,
    attr: &str,
    default: f64,
) {
    let attr_get = attr.to_string();
    let attr_set = attr.to_string();
    rw_prop(
        it,
        host,
        proto,
        prop,
        move |it, h, id, _| {
            let _ = it;
            Ok(Value::Number(
                h.dom
                    .attr(id, &attr_get)
                    .and_then(|v| parse_number(&v))
                    .unwrap_or(default),
            ))
        },
        move |_it, h, id, v| {
            let n = v.to_number();
            h.dom.set_attr(id, &attr_set, crate::host::num_string(n));
            h.touch();
            Ok(())
        },
    );
}

fn parse_number(s: &str) -> Option<f64> {
    let t = s.trim().trim_end_matches("px").trim();
    t.parse::<f64>().ok()
}

/// `<a href>` / `<img src>`: the *resolved* URL, like in a browser.
pub fn url_reflect(it: &mut Interp, host: &HostRef, proto: &ObjRef, prop: &str, attr: &str) {
    let attr_get = attr.to_string();
    let attr_set = attr.to_string();
    rw_prop(
        it,
        host,
        proto,
        prop,
        move |it, h, id, _| {
            let _ = it;
            let raw = h.dom.attr(id, &attr_get).unwrap_or_default();
            Ok(Value::string(resolve_display(h, &raw)))
        },
        move |it, h, id, v| {
            let s = it.to_string_val(&v)?;
            h.dom.set_attr(id, &attr_set, s);
            h.touch();
            Ok(())
        },
    );
}

/// An attribute URL as the page sees it (absolute whenever we have a base).
pub fn resolve_display(h: &Host, raw: &str) -> String {
    if raw.is_empty() {
        return String::new();
    }
    h.ctx
        .resolve(raw)
        .map(|u| u.to_string())
        .unwrap_or_else(|| raw.to_string())
}

/// `<a>.protocol/host/pathname/…` — derived from the resolved `href`.
fn anchor_part(it: &mut Interp, host: &HostRef, proto: &ObjRef, prop: &str) {
    let p_get = prop.to_string();
    let p_set = prop.to_string();
    rw_prop(
        it,
        host,
        proto,
        prop,
        move |it, h, id, _| {
            let _ = it;
            let raw = h.dom.attr(id, "href").unwrap_or_default();
            let Some(url) = h
                .ctx
                .resolve(&raw)
                .or_else(|| tb_net::Url::parse(&raw).ok())
            else {
                return Ok(Value::string(""));
            };
            Ok(Value::string(match p_get.as_str() {
                "protocol" => format!("{}:", url.scheme),
                "host" => match url.port {
                    Some(p) => format!("{}:{}", url.host, p),
                    None => url.host.clone(),
                },
                "hostname" => url.host.clone(),
                "port" => url.port.map(|p| p.to_string()).unwrap_or_default(),
                "pathname" => url.path.clone(),
                "search" => match &url.query {
                    Some(q) if !q.is_empty() => format!("?{q}"),
                    _ => String::new(),
                },
                "hash" => match &url.fragment {
                    Some(f) => format!("#{f}"),
                    _ => String::new(),
                },
                "origin" => url.origin(),
                _ => String::new(),
            }))
        },
        move |it, h, id, v| {
            // write the part back into the href attribute
            let raw = h.dom.attr(id, "href").unwrap_or_default();
            let mut url = h
                .ctx
                .resolve(&raw)
                .or_else(|| tb_net::Url::parse(&raw).ok())
                .unwrap_or_else(|| tb_net::Url::parse("about:blank").unwrap());
            let s = it.to_string_val(&v)?;
            match p_set.as_str() {
                "protocol" => url.scheme = s.trim_end_matches(':').to_ascii_lowercase(),
                "hostname" | "host" => url.host = s.split(':').next().unwrap_or("").to_string(),
                "pathname" => url.path = s,
                "search" => url.query = Some(s.trim_start_matches('?').to_string()),
                "hash" => url.fragment = Some(s.trim_start_matches('#').to_string()),
                _ => {}
            }
            h.dom.set_attr(id, "href", url.to_string());
            h.touch();
            Ok(())
        },
    );
}

pub fn install(it: &mut Interp, host: &HostRef, proto: &ObjRef) {
    // ------------------------------------------------------- generic HTML ----
    reflect_str(it, host, proto, "title", "title", "");
    reflect_str(it, host, proto, "lang", "lang", "");
    reflect_str(it, host, proto, "dir", "dir", "");
    reflect_str(it, host, proto, "accessKey", "accesskey", "");
    reflect_str(it, host, proto, "role", "role", "");
    reflect_str(it, host, proto, "slot", "slot", "");
    reflect_bool(it, host, proto, "hidden", "hidden");
    reflect_bool(it, host, proto, "draggable", "draggable");
    reflect_bool(it, host, proto, "spellcheck", "spellcheck");
    reflect_num(it, host, proto, "tabIndex", "tabindex", -1.0);
    rw_prop(
        it,
        host,
        proto,
        "contentEditable",
        |_it, h, id, _| {
            Ok(Value::string(
                h.dom
                    .attr(id, "contenteditable")
                    .unwrap_or_else(|| "inherit".into()),
            ))
        },
        |it, h, id, v| {
            let s = it.to_string_val(&v)?;
            h.dom.set_attr(id, "contenteditable", s);
            h.touch();
            Ok(())
        },
    );
    ro_prop(it, host, proto, "isContentEditable", |_it, h, id| {
        Ok(Value::Boolean(
            h.dom
                .attr(id, "contenteditable")
                .map(|v| v == "true" || v.is_empty())
                .unwrap_or(false),
        ))
    });

    // --------------------------------------------------- links and sources ---
    url_reflect(it, host, proto, "href", "href");
    url_reflect(it, host, proto, "src", "src");
    for part in [
        "protocol", "host", "hostname", "port", "pathname", "search", "hash", "origin",
    ] {
        anchor_part(it, host, proto, part);
    }
    reflect_str(it, host, proto, "target", "target", "");
    reflect_str(it, host, proto, "rel", "rel", "");
    reflect_str(it, host, proto, "download", "download", "");
    reflect_str(it, host, proto, "crossOrigin", "crossorigin", "");
    reflect_str(it, host, proto, "referrerPolicy", "referrerpolicy", "");
    reflect_str(it, host, proto, "loading", "loading", "auto");
    reflect_str(it, host, proto, "alt", "alt", "");
    reflect_str(it, host, proto, "type", "type", "");
    reflect_str(it, host, proto, "media", "media", "");
    reflect_num(it, host, proto, "width", "width", 0.0);
    reflect_num(it, host, proto, "height", "height", 0.0);
    ro_prop(it, host, proto, "naturalWidth", |it, _h, _id| {
        let _ = it;
        Ok(Value::Number(0.0))
    });
    ro_prop(it, host, proto, "naturalHeight", |it, _h, _id| {
        let _ = it;
        Ok(Value::Number(0.0))
    });
    ro_prop(it, host, proto, "complete", |it, _h, _id| {
        let _ = it;
        Ok(Value::Boolean(true))
    });
    ro_prop(it, host, proto, "currentSrc", |it, _h, _id| {
        let _ = it;
        Ok(Value::string(""))
    });

    // --------------------------------------------------------------- forms ---
    reflect_str(it, host, proto, "name", "name", "");
    reflect_str(it, host, proto, "placeholder", "placeholder", "");
    reflect_bool(it, host, proto, "disabled", "disabled");
    reflect_bool(it, host, proto, "required", "required");
    reflect_bool(it, host, proto, "readOnly", "readonly");
    reflect_bool(it, host, proto, "autofocus", "autofocus");
    reflect_bool(it, host, proto, "multiple", "multiple");
    reflect_bool(it, host, proto, "checked", "checked");
    reflect_bool(it, host, proto, "selected", "selected");
    reflect_bool(it, host, proto, "indeterminate", "indeterminate");
    // `form.action` is a URL: it reads back absolute, the attribute stays as written
    url_reflect(it, host, proto, "action", "action");
    reflect_str(it, host, proto, "method", "method", "get");
    reflect_str(
        it,
        host,
        proto,
        "enctype",
        "enctype",
        "application/x-www-form-urlencoded",
    );
    reflect_str(it, host, proto, "min", "min", "");
    reflect_str(it, host, proto, "max", "max", "");
    reflect_str(it, host, proto, "step", "step", "");
    reflect_str(it, host, proto, "pattern", "pattern", "");
    reflect_num(it, host, proto, "maxLength", "maxlength", -1.0);
    reflect_num(it, host, proto, "rows", "rows", 2.0);
    reflect_num(it, host, proto, "cols", "cols", 20.0);
    rw_prop(
        it,
        host,
        proto,
        "value",
        |_it, h, id, _| Ok(Value::string(h.form_value(id, || value_of(&h.dom, id)))),
        |it, h, id, v| {
            let s = if v.is_nullish() {
                String::new()
            } else {
                it.to_string_val(&v)?
            };
            set_value(h, id, &s)?;
            h.touch();
            Ok(())
        },
    );
    rw_prop(
        it,
        host,
        proto,
        "defaultValue",
        |_it, h, id, _| Ok(Value::string(h.dom.attr(id, "value").unwrap_or_default())),
        |it, h, id, v| {
            let s = it.to_string_val(&v)?;
            h.dom.set_attr(id, "value", s);
            h.touch();
            Ok(())
        },
    );
    ro_prop(it, host, proto, "options", |it, h, id| {
        let _ = it;
        let opts = h
            .dom
            .descendants(id)
            .into_iter()
            .filter(|n| h.dom.tag(*n) == Some("option"))
            .collect();
        Ok(node_list(it, h, opts))
    });
    ro_prop(it, host, proto, "length", |it, h, id| {
        let _ = it;
        match h.dom.tag(id) {
            Some("select") => Ok(Value::Number(
                h.dom
                    .descendants(id)
                    .into_iter()
                    .filter(|n| h.dom.tag(*n) == Some("option"))
                    .count() as f64,
            )),
            Some("form") => Ok(Value::Number(
                h.dom
                    .descendants(id)
                    .into_iter()
                    .filter(|n| {
                        matches!(
                            h.dom.tag(*n),
                            Some("input") | Some("select") | Some("textarea") | Some("button")
                        )
                    })
                    .count() as f64,
            )),
            _ => Ok(Value::Number(0.0)),
        }
    });
    rw_prop(
        it,
        host,
        proto,
        "selectedIndex",
        |_it, h, id, _| Ok(Value::Number(selected_index(&h.dom, id) as f64)),
        |_it, h, id, v| {
            let want = v.to_integer() as usize;
            let opts: Vec<NodeId> = h
                .dom
                .descendants(id)
                .into_iter()
                .filter(|n| h.dom.tag(*n) == Some("option"))
                .collect();
            for (i, o) in opts.iter().enumerate() {
                if i == want {
                    h.dom.set_attr(*o, "selected", "");
                } else {
                    h.dom.remove_attr(*o, "selected");
                }
            }
            // selecting an <option> *is* the value of a select
            h.clear_form_value(id);
            h.touch();
            Ok(())
        },
    );
    ro_prop(it, host, proto, "form", |it, h, id| {
        let _ = it;
        let hit = crate::query::closest(&h.dom, &h.match_state, id, "form");
        Ok(crate::node::maybe_wrap(h, hit))
    });
    ro_prop(it, host, proto, "elements", |it, h, id| {
        let _ = it;
        let hits =
            crate::query::query_all(&h.dom, &h.match_state, id, "input,select,textarea,button");
        Ok(node_list(it, h, hits))
    });

    // ------------------------------------------------------- scripts / media -
    reflect_bool(it, host, proto, "async", "async");
    reflect_bool(it, host, proto, "defer", "defer");
    reflect_bool(it, host, proto, "noModule", "nomodule");
    rw_prop(
        it,
        host,
        proto,
        "text",
        |_it, h, id, _| Ok(Value::string(h.dom.text_content(id))),
        |it, h, id, v| {
            let s = it.to_string_val(&v)?;
            h.dom.set_text_content(id, &s);
            h.touch();
            Ok(())
        },
    );
    ro_prop(it, host, proto, "contentDocument", |it, _h, _id| {
        let _ = it;
        Ok(Value::Null)
    });
    ro_prop(it, host, proto, "contentWindow", |it, _h, _id| {
        let _ = it;
        Ok(Value::Null)
    });
    ro_prop(it, host, proto, "shadowRoot", |it, _h, _id| {
        let _ = it;
        Ok(Value::Null)
    });
    method(it, host, proto, "attachShadow", 1, |it, _h, _id, _args| {
        Err(it.type_error("attachShadow: shadow DOM is not implemented"))
    });
    method(it, host, proto, "getContext", 1, |it, h, _id, _args| {
        h.log_problem("canvas getContext() returns null (no 2D context)");
        let _ = it;
        Ok(Value::Null)
    });
    method(it, host, proto, "toDataURL", 0, |it, _h, _id, _args| {
        let _ = it;
        Ok(Value::string(""))
    });
    method(it, host, proto, "play", 0, |_it, _h, _id, _args| {
        Ok(Value::Undefined)
    });
    method(it, host, proto, "pause", 0, |_it, _h, _id, _args| {
        Ok(Value::Undefined)
    });
    method(it, host, proto, "load", 0, |_it, _h, _id, _args| {
        Ok(Value::Undefined)
    });
    ro_prop(it, host, proto, "paused", |it, _h, _id| {
        let _ = it;
        Ok(Value::Boolean(true))
    });
    rw_prop(
        it,
        host,
        proto,
        "currentTime",
        |_it, _h, _id, _| Ok(Value::Number(0.0)),
        |_it, _h, _id, _v| Ok(()),
    );
    ro_prop(it, host, proto, "duration", |it, _h, _id| {
        let _ = it;
        Ok(Value::Number(f64::NAN))
    });

    // ---------------------------------------------------------- interaction --
    // click/focus/blur dispatch events → no host borrow while they run
    method_unlocked(it, host, proto, "click", 0, |it, href, id, _args| {
        // `fire` also runs the default action, so `el.click()` and a click the
        // embedder fired from outside behave exactly the same
        events::fire(href, it, EvtTarget::node(id), "click", true)?;
        Ok(Value::Undefined)
    });
    method_unlocked(it, host, proto, "focus", 0, |it, href, id, _args| {
        let previous = {
            let Ok(mut h) = href.try_borrow_mut() else {
                return Err(it.type_error("focus: DOM is busy"));
            };
            let prev = h.match_state.focus.filter(|p| *p != id);
            h.match_state.focus = Some(id);
            prev
        };
        // the old element blurs first, like in a browser
        if let Some(p) = previous {
            let ok = {
                let h = href
                    .try_borrow()
                    .map_err(|_| it.type_error("focus: DOM is busy"))?;
                h.dom.exists(p)
            };
            if ok {
                events::fire(href, it, EvtTarget::node(p), "blur", false)?;
            }
        }
        events::fire(href, it, EvtTarget::node(id), "focus", false)?;
        events::fire(href, it, EvtTarget::node(id), "focusin", true)?;
        Ok(Value::Undefined)
    });
    method_unlocked(it, host, proto, "blur", 0, |it, href, id, _args| {
        {
            let Ok(mut h) = href.try_borrow_mut() else {
                return Err(it.type_error("blur: DOM is busy"));
            };
            if h.match_state.focus == Some(id) {
                h.match_state.focus = None;
            }
        }
        events::fire(href, it, EvtTarget::node(id), "blur", false)?;
        events::fire(href, it, EvtTarget::node(id), "focusout", true)?;
        Ok(Value::Undefined)
    });
    method(it, host, proto, "select", 0, |_it, _h, _id, _args| {
        Ok(Value::Undefined)
    });
    method(
        it,
        host,
        proto,
        "setSelectionRange",
        2,
        |_it, _h, _id, _args| Ok(Value::Undefined),
    );
    method(
        it,
        host,
        proto,
        "scrollIntoView",
        1,
        |_it, _h, _id, _args| Ok(Value::Undefined),
    );
    method(
        it,
        host,
        proto,
        "checkValidity",
        0,
        |_it, _h, _id, _args| Ok(Value::Boolean(true)),
    );
    method(
        it,
        host,
        proto,
        "reportValidity",
        0,
        |_it, _h, _id, _args| Ok(Value::Boolean(true)),
    );
    method(
        it,
        host,
        proto,
        "setCustomValidity",
        1,
        |_it, _h, _id, _args| Ok(Value::Undefined),
    );
    method_unlocked(it, host, proto, "submit", 0, |it, href, id, _args| {
        let allow = events::fire(href, it, EvtTarget::node(id), "submit", true)?;
        if allow.to_boolean() {
            let action = href.try_borrow().ok().and_then(|h| {
                let raw = h.dom.attr(id, "action").unwrap_or_default();
                if raw.is_empty() || raw.starts_with('#') {
                    None
                } else {
                    Some(resolve_display(&h, &raw))
                }
            });
            if let Some(url) = action {
                let Ok(mut h) = href.try_borrow_mut() else {
                    return Ok(Value::Undefined);
                };
                h.request(crate::host::NavRequest::Load(url));
            }
        }
        Ok(Value::Undefined)
    });
    method_unlocked(it, host, proto, "reset", 0, |it, href, id, _args| {
        events::fire(href, it, EvtTarget::node(id), "reset", true)?;
        // every control goes back to what the markup said
        if let Ok(mut h) = href.try_borrow_mut() {
            if h.dom.tag(id) == Some("form") {
                let kids = h.dom.descendants(id);
                for k in kids {
                    h.clear_form_value(k);
                }
            }
            h.clear_form_value(id);
            h.touch();
        }
        Ok(Value::Undefined)
    });
}

// ---------------------------------------------------------------------------
// values
// ---------------------------------------------------------------------------

/// `el.value`: the `value` attribute for inputs, the text for textarea, the
/// selected option for a select.
pub fn value_of(dom: &tb_dom::Dom, id: NodeId) -> String {
    match dom.tag(id) {
        Some("textarea") => dom.text_content(id),
        Some("select") => {
            let opts: Vec<NodeId> = dom
                .descendants(id)
                .into_iter()
                .filter(|n| dom.tag(*n) == Some("option"))
                .collect();
            let picked = opts
                .iter()
                .find(|o| dom.has_attr(**o, "selected"))
                .or_else(|| opts.first());
            match picked {
                Some(o) => option_value(dom, *o),
                None => String::new(),
            }
        }
        Some("input") => {
            if dom.attr(id, "type").as_deref() == Some("checkbox") {
                return if dom.has_attr(id, "checked") {
                    dom.attr(id, "value").unwrap_or_else(|| "on".into())
                } else {
                    String::new()
                };
            }
            dom.attr(id, "value").unwrap_or_default()
        }
        Some("option") => option_value(dom, id),
        _ => dom.attr(id, "value").unwrap_or_default(),
    }
}

fn option_value(dom: &tb_dom::Dom, id: NodeId) -> String {
    dom.attr(id, "value")
        .unwrap_or_else(|| dom.text_content(id).trim().to_string())
}

/// Set the *API* value of a control. `input` and `textarea` remember it apart
/// from the attribute (`defaultValue` stays); `select` moves the `selected`
/// flag, because that is how a select's value is expressed in the markup.
fn set_value(h: &mut Host, id: NodeId, value: &str) -> JsResult<()> {
    match h.dom.tag(id) {
        Some("option") => h.dom.set_attr(id, "value", value),
        Some("select") => {
            let opts: Vec<NodeId> = h
                .dom
                .descendants(id)
                .into_iter()
                .filter(|n| h.dom.tag(*n) == Some("option"))
                .collect();
            for o in opts {
                if option_value(&h.dom, o) == value {
                    h.dom.set_attr(o, "selected", "");
                } else {
                    h.dom.remove_attr(o, "selected");
                }
            }
            h.clear_form_value(id);
        }
        // everything else keeps the value beside the attribute
        _ => {
            h.values.insert(id, value.to_string());
        }
    }
    Ok(())
}

fn selected_index(dom: &tb_dom::Dom, id: NodeId) -> usize {
    let opts: Vec<NodeId> = dom
        .descendants(id)
        .into_iter()
        .filter(|n| dom.tag(*n) == Some("option"))
        .collect();
    opts.iter()
        .position(|o| dom.has_attr(*o, "selected"))
        .unwrap_or(if opts.is_empty() { 0 } else { 0 })
}

/// What a click does when no listener cancelled it: a checkbox or radio button
/// changes state (and fires `change`), an anchor asks the browser to navigate.
///
/// Lives here and is called from `events::fire`, so `el.click()` and a click
/// the embedder fired (`Page::click`, a hit test, a real pointer event) take
/// the same path — a browser has one click, not two.
pub fn click_default_action(
    host: &crate::host::HostRef,
    it: &mut tb_js::Interp,
    id: tb_dom::NodeId,
) -> tb_js::JsResult<tb_js::Value> {
    // a checkbox or radio button reacts to the click, then says so
    let control = {
        let Ok(h) = host.try_borrow() else {
            return Ok(Value::Undefined);
        };
        match h.dom.tag(id) {
            Some("input") => match h.dom.attr(id, "type").as_deref() {
                Some("checkbox") => Some(true),
                Some("radio") => Some(false),
                _ => None,
            },
            _ => None,
        }
    };
    if let Some(toggle) = control {
        let changed = {
            let Ok(mut h) = host.try_borrow_mut() else {
                return Ok(Value::Undefined);
            };
            if toggle {
                if h.dom.has_attr(id, "checked") {
                    h.dom.remove_attr(id, "checked");
                } else {
                    h.dom.set_attr(id, "checked", "");
                }
                true
            } else if !h.dom.has_attr(id, "checked") {
                // a radio button unchecks its siblings of the same name
                let name = h.dom.attr(id, "name").unwrap_or_default();
                let siblings = h.dom.get_elements_by_tag("input");
                for s in siblings {
                    if s != id
                        && h.dom.attr(s, "type").as_deref() == Some("radio")
                        && h.dom.attr(s, "name").unwrap_or_default() == name
                    {
                        h.dom.remove_attr(s, "checked");
                    }
                }
                h.dom.set_attr(id, "checked", "");
                true
            } else {
                false
            }
        };
        if changed {
            events::fire(host, it, EvtTarget::node(id), "change", true)?;
        }
    }
    // a link whose click was not prevented asks the browser to navigate
    let (is_anchor, href_attr) = {
        let Ok(h) = host.try_borrow() else {
            return Ok(Value::Undefined);
        };
        (
            h.dom.tag(id) == Some("a"),
            h.dom.attr(id, "href").unwrap_or_default(),
        )
    };
    if is_anchor && !href_attr.is_empty() && !href_attr.starts_with('#') {
        let url = {
            let h = host
                .try_borrow()
                .map_err(|_| it.type_error("click: DOM is busy"))?;
            resolve_display(&h, &href_attr)
        };
        let mut h = host
            .try_borrow_mut()
            .map_err(|_| it.type_error("click: DOM is busy"))?;
        h.request(crate::host::NavRequest::Load(url));
    }
    Ok(Value::Undefined)
}
