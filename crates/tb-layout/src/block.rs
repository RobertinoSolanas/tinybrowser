//! Block, flex and out-of-flow layout.
//!
//! One entry point: [`Layout::layout`]. It walks the box tree top-down carrying
//! only the *containing block* (an absolute content rectangle), which keeps the
//! code small and every result in document coordinates — painting and hit
//! testing therefore need no extra transform.

use std::rc::Rc;
use tb_core::{Rect, Sides, Size};
use tb_css::media::Viewport;
use tb_dom::Dom;
use tb_gfx::Font;
use tb_style::{
    AlignItems, ComputedStyle, Display, FlexDirection, JustifyContent, Position, StyleMap, L,
};

use crate::box_tree::{build, BoxKind, LayoutBox};
use crate::inline::{inline_extents, layout_inline, FloatState, Side};

/// A containing block: the rectangle children lay out into, plus whether its
/// height is *definite* (CSS needs that to decide if `height: 100%` means
/// anything at all — with an auto-height parent it resolves to `auto`).
#[derive(Debug, Clone, Copy)]
pub struct Cb {
    pub rect: Rect,
    pub height_definite: bool,
}

impl Cb {
    pub fn new(rect: Rect, height_definite: bool) -> Cb {
        Cb {
            rect,
            height_definite,
        }
    }
}

/// The laid out document.
#[derive(Debug, Clone)]
pub struct Document {
    pub root: LayoutBox,
    /// Total scrollable content size (>= viewport).
    pub scroll_size: Size,
    /// Viewport the layout was done for.
    pub viewport: Size,
    /// How many boxes were laid out.
    pub box_count: usize,
}

/// Layout engine (cheap to build, holds only the viewport).
#[derive(Debug, Clone)]
pub struct Layout {
    pub viewport: Viewport,
}

impl Layout {
    pub fn new(viewport: Viewport) -> Layout {
        Layout { viewport }
    }

    /// Build and lay out the box tree for `dom` + `styles`.
    pub fn layout(&self, dom: &Dom, styles: &StyleMap) -> Option<Document> {
        let mut root = build(dom, styles)?;
        let icb = Rect::new(
            0.0,
            0.0,
            self.viewport.width.max(1.0),
            self.viewport.height.max(1.0),
        );
        let mut count = 0usize;
        let mut floats = FloatState::default();
        self.block_sized(
            &mut root,
            Cb::new(icb, true),
            None,
            None,
            &mut count,
            dom,
            &mut floats,
        );
        out_of_flow(&mut root, Some(icb), icb, dom, &mut count);
        count = count_boxes(&root);
        let mut max_x = 0.0f32;
        let mut max_y = 0.0f32;
        collect_extents(&root, &mut max_x, &mut max_y);
        let viewport = Size::new(self.viewport.width, self.viewport.height);
        root.scroll_extent = max_y;
        Some(Document {
            root,
            scroll_size: Size::new(max_x.max(viewport.width), max_y.max(viewport.height)),
            viewport,
            box_count: count,
        })
    }

    // ---------------------------------------------------------------- block --

    /// Lay out `box_` inside `containing` (the containing block's content box).
    ///
    /// `width_override` is how shrink-to-fit callers (floats, inline-blocks,
    /// flex items) constrain the width.
    pub fn block(
        &self,
        box_: &mut LayoutBox,
        containing: Rect,
        width_override: Option<f32>,
        count: &mut usize,
        dom: &Dom,
        inherited_floats: &mut FloatState,
    ) {
        self.block_sized(
            box_,
            Cb::new(containing, false),
            width_override,
            None,
            count,
            dom,
            inherited_floats,
        );
    }

