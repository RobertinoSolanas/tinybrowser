//! HTML tree builder: tokens in, DOM out.
//!
//! Implements the useful subset of the WHATWG tree construction algorithm:
//!
//! * implicit `html`, `head`, `body` (and `tbody`)
//! * void elements and self-closing tags (`<br>`, `<img/>`)
//! * raw text elements (`script`, `style`, `title`, `textarea`)
//! * the `<p>` auto-close rules for block level elements
//! * `li` / `dd` / `dt` / `tr` / `td` implied end tags
//! * foreign content (`svg`, `math`) with self-closing semantics
//! * tolerant error recovery: `</meta>` (yes, taz emits that), stray end tags,
//!   unclosed tags, tags closed out of order — all of it must not panic.

use crate::tokenizer::{raw_kind_of, RawKind, Token, Tokenizer};
use tb_dom::{Dom, Namespace, NodeId};

/// Insertion modes (a lot fewer than the spec, enough for real pages).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Initial,
    BeforeHtml,
    BeforeHead,
    InHead,
    AfterHead,
    InBody,
    /// Inside a raw text element (`script`/`style`/...)
    Text,
    AfterBody,
    AfterAfterBody,
}

/// Elements that end an open `<p>`.
const CLOSES_P: &[&str] = &[
    "address",
    "article",
    "aside",
    "blockquote",
    "details",
    "div",
    "dl",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hgroup",
    "hr",
    "main",
    "menu",
    "nav",
    "ol",
    "p",
    "pre",
    "search",
    "section",
    "table",
    "ul",
    "form",
    "center",
    "marquee",
    "legend",
    "caption",
    "colgroup",
    "tbody",
    "tfoot",
    "thead",
    "tr",
    "li",
    "dd",
    "dt",
];

/// Head level elements.
const HEAD_ELEMENTS: &[&str] = &[
    "base", "basefont", "bgsound", "link", "meta", "noframes", "script", "style", "template",
    "title",
];

/// Elements that break out of foreign (`svg`/`math`) content.
const FOREIGN_BREAK: &[&str] = &[
    "b",
    "big",
    "blockquote",
    "body",
    "br",
    "center",
    "code",
    "dd",
    "div",
    "dl",
    "dt",
    "em",
    "embed",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "head",
    "hr",
    "i",
    "img",
    "li",
    "listing",
    "menu",
    "meta",
    "nobr",
    "ol",
    "p",
    "pre",
    "ruby",
    "s",
    "small",
    "span",
    "strong",
    "strike",
    "sub",
    "sup",
    "table",
    "tt",
    "u",
    "ul",
    "var",
    "html",
    "body",
    "section",
    "article",
    "aside",
    "header",
    "footer",
    "nav",
    "main",
    "form",
    "figure",
    "figcaption",
];

/// Elements that are never pushed onto the stack of open elements.
fn is_void(tag: &str) -> bool {
    tb_dom::void::is_void(tag)
}

/// The tree builder.
pub struct TreeBuilder {
    dom: Dom,
    tokenizer: Tokenizer,
    /// Open elements; `stack[0]` is `<html>`.
    stack: Vec<NodeId>,
    mode: Mode,
    /// Mode to return to when a raw text element ends.
    text_mode: Mode,
    head: Option<NodeId>,
    body: Option<NodeId>,
    tables: usize,
}

impl TreeBuilder {
    /// Build a document from markup.
    pub fn new(input: &str) -> TreeBuilder {
        TreeBuilder {
            dom: Dom::new(),
            tokenizer: Tokenizer::new(input),
            stack: Vec::new(),
            mode: Mode::Initial,
            text_mode: Mode::InBody,
            head: None,
            body: None,
            tables: 0,
        }
    }

