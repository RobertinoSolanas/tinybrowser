//! `document` — the HTMLDocument surface.
//!
//! The document is a node wrapper like any other (its `"@@node"` is the arena's
//! document node), so `document.contains(x)`, `document.addEventListener(…)`
//! and `document.body` all come from the prototypes they share with elements.

use std::rc::Rc;

use tb_dom::NodeId;
use tb_js::value::{Key, ObjRef, Value};
use tb_js::Interp;

use crate::element::insert_fragment;
use crate::host::{method, node_list, ro_prop, rw_prop, rw_this_prop, Host, HostRef};
use crate::node::maybe_wrap;
use crate::query;

/// Find the first element with this tag below `root` (document order).
fn first_tag(dom: &tb_dom::Dom, root: NodeId, tag: &str) -> Option<NodeId> {
    dom.descendants(root)
        .into_iter()
        .find(|n| dom.tag(*n) == Some(tag))
}

/// `<html>` — created on demand, because scripts legitimately write
/// `document.documentElement.appendChild(…)` on hand-built documents.
fn html_root(h: &mut Host) -> Option<NodeId> {
    let doc = h.dom.document();
    if let Some(e) = h.dom.child_elements(doc).into_iter().next() {
        return Some(e);
    }
    let html = h.dom.create_element("html");
    h.dom.appendChild(doc, html);
    h.touch();
    Some(html)
}

fn ensure_child(h: &mut Host, parent: NodeId, tag: &str) -> NodeId {
    if let Some(existing) = first_tag(&h.dom, parent, tag) {
        return existing;
    }
    let el = h.dom.create_element(tag);
    h.dom.appendChild(parent, el);
    h.touch();
    el
}

/// `document.title`: the `<title>` element's text.
pub fn title_of(h: &Host) -> String {
    let doc = h.dom.document();
    match first_tag(&h.dom, doc, "title") {
        Some(t) => h.dom.text_content(t),
        None => h.ctx.title.clone(),
    }
}

fn set_title(h: &mut Host, text: &str) {
    let doc = h.dom.document();
    match first_tag(&h.dom, doc, "title") {
        Some(t) => h.dom.set_text_content(t, text),
        None => {
            let head = match html_root(h).and_then(|r| first_tag(&h.dom, r, "head")) {
                Some(head) => head,
                None => return,
            };
            let t = h.dom.create_element("title");
            h.dom.appendChild(head, t);
            h.dom.set_text_content(t, text);
        }
    }
    h.ctx.title = text.to_string();
    h.touch();
}

