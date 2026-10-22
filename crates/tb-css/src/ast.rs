//! CSS AST.

use crate::media::MediaQuery;
use crate::selector::ComplexSelector;
use crate::value::DeclarationValue;

/// A `name: value` pair.
#[derive(Debug, Clone, PartialEq)]
pub struct Declaration {
    pub name: String,
    pub value: DeclarationValue,
    pub important: bool,
    /// `--custom-prop` declarations live in their own namespace.
    pub is_custom: bool,
}

impl Declaration {
    pub fn new(name: &str, value: DeclarationValue) -> Declaration {
        let important = value.important;
        Declaration {
            name: name.to_ascii_lowercase(),
            value,
            important,
            is_custom: name.starts_with("--"),
        }
    }

    pub fn to_css(&self) -> String {
        format!("{}: {}", self.name, self.value.to_css())
    }
}

/// A rule with selectors and declarations (`a.b { color: red }`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StyleRule {
    pub selectors: Vec<ComplexSelector>,
    pub declarations: Vec<Declaration>,
    /// Nested rules (CSS nesting: `&:hover`, `> li`).
    pub nested: Vec<Rule>,
}

/// Any rule of a stylesheet.
#[derive(Debug, Clone, PartialEq)]
pub enum Rule {
    Style(StyleRule),
    Media {
        queries: Vec<MediaQuery>,
        rules: Vec<Rule>,
    },
    /// `@supports` — we ignore the condition and apply the rules.
    Supports {
        rules: Vec<Rule>,
    },
    FontFace {
        declarations: Vec<Declaration>,
    },
    Keyframes {
        name: String,
    },
    Import {
        url: String,
        media: Vec<MediaQuery>,
    },
    /// Something we deliberately skip (`@charset`, junk).
    Ignored {
        name: String,
    },
}

/// A parsed stylesheet.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StyleSheet {
    pub rules: Vec<Rule>,
    /// Where the sheet came from (`<link href>`), for diagnostics.
    pub href: String,
}

impl StyleSheet {
    /// Number of style rules (recursing into conditional groups).
    pub fn style_rule_count(&self) -> usize {
        fn count(rules: &[Rule]) -> usize {
            rules
                .iter()
                .map(|r| match r {
                    Rule::Style(s) => 1 + count(&s.nested),
                    Rule::Media { rules, .. } | Rule::Supports { rules } => count(rules),
                    _ => 0,
                })
                .sum()
        }
        count(&self.rules)
    }

    pub fn declaration_count(&self) -> usize {
        fn count(rules: &[Rule]) -> usize {
            rules
                .iter()
                .map(|r| match r {
                    Rule::Style(s) => s.declarations.len() + count(&s.nested),
                    Rule::Media { rules, .. } | Rule::Supports { rules } => count(rules),
                    Rule::FontFace { declarations } => declarations.len(),
                    _ => 0,
                })
                .sum()
        }
        count(&self.rules)
    }

    /// `@import` urls in document order.
    pub fn imports(&self) -> Vec<String> {
        let mut out = Vec::new();
        for r in &self.rules {
            if let Rule::Import { url, .. } = r {
                out.push(url.clone());
            }
        }
        out
    }

    /// Debug dump used by `tiny --css`.
    pub fn debug_dump(&self) -> String {
        let mut out = String::new();
        fn walk(rules: &[Rule], depth: usize, out: &mut String) {
            let pad = "  ".repeat(depth);
            for r in rules {
                match r {
                    Rule::Style(s) => {
                        let sels: Vec<String> = s.selectors.iter().map(|c| c.text.clone()).collect();
                        out.push_str(&format!("{pad}{} {{}}\n", sels.join(", ")));
                        for d in &s.declarations {
                            out.push_str(&format!("{pad}  {}\n", d.to_css()));
                        }
                        walk(&s.nested, depth + 1, out);
                    }
                    Rule::Media { queries, rules } => {
                        let q: Vec<String> = queries.iter().map(|q| q.text.clone()).collect();
                        out.push_str(&format!("{pad}@media {} {{\n", q.join(", ")));
                        walk(rules, depth + 1, out);
                        out.push_str(&format!("{pad}}}\n"));
                    }
                    Rule::Supports { rules } => {
                        out.push_str(&format!("{pad}@supports {{\n"));
                        walk(rules, depth + 1, out);
                        out.push_str(&format!("{pad}}}\n"));
                    }
                    Rule::FontFace { declarations } => {
                        out.push_str(&format!("{pad}@font-face {{ {} }}\n", declarations.len()));
                    }
                    Rule::Keyframes { name } => {
                        out.push_str(&format!("{pad}@keyframes {name}\n"));
                    }
                    Rule::Import { url, .. } => {
                        out.push_str(&format!("{pad}@import {url}\n"));
                    }
                    Rule::Ignored { name } => {
                        out.push_str(&format!("{pad}@{name} (skipped)\n"));
                    }
                }
            }
        }
        walk(&self.rules, 0, &mut out);
        out
    }
}
