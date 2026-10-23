//! Character reference decoding (named + numeric).
//!
//! A single left-to-right pass, so `&amp;#38;` correctly yields `&#38;` and not
//! `&`. Unknown references are left verbatim, exactly like a browser does.

/// Decode character references in text content.
pub fn decode(input: &str, in_attribute: bool) -> String {
    if !input.contains('&') {
        return input.to_string();
    }
    let bytes: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c != '&' {
            out.push(c);
            i += 1;
            continue;
        }
        match decode_one(&bytes, i, in_attribute) {
            Some((text, consumed)) => {
                out.push_str(&text);
                i += consumed;
            }
            None => {
                out.push('&');
                i += 1;
            }
        }
    }
    out
}

/// Decode a single character reference at `start` (`chars[start]` must be `&`).
///
/// Returns `(replacement, consumed_chars_including_the_ampersand)`.
pub fn decode_at(chars: &[char], start: usize) -> Option<(String, usize)> {
    decode_one(chars, start, false)
}

/// Returns `(replacement, consumed_bytes_including_ampersand)`.
fn decode_one(chars: &[char], start: usize, in_attribute: bool) -> Option<(String, usize)> {
    debug_assert_eq!(chars[start], '&');
    let mut j = start + 1;
    if j >= chars.len() {
        return None;
    }
    if chars[j] == '#' {
        j += 1;
        let hex = chars.get(j) == Some(&'x') || chars.get(j) == Some(&'X');
        if hex {
            j += 1;
        }
        let digits_start = j;
        while j < chars.len()
            && (chars[j].is_ascii_alphanumeric()
                && (hex == chars[j].is_ascii_alphabetic() || chars[j].is_ascii_digit()))
        {
            j += 1;
        }
        let digits: String = chars[digits_start..j].iter().collect();
        if digits.is_empty() {
            return None;
        }
        let radix = if hex { 16 } else { 10 };
        let cp = u32::from_str_radix(&digits, radix).ok()?;
        // the trailing semicolon is optional for numeric references
        let consumed = (j - start) + if chars.get(j) == Some(&';') { 1 } else { 0 };
        let _ = in_attribute;
        let decoded = if cp == 0 {
            '\u{fffd}'
        } else {
            char::from_u32(cp).unwrap_or('\u{fffd}')
        };
        Some((decoded.to_string(), consumed))
    } else {
        // named reference: [a-zA-Z0-9]+ then ';'
        let name_start = j;
        while j < chars.len() && (chars[j].is_ascii_alphanumeric()) {
            j += 1;
        }
        if j == name_start {
            return None;
        }
        let name: String = chars[name_start..j].iter().collect();
        let has_semi = chars.get(j) == Some(&';');
        if let Some(cp) = lookup(&name) {
            let consumed = if has_semi {
                (j - start) + 1
            } else {
                // Without the semicolon only a handful of legacy names decode.
                if LEGACY_NO_SEMI.contains(&name.as_str()) {
                    j - start
                } else {
                    return None;
                }
            };
            return Some((String::from(cp), consumed));
        }
        None
    }
}

/// Named references that decode even without a trailing semicolon.
const LEGACY_NO_SEMI: [&str; 8] = ["amp", "lt", "gt", "quot", "copy", "reg", "nbsp", "amp"];

