//! Keysyms: what an X key code means.
//!
//! X reports a *key code* (a physical key) and the server owns the layout. To
//! find out what a key code means we ask for the keyboard mapping
//! (`GetKeyboardMapping`) and translate the keysym ourselves — the standard
//! keysym table is small where a browser needs it: ASCII, Latin-1, the Unicode
//! keysyms (`0x0100_0000 | codepoint`) and the function/navigation block.

use tb_ui::{Key, NamedKey};

/// Translate a keysym into a key. `None` means "a modifier, or nothing we can
/// type with" — the modifier itself travels in the event's state mask.
pub fn keysym_to_key(sym: u32) -> Option<Key> {
    match sym {
        // ASCII, including ' ' at 0x20
        0x20..=0x7e => Some(Key::Char(sym as u8 as char)),
        // Latin-1: the keysym *is* the code point
        0xa0..=0xff => Some(char_key(sym)),
        // Unicode keysyms: 0x01000000 + code point
        0x0100_0000..=0x10ff_ffff => Some(char_key(sym & 0x00ff_ffff)),
        0xff0d => Some(Key::Named(NamedKey::Enter)),
        0xff8d => Some(Key::Named(NamedKey::Enter)), // KP_Enter
        0xff09 => Some(Key::Named(NamedKey::Tab)),
        0xff1b => Some(Key::Named(NamedKey::Escape)),
        0xff08 => Some(Key::Named(NamedKey::Backspace)),
        0xffff => Some(Key::Named(NamedKey::Delete)),
        0xff63 => Some(Key::Named(NamedKey::Insert)),
        0xff50 => Some(Key::Named(NamedKey::Home)),
        0xff57 => Some(Key::Named(NamedKey::End)),
        0xff51 => Some(Key::Named(NamedKey::Left)),
        0xff52 => Some(Key::Named(NamedKey::Up)),
        0xff53 => Some(Key::Named(NamedKey::Right)),
        0xff54 => Some(Key::Named(NamedKey::Down)),
        0xff55 => Some(Key::Named(NamedKey::PageUp)),
        0xff56 => Some(Key::Named(NamedKey::PageDown)),
        // F1…F12 are consecutive
        0xffbe..=0xffc9 => Some(Key::Named(NamedKey::F((sym - 0xffbe) as u8 + 1))),
        // keypad digits, so a numpad types numbers
        0xffb0..=0xffb9 => Some(Key::Char((b'0' + (sym - 0xffb0) as u8) as char)),
        0xffab => Some(Key::Char('+')),
        0xffad => Some(Key::Char('-')),
        0xffaa => Some(Key::Char('*')),
        0xffaf => Some(Key::Char('/')),
        0xffac => Some(Key::Char(',')),
        0xffae => Some(Key::Char('.')),
        // modifiers: the state mask says it better than the key event does
        0xffe1..=0xffee => None,
        _ => None,
    }
}

fn char_key(cp: u32) -> Key {
    match char::from_u32(cp) {
        Some(c) if !c.is_control() => Key::Char(c),
        _ => Key::Named(NamedKey::Escape),
    }
}

/// The keyboard map: key code → the keysyms of its levels.
#[derive(Debug, Default, Clone)]
pub struct Keymap {
    /// `levels[keycode]` is the slice from `GetKeyboardMapping` (level 0 plain,
    /// level 1 shifted, level 2 AltGr, level 3 AltGr+Shift).
    levels: Vec<Vec<u32>>,
}

impl Keymap {
    /// Build from the flat `GetKeyboardMapping` reply.
    pub fn from_reply(first_keycode: u8, per_keycode: usize, syms: &[u32]) -> Keymap {
        let mut levels = vec![Vec::new(); 256];
        if per_keycode == 0 {
            return Keymap { levels };
        }
        let count = syms.len() / per_keycode;
        for i in 0..count {
            let kc = first_keycode as usize + i;
            if kc >= 256 {
                break;
            }
            let start = i * per_keycode;
            levels[kc] = syms[start..start + per_keycode].to_vec();
        }
        Keymap { levels }
    }

    /// The key for a key code and the modifier mask of the event.
    pub fn key(&self, keycode: u8, shift: bool, alt_gr: bool) -> Option<Key> {
        let row = self.levels.get(keycode as usize)?;
        if row.is_empty() {
            return None;
        }
        // `GetKeyboardMapping` gives 2 levels per key unless the layout has an
        // AltGr; a key with a single symbols entry has no shifted form.
        let index = if row.len() >= 4 {
            match (alt_gr, shift) {
                (false, false) => 0,
                (false, true) => 1,
                (true, false) => 2,
                (true, true) => 3,
            }
        } else if row.len() == 2 && shift {
            1
        } else {
            0
        };
        let sym = row.get(index).copied().unwrap_or(0);
        if sym == 0 {
            // no symbol for this level: fall back to the plain one
            let plain = row.first().copied().unwrap_or(0);
            if plain == 0 || shift {
                return None;
            }
            return keysym_to_key(plain);
        }
        keysym_to_key(sym)
    }
}
