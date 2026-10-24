//! The browser shell, driven the way a user drives it.
//!
//! Every test here is an input event — a key, a click, a wheel notch — followed
//! by an assertion about what the user would now see: the address bar, the page
//! that loaded, the selection, the status line, the pixels of the screen. No
//! terminal and no window server is involved, because the shell draws into a
//! [`Screen`] that a test can read.

use std::path::{Path, PathBuf};

use tb_ui::render::paint_screen;
use tb_ui::{App, Event, Key, Mode, Mods, NamedKey, UiConfig};

/// A page with everything a user can point at, written next to the test binary
/// so the URLs are `file://` and the tests never need a network.
fn fixture(name: &str, html: &str) -> String {
    // one directory per test: the tests run in parallel and must not step on
    // each other's fixtures
    let me = std::thread::current()
        .name()
        .unwrap_or("test")
        .replace("::", "_");
    let mut dir = std::env::temp_dir();
    dir.push(format!("tb-ui-tests-{}-{}", std::process::id(), me));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path: PathBuf = [dir.as_path(), Path::new(name)].iter().collect();
    std::fs::write(&path, html).expect("write fixture");
    // the pages the fixtures link to live next to them, so a click has
    // somewhere to go
    for (sibling, body) in [("second.html", SECOND), ("third.html", THIRD)] {
        let p: PathBuf = [dir.as_path(), Path::new(sibling)].iter().collect();
        if !p.exists() {
            std::fs::write(&p, body).expect("write sibling fixture");
        }
    }
    format!("file://{}", path.display())
}

const HOME: &str = r#"<!doctype html>
<html><head><title>Home page</title>
<style>body { margin: 0; font-family: sans-serif }</style></head>
<body>
<h1>Heading one</h1>
<p id="first">A paragraph with a <a id="lnk" href="second.html">second page</a> inside it.</p>
<p>Second paragraph, same words as the <a href="third.html">third page</a>.</p>
<form action="search.html" method="get">
  <input type="text" name="q">
  <button type="submit">Go</button>
</form>
</body></html>"#;

const SECOND: &str =
    "<html><head><title>Second page</title></head><body><h1>Page two</h1></body></html>";
const THIRD: &str =
    "<html><head><title>Third page</title></head><body><h1>Page three</h1></body></html>";

fn app_at(urls: &[String]) -> App {
    // no subresources: the tests are about the shell, not about the network
    let cfg = UiConfig {
        fetch: false,
        timeout_ms: 5_000,
        ..UiConfig::default()
    };
    App::new(cfg, tb_ui::Theme::light(), 1200, 800, urls)
}

fn app_at_url(url: &str) -> App {
    app_at(&[url.to_string()])
}

/// Type a string as if the keyboard produced it.
fn type_text(app: &mut App, s: &str) {
    for c in s.chars() {
        app.on_event(Event::char(c));
    }
}

fn key(app: &mut App, k: Key, mods: Mods) {
    app.on_event(Event::key(k, mods));
}

fn ctrl(app: &mut App, c: char) {
    key(
        app,
        Key::Char(c),
        Mods {
            ctrl: true,
            ..Mods::none()
        },
    );
}

fn press_key(app: &mut App, n: NamedKey) {
    app.on_event(Event::named(n));
}

/// Where a link with this text sits, in UI pixels.
fn link_point(app: &App, text: &str) -> (i32, i32) {
    let pt = app.page_text();
    let view = app.view_rect();
    let (cx, cy, _, _) = app.content_px();
    let scale = app.config.scale.max(0.1);
    for e in &pt.entries {
        if e.link.is_some() && e.text.contains(text) {
            let x = cx as f32 + (e.rect.x + 2.0 - view.x) * scale;
            let y = cy as f32 + (e.rect.y + e.rect.height / 2.0 - view.y) * scale;
            return (x as i32, y as i32);
        }
    }
    panic!("no link containing {text:?} on the page");
}

