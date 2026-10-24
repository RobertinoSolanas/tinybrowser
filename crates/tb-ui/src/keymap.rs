//! Keys are not commands.
//!
//! The app acts on [`Command`]s; a keymap turns `(mode, key, mods)` into one.
//! Two reasons, both practical: a test can drive the browser with `Command::Back`
//! without knowing that `Alt+Left` is bound to it (and keeps working when the
//! binding changes), and the help overlay prints *this table*, so the documented
//! keys and the real keys cannot drift apart.

use crate::event::{Key, Mods, NamedKey};
use crate::mode::Mode;

/// Everything the shell can do, from anywhere.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// Do nothing (an unbound key must not beep, it must do nothing).
    Nop,
    /// Navigate to the address bar's current text.
    GoAddress,
    /// Open a URL directly.
    Go(String),
    Reload,
    /// Reload ignoring the HTTP cache.
    ReloadHard,
    Back,
    Forward,
    Home,
    ViewSource,
    NewTab(Option<String>),
    CloseTab,
    /// `Ctrl+Shift+T`: the tab we just closed.
    RestoreTab,
    UseTab(usize),
    NextTab,
    PrevTab,
    AddressBar,
    FindBar,
    FindNext,
    FindPrev,
    /// `Esc`: close whichever overlay is open.
    CloseBar,
    CommandBar,
    /// Link hints; the flag says "open in a new tab".
    Hints(bool),
    /// A hint label was completed.
    FollowHint(String, bool),
    DevtoolsToggle,
    DevtoolsPanel(usize),
    /// Inspector: pick the next clicked element.
    PickElement,
    /// Scroll by pixels / by pages / to the ends.
    ScrollPixels(f32, f32),
    ScrollPages(i32),
    ScrollTop,
    ScrollBottom,
    /// `+1`, `-1`, `0` = reset.
    Zoom(i8),
    ToggleTheme,
    ToggleBookmarks,
    BookmarkPage,
    Copy,
    Paste,
    SelectAll,
    Help,
    Quit,
    /// Keyboard focus inside the page.
    FocusPage,
    FocusNext,
    FocusPrev,
    /// `Enter`/`Space` on the focused link or control.
    ActivateFocused,
    /// A printable character that belongs to the *page* (typing into a field).
    PageChar(char),
}

/// `true` when the shortcut is "the plain key, no modifiers".
fn plain(mods: &Mods) -> bool {
    !mods.ctrl && !mods.alt && !mods.meta
}

/// The binding table for one key.
pub fn lookup(mode: Mode, key: Key, mods: Mods) -> Command {
    use Command::*;
    // Ctrl/Meta is a shortcut everywhere; Alt too, except for the arrow keys
    // below where Alt+Left/Right is navigation.
    if let Key::Char(c) = key {
        if mods.ctrl || mods.meta {
            return match c.to_ascii_lowercase() {
                'l' => AddressBar,
                't' => NewTab(None),
                'n' => NewTab(None),
                'w' => CloseTab,
                'r' => Reload,
                'f' => FindBar,
                'g' => FindNext,
                'j' => DevtoolsPanel(1),
                'd' => BookmarkPage,
                'b' => ToggleBookmarks,
                'u' => ViewSource,
                'a' => SelectAll,
                // Ctrl+Shift+C picks an element, plain Ctrl+C copies
                'c' if mods.shift => PickElement,
                'c' => Copy,
                'v' => Paste,
                'q' => Quit,
                'h' => Home,
                '0' => Zoom(0),
                '1'..='8' => UseTab(c as usize - '1' as usize),
                '9' => UseTab(usize::MAX),
                '=' | '+' => Zoom(1),
                '-' | '_' => Zoom(-1),
                'k' if mods.shift => ToggleTheme,
                _ => Nop,
            };
        }
        if mods.alt {
            return match c {
                'h' => Back,
                'l' => Forward,
                'd' => AddressBar,
                't' => NewTab(None),
                _ => Nop,
            };
        }
        if mode.is_text() {
            // text for a field; the field itself is edited by `App`
            return Nop;
        }
        if plain(&mods) {
            return match c {
                'f' => Hints(false),
                'F' => Hints(true),
                '/' => FindBar,
                '?' => Help,
                ':' => CommandBar,
                'i' => DevtoolsToggle,
                'H' => Back,
                'L' => Forward,
                'r' => Reload,
                'o' => AddressBar,
                'p' => Paste,
                'y' => Copy,
                'g' => ScrollTop,
                'G' => ScrollBottom,
                'k' => ScrollPixels(0.0, -40.0),
                'j' => ScrollPixels(0.0, 40.0),
                'h' => ScrollPixels(-40.0, 0.0),
                'l' => ScrollPixels(40.0, 0.0),
                'x' => CloseTab,
                'K' => ToggleTheme,
                'n' => FindNext,
                'N' => FindPrev,
                _ => Nop,
            };
        }
        // Shift+letter with no other modifier still lands above (`F`, `G`, …)
        Nop
    } else if mods.ctrl || mods.alt || mods.meta {
        match key {
            Key::Named(NamedKey::Left) if mods.alt => Back,
            Key::Named(NamedKey::Right) if mods.alt => Forward,
            // the terminal habits: Shift+Insert pastes, Ctrl+Insert copies
            Key::Named(NamedKey::Insert) if mods.shift => Paste,
            Key::Named(NamedKey::Insert) => Copy,
            Key::Named(NamedKey::Tab) if mods.shift => PrevTab,
            Key::Named(NamedKey::Tab) => NextTab,
            Key::Named(NamedKey::Home) => ScrollTop,
            Key::Named(NamedKey::End) => ScrollBottom,
            Key::Named(NamedKey::Enter) if mode.is_text() => GoAddress,
            _ => Nop,
        }
    } else {
        match key {
            Key::Named(n) => match n {
                NamedKey::Insert if mods.shift => Paste,
                NamedKey::Escape => CloseBar,
                NamedKey::Enter => {
                    if mode.is_text() {
                        GoAddress
                    } else {
                        ActivateFocused
                    }
                }
                NamedKey::Tab if mods.shift => {
                    if mode.is_text() {
                        Nop
                    } else {
                        FocusPrev
                    }
                }
                NamedKey::Tab => {
                    if mode.is_text() {
                        Nop
                    } else {
                        FocusNext
                    }
                }
                NamedKey::Backspace if mode.is_text() => Nop,
                NamedKey::Backspace => Back,
                NamedKey::Delete => Nop,
                NamedKey::F(1) => Help,
                NamedKey::F(5) => Reload,
                NamedKey::F(12) => DevtoolsToggle,
                NamedKey::Left
                | NamedKey::Right
                | NamedKey::Home
                | NamedKey::End
                | NamedKey::Up
                | NamedKey::Down
                    if mode.is_text() =>
                {
                    // the field edits itself; see `App::on_field_key`
                    Nop
                }
                NamedKey::Up => ScrollPixels(0.0, -40.0),
                NamedKey::Down => ScrollPixels(0.0, 40.0),
                NamedKey::Left => ScrollPixels(-40.0, 0.0),
                NamedKey::Right => ScrollPixels(40.0, 0.0),
                NamedKey::PageUp => ScrollPages(-1),
                NamedKey::PageDown => ScrollPages(1),
                NamedKey::Home => ScrollTop,
                NamedKey::End => ScrollBottom,
                NamedKey::Insert => Nop,
                NamedKey::F(_) => Nop,
            },
            Key::Char(' ') if !mode.is_text() => ScrollPages(1),
            _ => Nop,
        }
    }
}

