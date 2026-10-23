//! Character encoding support.
//!
//! Priority order follows the HTML standard: BOM, then `Content-Type`, then a
//! `<meta charset>` in the first KiB, then UTF-8 with lossy replacement.

/// A character encoding tinybrowser can decode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Charset {
    Utf8,
    Utf16Le,
    Utf16Be,
    Latin1,
    Windows1252,
    Ascii,
}

impl Charset {
    /// Map an IANA label to a charset (aliases included).
    pub fn from_label(label: &str) -> Option<Charset> {
        let l = label.trim().to_ascii_lowercase().replace('_', "-");
        let l = l.trim();
        match l {
            "utf-8" | "utf8" | "unicode" => Some(Charset::Utf8),
            "utf-16le" | "utf-16" | "utf-16le-bom" => Some(Charset::Utf16Le),
            "utf-16be" | "utf-16be-bom" => Some(Charset::Utf16Be),
            "iso-8859-1" | "latin1" | "latin-1" | "iso8859-1" => Some(Charset::Latin1),
            "windows-1252" | "win-1252" | "cp1252" => Some(Charset::Windows1252),
            "us-ascii" | "ascii" => Some(Charset::Ascii),
            // Anything else in the wild (iso-8859-15, koi8-r, ...) we treat as
            // latin-1 like a browser falling back would.
            "" => None,
            _ => Some(Charset::Latin1),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Charset::Utf8 => "utf-8",
            Charset::Utf16Le => "utf-16le",
            Charset::Utf16Be => "utf-16be",
            Charset::Latin1 => "iso-8859-1",
            Charset::Windows1252 => "windows-1252",
            Charset::Ascii => "us-ascii",
        }
    }

    /// Decode bytes to `String`. Never fails (invalid UTF-8 becomes U+FFFD).
    pub fn decode(self, bytes: &[u8]) -> String {
        match self {
            Charset::Utf8 => String::from_utf8_lossy(bytes).into_owned(),
            Charset::Utf16Le => decode_utf16(bytes, true),
            Charset::Utf16Be => decode_utf16(bytes, false),
            Charset::Latin1 => bytes.iter().map(|&b| b as char).collect(),
            Charset::Windows1252 => bytes.iter().map(|&b| cp1252(b)).collect(),
            Charset::Ascii => bytes
                .iter()
                .map(|&b| if b < 128 { b as char } else { '\u{fffd}' })
                .collect(),
        }
    }
}

fn decode_utf16(bytes: &[u8], little: bool) -> String {
    let mut units: Vec<u16> = Vec::with_capacity(bytes.len() / 2);
    let mut i = 0;
    while i + 1 < bytes.len() {
        let u = if little {
            u16::from_le_bytes([bytes[i], bytes[i + 1]])
        } else {
            u16::from_be_bytes([bytes[i], bytes[i + 1]])
        };
        units.push(u);
        i += 2;
    }
    String::from_utf16_lossy(&units)
}

/// Windows-1252 upper half block.
fn cp1252(b: u8) -> char {
    if b < 0x80 {
        return b as char;
    }
    let extra = match b {
        0x80 => '\u{20ac}',
        0x82 => '\u{201a}',
        0x83 => '\u{0192}',
        0x84 => '\u{201e}',
        0x85 => '\u{2026}',
        0x86 => '\u{2020}',
        0x87 => '\u{2021}',
        0x88 => '\u{02c6}',
        0x89 => '\u{2030}',
        0x8a => '\u{0160}',
        0x8b => '\u{2039}',
        0x8c => '\u{0152}',
        0x8e => '\u{017d}',
        0x91 => '\u{2018}',
        0x92 => '\u{2019}',
        0x93 => '\u{201c}',
        0x94 => '\u{201d}',
        0x95 => '\u{2022}',
        0x96 => '\u{2013}',
        0x97 => '\u{2014}',
        0x98 => '\u{02dc}',
        0x99 => '\u{2122}',
        0x9a => '\u{0161}',
        0x9b => '\u{203a}',
        0x9c => '\u{0153}',
        0x9e => '\u{017e}',
        0x9f => '\u{0178}',
        _ => return b as char,
    };
    extra
}

/// `true` when the first 1024 bytes are ASCII (an ASCII compatible prefix
/// makes a charset declaration trustworthy).
pub fn is_ascii_prefix(bytes: &[u8]) -> bool {
    bytes[..bytes.len().min(1024)].iter().all(|b| b.is_ascii())
}

/// Look for `<meta charset>` / `<meta http-equiv=content-type charset=...>`.
///
/// The HTML standard limits this sniff to the first 1024 bytes.
pub fn sniff_meta_charset(bytes: &[u8]) -> Option<String> {
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(2048)]);
    // No `<` in the first bytes => not markup.
    if !head.contains('<') {
        return None;
    }
    let lower = head.to_ascii_lowercase();
    let bytes_of = lower.as_bytes();
    let mut idx = 0;
    while let Some(rel) = lower[idx..].find("<meta") {
        let start = idx + rel;
        let end = lower[start..]
            .find('>')
            .map(|e| start + e)
            .unwrap_or(lower.len());
        let tag = &lower[start..end];
        if let Some(cs) = extract_charset(tag) {
            // sanity: only accept if it looks like a charset token
            if !cs.is_empty()
                && cs
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_' || c == b':')
            {
                return Some(cs);
            }
        }
        idx = end.saturating_add(1).max(start + 1);
        if idx >= bytes_of.len() {
            break;
        }
    }
    None
}