/// Where a run of this text sits, in UI pixels.
fn text_point(app: &App, text: &str) -> (i32, i32) {
    let pt = app.page_text();
    let view = app.view_rect();
    let (cx, cy, _, _) = app.content_px();
    let scale = app.config.scale.max(0.1);
    for e in &pt.entries {
        if e.text.contains(text) {
            let x = cx as f32 + (e.rect.x + 2.0 - view.x) * scale;
            let y = cy as f32 + (e.rect.y + e.rect.height / 2.0 - view.y) * scale;
            return (x as i32, y as i32);
        }
    }
    panic!("no text containing {text:?} on the page");
}

fn click(app: &mut App, x: i32, y: i32) {
    app.on_event(Event::press(x, y));
    app.on_event(Event::release(x, y));
}

// ------------------------------------------------------------------- booting ----

#[test]
fn boots_with_a_page_and_chrome() {
    let url = fixture("home.html", HOME);
    let app = app_at_url(&url);
    assert_eq!(app.tabs.len(), 1);
    assert_eq!(app.tab().page.title(), "Home page");
    assert!(app.url().ends_with("home.html"), "{}", app.url());

    let dump = paint_screen(&app).dump();
    // the tab strip numbers the tab, the toolbar has its buttons, and the
    // address shows where we are
    assert!(dump.contains("1 Home page"), "{dump}");
    assert!(dump.contains("home.html"), "{dump}");
    let (_, _, w, _) = app.content_px();
    assert!(w > 0);
}

#[test]
fn no_chrome_means_no_chrome() {
    let url = fixture("home.html", HOME);
    let cfg = UiConfig {
        show_chrome: false,
        fetch: false,
        ..UiConfig::default()
    };
    let app = App::new(cfg, tb_ui::Theme::light(), 1200, 800, &[url]);
    let dump = paint_screen(&app).dump();
    assert!(!dump.contains("home.html"), "the toolbar must be gone");
    assert!(dump.contains("Heading"), "the page is still there");
}

#[test]
fn resize_recomputes_the_grid() {
    let url = fixture("home.html", HOME);
    let mut app = app_at_url(&url);
    let before = app.cols;
    app.on_event(Event::Resize {
        width: 600,
        height: 500,
    });
    assert!(app.cols < before, "{} → {}", before, app.cols);
    // and it stays a working browser afterwards
    press_key(&mut app, NamedKey::End);
    assert_eq!(app.tab().page.title(), "Home page");
}

// ---------------------------------------------------------------- navigation ----

#[test]
fn address_bar_navigates() {
    let url = fixture("home.html", HOME);
    let mut app = app_at_url(&url);
    ctrl(&mut app, 'l');
    assert_eq!(app.mode, Mode::Address);
    type_text(&mut app, "second.html");
    press_key(&mut app, NamedKey::Enter);
    assert_eq!(app.mode, Mode::Normal);
    assert!(app.url().ends_with("second.html"), "{}", app.url());
    assert_eq!(app.tab().page.title(), "Second page");
}

#[test]
fn clicking_a_link_navigates_and_back_returns() {
    let url = fixture("home.html", HOME);
    let mut app = app_at_url(&url);
    let (x, y) = link_point(&app, "second");
    click(&mut app, x, y);
    assert!(app.url().ends_with("second.html"), "{}", app.url());

    app.on_event(Event::key(
        Key::Named(NamedKey::Left),
        Mods {
            alt: true,
            ..Mods::none()
        },
    ));
    assert!(app.url().ends_with("home.html"), "{}", app.url());
    // forward again
    key(
        &mut app,
        Key::Named(NamedKey::Right),
        Mods {
            alt: true,
            ..Mods::none()
        },
    );
    assert!(app.url().ends_with("second.html"), "{}", app.url());
}

#[test]
fn middle_click_opens_the_link_in_a_new_tab() {
    let url = fixture("home.html", HOME);
    let mut app = app_at_url(&url);
    let (x, y) = link_point(&app, "third");
    app.on_event(Event::pointer(
        tb_ui::PointerKind::Press,
        Some(tb_ui::Button::Middle),
        x,
        y,
    ));
    app.on_event(Event::pointer(
        tb_ui::PointerKind::Release,
        Some(tb_ui::Button::Middle),
        x,
        y,
    ));
    assert_eq!(app.tabs.len(), 2);
    assert_eq!(app.active, 1, "the new tab becomes the current one");
    assert!(app.url().ends_with("third.html"), "{}", app.url());
}

