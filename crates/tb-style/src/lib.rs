//! # tb-style — styled content
//!
//! This context turns *(DOM, CSS, viewport)* into the [`ComputedStyle`] of every
//! element. It owns three things that people usually mean by "the style engine":
//!
//! * [`matcher`] — selector matching against the DOM arena,
//! * [`cascade`] — rule index, origins, specificity, shorthand expansion,
//!   `var()`/`calc()` resolution, custom properties, `::before`/`::after`,
//! * [`ua`] — the user-agent stylesheet (in CSS, parsed by our own parser).
//!
//! ```text
//!   <style>/<link>/style=""  ─┐
//!                             ├─ collect_sources ─► resolve_sources ─► Vec<StyleSheet>
//!   DOM ──────────────────────┘                                                   │
//!   UA stylesheet ───────────────────────────────────────────────────────────────┤
//!                                                                                 ▼
//!                                                            cascade::compute ─► StyleMap
//! ```

mod cascade;
mod computed;
mod matcher;
mod ua;

pub use cascade::{compute, compute_with_attrs, AttrSheets, Origin, Styled, StyleMap};
pub use computed::{
    AlignItems, BorderStyle, ComputedStyle, Display, FlexDirection, JustifyContent, L, Overflow,
    Position, TextAlign, TextTransform, WhiteSpace,
};
pub use matcher::{matches, matches_compound, nth, nth_hit, MatchState};
pub use ua::{ua_stylesheet, UA_CSS};

pub use tb_core::ColorScheme;
pub use tb_css::media::Viewport;
pub use tb_css::StyleSheet;

/// Where author CSS comes from, in document order.
#[derive(Debug, Clone, PartialEq)]
pub enum CssSource {
    /// A `<style>` element (or `<script type="text/css">`).
    Inline { text: String },
    /// A `<link rel=stylesheet href=...>` — the browser resolves it.
    External { href: String, media: String },
    /// A `@import` found inside a sheet (already resolved to an absolute url
    /// by the browser before fetching).
    Import { href: String },
}

/// Collect author CSS sources in document order.
pub fn collect_sources(dom: &tb_dom::Dom) -> Vec<CssSource> {
    let mut out = Vec::new();
    for node in dom.descendants(dom.document()) {
        let Some(el) = dom.element(node) else {
            continue;
        };
        match el.tag.as_str() {
            "style" => {
                let media = el.attr("media").unwrap_or("").to_ascii_lowercase();
                if media_is_renderable(&media) {
                    out.push(CssSource::Inline {
                        text: dom.text_content(node),
                    });
                }
            }
            "link" => {
                let rel = el
                    .attr("rel")
                    .unwrap_or("")
                    .split_whitespace()
                    .any(|w| w.eq_ignore_ascii_case("stylesheet"));
                let href = el.attr("href").unwrap_or("").trim().to_string();
                let media = el.attr("media").unwrap_or("").to_ascii_lowercase();
                if rel && !href.is_empty() && media_is_renderable(&media) {
                    out.push(CssSource::External { href, media });
                }
            }
            _ => {}
        }
    }
    out
}

/// `media="print"` / `"speech"` sheets do not affect screen layout.
fn media_is_renderable(media: &str) -> bool {
    let m = media.trim();
    m.is_empty() || m == "all" || m == "screen" || m.contains("screen")
}

/// Fetch/inline all author CSS in document order, expanding `@import`.
///
/// `fetch(href)` returns the stylesheet text (already charset-decoded) or `None`
/// when it could not be loaded; ordering is preserved and import cycles are
/// broken by URL.
pub fn resolve_sources(
    sources: &[CssSource],
    viewport: Viewport,
    mut fetch: impl FnMut(&str) -> Option<String>,
    problems: &mut tb_core::Problems,
) -> Vec<StyleSheet> {
    let mut out: Vec<StyleSheet> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut queue: Vec<CssSource> = sources.to_vec();
    let mut depth = 0usize;
    // breadth-first per depth keeps document order for the top level
    while !queue.is_empty() && depth < 8 {
        let mut imports: Vec<CssSource> = Vec::new();
        for src in queue.drain(..) {
            let (text, href) = match src {
                CssSource::Inline { text } => (text, String::new()),
                CssSource::External { href, .. } | CssSource::Import { href } => {
                    if !seen.insert(href.clone()) {
                        continue;
                    }
                    match fetch(&href) {
                        Some(t) => (t, href),
                        None => {
                            problems.info(
                                tb_core::Phase::Style,
                                format!("stylesheet not loaded: {href}"),
                            );
                            continue;
                        }
                    }
                }
            };
            let mut sheet = tb_css::parse_stylesheet(&text);
            sheet.href = href;
            // hoist @import targets (resolved by the caller's fetch closure)
            let urls: Vec<String> = sheet
                .rules
                .iter()
                .filter_map(|r| match r {
                    tb_css::Rule::Import { url, media } => {
                        if url.is_empty()
                            || (!media.is_empty()
                                && !media.iter().any(|q| q.matches(&viewport)))
                        {
                            return None;
                        }
                        Some(url.clone())
                    }
                    _ => None,
                })
                .collect();
            for u in urls {
                imports.push(CssSource::Import { href: u });
            }
            out.push(sheet);
        }
        queue = imports;
        depth += 1;
    }
    out
}

