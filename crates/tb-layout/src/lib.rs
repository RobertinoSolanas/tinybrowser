//! # tb-layout — from styled document to boxes with geometry
//!
//! Input: DOM + computed styles + viewport. Output: [`Document`], a box tree in
//! **document coordinates** plus the scroll extent.
//!
//! * [`box_tree`] — DOM × `display` → block / inline / anonymous / atomic boxes
//! * [`block`] — block sizing (auto width, `margin: auto` centring, percentages,
//!   `min`/`max`, light margin collapse), flexbox, replaced elements
//! * [`inline`] — line boxes with word wrapping measured by our own bitmap font,
//!   floats, `text-align`, `white-space`, `text-transform`
//! * [`hit`] — `element_at(x, y)` and link lookup for the CLI/JS `document.elementFromPoint`
//!
//! ```
//! let dom = tb_html::parse_document("<div style='width:100px'><p>hi</p></div>");
//! let styles = tb_style::compute(&dom, &[], Default::default(), &Default::default()).styles;
//! let doc = tb_layout::layout(&dom, &styles, Default::default()).expect("layout");
//! assert!(doc.scroll_size.height > 0.0);
//! let div = dom.get_elements_by_tag("div")[0];
//! let box_ = doc.find(div).expect("div box");
//! assert_eq!(box_.border_box().width, 100.0);
//! ```

pub mod block;
pub mod box_tree;
pub mod hit;
pub mod inline;

pub use block::{text_width, Layout};
pub use box_tree::{is_replaced, BoxKind, LayoutBox, LineBox, TextRun};
pub use hit::{element_at, hit_test, Hit};
pub use inline::{FloatRect, FloatState, Side};

use tb_core::Size;
use tb_css::media::Viewport;
use tb_dom::{Dom, NodeId};
use tb_style::StyleMap;

/// The laid out document (see [`crate::block::Document`]).
pub type LayoutDocument = block::Document;

/// Convenience: build + lay out in one call.
pub fn layout(dom: &Dom, styles: &StyleMap, viewport: Viewport) -> Option<block::Document> {
    Layout::new(viewport).layout(dom, styles)
}

impl block::Document {
    /// The box of a specific element, if it generated one.
    pub fn find(&self, node: NodeId) -> Option<&LayoutBox> {
        find_box(&self.root, node)
    }

    /// Mutable variant (used by tests and by script-driven relayout).
    pub fn find_mut(&mut self, node: NodeId) -> Option<&mut LayoutBox> {
        find_box_mut(&mut self.root, node)
    }

    /// All boxes of every element with this tag (document order).
    pub fn boxes_of_tag(&self, dom: &Dom, tag: &str) -> Vec<&LayoutBox> {
        let mut out = Vec::new();
        collect_tag(&self.root, dom, tag, &mut out);
        out
    }

    /// Total content size (`scroll_size` in today's CSS terms).
    pub fn content_size(&self) -> Size {
        self.scroll_size
    }

    /// Pretty box tree (`tiny --layout`).
    pub fn debug_tree(&self, dom: &Dom) -> String {
        let mut out = String::new();
        self.root.debug_dump(dom, 0, &mut out);
        out
    }

    /// Visible text in reading order (layout-aware; `display:none` is gone).
    pub fn text(&self) -> String {
        let mut runs: Vec<&TextRun> = Vec::new();
        self.root.collect_runs(&mut runs);
        runs.sort_by(|a, b| {
            a.baseline
                .partial_cmp(&b.baseline)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal))
        });
        let mut out = String::new();
        let mut last_baseline = f32::MIN;
        for run in runs {
            if (run.baseline - last_baseline).abs() > 1.0 {
                if !out.is_empty() {
                    out.push('\n');
                }
                last_baseline = run.baseline;
            } else if !out.is_empty() && !out.ends_with(' ') {
                out.push(' ');
            }
            out.push_str(&run.text);
        }
        out
    }
}

pub fn find_box<'a>(root: &'a LayoutBox, node: NodeId) -> Option<&'a LayoutBox> {
    if root.node == Some(node) {
        return Some(root);
    }
    for c in &root.children {
        if let Some(b) = find_box(c, node) {
            return Some(b);
        }
    }
    None
}

pub fn find_box_mut<'a>(root: &'a mut LayoutBox, node: NodeId) -> Option<&'a mut LayoutBox> {
    if root.node == Some(node) {
        return Some(root);
    }
    for c in root.children.iter_mut() {
        if let Some(b) = find_box_mut(c, node) {
            return Some(b);
        }
    }
    None
}

fn collect_tag<'a>(root: &'a LayoutBox, dom: &Dom, tag: &str, out: &mut Vec<&'a LayoutBox>) {
    if let Some(n) = root.node {
        if dom.tag(n) == Some(tag) {
            out.push(root);
        }
    }
    for c in &root.children {
        collect_tag(c, dom, tag, out);
    }
}
