//! The cascade: rule index, matching, specificity and `ComputedStyle` building.
//!
//! For every element (pre-order, so the parent is already computed) we collect
//! the declarations of all matching rules, order them by
//! *(importance, origin, specificity, source order)* and interpret them into a
//! [`ComputedStyle`].

use crate::computed::*;
use crate::matcher::{matches, MatchState};
use crate::ua::ua_stylesheet;
use std::collections::HashMap;
use std::rc::Rc;
use tb_core::{Color, Problems};
use tb_css::ast::{Declaration, Rule, StyleSheet};
use tb_css::calc::Resolved;
use tb_css::media::Viewport;
use tb_css::selector::{ComplexSelector, SelectorPart, Simple};
use tb_css::value::DeclarationValue;
use tb_css::value::{parse_value, Unit, Value};
use tb_dom::{Dom, NodeId};

/// Where a declaration came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Origin {
    UserAgent,
    Author,
    /// The `style=""` attribute: author origin, but above every selector.
    AuthorAttr,
}

impl Origin {
    /// Cascade rank (low → high) — `!important` flips the origin order.
    fn rank(self, important: bool) -> u8 {
        match (self, important) {
            (Origin::UserAgent, false) => 1,
            (Origin::Author, false) => 2,
            (Origin::AuthorAttr, false) => 3,
            (Origin::Author, true) => 4,
            (Origin::AuthorAttr, true) => 5,
            (Origin::UserAgent, true) => 6,
        }
    }
}

/// Parsed `style=""` attributes, keyed by node index.
#[derive(Debug, Clone, Default)]
pub struct AttrSheets {
    map: std::collections::HashMap<usize, Vec<Declaration>>,
}

impl AttrSheets {
    /// Parse every `style` attribute in the document once.
    pub fn from_dom(dom: &tb_dom::Dom) -> AttrSheets {
        let mut map = std::collections::HashMap::new();
        for node in dom.descendants(dom.document()) {
            let Some(el) = dom.element(node) else {
                continue;
            };
            let Some(raw) = el.attr("style") else {
                continue;
            };
            if raw.trim().is_empty() {
                continue;
            }
            let decls = tb_css::parse_declarations(raw);
            if !decls.is_empty() {
                map.insert(node.idx_usize(), decls);
            }
        }
        AttrSheets { map }
    }

    pub fn get(&self, id: NodeId) -> Option<&[Declaration]> {
        self.map.get(&id.idx_usize()).map(|v| v.as_slice())
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

/// A rule with its conditions already evaluated.
struct FlatRule<'a> {
    selector: &'a ComplexSelector,
    decls: &'a [Declaration],
    origin: Origin,
    order: usize,
    /// `::before` / `::after`; pseudo-element rules are applied separately.
    pseudo: Option<&'a str>,
}

/// Bucket key of a rule (right-most compound).
#[derive(Hash, PartialEq, Eq, Clone)]
enum Key {
    Id(String),
    Class(String),
    Type(String),
    Universal,
}

/// Winning declaration candidate for one property of one element.
#[derive(Clone, Copy)]
struct Candidate<'a> {
    decl: &'a Declaration,
    specificity: (u32, u32, u32),
    origin: Origin,
    order: usize,
}

impl<'a> Candidate<'a> {
    fn wins_over(&self, other: &Candidate<'a>) -> bool {
        (
            self.origin.rank(self.decl.important),
            self.specificity,
            self.order,
        ) >= (
            other.origin.rank(other.decl.important),
            other.specificity,
            other.order,
        )
    }

    /// First value part (`color: red`).
    fn one(&self) -> Option<&'a Value> {
        self.decl.value.parts.first()
    }

    /// The whole declaration value as CSS text (shorthands, var substitution).
    fn text(&self) -> String {
        self.decl.value.to_css()
    }
}

/// Computed styles keyed by node index.
#[derive(Debug, Clone, Default)]
pub struct StyleMap {
    entries: Vec<Option<Rc<ComputedStyle>>>,
}

impl StyleMap {
    pub fn get(&self, id: NodeId) -> Option<&ComputedStyle> {
        self.entries
            .get(id.idx_usize())
            .and_then(|o| o.as_ref())
            .map(|r| &**r)
    }

    /// Shared handle — layout boxes keep the `Rc` instead of cloning the style.
    pub fn rc(&self, id: NodeId) -> Option<Rc<ComputedStyle>> {
        self.entries.get(id.idx_usize()).and_then(|o| o.clone())
    }

    /// The computed style, or a neutral default for unstyled nodes.
    pub fn get_or_default(&self, id: NodeId) -> ComputedStyle {
        self.get(id).cloned().unwrap_or_else(ComputedStyle::initial)
    }

    pub fn count(&self) -> usize {
        self.entries.iter().filter(|e| e.is_some()).count()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.iter().all(|e| e.is_none())
    }
}

/// Result of the style pass.
#[derive(Debug, Clone)]
pub struct Styled {
    pub styles: StyleMap,
    pub problems: Problems,
    /// Elements with at least one matching rule.
    pub matched_elements: usize,
    /// Total eligible rules after media evaluation.
    pub rule_count: usize,
}

/// Compute the style of every element (parses `style` attributes itself).
pub fn compute(
    dom: &Dom,
    author_sheets: &[StyleSheet],
    viewport: Viewport,
    state: &MatchState,
) -> Styled {
    compute_with_attrs(
        dom,
        author_sheets,
        viewport,
        state,
        &AttrSheets::from_dom(dom),
    )
}

