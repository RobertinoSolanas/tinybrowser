//! The cell grid — what a terminal shows, and what an X11 window shows when it
//! draws our own bitmap font.
//!
//! A [`Screen`] is a rectangle of [`Cell`]s plus a cursor position and a window
//! title. It is deliberately *dumb data*: the renderers fill it, the backends
//! turn it into SGR escapes or pixels, and a test compares it against a string.
//! That is the whole reason the UI has a canonical output type — no window
//! server needed to know what the user would have seen.

use tb_core::Color;

/// How one cell looks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
    pub underline: bool,
    pub inverse: bool,
}

impl Default for Style {
    /// Black on white: what a terminal shows before any escape sequence.
    fn default() -> Style {
        Style::new(Color::BLACK, Color::WHITE)
    }
}

impl Style {
    pub fn new(fg: Color, bg: Color) -> Style {
        Style {
            fg,
            bg,
            bold: false,
            underline: false,
            inverse: false,
        }
    }

    pub fn bold(mut self, on: bool) -> Style {
        self.bold = on;
        self
    }

    pub fn underline(mut self, on: bool) -> Style {
        self.underline = on;
        self
    }

    pub fn inverse(mut self, on: bool) -> Style {
        self.inverse = on;
        self
    }
}

/// One character cell.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cell {
    pub ch: char,
    pub style: Style,
    /// Drawn *above* the page (menu, hint label, help overlay). A pixel backend
    /// paints the page first and these cells afterwards.
    pub overlay: bool,
}

