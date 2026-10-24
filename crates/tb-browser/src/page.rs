//! `Page`: one document that has been through the whole pipeline.
//!
//! A page owns its DOM, the author stylesheets that applied to it, the computed
//! styles, the box tree, the problems collected while producing them, and —
//! when JS ran — the interpreter and its script host. That is the point of the
//! type: every question about a loaded page is answered from one place, and
//! neither the CLI nor a test has to re-run a pipeline stage to ask it.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use tb_core::{EngineConfig, Phase, Problems, Rect, Result};
use tb_css::StyleSheet;
use tb_dom::{Dom, NodeId};
use tb_dom_bindings::{
    DomBindings, EventInit, EvtTarget, Host, NavRequest, ScriptReport, EVENT_HANDLERS,
};
use tb_gfx::{Framebuffer, Image};
use tb_js::{Runtime, Value};
use tb_layout::{element_at, find_box, LayoutDocument};
use tb_net::Url;
use tb_style::{compute, MatchState, StyleMap};

use crate::paint::{paint_with_images, PaintOptions};
use crate::resources::SharedLoader;
use crate::ImageStore;

/// How long each stage took, in milliseconds.
#[derive(Debug, Clone, Default)]
pub struct Timings {
    pub fetch: f64,
    pub parse: f64,
    pub style: f64,
    pub layout: f64,
    pub script: f64,
    pub render: f64,
}

impl std::fmt::Display for Timings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut parts = Vec::new();
        {
            let mut add = |name: &str, v: f64| {
                if v >= 0.5 {
                    parts.push(format!("{name} {} ms", v.round() as i64));
                }
            };
            add("fetch", self.fetch);
            add("parse", self.parse);
            add("style", self.style);
            add("layout", self.layout);
            add("script", self.script);
            add("render", self.render);
        }
        if parts.is_empty() {
            write!(f, "<1 ms")
        } else {
            write!(f, "{}", parts.join(" · "))
        }
    }
}

/// The numbers that describe a page's size.
#[derive(Debug, Clone, Copy, Default)]
pub struct PageStats {
    pub nodes: usize,
    pub elements: usize,
    pub links: usize,
    /// `<img>` tags in the document.
    pub images: usize,
    /// …of which we actually have pixels for.
    pub images_decoded: usize,
    /// …and which failed to load or use a format we cannot decode.
    pub images_failed: usize,
    pub scripts: usize,
    pub stylesheets: usize,
    pub css_rules: usize,
    pub matched_elements: usize,
    pub boxes: usize,
    pub scripts_run: usize,
    pub script_skipped: usize,
    pub script_failures: usize,
    pub console_lines: usize,
    pub problems: usize,
}

/// A link as the page offers it, with the URL already resolved.
#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    pub text: String,
    pub url: String,
    pub rel: String,
    pub id: String,
}

/// An HTML heading, `level` 1..=6.
#[derive(Debug, Clone, PartialEq)]
pub struct Heading {
    pub level: u8,
    pub text: String,
}

/// A live scripting session.
pub(crate) struct Js {
    rt: Runtime,
    bindings: DomBindings,
}

/// The modifier keys that were held for a pointer or key event. Real handlers
/// read these (`if (e.ctrlKey) window.open(…)`), so they ride along.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub meta: bool,
}

impl Modifiers {
    pub fn none() -> Modifiers {
        Modifiers::default()
    }
}

impl Js {
    fn host(&self) -> Rc<RefCell<Host>> {
        self.bindings.host.clone()
    }
}

/// A document that has been fetched, parsed, styled, laid out and scripted.
pub struct Page {
    pub url: Url,
    pub status: u16,
    pub content_type: String,
    /// Size of the document body in bytes.
    pub bytes: usize,
    pub dom: Dom,
    /// Author CSS in document order (`<link>` + `<style>`, `@import` expanded).
    pub sheets: Vec<StyleSheet>,
    pub styles: StyleMap,
    pub layout: Option<LayoutDocument>,
    /// The pictures this page asked for, decoded and remembered by URL.
    pub images: ImageStore,
    /// `<img>` node → its picture, for the painter (natural size went to layout).
    pub image_nodes: std::collections::HashMap<NodeId, std::rc::Rc<Image>>,
    pub problems: Problems,
    pub timings: Timings,
    /// Elements with at least one author rule (0 when CSS is off).
    pub matched_elements: usize,
    pub script_report: ScriptReport,
    /// Navigations JS asked for (`location.href = …`, `form.submit()`, an
    /// anchor `click()`). A browser does not navigate in the middle of a
    /// script, and neither do we: the embedder decides.
    pub nav_requests: Vec<NavRequest>,
    pub loader: SharedLoader,
    pub config: EngineConfig,
    /// Scroll offset the UI last set (CSS px). The page keeps it so `render`,
    /// hit testing and `window.scrollY` cannot disagree.
    pub scroll_pos: (f32, f32),
    /// Hover/focus/active when no scripts run — the plain-data twin of
    /// `Host::match_state`, so `--no-js` pages still show a caret and a hover.
    ui_hover: Option<NodeId>,
    ui_focus: Option<NodeId>,
    ui_active: Option<NodeId>,
    /// The node the UI last reported as hovered (to notice *changes* — a
    /// restyle on every pixel move would not be interactive).
    hover_node: Option<NodeId>,
    js: Option<Js>,
    fb: Option<Framebuffer>,
}

