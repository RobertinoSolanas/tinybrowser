//! Resolving `calc()`, `var()` and relative lengths to pixels.
//!
//! The parser stores `calc()` as a term list; this module turns that into a
//! concrete number once the styling context (font size, viewport, containing
//! block, custom property values) is known.

use crate::value::{CalcExpr, CalcTerm, Unit, Value};

/// Everything needed to resolve relative values.
#[derive(Debug, Clone, Copy)]
pub struct Resolved {
    /// computed font size of the element (for `em`)
    pub font_size: f32,
    /// root font size (for `rem`)
    pub root_font_size: f32,
    /// viewport width/height in px
    pub vw: f32,
    pub vh: f32,
    /// containing block size in the axis being resolved (for `%`)
    pub basis: f32,
}

impl Resolved {
    pub fn new(font_size: f32, root_font_size: f32, vw: f32, vh: f32, basis: f32) -> Self {
        Resolved {
            font_size,
            root_font_size,
            vw,
            vh,
            basis,
        }
    }

    /// Resolve a single length to px.
    pub fn length(&self, value: f32, unit: Unit) -> f32 {
        match unit {
            Unit::Px | Unit::PxConverted | Unit::None => value,
            Unit::Em => value * self.font_size,
            Unit::Rem => value * self.root_font_size,
            Unit::Percent => value * self.basis / 100.0,
            Unit::Vw => value * self.vw / 100.0,
            Unit::Vh => value * self.vh / 100.0,
            Unit::Vmin => value * self.vw.min(self.vh) / 100.0,
            Unit::Vmax => value * self.vw.max(self.vh) / 100.0,
        }
    }

    /// Resolve a value that represents a length (`12px`, `50%`, `calc(...)`).
    ///
    /// `lookup` resolves custom properties (`var(--x)`).
    pub fn resolve_length<F: Fn(&str) -> Option<String>>(
        &self,
        v: &Value,
        lookup: &F,
    ) -> Option<f32> {
        match v {
            Value::Length(n, u) => Some(self.length(*n, *u)),
            Value::Number(n) => Some(*n),
            Value::Calc(expr) => self.calc(expr, lookup),
            Value::Function { name, args } => match name.as_str() {
                "var" => {
                    let name = args.first().and_then(|a| a.as_str())?;
                    let raw = lookup(name).or_else(|| {
                        // fallback: the first component after the comma
                        args.iter()
                            .skip(1)
                            .find(|v| !v.is_keyword(","))
                            .map(|v| v.to_css())
                    })?;
                    let parsed = crate::value::parse_value(&raw);
                    let first = parsed.parts.first()?;
                    self.resolve_length(first, lookup)
                }
                "min" => {
                    let mut best = f32::INFINITY;
                    for a in args {
                        if a.is_keyword(",") {
                            continue;
                        }
                        if let Some(v) = self.resolve_length(a, lookup) {
                            best = best.min(v);
                        }
                    }
                    Some(best)
                }
                "max" => {
                    let mut best = f32::NEG_INFINITY;
                    for a in args {
                        if a.is_keyword(",") {
                            continue;
                        }
                        if let Some(v) = self.resolve_length(a, lookup) {
                            best = best.max(v);
                        }
                    }
                    if best == f32::NEG_INFINITY {
                        None
                    } else {
                        Some(best)
                    }
                }
                "clamp" => {
                    let vals: Vec<f32> = args
                        .iter()
                        .filter(|a| !a.is_keyword(","))
                        .filter_map(|a| self.resolve_length(a, lookup))
                        .collect();
                    if vals.len() == 3 {
                        Some(vals[0].max(vals[1]).min(vals[2]))
                    } else {
                        None
                    }
                }
                _ => None,
            },
            _ => None,
        }
    }

