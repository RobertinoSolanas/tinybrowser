//! Turning a `Screen` into escape sequences, and escape sequences back into
//! input events.
//!
//! Both halves live here because they are the two ends of the same contract,
//! and both are pure: they take bytes or cells and produce a string or events,
//! so the whole terminal protocol can be tested without a terminal.

use tb_ui::{Button, Event, Key, Mods, NamedKey, PointerKind, Screen, Style};

/// Write the control sequences that put the terminal into the mode the browser
/// wants: alternate screen, no echo, mouse with SGR coordinates, bracketed
/// paste, and a geometry query.
pub fn enter_mode(out: &mut String) {
    out.push_str("\x1b[?1049h"); // alternate screen
    out.push_str("\x1b[?25l"); // hide the cursor until the app asks for it
    out.push_str("\x1b[0m"); // clean attributes
    out.push_str("\x1b[2J"); // and a clean screen
    out.push_str("\x1b[?1000h"); // click reporting
    out.push_str("\x1b[?1002h"); // …and drag reporting (text selection)
    out.push_str("\x1b[?1006h"); // SGR coordinates (`CSI < b ; x ; y M`)
    out.push_str("\x1b[?2004h"); // bracketed paste
    out.push_str("\x1b[>1u"); // ask for the kitty keyboard protocol
    out.push_str("\x1b[?u"); // …and ask whether we got it
    query_size(out);
}

/// Leave the mode: everything back the way it was.
pub fn leave_mode(out: &mut String) {
    out.push_str("\x1b[<u"); // kitty pop
    out.push_str("\x1b[?2004l");
    out.push_str("\x1b[?1006l");
    out.push_str("\x1b[?1002l");
    out.push_str("\x1b[?1000l");
    out.push_str("\x1b[0m");
    out.push_str("\x1b[?25h");
    out.push_str("\x1b[?1049l"); // leave the alternate screen
}

/// Ask the terminal for its size (`CSI 18 t` → `CSI 8 ; rows ; cols t`).
pub fn query_size(out: &mut String) {
    out.push_str("\x1b[18t");
}

/// Set the window title.
pub fn set_title(out: &mut String, title: &str) {
    out.push_str("\x1b]0;");
    for ch in title.chars() {
        // a bell or an ESC inside the title would terminate the sequence
        if ch != '\u{7}' && ch != '\x1b' {
            out.push(ch);
        }
    }
    out.push('\u{7}');
}

/// Copy to the system clipboard with OSC 52 (works over ssh, and in any
/// terminal that allows it; ignored silently elsewhere).
pub fn clipboard_set(out: &mut String, base64: &str) {
    out.push_str(&format!("\x1b]52;c;{base64}\x07"));
}

/// The cursor: shown at a cell, or hidden.
pub fn set_cursor(out: &mut String, cell: Option<(usize, usize)>) {
    match cell {
        Some((x, y)) => {
            out.push_str(&format!("\x1b[{};{}H", y + 1, x + 1));
            out.push_str("\x1b[?25h");
        }
        None => out.push_str("\x1b[?25l"),
    }
}

/// A running diff writer: remembers where the cursor is and what the terminal
/// currently looks like, so it only emits what changed.
#[derive(Debug, Default)]
pub struct Painter {
    /// Column of the terminal cursor.
    x: usize,
    /// Row of the terminal cursor.
    y: usize,
    /// Attributes currently in effect.
    style: Option<Style>,
    /// The last frame, to diff against.
    prev: Option<Screen>,
}

impl Painter {
    pub fn new() -> Painter {
        Painter::default()
    }

