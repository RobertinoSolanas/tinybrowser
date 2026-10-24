//! The shell's application state: tabs, modes, commands, navigation.
//!
//! `App` is the only thing in `tb-ui` that owns an `Engine`, and the only place
//! that decides what a user action *does*. Input arrives through
//! [`App::on_event`] (implemented in [`crate::input`], a second `impl App`),
//! commands run through [`App::run`] (in [`crate::commands`]), and the renderers
//! read the result. Nothing here knows whether it is drawn by a terminal or by
//! X11 — that is what [`crate::screen::Screen`] and [`crate::backend::Backend`]
//! are for.
//!
//! ```text
//!   Event ──▶ on_event ──▶ Command ──▶ run() ──▶ Engine / Page ──▶ dirty
//! ```

use std::path::PathBuf;
use std::time::Instant;

use tb_browser::{Engine, EngineConfig, NavRequest, Page, PaintOptions, Url};
use tb_core::Rect;
use tb_dom::NodeId;

use crate::bookmarks::Bookmarks;
use crate::chrome::{Chrome, CELL_H, CELL_W};
use crate::clipboard::Clipboard;
use crate::devtools::{Devtools, LineKind, Panel};
use crate::editor::Editor;
use crate::hints::HintState;
use crate::mode::{Caret, Mode};
use crate::pagetext::PageText;
use crate::tab::Tab;
use crate::theme::Theme;

/// Shell settings that are neither per-page nor colours.
#[derive(Clone, Debug)]
pub struct UiConfig {
    /// `Alt+H` and the page to open when nothing else was given.
    pub home: String,
    /// `{}` is replaced by the query.
    pub search_url: String,
    pub show_chrome: bool,
    pub show_bookmarks: bool,
    /// CSS px → UI px. Integers look best: our bitmap font scales by integers.
    pub scale: f32,
    /// Let `:hover` restyle the page (pretty, expensive on big documents).
    pub hover_css: bool,
    pub timeout_ms: u64,
    /// `None` = the config dir, `Some(path)` = that file, `Some("")` = disabled.
    pub bookmark_file: Option<PathBuf>,
    pub run_scripts: bool,
    pub apply_css: bool,
    pub fetch: bool,
}

impl Default for UiConfig {
    fn default() -> UiConfig {
        UiConfig {
            home: "about:blank".to_string(),
            search_url: "https://duckduckgo.com/?q={}".to_string(),
            show_chrome: true,
            show_bookmarks: true,
            scale: 1.0,
            hover_css: false,
            timeout_ms: 20_000,
            bookmark_file: None,
            run_scripts: true,
            apply_css: true,
            fetch: true,
        }
    }
}

/// An entry of a popup menu; `cmd: None` is a separator.
#[derive(Clone, Debug)]
pub struct MenuItem {
    pub label: String,
    pub cmd: Option<crate::keymap::Command>,
}

impl MenuItem {
    pub fn item(label: &str, cmd: crate::keymap::Command) -> MenuItem {
        MenuItem {
            label: label.to_string(),
            cmd: Some(cmd),
        }
    }

    pub fn sep() -> MenuItem {
        MenuItem {
            label: "─".repeat(24),
            cmd: None,
        }
    }
}

/// An open menu (hamburger or context menu).
#[derive(Clone, Debug)]
pub struct Menu {
    pub items: Vec<MenuItem>,
    pub sel: usize,
    /// Top-left corner in cells.
    pub col: usize,
    pub row: usize,
    /// The link a context menu was opened on.
    pub link: Option<String>,
}

impl Menu {
    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The screen row of item `index`.
    pub fn row_of(&self, index: usize) -> usize {
        self.row + 1 + index
    }

    /// Which item a screen row points at (skipping separators).
    pub fn index_at_row(&self, row: usize) -> Option<usize> {
        if row <= self.row {
            return None;
        }
        let i = row - self.row - 1;
        if i < self.items.len() && self.items[i].cmd.is_some() {
            Some(i)
        } else {
            None
        }
    }

    /// Move the selection, skipping separators.
    pub fn select_next(&mut self, dir: i32) {
        let n = self.items.len();
        if n == 0 {
            return;
        }
        let mut i = self.sel;
        for _ in 0..n {
            i = if dir > 0 {
                (i + 1) % n
            } else {
                (i + n - 1) % n
            };
            if self.items[i].cmd.is_some() {
                break;
            }
        }
        self.sel = i;
    }
}

