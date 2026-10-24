//! Rendering the shell: cells for every backend, plus real page pixels for the
//! backends that have them.
//!
//! [`paint_screen`] is the canonical picture: tab strip, toolbar, bookmark bar,
//! the page's text (from the layout runs, so it is the *same* text the engine
//! laid out, at the same columns), selection and find highlights, the caret of a
//! focused field, hint labels, the inspector drawer, the scrollbars and the
//! status line. A terminal writes it with SGR escapes; the X11 backend draws it
//! with our own bitmap font into a framebuffer.
//!
//! [`paint_frame`] is the same chrome, but the content area is filled with the
//! **page's own pixels** (`Page::paint_view`), so CSS colours, borders, images
//! and text metrics are the real thing. Cells marked `overlay` (menus, hints,
//! help) are painted afterwards, which is how a popup floats above a CSS page
//! without a compositor.

use tb_core::{Color, Rect};
use tb_gfx::{Framebuffer, TextPaint};

use crate::chrome::{CELL_H, CELL_W};
use crate::devtools::LineKind;
use crate::mode::Mode;
use crate::screen::{Screen, Style};
use crate::theme::Theme;
use crate::App;

/// The whole UI as a cell grid.
pub fn paint_screen(app: &App) -> Screen {
    let mut screen = Screen::new(app.cols, app.rows);
    screen.title = app.title();
    let theme = app.theme;
    screen.clear(theme.bg);
    let chrome = app.chrome();

    // ---- tab strip -------------------------------------------------------
    if chrome.show_chrome {
        draw_tabs(app, &mut screen);
        draw_toolbar(app, &mut screen);
        if let Some(row) = chrome.bookmark_row() {
            draw_bookmarks(app, &mut screen, row);
        }
    }

    // ---- the page --------------------------------------------------------
    draw_page(app, &mut screen);

    // ---- overlays that belong to the content area -------------------------
    draw_find_highlight(app, &mut screen);
    draw_selection(app, &mut screen);
    draw_field_caret(app, &mut screen);
    draw_hints(app, &mut screen);
    draw_scrollbar(app, &mut screen);

    // ---- inspector drawer -------------------------------------------------
    if chrome.devtools_rows > 0 {
        draw_devtools(
            app,
            &mut screen,
            chrome.devtools_top(),
            chrome.devtools_rows,
        );
    }

    // ---- find / command bar, status line ----------------------------------
    if chrome.find_open {
        draw_find_bar(app, &mut screen, chrome.bar_row());
    }
    if chrome.command_open {
        draw_command_bar(app, &mut screen, chrome.bar_row());
    }
    if let Some(row) = chrome.status_row() {
        draw_status(app, &mut screen, row);
    }

    // ---- popups and help --------------------------------------------------
    if let Some(menu) = app.menu.clone() {
        let width = menu
            .items
            .iter()
            .map(|i| i.label.chars().count())
            .max()
            .unwrap_or(20)
            + 4;
        let height = menu.items.len() + 2;
        screen.boxed_overlay(menu.col, menu.row, width, height, "menu", &theme);
        for (i, item) in menu.items.iter().enumerate() {
            let row = menu.row_of(i);
            let selected = i == menu.sel && item.cmd.is_some();
            let st = if item.cmd.is_none() {
                Style::new(theme.dim, theme.menu_bg)
            } else if selected {
                Style::new(theme.menu_sel_fg, theme.menu_sel_bg).bold(true)
            } else {
                Style::new(theme.menu_fg, theme.menu_bg)
            };
            screen.text_in(
                menu.col + 2,
                row,
                width.saturating_sub(4),
                &item.label,
                st,
                ' ',
                false,
            );
        }
    }
    if let Some(popup) = app.select.clone() {
        let width = popup
            .options
            .iter()
            .map(|o| o.chars().count())
            .max()
            .unwrap_or(8)
            + 4;
        let height = popup.options.len() + 2;
        screen.boxed_overlay(popup.col, popup.row, width, height, "options", &theme);
        for (i, opt) in popup.options.iter().enumerate() {
            let row = popup.row + 1 + i;
            let st = if i == popup.index {
                Style::new(theme.menu_sel_fg, theme.menu_sel_bg).bold(true)
            } else {
                Style::new(theme.menu_fg, theme.menu_bg)
            };
            let mark = if i == popup.index { "› " } else { "  " };
            screen.text_in(
                popup.col + 2,
                row,
                width.saturating_sub(4),
                &format!("{mark}{opt}"),
                st,
                ' ',
                false,
            );
        }
    }
    if app.mode == Mode::Help {
        draw_help(app, &mut screen);
    }

    // ---- the hardware cursor ---------------------------------------------
    screen.cursor = caret_cell(app);
    screen
}

