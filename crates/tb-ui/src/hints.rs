//! Keyboard link activation ("hints", the `f`/`F` of vimium and Firefox'
//! `f`).
//!
//! Every link the user can see gets a short label; typing the label activates
//! it, `F`-style opens it in a new tab. The labels are the shortest base-26
//! strings that stay unique (`a`, `b`, …, `za`), typing filters as you go and
//! the last candidate is taken when there is nothing left to disambiguate.

use tb_core::Rect;
use tb_dom::NodeId;

/// One hinted element.
#[derive(Clone, Debug, PartialEq)]
pub struct Hint {
    pub label: String,
    pub node: NodeId,
    /// Document coordinates (the cell renderer turns them into screen rows).
    pub rect: Rect,
    pub href: Option<String>,
    pub text: String,
}

/// The result of typing into hint mode.
#[derive(Clone, Debug, PartialEq)]
pub enum HintStep {
    /// Keep typing.
    More,
    /// This hint is chosen.
    Open(Hint),
    /// No hint starts with what was typed — the mode is over.
    Cancel,
}

#[derive(Clone, Debug, Default)]
pub struct HintState {
    pub active: bool,
    pub new_tab: bool,
    pub typed: String,
    pub hints: Vec<Hint>,
}

impl HintState {
    /// Build the hints for every focusable element inside `view` (document
    /// coordinates) that has a box with a real size.
    pub fn build(page: &tb_browser::Page, view: Rect, new_tab: bool) -> HintState {
        let mut hints: Vec<Hint> = Vec::new();
        for node in page.focusables() {
            let Some(r) = page.rect_of(node) else {
                continue;
            };
            if r.width < 1.0 || r.height < 1.0 {
                continue;
            }
            if r.bottom() < view.top() || r.top() > view.bottom() {
                continue;
            }
            if r.right() < view.left() || r.left() > view.right() {
                continue;
            }
            let href = page.closest_link(node);
            let text = tb_core::text::truncate(&page.describe(node), 60);
            hints.push(Hint {
                label: String::new(),
                node,
                rect: r,
                href,
                text,
            });
        }
        let mut st = HintState {
            active: !hints.is_empty(),
            new_tab,
            typed: String::new(),
            hints,
        };
        st.assign_labels();
        st
    }

    /// Shortest unique labels: `len = ceil(log26(n))`, then `a..z`, `aa..`.
    pub fn assign_labels(&mut self) {
        let n = self.hints.len();
        if n == 0 {
            return;
        }
        let mut len = 1;
        while 26usize.pow(len) < n {
            len += 1;
        }
        for (i, h) in self.hints.iter_mut().enumerate() {
            let mut v = i;
            let mut label = String::new();
            for _ in 0..len {
                label.insert(0, (b'a' + (v % 26) as u8) as char);
                v /= 26;
            }
            h.label = label;
        }
    }

    /// Hints that still match what has been typed.
    pub fn candidates(&self) -> Vec<&Hint> {
        if self.typed.is_empty() {
            return self.hints.iter().collect();
        }
        self.hints
            .iter()
            .filter(|h| h.label.starts_with(&self.typed))
            .collect()
    }

    /// Type one character of a label.
    pub fn press(&mut self, c: char) -> HintStep {
        if !self.active {
            return HintStep::Cancel;
        }
        let lc = c.to_ascii_lowercase();
        if !lc.is_ascii_alphabetic() {
            return HintStep::Cancel;
        }
        self.typed.push(lc);
        let cands = self.candidates();
        match cands.len() {
            0 => HintStep::Cancel,
            1 if cands[0].label == self.typed => {
                let h = cands[0].clone();
                self.reset();
                HintStep::Open(h)
            }
            _ if self.typed.chars().count()
                >= self
                    .hints
                    .iter()
                    .map(|h| h.label.chars().count())
                    .max()
                    .unwrap_or(0) =>
            {
                // typed everything of the shortest match: take it
                let h = cands[0].clone();
                self.reset();
                HintStep::Open(h)
            }
            _ => HintStep::More,
        }
    }

    /// `Enter` picks the single remaining candidate, if any.
    pub fn enter(&mut self) -> Option<Hint> {
        let cands = self.candidates();
        let h = match cands.len() {
            0 => None,
            _ => Some(cands[0].clone()),
        };
        if h.is_some() {
            self.reset();
        }
        h
    }

    pub fn reset(&mut self) {
        self.active = false;
        self.typed.clear();
        self.hints.clear();
    }
}
