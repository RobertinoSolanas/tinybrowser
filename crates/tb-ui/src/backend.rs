//! The boundary between the shell and whatever it is drawn on.
//!
//! A backend has four jobs: tell the shell its size, take a picture, hand over
//! the next event, and (optionally) carry text to the system clipboard. That is
//! the whole contract — which is why the same `App` runs in a terminal, in an X11
//! window, and head-less in a test with nothing but a string grid.
//!
//! [`Headless`] is the test/demo backend: scripted events, `cols × rows` cells,
//! and a `Screen` you can assert on. `tiny --ui-script events.txt` uses exactly
//! this path, which is how the GUI is verifiable in CI without a display.

use crate::event::Event;
use crate::keymap::Command;
use crate::screen::Screen;
use crate::App;

/// What `tiny --ui-backend` chooses.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BackendKind {
    /// Our own X11 client (real window, real pixels).
    X11,
    /// The terminal (cells, SGR, our own escape-sequence parser).
    Term,
    /// No display at all — for tests and `--ui-script`.
    Headless,
}

impl BackendKind {
    pub fn as_str(self) -> &'static str {
        match self {
            BackendKind::X11 => "x11",
            BackendKind::Term => "term",
            BackendKind::Headless => "headless",
        }
    }
}

/// A display the shell can drive.
pub trait Backend {
    /// The kind, for messages and for the shell's rendering choice.
    fn kind(&self) -> BackendKind;

    /// Size in **UI px** (a cell backend reports `cols*CELL_W × rows*CELL_H`).
    fn size(&self) -> (u32, u32);

    /// Show a cell grid (every backend can do this).
    fn present(&mut self, screen: &Screen) -> std::io::Result<()>;

    /// Show real pixels. Only pixel backends implement it; the shell checks
    /// [`Backend::wants_pixels`] and keeps the cell path otherwise.
    fn present_frame(&mut self, fb: &tb_gfx::Framebuffer) -> std::io::Result<()> {
        let _ = fb;
        Ok(())
    }

    /// `true` → the shell renders the page with its rasterizer and calls
    /// [`Backend::present_frame`].
    fn wants_pixels(&self) -> bool {
        false
    }

    /// Wait for the next event, giving up after `timeout_ms` (`None` = poll).
    fn next_event(&mut self, timeout_ms: u64) -> Option<Event>;

    /// The window manager's title / the terminal's OSC 0.
    fn set_title(&mut self, title: &str) {
        let _ = title;
    }

    /// Offer text to the system clipboard (OSC-52, X11 selection, …).
    fn store_clipboard(&mut self, text: &str) {
        let _ = text;
    }

    /// Read the system clipboard, when the backend can.
    fn load_clipboard(&mut self) -> Option<String> {
        None
    }

    /// Put the display back the way it was (raw mode off, alt screen left).
    fn shutdown(&mut self) {}
}

/// A backend that draws nothing and is driven by a script — the test rig, and
/// the engine behind `tiny --ui-script`.
pub struct Headless {
    pub cols: usize,
    pub rows: usize,
    pub events: Vec<Event>,
    pub cursor: usize,
    pub last: Screen,
    pub frames: usize,
    pub titles: Vec<String>,
    pub clipboard: Option<String>,
    /// How long the last idle wait was, so `Tick` can pump
    /// the page's timers with the time that really passed.
    pub tick_ms: u64,
}

impl Headless {
    /// A `cols × rows` display with no events (every poll says "idle" until the
    /// app itself asks to quit).
    pub fn new(cols: usize, rows: usize) -> Headless {
        Headless {
            cols,
            rows,
            events: Vec::new(),
            cursor: 0,
            last: Screen::new(cols, rows),
            frames: 0,
            titles: Vec::new(),
            clipboard: None,
            tick_ms: 16,
        }
    }

    /// A display that replays `events` and then reports `Event::Quit`, so a
    /// scripted session always terminates.
    pub fn scripted(cols: usize, rows: usize, events: Vec<Event>) -> Headless {
        let mut h = Headless::new(cols, rows);
        h.events = events;
        h.events.push(Event::Quit);
        h
    }

    /// The text of the last presented screen.
    pub fn dump(&self) -> String {
        self.last.dump()
    }
}

impl Backend for Headless {
    fn kind(&self) -> BackendKind {
        BackendKind::Headless
    }

    fn size(&self) -> (u32, u32) {
        (
            self.cols as u32 * crate::chrome::CELL_W as u32,
            self.rows as u32 * crate::chrome::CELL_H as u32,
        )
    }