/// The keys the text fields handle themselves (arrows, editing, history).
pub fn field_keys() -> Vec<(Key, &'static str)> {
    vec![
        (Key::Named(NamedKey::Left), "caret left"),
        (Key::Named(NamedKey::Right), "caret right"),
        (Key::Named(NamedKey::Home), "to start"),
        (Key::Named(NamedKey::End), "to end"),
        (Key::Named(NamedKey::Backspace), "delete left"),
        (Key::Named(NamedKey::Delete), "delete right"),
        (Key::Named(NamedKey::Up), "history back"),
        (Key::Named(NamedKey::Down), "history forward"),
        (Key::Named(NamedKey::Enter), "go"),
        (Key::Named(NamedKey::Tab), "complete"),
        (Key::Named(NamedKey::Escape), "cancel"),
    ]
}

/// The help overlay and `--help` print this table, and only this table — so
/// what is documented is what is bound.
pub const SHORTCUTS: &[(&str, &str)] = &[
    ("Ctrl+L / Alt+D", "edit the address"),
    ("Ctrl+T / Ctrl+N", "new tab"),
    ("Ctrl+W / x", "close tab"),
    ("Ctrl+Shift+T", "reopen the closed tab"),
    ("Ctrl+1…8 / Ctrl+9", "switch to tab n / the last one"),
    ("Ctrl+Tab", "next tab"),
    ("Alt+Left / H", "back"),
    ("Alt+Right / L", "forward"),
    ("Ctrl+R / F5 / r", "reload"),
    ("Ctrl+Shift+R", "reload, ignoring the cache"),
    ("Alt+H", "home page"),
    ("Ctrl+F / /", "find in page"),
    ("Enter / n", "next match"),
    ("Shift+Enter / N", "previous match"),
    ("f / F", "link hints (F = new tab)"),
    ("Tab / Shift+Tab", "focus next / previous control"),
    ("Enter / Space", "activate the focused control"),
    ("Space / PgDn", "page down"),
    ("Shift+Space / PgUp", "page up"),
    ("k j h l / arrows", "scroll"),
    ("g / G", "top / bottom"),
    ("Ctrl+ +/- / Ctrl+0", "zoom in / out / reset"),
    ("Ctrl+C / y", "copy selection"),
    ("Ctrl+V / p", "paste"),
    ("Ctrl+A", "select all"),
    ("Ctrl+D", "bookmark this page"),
    ("Ctrl+B", "toggle the bookmark bar"),
    ("Ctrl+U", "view source"),
    ("Ctrl+J", "console"),
    ("F12 / i", "inspector"),
    ("Ctrl+Shift+C", "pick an element"),
    ("K", "dark / light theme"),
    (":", "command line"),
    ("F1 / ?", "this help"),
    ("Esc", "close overlay / stop"),
    ("Ctrl+Q", "quit"),
];
