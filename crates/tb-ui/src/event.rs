//! Input events, device independent.
//!
//! A terminal reports `ESC [< 0 ; 41 ; 12 M`, X11 reports a `ButtonPress` with
//! a keycode and a state mask, a test calls `Event::key(...)`. All of them
//! become the same [`Event`], so `tb-ui` — and every test of it — talks about
//! *what the user did*, never about *what the device sent*.
//!
//! Coordinates are **UI pixels**: the cell backends define a cell as
//! [`crate::chrome::CELL_W`] × [`crate::chrome::CELL_H`] UI px, the pixel
//! backends use their real window pixels. One coordinate space for hit testing,
//! which is what makes a click testable without a window server.

use std::fmt;

/// Which modifier keys were down.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub meta: bool,
}

impl Mods {
    pub const fn none() -> Mods {
        Mods {
            ctrl: false,
            alt: false,
            shift: false,
            meta: false,
        }
    }

    /// Any of the "command" modifiers — with one of them a printable key is a
    /// shortcut, not text.
    pub fn any(&self) -> bool {
        self.ctrl || self.alt || self.meta
    }

    pub const fn ctrl() -> Mods {
        Mods {
            ctrl: true,
            alt: false,
            shift: false,
            meta: false,
        }
    }

    pub const fn shift() -> Mods {
        Mods {
            ctrl: false,
            alt: false,
            shift: true,
            meta: false,
        }
    }

    pub const fn alt() -> Mods {
        Mods {
            ctrl: false,
            alt: true,
            shift: false,
            meta: false,
        }
    }

    /// SGR mouse bits (`ESC [ < b ; …`): 2 alt, 4 ctrl, the shift bit comes with
    /// the other fields and is passed in separately by the caller.
    pub fn from_sgr(bits: u8, shift: bool) -> Mods {
        Mods {
            ctrl: bits & 16 != 0,
            alt: bits & 8 != 0,
            shift,
            meta: false,
        }
    }

    pub fn to_page(self) -> tb_browser::Modifiers {
        tb_browser::Modifiers {
            ctrl: self.ctrl,
            shift: self.shift,
            alt: self.alt,
            meta: self.meta,
        }
    }
}

/// Keys that are not a character.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum NamedKey {
    Enter,
    Tab,
    Escape,
    Backspace,
    Delete,
    Insert,
    Home,
    End,
    PageUp,
    PageDown,
    Up,
    Down,
    Left,
    Right,
    /// `F1`…`F12`.
    F(u8),
}

/// A key as the user experiences it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Key {
    /// A printable character (`a`, `ä`, `/`, ` `).
    Char(char),
    Named(NamedKey),
}

impl Key {
    /// The `key` value a browser would report.
    pub fn js_key(&self) -> String {
        match self {
            Key::Char(' ') => " ".to_string(),
            Key::Char(c) => c.to_string(),
            Key::Named(n) => match n {
                NamedKey::Enter => "Enter",
                NamedKey::Tab => "Tab",
                NamedKey::Escape => "Escape",
                NamedKey::Backspace => "Backspace",
                NamedKey::Delete => "Delete",
                NamedKey::Insert => "Insert",
                NamedKey::Home => "Home",
                NamedKey::End => "End",
                NamedKey::PageUp => "PageUp",
                NamedKey::PageDown => "PageDown",
                NamedKey::Up => "ArrowUp",
                NamedKey::Down => "ArrowDown",
                NamedKey::Left => "ArrowLeft",
                NamedKey::Right => "ArrowRight",
                NamedKey::F(n) => return format!("F{n}"),
            }
            .to_string(),
        }
    }

    /// The `code` value (physical key, layout independent — as close as a
    /// terminal and X11 both get).
    pub fn js_code(&self) -> String {
        match self {
            Key::Char(c) => match c {
                'a'..='z' => format!("Key{}", c.to_uppercase()),
                'A'..='Z' => format!("Key{c}"),
                '0'..='9' => format!("Digit{c}"),
                ' ' => "Space".into(),
                _ => "Minus".into(),
            },
            Key::Named(n) => match n {
                NamedKey::Enter => "Enter",
                NamedKey::Tab => "Tab",
                NamedKey::Escape => "Escape",
                NamedKey::Backspace => "Backspace",
                NamedKey::Delete => "Delete",
                NamedKey::Insert => "Insert",
                NamedKey::Home => "Home",
                NamedKey::End => "End",
                NamedKey::PageUp => "PageUp",
                NamedKey::PageDown => "PageDown",
                NamedKey::Up => "ArrowUp",
                NamedKey::Down => "ArrowDown",
                NamedKey::Left => "ArrowLeft",
                NamedKey::Right => "ArrowRight",
                NamedKey::F(_) => return "Fn".into(),
            }
            .to_string(),
        }
    }