/// Compute the style of every element.
pub fn compute_with_attrs(
    dom: &Dom,
    author_sheets: &[StyleSheet],
    viewport: Viewport,
    state: &MatchState,
    attrs: &AttrSheets,
) -> Styled {
    let problems = Problems::new();

    // 1. flatten rules; @media evaluated once per document
    let mut flat: Vec<FlatRule<'_>> = Vec::new();
    let mut order = 0usize;
    let ua = ua_stylesheet();
    collect_rules(
        &ua.rules,
        viewport,
        Origin::UserAgent,
        &mut order,
        &mut flat,
    );
    for sheet in author_sheets {
        collect_rules(
            &sheet.rules,
            viewport,
            Origin::Author,
            &mut order,
            &mut flat,
        );
    }

    // 2. index by right-most compound key
    let mut index: HashMap<Key, Vec<usize>> = HashMap::new();
    for (i, r) in flat.iter().enumerate() {
        if r.pseudo.is_some() {
            continue;
        }
        if let Some(key) = rule_key(r.selector) {
            index.entry(key).or_default().push(i);
        }
    }
    // `::before`/`::after`: strip the pseudo part once, not once per element
    let mut pseudo_rules: Vec<(ComplexSelector, &str, &[Declaration])> = Vec::new();
    for r in flat.iter() {
        if let Some(p) = r.pseudo {
            if let Some(sel) = strip_pseudo(r.selector) {
                pseudo_rules.push((sel, p, r.decls));
            }
        }
    }

    // 3. walk the DOM pre-order
    let mut entries: Vec<Option<Rc<ComputedStyle>>> = vec![None; dom.max_index()];
    let mut custom_cache: HashMap<usize, HashMap<String, String>> = HashMap::new();
    let mut matched_elements = 0usize;

    for node in dom.descendants(dom.document()) {
        if dom.tag(node).is_none() {
            continue;
        }
        let parent = dom.parent(node);
        let parent_style = parent
            .and_then(|p| entries.get(p.idx_usize()).and_then(|o| o.clone()))
            .map(|rc| (*rc).clone());
        let mut style = ComputedStyle::initial();
        if let Some(p) = &parent_style {
            style.inherit_from(p);
        }

        // -- candidate declarations ---------------------------------------
        let mut by_prop: HashMap<&'static str, Candidate<'_>> = HashMap::new();
        // custom properties take arbitrary author names, so owned keys
        let mut custom: Vec<(String, Candidate<'_>)> = Vec::new();
        let mut hit = false;

        // `style=""` beats every selector, `!important` included (except UA !important)
        if let Some(decls) = attrs.get(node) {
            hit = true;
            for (di, d) in decls.iter().enumerate() {
                let cand = Candidate {
                    decl: d,
                    specificity: (0, 0, 0),
                    origin: Origin::AuthorAttr,
                    // later declarations inside one attribute win
                    order: usize::MAX - decls.len() + di,
                };
                if d.is_custom {
                    match custom.iter().position(|(n, _)| n == &d.name) {
                        Some(pos) => {
                            if !custom[pos].1.wins_over(&cand) {
                                custom[pos].1 = cand;
                            }
                        }
                        None => custom.push((d.name.clone(), cand)),
                    }
                    continue;
                }
                let key = intern(&d.name);
                match by_prop.get(key) {
                    Some(e) if e.wins_over(&cand) => {}
                    _ => {
                        by_prop.insert(key, cand);
                    }
                }
            }
        }

        for i in candidate_ids(&index, dom, node) {
            let r = &flat[i];
            if !matches(dom, node, r.selector, state) {
                continue;
            }
            hit = true;
            let spec = r.selector.specificity();
            for d in r.decls {
                let cand = Candidate {
                    decl: d,
                    specificity: (spec.0, spec.1, spec.2),
                    origin: r.origin,
                    order: r.order,
                };
                if d.is_custom {
                    match custom.iter().position(|(n, _)| n == &d.name) {
                        Some(pos) => {
                            if !custom[pos].1.wins_over(&cand) {
                                custom[pos].1 = cand;
                            }
                        }
                        None => custom.push((d.name.clone(), cand)),
                    }
                    continue;
                }
                let key = intern(&d.name);
                match by_prop.get(key) {
                    Some(e) if e.wins_over(&cand) => {}
                    _ => {
                        by_prop.insert(key, cand);
                    }
                }
            }
        }
        if hit {
            matched_elements += 1;
        }

        // -- custom properties (inherit + cascade) -------------------------
        let mut custom_map: HashMap<String, String> = parent
            .map(|p| p.idx_usize())
            .and_then(|pi| custom_cache.get(&pi).cloned())
            .unwrap_or_default();
        for (name, c) in &custom {
            custom_map.insert(name.clone(), c.text());
        }
        custom_cache.insert(node.idx_usize(), custom_map.clone());

        // -- apply ---------------------------------------------------------
        let el = dom.element(node).unwrap();
        apply(&mut style, &by_prop, &custom_map, viewport, el.tag.as_str());

        // -- ::before / ::after -------------------------------------------
        for (sel, pseudo, decls) in &pseudo_rules {
            if !matches(dom, node, sel, state) {
                continue;
            }
            for d in decls.iter() {
                if d.name != "content" {
                    continue;
                }
                let text = content_of(&d.value, &custom_map);
                match *pseudo {
                    "before" => style.before_content = Some(text),
                    "after" => style.after_content = Some(text),
                    _ => {}
                }
            }
        }

        if entries.len() <= node.idx_usize() {
            entries.resize(node.idx_usize() + 1, None);
        }
        entries[node.idx_usize()] = Some(Rc::new(style));
    }

    Styled {
        styles: StyleMap { entries },
        problems,
        matched_elements,
        rule_count: flat.len(),
    }
}

/// Rule ids that could possibly match this element (bucket lookup).
fn candidate_ids(index: &HashMap<Key, Vec<usize>>, dom: &Dom, node: NodeId) -> Vec<usize> {
    let el = match dom.element(node) {
        Some(e) => e,
        None => return Vec::new(),
    };
    let mut ids: Vec<usize> = Vec::new();
    if let Some(id) = el.element_id() {
        if let Some(v) = index.get(&Key::Id(id.to_string())) {
            ids.extend(v.iter().copied());
        }
    }
    for c in el.classes() {
        if let Some(v) = index.get(&Key::Class(c.to_string())) {
            ids.extend(v.iter().copied());
        }
    }
    if let Some(v) = index.get(&Key::Type(el.tag.clone())) {
        ids.extend(v.iter().copied());
    }
    if let Some(v) = index.get(&Key::Universal) {
        ids.extend(v.iter().copied());
    }
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// Stable property keys so the per-element map needs no allocation.
fn intern(name: &str) -> &'static str {
    const KNOWN: &[&str] = &[
        "display",
        "position",
        "box-sizing",
        "float",
        "clear",
        "width",
        "height",
        "min-width",
        "max-width",
        "min-height",
        "max-height",
        "margin",
        "margin-top",
        "margin-right",
        "margin-bottom",
        "margin-left",
        "padding",
        "padding-top",
        "padding-right",
        "padding-bottom",
        "padding-left",
        "border",
        "border-top",
        "border-right",
        "border-bottom",
        "border-left",
        "border-width",
        "border-style",
        "border-color",
        "border-radius",
        "top",
        "right",
        "bottom",
        "left",
        "inset",
        "z-index",
        "overflow",
        "overflow-x",
        "overflow-y",
        "visibility",
        "opacity",
        "color",
        "background",
        "background-color",
        "background-image",
        "font",
        "font-family",
        "font-size",
        "font-weight",
        "font-style",
        "line-height",
        "text-align",
        "text-decoration",
        "text-decoration-line",
        "text-transform",
        "white-space",
        "letter-spacing",
        "word-spacing",
        "vertical-align",
        "flex",
        "flex-direction",
        "flex-wrap",
        "flex-grow",
        "flex-shrink",
        "flex-basis",
        "justify-content",
        "align-items",
        "align-self",
        "align-content",
        "gap",
        "row-gap",
        "column-gap",
        "list-style",
        "list-style-type",
        "content",
        "cursor",
        "transition",
        "animation",
        "transform",
    ];
    KNOWN
        .iter()
        .copied()
        .find(|k| *k == name)
        .unwrap_or("_unknown")
}

/// Flatten a rule list, evaluating `@media` against the viewport.
fn collect_rules<'a>(
    rules: &'a [Rule],
    viewport: Viewport,
    origin: Origin,
    order: &mut usize,
    out: &mut Vec<FlatRule<'a>>,
) {
    for r in rules {
        match r {
            Rule::Style(s) => {
                *order += 1;
                for sel in &s.selectors {
                    out.push(FlatRule {
                        selector: sel,
                        decls: &s.declarations,
                        origin,
                        order: *order,
                        pseudo: sel.pseudo_element(),
                    });
                }
                collect_rules(&s.nested, viewport, origin, order, out);
            }
            Rule::Media { queries, rules } => {
                if queries.is_empty() || queries.iter().any(|q| q.matches(&viewport)) {
                    collect_rules(rules, viewport, origin, order, out);
                }
            }
            Rule::Supports { rules } => collect_rules(rules, viewport, origin, order, out),
            _ => {}
        }
    }
}

