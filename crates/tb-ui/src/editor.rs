//! The single-line text editor of the address bar, the find bar and the command
//! line — the same editor every one of them uses, with the behaviours a user
//! expects without asking for them: caret, selection, word motions, `Home`/
//! `End`, `Ctrl+W`, kill-to-end, and ↑/↓ through what was typed before.
//!
//! Caret and selection are **character indices into `chars`**, `0`…`len`, where
//! `caret == len` means "behind the last character". `chars` rather than
//! `String` because `ä` and `€` are one position for a user and four bytes for
//! a program, and because `Home` in the middle of `https://…` has to be exact.

/// A line of editable text with a caret and an optional selection anchor.
#[derive(Clone, Debug, Default)]
pub struct Editor {
    pub chars: Vec<char>,
    pub caret: usize,
    /// `Some` while selecting; the caret is the other end.
    pub anchor: Option<usize>,
    /// What was entered before, oldest first (`↑` walks it).
    pub history: Vec<String>,
    /// `history.len()` while typing something new.
    pub hist_i: usize,
    /// Saved draft so `↑ ↑ ↓` gives back what was typed before the walk.
    draft: String,
}

impl Editor {
    pub fn new() -> Editor {
        Editor {
            chars: Vec::new(),
            caret: 0,
            anchor: None,
            history: Vec::new(),
            hist_i: 0,
            draft: String::new(),
        }
    }

    pub fn with_text(s: &str) -> Editor {
        let mut e = Editor::new();
        e.set(s);
        e
    }

    pub fn value(&self) -> String {
        self.chars.iter().collect()
    }

    pub fn len(&self) -> usize {
        self.chars.len()
    }

    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    pub fn set(&mut self, s: &str) {
        self.chars = s.chars().collect();
        self.caret = self.chars.len();
        self.anchor = None;
    }

    pub fn set_caret(&mut self, i: usize) {
        self.caret = i.min(self.chars.len());
        self.anchor = None;
    }

    pub fn clear(&mut self) {
        self.chars.clear();
        self.caret = 0;
        self.anchor = None;
    }

    /// The selected range `(from, to)` with `from < to`, `None` when empty.
    pub fn selection(&self) -> Option<(usize, usize)> {
        let a = self.anchor?;
        if a == self.caret {
            return None;
        }
        Some((a.min(self.caret), a.max(self.caret)))
    }

    pub fn selected_text(&self) -> String {
        match self.selection() {
            Some((a, b)) => self.chars[a..b].iter().collect(),
            None => String::new(),
        }
    }

    /// Delete the selection, leaving the caret at its start.
    fn kill_selection(&mut self) -> bool {
        match self.selection() {
            Some((a, b)) => {
                self.chars.drain(a..b);
                self.caret = a;
                self.anchor = None;
                true
            }
            None => false,
        }
    }

    pub fn insert(&mut self, c: char) {
        if c == '\n' || c == '\r' {
            return;
        }
        self.kill_selection();
        self.chars.insert(self.caret.min(self.chars.len()), c);
        self.caret += 1;
    }

    pub fn insert_str(&mut self, s: &str) {
        for c in s.chars().filter(|c| !c.is_control()) {
            self.insert(c);
        }
    }

    pub fn backspace(&mut self) {
        if !self.kill_selection() && self.caret > 0 {
            self.caret -= 1;
            self.chars.remove(self.caret);
        }
    }

    pub fn delete(&mut self) {
        if !self.kill_selection() && self.caret < self.chars.len() {
            self.chars.remove(self.caret);
        }
    }

    pub fn left(&mut self, select: bool) {
        self.move_to(self.caret.saturating_sub(1), select);
    }

    pub fn right(&mut self, select: bool) {
        self.move_to((self.caret + 1).min(self.chars.len()), select);
    }

    pub fn home(&mut self, select: bool) {
        self.move_to(0, select);
    }

    pub fn end(&mut self, select: bool) {
        self.move_to(self.chars.len(), select);
    }

    pub fn move_to(&mut self, i: usize, select: bool) {
        if select {
            if self.anchor.is_none() {
                self.anchor = Some(self.caret);
            }
        } else {
            self.anchor = None;
        }
        self.caret = i.min(self.chars.len());
    }

    pub fn word_left(&mut self, select: bool) {
        let mut i = self.caret;
        while i > 0 && !is_word(self.chars[i - 1]) {
            i -= 1;
        }
        while i > 0 && is_word(self.chars[i - 1]) {
            i -= 1;
        }
        self.move_to(i, select);
    }

