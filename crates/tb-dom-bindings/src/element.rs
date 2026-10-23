//! `Element.prototype` — attributes, selectors, HTML, geometry.

use std::rc::Rc;

use tb_dom::NodeId;
use tb_js::value::{Key, Obj, ObjRef, Value};
use tb_js::{Interp, JsResult};

use crate::host::{def_native, method, node_list, ro_prop, rw_prop, Host, HostRef};
use crate::node::node_arg;
use crate::query;

/// Parse `html` and put the result somewhere around `id`.
///
/// Fragments are parsed as children of a fresh `<div>` (that is what the real
/// `parseFragment` does with a context element) and the children are then moved
/// to the requested position.
pub fn insert_fragment(h: &mut Host, id: NodeId, position: &str, html: &str) -> JsResult<()> {
    let temp = h.dom.create_element("div");
    tb_html::parse_fragment(&mut h.dom, temp, html);
    let kids = h.dom.children(temp);
    match position {
        "beforeend" => {
            for k in kids {
                h.dom.appendChild(id, k);
            }
        }
        "afterbegin" => {
            let first = h.dom.first_child(id);
            for k in kids {
                h.dom.insertBefore(id, k, first);
            }
        }
        "beforebegin" => {
            let Some(parent) = h.dom.parent(id) else {
                return Ok(());
            };
            for k in kids {
                h.dom.insertBefore(parent, k, Some(id));
            }
        }
        "afterend" => {
            let Some(parent) = h.dom.parent(id) else {
                return Ok(());
            };
            let next = h.dom.next_sibling(id);
            for k in kids {
                h.dom.insertBefore(parent, k, next);
            }
        }
        other => {
            h.dom.detach(temp);
            h.dom.clear_children(temp);
            return Err(it_error(&format!(
                "insertAdjacentHTML: '{other}' is not a valid insertion position"
            )));
        }
    }
    h.dom.detach(temp);
    h.dom.clear_children(temp);
    h.touch();
    Ok(())
}

/// Throw-values do not need an interpreter, but our helpers return `Value`s.
fn it_error(msg: &str) -> Value {
    let o = Rc::new(std::cell::RefCell::new(Obj::plain()));
    o.borrow_mut()
        .define_internal("name", Value::string("SyntaxError"));
    o.borrow_mut()
        .define_internal("message", Value::string(msg));
    Value::Object(o)
}

/// `element.getBoundingClientRect()` — a plain rect object (document coordinates
/// minus the scroll offset, like the real thing).
pub fn rect_object(it: &mut Interp, x: f64, y: f64, w: f64, h: f64) -> Value {
    let o = it.plain_object();
    for (name, value) in [
        ("x", Value::Number(x)),
        ("y", Value::Number(y)),
        ("width", Value::Number(w)),
        ("height", Value::Number(h)),
        ("left", Value::Number(x)),
        ("top", Value::Number(y)),
        ("right", Value::Number(x + w)),
        ("bottom", Value::Number(y + h)),
    ] {
        let _ = it.set(&o, &Key::Name(Rc::from(name)), value);
    }
    o
}