/// Render, then paint the page's own pixels over the content area.
pub fn paint_frame(app: &App) -> Framebuffer {
    let screen = paint_screen(app);
    let theme = app.theme;
    let mut fb = Framebuffer::new(app.width.max(1), app.height.max(1));
    fb.clear(theme.bar_bg);
    draw_cells(&mut fb, &screen, false);
    // the page, in CSS pixels, inside the content area
    let (cx, cy, cw, ch) = app.content_px();
    if cw > 0 && ch > 0 && !app.tabs.is_empty() {
        let pagefb = app.page().paint_view(&app.paint_options());
        blit_clipped(&mut fb, &pagefb, cx, cy, cw, ch, theme.bg);
        // selection and find ride on top of the pixels, translucent so the text
        // underneath stays readable (CSS highlight is a multiply, this is close)
        for r in selection_rects_px(app) {
            fb.fill_rect(r, theme.selection_bg.blend_over(Color::rgba(0, 0, 0, 90)));
        }
        for r in find_rects_px(app, false) {
            fb.fill_rect(r, theme.find_bg);
        }
        for r in find_rects_px(app, true) {
            fb.fill_rect(r, theme.find_current_bg);
        }
    }
    // popups, hints, help: cells that float above the page
    draw_cells(&mut fb, &screen, true);
    fb
}

// ---------------------------------------------------------------- chrome ----

/// The tab strip: numbered titles, the active one highlighted, `×` per tab and
/// a `+` at the end of the row.
fn draw_tabs(app: &App, screen: &mut Screen) {
    let chrome = app.chrome();
    let widgets = chrome.widgets();
    let theme = app.theme;
    let row = chrome.tabstrip_row();
    screen.fill(0, row, app.cols, 1, theme.bar_bg);
    for (i, tab) in app.tabs.iter().enumerate() {
        let Some(span) = widgets
            .iter()
            .find(|(w, _)| *w == crate::chrome::Widget::Tab(i))
            .map(|(_, s)| *s)
        else {
            continue;
        };
        let active = i == app.active;
        let st = if active {
            Style::new(theme.tab_active_fg, theme.tab_active_bg).bold(true)
        } else {
            Style::new(theme.tab_fg, theme.tab_bg)
        };
        let title = tb_core::text::truncate(&tab.title(), span.w.saturating_sub(5).max(3));
        let label = format!(" {} {}", i + 1, title);
        screen.text_in(span.x, row, span.w, &label, st, ' ', false);
        if span.w >= 6 {
            let close = if active {
                Style::new(theme.error, theme.tab_active_bg).bold(true)
            } else {
                Style::new(theme.dim, theme.tab_bg)
            };
            screen.text(span.x + span.w - 2, row, " x", close);
        }
    }
    if let Some(span) = widgets
        .iter()
        .find(|(w, _)| *w == crate::chrome::Widget::NewTab)
        .map(|(_, s)| *s)
    {
        let st = Style::new(theme.accent, theme.bar_bg).bold(true);
        screen.text_in(span.x, row, span.w.max(2), " +", st, ' ', false);
    }
}