/// Bucket key from the right-most compound of a selector.
fn rule_key(sel: &ComplexSelector) -> Option<Key> {
    let compound = sel.parts.iter().rev().find_map(|p| match p {
        SelectorPart::Compound(c) => Some(c),
        _ => None,
    })?;
    let mut class = None;
    let mut ty = None;
    for s in compound.iter().rev() {
        match s {
            Simple::Id(i) => return Some(Key::Id(i.clone())),
            Simple::Class(c) => {
                if class.is_none() {
                    class = Some(Key::Class(c.clone()));
                }
            }
            Simple::Type(t) if ty.is_none() => {
                ty = Some(Key::Type(t.clone()));
            }
            _ => {}
        }
    }
    Some(class.or(ty).unwrap_or(Key::Universal))
}

/// Drop the pseudo-element part so the rest of the selector can be matched.
fn strip_pseudo(sel: &ComplexSelector) -> Option<ComplexSelector> {
    let mut stripped = sel.clone();
    let mut keep: Vec<SelectorPart> = Vec::new();
    for p in &stripped.parts {
        match p {
            SelectorPart::Compound(c) => {
                let without: Vec<Simple> = c
                    .iter()
                    .filter(|s| !matches!(s, Simple::PseudoElement { .. }))
                    .cloned()
                    .collect();
                if !without.is_empty() {
                    keep.push(SelectorPart::Compound(without));
                }
            }
            other => keep.push(other.clone()),
        }
    }
    if keep.is_empty() {
        return None;
    }
    stripped.parts = keep;
    Some(stripped)
}

/// `content: "text"`, with `var()` support.
fn content_of(value: &DeclarationValue, custom: &HashMap<String, String>) -> String {
    let mut out = String::new();
    for p in &value.parts {
        match p {
            Value::Str(s) => out.push_str(s.as_str()),
            Value::Keyword(k) if k == "none" => {}
            Value::Function { name, args } if name == "var" => {
                if let Some(n) = args.first().and_then(|a| a.as_str()) {
                    if let Some(v) = custom.get(n) {
                        out.push_str(&v.trim_matches('"').replace("\\a", "\n"));
                    }
                }
            }
            _ => {}
        }
    }
    out
}

// ---------------------------------------------------------------- applying --

type Lookup<'a> = &'a dyn Fn(&str) -> Option<String>;

