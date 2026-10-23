//! CSS values, as far as a small browser needs them.
//!
//! A value is a space separated list of components; each component is a
//! keyword, number, length, percentage, colour, string or function call
//! (`var()`, `calc()`, `rgb()`, `translate()` ...).

use crate::selector::{is_ident_char, is_ident_start};
use tb_core::Color;

/// Length units (relative ones are resolved by `tb-style`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Unit {
    Px,
    Em,
    Rem,
    Percent,
    Vw,
    Vh,
    Vmin,
    Vmax,
    /// unitless number
    None,
    /// `pt`, `cm`, ... converted to px at parse time
    PxConverted,
}

impl Unit {
    pub fn from(s: &str) -> Option<Unit> {
        match s {
            "px" => Some(Unit::Px),
            "em" => Some(Unit::Em),
            "rem" => Some(Unit::Rem),
            "%" => Some(Unit::Percent),
            "vw" => Some(Unit::Vw),
            "vh" => Some(Unit::Vh),
            "vmin" => Some(Unit::Vmin),
            "vmax" => Some(Unit::Vmax),
            "" => Some(Unit::None),
            "pt" => Some(Unit::PxConverted),
            "pc" => Some(Unit::PxConverted),
            "in" => Some(Unit::PxConverted),
            "cm" => Some(Unit::PxConverted),
            "mm" => Some(Unit::PxConverted),
            "ex" | "ch" | "cap" | "ic" | "lh" | "rlh" => Some(Unit::Em),
            "s" | "ms" | "deg" | "rad" | "turn" | "grad" => Some(Unit::None),
            _ => None,
        }
    }

    /// Factor to multiply the value with to get px (for absolute units only).
    pub fn absolute_factor(&self, raw: &str) -> f32 {
        match raw {
            "pt" => 96.0 / 72.0,
            "pc" => 96.0 / 6.0,
            "in" => 96.0,
            "cm" => 96.0 / 2.54,
            "mm" => 96.0 / 25.4,
            _ => 1.0,
        }
    }

    pub fn is_relative(&self) -> bool {
        matches!(
            self,
            Unit::Em | Unit::Rem | Unit::Percent | Unit::Vw | Unit::Vh | Unit::Vmin | Unit::Vmax
        )
    }
}

/// A term of a `calc()` expression.
#[derive(Debug, Clone, PartialEq)]
pub enum CalcTerm {
    /// additive term: value, unit, multiplier (`calc(var(--x) * 2)`)
    Length(f32, Unit, f32),
    Number(f32),
    Var {
        name: String,
        fallback: Option<Box<Value>>,
        mul: f32,
    },
    Calc(Box<CalcExpr>, f32),
}

/// A parsed `calc()` expression: sum of signed terms plus products.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CalcExpr {
    pub terms: Vec<(f32, CalcTerm)>,
}

/// One component of a declaration value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Keyword(String),
    Number(f32),
    Length(f32, Unit),
    Color(Color),
    Str(String),
    Function {
        name: String,
        args: Vec<Value>,
    },
    Calc(CalcExpr),
    /// preserved raw text (unknown at-rule garbage, filters, ...)
    Raw(String),
}

impl Value {
    pub fn keyword(&self) -> Option<&str> {
        match self {
            Value::Keyword(k) => Some(k),
            _ => None,
        }
    }

    pub fn is_keyword(&self, k: &str) -> bool {
        matches!(self, Value::Keyword(v) if v == k)
    }

    pub fn as_length(&self) -> Option<(f32, Unit)> {
        match self {
            Value::Length(v, u) => Some((*v, *u)),
            Value::Number(n) => Some((*n, Unit::None)),
            _ => None,
        }
    }

    pub fn as_number(&self) -> Option<f32> {
        match self {
            Value::Number(n) => Some(*n),
            Value::Length(v, Unit::None) => Some(*v),
            _ => None,
        }
    }