    /// Append the escape sequences that turn the terminal from the previous
    /// frame into `screen`.
    pub fn paint(&mut self, screen: &Screen, out: &mut String) {
        // The first frame (or a resize) is written in full.
        let fresh = self
            .prev
            .as_ref()
            .map(|p| p.cols != screen.cols || p.rows != screen.rows)
            .unwrap_or(true);
        if fresh {
            out.push_str("\x1b[2J");
            self.x = 0;
            self.y = 0;
            self.style = None;
        }

        for y in 0..screen.rows {
            let row_from = y * screen.cols;
            let row = &screen.cells[row_from..row_from + screen.cols];
            let start = match self.prev.as_ref() {
                Some(prev) if !fresh && prev.cols == screen.cols => {
                    let prev_row = &prev.cells[row_from..row_from + screen.cols];
                    match row
                        .iter()
                        .zip(prev_row.iter())
                        .position(|(a, b)| a.ch != b.ch || a.style != b.style)
                    {
                        // nothing at all changed on this row
                        None => continue,
                        Some(i) => i,
                    }
                }
                _ => 0,
            };
            let end = match self.prev.as_ref() {
                Some(prev) if !fresh && prev.cols == screen.cols => {
                    let prev_row = &prev.cells[row_from..row_from + screen.cols];
                    let last = row
                        .iter()
                        .zip(prev_row.iter())
                        .rposition(|(a, b)| a.ch != b.ch || a.style != b.style)
                        .unwrap_or(start);
                    last + 1
                }
                _ => row.len(),
            };

            self.move_to(start, y, out);
            for cell in &row[start..end] {
                self.write_style(cell.style, out);
                out.push(cell.ch);
            }
            // Leave the attributes off the trailing edge so a partial frame
            // cannot leak colour into the next one.
            self.style = None;
            out.push_str("\x1b[0m");
            self.x = 0;
            self.y = y + 1;
        }
        self.prev = Some(screen.clone());
    }

    fn move_to(&mut self, x: usize, y: usize, out: &mut String) {
        if self.x != x || self.y != y {
            out.push_str(&format!("\x1b[{};{}H", y + 1, x + 1));
        }
        self.x = x;
        self.y = y;
    }

    fn write_style(&mut self, st: Style, out: &mut String) {
        if self.style == Some(st) {
            return;
        }
        let old = self.style.unwrap_or_default();
        // The cheap, common case first: a full reset.
        if st == Style::default() {
            out.push_str("\x1b[0m");
            self.style = Some(st);
            return;
        }
        out.push_str("\x1b[0m");
        if st.bold {
            out.push_str("\x1b[1m");
        }
        if st.underline {
            out.push_str("\x1b[4m");
        }
        if st.inverse {
            out.push_str("\x1b[7m");
        }
        if st.fg != old.fg || self.style.is_none() {
            out.push_str(&format!("\x1b[38;2;{};{};{}m", st.fg.r, st.fg.g, st.fg.b));
        }
        if st.bg != old.bg || self.style.is_none() {
            out.push_str(&format!("\x1b[48;2;{};{};{}m", st.bg.r, st.bg.g, st.bg.b));
        }
        self.style = Some(st);
    }
}

// --------------------------------------------------------------------- input ----

/// What a chunk of terminal output turned into.
#[derive(Debug, Default, Clone)]
pub struct Decoded {
    pub events: Vec<Event>,
    /// The terminal answered our geometry query.
    pub size: Option<(usize, usize)>,
    /// The kitty keyboard protocol flags the terminal reported.
    pub kitty: Option<u32>,
}

/// The escape-sequence decoder: bytes in, events out. Incomplete sequences are
/// kept until the rest arrives, which is what happens when a terminal splits a
/// sequence across two reads.
#[derive(Debug, Default)]
pub struct Decoder {
    state: State,
    params: Vec<i32>,
    cur: i32,
    sgr_mouse: bool,
    priv_seq: bool,
    final_byte: u8,
    utf8: Vec<u8>,
    utf8_want: usize,
    /// Text between `CSI 200 ~` and `CSI 201 ~`.
    paste: Option<String>,
    /// The terminal speaks the kitty keyboard protocol.
    pub kitty: bool,
}

#[derive(Debug, Default, PartialEq)]
enum State {
    #[default]
    Text,
    Esc,
    Csi,
    Ss3,
    Osc,
}

impl Decoder {
    pub fn new() -> Decoder {
        Decoder::default()
    }

    /// Feed raw bytes read from the terminal.
    pub fn feed(&mut self, bytes: &[u8], out: &mut Decoded) {
        for &b in bytes {
            self.byte(b, out);
        }
    }