    /// Evaluate a calc expression to px.
    pub fn calc<F: Fn(&str) -> Option<String>>(&self, expr: &CalcExpr, lookup: &F) -> Option<f32> {
        let mut total = 0.0f32;
        let mut started = false;
        for (sign, term) in &expr.terms {
            let v = match term {
                CalcTerm::Length(n, u, mul) => self.length(*n, *u) * mul,
                CalcTerm::Number(n) => *n,
                CalcTerm::Calc(inner, mul) => self.calc(inner, lookup)? * mul,
                CalcTerm::Var { name, fallback, mul } => {
                    let raw = lookup(name).or_else(|| fallback.as_ref().map(|f| f.to_css()))?;
                    let parsed = crate::value::parse_value(&raw);
                    let first = parsed.parts.first()?;
                    self.resolve_length(first, lookup)? * mul
                }
            };
            total += sign * v;
            started = true;
        }
        if started {
            Some(total)
        } else {
            None
        }
    }

    /// `true` when a value needs a percentage basis (width/height/margin).
    pub fn is_percentage(v: &Value) -> bool {
        matches!(v, Value::Length(_, Unit::Percent))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> Resolved {
        Resolved::new(16.0, 16.0, 1000.0, 800.0, 500.0)
    }

    fn val(css: &str) -> Value {
        let mut parts = crate::value::parse_value(css).parts;
        parts.remove(0)
    }

    fn lookup(name: &str) -> Option<String> {
        match name {
            "--gap" => Some("8px".to_string()),
            "--wide" => Some("50%".to_string()),
            _ => None,
        }
    }

    #[test]
    fn absolute_and_relative_units() {
        let c = ctx();
        assert_eq!(c.resolve_length(&val("12px"), &lookup), Some(12.0));
        assert_eq!(c.resolve_length(&val("2em"), &lookup), Some(32.0));
        assert_eq!(c.resolve_length(&val("1.5rem"), &lookup), Some(24.0));
        assert_eq!(c.resolve_length(&val("10%"), &lookup), Some(50.0));
        assert_eq!(c.resolve_length(&val("10vw"), &lookup), Some(100.0));
        assert_eq!(c.resolve_length(&val("50vh"), &lookup), Some(400.0));
        assert_eq!(c.resolve_length(&val("1vmin"), &lookup), Some(8.0));
    }

    #[test]
    fn calc_expressions() {
        let c = ctx();
        assert_eq!(
            c.resolve_length(&val("calc(100% - 2rem)"), &lookup),
            Some(468.0)
        );
        assert_eq!(
            c.resolve_length(&val("calc(10px + 2em)"), &lookup),
            Some(42.0)
        );
        assert_eq!(
            c.resolve_length(&val("calc(var(--gap) * 2)"), &lookup),
            Some(16.0)
        );
        assert_eq!(
            c.resolve_length(&val("calc(var(--gap) + 4px)"), &lookup),
            Some(12.0)
        );
    }

    #[test]
    fn var_with_fallback() {
        let c = ctx();
        assert_eq!(
            c.resolve_length(&val("var(--gap)"), &lookup),
            Some(8.0)
        );
        assert_eq!(
            c.resolve_length(&val("var(--missing, 7px)"), &lookup),
            Some(7.0)
        );
        assert_eq!(c.resolve_length(&val("var(--missing)"), &lookup), None);
    }

    #[test]
    fn min_max_clamp() {
        let c = ctx();
        assert_eq!(
            c.resolve_length(&val("min(100px, 50%)"), &lookup),
            Some(100.0)
        );
        assert_eq!(
            c.resolve_length(&val("max(100px, 50%)"), &lookup),
            Some(250.0)
        );
        assert_eq!(
            c.resolve_length(&val("clamp(50px, 50%, 120px)"), &lookup),
            Some(120.0)
        );
    }

    #[test]
    fn percentage_detection() {
        assert!(Resolved::is_percentage(&val("33%")));
        assert!(!Resolved::is_percentage(&val("33px")));
    }
}