/// An open `<select>` popup.
#[derive(Clone, Debug)]
pub struct SelectPopup {
    pub node: NodeId,
    pub options: Vec<String>,
    /// The highlighted option.
    pub index: usize,
    pub col: usize,
    pub row: usize,
}

/// One pointer press, remembered until the matching release.
#[derive(Clone, Copy, Debug)]
pub struct Press {
    pub x: i32,
    pub y: i32,
    pub button: crate::event::Button,
    pub widget: crate::chrome::Widget,
    pub node: Option<NodeId>,
    pub at: Instant,
}

/// Where a resolved URL should go.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Target {
    Current,
    NewTab,
    /// A new tab that does not take the focus (ctrl+click).
    Background,
    /// Same tab, no history entry (`location.replace`, reload).
    Replace,
}

/// The browser window: its tabs, its chrome, its modes, its engine.
pub struct App {
    pub engine: Engine,
    pub tabs: Vec<Tab>,
    pub active: usize,
    pub mode: Mode,
    /// Which chrome widget shows a focus ring (`Caret::Page` = the page has it).
    pub caret: Caret,
    pub address: Editor,
    pub find_input: Editor,
    pub cmdline: Editor,
    pub hints: HintState,
    pub menu: Option<Menu>,
    pub select: Option<SelectPopup>,
    pub devtools: Devtools,
    pub theme: Theme,
    pub config: UiConfig,
    pub clipboard: Clipboard,
    pub bookmarks: Bookmarks,
    /// Everything this session visited (omnibox suggestions).
    pub visited: Vec<String>,
    /// Closed tabs, for `Ctrl+Shift+T`.
    pub closed: Vec<String>,
    pub status: String,
    pub status_kind: LineKind,
    /// Window size in UI px and the cell grid it implies.
    pub width: u32,
    pub height: u32,
    pub cols: usize,
    pub rows: usize,
    pub press: Option<Press>,
    /// Double/triple click detection.
    pub click_count: i32,
    pub last_click: Option<Instant>,
    /// Something changed; a frame is owed.
    pub dirty: bool,
    pub quit: bool,
    /// Guards against a page that navigates forever.
    nav_depth: usize,
}

impl App {
    /// A shell with the given tabs; `urls` empty opens [`UiConfig::home`].
    pub fn new(config: UiConfig, theme: Theme, width: u32, height: u32, urls: &[String]) -> App {
        let mut app = App {
            engine: Engine::new(EngineConfig::default()),
            tabs: Vec::new(),
            active: 0,
            mode: Mode::Normal,
            caret: Caret::Page,
            address: Editor::new(),
            find_input: Editor::new(),
            cmdline: Editor::new(),
            hints: HintState::default(),
            menu: None,
            select: None,
            devtools: Devtools::default(),
            theme,
            config: config.clone(),
            clipboard: Clipboard::new(),
            bookmarks: Bookmarks::open(config.bookmark_file.clone()),
            visited: Vec::new(),
            closed: Vec::new(),
            status: String::new(),
            status_kind: LineKind::Dim,
            width: width.max(240),
            height: height.max(160),
            cols: 20,
            rows: 8,
            press: None,
            click_count: 0,
            last_click: None,
            dirty: true,
            quit: false,
            nav_depth: 0,
        };
        app.apply_engine_config();
        app.recompute_grid();
        let list: Vec<String> = if urls.is_empty() {
            vec![config.home.clone()]
        } else {
            urls.to_vec()
        };
        for (i, url) in list.iter().enumerate() {
            let target = if i == 0 {
                Target::Current
            } else {
                Target::NewTab
            };
            app.open(url, target);
        }
        app.dirty = true;
        app
    }

    /// Push the shell's settings into a fresh engine (viewport is per tab).
    pub fn apply_engine_config(&mut self) {
        use tb_core::Capabilities;
        let cfg = EngineConfig {
            scale: self.config.scale,
            timeout_ms: self.config.timeout_ms,
            color_scheme: self.theme.kind.color_scheme(),
            caps: Capabilities {
                run_scripts: self.config.run_scripts,
                apply_css: self.config.apply_css,
                layout: true,
                fetch_subresources: self.config.fetch,
                ..Capabilities::default()
            },
            ..EngineConfig::default()
        };
        self.engine = Engine::new(cfg);
    }

