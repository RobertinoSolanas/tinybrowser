//! The CSS parser.
//!
//! Written directly on characters (no separate token struct) and built for
//! error recovery: a broken rule is skipped, the rest of the sheet still
//! applies. That matters because real sites ship minified CSS, proprietary
//! at-rules and the occasional `@scsssope` (yes, taz.de has one).

use crate::ast::{Declaration, Rule, StyleRule, StyleSheet};
use crate::media::parse_media_list;
use crate::selector::parse_selector_list;
use crate::value::{parse_value, DeclarationValue};

/// Parse a stylesheet.
pub fn parse_stylesheet(css: &str) -> StyleSheet {
    let chars: Vec<char> = css.chars().collect();
    let mut p = Parser { c: &chars, i: 0 };
    let rules = p.parse_rules();
    StyleSheet {
        rules,
        href: String::new(),
    }
}

/// Parse a single rule (used in tests and for `element.style`).
pub fn parse_rule(css: &str) -> Option<Rule> {
    parse_stylesheet(css).rules.into_iter().next()
}

/// Parse a bare declaration list — the content of a `style=""` attribute.
pub fn parse_declarations(css: &str) -> Vec<Declaration> {
    let chars: Vec<char> = css.chars().collect();
    let mut p = Parser { c: &chars, i: 0 };
    let (declarations, _nested) = p.parse_block_body();
    declarations
}

#[cfg(test)]
mod decl_list_tests {
    use super::*;

    #[test]
    fn style_attribute_list() {
        let d = parse_declarations("color: red; margin: 0 auto;font-size:12px !important");
        let out: Vec<(String, String, bool)> = d
            .iter()
            .map(|x| (x.name.clone(), x.value.to_css(), x.important))
            .collect();
        assert_eq!(
            out,
            vec![
                ("color".into(), "red".into(), false),
                ("margin".into(), "0 auto".into(), false),
                // to_css() round-trips the `!important` marker
                ("font-size".into(), "12px !important".into(), true),
            ]
        );
    }

    #[test]
    fn tolerant_of_junk_and_urls() {
        let d =
            parse_declarations(";;background:url(data:image/png;base64,AAA); : nope; color:red");
        assert_eq!(d.len(), 2, "{d:?}");
        assert_eq!(d[1].name, "color");
    }
}

struct Parser<'a> {
    c: &'a [char],
    i: usize,
}