/// Apply the winning declarations to `style`.
fn apply(
    style: &mut ComputedStyle,
    props: &HashMap<&'static str, Candidate<'_>>,
    custom: &HashMap<String, String>,
    viewport: Viewport,
    tag: &str,
) {
    let lookup: Lookup = &|name: &str| custom.get(name).cloned();

    // font-size first: `em` depends on it
    let parent_size = style.font_size;
    if let Some(c) = props.get("font-size") {
        if let Some(v) = c.one() {
            if let Some(px) = font_size(v, parent_size, &lookup, viewport) {
                style.font_size = px;
            }
        }
    }
    let res = Resolved::new(style.font_size, 16.0, viewport.width, viewport.height, 0.0);
    let len = |v: &Value| -> Option<L> { linear(v, &res, &lookup) };

    // ---- text -----------------------------------------------------------
    if let Some(c) = props.get("color") {
        if let Some(col) = color_of(&c.text(), &lookup) {
            style.color = col;
        }
    }
    if let Some(c) = props.get("font-family") {
        let parsed = parse_value(&substitute(&c.text(), &lookup));
        let fam = parsed.font_family();
        if !fam.is_empty() {
            style.font_family = fam;
        }
    }
    if let Some(c) = props.get("font-weight") {
        style.font_weight = weight_of(&substitute(&c.text(), &lookup));
    }
    if let Some(c) = props.get("font-style") {
        let v = substitute(&c.text(), &lookup);
        if v == "italic" || v == "oblique" {
            style.italic = true;
        } else if v == "normal" {
            style.italic = false;
        }
    }
    if let Some(c) = props.get("font") {
        apply_font_shorthand(style, &c.decl.value, parent_size, viewport);
    }
    if let Some(c) = props.get("line-height") {
        style.line_height = line_height_of(
            &substitute(&c.text(), &lookup),
            style.font_size,
            &res,
            &lookup,
        );
    }
    if let Some(c) = props.get("text-align") {
        style.text_align = match substitute(&c.text(), &lookup).as_str() {
            "center" | "-internal-center" | "middle" => TextAlign::Center,
            "right" | "end" => TextAlign::Right,
            "justify" => TextAlign::Justify,
            "left" | "start" => TextAlign::Left,
            _ => style.text_align,
        };
    }
    for key in ["text-decoration", "text-decoration-line"] {
        if let Some(c) = props.get(key) {
            let v = substitute(&c.text(), &lookup);
            match v.as_str() {
                "none" => {
                    style.text_decoration = false;
                    style.text_decoration_line = false;
                }
                "underline" => style.text_decoration = true,
                "line-through" => style.text_decoration_line = true,
                _ => {
                    if v.contains("underline") {
                        style.text_decoration = true;
                    }
                    if v.contains("line-through") {
                        style.text_decoration_line = true;
                    }
                }
            }
        }
    }
    if let Some(c) = props.get("text-transform") {
        style.text_transform = match substitute(&c.text(), &lookup).as_str() {
            "uppercase" => TextTransform::Uppercase,
            "lowercase" => TextTransform::Lowercase,
            "capitalize" => TextTransform::Capitalize,
            _ => TextTransform::None,
        };
    }
    if let Some(c) = props.get("white-space") {
        style.white_space = match substitute(&c.text(), &lookup).as_str() {
            "nowrap" => WhiteSpace::Nowrap,
            "pre" => WhiteSpace::Pre,
            "pre-wrap" => WhiteSpace::PreWrap,
            "break-spaces" => WhiteSpace::BreakSpaces,
            _ => WhiteSpace::Normal,
        };
    }
    if let Some(c) = props.get("letter-spacing") {
        let v = substitute(&c.text(), &lookup);
        if v == "normal" {
            style.letter_spacing = 0.0;
        } else if let Some(x) = parse_value(&v).parts.first().and_then(&len) {
            style.letter_spacing = x.resolve_zero(0.0);
        }
    }
    if let Some(c) = props.get("word-spacing") {
        let v = substitute(&c.text(), &lookup);
        if let Some(x) = parse_value(&v).parts.first().and_then(&len) {
            style.word_spacing = x.resolve_zero(0.0);
        }
    }
    if let Some(c) = props.get("vertical-align") {
        style.vertical_align_top = matches!(
            substitute(&c.text(), &lookup).as_str(),
            "top" | "text-top" | "super"
        );
    }
    if let Some(c) = props.get("visibility") {
        style.visibility = !matches!(
            substitute(&c.text(), &lookup).as_str(),
            "hidden" | "collapse"
        );
    }
    if let Some(c) = props.get("opacity") {
        if let Some(n) = c.one().and_then(|v| v.as_number()) {
            style.opacity = n.clamp(0.0, 1.0);
        }
    }

    // ---- box ------------------------------------------------------------
    if let Some(c) = props.get("display") {
        if let Some(d) = display_of(&substitute(&c.text(), &lookup), tag) {
            style.display = d;
        }
    }
    if let Some(c) = props.get("position") {
        style.position = match substitute(&c.text(), &lookup).as_str() {
            "relative" => Position::Relative,
            "absolute" => Position::Absolute,
            "fixed" => Position::Fixed,
            "sticky" => Position::Sticky,
            _ => Position::Static,
        };
    }
    if let Some(c) = props.get("box-sizing") {
        style.box_sizing_border = substitute(&c.text(), &lookup) == "border-box";
    }
    if let Some(c) = props.get("float") {
        let v = substitute(&c.text(), &lookup);
        style.float_left = v == "left";
        style.float_right = v == "right";
    }
    if let Some(c) = props.get("clear") {
        style.clear = substitute(&c.text(), &lookup) != "none";
    }
    for (name, slot) in [
        ("width", &mut style.width as &mut L),
        ("height", &mut style.height),
        ("min-width", &mut style.min_width),
        ("max-width", &mut style.max_width),
        ("min-height", &mut style.min_height),
        ("max-height", &mut style.max_height),
    ] {
        if let Some(c) = props.get(name) {
            let v = substitute(&c.text(), &lookup);
            let parsed = parse_value(&v);
            match parsed.parts.first() {
                Some(p) if p.is_keyword("auto") || p.is_keyword("none") => *slot = L::Auto,
                Some(p) if p.is_keyword("fit-content") || p.is_keyword("max-content") => {
                    *slot = L::Auto
                }
                Some(p) => {
                    if let Some(x) = len(p) {
                        *slot = x;
                    }
                }
                None => {}
            }
        }
    }
    for (name, x, y) in [
        ("overflow", true, true),
        ("overflow-x", true, false),
        ("overflow-y", false, true),
    ] {
        if let Some(c) = props.get(name) {
            let o = overflow_of(&substitute(&c.text(), &lookup));
            if x {
                style.overflow_x = o;
            }
            if y {
                style.overflow_y = o;
            }
        }
    }
    if let Some(c) = props.get("z-index") {
        let v = substitute(&c.text(), &lookup);
        style.z_index = match v.as_str() {
            "auto" | "" => None,
            _ => v.parse::<i32>().ok(),
        };
    }

    // ---- margins / padding / offsets / borders --------------------------
    if let Some(c) = props.get("margin") {
        style.margin = sides(&substitute(&c.text(), &lookup), &len);
    }
    for (name, i) in [
        ("margin-top", ComputedStyle::TOP),
        ("margin-right", ComputedStyle::RIGHT),
        ("margin-bottom", ComputedStyle::BOTTOM),
        ("margin-left", ComputedStyle::LEFT),
    ] {
        if let Some(c) = props.get(name) {
            let v = substitute(&c.text(), &lookup);
            if let Some(x) = single(&v, &len) {
                style.margin[i] = x;
            }
        }
    }
    if let Some(c) = props.get("padding") {
        style.padding = sides(&substitute(&c.text(), &lookup), &len);
    }
    for (name, i) in [
        ("padding-top", ComputedStyle::TOP),
        ("padding-right", ComputedStyle::RIGHT),
        ("padding-bottom", ComputedStyle::BOTTOM),
        ("padding-left", ComputedStyle::LEFT),
    ] {
        if let Some(c) = props.get(name) {
            let v = substitute(&c.text(), &lookup);
            if let Some(x) = single(&v, &len) {
                style.padding[i] = x;
            }
        }
    }
    if let Some(c) = props.get("inset") {
        style.inset = sides(&substitute(&c.text(), &lookup), &len);
    }
    for (name, i) in [
        ("top", ComputedStyle::TOP),
        ("right", ComputedStyle::RIGHT),
        ("bottom", ComputedStyle::BOTTOM),
        ("left", ComputedStyle::LEFT),
    ] {
        if let Some(c) = props.get(name) {
            let v = substitute(&c.text(), &lookup);
            if let Some(x) = single(&v, &len) {
                style.inset[i] = x;
            }
        }
    }

    // borders
    if let Some(c) = props.get("border") {
        let (w, st, col) = border_parts(&substitute(&c.text(), &lookup), &len);
        if let Some(w) = w {
            style.border_width = [w; 4];
        }
        if let Some(s) = st {
            style.border_style = [s; 4];
        }
        if let Some(c) = col {
            style.border_color = [c; 4];
        }
    }
    for (name, i) in [
        ("border-top", ComputedStyle::TOP),
        ("border-right", ComputedStyle::RIGHT),
        ("border-bottom", ComputedStyle::BOTTOM),
        ("border-left", ComputedStyle::LEFT),
    ] {
        if let Some(c) = props.get(name) {
            let (w, st, col) = border_parts(&substitute(&c.text(), &lookup), &len);
            if let Some(w) = w {
                style.border_width[i] = w;
            }
            if let Some(s) = st {
                style.border_style[i] = s;
            }
            if let Some(c) = col {
                style.border_color[i] = c;
            }
        }
    }
    if let Some(c) = props.get("border-width") {
        let v = substitute(&c.text(), &lookup);
        let nums: Vec<f32> = parse_value(&v)
            .parts
            .iter()
            .filter_map(|p| len(p).map(|l| l.resolve_zero(0.0)))
            .collect();
        style.border_width = spread4(nums);
    }
    if let Some(c) = props.get("border-style") {
        let v = substitute(&c.text(), &lookup);
        let list: Vec<BorderStyle> = v.split_whitespace().map(border_style_of).collect();
        if !list.is_empty() {
            style.border_style = if list.len() == 1 {
                [list[0]; 4]
            } else {
                [list[0], list[1], list[0], list[1]]
            };
        }
    }
    if let Some(c) = props.get("border-color") {
        let v = substitute(&c.text(), &lookup);
        let cols: Vec<Color> = parse_value(&v)
            .parts
            .iter()
            .filter_map(|p| color_part(p, &lookup))
            .collect();
        if cols.len() == 1 {
            style.border_color = [cols[0]; 4];
        } else if cols.len() >= 2 {
            style.border_color = [cols[0], cols[1], cols[0], cols[1]];
        }
    }
    if let Some(c) = props.get("border-radius") {
        let v = substitute(&c.text(), &lookup);
        let nums: Vec<f32> = parse_value(&v)
            .parts
            .iter()
            .filter_map(|p| linear(p, &res, &lookup).map(|l| l.resolve_zero(0.0)))
            .collect();
        style.border_radius = spread4(nums);
    }
    // a width without a style still paints (thin line, like browsers do)
    for i in 0..4 {
        if style.border_width[i] > 0.0 && style.border_style[i] == BorderStyle::None {
            style.border_style[i] = BorderStyle::Solid;
        }
    }

    // ---- background ----------------------------------------------------
    if let Some(c) = props.get("background") {
        let v = substitute(&c.text(), &lookup);
        let parsed = parse_value(&v);
        let mut image = false;
        for p in &parsed.parts {
            match p {
                Value::Function { name, .. }
                    if matches!(
                        name.as_str(),
                        "url" | "linear-gradient" | "radial-gradient" | "image"
                    ) =>
                {
                    image = true;
                }
                _ => {
                    if let Some(col) = color_part(p, &lookup) {
                        style.background_color = col;
                    }
                }
            }
        }
        style.background_image = image;
    }
    if let Some(c) = props.get("background-color") {
        let v = substitute(&c.text(), &lookup);
        if let Some(col) = color_of(&v, &lookup) {
            style.background_color = col;
        }
    }
    if let Some(c) = props.get("background-image") {
        let v = substitute(&c.text(), &lookup);
        style.background_image = v != "none" && !v.is_empty();
    }

    // ---- flex ----------------------------------------------------------
    if let Some(c) = props.get("flex-direction") {
        style.flex_direction = match substitute(&c.text(), &lookup).as_str() {
            "column" => FlexDirection::Column,
            "column-reverse" => FlexDirection::ColumnReverse,
            "row-reverse" => FlexDirection::RowReverse,
            _ => FlexDirection::Row,
        };
    }
    if let Some(c) = props.get("flex-wrap") {
        style.flex_wrap = substitute(&c.text(), &lookup) != "nowrap";
    }
    if let Some(c) = props.get("flex-grow") {
        if let Some(n) = c.one().and_then(|v| v.as_number()) {
            style.flex_grow = n.max(0.0);
        }
    }
    if let Some(c) = props.get("flex-shrink") {
        if let Some(n) = c.one().and_then(|v| v.as_number()) {
            style.flex_shrink = n.max(0.0);
        }
    }
    if let Some(c) = props.get("flex-basis") {
        let v = substitute(&c.text(), &lookup);
        style.flex_basis = if v == "auto" || v == "content" {
            L::Auto
        } else {
            parse_value(&v)
                .parts
                .first()
                .and_then(&len)
                .unwrap_or(L::Auto)
        };
    }
    if let Some(c) = props.get("flex") {
        let v = substitute(&c.text(), &lookup);
        let parts: Vec<&str> = v.split_whitespace().collect();
        match parts.first().copied().unwrap_or("") {
            "none" => {
                style.flex_grow = 0.0;
                style.flex_shrink = 0.0;
                style.flex_basis = L::Auto;
            }
            "auto" => {
                style.flex_grow = 1.0;
                style.flex_shrink = 1.0;
                style.flex_basis = L::Auto;
            }
            "initial" => {
                style.flex_grow = 0.0;
                style.flex_shrink = 1.0;
                style.flex_basis = L::Px(0.0);
            }
            _ => {
                if let Some(g) = parts.first().and_then(|s| s.parse::<f32>().ok()) {
                    style.flex_grow = g.max(0.0);
                    style.flex_shrink = parts
                        .get(1)
                        .and_then(|s| s.parse::<f32>().ok())
                        .unwrap_or(1.0)
                        .max(0.0);
                    match parts.get(2) {
                        Some(b) if *b == "auto" || *b == "content" => style.flex_basis = L::Auto,
                        Some(b) => {
                            style.flex_basis = parse_value(b)
                                .parts
                                .first()
                                .and_then(&len)
                                .unwrap_or(L::Px(0.0))
                        }
                        None => style.flex_basis = L::Px(0.0),
                    }
                }
            }
        }
    }
    if let Some(c) = props.get("justify-content") {
        style.justify_content = match substitute(&c.text(), &lookup).as_str() {
            "center" => JustifyContent::Center,
            "flex-end" | "end" | "right" => JustifyContent::FlexEnd,
            "space-between" => JustifyContent::SpaceBetween,
            "space-around" => JustifyContent::SpaceAround,
            "space-evenly" => JustifyContent::SpaceEvenly,
            _ => JustifyContent::FlexStart,
        };
    }
    for (name, is_self) in [("align-items", false), ("align-self", true)] {
        if let Some(c) = props.get(name) {
            let ai = match substitute(&c.text(), &lookup).as_str() {
                "center" => Some(AlignItems::Center),
                "flex-start" | "start" => Some(AlignItems::FlexStart),
                "flex-end" | "end" => Some(AlignItems::FlexEnd),
                "stretch" => Some(AlignItems::Stretch),
                "baseline" => Some(AlignItems::Baseline),
                _ => None,
            };
            if let Some(v) = ai {
                if is_self {
                    style.align_self = Some(v);
                } else {
                    style.align_items = v;
                }
            }
        }
    }
    for name in ["gap", "column-gap", "row-gap"] {
        if let Some(c) = props.get(name) {
            let v = substitute(&c.text(), &lookup);
            if let Some(x) = parse_value(&v).parts.first().and_then(&len) {
                let px = x.resolve_zero(0.0);
                if name != "column-gap" {
                    style.row_gap = px;
                }
                if name != "row-gap" {
                    style.column_gap = px;
                }
            }
        }
    }
    if let Some(c) = props.get("align-content") {
        // only `center` and `flex-end` change anything for our 1-pass layout
        match substitute(&c.text(), &lookup).as_str() {
            "center" => style.align_items = AlignItems::Center,
            "flex-end" | "end" => style.align_items = AlignItems::FlexEnd,
            _ => {}
        }
    }

    // ---- lists ---------------------------------------------------------
    for name in ["list-style", "list-style-type"] {
        if let Some(c) = props.get(name) {
            let v = substitute(&c.text(), &lookup);
            if v.contains("none") {
                style.list_style_bullet = false;
                style.list_style_number = false;
            } else if v.contains("decimal") || v.contains("number") {
                style.list_style_bullet = false;
                style.list_style_number = true;
            } else if v.contains("disc") || v.contains("circle") || v.contains("square") {
                style.list_style_bullet = true;
                style.list_style_number = false;
            }
        }
    }
}

