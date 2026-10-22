//! 8 bit RGBA colour value object plus CSS colour syntax parsing.

use std::fmt;

/// sRGB colour with alpha channel (0..=255).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const TRANSPARENT: Color = Color {
        r: 0,
        g: 0,
        b: 0,
        a: 0,
    };
    pub const BLACK: Color = Color {
        r: 0,
        g: 0,
        b: 0,
        a: 255,
    };
    pub const WHITE: Color = Color {
        r: 255,
        g: 255,
        b: 255,
        a: 255,
    };

    pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color { r, g, b, a: 255 }
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color {
        Color { r, g, b, a }
    }

    /// Pack as 0xAARRGGBB.
    pub fn to_argb(self) -> u32 {
        ((self.a as u32) << 24) | ((self.r as u32) << 16) | ((self.g as u32) << 8) | self.b as u32
    }

    pub fn from_argb(v: u32) -> Color {
        Color {
            r: ((v >> 16) & 0xff) as u8,
            g: ((v >> 8) & 0xff) as u8,
            b: (v & 0xff) as u8,
            a: ((v >> 24) & 0xff) as u8,
        }
    }

    /// `color * alpha + dest * (1 - alpha)` (source-over with premultiplied src).
    pub fn blend_over(self, dst: Color) -> Color {
        if self.a == 255 {
            return self;
        }
        if self.a == 0 {
            return dst;
        }
        let sa = self.a as f32 / 255.0;
        let da = dst.a as f32 / 255.0;
        let out_a = sa + da * (1.0 - sa);
        if out_a <= 0.0 {
            return Color::TRANSPARENT;
        }
        let mix = |s: u8, d: u8| -> u8 {
            let v = (s as f32 * sa + d as f32 * da * (1.0 - sa)) / out_a;
            v.round().clamp(0.0, 255.0) as u8
        };
        Color {
            r: mix(self.r, dst.r),
            g: mix(self.g, dst.g),
            b: mix(self.b, dst.b),
            a: (out_a * 255.0).round().clamp(0.0, 255.0) as u8,
        }
    }

    /// Perceived luminance 0..1 (Rec. 601).
    pub fn luminance(self) -> f32 {
        (0.299 * self.r as f32 + 0.587 * self.g as f32 + 0.114 * self.b as f32) / 255.0
    }

    /// A readable foreground (black/white) for this background.
    pub fn readable_on(self) -> Color {
        if self.luminance() > 0.6 {
            Color::BLACK
        } else {
            Color::WHITE
        }
    }

    /// Parse a CSS colour: `#rgb`, `#rrggbb`, `#rrggbbaa`, `#rgba`,
    /// `rgb()/rgba()/hsl()/hsla()` and the CSS colour keywords.
    ///
    /// Returns `None` for unparseable input (never panics).
    pub fn parse(s: &str) -> Option<Color> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }
        if s == "transparent" {
            return Some(Color::TRANSPARENT);
        }
        if s == "currentcolor" {
            return None;
        }
        if let Some(hex) = s.strip_prefix('#') {
            return parse_hex(hex);
        }
        let lower = s.to_ascii_lowercase();
        if let Some(inner) = strip_fn(&lower, "rgb").or_else(|| strip_fn(&lower, "rgba")) {
            return parse_rgb_fn(inner);
        }
        if let Some(inner) = strip_fn(&lower, "hsl").or_else(|| strip_fn(&lower, "hsla")) {
            return parse_hsl_fn(inner);
        }
        named(&lower)
    }

    /// `#rrggbb` string, useful for serialisation and tests.
    pub fn to_hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }
}

impl fmt::Debug for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Color({})", self)
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.a == 255 {
            write!(f, "{}", self.to_hex())
        } else {
            write!(
                f,
                "rgba({},{},{},{})",
                self.r,
                self.g,
                self.b,
                self.a as f32 / 255.0
            )
        }
    }
}

fn strip_fn<'a>(s: &'a str, name: &str) -> Option<&'a str> {
    let rest = s.strip_prefix(name)?;
    let rest = rest.strip_prefix('(')?;
    let inner = rest.strip_suffix(')')?;
    Some(inner)
}

