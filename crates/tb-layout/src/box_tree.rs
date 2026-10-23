//! The box tree: what the DOM looks like once `display` is applied.
//!
//! We build a *fragment tree* the way a real engine does, only smaller:
//!
//! * `display: none` subtrees are dropped entirely,
//! * block-level children of a block become child boxes,
//! * consecutive inline-level children (elements and text) are grouped into one
//!   anonymous *inline container* which the inline pass turns into line boxes,
//! * mixed content (block *and* inline children, e.g. taz's `<p>text<div>`)
//!   gets anonymous block wrappers so the inline runs keep their order,
//! * `inline-block` / replaced elements are *atomic*: they take part in line
//!   boxes but lay themselves out as small block roots.

use std::rc::Rc;
use tb_core::{Rect, Sides};
use tb_dom::{Dom, NodeId};
use tb_style::{ComputedStyle, Display, StyleMap};

/// What kind of box this is after `display` mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoxKind {
    /// Block container (may have block children and/or one inline container).
    Block,
    /// The `<body>`/root block of the document.
    Root,
    /// Anonymous block generated to keep inline and block children ordered.
    AnonymousBlock,
    /// A run of inline content that the inline pass lays out into line boxes.
    InlineContainer,
    /// Text inside an inline container.
    Text,
    /// Atomic inline element (`inline-block`, `img`, `input`, …).
    AtomicInline,
    /// Flex container.
    Flex,
    /// Table row group / row / cell — laid out in a simplified grid.
    Table,
    TableRow,
    TableCell,
}

impl BoxKind {
    pub fn is_block(&self) -> bool {
        matches!(
            self,
            BoxKind::Block
                | BoxKind::Root
                | BoxKind::AnonymousBlock
                | BoxKind::Flex
                | BoxKind::Table
                | BoxKind::TableRow
        )
    }
}

/// One painted line of text inside an inline container.
#[derive(Debug, Clone)]
pub struct LineBox {
    /// Line box rectangle in document coordinates.
    pub rect: Rect,
    /// Baseline of this line (text is drawn on it).
    pub baseline: f32,
    /// The runs of the line, in visual order.
    pub runs: Vec<TextRun>,
}

/// A run of text drawn with one style.
#[derive(Debug, Clone)]
pub struct TextRun {
    pub node: NodeId,
    pub x: f32,
    pub baseline: f32,
    pub width: f32,
    pub text: String,
    pub style: Rc<ComputedStyle>,
}

/// A laid out box. Geometry is in **document coordinates** (not relative to the
/// parent), which keeps painting and hit testing trivial.
#[derive(Debug, Clone)]
pub struct LayoutBox {
    pub kind: BoxKind,
    /// `None` for anonymous boxes.
    pub node: Option<NodeId>,
    pub style: Rc<ComputedStyle>,
    pub children: Vec<LayoutBox>,
    /// Content rectangle (inside padding and border).
    pub content: Rect,
    /// Resolved box decoration around the content rect.
    pub margin: Sides,
    pub padding: Sides,
    pub border: Sides,
    /// True when this block container's children are inline-level, i.e. it
    /// breaks lines itself. Mixed content gets anonymous block wrappers.
    pub inline_flow: bool,
    /// Line boxes when this is an inline container.
    pub lines: Vec<LineBox>,
    /// Size of an atomic inline (`img`, `inline-block`) or explicit height.
    pub measured_height: f32,
    /// Content height of a scroll container (>= visible height).
    pub scroll_extent: f32,
}

impl LayoutBox {
    /// Stand-in while a box is being re-laid out (we move values around).
    pub(crate) fn placeholder(style: Rc<ComputedStyle>) -> LayoutBox {
        LayoutBox::new(BoxKind::Block, None, style)
    }

    /// Shift this box and everything inside it (relative positioning, moving
    /// an atomic inline onto its line).
    pub fn translate_by(&mut self, dx: f32, dy: f32) {
        if dx == 0.0 && dy == 0.0 {
            return;
        }
        self.content = self.content.translated(dx, dy);
        for line in self.lines.iter_mut() {
            line.rect = line.rect.translated(dx, dy);
            line.baseline += dy;
            for run in line.runs.iter_mut() {
                run.x += dx;
                run.baseline += dy;
            }
        }
        for c in self.children.iter_mut() {
            c.translate_by(dx, dy);
        }
    }