/// Back / forward / reload / home, the address bar, then the bookmark star, the
/// inspector and the menu.
fn draw_toolbar(app: &App, screen: &mut Screen) {
    let chrome = app.chrome();
    let widgets = chrome.widgets();
    let theme = app.theme;
    let row = chrome.toolbar_row();
    screen.fill(0, row, app.cols, 1, theme.bar_bg);
    for (i, (label, widget)) in crate::chrome::Chrome::NAV_BUTTONS.iter().enumerate() {
        let enabled = match widget {
            crate::chrome::Widget::Back => app.tab().can_back(),
            crate::chrome::Widget::Forward => app.tab().can_forward(),
            _ => true,
        };
        let st = if enabled {
            Style::new(theme.bar_fg, theme.bar_bg)
        } else {
            Style::new(theme.dim, theme.bar_bg)
        };
        screen.text_in(i * 4, row, 3, label, st, ' ', false);
    }
    // the address bar: field, security mark, text, caret
    if let Some(span) = widgets
        .iter()
        .find(|(w, _)| *w == crate::chrome::Widget::Address)
        .map(|(_, s)| *s)
    {
        let focused = app.mode == Mode::Address;
        let st = if focused {
            Style::new(theme.url_fg, theme.url_bg).bold(true)
        } else {
            Style::new(theme.url_fg, theme.url_bg)
        };
        screen.fill(span.x, row, span.w, 1, theme.url_bg);
        screen.text(
            span.x,
            row,
            secure_mark(app),
            Style::new(theme.ok, theme.url_bg),
        );
        let inner = span.w.saturating_sub(2);
        let shown = app.address.display(inner);
        screen.text_in(span.x + 2, row, inner, &shown, st, ' ', false);
        if focused {
            let caret = span.x + 2 + app.address.display_caret(inner);
            screen.put(
                caret.min(app.cols.saturating_sub(1)),
                row,
                ' ',
                Style::new(theme.caret, theme.url_bg).inverse(true),
            );
        }
    }
    for (label, widget) in crate::chrome::Chrome::RIGHT_BUTTONS.iter() {
        let Some(span) = widgets.iter().find(|(w, _)| *w == *widget).map(|(_, s)| *s) else {
            continue;
        };
        let on = match widget {
            crate::chrome::Widget::Star => app.bookmarks.has(&app.url()),
            crate::chrome::Widget::Info => app.devtools.open,
            _ => false,
        };
        let st = if on {
            Style::new(theme.accent, theme.bar_bg).bold(true)
        } else {
            Style::new(theme.bar_fg, theme.bar_bg)
        };
        screen.text_in(span.x, row, 3, label, st, ' ', false);
    }
}

/// The bookmark bar: one label per bookmark, click to go there.
fn draw_bookmarks(app: &App, screen: &mut Screen, row: usize) {
    let theme = app.theme;
    screen.fill(0, row, app.cols, 1, theme.bar_bg);
    for (widget, span) in app.chrome().widgets() {
        let crate::chrome::Widget::Bookmark(i) = widget else {
            continue;
        };
        let Some(b) = app.bookmarks.find(i) else {
            continue;
        };
        let label = if b.title.is_empty() { &b.url } else { &b.title };
        screen.text_in(
            span.x,
            row,
            span.w,
            label,
            Style::new(theme.link, theme.bar_bg).underline(true),
            ' ',
            false,
        );
    }
}

fn draw_status(app: &App, screen: &mut Screen, row: usize) {
    let theme = app.theme;
    let (text, kind) = app.status_line();
    let st = Style::new(line_color(&theme, kind), theme.status_bg);
    screen.fill(0, row, app.cols, 1, theme.status_bg);
    let mode = mode_label(app);
    let line = if mode.is_empty() {
        text
    } else {
        format!("{mode} {text}")
    };
    screen.text_in(0, row, app.cols, &line, st, ' ', false);
}

fn mode_label(app: &App) -> String {
    match app.mode {
        Mode::Normal => String::new(),
        m => format!("[{}] ", m.as_str()),
    }
}