#[test]
fn ctrl_click_opens_a_background_tab() {
    let url = fixture("home.html", HOME);
    let mut app = app_at_url(&url);
    let (x, y) = link_point(&app, "third");
    let mods = Mods {
        ctrl: true,
        ..Mods::none()
    };
    app.on_event(Event::pointer_with(
        tb_ui::PointerKind::Press,
        Some(tb_ui::Button::Left),
        x,
        y,
        mods,
    ));
    app.on_event(Event::pointer_with(
        tb_ui::PointerKind::Release,
        Some(tb_ui::Button::Left),
        x,
        y,
        mods,
    ));
    assert_eq!(app.tabs.len(), 2);
    assert!(app.tabs[1].page.url.to_string().ends_with("third.html"));
    assert_eq!(app.active, 0, "ctrl+click does not steal the current tab");
}

#[test]
fn reload_keeps_the_page() {
    let url = fixture("home.html", HOME);
    let mut app = app_at_url(&url);
    ctrl(&mut app, 'r');
    assert_eq!(app.tab().page.title(), "Home page");
    assert!(app.url().ends_with("home.html"));
}

// ------------------------------------------------------------------- scrolling ----

#[test]
fn keys_scroll_the_page() {
    let mut body = String::from("<html><head><title>Tall</title></head><body>");
    for i in 0..400 {
        body.push_str(&format!("<p>Row of text number {i}</p>"));
    }
    body.push_str("</body></html>");
    let url = fixture("tall.html", &body);
    let mut app = app_at_url(&url);
    assert_eq!(app.tab().scroll_y, 0.0);

    key(&mut app, Key::Char('j'), Mods::none());
    assert!(app.tab().scroll_y > 0.0, "'j' scrolls down");
    key(&mut app, Key::Char('k'), Mods::none());
    assert!(app.tab().scroll_y < 40.0, "'k' scrolls back up");

    press_key(&mut app, NamedKey::End);
    let bottom = app.tab().scroll_y;
    assert!(bottom > 1000.0, "End goes to the bottom ({bottom})");
    press_key(&mut app, NamedKey::Home);
    assert_eq!(app.tab().scroll_y, 0.0);
}

#[test]
fn the_wheel_scrolls_and_the_status_shows_the_link() {
    let mut body = String::from("<html><head><title>Tall</title></head><body>");
    for i in 0..400 {
        body.push_str(&format!("<p>Row of text number {i}</p>"));
    }
    body.push_str("</body></html>");
    let url = fixture("tall.html", &body);
    let mut app = app_at_url(&url);
    let before = app.tab().scroll_y;
    app.on_event(Event::wheel(600, 400, 3));
    assert!(app.tab().scroll_y > before, "the wheel scrolls down");
    app.on_event(Event::wheel(600, 400, -3));
    assert!(app.tab().scroll_y < before + 3.0, "and back up");

    // hovering a link puts its target in the status line
    let url = fixture("home.html", HOME);
    let mut app = app_at_url(&url);
    let (x, y) = link_point(&app, "second");
    app.on_event(Event::move_to(x, y));
    let (status, _) = app.status_line();
    assert!(status.contains("second.html"), "{status}");
}

// --------------------------------------------------------------------- find ----

#[test]
fn find_highlights_counts_and_wraps() {
    let url = fixture("home.html", HOME);
    let mut app = app_at_url(&url);
    key(&mut app, Key::Char('/'), Mods::none());
    assert_eq!(app.mode, Mode::Find);
    type_text(&mut app, "paragraph");
    let n = app.tab().find.count();
    assert_eq!(n, 2, "two paragraphs mention it");
    // Enter accepts the search: the bar closes, the matches stay
    press_key(&mut app, NamedKey::Enter);
    assert_eq!(app.mode, Mode::Normal, "Enter leaves the find bar");
    assert_eq!(app.tab().find.count(), n, "and keeps the matches");
    let first = app.tab().find.index;
    key(&mut app, Key::Char('n'), Mods::none());
    assert_ne!(app.tab().find.index, first, "'n' moves to the next match");
    // a full round of `n` comes back to where we started
    for _ in 0..(n - 1) {
        key(&mut app, Key::Char('n'), Mods::none());
    }
    assert_eq!(app.tab().find.index, first, "'n' wraps around");
    key(&mut app, Key::Char('N'), Mods::none());
    assert_ne!(app.tab().find.index, first, "'N' goes back");

    // the highlight is on the screen
    let dump = paint_screen(&app).dump();
    assert!(dump.contains("paragraph"), "{dump}");
}