impl Page {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        url: Url,
        status: u16,
        content_type: String,
        bytes: usize,
        dom: Dom,
        sheets: Vec<StyleSheet>,
        styles: StyleMap,
        layout: Option<LayoutDocument>,
        problems: Problems,
        timings: Timings,
        matched_elements: usize,
        script_report: ScriptReport,
        loader: SharedLoader,
        config: EngineConfig,
        js: Option<(Runtime, DomBindings)>,
    ) -> Page {
        Page {
            url,
            status,
            content_type,
            bytes,
            dom,
            sheets,
            styles,
            layout,
            images: ImageStore::default(),
            image_nodes: std::collections::HashMap::new(),
            problems,
            timings,
            matched_elements,
            script_report,
            nav_requests: Vec::new(),
            loader,
            config,
            scroll_pos: (0.0, 0.0),
            ui_hover: None,
            ui_focus: None,
            ui_active: None,
            hover_node: None,
            js: js.map(|(rt, bindings)| Js { rt, bindings }),
            fb: None,
        }
    }

    // ------------------------------------------------------------------ info -

    /// `document.title`.
    pub fn title(&self) -> String {
        let title = self
            .dom
            .descendants(self.dom.document())
            .into_iter()
            .find(|n| self.dom.tag(*n) == Some("title"));
        match title {
            Some(t) => collapse_ws(&self.dom.text_content(t)),
            None => String::new(),
        }
    }

    pub fn stats(&self) -> PageStats {
        let mut s = PageStats {
            nodes: self.dom.node_count(),
            elements: self.dom.element_count(),
            stylesheets: self.sheets.len(),
            matched_elements: self.matched_elements,
            console_lines: self.console().len(),
            problems: self.problems.all().len(),
            scripts_run: self.script_report.executed,
            script_skipped: self.script_report.skipped,
            script_failures: self.script_report.failed,
            ..Default::default()
        };
        for sheet in &self.sheets {
            s.css_rules += sheet
                .rules
                .iter()
                .filter(|r| matches!(r, tb_css::Rule::Style(_)))
                .count();
        }
        for n in self.dom.descendants(self.dom.document()) {
            let Some(el) = self.dom.element(n) else {
                continue;
            };
            match el.tag.as_str() {
                "a" | "area" => {
                    if el.has_attr("href") {
                        s.links += 1
                    }
                }
                "img" => {
                    s.images += 1;
                    if self.image_nodes.contains_key(&n) {
                        s.images_decoded += 1;
                    } else if let Some(src) = el.attr("src") {
                        let tried = self
                            .loader
                            .borrow()
                            .resolve(src)
                            .map(|u| self.images.knows_url(&u.to_string()))
                            .unwrap_or(false);
                        if tried {
                            s.images_failed += 1;
                        }
                    }
                }
                "script" => s.scripts += 1,
                _ => (),
            }
        }
        if let Some(l) = &self.layout {
            s.boxes = l.box_count;
        }
        s
    }

    /// What the page says, in document order. `script`, `style`, `head` and
    /// friends are not rendered, so they are not text either.
    pub fn text(&self) -> String {
        let mut raw = String::new();
        for c in self.dom.children(self.dom.document()) {
            self.collect_text(c, &mut raw);
        }
        tidy_lines(&raw)
    }

    fn collect_text(&self, node: NodeId, out: &mut String) {
        match node_kind(&self.dom, node) {
            "text" => out.push_str(&self.dom.text_content(node)),
            "element" => {
                // `display: none` and `visibility: hidden` are not rendered, so
                // they are not text either — computed styles decide, not tags
                if let Some(st) = self.styles.get(node) {
                    if st.display == tb_style::Display::None || !st.visibility {
                        return;
                    }
                }
                let tag = self.dom.tag(node).unwrap_or("");
                if matches!(
                    tag,
                    "script"
                        | "style"
                        | "head"
                        | "noscript"
                        | "template"
                        | "title"
                        | "meta"
                        | "link"
                        | "svg"
                ) {
                    return;
                }
                let block = matches!(
                    tag,
                    "p" | "div"
                        | "section"
                        | "article"
                        | "header"
                        | "footer"
                        | "nav"
                        | "aside"
                        | "main"
                        | "ul"
                        | "ol"
                        | "li"
                        | "table"
                        | "tr"
                        | "h1"
                        | "h2"
                        | "h3"
                        | "h4"
                        | "h5"
                        | "h6"
                        | "blockquote"
                        | "pre"
                        | "br"
                        | "hr"
                        | "form"
                        | "figure"
                        | "figcaption"
                        | "dt"
                        | "dd"
                        | "address"
                );
                if block && !out.is_empty() && !out.ends_with('\n') {
                    out.push('\n');
                }
                if tag == "li" {
                    out.push_str("• ");
                }
                for c in self.dom.children(node) {
                    self.collect_text(c, out);
                }
                if block {
                    out.push('\n');
                }
            }
            _ => (),
        }
    }

    /// Every `<a href>`/`<area href>`, resolved against the page URL.
    pub fn links(&self) -> Vec<Link> {
        let mut out = Vec::new();
        for n in self.dom.descendants(self.dom.document()) {
            let Some(el) = self.dom.element(n) else {
                continue;
            };
            if !matches!(el.tag.as_str(), "a" | "area") || !el.has_attr("href") {
                continue;
            }
            let href = el.attr("href").unwrap_or("").trim();
            // what `a.href` says: fragments and `mailto:` included, resolved
            // against the page — not "can the loader fetch this"
            let url = Url::join(&self.url, href)
                .ok()
                .or_else(|| self.loader.borrow().resolve(href))
                .map(|u| u.to_string())
                .unwrap_or_else(|| href.to_string());
            out.push(Link {
                text: collapse_ws(&self.dom.text_content(n)),
                url,
                rel: el.attr("rel").unwrap_or("").to_string(),
                id: el.element_id().unwrap_or("").to_string(),
            });
        }
        out
    }

    /// Headings in document order — the page's outline.
    pub fn headings(&self) -> Vec<Heading> {
        let mut out = Vec::new();
        for n in self.dom.descendants(self.dom.document()) {
            let Some(tag) = self.dom.tag(n) else {
                continue;
            };
            let level = match tag {
                "h1" => 1,
                "h2" => 2,
                "h3" => 3,
                "h4" => 4,
                "h5" => 5,
                "h6" => 6,
                _ => continue,
            };
            let text = collapse_ws(&self.dom.text_content(n));
            if !text.is_empty() {
                out.push(Heading { level, text });
            }
        }
        out
    }

    /// `<title>` plus headings: what the page is about.
    pub fn titles(&self) -> Vec<String> {
        let mut out = Vec::new();
        let t = self.title();
        if !t.is_empty() {
            out.push(t);
        }
        out.extend(self.headings().into_iter().map(|h| h.text));
        out
    }

    /// Everything `console.*` printed, oldest first.
    pub fn console(&self) -> Vec<String> {
        self.js.as_ref().map(|j| j.rt.console()).unwrap_or_default()
    }

    /// Console output with levels: `("log", "…")`, `("error", "…")`.
    pub fn console_lines(&self) -> Vec<(String, String)> {
        match &self.js {
            Some(j) => {
                j.rt.interp
                    .console
                    .iter()
                    .map(|(l, t)| (l.clone(), t.clone()))
                    .collect()
            }
            None => Vec::new(),
        }
    }

    pub fn problems(&self) -> &Problems {
        &self.problems
    }

    /// `history.state` of the current entry.
    pub fn history_state(&self) -> Option<Value> {
        let js = self.js.as_ref()?;
        let h = js.host();
        let h = h.borrow();
        h.states.get(h.history_pos).cloned()
    }

    // ----------------------------------------------------------------- dumps -

    /// A compact DOM tree: tag, `#id`, `.classes`, the interesting attributes.
    pub fn dump_dom(&self) -> String {
        let mut out = String::new();
        self.dump_node(self.dom.document(), 0, &mut out);
        out
    }

    fn dump_node(&self, node: NodeId, depth: usize, out: &mut String) {
        match node_kind(&self.dom, node) {
            "text" => {
                let t = collapse_ws(&self.dom.text_content(node));
                if !t.is_empty() {
                    indent(out, depth);
                    out.push_str(&format!("#text {:?}\n", tb_core::text::truncate(&t, 60)));
                }
            }
            "comment" => {
                indent(out, depth);
                out.push_str("<!-- comment -->\n");
            }
            "#document" | "#doctype" => {
                for c in self.dom.children(node) {
                    self.dump_node(c, depth, out);
                }
            }
            "element" => {
                let el = self.dom.element(node).expect("element");
                indent(out, depth);
                out.push('<');
                out.push_str(&el.tag);
                if let Some(id) = el.element_id() {
                    out.push_str(&format!("#{id}"));
                }
                for c in el.classes() {
                    out.push_str(&format!(".{c}"));
                }
                for name in ["href", "src", "type", "alt", "value", "name", "media"] {
                    if let Some(v) = el.attr(name) {
                        out.push_str(&format!(" {name}={}", tb_core::text::truncate(v, 40)));
                    }
                }
                out.push_str(">\n");
                for c in self.dom.children(node) {
                    self.dump_node(c, depth + 1, out);
                }
            }
            _ => (),
        }
    }

    /// Which sheets applied and how much is in each.
    pub fn dump_css(&self) -> String {
        let mut out = String::new();
        for sheet in &self.sheets {
            let rules = sheet
                .rules
                .iter()
                .filter(|r| matches!(r, tb_css::Rule::Style(_)))
                .count();
            let media = sheet
                .rules
                .iter()
                .filter(|r| matches!(r, tb_css::Rule::Media { .. }))
                .count();
            let name = if sheet.href.is_empty() {
                "<style>"
            } else {
                &sheet.href
            };
            out.push_str(&format!("{name}: {rules} rules, {media} @media\n"));
        }
        out
    }

    /// The box tree with measurements.
    pub fn dump_layout(&self) -> String {
        match &self.layout {
            Some(l) => l.debug_tree(&self.dom),
            None => "no box tree (layout disabled or nothing to lay out)\n".into(),
        }
    }

    // -------------------------------------------------------------- geometry -

    /// The element at a viewport point (CSS px, unscrolled).
    pub fn element_at(&self, x: f32, y: f32) -> Option<NodeId> {
        self.layout
            .as_ref()
            .and_then(|l| element_at(l, &self.dom, x, y))
    }

    /// `getBoundingClientRect` without going through JS.
    pub fn rect_of(&self, node: NodeId) -> Option<Rect> {
        self.layout
            .as_ref()
            .and_then(|l| find_box(&l.root, node))
            .map(|b| b.border_box())
    }

    pub fn style_of(&self, node: NodeId) -> Option<&tb_style::ComputedStyle> {
        self.styles.get(node)
    }

    /// The tag name at a point — what a click would hit.
    pub fn tag_at(&self, x: f32, y: f32) -> Option<String> {
        self.element_at(x, y)
            .and_then(|n| self.dom.tag(n))
            .map(String::from)
    }

    /// Which of the page's own event handlers could react at all — worth
    /// knowing before firing anything into a page (`GUI` diagnostics).
    pub fn handler_kinds(&self) -> Vec<&'static str> {
        EVENT_HANDLERS.to_vec()
    }

    // ------------------------------------------------------------ interaction --
    //
    // A screenshot needs none of this; a *browser* needs all of it. Everything
    // here answers a question a pointer or a key raises — "what is under the
    // cursor", "what would a click do", "give this node focus" — and goes
    // through the same event machinery the page's own scripts see, so a handler
    // can cancel a navigation exactly once and nobody has to guess who won.

    /// The element under a **viewport** point: CSS px, `0/0` top-left of the
    /// content area, scroll applied (the layout works in document coordinates).
    pub fn element_at_viewport(&self, x: f32, y: f32) -> Option<NodeId> {
        let (sx, sy) = self.scroll();
        self.element_at(x + sx, y + sy)
    }

    /// The `<a href>` a click on `node` would follow — `node` itself or the
    /// nearest anchor above it, resolved for display. `<area>` counts too.
    pub fn closest_link(&self, node: NodeId) -> Option<String> {
        let mut cur = Some(node);
        while let Some(c) = cur {
            if matches!(self.dom.tag(c), Some("a") | Some("area")) {
                if let Some(href) = self.dom.attr(c, "href") {
                    return Some(self.resolve_display(&href));
                }
            }
            cur = self.dom.parent(c);
        }
        None
    }

    /// The link under a viewport point (what the status bar shows).
    pub fn link_at_viewport(&self, x: f32, y: f32) -> Option<String> {
        self.element_at_viewport(x, y)
            .and_then(|n| self.closest_link(n))
    }

    /// `(tag, type)` of the form control at or above `node` — what a click or a
    /// keystroke has to do: focus a text field, toggle a checkbox, open a
    /// select, submit a button.
    pub fn control_at(&self, node: NodeId) -> Option<(String, String)> {
        let mut cur = Some(node);
        while let Some(c) = cur {
            let tag = self.dom.tag(c)?;
            match tag {
                "input" | "textarea" | "select" | "button" | "a" | "area" => {
                    let kind = if tag == "input" {
                        self.dom.attr(c, "type").unwrap_or_else(|| "text".into())
                    } else {
                        String::new()
                    };
                    return Some((tag.to_string(), kind));
                }
                _ => (),
            }
            cur = self.dom.parent(c);
        }
        None
    }

    /// A short `<a href="…">text</a>`-style description of a node — the
    /// inspector and the status bar speak the user's language, not `NodeId(412)`.
    pub fn describe(&self, node: NodeId) -> String {
        let tag = self.dom.tag(node).unwrap_or("#text");
        let mut out = String::from(tag);
        if let Some(el) = self.dom.element(node) {
            if let Some(id) = el.element_id() {
                out.push('#');
                out.push_str(id);
            }
            for c in el.classes().into_iter().take(3) {
                out.push('.');
                out.push_str(c);
            }
            for name in ["href", "src", "type", "alt", "name", "value"] {
                if let Some(v) = el.attr(name) {
                    out.push_str(&format!(" {name}={}", tb_core::text::truncate(v, 40)));
                }
            }
        }
        let text = collapse_ws(&self.dom.text_content(node));
        if !text.is_empty() && !matches!(tag, "script" | "style") {
            out.push_str(&format!("  {}", tb_core::text::truncate(&text, 48)));
        }
        out
    }

    /// Every element `Tab` can reach, in DOM order: links with `href`, form
    /// controls that are not disabled, and anything with a `tabindex`.
    pub fn focusables(&self) -> Vec<NodeId> {
        let mut out = Vec::new();
        for n in self.dom.descendants(self.dom.document()) {
            let Some(el) = self.dom.element(n) else {
                continue;
            };
            if el.has_attr("disabled") {
                continue;
            }
            let ok = match el.tag.as_str() {
                "a" | "area" => el.has_attr("href"),
                "input" | "textarea" | "select" | "button" => el.attr("type") != Some("hidden"),
                _ => el.has_attr("tabindex"),
            };
            if ok {
                // `display: none` is not reachable by keyboard, and neither is
                // an element without a box (the layout is the authority here)
                if let Some(st) = self.styles.get(n) {
                    if st.display == tb_style::Display::None {
                        continue;
                    }
                }
                out.push(n);
            }
        }
        out
    }

    /// `document.activeElement`, or `None` while the chrome has the caret.
    pub fn focused(&self) -> Option<NodeId> {
        let h = self.host()?;
        let h = h.borrow();
        h.match_state.focus.filter(|n| h.dom.exists(*n))
    }

    /// Move focus into the page (`None` = the chrome took the caret). Fires
    /// `blur`/`focus` and makes `:focus` rules apply — the page notices.
    pub fn set_focus(&mut self, node: Option<NodeId>) {
        if !self.has_js() {
            // no scripts, no event machinery: remember it on the page instead so
            // at least the caret and `Tab` still work
            self.ui_focus = node;
            return;
        }
        let current = self.focused();
        if current == node {
            return;
        }
        let Some(js) = self.js.as_mut() else {
            return;
        };
        let _ = js.bindings.set_focus(&mut js.rt, node);
        self.sync_after_js();
    }

    /// `:hover` for the node under the pointer.
    ///
    /// `restyle` says whether the cascade may run again: a page with 3 000
    /// elements takes a visible moment for that, so a GUI opts in only when the
    /// hovered element actually *changed* (and the CLI/offscreen tests keep it
    /// off because nobody sees the difference).
    pub fn set_hover(&mut self, node: Option<NodeId>, restyle: bool) {
        let changed = self.hover_node != node;
        if !changed {
            return;
        }
        self.hover_node = node;
        if !self.has_js() {
            self.ui_hover = node;
            return;
        }
        if let Some(js) = self.js.as_mut() {
            js.bindings.set_hover(node);
        }
        if restyle {
            self.restyle_and_layout();
        }
    }

    /// `:active` while a mouse button is held on a node.
    pub fn set_active(&mut self, node: Option<NodeId>) {
        if let Some(js) = self.js.as_mut() {
            js.bindings.set_active(node);
        }
        if node.is_none() {
            self.ui_active = None;
        } else {
            self.ui_active = node;
        }
    }

    /// The node the UI treats as hovered/focused/active when no scripts run —
    /// the plain-data twin of `Host.match_state`.
    pub fn ui_state(&self) -> (Option<NodeId>, Option<NodeId>, Option<NodeId>) {
        if let Some(h) = self.host() {
            let h = h.borrow();
            return (
                h.match_state.hover,
                h.match_state.focus,
                h.match_state.active,
            );
        }
        (self.ui_hover, self.ui_focus, self.ui_active)
    }

    /// Current scroll offset (CSS px).
    pub fn scroll(&self) -> (f32, f32) {
        self.scroll_pos
    }

    /// Furthest down the page can go with the current viewport.
    pub fn max_scroll(&self) -> f32 {
        (self.content_height() - self.config.viewport.height).max(0.0)
    }

    /// How far right the content reaches (a wide table, `white-space: pre`).
    pub fn content_width(&self) -> f32 {
        self.layout
            .as_ref()
            .map(|l| l.scroll_size.width)
            .unwrap_or(self.config.viewport.width)
    }

    /// The horizontal counterpart of [`Page::max_scroll`].
    pub fn max_scroll_x(&self) -> f32 {
        (self.content_width() - self.config.viewport.width).max(0.0)
    }

    /// Scroll to `(x, y)` — clamped, and told to the page so `window.scrollY`,
    /// `position: sticky` and the paint cache all agree.
    pub fn set_scroll(&mut self, x: f32, y: f32) {
        let x = x.max(0.0);
        let y = y.max(0.0).min(self.max_scroll());
        if (self.scroll_pos.0 - x).abs() < 0.5 && (self.scroll_pos.1 - y).abs() < 0.5 {
            return;
        }
        self.scroll_pos = (x, y);
        if let Some(js) = self.js.as_mut() {
            js.bindings.set_scroll(x as f64, y as f64);
        }
        self.invalidate_render();
    }

    /// Scroll the minimum so that `node` is inside the viewport.
    pub fn scroll_into_view(&mut self, node: NodeId) {
        let Some(r) = self.rect_of(node) else {
            return;
        };
        let (sx, sy) = self.scroll_pos;
        let vh = self.config.viewport.height;
        let mut dy = 0.0;
        if r.top() - sy < 0.0 {
            dy = r.top() - sy;
        } else if r.bottom() - sy > vh {
            dy = r.bottom() - sy - vh;
        }
        let mut dx = 0.0;
        let vw = self.config.viewport.width;
        if r.left() - sx < 0.0 {
            dx = r.left() - sx;
        } else if r.right() - sx > vw {
            dx = r.right() - sx - vw;
        }
        if dy != 0.0 || dx != 0.0 {
            self.set_scroll(sx + dx, sy + dy);
        }
    }

    /// Fire an event **with a payload** (a real click has coordinates, a real
    /// key press has `key`). The answer is `false` when a listener called
    /// `preventDefault()`, i.e. the embedder must not do the default action.
    pub fn fire_init(&mut self, target: EvtTarget, kind: &str, init: EventInit) -> bool {
        let allow = match self.js.as_mut() {
            Some(js) => match js
                .bindings
                .fire_with(&mut js.rt, target, kind, init)
                .map(|v| v.to_boolean())
            {
                Ok(v) => v,
                Err(e) => {
                    self.problems
                        .warn(Phase::Script, format!("{kind} handler: {e}"));
                    true
                }
            },
            None => true,
        };
        self.sync_after_js();
        allow
    }

    /// A pointer click on `node`: `mousedown` → `mouseup` → `click`, in that
    /// order and with coordinates, so `preventDefault` and `clientX` behave.
    /// `true` means a listener cancelled the default action (do not navigate).
    pub fn pointer_click(
        &mut self,
        node: NodeId,
        x: f32,
        y: f32,
        button: i32,
        detail: i32,
        mods: Modifiers,
    ) -> bool {
        let (sx, sy) = self.scroll_pos;
        let init = |kind: &str| {
            EventInit::new()
                .text("type", kind)
                .num("clientX", x as f64)
                .num("clientY", y as f64)
                .num("pageX", (x + sx) as f64)
                .num("pageY", (y + sy) as f64)
                .num("button", button as f64)
                .num("detail", detail as f64)
                .bit("ctrlKey", mods.ctrl)
                .bit("shiftKey", mods.shift)
                .bit("altKey", mods.alt)
                .bit("metaKey", mods.meta)
        };
        let target = EvtTarget::node(node);
        // `mousedown` may already cancel (a drag-to-select must not focus, a
        // `mousedown` handler that calls preventDefault stops the click too)
        let down = self.fire_init(target, "mousedown", init("mousedown"));
        self.fire_init(target, "mouseup", init("mouseup"));
        if !down {
            return true;
        }
        !self.fire_init(target, "click", init("click"))
    }

    /// A key event on `node` (`keydown`/`keypress`/`keyup`). `true` when a
    /// handler cancelled it — `preventDefault` on `keydown` means the field
    /// must not insert the character.
    pub fn key_event(
        &mut self,
        node: NodeId,
        kind: &str,
        key: &str,
        code: &str,
        mods: Modifiers,
    ) -> bool {
        self.fire_init(
            EvtTarget::node(node),
            kind,
            EventInit::new()
                .text("type", kind)
                .text("key", key)
                .text("code", code)
                .num(
                    "charCode",
                    if key.chars().count() == 1 {
                        key.chars().next().unwrap() as u32 as f64
                    } else {
                        0.0
                    },
                )
                .bit("ctrlKey", mods.ctrl)
                .bit("shiftKey", mods.shift)
                .bit("altKey", mods.alt)
                .bit("metaKey", mods.meta),
        )
    }

    /// `keyup` for the key the user released.
    pub fn keyup(&mut self, node: NodeId, key: &str) {
        self.key_event(node, "keyup", key, key, Modifiers::default());
    }

    /// `change` — a select that moved, a checkbox the user ticked.
    pub fn fire_change(&mut self, node: NodeId) {
        self.fire_init(EvtTarget::node(node), "change", EventInit::new());
    }

    /// `submit` on a form. `false` means a listener cancelled it, so the
    /// browser must not follow the action.
    pub fn fire_submit(&mut self, node: NodeId) -> bool {
        self.fire_init(
            EvtTarget::node(node),
            "submit",
            EventInit::new().cancelable(true),
        )
    }

    /// Type into a form control: the **dirty value** changes (the `value`
    /// attribute does not), then `input`, then `change` like a real edit ends.
    pub fn type_into(&mut self, node: NodeId, value: &str) {
        if let Some(js) = self.js.as_mut() {
            if js.bindings.set_input_value(node, value).is_err() {
                return;
            }
        }
        self.fire_init(
            EvtTarget::node(node),
            "input",
            EventInit::new().cancelable(false),
        );
    }

    /// `el.value` of a control, including what the user typed.
    pub fn input_value(&self, node: NodeId) -> String {
        match &self.host() {
            Some(h) => h.borrow().form_value(node, || self.markup_value(node)),
            None => self.markup_value(node),
        }
    }

    /// The default action of a click **without scripts**: a link navigates, a
    /// submit button submits its form. With scripts this is the bindings'
    /// business (`click_default_action`), and a browser must still follow links
    /// on a page that has no JavaScript at all.
    pub fn default_action(&self, node: NodeId) -> Option<NavRequest> {
        if let Some(url) = self.closest_link(node) {
            return Some(NavRequest::Load(url));
        }
        let (tag, kind) = self.control_at(node)?;
        if tag == "input" && matches!(kind.as_str(), "checkbox" | "radio") {
            return None;
        }
        let submits = (tag == "button" && kind != "button")
            || (tag == "input" && matches!(kind.as_str(), "submit" | "image"));
        if submits {
            return self.form_action(self.ancestor_of(node, "form")?);
        }
        None
    }

    /// Where a form would be submitted to, with the values of its named
    /// controls in the query string (`GET`; `POST` is reported as a problem by
    /// the caller — we do not send request bodies yet).
    pub fn form_action(&self, form: NodeId) -> Option<NavRequest> {
        let action = self.dom.attr(form, "action").unwrap_or_default();
        let base = if action.is_empty() {
            self.url.clone()
        } else {
            Url::parse(&self.resolve_display(&action)).ok()?
        };
        let mut parts: Vec<String> = Vec::new();
        for n in self.dom.descendants(form) {
            let Some(name) = self.dom.attr(n, "name") else {
                continue;
            };
            if name.is_empty() {
                continue;
            }
            let tag = self.dom.tag(n).unwrap_or("");
            let kind = self.dom.attr(n, "type").unwrap_or_else(|| "text".into());
            match tag {
                "input" if matches!(kind.as_str(), "checkbox" | "radio") => {
                    if self.dom.has_attr(n, "checked") {
                        let v = self.input_value(n);
                        parts.push(format!(
                            "{}={}",
                            tb_net::percent_encode(&name),
                            tb_net::percent_encode(if v.is_empty() { "on" } else { &v })
                        ));
                    }
                }
                "input" | "textarea" | "select" => {
                    let v = self.input_value(n);
                    parts.push(format!(
                        "{}={}",
                        tb_net::percent_encode(&name),
                        tb_net::percent_encode(&v)
                    ));
                }
                _ => (),
            }
        }
        let mut url = base;
        url.query = if parts.is_empty() {
            None
        } else {
            Some(parts.join("&"))
        };
        url.fragment = None;
        Some(NavRequest::Load(url.to_string()))
    }

    /// Every text run of the laid out page, in reading order — the base of
    /// selection, find-in-page and the text renderer.
    pub fn text_runs(&self) -> Vec<&tb_layout::TextRun> {
        let Some(l) = &self.layout else {
            return Vec::new();
        };
        let mut runs = Vec::new();
        l.root.collect_runs(&mut runs);
        runs.sort_by(|a, b| {
            a.baseline
                .partial_cmp(&b.baseline)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal))
        });
        runs
    }

    /// The rectangles of the visible text runs, document coordinates — used to
    /// paint a selection or a find highlight without re-measuring anything.
    pub fn run_rects(&self) -> Vec<(Rect, String)> {
        self.text_runs()
            .into_iter()
            .map(|r| {
                let asc = r.style.font_size * 0.85;
                (
                    Rect::new(r.x, r.baseline - asc, r.width, r.style.line_box()),
                    r.text.clone(),
                )
            })
            .collect()
    }

    // ---------------------------------------------------------- tiny helpers --

    /// `el.value` straight out of the markup (no script wrote one).
    fn markup_value(&self, node: NodeId) -> String {
        match self.dom.tag(node) {
            Some("textarea") => self.dom.text_content(node),
            Some("option") => self
                .dom
                .attr(node, "value")
                .unwrap_or_else(|| self.dom.text_content(node).trim().to_string()),
            Some("select") => {
                let opts: Vec<NodeId> = self
                    .dom
                    .descendants(node)
                    .into_iter()
                    .filter(|n| self.dom.tag(*n) == Some("option"))
                    .collect();
                opts.iter()
                    .find(|o| self.dom.has_attr(**o, "selected"))
                    .or_else(|| opts.first())
                    .map(|o| self.markup_value(*o))
                    .unwrap_or_default()
            }
            _ => self.dom.attr(node, "value").unwrap_or_default(),
        }
    }

    fn ancestor_of(&self, node: NodeId, tag: &str) -> Option<NodeId> {
        let mut cur = self.dom.parent(node);
        while let Some(c) = cur {
            if self.dom.tag(c) == Some(tag) {
                return Some(c);
            }
            cur = self.dom.parent(c);
        }
        None
    }

    /// Resolve an `href` the way `a.href` does: absolute, no `#`-stripping,
    /// `about:`/`mailto:` keep their scheme.
    pub fn resolve_display(&self, raw: &str) -> String {
        let raw = raw.trim();
        if raw.is_empty() {
            return self.url.to_string();
        }
        Url::join(&self.url, raw)
            .ok()
            .or_else(|| self.loader.borrow().resolve(raw))
            .map(|u| u.to_string())
            .unwrap_or_else(|| raw.to_string())
    }

    // --------------------------------------------------------------- render -

    /// Paint the page (cached — a re-layout drops the cache).
    pub fn render(&mut self, opts: &PaintOptions) -> &Framebuffer {
        if self.fb.is_none() {
            let fb = self.paint(opts);
            self.fb = Some(fb);
        }
        self.fb.as_ref().expect("just painted")
    }

    pub fn paint(&mut self, opts: &PaintOptions) -> Framebuffer {
        let started = std::time::Instant::now();
        let fb = self.paint_view(opts);
        self.timings.render += started.elapsed().as_secs_f64() * 1000.0;
        fb
    }

    /// Paint without touching the timings — a GUI repaints dozens of times per
    /// second and must not book all of it against one page load.
    pub fn paint_view(&self, opts: &PaintOptions) -> Framebuffer {
        match &self.layout {
            Some(layout) => paint_with_images(layout, &self.dom, opts, &self.image_nodes),
            // no box tree: a blank canvas of viewport size still says "rendered"
            None => Framebuffer::new(
                self.config.viewport.width.max(1.0) as u32,
                self.config.viewport.height.max(1.0) as u32,
            ),
        }
    }

    /// Write a PNG with our own encoder; returns the number of bytes written.
    pub fn screenshot(&mut self, path: &Path, opts: &PaintOptions) -> Result<usize> {
        let fb = self.render(opts).clone();
        let bytes = tb_gfx::screenshot(&fb, Some(path))?;
        Ok(bytes.len())
    }

    pub fn invalidate_render(&mut self) {
        self.fb = None;
    }

    /// Height of the whole document, not just the viewport.
    pub fn content_height(&self) -> f32 {
        self.layout
            .as_ref()
            .map(|l| l.scroll_size.height)
            .unwrap_or(self.config.viewport.height)
    }

    /// Resize the viewport (a browser resize, effectively) and re-run style and
    /// layout — `--height 4000` for a full-page screenshot uses this.
    pub fn set_viewport(&mut self, width: f32, height: f32) {
        if (self.config.viewport.width - width).abs() < 0.01
            && (self.config.viewport.height - height).abs() < 0.01
        {
            return;
        }
        self.config.viewport.width = width;
        self.config.viewport.height = height;
        if let Some(host) = self.host() {
            host.borrow_mut().ctx.config.viewport.width = width;
            host.borrow_mut().ctx.config.viewport.height = height;
        }
        self.restyle_and_layout();
    }

    // -------------------------------------------------------------------- js -

    pub fn has_js(&self) -> bool {
        self.js.is_some()
    }

    /// The document's mutation counter: style, layout and pictures are only
    /// worth redoing when this changed. A UI caches its paint on it.
    pub fn document_version(&self) -> u64 {
        self.host().map(|h| h.borrow().version()).unwrap_or(0)
    }

    pub fn host(&self) -> Option<Rc<RefCell<Host>>> {
        self.js.as_ref().map(|j| j.host())
    }

    /// Evaluate a script in the page's realm, then bring style and layout back
    /// in line with whatever the script did to the document.
    pub fn eval(&mut self, source: &str) -> std::result::Result<Value, String> {
        let Some(js) = self.js.as_mut() else {
            return Err("javascript is disabled".into());
        };
        let out = js.bindings.eval(&mut js.rt, source);
        self.sync_after_js();
        out
    }

    /// Fire an event into the page (a real click, a `visibilitychange`, …).
    pub fn fire(&mut self, target: EvtTarget, kind: &str) {
        if let Some(js) = self.js.as_mut() {
            let _ = js.bindings.fire(&mut js.rt, target, kind);
            self.sync_after_js();
        }
    }

    /// A click on a node: `mousedown`, `mouseup`, `click` — the order a real
    /// pointer produces, so `preventDefault` on an anchor works.
    pub fn click(&mut self, node: NodeId) {
        if !self.has_js() {
            return;
        }
        for kind in ["mousedown", "mouseup", "click"] {
            self.fire(EvtTarget::node(node), kind);
        }
    }

    /// Let timers and microtasks run (`ms` of virtual time).
    pub fn pump(&mut self, ms: f64) {
        if let Some(js) = self.js.as_mut() {
            js.bindings.pump(&mut js.rt, ms);
            self.sync_after_js();
        }
    }

    /// What the cookie jar would send for this page (`Cookie:` header).
    pub fn cookie_header(&self) -> String {
        self.host()
            .map(|h| h.borrow().cookie_header())
            .unwrap_or_default()
    }

    // ------------------------------------------------------------- plumbing --

    /// Pull everything the script host changed into the page.
    fn sync_after_js(&mut self) {
        let Some(js) = self.js.as_mut() else {
            return;
        };
        let host = js.host();
        let (dom, navs, cookies, problems, report) = {
            let mut h = host.borrow_mut();
            (
                h.dom.clone(),
                std::mem::take(&mut h.nav),
                h.cookies.clone(),
                std::mem::take(&mut h.problems),
                ScriptReport {
                    executed: h.scripts_run,
                    skipped: self.script_report.skipped,
                    failed: self.script_report.failed,
                },
            )
        };
        let _ = js; // borrow of `self.js` ends with the statement above
                    // A cookie a script set belongs to the session, not to the page object:
                    // push it back into the jar so the next request carries it.
        if let Some(client) = self.loader.borrow().client() {
            let mut jar = client.cookie_jar();
            for (k, v) in &cookies {
                if let Some(c) = tb_net::Cookie::parse(&format!("{k}={v}; path=/"), &self.url) {
                    jar.insert(c);
                }
            }
        }
        self.problems.extend(problems);
        self.nav_requests.extend(navs);
        self.script_report = report;
        self.dom = dom;
        self.restyle_and_layout();
    }

    /// Re-run style and layout on the current DOM. Public because a theme or
    /// `prefers-color-scheme` change has to force it.
    pub fn restyle_and_layout(&mut self) {
        let started = std::time::Instant::now();
        let sheets = crate::engine::assemble_sheets(&self.dom, &self.external_sheets());
        let viewport = tb_style::Viewport {
            width: self.config.viewport.width,
            height: self.config.viewport.height,
            color_scheme: self.config.color_scheme,
            reduced_motion: false,
        };
        let styled = compute(&self.dom, &sheets, viewport, &MatchState::default());
        self.problems.extend(styled.problems);
        self.matched_elements = styled.matched_elements;
        self.styles = styled.styles;
        self.sheets = sheets;
        self.timings.style += started.elapsed().as_secs_f64() * 1000.0;

        // a picture a script inserted is part of the page now, so it is
        // fetched and decoded here too (already-decoded URLs cost nothing)
        let (intrinsic, nodes) = crate::images::load_images(
            &self.dom,
            &self.loader,
            &self.images,
            &mut self.problems,
            self.config.caps.fetch_subresources,
        );
        self.image_nodes = nodes;

        if self.config.caps.layout {
            let started = std::time::Instant::now();
            self.layout = tb_layout::Layout::with_intrinsic(
                tb_style::Viewport {
                    width: self.config.viewport.width,
                    height: self.config.viewport.height,
                    color_scheme: self.config.color_scheme,
                    reduced_motion: false,
                },
                intrinsic,
            )
            .layout(&self.dom, &self.styles);
            self.timings.layout += started.elapsed().as_secs_f64() * 1000.0;
        }
        self.invalidate_render();
    }

    /// The sheets that came from outside the document (`<link>`), which is what
    /// the script host was given; `<style>` elements it parses itself.
    fn external_sheets(&self) -> Vec<StyleSheet> {
        self.sheets
            .iter()
            .filter(|s| !s.href.is_empty())
            .cloned()
            .collect()
    }
}

/// `#text` / `element` / `#comment` … as CSS and the DOM spec name them.
pub(crate) fn node_kind(dom: &Dom, node: NodeId) -> &'static str {
    match dom.get(node) {
        Some(n) => n.kind.type_name(),
        None => "#detached",
    }
}

// ---------------------------------------------------------------- text utils -

fn indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

/// Collapse runs of whitespace into single spaces (for one-line summaries).
pub(crate) fn collapse_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut space = false;
    for c in s.chars() {
        if c.is_whitespace() {
            space = true;
            continue;
        }
        if space && !out.is_empty() {
            out.push(' ');
        }
        space = false;
        out.push(c);
    }
    out
}

/// Trim each line, drop trailing blanks, never more than one blank line.
pub(crate) fn tidy_lines(raw: &str) -> String {
    let lines: Vec<&str> = raw.split('\n').map(str::trim_end).collect();
    let mut out = String::new();
    let mut blank = true;
    for line in lines {
        if line.trim().is_empty() {
            if !blank && !out.is_empty() {
                out.push('\n');
            }
            blank = true;
            continue;
        }
        blank = false;
        out.push_str(line.trim_start());
        out.push('\n');
    }
    while out.ends_with("\n\n") {
        out.pop();
    }
    out
}
