//! `Node.prototype` — the parts every node shares.
//!
//! All of it lives on the prototype: the accessors read the hidden `"@@node"`
//! off `this`, so 3 000 elements cost 3 000 small objects and *no* closures.
//!
//! Binding closures always name their interpreter parameter `it` — the outer
//! `it` must never be captured (the closure outlives the install call).

use tb_dom::{Dom, NodeId, NodeKind};
use tb_js::value::{ObjRef, Value};
use tb_js::{Interp, JsResult};

use crate::host::{method, node_list, node_of, ro_prop, rw_prop, Host, HostRef};

/// `nodeType` as a number (9 document, 3 text, 8 comment, 1 element).
pub fn node_type(dom: &Dom, id: NodeId) -> f64 {
    match dom.get(id).map(|n| &n.kind) {
        Some(NodeKind::Document) => 9.0,
        Some(NodeKind::Doctype { .. }) => 10.0,
        Some(NodeKind::Text(_)) => 3.0,
        Some(NodeKind::Comment(_)) => 8.0,
        Some(NodeKind::Element(_)) => 1.0,
        Some(NodeKind::ProcessingInstruction { .. }) => 7.0,
        None => 0.0,
    }
}

/// `nodeName` (`"#text"`, `"DIV"`, …).
pub fn node_name(dom: &Dom, id: NodeId) -> String {
    match dom.get(id).map(|n| &n.kind) {
        Some(NodeKind::Document) => "#document".into(),
        Some(NodeKind::Doctype { name, .. }) => name.clone(),
        Some(NodeKind::Text(_)) => "#text".into(),
        Some(NodeKind::Comment(_)) => "#comment".into(),
        Some(NodeKind::Element(e)) => e.tag.to_ascii_uppercase(),
        Some(NodeKind::ProcessingInstruction { target, .. }) => target.clone(),
        None => String::new(),
    }
}

/// A node id as JS: the wrapper object, or `null`.
pub fn maybe_wrap(h: &mut Host, id: Option<NodeId>) -> Value {
    match id {
        Some(i) if h.dom.exists(i) => h.wrap(i),
        _ => Value::Null,
    }
}

/// An argument that has to be a node — anything else is a `TypeError`.
pub fn node_arg(it: &mut Interp, args: &[Value], i: usize, what: &str) -> JsResult<NodeId> {
    args.get(i)
        .and_then(node_of)
        .ok_or_else(|| it.type_error(&format!("{what}: argument {i} is not a node")))
}

/// `<template>`-style fragment: an element whose children get moved out.
pub fn is_fragment(dom: &Dom, id: NodeId) -> bool {
    dom.tag(id) == Some("#fragment")
}

/// The nodes `append(…)`/`before(…)`/`replaceChildren(…)` insert: strings
/// become text nodes, a fragment contributes its children.
fn insertable(h: &mut Host, args: &[Value]) -> Vec<NodeId> {
    let mut out = Vec::new();
    for v in args {
        if let Value::Str(s) = v {
            out.push(h.dom.create_text(s));
            continue;
        }
        let Some(c) = node_of(v) else { continue };
        if is_fragment(&h.dom, c) {
            out.extend(h.dom.children(c));
        } else {
            out.push(c);
        }
    }
    out
}