fn extract_charset(tag: &str) -> Option<String> {
    // charset="utf-8" | charset=utf-8 | charset='utf-8'
    let i = tag.find("charset")? + "charset".len();
    let rest = tag[i..].trim_start();
    let rest = rest.strip_prefix('=')?.trim_start();
    let rest = rest.strip_prefix(['\'', '"']).unwrap_or(rest);
    let end = rest
        .find(|c| matches!(c, '\'' | '"' | ';' | ' ' | '>' | '\t' | '\n'))
        .unwrap_or(rest.len());
    Some(rest[..end].to_string())
}

/// Detect BOM if present.
pub fn detect_bom(bytes: &[u8]) -> Option<(Charset, usize)> {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        Some((Charset::Utf8, 3))
    } else if bytes.starts_with(&[0xFF, 0xFE]) {
        Some((Charset::Utf16Le, 2))
    } else if bytes.starts_with(&[0xFE, 0xFF]) {
        Some((Charset::Utf16Be, 2))
    } else {
        None
    }
}

/// Full document decode: returns text plus the charset name that was used.
pub fn decode_document(bytes: &[u8], header_charset: Option<&str>) -> (String, String) {
    if let Some((cs, skip)) = detect_bom(bytes) {
        return (cs.decode(&bytes[skip..]), cs.label().to_string());
    }
    if let Some(cs) = header_charset.and_then(Charset::from_label) {
        // A server that blindly claims utf-8 while the document declares
        // latin-1 gets rescued (strict utf-8 would produce replacement chars).
        if cs == Charset::Utf8 && std::str::from_utf8(bytes).is_err() {
            if let Some(meta_cs) = sniff_meta_charset(bytes)
                .as_deref()
                .and_then(Charset::from_label)
            {
                if meta_cs != Charset::Utf8 {
                    return (meta_cs.decode(bytes), meta_cs.label().to_string());
                }
            }
        }
        return (cs.decode(bytes), cs.label().to_string());
    }
    if let Some(meta) = sniff_meta_charset(bytes) {
        if let Some(cs) = Charset::from_label(&meta) {
            return (cs.decode(bytes), cs.label().to_string());
        }
    }
    // Strict UTF-8 first, latin-1 fallback (mirrors the windows-1252 habit of
    // browsers when utf-8 decoding fails on a page without declaration).
    match std::str::from_utf8(bytes) {
        Ok(s) => (s.to_string(), "utf-8".to_string()),
        Err(_) => (
            Charset::Windows1252.decode(bytes),
            "windows-1252".to_string(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_map_to_charsets() {
        assert_eq!(Charset::from_label("UTF-8"), Some(Charset::Utf8));
        assert_eq!(Charset::from_label("ISO_8859-1"), Some(Charset::Latin1));
        assert_eq!(Charset::from_label("cp1252"), Some(Charset::Windows1252));
        assert_eq!(Charset::from_label(""), None);
        assert_eq!(Charset::from_label("koi8-r"), Some(Charset::Latin1));
    }

    #[test]
    fn decodes_latin1_and_cp1252() {
        assert_eq!(Charset::Latin1.decode(&[0xfc]), "ü");
        // Windows-1252 smart quotes and euro
        assert_eq!(
            Charset::Windows1252.decode(&[0x93, 0x41, 0x94]),
            "\u{201c}A\u{201d}"
        );
        assert_eq!(Charset::Windows1252.decode(&[0x80]), "€");
    }

    #[test]
    fn utf16_with_bom() {
        let bytes = [0xFF, 0xFE, b'a', 0, b'b', 0];
        let (text, used) = decode_document(&bytes, None);
        assert_eq!(text, "ab");
        assert_eq!(used, "utf-16le");
    }

    #[test]
    fn sniffs_meta_charset() {
        let html = br#"<html><head><meta http-equiv="Content-Type" content="text/html; charset=ISO-8859-1">"#;
        assert_eq!(sniff_meta_charset(html).as_deref(), Some("iso-8859-1"));
        let html2 = b"<html><head><meta charset='windows-1252'>";
        assert_eq!(sniff_meta_charset(html2).as_deref(), Some("windows-1252"));
        let none = b"<html><head><title>nothing here</title>";
        assert_eq!(sniff_meta_charset(none), None);
    }

    #[test]
    fn header_wins_but_meta_can_rescue_blind_utf8() {
        // 0xfc 0xfe = "üß" in latin-1, but invalid utf-8.
        let bytes = b"<html><meta charset=\"iso-8859-1\">Gr\xfc\xdfe";
        let (text, used) = decode_document(bytes, Some("utf-8"));
        assert_eq!(used, "iso-8859-1");
        assert!(text.contains('ü'), "got {text}");
        // Valid utf-8 keeps the header charset.
        let (t2, u2) = decode_document("<p>ü</p>".as_bytes(), Some("utf-8"));
        assert_eq!(u2, "utf-8");
        assert_eq!(t2, "<p>ü</p>");
    }

    #[test]
    fn invalid_utf8_without_hint_falls_back_to_cp1252() {
        let (text, used) = decode_document(b"Gr\xfc\xdfe", None);
        assert_eq!(used, "windows-1252");
        assert!(text.contains('ü'));
    }
}
