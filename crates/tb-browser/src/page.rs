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
use tb_core::{EngineConfig, Problems, Rect, Result};
use tb_css::StyleSheet;
use tb_dom::{Dom, NodeId};
use tb_dom_bindings::{DomBindings, EvtTarget, Host, NavRequest, ScriptReport};
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
struct Js {
    rt: Runtime,
    bindings: DomBindings,
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
        let fb = match self.layout.clone() {
            Some(layout) => paint_with_images(&layout, &self.dom, opts, &self.image_nodes),
            // no box tree: a blank canvas of viewport size still says "rendered"
            None => Framebuffer::new(
                self.config.viewport.width.max(1.0) as u32,
                self.config.viewport.height.max(1.0) as u32,
            ),
        };
        self.timings.render += started.elapsed().as_secs_f64() * 1000.0;
        fb
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

    /// Re-run style and layout on the current DOM.
    pub(crate) fn restyle_and_layout(&mut self) {
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
