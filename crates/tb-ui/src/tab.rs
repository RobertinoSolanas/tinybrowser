//! A tab: one document plus everything that belongs to *showing* it.
//!
//! `Page` is the document; a `Tab` is the view. Scroll offset, per-tab session
//! history, the selection, the find state, the hovered link, which form control
//! has the caret and where in its value that caret sits — none of that is part
//! of the document, all of it must survive switching tabs and must not leak into
//! the next page that loads in the same tab.

use tb_browser::Page;
use tb_dom::NodeId;

use crate::pagetext::PageText;
use crate::search::Find;
use crate::selection::Selection;

/// One open document.
pub struct Tab {
    pub page: Page,
    /// Scroll offset in CSS px.
    pub scroll_x: f32,
    pub scroll_y: f32,
    /// This tab's own session history (the engine's history is shared and is
    /// not what `Alt+Left` in one tab should walk).
    pub history: Vec<String>,
    pub hist_index: usize,
    pub selection: Selection,
    pub find: Find,
    /// Link under the mouse (for the status line and the middle click).
    pub hover_link: Option<String>,
    /// …and the element it belongs to (so a move over the same link is free).
    pub hover_node: Option<NodeId>,
    /// The control the caret is in, and the caret position inside its value.
    pub caret_node: Option<NodeId>,
    pub caret_index: usize,
    /// A load is in flight (spinner) or the page asked for another one.
    pub loading: bool,
}

impl Tab {
    /// A tab showing `page`, with `history` as its session history.
    pub fn with_page(page: Page, history: Vec<String>) -> Tab {
        let hist_index = history.len().saturating_sub(1);
        Tab {
            page,
            scroll_x: 0.0,
            scroll_y: 0.0,
            history,
            hist_index,
            selection: Selection::default(),
            find: Find::default(),
            hover_link: None,
            hover_node: None,
            caret_node: None,
            caret_index: 0,
            loading: false,
        }
    }

    /// A tab for a freshly loaded document (history starts here).
    pub fn new(page: Page) -> Tab {
        let url = page.url.to_string();
        Tab::with_page(page, vec![url])
    }

    /// The tab's title: `document.title`, falling back to the URL like a real
    /// browser does for a document without one.
    pub fn title(&self) -> String {
        let t = self.page.title();
        if t.trim().is_empty() {
            let u = self.page.url.to_string();
            u.trim_start_matches("https://")
                .trim_start_matches("http://")
                .to_string()
        } else {
            t
        }
    }

    /// Current scroll offset (CSS px).
    pub fn scroll_pos(&self) -> (f32, f32) {
        (self.scroll_x, self.scroll_y)
    }

    /// Replace the document and forget everything that described the old one.
    pub fn replace_page(&mut self, page: Page, push_history: bool) {
        let url = page.url.to_string();
        self.page = page;
        self.scroll_x = 0.0;
        self.scroll_y = 0.0;
        self.selection.clear();
        self.find.clear();
        self.hover_link = None;
        self.hover_node = None;
        self.caret_node = None;
        self.caret_index = 0;
        if push_history {
            if self.hist_index + 1 < self.history.len() {
                self.history.truncate(self.hist_index + 1);
            }
            if self.history.last().map(|u| *u != url).unwrap_or(true) {
                self.history.push(url);
            }
            self.hist_index = self.history.len().saturating_sub(1);
        }
    }

    pub fn can_back(&self) -> bool {
        self.hist_index > 0
    }

    pub fn can_forward(&self) -> bool {
        self.hist_index + 1 < self.history.len()
    }

    pub fn back_url(&self) -> Option<&str> {
        if self.can_back() {
            Some(&self.history[self.hist_index - 1])
        } else {
            None
        }
    }

    pub fn forward_url(&self) -> Option<&str> {
        if self.can_forward() {
            Some(&self.history[self.hist_index + 1])
        } else {
            None
        }
    }

    /// The page's text with geometry (selection, find, hit testing).
    pub fn text(&self) -> PageText {
        PageText::from_page(&self.page)
    }

    /// Is the caret in something you can type into?
    pub fn caret_is_text(&self) -> bool {
        let Some(node) = self.caret_node else {
            return false;
        };
        match self.page.dom.tag(node) {
            Some("textarea") => true,
            Some("input") => !matches!(
                self.page
                    .dom
                    .attr(node, "type")
                    .as_deref()
                    .unwrap_or("text"),
                "checkbox"
                    | "radio"
                    | "submit"
                    | "button"
                    | "image"
                    | "reset"
                    | "hidden"
                    | "file"
                    | "color"
            ),
            _ => false,
        }
    }

    /// The value under the caret, as text.
    pub fn caret_value(&self) -> String {
        match self.caret_node {
            Some(n) => self.page.input_value(n),
            None => String::new(),
        }
    }
}