pub fn install(it: &mut Interp, host: &HostRef, proto: &ObjRef) {
    ro_prop(it, host, proto, "nodeType", |it, h, id| {
        let _ = it;
        Ok(Value::Number(node_type(&h.dom, id)))
    });
    ro_prop(it, host, proto, "nodeName", |it, h, id| {
        let _ = it;
        Ok(Value::string(node_name(&h.dom, id)))
    });
    rw_prop(
        it,
        host,
        proto,
        "nodeValue",
        |it, h, id, _| {
            let _ = it;
            Ok(match h.dom.get(id).map(|n| &n.kind) {
                Some(NodeKind::Text(t)) | Some(NodeKind::Comment(t)) => Value::string(t.clone()),
                _ => Value::Null,
            })
        },
        |it, h, id, v| {
            let s = if v.is_nullish() {
                String::new()
            } else {
                it.to_string_val(&v)?
            };
            if let Some(n) = h.dom.get_mut(id) {
                match &mut n.kind {
                    NodeKind::Text(t) | NodeKind::Comment(t) => *t = s,
                    _ => {}
                }
            }
            h.touch();
            Ok(())
        },
    );
    rw_prop(
        it,
        host,
        proto,
        "textContent",
        |it, h, id, _| {
            let _ = it;
            Ok(Value::string(h.dom.text_content(id)))
        },
        |it, h, id, v| {
            if v.is_nullish() {
                h.dom.clear_children(id);
            } else {
                let s = it.to_string_val(&v)?;
                h.dom.set_text_content(id, &s);
            }
            h.touch();
            Ok(())
        },
    );

    // --------------------------------------------------------------- tree ----
    ro_prop(it, host, proto, "parentNode", |it, h, id| {
        let _ = it;
        let p = h.dom.parent(id);
        Ok(maybe_wrap(h, p))
    });
    ro_prop(it, host, proto, "parentElement", |it, h, id| {
        let _ = it;
        let p = h.dom.parent(id).filter(|p| h.dom.tag(*p).is_some());
        Ok(maybe_wrap(h, p))
    });
    ro_prop(it, host, proto, "childNodes", |it, h, id| {
        let kids = h.dom.children(id);
        Ok(node_list(it, h, kids))
    });
    ro_prop(it, host, proto, "children", |it, h, id| {
        let kids = h.dom.child_elements(id);
        Ok(node_list(it, h, kids))
    });
    ro_prop(it, host, proto, "firstChild", |it, h, id| {
        let _ = it;
        let c = h.dom.first_child(id);
        Ok(maybe_wrap(h, c))
    });
    ro_prop(it, host, proto, "lastChild", |it, h, id| {
        let _ = it;
        let c = h.dom.last_child(id);
        Ok(maybe_wrap(h, c))
    });
    ro_prop(it, host, proto, "nextSibling", |it, h, id| {
        let _ = it;
        let c = h.dom.next_sibling(id);
        Ok(maybe_wrap(h, c))
    });
    ro_prop(it, host, proto, "previousSibling", |it, h, id| {
        let _ = it;
        let c = h.dom.prev_sibling(id);
        Ok(maybe_wrap(h, c))
    });
    ro_prop(it, host, proto, "firstElementChild", |it, h, id| {
        let _ = it;
        let c = h.dom.child_elements(id).into_iter().next();
        Ok(maybe_wrap(h, c))
    });
    ro_prop(it, host, proto, "lastElementChild", |it, h, id| {
        let _ = it;
        let c = h.dom.child_elements(id).into_iter().next_back();
        Ok(maybe_wrap(h, c))
    });
    ro_prop(it, host, proto, "nextElementSibling", |it, h, id| {
        let _ = it;
        let c = h.dom.next_element_sibling(id);
        Ok(maybe_wrap(h, c))
    });
    ro_prop(it, host, proto, "previousElementSibling", |it, h, id| {
        let _ = it;
        let c = h.dom.previous_element_sibling(id);
        Ok(maybe_wrap(h, c))
    });
    ro_prop(it, host, proto, "childElementCount", |it, h, id| {
        let _ = it;
        Ok(Value::Number(h.dom.child_elements(id).len() as f64))
    });
    ro_prop(it, host, proto, "ownerDocument", |it, h, id| {
        let _ = it;
        Ok(if id == h.dom.document() {
            Value::Null
        } else {
            let d = h.dom.document();
            h.wrap(d)
        })
    });
    ro_prop(it, host, proto, "isConnected", |it, h, id| {
        let _ = it;
        Ok(Value::Boolean(
            h.dom.get(id).map(|n| n.attached).unwrap_or(false),
        ))
    });
    ro_prop(it, host, proto, "baseURI", |it, h, _id| {
        let _ = it;
        Ok(Value::string(h.url_string()))
    });
    ro_prop(it, host, proto, "namespaceURI", |it, h, id| {
        let _ = it;
        Ok(match h.dom.element(id).map(|e| e.namespace) {
            Some(tb_dom::Namespace::Svg) => Value::string("http://www.w3.org/2000/svg"),
            Some(tb_dom::Namespace::MathMl) => Value::string("http://www.w3.org/1998/Math/MathML"),
            _ => Value::string("http://www.w3.org/1999/xhtml"),
        })
    });

    // ------------------------------------------------------------ mutation ---
    method(it, host, proto, "appendChild", 1, |it, h, id, args| {
        let child = node_arg(it, &args, 0, "appendChild")?;
        if is_fragment(&h.dom, child) {
            for k in h.dom.children(child) {
                h.dom.appendChild(id, k);
            }
        } else {
            h.dom.appendChild(id, child);
        }
        h.touch();
        Ok(args[0].clone())
    });
    method(it, host, proto, "insertBefore", 2, |it, h, id, args| {
        let child = node_arg(it, &args, 0, "insertBefore")?;
        let reference = args.get(1).and_then(node_of);
        if is_fragment(&h.dom, child) {
            for k in h.dom.children(child) {
                h.dom.insertBefore(id, k, reference);
            }
        } else {
            h.dom.insertBefore(id, child, reference);
        }
        h.touch();
        Ok(args[0].clone())
    });
    method(it, host, proto, "removeChild", 1, |it, h, _id, args| {
        let child = node_arg(it, &args, 0, "removeChild")?;
        h.dom.detach(child);
        h.touch();
        Ok(args[0].clone())
    });
    method(it, host, proto, "replaceChild", 2, |it, h, id, args| {
        let new = node_arg(it, &args, 0, "replaceChild")?;
        let old = node_arg(it, &args, 1, "replaceChild")?;
        h.dom.replace_child(id, new, old);
        h.touch();
        Ok(args[1].clone())
    });
    method(it, host, proto, "remove", 0, |_it, h, id, _args| {
        h.dom.detach(id);
        h.touch();
        Ok(h.wrap(id))
    });
    method(it, host, proto, "cloneNode", 1, |_it, h, id, args| {
        let deep = args.first().map(|v| v.to_boolean()).unwrap_or(false);
        let copy = h.dom.clone_node(id, deep);
        h.touch();
        Ok(h.wrap(copy))
    });
    method(it, host, proto, "contains", 1, |it, h, id, args| {
        let _ = it;
        let other = args.first().and_then(node_of);
        // a node contains itself
        Ok(Value::Boolean(match other {
            Some(o) => o == id || h.dom.contains(id, o),
            None => false,
        }))
    });
    method(it, host, proto, "isEqualNode", 1, |it, h, id, args| {
        let Ok(other) = node_arg(it, &args, 0, "isEqualNode") else {
            return Ok(Value::Boolean(false));
        };
        let same_kind = match (h.dom.get(id), h.dom.get(other)) {
            (Some(a), Some(b)) => {
                std::mem::discriminant(&a.kind) == std::mem::discriminant(&b.kind)
            }
            _ => false,
        };
        Ok(Value::Boolean(
            same_kind && h.dom.outer_html(id) == h.dom.outer_html(other),
        ))
    });
    method(it, host, proto, "hasChildNodes", 0, |_it, h, id, _args| {
        Ok(Value::Boolean(!h.dom.children(id).is_empty()))
    });
    method(it, host, proto, "normalize", 0, |_it, h, id, _args| {
        h.dom.normalize(id);
        h.touch();
        Ok(Value::Undefined)
    });
    method(it, host, proto, "getRootNode", 0, |_it, h, _id, _args| {
        let d = h.dom.document();
        Ok(h.wrap(d))
    });
    method(it, host, proto, "replaceChildren", 0, |_it, h, id, args| {
        // collect the newcomers first: `clear_children` must not free a node
        // that was handed in as a replacement
        let kids = insertable(h, &args);
        let olds = h.dom.children(id);
        for c in &kids {
            h.dom.appendChild(id, *c);
        }
        for o in olds {
            if !kids.contains(&o) {
                h.dom.detach(o);
            }
        }
        h.touch();
        Ok(Value::Undefined)
    });
    method(it, host, proto, "replaceWith", 1, |_it, h, id, args| {
        let Some(parent) = h.dom.parent(id) else {
            return Ok(Value::Undefined);
        };
        let kids = insertable(h, &args);
        for c in kids {
            h.dom.insertBefore(parent, c, Some(id));
        }
        h.dom.detach(id);
        h.touch();
        Ok(Value::Undefined)
    });
    method(it, host, proto, "before", 1, |_it, h, id, args| {
        let Some(parent) = h.dom.parent(id) else {
            return Ok(Value::Undefined);
        };
        let kids = insertable(h, &args);
        for c in kids {
            h.dom.insertBefore(parent, c, Some(id));
        }
        h.touch();
        Ok(Value::Undefined)
    });
    method(it, host, proto, "after", 1, |_it, h, id, args| {
        let Some(parent) = h.dom.parent(id) else {
            return Ok(Value::Undefined);
        };
        let kids = insertable(h, &args);
        let next = h.dom.next_sibling(id);
        for c in kids {
            h.dom.insertBefore(parent, c, next);
        }
        h.touch();
        Ok(Value::Undefined)
    });
    method(it, host, proto, "prepend", 1, |_it, h, id, args| {
        let kids = insertable(h, &args);
        let first = h.dom.first_child(id);
        for c in kids {
            h.dom.insertBefore(id, c, first);
        }
        h.touch();
        Ok(Value::Undefined)
    });
    method(it, host, proto, "append", 1, |_it, h, id, args| {
        let kids = insertable(h, &args);
        for c in kids {
            h.dom.appendChild(id, c);
        }
        h.touch();
        Ok(Value::Undefined)
    });
    method(it, host, proto, "lookupPrefix", 1, |_it, _h, _id, _args| {
        Ok(Value::Null)
    });
}