fn is_ws(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{c}')
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<char> {
        self.c.get(self.i).copied()
    }

    fn at_end(&self) -> bool {
        self.i >= self.c.len()
    }

    fn skip_ws_and_comments(&mut self) {
        loop {
            while matches!(self.peek(), Some(c) if is_ws(c)) {
                self.i += 1;
            }
            if self.starts_with("/*") {
                self.i += 2;
                while !self.at_end() {
                    if self.starts_with("*/") {
                        self.i += 2;
                        break;
                    }
                    self.i += 1;
                }
                continue;
            }
            break;
        }
    }

    fn starts_with(&self, s: &str) -> bool {
        s.chars()
            .enumerate()
            .all(|(k, ch)| self.c.get(self.i + k) == Some(&ch))
    }

    /// Skip a balanced `{ ... }` block starting at the current position.
    fn skip_block(&mut self) {
        if self.peek() != Some('{') {
            // no block: skip to ';'
            while !self.at_end() && self.peek() != Some(';') && self.peek() != Some('{') {
                self.i += 1;
            }
            if self.peek() == Some(';') {
                self.i += 1;
            }
            return;
        }
        let mut depth = 0i32;
        let mut quote: Option<char> = None;
        while !self.at_end() {
            let c = self.c[self.i];
            match quote {
                Some(q) => {
                    if c == q {
                        quote = None;
                    }
                }
                None => match c {
                    '"' | '\'' => quote = Some(c),
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            self.i += 1;
                            return;
                        }
                    }
                    _ => {}
                },
            }
            self.i += 1;
        }
    }

    /// Parse a list of rules until EOF or the closing `}` of our block.
    fn parse_rules(&mut self) -> Vec<Rule> {
        let mut rules = Vec::new();
        loop {
            self.skip_ws_and_comments();
            if self.at_end() {
                break;
            }
            if self.peek() == Some('}') {
                self.i += 1;
                break;
            }
            if self.peek() == Some(';') {
                self.i += 1;
                continue;
            }
            if self.peek() == Some('@') {
                rules.extend(self.parse_at_rule());
                continue;
            }
            // style rule: read the prelude
            let prelude = self.read_prelude();
            if self.peek() != Some('{') {
                // stray text at the end of the sheet
                if prelude.trim().is_empty() {
                    break;
                }
                continue;
            }
            self.i += 1; // '{'
            let (declarations, nested) = self.parse_block_body();
            let selectors = parse_selector_list(&prelude);
            if !selectors.is_empty() {
                rules.push(Rule::Style(StyleRule {
                    selectors,
                    declarations,
                    nested,
                }));
            }
        }
        rules
    }

    /// Read up to (not including) `{`, `;` or `}`.
    fn read_prelude(&mut self) -> String {
        let mut out = String::new();
        let mut quote: Option<char> = None;
        let mut depth = 0i32;
        while !self.at_end() {
            let c = self.c[self.i];
            match quote {
                Some(q) => {
                    out.push(c);
                    if c == q {
                        quote = None;
                    }
                    self.i += 1;
                    continue;
                }
                None => match c {
                    '"' | '\'' => {
                        quote = Some(c);
                        out.push(c);
                        self.i += 1;
                        continue;
                    }
                    '(' => depth += 1,
                    ')' => depth -= 1,
                    '{' | ';' => break,
                    '}' if depth <= 0 => break,
                    _ => {}
                },
            }
            out.push(c);
            self.i += 1;
        }
        out
    }

    /// Parse the inside of a `{ ... }` block: declarations and nested rules.
    fn parse_block_body(&mut self) -> (Vec<Declaration>, Vec<Rule>) {
        let mut declarations = Vec::new();
        let mut nested = Vec::new();
        loop {
            self.skip_ws_and_comments();
            if self.at_end() {
                break;
            }
            match self.peek() {
                Some('}') => {
                    self.i += 1;
                    break;
                }
                Some(';') => {
                    self.pos();
                    continue;
                }
                _ => {}
            }
            if self.peek() == Some('@') {
                nested.extend(self.parse_at_rule());
                continue;
            }
            // Read one segment; decide between declaration and nested rule.
            let save = self.i;
            let prelude = self.read_declaration_name();
            if self.peek() == Some(':') && self.block_ahead() {
                // CSS nesting (`&:hover { ... }`): rewind and parse as a rule.
                self.i = save;
                let sel_prelude = self.read_prelude();
                if self.peek() == Some('{') {
                    self.i += 1;
                    let (d, n) = self.parse_block_body();
                    let selectors = parse_selector_list(&sel_prelude);
                    if !selectors.is_empty() {
                        nested.push(Rule::Style(StyleRule {
                            selectors,
                            declarations: d,
                            nested: n,
                        }));
                    }
                }
                continue;
            }
            match self.peek() {
                Some('{') => {
                    self.i += 1;
                    let (d, n) = self.parse_block_body();
                    let sel_text = prelude.trim().to_string();
                    let selectors = parse_selector_list(&sel_text);
                    if !selectors.is_empty() {
                        nested.push(Rule::Style(StyleRule {
                            selectors,
                            declarations: d,
                            nested: n,
                        }));
                    }
                }
                Some(':') if prelude.trim().is_empty() => {
                    // garbage colon: skip the token and continue
                    self.i += 1;
                }
                Some(':') => {
                    self.i += 1; // ':'
                    let value = self.read_value();
                    if !prelude.trim().is_empty() {
                        declarations.push(Declaration::new(prelude.trim(), value));
                    }
                }
                Some(';') => {
                    self.i += 1;
                }
                Some('}') => {
                    self.i += 1;
                    break;
                }
                _ => {
                    if self.at_end() {
                        break;
                    }
                    // unparseable junk inside a block: skip to the next ';'
                    self.skip_to_decl_end();
                    if self.i == save {
                        self.i += 1;
                    }
                }
            }
        }
        (declarations, nested)
    }

    /// Look ahead: is there a `{` before the end of this declaration?
    fn block_ahead(&self) -> bool {
        let mut i = self.i;
        let mut quote: Option<char> = None;
        let mut depth = 0i32;
        while i < self.c.len() {
            let c = self.c[i];
            match quote {
                Some(q) => {
                    if c == q {
                        quote = None;
                    }
                }
                None => match c {
                    '"' | '\'' => quote = Some(c),
                    '(' => depth += 1,
                    ')' => depth -= 1,
                    '{' if depth <= 0 => return true,
                    ';' | '}' if depth <= 0 => return false,
                    _ => {}
                },
            }
            i += 1;
        }
        false
    }

    fn pos(&mut self) {
        if self.peek() == Some(';') {
            self.i += 1;
        }
    }

    /// Read a declaration name up to `:` (stops at `{`/`;`/`}`).
    fn read_declaration_name(&mut self) -> String {
        let mut out = String::new();
        while !self.at_end() {
            let c = self.c[self.i];
            match c {
                ':' | '{' | ';' | '}' => break,
                _ => {
                    out.push(c);
                    self.i += 1;
                }
            }
        }
        out
    }

    /// Read a declaration value up to `;` or the end of the block.
    fn read_value(&mut self) -> DeclarationValue {
        let mut raw = String::new();
        let mut quote: Option<char> = None;
        let mut depth = 0i32;
        while !self.at_end() {
            let c = self.c[self.i];
            match quote {
                Some(q) => {
                    raw.push(c);
                    if c == q {
                        quote = None;
                    }
                    self.i += 1;
                    continue;
                }
                None => match c {
                    '"' | '\'' => {
                        quote = Some(c);
                        raw.push(c);
                        self.i += 1;
                        continue;
                    }
                    '(' => {
                        depth += 1;
                        raw.push(c);
                        self.i += 1;
                        continue;
                    }
                    ')' => {
                        depth -= 1;
                        raw.push(c);
                        self.i += 1;
                        continue;
                    }
                    ';' if depth <= 0 => {
                        self.i += 1;
                        break;
                    }
                    '}' => break,
                    _ => {}
                },
            }
            raw.push(c);
            self.i += 1;
        }
        parse_value(&raw)
    }

    fn skip_to_decl_end(&mut self) {
        let mut depth = 0i32;
        let mut quote: Option<char> = None;
        while !self.at_end() {
            let c = self.c[self.i];
            match quote {
                Some(q) => {
                    if c == q {
                        quote = None;
                    }
                }
                None => match c {
                    '"' | '\'' => quote = Some(c),
                    '(' => depth += 1,
                    ')' => depth -= 1,
                    ';' if depth <= 0 => {
                        self.i += 1;
                        return;
                    }
                    '}' if depth <= 0 => return,
                    '{' if depth <= 0 => return,
                    _ => {}
                },
            }
            self.i += 1;
        }
    }

    /// `@media`, `@import`, `@font-face`, `@keyframes`, `@supports`, unknown.
    fn parse_at_rule(&mut self) -> Vec<Rule> {
        self.i += 1; // '@'
        let mut name = String::new();
        while let Some(c) = self.peek() {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                name.push(c.to_ascii_lowercase());
                self.i += 1;
            } else {
                break;
            }
        }
        let prelude = self.read_prelude();
        match name.as_str() {
            "media" => {
                let queries = parse_media_list(&prelude);
                let rules = if self.peek() == Some('{') {
                    self.i += 1;
                    self.parse_rules()
                } else {
                    Vec::new()
                };
                vec![Rule::Media { queries, rules }]
            }
            "supports" => {
                let rules = if self.peek() == Some('{') {
                    self.i += 1;
                    self.parse_rules()
                } else {
                    Vec::new()
                };
                vec![Rule::Supports { rules }]
            }
            "font-face" | "page" | "viewport" | "-ms-viewport" => {
                let declarations = if self.peek() == Some('{') {
                    self.i += 1;
                    let (d, _) = self.parse_block_body();
                    d
                } else {
                    Vec::new()
                };
                vec![Rule::FontFace { declarations }]
            }
            "import" => {
                if self.peek() == Some(';') {
                    self.i += 1;
                }
                let url = extract_url(&prelude);
                let media_part = prelude
                    .split_once(char::is_whitespace)
                    .map(|(_, rest)| rest.to_string())
                    .unwrap_or_default();
                let media = if media_part.trim().is_empty() {
                    Vec::new()
                } else {
                    parse_media_list(&media_part)
                };
                if url.is_empty() {
                    vec![Rule::Ignored { name }]
                } else {
                    vec![Rule::Import { url, media }]
                }
            }
            "charset" => {
                self.skip_block();
                vec![Rule::Ignored { name }]
            }
            "keyframes" | "-webkit-keyframes" | "-moz-keyframes" => {
                let n = prelude.trim().to_string();
                self.skip_block();
                vec![Rule::Keyframes { name: n }]
            }
            // at-rules whose body is normal CSS
            "layer" | "scope" | "container" | "document" | "-moz-document" => {
                if self.peek() == Some('{') {
                    self.i += 1;
                    self.parse_rules()
                } else {
                    Vec::new()
                }
            }
            // everything else (including `@scsssope`, `@include`, `@apply`)
            _ => {
                self.skip_block();
                vec![Rule::Ignored { name }]
            }
        }
    }
}

