//! Inline layout: the line breaker.
//!
//! Browsers don't lay out "an inline element"; they build a *flow* of inline
//! content and break it into line boxes. We do the same:
//!
//! 1. **flatten** the inline subtree into tokens (words, collapsible spaces,
//!    forced breaks, atomic boxes, floats) — a `<b>` inside a paragraph
//!    contributes its words to the paragraph's lines, in its own style
//! 2. **greedy line breaking**: place tokens until the next one doesn't fit,
//!    then start a new line
//! 3. **finalize** each line: baseline from the tallest fragment, `text-align`,
//!    justified spacing, float insets from [`FloatState`]
//!
//! Measurement goes through [`tb_gfx::Font`], the same rasterizer that paints —
//! so text never overflows a line box because metrics disagreed.

use tb_core::Rect;
use tb_dom::{Dom, NodeId};
use tb_gfx::Font;
use tb_style::{ComputedStyle, TextAlign, WhiteSpace};

use crate::box_tree::{BoxKind, LayoutBox, LineBox, TextRun};

/// Which side of a float we are talking about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

/// A float in the current block formatting context.
#[derive(Debug, Clone, Copy)]
pub struct FloatRect {
    pub rect: Rect,
    pub side: Side,
}

/// Float state shared by all line boxes of one block formatting context.
#[derive(Debug, Clone, Default)]
pub struct FloatState {
    floats: Vec<FloatRect>,
}

impl FloatState {
    /// Register a float.
    pub fn add(&mut self, rect: Rect, side: Side) {
        self.floats.push(FloatRect { rect, side });
    }

    /// Horizontal insets at a given vertical band, relative to `rect`.
    pub fn insets(&self, top: f32, bottom: f32, rect: Rect) -> (f32, f32) {
        let mut left = 0.0f32;
        let mut right = 0.0f32;
        for f in &self.floats {
            if f.rect.bottom() <= top || f.rect.y >= bottom {
                continue;
            }
            match f.side {
                Side::Left => left = left.max(f.rect.right() - rect.x),
                Side::Right => right = right.max(rect.right() - f.rect.x),
            }
        }
        (left.max(0.0), right.max(0.0))
    }

    /// Where the next float on `side` may be placed (y of first free slot).
    pub fn next_float_y(&self, mut y: f32, side: Side, _rect: Rect) -> f32 {
        loop {
            let mut moved = false;
            for f in &self.floats {
                if f.side != side {
                    continue;
                }
                if f.rect.bottom() <= y || f.rect.y >= y + 1.0 {
                    continue;
                }
                y = f.rect.bottom();
                moved = true;
            }
            if !moved {
                return y;
            }
        }
    }

    /// Horizontal start position for a float on `side` at `y`.
    pub fn next_float_x(&self, y: f32, side: Side, rect: Rect) -> f32 {
        let y = self.next_float_y(y, side, rect);
        let mut x = match side {
            Side::Left => rect.x,
            Side::Right => rect.right(),
        };
        for f in &self.floats {
            if f.rect.bottom() <= y || f.rect.y >= y + 1.0 {
                continue;
            }
            match side {
                Side::Left => x = x.max(f.rect.right()),
                Side::Right => x = x.min(f.rect.x),
            }
        }
        x
    }

    /// `clear: both` — nothing flows around floats any more below this point.
    pub fn clear_all(&mut self) {
        self.floats.clear();
    }

    /// Lowest bottom edge of all floats (used for `clear`).
    pub fn lowest(&self, side: Option<Side>) -> f32 {
        self.floats
            .iter()
            .filter(|f| side.map(|s| f.side == s).unwrap_or(true))
            .map(|f| f.rect.bottom())
            .fold(0.0f32, f32::max)
    }
}

// ------------------------------------------------------------------ tokens --

/// One unit of inline content after flattening.
#[derive(Debug, Clone)]
enum Flow {
    /// A word (never broken here, except when wider than the line).
    Word {
        node: NodeId,
        style: std::rc::Rc<ComputedStyle>,
        text: String,
        width: f32,
    },
    /// One collapsible gap between words.
    Space {
        node: NodeId,
        style: std::rc::Rc<ComputedStyle>,
        width: f32,
    },
    /// `<br>` or a newline that must break the line.
    Break { style: std::rc::Rc<ComputedStyle> },
    /// `inline-block` / replaced element: laid out as a block, placed as a box.
    /// The path addresses the box inside the container's tree, because nested
    /// inline elements contribute their atomics to this flow.
    Atomic { path: Vec<usize> },
    /// A floated inline-level box: taken out of the line, other text flows around it.
    Float { path: Vec<usize>, side: Side },
}