    pub fn word_right(&mut self, select: bool) {
        let mut i = self.caret;
        while i < self.chars.len() && is_word(self.chars[i]) {
            i += 1;
        }
        while i < self.chars.len() && !is_word(self.chars[i]) {
            i += 1;
        }
        self.move_to(i, select);
    }

    /// `Ctrl+W` — delete the word to the left.
    pub fn delete_word_left(&mut self) {
        // measure with a copy, then cut: the copy is a handful of chars and the
        // alternative (an index-returning motion) would leak the walk
        let from = {
            let mut e = Editor {
                chars: self.chars.clone(),
                caret: self.caret,
                anchor: None,
                history: Vec::new(),
                hist_i: 0,
                draft: String::new(),
            };
            e.word_left(false);
            e.caret
        };
        if from < self.caret {
            self.chars.drain(from..self.caret);
            self.caret = from;
            self.anchor = None;
        }
    }

    pub fn kill_to_end(&mut self) {
        if self.caret < self.chars.len() {
            self.chars.truncate(self.caret);
        }
    }

    pub fn select_all(&mut self) {
        self.anchor = Some(0);
        self.caret = self.chars.len();
    }

    pub fn select_word_at(&mut self, index: usize) {
        let i = index.min(self.chars.len().saturating_sub(1));
        if self.chars.is_empty() {
            return;
        }
        let mut a = i;
        while a > 0 && is_word(self.chars[a - 1]) {
            a -= 1;
        }
        let mut b = i + 1;
        while b < self.chars.len() && is_word(self.chars[b]) {
            b += 1;
        }
        self.anchor = Some(a);
        self.caret = b;
    }

    /// `Tab` completes the typed prefix from `candidates` (Chrome's omnibox
    /// behaviour: longest common prefix, nothing when ambiguous).
    pub fn complete(&mut self, candidates: &[String]) -> bool {
        let typed = self.value();
        if typed.is_empty() {
            return false;
        }
        let hits: Vec<&String> = candidates
            .iter()
            .filter(|c| c.as_str() != typed && c.starts_with(typed.as_str()))
            .collect();
        if hits.is_empty() {
            return false;
        }
        let mut common: Vec<char> = hits[0].chars().collect();
        for h in &hits[1..] {
            let hc: Vec<char> = h.chars().collect();
            let mut n = 0;
            while n < common.len() && n < hc.len() && common[n] == hc[n] {
                n += 1;
            }
            common.truncate(n);
        }
        if common.len() <= typed.chars().count() {
            return false;
        }
        self.set(&common.iter().collect::<String>());
        true
    }

    /// `↑`/`↓` through the history; `up=false` means down.
    pub fn history_step(&mut self, up: bool) {
        if self.history.is_empty() {
            return;
        }
        if self.hist_i == self.history.len() {
            self.draft = self.value();
        }
        self.hist_i = if up {
            self.hist_i.saturating_sub(1)
        } else {
            (self.hist_i + 1).min(self.history.len())
        };
        let next = if self.hist_i == self.history.len() {
            self.draft.clone()
        } else {
            self.history[self.hist_i].clone()
        };
        self.set(&next);
    }

    /// Remember a submitted value (consecutive duplicates are not history).
    pub fn remember(&mut self, s: &str) {
        if s.trim().is_empty() {
            return;
        }
        if self.history.last().map(|v| v == s).unwrap_or(false) {
            return;
        }
        self.history.push(s.to_string());
        if self.history.len() > 200 {
            self.history.remove(0);
        }
        self.hist_i = self.history.len();
        self.draft.clear();
    }

    /// The text to show in a field of `width` cells: Chrome elides the middle of
    /// a long URL so the interesting part (host + path tail) stays visible.
    pub fn display(&self, width: usize) -> String {
        let s = self.value();
        let chars: Vec<char> = s.chars().collect();
        if chars.len() <= width {
            return s;
        }
        if width < 8 {
            return chars[..width].iter().collect();
        }
        let head = 6.min(width / 3);
        let tail = width - head - 1;
        let mut out: String = chars[..head].iter().collect();
        out.push('…');
        out.push_str(&chars[chars.len() - tail..].iter().collect::<String>());
        out
    }

    /// Where the caret sits inside `display(width)` — a field showing the middle
    /// of a URL has to move the cursor with it, or the caret "runs away".
    pub fn display_caret(&self, width: usize) -> usize {
        let total = self.chars.len();
        if total <= width {
            return self.caret;
        }
        let head = 6.min(width / 3);
        let tail = width.saturating_sub(head + 1);
        if self.caret <= head {
            self.caret
        } else if self.caret >= total.saturating_sub(tail) {
            head + 1 + (self.caret - (total - tail))
        } else {
            head + 1
        }
        .min(width.saturating_sub(1))
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-' || c == '.'
}