    pub fn as_color(&self) -> Option<Color> {
        match self {
            Value::Color(c) => Some(*c),
            Value::Keyword(k) => Color::parse(k),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Keyword(k) => Some(k),
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn function(&self) -> Option<(&str, &[Value])> {
        match self {
            Value::Function { name, args } => Some((name.as_str(), args.as_slice())),
            _ => None,
        }
    }

    /// `true` for `inherit`.
    pub fn is_inherit(&self) -> bool {
        self.is_keyword("inherit")
    }

    /// `initial` / `unset` / `revert` — we treat all as "use the initial value".
    pub fn is_reset(&self) -> bool {
        matches!(self, Value::Keyword(k) if matches!(k.as_str(), "initial" | "unset" | "revert" | "revert-layer"))
    }

    /// String form (used for `cssText`, diagnostics and font families).
    pub fn to_css(&self) -> String {
        match self {
            Value::Keyword(k) => k.clone(),
            Value::Number(n) => fmt_num(*n),
            Value::Length(v, u) => match u {
                Unit::Px => format!("{}px", fmt_num(*v)),
                Unit::Em => format!("{}em", fmt_num(*v)),
                Unit::Rem => format!("{}rem", fmt_num(*v)),
                Unit::Percent => format!("{}%", fmt_num(*v)),
                Unit::Vw => format!("{}vw", fmt_num(*v)),
                Unit::Vh => format!("{}vh", fmt_num(*v)),
                Unit::Vmin => format!("{}vmin", fmt_num(*v)),
                Unit::Vmax => format!("{}vmax", fmt_num(*v)),
                Unit::None => fmt_num(*v),
                Unit::PxConverted => format!("{}px", fmt_num(*v)),
            },
            Value::Color(c) => c.to_string(),
            Value::Str(s) => format!("\"{s}\""),
            Value::Function { name, args } => {
                let inner: Vec<String> = args.iter().map(|a| a.to_css()).collect();
                format!("{name}({})", inner.join(", "))
            }
            Value::Calc(c) => format!(
                "calc({})",
                c.terms
                    .iter()
                    .map(|(s, t)| {
                        let sign = if *s < 0.0 { "-" } else { "+" };
                        format!(" {sign} {}", term_css(t))
                    })
                    .collect::<String>()
            ),
            Value::Raw(r) => r.clone(),
        }
    }
}

/// Serialise one calc term, **including** its multiplier (otherwise a
/// round-trip through `to_css()` would silently drop `calc(var(--x) * 2)`).
fn term_css(t: &CalcTerm) -> String {
    match t {
        CalcTerm::Length(v, u, mul) => Value::Length(v * mul, *u).to_css(),
        CalcTerm::Number(n) => fmt_num(*n),
        CalcTerm::Var {
            name,
            fallback,
            mul,
        } => {
            let base = match fallback {
                Some(f) => format!("var({name}, {})", f.to_css()),
                None => format!("var({name})"),
            };
            if *mul == 1.0 {
                base
            } else {
                format!("{base} * {}", fmt_num(*mul))
            }
        }
        CalcTerm::Calc(c, mul) => {
            let inner = Value::Calc((**c).clone()).to_css();
            if *mul == 1.0 {
                inner
            } else {
                format!("{inner} * {}", fmt_num(*mul))
            }
        }
    }
}

/// A test-only helper: `true` when a value carries a percentage.
pub fn has_percentage(v: &Value) -> bool {
    match v {
        Value::Length(_, Unit::Percent) => true,
        Value::Calc(c) => c
            .terms
            .iter()
            .any(|(_, t)| matches!(t, CalcTerm::Length(_, Unit::Percent, _))),
        _ => false,
    }
}

pub fn fmt_num(n: f32) -> String {
    if n == n.round() && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        let s = format!("{n:.4}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// A parsed declaration value: components plus helpers.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DeclarationValue {
    pub parts: Vec<Value>,
    pub important: bool,
}

impl DeclarationValue {
    pub fn new() -> Self {
        Self::default()
    }

    /// `a b / c` style splits are kept in `parts`.
    pub fn first(&self) -> Option<&Value> {
        self.parts.first()
    }

    pub fn has_keyword(&self, k: &str) -> bool {
        self.parts.iter().any(|p| p.is_keyword(k))
    }

    /// Iterate the comma separated groups (`background-image: a, b`).
    pub fn groups(&self) -> Vec<Vec<Value>> {
        let mut out = vec![Vec::new()];
        for p in &self.parts {
            if p.is_keyword(",") {
                out.push(Vec::new());
            } else {
                out.last_mut().unwrap().push(p.clone());
            }
        }
        out
    }

    pub fn to_css(&self) -> String {
        let mut s = self
            .parts
            .iter()
            .map(|p| match p {
                Value::Keyword(k) if k == "," => ", ".to_string(),
                other => other.to_css(),
            })
            .collect::<Vec<_>>()
            .join(" ")
            .replace(" , ", ", ");
        if self.important {
            s.push_str(" !important");
        }
        s
    }

    /// The value as a single font-family list.
    pub fn font_family(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut current = String::new();
        for p in &self.parts {
            if p.is_keyword(",") {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
                continue;
            }
            match p {
                Value::Str(s) => current.push_str(s),
                Value::Keyword(k) => {
                    if !current.is_empty() {
                        current.push(' ');
                    }
                    current.push_str(k);
                }
                _ => {}
            }
        }
        if !current.is_empty() {
            out.push(current);
        }
        out
    }
}

/// Parse a declaration value (`1px solid red`, `calc(100% - 2rem)`, ...).
pub fn parse_value(input: &str) -> DeclarationValue {
    let chars: Vec<char> = input.chars().collect();
    let mut p = ValueParser { c: &chars, i: 0 };
    let mut parts = Vec::new();
    let mut important = false;
    while let Some(v) = p.next_component() {
        if v.is_keyword("!") {
            // !important
            if let Some(Value::Keyword(k)) = p.next_component() {
                if k == "important" {
                    important = true;
                }
            }
            break;
        }
        parts.push(v);
    }
    DeclarationValue { parts, important }
}

struct ValueParser<'a> {
    c: &'a [char],
    i: usize,
}

impl<'a> ValueParser<'a> {
    fn peek(&self) -> Option<char> {
        self.c.get(self.i).copied()
    }

    fn skip_ws(&mut self) {
        while matches!(
            self.peek(),
            Some(' ') | Some('\t') | Some('\n') | Some('\r') | Some('\u{c}')
        ) {
            self.i += 1;
        }
    }

    fn next_component(&mut self) -> Option<Value> {
        self.skip_ws();
        let c = self.peek()?;
        match c {
            ',' => {
                self.i += 1;
                Some(Value::Keyword(",".into()))
            }
            '"' | '\'' => self.parse_string().map(Value::Str),
            '#' => self.parse_hash(),
            '(' => {
                // parenthesised group
                self.i += 1;
                let mut inner = Vec::new();
                while let Some(v) = self.next_component() {
                    if v.is_keyword(")") {
                        break;
                    }
                    inner.push(v);
                }
                Some(Value::Function {
                    name: String::new(),
                    args: inner,
                })
            }
            '/' => {
                self.i += 1;
                Some(Value::Keyword("/".into()))
            }
            c if c.is_ascii_digit()
                || c == '.'
                || ((c == '+' || c == '-') && self.is_number_start()) =>
            {
                self.parse_number()
            }
            c if is_ident_start(c) => self.parse_ident_or_function(),
            _ => {
                self.i += 1;
                Some(Value::Keyword(c.to_string()))
            }
        }
    }

    fn is_number_start(&self) -> bool {
        matches!(self.c.get(self.i + 1), Some(d) if d.is_ascii_digit() || *d == '.')
    }

    /// `e` is only an exponent when digits follow (`1e3`, not `2em`).
    fn is_exponent(&self) -> bool {
        let s: String = self.c[self.i..(self.i + 3).min(self.c.len())]
            .iter()
            .collect();
        let bytes: Vec<char> = s.chars().collect();
        match bytes.as_slice() {
            [_, d, ..] if d.is_ascii_digit() => true,
            [_, sign, d, ..] if (sign == &'+' || sign == &'-') && d.is_ascii_digit() => true,
            _ => false,
        }
    }

    fn parse_string(&mut self) -> Option<String> {
        let quote = self.c.get(self.i).copied()?;
        self.i += 1;
        let mut out = String::new();
        while let Some(c) = self.peek() {
            self.i += 1;
            if c == quote {
                return Some(out);
            }
            if c == '\\' {
                if let Some(n) = self.peek() {
                    self.i += 1;
                    out.push(n);
                }
                continue;
            }
            if c == '\n' {
                // unterminated string: keep going like a browser (value is junk)
                return Some(out);
            }
            out.push(c);
        }
        Some(out)
    }

    fn parse_hash(&mut self) -> Option<Value> {
        self.i += 1; // '#'
        let start = self.i;
        while matches!(self.peek(), Some(c) if c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            self.i += 1;
        }
        let text: String = self.c[start..self.i].iter().collect();
        if let Some(color) = Color::parse(&format!("#{text}")) {
            return Some(Value::Color(color));
        }
        Some(Value::Keyword(format!("#{text}")))
    }

    fn parse_number(&mut self) -> Option<Value> {
        let mut s = String::new();
        if matches!(self.peek(), Some('+') | Some('-')) {
            s.push(self.c[self.i]);
            self.i += 1;
        }
        let mut dots = 0;
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                s.push(c);
                self.i += 1;
            } else if c == '.' && dots == 0 {
                dots += 1;
                s.push(c);
                self.i += 1;
            } else if (c == 'e' || c == 'E') && self.is_exponent() {
                s.push(c);
                self.i += 1;
                if matches!(self.peek(), Some('+') | Some('-')) {
                    s.push(self.c[self.i]);
                    self.i += 1;
                }
            } else {
                break;
            }
        }
        let num: f32 = s.parse().unwrap_or(0.0);
        // unit
        let unit_start = self.i;
        if matches!(self.peek(), Some('%')) {
            self.i += 1;
            return Some(Value::Length(num, Unit::Percent));
        }
        while matches!(self.peek(), Some(c) if c.is_ascii_alphabetic() || c == '-' || c == '%') {
            self.i += 1;
        }
        let unit_raw: String = self.c[unit_start..self.i].iter().collect();
        let unit = Unit::from(&unit_raw).unwrap_or(Unit::None);
        let value = if unit == Unit::PxConverted {
            num * unit.absolute_factor(&unit_raw)
        } else {
            num
        };
        if unit == Unit::None {
            return Some(Value::Number(value));
        }
        Some(Value::Length(value, unit))
    }

