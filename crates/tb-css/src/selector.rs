//! Selector syntax: `div.a > p[data-x]:not(.b):hover::before`

/// Combinators between compound selectors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Combinator {
    /// `a b`
    Descendant,
    /// `a > b`
    Child,
    /// `a + b`
    NextSibling,
    /// `a ~ b`
    SubsequentSibling,
}

/// Attribute selector operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttrOp {
    Exists,
    Equals,
    Tilde,
    Caret,
    Dollar,
    Star,
    Hyphen,
}

/// One simple selector inside a compound selector.
#[derive(Debug, Clone, PartialEq)]
pub enum Simple {
    /// `*` or nothing
    Universal,
    /// `div`
    Type(String),
    /// `#id`
    Id(String),
    /// `.class`
    Class(String),
    /// `[href^="https"]`
    Attr {
        name: String,
        op: AttrOp,
        value: String,
        insensitive: bool,
    },
    /// `:hover`, `:not(...)`, `:nth-child(2n+1)`
    PseudoClass { name: String, args: String },
    /// `::before`
    PseudoElement { name: String, args: String },
}

/// A complex selector split into simple selectors and combinators.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ComplexSelector {
    /// parts[0] is the left-most compound; always starts with a simple part.
    pub parts: Vec<SelectorPart>,
    /// raw text (for diagnostics)
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SelectorPart {
    Simple(Simple),
    Compound(Vec<Simple>),
    Combinator(Combinator),
}

/// Specificity `(id, class/attr/pseudo-class, type/pseudo-element)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Specificity(pub u32, pub u32, pub u32);

impl ComplexSelector {
    /// CSS specificity, `:not()`/`:is()` contribute their most specific argument.
    pub fn specificity(&self) -> Specificity {
        let mut s = Specificity(0, 0, 0);
        for part in &self.parts {
            let compound = match part {
                SelectorPart::Compound(c) => c,
                _ => continue,
            };
            for simple in compound {
                match simple {
                    Simple::Id(_) => s.0 += 1,
                    Simple::Class(_) | Simple::Attr { .. } => s.1 += 1,
                    Simple::Type(_) => s.2 += 1,
                    Simple::Universal => {}
                    Simple::PseudoElement { .. } => s.2 += 1,
                    Simple::PseudoClass { name, args } => match name.as_str() {
                        "where" => {}
                        "has" => s.1 += 1,
                        "not" | "is" | "matches" => {
                            let inner = parse_selector_list(args)
                                .into_iter()
                                .map(|c| c.specificity())
                                .max()
                                .unwrap_or_default();
                            s.0 += inner.0;
                            s.1 += inner.1;
                            s.2 += inner.2;
                        }
                        _ => s.1 += 1,
                    },
                }
            }
        }
        s
    }

    /// `true` when the right-most compound is a given pseudo element.
    pub fn pseudo_element(&self) -> Option<&str> {
        self.parts.iter().rev().find_map(|p| match p {
            SelectorPart::Compound(c) => c.iter().rev().find_map(|s| match s {
                Simple::PseudoElement { name, .. } => Some(name.as_str()),
                _ => None,
            }),
            _ => None,
        })
    }
}

/// Parse a selector list (`a, b > c`).
pub fn parse_selector_list(input: &str) -> Vec<ComplexSelector> {
    let mut out = Vec::new();
    for chunk in split_top_level(input, ',') {
        let t = chunk.trim();
        if t.is_empty() {
            continue;
        }
        let sel = parse_complex(t);
        if !sel.parts.is_empty() {
            out.push(sel);
        }
    }
    out
}

/// Split on `sep` but ignore separators inside `[]`, `()`, quotes.
pub fn split_top_level(input: &str, sep: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut cur = String::new();
    for c in input.chars() {
        match quote {
            Some(q) => {
                cur.push(c);
                if c == q {
                    quote = None;
                }
                continue;
            }
            None => match c {
                '"' | '\'' => {
                    quote = Some(c);
                    cur.push(c);
                }
                '(' | '[' => {
                    depth += 1;
                    cur.push(c);
                }
                ')' | ']' => {
                    depth -= 1;
                    cur.push(c);
                }
                c if c == sep && depth <= 0 => {
                    out.push(std::mem::take(&mut cur));
                }
                _ => cur.push(c),
            },
        }
    }
    if !cur.trim().is_empty() || !out.is_empty() {
        out.push(cur);
    }
    out
}