fn draw_find_bar(app: &App, screen: &mut Screen, row: usize) {
    let theme = app.theme;
    let x = app.cols.saturating_sub(56);
    let w = app.cols.saturating_sub(x).saturating_add(1);
    screen.fill(x, row, w, 1, theme.bar_bg);
    screen.text(
        x,
        row,
        " Find: ",
        Style::new(theme.bar_fg, theme.bar_bg).bold(true),
    );
    let field_x = x + 7;
    let field_w = 22;
    screen.fill(field_x, row, field_w, 1, theme.url_bg);
    let shown = app.find_input.display(field_w);
    screen.text(field_x, row, &shown, Style::new(theme.url_fg, theme.url_bg));
    let caret = field_x + app.find_input.display_caret(field_w);
    screen.put(
        caret.min(app.cols.saturating_sub(1)),
        row,
        ' ',
        Style::new(theme.caret, theme.url_bg).inverse(true),
    );
    let (i, n) = {
        let f = &app.tab().find;
        if f.count() == 0 {
            (0, 0)
        } else {
            (f.index + 1, f.count())
        }
    };
    screen.text_in(
        field_x + field_w + 1,
        row,
        8,
        &format!("{i}/{n}"),
        Style::new(theme.dim, theme.bar_bg),
        ' ',
        false,
    );
    screen.text(
        app.cols.saturating_sub(9),
        row,
        crate::chrome::LABEL_PREV,
        Style::new(theme.bar_fg, theme.bar_bg),
    );
    screen.text(
        app.cols.saturating_sub(6),
        row,
        crate::chrome::LABEL_NEXT,
        Style::new(theme.bar_fg, theme.bar_bg),
    );
    screen.text(
        app.cols.saturating_sub(3),
        row,
        crate::chrome::LABEL_FIND_CLOSE,
        Style::new(theme.error, theme.bar_bg),
    );
}

fn draw_command_bar(app: &App, screen: &mut Screen, row: usize) {
    let theme = app.theme;
    screen.fill(0, row, app.cols, 1, theme.bar_bg);
    screen.text(
        0,
        row,
        ": ",
        Style::new(theme.accent, theme.bar_bg).bold(true),
    );
    screen.text(
        2,
        row,
        &app.cmdline.value(),
        Style::new(theme.url_fg, theme.url_bg),
    );
    let caret = 2 + app.cmdline.caret.min(app.cols.saturating_sub(3));
    screen.put(
        caret,
        row,
        ' ',
        Style::new(theme.caret, theme.bar_bg).inverse(true),
    );
}

fn draw_devtools(app: &App, screen: &mut Screen, top: usize, rows: usize) {
    let theme = app.theme;
    screen.fill(0, top, app.cols, rows, theme.bar_bg);
    screen.hline(
        0,
        top,
        app.cols,
        '─',
        Style::new(theme.border, theme.bar_bg),
    );
    // panel tabs
    let mut x = 1usize;
    for (i, p) in crate::devtools::PANELS.iter().enumerate() {
        let label = format!("{}·{}", i + 1, p.title());
        let st = if *p == app.devtools.panel {
            Style::new(theme.accent_fg, theme.accent).bold(true)
        } else {
            Style::new(theme.dim, theme.bar_bg)
        };
        x += screen.text(x, top + 1, &label, st) + 1;
        if x > app.cols.saturating_sub(6) {
            break;
        }
    }
    let focused = app.mode == Mode::Console;
    screen.text(
        app.cols.saturating_sub(16),
        top + 1,
        if focused { "prompt ▍" } else { "" },
        Style::new(theme.accent, theme.bar_bg).bold(true),
    );
    let lines = app.devtools.lines(app.page(), rows.saturating_sub(2));
    for (i, line) in lines.iter().enumerate() {
        let y = top + 2 + i;
        if y >= top + rows {
            break;
        }
        let st = Style::new(line_color(&theme, line.kind), theme.bar_bg);
        screen.text_in(1, y, app.cols.saturating_sub(2), &line.text, st, ' ', false);
    }
    if app.devtools.panel == crate::devtools::Panel::Console && focused {
        let y = (top + rows).saturating_sub(1);
        screen.text_in(
            1,
            y,
            app.cols.saturating_sub(2),
            &format!("> {}", app.devtools.prompt.value()),
            Style::new(theme.fg, theme.bar_bg),
            ' ',
            false,
        );
    }
}

