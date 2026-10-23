//! `CSSStyleDeclaration` (`el.style`), `element.dataset` and the read-only
//! snapshot `window.getComputedStyle()` hands out.
//!
//! `el.style` is backed by the `style` attribute: reading `el.style.marginTop`
//! looks for a `margin-top` declaration and, if only the `margin` shorthand is
//! there, picks the right one of its 1–4 components — which is what the real
//! CSSOM does. Writing always rewrites the attribute, so CSS and DOM stay one
//! truth.

use std::rc::Rc;

use tb_css::parse_declarations;
use tb_dom::{camel_to_kebab, kebab_to_camel, Dom, NodeId};
use tb_js::value::{Key, ObjRef, Prop, Value};
use tb_js::{Interp, JsResult};

use crate::host::{method, ro_prop, rw_prop, Host, HostRef};

/// One declaration of a `style=""` attribute.
#[derive(Debug, Clone)]
struct Decl {
    name: String,
    value: String,
    important: bool,
}

fn parse_style_attr(dom: &Dom, id: NodeId) -> Vec<Decl> {
    let raw = match dom.attr(id, "style") {
        Some(s) => s,
        None => return Vec::new(),
    };
    parse_declarations(&raw)
        .into_iter()
        .map(|d| {
            let mut v = d.value.clone();
            let important = v.important || d.important;
            v.important = false;
            Decl {
                name: d.name,
                value: v.to_css(),
                important,
            }
        })
        .collect()
}

