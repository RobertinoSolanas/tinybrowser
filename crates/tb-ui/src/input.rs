//! Turning raw events into commands and into page events.
//!
//! The order of work here is the order a browser resolves an input event:
//! an open overlay first (a menu eats the click), then our own chrome (the back
//! button is ours, not the page's), then the page — where a hit-tested node gets
//! the real `mousedown`/`mouseup`/`click` trio, so `preventDefault` and
//! `target="_blank"` behave before we ever think about navigating.
//!
//! Everything that changes state goes through [`crate::app::App`] methods or
//! [`crate::keymap::Command`]s, so a test can replay exactly what a user did —
//! `Event::press(x, y)` is the same path a real pointer takes.

use std::time::Instant;

use tb_dom::NodeId;

use crate::app::{App, Press, Target};
use crate::chrome::Widget;
use crate::devtools::LineKind;
use crate::editor::Editor;
use crate::event::{Button, Event, Key, Mods, NamedKey, PointerKind};
use crate::keymap::{lookup, Command};
use crate::mode::{Caret, Mode};
use crate::selection::{self, Selection};

/// Two clicks this close together and this close apart are a double click
/// (X11's default is 400 ms; the terminal cannot tell us, so we decide).
const CLICK_TIME: std::time::Duration = std::time::Duration::from_millis(400);
/// A pointer that moved less than this is a click, not a drag.
const CLICK_SLOP: i32 = 6;

impl App {
    /// One event. Everything the UI does starts here.
    pub fn on_event(&mut self, ev: Event) {
        match ev {
            Event::Quit => self.quit = true,
            Event::Resize { width, height } => {
                self.width = width.max(240);
                self.height = height.max(160);
                self.recompute_grid();
            }
            Event::Tick => self.pump(16.0),
            Event::Paste(text) => self.paste_text(&text),
            Event::Key { key, mods } => self.on_key(key, mods),
            Event::Pointer {
                kind,
                button,
                x,
                y,
                dx,
                dy,
                mods,
            } => self.on_pointer(kind, button, x, y, dx, dy, mods),
        }
        self.dirty = true;
    }

    // ------------------------------------------------------------------ keys --

