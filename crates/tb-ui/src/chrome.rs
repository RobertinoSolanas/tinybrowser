//! Where the chrome is, in cells and in UI pixels, and what a click on it
//! means.
//!
//! The chrome is the browser's own UI, not the page: tab strip, toolbar with the
//! navigation buttons and the address bar, optional bookmark bar, the status
//! line, the find bar, the command line, the devtools drawer and the scrollbar.
//! [`Chrome`] owns no state — it is given the numbers (tab count, which
//! overlays are open) and answers two questions:
//!
//! * **layout** — a list of [`Widget`]s with row/column/width, used by both
//!   renderers and by the focus ring;
//! * **hit test** — `Widget` at a pixel point, so `App` can decide "that was the
//!   back button" before it ever looks at the page.
//!
//! UI pixels: a cell is [`CELL_W`] × [`CELL_H`], which is exactly the advance of
//! our 5×7 bitmap font at scale 2 — one character per cell for the terminal, a
//! real glyph for the X11 window, and the same numbers for the mouse.

use crate::event::Mods;

/// One character cell in UI pixels.
pub const CELL_W: i32 = 12;
/// …and its height (the 7 glyph rows at scale 2 plus leading).
pub const CELL_H: i32 = 16;

pub fn px_to_col(x: i32) -> usize {
    (x.max(0) / CELL_W) as usize
}

pub fn px_to_row(y: i32) -> usize {
    (y.max(0) / CELL_H) as usize
}

pub fn col_to_px(col: usize) -> i32 {
    col as i32 * CELL_W
}

pub fn row_to_px(row: usize) -> i32 {
    row as i32 * CELL_H
}

/// A clickable/focusable part of the chrome.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Widget {
    /// Nothing of ours: the page, or the gap below a short page.
    None,
    Back,
    Forward,
    Reload,
    Home,
    Address,
    /// Bookmark star.
    Star,
    /// Inspector toggle.
    Info,
    /// The hamburger menu.
    Menu,
    NewTab,
    /// A tab, by index.
    Tab(usize),
    /// The `×` of a tab.
    CloseTab(usize),
    /// The vertical scrollbar (thumb or track — `App` works out which).
    Scrollbar,
    /// The bottom horizontal scrollbar.
    HScrollbar,
    /// Find bar pieces.
    FindField,
    FindPrev,
    FindNext,
    FindClose,
    /// The `:` command line.
    Command,
    /// An entry of the bookmark bar, by index.
    Bookmark(usize),
    /// Devtools drawer (its own widgets live in `devtools`).
    Devtools,
}

/// Columns of one widget in one row.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Span {
    pub row: usize,
    pub x: usize,
    pub w: usize,
}

impl Span {
    pub fn contains(&self, col: usize, row: usize) -> bool {
        self.row == row && col >= self.x && col < self.x + self.w
    }

    pub fn px(&self) -> (i32, i32, i32, i32) {
        (
            col_to_px(self.x),
            row_to_px(self.row),
            self.w as i32 * CELL_W,
            CELL_H,
        )
    }
}

/// The chrome's geometry for one frame.
#[derive(Clone, Debug)]
pub struct Chrome {
    pub cols: usize,
    pub rows: usize,
    /// `false` for `--no-chrome` (a bare page, the "app window" mode).
    pub show_chrome: bool,
    pub show_bookmarks: bool,
    pub show_status: bool,
    pub tab_count: usize,
    pub active_tab: usize,
    pub find_open: bool,
    pub command_open: bool,
    /// Rows the devtools drawer takes at the bottom of the content area.
    pub devtools_rows: usize,
    /// A horizontal scrollbar is needed (zoomed-in page wider than the view).
    pub h_scroll: bool,
    /// Labels shown in the bookmark bar (already truncated by `App`).
    pub bookmark_labels: Vec<String>,
}

