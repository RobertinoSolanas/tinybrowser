//! Find in page.
//!
//! Matches are `(entry, from, to)` into [`PageText`], computed case
//! insensitively over each run — and, to be useful, also across runs: "taz"
//! inside `<p>the <b>taz</b>-de</p>` is one word for the user even though the
//! markup made two runs. The simple version we do: search each run, then search
//! the concatenation of a line and map hits back to the run that owns the
//! character. Current match is remembered by index, so `n`/`N` wrap.

use tb_core::Rect;

use crate::pagetext::PageText;

/// One match.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Found {
    pub entry: usize,
    pub from: usize,
    pub to: usize,
}

/// The find state of one tab.
#[derive(Clone, Debug, Default)]
pub struct Find {
    pub needle: String,
    pub matches: Vec<Found>,
    /// Index into `matches` of the highlighted one.
    pub index: usize,
}

impl Find {
    /// Recompute the matches for `needle` (empty needle = no matches).
    ///
    /// `keep` tries to stay on the same match the user was looking at, so
    /// typing another character does not jump back to the top of the page.
    pub fn search(&mut self, pt: &PageText, needle: &str) {
        let keep = self.current().copied();
        self.needle = needle.to_string();
        self.matches = match_matches(pt, needle);
        self.index = match keep {
            Some(k) => self
                .matches
                .iter()
                .position(|f| f.entry == k.entry && f.from >= k.from)
                .unwrap_or(0),
            None => 0,
        };
    }

    pub fn count(&self) -> usize {
        self.matches.len()
    }

    pub fn current(&self) -> Option<&Found> {
        self.matches.get(self.index)
    }

    /// `n` — advance, wrapping around.
    pub fn advance(&mut self) -> Option<Found> {
        if self.matches.is_empty() {
            return None;
        }
        self.index = (self.index + 1) % self.matches.len();
        Some(self.matches[self.index])
    }

    /// `N` — backwards, wrapping around.
    pub fn rewind(&mut self) -> Option<Found> {
        if self.matches.is_empty() {
            return None;
        }
        self.index = if self.index == 0 {
            self.matches.len() - 1
        } else {
            self.index - 1
        };
        Some(self.matches[self.index])
    }

    /// The rectangle of the current match, document coordinates.
    pub fn current_rect(&self, pt: &PageText) -> Option<Rect> {
        let f = self.current()?;
        let e = pt.entries.get(f.entry)?;
        Some(e.char_rect(f.from, f.to))
    }

    pub fn clear(&mut self) {
        self.needle.clear();
        self.matches.clear();
        self.index = 0;
    }
}

/// All case-insensitive occurrences of `needle`, per run and across runs that
/// share a baseline.
pub fn match_matches(pt: &PageText, needle: &str) -> Vec<Found> {
    let mut out = Vec::new();
    if needle.is_empty() {
        return out;
    }
    let want: Vec<char> = needle.to_lowercase().chars().collect();

    // 1. inside one run
    for (i, e) in pt.entries.iter().enumerate() {
        let hay: Vec<char> = e.text.to_lowercase().chars().collect();
        let mut p = 0;
        while p + want.len() <= hay.len() {
            if hay[p..p + want.len()] == want[..] {
                out.push(Found {
                    entry: i,
                    from: p,
                    to: p + want.len(),
                });
                p += want.len();
            } else {
                p += 1;
            }
        }
    }

    // 2. across runs of one line ("ta|z" split by a <b>)
    let mut i = 0;
    while i < pt.entries.len() {
        let base = pt.entries[i].baseline;
        let mut j = i;
        let mut line: Vec<char> = Vec::new();
        // (entry, offset of that entry inside `line`)
        let mut owners: Vec<(usize, usize)> = Vec::new();
        while j < pt.entries.len() && (pt.entries[j].baseline - base).abs() <= 2.0 {
            let start = line.len();
            for c in pt.entries[j].text.to_lowercase().chars() {
                line.push(c);
            }
            owners.push((j, start));
            j += 1;
        }
        if owners.len() > 1 && line.len() >= want.len() {
            let mut p = 0;
            while p + want.len() <= line.len() {
                if line[p..p + want.len()] == want[..] {
                    // report it on the entry where it starts — and only when it
                    // runs past that entry (a hit inside one run is found above)
                    let mut owner: Option<(usize, usize)> = None;
                    for &(e, ls) in &owners {
                        let len = pt.entries[e].text.chars().count();
                        if ls <= p && p < ls + len {
                            owner = Some((e, ls));
                            break;
                        }
                    }
                    if let Some((entry, ls)) = owner {
                        let entry_end = ls + pt.entries[entry].text.chars().count();
                        if p + want.len() > entry_end {
                            out.push(Found {
                                entry,
                                from: p - ls,
                                to: pt.entries[entry].text.chars().count(),
                            });
                        }
                    }
                    p += want.len();
                } else {
                    p += 1;
                }
            }
        }
        i = j.max(i + 1);
    }
    out.sort_by_key(|f| (f.entry, f.from));
    out.dedup();
    out
}
