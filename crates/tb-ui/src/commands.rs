//! `Command` → what happens.
//!
//! Every action the shell can take, in one `match`. Keeping it separate from the
//! keymap (which only *names* commands) and from `input` (which only *produces*
//! them) pays for itself twice: tests drive the browser by commanding it, and a
//! new shortcut means one line in the table plus at most one arm here.

use crate::app::{App, Target};
use crate::devtools::{LineKind, Panel};
use crate::hints::HintStep;
use crate::keymap::Command;
use crate::mode::{Caret, Mode};
use crate::selection::Selection;
use crate::tab::Tab;

impl App {
    /// Run one command. Every user action ends here.
    pub fn run(&mut self, cmd: Command) {
        match cmd {
            Command::Nop => (),
            Command::GoAddress => {
                let text = self.address.value();
                if text.trim().is_empty() {
                    self.mode = Mode::Normal;
                    self.caret = Caret::Page;
                    self.sync_address();
                    return;
                }
                self.address.remember(&text);
                self.mode = Mode::Normal;
                self.caret = Caret::Page;
                self.open(&text, Target::Current);
            }
            Command::Go(url) => {
                self.address.remember(&url);
                self.open(&url, Target::Current);
            }
            Command::Reload => self.reload(false),
            Command::ReloadHard => self.reload(true),
            Command::Back => self.back(),
            Command::Forward => self.forward(),
            Command::Home => {
                let home = self.config.home.clone();
                self.open(&home, Target::Current);
            }
            Command::ViewSource => self.show_source(),
            Command::NewTab(url) => {
                let u = url.unwrap_or_else(|| self.config.home.clone());
                self.open(&u, Target::NewTab);
            }
            Command::CloseTab => {
                let i = self.active;
                self.close_tab(i);
            }
            Command::RestoreTab => self.restore_tab(),
            Command::UseTab(i) => {
                if i == usize::MAX {
                    let last = self.tabs.len().saturating_sub(1);
                    self.use_tab(last);
                } else {
                    self.use_tab(i);
                }
            }
            Command::NextTab => self.next_tab(),
            Command::PrevTab => self.prev_tab(),
            Command::AddressBar => self.open_address(),
            Command::FindBar => self.open_find(),
            Command::FindNext => self.find_step(1),
            Command::FindPrev => self.find_step(-1),
            Command::CloseBar => self.close_overlays(),
            Command::CommandBar => {
                self.mode = Mode::Command;
                self.cmdline.clear();
                self.dirty = true;
            }
            Command::Hints(new_tab) => self.enter_hints(new_tab),
            Command::FollowHint(label, new_tab) => self.follow_label(&label, new_tab),
            Command::DevtoolsToggle => {
                self.devtools.open = !self.devtools.open;
                self.ensure_viewport();
                self.dirty = true;
            }
            Command::DevtoolsPanel(i) => {
                self.devtools.open = true;
                self.devtools.panel = Panel::nth(i);
                self.ensure_viewport();
                if self.devtools.panel == Panel::Console {
                    // the console is only useful if you can type in it
                    self.mode = Mode::Console;
                    self.devtools.prompt.end(false);
                }
                self.dirty = true;
            }
            Command::PickElement => {
                self.devtools.open = true;
                self.devtools.panel = Panel::Elements;
                self.mode = Mode::Pick;
                self.ensure_viewport();
                self.status("click the element you want to inspect", LineKind::Accent);
            }
            Command::ScrollPixels(dx, dy) => self.scroll_by(dx, dy),
            Command::ScrollPages(n) => self.scroll_pages(n),
            Command::ScrollTop => self.scroll_top(),
            Command::ScrollBottom => self.scroll_bottom(),
            Command::Zoom(dir) => self.zoom(dir),
            Command::ToggleTheme => self.toggle_theme(),
            Command::ToggleBookmarks => self.toggle_bookmark_bar(),
            Command::BookmarkPage => self.bookmark_this(),
            Command::Copy => self.copy_selection(),
            Command::Paste => self.paste(),
            Command::SelectAll => {
                let pt = self.page_text();
                let sel = Selection::all(&pt);
                self.tab_mut().selection = sel;
                self.status("all text selected — Ctrl+C copies it", LineKind::Ok);
            }
            Command::Help => {
                self.mode = Mode::Help;
                self.dirty = true;
            }
            Command::Quit => self.quit = true,
            Command::FocusPage => {
                self.mode = Mode::Normal;
                self.caret = Caret::Page;
                self.dirty = true;
            }
            Command::FocusNext => self.move_focus(1),
            Command::FocusPrev => self.move_focus(-1),
            Command::ActivateFocused => self.activate_focused(),
            Command::PageChar(c) => self.page_type_char(c),
        }
    }