    // ------------------------------------------------------------------ grid --

    /// Window size → cell grid → page viewport.
    pub fn recompute_grid(&mut self) {
        let cols = (self.width / CELL_W as u32).clamp(20, 4000) as usize;
        let rows = (self.height / CELL_H as u32).clamp(6, 4000) as usize;
        if cols != self.cols || rows != self.rows {
            self.cols = cols;
            self.rows = rows;
            self.dirty = true;
        }
        self.ensure_viewport();
    }

    /// The chrome's geometry for this frame.
    pub fn chrome(&self) -> Chrome {
        let labels: Vec<String> = if self.config.show_bookmarks {
            self.bookmarks
                .items
                .iter()
                .take(24)
                .map(|b| {
                    let t = if b.title.is_empty() {
                        b.url.clone()
                    } else {
                        b.title.clone()
                    };
                    tb_core::text::truncate(&t, 18)
                })
                .collect()
        } else {
            Vec::new()
        };
        let mut chrome = Chrome {
            cols: self.cols,
            rows: self.rows,
            show_chrome: self.config.show_chrome,
            show_bookmarks: self.config.show_bookmarks && !labels.is_empty(),
            show_status: self.config.show_chrome,
            tab_count: self.tabs.len(),
            active_tab: self.active,
            find_open: self.mode == Mode::Find,
            command_open: self.mode == Mode::Command,
            devtools_rows: if self.devtools.open {
                self.devtools.rows.min(self.rows.saturating_sub(4))
            } else {
                0
            },
            // filled in just below, from the geometry this very struct
            // describes — asking `content_px()` would build a second Chrome and
            // never come back
            h_scroll: false,
            bookmark_labels: labels,
        };
        let (_cx, _cy, cw, _ch) = chrome.content_cells();
        chrome.h_scroll = self.page_width_px() > (cw as i32 * CELL_W) as f32;
        chrome
    }

    /// The content area in UI px: `(x, y, w, h)`.
    pub fn content_px(&self) -> (i32, i32, i32, i32) {
        let (cx, cy, cw, ch) = self.chrome().content_cells();
        (
            cx as i32 * CELL_W,
            cy as i32 * CELL_H,
            cw as i32 * CELL_W,
            ch as i32 * CELL_H,
        )
    }

    /// Laid-out page width in UI px (a zoomed page can be wider than the view).
    pub fn page_width_px(&self) -> f32 {
        if self.tabs.is_empty() {
            return 0.0;
        }
        self.tab().page.config.viewport.width * self.config.scale
    }

    /// Make the active page's viewport match the content area (relayout if not).
    pub fn ensure_viewport(&mut self) {
        if self.tabs.is_empty() {
            return;
        }
        let (_, _, w, h) = self.content_px();
        if w <= 0 || h <= 0 {
            return;
        }
        let scale = self.config.scale.max(0.1);
        let css_w = (w as f32 / scale).max(80.0);
        let css_h = (h as f32 / scale).max(80.0);
        let i = self.active;
        let (cur_w, cur_h) = {
            let p = &self.tabs[i].page;
            (p.config.viewport.width, p.config.viewport.height)
        };
        if (cur_w - css_w).abs() > 0.5 || (cur_h - css_h).abs() > 0.5 {
            self.tabs[i].page.set_viewport(css_w, css_h);
            // the geometry the selection and caret referred to is gone
            self.tabs[i].selection.clear();
            let (sx, sy) = (self.tabs[i].scroll_x, self.tabs[i].scroll_y);
            self.tabs[i].page.set_scroll(sx, sy);
        }
    }

    // ------------------------------------------------------------------ tabs --

    pub fn tab(&self) -> &Tab {
        let i = self.active.min(self.tabs.len().saturating_sub(1));
        &self.tabs[i]
    }

    pub fn tab_mut(&mut self) -> &mut Tab {
        let i = self.active.min(self.tabs.len().saturating_sub(1));
        &mut self.tabs[i]
    }

    pub fn page(&self) -> &Page {
        &self.tab().page
    }