fn tokenise(children: &[LayoutBox], dom: &Dom, font: &Font, path: &[usize], flows: &mut Vec<Flow>) {
    for (index, child) in children.iter().enumerate() {
        let mut path = path.to_vec();
        path.push(index);
        let style = &child.style;
        if style.display == tb_style::Display::None {
            continue;
        }
        if style.float_left || style.float_right {
            flows.push(Flow::Float {
                path,
                side: if style.float_left {
                    Side::Left
                } else {
                    Side::Right
                },
            });
            continue;
        }
        if is_break(dom, child) && child.node.is_some() {
            flows.push(Flow::Break {
                style: style.clone(),
            });
            continue;
        }
        match child.kind {
            BoxKind::Text => {
                let Some(node) = child.node else { continue };
                let raw = dom.text(node).unwrap_or_default();
                push_text(&raw, node, style, font, flows);
            }
            BoxKind::AtomicInline => flows.push(Flow::Atomic { path }),
            // nested inline element: its content joins this flow
            _ => tokenise(&child.children, dom, font, &path, flows),
        }
    }
}

/// `<br>` is a forced break even though its `display` is `inline`.
fn is_break(dom: &Dom, child: &LayoutBox) -> bool {
    child.node.and_then(|n| dom.tag(n)) == Some("br")
}

fn push_text(
    raw: &str,
    node: NodeId,
    style: &std::rc::Rc<ComputedStyle>,
    font: &Font,
    flows: &mut Vec<Flow>,
) {
    let ws = style.white_space;
    let size = style.font_size.max(4.0);
    let bold = style.font_weight >= 600;
    let ls = style.letter_spacing;
    let width_of = |s: &str| font.text_width(s, size, bold, ls);

    if ws.preserves() {
        // white-space: pre | pre-wrap | break-spaces — newlines stay
        let mut line = String::new();
        for ch in raw.chars() {
            if ch == '\n' {
                if !line.is_empty() {
                    push_word(&line, node, style, width_of(&line), flows);
                    line.clear();
                }
                flows.push(Flow::Break {
                    style: style.clone(),
                });
            } else if ws == WhiteSpace::Pre {
                line.push(ch); // kept verbatim, never breaks
            } else {
                line.push(ch);
            }
        }
        if !line.is_empty() {
            push_word(&line, node, style, width_of(&line), flows);
        }
        return;
    }

    // collapsing mode: runs of whitespace collapse to one break opportunity
    let mut word = String::new();
    for ch in raw.chars() {
        if is_space(ch) {
            if !word.is_empty() {
                push_word(&word, node, style, width_of(&word), flows);
                word.clear();
            }
            if ws != WhiteSpace::Nowrap {
                flows.push(Flow::Space {
                    node,
                    style: style.clone(),
                    width: width_of(" "),
                });
            }
        } else {
            word.push(ch);
        }
    }
    if !word.is_empty() {
        push_word(&word, node, style, width_of(&word), flows);
    }
}

fn push_word(
    text: &str,
    node: NodeId,
    style: &std::rc::Rc<ComputedStyle>,
    _width: f32,
    flows: &mut Vec<Flow>,
) {
    let text = transform(text, style.text_transform);
    if text.is_empty() {
        return;
    }
    let width = Font::system().text_width(
        &text,
        style.font_size.max(4.0),
        style.font_weight >= 600,
        style.letter_spacing,
    );
    flows.push(Flow::Word {
        width,
        node,
        style: style.clone(),
        text,
    });
}

fn transform(text: &str, t: tb_style::TextTransform) -> String {
    use tb_style::TextTransform::*;
    match t {
        None => text.to_string(),
        Lowercase => text.to_lowercase(),
        Uppercase => text.to_uppercase(),
        Capitalize => {
            let mut out = String::with_capacity(text.len());
            let mut at_start = true;
            for ch in text.chars() {
                if at_start {
                    out.extend(ch.to_uppercase());
                } else {
                    out.extend(ch.to_lowercase());
                }
                at_start = !ch.is_alphanumeric();
            }
            out
        }
    }
}