    fn byte(&mut self, b: u8, out: &mut Decoded) {
        match self.state {
            State::Text => self.text_byte(b, out),
            State::Esc => {
                if b == b'[' {
                    self.begin_csi();
                } else if b == b']' {
                    self.state = State::Osc;
                } else if b == b'O' {
                    self.state = State::Ss3;
                } else if b == 0x1b {
                    out.events.push(Event::Key {
                        key: Key::Named(NamedKey::Escape),
                        mods: Mods::none(),
                    });
                    self.state = State::Text;
                } else {
                    // `ESC x` is Alt+x
                    let (key, mut mods) = key_from_byte(b);
                    mods.alt = true;
                    if let Some(text) = self.paste.as_mut() {
                        text.push('\x1b');
                        if let Key::Char(c) = key {
                            text.push(c);
                        }
                        self.state = State::Text;
                        return;
                    }
                    out.events.push(Event::Key { key, mods });
                    self.state = State::Text;
                }
            }
            State::Ss3 => {
                let key = match b {
                    b'A' => NamedKey::Up,
                    b'B' => NamedKey::Down,
                    b'C' => NamedKey::Right,
                    b'D' => NamedKey::Left,
                    b'P' => NamedKey::F(1),
                    b'Q' => NamedKey::F(2),
                    b'R' => NamedKey::F(3),
                    b'S' => NamedKey::F(4),
                    b'F' => NamedKey::End,
                    b'H' => NamedKey::Home,
                    _ => NamedKey::Escape,
                };
                self.push_key(Key::Named(key), Mods::none(), out);
                self.state = State::Text;
            }
            State::Csi => self.csi_byte(b, out),
            // OSC is how the terminal talks back to us; nothing we ask for uses
            // it beyond the title, so swallow it up to the terminator
            State::Osc => {
                if b == 0x07 {
                    self.state = State::Text;
                } else if b == 0x1b {
                    self.state = State::Esc;
                }
            }
        }
    }

    fn begin_csi(&mut self) {
        self.state = State::Csi;
        self.params.clear();
        self.cur = 0;
        self.sgr_mouse = false;
        self.priv_seq = false;
        self.final_byte = 0;
    }

    fn csi_byte(&mut self, b: u8, out: &mut Decoded) {
        match b {
            b'0'..=b'9' => self.cur = self.cur.saturating_mul(10) + (b - b'0') as i32,
            b';' => {
                self.params.push(self.cur);
                self.cur = 0;
            }
            b'<' => self.sgr_mouse = true,
            b'?' => self.priv_seq = true,
            // intermediate bytes (`$`, `!`, space): keep collecting
            0x20..=0x2f => {}
            0x40..=0x7e => {
                self.params.push(self.cur);
                self.final_byte = b;
                self.dispatch(b, out);
                self.state = State::Text;
            }
            _ => self.state = State::Text,
        }
    }

    fn param(&self, i: usize) -> i32 {
        self.params.get(i).copied().unwrap_or(0)
    }

    fn dispatch(&mut self, fin: u8, out: &mut Decoded) {
        match fin {
            b'M' | b'm' if self.sgr_mouse => {
                let press = fin == b'M';
                if let Some(ev) = mouse_event(self.param(0), self.param(1), self.param(2), press) {
                    out.events.push(ev);
                }
            }
            b'R' => {
                // a cursor-position report: it carries the size just as well
                out.size = Some((self.param(1).max(1) as usize, self.param(0).max(1) as usize));
            }
            b't' if self.param(0) == 8 => {
                out.size = Some((self.param(2).max(1) as usize, self.param(1).max(1) as usize));
            }
            b'u' if self.priv_seq => {
                out.kitty = Some(self.param(0) as u32);
                self.kitty = true;
            }
            b'u' => {
                // kitty CSI-u: `CSI codepoint ; mods [:event-type] u`
                let cp = self.param(0) as u32;
                let mods = kitty_mods(self.param(1));
                let kind = if self.params.len() > 2 {
                    self.param(2)
                } else {
                    1
                };
                if kind != 3 {
                    // ignore key *release*: the app acts on press
                    out.events.push(Event::Key {
                        key: csi_u_key(cp, mods),
                        mods,
                    });
                }
            }
            b'~' => {
                let code = self.param(0);
                match code {
                    // bracketed paste
                    200 => {
                        self.paste = Some(String::new());
                        return;
                    }
                    201 => {
                        if let Some(text) = self.paste.take() {
                            out.events.push(Event::Paste(text));
                        }
                        return;
                    }
                    _ => {}
                }
                let (key, mods) = (named_key(code), csi_mods(self.param(1)));
                out.events.push(Event::Key { key, mods });
            }
            b'A'..=b'F' | b'H' => {
                let key = match fin {
                    b'A' => NamedKey::Up,
                    b'B' => NamedKey::Down,
                    b'C' => NamedKey::Right,
                    b'D' => NamedKey::Left,
                    b'H' => NamedKey::Home,
                    b'F' => NamedKey::End,
                    _ => NamedKey::Escape,
                };
                out.events.push(Event::Key {
                    key: Key::Named(key),
                    mods: csi_mods(self.param(1)),
                });
            }
            // `ESC [ Z` is Shift+Tab
            b'Z' => out.events.push(Event::Key {
                key: Key::Named(NamedKey::Tab),
                mods: Mods {
                    shift: true,
                    ..Mods::none()
                },
            }),
            _ => {}
        }
    }