/// Replace `var(...)` occurrences with their values (text level, then re-parsed).
fn substitute(css: &str, lookup: Lookup) -> String {
    if !css.contains("var(") {
        return css.to_string();
    }
    let parsed = parse_value(css);
    let mut out: Vec<String> = Vec::new();
    for p in &parsed.parts {
        out.push(value_of(p, lookup));
    }
    out.join(" ")
}

/// Split `1em 0` / `0 auto` style shorthands into four sides.
fn sides(css: &str, len: &dyn Fn(&Value) -> Option<L>) -> [L; 4] {
    let parts = parse_value(css);
    let vals: Vec<L> = parts
        .parts
        .iter()
        .map(|p| {
            if p.is_keyword("auto") {
                L::Auto
            } else {
                len(p).unwrap_or(L::Px(0.0))
            }
        })
        .collect();
    match vals.len() {
        0 => [L::Px(0.0); 4],
        1 => [vals[0]; 4],
        2 => [vals[0], vals[1], vals[0], vals[1]],
        3 => [vals[0], vals[1], vals[2], vals[1]],
        _ => [vals[0], vals[1], vals[2], vals[3]],
    }
}

fn single(css: &str, len: &dyn Fn(&Value) -> Option<L>) -> Option<L> {
    let parts = parse_value(css);
    let p = parts.parts.first()?;
    if p.is_keyword("auto") {
        return Some(L::Auto);
    }
    len(p)
}