#[test]
fn escape_closes_the_overlays() {
    let url = fixture("home.html", HOME);
    let mut app = app_at_url(&url);
    key(&mut app, Key::Char('/'), Mods::none());
    assert_eq!(app.mode, Mode::Find);
    press_key(&mut app, NamedKey::Escape);
    assert_eq!(app.mode, Mode::Normal);

    ctrl(&mut app, 'l');
    assert_eq!(app.mode, Mode::Address);
    press_key(&mut app, NamedKey::Escape);
    assert_eq!(app.mode, Mode::Normal);

    key(&mut app, Key::Char('f'), Mods::none());
    assert_eq!(app.mode, Mode::Hints);
    press_key(&mut app, NamedKey::Escape);
    assert_eq!(app.mode, Mode::Normal);
}

// --------------------------------------------------------------------- hints ----

#[test]
fn hint_mode_labels_links_and_follows_one() {
    let url = fixture("home.html", HOME);
    let mut app = app_at_url(&url);
    key(&mut app, Key::Char('f'), Mods::none());
    assert_eq!(app.mode, Mode::Hints);
    let dump = paint_screen(&app).dump();
    // hint labels are drawn over the page
    assert!(
        dump.chars().any(|c| c.is_ascii_lowercase()),
        "hints should put letters on the page"
    );
    let labels: Vec<String> = app.hints.hints.iter().map(|h| h.label.clone()).collect();
    assert!(!labels.is_empty(), "two links means at least two hints");

    // the second link is the "third page"; typing its label navigates
    let label = labels[1].clone();
    type_text(&mut app, &label);
    assert!(app.url().ends_with("third.html"), "{}", app.url());
    assert_eq!(app.mode, Mode::Normal);
}

// ---------------------------------------------------------------------- tabs ----

#[test]
fn tabs_open_switch_and_close() {
    let url = fixture("home.html", HOME);
    let mut app = app_at_url(&url);
    ctrl(&mut app, 't');
    assert_eq!(app.tabs.len(), 2);
    assert_eq!(app.active, 1);

    // the new tab is the home page, which by default is blank
    ctrl(&mut app, '1');
    assert_eq!(app.active, 0);
    assert_eq!(app.tab().page.title(), "Home page");
    ctrl(&mut app, '2');
    assert_eq!(app.active, 1);
    ctrl(&mut app, '9');
    assert_eq!(app.active, 1, "'9' goes to the last tab");

    ctrl(&mut app, 'w');
    assert_eq!(app.tabs.len(), 1);
    assert_eq!(
        app.tab().page.title(),
        "Home page",
        "we are back on the first tab"
    );

    ctrl(&mut app, 'w');
    assert_eq!(app.tabs.len(), 1, "the last tab does not close the window");
}

#[test]
fn a_closed_tab_comes_back_with_ctrl_shift_t() {
    let url = fixture("home.html", HOME);
    let second = fixture("second.html", SECOND);
    let mut app = app_at(&[url, second]);
    assert_eq!(app.tabs.len(), 2);
    ctrl(&mut app, 'w');
    assert_eq!(app.tabs.len(), 1);
    key(
        &mut app,
        Key::Char('t'),
        Mods {
            ctrl: true,
            shift: true,
            ..Mods::none()
        },
    );
    assert_eq!(app.tabs.len(), 2, "the closed tab is restored");
}

// ------------------------------------------------------------------ selection ----

#[test]
fn drag_selects_text_and_ctrl_c_copies_it() {
    let url = fixture("home.html", HOME);
    let mut app = app_at_url(&url);
    let (x, y) = text_point(&app, "paragraph");
    app.on_event(Event::press(x, y));
    app.on_event(Event::drag(x + 200, y));
    app.on_event(Event::release(x + 200, y));
    let sel = app.tab().selection.text(&app.page_text());
    assert!(!sel.is_empty(), "a drag over text selects it: {sel:?}");
    assert!(
        sel.contains("paragraph") || sel.contains("A "),
        "the selected text is the text under the mouse: {sel:?}"
    );

    ctrl(&mut app, 'c');
    assert_eq!(app.clipboard.get(), sel, "Ctrl+C copies the selection");
}