    fn new(kind: BoxKind, node: Option<NodeId>, style: Rc<ComputedStyle>) -> LayoutBox {
        LayoutBox {
            kind,
            node,
            style,
            children: Vec::new(),
            content: Rect::ZERO,
            margin: Sides::ZERO,
            padding: Sides::ZERO,
            border: Sides::ZERO,
            inline_flow: false,
            lines: Vec::new(),
            measured_height: 0.0,
            scroll_extent: 0.0,
        }
    }

    /// Border box = content + padding + border.
    pub fn border_box(&self) -> Rect {
        Rect::new(
            self.content.x - self.border.left - self.padding.left,
            self.content.y - self.border.top - self.padding.top,
            self.content.width + self.padding.horizontal() + self.border.horizontal(),
            self.content.height + self.padding.vertical() + self.border.vertical(),
        )
    }

    /// Padding box (the clip rect for `overflow: hidden`).
    pub fn padding_box(&self) -> Rect {
        let b = self.border_box();
        Rect::new(
            b.x + self.border.left,
            b.y + self.border.top,
            b.width - self.border.horizontal(),
            b.height - self.border.vertical(),
        )
    }

    pub fn is_text(&self) -> bool {
        self.kind == BoxKind::Text
    }

    /// Deepest element box containing the point (hit testing helper).
    pub fn hit(&self, x: f32, y: f32) -> Option<(NodeId, Rect)> {
        // children in reverse paint order first
        for child in self.children.iter().rev() {
            if let Some(hit) = child.hit(x, y) {
                return Some(hit);
            }
        }
        let box_rect = match self.kind {
            BoxKind::Text => Rect::new(
                self.content.x,
                self.content.y,
                self.content.width.max(0.0),
                self.content.height.max(1.0),
            ),
            _ => self.border_box(),
        };
        if box_rect.contains(tb_core::Point::new(x, y)) {
            if let Some(node) = self.node {
                return Some((node, box_rect));
            }
        }
        None
    }

    /// All text runs inside this subtree (used by `--text` and painting).
    pub fn collect_runs<'a>(&'a self, out: &mut Vec<&'a TextRun>) {
        for line in &self.lines {
            for run in &line.runs {
                out.push(run);
            }
        }
        for c in &self.children {
            c.collect_runs(out);
        }
    }

    /// Indented debug dump (`tiny --layout`).
    pub fn debug_dump(&self, dom: &Dom, indent: usize, out: &mut String) {
        let pad = " ".repeat(indent);
        let name = match self.node {
            Some(n) => dom
                .tag(n)
                .map(|t| format!("<{t}>"))
                .unwrap_or_else(|| "#text".to_string()),
            None => match self.kind {
                BoxKind::AnonymousBlock => "«anonymous»".to_string(),
                BoxKind::InlineContainer => "«inline»".to_string(),
                _ => "«block»".to_string(),
            },
        };
        if self.kind == BoxKind::InlineContainer {
            out.push_str(&format!(
                "{pad}{name} {} lines, {}×{}\n",
                self.lines.len(),
                self.content.width.round(),
                self.content.height.round()
            ));
        } else {
            let b = self.border_box();
            out.push_str(&format!(
                "{pad}{name} {}×{} @ {},{} [{}]\n",
                b.width.round(),
                b.height.round(),
                b.x.round(),
                b.y.round(),
                self.style.summary()
            ));
        }
        for c in &self.children {
            c.debug_dump(dom, indent + 2, out);
        }
    }
}

/// Build the box tree from DOM + computed styles.
pub fn build(dom: &Dom, styles: &StyleMap) -> Option<LayoutBox> {
    let html = dom.get_elements_by_tag("html").into_iter().next()?;
    let style = styles.rc(html)?;
    let mut root = LayoutBox::new(BoxKind::Root, Some(html), style);
    build_children(dom, styles, html, &mut root);
    Some(root)
}