    /// Like [`Layout::block`] but with an explicit containing block and a forced
    /// height (used for flex items on a column main axis).
    pub fn block_sized(
        &self,
        box_: &mut LayoutBox,
        cb: Cb,
        width_override: Option<f32>,
        height_override: Option<f32>,
        count: &mut usize,
        dom: &Dom,
        inherited_floats: &mut FloatState,
    ) {
        let containing = cb.rect;
        *count += 1;
        let style = box_.style.clone();
        // floats affect every line box of the same block formatting context;
        // atomic inlines and flex containers start a new one
        let creates_bfc = box_.kind == BoxKind::AtomicInline || style.display.is_flex();
        let mut fresh = FloatState::default();
        let floats: &mut FloatState = if creates_bfc {
            &mut fresh
        } else {
            inherited_floats
        };
        if style.display == Display::None {
            box_.content = Rect::new(containing.x, containing.y, 0.0, 0.0);
            return;
        }

        let deco = resolve_decoration(&style, containing.width);

        // ---- width ------------------------------------------------------
        let available = (containing.width
            - deco.padding.horizontal()
            - deco.border.horizontal()
            - margin_box(&deco))
        .max(0.0);
        let mut content_width = match style.width {
            L::Auto => width_override.unwrap_or(available),
            other => {
                let mut w = other.resolve(containing.width);
                if style.box_sizing_border {
                    w -= deco.padding.horizontal() + deco.border.horizontal();
                }
                w.max(0.0)
            }
        };
        content_width = content_width
            .max(resolve_len(&style.min_width, containing.width))
            .min(resolve_len_max(&style.max_width, containing.width))
            .max(0.0);

        // `margin: auto` centres a box with a definite width
        let mut margin = deco.margin;
        if !matches!(style.width, L::Auto) {
            // free space is distributed to `auto` margins: `0 auto` centres,
            // `auto 0` pins left — exactly like a browser
            let free = (available - content_width).max(0.0);
            match (margin.left.is_auto, margin.right.is_auto) {
                (true, true) => {
                    margin.left = Edge::resolved(free / 2.0);
                    margin.right = Edge::resolved(free / 2.0);
                }
                (true, false) => margin.left = Edge::resolved(free),
                (false, true) => margin.right = Edge::resolved(free),
                _ => {}
            }
        }
        margin.left.resolve_auto();
        margin.right.resolve_auto();
        margin.top.resolve_auto();
        margin.bottom.resolve_auto();

        box_.margin = margin.to_sides();
        box_.padding = deco.padding;
        box_.border = deco.border;

        let x = containing.x + margin.left.value + deco.border.left + deco.padding.left;
        let y = containing.y + margin.top.value + deco.border.top + deco.padding.top;
        // A definite height is known *before* the children are laid out — flex
        // boxes and absolutely positioned children need it to align against.
        let height_is_definite = match style.height {
            L::Auto => false,
            other => other.is_definite() || cb.height_definite,
        };
        let explicit_height = match style.height {
            L::Auto => height_override,
            other if height_is_definite => {
                let basis = containing.height.max(self.viewport.height);
                let mut h = other.resolve(basis);
                if style.box_sizing_border {
                    h -= deco.padding.vertical() + deco.border.vertical();
                }
                Some(h.max(0.0))
            }
            // `height: 100%` inside an auto-height parent means auto
            _ => None,
        };
        box_.content = Rect::new(x, y, content_width, explicit_height.unwrap_or(0.0));

        // ---- children ---------------------------------------------------
        let content = box_.content;
        let child_cb = Cb::new(content, explicit_height.is_some());
        if box_.inline_flow {
            // this box breaks lines itself
            let mut atomic = |b: &mut LayoutBox, place: Rect| {
                let mut f = FloatState::default();
                self.block(b, place, None, count, dom, &mut f);
            };
            let _ = child_cb;
            layout_inline(box_, content, dom, floats, &mut atomic);
        } else if style.display.is_flex() {
            self.flex(box_, content, count, dom);
        } else {
            let mut cursor = content.y;
            let mut prev_bottom_margin = 0.0f32;
            let mut first = true;
            for child in box_.children.iter_mut() {
                let cstyle = child.style.clone();
                if matches!(cstyle.position, Position::Absolute | Position::Fixed) {
                    continue; // second pass
                }
                if cstyle.clear {
                    // `clear` moves below the floats; they keep affecting later lines
                    let bottom = floats.lowest(None);
                    if bottom > cursor {
                        cursor = bottom;
                    }
                }
                if cstyle.float_left || cstyle.float_right {
                    let side = if cstyle.float_left {
                        Side::Left
                    } else {
                        Side::Right
                    };
                    let mut tmp = std::mem::replace(child, LayoutBox::placeholder(cstyle.clone()));
                    let fy = floats.next_float_y(cursor, side, content);
                    let fx = floats.next_float_x(fy, side, content);
                    let start = match side {
                        Side::Left => fx,
                        Side::Right => (content.right() - content.width).max(0.0),
                    };
                    // a float establishes a new BFC and shrink-to-fits
                    let fit = self.fit_width(&tmp, dom, content.width);
                    let mut inner = FloatState::default();
                    self.block(
                        &mut tmp,
                        Rect::new(start, fy, fit, 0.0),
                        Some(fit),
                        count,
                        dom,
                        &mut inner,
                    );
                    let bb = tmp.border_box();
                    let placed = match side {
                        Side::Left => bb.x,
                        Side::Right => content.right() - bb.width,
                    };
                    tmp.translate_by(placed - bb.x, fy - bb.y);
                    let bb = tmp.border_box();
                    floats.add(bb, side);
                    *child = tmp;
                } else {
                    // light margin collapse: the bigger of the two adjacent
                    // margins wins (no collapsing through padding/border)
                    let collapse = if first {
                        if deco.padding.top > 0.0 || deco.border.top > 0.0 {
                            0.0
                        } else {
                            prev_bottom_margin.min(cmargin_top(child, 0.0))
                        }
                    } else {
                        prev_bottom_margin.min(cmargin_top(child, 0.0))
                    };
                    let mut c = content;
                    c.y = cursor - collapse;
                    self.block_sized(
                        child,
                        Cb::new(c, explicit_height.is_some()),
                        None,
                        None,
                        count,
                        dom,
                        floats,
                    );
                    // the child resolved its own margins; read them back
                    let m = child.margin;
                    let bb = child.border_box();
                    cursor = c.y - collapse + bb.height + m.top + m.bottom;
                    prev_bottom_margin = m.bottom;
                    first = false;
                }
            }
            let h = (cursor - content.y).max(0.0);
            box_.content = Rect::new(content.x, content.y, content_width, h);
        }

        // ---- height -----------------------------------------------------
        if matches!(style.height, L::Auto) {
            if let Some(h) = height_override {
                // a flex item on a column main axis gets its distributed size
                box_.content.height = box_.content.height.max(h);
            }
        }
        if height_is_definite && !matches!(style.height, L::Auto) {
            let basis = containing.height.max(self.viewport.height);
            let mut h = style.height.resolve(basis);
            if style.box_sizing_border {
                h -= deco.padding.vertical() + deco.border.vertical();
            }
            box_.content.height = h.max(0.0);
        }
        let h_basis = |l: &L, auto: f32| -> f32 {
            match l {
                L::Auto => auto,
                other if other.is_definite() || cb.height_definite => {
                    other.resolve(containing.height.max(self.viewport.height))
                }
                _ => auto,
            }
        };
        box_.content.height = box_
            .content
            .height
            .max(h_basis(&style.min_height, 0.0))
            .min(h_basis(&style.max_height, f32::INFINITY))
            .max(0.0);

        // replaced elements decide their own size
        if box_.kind == BoxKind::AtomicInline {
            self.replace_box(box_, containing, cb.height_definite, dom);
        }

        // ---- relative positioning --------------------------------------
        if matches!(style.position, Position::Relative | Position::Sticky) {
            let dx = resolve_len(&style.inset[ComputedStyle::LEFT], containing.width)
                - resolve_len(&style.inset[ComputedStyle::RIGHT], containing.width);
            let dy = resolve_len(&style.inset[ComputedStyle::TOP], containing.height)
                - resolve_len(&style.inset[ComputedStyle::BOTTOM], containing.height);
            if dx != 0.0 || dy != 0.0 {
                box_.translate_by(dx, dy);
            }
        }
    }

