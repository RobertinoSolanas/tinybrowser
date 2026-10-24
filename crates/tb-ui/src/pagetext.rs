//! The page as *text with geometry* — what selection, find-in-page, the
//! inspector and the cell renderer all work on.
//!
//! The layout already produced text runs (position, baseline, style, owning
//! text node). This turns them into a flat, indexable list in reading order and
//! adds the two things a UI needs to know about every fragment: **which column
//! one character occupies** (our font is monospace per scale, so a character is
//! `advance` px wide and an offset is exact without measuring) and **which link
//! it belongs to** (so a link can be underlined, coloured, clicked and hinted).

use tb_core::{Color, Rect};
use tb_dom::NodeId;
use tb_gfx::Font;

/// One fragment of rendered text.
#[derive(Clone, Debug)]
pub struct Entry {
    /// Document coordinates (not scrolled).
    pub rect: Rect,
    pub baseline: f32,
    pub text: String,
    pub node: NodeId,
    pub font_size: f32,
    /// Width of one character of this run.
    pub advance: f32,
    pub bold: bool,
    /// CSS `color` / `background-color` — the cell renderer paints with these.
    pub color: Color,
    pub bg: Color,
    /// CSS `text-decoration: underline` (or the run being a link).
    pub underline: bool,
    pub link: Option<String>,
}

impl Entry {
    /// The rectangle of characters `[from..to]` (selection and highlight).
    pub fn char_rect(&self, from: usize, to: usize) -> Rect {
        let a = from.min(self.text.chars().count()) as f32;
        let b = to.min(self.text.chars().count()).max(from) as f32;
        Rect::new(
            self.rect.x + a * self.advance,
            self.rect.y,
            (b - a) * self.advance,
            self.rect.height,
        )
    }

    pub fn chars(&self) -> impl Iterator<Item = char> + '_ {
        self.text.chars()
    }
}

/// The whole page's text, in reading order.
#[derive(Clone, Debug, Default)]
pub struct PageText {
    pub entries: Vec<Entry>,
}

impl PageText {
    /// Snapshot the laid-out page. Cheap enough to do per frame for a document
    /// of a few thousand runs (it copies the text, nothing else).
    pub fn from_page(page: &tb_browser::Page) -> PageText {
        let font = Font::system();
        let mut entries = Vec::new();
        let runs: Vec<_> = page
            .text_runs()
            .into_iter()
            .filter(|r| !r.text.trim().is_empty())
            .collect();
        for (i, run) in runs.iter().enumerate() {
            let size = run.style.font_size;
            let advance = font.advance(size);
            let ascent = size * 0.85;
            let link = page.closest_link(run.node);
            entries.push(Entry {
                rect: Rect::new(
                    run.x,
                    run.baseline - ascent,
                    run.width,
                    run.style.line_box(),
                ),
                baseline: run.baseline,
                text: run.text.clone(),
                node: run.node,
                font_size: size,
                advance: advance.max(1.0),
                bold: run.style.font_weight >= 600,
                color: run.style.color,
                bg: run.style.background_color,
                underline: run.style.text_decoration,
                link,
            });
            // Layout breaks lines between words, so the space that separated two
            // runs is not in either of them — but the user reads it, and copy
            // and find must see it. A gap on the same baseline is that space.
            if let Some(next) = runs.get(i + 1) {
                let same_line = (next.baseline - run.baseline).abs() < 0.5;
                let gap = next.x - (run.x + run.width);
                if same_line && gap > 0.1 {
                    let width = gap.min(advance.max(1.0));
                    entries.push(Entry {
                        rect: Rect::new(
                            run.x + run.width,
                            run.baseline - ascent,
                            width,
                            run.style.line_box(),
                        ),
                        baseline: run.baseline,
                        text: " ".to_string(),
                        node: run.node,
                        font_size: size,
                        advance: advance.max(1.0),
                        bold: false,
                        color: run.style.color,
                        bg: run.style.background_color,
                        underline: false,
                        link: None,
                    });
                }
            }
        }
        PageText { entries }
    }

    /// The text entry under a **document** point, and the character index inside
    /// it (`None` when the point is on no text).
    pub fn hit(&self, doc_x: f32, doc_y: f32) -> Option<(usize, usize)> {
        let mut best: Option<(usize, usize, f32)> = None;
        for (i, e) in self.entries.iter().enumerate() {
            let in_x = doc_x >= e.rect.x - 0.5 && doc_x <= e.rect.right() + 0.5;
            let in_y = doc_y >= e.rect.y - 1.0 && doc_y <= e.rect.bottom() + 1.0;
            if !in_x {
                continue;
            }
            // a click between words still lands on the nearer word
            let idx =
                (((doc_x - e.rect.x) / e.advance).round() as usize).min(e.text.chars().count());
            let y_dist = if in_y {
                0.0
            } else {
                (doc_y - (e.rect.y + e.rect.height / 2.0)).abs()
            };
            let rank = if in_y { 0.0 } else { y_dist };
            match &best {
                Some((_, _, d)) if *d <= rank => (),
                _ => best = Some((i, idx, rank)),
            }
        }
        best.map(|(i, idx, _)| (i, idx))
    }

    /// Character index of entry `i` at an x position (a caret in a text field).
    pub fn index_in(&self, entry: usize, doc_x: f32) -> usize {
        match self.entries.get(entry) {
            Some(e) => (((doc_x - e.rect.x) / e.advance).round() as isize).max(0) as usize,
            None => 0,
        }
    }

    /// All entries whose rectangle touches `view` (document coordinates).
    pub fn visible(&self, view: Rect) -> Vec<usize> {
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.rect.bottom() > view.top() && e.rect.top() < view.bottom())
            .map(|(i, _)| i)
            .collect()
    }

    /// Everything visible, as text — the cell renderer walks this.
    pub fn visible_text(&self, view: Rect) -> Vec<&Entry> {
        self.visible(view)
            .into_iter()
            .map(|i| &self.entries[i])
            .collect()
    }

    /// The whole page's text, paragraphs separated (used by `--text`-like
    /// commands and by `Ctrl+A` + copy).
    pub fn all_text(&self) -> String {
        let mut out = String::new();
        let mut last_baseline = f32::MIN;
        for e in &self.entries {
            if (e.baseline - last_baseline).abs() > 1.0 {
                if !out.is_empty() {
                    out.push('\n');
                }
                last_baseline = e.baseline;
            } else if !out.is_empty() && !out.ends_with(' ') {
                out.push(' ');
            }
            out.push_str(&e.text);
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
}
