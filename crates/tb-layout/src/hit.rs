//! Hit testing: which element is at this point?
//!
//! Painting and hit testing walk the same tree; the deepest element whose
//! border box (or text run) contains the point wins, later siblings before
//! earlier ones because they paint on top.

use tb_core::{Point, Rect};
use tb_dom::{Dom, NodeId};

use crate::block::Document;
use crate::box_tree::LayoutBox;

/// What was hit.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub node: NodeId,
    /// The box that was hit (border box of the element).
    pub rect: Rect,
    /// Tag name, for nicer diagnostics.
    pub tag: String,
    /// The text of the run if the point landed on text.
    pub text: Option<String>,
}

/// Deepest **element** at `(x, y)` (what `document.elementFromPoint` returns).
pub fn element_at(doc: &Document, dom: &Dom, x: f32, y: f32) -> Option<NodeId> {
    let hit = hit_test(doc, dom, x, y)?;
    as_element(dom, hit.node)
}

/// Deepest element at `(x, y)`, with details.
pub fn hit_test(doc: &Document, dom: &Dom, x: f32, y: f32) -> Option<Hit> {
    let p = Point::new(x, y);
    let (node, rect, text) = walk(&doc.root, p)?;
    Some(Hit {
        node,
        rect,
        tag: dom.tag(node).unwrap_or("text").to_string(),
        text,
    })
}

fn walk(box_: &LayoutBox, p: Point) -> Option<(NodeId, Rect, Option<String>)> {
    if box_.style.display == tb_style::Display::None {
        return None;
    }
    // text runs first: they are the topmost content of a line
    for line in &box_.lines {
        if !line.rect.contains(p) {
            continue;
        }
        for run in line.runs.iter().rev() {
            let r = Rect::new(
                run.x,
                line.baseline - ascent(&run.style),
                run.width,
                run.style.line_box(),
            );
            if r.contains(p) {
                if let Some(node) = run_text_owner(box_, run.node, &r) {
                    return Some((node, r, Some(run.text.clone())));
                }
            }
        }
    }
    // then children, in reverse paint order
    for child in box_.children.iter().rev() {
        if let Some(found) = walk(child, p) {
            return Some(found);
        }
    }
    // finally ourselves
    let bb = box_.border_box();
    if !box_.inline_flow && bb.contains(p) {
        if let Some(node) = box_.node {
            return Some((node, bb, None));
        }
    }
    None
}

/// Text ascent for hit boxes: 80 % of the font size, like our rasterizer's
/// 6-of-7-row baseline (kept local so `tb-style` stays free of graphics code).
pub fn ascent(style: &tb_style::ComputedStyle) -> f32 {
    style.font_size * 0.85
}

/// A text run belongs to its own text node for DOM purposes, but callers
/// usually want the *element*; we return the text node and let the caller walk
/// up if it wants an element (that is what `document.elementFromPoint` does).
fn run_text_owner(_owner: &LayoutBox, text_node: NodeId, _rect: &Rect) -> Option<NodeId> {
    Some(text_node)
}

/// The nearest ancestor element (or the node itself when it is an element).
pub fn as_element(dom: &Dom, node: NodeId) -> Option<NodeId> {
    let mut cur = Some(node);
    while let Some(c) = cur {
        if dom.tag(c).is_some() {
            return Some(c);
        }
        cur = dom.parent(c);
    }
    None
}

/// The closest `<a href>` around a point — how a browser decides a click is a
/// navigation.
pub fn link_at(doc: &Document, dom: &Dom, x: f32, y: f32) -> Option<String> {
    let hit = hit_test(doc, dom, x, y)?;
    let mut cur = as_element(dom, hit.node);
    while let Some(c) = cur {
        if dom.tag(c) == Some("a") {
            if let Some(href) = dom.attr(c, "href") {
                return Some(href);
            }
        }
        cur = dom.parent(c);
    }
    None
}