    /// A key press, in the current mode.
    pub fn on_key(&mut self, key: Key, mods: Mods) {
        // overlays first: they eat everything they understand
        match self.mode {
            Mode::Help => {
                self.mode = Mode::Normal;
                return;
            }
            Mode::Menu => {
                self.menu_key(key, mods);
                return;
            }
            Mode::Select => {
                self.select_key(key, mods);
                return;
            }
            Mode::Hints => {
                match key {
                    Key::Char(c) if c.is_ascii_alphabetic() => self.hint_key(c),
                    Key::Named(NamedKey::Enter) => {
                        if let Some(h) = self.hints.enter() {
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
                    }
                    Key::Named(NamedKey::Escape) => {
                        self.hints.reset();
                        self.mode = Mode::Normal;
                    }
                    Key::Named(NamedKey::Backspace) => {
                        let mut t: Vec<char> = self.hints.typed.chars().collect();
                        t.pop();
                        self.hints.typed = t.into_iter().collect();
                        if self.hints.typed.is_empty() && self.hints.hints.is_empty() {
                            self.mode = Mode::Normal;
                        }
                    }
                    _ => (),
                }
                self.dirty = true;
                return;
            }
            Mode::Pick => {
                if key == Key::Named(NamedKey::Escape) {
                    self.mode = Mode::Normal;
                    self.dirty = true;
                }
                return;
            }
            Mode::Address | Mode::Find | Mode::Command | Mode::Console => {
                self.on_field_key(key, mods);
                return;
            }
            Mode::Normal => (),
        }

        // a text field in the page has the caret: keys are text, not shortcuts
        if self.tab().caret_is_text() {
            match key {
                Key::Char(c) if !mods.any() => {
                    self.page_type_char(c);
                    return;
                }
                Key::Named(n) => {
                    // a match guard would hide that `page_key_named` *does*
                    // something, so the nested `if` stays
                    #[allow(clippy::collapsible_match, clippy::collapsible_if)]
                    if self.page_key_named(n, mods) {
                        return;
                    }
                }
                _ => (),
            }
        }

        // With the inspector open, the number keys are its panel tabs — the
        // page has no use for a bare digit in Normal mode.
        if self.devtools.open {
            if let Key::Char(c @ '1'..='9') = key {
                if !mods.any() {
                    self.run(Command::DevtoolsPanel(c as usize - '1' as usize));
                    return;
                }
            }
        }
        let cmd = lookup(self.mode, key, mods);
        if cmd != Command::Nop {
            self.run(cmd);
        }
    }

    /// Editing keys for the address / find / command / console fields.
    fn on_field_key(&mut self, key: Key, mods: Mods) {
        let mode = self.mode;
        // Ctrl and friends inside a field: word motions, edit, clipboard
        if mods.ctrl || mods.meta {
            match key {
                Key::Named(NamedKey::Left) => {
                    self.field_mut().word_left(true);
                    return;
                }
                Key::Named(NamedKey::Right) => {
                    self.field_mut().word_right(true);
                    return;
                }
                Key::Char('a') => {
                    self.field_mut().select_all();
                    return;
                }
                Key::Char('w') if mode == Mode::Address => {
                    self.field_mut().delete_word_left();
                    return;
                }
                Key::Char('c') => {
                    self.copy_selection();
                    return;
                }
                Key::Char('v') => {
                    self.paste();
                    return;
                }
                Key::Char('l') => {
                    // Ctrl+L again: reselect everything
                    self.field_mut().select_all();
                    return;
                }
                _ => (),
            }
        }
        match key {
            Key::Named(n) => match n {
                NamedKey::Enter => match mode {
                    Mode::Address => self.run(Command::GoAddress),
                    // Enter accepts the search: the bar closes but the matches
                    // stay, so `n` and `N` keep working on the page
                    Mode::Find if mods.shift => {
                        self.find_step(-1);
                        self.accept_find();
                    }
                    Mode::Find => {
                        self.find_step(1);
                        self.accept_find();
                    }
                    Mode::Command => self.run_cmdline(),
                    Mode::Console => self.run_console_prompt(),
                    _ => (),
                },
                NamedKey::Escape => self.close_overlays(),
                // the terminal's paste key works inside our fields too
                NamedKey::Insert if mods.shift => self.run(Command::Paste),
                NamedKey::Left => self.field_mut().left(mods.shift),
                NamedKey::Right => self.field_mut().right(mods.shift),
                NamedKey::Home => self.field_mut().home(mods.shift),
                NamedKey::End => self.field_mut().end(mods.shift),
                NamedKey::Backspace => {
                    self.field_mut().backspace();
                    if mode == Mode::Find {
                        self.update_find();
                    }
                }
                NamedKey::Delete => {
                    self.field_mut().delete();
                    if mode == Mode::Find {
                        self.update_find();
                    }
                }
                NamedKey::Up => match mode {
                    Mode::Address => self.address.history_step(true),
                    Mode::Find => self.find_input.history_step(true),
                    Mode::Console => self.devtools.prompt.history_step(true),
                    _ => (),
                },
                NamedKey::Down => match mode {
                    Mode::Address => self.address.history_step(false),
                    Mode::Find => self.find_input.history_step(false),
                    Mode::Console => self.devtools.prompt.history_step(false),
                    _ => (),
                },
                NamedKey::Tab if mode == Mode::Address => {
                    let mut candidates: Vec<String> = self.visited.clone();
                    for b in &self.bookmarks.items {
                        candidates.push(b.url.clone());
                    }
                    if !self.address.complete(&candidates) {
                        self.status("nothing completes “{}”", LineKind::Dim);
                    }
                    let v = self.address.value();
                    self.address.end(false);
                    let _ = v;
                }
                NamedKey::Tab => {
                    // Tab leaves the field: the page gets the caret again
                    self.mode = Mode::Normal;
                    self.caret = Caret::Page;
                    self.move_focus(1);
                }
                _ => (),
            },
            Key::Char(c) if !c.is_control() => {
                // `insert` replaces the selection itself, if there is one
                self.field_mut().insert(c);
                if mode == Mode::Find {
                    self.update_find();
                }
            }
            _ => (),
        }
        self.dirty = true;
    }

    /// The editor that currently has the caret.
    fn field_mut(&mut self) -> &mut Editor {
        match self.mode {
            Mode::Address => &mut self.address,
            Mode::Find => &mut self.find_input,
            Mode::Command => &mut self.cmdline,
            Mode::Console => &mut self.devtools.prompt,
            _ => &mut self.cmdline,
        }
    }

    /// Typing one character into the focused page control.
    pub fn page_type_char(&mut self, c: char) {
        let Some(node) = self.tab().caret_node else {
            return;
        };
        let key_name = c.to_string();
        // `keydown` may cancel the character (a page that filters input does)
        let allowed = self.tab_mut().page.key_event(
            node,
            "keydown",
            &key_name,
            &Key::Char(c).js_code(),
            Mods::none().to_page(),
        );
        if !allowed {
            return;
        }
        let mut value: Vec<char> = self.tab().caret_value().chars().collect();
        let at = self.tab().caret_index.min(value.len());
        value.insert(at, c);
        let text: String = value.into_iter().collect();
        let t = self.tab_mut();
        t.page.type_into(node, &text);
        t.caret_index = at + 1;
        self.tab_mut().page.keyup(node, &key_name);
        self.dirty = true;
    }

    /// Editing keys inside a focused page field. `true` when the key was ours.
    fn page_key_named(&mut self, n: NamedKey, mods: Mods) -> bool {
        let Some(node) = self.tab().caret_node else {
            return false;
        };
        let multi = self.page().dom.tag(node) == Some("textarea");
        let value_len = self.tab().caret_value().chars().count();
        let caret = self.tab().caret_index;
        match n {
            NamedKey::Left => {
                let t = self.tab_mut();
                t.caret_index = caret.saturating_sub(1);
                true
            }
            NamedKey::Right => {
                let t = self.tab_mut();
                t.caret_index = (caret + 1).min(value_len);
                true
            }
            NamedKey::Home => {
                self.tab_mut().caret_index = 0;
                true
            }
            NamedKey::End => {
                self.tab_mut().caret_index = value_len;
                true
            }
            NamedKey::Backspace => {
                if caret == 0 {
                    return true;
                }
                let mut value: Vec<char> = self.tab().caret_value().chars().collect();
                if caret <= value.len() {
                    value.remove(caret - 1);
                }
                let text: String = value.into_iter().collect();
                let t = self.tab_mut();
                t.page.type_into(node, &text);
                t.caret_index = caret - 1;
                true
            }
            NamedKey::Delete => {
                let mut value: Vec<char> = self.tab().caret_value().chars().collect();
                if caret < value.len() {
                    value.remove(caret);
                }
                let text: String = value.into_iter().collect();
                let t = self.tab_mut();
                t.page.type_into(node, &text);
                true
            }
            NamedKey::Enter => {
                if multi {
                    self.page_type_char('\n');
                    return true;
                }
                // Enter in a single-line field submits its form
                self.submit_focused_form();
                true
            }
            NamedKey::Tab => {
                self.move_focus(if mods.shift { -1 } else { 1 });
                true
            }
            NamedKey::Escape => {
                self.focus_control(None);
                true
            }
            _ => false,
        }
    }

    // ------------------------------------------------------------- page focus --

    /// Give a control the caret (and the page the `:focus` state).
    pub fn focus_control(&mut self, node: Option<NodeId>) {
        if node == self.tab().caret_node {
            return;
        }
        self.tab_mut().page.set_focus(node);
        let text_field = {
            let t = self.tab();
            match node {
                Some(n) => matches!(
                    t.page.dom.tag(n),
                    Some("textarea") | Some("input") | Some("select")
                ),
                None => false,
            }
        };
        let len = if text_field {
            match node {
                Some(n) => self.page().input_value(n).chars().count(),
                None => 0,
            }
        } else {
            0
        };
        let t = self.tab_mut();
        t.caret_node = node;
        t.caret_index = len;
        if let Some(n) = node {
            t.page.scroll_into_view(n);
        }
        self.mode = Mode::Normal;
        self.caret = Caret::Page;
        self.dirty = true;
    }

    /// `Tab` / `Shift+Tab` through the page's focusable elements.
    pub fn move_focus(&mut self, dir: i32) {
        let list = self.page().focusables();
        if list.is_empty() {
            self.status("nothing focusable on this page", LineKind::Dim);
            return;
        }
        let current = self.tab().caret_node;
        let pos = current.and_then(|n| list.iter().position(|f| *f == n));
        let next = match pos {
            Some(i) => {
                if dir > 0 {
                    (i + 1) % list.len()
                } else {
                    (i + list.len() - 1) % list.len()
                }
            }
            None => {
                if dir > 0 {
                    0
                } else {
                    list.len() - 1
                }
            }
        };
        let node = list[next];
        self.focus_control(Some(node));
        let desc = self.page().describe(node);
        self.status(&tb_core::text::truncate(&desc, 60), LineKind::Accent);
    }

    /// `Enter`/`Space` on the focused element.
    pub fn activate_focused(&mut self) {
        let Some(node) = self.tab().caret_node else {
            self.status("nothing is focused (Tab focuses the page)", LineKind::Dim);
            return;
        };
        // a focused text field: Enter submits, Space types
        if self.tab().caret_is_text() {
            self.submit_focused_form();
            return;
        }
        let r = self.page().rect_of(node);
        let (x, y) = match r {
            Some(r) => (r.x + r.width / 2.0, r.y + r.height / 2.0),
            None => (0.0, 0.0),
        };
        self.click_node(node, x, y, 1);
    }

    /// Submit the form the focused control belongs to. With scripts the page's
    /// own `submit` handler decides; without them we build the GET ourselves.
    pub fn submit_focused_form(&mut self) {
        let Some(node) = self.tab().caret_node else {
            return;
        };
        let mut form = None;
        let mut cur = self.page().dom.parent(node);
        while let Some(c) = cur {
            if self.page().dom.tag(c) == Some("form") {
                form = Some(c);
                break;
            }
            cur = self.page().dom.parent(c);
        }
        let Some(form) = form else {
            self.status("this field is not inside a form", LineKind::Dim);
            return;
        };
        if self.page().has_js() {
            // fire `submit`; a listener may navigate, and may cancel — but if it
            // did neither, the form still submits, the way clicking a submit
            // button does when the page has no handler for it
            let allowed = self.tab_mut().page.fire_submit(form);
            let pending: Vec<tb_browser::NavRequest> =
                std::mem::take(&mut self.tab_mut().page.nav_requests);
            for nr in pending.clone() {
                self.handle_nav(nr);
            }
            if !allowed {
                self.status("the page cancelled the submit", LineKind::Dim);
                return;
            }
            if !pending.is_empty() {
                return;
            }
        }
        if let Some(nr) = self.page().form_action(form) {
            let url = match nr {
                tb_browser::NavRequest::Load(u) => u,
                tb_browser::NavRequest::Replace(u) => u,
                _ => return,
            };
            self.open(&url, Target::Current);
        }
    }

    // -------------------------------------------------------------- pointers --

    /// A pointer event (UI px). Eight arguments is the pointer: kind, button,
    /// position, wheel deltas and modifiers — all of them needed, none of them
    /// a bundle worth a struct that would only be assembled once.
    #[allow(clippy::too_many_arguments)]
    pub fn on_pointer(
        &mut self,
        kind: PointerKind,
        button: Option<Button>,
        x: i32,
        y: i32,
        dx: i32,
        dy: i32,
        mods: Mods,
    ) {
        // a menu or a select popup eats the event entirely
        if self.menu.is_some() {
            match kind {
                PointerKind::Press | PointerKind::Release => {
                    if !self.menu_hit(x, y) {
                        self.menu = None;
                        self.mode = Mode::Normal;
                    }
                }
                PointerKind::Move => self.menu_hover(x, y),
                _ => (),
            }
            self.dirty = true;
            return;
        }
        if self.select.is_some() {
            match kind {
                PointerKind::Press | PointerKind::Release => {
                    self.select_hit(x, y);
                }
                _ => (),
            }
            self.dirty = true;
            return;
        }

        let chrome = self.chrome();
        let widget = chrome.hit(x, y, mods);

        match kind {
            PointerKind::Wheel => self.wheel(x, y, dx, dy, mods),
            PointerKind::Move => {
                self.update_hover(x, y);
            }
            PointerKind::Drag => {
                // dragging a scrollbar thumb keeps scrubbing the page
                match self.press.map(|p| p.widget) {
                    Some(Widget::Scrollbar) => {
                        self.scrollbar_click(x, y);
                        return;
                    }
                    Some(Widget::HScrollbar) => {
                        self.hscrollbar_drag(x);
                        return;
                    }
                    _ => (),
                }
                if self.press.is_some() && self.viewport_point(x, y).is_some() {
                    self.extend_selection(x, y);
                }
            }
            PointerKind::Press => {
                self.press = Some(Press {
                    x,
                    y,
                    button: button.unwrap_or(Button::Left),
                    widget,
                    node: self.element_at_px(x, y),
                    at: Instant::now(),
                });
                // `:active` while the button is down
                if let Some(node) = self.press.and_then(|p| p.node) {
                    self.tab_mut().page.set_active(Some(node));
                }
                match widget {
                    Widget::None => self.page_press(x, y, button.unwrap_or(Button::Left), mods),
                    Widget::Menu => self.open_hamburger(),
                    _ => {
                        self.caret = Caret::Widget(widget);
                    }
                }
            }
            PointerKind::DoubleClick => {
                self.click_count = 2;
                self.page_double_click(x, y);
            }
            PointerKind::Release => {
                let press = self.press.take();
                self.tab_mut().page.set_active(None);
                let button = button.or(press.map(|p| p.button)).unwrap_or(Button::Left);
                let pressed_widget = press.map(|p| p.widget);
                let moved = press
                    .map(|p| (p.x - x).abs() + (p.y - y).abs())
                    .unwrap_or(0);
                // A press that began on the page (`None`, or no press record at
                // all) is a click or the end of a drag; one that began on a
                // chrome widget only fires when the button comes back up on the
                // same widget.
                let on_page = matches!(pressed_widget, None | Some(Widget::None));
                if !on_page {
                    match (pressed_widget, widget) {
                        (Some(w), now) if w == now => self.chrome_release(w, x, y, button, mods),
                        _ => (),
                    }
                } else if moved <= CLICK_SLOP {
                    // a click on the page: mouseup has to run before we decide
                    self.page_release(x, y, button, mods);
                } else if !self.tab().selection.is_empty() {
                    // drag-select ends with a copy (the primary-selection habit)
                    let pt = self.page_text();
                    let text = self.tab().selection.text(&pt);
                    if !text.is_empty() {
                        let n = text.chars().count();
                        self.clipboard.set(&text);
                        self.status(&format!("copied {n} characters"), LineKind::Ok);
                    }
                }
            }
        }
        self.dirty = true;
    }

    /// Wheel: the page scrolls, `Ctrl` zooms, over the tab strip it switches.
    fn wheel(&mut self, x: i32, y: i32, dx: i32, dy: i32, mods: Mods) {
        let chrome = self.chrome();
        let row = crate::chrome::px_to_row(y);
        if chrome.show_chrome && row == chrome.tabstrip_row() {
            if dy > 0 {
                self.next_tab();
            } else if dy < 0 {
                self.prev_tab();
            }
            return;
        }
        if mods.ctrl {
            self.zoom(if dy > 0 { -1 } else { 1 });
            return;
        }
        if chrome.hit_scrollbar(crate::chrome::px_to_col(x), row) {
            // the wheel over the scrollbar still scrolls, one step at a time
            self.scroll_by(0.0, dy as f32 * 40.0);
            return;
        }
        if self.viewport_point(x, y).is_none() {
            return;
        }
        let step = if mods.shift { 120.0 } else { 40.0 };
        self.scroll_by(dx as f32 * step, dy as f32 * step);
    }

    /// Hover: the link under the cursor goes to the status bar, `:hover` to the
    /// page (only when the hovered element actually changed — see G1.8).
    pub fn update_hover(&mut self, x: i32, y: i32) {
        if self.viewport_point(x, y).is_none() {
            return;
        }
        let node = self.element_at_px(x, y);
        let link = node.and_then(|n| self.page().closest_link(n));
        let changed = link != self.tab().hover_link || node != self.tab().hover_node;
        if !changed {
            return;
        }
        let restyle = self.config.hover_css;
        let t = self.tab_mut();
        t.hover_link = link;
        t.hover_node = node;
        t.page.set_hover(node, restyle);
        self.dirty = true;
    }

    /// Mouse down on the page.
    fn page_press(&mut self, x: i32, y: i32, button: Button, mods: Mods) {
        let Some((vx, vy)) = self.viewport_point(x, y) else {
            return;
        };
        if self.mode == Mode::Hints {
            self.hint_click(x, y);
            return;
        }
        let node = self.element_at_px(x, y);
        let Some(node) = node else {
            // clicked the canvas beside the content
            self.tab_mut().selection.clear();
            self.focus_control(None);
            return;
        };
        if self.mode == Mode::Pick {
            self.devtools.picked = Some(node);
            self.devtools.open = true;
            self.devtools.panel = crate::devtools::Panel::Elements;
            self.mode = Mode::Normal;
            let desc = self.page().describe(node);
            self.status(&tb_core::text::truncate(&desc, 70), LineKind::Accent);
            return;
        }
        let link = self.page().closest_link(node);
        let control = self.page().control_at(node);

        // Ctrl+click (or Cmd, which we map to Alt on keyboards without a Super)
        // opens the link in a new tab and leaves this page where it is — the
        // new tab waits in the background, which is the whole point.
        if mods.ctrl && button == Button::Left {
            if let Some(href) = link {
                self.open(&href, Target::Background);
                self.status("opened in a new tab (background)", LineKind::Ok);
                return;
            }
        }

        match button {
            Button::Middle => {
                if let Some(href) = link {
                    self.open(&href, Target::NewTab);
                    self.status("opened in a new tab", LineKind::Ok);
                }
                return;
            }
            Button::Right => {
                self.open_context_menu(x, y, link);
                return;
            }
            Button::Left => (),
        }

        // focus follows the click, exactly like it does in a browser
        let kind = control.as_ref().map(|(t, k)| (t.as_str(), k.as_str()));
        match kind {
            Some(("input", "checkbox")) | Some(("input", "radio")) => (),
            Some((
                "input",
                "text" | "search" | "email" | "url" | "password" | "number" | "tel",
            )) => {
                self.focus_control(Some(node));
                self.place_caret(node, x);
                return;
            }
            Some(("textarea", _)) => {
                self.focus_control(Some(node));
                self.place_caret(node, x);
                return;
            }
            Some(("select", _)) => {
                self.focus_control(Some(node));
                self.open_select(node, x, y);
                return;
            }
            Some(("option", _)) => {}
            Some(("a", _)) | Some(("area", _)) | Some(("button", _)) | Some(("input", _)) => {
                self.focus_control(Some(node));
            }
            _ => {
                if self.tab().caret_node.is_some() {
                    self.focus_control(None);
                }
            }
        }

        // a text selection starts where the pointer went down
        let pt = self.page_text();
        if let Some((entry, ch)) = pt.hit(vx + self.tab().scroll_x, vy + self.tab().scroll_y) {
            self.tab_mut()
                .selection
                .begin(selection::Point { entry, ch });
            self.click_count = self.bump_click_count();
            if self.click_count == 2 {
                let p = selection::Point { entry, ch };
                let sel = Selection::word(&pt, p);
                self.tab_mut().selection = sel;
            } else if self.click_count >= 3 {
                let p = selection::Point { entry, ch };
                self.tab_mut().selection = Selection::line(&pt, p);
            }
        } else {
            self.tab_mut().selection.clear();
        }
    }

    /// Mouse up on the page: this is where a browser decides about a click.
    fn page_release(&mut self, x: i32, y: i32, button: Button, mods: Mods) {
        let Some((vx, vy)) = self.viewport_point(x, y) else {
            return;
        };
        let Some(node) = self.element_at_px(x, y) else {
            return;
        };
        self.click_node(node, vx, vy, 1);
        let _ = (button, mods);
    }

    /// Fire the click trio on a node and deal with what comes back.
    pub fn click_node(&mut self, node: NodeId, vx: f32, vy: f32, detail: i32) {
        let mods = Mods::none();
        let prevented = self
            .tab_mut()
            .page
            .pointer_click(node, vx, vy, 0, detail, mods.to_page());
        // `<a target="_blank">` means "somewhere else", whatever the request is
        let blank = self.targets_new_tab(node);
        self.tab_mut().page.pump(0.0);
        let requests: Vec<tb_browser::NavRequest> =
            std::mem::take(&mut self.tab_mut().page.nav_requests);
        if !requests.is_empty() {
            for nr in requests {
                self.handle_nav_in(nr, blank);
            }
            return;
        }
        if prevented {
            self.status("the page handled this click", LineKind::Dim);
            return;
        }
        // no scripts (or scripts that did not ask): the markup's own meaning
        if let Some(nr) = self.page().default_action(node) {
            self.handle_nav_in(nr, blank);
        }
    }

    /// A navigation as the click asked for it: a `target="_blank"` link turns a
    /// plain load into a new tab.
    fn handle_nav_in(&mut self, nr: tb_browser::NavRequest, new_tab: bool) {
        if new_tab {
            if let tb_browser::NavRequest::Load(u) = nr {
                self.open(&u, Target::NewTab);
                return;
            }
        }
        self.handle_nav(nr);
    }

    /// `<a target="_blank">`, on the node itself or on the anchor around it.
    fn targets_new_tab(&self, node: NodeId) -> bool {
        let mut cur = Some(node);
        while let Some(n) = cur {
            if let Some(t) = self.page().dom.attr(n, "target") {
                if t == "_blank" {
                    return true;
                }
            }
            if self.page().dom.tag(n) == Some("a") {
                break;
            }
            cur = self.page().dom.parent(n);
        }
        false
    }

    /// A double click selects the word under the pointer.
    fn page_double_click(&mut self, x: i32, y: i32) {
        let Some((vx, vy)) = self.viewport_point(x, y) else {
            return;
        };
        let pt = self.page_text();
        if let Some((entry, ch)) = pt.hit(vx + self.tab().scroll_x, vy + self.tab().scroll_y) {
            let p = selection::Point { entry, ch };
            self.tab_mut().selection = Selection::word(&pt, p);
            let text = self.tab().selection.text(&pt);
            if !text.is_empty() {
                self.clipboard.set(&text);
            }
        }
    }

    /// Drag the selection end to the pointer.
    fn extend_selection(&mut self, x: i32, y: i32) {
        let Some((vx, vy)) = self.viewport_point(x, y) else {
            return;
        };
        let pt = self.page_text();
        if let Some((entry, ch)) = pt.hit(vx + self.tab().scroll_x, vy + self.tab().scroll_y) {
            self.tab_mut()
                .selection
                .extend(selection::Point { entry, ch });
        }
    }

    /// Put the caret in a text field at the character the pointer is on.
    fn place_caret(&mut self, node: NodeId, px_x: i32) {
        let Some(r) = self.page().rect_of(node) else {
            return;
        };
        let scale = self.config.scale.max(0.1);
        let doc_x = (px_x - self.content_px().0) as f32 / scale + self.tab().scroll_x;
        let pt = self.page_text();
        let index = pt
            .hit(doc_x, r.y + r.height / 2.0)
            .map(|(e, _ch)| pt.index_in(e, doc_x))
            .unwrap_or_else(|| {
                // no text yet: the caret is wherever the pointer is along the box
                let adv = tb_gfx::Font::system().advance(
                    self.page()
                        .style_of(node)
                        .map(|s| s.font_size)
                        .unwrap_or(16.0),
                );
                (((doc_x - r.x - 4.0) / adv.max(1.0)).round() as isize).max(0) as usize
            });
        let len = self.page().input_value(node).chars().count();
        self.tab_mut().caret_index = index.min(len);
    }

    /// Click a hint label.
    fn hint_click(&mut self, x: i32, y: i32) {
        let view = self.view_rect();
        let scale = self.config.scale.max(0.1);
        let doc_x = (x - self.content_px().0) as f32 / scale + view.x;
        let doc_y = (y - self.content_px().1) as f32 / scale + view.y;
        if let Some(h) = self
            .hints
            .hints
            .iter()
            .find(|h| h.rect.contains(tb_core::Point::new(doc_x, doc_y)))
            .cloned()
        {
            let new_tab = self.hints.new_tab;
            self.hints.reset();
            self.mode = Mode::Normal;
            if let Some(href) = h.href {
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
    }

    /// `click_count` with the double-click window applied.
    fn bump_click_count(&mut self) -> i32 {
        let now = Instant::now();
        let count = match self.last_click {
            Some(t) if now.duration_since(t) < CLICK_TIME => self.click_count + 1,
            _ => 1,
        };
        self.last_click = Some(now);
        self.click_count = count;
        count
    }

    // -------------------------------------------------------------- menus ----

    /// The hamburger menu (`[=]`).
    pub fn open_hamburger(&mut self) {
        let (_, _, _, x) = self.content_px();
        let _ = x;
        let items = vec![
            crate::app::MenuItem::item("New tab                Ctrl+T", Command::NewTab(None)),
            crate::app::MenuItem::item("Close tab              Ctrl+W", Command::CloseTab),
            crate::app::MenuItem::item("Reopen closed tab  Ctrl+Shift+T", Command::RestoreTab),
            crate::app::MenuItem::sep(),
            crate::app::MenuItem::item("Reload                     Ctrl+R", Command::Reload),
            crate::app::MenuItem::item("Reload (no cache)   Ctrl+Shift+R", Command::ReloadHard),
            crate::app::MenuItem::item("Home                          Alt+H", Command::Home),
            crate::app::MenuItem::sep(),
            crate::app::MenuItem::item("Find in page            Ctrl+F", Command::FindBar),
            crate::app::MenuItem::item("Link hints                     f", Command::Hints(false)),
            crate::app::MenuItem::item("Copy selection         Ctrl+C", Command::Copy),
            crate::app::MenuItem::item("Bookmark this page   Ctrl+D", Command::BookmarkPage),
            crate::app::MenuItem::sep(),
            crate::app::MenuItem::item("Inspector                   F12", Command::DevtoolsToggle),
            crate::app::MenuItem::item(
                "Console                     Ctrl+J",
                Command::DevtoolsPanel(1),
            ),
            crate::app::MenuItem::item("Dark / light theme        K", Command::ToggleTheme),
            crate::app::MenuItem::item("Help                            F1", Command::Help),
            crate::app::MenuItem::item("Quit                             Ctrl+Q", Command::Quit),
        ];
        self.menu = Some(crate::app::Menu {
            items,
            sel: 0,
            col: self.cols.saturating_sub(26),
            row: self.chrome().toolbar_row(),
            link: None,
        });
        self.mode = Mode::Menu;
        self.dirty = true;
    }

    /// The context menu (right click), with the link if there is one.
    pub fn open_context_menu(&mut self, x: i32, y: i32, link: Option<String>) {
        let col = crate::chrome::px_to_col(x).min(self.cols.saturating_sub(26));
        let row = crate::chrome::px_to_row(y).min(self.rows.saturating_sub(12));
        let mut items = Vec::new();
        if let Some(href) = link.clone() {
            let short = tb_core::text::truncate(&href, 20);
            items.push(crate::app::MenuItem {
                label: format!("→ {short}"),
                cmd: None,
            });
            items.push(crate::app::MenuItem::item(
                "Open link in new tab",
                Command::NewTab(Some(href.clone())),
            ));
            let app_href = href;
            items.push(crate::app::MenuItem {
                label: "Copy link address".to_string(),
                cmd: Some(Command::Go(app_href.clone())),
            });
        }
        items.push(crate::app::MenuItem::item("Back", Command::Back));
        items.push(crate::app::MenuItem::item("Forward", Command::Forward));
        items.push(crate::app::MenuItem::item("Reload", Command::Reload));
        items.push(crate::app::MenuItem::sep());
        items.push(crate::app::MenuItem::item("Select all", Command::SelectAll));
        items.push(crate::app::MenuItem::item("Copy", Command::Copy));
        items.push(crate::app::MenuItem::item(
            "View source",
            Command::ViewSource,
        ));
        items.push(crate::app::MenuItem::item(
            "Inspect element",
            Command::PickElement,
        ));
        self.menu = Some(crate::app::Menu {
            items,
            sel: 0,
            col,
            row,
            link,
        });
        self.mode = Mode::Menu;
        self.dirty = true;
    }

    /// Is `(x, y)` inside the open menu? If so, pick that item.
    fn menu_hit(&mut self, x: i32, y: i32) -> bool {
        let Some(menu) = self.menu.clone() else {
            return false;
        };
        let col = crate::chrome::px_to_col(x);
        let row = crate::chrome::px_to_row(y);
        let w = menu
            .items
            .iter()
            .map(|i| i.label.chars().count())
            .max()
            .unwrap_or(20)
            + 4;
        let h = menu.items.len() + 2;
        if col < menu.col || col >= menu.col + w || row < menu.row || row >= menu.row + h {
            return false;
        }
        if let Some(i) = menu.index_at_row(row) {
            let cmd = menu.items[i].cmd.clone();
            self.menu = None;
            self.mode = Mode::Normal;
            if let Some(c) = cmd {
                self.run(c);
            }
            return true;
        }
        true
    }

    fn menu_hover(&mut self, _x: i32, y: i32) {
        let row = crate::chrome::px_to_row(y);
        if let Some(menu) = &mut self.menu {
            if let Some(i) = menu.index_at_row(row) {
                menu.sel = i;
            }
        }
    }

    /// Keyboard inside the menu.
    fn menu_key(&mut self, key: Key, _mods: Mods) {
        match key {
            Key::Named(NamedKey::Escape) => {
                self.menu = None;
                self.mode = Mode::Normal;
            }
            Key::Named(NamedKey::Down) => {
                if let Some(m) = &mut self.menu {
                    m.select_next(1);
                }
            }
            Key::Named(NamedKey::Up) => {
                if let Some(m) = &mut self.menu {
                    m.select_next(-1);
                }
            }
            Key::Named(NamedKey::Enter) => {
                let cmd = self
                    .menu
                    .as_ref()
                    .and_then(|m| m.items.get(m.sel))
                    .and_then(|i| i.cmd.clone());
                self.menu = None;
                self.mode = Mode::Normal;
                if let Some(c) = cmd {
                    self.run(c);
                }
            }
            _ => {
                self.menu = None;
                self.mode = Mode::Normal;
            }
        }
        self.dirty = true;
    }

    // --------------------------------------------------------- select popup --

    /// Open the option list of a `<select>`.
    pub fn open_select(&mut self, node: NodeId, x: i32, y: i32) {
        let options = App::options_of(node, self.page());
        if options.is_empty() {
            return;
        }
        let current = self
            .page()
            .dom
            .descendants(node)
            .into_iter()
            .filter(|n| self.page().dom.tag(*n) == Some("option"))
            .position(|n| self.page().dom.has_attr(n, "selected"))
            .unwrap_or(0);
        self.select = Some(crate::app::SelectPopup {
            node,
            options,
            index: current,
            col: crate::chrome::px_to_col(x).min(self.cols.saturating_sub(24)),
            row: crate::chrome::px_to_row(y).min(self.rows.saturating_sub(3)),
        });
        self.mode = Mode::Select;
        self.dirty = true;
    }

    fn select_hit(&mut self, x: i32, y: i32) {
        let row = crate::chrome::px_to_row(y);
        let col = crate::chrome::px_to_col(x);
        let Some(p) = self.select.clone() else {
            return;
        };
        let width = p
            .options
            .iter()
            .map(|o| o.chars().count())
            .max()
            .unwrap_or(8)
            + 4;
        if col < p.col || col >= p.col + width {
            self.select = None;
            self.mode = Mode::Normal;
            return;
        }
        if row > p.row && row <= p.row + p.options.len() {
            let index = row - p.row - 1;
            if index < p.options.len() {
                self.choose_option(index);
                return;
            }
        }
        self.select = None;
        self.mode = Mode::Normal;
    }

    /// Pick an option: the `selected` flag moves and `change` fires.
    pub fn choose_option(&mut self, index: usize) {
        let Some(popup) = self.select.clone() else {
            return;
        };
        let node = popup.node;
        let options: Vec<NodeId> = self
            .page()
            .dom
            .descendants(node)
            .into_iter()
            .filter(|n| self.page().dom.tag(*n) == Some("option"))
            .collect();
        if let Some(opt) = options.get(index) {
            let t = self.tab_mut();
            for o in &options {
                if *o == *opt {
                    t.page.dom.set_attr(*o, "selected", "");
                } else {
                    t.page.dom.remove_attr(*o, "selected");
                }
            }
            t.page.fire_change(node);
            t.page.pump(0.0);
        }
        self.select = None;
        self.mode = Mode::Normal;
        self.dirty = true;
    }

    fn select_key(&mut self, key: Key, mods: Mods) {
        match key {
            Key::Named(NamedKey::Escape) => {
                self.select = None;
                self.mode = Mode::Normal;
            }
            Key::Named(NamedKey::Down) => {
                if let Some(p) = &mut self.select {
                    p.index = (p.index + 1).min(p.options.len().saturating_sub(1));
                }
            }
            Key::Named(NamedKey::Up) => {
                if let Some(p) = &mut self.select {
                    p.index = p.index.saturating_sub(1);
                }
            }
            Key::Named(NamedKey::Enter) | Key::Char(' ') => {
                let index = self.select.as_ref().map(|p| p.index).unwrap_or(0);
                let _ = mods;
                self.choose_option(index);
                return;
            }
            _ => {
                self.select = None;
                self.mode = Mode::Normal;
            }
        }
        self.dirty = true;
    }

    /// Release on one of our own buttons.
    pub fn chrome_release(&mut self, widget: Widget, x: i32, y: i32, button: Button, mods: Mods) {
        match widget {
            Widget::Back => self.back(),
            Widget::Forward => self.forward(),
            Widget::Reload => self.reload(mods.shift),
            Widget::Home => {
                let home = self.config.home.clone();
                self.open(&home, Target::Current);
            }
            Widget::Address => self.open_address(),
            Widget::Star => self.bookmark_this(),
            Widget::Info => self.run(Command::DevtoolsToggle),
            Widget::Menu => (), // the menu is already open from the press
            Widget::NewTab => self.new_tab(None),
            Widget::CloseTab(i) => self.close_tab(i),
            Widget::Tab(i) => {
                if button == Button::Middle {
                    self.close_tab(i);
                } else {
                    self.use_tab(i);
                }
            }
            Widget::Bookmark(i) => {
                if let Some(b) = self.bookmarks.find(i).cloned() {
                    if button == Button::Middle {
                        self.open(&b.url, Target::NewTab);
                    } else {
                        self.open(&b.url, Target::Current);
                    }
                }
            }
            Widget::Scrollbar => self.scrollbar_click(x, y),
            Widget::HScrollbar => (),
            Widget::FindField => {
                if self.mode != Mode::Find {
                    self.open_find();
                }
            }
            Widget::FindPrev => self.find_step(-1),
            Widget::FindNext => self.find_step(1),
            Widget::FindClose => self.close_overlays(),
            Widget::Command => {
                self.mode = Mode::Command;
                self.dirty = true;
            }
            Widget::Devtools => {
                if self.devtools.panel == crate::devtools::Panel::Console {
                    self.mode = Mode::Console;
                }
            }
            Widget::None => (),
        }
    }

    /// Click on the scrollbar: a thumb drag scrolls to the fraction, a track
    /// click pages (both like Chrome).
    /// Drag along the horizontal scrollbar: the x fraction of the track.
    fn hscrollbar_drag(&mut self, x: i32) {
        let chrome = self.chrome();
        let (cx, _, cw, _) = chrome.content_cells();
        if cw == 0 {
            return;
        }
        let col = crate::chrome::px_to_col(x);
        let frac = ((col.saturating_sub(cx)) as f32 / cw as f32).clamp(0.0, 1.0);
        let max = self.page().max_scroll_x();
        self.scroll_to(frac * max, self.tab().scroll_y);
    }

    fn scrollbar_click(&mut self, _x: i32, y: i32) {
        let chrome = self.chrome();
        let (_, cy, _, ch) = chrome.content_cells();
        if ch == 0 {
            return;
        }
        let row = crate::chrome::px_to_row(y);
        let frac = ((row.saturating_sub(cy)) as f32 / ch as f32).clamp(0.0, 1.0);
        let max = self.page().max_scroll();
        self.scroll_to(self.tab().scroll_x, frac * max);
    }
}