    /// Consume the whole input and return the finished document.
    pub fn run(mut self) -> Dom {
        let mut guard = 0usize;
        let limit = self.tokenizer.input_len() + 16;
        loop {
            guard += 1;
            if guard > limit.saturating_mul(4) {
                break; // defensive: never loop forever
            }
            let token = self.tokenizer.next_token();
            if token.is_eof() {
                break;
            }
            self.process(token);
        }
        self.finish();
        self.dom
    }

    /// Always hand back a document with `html`, `head` and `body`, even for
    /// empty or completely broken input (that is what browsers do).
    fn finish(&mut self) {
        self.ensure_root();
        self.ensure_head();
        self.ensure_body();
        if self.dom.doctype.is_empty() {
            self.dom.quirks = true;
        }
    }

    fn current(&self) -> NodeId {
        *self.stack.last().unwrap_or(&self.dom.document())
    }

    /// Insert an element (not pushed) and return it.
    fn insert_element(&mut self, tag: &str, attrs: &[(String, String)], ns: Namespace) -> NodeId {
        let parent = self.current();
        let id = self.dom.create_element_ns(tag, ns);
        for (n, v) in attrs {
            self.dom.set_attr(id, n, v.clone());
        }
        self.dom.appendChild(parent, id);
        id
    }

