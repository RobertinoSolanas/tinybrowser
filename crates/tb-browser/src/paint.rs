//! Painting: the box tree becomes pixels.
//!
//! Deliberately dumb — no compositing, no layers, no anti-aliasing. What it
//! does get right is the order that makes a page readable:
//!
//! * backgrounds and borders bottom-up, then text on top,
//! * negative `z-index` first, `position: fixed` painted as if the page were
//!   not scrolled (the only scroll-awareness a screenshot needs),
//! * `overflow: hidden|scroll|auto` clips its subtree to the padding box,
//! * `visibility: hidden` paints nothing but still lets a descendant say
//!   `visibility: visible` — CSS, not an approximation,
//! * a decoded `<img>` is blitted into its content box (`object-fit` is not
//!   modelled: the picture fills the box the layout made), and everything we
//!   cannot rasterise — `<video>`, `<canvas>`, a JPEG, a 404 — gets the same
//!   grey placeholder with its `alt` text that a browser shows when the
//!   picture never arrived.

use std::collections::HashMap;
use std::rc::Rc;

use tb_core::{Color, Rect};
use tb_dom::{Dom, NodeId};
use tb_gfx::draw::paint_placeholder;
use tb_gfx::{BoxPaint, Dash, Framebuffer, Image, TextPaint};
use tb_layout::{BoxKind, LayoutBox, LayoutDocument};
use tb_style::{BorderStyle, Overflow, Position};

/// How to rasterise a laid-out document.
#[derive(Debug, Clone)]
pub struct PaintOptions {
    /// Device pixels per CSS pixel.
    pub scale: f32,
    /// Scroll offset of the viewport (CSS px).
    pub scroll_x: f32,
    pub scroll_y: f32,
    /// Canvas colour painted before anything else.
    pub background: Color,
    /// Outline every box (a debug view worth having when geometry is wrong).
    pub show_boxes: bool,
    /// Include text at all (`--no-text`).
    pub draw_text: bool,
}

impl Default for PaintOptions {
    fn default() -> Self {
        PaintOptions {
            scale: 1.0,
            scroll_x: 0.0,
            scroll_y: 0.0,
            background: Color::WHITE,
            show_boxes: false,
            draw_text: true,
        }
    }
}

/// Paint a laid-out document without pictures (placeholders only).
pub fn paint(doc: &LayoutDocument, dom: &Dom, opts: &PaintOptions) -> Framebuffer {
    paint_with_images(doc, dom, opts, &HashMap::new())
}

/// Paint a laid-out document. `images` maps an `<img>` node to its decoded
/// picture — [`crate::images::load_images`] builds it, the layout already
/// knew its natural size.
pub fn paint_with_images(
    doc: &LayoutDocument,
    dom: &Dom,
    opts: &PaintOptions,
    images: &HashMap<NodeId, Rc<Image>>,
) -> Framebuffer {
    let scale = opts.scale.max(0.01);
    let w = (doc.viewport.width * scale).round().max(1.0) as u32;
    let h = (doc.viewport.height * scale).round().max(1.0) as u32;
    let mut fb = Framebuffer::new(w, h);
    fb.clear(opts.background);
    let ctx = Ctx {
        scale,
        scroll_x: opts.scroll_x,
        scroll_y: opts.scroll_y,
        show_boxes: opts.show_boxes,
        draw_text: opts.draw_text,
        images,
    };
    // the root's own background belongs to the canvas, skip its border
    paint_box(&mut fb, dom, &doc.root, None, &ctx, true);
    if opts.show_boxes {
        fb.stroke_rect(
            Rect::new(0.0, 0.0, w as f32, h as f32),
            [1.0; 4],
            [Color::rgb(200, 40, 40); 4],
            Dash::Solid,
            None,
        );
    }
    fb
}

struct Ctx<'a> {
    scale: f32,
    scroll_x: f32,
    scroll_y: f32,
    show_boxes: bool,
    draw_text: bool,
    images: &'a HashMap<NodeId, Rc<Image>>,
}

impl Ctx<'_> {
    /// Document coordinates → device pixels. Fixed boxes ignore the scroll.
    fn rect(&self, r: Rect, fixed: bool) -> Rect {
        let dy = if fixed { 0.0 } else { self.scroll_y };
        let dx = if fixed { 0.0 } else { self.scroll_x };
        Rect::new(
            (r.x - dx) * self.scale,
            (r.y - dy) * self.scale,
            r.width * self.scale,
            r.height * self.scale,
        )
    }

    fn point(&self, x: f32, y: f32, fixed: bool) -> (f32, f32) {
        let dy = if fixed { 0.0 } else { self.scroll_y };
        let dx = if fixed { 0.0 } else { self.scroll_x };
        ((x - dx) * self.scale, (y - dy) * self.scale)
    }
}