fn is_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{b}' | '\u{c}')
}

// ------------------------------------------------------------------ layout --

#[allow(unused_assignments)]
/// Lay out the inline content of `container` into line boxes.
///
/// `rect` is the line container (the container's content box, already shrunk
/// by any float that spans its top). Atomic children are laid out through
/// `layout_atomic` so this module stays free of block layout.
pub fn layout_inline(
    container: &mut LayoutBox,
    rect: Rect,
    dom: &Dom,
    floats: &mut FloatState,
    intrinsic: &crate::box_tree::IntrinsicSizes,
    layout_atomic: &mut dyn FnMut(&mut LayoutBox, Rect),
) {
    let base = container.style.clone();
    let font = Font::system();
    let mut flows: Vec<Flow> = Vec::new();
    tokenise(&container.children, dom, font, &[], &mut flows);

    let mut lines: Vec<LineBox> = Vec::new();
    let mut runs: Vec<TextRun> = Vec::new();
    let mut line_top = rect.y;
    let mut cursor = rect.y;
    let mut pen_x = 0.0f32;
    let mut max_font = 0.0f32;
    let mut max_line_box = 0.0f32;
    let mut pending_space = 0.0f32;
    let mut has_content = false;
    let mut left_inset = 0.0f32;
    let mut right_inset = 0.0f32;

    // a first guess for the line's usable width
    let usable = |f: &FloatState, cursor: f32, font_est: f32| -> (f32, f32, f32) {
        let h = (font_est * 1.2).max(6.0);
        let (l, r) = f.insets(cursor, cursor + h, rect);
        let w = (rect.width - l - r).max(1.0);
        (l, r, w)
    };
    (left_inset, right_inset, _) = usable(floats, cursor, base.font_size);
    pen_x = rect.x + left_inset;

    // a line is never shorter than its own font metrics; `line-height` handles the rest
    let min_height = 0.0f32;

    for flow in flows.into_iter() {
        match flow {
            Flow::Break { style } => {
                max_font = max_font.max(style.font_size);
                max_line_box = max_line_box.max(style.line_box());
                pending_space = 0.0;
                let h = finish_line(
                    &mut runs,
                    line_top,
                    max_font,
                    max_line_box,
                    min_height,
                    left_inset,
                    right_inset,
                    rect,
                    base.text_align,
                    &mut lines,
                );
                cursor = line_top + h;
                line_top = cursor;
                max_font = 0.0;
                max_line_box = 0.0;
                has_content = false;
                (left_inset, right_inset, _) = usable(floats, cursor, max_font.max(base.font_size));
                pen_x = rect.x + left_inset;
            }
            Flow::Space { node, style, width } => {
                max_font = max_font.max(style.font_size);
                max_line_box = max_line_box.max(style.line_box());
                if has_content {
                    pending_space += width;
                }
                let _ = node;
            }
            Flow::Word {
                node,
                style,
                text,
                width,
            } => {
                max_font = max_font.max(style.font_size);
                max_line_box = max_line_box.max(style.line_box());
                let wraps = style.white_space.wraps();
                let fits =
                    !wraps || pen_x + pending_space + width <= rect.right() - right_inset + 0.5;
                if !fits && has_content {
                    // wrap to the next line and try again
                    pending_space = 0.0;
                    let h = finish_line(
                        &mut runs,
                        line_top,
                        max_font,
                        max_line_box,
                        min_height,
                        left_inset,
                        right_inset,
                        rect,
                        base.text_align,
                        &mut lines,
                    );
                    cursor = line_top + h;
                    line_top = cursor;
                    has_content = false;
                    (left_inset, right_inset, _) =
                        usable(floats, cursor, max_font.max(base.font_size));
                    pen_x = rect.x + left_inset;
                }
                if !fits && !has_content && wraps {
                    // a single word wider than the line: break it anywhere
                    let mut rest: &str = &text;
                    let mut w_rest = width;
                    while !rest.is_empty() {
                        let room = (rect.right() - right_inset - pen_x).max(1.0);
                        if w_rest <= room || rest.chars().count() == 1 {
                            let r = TextRun {
                                node,
                                x: pen_x,
                                baseline: 0.0,
                                width: w_rest,
                                text: rest.to_string(),
                                style: style.clone(),
                            };
                            pen_x += w_rest;
                            runs.push(r);
                            has_content = true;
                            break;
                        }
                        // never split a UTF-8 sequence: accumulate whole chars
                        let mut take = 0usize;
                        let mut acc = 0.0f32;
                        for ch in rest.chars() {
                            let cw = font.text_width(
                                &ch.to_string(),
                                style.font_size.max(4.0),
                                style.font_weight >= 600,
                                style.letter_spacing,
                            );
                            if acc + cw > room && take > 0 {
                                break;
                            }
                            acc += cw;
                            take += ch.len_utf8();
                        }
                        let chunk = &rest[..take];
                        runs.push(TextRun {
                            node,
                            x: pen_x,
                            baseline: 0.0,
                            width: acc,
                            text: chunk.to_string(),
                            style: style.clone(),
                        });
                        pen_x += acc;
                        has_content = true;
                        rest = &rest[take..];
                        w_rest -= acc;
                        // chunk fills the line: continue on the next one
                        let h = finish_line(
                            &mut runs,
                            line_top,
                            max_font,
                            max_line_box,
                            min_height,
                            left_inset,
                            right_inset,
                            rect,
                            base.text_align,
                            &mut lines,
                        );
                        cursor = line_top + h;
                        line_top = cursor;
                        (left_inset, right_inset, _) =
                            usable(floats, cursor, max_font.max(base.font_size));
                        pen_x = rect.x + left_inset;
                    }
                    pending_space = 0.0;
                    continue;
                }
                // write the pending space (advance only — spaces are not drawn)
                if pending_space > 0.0 {
                    pen_x += pending_space;
                    pending_space = 0.0;
                }
                runs.push(TextRun {
                    node,
                    x: pen_x,
                    baseline: 0.0,
                    width,
                    text,
                    style: style.clone(),
                });
                pen_x += width;
                has_content = true;
            }
            Flow::Atomic { path } => {
                let style = descend(&container.children, &path).style.clone();
                max_font = max_font.max(style.font_size);
                max_line_box = max_line_box.max(style.line_box());
                let (_, _, avail) = usable(floats, cursor, max_font.max(base.font_size));
                let (w, h) = replaced_size(
                    descend(&container.children, &path),
                    &style,
                    avail,
                    dom,
                    intrinsic,
                );
                if pen_x + pending_space + w > rect.right() - right_inset + 0.5 && has_content {
                    pending_space = 0.0;
                    let lh = finish_line(
                        &mut runs,
                        line_top,
                        max_font,
                        max_line_box,
                        min_height,
                        left_inset,
                        right_inset,
                        rect,
                        base.text_align,
                        &mut lines,
                    );
                    cursor = line_top + lh;
                    line_top = cursor;
                    has_content = false;
                    (left_inset, right_inset, _) =
                        usable(floats, cursor, max_font.max(base.font_size));
                    pen_x = rect.x + left_inset;
                }
                let x = pen_x + if has_content { pending_space } else { 0.0 };
                pending_space = 0.0;
                {
                    let target = descend_mut(&mut container.children, &path);
                    layout_atomic(target, Rect::new(x, line_top, w, h));
                    let bb = target.border_box();
                    target
                        .translate_by(0.0, (line_top + max_line_box.max(bb.height)) - bb.bottom());
                }
                let bb = descend(&container.children, &path).border_box();
                pen_x = x + bb.width;
                has_content = true;
            }
            Flow::Float { path, side } => {
                let style = descend(&container.children, &path).style.clone();
                let fy = floats.next_float_y(line_top.max(cursor), side, rect);
                let fx = floats.next_float_x(fy, side, rect);
                let (_, _, avail) = usable(floats, fy, style.font_size);
                let (w, h) = replaced_size(
                    descend(&container.children, &path),
                    &style,
                    avail,
                    dom,
                    intrinsic,
                );
                {
                    let target = descend_mut(&mut container.children, &path);
                    layout_atomic(target, Rect::new(fx, fy, w, h));
                }
                let bb = descend(&container.children, &path).border_box();
                floats.add(bb, side);
                // the current line must now make room for the float
                (left_inset, right_inset, _) =
                    usable(floats, line_top, max_font.max(base.font_size));
            }
        }
    }
    if !runs.is_empty() || lines.is_empty() {
        finish_line(
            &mut runs,
            line_top,
            max_font.max(base.font_size),
            max_line_box.max(base.line_box()),
            min_height,
            left_inset,
            right_inset,
            rect,
            base.text_align,
            &mut lines,
        );
        cursor = line_top
            + lines
                .last()
                .map(|l| l.rect.height)
                .unwrap_or(base.line_box());
    }

    // text nodes — and the inline elements wrapping them — get their geometry
    // from the runs that were actually placed
    {
        let mut extents: std::collections::HashMap<NodeId, Rect> = std::collections::HashMap::new();
        for line in &lines {
            for run in &line.runs {
                let r = Rect::new(run.x, line.rect.y, run.width, line.rect.height);
                extents
                    .entry(run.node)
                    .and_modify(|e| *e = e.union(r))
                    .or_insert(r);
            }
        }
        assign_extents(&mut container.children, &extents);
    }

    let mut total = cursor - rect.y;
    for f in floats.remaining_in(rect) {
        total = total.max(f.bottom() - rect.y);
    }
    container.lines = lines;
    container.measured_height = total.max(0.0);
    container.content = Rect::new(rect.x, rect.y, rect.width, total.max(0.0));
}

