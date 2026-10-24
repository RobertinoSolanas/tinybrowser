//! The input modes of the shell.
//!
//! Like `vim`, and unlike a mouse-only browser, the shell knows *what has the
//! keyboard*: the page, the address bar, the find bar, the command line, a hint
//! sequence, an overlay. The mode decides which keys are text and which are
//! commands, which is exactly the question a browser answers with focus.

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Default)]
pub enum Mode {
    /// The page has the keyboard (or nothing does).
    #[default]
    Normal,
    /// The address bar is open and editing.
    Address,
    /// `Ctrl+F` bar.
    Find,
    /// `:` command line.
    Command,
    /// The inspector's console prompt (`Ctrl+J`).
    Console,
    /// Typing a hint label (`f`, `F`).
    Hints,
    /// Help overlay (`F1`, `?`).
    Help,
    /// Context or hamburger menu open.
    Menu,
    /// Inspector: the next click picks an element (`Ctrl+Shift+C`).
    Pick,
    /// A `<select>` popup is open.
    Select,
}

impl Mode {
    /// Keys in this mode are text for a field, not commands.
    pub fn is_text(&self) -> bool {
        matches!(
            self,
            Mode::Address | Mode::Find | Mode::Command | Mode::Console
        )
    }

    /// An overlay that `Esc` closes and that swallows everything else.
    pub fn is_overlay(&self) -> bool {
        matches!(
            self,
            Mode::Help | Mode::Menu | Mode::Hints | Mode::Select | Mode::Pick
        )
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Mode::Normal => "normal",
            Mode::Address => "address",
            Mode::Find => "find",
            Mode::Command => "command",
            Mode::Console => "console",
            Mode::Hints => "hints",
            Mode::Help => "help",
            Mode::Menu => "menu",
            Mode::Pick => "pick",
            Mode::Select => "select",
        }
    }
}

/// Who has the caret: the chrome (which widget) or the page.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Caret {
    Page,
    Widget(crate::chrome::Widget),
}