fn draw_help(app: &App, screen: &mut Screen) {
    let theme = app.theme;
    let rows = app.rows;
    let cols = app.cols;
    let w = 76.min(cols.saturating_sub(4));
    let h = rows
        .saturating_sub(4)
        .min(crate::keymap::SHORTCUTS.len() + 4);
    let x = cols.saturating_sub(w) / 2;
    let y = 2;
    screen.boxed_overlay(x, y, w, h, "tiny — keyboard", &theme);
    let per_col = (h - 2).max(1);
    let two_cols = w > 80;
    for (i, (keys, what)) in crate::keymap::SHORTCUTS.iter().enumerate() {
        let (col_index, row_index) = if two_cols {
            (i / per_col, i % per_col)
        } else {
            (0, i)
        };
        if row_index >= per_col {
            break;
        }
        let base = if col_index == 0 { x + 2 } else { x + w / 2 + 1 };
        let line_w = (w / 2).saturating_sub(4).max(24);
        let st = Style::new(theme.menu_fg, theme.menu_bg);
        screen.text_in(base, y + 1 + row_index, 22, keys, st.bold(true), ' ', false);
        screen.text_in(
            base + 23,
            y + 1 + row_index,
            line_w.saturating_sub(23),
            what,
            st,
            ' ',
            false,
        );
    }
    screen.text_in(
        x + 2,
        y + h - 2,
        w.saturating_sub(4),
        "any key closes this — mouse: click, middle-click new tab, wheel, drag to select, right-click menu",
        Style::new(theme.dim, theme.menu_bg),
        ' ',
        false,
    );
}

// ------------------------------------------------------------------ page ----

/// The page's text runs, mapped to cells.
fn draw_page(app: &App, screen: &mut Screen) {
    if app.tabs.is_empty() {
        return;
    }
    let theme = app.theme;
    let chrome = app.chrome();
    let (_, top, w, h) = chrome.content_cells();
    let (cx, cy, _cw, chh) = app.content_px();
    let scale = app.config.scale.max(0.1);
    let view = app.view_rect();
    let pt = app.page_text();
    for entry in pt.entries.iter() {
        let sx = cx as f32 + (entry.rect.x - view.x) * scale;
        let sy = cy as f32 + (entry.rect.y - view.y) * scale;
        if sy > chh as f32 || sy + entry.rect.height * scale < 0.0 {
            continue;
        }
        let col = (sx / CELL_W as f32).round() as i32;
        let row = ((sy + 1.0) / CELL_H as f32).floor() as i32 - top as i32;
        if row < 0 || row >= h as i32 {
            continue;
        }
        let fg = if let Some(link) = &entry.link {
            let _ = link;
            theme.link
        } else {
            entry.color
        };
        let mut st = Style::new(fg, entry.bg);
        if entry.bold {
            st = st.bold(true);
        }
        if entry.link.is_some() || entry.underline {
            st = st.underline(true);
        }
        let _ = scale;
        let _ = w;
        for (i, ch) in entry.chars().enumerate() {
            let c = col + i as i32;
            if c < 0 || c >= app.cols as i32 {
                break;
            }
            screen.put(c as usize, (row + top as i32) as usize, ch, st);
        }
    }
}