    // ------------------------------------------------------------- overlays --

    /// Focus the address bar (and select it all, like `Ctrl+L` does).
    pub fn open_address(&mut self) {
        self.mode = Mode::Address;
        self.caret = Caret::Widget(crate::chrome::Widget::Address);
        let url = self.url();
        if self.address.value() != url {
            self.address.set(&url);
        }
        self.address.select_all();
        self.menu = None;
        self.hints.reset();
        self.dirty = true;
    }

    /// Focus the find bar.
    pub fn open_find(&mut self) {
        self.mode = Mode::Find;
        self.find_input.end(false);
        self.menu = None;
        self.dirty = true;
    }

    /// `Esc`: close whatever overlay is open, otherwise clear the selection.
    /// Leave the find bar open in the memory of the matches: `Enter` accepts a
    /// search, `n` then continues it from the page.
    pub fn accept_find(&mut self) {
        if self.mode == Mode::Find && self.tab().find.count() > 0 {
            self.mode = Mode::Normal;
            self.caret = Caret::Page;
        }
    }

    pub fn close_overlays(&mut self) {
        match self.mode {
            Mode::Address => {
                self.mode = Mode::Normal;
                self.caret = Caret::Page;
                let url = self.url();
                self.address.set(&url);
            }
            Mode::Find => {
                self.mode = Mode::Normal;
                self.find_input.clear();
                let pt = self.page_text();
                self.tab_mut().find.search(&pt, "");
            }
            Mode::Command | Mode::Console => {
                self.mode = Mode::Normal;
                self.caret = Caret::Page;
            }
            Mode::Hints => self.hints.reset(),
            Mode::Menu => self.menu = None,
            Mode::Select => self.select = None,
            Mode::Pick => self.mode = Mode::Normal,
            Mode::Help => self.mode = Mode::Normal,
            Mode::Normal => {
                let had = !self.tab().selection.is_empty();
                self.tab_mut().selection.clear();
                if !had {
                    self.status(String::new().as_str(), LineKind::Dim);
                }
            }
        }
        self.dirty = true;
    }

    /// `f` / `F`: label every visible link.
    pub fn enter_hints(&mut self, new_tab: bool) {
        let view = self.view_rect();
        self.hints = crate::hints::HintState::build(self.page(), view, new_tab);
        if self.hints.hints.is_empty() {
            self.hints.reset();
            self.status("no links on this screen", LineKind::Dim);
            self.mode = Mode::Normal;
        } else {
            self.mode = Mode::Hints;
            self.status(
                &format!("{} links on this screen", self.hints.hints.len()),
                LineKind::Accent,
            );
        }
        self.dirty = true;
    }

    /// Follow a completed hint label.
    pub fn follow_label(&mut self, label: &str, new_tab: bool) {
        let hint = self.hints.hints.iter().find(|h| h.label == label).cloned();
        self.hints.reset();
        self.mode = Mode::Normal;
        let Some(hint) = hint else {
            self.status(&format!("no hint “{label}”"), LineKind::Warn);
            return;
        };
        if let Some(href) = hint.href.clone() {
            if new_tab {
                self.open(&href, Target::NewTab);
            } else {
                self.open(&href, Target::Current);
            }
            return;
        }
        // not a link: focus it and fire a click, like pressing a button
        self.focus_control(Some(hint.node));
        self.click_node(hint.node, 0.0, 0.0, 1);
    }

    /// A hint key press (`f` mode): typing a label, or the click on a label.
    pub fn hint_key(&mut self, c: char) {
        match self.hints.press(c) {
            HintStep::More => self.dirty = true,
            HintStep::Open(h) => {
                let new_tab = self.hints.new_tab;
                self.hints.reset();
                self.mode = Mode::Normal;
                if let Some(href) = h.href.clone() {
                    self.open(
                        &href,
                        if new_tab {
                            Target::NewTab
                        } else {
                            Target::Current
                        },
                    );
                } else {
                    self.focus_control(Some(h.node));
                    self.click_node(h.node, 0.0, 0.0, 1);
                }
            }
            HintStep::Cancel => {
                self.hints.reset();
                self.mode = Mode::Normal;
                self.status("hint cancelled", LineKind::Dim);
                self.dirty = true;
            }
        }
    }

    // ----------------------------------------------------------- clipboard ---

    /// Copy the page selection (or the field selection when a field is focused).
    pub fn copy_selection(&mut self) {
        let text = if let Some(sel) = self.field_selection() {
            sel
        } else {
            let pt = self.page_text();
            self.tab().selection.text(&pt)
        };
        if text.is_empty() {
            self.status("nothing selected", LineKind::Dim);
            return;
        }
        let chars = text.chars().count();
        self.clipboard.set(&text);
        self.status(
            &format!("copied {chars} characters (Ctrl+V pastes)"),
            LineKind::Ok,
        );
    }