/// The button labels are ASCII on purpose: our bitmap font covers ASCII 32–126
/// plus a handful of typographic extras, and a UI that promises a glyph must
/// have it in every backend.
pub const LABEL_BACK: &str = "[<]";
pub const LABEL_FORWARD: &str = "[>]";
pub const LABEL_RELOAD: &str = "[o]";
pub const LABEL_HOME: &str = "[^]";
pub const LABEL_STAR: &str = "[*]";
pub const LABEL_INFO: &str = "[i]";
pub const LABEL_MENU: &str = "[=]";
pub const LABEL_NEW_TAB: &str = " +";
pub const LABEL_CLOSE: &str = "x";
/// `←`/`→` inside the find bar.
pub const LABEL_PREV: &str = "[<]";
pub const LABEL_NEXT: &str = "[>]";
pub const LABEL_FIND_CLOSE: &str = "[x]";

impl Chrome {
    /// One row of chrome buttons: label + widget + column.
    pub const NAV_BUTTONS: [(&'static str, Widget); 4] = [
        (LABEL_BACK, Widget::Back),
        (LABEL_FORWARD, Widget::Forward),
        (LABEL_RELOAD, Widget::Reload),
        (LABEL_HOME, Widget::Home),
    ];

    pub const RIGHT_BUTTONS: [(&'static str, Widget); 3] = [
        (LABEL_STAR, Widget::Star),
        (LABEL_INFO, Widget::Info),
        (LABEL_MENU, Widget::Menu),
    ];

    /// Left-hand buttons take 4 columns each (`[<] `).
    fn nav_end(&self) -> usize {
        (Self::NAV_BUTTONS.len() * 4).min(self.cols)
    }

    fn right_start(&self) -> usize {
        // leave one blank column between the address bar and the buttons
        self.cols.saturating_sub(Self::RIGHT_BUTTONS.len() * 4 + 1)
    }

    pub fn tabstrip_row(&self) -> usize {
        0
    }

    pub fn toolbar_row(&self) -> usize {
        if self.show_chrome {
            1
        } else {
            0
        }
    }

    pub fn bookmark_row(&self) -> Option<usize> {
        if self.show_chrome && self.show_bookmarks {
            Some(2)
        } else {
            None
        }
    }

    /// First content row.
    pub fn content_top(&self) -> usize {
        if !self.show_chrome {
            return 0;
        }
        let mut r = 2;
        if self.show_bookmarks {
            r += 1;
        }
        r
    }

    /// The status line, if shown.
    pub fn status_row(&self) -> Option<usize> {
        if self.show_status {
            Some(self.rows.saturating_sub(1))
        } else {
            None
        }
    }

    /// Last content row (exclusive of status/find/devtools).
    pub fn content_bottom(&self) -> usize {
        let mut bottom = self.status_row().unwrap_or(self.rows);
        if self.find_open || self.command_open {
            bottom = bottom.saturating_sub(1);
        }
        bottom.saturating_sub(self.devtools_rows)
    }

    pub fn content_rows(&self) -> usize {
        self.content_bottom().saturating_sub(self.content_top())
    }

    /// The find/command bar row (the last row above the status line).
    pub fn bar_row(&self) -> usize {
        let bottom = self.status_row().unwrap_or(self.rows);
        bottom.saturating_sub(1)
    }

    pub fn devtools_top(&self) -> usize {
        self.content_bottom()
    }

    /// Content area in cells: `(x, y, w, h)`.
    pub fn content_cells(&self) -> (usize, usize, usize, usize) {
        let w = self.cols.saturating_sub(if self.h_scroll { 0 } else { 1 });
        (0, self.content_top(), w, self.content_rows())
    }

    /// Every widget of this frame, in paint order.
    pub fn widgets(&self) -> Vec<(Widget, Span)> {
        let mut out = Vec::new();
        if !self.show_chrome {
            return out;
        }
        // tab strip
        let (tab_w, gap) = self.tab_layout();
        for i in 0..self.tab_count.min(64) {
            let x = i * (tab_w + gap);
            if x + tab_w > self.cols.saturating_sub(2) {
                break;
            }
            out.push((
                Widget::Tab(i),
                Span {
                    row: self.tabstrip_row(),
                    x,
                    w: tab_w,
                },
            ));
            if tab_w >= 6 {
                out.push((
                    Widget::CloseTab(i),
                    Span {
                        row: self.tabstrip_row(),
                        x: x + tab_w - 2,
                        w: 2,
                    },
                ));
            }
        }
        out.push((
            Widget::NewTab,
            Span {
                row: self.tabstrip_row(),
                x: self.cols.saturating_sub(2),
                w: 2,
            },
        ));
        // toolbar
        let row = self.toolbar_row();
        for (i, (_, w)) in Self::NAV_BUTTONS.iter().enumerate() {
            out.push((
                *w,
                Span {
                    row,
                    x: i * 4,
                    w: 3,
                },
            ));
        }
        let nav_end = self.nav_end();
        let right = self.right_start();
        if right > nav_end + 1 {
            out.push((
                Widget::Address,
                Span {
                    row,
                    x: nav_end + 1,
                    w: right - nav_end - 1,
                },
            ));
        }
        for (i, (_, w)) in Self::RIGHT_BUTTONS.iter().enumerate() {
            out.push((
                *w,
                Span {
                    row,
                    x: right + 1 + i * 4,
                    w: 3,
                },
            ));
        }
        // bookmark bar: labels separated by one blank column
        if let Some(brow) = self.bookmark_row() {
            let mut x = 1usize;
            for (i, label) in self.bookmark_labels.iter().enumerate() {
                let w = (*label).chars().count().max(3) + 1;
                if x + w > self.cols.saturating_sub(1) {
                    break;
                }
                out.push((Widget::Bookmark(i), Span { row: brow, x, w }));
                x += w + 1;
            }
        }
        out
    }

    /// `(tab width, gap)` so `tab_count` tabs share the row, with a `+` at the
    /// end and a minimum width so they stay clickable.
    pub fn tab_layout(&self) -> (usize, usize) {
        let n = self.tab_count.max(1);
        let avail = self.cols.saturating_sub(3);
        let gap = if n > 1 { 1 } else { 0 };
        let w = (avail / n).saturating_sub(gap);
        (w.max(4), gap)
    }

    /// The widget at a pixel point.
    pub fn hit(&self, px_x: i32, px_y: i32, mods: Mods) -> Widget {
        let _ = mods;
        let col = px_to_col(px_x);
        let row = px_to_row(px_y);
        if row >= self.rows || col >= self.cols {
            return Widget::None;
        }
        if (self.find_open || self.command_open) && row == self.bar_row() {
            return self.hit_bar(col);
        }
        if self.devtools_rows > 0
            && row >= self.devtools_top()
            && row < self.devtools_top() + self.devtools_rows
        {
            return Widget::Devtools;
        }
        // the scrollbar is part of the content area but belongs to the browser
        let (_cx, cy, _cw, ch) = self.content_cells();
        if row >= cy && row < cy + ch {
            if self.hit_scrollbar(col, row) {
                return Widget::Scrollbar;
            }
            if self.h_scroll && Some(row) == self.h_scrollbar_row() {
                return Widget::HScrollbar;
            }
            return Widget::None;
        }
        for (widget, span) in self.widgets() {
            if span.contains(col, row) {
                return widget;
            }
        }
        Widget::None
    }

    fn hit_bar(&self, col: usize) -> Widget {
        if self.command_open {
            return Widget::Command;
        }
        // ` Find: [text        ] [1/3] [<] [>] [x]`
        let find_x = self.cols.saturating_sub(24);
        if col < find_x + 7 {
            return Widget::FindField;
        }
        if col >= self.cols.saturating_sub(3) {
            return Widget::FindClose;
        }
        if col >= self.cols.saturating_sub(9) && col < self.cols.saturating_sub(6) {
            return Widget::FindPrev;
        }
        if col >= self.cols.saturating_sub(6) {
            return Widget::FindNext;
        }
        Widget::FindField
    }

    pub fn h_scrollbar_row(&self) -> Option<usize> {
        if self.h_scroll {
            Some(self.content_bottom().saturating_sub(1))
        } else {
            None
        }
    }

    /// The right-most column is the scrollbar while the page can scroll.
    pub fn hit_scrollbar(&self, col: usize, row: usize) -> bool {
        let (_, cy, _, ch) = self.content_cells();
        col + 1 >= self.cols && row >= cy && row < cy + ch
    }
}