/// Parse `1px solid red` into its three components.
fn border_parts(
    css: &str,
    len: &dyn Fn(&Value) -> Option<L>,
) -> (Option<f32>, Option<BorderStyle>, Option<Color>) {
    let parts = parse_value(css);
    let mut w = None;
    let mut st = None;
    let mut col = None;
    for p in &parts.parts {
        if let Some(x) = len(p) {
            w = Some(x.resolve_zero(0.0));
            continue;
        }
        if let Some(k) = p.as_str() {
            if k == "none" || k == "hidden" {
                st = Some(BorderStyle::None);
                w = Some(0.0);
                continue;
            }
            if let Some(s) = border_style_opt(k) {
                st = Some(s);
                continue;
            }
            if k == "thin" {
                w = Some(1.0);
                continue;
            }
            if k == "medium" {
                w = Some(3.0);
                continue;
            }
            if k == "thick" {
                w = Some(5.0);
                continue;
            }
        }
        if let Some(c) = color_part(p, &|_: &str| None) {
            col = Some(c);
        }
    }
    (w, st, col)
}

fn border_style_opt(k: &str) -> Option<BorderStyle> {
    Some(match k {
        "solid" => BorderStyle::Solid,
        "dashed" => BorderStyle::Dashed,
        "dotted" => BorderStyle::Dotted,
        "double" => BorderStyle::Double,
        "none" | "hidden" => BorderStyle::None,
        _ => return None,
    })
}