fn hex_val(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

fn parse_hex(hex: &str) -> Option<Color> {
    let b = hex.as_bytes();
    let nib = |i: usize| hex_val(b[i]).map(|v| v as u32);
    match b.len() {
        3 => {
            let (r, g, bl) = (nib(0)?, nib(1)?, nib(2)?);
            Some(Color::rgb((r * 17) as u8, (g * 17) as u8, (bl * 17) as u8))
        }
        4 => {
            let (r, g, bl, a) = (nib(0)?, nib(1)?, nib(2)?, nib(3)?);
            Some(Color::rgba(
                (r * 17) as u8,
                (g * 17) as u8,
                (bl * 17) as u8,
                (a * 17) as u8,
            ))
        }
        6 => {
            let v = u32::from_str_radix(hex, 16).ok()?;
            Some(Color::rgb(((v >> 16) & 0xff) as u8, ((v >> 8) & 0xff) as u8, (v & 0xff) as u8))
        }
        8 => {
            let v = u32::from_str_radix(hex, 16).ok()?;
            Some(Color::rgba(
                ((v >> 24) & 0xff) as u8,
                ((v >> 16) & 0xff) as u8,
                ((v >> 8) & 0xff) as u8,
                (v & 0xff) as u8,
            ))
        }
        _ => None,
    }
}

fn split_commas(s: &str) -> Vec<String> {
    s.split(|c| c == ',' || c == ' ')
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty() && p != "/")
        .collect()
}

fn parse_component(p: &str) -> Option<f32> {
    if let Some(pct) = p.strip_suffix('%') {
        return Some(pct.trim().parse::<f32>().ok()? * 255.0 / 100.0);
    }
    p.parse::<f32>().ok()
}

fn parse_alpha(p: &str) -> Option<u8> {
    if let Some(pct) = p.strip_suffix('%') {
        return Some((pct.trim().parse::<f32>().ok()? * 255.0 / 100.0) as u8);
    }
    p.parse::<f32>().ok().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
}

fn parse_rgb_fn(inner: &str) -> Option<Color> {
    let parts = split_commas(inner);
    if parts.len() < 3 {
        return None;
    }
    let r = parse_component(&parts[0])?.clamp(0.0, 255.0).round() as u8;
    let g = parse_component(&parts[1])?.clamp(0.0, 255.0).round() as u8;
    let b = parse_component(&parts[2])?.clamp(0.0, 255.0).round() as u8;
    let a = parts.get(3).and_then(|p| parse_alpha(p)).unwrap_or(255);
    Some(Color::rgba(r, g, b, a))
}

fn hue_to_rgb(p: f32, q: f32, t: f32) -> f32 {
    let mut t = t;
    if t < 0.0 {
        t += 1.0;
    }
    if t > 1.0 {
        t -= 1.0;
    }
    if t < 1.0 / 6.0 {
        return p + (q - p) * 6.0 * t;
    }
    if t < 1.0 / 2.0 {
        return q;
    }
    if t < 2.0 / 3.0 {
        return p + (q - p) * (2.0 / 3.0 - t) * 6.0;
    }
    p
}

fn parse_hsl_fn(inner: &str) -> Option<Color> {
    let parts = split_commas(inner);
    if parts.len() < 3 {
        return None;
    }
    let h = {
        let raw = parts[0].trim();
        let num = raw
            .trim_end_matches("deg")
            .trim_end_matches("rad")
            .trim()
            .parse::<f32>()
            .ok()?;
        if raw.contains("rad") {
            (num.to_degrees() / 360.0).rem_euclid(1.0)
        } else {
            (num / 360.0).rem_euclid(1.0)
        }
    };
    let s = parts[1].strip_suffix('%')?.trim().parse::<f32>().ok()? / 100.0;
    let l = parts[2].strip_suffix('%')?.trim().parse::<f32>().ok()? / 100.0;
    let (r, g, b) = if s == 0.0 {
        (l, l, l)
    } else {
        let q = if l < 0.5 {
            l * (1.0 + s)
        } else {
            l + s - l * s
        };
        let p = 2.0 * l - q;
        (
            hue_to_rgb(p, q, h + 1.0 / 3.0),
            hue_to_rgb(p, q, h),
            hue_to_rgb(p, q, h - 1.0 / 3.0),
        )
    };
    let a = parts.get(3).and_then(|p| parse_alpha(p)).unwrap_or(255);
    Some(Color::rgba(
        (r * 255.0).round() as u8,
        (g * 255.0).round() as u8,
        (b * 255.0).round() as u8,
        a,
    ))
}