    // ----------------------------------------------------------------- flex --

    /// Flexbox, simplified but honest: wrapping lines, `flex-basis`,
    /// grow/shrink distribution, gaps, `justify-content`, `align-items`.
    fn flex(&self, box_: &mut LayoutBox, content: Rect, count: &mut usize, dom: &Dom) {
        let style = box_.style.clone();
        let column = matches!(
            style.flex_direction,
            FlexDirection::Column | FlexDirection::ColumnReverse
        );
        let reverse = matches!(
            style.flex_direction,
            FlexDirection::RowReverse | FlexDirection::ColumnReverse
        );
        let height_definite = !matches!(style.height, L::Auto);
        let main_extent = if column {
            if height_definite {
                style.height.resolve(content.height)
            } else {
                f32::INFINITY
            }
        } else {
            content.width
        };

        #[derive(Debug)]
        struct Item {
            basis: f32,
            grow: f32,
            shrink: f32,
            cross: f32,
        }
        let mut items: Vec<Item> = Vec::new();
        for child in box_.children.iter_mut() {
            let cs = child.style.clone();
            // `flex-basis: auto` means "the main axis size property, or the
            // content size if that is auto too"
            let main_len = if column { &cs.height } else { &cs.width };
            let main_basis = if column {
                content.height
            } else {
                content.width
            };
            let len_is_definite = if column {
                main_len.is_definite() && height_definite
            } else {
                true
            };
            let mut basis = if !matches!(cs.flex_basis, L::Auto) {
                cs.flex_basis.resolve(main_basis)
            } else if !matches!(main_len, L::Auto) && len_is_definite {
                main_len.resolve(main_basis)
            } else {
                // content sized item: measure it with shrink-to-fit
                let mut tmp = std::mem::replace(child, LayoutBox::placeholder(cs.clone()));
                let mut f = FloatState::default();
                if column {
                    self.block_sized(
                        &mut tmp,
                        Cb::new(Rect::new(content.x, content.y, content.width, 0.0), false),
                        Some(content.width),
                        None,
                        count,
                        dom,
                        &mut f,
                    );
                } else {
                    let fit = self.fit_width(&tmp, dom, content.width);
                    self.block_sized(
                        &mut tmp,
                        Cb::new(Rect::new(content.x, content.y, fit, 0.0), false),
                        Some(fit),
                        None,
                        count,
                        dom,
                        &mut f,
                    );
                }
                let bb = tmp.border_box();
                *child = tmp;
                if column {
                    bb.height
                } else {
                    bb.width
                }
            };
            // min/max along the main axis
            let min_main = if column {
                resolve_len(&cs.min_height, main_basis)
            } else {
                resolve_len(&cs.min_width, main_basis)
            };
            let max_main = if column {
                resolve_len_max(&cs.max_height, main_basis)
            } else {
                resolve_len_max(&cs.max_width, main_basis)
            };
            basis = basis.max(min_main).min(max_main).max(0.0);
            items.push(Item {
                basis,
                grow: cs.flex_grow.max(0.0),
                shrink: cs.flex_shrink.max(0.0),
                cross: 0.0,
            });
        }

        let gap_main = if column {
            style.row_gap
        } else {
            style.column_gap
        };
        let gap_cross = if column {
            style.column_gap
        } else {
            style.row_gap
        };

        // ---- wrap into lines -------------------------------------------
        let mut lines: Vec<Vec<usize>> = Vec::new();
        let mut current: Vec<usize> = Vec::new();
        let mut used = 0.0f32;
        for (i, item) in items.iter().enumerate() {
            let need = item.basis + if current.is_empty() { 0.0 } else { gap_main };
            if style.flex_wrap && !current.is_empty() && used + need > main_extent {
                lines.push(std::mem::take(&mut current));
                current.push(i);
                used = item.basis;
            } else {
                current.push(i);
                used += need;
            }
        }
        if !current.is_empty() {
            lines.push(current);
        }
        if lines.is_empty() {
            lines.push(Vec::new());
        }

        // ---- distribute each line --------------------------------------
        let mut cross_offset = 0.0f32;
        let mut max_line_main = 0.0f32;
        for line in &lines {
            if line.is_empty() {
                continue;
            }
            let total_basis: f32 = line.iter().map(|i| items[*i].basis).sum::<f32>()
                + gap_main * (line.len() - 1) as f32;
            let free = if main_extent.is_finite() {
                main_extent - total_basis
            } else {
                0.0
            };
            if free > 0.0 {
                let grow: f32 = line.iter().map(|i| items[*i].grow).sum();
                if grow > 0.0 {
                    for i in line {
                        items[*i].basis += free * items[*i].grow / grow;
                    }
                }
            } else if free < 0.0 {
                let weighted: f32 = line
                    .iter()
                    .map(|i| items[*i].shrink * items[*i].basis)
                    .sum();
                if weighted > 0.0 {
                    for i in line {
                        let d = free * items[*i].shrink * items[*i].basis / weighted;
                        items[*i].basis = (items[*i].basis + d).max(0.0);
                    }
                }
            }
            let line_main: f32 = line.iter().map(|i| items[*i].basis).sum::<f32>()
                + gap_main * (line.len() - 1) as f32;
            max_line_main = max_line_main.max(line_main);

            let free_main = if main_extent.is_finite() {
                (main_extent - line_main).max(0.0)
            } else {
                0.0
            };
            let n = line.len();
            let (mut main_pos, extra_gap) = match style.justify_content {
                JustifyContent::FlexEnd => (free_main, gap_main),
                JustifyContent::Center => (free_main / 2.0, gap_main),
                JustifyContent::SpaceBetween if n > 1 => {
                    (0.0, gap_main + free_main / (n - 1) as f32)
                }
                JustifyContent::SpaceAround => {
                    (free_main / (2 * n) as f32, gap_main + free_main / n as f32)
                }
                JustifyContent::SpaceEvenly => (
                    free_main / (n + 1) as f32,
                    gap_main + free_main / (n + 1) as f32,
                ),
                _ => (0.0, gap_main),
            };

            // ---- place the items along the main axis, then align them ----
            let order: Vec<usize> = if reverse {
                line.iter().rev().copied().collect()
            } else {
                line.clone()
            };
            let mut line_cross = 0.0f32;
            for &i in &order {
                let child_style = box_.children[i].style.clone();
                let main_size = items[i].basis;
                let child_rect = if column {
                    Rect::new(
                        content.x,
                        content.y + cross_offset + main_pos,
                        content.width,
                        0.0,
                    )
                } else {
                    Rect::new(
                        content.x + main_pos,
                        content.y + cross_offset,
                        main_size,
                        0.0,
                    )
                };
                let mut tmp =
                    std::mem::replace(&mut box_.children[i], LayoutBox::placeholder(child_style));
                let mut f = FloatState::default();
                self.block_sized(
                    &mut tmp,
                    Cb::new(child_rect, height_definite),
                    if column { None } else { Some(main_size) },
                    if column { Some(main_size) } else { None },
                    count,
                    dom,
                    &mut f,
                );
                let bb = tmp.border_box();
                line_cross = line_cross.max(if column { bb.width } else { bb.height });
                box_.children[i] = tmp;
                main_pos += main_size + extra_gap;
            }

            // cross-axis alignment (`align-items`, overridden by `align-self`)
            for &i in &order {
                let align = box_.children[i]
                    .style
                    .align_self
                    .unwrap_or(style.align_items);
                let bb = box_.children[i].border_box();
                let child_cross = if column { bb.height } else { bb.width };
                let target = if column {
                    content.width
                } else if height_definite {
                    content.height
                } else {
                    line_cross
                };
                let slack = (target - child_cross).max(0.0);
                let cur_style = box_.children[i].style.clone();
                let mut tmp =
                    std::mem::replace(&mut box_.children[i], LayoutBox::placeholder(cur_style));
                let (dx, dy) = if column {
                    match align {
                        AlignItems::Center => (slack / 2.0, 0.0),
                        AlignItems::FlexEnd => (slack, 0.0),
                        AlignItems::Stretch if slack > 0.0 => {
                            tmp.content.width += slack;
                            (0.0, 0.0)
                        }
                        _ => (0.0, 0.0),
                    }
                } else {
                    match align {
                        AlignItems::Center => (0.0, slack / 2.0),
                        AlignItems::FlexEnd => (0.0, slack),
                        AlignItems::Stretch if slack > 0.0 && height_definite => {
                            tmp.content.height += slack;
                            (0.0, 0.0)
                        }
                        _ => (0.0, 0.0),
                    }
                };
                tmp.translate_by(dx, dy);
                let bb = tmp.border_box();
                items[i].cross = if column { bb.height } else { bb.width };
                box_.children[i] = tmp;
            }
            cross_offset += line_cross + gap_cross;
        }

        let height = if column {
            max_line_main.max(cross_offset - gap_cross)
        } else {
            (cross_offset - gap_cross).max(0.0)
        };
        box_.content = Rect::new(content.x, content.y, content.width, height.max(0.0));
    }