    fn insert_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let parent = self.current();
        let t = self.dom.create_text(text);
        self.dom.appendChild(parent, t);
    }

    fn insert_comment(&mut self, text: &str) {
        let parent = self.current();
        let c = self.dom.create_comment(text);
        self.dom.appendChild(parent, c);
    }

    fn push(&mut self, id: NodeId) {
        self.stack.push(id);
    }

    fn pop(&mut self) {
        if self.stack.len() > 1 {
            self.stack.pop();
        }
    }

    /// Is `tag` among the open elements (top of stack included)?
    fn has_open(&self, tag: &str) -> bool {
        self.stack.iter().any(|n| self.dom.tag(*n) == Some(tag))
    }

    /// Like [`Self::has_open`] but stops at a scope boundary.
    fn has_in_scope(&self, tag: &str, boundaries: &[&str]) -> bool {
        for n in self.stack.iter().rev() {
            let t = self.dom.tag(*n);
            if t == Some(tag) {
                return true;
            }
            if let Some(t) = t {
                if boundaries.contains(&t) {
                    return false;
                }
            }
        }
        false
    }

    /// Generate implied end tags (`p`, `li`, `dd`, `dt`, `tbody`, ...).
    fn generate_implied_end_tags(&mut self) {
        const IMPLIED: &[&str] = &[
            "p", "dd", "dt", "li", "option", "optgroup", "tbody", "td", "tfoot", "th", "thead",
            "tr",
        ];
        while let Some(top) = self.stack.last().copied() {
            if top == self.dom.document() {
                break;
            }
            let tag: Option<String> = self.dom.tag(top).map(|t| t.to_string());
            match tag.as_deref() {
                Some(t) if IMPLIED.contains(&t) => self.pop(),
                _ => break,
            }
        }
    }

    /// Pop open elements until `tag` is closed.
    fn close_until(&mut self, tag: &str) {
        loop {
            if self.stack.len() <= 1 {
                break;
            }
            let top = match self.stack.last().copied() {
                Some(t) => t,
                None => break,
            };
            let matched = self.dom.tag(top) == Some(tag);
            self.pop();
            if matched {
                break;
            }
        }
    }

    /// Ensure the html/head/body skeleton exists.
    fn ensure_root(&mut self) -> NodeId {
        if self.stack.is_empty() {
            let html = self.dom.create_element("html");
            self.dom.appendChild(self.dom.document(), html);
            self.stack.push(html);
        }
        self.stack[0]
    }

    fn ensure_head(&mut self) -> NodeId {
        self.ensure_root();
        if let Some(h) = self.head {
            return h;
        }
        let html = self.stack[0];
        let head = self.dom.create_element("head");
        self.dom.appendChild(html, head);
        self.head = Some(head);
        head
    }

    fn ensure_body(&mut self) -> NodeId {
        self.ensure_root();
        if let Some(b) = self.body {
            if self.dom.exists(b) {
                return b;
            }
        }
        let head = self.ensure_head();
        let html = self.stack[0];
        let body = self.dom.create_element("body");
        self.dom.appendChild(html, body);
        let _ = head;
        self.body = Some(body);
        // The stack now points at body.
        self.stack.truncate(1);
        self.stack.push(body);
        body
    }

    fn merge_attrs_into_root(&mut self, attrs: &[(String, String)]) {
        let html = self.ensure_root();
        for (n, v) in attrs {
            if !self.dom.has_attr(html, n) {
                self.dom.set_attr(html, n, v.clone());
            }
        }
    }

    /// One token of the algorithm.
    fn process(&mut self, token: Token) {
        match token {
            Token::Doctype {
                name,
                public_id,
                system_id,
            } => self.do_doctype(name, public_id, system_id),
            Token::Comment { text } => match self.mode {
                Mode::Initial | Mode::BeforeHtml | Mode::BeforeHead | Mode::AfterHead => {
                    self.insert_comment(&text)
                }
                _ => self.insert_comment(&text),
            },
            Token::Text { text } => self.do_text(&text),
            Token::StartTag {
                name,
                attrs,
                self_closing,
            } => self.do_start_tag(&name, &attrs, self_closing),
            Token::EndTag { name } => self.do_end_tag(&name),
            Token::Eof => {}
        }
    }

    fn do_doctype(&mut self, name: String, public_id: String, system_id: String) {
        if self.mode == Mode::Initial {
            self.dom.append_doctype(&name, &public_id, &system_id);
            self.dom.quirks = is_quirks_mode(&name, &public_id, &system_id);
            self.mode = Mode::BeforeHtml;
        }
    }

    fn do_text(&mut self, raw: &str) {
        match self.mode {
            Mode::Initial => {
                if !raw.trim().is_empty() {
                    self.mode = Mode::BeforeHtml;
                    // reprocess as markup text
                    self.do_text(raw);
                }
            }
            Mode::BeforeHtml => {
                if raw.trim().is_empty() {
                    return;
                }
                self.mode = Mode::BeforeHead;
                self.ensure_root();
                self.do_text(raw);
            }
            Mode::BeforeHead => {
                if raw.trim().is_empty() {
                    return;
                }
                let head = self.ensure_head();
                self.mode = Mode::InHead;
                self.stack.push(head);
                self.do_text(raw);
            }
            Mode::InHead => {
                if raw.trim().is_empty() {
                    self.insert_text(raw);
                    return;
                }
                // non whitespace text in head ends the head
                if let Some(head) = self.stack.last().copied() {
                    if self.dom.tag(head) == Some("head") {
                        self.pop();
                    }
                }
                self.mode = Mode::AfterHead;
                self.do_text(raw);
            }
            Mode::AfterHead => {
                if raw.trim().is_empty() {
                    self.insert_text(raw);
                    return;
                }
                self.mode = Mode::InBody;
                self.ensure_body();
                self.insert_text(raw);
            }
            Mode::Text => {
                self.insert_text(raw);
            }
            Mode::InBody | Mode::AfterBody | Mode::AfterAfterBody => {
                if self.mode != Mode::InBody {
                    self.mode = Mode::InBody;
                    self.ensure_body();
                }
                if self.in_foreign() {
                    self.insert_text(raw);
                    return;
                }
                self.insert_text(raw);
            }
        }
    }

    /// Are we inside `<svg>`/`<math>` content?
    fn in_foreign(&self) -> bool {
        for n in self.stack.iter().rev() {
            match self.dom.tag(*n) {
                Some("svg") | Some("math") => return true,
                Some("html") => return false,
                _ => {}
            }
        }
        false
    }

    fn do_start_tag(&mut self, name: &str, attrs: &[(String, String)], self_closing: bool) {
        match self.mode {
            Mode::Initial => {
                self.mode = Mode::BeforeHtml;
                self.do_start_tag(name, attrs, self_closing);
            }
            Mode::BeforeHtml => {
                if name == "html" {
                    let html = self.ensure_root();
                    for (n, v) in attrs {
                        self.dom.set_attr(html, n, v.clone());
                    }
                    self.mode = Mode::BeforeHead;
                } else {
                    self.ensure_root();
                    self.mode = Mode::BeforeHead;
                    self.do_start_tag(name, attrs, self_closing);
                }
            }
            Mode::BeforeHead => {
                if name == "head" {
                    let head = self.ensure_head();
                    self.stack.push(head);
                    self.mode = Mode::InHead;
                } else {
                    let head = self.ensure_head();
                    self.stack.push(head);
                    self.mode = Mode::InHead;
                    self.do_start_tag(name, attrs, self_closing);
                }
            }
            Mode::InHead => {
                if HEAD_ELEMENTS.contains(&name) {
                    let el = self.insert_element(name, attrs, Namespace::Html);
                    if is_void(name) || self_closing {
                        // `<meta ... />` and void head elements are not pushed
                        if name == "title"
                            || name == "style"
                            || name == "script"
                            || name == "noscript"
                        {
                            self.push(el);
                            self.start_raw_text(name);
                        }
                        return;
                    }
                    match name {
                        "title" | "style" | "script" | "noscript" => {
                            self.push(el);
                            self.start_raw_text(name);
                        }
                        _ => {
                            // link/meta/base stay closed immediately
                        }
                    }
                    return;
                }
                match name {
                    "head" => {}
                    "body" => {
                        self.mode = Mode::AfterHead;
                        self.ensure_body();
                        self.mode = Mode::InBody;
                        self.do_start_tag(name, attrs, self_closing);
                    }
                    "html" => self.merge_attrs_into_root(attrs),
                    _ => {
                        // stray content: close head, continue in body
                        if let Some(head) = self.stack.last().copied() {
                            if self.dom.tag(head) == Some("head") {
                                self.pop();
                            }
                        }
                        self.mode = Mode::AfterHead;
                        self.do_start_tag(name, attrs, self_closing);
                    }
                }
            }
            Mode::AfterHead => match name {
                "body" => {
                    self.ensure_body();
                    for (n, v) in attrs {
                        if let Some(b) = self.body {
                            self.dom.set_attr(b, n, v.clone());
                        }
                    }
                    self.mode = Mode::InBody;
                }
                "frameset" => {
                    self.ensure_body();
                    let el = self.insert_element(name, attrs, Namespace::Html);
                    self.push(el);
                    self.mode = Mode::InBody;
                }
                "html" => self.merge_attrs_into_root(attrs),
                "head" => {}
                _ => {
                    self.mode = Mode::InBody;
                    self.ensure_body();
                    self.do_start_tag(name, attrs, self_closing);
                }
            },
            Mode::Text => {
                // A start tag inside raw text (e.g. unclosed <script>) just
                // becomes text; the tokenizer already treated it as raw.
                self.do_start_tag_in_body(name, attrs, self_closing);
            }
            _ => self.do_start_tag_in_body(name, attrs, self_closing),
        }
    }

    /// Enter raw text mode for `script`, `style`, `title`, `textarea`, ...
    fn start_raw_text(&mut self, name: &str) {
        let kind = raw_kind_of(name);
        if kind != RawKind::None {
            self.text_mode = if self.mode == Mode::InHead {
                Mode::InHead
            } else {
                Mode::InBody
            };
            self.tokenizer.switch_to_raw_text(kind, name);
            self.mode = Mode::Text;
        }
    }

    fn do_start_tag_in_body(&mut self, name: &str, attrs: &[(String, String)], self_closing: bool) {
        // Foreign content: svg/math have their own rules.
        if self.in_foreign() {
            if FOREIGN_BREAK.contains(&name) {
                // close the foreign element and reprocess as HTML
                self.pop();
                self.do_start_tag_in_body(name, attrs, self_closing);
                return;
            }
            let ns = if self.has_open("svg") {
                Namespace::Svg
            } else {
                Namespace::MathMl
            };
            let el = self.insert_element(name, attrs, ns);
            if !self_closing {
                self.push(el);
            }
            return;
        }

        match name {
            "html" => self.merge_attrs_into_root(attrs),
            "head" | "frameset" => {}
            "body" => {
                if let Some(b) = self.body {
                    for (n, v) in attrs {
                        if !self.dom.has_attr(b, n) {
                            self.dom.set_attr(b, n, v.clone());
                        }
                    }
                }
            }
            "form" => {
                self.close_p_if_open();
                let el = self.insert_element(name, attrs, Namespace::Html);
                self.push(el);
            }
            "p" => {
                self.close_p_if_open();
                let el = self.insert_element("p", attrs, Namespace::Html);
                self.push(el);
            }
            "li" => {
                self.close_list_item();
                self.close_p_if_open();
                let el = self.insert_element("li", attrs, Namespace::Html);
                self.push(el);
            }
            "dd" | "dt" => {
                for t in ["dd", "dt"] {
                    if self.has_in_scope(t, &["ul", "ol", "div", "body", "html"]) {
                        self.close_until(t);
                    }
                }
                self.close_p_if_open();
                let el = self.insert_element(name, attrs, Namespace::Html);
                self.push(el);
            }
            "option" => {
                if self.dom.tag(self.current()) == Some("option") {
                    self.pop();
                }
                let el = self.insert_element(name, attrs, Namespace::Html);
                self.push(el);
            }
            "optgroup" => {
                if self.dom.tag(self.current()) == Some("option") {
                    self.pop();
                }
                if self.dom.tag(self.current()) == Some("optgroup") {
                    self.pop();
                }
                let el = self.insert_element(name, attrs, Namespace::Html);
                self.push(el);
            }
            "tr" => {
                self.implied_table_row();
                let el = self.insert_element("tr", attrs, Namespace::Html);
                self.push(el);
            }
            "td" | "th" => {
                if matches!(self.dom.tag(self.current()), Some("td") | Some("th")) {
                    self.pop();
                    if self.dom.tag(self.current()) == Some("tr") {
                        // leave tr open, it will be closed by the next <tr>
                    }
                }
                self.implied_table_row();
                let el = self.insert_element(name, attrs, Namespace::Html);
                self.push(el);
            }
            "table" => {
                self.close_p_if_open();
                let el = self.insert_element("table", attrs, Namespace::Html);
                self.push(el);
                self.tables += 1;
            }
            "br" => {
                // `<br>` closes an open p? No — it is inserted inside it.
                let el = self.insert_element("br", attrs, Namespace::Html);
                let _ = el;
            }
            "a" | "b" | "i" | "em" | "strong" | "u" | "span" | "small" | "code" | "font"
            | "label" | "button" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                if matches!(name, "h1" | "h2" | "h3" | "h4" | "h5" | "h6") {
                    self.close_p_if_open();
                    if matches!(
                        self.dom.tag(self.current()),
                        Some("h1") | Some("h2") | Some("h3") | Some("h4") | Some("h5") | Some("h6")
                    ) {
                        self.pop();
                    }
                }
                let el = self.insert_element(name, attrs, Namespace::Html);
                self.push(el);
            }
            _ if is_void(name) => {
                self.insert_element(name, attrs, Namespace::Html);
            }
            _ if raw_kind_of(name) != RawKind::None => {
                self.close_p_if_open();
                let el = self.insert_element(name, attrs, Namespace::Html);
                self.push(el);
                self.start_raw_text(name);
            }
            _ if CLOSES_P.contains(&name) => {
                self.close_p_if_open();
                let el = self.insert_element(name, attrs, Namespace::Html);
                self.push(el);
            }
            _ => {
                let ns = match name {
                    "svg" => Namespace::Svg,
                    "math" => Namespace::MathMl,
                    _ => Namespace::Html,
                };
                let el = self.insert_element(name, attrs, ns);
                if !self_closing {
                    self.push(el);
                }
            }
        }
    }

    /// `<p>` may not contain block content, and two `<p>` may not nest.
    fn close_p_if_open(&mut self) {
        if self.has_in_scope("p", &["html", "table", "tbody", "tr", "td", "th", "button"]) {
            self.close_until("p");
        }
    }

    fn close_list_item(&mut self) {
        if self.has_in_scope("li", &["ul", "ol", "html"]) {
            self.close_until("li");
        }
    }

    /// `<tr>`/`<td>` inside a `<table>` imply a `<tbody>`.
    fn implied_table_row(&mut self) {
        if self.dom.tag(self.current()) == Some("table") {
            let tbody = self.insert_element("tbody", &[], Namespace::Html);
            self.push(tbody);
        }
    }

    fn do_end_tag(&mut self, name: &str) {
        match self.mode {
            Mode::Text => {
                // the raw text element closed
                if self.stack.len() > 1 {
                    self.pop();
                }
                self.mode = self.text_mode;
            }
            Mode::InHead => match name {
                "head" => {
                    if self.dom.tag(self.current()) == Some("head") {
                        self.pop();
                    }
                    self.mode = Mode::AfterHead;
                }
                "body" => {
                    self.mode = Mode::AfterHead;
                    self.do_end_tag(name);
                }
                "html" => {
                    self.mode = Mode::AfterHead;
                    self.do_end_tag(name);
                }
                "script" => {
                    if self.dom.tag(self.current()) == Some("script") {
                        self.pop();
                    }
                }
                "title" | "style" if self.dom.tag(self.current()) == Some(name) => {
                    self.pop();
                }
                _ => {}
            },
            Mode::Initial | Mode::BeforeHtml | Mode::BeforeHead | Mode::AfterHead => {
                // implicit body, then handle it as body content
                if name == "head" {
                    self.mode = Mode::AfterHead;
                    return;
                }
                self.mode = Mode::InBody;
                self.ensure_body();
                self.do_end_tag(name);
            }
            Mode::InBody => self.do_end_tag_in_body(name),
            Mode::AfterBody => match name {
                "body" => self.mode = Mode::AfterAfterBody,
                "html" => self.mode = Mode::AfterAfterBody,
                _ => {
                    self.mode = Mode::InBody;
                    self.do_end_tag(name);
                }
            },
            Mode::AfterAfterBody => {
                if name != "html" {
                    self.mode = Mode::InBody;
                    self.do_end_tag(name);
                }
            }
        }
    }

    fn do_end_tag_in_body(&mut self, name: &str) {
        // `</meta>`, `</br>`, `</img>` ... appear in the wild (taz sends them).
        if is_void(name) {
            return;
        }
        if self.in_foreign() {
            let tag = self.dom.tag(self.current());
            if tag == Some(name) {
                self.pop();
            } else {
                // search the foreign part of the stack
                let mut idx = self.stack.len().saturating_sub(1);
                while idx > 0 {
                    if self.dom.tag(self.stack[idx - 1]) == Some(name) {
                        self.stack.truncate(idx - 1);
                        break;
                    }
                    idx -= 1;
                }
            }
            return;
        }

        match name {
            "p" => self.close_p_if_open(),
            "li" => self.close_list_item(),
            "dd" | "dt" => {
                for t in ["dd", "dt"] {
                    if self.has_in_scope(t, &["ul", "ol", "div", "html"]) {
                        self.close_until(t);
                    }
                }
            }
            "body" => self.mode = Mode::AfterBody,
            "html" => {
                self.mode = Mode::AfterAfterBody;
            }
            "table" => {
                if self.has_open("table") {
                    self.close_until("table");
                    self.tables = self.tables.saturating_sub(1);
                }
            }
            "form" => {
                if self.has_open("form") {
                    self.close_until("form");
                }
            }
            "td" | "th" => {
                if self.has_in_scope(name, &["table", "html"]) {
                    self.close_until(name);
                }
            }
            "tr" => {
                if self.has_in_scope("tr", &["table", "html"]) {
                    self.close_until("tr");
                }
            }
            "tbody" | "thead" | "tfoot" => {
                if self.has_open(name) {
                    self.close_until(name);
                }
            }
            "head" => {}
            _ => {
                self.generate_implied_end_tags();
                // Generic: close the nearest matching open element, if any.
                if let Some(idx) = self
                    .stack
                    .iter()
                    .rposition(|n| self.dom.tag(*n) == Some(name))
                {
                    if idx >= 1 {
                        self.stack.truncate(idx);
                    }
                }
                // no match => ignore the stray end tag
            }
        }
    }
}