fn border_style_of(k: &str) -> BorderStyle {
    border_style_opt(k).unwrap_or(BorderStyle::None)
}

fn spread4(nums: Vec<f32>) -> [f32; 4] {
    match nums.len() {
        0 => [0.0; 4],
        1 => [nums[0]; 4],
        2 => [nums[0], nums[1], nums[0], nums[1]],
        3 => [nums[0], nums[1], nums[2], nums[1]],
        _ => [nums[0], nums[1], nums[2], nums[3]],
    }
}

/// `font: bold 12px/1.4 Arial`
/// `font: bold italic 12px/1.4 Georgia, serif`
///
/// Works on the parsed value parts: the serialiser keeps `/` as its own token,
/// so a plain whitespace split would misread the size.
fn apply_font_shorthand(
    style: &mut ComputedStyle,
    dv: &DeclarationValue,
    parent: f32,
    viewport: Viewport,
) {
    let lookup: Lookup = &|_| None;
    let mut size: Option<Value> = None;
    let mut line: Option<Value> = None;
    let mut family: Vec<String> = Vec::new();
    let mut after_slash = false;
    let mut size_seen = false;
    for p in &dv.parts {
        if p.is_keyword("/") {
            after_slash = true;
            continue;
        }
        if p.is_keyword(",") {
            continue;
        }
        if after_slash {
            line = Some(p.clone());
            after_slash = false;
            continue;
        }
        match p {
            Value::Keyword(k) => match k.as_str() {
                "bold" => style.font_weight = 700,
                "bolder" => style.font_weight = 800,
                "lighter" | "light" => style.font_weight = 300,
                "italic" | "oblique" => style.italic = true,
                "normal" | "caption" | "icon" | "menu" | "message-box" | "small-caption"
                | "status-bar" => {}
                _ if looks_like_length(k) => {
                    // e.g. `12px` parsed as a keyword-ish token
                    if !size_seen {
                        size = Some(p.clone());
                        size_seen = true;
                    }
                }
                _ => family.push(k.clone()),
            },
            Value::Length(v, u) => {
                let base = style.font_size;
                let px = match u {
                    Unit::Px | Unit::PxConverted => *v,
                    _ => Resolved::new(base, 16.0, viewport.width, viewport.height, 0.0)
                        .length(*v, *u),
                };
                // the size is the first absolute length in the shorthand;
                // later lengths would be part of a family name (rare)
                if px > 1.0 && !size_seen {
                    size = Some(p.clone());
                    size_seen = true;
                }
            }
            Value::Number(_) => {
                if !size_seen {
                    size = Some(p.clone());
                    size_seen = true;
                }
            }
            Value::Str(s) => family.push(s.clone()),
            _ => {}
        }
    }
    if let Some(s) = &size {
        if let Some(px) = font_size(s, parent, &lookup, viewport) {
            style.font_size = px;
        }
    }
    if let Some(l) = &line {
        let res = Resolved::new(style.font_size, 16.0, viewport.width, viewport.height, 0.0);
        style.line_height = line_height_of_value(l, style.font_size, &res, &lookup);
    }
    if !family.is_empty() {
        style.font_family = family;
    }
}

fn looks_like_length(s: &str) -> bool {
    s.chars()
        .next()
        .map(|c| c.is_ascii_digit() || c == '.' || c == '-' || c == '+')
        .unwrap_or(false)
}

fn display_of(css: &str, _tag: &str) -> Option<Display> {
    Some(match css {
        "none" => Display::None,
        "block" => Display::Block,
        "inline" => Display::Inline,
        "inline-block" => Display::InlineBlock,
        "flex" => Display::Flex,
        "inline-flex" => Display::InlineFlex,
        "list-item" => Display::ListItem,
        "table" => Display::Table,
        "table-row" => Display::TableRow,
        "table-row-group" | "table-header-group" | "table-footer-group" => Display::TableRowGroup,
        "table-cell" | "table-column" | "table-column-group" => Display::TableCell,
        "contents" => Display::Contents,
        // grid / flow-root / … are approximated so pages stay readable
        "grid" | "inline-grid" | "flow-root" | "inline-table" => {
            if css.starts_with("inline") {
                Display::InlineBlock
            } else {
                Display::Block
            }
        }
        _ => return None,
    })
}

fn overflow_of(css: &str) -> Overflow {
    match css {
        "hidden" | "clip" => Overflow::Hidden,
        "scroll" => Overflow::Scroll,
        "auto" => Overflow::Auto,
        _ => Overflow::Visible,
    }
}

fn weight_of(css: &str) -> u16 {
    match css {
        "bold" => 700,
        "bolder" => 800,
        "lighter" => 300,
        "normal" => 400,
        _ => css.trim().parse::<f32>().map(|n| n as u16).unwrap_or(400),
    }
}