    /// Paste into the address bar, the find bar, the console or a page field.
    pub fn paste(&mut self) {
        let text = self.clipboard.get().to_string();
        if text.is_empty() {
            self.status("clipboard is empty", LineKind::Dim);
            return;
        }
        self.paste_text(&text);
    }

    /// The shared body of `Ctrl+V` and a bracketed paste.
    pub fn paste_text(&mut self, text: &str) {
        let one_line: String = text
            .chars()
            .filter(|c| !c.is_control() || *c == '\n')
            .collect();
        match self.mode {
            Mode::Address => self.address.insert_str(&one_line.replace('\n', " ")),
            Mode::Find => {
                self.find_input.insert_str(&one_line.replace('\n', " "));
                let needle = self.find_input.value();
                let pt = self.page_text();
                self.tab_mut().find.search(&pt, &needle);
                self.find_step(1);
            }
            Mode::Command => self.cmdline.insert_str(&one_line.replace('\n', " ")),
            Mode::Console => self
                .devtools
                .prompt
                .insert_str(&one_line.replace('\n', " ")),
            _ => {
                if self.tab().caret_is_text() {
                    for c in one_line.chars() {
                        self.page_type_char(c);
                    }
                } else {
                    self.status("paste needs a text field or the address bar", LineKind::Dim);
                    return;
                }
            }
        }
        self.dirty = true;
    }

    /// The selection of whichever chrome field has the caret.
    pub fn field_selection(&self) -> Option<String> {
        let ed = match self.mode {
            Mode::Address => &self.address,
            Mode::Find => &self.find_input,
            Mode::Command => &self.cmdline,
            Mode::Console => &self.devtools.prompt,
            _ => return None,
        };
        if ed.selection().is_some() {
            Some(ed.selected_text())
        } else {
            None
        }
    }

    // ----------------------------------------------------------- the console --

    /// Evaluate the inspector prompt in the page and keep the result.
    pub fn run_console_prompt(&mut self) {
        let src = self.devtools.prompt.value();
        self.devtools.prompt.clear();
        if src.trim().is_empty() {
            return;
        }
        self.devtools.log.push(crate::devtools::Line {
            text: format!("> {src}"),
            kind: LineKind::Accent,
        });
        if !self.tab().page.has_js() {
            self.devtools.log.push(crate::devtools::Line {
                text: "javascript is off".to_string(),
                kind: LineKind::Warn,
            });
            self.dirty = true;
            return;
        }
        match self.tab_mut().page.eval(&src) {
            Ok(v) => {
                let out = v.to_string();
                self.devtools.log.push(crate::devtools::Line {
                    text: if out.is_empty() {
                        "undefined".into()
                    } else {
                        out
                    },
                    kind: LineKind::Text,
                });
            }
            Err(e) => self.devtools.log.push(crate::devtools::Line {
                text: format!("! {e}"),
                kind: LineKind::Error,
            }),
        }
        // the script may have navigated
        let pending: Vec<tb_browser::NavRequest> =
            std::mem::take(&mut self.tab_mut().page.nav_requests);
        for nr in pending {
            self.handle_nav(nr);
        }
        self.dirty = true;
    }

    // ------------------------------------------------------------ command ---

    /// Run a `:` command line. Unknown commands say so instead of doing nothing.
    pub fn run_cmdline(&mut self) {
        let line = self.cmdline.value();
        self.cmdline.clear();
        self.mode = Mode::Normal;
        let (name, arg) = match line.trim().split_once(char::is_whitespace) {
            Some((n, a)) => (n, a.trim()),
            None => (line.trim(), ""),
        };
        let cmd = match name {
            "" => Command::Nop,
            "open" | "o" | "go" | "goto" => Command::Go(arg.to_string()),
            "tabnew" | "tn" | "tab" => Command::NewTab(if arg.is_empty() {
                None
            } else {
                Some(arg.to_string())
            }),
            "close" => Command::CloseTab,
            "reload" => Command::Reload,
            "back" => Command::Back,
            "forward" => Command::Forward,
            "home" => Command::Home,
            "find" => {
                self.find_input.set(arg);
                self.mode = Mode::Find;
                let pt = self.page_text();
                self.tab_mut().find.search(&pt, arg);
                self.find_step(1);
                Command::Nop
            }
            "bookmarks" | "bm" => {
                let list: Vec<String> = self
                    .bookmarks
                    .items
                    .iter()
                    .map(|b| format!("{} — {}", b.title, b.url))
                    .collect();
                self.status(&format!("{} bookmarks", list.len()), LineKind::Dim);
                self.devtools.open = true;
                self.devtools.panel = Panel::Info;
                self.dirty = true;
                Command::Nop
            }
            "theme" => match arg {
                "dark" => {
                    if self.theme.kind.as_str() != "dark" {
                        Command::ToggleTheme
                    } else {
                        Command::Nop
                    }
                }
                "light" => {
                    if self.theme.kind.as_str() != "light" {
                        Command::ToggleTheme
                    } else {
                        Command::Nop
                    }
                }
                _ => Command::ToggleTheme,
            },
            "zoom" => match arg.parse::<f32>() {
                Ok(pct) if pct > 0.0 => {
                    let steps = ((pct / 100.0) - self.config.scale) as i8;
                    Command::Zoom(if steps == 0 { 0 } else { steps.signum() })
                }
                _ => Command::Zoom(0),
            },
            "devtools" | "inspector" => Command::DevtoolsToggle,
            "console" => Command::DevtoolsPanel(1),
            "source" => Command::ViewSource,
            "help" => Command::Help,
            "quit" | "q" => Command::Quit,
            other => {
                self.status(&format!("unknown command :{other} (:help)"), LineKind::Warn);
                Command::Nop
            }
        };
        self.run(cmd);
    }

