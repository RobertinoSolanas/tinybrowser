//! HTML tokenizer: a state machine over `char`s producing [`Token`]s.
//!
//! It follows the shape of the WHATWG tokenizer (data / tag open / attribute
//! states / bogus comment / raw text) but is deliberately forgiving: real pages
//! contain `</meta>`, unquoted attribute values, `<div/>`, comments that never
//! close and scripts full of `<`.

use crate::entities;

/// A token emitted by [`Tokenizer`].
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Doctype {
        name: String,
        public_id: String,
        system_id: String,
    },
    StartTag {
        name: String,
        attrs: Vec<(String, String)>,
        self_closing: bool,
    },
    EndTag {
        name: String,
    },
    Text {
        text: String,
    },
    Comment {
        text: String,
    },
    Eof,
}

impl Token {
    pub fn is_eof(&self) -> bool {
        matches!(self, Token::Eof)
    }
}

/// Elements whose content is *raw text* (no markup, no entities).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawKind {
    None,
    /// `script`, `iframe`, `noembed`, `noframes`
    Script,
    /// `style`, `title`, `textarea`, `xmp`
    RawText,
    /// `plaintext`: everything until EOF
    Plaintext,
}

/// Element → raw text kind.
pub fn raw_kind_of(tag: &str) -> RawKind {
    match tag {
        "script" | "iframe" | "noembed" | "noframes" => RawKind::Script,
        "style" | "title" | "textarea" | "xmp" => RawKind::RawText,
        "plaintext" => RawKind::Plaintext,
        _ => RawKind::None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Data,
    TagOpen,
    EndTagOpen,
    TagName,
    BeforeAttrName,
    AttrName,
    AfterAttrName,
    BeforeAttrValue,
    AttrValueDouble,
    AttrValueSingle,
    AttrValueUnquoted,
    AfterAttrValue,
    SelfClosing,
    MarkupDeclOpen,
    BogusComment,
    Comment,
    DoctypeBeforeName,
    DoctypeName,
    DoctypeAfterName,
    DoctypeId,
    DoctypeBogus,
    RawText,
    CData,
}

/// The tokenizer.
pub struct Tokenizer {
    input: Vec<char>,
    pos: usize,
    state: State,
    /// Text / comment / doctype-name accumulator.
    buf: String,
    tag_name: String,
    is_end_tag: bool,
    attrs: Vec<(String, String)>,
    attr_name: String,
    attr_value: String,
    self_closing: bool,
    raw: RawKind,
    raw_tag: String,
    pub_id: String,
    sys_id: String,
    in_public: bool,
    /// A token that has to be handed out after the current one.
    pending: Option<Token>,
}

impl Default for Tokenizer {
    fn default() -> Self {
        Tokenizer::new("")
    }
}

impl Tokenizer {
    pub fn new(input: &str) -> Tokenizer {
        // Spec: CR, CRLF and LF are all LF.
        let normalized = input.replace("\r\n", "\n").replace('\r', "\n");
        Tokenizer {
            input: normalized.chars().collect(),
            pos: 0,
            state: State::Data,
            buf: String::new(),
            tag_name: String::new(),
            is_end_tag: false,
            attrs: Vec::new(),
            attr_name: String::new(),
            attr_value: String::new(),
            self_closing: false,
            raw: RawKind::None,
            raw_tag: String::new(),
            pub_id: String::new(),
            sys_id: String::new(),
            in_public: false,
            pending: None,
        }
    }

    /// The tree builder calls this after pushing a raw-text element; the
    /// tokenizer then ignores markup until the matching end tag.
    pub fn switch_to_raw_text(&mut self, kind: RawKind, tag: &str) {
        if kind == RawKind::None {
            return;
        }
        self.raw = kind;
        self.raw_tag = tag.to_ascii_lowercase();
        self.state = State::RawText;
        self.buf.clear();
    }

    /// Number of characters in the (normalised) input.
    pub fn input_len(&self) -> usize {
        self.input.len()
    }

    fn peek(&self, ahead: usize) -> Option<char> {
        self.input.get(self.pos + ahead).copied()
    }

    fn at_end(&self) -> bool {
        self.pos >= self.input.len()
    }

    fn starts_with_at(&self, at: usize, s: &str) -> bool {
        s.chars()
            .enumerate()
            .all(|(i, c)| self.input.get(at + i) == Some(&c))
    }

    fn starts_with(&self, s: &str) -> bool {
        self.starts_with_at(self.pos, s)
    }

    fn starts_with_ic_at(&self, at: usize, s: &str) -> bool {
        s.chars().enumerate().all(|(i, c)| {
            self.input
                .get(at + i)
                .map(|x| x.eq_ignore_ascii_case(&c))
                .unwrap_or(false)
        })
    }

    fn starts_with_ic(&self, s: &str) -> bool {
        self.starts_with_ic_at(self.pos, s)
    }

    /// Read the next token (returns [`Token::Eof`] once at the end).
    pub fn next_token(&mut self) -> Token {
        if let Some(t) = self.pending.take() {
            return t;
        }
        loop {
            match self.state {
                // ------------------------------------------------------------
                State::Data => {
                    if self.at_end() {
                        return self.take_text_or_eof();
                    }
                    let c = self.input[self.pos];
                    self.pos += 1;
                    match c {
                        '<' => {
                            let next = self.peek(0);
                            if next == Some('/')
                                || next.map(|c| c.is_ascii_alphabetic()).unwrap_or(false)
                                || next == Some('!')
                                || next == Some('?')
                            {
                                if self.buf.is_empty() {
                                    self.state = State::TagOpen;
                                } else {
                                    self.state = State::TagOpen;
                                    return self.take_text_token();
                                }
                            } else {
                                self.buf.push('<');
                            }
                        }
                        '&' => {
                            self.pos -= 1;
                            match entities::decode_at(&self.input, self.pos) {
                                Some((s, n)) => {
                                    self.buf.push_str(&s);
                                    self.pos += n;
                                }
                                None => {
                                    self.buf.push('&');
                                    self.pos += 1;
                                }
                            }
                        }
                        '\0' => self.buf.push('\u{fffd}'),
                        _ => self.buf.push(c),
                    }
                }
                // ------------------------------------------------------------
                State::TagOpen => {
                    let Some(c) = self.peek(0) else {
                        self.pos += 1;
                        self.state = State::Data;
                        return Token::Text { text: "<".into() };
                    };
                    match c {
                        '!' => {
                            self.pos += 1;
                            self.state = State::MarkupDeclOpen;
                        }
                        '/' => {
                            self.pos += 1;
                            self.state = State::EndTagOpen;
                        }
                        '?' => {
                            self.pos += 1;
                            self.buf.clear();
                            self.state = State::BogusComment;
                        }
                        c if c.is_ascii_alphabetic() => {
                            self.tag_name.clear();
                            self.attrs.clear();
                            self.self_closing = false;
                            self.is_end_tag = false;
                            self.state = State::TagName;
                        }
                        _ => {
                            self.pos += 1;
                            self.buf.push('<');
                            self.buf.push(c);
                            self.state = State::Data;
                        }
                    }
                }
                // ------------------------------------------------------------
                State::EndTagOpen => match self.peek(0) {
                    Some('>') => {
                        self.pos += 1;
                        self.state = State::Data;
                    }
                    Some(c) if c.is_ascii_alphabetic() => {
                        self.tag_name.clear();
                        self.attrs.clear();
                        self.self_closing = false;
                        self.is_end_tag = true;
                        self.state = State::TagName;
                    }
                    _ => {
                        self.pos += 1;
                        self.buf.clear();
                        self.state = State::BogusComment;
                    }
                },
                // ------------------------------------------------------------
                State::TagName => {
                    let Some(c) = self.peek(0) else {
                        self.pos += 1;
                        return self.emit_tag();
                    };
                    self.pos += 1;
                    match c {
                        ' ' | '\t' | '\n' | '\u{c}' => self.state = State::BeforeAttrName,
                        '/' => self.state = State::SelfClosing,
                        '>' => return self.emit_tag(),
                        '\0' => self.tag_name.push('\u{fffd}'),
                        c => self.tag_name.push(c.to_ascii_lowercase()),
                    }
                }
                // ------------------------------------------------------------
                State::BeforeAttrName => {
                    let Some(c) = self.peek(0) else {
                        self.pos += 1;
                        return self.emit_tag();
                    };
                    match c {
                        ' ' | '\t' | '\n' | '\u{c}' => self.pos += 1,
                        '/' => {
                            self.pos += 1;
                            self.state = State::SelfClosing;
                        }
                        '>' => {
                            self.pos += 1;
                            return self.emit_tag();
                        }
                        _ => {
                            self.attr_name.clear();
                            self.attr_value.clear();
                            self.state = State::AttrName;
                        }
                    }
                }
                // ------------------------------------------------------------
                State::AttrName => {
                    let Some(c) = self.peek(0) else {
                        self.pos += 1;
                        self.push_attr();
                        return self.emit_tag();
                    };
                    match c {
                        ' ' | '\t' | '\n' | '\u{c}' => {
                            self.pos += 1;
                            self.push_attr();
                            self.state = State::AfterAttrName;
                        }
                        '/' => {
                            self.pos += 1;
                            self.push_attr();
                            self.state = State::SelfClosing;
                        }
                        '=' => {
                            // keep the name, the value follows
                            self.pos += 1;
                            self.state = State::BeforeAttrValue;
                        }
                        '>' => {
                            self.pos += 1;
                            self.push_attr();
                            return self.emit_tag();
                        }
                        '\0' => {
                            self.attr_name.push('\u{fffd}');
                            self.pos += 1;
                        }
                        c => {
                            self.attr_name.push(c.to_ascii_lowercase());
                            self.pos += 1;
                        }
                    }
                }
                // ------------------------------------------------------------
                State::AfterAttrName => {
                    let Some(c) = self.peek(0) else {
                        self.pos += 1;
                        return self.emit_tag();
                    };
                    match c {
                        ' ' | '\t' | '\n' | '\u{c}' => self.pos += 1,
                        '/' => {
                            self.pos += 1;
                            self.state = State::SelfClosing;
                        }
                        '=' => {
                            self.pos += 1;
                            self.state = State::BeforeAttrValue;
                        }
                        '>' => {
                            self.pos += 1;
                            return self.emit_tag();
                        }
                        _ => {
                            // attribute without value, then another attribute
                            self.attr_name.clear();
                            self.attr_value.clear();
                            self.state = State::AttrName;
                        }
                    }
                }
                // ------------------------------------------------------------
                State::BeforeAttrValue => {
                    let Some(c) = self.peek(0) else {
                        self.pos += 1;
                        self.push_attr();
                        return self.emit_tag();
                    };
                    match c {
                        ' ' | '\t' | '\n' | '\u{c}' => self.pos += 1,
                        '"' => {
                            self.pos += 1;
                            self.state = State::AttrValueDouble;
                        }
                        '\'' => {
                            self.pos += 1;
                            self.state = State::AttrValueSingle;
                        }
                        '>' => {
                            self.pos += 1;
                            self.push_attr();
                            return self.emit_tag();
                        }
                        _ => self.state = State::AttrValueUnquoted,
                    }
                }
                // ------------------------------------------------------------
                State::AttrValueDouble | State::AttrValueSingle | State::AttrValueUnquoted => {
                    let quote = match self.state {
                        State::AttrValueDouble => '"',
                        State::AttrValueSingle => '\'',
                        _ => '\0',
                    };
                    loop {
                        let Some(c) = self.peek(0) else {
                            self.pos += 1;
                            self.push_attr();
                            return self.emit_tag();
                        };
                        if quote != '\0' {
                            if c == quote {
                                self.pos += 1;
                                self.push_attr();
                                self.state = State::AfterAttrValue;
                                break;
                            }
                            if c == '&' {
                                match entities::decode_at(&self.input, self.pos) {
                                    Some((s, n)) => {
                                        self.attr_value.push_str(&s);
                                        self.pos += n;
                                    }
                                    None => {
                                        self.attr_value.push('&');
                                        self.pos += 1;
                                    }
                                }
                                continue;
                            }
                            self.attr_value.push(if c == '\0' { '\u{fffd}' } else { c });
                            self.pos += 1;
                        } else {
                            match c {
                                ' ' | '\t' | '\n' | '\u{c}' => {
                                    self.pos += 1;
                                    self.push_attr();
                                    self.state = State::BeforeAttrName;
                                    break;
                                }
                                '>' => {
                                    self.pos += 1;
                                    self.push_attr();
                                    return self.emit_tag();
                                }
                                '&' => match entities::decode_at(&self.input, self.pos) {
                                    Some((s, n)) => {
                                        self.attr_value.push_str(&s);
                                        self.pos += n;
                                    }
                                    None => {
                                        self.attr_value.push('&');
                                        self.pos += 1;
                                    }
                                },
                                _ => {
                                    self.attr_value.push(if c == '\0' { '\u{fffd}' } else { c });
                                    self.pos += 1;
                                }
                            }
                        }
                    }
                }
                // ------------------------------------------------------------
                State::AfterAttrValue => {
                    let Some(c) = self.peek(0) else {
                        self.pos += 1;
                        return self.emit_tag();
                    };
                    match c {
                        ' ' | '\t' | '\n' | '\u{c}' => {
                            self.pos += 1;
                            self.state = State::BeforeAttrName;
                        }
                        '/' => {
                            self.pos += 1;
                            self.state = State::SelfClosing;
                        }
                        '>' => {
                            self.pos += 1;
                            return self.emit_tag();
                        }
                        _ => {
                            // run-on attributes: `href=/a href=/b`
                            self.attr_name.clear();
                            self.attr_value.clear();
                            self.state = State::AttrName;
                        }
                    }
                }
                // ------------------------------------------------------------
                State::SelfClosing => match self.peek(0) {
                    Some('>') => {
                        self.pos += 1;
                        self.self_closing = true;
                        return self.emit_tag();
                    }
                    None => {
                        self.pos += 1;
                        return self.emit_tag();
                    }
                    _ => self.state = State::BeforeAttrName,
                },
                // ------------------------------------------------------------
                State::MarkupDeclOpen => {
                    if self.starts_with("--") {
                        self.pos += 2;
                        self.buf.clear();
                        self.state = State::Comment;
                    } else if self.starts_with("[CDATA[") {
                        self.pos += 9;
                        self.buf.clear();
                        self.state = State::CData;
                    } else if self.starts_with_ic("doctype") {
                        self.pos += 7;
                        self.buf.clear();
                        self.pub_id.clear();
                        self.sys_id.clear();
                        self.state = State::DoctypeBeforeName;
                    } else {
                        self.buf.clear();
                        self.state = State::BogusComment;
                    }
                }
                // ------------------------------------------------------------
                State::BogusComment => loop {
                    let Some(c) = self.peek(0) else {
                        self.pos += 1;
                        self.state = State::Data;
                        return Token::Comment {
                            text: std::mem::take(&mut self.buf),
                        };
                    };
                    self.pos += 1;
                    if c == '>' {
                        self.state = State::Data;
                        return Token::Comment {
                            text: std::mem::take(&mut self.buf),
                        };
                    }
                    self.buf.push(c);
                },
                // ------------------------------------------------------------
                State::Comment => loop {
                    let Some(c) = self.peek(0) else {
                        self.pos += 1;
                        self.state = State::Data;
                        return Token::Comment {
                            text: std::mem::take(&mut self.buf),
                        };
                    };
                    if c == '-' && self.starts_with_at(self.pos, "-->") {
                        self.pos += 3;
                        self.state = State::Data;
                        return Token::Comment {
                            text: std::mem::take(&mut self.buf),
                        };
                    }
                    self.pos += 1;
                    self.buf.push(c);
                },
                // ------------------------------------------------------------
                State::DoctypeBeforeName => match self.peek(0) {
                    None => {
                        self.pos += 1;
                        self.state = State::Data;
                        return self.emit_doctype();
                    }
                    Some('>') => {
                        self.pos += 1;
                        self.state = State::Data;
                        return self.emit_doctype();
                    }
                    Some(c) if matches!(c, ' ' | '\t' | '\n' | '\u{c}') => self.pos += 1,
                    Some(_) => self.state = State::DoctypeName,
                },
                State::DoctypeName => {
                    let Some(c) = self.peek(0) else {
                        self.pos += 1;
                        self.state = State::Data;
                        return self.emit_doctype();
                    };
                    match c {
                        '>' => {
                            self.pos += 1;
                            self.state = State::Data;
                            return self.emit_doctype();
                        }
                        ' ' | '\t' | '\n' | '\u{c}' => {
                            self.pos += 1;
                            self.state = State::DoctypeAfterName;
                        }
                        c => {
                            self.buf.push(c.to_ascii_lowercase());
                            self.pos += 1;
                        }
                    }
                }
                State::DoctypeAfterName => {
                    let Some(c) = self.peek(0) else {
                        self.pos += 1;
                        self.state = State::Data;
                        return self.emit_doctype();
                    };
                    match c {
                        '>' => {
                            self.pos += 1;
                            self.state = State::Data;
                            return self.emit_doctype();
                        }
                        ' ' | '\t' | '\n' | '\u{c}' => self.pos += 1,
                        _ => {
                            if self.starts_with_ic("public") {
                                self.pos += 6;
                                self.state = State::DoctypeId;
                                self.in_public = true;
                            } else if self.starts_with_ic("system") {
                                self.pos += 6;
                                self.state = State::DoctypeId;
                                self.in_public = false;
                            } else {
                                self.state = State::DoctypeBogus;
                            }
                        }
                    }
                }
                State::DoctypeId => {
                    // optional whitespace then an optional quoted string
                    loop {
                        let Some(c) = self.peek(0) else {
                            self.pos += 1;
                            self.state = State::Data;
                            return self.emit_doctype();
                        };
                        match c {
                            ' ' | '\t' | '\n' | '\u{c}' => self.pos += 1,
                            '"' | '\'' => {
                                let quote = c;
                                self.pos += 1;
                                loop {
                                    let Some(c2) = self.peek(0) else {
                                        self.pos += 1;
                                        self.state = State::Data;
                                        return self.emit_doctype();
                                    };
                                    self.pos += 1;
                                    if c2 == quote {
                                        break;
                                    }
                                    if self.in_public {
                                        self.pub_id.push(c2);
                                    } else {
                                        self.sys_id.push(c2);
                                    }
                                }
                                self.state = State::DoctypeAfterName;
                                break;
                            }
                            '>' => {
                                self.pos += 1;
                                self.state = State::Data;
                                return self.emit_doctype();
                            }
                            _ => {
                                // unquoted identifier we do not care about
                                self.state = State::DoctypeAfterName;
                                self.pos += 1;
                                break;
                            }
                        }
                    }
                }
                State::DoctypeBogus => loop {
                    let Some(c) = self.peek(0) else {
                        self.pos += 1;
                        self.state = State::Data;
                        return self.emit_doctype();
                    };
                    self.pos += 1;
                    if c == '>' {
                        self.state = State::Data;
                        return self.emit_doctype();
                    }
                },
                // ------------------------------------------------------------
                State::RawText => {
                    if self.raw == RawKind::Plaintext {
                        while !self.at_end() {
                            self.buf.push(self.input[self.pos]);
                            self.pos += 1;
                        }
                        self.raw = RawKind::None;
                        self.state = State::Data;
                        let text = std::mem::take(&mut self.buf);
                        if !text.is_empty() {
                            return Token::Text { text };
                        }
                        return Token::Eof;
                    }
                    while !self.at_end() {
                        let c = self.input[self.pos];
                        if c == '<'
                            && self.peek(1) == Some('/')
                            && self.matches_raw_end_tag(self.pos)
                        {
                            // consume "</" + name + up to ">"
                            self.pos += 2;
                            self.tag_name.clear();
                            self.attrs.clear();
                            self.is_end_tag = true;
                            while let Some(c) = self.peek(0) {
                                if matches!(c, '>' | ' ' | '\t' | '\n' | '\u{c}' | '/') {
                                    break;
                                }
                                self.tag_name.push(c.to_ascii_lowercase());
                                self.pos += 1;
                            }
                            self.state = State::TagName;
                            self.raw = RawKind::None;
                            let text = std::mem::take(&mut self.buf);
                            if !text.is_empty() {
                                // hand out the text now, the end tag comes next
                                return Token::Text { text };
                            }
                            return self.emit_tag();
                        }
                        self.buf.push(c);
                        self.pos += 1;
                    }
                    // EOF inside raw text
                    self.state = State::Data;
                    self.raw = RawKind::None;
                    let text = std::mem::take(&mut self.buf);
                    return if text.is_empty() {
                        Token::Eof
                    } else {
                        Token::Text { text }
                    };
                }
                // ------------------------------------------------------------
                State::CData => loop {
                    let Some(c) = self.peek(0) else {
                        self.pos += 1;
                        self.state = State::Data;
                        let text = std::mem::take(&mut self.buf);
                        return if text.is_empty() {
                            Token::Eof
                        } else {
                            Token::Text { text }
                        };
                    };
                    if c == ']' && self.starts_with_at(self.pos, "]]>") {
                        self.pos += 3;
                        self.state = State::Data;
                        let text = std::mem::take(&mut self.buf);
                        return if text.is_empty() {
                            Token::Eof
                        } else {
                            Token::Text { text }
                        };
                    }
                    self.pos += 1;
                    self.buf.push(c);
                },
            }
        }
    }

    fn matches_raw_end_tag(&self, at: usize) -> bool {
        if self.raw_tag.is_empty() {
            return false;
        }
        let mut i = at + 2;
        for c in self.raw_tag.chars() {
            match self.input.get(i) {
                Some(x) if x.eq_ignore_ascii_case(&c) => i += 1,
                _ => return false,
            }
        }
        matches!(
            self.input.get(i),
            Some('>') | Some(' ') | Some('\t') | Some('\n') | Some('\u{c}') | Some('/') | None
        )
    }

    fn push_attr(&mut self) {
        if self.attr_name.is_empty() {
            self.attr_value.clear();
            return;
        }
        let name = std::mem::take(&mut self.attr_name);
        let value = std::mem::take(&mut self.attr_value);
        if !self.attrs.iter().any(|(n, _)| *n == name) {
            self.attrs.push((name, value));
        }
    }

    fn take_text_token(&mut self) -> Token {
        Token::Text {
            text: std::mem::take(&mut self.buf),
        }
    }

    fn take_text_or_eof(&mut self) -> Token {
        if self.buf.is_empty() {
            Token::Eof
        } else {
            self.take_text_token()
        }
    }

    fn emit_tag(&mut self) -> Token {
        self.state = State::Data;
        let name = std::mem::take(&mut self.tag_name);
        if self.is_end_tag {
            self.is_end_tag = false;
            self.attrs.clear();
            return Token::EndTag { name };
        }
        Token::StartTag {
            name,
            attrs: std::mem::take(&mut self.attrs),
            self_closing: std::mem::take(&mut self.self_closing),
        }
    }

    fn emit_doctype(&mut self) -> Token {
        Token::Doctype {
            name: std::mem::take(&mut self.buf),
            public_id: std::mem::take(&mut self.pub_id),
            system_id: std::mem::take(&mut self.sys_id),
        }
    }
}