/// Quirks mode detection (simplified).
fn is_quirks_mode(name: &str, public_id: &str, system_id: &str) -> bool {
    if name != "html" {
        return true;
    }
    if !public_id.is_empty() {
        let p = public_id.to_ascii_lowercase();
        if p.contains("-//war3netscape//")
            || p.contains("-//netscape communications//")
            || p.contains("-//microsoft//ie 4")
            || p.contains("-//microsoft//ie 5")
            || p.contains("+//silmaril//dtd html pro v0r11")
        {
            return true;
        }
    }
    if !system_id.is_empty() && system_id.starts_with("http://") && system_id.contains("legacy") {
        return true;
    }
    false
}

/// Parse a full HTML document into a [`Dom`].
pub fn parse_document(html: &str) -> Dom {
    TreeBuilder::new(html).run()
}

/// Parse an HTML fragment (`innerHTML`) inside `container`.
///
/// Returns the container node that holds the parsed children.
pub fn parse_fragment(dom: &mut Dom, container: NodeId, html: &str) {
    // A tiny document is parsed and the matching element's children are moved.
    let mut sub = parse_document(html);
    let tag = dom.tag(container).unwrap_or("div").to_string();
    // find the element with the same tag as the container, else use body
    let source_parent = sub
        .elements()
        .into_iter()
        .find(|n| sub.tag(*n) == Some(tag.as_str()) && sub.parent(*n).is_some())
        .or_else(|| sub.get_elements_by_tag("body").into_iter().next())
        .unwrap_or(sub.document());
    let kids = sub.children(source_parent);
    for k in kids {
        move_node(dom, &mut sub, k, container);
    }
}

/// Move one node out of `other` into `dom` (deep copy, then drop source).
fn move_node(dom: &mut Dom, other: &mut Dom, node: NodeId, target: NodeId) {
    let copy = clone_across(other, dom, node);
    if let Some(copy) = copy {
        dom.appendChild(target, copy);
    }
}

fn clone_across(from: &Dom, into: &mut Dom, node: NodeId) -> Option<NodeId> {
    use tb_dom::NodeKind;
    let n = from.get(node)?;
    let created = match &n.kind {
        NodeKind::Document => return None,
        NodeKind::Doctype { .. } => return None,
        NodeKind::Element(e) => {
            let id = into.create_element_ns(&e.tag, e.namespace);
            for a in &e.attrs {
                into.set_attr(id, &a.name, a.value.clone());
            }
            id
        }
        NodeKind::Text(t) => into.create_text(t),
        NodeKind::Comment(c) => into.create_comment(c),
        NodeKind::ProcessingInstruction { .. } => return None,
    };
    for c in from.children(node) {
        if let Some(child) = clone_across(from, into, c) {
            into.appendChild(created, child);
        }
    }
    Some(created)
}