    fn text_byte(&mut self, b: u8, out: &mut Decoded) {
        if b == 0x1b {
            self.utf8.clear();
            self.utf8_want = 0;
            // the terminator of a paste is an escape sequence too
            self.state = State::Esc;
            return;
        }
        if self.paste.is_some() {
            // inside a paste bracket everything is text, even control codes
            if let Some(text) = self.paste.as_mut() {
                push_byte(text, b, &mut self.utf8, &mut self.utf8_want);
            }
            return;
        }
        if b & 0x80 == 0 {
            let (key, mods) = key_from_byte(b);
            out.events.push(Event::Key { key, mods });
            return;
        }
        // UTF-8: wait for the whole sequence before emitting a character
        self.utf8.push(b);
        if self.utf8_want == 0 {
            self.utf8_want = utf8_len(b);
        }
        if self.utf8.len() >= self.utf8_want {
            if let Ok(s) = std::str::from_utf8(&self.utf8) {
                if let Some(c) = s.chars().next() {
                    if !c.is_control() {
                        out.events.push(Event::Key {
                            key: Key::Char(c),
                            mods: Mods::none(),
                        });
                    }
                }
            }
            self.utf8.clear();
            self.utf8_want = 0;
        }
    }

    /// Keys found while a paste is in flight still need to reach the app when
    /// the terminal does not do bracketed paste (it will not be bracketed).
    fn push_key(&mut self, key: Key, mods: Mods, out: &mut Decoded) {
        if let Some(text) = self.paste.as_mut() {
            if let Key::Char(c) = key {
                text.push(c);
                return;
            }
        }
        out.events.push(Event::Key { key, mods });
    }
}

/// Append a byte to a paste buffer, decoding UTF-8 as it goes.
fn push_byte(text: &mut String, b: u8, buf: &mut Vec<u8>, want: &mut usize) {
    if b & 0x80 == 0 {
        text.push(b as char);
        return;
    }
    buf.push(b);
    if *want == 0 {
        *want = utf8_len(b);
    }
    if buf.len() >= *want {
        if let Ok(s) = std::str::from_utf8(buf) {
            text.push_str(s);
        }
        buf.clear();
        *want = 0;
    }
}

/// How many bytes a UTF-8 sequence starting with `b` has.
fn utf8_len(b: u8) -> usize {
    if b & 0xE0 == 0xC0 {
        2
    } else if b & 0xF0 == 0xE0 {
        3
    } else if b & 0xF8 == 0xF0 {
        4
    } else {
        1
    }
}

/// Decode a single non-escape byte into a key and its modifiers.
pub fn key_from_byte(b: u8) -> (Key, Mods) {
    let mods = Mods::none();
    match b {
        0x0d | 0x0a => (Key::Named(NamedKey::Enter), mods),
        0x09 => (Key::Named(NamedKey::Tab), mods),
        0x7f | 0x08 => (Key::Named(NamedKey::Backspace), mods),
        0x00 => (Key::Char(' '), mods),
        0x01..=0x1a => (
            Key::Char((b'`' + b) as char), // 1 → 'a'
            Mods {
                ctrl: true,
                ..Mods::none()
            },
        ),
        _ if (b as char).is_control() => (Key::Named(NamedKey::Escape), mods),
        _ => (Key::Char(b as char), mods),
    }
}

/// `CSI 1 ; mods` where 2 = shift, 3 = alt, 5 = ctrl, 9 = super.
fn csi_mods(n: i32) -> Mods {
    let n = (n - 1).max(0);
    Mods {
        shift: n & 1 != 0,
        alt: n & 2 != 0,
        ctrl: n & 4 != 0,
        meta: n & 8 != 0,
    }
}

/// Kitty's modifier bits: 1 shift, 2 alt, 4 ctrl, 8 super, 16 hyper, 32 meta.
fn kitty_mods(n: i32) -> Mods {
    let n = (n - 1).max(0);
    Mods {
        shift: n & 1 != 0,
        alt: n & 2 != 0,
        ctrl: n & 4 != 0,
        meta: n & 8 != 0 || n & 32 != 0,
    }
}