    /// The page's text with geometry (selection, find, cell rendering).
    pub fn page_text(&self) -> PageText {
        PageText::from_page(self.page())
    }

    pub fn url(&self) -> String {
        if self.tabs.is_empty() {
            return String::new();
        }
        self.tab().page.url.to_string()
    }

    pub fn title(&self) -> String {
        if self.tabs.is_empty() {
            return "tiny".to_string();
        }
        format!(
            "{} — tiny",
            tb_core::text::truncate(&self.tab().title(), 60)
        )
    }

    pub fn use_tab(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        self.active = index;
        self.mode = Mode::Normal;
        self.caret = Caret::Page;
        self.hints.reset();
        self.ensure_viewport();
        self.sync_address();
        self.dirty = true;
    }

    pub fn next_tab(&mut self) {
        if !self.tabs.is_empty() {
            let i = (self.active + 1) % self.tabs.len();
            self.use_tab(i);
        }
    }

    pub fn prev_tab(&mut self) {
        if !self.tabs.is_empty() {
            let i = (self.active + self.tabs.len() - 1) % self.tabs.len();
            self.use_tab(i);
        }
    }

    pub fn close_tab(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        let url = self.tabs[index].page.url.to_string();
        self.tabs.remove(index);
        self.closed.push(url);
        if self.closed.len() > 50 {
            self.closed.remove(0);
        }
        if self.tabs.is_empty() {
            // an empty window is a browser, not an error: show the home page
            let home = self.config.home.clone();
            self.active = 0;
            self.open(&home, Target::NewTab);
            return;
        }
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len() - 1;
        }
        self.ensure_viewport();
        self.sync_address();
        self.dirty = true;
    }

    /// `Ctrl+T` — a new tab, optionally at `url`.
    pub fn new_tab(&mut self, url: Option<&str>) {
        let target = url
            .map(|s| s.to_string())
            .unwrap_or_else(|| self.config.home.clone());
        self.open(&target, Target::NewTab);
    }

    pub fn restore_tab(&mut self) {
        if let Some(url) = self.closed.pop() {
            self.open(&url, Target::NewTab);
        }
    }

    // ----------------------------------------------------------- navigation --

    /// Load `raw` (URL, path, or search words) in `target`.
    pub fn open(&mut self, raw: &str, target: Target) {
        // Path-shaped input ("about/team.html", "../x", "/y") is relative to the
        // page we are on, the same way a browser resolves a link; anything else
        // goes through `resolve_input`.
        let trimmed = raw.trim();
        // something with a scheme of its own is already a URL
        let absolute = trimmed.contains("://")
            || trimmed.starts_with("file:")
            || trimmed.starts_with("about:")
            || trimmed.starts_with("data:");
        let path_shaped = !absolute
            && (trimmed.starts_with('/')
                || trimmed.starts_with("./")
                || trimmed.starts_with("../")
                || trimmed.contains('/'));
        // Reading a folder of local pages, "second.html" means the file next to
        // this one — searching for it would be the surprise.
        let here = self.url();
        let local_sibling = !absolute
            && here.starts_with("file:")
            && DOC_SUFFIXES.iter().any(|s| trimmed.ends_with(s));
        if (path_shaped || local_sibling)
            && !trimmed.contains(char::is_whitespace)
            && !std::path::Path::new(trimmed).exists()
        {
            if let Ok(base) = Url::parse(&here) {
                if let Ok(joined) = Url::join(&base, trimmed) {
                    // only when joining actually changed something, or this
                    // would be a call to `open` from inside `open` forever
                    if joined.to_string() != here {
                        return self.open(&joined.to_string(), target);
                    }
                }
            }
        }
        let url = resolve_input(raw, &self.config);
        if url.is_empty() {
            return;
        }
        if target == Target::Current && !self.tabs.is_empty() {
            if let Some(frag) = fragment_only_change(&self.url(), &url) {
                self.scroll_to_fragment(&frag);
                return;
            }
        }
        if self.nav_depth > 3 {
            self.status(
                "stopped: this page keeps asking to navigate",
                LineKind::Warn,
            );
            return;
        }
        self.nav_depth += 1;
        let loaded = self.engine.load(&url);
        let page = match loaded {
            Ok(p) => p,
            Err(e) => {
                let html = error_page(&url, &e.to_string());
                match self
                    .engine
                    .load_html(&format!("about:error?u={url}"), &html)
                {
                    Ok(p) => p,
                    Err(_) => {
                        self.nav_depth -= 1;
                        self.status(&format!("cannot load {url}: {e}"), LineKind::Error);
                        return;
                    }
                }
            }
        };
        self.attach(page, target, target != Target::Replace);
        self.nav_depth -= 1;
        // scripts may have asked for a navigation during the load
        let pending: Vec<NavRequest> = std::mem::take(&mut self.tab_mut().page.nav_requests);
        for nr in pending {
            self.handle_nav(nr);
        }
        self.dirty = true;
    }

    /// Put a loaded page into the tab list, measured with *our* viewport.
    fn attach(&mut self, mut page: Page, target: Target, push_history: bool) {
        let url = page.url.to_string();
        if !self.visited.contains(&url) {
            self.visited.push(url.clone());
            if self.visited.len() > 500 {
                self.visited.remove(0);
            }
        }
        let scale = self.config.scale;
        let (_, _, w, h) = self.content_px();
        if w > 0 && h > 0 {
            page.set_viewport((w as f32 / scale).max(80.0), (h as f32 / scale).max(80.0));
        }
        if matches!(target, Target::NewTab | Target::Background) || self.tabs.is_empty() {
            let history = if push_history {
                vec![url.clone()]
            } else {
                Vec::new()
            };
            let tab = Tab::with_page(page, history);
            let at = (self.active + 1).min(self.tabs.len());
            if self.tabs.is_empty() {
                self.tabs.push(tab);
                self.active = 0;
            } else {
                self.tabs.insert(at, tab);
                if target != Target::Background {
                    self.active = at;
                }
            }
        } else {
            let i = self.active;
            self.tabs[i].replace_page(page, push_history);
        }
        self.mode = Mode::Normal;
        self.hints.reset();
        self.menu = None;
        self.select = None;
        self.caret = Caret::Page;
        self.sync_address();
        let page = self.page();
        let problems = page.problems().all().len();
        if problems > 0 {
            self.status(
                &format!("{} problems (F12 → Problems)", problems),
                LineKind::Dim,
            );
        }
    }

    /// What one of the page's navigation requests means.
    pub fn handle_nav(&mut self, nr: NavRequest) {
        match nr {
            NavRequest::Load(u) => {
                if u.starts_with("javascript:") || u.starts_with("blob:") {
                    self.status(
                        &format!("not supported: {}", tb_core::text::truncate(&u, 40)),
                        LineKind::Warn,
                    );
                    return;
                }
                self.open(&u, Target::Current);
            }
            NavRequest::Replace(u) => self.open(&u, Target::Replace),
            NavRequest::Reload => self.reload(false),
            NavRequest::Back(n) => {
                for _ in 0..n.clamp(1, 20) {
                    self.back();
                }
            }
            NavRequest::Scroll(_, y) => {
                let x = self.tab().scroll_x;
                self.scroll_to(x, y as f32);
            }
        }
    }

    pub fn reload(&mut self, hard: bool) {
        let url = self.url();
        if url.is_empty() {
            return;
        }
        if hard {
            if let Some(c) = self.engine.client() {
                c.cache().clear();
            }
            self.status("reloaded (cache cleared)", LineKind::Dim);
        } else {
            self.status("reloaded", LineKind::Dim);
        }
        self.open(&url, Target::Replace);
    }

    pub fn back(&mut self) {
        let url = self.tab().back_url().map(|s| s.to_string());
        match url {
            Some(u) => {
                self.open(&u, Target::Replace);
                let i = self.active;
                if self.tabs[i].hist_index > 0 {
                    self.tabs[i].hist_index -= 1;
                }
            }
            None => self.status("no page before this one", LineKind::Dim),
        }
    }

    pub fn forward(&mut self) {
        let url = self.tab().forward_url().map(|s| s.to_string());
        match url {
            Some(u) => {
                self.open(&u, Target::Replace);
                let i = self.active;
                self.tabs[i].hist_index =
                    (self.tabs[i].hist_index + 1).min(self.tabs[i].history.len().saturating_sub(1));
            }
            None => self.status("no page after this one", LineKind::Dim),
        }
    }

    /// `#id` / `<a name>`: scroll, do not reload.
    pub fn scroll_to_fragment(&mut self, frag: &str) {
        let node = find_anchor(self.page(), frag);
        match node {
            Some(n) => {
                if let Some(r) = self.page().rect_of(n) {
                    let x = self.tab().scroll_x;
                    self.scroll_to(x, r.top() - 8.0);
                }
                self.status(
                    &format!("#{}", tb_core::text::truncate(frag, 40)),
                    LineKind::Dim,
                );
            }
            None => self.status(&format!("no #{frag} on this page"), LineKind::Warn),
        }
    }

    // ----------------------------------------------------------- scrolling ---

    /// Absolute document offset (CSS px), clamped by the page itself.
    pub fn scroll_to(&mut self, x: f32, y: f32) {
        if self.tabs.is_empty() {
            return;
        }
        self.tab_mut().page.set_scroll(x.max(0.0), y.max(0.0));
        let (sx, sy) = self.tab().page.scroll();
        let t = self.tab_mut();
        t.scroll_x = sx;
        t.scroll_y = sy;
        self.dirty = true;
    }

    pub fn scroll_by(&mut self, dx: f32, dy: f32) {
        let (x, y) = self.tab().scroll_pos();
        self.scroll_to(x + dx, y + dy);
    }

    pub fn scroll_pages(&mut self, pages: i32) {
        let (_, _, _, h) = self.content_px();
        let step = h as f32 * 0.9 * pages as f32 / self.config.scale.max(0.1);
        self.scroll_by(0.0, step);
    }

    pub fn scroll_top(&mut self) {
        let x = self.tab().scroll_x;
        self.scroll_to(x, 0.0);
    }

    pub fn scroll_bottom(&mut self) {
        let x = self.tab().scroll_x;
        let max = self.page().max_scroll();
        self.scroll_to(x, max);
    }

    /// Scroll the minimum to bring a document rectangle into view.
    pub fn reveal(&mut self, r: Rect) {
        let view = self.view_rect();
        let mut dy = 0.0;
        if r.top() < view.top() {
            dy = r.top() - view.top() - 16.0;
        } else if r.bottom() > view.bottom() {
            dy = r.bottom() - view.bottom() + 16.0;
        }
        if dy != 0.0 {
            let (x, y) = self.tab().scroll_pos();
            self.scroll_to(x, y + dy);
        }
    }

    // ------------------------------------------------------------------ zoom --

    pub fn zoom(&mut self, dir: i8) {
        const STEPS: [f32; 6] = [0.5, 0.75, 1.0, 1.5, 2.0, 3.0];
        let cur = self.config.scale;
        let idx = STEPS
            .iter()
            .enumerate()
            .min_by_key(|(_, s)| ((**s - cur).abs() * 100.0) as i32)
            .map(|(i, _)| i)
            .unwrap_or(2);
        let next = match dir {
            d if d > 0 => (idx + 1).min(STEPS.len() - 1),
            d if d < 0 => idx.saturating_sub(1),
            _ => 2,
        };
        let scale = STEPS[next];
        self.config.scale = scale;
        for t in self.tabs.iter_mut() {
            t.page.config.scale = scale;
            t.page.invalidate_render();
        }
        self.ensure_viewport();
        self.status(&format!("zoom {}%", (scale * 100.0) as i32), LineKind::Dim);
        self.dirty = true;
    }

    // ------------------------------------------------------------ theme & co --

    pub fn toggle_theme(&mut self) {
        self.theme = Theme::of(self.theme.kind.flipped());
        let scheme = self.theme.kind.color_scheme();
        self.engine.config.color_scheme = scheme;
        for t in self.tabs.iter_mut() {
            t.page.config.color_scheme = scheme;
            t.page.restyle_and_layout();
        }
        self.status(
            &format!("{} theme", self.theme.kind.as_str()),
            LineKind::Dim,
        );
        self.dirty = true;
    }

    pub fn toggle_bookmark_bar(&mut self) {
        self.config.show_bookmarks = !self.config.show_bookmarks;
        self.ensure_viewport();
        self.dirty = true;
    }

    pub fn bookmark_this(&mut self) {
        let url = self.url();
        let title = self.tab().title();
        if self.bookmarks.add(&title, &url) {
            self.status(
                &format!("bookmarked {}", tb_core::text::truncate(&title, 40)),
                LineKind::Ok,
            );
        } else {
            self.status("already bookmarked", LineKind::Dim);
        }
    }

    pub fn panel(&self) -> Panel {
        self.devtools.panel
    }

    // ------------------------------------------------------------------ misc --

    /// A status line with a colour.
    pub fn status(&mut self, msg: &str, kind: LineKind) {
        self.status = msg.to_string();
        self.status_kind = kind;
        self.dirty = true;
    }

    /// Keep the address bar in step with the page (unless the user is typing).
    pub fn sync_address(&mut self) {
        if self.mode == Mode::Address {
            return;
        }
        let url = self.url();
        if self.address.value() != url {
            self.address.set(&url);
        }
    }

    /// What the status line says right now.
    pub fn status_line(&self) -> (String, LineKind) {
        let t = self.tab();
        match self.mode {
            Mode::Address => {
                return (
                    "address — Enter loads, Esc cancels, ↑/↓ history, Tab completes".to_string(),
                    LineKind::Accent,
                )
            }
            Mode::Find => {
                let n = t.find.count();
                let i = if n == 0 { 0 } else { t.find.index + 1 };
                return (
                    format!(
                        "find “{}” {i}/{n} — Enter next, Shift+Enter previous, Esc closes",
                        self.find_input.value()
                    ),
                    if n == 0 && !self.find_input.is_empty() {
                        LineKind::Warn
                    } else {
                        LineKind::Accent
                    },
                );
            }
            Mode::Command => return (": command — :help lists them".to_string(), LineKind::Accent),
            Mode::Console => {
                return (
                    "console prompt — Enter evaluates in the page".to_string(),
                    LineKind::Accent,
                )
            }
            Mode::Hints => {
                return (
                    format!(
                        "hint {} — type a label, {} possible",
                        self.hints.typed,
                        self.hints.candidates().len()
                    ),
                    LineKind::Accent,
                )
            }
            Mode::Pick => {
                return (
                    "pick — click the element to inspect, Esc cancels".to_string(),
                    LineKind::Accent,
                )
            }
            Mode::Normal | Mode::Help | Mode::Menu | Mode::Select => (),
        }
        if !self.status.is_empty() {
            return (self.status.clone(), self.status_kind);
        }
        if let Some(link) = &t.hover_link {
            return (link.clone(), LineKind::Accent);
        }
        let page = self.page();
        let max = page.max_scroll().max(1.0);
        let pct = ((t.scroll_y / max) * 100.0).clamp(0.0, 100.0) as i32;
        (
            format!(
                "{} · {}% of {} px · zoom {}% · {} problems",
                tb_core::text::truncate(&self.url(), 46),
                pct,
                page.content_height() as i32,
                (self.config.scale * 100.0) as i32,
                page.problems().all().len()
            ),
            LineKind::Dim,
        )
    }

    /// Run the page's timers and promises for `ms` of virtual time.
    pub fn pump(&mut self, ms: f64) {
        if self.tabs.is_empty() || !self.tab().page.has_js() {
            return;
        }
        let i = self.active;
        self.tabs[i].page.pump(ms);
        let pending: Vec<NavRequest> = std::mem::take(&mut self.tabs[i].page.nav_requests);
        for nr in pending {
            self.handle_nav(nr);
        }
    }

    /// The document rectangle the content area shows.
    pub fn view_rect(&self) -> Rect {
        let (_, _, w, h) = self.content_px();
        let scale = self.config.scale.max(0.1);
        Rect::new(
            self.tab().scroll_x,
            self.tab().scroll_y,
            (w as f32 / scale).max(1.0),
            (h as f32 / scale).max(1.0),
        )
    }

    /// A UI-px point inside the content area → viewport point (CSS px,
    /// unscrolled). `None` when the point is on the chrome.
    pub fn viewport_point(&self, px_x: i32, px_y: i32) -> Option<(f32, f32)> {
        let (cx, cy, cw, ch) = self.content_px();
        if px_x < cx || px_y < cy || px_x >= cx + cw || px_y >= cy + ch {
            return None;
        }
        let scale = self.config.scale.max(0.1);
        Some(((px_x - cx) as f32 / scale, (px_y - cy) as f32 / scale))
    }

    /// The element under a UI-px point (`None` = not over the page).
    pub fn element_at_px(&self, px_x: i32, px_y: i32) -> Option<NodeId> {
        let (vx, vy) = self.viewport_point(px_x, px_y)?;
        self.page().element_at_viewport(vx, vy)
    }

    /// The options of a `<select>`.
    pub fn options_of(node: NodeId, page: &Page) -> Vec<String> {
        let mut out = Vec::new();
        for n in page.dom.descendants(node) {
            if page.dom.tag(n) == Some("option") {
                let t = page.dom.text_content(n).trim().to_string();
                out.push(if t.is_empty() {
                    page.dom.attr(n, "value").unwrap_or_default()
                } else {
                    t
                });
            }
        }
        out
    }

    /// The paint options for the active tab.
    pub fn paint_options(&self) -> PaintOptions {
        PaintOptions {
            scale: self.config.scale,
            scroll_x: self.tab().scroll_x,
            scroll_y: self.tab().scroll_y,
            background: self.theme.bg,
            show_boxes: false,
            draw_text: true,
        }
    }
}