/// Follow a path from `children` down into the box tree.
fn descend<'a>(children: &'a [LayoutBox], path: &[usize]) -> &'a LayoutBox {
    let mut cur = &children[path[0]];
    for &i in &path[1..] {
        cur = &cur.children[i];
    }
    cur
}

fn descend_mut<'a>(children: &'a mut [LayoutBox], path: &[usize]) -> &'a mut LayoutBox {
    let mut cur = &mut children[path[0]];
    for &i in &path[1..] {
        cur = &mut cur.children[i];
    }
    cur
}

/// Bottom-up: every text node takes its run union, inline elements the union of
/// their children (so `<b>` — or a link around an image — knows where its
/// content ended up).
fn assign_extents(
    children: &mut [LayoutBox],
    extents: &std::collections::HashMap<NodeId, Rect>,
) -> Option<Rect> {
    let mut union: Option<Rect> = None;
    for child in children.iter_mut() {
        let own = match child.kind {
            BoxKind::Text => child
                .node
                .and_then(|n| extents.get(&n))
                .copied()
                .or_else(|| assign_extents(&mut child.children, extents)),
            BoxKind::AtomicInline => Some(child.content).filter(|r| !r.is_empty()),
            _ => assign_extents(&mut child.children, extents),
        };
        if let Some(r) = own {
            child.content = r;
            union = Some(match union {
                Some(u) => u.union(r),
                None => r,
            });
        }
    }
    union
}