/// The CSS named colours (extended colour keywords).
pub fn named(name: &str) -> Option<Color> {
    let c = match name {
        "black" => (0, 0, 0),
        "silver" => (192, 192, 192),
        "gray" | "grey" => (128, 128, 128),
        "white" => (255, 255, 255),
        "maroon" => (128, 0, 0),
        "red" => (255, 0, 0),
        "purple" => (128, 0, 128),
        "fuchsia" | "magenta" => (255, 0, 255),
        "green" => (0, 128, 0),
        "lime" => (0, 255, 0),
        "olive" => (128, 128, 0),
        "yellow" => (255, 255, 0),
        "navy" => (0, 0, 128),
        "blue" => (0, 0, 255),
        "teal" => (0, 128, 128),
        "aqua" | "cyan" => (0, 255, 255),
        "orange" => (255, 165, 0),
        "aliceblue" => (240, 248, 255),
        "antiquewhite" => (250, 235, 215),
        "aquamarine" => (127, 255, 212),
        "azure" => (240, 255, 255),
        "beige" => (245, 245, 220),
        "bisque" => (255, 228, 196),
        "blanchedalmond" => (255, 235, 205),
        "blueviolet" => (138, 43, 226),
        "brown" => (165, 42, 42),
        "burlywood" => (222, 184, 135),
        "cadetblue" => (95, 158, 160),
        "chartreuse" => (127, 255, 0),
        "chocolate" => (210, 105, 30),
        "coral" => (255, 127, 80),
        "cornflowerblue" => (100, 149, 237),
        "cornsilk" => (255, 248, 220),
        "crimson" => (220, 20, 60),
        "darkblue" => (0, 0, 139),
        "darkcyan" => (0, 139, 139),
        "darkgoldenrod" => (184, 134, 11),
        "darkgray" | "darkgrey" => (169, 169, 169),
        "darkgreen" => (0, 100, 0),
        "darkkhaki" => (189, 183, 107),
        "darkmagenta" => (139, 0, 139),
        "darkolivegreen" => (85, 107, 47),
        "darkorange" => (255, 140, 0),
        "darkorchid" => (153, 50, 204),
        "darkred" => (139, 0, 0),
        "darksalmon" => (233, 150, 122),
        "darkseagreen" => (143, 188, 143),
        "darkslateblue" => (72, 61, 139),
        "darkslategray" | "darkslategrey" => (47, 79, 79),
        "darkturquoise" => (0, 206, 209),
        "darkviolet" => (148, 0, 211),
        "deeppink" => (255, 20, 147),
        "deepskyblue" => (0, 191, 255),
        "dimgray" | "dimgrey" => (105, 105, 105),
        "dodgerblue" => (30, 144, 255),
        "firebrick" => (178, 34, 34),
        "floralwhite" => (255, 250, 240),
        "forestgreen" => (34, 139, 34),
        "gainsboro" => (220, 220, 220),
        "ghostwhite" => (248, 248, 255),
        "gold" => (255, 215, 0),
        "goldenrod" => (218, 165, 32),
        "greenyellow" => (173, 255, 47),
        "honeydew" => (240, 255, 240),
        "hotpink" => (255, 105, 180),
        "indianred" => (205, 92, 92),
        "indigo" => (75, 0, 130),
        "ivory" => (255, 255, 240),
        "khaki" => (240, 230, 140),
        "lavender" => (230, 230, 250),
        "lavenderblush" => (255, 240, 245),
        "lawngreen" => (124, 252, 0),
        "lemonchiffon" => (255, 250, 205),
        "lightblue" => (173, 216, 230),
        "lightcoral" => (240, 128, 128),
        "lightcyan" => (224, 255, 255),
        "lightgoldenrodyellow" => (250, 250, 210),
        "lightgray" | "lightgrey" => (211, 211, 211),
        "lightgreen" => (144, 238, 144),
        "lightpink" => (255, 182, 193),
        "lightsalmon" => (255, 160, 122),
        "lightseagreen" => (32, 178, 170),
        "lightskyblue" => (135, 206, 250),
        "lightslategray" | "lightslategrey" => (119, 136, 153),
        "lightsteelblue" => (176, 196, 222),
        "lightyellow" => (255, 255, 224),
        "limegreen" => (50, 205, 50),
        "linen" => (250, 240, 230),
        "mediumaquamarine" => (102, 205, 170),
        "mediumblue" => (0, 0, 205),
        "mediumorchid" => (186, 85, 211),
        "mediumpurple" => (147, 112, 219),
        "mediumseagreen" => (60, 179, 113),
        "mediumslateblue" => (123, 104, 238),
        "mediumspringgreen" => (0, 250, 154),
        "mediumturquoise" => (72, 209, 204),
        "mediumvioletred" => (199, 21, 133),
        "midnightblue" => (25, 25, 112),
        "mintcream" => (245, 255, 250),
        "mistyrose" => (255, 228, 225),
        "moccasin" => (255, 228, 181),
        "navajowhite" => (255, 222, 173),
        "oldlace" => (253, 245, 230),
        "olivedrab" => (107, 142, 35),
        "orangered" => (255, 69, 0),
        "orchid" => (218, 112, 214),
        "palegoldenrod" => (238, 232, 170),
        "palegreen" => (152, 251, 152),
        "paleturquoise" => (175, 238, 238),
        "palevioletred" => (219, 112, 147),
        "papayawhip" => (255, 239, 213),
        "peachpuff" => (255, 218, 185),
        "peru" => (205, 133, 63),
        "pink" => (255, 192, 203),
        "plum" => (221, 160, 221),
        "powderblue" => (176, 224, 230),
        "rebeccapurple" => (102, 51, 153),
        "rosybrown" => (188, 143, 143),
        "royalblue" => (65, 105, 225),
        "saddlebrown" => (139, 69, 19),
        "salmon" => (250, 128, 114),
        "sandybrown" => (244, 164, 96),
        "seagreen" => (46, 139, 87),
        "seashell" => (255, 245, 238),
        "sienna" => (160, 82, 45),
        "skyblue" => (135, 206, 235),
        "slateblue" => (106, 90, 205),
        "slategray" | "slategrey" => (112, 128, 144),
        "snow" => (255, 250, 250),
        "springgreen" => (0, 255, 127),
        "steelblue" => (70, 130, 180),
        "tan" => (210, 180, 140),
        "thistle" => (216, 191, 216),
        "tomato" => (255, 99, 71),
        "turquoise" => (64, 224, 208),
        "violet" => (238, 130, 238),
        "wheat" => (245, 222, 179),
        "whitesmoke" => (245, 245, 245),
        "yellowgreen" => (154, 205, 50),
        _ => return None,
    };
    Some(Color::rgb(c.0, c.1, c.2))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hex_forms() {
        assert_eq!(Color::parse("#fff"), Some(Color::WHITE));
        assert_eq!(Color::parse("#FFF"), Some(Color::WHITE));
        assert_eq!(Color::parse("#1a2b3c"), Some(Color::rgb(0x1a, 0x2b, 0x3c)));
        assert_eq!(
            Color::parse("#11223344"),
            Some(Color::rgba(0x11, 0x22, 0x33, 0x44))
        );
        assert_eq!(Color::parse("#12g"), None);
    }

    #[test]
    fn parses_rgb_and_hsl() {
        assert_eq!(Color::parse("rgb(1,2,3)"), Some(Color::rgb(1, 2, 3)));
        assert_eq!(Color::parse("rgb(0% 50% 100%)"), Some(Color::rgb(0, 128, 255)));
        assert_eq!(
            Color::parse("rgba(255, 0, 0, 0.5)"),
            Some(Color::rgba(255, 0, 0, 128))
        );
        assert_eq!(Color::parse("hsl(0, 100%, 50%)"), Some(Color::rgb(255, 0, 0)));
        assert_eq!(Color::parse("hsl(120,100%,50%)"), Some(Color::rgb(0, 255, 0)));
        assert_eq!(Color::parse("hsl(240 100% 50%)"), Some(Color::rgb(0, 0, 255)));
    }

    #[test]
    fn named_colors_and_transparency() {
        assert_eq!(Color::parse("tomato"), Some(Color::rgb(255, 99, 71)));
        assert_eq!(Color::parse("transparent"), Some(Color::TRANSPARENT));
        assert_eq!(Color::parse("rebeccapurple"), Some(Color::rgb(102, 51, 153)));
        assert_eq!(Color::parse("notacolor"), None);
    }

    #[test]
    fn blending_is_source_over() {
        let red = Color::rgba(255, 0, 0, 128);
        let out = red.blend_over(Color::WHITE);
        assert!(out.r > 250);
        assert!(out.g > 100 && out.g < 140);
        assert_eq!(Color::BLACK.blend_over(Color::WHITE), Color::BLACK);
    }

    #[test]
    fn argb_roundtrip() {
        let c = Color::rgba(1, 2, 3, 4);
        assert_eq!(Color::from_argb(c.to_argb()), c);
    }
}