/// Turn what the user typed into a URL: a URL stays one, a path becomes
/// `file:`, anything else goes to the search engine.
pub fn resolve_input(raw: &str, config: &UiConfig) -> String {
    let s = raw.trim();
    if s.is_empty() {
        return String::new();
    }
    let lower = s.to_ascii_lowercase();
    for scheme in [
        "http://",
        "https://",
        "file:",
        "about:",
        "data:",
        "view-source:",
        "mailto:",
    ] {
        if lower.starts_with(scheme) {
            return s.to_string();
        }
    }
    // a filesystem path that exists (absolute, because `file:` needs it)
    if s.contains('/') && std::path::Path::new(s).exists() {
        let path = std::path::Path::new(s);
        let abs = if path.is_absolute() {
            path.to_path_buf()
        } else if let Ok(cwd) = std::env::current_dir() {
            cwd.join(path)
        } else {
            path.to_path_buf()
        };
        return format!("file://{}", abs.display());
    }
    // `example.com`, `example.com/path`, `localhost:8080`, `127.0.0.1:8000`
    let looks_like_host =
        !s.contains(' ') && (s.contains('/') || s.contains('.') || s.contains(':'));
    if looks_like_host {
        if s.starts_with("localhost") || s.starts_with("127.") || s.starts_with('[') {
            return format!("http://{s}");
        }
        return format!("https://{s}");
    }
    let q = tb_net::percent_encode(s);
    if config.search_url.contains("{}") {
        config.search_url.replace("{}", &q)
    } else {
        format!("{}{q}", config.search_url)
    }
}