fn serialize(decls: &[Decl]) -> String {
    decls
        .iter()
        .map(|d| {
            if d.important {
                format!("{}: {} !important;", d.name, d.value)
            } else {
                format!("{}: {};", d.name, d.value)
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// `1 2` → sides `1 2 1 2`, `1 2 3` → `1 2 3 2`, …
fn side(value: &str, index: usize) -> Option<String> {
    let parts: Vec<&str> = value.split_whitespace().collect();
    if parts.is_empty() {
        return None;
    }
    let pick = match parts.len() {
        1 => [0, 0, 0, 0],
        2 => [0, 1, 0, 1],
        3 => [0, 1, 2, 1],
        _ => [0, 1, 2, 3],
    }[index];
    parts.get(pick).map(|s| s.to_string())
}

/// The longhands a shorthand can answer.
fn from_shorthand(decls: &[Decl], name: &str) -> Option<String> {
    let (shorthand, index) = match name {
        "margin-top" => ("margin", 0),
        "margin-right" => ("margin", 1),
        "margin-bottom" => ("margin", 2),
        "margin-left" => ("margin", 3),
        "padding-top" => ("padding", 0),
        "padding-right" => ("padding", 1),
        "padding-bottom" => ("padding", 2),
        "padding-left" => ("padding", 3),
        "inset-top" => ("inset", 0),
        "inset-right" => ("inset", 1),
        "inset-bottom" => ("inset", 2),
        "inset-left" => ("inset", 3),
        "border-top-width" => ("border-width", 0),
        "border-right-width" => ("border-width", 1),
        "border-bottom-width" => ("border-width", 2),
        "border-left-width" => ("border-width", 3),
        "border-top-style" => ("border-style", 0),
        "border-right-style" => ("border-style", 1),
        "border-bottom-style" => ("border-style", 2),
        "border-left-style" => ("border-style", 3),
        "border-top-color" => ("border-color", 0),
        "border-right-color" => ("border-color", 1),
        "border-bottom-color" => ("border-color", 2),
        "border-left-color" => ("border-color", 3),
        _ => return None,
    };
    decls
        .iter()
        .find(|d| d.name == shorthand)
        .and_then(|d| side(&d.value, index))
}

fn style_get(dom: &Dom, id: NodeId, name: &str) -> String {
    let decls = parse_style_attr(dom, id);
    if let Some(d) = decls.iter().find(|d| d.name == name) {
        return d.value.clone();
    }
    from_shorthand(&decls, name).unwrap_or_default()
}

fn style_set(dom: &mut Dom, id: NodeId, name: &str, value: &str, important: bool) {
    let mut decls = parse_style_attr(dom, id);
    if value.trim().is_empty() {
        decls.retain(|d| d.name != name);
    } else {
        match decls.iter_mut().find(|d| d.name == name) {
            Some(d) => {
                d.value = value.to_string();
                d.important = important;
            }
            None => decls.push(Decl {
                name: name.to_string(),
                value: value.to_string(),
                important,
            }),
        }
    }
    let text = serialize(&decls);
    if text.is_empty() {
        dom.remove_attr(id, "style");
    } else {
        dom.set_attr(id, "style", text);
    }
}

fn style_remove(dom: &mut Dom, id: NodeId, name: &str) -> String {
    let decls = parse_style_attr(dom, id);
    let old = decls
        .iter()
        .find(|d| d.name == name)
        .map(|d| d.value.clone())
        .unwrap_or_default();
    style_set(dom, id, name, "", false);
    old
}

/// The live `el.style` object.
pub fn style_object(_it: &mut Interp, h: &mut Host, id: NodeId) -> Value {
    let proto = h.protos.style_decl.clone();
    h.aux(id, "CSSStyleDeclaration", proto)
}

pub fn install_style(it: &mut Interp, host: &HostRef, proto: &ObjRef) {
    ro_prop(it, host, proto, "length", |it, h, id| {
        let _ = it;
        Ok(Value::Number(parse_style_attr(&h.dom, id).len() as f64))
    });
    rw_prop(
        it,
        host,
        proto,
        "cssText",
        |it, h, id, _| {
            let _ = it;
            Ok(Value::string(h.dom.attr(id, "style").unwrap_or_default()))
        },
        |it, h, id, v| {
            let text = it.to_string_val(&v)?;
            if text.trim().is_empty() {
                h.dom.remove_attr(id, "style");
            } else {
                // store verbatim: our own parser reads it back on demand
                h.dom.set_attr(id, "style", text);
            }
            h.touch();
            Ok(())
        },
    );
    method(it, host, proto, "getPropertyValue", 1, |it, h, id, args| {
        let name = prop_name(it, args.first())?;
        Ok(Value::string(style_get(&h.dom, id, &name)))
    });
    method(it, host, proto, "setProperty", 2, |it, h, id, args| {
        // `style.setProperty('--x', '1px')` too: custom properties pass through
        let name = prop_name(it, args.first())?;
        let value = args
            .get(1)
            .map(|v| it.to_string_val(v))
            .transpose()?
            .unwrap_or_default();
        let priority = args
            .get(2)
            .map(|v| it.to_string_val(v).unwrap_or_default())
            .unwrap_or_default();
        let important = priority == "important";
        style_set(&mut h.dom, id, &name, &value, important);
        h.touch();
        Ok(Value::Undefined)
    });
    method(it, host, proto, "removeProperty", 1, |it, h, id, args| {
        let name = prop_name(it, args.first())?;
        let old = style_remove(&mut h.dom, id, &name);
        h.touch();
        Ok(Value::string(old))
    });
    method(it, host, proto, "item", 1, |it, h, id, args| {
        let _ = it;
        let i = args.first().map(|v| v.to_integer()).unwrap_or(-1.0);
        let decls = parse_style_attr(&h.dom, id);
        if i < 0.0 || (i as usize) >= decls.len() {
            return Ok(Value::string(""));
        }
        Ok(Value::string(&decls[i as usize].name))
    });

    // one accessor per CSS property; the list is long but each entry is a
    // single line and missing one property only ever costs a fallback
    for camel in STYLE_PROPS {
        let kebab = camel_to_kebab(camel);
        let g_name = format!("style:{camel}");
        let getter = {
            let kebab = kebab.clone();
            let host2 = host.clone();
            let k = kebab.clone();
            it.native_fn(&g_name, 0, move |it: &mut Interp, this: Value, _| {
                let Some(id) = crate::host::node_of(&this) else {
                    return Ok(Value::string(""));
                };
                let h = host2
                    .try_borrow()
                    .map_err(|_| it.type_error("style: DOM is busy"))?;
                Ok(Value::string(style_get(&h.dom, id, &k)))
            })
        };
        let s_name = format!("style:{camel}");
        let setter = {
            let host2 = host.clone();
            let k = kebab.clone();
            it.native_fn(
                &s_name,
                1,
                move |it: &mut Interp, this: Value, a: Vec<Value>| {
                    let Some(id) = crate::host::node_of(&this) else {
                        return Ok(Value::Undefined);
                    };
                    let value = a
                        .first()
                        .map(|v| it.to_string_val(v))
                        .transpose()?
                        .unwrap_or_default();
                    let mut h = host2
                        .try_borrow_mut()
                        .map_err(|_| it.type_error("style: DOM is busy"))?;
                    let (value, important) = match value.rfind("!important") {
                        Some(i) => {
                            let (v, _) = value.split_at(i);
                            (v.trim().to_string(), true)
                        }
                        None => (value, false),
                    };
                    style_set(&mut h.dom, id, &k, &value, important);
                    h.touch();
                    Ok(Value::Undefined)
                },
            )
        };
        proto.borrow_mut().define(
            camel,
            Prop::accessor(
                Some(tb_js::builtins::obj_of(getter)),
                Some(tb_js::builtins::obj_of(setter)),
            ),
        );
    }
}

fn prop_name(it: &mut Interp, v: Option<&Value>) -> JsResult<String> {
    match v {
        None => Err(it.type_error("getPropertyValue: property name required")),
        Some(v) => {
            let s = it.to_string_val(v)?;
            // scripts mix camelCase and kebab-case; the attribute is kebab
            Ok(camel_to_kebab(&s))
        }
    }
}

// ---------------------------------------------------------------------------
// dataset
// ---------------------------------------------------------------------------

/// `element.dataset` — one property per `data-*` attribute.
///
/// [`tb_js`] has no `Proxy`, so this is a **synced mirror** instead of a set of
/// accessors: the object is cached per node, and on every access — plus at the
/// sync points in [`Host::flush_mirrors`] — changed attributes are pulled in
/// and changed (or deleted) properties are pushed back out. That means new keys
/// work like in a browser, `Object.keys()` lists exactly the data attributes,
/// and `el.dataset.x = 1` shows up in `el.getAttribute('data-x')` no later than
/// the next script, timer or event.
pub fn dataset(h: &mut Host, id: NodeId) -> Value {
    let obj = h.aux(id, "dataset", None);
    if let Some(o) = obj.as_object().cloned() {
        if sync_dataset(&mut h.dom, id, &o) {
            h.touch();
        }
    }
    obj
}

/// The `data-*` name of a `dataset` key: `fooBar` → `data-foo-bar`.
fn attr_of(name: &str) -> String {
    format!("data-{}", camel_to_kebab(name))
}

/// The `dataset` key of a `data-*` attribute: `data-foo-bar` → `fooBar`.
fn dataset_key(attr: &str) -> String {
    kebab_to_camel(attr.strip_prefix("data-").unwrap_or(attr))
}

/// Marker property remembering the value a mirror last agreed on with the DOM.
const DS_MARK: &str = "@@ds:";

/// Bring one `dataset` mirror and the `data-*` attributes back in agreement.
/// Returns `true` when an attribute changed.
/// What a `dataset` mirror holds: its own keys, and the `@@ds:` markers that
/// say which side wrote them last.
type Mirrors = (Vec<(String, Value)>, Vec<(String, Value)>);

pub fn sync_dataset(dom: &mut Dom, id: NodeId, obj: &ObjRef) -> bool {
    // What the page sees right now, and what we last agreed on.
    let (props, marks): Mirrors = {
        let b = obj.borrow();
        let mut props = Vec::new();
        let mut marks = Vec::new();
        for (k, p) in &b.props {
            let Key::Name(n) = k else { continue };
            if let Some(rest) = n.strip_prefix(DS_MARK) {
                if let Some(v) = p.value() {
                    marks.push((rest.to_string(), v.clone()));
                }
            } else if !n.starts_with("@@") {
                if let Some(v) = p.value() {
                    props.push((n.to_string(), v.clone()));
                }
            }
        }
        (props, marks)
    };
    let mark_of = |name: &str| {
        marks
            .iter()
            .find(|(m, _)| m == name)
            .map(|(_, v)| v.clone())
    };
    let mut changed = false;

    // 1. properties the page wrote or changed → attributes
    for (name, value) in &props {
        if mark_of(name).is_some_and(|m| m.to_string() == value.to_string()) {
            continue;
        }
        let attr = attr_of(name);
        if value.is_nullish() {
            dom.remove_attr(id, &attr);
        } else {
            dom.set_attr(id, &attr, value.to_string());
        }
        obj.borrow_mut()
            .define_internal(&format!("{DS_MARK}{name}"), value.clone());
        changed = true;
    }
    // 2. properties the page deleted → the attribute goes away
    for (name, _) in &marks {
        if props.iter().any(|(n, _)| n == name) {
            continue;
        }
        dom.remove_attr(id, &attr_of(name));
        obj.borrow_mut()
            .delete(&Key::Name(Rc::from(format!("{DS_MARK}{name}"))));
        changed = true;
    }

    // 3. attributes → properties: new, changed, and removed ones
    let attrs: Vec<(String, String)> = dom
        .element(id)
        .map(|e| {
            e.attrs
                .iter()
                .filter(|a| a.name.starts_with("data-"))
                .map(|a| (a.name.clone(), a.value.clone()))
                .collect()
        })
        .unwrap_or_default();
    for (attr, value) in attrs {
        let key = dataset_key(&attr);
        if mark_of(&key).is_some_and(|m| m.to_string() == value) {
            continue;
        }
        let mut b = obj.borrow_mut();
        b.define(&key, Prop::data(Value::string(&value)));
        b.define_internal(&format!("{DS_MARK}{key}"), Value::string(&value));
    }
    for (name, _) in &marks {
        if dom.attr(id, &attr_of(name)).is_none() {
            let mut b = obj.borrow_mut();
            b.delete(&Key::Name(Rc::from(name.as_str())));
            b.delete(&Key::Name(Rc::from(format!("{DS_MARK}{name}"))));
        }
    }
    changed
}

// ---------------------------------------------------------------------------
// getComputedStyle
// ---------------------------------------------------------------------------

/// A snapshot of the *computed* style: camelCase properties plus
/// `getPropertyValue('kebab-case')`.
/// Computed colours serialise as `rgb(…)`, not as the hex the author wrote —
/// `getComputedStyle(el).color` never gives `#ff0000`.
fn color_css(c: tb_core::Color) -> String {
    if c.a == 255 {
        format!("rgb({}, {}, {})", c.r, c.g, c.b)
    } else {
        let a = c.a as f64 / 255.0;
        let a = (a * 1000.0).round() / 1000.0;
        format!("rgba({}, {}, {}, {})", c.r, c.g, c.b, a)
    }
}

pub fn computed_object(it: &mut Interp, h: &mut Host, id: NodeId) -> Value {
    use tb_style::{ComputedStyle, L};
    let styles = h.style_map();
    let cs: tb_style::ComputedStyle = styles
        .get(id)
        .cloned()
        .unwrap_or_else(ComputedStyle::initial);
    let rect = h.rect_of(id);
    let resolved = |l: &L| -> String {
        match l {
            L::Auto => "auto".to_string(),
            L::Px(v) => format!("{}px", v.round()),
            L::Linear { pct, fixed } => format!("{}px", (pct + fixed).round()),
        }
    };
    let sides =
        |arr: &[L; 4]| -> Vec<String> { [0, 1, 2, 3].iter().map(|i| resolved(&arr[*i])).collect() };
    let mut out: Vec<(String, String)> = Vec::new();
    let mut add = |name: &str, value: String| out.push((name.to_string(), value));

    add("display", display_css(cs.display).into());
    add("position", position_css(cs.position).into());
    add("color", color_css(cs.color));
    add("background-color", color_css(cs.background_color));
    add(
        "background-image",
        if cs.background_image {
            "url(...)".into()
        } else {
            "none".into()
        },
    );
    add("opacity", num(cs.opacity));
    add(
        "visibility",
        if cs.visibility { "visible" } else { "hidden" }.into(),
    );
    add("font-family", cs.font_family.join(", "));
    add("font-size", format!("{}px", cs.font_size));
    add("font-weight", cs.font_weight.to_string());
    add(
        "font-style",
        if cs.italic { "italic" } else { "normal" }.into(),
    );
    add(
        "line-height",
        if cs.line_height > 0.0 {
            format!("{}px", cs.line_height)
        } else {
            "normal".into()
        },
    );
    add("text-align", text_align_css(cs.text_align).into());
    add(
        "text-decoration-line",
        if cs.text_decoration {
            "underline"
        } else {
            "none"
        }
        .into(),
    );
    add(
        "text-transform",
        text_transform_css(cs.text_transform).into(),
    );
    add("white-space", white_space_css(cs.white_space).into());
    add("letter-spacing", px_or_normal(cs.letter_spacing));
    add("word-spacing", px_or_normal(cs.word_spacing));
    add(
        "box-sizing",
        if cs.box_sizing_border {
            "border-box"
        } else {
            "content-box"
        }
        .into(),
    );
    add(
        "width",
        rect.map(|r| format!("{}px", r.width.round()))
            .unwrap_or_else(|| resolved(&cs.width)),
    );
    add(
        "height",
        rect.map(|r| format!("{}px", r.height.round()))
            .unwrap_or_else(|| resolved(&cs.height)),
    );
    add("min-width", resolved(&cs.min_width));
    add("max-width", resolved(&cs.max_width));
    add("min-height", resolved(&cs.min_height));
    add("max-height", resolved(&cs.max_height));
    let m = sides(&cs.margin);
    add("margin-top", m[0].clone());
    add("margin-right", m[1].clone());
    add("margin-bottom", m[2].clone());
    add("margin-left", m[3].clone());
    let p = sides(&cs.padding);
    add("padding-top", p[0].clone());
    add("padding-right", p[1].clone());
    add("padding-bottom", p[2].clone());
    add("padding-left", p[3].clone());
    add("border-top-width", format!("{}px", cs.border_width[0]));
    add("border-right-width", format!("{}px", cs.border_width[1]));
    add("border-bottom-width", format!("{}px", cs.border_width[2]));
    add("border-left-width", format!("{}px", cs.border_width[3]));
    add("border-radius", format!("{}px", cs.border_radius[0]));
    add("overflow-x", overflow_css(cs.overflow_x).into());
    add("overflow-y", overflow_css(cs.overflow_y).into());
    add(
        "flex-direction",
        flex_direction_css(cs.flex_direction).into(),
    );
    add(
        "flex-wrap",
        if cs.flex_wrap { "wrap" } else { "nowrap" }.into(),
    );
    add("flex-grow", num(cs.flex_grow));
    add("flex-shrink", num(cs.flex_shrink));
    add("flex-basis", resolved(&cs.flex_basis));
    add("justify-content", justify_css(cs.justify_content).into());
    add("align-items", align_css(cs.align_items).into());
    add("row-gap", format!("{}px", cs.row_gap));
    add("column-gap", format!("{}px", cs.column_gap));
    add(
        "z-index",
        cs.z_index
            .map(|z| z.to_string())
            .unwrap_or_else(|| "auto".into()),
    );
    match cs.inset {
        [t, r, b, l] => {
            add("top", resolved(&t));
            add("right", resolved(&r));
            add("bottom", resolved(&b));
            add("left", resolved(&l));
        }
    }

    let obj = it.plain_object();
    let table: Rc<Vec<(String, String)>> = Rc::new(out.clone());
    for (kebab, value) in &out {
        let _ = it.set(
            &obj,
            &Key::Name(Rc::from(kebab.as_str())),
            Value::string(value),
        );
        let camel = kebab_to_camel(kebab);
        let _ = it.set(
            &obj,
            &Key::Name(Rc::from(camel.as_str())),
            Value::string(value),
        );
    }
    let get = {
        let table = table.clone();
        it.native_fn(
            "getPropertyValue",
            1,
            move |it: &mut Interp, _this: Value, a: Vec<Value>| {
                let want = a
                    .first()
                    .map(|v| camel_to_kebab(&it.to_string_val(v).unwrap_or_default()))
                    .unwrap_or_default();
                Ok(Value::string(
                    table
                        .iter()
                        .find(|(k, _)| *k == want)
                        .map(|(_, v)| v.as_str())
                        .unwrap_or(""),
                ))
            },
        )
    };
    let item = {
        let table = table.clone();
        it.native_fn(
            "item",
            1,
            move |_it: &mut Interp, _this: Value, a: Vec<Value>| {
                let i = a.first().map(|v| v.to_integer()).unwrap_or(-1.0);
                if i < 0.0 || (i as usize) >= table.len() {
                    return Ok(Value::string(""));
                }
                Ok(Value::string(&table[i as usize].0))
            },
        )
    };
    let priority = it.native_fn(
        "getPropertyPriority",
        1,
        |_it: &mut Interp, _this: Value, _| Ok(Value::string("")),
    );
    {
        let target = obj.as_object().unwrap().clone();
        let mut b = target.borrow_mut();
        b.define_internal("getPropertyValue", get);
        b.define_internal("item", item);
        b.define_internal("getPropertyPriority", priority);
    }
    obj
}

// ---------------------------------------------------------------------------
// enum → CSS
// ---------------------------------------------------------------------------

fn display_css(d: tb_style::Display) -> &'static str {
    use tb_style::Display::*;
    match d {
        None => "none",
        Inline => "inline",
        Block => "block",
        InlineBlock => "inline-block",
        Flex => "flex",
        InlineFlex => "inline-flex",
        ListItem => "list-item",
        Table => "table",
        TableRowGroup => "table-row-group",
        TableRow => "table-row",
        TableCell => "table-cell",
        Contents => "contents",
    }
}

fn position_css(p: tb_style::Position) -> &'static str {
    use tb_style::Position::*;
    match p {
        Static => "static",
        Relative => "relative",
        Absolute => "absolute",
        Fixed => "fixed",
        Sticky => "sticky",
    }
}

fn text_align_css(a: tb_style::TextAlign) -> &'static str {
    use tb_style::TextAlign::*;
    match a {
        Left => "left",
        Right => "right",
        Center => "center",
        Justify => "justify",
    }
}

fn text_transform_css(t: tb_style::TextTransform) -> &'static str {
    use tb_style::TextTransform::*;
    match t {
        None => "none",
        Uppercase => "uppercase",
        Lowercase => "lowercase",
        Capitalize => "capitalize",
    }
}