fn draw_find_highlight(app: &App, screen: &mut Screen) {
    if app.tab().find.needle.is_empty() {
        return;
    }
    let theme = app.theme;
    let pt = app.page_text();
    let map = Mapper::new(app);
    for (i, f) in app.tab().find.matches.iter().enumerate() {
        let Some(entry) = pt.entries.get(f.entry) else {
            continue;
        };
        let bg = if i == app.tab().find.index {
            theme.find_current_bg
        } else {
            theme.find_bg
        };
        map.fill_cells(screen, entry.char_rect(f.from, f.to), bg);
    }
}

fn draw_selection(app: &App, screen: &mut Screen) {
    let sel = app.tab().selection;
    if sel.is_empty() {
        return;
    }
    let theme = app.theme;
    let pt = app.page_text();
    let map = Mapper::new(app);
    for r in sel.rects(&pt) {
        map.fill_cells(screen, r, theme.selection_bg);
    }
}

/// Document coordinates → cell coordinates: the content area's origin, the
/// zoom factor and the first content row, in one place, so every overlay maps
/// the same way the page text does.
struct Mapper {
    view: Rect,
    cx: f32,
    cy: f32,
    scale: f32,
    top: usize,
    cols: usize,
}

impl Mapper {
    fn new(app: &App) -> Mapper {
        let (_, top, _, _) = app.chrome().content_cells();
        let (cx, cy, _, _) = app.content_px();
        Mapper {
            view: app.view_rect(),
            cx: cx as f32,
            cy: cy as f32,
            scale: app.config.scale.max(0.1),
            top,
            cols: app.cols,
        }
    }

    /// The cell rectangle a document-space rectangle covers.
    fn cells(&self, r: Rect) -> (usize, usize, usize, usize) {
        let x0 = (self.cx + (r.x - self.view.x) * self.scale) / CELL_W as f32;
        let x1 = (self.cx + (r.right() - self.view.x) * self.scale) / CELL_W as f32;
        let y0 = (self.cy + (r.y - self.view.y) * self.scale) / CELL_H as f32;
        let y1 = (self.cy + (r.bottom() - self.view.y) * self.scale) / CELL_H as f32;
        (
            x0.floor().max(0.0) as usize,
            x1.ceil().min(self.cols as f32) as usize,
            (y0.floor().max(0.0) as usize).saturating_sub(self.top),
            (y1.ceil().max(0.0) as usize).saturating_sub(self.top),
        )
    }

    /// Background of the cells a rectangle covers (selection, find highlight).
    fn fill_cells(&self, screen: &mut Screen, r: Rect, bg: Color) {
        let (c0, c1, r0, r1) = self.cells(r);
        for row in r0..r1.min(screen.rows) {
            for col in c0..c1 {
                screen.put_bg(col, row + self.top, bg);
            }
        }
    }
}

/// The caret of the page control that has focus.
fn draw_field_caret(app: &App, screen: &mut Screen) {
    let Some(node) = app.tab().caret_node else {
        return;
    };
    let theme = app.theme;
    let Some(r) = app.page().rect_of(node) else {
        return;
    };
    let (_, top, _, _) = app.chrome().content_cells();
    let (cx, cy, _, _) = app.content_px();
    let scale = app.config.scale.max(0.1);
    let view = app.view_rect();
    let value = app.page().input_value(node);
    let index = app.tab().caret_index.min(value.chars().count());
    let adv = tb_gfx::Font::system().advance(
        app.page()
            .style_of(node)
            .map(|s| s.font_size)
            .unwrap_or(16.0),
    );
    let doc_x = r.x + 4.0 + index as f32 * adv;
    let sx = cx as f32 + (doc_x - view.x) * scale;
    let sy = cy as f32 + (r.y - view.y) * scale;
    let col = (sx / CELL_W as f32).round().max(0.0) as usize;
    let row = ((sy / CELL_H as f32).floor() as usize).saturating_sub(top);
    if row < app.rows && col < app.cols {
        let ch = screen.cell(col, row).map(|c| c.ch).unwrap_or(' ');
        screen.put(
            col,
            row,
            ch,
            Style::new(theme.bg, theme.caret).inverse(true),
        );
    }
}