    // ------------------------------------------------------------- find-in-page

    /// Recompute and jump to the first match (typing in the find bar).
    pub fn update_find(&mut self) {
        let needle = self.find_input.value();
        let pt = self.page_text();
        self.tab_mut().find.search(&pt, &needle);
        if self.tab().find.count() > 0 {
            self.find_step(1);
        }
        self.dirty = true;
    }

    /// `n` / `N`: move to the next/previous match and reveal it.
    pub fn find_step(&mut self, dir: i32) {
        let pt = self.page_text();
        let rect = if dir >= 0 {
            self.tab_mut()
                .find
                .advance()
                .and_then(|_| self.tab().find.current_rect(&pt))
        } else {
            self.tab_mut()
                .find
                .rewind()
                .and_then(|_| self.tab().find.current_rect(&pt))
        };
        match rect {
            Some(r) => {
                self.reveal(r);
                let (i, n) = (self.tab().find.index + 1, self.tab().find.count());
                self.status(&format!("match {i} of {n}"), LineKind::Accent);
            }
            None => {
                let needle = self.find_input.value();
                if needle.is_empty() {
                    self.status("type what to find", LineKind::Dim);
                } else {
                    self.status(&format!("“{needle}” not found"), LineKind::Warn);
                }
            }
        }
        self.dirty = true;
    }
}

/// `view-source`: the bytes the server sent, escaped into a `<pre>`.
///
/// Local files and `data:` URLs are read directly; an http(s) URL would need the
/// loader to hand back the raw body instead of running the pipeline, so the
/// status line says what is missing instead of pretending.
fn source_text(url: &str) -> Option<String> {
    if let Some(path) = url.strip_prefix("file://") {
        return std::fs::read_to_string(path).ok();
    }
    if let Some(rest) = url.strip_prefix("data:") {
        let (_, body) = rest.split_once(',')?;
        return Some(tb_net::percent_decode(body));
    }
    None
}

impl App {
    /// `Ctrl+U`.
    pub fn show_source(&mut self) {
        let url = self.url();
        let Some(html) = source_text(&url) else {
            // fall back to the DOM we have, which is honest enough for a page
            // we already fetched and good enough to read
            let dom_dump = self.page().dump_dom();
            let title = format!("dom: {url}");
            if let Ok(page) = self.engine.load_html(
                &format!("about:source?u={url}"),
                &source_page(&title, &dom_dump),
            ) {
                let tab = Tab::new(page);
                let at = (self.active + 1).min(self.tabs.len());
                self.tabs.insert(at, tab);
                self.active = at;
                self.status(
                    "DOM tree (raw source is not available for this URL)",
                    LineKind::Dim,
                );
                self.dirty = true;
            }
            return;
        };
        let title = format!("source: {url}");
        match self.engine.load_html(
            &format!("about:source?u={url}"),
            &source_page(&title, &html),
        ) {
            Ok(page) => {
                let tab = Tab::new(page);
                let at = (self.active + 1).min(self.tabs.len());
                self.tabs.insert(at, tab);
                self.active = at;
                self.dirty = true;
            }
            Err(e) => self.status(&format!("cannot show source: {e}"), LineKind::Error),
        }
    }
}

/// The document `view-source` shows.
fn source_page(title: &str, text: &str) -> String {
    let escaped = text
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    format!(
        "<!doctype html><html><head><title>{title}</title><style>\
         body{{margin:0;padding:16px;background:#181a1e;color:#dcdfe4}}\
         pre{{white-space:pre-wrap;word-break:break-word}}</style></head>\
         <body><pre>{escaped}</pre></body></html>"
    )
}