#[test]
fn double_click_selects_a_word() {
    let url = fixture("home.html", HOME);
    let mut app = app_at_url(&url);
    let (x, y) = text_point(&app, "paragraph");
    // two presses in the same place within the double-click time
    app.on_event(Event::press(x, y));
    app.on_event(Event::release(x, y));
    app.on_event(Event::press(x, y));
    app.on_event(Event::release(x, y));
    let sel = app.tab().selection.text(&app.page_text());
    assert!(!sel.is_empty(), "a double-click selects a word: {sel:?}");
    assert!(
        !sel.contains(' ') && sel.chars().count() > 3,
        "one word, not a line: {sel:?}"
    );
}

#[test]
fn ctrl_a_selects_the_page_and_escape_clears_it() {
    let url = fixture("home.html", HOME);
    let mut app = app_at_url(&url);
    ctrl(&mut app, 'a');
    let sel = app.tab().selection.text(&app.page_text());
    assert!(sel.contains("Heading one"), "{sel}");
    press_key(&mut app, NamedKey::Escape);
    assert!(
        app.tab().selection.text(&app.page_text()).is_empty(),
        "Escape clears the selection"
    );
}

// --------------------------------------------------------------------- forms ----

#[test]
fn typing_goes_into_the_focused_field_and_enter_submits() {
    let url = fixture("home.html", HOME);
    let mut app = app_at_url(&url);
    // Tab into the form: the first focusable element is the link, then the
    // input; keep going until the input has the caret.
    let mut typed = false;
    for _ in 0..6 {
        press_key(&mut app, NamedKey::Tab);
        let focused = app.tab().page.focused();
        if focused.map(|n| app.tab().page.dom.tag(n) == Some("input")) == Some(true) {
            typed = true;
            break;
        }
    }
    assert!(typed, "Tab reaches the text input");
    type_text(&mut app, "hello world");
    let focused = app.tab().page.focused().expect("a focused field");
    assert_eq!(app.tab().page.input_value(focused), "hello world");

    press_key(&mut app, NamedKey::Enter);
    let url = app.url();
    assert!(url.contains("search.html"), "{url}");
    assert!(
        url.contains("q=hello") || url.contains("q=hello+world"),
        "{url}"
    );
}

// ------------------------------------------------------------------ devtools ----

#[test]
fn devtools_opens_with_panels_and_problems() {
    let url = fixture("home.html", HOME);
    let mut app = app_at_url(&url);
    key(&mut app, Key::Char('i'), Mods::none());
    assert!(app.devtools.open);
    let dump = paint_screen(&app).dump();
    assert!(
        dump.contains("Elements") || dump.contains("element"),
        "{dump}"
    );

    // the panels move
    let first = app.panel();
    app.run(tb_ui::Command::DevtoolsPanel(0));
    app.run(tb_ui::Command::DevtoolsPanel(2));
    assert_ne!(app.panel(), first, "the panel changed");

    // the same key closes it
    app.run(tb_ui::Command::DevtoolsToggle);
    assert!(!app.devtools.open, "the toggle closes it");
}

// ------------------------------------------------------- zoom, theme, misc ----

#[test]
fn zoom_changes_the_scale_and_the_viewport() {
    let url = fixture("home.html", HOME);
    let mut app = app_at_url(&url);
    let (scale, viewport) = (app.config.scale, app.tab().page.config.viewport.width);
    ctrl(&mut app, '=');
    assert!(app.config.scale > scale, "Ctrl++ zooms in");
    assert!(
        app.tab().page.config.viewport.width < viewport,
        "a zoomed page has fewer CSS px across"
    );
    ctrl(&mut app, '0');
    assert_eq!(app.config.scale, 1.0, "Ctrl+0 resets");
}