    pub fn is_printable(&self) -> bool {
        matches!(self, Key::Char(c) if !c.is_control())
    }

    /// What the help overlay and menus print.
    pub fn label(&self) -> String {
        match self {
            Key::Char(' ') => "Space".into(),
            Key::Char(c) => c.to_string(),
            Key::Named(NamedKey::PageUp) => "PgUp".into(),
            Key::Named(NamedKey::PageDown) => "PgDn".into(),
            Key::Named(NamedKey::Backspace) => "Bksp".into(),
            Key::Named(n) => {
                let s = format!("{n:?}");
                s
            }
        }
    }
}

/// A mouse button.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Button {
    Left,
    Middle,
    Right,
}

/// What the pointer did.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum PointerKind {
    Press,
    Release,
    /// No button down.
    Move,
    /// A button is down and the pointer moved (drag).
    Drag,
    Wheel,
    /// Terminals do not report this; we synthesise it from two presses in the
    /// same cell within the double-click time.
    DoubleClick,
}

/// One thing the user (or a test) did.
#[derive(Clone, PartialEq, Debug)]
pub enum Event {
    Key {
        key: Key,
        mods: Mods,
    },
    Pointer {
        kind: PointerKind,
        /// `None` for a plain move or a wheel.
        button: Option<Button>,
        x: i32,
        y: i32,
        /// Wheel steps (`dy` > 0 is down; X11 buttons 4/5, terminal wheel).
        dx: i32,
        dy: i32,
        mods: Mods,
    },
    /// The display changed size (UI px).
    Resize {
        width: u32,
        height: u32,
    },
    /// Bracketed paste / `Ctrl+V` into a text field.
    Paste(String),
    /// Idle timer — the app pumps page timers and repaints if something moved.
    Tick,
    Quit,
}

impl Event {
    pub fn key(key: Key, mods: Mods) -> Event {
        Event::Key { key, mods }
    }

    pub fn char(c: char) -> Event {
        Event::Key {
            key: Key::Char(c),
            mods: Mods::none(),
        }
    }

    pub fn named(n: NamedKey) -> Event {
        Event::Key {
            key: Key::Named(n),
            mods: Mods::none(),
        }
    }

    pub fn press(x: i32, y: i32) -> Event {
        Event::Pointer {
            kind: PointerKind::Press,
            button: Some(Button::Left),
            x,
            y,
            dx: 0,
            dy: 0,
            mods: Mods::none(),
        }
    }

    pub fn release(x: i32, y: i32) -> Event {
        Event::Pointer {
            kind: PointerKind::Release,
            button: Some(Button::Left),
            x,
            y,
            dx: 0,
            dy: 0,
            mods: Mods::none(),
        }
    }

    /// A pointer move with the left button held down (a drag).
    pub fn drag(x: i32, y: i32) -> Event {
        Event::Pointer {
            kind: PointerKind::Drag,
            button: Some(Button::Left),
            x,
            y,
            dx: 0,
            dy: 0,
            mods: Mods::none(),
        }
    }

    pub fn move_to(x: i32, y: i32) -> Event {
        Event::Pointer {
            kind: PointerKind::Move,
            button: None,
            x,
            y,
            dx: 0,
            dy: 0,
            mods: Mods::none(),
        }
    }

    pub fn wheel(x: i32, y: i32, dy: i32) -> Event {
        Event::Pointer {
            kind: PointerKind::Wheel,
            button: None,
            x,
            y,
            dx: 0,
            dy,
            mods: Mods::none(),
        }
    }

    /// A pointer event with no motion — the shape a button press takes.
    pub fn pointer(kind: PointerKind, button: Option<Button>, x: i32, y: i32) -> Event {
        Event::Pointer {
            kind,
            button,
            x,
            y,
            dx: 0,
            dy: 0,
            mods: Mods::none(),
        }
    }

    /// The same press with modifiers held (ctrl+click, shift+click).
    pub fn pointer_with(
        kind: PointerKind,
        button: Option<Button>,
        x: i32,
        y: i32,
        mods: Mods,
    ) -> Event {
        Event::Pointer {
            kind,
            button,
            x,
            y,
            dx: 0,
            dy: 0,
            mods,
        }
    }

    pub fn text(s: &str) -> Vec<Event> {
        s.chars().map(Event::char).collect()
    }
}

/// `Ctrl+Shift+L` — the way a key reference and the tests name a shortcut.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct Shortcut {
    pub key: Key,
    pub mods: Mods,
}

impl fmt::Display for Shortcut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> fmt::Result {
        if self.mods.ctrl {
            f.write_str("Ctrl+")?;
        }
        if self.mods.alt {
            f.write_str("Alt+")?;
        }
        if self.mods.shift && !matches!(self.key, Key::Char(c) if c.is_uppercase()) {
            f.write_str("Shift+")?;
        }
        f.write_str(&self.key.label())
    }
}