fn line_height_of(
    css: &str,
    font_size: f32,
    res: &Resolved,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> f32 {
    let parts = parse_value(css);
    match parts.parts.first() {
        None => 0.0,
        Some(v) => line_height_of_value(v, font_size, res, lookup),
    }
}

/// `line-height`: a plain number is a multiplier of the font size, a length is
/// used as is, `normal` is left to the renderer (0).
fn line_height_of_value(
    v: &Value,
    font_size: f32,
    res: &Resolved,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> f32 {
    match v {
        Value::Keyword(k) if k == "normal" => 0.0,
        Value::Number(n) => n * font_size,
        Value::Length(n, Unit::None) => n * font_size,
        other => linear(other, res, lookup)
            .map(|l| l.resolve_zero(0.0))
            .unwrap_or(0.0),
    }
}

/// `font-size`: keywords, lengths, percentages (of parent).
fn font_size(v: &Value, parent: f32, lookup: Lookup, viewport: Viewport) -> Option<f32> {
    if let Some(k) = v.as_str() {
        let kw = match k {
            "xx-small" => 9.0,
            "x-small" => 10.0,
            "small" => 13.0,
            "medium" => 16.0,
            "large" => 18.0,
            "x-large" => 24.0,
            "xx-large" => 32.0,
            "xxx-large" => 48.0,
            "smaller" => parent / 1.2,
            "larger" => parent * 1.2,
            _ => return None,
        };
        return Some(kw);
    }
    if let Value::Length(n, Unit::Percent) = v {
        return Some(parent * n / 100.0);
    }
    let res = Resolved::new(parent, 16.0, viewport.width, viewport.height, 0.0);
    match linear(v, &res, lookup)? {
        L::Px(v) => Some(v),
        L::Linear { pct, fixed } => Some(parent * pct / 100.0 + fixed),
        L::Auto => None,
    }
}

/// Resolve a single `var()` to text, else return the value's CSS text.
fn value_of(v: &Value, lookup: Lookup) -> String {
    match v {
        Value::Function { name, args } if name == "var" => {
            let Some(n) = args.first().and_then(|a| a.as_str()) else {
                return String::new();
            };
            if let Some(val) = lookup(n) {
                val
            } else if let Some(fb) = args.iter().skip(1).find(|a| !a.is_keyword(",")) {
                value_of(fb, lookup)
            } else {
                String::new()
            }
        }
        other => other.to_css(),
    }
}

fn color_of(css: &str, lookup: Lookup) -> Option<Color> {
    let css = substitute(css, lookup);
    let parsed = parse_value(&css);
    parsed.parts.iter().find_map(|p| color_part(p, lookup))
}

fn color_part(v: &Value, lookup: Lookup) -> Option<Color> {
    match v {
        Value::Color(c) => Some(*c),
        Value::Keyword(k) if k == "transparent" => Some(Color::TRANSPARENT),
        Value::Keyword(k) if k == "currentcolor" => None,
        Value::Keyword(k) => Color::parse(k),
        Value::Function { name, args } if name == "var" => {
            let n = args.first().and_then(|a| a.as_str())?;
            let raw = lookup(n)?;
            color_of(&raw, lookup)
        }
        _ => None,
    }
}

/// Turn a value into our linear length form (`pct% of a basis + fixed px`).
fn linear(v: &Value, res: &Resolved, lookup: &dyn Fn(&str) -> Option<String>) -> Option<L> {
    use tb_css::value::CalcTerm as T;
    match v {
        Value::Keyword(k) if k == "auto" || k == "none" => Some(L::Auto),
        Value::Length(n, Unit::Percent) => Some(L::Linear {
            pct: *n,
            fixed: 0.0,
        }),
        Value::Length(n, u) => Some(L::Px(res.length(*n, *u))),
        Value::Number(n) => Some(L::Px(*n)),
        Value::Calc(expr) => {
            let mut pct = 0.0f32;
            let mut fixed = 0.0f32;
            for (sign, term) in &expr.terms {
                let (tp, tf) = match term {
                    T::Length(n, Unit::Percent, mul) => (sign * n * mul, 0.0),
                    T::Length(n, u, mul) => (0.0, sign * res.length(*n, *u) * mul),
                    T::Number(n) => (0.0, sign * n),
                    T::Calc(inner, mul) => {
                        match linear(&Value::Calc((**inner).clone()), res, lookup) {
                            Some(L::Px(p)) => (0.0, sign * p * mul),
                            Some(L::Linear { pct: p, fixed: f }) => {
                                (sign * p * mul, sign * f * mul)
                            }
                            _ => (0.0, 0.0),
                        }
                    }
                    T::Var {
                        name,
                        fallback,
                        mul,
                    } => {
                        let raw = lookup(name)
                            .or_else(|| fallback.as_ref().map(|f| f.to_css()))
                            .unwrap_or_default();
                        match parse_value(&raw)
                            .parts
                            .first()
                            .and_then(|p| linear(p, res, lookup))
                        {
                            Some(L::Px(p)) => (0.0, sign * p * mul),
                            Some(L::Linear { pct: p, fixed: f }) => {
                                (sign * p * mul, sign * f * mul)
                            }
                            _ => (0.0, 0.0),
                        }
                    }
                };
                pct += tp;
                fixed += tf;
            }
            if pct != 0.0 {
                Some(L::Linear { pct, fixed })
            } else {
                Some(L::Px(fixed))
            }
        }
        Value::Function { name, .. } if name == "var" => {
            let raw = value_of(v, lookup);
            if raw.is_empty() {
                None
            } else {
                parse_value(&raw)
                    .parts
                    .first()
                    .and_then(|p| linear(p, res, lookup))
            }
        }
        Value::Function { name, args } if matches!(name.as_str(), "min" | "max" | "clamp") => {
            let nums: Vec<f32> = args
                .iter()
                .filter(|a| !a.is_keyword(","))
                .filter_map(|a| linear(a, res, lookup).map(|l| l.resolve_zero(0.0)))
                .collect();
            match (name.as_str(), nums.len()) {
                ("min", n) if n >= 2 => {
                    Some(L::Px(nums.iter().cloned().fold(f32::INFINITY, f32::min)))
                }
                ("max", n) if n >= 2 => Some(L::Px(
                    nums.iter().cloned().fold(f32::NEG_INFINITY, f32::max),
                )),
                ("clamp", 3) => Some(L::Px(nums[0].max(nums[1]).min(nums[2]))),
                _ => None,
            }
        }
        _ => None,
    }
}