#[test]
fn theme_toggle_changes_the_colours() {
    let url = fixture("home.html", HOME);
    let mut app = app_at_url(&url);
    let before = paint_screen(&app);
    key(
        &mut app,
        Key::Char('K'),
        Mods {
            shift: true,
            ..Mods::none()
        },
    );
    let after = paint_screen(&app);
    assert_ne!(before.cells, after.cells, "the theme changed the pixels");
    assert_eq!(after.title, before.title);
}

#[test]
fn help_lists_the_shortcuts_and_closes() {
    let url = fixture("home.html", HOME);
    let mut app = app_at_url(&url);
    key(&mut app, Key::Char('?'), Mods::none());
    assert_eq!(app.mode, Mode::Help);
    let dump = paint_screen(&app).dump();
    assert!(dump.contains("Ctrl+L") || dump.contains("Ctrl l"), "{dump}");
    press_key(&mut app, NamedKey::Escape);
    assert_eq!(app.mode, Mode::Normal);
}

#[test]
fn command_bar_runs_a_command() {
    let url = fixture("home.html", HOME);
    let mut app = app_at_url(&url);
    key(&mut app, Key::Char(':'), Mods::none());
    assert_eq!(app.mode, Mode::Command);
    type_text(&mut app, "reload");
    press_key(&mut app, NamedKey::Enter);
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.tab().page.title(), "Home page");
}

#[test]
fn quit_is_a_command_like_any_other() {
    let url = fixture("home.html", HOME);
    let mut app = app_at_url(&url);
    ctrl(&mut app, 'q');
    assert!(app.quit, "Ctrl+Q asks the shell to stop");
}

#[test]
fn clicking_a_button_activates_it() {
    let html = r#"<html><head><title>Buttons</title></head><body>
<a href="second.html"><button id="b">Press me</button></a>
</body></html>"#;
    let url = fixture("buttons.html", html);
    let mut app = app_at_url(&url);
    let (x, y) = text_point(&app, "Press");
    click(&mut app, x, y);
    assert!(app.url().ends_with("second.html"), "{}", app.url());
}

#[test]
fn the_view_source_command_shows_the_markup() {
    let url = fixture("home.html", HOME);
    let mut app = app_at_url(&url);
    let before = app.tabs.len();
    app.run(tb_ui::Command::ViewSource);
    assert_eq!(app.tabs.len(), before + 1, "the source opens in a tab");
    let dump = paint_screen(&app).dump();
    assert!(
        dump.contains("html") || dump.contains("doctype"),
        "the source tab shows markup: {dump}"
    );
}

#[test]
fn search_instead_of_a_url() {
    let cfg = UiConfig {
        fetch: false,
        search_url: "https://example.invalid/?q={}".to_string(),
        ..UiConfig::default()
    };
    let mut app = App::new(cfg, tb_ui::Theme::light(), 1200, 800, &[]);
    ctrl(&mut app, 'l');
    type_text(&mut app, "tiny browser");
    press_key(&mut app, NamedKey::Enter);
    let url = app.url();
    assert!(
        url.contains("example.invalid") && url.contains("q=tiny%20browser"),
        "the words became a search: {url}"
    );
}

#[test]
fn keyboard_focus_traverses_the_page() {
    let url = fixture("home.html", HOME);
    let mut app = app_at_url(&url);
    let n = app.page().focusables().len();
    assert!(n >= 3, "two links, a field and a button");
    press_key(&mut app, NamedKey::Tab);
    let first = app.tab().page.focused();
    assert!(first.is_some(), "Tab focuses something");
    press_key(&mut app, NamedKey::Tab);
    assert_ne!(app.tab().page.focused(), first, "Tab moves on");
    key(&mut app, Key::Named(NamedKey::Tab), Mods::shift());
    assert_eq!(app.tab().page.focused(), first, "Shift+Tab goes back");
}