    fn present(&mut self, screen: &Screen) -> std::io::Result<()> {
        self.last = screen.clone();
        self.frames += 1;
        Ok(())
    }

    fn next_event(&mut self, _timeout_ms: u64) -> Option<Event> {
        if self.cursor < self.events.len() {
            let ev = self.events[self.cursor].clone();
            self.cursor += 1;
            return Some(ev);
        }
        // a scripted session is over; an interactive one would block here
        Some(Event::Quit)
    }

    fn set_title(&mut self, title: &str) {
        if self.titles.last().map(|t| t != title).unwrap_or(true) {
            self.titles.push(title.to_string());
        }
    }

    fn store_clipboard(&mut self, text: &str) {
        self.clipboard = Some(text.to_string());
    }

    fn load_clipboard(&mut self) -> Option<String> {
        self.clipboard.clone()
    }
}

/// Drive `app` against `backend` until it quits: repaint when dirty, wait for
/// events, pump the page's timers while idle.
///
/// This is the whole main loop, written once, so every backend gets identical
/// behaviour (and a test gets the same code path a human uses).
pub fn run(app: &mut App, backend: &mut dyn Backend) -> std::io::Result<()> {
    let (w, h) = backend.size();
    if w != app.width || h != app.height {
        app.width = w;
        app.height = h;
        app.recompute_grid();
    }
    let mut painted = std::time::Instant::now();
    loop {
        if app.quit {
            break;
        }
        if app.dirty {
            let screen = crate::render::paint_screen(app);
            if backend.wants_pixels() {
                let fb = crate::render::paint_frame(app);
                backend.present_frame(&fb)?;
            } else {
                backend.present(&screen)?;
            }
            backend.set_title(&app.title());
            app.dirty = false;
            painted = std::time::Instant::now();
        }
        let timeout = if app.tab().page.has_js() { 16 } else { 250 };
        if let Some(ev) = backend.next_event(timeout) {
            if matches!(ev, Event::Quit) {
                app.quit = true;
                break;
            }
            app.on_event(ev);
        } else if painted.elapsed().as_millis() > 200 {
            // idle: let the page's timers run with the time that really passed
            let ms = painted.elapsed().as_millis() as f64;
            app.pump(ms);
            painted = std::time::Instant::now();
        }
    }
    backend.shutdown();
    Ok(())
}