    fn parse_ident_or_function(&mut self) -> Option<Value> {
        let start = self.i;
        while let Some(c) = self.peek() {
            if is_ident_char(c) {
                self.i += 1;
            } else {
                break;
            }
        }
        let name: String = self.c[start..self.i].iter().collect();
        if self.peek() == Some('(') {
            self.i += 1;
            let mut args = Vec::new();
            while let Some(v) = self.next_component() {
                if v.is_keyword(")") {
                    break;
                }
                args.push(v);
            }
            let lower = name.to_ascii_lowercase();
            return Some(match lower.as_str() {
                "calc" => Value::Calc(parse_calc("calc", args)),
                "rgb" | "rgba" | "hsl" | "hsla" => {
                    let css = format!(
                        "{}({})",
                        lower,
                        args.iter()
                            .map(|a| a.to_css())
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                    match Color::parse(&css) {
                        Some(c) => Value::Color(c),
                        None => Value::Function { name: lower, args },
                    }
                }
                other => Value::Function {
                    name: other.to_string(),
                    args,
                },
            });
        }
        Some(Value::Keyword(name.to_ascii_lowercase()))
    }
}

/// Turn `calc(...)`/`min(...)`/`max(...)`/`clamp(...)` arguments into a term list.
///
/// Only the parts a real page uses are supported: `+ - * /`, nested
/// parentheses and `var()` fallbacks. `min()`/`max()` are stored as nested
/// calcs and resolved by `tb-style`.
pub fn parse_calc(name: &str, args: Vec<Value>) -> CalcExpr {
    let mut terms: Vec<(f32, CalcTerm)> = Vec::new();
    let mut sign = 1.0f32;
    let mut pending_op: Option<char> = None;
    for a in args {
        match &a {
            Value::Keyword(k) if k == "+" => {
                sign = 1.0;
                pending_op = None;
            }
            Value::Keyword(k) if k == "-" => {
                sign = -1.0;
                pending_op = None;
            }
            Value::Keyword(k) if k == "*" => pending_op = Some('*'),
            Value::Keyword(k) if k == "/" => pending_op = Some('/'),
            _ => {
                let term = match &a {
                    Value::Length(v, u) => Some(CalcTerm::Length(*v, *u, 1.0)),
                    Value::Number(n) => Some(CalcTerm::Number(*n)),
                    Value::Function { name: f, args } if f == "var" => {
                        let var_name = args
                            .first()
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let fallback = if args.len() > 1 {
                            let mut f = DeclarationValue::new();
                            f.parts = args[1..].to_vec();
                            Some(Box::new(Value::Raw(f.to_css())))
                        } else {
                            None
                        };
                        Some(CalcTerm::Var {
                            name: var_name,
                            fallback,
                            mul: 1.0,
                        })
                    }
                    Value::Function { name: f, args }
                        if matches!(f.as_str(), "calc" | "min" | "max" | "clamp") =>
                    {
                        Some(CalcTerm::Calc(Box::new(parse_calc(f, args.clone())), 1.0))
                    }
                    Value::Keyword(kk) if kk == ")" => None,
                    _ => None,
                };
                if let Some(term) = term {
                    match pending_op.take() {
                        Some(op @ ('*' | '/')) => {
                            // `length * number` / `length / number`
                            let factor = match (&term, op) {
                                (CalcTerm::Number(n), '*') => *n,
                                (CalcTerm::Number(n), '/') if *n != 0.0 => 1.0 / *n,
                                _ => 1.0,
                            };
                            apply_factor_to_last(&mut terms, factor);
                        }
                        _ => terms.push((sign, term)),
                    }
                    sign = 1.0;
                }
            }
        }
    }
    let _ = name;
    CalcExpr { terms }
}

fn apply_factor_to_last(terms: &mut [(f32, CalcTerm)], factor: f32) {
    if let Some((_, term)) = terms.last_mut() {
        match term {
            CalcTerm::Length(_, _, m) => *m *= factor,
            CalcTerm::Number(n) => *n *= factor,
            CalcTerm::Var { mul, .. } => *mul *= factor,
            CalcTerm::Calc(_, m) => *m *= factor,
        }
    }
}
