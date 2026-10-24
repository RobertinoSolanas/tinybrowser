//! The clipboard.
//!
//! An internal buffer always works — copy with `Ctrl+C`, paste with `Ctrl+V`,
//! inside this browser, in any backend, in a test. Where the display can carry
//! text out we also send it: terminals that allow OSC-52 put it in the *system*
//! clipboard, and the X11 backend owns the primary selection. Reading the system
//! clipboard is a different story (a terminal cannot, and X11 needs a full
//! `CLIPBOARD` negotiation), so pasting is the internal buffer plus what the
//! backend could read — and the address bar takes a plain typed path anyway.

#[derive(Clone, Debug, Default)]
pub struct Clipboard {
    text: String,
}

impl Clipboard {
    pub fn new() -> Clipboard {
        Clipboard::default()
    }

    pub fn set(&mut self, text: &str) {
        self.text = text.to_string();
    }

    pub fn get(&self) -> &str {
        &self.text
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// OSC-52: `ESC ] 52 ; c ; <base64> BEL`. A terminal that honours it puts
    /// the text on the system clipboard; one that does not prints nothing
    /// (they are filtered, not echoed) — so it is safe to always send.
    pub fn osc52(&self) -> String {
        Self::osc52_for(&self.text)
    }

    pub fn osc52_for(text: &str) -> String {
        if text.is_empty() {
            return String::new();
        }
        format!("\x1b]52;c;{}\x07", base64(text.as_bytes()))
    }
}

/// Base64, because OSC-52 wants it and we do not pull in a crate for eight
/// lines of table lookup.
pub fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(T[(n >> 18 & 63) as usize] as char);
        out.push(T[(n >> 12 & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            T[(n >> 6 & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}