    // ------------------------------------------------------ intrinsic size --

    /// Narrowest width without overflowing: longest word, or the widest
    /// unbreakable descendant.
    fn min_content(&self, box_: &LayoutBox, dom: &Dom) -> f32 {
        self.content_extent(box_, dom, true)
    }

    /// Width of the widest line if nothing wrapped.
    fn max_content(&self, box_: &LayoutBox, dom: &Dom) -> f32 {
        self.content_extent(box_, dom, false)
    }

    fn content_extent(&self, box_: &LayoutBox, dom: &Dom, min: bool) -> f32 {
        use tb_style::Display;
        if box_.style.display == Display::None {
            return 0.0;
        }
        let deco = resolve_decoration(&box_.style, 0.0);
        let extra = deco.padding.horizontal() + deco.border.horizontal();
        // a definite width wins; percentages have no intrinsic meaning
        if let L::Px(w) = box_.style.width {
            return if box_.style.box_sizing_border {
                w
            } else {
                w + extra
            };
        }
        let kids: Vec<&LayoutBox> = box_
            .children
            .iter()
            .filter(|c| c.style.display != Display::None)
            .collect();
        let inner = if box_.kind == BoxKind::AtomicInline {
            // replaced elements contribute their intrinsic width; a percentage
            // width is indefinite and therefore says nothing about content size
            crate::box_tree::intrinsic_replaced_width(box_, &box_.style, dom)
        } else if box_.inline_flow {
            let (lo, hi) = inline_extents(box_, dom);
            if min {
                lo
            } else {
                hi
            }
        } else if box_.style.display.is_flex() {
            let column = matches!(
                box_.style.flex_direction,
                FlexDirection::Column | FlexDirection::ColumnReverse
            );
            let gap = if column {
                box_.style.row_gap
            } else {
                box_.style.column_gap
            };
            let parts: Vec<f32> = kids
                .iter()
                .map(|c| self.content_extent(c, dom, min))
                .collect();
            let gaps = gap * parts.len().saturating_sub(1) as f32;
            if column || min {
                parts.into_iter().fold(0.0f32, f32::max)
            } else {
                parts.into_iter().sum::<f32>() + gaps
            }
        } else {
            kids.iter()
                .map(|c| self.content_extent(c, dom, min))
                .fold(0.0f32, f32::max)
        };
        inner
            + if box_.style.box_sizing_border {
                0.0
            } else {
                extra
            }
    }