/// Hint labels over the page.
fn draw_hints(app: &App, screen: &mut Screen) {
    if app.mode != Mode::Hints || !app.hints.active {
        return;
    }
    let theme = app.theme;
    let view = app.view_rect();
    let (_, top, _, _) = app.chrome().content_cells();
    let (cx, cy, _, _) = app.content_px();
    let scale = app.config.scale.max(0.1);
    for h in app.hints.candidates() {
        let exact = h.label == app.hints.typed;
        let st = if exact {
            Style::new(Color::WHITE, theme.error).bold(true)
        } else {
            Style::new(Color::WHITE, theme.accent).bold(true)
        };
        let sx = cx as f32 + (h.rect.x - view.x) * scale;
        let sy = cy as f32 + (h.rect.y - view.y) * scale;
        let col = (sx / CELL_W as f32).floor().max(0.0) as usize;
        let row = ((sy / CELL_H as f32).floor() as usize).saturating_sub(top);
        // the label, with the typed prefix dimmed
        let label = format!("<{}>", h.label);
        for (i, ch) in label.chars().enumerate() {
            let typed = i < app.hints.typed.chars().count() + 1;
            let s = if typed {
                Style::new(Color::WHITE, theme.error).bold(true)
            } else {
                st
            };
            screen.put_overlay(col + i, row, ch, s);
        }
    }
}

/// The right-edge scrollbar: thumb position says where we are, the track says
/// how much there is.
fn draw_scrollbar(app: &App, screen: &mut Screen) {
    let theme = app.theme;
    let chrome = app.chrome();
    let (_, top, _, h) = chrome.content_cells();
    if h < 3 {
        return;
    }
    let max = app.page().max_scroll();
    let page = app.view_rect().height.max(1.0);
    let doc = (page + max).max(1.0);
    let col = app.cols.saturating_sub(1);
    let track = Style::new(theme.dim, theme.bg);
    for row in top..top + h {
        screen.put(col, row, '│', track);
    }
    if max <= 1.0 {
        return;
    }
    let thumb_h = ((page / doc) * h as f32).clamp(1.0, h as f32);
    let pos = (app.tab().scroll_y / max).clamp(0.0, 1.0);
    let thumb_y = pos * (h as f32 - thumb_h);
    let y0 = top + thumb_y as usize;
    let y1 = (y0 + thumb_h as usize).max(y0 + 1).min(top + h);
    for row in y0..y1 {
        screen.put(col, row, '█', Style::new(theme.border, theme.bg));
    }
}

// -------------------------------------------------------------- pixel glue ---

/// Draw every cell of `screen` into `fb` with our bitmap font; `overlays_only`
/// draws only the cells that float above the page.
pub fn draw_cells(fb: &mut Framebuffer, screen: &Screen, overlays_only: bool) {
    let scale = 2.0f32; // 5×7 at scale 2 = 10×14, the cell is 12×16
    for y in 0..screen.rows {
        for x in 0..screen.cols {
            let Some(cell) = screen.cell(x, y) else {
                continue;
            };
            if overlays_only && !cell.overlay {
                continue;
            }
            let px = x as i32 * CELL_W;
            let py = y as i32 * CELL_H;
            let (fg, bg) = if cell.style.inverse {
                (cell.style.bg, cell.style.fg)
            } else {
                (cell.style.fg, cell.style.bg)
            };
            // background first (a full cell, so the pixel grid stays clean)
            fb.fill_rect(
                Rect::new(px as f32, py as f32, CELL_W as f32, CELL_H as f32),
                bg,
            );
            if cell.ch == ' ' {
                continue;
            }
            let baseline = py as f32 + (CELL_H as f32 + scale * 7.0) / 2.0 - 2.0;
            TextPaint {
                color: fg,
                size: 14.0,
                bold: cell.style.bold,
                underline: cell.style.underline,
                ..Default::default()
            }
            .draw(fb, px as f32, baseline, &cell.ch.to_string(), None);
        }
    }
}

