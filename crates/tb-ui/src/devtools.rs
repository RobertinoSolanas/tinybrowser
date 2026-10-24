//! The inspector drawer (`F12`).
//!
//! Six panels, all rendered as text lines with a [`LineKind`] instead of a
//! stylesheet, because the same lines go to a terminal, to an X11 window and to
//! a test:
//!
//! * **Elements** — the DOM path of the picked node, its attributes, its
//!   computed style, its box.
//! * **Console** — what the page printed, errors included, plus a prompt that
//!   evaluates in the page's realm.
//! * **Problems** — everything that went wrong without stopping the page.
//! * **CSS** — which sheets applied and how much is in them.
//! * **Layout** — the box tree under the picked node.
//! * **Info** — URL, status, timings, counters: the `tiny <url>` summary.

use crate::editor::Editor;
use tb_browser::Page;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Panel {
    Elements,
    Console,
    Problems,
    Css,
    Layout,
    Info,
}

pub const PANELS: [Panel; 6] = [
    Panel::Elements,
    Panel::Console,
    Panel::Problems,
    Panel::Css,
    Panel::Layout,
    Panel::Info,
];

impl Panel {
    pub fn title(self) -> &'static str {
        match self {
            Panel::Elements => "Elements",
            Panel::Console => "Console",
            Panel::Problems => "Problems",
            Panel::Css => "CSS",
            Panel::Layout => "Layout",
            Panel::Info => "Info",
        }
    }

    pub fn index(self) -> usize {
        PANELS.iter().position(|p| *p == self).unwrap_or(0)
    }

    pub fn nth(i: usize) -> Panel {
        PANELS.get(i).copied().unwrap_or(Panel::Info)
    }
}

/// How a line should look.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LineKind {
    Heading,
    Text,
    Dim,
    Ok,
    Warn,
    Error,
    Accent,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub text: String,
    pub kind: LineKind,
}

impl Line {
    pub fn new(text: impl Into<String>, kind: LineKind) -> Line {
        Line {
            text: text.into(),
            kind,
        }
    }
    pub fn text(text: impl Into<String>) -> Line {
        Line::new(text, LineKind::Text)
    }
    pub fn dim(text: impl Into<String>) -> Line {
        Line::new(text, LineKind::Dim)
    }
    pub fn head(text: impl Into<String>) -> Line {
        Line::new(text, LineKind::Heading)
    }
}

/// The drawer's state.
#[derive(Clone, Debug)]
pub struct Devtools {
    pub open: bool,
    pub panel: Panel,
    /// Rows the drawer occupies (the user can grow it with `Ctrl+Up/Down`).
    pub rows: usize,
    /// The node the user picked (`Ctrl+Shift+C` then click).
    pub picked: Option<tb_dom::NodeId>,
    /// The console prompt.
    pub prompt: Editor,
    /// Results typed into the console (they are not the page's output).
    pub log: Vec<Line>,
    /// Scroll within the panel, in rows.
    pub scroll: usize,
}

impl Default for Devtools {
    fn default() -> Devtools {
        Devtools {
            open: false,
            panel: Panel::Elements,
            rows: 10,
            picked: None,
            prompt: Editor::new(),
            log: Vec::new(),
            scroll: 0,
        }
    }
}

impl Devtools {
    /// Everything the current panel wants to show, in rows, already trimmed to
    /// `rows` (the caller paints them at `devtools_top`).
    pub fn lines(&self, page: &Page, rows: usize) -> Vec<Line> {
        let mut all = match self.panel {
            Panel::Elements => self.elements(page),
            Panel::Console => self.console(page),
            Panel::Problems => self.problems(page),
            Panel::Css => self.css(page),
            Panel::Layout => self.layout(page),
            Panel::Info => self.info(page),
        };
        if self.scroll > 0 {
            let s = self.scroll.min(all.len().saturating_sub(1));
            all = all.split_off(s);
        }
        all.truncate(rows.max(1));
        all
    }