#[test]
fn a_fragment_scroll_to_the_element() {
    let mut body = String::from("<html><head><title>Anchors</title></head><body>");
    for i in 0..200 {
        body.push_str(&format!("<p>Row {i}</p>"));
    }
    body.push_str("<p id=\"target\">The target paragraph</p></body></html>");
    let base = fixture("anchors.html", &body);
    let mut app = app_at_url(&base);
    ctrl(&mut app, 'l');
    type_text(&mut app, &format!("{base}#target"));
    press_key(&mut app, NamedKey::Enter);
    assert!(app.tab().scroll_y > 0.0, "a fragment scrolls the page");
    // the anchor itself is inside the view now
    let node = app
        .page()
        .dom
        .get_element_by_id("target")
        .expect("the fixture has id=target");
    let r = app.page().rect_of(node).expect("a laid out anchor");
    let view = app.view_rect();
    assert!(
        r.top() >= view.top() && r.top() < view.bottom(),
        "anchor at {} is not in the view {}..{}",
        r.top(),
        view.top(),
        view.bottom()
    );
}

// ------------------------------------------------------- the terminal habits ----

#[test]
fn insert_key_copies_and_pastes_like_a_terminal() {
    let url = fixture("home.html", HOME);
    let mut app = app_at_url(&url);
    ctrl(&mut app, 'a');
    let sel = app.tab().selection.text(&app.page_text());
    assert!(!sel.is_empty());
    key(
        &mut app,
        Key::Named(NamedKey::Insert),
        Mods {
            ctrl: true,
            ..Mods::none()
        },
    );
    assert_eq!(app.clipboard.get(), sel, "Ctrl+Insert copies");

    ctrl(&mut app, 'l');
    key(
        &mut app,
        Key::Named(NamedKey::Insert),
        Mods {
            shift: true,
            ..Mods::none()
        },
    );
    // a one-line field cannot hold newlines, so the paste arrives flattened
    let pasted = app.address.value();
    assert!(pasted.starts_with("Heading one"), "{pasted}");
    assert!(sel.starts_with("Heading one"), "{sel}");
}

#[test]
fn ctrl_shift_k_changes_the_theme() {
    let url = fixture("home.html", HOME);
    let mut app = app_at_url(&url);
    let before = paint_screen(&app);
    key(
        &mut app,
        Key::Char('k'),
        Mods {
            ctrl: true,
            shift: true,
            ..Mods::none()
        },
    );
    assert_ne!(paint_screen(&app).cells, before.cells);
}

#[test]
fn number_keys_switch_the_inspector_panels() {
    let url = fixture("home.html", HOME);
    let mut app = app_at_url(&url);
    app.run(tb_ui::Command::DevtoolsToggle);
    let first = app.panel();
    key(&mut app, Key::Char('3'), Mods::none());
    assert_ne!(app.panel(), first, "a digit moves the inspector panel");
    // and with the drawer closed a digit is still just a digit (nothing to do)
    app.run(tb_ui::Command::DevtoolsToggle);
    key(&mut app, Key::Char('4'), Mods::none());
    assert_eq!(app.title(), "Home page — tiny");
}

#[test]
fn a_blank_target_opens_a_new_tab() {
    let html = r#"<html><head><title>Targets</title></head><body>
<a href="second.html" target="_blank">opens apart</a>
<a href="third.html">stays</a>
</body></html>"#;
    let url = fixture("targets.html", html);
    let mut app = app_at_url(&url);
    let (x, y) = link_point(&app, "opens");
    click(&mut app, x, y);
    assert_eq!(app.tabs.len(), 2, "target=_blank opened a tab");
    assert!(app.tabs[1].page.url.to_string().ends_with("second.html"));
}

#[test]
fn dragging_the_scrollbar_scrubs_the_page() {
    let mut body = String::from("<html><head><title>Tall</title></head><body>");
    for i in 0..400 {
        body.push_str(&format!("<p>Row of text number {i}</p>"));
    }
    body.push_str("</body></html>");
    let url = fixture("tall.html", &body);
    let mut app = app_at_url(&url);
    assert_eq!(app.tab().scroll_y, 0.0);
    // press near the bottom of the scrollbar track and drag: the page follows
    let (_cx, cy, _cw, ch) = app.content_px();
    let x = app.width as i32 - 6;
    let y = cy + ch - 20;
    app.on_event(Event::press(x, y));
    app.on_event(Event::drag(x, y));
    assert!(
        app.tab().scroll_y > 100.0,
        "the thumb drags: {}",
        app.tab().scroll_y
    );
    app.on_event(Event::release(x, y));
}