/// A style engine bound to a viewport: `style_document()` is the entry point.
#[derive(Debug, Clone)]
pub struct StyleEngine {
    pub viewport: Viewport,
    pub state: MatchState,
}

impl Default for StyleEngine {
    fn default() -> Self {
        StyleEngine {
            viewport: Viewport::default(),
            state: MatchState::default(),
        }
    }
}

impl StyleEngine {
    pub fn new(viewport: Viewport) -> StyleEngine {
        StyleEngine {
            viewport,
            state: MatchState::default(),
        }
    }

    /// Cascade the UA sheet, the given author sheets and all `style=""`
    /// attributes over the document.
    pub fn style_document(&self, dom: &tb_dom::Dom, sheets: &[StyleSheet]) -> Styled {
        compute_with_attrs(dom, sheets, self.viewport, &self.state, &AttrSheets::from_dom(dom))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tb_html::parse_document;

    fn vp() -> Viewport {
        Viewport {
            width: 1000.0,
            height: 800.0,
            ..Default::default()
        }
    }

    fn styled(html: &str) -> tb_dom::Dom {
        parse_document(html)
    }

    #[test]
    fn collects_style_and_link_in_document_order() {
        let dom = styled(
            "<html><head>\
             <style>a{color:red}</style>\
             <link rel=stylesheet href=/a.css>\
             <style media=print>b{color:blue}</style>\
             <style>b{color:green}</style>\
             <link rel=icon href=/f.ico>\
             </head><body></body></html>",
        );
        let s = collect_sources(&dom);
        assert_eq!(s.len(), 3, "{s:?}");
        assert!(matches!(&s[0], CssSource::Inline { text } if text.contains("red")));
        assert!(matches!(&s[1], CssSource::External { href, .. } if href == "/a.css"));
        assert!(matches!(&s[2], CssSource::Inline { text } if text.contains("green")));
    }

    #[test]
    fn resolve_sources_follows_imports_once_and_keeps_order() {
        let mut problems = tb_core::Problems::new();
        let dom = styled("<style>@import url(base.css); h1{color:red}</style>");
        let sources = collect_sources(&dom);
        let sheets = resolve_sources(&sources, vp(), |href| match href {
            "base.css" => Some("body{margin:0}".to_string()),
            _ => None,
        }, &mut problems);
        assert_eq!(sheets.len(), 2);
        assert_eq!(sheets[0].style_rule_count(), 1);
        assert_eq!(sheets[1].style_rule_count(), 1);
        // cycle protection
        let sheets = resolve_sources(
            &[CssSource::Inline {
                text: "@import url(self.css);".into(),
            }],
            vp(),
            |href| Some(format!("@import url({href}); x{{color:red}}")),
            &mut problems,
        );
        assert!(sheets.iter().all(|s| s.rules.len() >= 1));
    }

    #[test]
    fn engine_styles_a_document() {
        let dom = styled("<h1>Hello</h1><p class=x>World</p>");
        let sheets = resolve_sources(
            &[CssSource::Inline {
                text: "h1{color:#ff0000} .x{font-size:20px}".into(),
            }],
            vp(),
            |_| None,
            &mut tb_core::Problems::new(),
        );
        let engine = StyleEngine::new(vp());
        let out = engine.style_document(&dom, &sheets);
        let h1 = dom.get_elements_by_tag("h1")[0];
        let p = dom.get_elements_by_tag("p")[0];
        assert_eq!(out.styles.get(h1).unwrap().color, tb_core::Color::rgb(255, 0, 0));
        assert_eq!(out.styles.get(h1).unwrap().display, Display::Block);
        assert_eq!(out.styles.get(p).unwrap().font_size, 20.0);
        assert_eq!(out.styles.get(p).unwrap().display, Display::Block);
        assert!(out.matched_elements >= 2);
    }
}