pub fn install(it: &mut Interp, host: &HostRef, proto: &ObjRef) {
    // ------------------------------------------------------------- document --
    ro_prop(it, host, proto, "documentElement", |it, h, _id| {
        let _ = it;
        let root = html_root(h);
        Ok(maybe_wrap(h, root))
    });
    ro_prop(it, host, proto, "head", |it, h, _id| {
        let _ = it;
        let head = html_root(h).and_then(|r| Some(ensure_child(h, r, "head")));
        Ok(maybe_wrap(h, head))
    });
    rw_prop(
        it,
        host,
        proto,
        "body",
        |it, h, _id, _| {
            let _ = it;
            let doc = h.dom.document();
            let body =
                first_tag(&h.dom, doc, "body").or_else(|| first_tag(&h.dom, doc, "frameset"));
            Ok(maybe_wrap(h, body))
        },
        |it, h, _id, v| {
            let _ = it;
            let Some(new_body) = args_node(v) else {
                return Ok(());
            };
            let Some(root) = html_root(h) else {
                return Ok(());
            };
            match first_tag(&h.dom, root, "body") {
                Some(old) => h.dom.replace_child(root, new_body, old),
                None => h.dom.appendChild(root, new_body),
            }
            h.touch();
            Ok(())
        },
    );
    rw_prop(
        it,
        host,
        proto,
        "title",
        |_it, h, _id, _| Ok(Value::string(title_of(h))),
        |_it, h, _id, v| {
            let text = _it.to_string_val(&v)?;
            set_title(h, &text);
            Ok(())
        },
    );
    ro_prop(it, host, proto, "readyState", |it, h, _id| {
        let _ = it;
        Ok(Value::string(h.ready.as_str()))
    });
    ro_prop(it, host, proto, "referrer", |it, h, _id| {
        let _ = it;
        Ok(Value::string(&h.ctx.referrer))
    });
    ro_prop(it, host, proto, "URL", |it, h, _id| {
        let _ = it;
        Ok(Value::string(h.url_string()))
    });
    ro_prop(it, host, proto, "documentURI", |it, h, _id| {
        let _ = it;
        Ok(Value::string(h.url_string()))
    });
    ro_prop(it, host, proto, "xmlVersion", |_it, _h, _id| {
        Ok(Value::Null)
    });
    ro_prop(it, host, proto, "inputMode", |_it, _h, _id| {
        Ok(Value::string("text/html"))
    });
    ro_prop(it, host, proto, "contentType", |_it, _h, _id| {
        Ok(Value::string("text/html"))
    });
    ro_prop(it, host, proto, "compatMode", |_it, h, id| {
        // no doctype (or anything but HTML5) means quirks mode
        let standards = h.dom.children(id).into_iter().any(|c| {
            matches!(
                h.dom.get(c).map(|n| &n.kind),
                Some(tb_dom::NodeKind::Doctype { .. })
            )
        });
        Ok(Value::string(if standards {
            "CSS1Compat"
        } else {
            "BackCompat"
        }))
    });
    ro_prop(it, host, proto, "characterSet", |it, _h, _id| {
        let _ = it;
        Ok(Value::string("UTF-8"))
    });
    ro_prop(it, host, proto, "charset", |it, _h, _id| {
        let _ = it;
        Ok(Value::string("UTF-8"))
    });
    ro_prop(it, host, proto, "inputEncoding", |it, _h, _id| {
        let _ = it;
        Ok(Value::string("UTF-8"))
    });
    ro_prop(it, host, proto, "compatMode", |_it, h, _id| {
        Ok(Value::string(if h.dom.quirks {
            "BackCompat"
        } else {
            "CSS1Compat"
        }))
    });
    ro_prop(it, host, proto, "hidden", |it, _h, _id| {
        let _ = it;
        Ok(Value::Boolean(false))
    });
    ro_prop(it, host, proto, "visibilityState", |it, _h, _id| {
        let _ = it;
        Ok(Value::string("visible"))
    });
    ro_prop(it, host, proto, "activeElement", |it, h, _id| {
        let _ = it;
        match h.match_state.focus {
            Some(f) => Ok(maybe_wrap(h, Some(f))),
            None => {
                let body = first_tag(&h.dom, h.dom.document(), "body");
                Ok(maybe_wrap(h, body))
            }
        }
    });
    ro_prop(it, host, proto, "defaultView", |it, h, _id| {
        let _ = it;
        Ok(h.window.clone().map(Value::Object).unwrap_or(Value::Null))
    });
    // `document.cookie` — read the jar, write one cookie line
    rw_this_prop(
        it,
        host,
        proto,
        "cookie",
        |_it, h, _this, _| Ok(Value::string(h.cookie_header())),
        |it, h, _this, v| {
            let raw = it.to_string_val(&v)?;
            h.set_cookie(&raw);
            Ok(())
        },
    );
    ro_prop(it, host, proto, "doctype", |it, _h, _id| {
        let _ = it;
        Ok(Value::Null)
    });
    ro_prop(it, host, proto, "implementation", |it, _h, _id| {
        let has_feature = it.native_fn("hasFeature", 2, |_it: &mut Interp, _t: Value, _| {
            Ok(Value::Boolean(true))
        });
        let impl_obj = it.plain_object();
        let _ = it.set(&impl_obj, &Key::Name(Rc::from("hasFeature")), has_feature);
        Ok(impl_obj)
    });

    // ---------------------------------------------------------- collections --
    ro_prop(it, host, proto, "all", |it, h, _id| {
        let _ = it;
        let els = h.dom.elements();
        Ok(node_list(it, h, els))
    });
    for (prop, tag) in [
        ("forms", "form"),
        ("images", "img"),
        ("embeds", "embed"),
        ("plugins", "embed"),
    ] {
        ro_prop(it, host, proto, prop, move |it, h, _id| {
            let _ = it;
            let hits = h.dom.get_elements_by_tag(tag);
            Ok(node_list(it, h, hits))
        });
    }
    ro_prop(it, host, proto, "links", |it, h, _id| {
        let _ = it;
        let hits: Vec<NodeId> = h
            .dom
            .elements()
            .into_iter()
            .filter(|n| {
                matches!(h.dom.tag(*n), Some("a") | Some("area")) && h.dom.has_attr(*n, "href")
            })
            .collect();
        Ok(node_list(it, h, hits))
    });
    ro_prop(it, host, proto, "scripts", |it, h, _id| {
        let _ = it;
        let hits = h.dom.get_elements_by_tag("script");
        Ok(node_list(it, h, hits))
    });
    ro_prop(it, host, proto, "styleSheets", |it, h, _id| {
        let _ = it;
        let items: Vec<Value> = h
            .sheets
            .iter()
            .map(|s| {
                let o = it.plain_object();
                let _ = it.set(
                    &o,
                    &Key::Name(Rc::from("href")),
                    if s.href.is_empty() {
                        Value::Null
                    } else {
                        Value::string(&s.href)
                    },
                );
                let _ = it.set(&o, &Key::Name(Rc::from("type")), Value::string("text/css"));
                let _ = it.set(&o, &Key::Name(Rc::from("disabled")), Value::Boolean(false));
                let _ = it.set(&o, &Key::Name(Rc::from("ownerNode")), Value::Null);
                o
            })
            .collect();
        Ok(it.array(items))
    });

    // ------------------------------------------------------------- factories -
    method(it, host, proto, "createElement", 1, |it, h, _id, args| {
        let tag = args
            .first()
            .map(|v| it.to_string_val(v).unwrap_or_default())
            .unwrap_or_else(|| "div".into());
        let el = h.dom.create_element(&tag);
        h.touch();
        Ok(h.wrap(el))
    });
    method(it, host, proto, "createElementNS", 2, |it, h, _id, args| {
        let ns = args
            .first()
            .map(|v| it.to_string_val(v).unwrap_or_default())
            .unwrap_or_default();
        let tag = args
            .get(1)
            .map(|v| it.to_string_val(v).unwrap_or_default())
            .unwrap_or_else(|| "div".into());
        let namespace = match ns.as_str() {
            "http://www.w3.org/2000/svg" => tb_dom::Namespace::Svg,
            "http://www.w3.org/1998/Math/MathML" => tb_dom::Namespace::MathMl,
            _ => tb_dom::Namespace::Html,
        };
        let el = h.dom.create_element_ns(&tag, namespace);
        h.touch();
        Ok(h.wrap(el))
    });
    method(it, host, proto, "createTextNode", 1, |it, h, _id, args| {
        let text = args
            .first()
            .map(|v| it.to_string_val(v).unwrap_or_default())
            .unwrap_or_default();
        let t = h.dom.create_text(&text);
        h.touch();
        Ok(h.wrap(t))
    });
    method(it, host, proto, "createComment", 1, |it, h, _id, args| {
        let text = args
            .first()
            .map(|v| it.to_string_val(v).unwrap_or_default())
            .unwrap_or_default();
        let c = h.dom.create_comment(&text);
        h.touch();
        Ok(h.wrap(c))
    });
    method(
        it,
        host,
        proto,
        "createDocumentFragment",
        0,
        |_it, h, _id, _args| {
            // our stand-in: an element named `#fragment` whose children get moved
            // by appendChild/insertBefore (see `node::is_fragment`)
            let f = h.dom.create_element("#fragment");
            Ok(h.wrap(f))
        },
    );
    method(it, host, proto, "createEvent", 1, |it, h, _id, args| {
        let _kind = args
            .first()
            .map(|v| it.to_string_val(v).unwrap_or_default())
            .unwrap_or_default();
        let proto = h.protos.event.clone();
        let ev = crate::events::make_event(it, "", None, proto);
        // initEvent(type, bubbles, cancelable) fills in what we left open
        if let Some(o) = ev.as_object() {
            let init = it.native_fn(
                "initEvent",
                3,
                |it: &mut Interp, this: Value, a: Vec<Value>| {
                    let kind = a
                        .first()
                        .map(|v| it.to_string_val(v).unwrap_or_default())
                        .unwrap_or_default();
                    crate::events::set_field(it, &this, crate::events::F_TYPE, Value::string(kind));
                    crate::events::set_field(
                        it,
                        &this,
                        crate::events::F_BUBBLES,
                        Value::Boolean(a.get(1).map(|v| v.to_boolean()).unwrap_or(false)),
                    );
                    crate::events::set_field(
                        it,
                        &this,
                        crate::events::F_CANCELABLE,
                        Value::Boolean(a.get(2).map(|v| v.to_boolean()).unwrap_or(false)),
                    );
                    Ok(Value::Undefined)
                },
            );
            o.borrow_mut().define_internal("initEvent", init);
        }
        Ok(ev)
    });
    method(it, host, proto, "importNode", 2, |it, h, _id, args| {
        let Some(node) = args.first().and_then(crate::host::node_of) else {
            return Err(it.type_error("importNode: argument is not a node"));
        };
        let deep = args.get(1).map(|v| v.to_boolean()).unwrap_or(true);
        let copy = h.dom.clone_node(node, deep);
        h.touch();
        Ok(h.wrap(copy))
    });
    method(it, host, proto, "adoptNode", 1, |_it, _h, _id, args| {
        Ok(args.first().cloned().unwrap_or(Value::Null))
    });

    // -------------------------------------------------------------- lookups --
    method(it, host, proto, "getElementById", 1, |it, h, _id, args| {
        let want = args
            .first()
            .map(|v| it.to_string_val(v).unwrap_or_default())
            .unwrap_or_default();
        let doc = h.dom.document();
        let found = h
            .dom
            .elements()
            .into_iter()
            .find(|n| h.dom.element_id(*n).as_deref() == Some(want.as_str()));
        let _ = doc;
        Ok(maybe_wrap(h, found))
    });
    method(
        it,
        host,
        proto,
        "getElementsByTagName",
        1,
        |it, h, _id, args| {
            let tag = args
                .first()
                .map(|v| it.to_string_val(v).unwrap_or_default())
                .unwrap_or_default();
            let hits = if tag == "*" {
                h.dom.elements()
            } else {
                h.dom.get_elements_by_tag(&tag.to_ascii_lowercase())
            };
            Ok(node_list(it, h, hits))
        },
    );
    method(
        it,
        host,
        proto,
        "getElementsByClassName",
        1,
        |it, h, _id, args| {
            let name = args
                .first()
                .map(|v| it.to_string_val(v).unwrap_or_default())
                .unwrap_or_default();
            let hits: Vec<NodeId> = if name.split_whitespace().count() > 1 {
                h.dom
                    .elements()
                    .into_iter()
                    .filter(|n| {
                        name.split_whitespace()
                            .all(|c| h.dom.element(*n).map(|e| e.has_class(c)).unwrap_or(false))
                    })
                    .collect()
            } else {
                h.dom.get_elements_by_class(&name)
            };
            Ok(node_list(it, h, hits))
        },
    );
    method(
        it,
        host,
        proto,
        "getElementsByName",
        1,
        |it, h, _id, args| {
            let name = args
                .first()
                .map(|v| it.to_string_val(v).unwrap_or_default())
                .unwrap_or_default();
            let hits: Vec<NodeId> = h
                .dom
                .elements()
                .into_iter()
                .filter(|n| h.dom.attr(*n, "name").as_deref() == Some(name.as_str()))
                .collect();
            Ok(node_list(it, h, hits))
        },
    );
    method(it, host, proto, "querySelector", 1, |it, h, id, args| {
        let sel = args
            .first()
            .map(|v| it.to_string_val(v).unwrap_or_default())
            .unwrap_or_default();
        let hit = query::query_first(&h.dom, &h.match_state, id, &sel);
        Ok(maybe_wrap(h, hit))
    });
    method(it, host, proto, "querySelectorAll", 1, |it, h, id, args| {
        let sel = args
            .first()
            .map(|v| it.to_string_val(v).unwrap_or_default())
            .unwrap_or_default();
        let hits = query::query_all(&h.dom, &h.match_state, id, &sel);
        Ok(node_list(it, h, hits))
    });

    // ------------------------------------------------------------ document.io -
    method(it, host, proto, "write", 1, |it, h, id, args| {
        let html = args
            .first()
            .map(|v| it.to_string_val(v).unwrap_or_default())
            .unwrap_or_default();
        // no insertion point model: content lands in <body> (or the document)
        let target = first_tag(&h.dom, id, "body").unwrap_or(id);
        insert_fragment(h, target, "beforeend", &html).map(|_| Value::Undefined)
    });
    method(it, host, proto, "writeln", 1, |it, h, id, args| {
        let mut a = args.clone();
        if let Some(Value::Str(s)) = a.first().cloned() {
            a[0] = Value::string(format!("{s}\n"));
        }
        let html = a
            .first()
            .map(|v| it.to_string_val(v).unwrap_or_default())
            .unwrap_or_default();
        let target = first_tag(&h.dom, id, "body").unwrap_or(id);
        insert_fragment(h, target, "beforeend", &html).map(|_| Value::Undefined)
    });
    method(it, host, proto, "open", 0, |_it, _h, _id, _args| {
        Ok(Value::Undefined)
    });
    method(it, host, proto, "close", 0, |_it, _h, _id, _args| {
        Ok(Value::Undefined)
    });
    method(it, host, proto, "hasFocus", 0, |_it, _h, _id, _args| {
        Ok(Value::Boolean(true))
    });
    method(
        it,
        host,
        proto,
        "exitPointerLock",
        0,
        |_it, _h, _id, _args| Ok(Value::Undefined),
    );
    method(it, host, proto, "execCommand", 1, |_it, _h, _id, _args| {
        Ok(Value::Boolean(false))
    });
    method(it, host, proto, "createRange", 0, |_it, _h, _id, _args| {
        Ok(Value::Null)
    });
    method(
        it,
        host,
        proto,
        "elementFromPoint",
        2,
        |_it, h, _id, args| {
            let x = args.first().map(|v| v.to_number()).unwrap_or(0.0) as f32;
            let y = args.get(1).map(|v| v.to_number()).unwrap_or(0.0) as f32;
            let hit = match h.layout() {
                Some(doc) => tb_layout::element_at(
                    &doc,
                    &h.dom,
                    x + h.scroll.0 as f32,
                    y + h.scroll.1 as f32,
                ),
                None => None,
            };
            Ok(maybe_wrap(h, hit))
        },
    );
    method(
        it,
        host,
        proto,
        "createTreeWalker",
        1,
        |_it, _h, _id, _args| Ok(Value::Null),
    );
}

fn args_node(v: Value) -> Option<NodeId> {
    crate::host::node_of(&v)
}