    /// `fit-content(available)` = clamp(max-content, min-content, available).
    fn fit_width(&self, box_: &LayoutBox, dom: &Dom, available: f32) -> f32 {
        let lo = self.min_content(box_, dom);
        let hi = self.max_content(box_, dom);
        hi.min(available).max(lo)
    }

    // ------------------------------------------------------------- replaced --

    /// `<img>`, `<input>`, … — we do not decode pictures, but the box still has
    /// a size (CSS first, then the `width`/`height` attributes, aspect ratio
    /// preserved), so layout and later painting stay truthful.
    fn replace_box(
        &self,
        box_: &mut LayoutBox,
        containing: Rect,
        height_definite: bool,
        dom: &Dom,
    ) {
        let style = box_.style.clone();
        let (w, h) = crate::box_tree::replaced_size(
            box_,
            &style,
            containing,
            self.viewport.height,
            height_definite,
            dom,
        );
        let (x, y) = (box_.content.x, box_.content.y);
        box_.content = Rect::new(x, y, w.max(0.0), h.max(0.0));
        box_.measured_height = box_.content.height;
    }

    /// Lay out `box_` as a standalone block of at most `width`; returns the
    /// border-box size (used by tests and shrink-to-fit callers).
    pub fn measure(&self, box_: &mut LayoutBox, width: f32, dom: &Dom) -> (f32, f32) {
        let mut count = 0usize;
        let mut floats = FloatState::default();
        self.block(
            box_,
            Rect::new(0.0, 0.0, width, 0.0),
            None,
            &mut count,
            dom,
            &mut floats,
        );
        let bb = box_.border_box();
        (bb.width, bb.height)
    }
}