/// `url(...)`, `"..."` or `...` from an `@import` prelude.
pub fn extract_url(s: &str) -> String {
    let s = s.trim();
    if let Some(rest) = s.strip_prefix("url(") {
        let inner = rest.split(')').next().unwrap_or("").trim();
        return inner.trim_matches('"').trim_matches('\'').to_string();
    }
    // `@import "file.css" screen;`
    let cut = s.find(char::is_whitespace).unwrap_or(s.len());
    let first = &s[..cut];
    if first.ends_with(".css") || first.contains('/') || first.contains('.') {
        return first.trim_matches('"').trim_matches('\'').to_string();
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decls(css: &str) -> Vec<(String, String, bool)> {
        match parse_rule(css) {
            Some(Rule::Style(s)) => s
                .declarations
                .iter()
                .map(|d| (d.name.clone(), d.value.to_css(), d.important))
                .collect(),
            other => panic!("expected style rule, got {other:?}"),
        }
    }

    #[test]
    fn simple_rule() {
        let sheet = parse_stylesheet("h1 { color: red; font-size: 20px }");
        assert_eq!(sheet.style_rule_count(), 1);
        assert_eq!(sheet.declaration_count(), 2);
    }

    #[test]
    fn selectors_and_declaration_values() {
        assert_eq!(decls("a { color: red }")[0].1, "red");
        assert_eq!(decls("a { margin: 1px 2% 3em }")[0].1, "1px 2% 3em");
        assert_eq!(decls("a { color: #fff }")[0].1, "#ffffff");
        assert_eq!(decls("a { content: \"a, b\" }")[0].1, "\"a, b\"");
        assert!(decls("a { color: red !important }")[0].2);
    }

    #[test]
    fn comments_are_skipped_including_inside_values() {
        let sheet = parse_stylesheet("/* hi */ a { color: /*x*/ red; } /* bye");
        assert_eq!(sheet.style_rule_count(), 1);
        assert_eq!(sheet.declaration_count(), 1);
    }

    #[test]
    fn media_queries_nest_rules() {
        let sheet = parse_stylesheet(
            "@media screen and (max-width: 600px) { .a { display: none } .b { color: red } }",
        );
        match &sheet.rules[0] {
            Rule::Media { queries, rules } => {
                assert_eq!(queries.len(), 1);
                assert_eq!(rules.len(), 2);
                assert!(queries[0].matches(&crate::media::Viewport {
                    width: 400.0,
                    ..Default::default()
                }));
            }
            r => panic!("{r:?}"),
        }
    }

    #[test]
    fn keyframes_are_recognised_but_not_expanded() {
        let sheet = parse_stylesheet("@keyframes fade { from { opacity: 0 } to { opacity: 1 } }");
        assert!(matches!(&sheet.rules[0], Rule::Keyframes { name } if name == "fade"));
    }

    #[test]
    fn font_face_and_import() {
        let sheet = parse_stylesheet(
            "@import url(\"base.css\") screen; @font-face { font-family: X; src: url(a.woff); }",
        );
        assert_eq!(sheet.imports(), vec!["base.css".to_string()]);
        assert!(
            matches!(&sheet.rules[1], Rule::FontFace { declarations } if declarations.len() == 2)
        );
    }

    #[test]
    fn broken_css_is_skipped_not_fatal() {
        let sheet = parse_stylesheet(
            "a { color: } .b { color: red } .broken { color: rgb( } .c{color:blue}",
        );
        let css = sheet.debug_dump();
        assert!(css.contains(".b"), "{css}");
        assert!(css.contains(".c"), "{css}");
    }

    #[test]
    fn proprietary_at_rules_are_skipped() {
        let css = "@charset \"utf-8\"; @scsssope{ #tomImage { @include from($tablet) { min-height: 490px; } } } .real { color: green }";
        let sheet = parse_stylesheet(css);
        let dump = sheet.debug_dump();
        assert!(dump.contains(".real"), "{dump}");
        assert!(!dump.contains("tomImage"), "{dump}");
    }

    #[test]
    fn supports_and_layer_are_applied() {
        let sheet = parse_stylesheet(
            "@supports (display: grid) { .g { color: red } } @layer base { .l { color: blue } }",
        );
        let dump = sheet.debug_dump();
        assert!(dump.contains(".g") && dump.contains(".l"), "{dump}");
    }

    #[test]
    fn nested_css_rules() {
        let sheet = parse_stylesheet(
            ".card { color: red; &:hover { color: blue } > .title { font-weight: bold } }",
        );
        match &sheet.rules[0] {
            Rule::Style(s) => {
                assert_eq!(s.declarations.len(), 1);
                assert_eq!(s.nested.len(), 2);
            }
            r => panic!("{r:?}"),
        }
    }

    #[test]
    fn functions_and_calc() {
        let d = decls("a { width: calc(100% - 2rem); background: var(--c, #fff) }");
        assert!(d[0].1.contains("calc"), "{}", d[0].1);
        assert!(d[1].1.contains("var"), "{}", d[1].1);
    }

    #[test]
    fn minified_css_parses() {
        let css = ".a{color:#333}.b,.c{margin:0 auto}.d::before{content:\"\"}.e:hover{opacity:.5}@media (min-width:600px){.a{color:red}}";
        let sheet = parse_stylesheet(css);
        assert_eq!(sheet.style_rule_count(), 5);
    }
}