fn parse_complex(text: &str) -> ComplexSelector {
    let chars: Vec<char> = text.chars().collect();
    let mut parts: Vec<SelectorPart> = Vec::new();
    let mut i = 0usize;
    let mut compound: Vec<Simple> = Vec::new();
    let mut pending_combinator: Option<Combinator> = None;

    let flush = |parts: &mut Vec<SelectorPart>,
                 compound: &mut Vec<Simple>,
                 combinator: &mut Option<Combinator>| {
        if !compound.is_empty() {
            if let Some(c) = combinator.take() {
                parts.push(SelectorPart::Combinator(c));
            }
            parts.push(SelectorPart::Compound(std::mem::take(compound)));
        }
    };

    while i < chars.len() {
        let c = chars[i];
        match c {
            ' ' | '\t' | '\n' | '\r' | '\u{c}' => {
                if !compound.is_empty() {
                    // whitespace may be a descendant combinator, but only if the
                    // next non space char is not another combinator
                    let mut j = i + 1;
                    while matches!(chars.get(j), Some(' ') | Some('\t') | Some('\n')) {
                        j += 1;
                    }
                    if !matches!(chars.get(j), Some('>') | Some('+') | Some('~') | None) {
                        flush(&mut parts, &mut compound, &mut pending_combinator);
                        pending_combinator = Some(Combinator::Descendant);
                    }
                }
                i += 1;
            }
            '>' | '+' | '~' => {
                flush(&mut parts, &mut compound, &mut pending_combinator);
                pending_combinator = Some(match c {
                    '>' => Combinator::Child,
                    '+' => Combinator::NextSibling,
                    _ => Combinator::SubsequentSibling,
                });
                i += 1;
            }
            '*' => {
                compound.push(Simple::Universal);
                i += 1;
            }
            '#' => {
                i += 1;
                let start = i;
                while i < chars.len() && is_ident_char(chars[i]) {
                    i += 1;
                }
                compound.push(Simple::Id(chars[start..i].iter().collect()));
            }
            '.' => {
                i += 1;
                let start = i;
                while i < chars.len() && is_ident_char(chars[i]) {
                    i += 1;
                }
                compound.push(Simple::Class(chars[start..i].iter().collect()));
            }
            '[' => {
                i += 1;
                let start = i;
                let mut depth = 1;
                while i < chars.len() {
                    match chars[i] {
                        '[' => depth += 1,
                        ']' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                let inner: String = chars[start..i.min(chars.len())].iter().collect();
                i += 1;
                if let Some(s) = parse_attr(&inner) {
                    compound.push(s);
                }
            }
            ':' => {
                i += 1;
                let element = chars.get(i) == Some(&':');
                if element {
                    i += 1;
                }
                let start = i;
                while i < chars.len() && is_ident_char(chars[i]) {
                    i += 1;
                }
                let name: String = chars[start..i].iter().collect();
                let mut args = String::new();
                if chars.get(i) == Some(&'(') {
                    let mut depth = 1;
                    i += 1;
                    let a_start = i;
                    while i < chars.len() && depth > 0 {
                        match chars[i] {
                            '(' => depth += 1,
                            ')' => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            _ => {}
                        }
                        i += 1;
                    }
                    args = chars[a_start..i].iter().collect();
                    if i < chars.len() {
                        i += 1; // ')'
                    }
                }
                if element {
                    compound.push(Simple::PseudoElement {
                        name: name.to_ascii_lowercase(),
                        args,
                    });
                } else {
                    compound.push(Simple::PseudoClass {
                        name: name.to_ascii_lowercase(),
                        args,
                    });
                }
            }
            '|' => {
                // namespace separator: `svg|path` -> treat as type selector
                i += 1;
            }
            c if is_ident_start(c) => {
                let start = i;
                while i < chars.len() && is_ident_char(chars[i]) {
                    i += 1;
                }
                let name: String = chars[start..i].iter().collect();
                compound.push(Simple::Type(name.to_ascii_lowercase()));
            }
            _ => {
                // unknown character: ignore (error recovery)
                i += 1;
            }
        }
    }
    flush(&mut parts, &mut compound, &mut pending_combinator);
    ComplexSelector {
        parts,
        text: text.trim().to_string(),
    }
}

fn parse_attr(inner: &str) -> Option<Simple> {
    let chars: Vec<char> = inner.trim().chars().collect();
    let mut quote: Option<char> = None;
    let mut op_idx: Option<usize> = None;
    let mut op_ch = '=';
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                }
            }
            None => match c {
                '"' | '\'' => quote = Some(c),
                '=' => {
                    op_idx = Some(i);
                    op_ch = '=';
                    break;
                }
                '~' | '^' | '$' | '*' | '|' if chars.get(i + 1) == Some(&'=') => {
                    op_idx = Some(i);
                    op_ch = c;
                    break;
                }
                _ => {}
            },
        }
        i += 1;
    }
    let (name, op, value, insensitive) = match op_idx {
        None => (
            inner.trim().to_ascii_lowercase(),
            AttrOp::Exists,
            String::new(),
            false,
        ),
        Some(idx) => {
            let name: String = chars[..idx].iter().collect();
            let from = if op_ch == '=' { idx + 1 } else { idx + 2 };
            let rest: String = chars[from..].iter().collect();
            let rest = rest.trim();
            // trailing `i` flag (case insensitive match), always after a space
            let (raw, insensitive) =
                if rest.len() > 2 && (rest.ends_with(" i") || rest.ends_with(" I")) {
                    (rest[..rest.len() - 2].trim(), true)
                } else {
                    (rest, false)
                };
            let value = raw.trim_matches('"').trim_matches('\'').to_string();
            let op = match op_ch {
                '=' => AttrOp::Equals,
                '~' => AttrOp::Tilde,
                '^' => AttrOp::Caret,
                '$' => AttrOp::Dollar,
                '*' => AttrOp::Star,
                '|' => AttrOp::Hyphen,
                _ => AttrOp::Exists,
            };
            (name.trim().to_ascii_lowercase(), op, value, insensitive)
        }
    };
    if name.is_empty() {
        return None;
    }
    Some(Simple::Attr {
        name,
        op,
        value,
        insensitive,
    })
}