fn paint_box(
    fb: &mut Framebuffer,
    dom: &Dom,
    b: &LayoutBox,
    clip: Option<Rect>,
    ctx: &Ctx,
    is_root: bool,
) {
    let st = &b.style;
    if st.opacity <= 0.001 {
        return;
    }
    let fixed = st.position == Position::Fixed;
    let hidden = !st.visibility;

    if !hidden {
        let border_box = ctx.rect(b.border_box(), fixed);
        let paint = box_paint(st);
        if is_root {
            // the document's background already is the canvas colour
            let _ = paint;
        } else {
            paint.paint(fb, border_box, clip);
        }
    }

    // children are clipped by a scroll container's padding box
    let child_clip = match (st.overflow_x, st.overflow_y) {
        (Overflow::Visible, Overflow::Visible) => clip,
        _ => {
            let pb = ctx.rect(b.padding_box(), fixed);
            Some(match clip {
                Some(c) => c.intersect(pb),
                None => pb,
            })
        }
    };

    // negative z-index first, then everything in tree order (which is paint
    // order for static and positioned boxes without an explicit z-index)
    let mut negatives: Vec<&LayoutBox> = Vec::new();
    let mut rest: Vec<&LayoutBox> = Vec::new();
    for child in &b.children {
        match child.style.z_index {
            Some(z) if z < 0 => negatives.push(child),
            _ => rest.push(child),
        }
    }
    for child in negatives.iter().chain(rest.iter()) {
        paint_box(fb, dom, child, child_clip, ctx, false);
    }

    if !hidden {
        for line in &b.lines {
            for run in &line.runs {
                paint_run(fb, run, child_clip, ctx);
            }
        }
        // a picture we have goes in the content box (padding and border stay
        // its own); anything we could not rasterise takes the placeholder
        let picture = b.node.and_then(|n| ctx.images.get(&n));
        if let Some(img) = picture {
            let r = ctx.rect(b.content, fixed);
            if r.width > 0.5 && r.height > 0.5 {
                tb_gfx::image_blit(fb, img, r, child_clip);
            }
        } else if wants_placeholder(dom, b) {
            let r = ctx.rect(b.border_box(), fixed);
            let alt = b
                .node
                .and_then(|n| dom.element(n))
                .and_then(|e| e.attr("alt"))
                .unwrap_or("");
            paint_placeholder(fb, r, alt, (12.0 * ctx.scale).max(5.0), child_clip);
        }
    }

    if ctx.show_boxes && b.node.is_some() {
        let r = ctx.rect(b.border_box(), fixed);
        if !r.is_empty() {
            fb.stroke_rect(
                r,
                [1.0; 4],
                [Color::rgb(255, 0, 128); 4],
                Dash::Dotted,
                clip,
            );
        }
    }
}

fn paint_run(fb: &mut Framebuffer, run: &tb_layout::TextRun, clip: Option<Rect>, ctx: &Ctx) {
    if !ctx.draw_text || run.text.trim().is_empty() {
        return;
    }
    let st = &run.style;
    if !st.visibility {
        return;
    }
    let (x, baseline) = ctx.point(run.x, run.baseline, false);
    let paint = TextPaint {
        color: st.color,
        size: (st.font_size * ctx.scale).max(1.0),
        bold: st.font_weight >= 600,
        italic: st.italic,
        underline: st.text_decoration_line,
        strike: false,
        letter_spacing: st.letter_spacing * ctx.scale,
    };
    paint.draw(fb, x, baseline, &run.text, clip);
}

/// Map the computed box decoration onto `tb_gfx`'s paint struct.
fn box_paint(st: &tb_style::ComputedStyle) -> BoxPaint {
    let mut widths = [0.0f32; 4];
    let mut colors = [Color::TRANSPARENT; 4];
    let mut dash = Dash::Solid;
    for i in 0..4 {
        if st.border_style[i].painted() {
            widths[i] = st.border_width[i];
            colors[i] = st.border_color[i];
            dash = match st.border_style[i] {
                BorderStyle::Dashed => Dash::Dashed,
                BorderStyle::Dotted => Dash::Dotted,
                BorderStyle::Double => Dash::Double,
                _ => Dash::Solid,
            };
        }
    }
    BoxPaint {
        background: st.background_color,
        border_widths: widths,
        border_colors: colors,
        border_dash: dash,
        radius: st.border_radius[0],
    }
}

/// Boxes that stand in for content we cannot rasterise: no image decoding
/// here, so a picture becomes the grey placeholder with its `alt` text.
fn wants_placeholder(dom: &Dom, b: &LayoutBox) -> bool {
    if b.kind != BoxKind::AtomicInline {
        return false;
    }
    if b.lines.iter().any(|l| !l.runs.is_empty()) {
        return false;
    }
    matches!(
        b.node.and_then(|n| dom.tag(n)),
        Some("img")
            | Some("video")
            | Some("canvas")
            | Some("embed")
            | Some("iframe")
            | Some("audio")
            | Some("object")
    )
}