fn build_children(dom: &Dom, styles: &StyleMap, parent: NodeId, out: &mut LayoutBox) {
    let parent_style = out.style.clone();
    let is_flex = parent_style.display.is_flex();

    enum Pending {
        Inline(LayoutBox),
        Block(LayoutBox),
    }
    let mut pending: Vec<Pending> = Vec::new();

    for child in dom.children(parent) {
        match styles.rc(child) {
            Some(style) => {
                if style.display == Display::None {
                    continue;
                }
                let kind = classify(&style, dom, child);
                let mut built = LayoutBox::new(kind, Some(child), style.clone());
                build_children(dom, styles, child, &mut built);
                let inline_level = style.display.is_inline();
                if !is_flex && inline_level {
                    pending.push(Pending::Inline(built));
                } else {
                    pending.push(Pending::Block(built));
                }
            }
            None => {
                // text and comment nodes have no style of their own
                let text = match dom.get(child) {
                    Some(node) if node.is_text() => node.text().unwrap_or("").to_string(),
                    _ => continue,
                };
                if text.trim().is_empty() && !parent_style.white_space.preserves() {
                    continue;
                }
                let mut b = LayoutBox::new(BoxKind::Text, Some(child), parent_style.clone());
                b.content.height = parent_style.line_box();
                pending.push(Pending::Inline(b));
            }
        }
    }

    let inline_only = pending.iter().all(|p| matches!(p, Pending::Inline(_)));
    let any_inline = pending.iter().any(|p| matches!(p, Pending::Inline(_)));

    if is_flex || !any_inline {
        out.children = pending
            .into_iter()
            .map(|p| match p {
                Pending::Inline(b) | Pending::Block(b) => b,
            })
            .collect();
        return;
    }

    if inline_only {
        // a block container with only inline children lays out the lines itself
        out.inline_flow = true;
        out.children = pending
            .into_iter()
            .map(|p| match p {
                Pending::Inline(b) | Pending::Block(b) => b,
            })
            .collect();
        return;
    }

    // mixed content: wrap each run of inline level children in an anonymous block
    let mut children: Vec<LayoutBox> = Vec::new();
    let mut run: Vec<LayoutBox> = Vec::new();
    let flush = |run: &mut Vec<LayoutBox>, children: &mut Vec<LayoutBox>| {
        if run.is_empty() {
            return;
        }
        let kids = std::mem::take(run);
        let style = kids[0].style.clone();
        let mut anon = LayoutBox::new(BoxKind::InlineContainer, None, style);
        anon.inline_flow = true;
        anon.children = kids;
        children.push(anon);
    };
    for p in pending {
        match p {
            Pending::Inline(b) => run.push(b),
            Pending::Block(b) => {
                flush(&mut run, &mut children);
                children.push(b);
            }
        }
    }
    flush(&mut run, &mut children);
    out.children = children;
}

/// Size of a replaced element / inline-block.
///
/// Precedence like a browser: CSS `width`/`height`, else the `width`/`height`
/// attributes as intrinsic size, then `min`/`max` clamping. With `height: auto`
/// and a known intrinsic ratio the ratio is preserved — which is what keeps
/// responsive images from turning into 300×150 bricks.
pub fn replaced_size(
    box_: &LayoutBox,
    style: &ComputedStyle,
    containing: Rect,
    viewport_height: f32,
    height_definite: bool,
    dom: &Dom,
    intrinsic: Option<(f32, f32)>,
) -> (f32, f32) {
    use tb_style::{Display, L};
    let attr = |name: &str| -> Option<f32> {
        box_.node.and_then(|n| {
            dom.attr(n, name).and_then(|v| {
                let v = v.trim().trim_end_matches("px");
                if let Some(pct) = v.strip_suffix('%') {
                    pct.parse::<f32>()
                        .ok()
                        .map(|p| containing.width * p / 100.0)
                } else {
                    v.parse::<f32>().ok()
                }
            })
        })
    };
    let tag = box_.node.and_then(|n| dom.tag(n)).unwrap_or("");
    let replaced = is_replaced(tag);
    // A decoded picture *is* the intrinsic size. The `width`/`height`
    // attributes are the page's guess about it — used when the picture is not
    // there (we do not decode JPEG, it 404s, it is lazy-loaded), and ignored
    // in favour of the real thing when we have it.
    let (intrinsic_w, intrinsic_h) = match intrinsic {
        Some((w, h)) if w > 0.0 && h > 0.0 => (Some(w), Some(h)),
        _ => (attr("width"), attr("height")),
    };
    let ratio = match (intrinsic_w, intrinsic_h) {
        (Some(w), Some(h)) if h > 0.0 && w > 0.0 => Some(w / h),
        _ => None,
    };
    let (def_w, def_h) = if replaced {
        (300.0f32, 150.0f32)
    } else {
        (0.0, 0.0)
    };
    let extra_h = box_.padding.horizontal() + box_.border.horizontal();

    let mut w = match style.width {
        L::Auto => intrinsic_w.unwrap_or(def_w),
        other => {
            let mut v = other.resolve(containing.width);
            if style.box_sizing_border {
                v -= extra_h;
            }
            v
        }
    };
    w = w
        .min(match style.max_width {
            L::Auto => f32::INFINITY,
            other => other.resolve(containing.width),
        })
        .max(match style.min_width {
            L::Auto => 0.0,
            other => other.resolve(containing.width),
        })
        .max(0.0);

    // A percentage height needs a definite containing block; without one CSS
    // falls back to `auto` — which for images means "keep the aspect ratio".
    let height_is_auto = match style.height {
        L::Auto => true,
        other => !(other.is_definite() || height_definite),
    };
    let mut h = if height_is_auto {
        match ratio {
            Some(r) if r > 0.0 => {
                (w - if style.box_sizing_border {
                    extra_h
                } else {
                    0.0
                }) / r
            }
            _ => intrinsic_h.unwrap_or(def_h),
        }
    } else {
        let other = style.height;
        let mut v = other.resolve(containing.height.max(viewport_height));
        if style.box_sizing_border {
            v -= box_.padding.vertical() + box_.border.vertical();
        }
        v
    };
    h = h
        .min(match style.max_height {
            L::Auto => f32::INFINITY,
            other => other.resolve(containing.height.max(viewport_height)),
        })
        .max(match style.min_height {
            L::Auto => 0.0,
            other if other.is_definite() || height_definite => {
                other.resolve(containing.height.max(viewport_height))
            }
            _ => 0.0,
        })
        .max(0.0);

    if !replaced && w == 0.0 && h == 0.0 {
        // an `inline-block` with no size: shrink to its content (one line for now)
        h = if style.display == Display::InlineBlock {
            style.line_box()
        } else {
            h
        };
    }
    (w, h)
}