pub fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || c == '-' || (c as u32) > 127
}

pub fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '%' || (c as u32) > 127
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sel(s: &str) -> ComplexSelector {
        parse_selector_list(s).remove(0)
    }

    #[test]
    fn parses_type_id_class() {
        let s = sel("div#main.header");
        assert_eq!(
            s.parts,
            vec![SelectorPart::Compound(vec![
                Simple::Type("div".into()),
                Simple::Id("main".into()),
                Simple::Class("header".into()),
            ])]
        );
        assert_eq!(s.specificity(), Specificity(1, 1, 1));
    }

    #[test]
    fn parses_combinators() {
        let s = sel("ul > li + a ~ b");
        let kinds: Vec<&str> = s
            .parts
            .iter()
            .map(|p| match p {
                SelectorPart::Combinator(Combinator::Child) => ">",
                SelectorPart::Combinator(Combinator::NextSibling) => "+",
                SelectorPart::Combinator(Combinator::SubsequentSibling) => "~",
                SelectorPart::Combinator(Combinator::Descendant) => " ",
                SelectorPart::Compound(_) => "c",
                SelectorPart::Simple(_) => "s",
            })
            .collect();
        assert_eq!(kinds, vec!["c", ">", "c", "+", "c", "~", "c"]);
    }

    #[test]
    fn descendant_needs_whitespace() {
        let s = sel("div p");
        assert_eq!(s.parts.len(), 3);
        let s2 = sel("divp");
        assert_eq!(s2.parts.len(), 1);
    }

    #[test]
    fn attribute_selectors() {
        match sel("a[href^='https://']").parts.as_slice() {
            [SelectorPart::Compound(v)] => match &v[1] {
                Simple::Attr {
                    name, op, value, ..
                } => {
                    assert_eq!(name, "href");
                    assert_eq!(*op, AttrOp::Caret);
                    assert_eq!(value, "https://");
                }
                other => panic!("{other:?}"),
            },
            _ => panic!("bad"),
        }
        match sel("input[disabled]").parts.as_slice() {
            [SelectorPart::Compound(v)] => assert_eq!(
                v[1],
                Simple::Attr {
                    name: "disabled".into(),
                    op: AttrOp::Exists,
                    value: String::new(),
                    insensitive: false
                }
            ),
            _ => panic!("bad"),
        }
    }

    #[test]
    fn pseudo_classes_with_arguments() {
        let s = sel("li:not(.a, .b):nth-child(2n+1)::before");
        let pseudo: Vec<(String, String)> = s
            .parts
            .iter()
            .filter_map(|p| match p {
                SelectorPart::Compound(c) => Some(c),
                _ => None,
            })
            .flatten()
            .filter_map(|x| match x {
                Simple::PseudoClass { name, args } => Some((name.clone(), args.clone())),
                Simple::PseudoElement { name, args } => Some((format!("::{name}"), args.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(
            pseudo,
            vec![
                ("not".to_string(), ".a, .b".to_string()),
                ("nth-child".to_string(), "2n+1".to_string()),
                ("::before".to_string(), String::new())
            ]
        );
    }

    #[test]
    fn specificity_ordering() {
        assert!(sel("#a").specificity() > sel(".b.c").specificity());
        assert!(sel(".b.c").specificity() > sel("div").specificity());
        assert_eq!(sel("div").specificity(), Specificity(0, 0, 1));
        assert_eq!(sel(":not(.x)").specificity(), Specificity(0, 1, 0));
        assert_eq!(sel(":where(.x)").specificity(), Specificity(0, 0, 0));
        assert_eq!(sel("*").specificity(), Specificity(0, 0, 0));
        assert_eq!(sel("a:hover").specificity(), Specificity(0, 1, 1));
    }

    #[test]
    fn selector_list_splits() {
        let l = parse_selector_list("h1, h2, .x > .y");
        assert_eq!(l.len(), 3);
        assert_eq!(l[2].text, ".x > .y");
    }

    #[test]
    fn garbage_selectors_do_not_panic() {
        for s in [">", "###", "a..b", "[:", "[a=]", ":", "::", "a b>", "@#$"] {
            let _ = parse_selector_list(s);
        }
    }
}