    fn elements(&self, page: &Page) -> Vec<Line> {
        let mut out = Vec::new();
        let Some(node) = self.picked else {
            return vec![
                Line::head("Elements"),
                Line::dim("nothing picked — Ctrl+Shift+C, then click the page"),
            ];
        };
        out.push(Line::head(format!("Elements — {}", page.describe(node))));
        // the path, `<html> <body> <div.card> …`
        let path: Vec<String> = page
            .dom
            .ancestors(node, true)
            .into_iter()
            .rev()
            .filter_map(|n| page.dom.tag(n))
            .map(|t| format!("<{t}>"))
            .collect();
        out.push(Line::dim(path.join(" ")));
        if let Some(el) = page.dom.element(node) {
            let mut attrs: Vec<String> = Vec::new();
            for name in [
                "id", "class", "href", "src", "type", "name", "value", "alt", "style", "title",
            ] {
                if let Some(v) = el.attr(name) {
                    attrs.push(format!("{name}=\"{}\"", tb_core::text::truncate(v, 48)));
                }
            }
            if !attrs.is_empty() {
                out.push(Line::text(attrs.join("  ")));
            }
        }
        let raw = page.dom.text_content(node);
        let text = tb_core::text::truncate(raw.trim(), 120);
        if !text.is_empty() {
            out.push(Line::text(format!("text: {text}")));
        }
        if let Some(st) = page.style_of(node) {
            let display = format!("{:?}", st.display);
            let position = format!("{:?}", st.position);
            out.push(Line::dim(format!(
                "display={display}  font={}px w={}  color={}  bg={}  pos={position}",
                st.font_size,
                st.font_weight,
                st.color.to_hex(),
                st.background_color.to_hex(),
            )));
        }
        match page.rect_of(node) {
            Some(r) => out.push(Line::dim(format!(
                "box: x={} y={} w={} h={}  (scroll y={})",
                r.x as i32,
                r.y as i32,
                r.width as i32,
                r.height as i32,
                page.scroll().1 as i32
            ))),
            None => out.push(Line::dim("box: none (display:none, or not laid out)")),
        }
        out
    }

    fn console(&self, page: &Page) -> Vec<Line> {
        let mut out = vec![Line::head("Console")];
        for (level, line) in page.console_lines() {
            let kind = match level.as_str() {
                "error" => LineKind::Error,
                "warn" | "warning" => LineKind::Warn,
                _ => LineKind::Text,
            };
            out.push(Line::new(format!("[{level}] {line}"), kind));
        }
        for line in &self.log {
            out.push(line.clone());
        }
        out
    }

    fn problems(&self, page: &Page) -> Vec<Line> {
        let all = page.problems().all();
        let mut out = vec![Line::head(format!("Problems — {}", all.len()))];
        if all.is_empty() {
            out.push(Line::new("nothing to report", LineKind::Ok));
            return out;
        }
        for p in all.iter().take(200) {
            let kind = match p.severity {
                tb_core::Severity::Error => LineKind::Error,
                tb_core::Severity::Warning => LineKind::Warn,
                _ => LineKind::Dim,
            };
            let ctx = if p.context.is_empty() {
                String::new()
            } else {
                tb_core::text::truncate(&p.context, 40)
            };
            out.push(Line::new(
                format!(
                    "[{:?} {}] {}{}",
                    p.severity,
                    p.phase,
                    p.message,
                    if ctx.is_empty() {
                        String::new()
                    } else {
                        format!("  @ {ctx}")
                    }
                ),
                kind,
            ));
        }
        out
    }

    fn css(&self, page: &Page) -> Vec<Line> {
        let mut out = vec![Line::head("CSS")];
        let dump = page.dump_css();
        for line in dump.lines() {
            out.push(Line::text(line.to_string()));
        }
        out.push(Line::dim(format!(
            "{} elements matched of {}",
            page.matched_elements,
            page.stats().elements
        )));
        if let Some(node) = self.picked {
            if let Some(st) = page.style_of(node) {
                let align = format!("{:?}", st.text_align);
                out.push(Line::dim(format!(
                    "picked: font-family {}  size {}px  line-height {}px  text-align {align}",
                    st.font_family.join(","),
                    st.font_size,
                    st.line_height
                )));
            }
        }
        out
    }

    fn layout(&self, page: &Page) -> Vec<Line> {
        let mut out = vec![Line::head("Layout")];
        let dump = page.dump_layout();
        for line in dump.lines().take(200) {
            out.push(Line::dim(line.to_string()));
        }
        out
    }

    fn info(&self, page: &Page) -> Vec<Line> {
        let s = page.stats();
        vec![
            Line::head("Info"),
            Line::text(format!("{}", page.url)),
            Line::text(format!(
                "{} {}  {} bytes  fetch {} ms",
                page.status,
                page.content_type,
                page.bytes,
                page.timings.fetch.round() as i64
            )),
            Line::text(format!(
                "elements {}  nodes {}  links {}  images {} ({} decoded)  scripts {}",
                s.elements, s.nodes, s.links, s.images, s.images_decoded, s.scripts
            )),
            Line::text(format!(
                "css {} sheets / {} rules / {} matched   boxes {}",
                s.stylesheets, s.css_rules, s.matched_elements, s.boxes
            )),
            Line::text(format!(
                "js {} run, {} skipped, {} failed   problems {}",
                s.scripts_run, s.script_skipped, s.script_failures, s.problems
            )),
            Line::dim(format!("pipeline {}", page.timings)),
            Line::dim(format!(
                "scroll {} x {}   document {} px high   zoom {}",
                page.scroll().0 as i32,
                page.scroll().1 as i32,
                page.content_height() as i32,
                page.config.scale
            )),
        ]
    }
}