/// Blit `src` into `dst` at `(dx, dy)`, clipped to a `w × h` window.
pub fn blit_clipped(
    dst: &mut Framebuffer,
    src: &Framebuffer,
    dx: i32,
    dy: i32,
    w: i32,
    h: i32,
    fill: Color,
) {
    let x0 = dx.max(0);
    let y0 = dy.max(0);
    let x1 = (dx + w).min(dst.width as i32);
    let y1 = (dy + h).min(dst.height as i32);
    for y in y0..y1 {
        for x in x0..x1 {
            let sx = (x - dx) as u32;
            let sy = (y - dy) as u32;
            let c = if sx < src.width && sy < src.height {
                src.get(sx, sy)
            } else {
                fill
            };
            dst.blend_at(x, y, c);
        }
    }
}

/// Selection rectangles in device px (the content area is the origin).
fn selection_rects_px(app: &App) -> Vec<Rect> {
    let sel = app.tab().selection;
    if sel.is_empty() {
        return Vec::new();
    }
    let pt = app.page_text();
    rects_px(app, sel.rects(&pt))
}

fn find_rects_px(app: &App, current: bool) -> Vec<Rect> {
    let f = &app.tab().find;
    if f.needle.is_empty() {
        return Vec::new();
    }
    let pt = app.page_text();
    let mut out = Vec::new();
    for (i, m) in f.matches.iter().enumerate() {
        if (i == f.index) != current {
            continue;
        }
        if let Some(e) = pt.entries.get(m.entry) {
            out.extend(rects_px(app, vec![e.char_rect(m.from, m.to)]));
        }
    }
    out
}

/// Document-space rectangles → device px inside the content area.
fn rects_px(app: &App, rects: Vec<Rect>) -> Vec<Rect> {
    let (cx, cy, _, _) = app.content_px();
    let view = app.view_rect();
    let scale = app.config.scale.max(0.1);
    rects
        .into_iter()
        .map(|r| {
            Rect::new(
                cx as f32 + (r.x - view.x) * scale,
                cy as f32 + (r.y - view.y) * scale,
                r.width * scale,
                r.height * scale,
            )
        })
        .collect()
}

fn line_color(theme: &Theme, kind: LineKind) -> Color {
    match kind {
        LineKind::Heading => theme.fg,
        LineKind::Text => theme.fg,
        LineKind::Dim => theme.dim,
        LineKind::Ok => theme.ok,
        LineKind::Warn => theme.warn,
        LineKind::Error => theme.error,
        LineKind::Accent => theme.accent,
    }
}

/// `https` gets a closed lock, `http` an open one, `data:`/`about:` a dot — the
/// same three-glance affordance a real browser gives.
fn secure_mark(app: &App) -> &'static str {
    let url = app.url();
    if url.starts_with("https://") {
        "(*)"
    } else if url.starts_with("http://") {
        "(!)"
    } else {
        "(·)"
    }
}

/// Where the hardware cursor should be.
fn caret_cell(app: &App) -> Option<(usize, usize)> {
    let chrome = app.chrome();
    match app.mode {
        Mode::Address => {
            let span = chrome.widgets().into_iter().find_map(|(w, s)| {
                if w == crate::chrome::Widget::Address {
                    Some(s)
                } else {
                    None
                }
            })?;
            let w = span.w.saturating_sub(2);
            Some((span.x + 2 + app.address.display_caret(w), span.row))
        }
        Mode::Find => Some((
            app.cols
                .saturating_sub(56)
                .saturating_add(9)
                .saturating_add(app.find_input.caret),
            chrome.bar_row(),
        )),
        Mode::Command => Some((
            2 + app.cmdline.caret.min(app.cols.saturating_sub(3)),
            chrome.bar_row(),
        )),
        Mode::Console => Some((
            3 + app.devtools.prompt.caret.min(app.cols.saturating_sub(4)),
            (chrome.devtools_top() + chrome.devtools_rows).saturating_sub(1),
        )),
        _ => None,
    }
}