/// Floats that still overlap the region below `rect`.
trait FloatTail {
    fn remaining_in(&self, rect: Rect) -> Vec<Rect>;
}

impl FloatTail for FloatState {
    fn remaining_in(&self, _rect: Rect) -> Vec<Rect> {
        self.floats.iter().map(|f| f.rect).collect()
    }
}

/// Close a line box: baseline, height, alignment.
#[allow(clippy::too_many_arguments)]
fn finish_line(
    runs: &mut Vec<TextRun>,
    top: f32,
    max_font: f32,
    max_line_box: f32,
    min_height: f32,
    left_inset: f32,
    right_inset: f32,
    rect: Rect,
    align: TextAlign,
    lines: &mut Vec<LineBox>,
) -> f32 {
    let font = max_font.max(8.0);
    let ascent = font * 0.8;
    let descent = font * 0.2;
    let mut height = max_line_box.max(ascent + descent);
    if height < min_height && min_height > 0.0 {
        height = min_height;
    }
    let baseline = top + ascent + (height - (ascent + descent)) / 2.0;
    for run in runs.iter_mut() {
        run.baseline = baseline;
    }
    // alignment: shift the whole run sequence inside the available width
    let x0 = rect.x + left_inset;
    let x1 = rect.right() - right_inset;
    let first = runs.first().map(|r| r.x).unwrap_or(x0);
    let last = runs.iter().map(|r| r.x + r.width).fold(f32::MIN, f32::max);
    let content_w = (last - first).max(0.0);
    match align {
        TextAlign::Center => {
            let dx = ((x1 - x0) - content_w) / 2.0;
            if dx > 0.0 {
                for run in runs.iter_mut() {
                    run.x += dx;
                }
            }
        }
        TextAlign::Right => {
            let dx = (x1 - x0) - content_w;
            if dx > 0.0 {
                for run in runs.iter_mut() {
                    run.x += dx;
                }
            }
        }
        TextAlign::Justify => {
            // distribute the leftover over the gaps between runs
            let gaps = runs.len().saturating_sub(1);
            if gaps > 0 {
                let extra = ((x1 - x0) - content_w) / gaps as f32;
                if extra > 0.0 {
                    let mut shift = 0.0f32;
                    for (i, run) in runs.iter_mut().enumerate() {
                        if i > 0 {
                            shift += extra;
                        }
                        run.x += shift;
                    }
                }
            }
        }
        _ => {}
    }
    let line = LineBox {
        rect: Rect::new(x0, top, (x1 - x0).max(0.0), height),
        baseline,
        runs: std::mem::take(runs),
    };
    let h = line.rect.height;
    lines.push(line);
    h
}