fn cmargin_top(child: &LayoutBox, fallback: f32) -> f32 {
    // margins are resolved inside `block`; before that we read the style
    if child.margin.top > 0.0 {
        child.margin.top
    } else {
        child.style.margin[ComputedStyle::TOP]
            .resolve_zero(0.0)
            .max(fallback)
    }
}

// ------------------------------------------------------------- decoration --

#[derive(Debug, Clone, Copy)]
struct Edge {
    value: f32,
    is_auto: bool,
}

impl Edge {
    fn resolved(v: f32) -> Edge {
        Edge {
            value: v,
            is_auto: false,
        }
    }
    fn resolve_auto(&mut self) {
        if self.is_auto {
            self.value = 0.0;
            self.is_auto = false;
        }
    }
}

/// Margins keep `auto` until the width is known.
#[derive(Debug, Clone, Copy)]
struct Margins {
    top: Edge,
    right: Edge,
    bottom: Edge,
    left: Edge,
}

impl Margins {
    fn to_sides(&self) -> Sides {
        Sides {
            top: self.top.value,
            right: self.right.value,
            bottom: self.bottom.value,
            left: self.left.value,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Decoration {
    margin: Margins,
    padding: Sides,
    border: Sides,
}

fn resolve_decoration(style: &ComputedStyle, basis: f32) -> Decoration {
    let edge = |l: &L| match l {
        L::Auto => Edge {
            value: 0.0,
            is_auto: true,
        },
        other => Edge {
            value: other.resolve_zero(basis),
            is_auto: false,
        },
    };
    let side = |v: [f32; 4]| Sides {
        top: v[0],
        right: v[1],
        bottom: v[2],
        left: v[3],
    };
    let border_px = |i: usize| {
        if style.border_style[i].painted() {
            style.border_width[i]
        } else {
            0.0
        }
    };
    Decoration {
        margin: Margins {
            top: edge(&style.margin[0]),
            right: edge(&style.margin[1]),
            bottom: edge(&style.margin[2]),
            left: edge(&style.margin[3]),
        },
        padding: side([
            style.padding[0].resolve_zero(basis),
            style.padding[1].resolve_zero(basis),
            style.padding[2].resolve_zero(basis),
            style.padding[3].resolve_zero(basis),
        ]),
        border: side([border_px(0), border_px(1), border_px(2), border_px(3)]),
    }
}

fn margin_box(deco: &Decoration) -> f32 {
    deco.margin.left.value + deco.margin.right.value
}

fn resolve_len(l: &L, basis: f32) -> f32 {
    match l {
        L::Auto => 0.0,
        other => other.resolve_zero(basis),
    }
}

fn resolve_len_max(l: &L, basis: f32) -> f32 {
    match l {
        L::Auto => f32::INFINITY,
        other => other.resolve_zero(basis),
    }
}

fn inset_px(style: &ComputedStyle, side: usize, basis: f32) -> Option<f32> {
    match style.inset[side] {
        L::Auto => None,
        other => Some(other.resolve_zero(basis)),
    }
}

// --------------------------------------------------------- out of flow pass --

/// Position `position: absolute|fixed` descendants after normal flow.
fn out_of_flow(
    box_: &mut LayoutBox,
    containing: Option<Rect>,
    viewport: Rect,
    dom: &Dom,
    count: &mut usize,
) {
    let positioned = !matches!(box_.style.position, Position::Static);
    let own = if positioned {
        Some(box_.padding_box())
    } else {
        containing
    };
    let layout = Layout {
        viewport: Viewport {
            width: viewport.width,
            height: viewport.height,
            ..Default::default()
        },
    };
    let mut children = std::mem::take(&mut box_.children);
    let mut placed: Vec<LayoutBox> = Vec::with_capacity(children.len());
    for mut child in children.drain(..) {
        let pos = child.style.position;
        if matches!(pos, Position::Absolute | Position::Fixed) {
            let cb = match pos {
                Position::Fixed => viewport,
                _ => own.unwrap_or(viewport),
            };
            place_absolute(&mut child, cb, viewport, dom, count, &layout);
        } else {
            let inner = own.or_else(|| Some(box_.padding_box()));
            out_of_flow(&mut child, inner, viewport, dom, count);
        }
        placed.push(child);
    }
    box_.children = placed;
}

fn place_absolute(
    box_: &mut LayoutBox,
    cb: Rect,
    viewport: Rect,
    dom: &Dom,
    count: &mut usize,
    layout: &Layout,
) {
    let style = box_.style.clone();
    let left = inset_px(&style, ComputedStyle::LEFT, cb.width);
    let right = inset_px(&style, ComputedStyle::RIGHT, cb.width);
    let top = inset_px(&style, ComputedStyle::TOP, cb.height);
    let bottom = inset_px(&style, ComputedStyle::BOTTOM, cb.height);

    // static position: where the box would have been in flow (approximated by
    // the position the box tree construction left it at)
    let static_x = box_.content.x;
    let static_y = box_.content.y;

    let mut width = match style.width {
        L::Auto => None,
        other => Some(other.resolve(cb.width)),
    };
    let mut x = match (left, right) {
        (Some(l), Some(r)) => {
            let w = width.unwrap_or((cb.width - l - r).max(0.0));
            width = Some(w);
            cb.x + l
        }
        (Some(l), None) => cb.x + l,
        (None, Some(r)) => cb.right() - r - width.unwrap_or(0.0),
        (None, None) => static_x,
    };
    let mut y = match (top, bottom) {
        (Some(t), _) => cb.y + t,
        (None, Some(b)) => cb.bottom() - b - resolve_len(&style.height, cb.height),
        (None, None) => static_y,
    };
    if style.position == Position::Fixed {
        x = left.map(|l| viewport.x + l).unwrap_or(static_x);
        y = top.map(|t| viewport.y + t).unwrap_or(static_y);
    }

    let mut tmp = std::mem::replace(box_, LayoutBox::placeholder(style.clone()));
    let rect = Rect::new(x, y, width.unwrap_or(cb.width), 0.0);
    let mut f = FloatState::default();
    layout.block(&mut tmp, rect, width, count, dom, &mut f);
    let own_pad = tmp.padding_box();
    out_of_flow(&mut tmp, Some(own_pad), viewport, dom, count);
    let bb = tmp.border_box();
    if width.is_none() {
        if let Some(r) = right {
            x = cb.right() - r - bb.width;
        }
    }
    tmp.translate_by(x - bb.x, y - bb.y);
    *box_ = tmp;
}

// ---------------------------------------------------------------- extents --

/// Every box in the tree, including the inline ones.
fn count_boxes(box_: &LayoutBox) -> usize {
    1 + box_.children.iter().map(count_boxes).sum::<usize>()
}

/// Total document extent: the deepest and widest any box reaches.
///
/// `overflow` on an ancestor contains its content: a carousel wider than the
/// page, or a fixed-height box with too much text, must not stretch the page in
/// the clipped axis. We still let the other axis propagate, because
/// `overflow-x: hidden` on `<body>` should not collapse the page height.
fn collect_extents(box_: &LayoutBox, max_x: &mut f32, max_y: &mut f32) {
    if box_.style.display == Display::None {
        return;
    }
    let bb = box_.border_box();
    *max_x = max_x.max(bb.right());
    *max_y = max_y.max(bb.bottom());

    let clip_x = box_.style.overflow_x != tb_style::Overflow::Visible;
    let clip_y = box_.style.overflow_y != tb_style::Overflow::Visible;
    if !clip_x && !clip_y {
        collect_extents_inner(box_, max_x, max_y);
        return;
    }
    let mut inner_x = 0.0f32;
    let mut inner_y = 0.0f32;
    collect_extents_inner(box_, &mut inner_x, &mut inner_y);
    let pb = box_.padding_box();
    if clip_x {
        *max_x = max_x.max(inner_x.min(pb.right()));
    } else {
        *max_x = max_x.max(inner_x);
    }
    if clip_y {
        *max_y = max_y.max(inner_y.min(pb.bottom()));
    } else {
        *max_y = max_y.max(inner_y);
    }
}

fn collect_extents_inner(box_: &LayoutBox, max_x: &mut f32, max_y: &mut f32) {
    for line in &box_.lines {
        for run in &line.runs {
            *max_x = max_x.max(run.x + run.width);
            *max_y = max_y.max(run.baseline + 2.0);
        }
    }
    for c in &box_.children {
        collect_extents(c, max_x, max_y);
    }
}

/// Text measurement with our bitmap font (used by the painter and CLI).
pub fn text_width(text: &str, style: &Rc<ComputedStyle>) -> f32 {
    Font::system().text_width(
        text,
        style.font_size,
        style.font_weight >= 600,
        style.letter_spacing,
    )
}
