//! `@media` queries: parsing and evaluation against the viewport.

use crate::value::{parse_value, Unit, Value};
use tb_core::ColorScheme;

/// Everything a media query can ask about.
#[derive(Debug, Clone, Copy)]
pub struct Viewport {
    pub width: f32,
    pub height: f32,
    pub color_scheme: ColorScheme,
    /// `prefers-reduced-motion: reduce`
    pub reduced_motion: bool,
}

impl Default for Viewport {
    fn default() -> Self {
        Viewport {
            width: 1280.0,
            height: 1024.0,
            color_scheme: ColorScheme::Light,
            reduced_motion: false,
        }
    }
}

/// Comparison used by range syntax (`@media (400px < width)`) — we only
/// implement the classic `feature: value` form plus min/max.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CondOp {
    Colon,
    Min,
    Max,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Condition {
    pub name: String,
    pub op: CondOp,
    pub value: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct MediaQuery {
    pub negate: bool,
    pub only: bool,
    /// `screen`, `print`, `all`, ... (`None` means "any media")
    pub qualifier: Option<String>,
    pub conditions: Vec<Condition>,
    pub text: String,
}

impl MediaQuery {
    /// Evaluate against our viewport. Unknown features evaluate to `true`
    /// (permissive: a page should render rather than disappear).
    pub fn matches(&self, vp: &Viewport) -> bool {
        let mut result = true;
        if let Some(q) = &self.qualifier {
            result = match q.as_str() {
                "all" => true,
                "screen" => true,
                "print" => false,
                _ => true,
            };
        }
        if result {
            for c in &self.conditions {
                result = self.condition_matches(c, vp);
                if !result {
                    break;
                }
            }
        }
        if self.negate {
            !result
        } else {
            result
        }
    }

    fn condition_matches(&self, c: &Condition, vp: &Viewport) -> bool {
        let px = |v: &Option<Value>| -> Option<f32> {
            let (n, u) = v.as_ref()?.as_length()?;
            Some(match u {
                Unit::Px => n,
                Unit::Em | Unit::Rem => n * 16.0,
                Unit::Percent => n,
                Unit::Vw => n * vp.width / 100.0,
                Unit::Vh => n * vp.height / 100.0,
                Unit::Vmin => n * vp.width.min(vp.height) / 100.0,
                Unit::Vmax => n * vp.width.max(vp.height) / 100.0,
                Unit::None | Unit::PxConverted => n,
            })
        };
        match c.name.as_str() {
            "width" => px(&c.value).map(|w| (vp.width - w).abs() < 0.5).unwrap_or(true),
            "min-width" => px(&c.value).map(|w| vp.width >= w).unwrap_or(true),
            "max-width" => px(&c.value).map(|w| vp.width <= w).unwrap_or(true),
            "height" => px(&c.value).map(|h| (vp.height - h).abs() < 0.5).unwrap_or(true),
            "min-height" => px(&c.value).map(|h| vp.height >= h).unwrap_or(true),
            "max-height" => px(&c.value).map(|h| vp.height <= h).unwrap_or(true),
            "device-width" | "device-height" => true,
            "orientation" => {
                let want = c.value.as_ref().and_then(|v| v.as_str()).unwrap_or("");
                (want == "landscape") == (vp.width >= vp.height)
            }
            "aspect-ratio" | "device-aspect-ratio" => true,
            "color" => true,
            "color-gamut" => true,
            "monochrome" => false,
            "resolution" | "min-resolution" | "max-resolution" => true,
            "prefers-color-scheme" => {
                let want = c.value.as_ref().and_then(|v| v.as_str()).unwrap_or("light");
                match want {
                    "dark" => vp.color_scheme == ColorScheme::Dark,
                    _ => vp.color_scheme == ColorScheme::Light,
                }
            }
            "prefers-reduced-motion" => {
                let want = c.value.as_ref().and_then(|v| v.as_str()).unwrap_or("no-preference");
                (want == "reduce") == vp.reduced_motion
            }
            "prefers-contrast" => true,
            "hover" | "any-hover" | "pointer" | "any-pointer" => {
                let v = c.value.as_ref().and_then(|x| x.as_str()).unwrap_or("hover");
                !matches!(v, "none")
            }
            "display-mode" => true,
            "update" | "update-frequency" => true,
            "scripting" => true,
            // unknown feature: be permissive
            _ => true,
        }
    }
}

/// Parse `screen and (min-width: 700px), print`.
pub fn parse_media_list(input: &str) -> Vec<MediaQuery> {
    let mut out = Vec::new();
    for chunk in crate::selector::split_top_level(input, ',') {
        let text = chunk.trim().to_string();
        if text.is_empty() {
            continue;
        }
        out.push(parse_media_query(&text));
    }
    if out.is_empty() {
        out.push(MediaQuery {
            qualifier: Some("all".into()),
            text: input.to_string(),
            ..Default::default()
        });
    }
    out
}

fn parse_media_query(text: &str) -> MediaQuery {
    let mut q = MediaQuery {
        text: text.to_string(),
        ..Default::default()
    };
    let mut rest = text.trim();
    if let Some(r) = rest.strip_prefix("not ") {
        q.negate = true;
        rest = r.trim();
    }
    if let Some(r) = rest.strip_prefix("only ") {
        q.only = true;
        rest = r.trim();
    }
    // qualifier
    if !rest.starts_with('(') {
        let cut = rest.find('(').unwrap_or(rest.len());
        let (qual, tail) = rest.split_at(cut);
        let qual = qual.trim().trim_end_matches("and").trim();
        if !qual.is_empty() && !qual.eq_ignore_ascii_case("and") {
            q.qualifier = Some(qual.to_ascii_lowercase());
        }
        rest = tail.trim();
    }
    // conditions: `and (...)` separated list
    for part in split_and(rest) {
        let p = part.trim();
        if p.is_empty() {
            continue;
        }
        let inner = p.trim_start_matches('(').trim_end_matches(')').trim();
        if inner.is_empty() {
            continue;
        }
        // `min-width: 700px`
        if let Some((k, v)) = inner.split_once(':') {
            let name = k.trim().to_ascii_lowercase();
            let val = parse_value(v.trim()).parts.into_iter().next();
            let op = if name.starts_with("min-") {
                CondOp::Min
            } else if name.starts_with("max-") {
                CondOp::Max
            } else {
                CondOp::Colon
            };
            q.conditions.push(Condition {
                name,
                op,
                value: val,
            });
        } else {
            // bare feature, e.g. `(hover)`
            q.conditions.push(Condition {
                name: inner.to_ascii_lowercase(),
                op: CondOp::Colon,
                value: None,
            });
        }
    }
    q
}

/// Split on ` and ` outside parentheses.
fn split_and(input: &str) -> Vec<String> {
    let chars: Vec<char> = input.chars().collect();
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut cur = String::new();
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '(' => {
                depth += 1;
                cur.push(c);
            }
            ')' => {
                depth -= 1;
                cur.push(c);
            }
            _ if depth == 0 && c.to_ascii_lowercase() == 'a' => {
                // possible " and "
                let word: String = chars[i..(i + 5).min(chars.len())].iter().collect();
                if word.eq_ignore_ascii_case(" and ") {
                    out.push(std::mem::take(&mut cur));
                    i += 5;
                    continue;
                }
                cur.push(c);
            }
            _ => cur.push(c),
        }
        i += 1;
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vp(w: f32, h: f32) -> Viewport {
        Viewport {
            width: w,
            height: h,
            ..Default::default()
        }
    }

    #[test]
    fn media_qualifiers() {
        let q = &parse_media_list("screen")[0];
        assert!(q.matches(&vp(1280.0, 800.0)));
        let p = &parse_media_list("print")[0];
        assert!(!p.matches(&vp(1280.0, 800.0)));
        let all = &parse_media_list("all")[0];
        assert!(all.matches(&vp(1.0, 1.0)));
    }

    #[test]
    fn min_max_width() {
        let q = &parse_media_list("screen and (max-width: 768px)")[0];
        assert!(q.matches(&vp(400.0, 800.0)));
        assert!(!q.matches(&vp(1280.0, 800.0)));
        let q2 = &parse_media_list("(min-width: 600px)")[0];
        assert!(q2.matches(&vp(1280.0, 800.0)));
        assert!(!q2.matches(&vp(599.0, 800.0)));
    }

    #[test]
    fn comma_separated_queries() {
        let qs = parse_media_list("screen and (min-width: 900px), screen and (max-width: 300px)");
        assert_eq!(qs.len(), 2);
        assert!(qs[0].matches(&vp(1000.0, 800.0)));
        assert!(qs[1].matches(&vp(200.0, 800.0)));
        assert!(!qs[1].matches(&vp(1000.0, 800.0)));
    }

    #[test]
    fn em_units_and_orientation() {
        let q = &parse_media_list("(min-width: 40em)")[0];
        assert!(q.matches(&vp(700.0, 400.0)));
        assert!(!q.matches(&vp(600.0, 400.0)));
        let o = &parse_media_list("(orientation: landscape)")[0];
        assert!(o.matches(&vp(800.0, 400.0)));
        assert!(!o.matches(&vp(400.0, 800.0)));
    }

    #[test]
    fn color_scheme_and_reduced_motion() {
        let q = &parse_media_list("(prefers-color-scheme: dark)")[0];
        let mut v = Viewport::default();
        assert!(!q.matches(&v));
        v.color_scheme = ColorScheme::Dark;
        assert!(q.matches(&v));
        let r = &parse_media_list("(prefers-reduced-motion: reduce)")[0];
        assert!(!r.matches(&Viewport::default()));
        assert!(r.matches(&Viewport {
            reduced_motion: true,
            ..Default::default()
        }));
    }

    #[test]
    fn negation_and_unknown_features() {
        let q = &parse_media_list("not all")[0];
        assert!(!q.matches(&vp(800.0, 600.0)));
        let u = &parse_media_list("(some-unknown-feature: 4)")[0];
        assert!(u.matches(&vp(800.0, 600.0)), "unknown features are permissive");
    }
}