/// Named keys that arrive as `CSI n ~`.
fn named_key(code: i32) -> Key {
    let named = match code {
        1 | 7 => NamedKey::Home,
        2 => NamedKey::Insert,
        3 => NamedKey::Delete,
        4 | 8 => NamedKey::End,
        5 => NamedKey::PageUp,
        6 => NamedKey::PageDown,
        11 => NamedKey::F(1),
        12 => NamedKey::F(2),
        13 => NamedKey::F(3),
        14 => NamedKey::F(4),
        15 => NamedKey::F(5),
        17..=21 => NamedKey::F((code - 11) as u8),
        23..=26 => NamedKey::F((code - 12) as u8),
        27 => NamedKey::Escape,
        _ => NamedKey::Escape,
    };
    Key::Named(named)
}

/// A kitty CSI-u code point becomes a key: control code points keep their
/// legacy meaning, printable ones are characters, and the functional-key block
/// (57344 upwards) maps to our named keys.
fn csi_u_key(cp: u32, mods: Mods) -> Key {
    match cp {
        0x0d | 0x0a => Key::Named(NamedKey::Enter),
        0x09 => Key::Named(NamedKey::Tab),
        0x7f => Key::Named(NamedKey::Backspace),
        0x1b => Key::Named(NamedKey::Escape),
        0x20 => Key::Char(' '),
        57344..=57433 => Key::Named(functional_key(cp)),
        _ if cp < 0x20 => Key::Named(NamedKey::Escape),
        _ => match char::from_u32(cp) {
            Some(c) => {
                // in kitty `Ctrl+I` is reported as the literal letter, which is
                // what our keymap wants (`Ctrl+i`), not a Tab
                if mods.ctrl && c.is_ascii_uppercase() {
                    Key::Char(c.to_ascii_lowercase())
                } else {
                    Key::Char(c)
                }
            }
            None => Key::Named(NamedKey::Escape),
        },
    }
}

fn functional_key(cp: u32) -> NamedKey {
    match cp {
        57344 => NamedKey::Escape,
        57345 => NamedKey::Enter,
        57346 => NamedKey::Tab,
        57347 => NamedKey::Backspace,
        57348 => NamedKey::Insert,
        57349 => NamedKey::Delete,
        57350 => NamedKey::Left,
        57351 => NamedKey::Right,
        57352 => NamedKey::Up,
        57353 => NamedKey::Down,
        57354 => NamedKey::PageUp,
        57355 => NamedKey::PageDown,
        57356 => NamedKey::Home,
        57357 => NamedKey::End,
        57373 => NamedKey::F(1),
        57374 => NamedKey::F(2),
        57375 => NamedKey::F(3),
        57376 => NamedKey::F(4),
        57377 => NamedKey::F(5),
        57378 => NamedKey::F(6),
        57379 => NamedKey::F(7),
        57380 => NamedKey::F(8),
        57381 => NamedKey::F(9),
        57382 => NamedKey::F(10),
        57383 => NamedKey::F(11),
        57384 => NamedKey::F(12),
        _ => NamedKey::Escape,
    }
}

/// SGR mouse reporting: `CSI < button ; x ; y M`. Bit 32 is "a move", bits 64
/// and 65 are the wheel, bit 128 is a release.
fn mouse_event(flags: i32, x: i32, y: i32, press: bool) -> Option<Event> {
    let x = (x.max(1) as usize) - 1;
    let y = (y.max(1) as usize) - 1;
    let mods = Mods {
        shift: flags & 4 != 0,
        alt: flags & 8 != 0,
        ctrl: flags & 16 != 0,
        meta: false,
    };
    let button = match flags & 0b11 {
        0 => Button::Left,
        1 => Button::Middle,
        2 => Button::Right,
        _ => Button::Left,
    };
    if flags & 64 != 0 {
        // the wheel: button 64 up, 65 down — one notch is three lines
        let dy = if flags & 1 == 0 { -3 } else { 3 };
        return Some(Event::Pointer {
            kind: PointerKind::Wheel,
            button: None,
            x: x as i32,
            y: y as i32,
            dx: 0,
            dy,
            mods,
        });
    }
    if flags & 32 != 0 {
        // motion; button 3 means "no button held", otherwise it is a drag
        let kind = if flags & 128 != 0 {
            PointerKind::Release
        } else if flags & 0b11 == 3 {
            PointerKind::Move
        } else {
            PointerKind::Drag
        };
        return Some(Event::Pointer {
            kind,
            button: Some(button),
            x: x as i32,
            y: y as i32,
            dx: 0,
            dy: 0,
            mods,
        });
    }
    let kind = if flags & 128 != 0 || !press {
        PointerKind::Release
    } else {
        PointerKind::Press
    };
    Some(Event::Pointer {
        kind,
        button: Some(button),
        x: x as i32,
        y: y as i32,
        dx: 0,
        dy: 0,
        mods,
    })
}