pub fn install(it: &mut Interp, host: &HostRef, proto: &ObjRef) {
    // ------------------------------------------------------------ identity ---
    ro_prop(it, host, proto, "tagName", |it, h, id| {
        let _ = it;
        Ok(Value::string(
            h.dom.tag(id).unwrap_or("").to_ascii_uppercase(),
        ))
    });
    ro_prop(it, host, proto, "localName", |it, h, id| {
        let _ = it;
        Ok(Value::string(h.dom.tag(id).unwrap_or("")))
    });
    ro_prop(it, host, proto, "nodeType", |_it, h, id| {
        Ok(Value::Number(crate::node::node_type(&h.dom, id)))
    });

    // ----------------------------------------------------------- attributes --
    rw_prop(
        it,
        host,
        proto,
        "id",
        |it, h, id, _| {
            let _ = it;
            Ok(Value::string(h.dom.attr(id, "id").unwrap_or_default()))
        },
        |it, h, id, v| {
            let s = it.to_string_val(&v)?;
            h.dom.set_attr(id, "id", s);
            h.touch();
            Ok(())
        },
    );
    rw_prop(
        it,
        host,
        proto,
        "className",
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
    ro_prop(it, host, proto, "classList", |it, h, id| {
        Ok(crate::tokens::token_list(it, h, id))
    });
    ro_prop(it, host, proto, "style", |it, h, id| {
        Ok(crate::style_api::style_object(it, h, id))
    });
    ro_prop(it, host, proto, "dataset", |it, h, id| {
        let _ = it;
        Ok(crate::style_api::dataset(h, id))
    });
    // `el.attributes` — a `NamedNodeMap`: an array of attribute objects plus
    // `getNamedItem`/`setNamedItem`/`removeNamedItem`, which work on the node
    // rather than on the snapshot in the array.
    crate::host::ro_prop_h(it, host, proto, "attributes", |it, host, h, id| {
        let owner = h.wrap(id);
        let items: Vec<Value> = h
            .dom
            .element(id)
            .map(|el| {
                el.attrs
                    .iter()
                    .map(|a| attr_obj(it, Some(owner.clone()), &a.name, &a.value))
                    .collect()
            })
            .unwrap_or_default();
        let list = it.array(items);

        def_native(it, &list, "getNamedItem", 1, {
            let hs = host.clone();
            move |it: &mut Interp, _this: Value, a: Vec<Value>| {
                let name = attr_name(it, a.first())?;
                let mut h = hs
                    .try_borrow_mut()
                    .map_err(|_| it.type_error("attributes: DOM is busy"))?;
                Ok(match h.dom.attr(id, &name) {
                    Some(v) => attr_obj(it, Some(h.wrap(id)), &name, &v),
                    None => Value::Null,
                })
            }
        });
        def_native(it, &list, "setNamedItem", 1, {
            let hs = host.clone();
            move |it: &mut Interp, _this: Value, a: Vec<Value>| {
                let Some(attr) = a.first().and_then(|v| v.as_object()).cloned() else {
                    return Err(it.type_error("setNamedItem: an attribute object is required"));
                };
                let name = it
                    .get(&Value::Object(attr.clone()), &Key::Name(Rc::from("name")))
                    .map(|v| v.to_string())
                    .unwrap_or_default();
                let value = it
                    .get(&Value::Object(attr), &Key::Name(Rc::from("value")))
                    .map(|v| v.to_string())
                    .unwrap_or_default();
                let mut h = hs
                    .try_borrow_mut()
                    .map_err(|_| it.type_error("attributes: DOM is busy"))?;
                h.dom.set_attr(id, &name, value);
                h.touch();
                Ok(Value::Null)
            }
        });
        def_native(it, &list, "removeNamedItem", 1, {
            let hs = host.clone();
            move |it: &mut Interp, _this: Value, a: Vec<Value>| {
                let name = attr_name(it, a.first())?;
                let mut h = hs
                    .try_borrow_mut()
                    .map_err(|_| it.type_error("attributes: DOM is busy"))?;
                let old = h.dom.attr(id, &name);
                h.dom.remove_attr(id, &name);
                h.touch();
                Ok(match old {
                    Some(v) => attr_obj(it, None, &name, &v),
                    None => Value::Null,
                })
            }
        });
        let named = {
            let hs = host.clone();
            it.native_fn(
                "namedItem",
                1,
                move |it: &mut Interp, _this: Value, a: Vec<Value>| {
                    let name = attr_name(it, a.first())?;
                    let mut h = hs
                        .try_borrow_mut()
                        .map_err(|_| it.type_error("attributes: DOM is busy"))?;
                    Ok(match h.dom.attr(id, &name) {
                        Some(v) => attr_obj(it, Some(h.wrap(id)), &name, &v),
                        None => Value::Null,
                    })
                },
            )
        };
        list.as_object()
            .unwrap()
            .borrow_mut()
            .define_internal("namedItem", named);
        Ok(list)
    });

    method(it, host, proto, "getAttribute", 1, |it, h, id, args| {
        let name = attr_name(it, args.first())?;
        Ok(match h.dom.attr(id, &name) {
            Some(v) => Value::string(v),
            None => Value::Null,
        })
    });
    method(it, host, proto, "setAttribute", 2, |it, h, id, args| {
        let name = attr_name(it, args.first())?;
        let value = args
            .get(1)
            .map(|v| it.to_string_val(v))
            .transpose()?
            .unwrap_or_default();
        h.dom.set_attr(id, &name, value);
        h.touch();
        Ok(Value::Undefined)
    });
    method(it, host, proto, "removeAttribute", 1, |it, h, id, args| {
        let name = attr_name(it, args.first())?;
        h.dom.remove_attr(id, &name);
        h.touch();
        Ok(Value::Undefined)
    });
    method(it, host, proto, "hasAttribute", 1, |it, h, id, args| {
        let name = attr_name(it, args.first())?;
        Ok(Value::Boolean(h.dom.has_attr(id, &name)))
    });
    method(it, host, proto, "hasAttributes", 0, |_it, h, id, _args| {
        Ok(Value::Boolean(
            h.dom
                .element(id)
                .map(|e| !e.attrs.is_empty())
                .unwrap_or(false),
        ))
    });
    method(
        it,
        host,
        proto,
        "getAttributeNames",
        0,
        |it, h, id, _args| {
            let _ = it;
            let names: Vec<Value> = h
                .dom
                .element(id)
                .map(|e| e.attrs.iter().map(|a| Value::string(&a.name)).collect())
                .unwrap_or_default();
            Ok(it.array(names))
        },
    );
    method(it, host, proto, "toggleAttribute", 1, |it, h, id, args| {
        let name = attr_name(it, args.first())?;
        let force = args
            .get(1)
            .filter(|v| !v.is_undefined())
            .map(|v| v.to_boolean());
        let on = match force {
            Some(f) => f,
            None => !h.dom.has_attr(id, &name),
        };
        if on {
            h.dom.set_attr(id, &name, "");
        } else {
            h.dom.remove_attr(id, &name);
        }
        h.touch();
        Ok(Value::Boolean(on))
    });

    // ------------------------------------------------------------ selectors --
    method(it, host, proto, "matches", 1, |it, h, id, args| {
        let sel = string_arg(it, args.first(), "matches")?;
        Ok(Value::Boolean(query::matches_selector(
            &h.dom,
            &h.match_state,
            id,
            &sel,
        )))
    });
    method(it, host, proto, "closest", 1, |it, h, id, args| {
        let sel = string_arg(it, args.first(), "closest")?;
        let hit = query::closest(&h.dom, &h.match_state, id, &sel);
        Ok(crate::node::maybe_wrap(h, hit))
    });
    method(it, host, proto, "querySelector", 1, |it, h, id, args| {
        let sel = string_arg(it, args.first(), "querySelector")?;
        let hit = query::query_first(&h.dom, &h.match_state, id, &sel);
        Ok(crate::node::maybe_wrap(h, hit))
    });
    method(it, host, proto, "querySelectorAll", 1, |it, h, id, args| {
        let sel = string_arg(it, args.first(), "querySelectorAll")?;
        let hits = query::query_all(&h.dom, &h.match_state, id, &sel);
        Ok(node_list(it, h, hits))
    });

    // ----------------------------------------------------------------- html --
    rw_prop(
        it,
        host,
        proto,
        "innerHTML",
        |it, h, id, _| {
            let _ = it;
            Ok(Value::string(h.dom.inner_html(id)))
        },
        |it, h, id, v| {
            let html = if v.is_nullish() {
                String::new()
            } else {
                it.to_string_val(&v)?
            };
            h.dom.clear_children(id);
            insert_fragment(h, id, "beforeend", &html)?;
            h.touch();
            Ok(())
        },
    );
    rw_prop(
        it,
        host,
        proto,
        "outerHTML",
        |it, h, id, _| {
            let _ = it;
            Ok(Value::string(h.dom.outer_html(id)))
        },
        |it, h, id, v| {
            let html = it.to_string_val(&v)?;
            let Some(parent) = h.dom.parent(id) else {
                return Ok(());
            };
            let temp = h.dom.create_element("div");
            tb_html::parse_fragment(&mut h.dom, temp, &html);
            for k in h.dom.children(temp) {
                h.dom.insertBefore(parent, k, Some(id));
            }
            h.dom.detach(temp);
            h.dom.clear_children(temp);
            h.dom.detach(id);
            h.touch();
            Ok(())
        },
    );
    rw_prop(
        it,
        host,
        proto,
        "innerText",
        |it, h, id, _| {
            let _ = it;
            // readable text, whitespace collapsed like a browser renders it
            Ok(Value::string(tb_core::text::collapse_whitespace(
                &h.dom.visible_text(id),
            )))
        },
        |it, h, id, v| {
            let s = it.to_string_val(&v)?;
            h.dom.set_text_content(id, &s);
            h.touch();
            Ok(())
        },
    );
    method(
        it,
        host,
        proto,
        "insertAdjacentHTML",
        2,
        |it, h, id, args| {
            let position = string_arg(it, args.first(), "insertAdjacentHTML")?.to_ascii_lowercase();
            let html = string_arg(it, args.get(1), "insertAdjacentHTML")?;
            insert_fragment(h, id, &position, &html).map(|_| Value::Undefined)
        },
    );
    method(
        it,
        host,
        proto,
        "insertAdjacentElement",
        2,
        |it, h, id, args| {
            let position =
                string_arg(it, args.first(), "insertAdjacentElement")?.to_ascii_lowercase();
            let child = node_arg(it, &args, 1, "insertAdjacentElement")?;
            match position.as_str() {
                "beforeend" => h.dom.appendChild(id, child),
                "afterbegin" => {
                    let first = h.dom.first_child(id);
                    h.dom.insertBefore(id, child, first);
                }
                "beforebegin" => {
                    if let Some(p) = h.dom.parent(id) {
                        h.dom.insertBefore(p, child, Some(id));
                    }
                }
                "afterend" => {
                    if let Some(p) = h.dom.parent(id) {
                        let next = h.dom.next_sibling(id);
                        h.dom.insertBefore(p, child, next);
                    }
                }
                other => {
                    return Err(it.type_error(&format!(
                        "insertAdjacentElement: '{other}' is not a valid position"
                    )))
                }
            }
            h.touch();
            Ok(args[1].clone())
        },
    );
    method(it, host, proto, "appendText", 1, |it, h, id, args| {
        let s = string_arg(it, args.first(), "appendText")?;
        let t = h.dom.create_text(&s);
        h.dom.appendChild(id, t);
        h.touch();
        Ok(Value::Undefined)
    });

    // ------------------------------------------------------------- geometry --
    method(
        it,
        host,
        proto,
        "getBoundingClientRect",
        0,
        |_it, h, id, _args| {
            let (x, y, w, hh) = rect_tuple(h, id);
            Ok(rect_object(_it, x, y, w, hh))
        },
    );
    method(it, host, proto, "getClientRects", 0, |_it, h, id, _args| {
        let (x, y, w, hh) = rect_tuple(h, id);
        let r = rect_object(_it, x, y, w, hh);
        Ok(_it.array(vec![r]))
    });
    ro_prop(it, host, proto, "clientWidth", |it, h, id| {
        let _ = it;
        Ok(Value::Number(box_dim(h, id, true)))
    });
    ro_prop(it, host, proto, "clientHeight", |it, h, id| {
        let _ = it;
        Ok(Value::Number(box_dim(h, id, false)))
    });
    ro_prop(it, host, proto, "offsetWidth", |it, h, id| {
        let _ = it;
        Ok(Value::Number(
            h.rect_of(id).map(|r| r.width as f64).unwrap_or(0.0).round(),
        ))
    });
    ro_prop(it, host, proto, "offsetHeight", |it, h, id| {
        let _ = it;
        Ok(Value::Number(
            h.rect_of(id)
                .map(|r| r.height as f64)
                .unwrap_or(0.0)
                .round(),
        ))
    });
    ro_prop(it, host, proto, "offsetLeft", |it, h, id| {
        let _ = it;
        Ok(Value::Number(offset_side(h, id, true)))
    });
    ro_prop(it, host, proto, "offsetTop", |it, h, id| {
        let _ = it;
        Ok(Value::Number(offset_side(h, id, false)))
    });
    ro_prop(it, host, proto, "offsetParent", |it, h, id| {
        let _ = it;
        let p = positioned_ancestor(h, id);
        Ok(crate::node::maybe_wrap(h, p))
    });
    ro_prop(it, host, proto, "scrollWidth", |it, h, id| {
        let _ = it;
        Ok(Value::Number(scroll_extent(h, id, true)))
    });
    ro_prop(it, host, proto, "scrollHeight", |it, h, id| {
        let _ = it;
        Ok(Value::Number(scroll_extent(h, id, false)))
    });
    rw_prop(
        it,
        host,
        proto,
        "scrollTop",
        |it, h, _id, _| {
            let _ = (it, _id);
            Ok(Value::Number(h.scroll.1))
        },
        |it, h, _id, v| {
            let _ = it;
            let y = v.to_number().max(0.0);
            h.scroll = (h.scroll.0, y);
            h.request(crate::host::NavRequest::Scroll(h.scroll.0, h.scroll.1));
            Ok(())
        },
    );
    rw_prop(
        it,
        host,
        proto,
        "scrollLeft",
        |it, h, _id, _| {
            let _ = (it, _id);
            Ok(Value::Number(h.scroll.0))
        },
        |it, h, _id, v| {
            let _ = it;
            let x = v.to_number().max(0.0);
            h.scroll = (x, h.scroll.1);
            h.request(crate::host::NavRequest::Scroll(h.scroll.0, h.scroll.1));
            Ok(())
        },
    );
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

/// An `Attr`-shaped snapshot (`el.attributes[0]`, `getNamedItem(…)`).
fn attr_obj(it: &mut Interp, owner: Option<Value>, name: &str, value: &str) -> Value {
    let o = it.plain_object();
    let _ = it.set(&o, &Key::Name(Rc::from("name")), Value::string(name));
    let _ = it.set(&o, &Key::Name(Rc::from("value")), Value::string(value));
    let _ = it.set(&o, &Key::Name(Rc::from("specified")), Value::Boolean(true));
    let _ = it.set(
        &o,
        &Key::Name(Rc::from("ownerElement")),
        owner.unwrap_or(Value::Null),
    );
    o
}

pub fn attr_name(it: &mut Interp, v: Option<&Value>) -> JsResult<String> {
    match v {
        None => Err(it.type_error("attribute name is required")),
        Some(v) => Ok(it.to_string_val(v)?.to_ascii_lowercase()),
    }
}

pub fn string_arg(it: &mut Interp, v: Option<&Value>, what: &str) -> JsResult<String> {
    match v {
        None => Err(it.type_error(&format!("{what}: argument missing"))),
        Some(v) => it.to_string_val(v),
    }
}

/// Border box in viewport coordinates (scroll offset applied), all zeros when
/// the element is not laid out.
pub fn rect_tuple(h: &mut Host, id: NodeId) -> (f64, f64, f64, f64) {
    match h.rect_of(id) {
        Some(r) => (r.x as f64, r.y as f64, r.width as f64, r.height as f64),
        None => (0.0, 0.0, 0.0, 0.0),
    }
}

/// Width (`true`) or height of the padding box; the root element reports the
/// viewport, which is what scripts measuring `document.documentElement` expect.
fn box_dim(h: &mut Host, id: NodeId, width: bool) -> f64 {
    if h.dom.parent(id).is_none() {
        let vp = h.ctx.viewport();
        return f64::from(if width { vp.width } else { vp.height });
    }
    let Some(doc) = h.layout() else { return 0.0 };
    let Some(b) = doc.find(id) else {
        return 0.0;
    };
    let r = b.padding_box();
    f64::from(if width { r.width } else { r.height }).round()
}

/// `offsetLeft`/`offsetTop`: distance to the offset parent's border box.
fn offset_side(h: &mut Host, id: NodeId, left: bool) -> f64 {
    let Some(rect) = h.rect_of(id) else {
        return 0.0;
    };
    let base = match positioned_ancestor(h, id) {
        Some(p) => h.rect_of(p).unwrap_or(rect),
        None => tb_core::Rect::new(0.0, 0.0, 0.0, 0.0),
    };
    // rect_of subtracts the scroll offset for both, so it cancels out here
    f64::from(if left {
        rect.x - base.x
    } else {
        rect.y - base.y
    })
    .round()
}

fn positioned_ancestor(h: &mut Host, id: NodeId) -> Option<NodeId> {
    let styles = h.style_map();
    for a in h.dom.ancestors(id, false) {
        let Some(tag) = h.dom.tag(a) else { continue };
        let Some(style) = styles.get(a) else { continue };
        let positioned = style.position != tb_style::Position::Static;
        if positioned || matches!(tag, "body" | "td" | "th" | "table") {
            return Some(a);
        }
    }
    None
}

fn scroll_extent(h: &mut Host, id: NodeId, width: bool) -> f64 {
    let is_root =
        h.dom.parent(id).is_none() || matches!(h.dom.tag(id), Some("body") | Some("html"));
    if let Some(doc) = h.layout() {
        if is_root {
            let s = doc.scroll_size;
            return f64::from(if width { s.width } else { s.height }).round();
        }
        if let Some(b) = doc.find(id) {
            let bb = b.border_box();
            let content = f64::from(b.scroll_extent);
            let own = f64::from(if width { bb.width } else { bb.height });
            return (content.max(own)).round();
        }
    }
    0.0
}