/// The named character references tinybrowser knows.
pub fn lookup(name: &str) -> Option<char> {
    let cp = match name {
        "quot" => 34,
        "amp" => 38,
        "apos" => 39,
        "lt" => 60,
        "gt" => 62,
        "nbsp" => 160,
        "iexcl" => 161,
        "cent" => 162,
        "pound" => 163,
        "curren" => 164,
        "yen" => 165,
        "brvbar" => 166,
        "sect" => 167,
        "uml" => 168,
        "copy" => 169,
        "ordf" => 170,
        "laquo" => 171,
        "not" => 172,
        "shy" => 173,
        "reg" => 174,
        "macr" => 175,
        "deg" => 176,
        "plusmn" => 177,
        "sup2" => 178,
        "sup3" => 179,
        "acute" => 180,
        "micro" => 181,
        "para" => 182,
        "middot" => 183,
        "cedil" => 184,
        "sup1" => 185,
        "ordm" => 186,
        "raquo" => 187,
        "frac14" => 188,
        "frac12" => 189,
        "frac34" => 190,
        "iquest" => 191,
        "Agrave" => 192,
        "Aacute" => 193,
        "Acirc" => 194,
        "Atilde" => 195,
        "Auml" => 196,
        "Aring" => 197,
        "AElig" => 198,
        "Ccedil" => 199,
        "Egrave" => 200,
        "Eacute" => 201,
        "Ecirc" => 202,
        "Euml" => 203,
        "Igrave" => 204,
        "Iacute" => 205,
        "Icirc" => 206,
        "Iuml" => 207,
        "ETH" => 208,
        "Ntilde" => 209,
        "Ograve" => 210,
        "Oacute" => 211,
        "Ocirc" => 212,
        "Otilde" => 213,
        "Ouml" => 214,
        "times" => 215,
        "Oslash" => 216,
        "Ugrave" => 217,
        "Uacute" => 218,
        "Ucirc" => 219,
        "Uuml" => 220,
        "Yacute" => 221,
        "THORN" => 222,
        "szlig" => 223,
        "agrave" => 224,
        "aacute" => 225,
        "acirc" => 226,
        "atilde" => 227,
        "auml" => 228,
        "aring" => 229,
        "aelig" => 230,
        "ccedil" => 231,
        "egrave" => 232,
        "eacute" => 233,
        "ecirc" => 234,
        "euml" => 235,
        "igrave" => 236,
        "iacute" => 237,
        "icirc" => 238,
        "iuml" => 239,
        "eth" => 240,
        "ntilde" => 241,
        "ograve" => 242,
        "oacute" => 243,
        "ocirc" => 244,
        "otilde" => 245,
        "ouml" => 246,
        "divide" => 247,
        "oslash" => 248,
        "ugrave" => 249,
        "uacute" => 250,
        "ucirc" => 251,
        "uuml" => 252,
        "yacute" => 253,
        "thorn" => 254,
        "yuml" => 255,
        "euro" => 8364,
        "ndash" => 8211,
        "mdash" => 8212,
        "lsquo" => 8216,
        "rsquo" => 8217,
        "sbquo" => 8218,
        "ldquo" => 8220,
        "rdquo" => 8221,
        "bdquo" => 8222,
        "dagger" => 8224,
        "bull" => 8226,
        "hellip" => 8230,
        "permil" => 8240,
        "prime" => 8242,
        "Prime" => 8243,
        "lsaquo" => 8249,
        "rsaquo" => 8250,
        "oline" => 8254,
        "frasl" => 8260,
        "trade" => 8482,
        "alefsym" => 8501,
        "larr" => 8592,
        "uarr" => 8593,
        "rarr" => 8594,
        "darr" => 8595,
        "harr" => 8596,
        "crarr" => 8629,
        "forall" => 8704,
        "part" => 8706,
        "exist" => 8707,
        "empty" => 8709,
        "nabla" => 8711,
        "isin" => 8712,
        "notin" => 8713,
        "ni" => 8715,
        "prod" => 8719,
        "sum" => 8721,
        "minus" => 8722,
        "lowast" => 8727,
        "radic" => 8730,
        "prop" => 8733,
        "infin" => 8734,
        "ang" => 8736,
        "and" => 8743,
        "or" => 8744,
        "cap" => 8745,
        "cup" => 8746,
        "int" => 8747,
        "there4" => 8756,
        "sim" => 8764,
        "cong" => 8773,
        "asymp" => 8776,
        "ne" => 8800,
        "equiv" => 8801,
        "le" => 8804,
        "ge" => 8805,
        "sub" => 8834,
        "sup" => 8835,
        "nsub" => 8836,
        "sube" => 8838,
        "supe" => 8839,
        "oplus" => 8853,
        "otimes" => 8855,
        "perp" => 8869,
        "sdot" => 8901,
        "lceil" => 8968,
        "rceil" => 8969,
        "lfloor" => 8970,
        "rfloor" => 8971,
        "loz" => 9674,
        "spades" => 9824,
        "clubs" => 9827,
        "hearts" => 9829,
        "diams" => 9830,
        "ensp" => 8194,
        "emsp" => 8195,
        "thinsp" => 8201,
        "zwnj" => 8204,
        "zwj" => 8205,
        "lrm" => 8206,
        "rlm" => 8207,
        "Alpha" => 913,
        "Beta" => 914,
        "Gamma" => 915,
        "Delta" => 916,
        "Epsilon" => 917,
        "Zeta" => 918,
        "Eta" => 919,
        "Theta" => 920,
        "Iota" => 921,
        "Kappa" => 922,
        "Lambda" => 923,
        "Mu" => 924,
        "Nu" => 925,
        "Xi" => 926,
        "Omicron" => 927,
        "Pi" => 928,
        "Sigma" => 931,
        "Tau" => 932,
        "Upsilon" => 933,
        "Phi" => 934,
        "Chi" => 935,
        "Psi" => 936,
        "Omega" => 937,
        "alpha" => 945,
        "beta" => 946,
        "gamma" => 947,
        "delta" => 948,
        "epsilon" => 949,
        "zeta" => 950,
        "eta" => 951,
        "theta" => 952,
        "iota" => 953,
        "kappa" => 954,
        "lambda" => 955,
        "mu" => 956,
        "nu" => 957,
        "xi" => 958,
        "pi" => 960,
        "sigma" => 963,
        "tau" => 964,
        "upsilon" => 965,
        "phi" => 966,
        "chi" => 967,
        "psi" => 968,
        "omega" => 969,
        _ => return None,
    };
    char::from_u32(cp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_named_and_numeric() {
        assert_eq!(decode("a &amp; b", false), "a & b");
        assert_eq!(decode("&lt;div&gt;", false), "<div>");
        assert_eq!(decode("&nbsp;x", false), "\u{a0}x");
        assert_eq!(decode("&#65;&#x42;", false), "AB");
        assert_eq!(decode("&#169;", false), "©");
        assert_eq!(decode("&mdash;&ndash;", false), "—–");
        assert_eq!(decode("&uuml;", false), "ü");
    }

    #[test]
    fn single_pass_only() {
        assert_eq!(decode("&amp;#38;", false), "&#38;");
        assert_eq!(decode("&amp;amp;", false), "&amp;");
    }

    #[test]
    fn unknown_references_stay() {
        assert_eq!(decode("&notarealentity;", false), "&notarealentity;");
        assert_eq!(decode("AT&T", false), "AT&T");
        assert_eq!(decode("&", false), "&");
        assert_eq!(decode("&x", false), "&x");
    }

    #[test]
    fn legacy_names_without_semicolon() {
        assert_eq!(decode("a&lt;b", false), "a<b");
        assert_eq!(decode("5&times3", false), "5&times3", "needs the semicolon");
        assert_eq!(decode("5&times;3", false), "5×3");
    }

    #[test]
    fn numeric_without_semicolon_and_invalid() {
        assert_eq!(decode("&#72;i", false), "Hi");
        assert_eq!(decode("&#0;", false), "\u{fffd}");
        assert_eq!(
            decode("&#xD800;", false),
            "\u{fffd}",
            "surrogate -> replacement"
        );
    }
}