/// Intrinsic width of a replaced element: CSS `width` if absolute, otherwise
/// the decoded picture, otherwise the `width` attribute, otherwise the default
/// object size. Percentages are indefinite, so they do not contribute.
pub fn intrinsic_replaced_width(
    box_: &LayoutBox,
    style: &ComputedStyle,
    dom: &Dom,
    intrinsic: Option<(f32, f32)>,
) -> f32 {
    use tb_style::L;
    match style.width {
        L::Px(w) => w,
        L::Auto | L::Linear { .. } => {
            if let Some((w, _)) = intrinsic {
                if w > 0.0 {
                    return w;
                }
            }
            let tag = box_.node.and_then(|n| dom.tag(n)).unwrap_or("");
            let def = if is_replaced(tag) { 300.0 } else { 0.0 };
            box_.node
                .and_then(|n| dom.attr(n, "width"))
                .and_then(|v| v.trim().trim_end_matches("px").parse::<f32>().ok())
                .unwrap_or(def)
        }
    }
}

/// Natural sizes the embedder resolved for replaced elements — decoded pictures
/// keyed by the `<img>` that points at them. Layout knows *that* a picture is
/// 640×360; only the embedder knows *which* picture and what it turned into.
#[derive(Debug, Default, Clone)]
pub struct IntrinsicSizes {
    sizes: std::collections::HashMap<NodeId, (f32, f32)>,
}

impl IntrinsicSizes {
    pub fn new() -> IntrinsicSizes {
        IntrinsicSizes::default()
    }
    pub fn insert(&mut self, node: NodeId, width: f32, height: f32) {
        self.sizes.insert(node, (width, height));
    }
    pub fn get(&self, node: NodeId) -> Option<(f32, f32)> {
        self.sizes.get(&node).copied()
    }
    pub fn len(&self) -> usize {
        self.sizes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.sizes.is_empty()
    }
}

/// Map `display` onto a box kind (plus "this is an atomic inline object").
fn classify(style: &ComputedStyle, dom: &Dom, node: NodeId) -> BoxKind {
    let tag = dom.tag(node).unwrap_or("");
    if is_replaced(tag)
        || style.display == Display::InlineBlock
        || style.display == Display::InlineFlex
    {
        return BoxKind::AtomicInline;
    }
    match style.display {
        Display::Flex => BoxKind::Flex,
        Display::Table => BoxKind::Table,
        Display::TableRow => BoxKind::TableRow,
        Display::TableRowGroup | Display::TableCell | Display::ListItem | Display::Block => {
            BoxKind::Block
        }
        Display::Inline => BoxKind::Block, // inline elements are handled by their parent's inline run
        _ => BoxKind::Block,
    }
}

/// Elements that render themselves instead of their content.
pub fn is_replaced(tag: &str) -> bool {
    matches!(
        tag,
        "img"
            | "input"
            | "button"
            | "select"
            | "textarea"
            | "video"
            | "canvas"
            | "svg"
            | "progress"
            | "iframe"
            | "audio"
            | "object"
            | "embed"
    )
}
