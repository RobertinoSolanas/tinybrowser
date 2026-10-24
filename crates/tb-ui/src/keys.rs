//! Parse a key combo name (`ctrl+shift+r`, `F12`, `Enter`, `Alt+Left`) into a
//! key and its modifiers. Used by `--ui-script`, the help overlay and the tests,
//! so all three name keys the same way a human would type them.

use crate::event::{Key, Mods, NamedKey};

/// `"ctrl+shift+r"` → `(Key::Char('R'), Mods{ctrl,shift,..})`.
pub fn parse_key_combo(name: &str) -> (Option<Key>, Mods) {
    let mut mods = Mods::none();
    let mut key: Option<Key> = None;
    for part in name.split('+') {
        let p = part.trim().to_ascii_lowercase();
        if p.is_empty() {
            continue;
        }
        match p.as_str() {
            "ctrl" | "control" => mods.ctrl = true,
            "shift" => mods.shift = true,
            "alt" | "option" => mods.alt = true,
            "super" | "cmd" | "win" | "meta" => mods.meta = true,
            _ => key = parse_key_name(&p),
        }
    }
    (key, mods)
}

/// One key name (case-insensitive).
pub fn parse_key_name(p: &str) -> Option<Key> {
    let lower = p.to_ascii_lowercase();
    let p = lower.as_str();
    match p {
        "enter" | "return" => return Some(Key::Named(NamedKey::Enter)),
        "esc" | "escape" => return Some(Key::Named(NamedKey::Escape)),
        "tab" => return Some(Key::Named(NamedKey::Tab)),
        "backspace" | "bs" => return Some(Key::Named(NamedKey::Backspace)),
        "delete" | "del" => return Some(Key::Named(NamedKey::Delete)),
        "insert" | "ins" => return Some(Key::Named(NamedKey::Insert)),
        "home" => return Some(Key::Named(NamedKey::Home)),
        "end" => return Some(Key::Named(NamedKey::End)),
        "pgup" | "pageup" => return Some(Key::Named(NamedKey::PageUp)),
        "pgdn" | "pagedn" => return Some(Key::Named(NamedKey::PageDown)),
        "up" => return Some(Key::Named(NamedKey::Up)),
        "down" => return Some(Key::Named(NamedKey::Down)),
        "left" => return Some(Key::Named(NamedKey::Left)),
        "right" => return Some(Key::Named(NamedKey::Right)),
        "space" => return Some(Key::Char(' ')),
        _ => (),
    }
    if let Some(rest) = p.strip_prefix('f') {
        if let Ok(n) = rest.parse::<u8>() {
            if (1..=12).contains(&n) {
                return Some(Key::Named(NamedKey::F(n)));
            }
        }
    }
    let mut chars = p.chars();
    let first = chars.next()?;
    if chars.next().is_none() {
        return Some(Key::Char(first));
    }
    None
}
