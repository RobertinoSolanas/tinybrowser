//! Text selection in the page.
//!
//! A selection is two points `(entry, character)` into [`PageText`], nothing
//! more. Everything else follows from that: the text to copy, the rectangles to
//! highlight, the word a double click selects, the line a triple click selects.
//! Because the layout already did the wrapping, we never have to guess what the
//! user meant to point at — the run under the cursor is exact.

use tb_core::Rect;

use crate::pagetext::PageText;

/// A position inside the page's text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Point {
    pub entry: usize,
    pub ch: usize,
}

impl Point {
    pub fn before(&self, other: &Point) -> bool {
        self.entry < other.entry || (self.entry == other.entry && self.ch < other.ch)
    }
}

/// The selection of one tab.
#[derive(Clone, Copy, Debug, Default)]
pub struct Selection {
    pub start: Option<Point>,
    pub end: Option<Point>,
}

impl Selection {
    pub fn is_empty(&self) -> bool {
        match (self.start, self.end) {
            (Some(a), Some(b)) => a == b,
            _ => true,
        }
    }

    pub fn clear(&mut self) {
        self.start = None;
        self.end = None;
    }

    /// A new selection anchored at `p` (mouse down).
    pub fn begin(&mut self, p: Point) {
        self.start = Some(p);
        self.end = Some(p);
    }

    /// Drag to `p`.
    pub fn extend(&mut self, p: Point) {
        if self.start.is_none() {
            self.start = Some(p);
        }
        self.end = Some(p);
    }

    /// The ordered range, `None` when empty.
    pub fn range(&self) -> Option<(Point, Point)> {
        let (a, b) = (self.start?, self.end?);
        if a == b {
            return None;
        }
        Some(if a.before(&b) { (a, b) } else { (b, a) })
    }

    pub fn contains(&self, p: Point) -> bool {
        match self.range() {
            Some((a, b)) => !p.before(&a) && p.before(&b),
            None => false,
        }
    }

    /// Select the word around `p` (a double click).
    pub fn word(pt: &PageText, p: Point) -> Selection {
        let Some(entry) = pt.entries.get(p.entry) else {
            return Selection::default();
        };
        let chars: Vec<char> = entry.text.chars().collect();
        if chars.is_empty() {
            return Selection::default();
        }
        let i = p.ch.min(chars.len().saturating_sub(1));
        let mut a = i;
        while a > 0 && is_word(chars[a - 1]) {
            a -= 1;
        }
        let mut b = i + 1;
        while b < chars.len() && is_word(chars[b]) {
            b += 1;
        }
        Selection {
            start: Some(Point {
                entry: p.entry,
                ch: a,
            }),
            end: Some(Point {
                entry: p.entry,
                ch: b,
            }),
        }
    }

    /// Select the whole line the point is on (a triple click): every entry that
    /// shares the point's baseline.
    pub fn line(pt: &PageText, p: Point) -> Selection {
        let Some(base) = pt.entries.get(p.entry).map(|e| e.baseline) else {
            return Selection::default();
        };
        let mut first = None;
        let mut last = None;
        for (i, e) in pt.entries.iter().enumerate() {
            if (e.baseline - base).abs() <= 2.0 {
                if first.is_none() {
                    first = Some(i);
                }
                last = Some(i);
            }
        }
        let (f, l) = match (first, last) {
            (Some(f), Some(l)) => (f, l),
            _ => return Selection::default(),
        };
        Selection {
            start: Some(Point { entry: f, ch: 0 }),
            end: Some(Point {
                entry: l,
                ch: pt.entries[l].text.chars().count(),
            }),
        }
    }

    pub fn all(pt: &PageText) -> Selection {
        match pt.entries.len() {
            0 => Selection::default(),
            n => Selection {
                start: Some(Point { entry: 0, ch: 0 }),
                end: Some(Point {
                    entry: n - 1,
                    ch: pt.entries[n - 1].text.chars().count(),
                }),
            },
        }
    }

    /// The selected text, with a newline between lines (so a copied table or
    /// list survives the trip to the clipboard).
    pub fn text(&self, pt: &PageText) -> String {
        let Some((a, b)) = self.range() else {
            return String::new();
        };
        let mut out = String::new();
        let mut last_baseline = f32::MIN;
        for i in a.entry..=b.entry.min(pt.entries.len().saturating_sub(1)) {
            let e = &pt.entries[i];
            let from = if i == a.entry { a.ch } else { 0 };
            let to = if i == b.entry {
                b.ch
            } else {
                e.text.chars().count()
            };
            if to <= from {
                continue;
            }
            if !out.is_empty() && (e.baseline - last_baseline).abs() > 1.0 {
                out.push('\n');
            }
            let picked: String = e.chars().skip(from).take(to - from).collect();
            out.push_str(&picked);
            last_baseline = e.baseline;
        }
        out
    }

    /// Rectangles to paint (document coordinates).
    pub fn rects(&self, pt: &PageText) -> Vec<Rect> {
        let Some((a, b)) = self.range() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for i in a.entry..=b.entry.min(pt.entries.len().saturating_sub(1)) {
            let e = &pt.entries[i];
            let from = if i == a.entry { a.ch } else { 0 };
            let to = if i == b.entry {
                b.ch
            } else {
                e.text.chars().count()
            };
            if to <= from {
                continue;
            }
            out.push(e.char_rect(from, to));
        }
        out
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '\'' || c == '_' || c == '-'
}