/// `Some(fragment)` when `to` is the same document as `from` but another
/// fragment — that is a scroll, not a reload.
pub fn fragment_only_change(from: &str, to: &str) -> Option<String> {
    let a = from.split('#').next().unwrap_or(from);
    let b = to.split('#').next().unwrap_or(to);
    if a != b {
        return None;
    }
    to.split_once('#').map(|(_, f)| f.to_string())
}

/// A bare "second.html" is a file, not a search phrase.
const DOC_SUFFIXES: [&str; 12] = [
    ".html", ".htm", ".xhtml", ".xml", ".txt", ".md", ".json", ".svg", ".css", ".js", ".pdf",
    ".png",
];

/// `#id` or `<a name="id">`.
fn find_anchor(page: &Page, frag: &str) -> Option<NodeId> {
    if let Some(n) = page.dom.get_element_by_id(frag) {
        return Some(n);
    }
    page.dom
        .elements()
        .into_iter()
        .find(|n| page.dom.attr(*n, "name").as_deref() == Some(frag))
}

/// The page we show when a load failed — a browser does not print to stderr.
pub fn error_page(url: &str, why: &str) -> String {
    format!(
        "<!doctype html><html><head><title>Cannot open page</title>\
         <style>body{{margin:40px;color:#202020}}\
         .url{{color:#0b4da8}}pre{{color:#b3261e}}</style></head>\
         <body><h1>This page could not be opened</h1>\
         <p><span class=url>{url}</span></p><pre>{why}</pre>\
         <p>Check the address, the network, and whether the site is up.</p></body></html>"
    )
}