impl Default for Cell {
    fn default() -> Cell {
        Cell {
            ch: ' ',
            style: Style::new(Color::BLACK, Color::WHITE),
            overlay: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Screen {
    pub cols: usize,
    pub rows: usize,
    pub cells: Vec<Cell>,
    /// Where the hardware cursor should sit (an addressed terminal and an X11
    /// text caret both want this).
    pub cursor: Option<(usize, usize)>,
    pub title: String,
}

impl Screen {
    pub fn new(cols: usize, rows: usize) -> Screen {
        let cols = cols.max(1);
        let rows = rows.max(1);
        Screen {
            cols,
            rows,
            cells: vec![Cell::default(); cols * rows],
            cursor: None,
            title: String::new(),
        }
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        self.cols = cols.max(1);
        self.rows = rows.max(1);
        self.cells = vec![Cell::default(); self.cols * self.rows];
        self.cursor = None;
    }

    pub fn clear(&mut self, bg: Color) {
        let style = Style::new(Color::BLACK, bg);
        for c in self.cells.iter_mut() {
            *c = Cell {
                ch: ' ',
                style,
                overlay: false,
            };
        }
    }

    fn index(&self, x: usize, y: usize) -> Option<usize> {
        if x < self.cols && y < self.rows {
            Some(y * self.cols + x)
        } else {
            None
        }
    }

    pub fn cell(&self, x: usize, y: usize) -> Option<Cell> {
        self.index(x, y).map(|i| self.cells[i])
    }

    pub fn put(&mut self, x: usize, y: usize, ch: char, style: Style) {
        if let Some(i) = self.index(x, y) {
            self.cells[i] = Cell {
                ch,
                style,
                overlay: false,
            };
        }
    }

    /// A cell that floats above the page (menu, hint, help). Pixel backends draw
    /// the page first and these afterwards — that is what makes a popup over a
    /// real CSS-rendered page possible without a second compositor.
    pub fn put_overlay(&mut self, x: usize, y: usize, ch: char, style: Style) {
        if let Some(i) = self.index(x, y) {
            self.cells[i] = Cell {
                ch,
                style,
                overlay: true,
            };
        }
    }

    /// Overlay box: background + characters, all marked as floating above.
    pub fn overlay_box(
        &mut self,
        x: usize,
        y: usize,
        w: usize,
        h: usize,
        lines: &[(String, usize)],
        style: Style,
    ) {
        for row in y..y + h {
            for col in x..x + w {
                self.put_overlay(col, row, ' ', style);
            }
        }
        for (i, (text, indent)) in lines.iter().enumerate() {
            let yy = y + i;
            if yy >= y + h {
                break;
            }
            for (j, ch) in text.chars().skip(*indent).enumerate() {
                self.put_overlay(x + j, yy, ch, style);
            }
        }
    }

    /// Only the background, keeping whatever character is there (selection and
    /// highlight overlays do this).
    pub fn put_bg(&mut self, x: usize, y: usize, bg: Color) {
        if let Some(i) = self.index(x, y) {
            self.cells[i].style.bg = bg;
        }
    }

    /// Write a string, clipped at the right edge; returns the cells consumed.
    pub fn text(&mut self, x: usize, y: usize, s: &str, style: Style) -> usize {
        let mut n = 0;
        for ch in s.chars() {
            if x + n >= self.cols {
                break;
            }
            self.put(x + n, y, ch, style);
            n += 1;
        }
        n
    }

    /// Write a string inside a fixed width, padding with `pad` and eliding with
    /// `…` when it does not fit.
    #[allow(clippy::too_many_arguments)] // position, width, text, style, pad, clip
    pub fn text_in(
        &mut self,
        x: usize,
        y: usize,
        w: usize,
        s: &str,
        style: Style,
        pad: char,
        align_right: bool,
    ) {
        if w == 0 {
            return;
        }
        let chars: Vec<char> = s.chars().collect();
        if chars.len() > w {
            let mut v = chars[..w.saturating_sub(1)].to_vec();
            v.push('…');
            for (i, ch) in v.iter().enumerate() {
                self.put(x + i, y, *ch, style);
            }
            return;
        }
        let skip = if align_right { w - chars.len() } else { 0 };
        for i in 0..w {
            let ch = if i < skip {
                pad
            } else {
                match chars.get(i - skip) {
                    Some(c) => *c,
                    None => pad,
                }
            };
            self.put(x + i, y, ch, style);
        }
    }

    pub fn fill(&mut self, x: usize, y: usize, w: usize, h: usize, bg: Color) {
        for row in y..y + h {
            for col in x..x + w {
                if let Some(i) = self.index(col, row) {
                    self.cells[i].style.bg = bg;
                }
            }
        }
    }

    pub fn fill_styled(&mut self, x: usize, y: usize, w: usize, h: usize, ch: char, style: Style) {
        for row in y..y + h {
            for col in x..x + w {
                self.put(col, row, ch, style);
            }
        }
    }

    pub fn hline(&mut self, x: usize, y: usize, w: usize, ch: char, style: Style) {
        for col in x..x + w {
            self.put(col, y, ch, style);
        }
    }

    pub fn vline(&mut self, x: usize, y: usize, h: usize, ch: char, style: Style) {
        for row in y..y + h {
            self.put(x, row, ch, style);
        }
    }

    /// A box with a one-cell border (`title` sits in the top border).
    pub fn boxed(
        &mut self,
        x: usize,
        y: usize,
        w: usize,
        h: usize,
        title: &str,
        theme: &crate::theme::Theme,
    ) {
        if w < 2 || h < 2 {
            return;
        }
        let st = Style::new(theme.menu_fg, theme.menu_bg);
        let border = Style::new(theme.border, theme.menu_bg);
        self.fill_styled(x, y, w, h, ' ', st);
        self.hline(x, y, w, '─', border);
        self.hline(x, y + h - 1, w, '─', border);
        self.vline(x, y, h, '│', border);
        self.vline(x + w - 1, y, h, '│', border);
        self.put(x, y, '┌', border);
        self.put(x + w - 1, y, '┐', border);
        self.put(x, y + h - 1, '└', border);
        self.put(x + w - 1, y + h - 1, '┘', border);
        if !title.is_empty() {
            self.text(
                x + 2,
                y,
                title,
                Style::new(theme.fg, theme.menu_bg).bold(true),
            );
        }
    }

    /// A boxed popup that floats above the page (`title` in the top border).
    #[allow(clippy::too_many_arguments)] // x, y, w, h, title, theme — the box is the API
    pub fn boxed_overlay(
        &mut self,
        x: usize,
        y: usize,
        w: usize,
        h: usize,
        title: &str,
        theme: &crate::theme::Theme,
    ) {
        if w < 2 || h < 2 {
            return;
        }
        let bg = Style::new(theme.menu_fg, theme.menu_bg);
        let border = Style::new(theme.border, theme.menu_bg);
        for row in y..y + h {
            for col in x..x + w {
                self.put_overlay(col, row, ' ', bg);
            }
        }
        for col in x..x + w {
            self.put_overlay(col, y, '─', border);
            self.put_overlay(col, y + h - 1, '─', border);
        }
        for row in y..y + h {
            self.put_overlay(x, row, '│', border);
            self.put_overlay(x + w - 1, row, '│', border);
        }
        self.put_overlay(x, y, '┌', border);
        self.put_overlay(x + w - 1, y, '┐', border);
        self.put_overlay(x, y + h - 1, '└', border);
        self.put_overlay(x + w - 1, y + h - 1, '┘', border);
        if !title.is_empty() {
            for (i, ch) in title.chars().enumerate() {
                if x + 2 + i < x + w - 1 {
                    self.put_overlay(x + 2 + i, y, ch, bg.bold(true));
                }
            }
        }
    }

    pub fn row_text(&self, y: usize) -> String {
        let mut out = String::new();
        if y >= self.rows {
            return out;
        }
        for c in &self.cells[y * self.cols..(y + 1) * self.cols] {
            out.push(c.ch);
        }
        while out.ends_with(' ') {
            out.pop();
        }
        out
    }

    pub fn dump(&self) -> String {
        (0..self.rows)
            .map(|y| self.row_text(y))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Any cell containing `needle` (tests ask "did the URL show up?").
    pub fn contains(&self, needle: &str) -> bool {
        (0..self.rows).any(|y| self.row_text(y).contains(needle))
    }

    pub fn row_containing(&self, needle: &str) -> Option<usize> {
        (0..self.rows).find(|y| self.row_text(*y).contains(needle))
    }

    /// The style of the first cell that renders `needle` — tests check colours
    /// (link blue, selection, the current find match) without knowing layout.
    pub fn style_of(&self, needle: &str) -> Option<Style> {
        let want: Vec<char> = needle.chars().collect();
        if want.is_empty() {
            return None;
        }
        for y in 0..self.rows {
            let row: Vec<char> = (0..self.cols)
                .map(|x| self.cell(x, y).map(|c| c.ch).unwrap_or(' '))
                .collect();
            if row.len() < want.len() {
                continue;
            }
            for start in 0..=row.len() - want.len() {
                if row[start..start + want.len()] == want[..] {
                    return self.cell(start, y).map(|c| c.style);
                }
            }
        }
        None
    }

    /// Cells that differ between two screens (the terminal sends exactly these).
    pub fn diff<'a>(&'a self, other: &'a Screen) -> Vec<(usize, usize, Cell)> {
        let mut out = Vec::new();
        for y in 0..self.rows.min(other.rows) {
            for x in 0..self.cols.min(other.cols) {
                if self.cell(x, y) != other.cell(x, y) {
                    if let Some(c) = self.cell(x, y) {
                        out.push((x, y, c));
                    }
                }
            }
        }
        out
    }
}