/// Parse `--ui-script` into events.
///
/// One event per line, in the tiny command language the CLI documents:
///
/// ```text
/// key ctrl+l            a shortcut
/// key F5
/// type https://example.com
/// enter
/// click 120 300         UI px
/// cell 40 3             click the centre of a cell
/// drag 100 300 400 300  press, move, release
/// wheel 0 200 3         three clicks down
/// move 400 400          hover
/// resize 1400 900
/// wait 250              milliseconds of idle (pumps timers)
/// title                 print the app title
/// dump                  print the screen
/// status                print the status line
/// ```
pub fn parse_script(text: &str) -> Vec<Event> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let verb = parts.next().unwrap_or("");
        let arg = |i: usize| -> Option<String> { parts.clone().nth(i).map(|s| s.to_string()) };
        match verb {
            "key" => {
                let name = arg(0).unwrap_or_default();
                let (key, mods) = crate::keys::parse_key_combo(&name);
                if let Some(k) = key {
                    out.push(Event::Key { key: k, mods });
                }
            }
            "type" => {
                // everything after the verb, verbatim (a URL has spaces sometimes)
                let rest = line["type".len()..].trim().to_string();
                for c in rest.chars() {
                    out.push(Event::Key {
                        key: crate::event::Key::Char(c),
                        mods: crate::event::Mods::none(),
                    });
                }
            }
            "paste" => {
                let rest = line["paste".len()..].trim().to_string();
                out.push(Event::Paste(rest));
            }
            "enter" => out.push(Event::named(crate::event::NamedKey::Enter)),
            "escape" | "esc" => out.push(Event::named(crate::event::NamedKey::Escape)),
            "tab" => out.push(Event::named(crate::event::NamedKey::Tab)),
            "click" => {
                if let (Some(x), Some(y)) = (arg(0), arg(1)) {
                    if let (Ok(x), Ok(y)) = (x.parse::<i32>(), y.parse::<i32>()) {
                        out.push(Event::press(x, y));
                        out.push(Event::release(x, y));
                    }
                }
            }
            "cell" => {
                if let (Some(c), Some(r)) = (arg(0), arg(1)) {
                    if let (Ok(c), Ok(r)) = (c.parse::<i32>(), r.parse::<i32>()) {
                        let x = c * crate::chrome::CELL_W + crate::chrome::CELL_W / 2;
                        let y = r * crate::chrome::CELL_H + crate::chrome::CELL_H / 2;
                        out.push(Event::press(x, y));
                        out.push(Event::release(x, y));
                    }
                }
            }
            "move" => {
                if let (Some(x), Some(y)) = (arg(0), arg(1)) {
                    if let (Ok(x), Ok(y)) = (x.parse::<i32>(), y.parse::<i32>()) {
                        out.push(Event::move_to(x, y));
                    }
                }
            }
            "drag" => {
                // drag X0 Y0 X1 Y1 — press at one point, come up at the other
                let n = |i| arg(i).and_then(|s| s.parse::<i32>().ok());
                if let (Some(x0), Some(y0), Some(x1), Some(y1)) = (n(0), n(1), n(2), n(3)) {
                    out.push(Event::press(x0, y0));
                    out.push(Event::drag(x1, y1));
                    out.push(Event::release(x1, y1));
                }
            }
            "wheel" => {
                let x = arg(0).and_then(|s| s.parse::<i32>().ok()).unwrap_or(200);
                let y = arg(1).and_then(|s| s.parse::<i32>().ok()).unwrap_or(200);
                let n = arg(2).and_then(|s| s.parse::<i32>().ok()).unwrap_or(1);
                out.push(Event::wheel(x, y, n));
            }
            "resize" => {
                if let (Some(w), Some(h)) = (arg(0), arg(1)) {
                    if let (Ok(w), Ok(h)) = (w.parse::<u32>(), h.parse::<u32>()) {
                        out.push(Event::Resize {
                            width: w,
                            height: h,
                        });
                    }
                }
            }
            "wait" => {
                let ms = arg(0).and_then(|s| s.parse::<u64>().ok()).unwrap_or(100);
                // a wait is a row of ticks: the app pumps its timers in between
                for _ in 0..(ms / 16).max(1) {
                    out.push(Event::Tick);
                }
            }
            _ => (), // print verbs (`dump`, `title`, …) are handled by the CLI
        }
    }
    out
}

/// `--ui-script`: the lines that are browser commands rather than input events.
/// Returns whether the line was one of them.
pub fn script_command(app: &mut App, line: &str) -> bool {
    let verb = line.split_whitespace().next().unwrap_or("");
    let rest = line[verb.len()..].trim();
    let cmd = match verb {
        "open" | "goto" | "go" | "navigate" if !rest.is_empty() => Command::Go(rest.to_string()),
        "back" => Command::Back,
        "forward" => Command::Forward,
        "reload" => Command::Reload,
        "home" => Command::Home,
        "newtab" | "new-tab" => Command::NewTab((!rest.is_empty()).then(|| rest.to_string())),
        "close" => Command::CloseTab,
        "tab" => rest
            .parse::<usize>()
            .map(Command::UseTab)
            .unwrap_or(Command::Nop),
        "zoom" => match rest {
            "in" | "+" => Command::Zoom(1),
            "out" | "-" => Command::Zoom(-1),
            _ => Command::Zoom(0),
        },
        "theme" => Command::ToggleTheme,
        _ => return false,
    };
    app.run(cmd);
    true
}

/// The verbs `--ui-script` understands for output.
pub fn script_output(app: &App, line: &str) -> Option<String> {
    let verb = line.split_whitespace().next().unwrap_or("");
    match verb {
        "dump" => Some(crate::render::paint_screen(app).dump()),
        "title" => Some(app.title()),
        "status" => Some(app.status_line().0),
        "url" => Some(app.url()),
        "text" => Some(app.page().text()),
        "links" => Some(
            app.page()
                .links()
                .iter()
                .take(40)
                .map(|l| format!("{} {}", l.text, l.url))
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        "problems" => Some(
            app.page()
                .problems()
                .all()
                .iter()
                .take(20)
                .map(|p| format!("[{:?} {}] {}", p.severity, p.phase, p.message))
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        "console" => Some(
            app.page()
                .console_lines()
                .iter()
                .map(|(l, t)| format!("[{l}] {t}"))
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        "tabcount" => Some(app.tabs.len().to_string()),
        _ => None,
    }
}