fn white_space_css(w: tb_style::WhiteSpace) -> &'static str {
    use tb_style::WhiteSpace::*;
    match w {
        Normal => "normal",
        Nowrap => "nowrap",
        Pre => "pre",
        PreWrap => "pre-wrap",
        BreakSpaces => "break-spaces",
    }
}

fn overflow_css(o: tb_style::Overflow) -> &'static str {
    use tb_style::Overflow::*;
    match o {
        Visible => "visible",
        Hidden => "hidden",
        Scroll => "scroll",
        Auto => "auto",
    }
}

fn flex_direction_css(d: tb_style::FlexDirection) -> &'static str {
    use tb_style::FlexDirection::*;
    match d {
        Row => "row",
        RowReverse => "row-reverse",
        Column => "column",
        ColumnReverse => "column-reverse",
    }
}

fn justify_css(j: tb_style::JustifyContent) -> &'static str {
    use tb_style::JustifyContent::*;
    match j {
        FlexStart => "flex-start",
        FlexEnd => "flex-end",
        Center => "center",
        SpaceBetween => "space-between",
        SpaceAround => "space-around",
        SpaceEvenly => "space-evenly",
    }
}

fn align_css(a: tb_style::AlignItems) -> &'static str {
    use tb_style::AlignItems::*;
    match a {
        Stretch => "stretch",
        FlexStart => "flex-start",
        FlexEnd => "flex-end",
        Center => "center",
        Baseline => "baseline",
    }
}