/// Natural size of a replaced element / inline-block, for line-breaking purposes.
fn replaced_size(
    box_: &LayoutBox,
    style: &ComputedStyle,
    avail: f32,
    dom: &Dom,
    intrinsic: &crate::box_tree::IntrinsicSizes,
) -> (f32, f32) {
    let nat = box_.node.and_then(|n| intrinsic.get(n));
    // an inline container never has a definite height
    crate::box_tree::replaced_size(
        box_,
        style,
        Rect::new(0.0, 0.0, avail, 0.0),
        avail,
        false,
        dom,
        nat,
    )
}

/// Intrinsic widths of inline content: `(min-content, max-content)`.
///
/// * `min-content` — widest unbreakable fragment (longest word / atomic box)
/// * `max-content` — the longest line if nothing wrapped
///
/// This is what floats and flex items are measured against.
pub fn inline_extents(
    container: &LayoutBox,
    dom: &Dom,
    intrinsic: &crate::box_tree::IntrinsicSizes,
) -> (f32, f32) {
    let font = Font::system();
    let mut flows: Vec<Flow> = Vec::new();
    tokenise(&container.children, dom, font, &[], &mut flows);
    let mut min = 0.0f32;
    let mut max = 0.0f32;
    let mut line_min = 0.0f32;
    let mut line_max = 0.0f32;
    let mut pending = 0.0f32;
    for flow in flows {
        match flow {
            Flow::Word { width, .. } => {
                line_min = line_min.max(width);
                line_max += pending + width;
                pending = 0.0;
            }
            Flow::Space { width, .. } => {
                pending += width;
            }
            Flow::Break { .. } => {
                min = min.max(line_min);
                max = max.max(line_max);
                line_min = 0.0;
                line_max = 0.0;
                pending = 0.0;
            }
            Flow::Atomic { path } => {
                let b = descend(&container.children, &path);
                let (w, _) = crate::box_tree::replaced_size(
                    b,
                    &b.style,
                    Rect::new(0.0, 0.0, 0.0, 0.0),
                    0.0,
                    false,
                    dom,
                    b.node.and_then(|n| intrinsic.get(n)),
                );
                line_min = line_min.max(w);
                line_max += pending + w;
                pending = 0.0;
            }
            // a float is out of flow: it does not add to the line's width
            Flow::Float { .. } => {}
        }
    }
    min = min.max(line_min);
    max = max.max(line_max);
    (min, max.max(min))
}

/// Width of the text if it were laid out on one line (`canvas.measureText`).
pub fn measure_text(text: &str, style: &ComputedStyle) -> f32 {
    Font::system().text_width(
        text,
        style.font_size.max(4.0),
        style.font_weight >= 600,
        style.letter_spacing,
    )
}