/// `1` not `1.0`, like CSS writes it.
fn num(v: f32) -> String {
    if v == v.round() {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

fn px_or_normal(v: f32) -> String {
    if v == 0.0 {
        "normal".into()
    } else {
        format!("{}px", v)
    }
}

/// The CSS properties `el.style` exposes. Anything else still works through
/// `getPropertyValue`/`setProperty`.
pub const STYLE_PROPS: &[&str] = &[
    "alignContent",
    "alignItems",
    "alignSelf",
    "animation",
    "animationDelay",
    "animationDuration",
    "animationName",
    "backfaceVisibility",
    "background",
    "backgroundColor",
    "backgroundImage",
    "backgroundPosition",
    "backgroundRepeat",
    "backgroundSize",
    "border",
    "borderBottom",
    "borderBottomColor",
    "borderBottomLeftRadius",
    "borderBottomRightRadius",
    "borderBottomStyle",
    "borderBottomWidth",
    "borderCollapse",
    "borderColor",
    "borderLeft",
    "borderLeftColor",
    "borderLeftStyle",
    "borderLeftWidth",
    "borderRadius",
    "borderRight",
    "borderRightColor",
    "borderRightStyle",
    "borderRightWidth",
    "borderSpacing",
    "borderStyle",
    "borderTop",
    "borderTopColor",
    "borderTopLeftRadius",
    "borderTopRightRadius",
    "borderTopStyle",
    "borderTopWidth",
    "borderWidth",
    "bottom",
    "boxShadow",
    "boxSizing",
    "clear",
    "clip",
    "color",
    "columnCount",
    "content",
    "cursor",
    "direction",
    "display",
    "flex",
    "flexBasis",
    "flexDirection",
    "flexFlow",
    "flexGrow",
    "flexShrink",
    "flexWrap",
    "float",
    "font",
    "fontFamily",
    "fontSize",
    "fontStyle",
    "fontVariant",
    "fontWeight",
    "gap",
    "height",
    "inset",
    "justifyContent",
    "left",
    "letterSpacing",
    "lineHeight",
    "listStyle",
    "listStyleType",
    "margin",
    "marginBottom",
    "marginLeft",
    "marginRight",
    "marginTop",
    "maxHeight",
    "maxWidth",
    "minHeight",
    "minWidth",
    "objectFit",
    "opacity",
    "order",
    "outline",
    "overflow",
    "overflowX",
    "overflowY",
    "padding",
    "paddingBottom",
    "paddingLeft",
    "paddingRight",
    "paddingTop",
    "pointerEvents",
    "position",
    "resize",
    "right",
    "rowGap",
    "scrollBehavior",
    "textAlign",
    "textDecoration",
    "textIndent",
    "textOverflow",
    "textTransform",
    "top",
    "transform",
    "transformOrigin",
    "transition",
    "userSelect",
    "verticalAlign",
    "visibility",
    "whiteSpace",
    "width",
    "wordBreak",
    "wordSpacing",
    "zIndex",
];
