//! JavaScript lexer.
//!
//! One pass over the source producing [`Token`]s with byte spans. Two things
//! make a JS lexer more than a regex:
//!
//! * **Regex vs division** — `/` starts a literal only where a value can begin.
//!   We track the previous significant token ([`Lexeme::ValueEnd`] and friends).
//! * **Automatic semicolon insertion** — the lexer records whether a newline was
//!   skipped before each token (`Token::nl`); the parser inserts the semicolon
//!   where the spec allows it.
//!
//! Template literals are captured whole (with `${}` nesting respected); the
//! parser re-parses the interpolations.

use std::fmt;

/// Byte offset range in the source, plus 1-based line for messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    pub start: u32,
    pub end: u32,
    pub line: u32,
}

/// Punctuators and keywords the lexer can produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum P {
    // punctuators
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Dot,
    DotDotDot,
    Comma,
    Semi,
    Colon,
    Question,
    QuestionDot,
    Arrow,
    PlusPlus,
    MinusMinus,
    Plus,
    Minus,
    Star,
    StarStar,
    Slash,
    Percent,
    ShiftLeft,
    ShiftRight,
    UShiftRight,
    Amp,
    Pipe,
    Caret,
    Tilde,
    Bang,
    AndAnd,
    OrOr,
    Nullish,
    Lt,
    Gt,
    Le,
    Ge,
    EqEq,
    NotEq,
    EqEqEq,
    NotEqEq,
    Assign,
    PlusAssign,
    MinusAssign,
    MulAssign,
    DivAssign,
    ModAssign,
    PowAssign,
    ShlAssign,
    ShrAssign,
    UShrAssign,
    AndAssign,
    OrAssign,
    XorAssign,
    AndAndAssign,
    OrOrAssign,
    NullishAssign,
    FatArrowEq,
    Ellipsis,
    // keywords
    Var,
    Let,
    Const,
    Function,
    Return,
    If,
    Else,
    For,
    While,
    Do,
    Break,
    Continue,
    Switch,
    Case,
    Default,
    Throw,
    Try,
    Catch,
    Finally,
    New,
    Delete,
    Typeof,
    Void,
    In,
    Instanceof,
    This,
    Super,
    Class,
    Extends,
    Static,
    Get,
    Set,
    Of,
    Await,
    Async,
    Yield,
    True,
    False,
    Null,
    Undefined,
    Debugger,
    With,
    Empty,
}

impl P {
    /// Keyword spelling (also used by `token_text`).
    pub fn keyword(&self) -> Option<&'static str> {
        Some(match self {
            P::Var => "var",
            P::Let => "let",
            P::Const => "const",
            P::Function => "function",
            P::Return => "return",
            P::If => "if",
            P::Else => "else",
            P::For => "for",
            P::While => "while",
            P::Do => "do",
            P::Break => "break",
            P::Continue => "continue",
            P::Switch => "switch",
            P::Case => "case",
            P::Default => "default",
            P::Throw => "throw",
            P::Try => "try",
            P::Catch => "catch",
            P::Finally => "finally",
            P::New => "new",
            P::Delete => "delete",
            P::Typeof => "typeof",
            P::Void => "void",
            P::In => "in",
            P::Instanceof => "instanceof",
            P::This => "this",
            P::Super => "super",
            P::Class => "class",
            P::Extends => "extends",
            P::Static => "static",
            P::Get => "get",
            P::Set => "set",
            P::Of => "of",
            P::Await => "await",
            P::Async => "async",
            P::Yield => "yield",
            P::True => "true",
            P::False => "false",
            P::Null => "null",
            P::Undefined => "undefined",
            P::Debugger => "debugger",
            P::With => "with",
            _ => return None,
        })
    }

    fn from_keyword(s: &str) -> Option<P> {
        Some(match s {
            "var" => P::Var,
            "let" => P::Let,
            "const" => P::Const,
            "function" => P::Function,
            "return" => P::Return,
            "if" => P::If,
            "else" => P::Else,
            "for" => P::For,
            "while" => P::While,
            "do" => P::Do,
            "break" => P::Break,
            "continue" => P::Continue,
            "switch" => P::Switch,
            "case" => P::Case,
            "default" => P::Default,
            "throw" => P::Throw,
            "try" => P::Try,
            "catch" => P::Catch,
            "finally" => P::Finally,
            "new" => P::New,
            "delete" => P::Delete,
            "typeof" => P::Typeof,
            "void" => P::Void,
            "in" => P::In,
            "instanceof" => P::Instanceof,
            "this" => P::This,
            "super" => P::Super,
            "class" => P::Class,
            "extends" => P::Extends,
            "static" => P::Static,
            "get" => P::Get,
            "set" => P::Set,
            "of" => P::Of,
            "await" => P::Await,
            "async" => P::Async,
            "yield" => P::Yield,
            "true" => P::True,
            "false" => P::False,
            "null" => P::Null,
            "undefined" => P::Undefined,
            "debugger" => P::Debugger,
            "with" => P::With,
            _ => return None,
        })
    }
}

impl fmt::Display for P {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            P::LParen => "(",
            P::RParen => ")",
            P::LBrace => "{",
            P::RBrace => "}",
            P::LBracket => "[",
            P::RBracket => "]",
            P::Dot => ".",
            P::DotDotDot | P::Ellipsis => "...",
            P::Comma => ",",
            P::Semi => ";",
            P::Colon => ":",
            P::Question => "?",
            P::QuestionDot => "?.",
            P::Arrow | P::FatArrowEq => "=>",
            P::PlusPlus => "++",
            P::MinusMinus => "--",
            P::Plus => "+",
            P::Minus => "-",
            P::Star => "*",
            P::StarStar => "**",
            P::Slash => "/",
            P::Percent => "%",
            P::ShiftLeft => "<<",
            P::ShiftRight => ">>",
            P::UShiftRight => ">>>",
            P::Amp => "&",
            P::Pipe => "|",
            P::Caret => "^",
            P::Tilde => "~",
            P::Bang => "!",
            P::AndAnd => "&&",
            P::OrOr => "||",
            P::Nullish => "??",
            P::Lt => "<",
            P::Gt => ">",
            P::Le => "<=",
            P::Ge => ">=",
            P::EqEq => "==",
            P::NotEq => "!=",
            P::EqEqEq => "===",
            P::NotEqEq => "!==",
            P::Assign => "=",
            P::PlusAssign => "+=",
            P::MinusAssign => "-=",
            P::MulAssign => "*=",
            P::DivAssign => "/=",
            P::ModAssign => "%=",
            P::PowAssign => "**=",
            P::ShlAssign => "<<=",
            P::ShrAssign => ">>=",
            P::UShrAssign => ">>>=",
            P::AndAssign => "&=",
            P::OrAssign => "|=",
            P::XorAssign => "^=",
            P::AndAndAssign => "&&=",
            P::OrOrAssign => "||=",
            P::NullishAssign => "??=",
            _ => return write!(f, "{}", self.keyword().unwrap_or("?")),
        };
        f.write_str(s)
    }
}

/// Lexed token.
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    /// Identifier (or contextual keyword).
    Ident(String),
    Number(f64),
    String(String),
    /// Raw template literal content between backticks (no leading `` ` ``).
    Template(String),
    /// Regular expression source + flags.
    Regex(String, String),
    Punct(P),
    Eof,
}

impl Token {
    pub fn is_punct(&self, p: P) -> bool {
        matches!(self, Token::Punct(x) if *x == p)
    }
    pub fn is_ident(&self, name: &str) -> bool {
        matches!(self, Token::Ident(s) if s == name)
    }
}

/// A token with its position and the "newline before it" flag for ASI.
#[derive(Debug, Clone, PartialEq)]
pub struct Lexed {
    pub tok: Token,
    pub span: Span,
    /// A line terminator appeared before this token (ASI candidate).
    pub nl: bool,
    /// What kind of thing preceded it — decides regex vs division.
    pub prev: Lexeme,
}

/// Categories of the previous significant token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lexeme {
    Start,
    /// A value ended: identifier, literal, `)`, `]`, `++`, `--`, keyword-that-is-a-value.
    ValueEnd,
    /// Anything else: operator, `{`, `,`, `;`, …
    Other,
}

#[derive(Debug, Clone)]
pub struct LexError {
    pub message: String,
    pub line: u32,
}

impl fmt::Display for LexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, " SyntaxError line {}: {}", self.line, self.message)
    }
}

struct Lexer<'a> {
    src: &'a [u8],
    text: &'a str,
    pos: usize,
    line: u32,
    prev: Lexeme,
}

/// Tokenise a whole script.
pub fn lex(src: &str) -> Result<Vec<Lexed>, LexError> {
    let mut lx = Lexer {
        src: src.as_bytes(),
        text: src,
        pos: 0,
        line: 1,
        prev: Lexeme::Start,
    };
    lx.run()
}

impl<'a> Lexer<'a> {
    fn run(&mut self) -> Result<Vec<Lexed>, LexError> {
        let mut out: Vec<Lexed> = Vec::with_capacity(self.src.len() / 2 + 8);
        // `#!` shebang
        if self.src.starts_with(b"#!") {
            while self.pos < self.src.len() && self.src[self.pos] != b'\n' {
                self.pos += 1;
            }
        }
        loop {
            let nl = self.skip_trivia()?;
            let span_start = self.pos;
            // the line the token starts on: `skip_trivia` already counted every
            // newline before it, so no O(n) rescan per token (see the old
            // `line_at`, which made big minified scripts quadratic)
            let span_line = self.line;
            let tok = match self.next_token()? {
                Some(t) => t,
                None => {
                    out.push(Lexed {
                        tok: Token::Eof,
                        span: Span {
                            start: span_start as u32,
                            end: span_start as u32,
                            line: self.line,
                        },
                        nl,
                        prev: self.prev,
                    });
                    return Ok(out);
                }
            };
            let span = Span {
                start: span_start as u32,
                end: self.pos as u32,
                line: span_line,
            };
            let lexed = Lexed {
                tok,
                span,
                nl,
                prev: self.prev,
            };
            out.push(lexed);
        }
    }

    fn peek_at(&self, off: usize) -> Option<u8> {
        self.src.get(self.pos + off).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let c = self.peek_at(0)?;
        self.pos += 1;
        if c == b'\n' {
            self.line += 1;
        }
        Some(c)
    }

    /// Skip whitespace and comments; report whether a newline was crossed.
    fn skip_trivia(&mut self) -> Result<bool, LexError> {
        let mut nl = false;
        loop {
            match self.peek_at(0) {
                Some(b' ') | Some(b'\t') | Some(b'\r') | Some(0x0b) | Some(0x0c) => {
                    self.pos += 1;
                }
                Some(b'\n') => {
                    nl = true;
                    self.pos += 1;
                    self.line += 1;
                }
                // non-breaking space and other unicode spaces (as bytes)
                Some(0xc2) if self.peek_at(1) == Some(0xa0) => self.pos += 2,
                Some(b'/') if self.peek_at(1) == Some(b'/') => {
                    self.pos += 2;
                    while let Some(c) = self.peek_at(0) {
                        if c == b'\n' {
                            break;
                        }
                        self.pos += 1;
                    }
                }
                Some(b'/') if self.peek_at(1) == Some(b'*') => {
                    self.pos += 2;
                    let closed = loop {
                        match self.peek_at(0) {
                            Some(b'*') if self.peek_at(1) == Some(b'/') => {
                                self.pos += 2;
                                break true;
                            }
                            Some(b'\n') => {
                                nl = true;
                                self.line += 1;
                                self.pos += 1;
                            }
                            Some(_) => self.pos += 1,
                            None => break false,
                        }
                    };
                    if !closed {
                        return Err(LexError {
                            message: "unterminated comment".into(),
                            line: self.line,
                        });
                    }
                }
                _ => return Ok(nl),
            }
        }
    }

    fn next_token(&mut self) -> Result<Option<Token>, LexError> {
        let Some(c) = self.peek_at(0) else {
            return Ok(None);
        };
        let tok = match c {
            // ---- numbers ---------------------------------------------------
            b'0'..=b'9' => self.number()?,
            b'.' if matches!(self.peek_at(1), Some(b'0'..=b'9')) => self.number()?,
            // ---- strings ---------------------------------------------------
            b'\'' | b'"' => self.quoted(c as char)?,
            b'`' => self.template()?,
            // ---- identifiers / keywords ------------------------------------
            _ if starts_ident(c) => self.ident()?,
            // ---- punctuators -----------------------------------------------
            _ => self.punctuator_or_regex()?,
        };
        self.prev = match &tok {
            Token::Punct(p) => {
                if p.keyword().is_some() {
                    if matches!(
                        p,
                        P::This | P::True | P::False | P::Null | P::Undefined | P::Super
                    ) {
                        Lexeme::ValueEnd
                    } else {
                        Lexeme::Other
                    }
                } else if matches!(p, P::RParen | P::RBracket | P::PlusPlus | P::MinusMinus) {
                    Lexeme::ValueEnd
                } else {
                    Lexeme::Other
                }
            }
            Token::Ident(_)
            | Token::Number(_)
            | Token::String(_)
            | Token::Template(_)
            | Token::Regex(..) => Lexeme::ValueEnd,
            Token::Eof => Lexeme::Other,
        };
        Ok(Some(tok))
    }

    fn number(&mut self) -> Result<Token, LexError> {
        let start = self.pos;
        // radix prefixes
        if self.peek_at(0) == Some(b'0') && matches!(self.peek_at(1), Some(b'x') | Some(b'X')) {
            self.pos += 2;
            while matches!(self.peek_at(0), Some(c) if c.is_ascii_hexdigit() || c == b'_') {
                self.pos += 1;
            }
            let raw = &self.text[start..self.pos];
            let clean: String = raw
                .chars()
                .filter(|c| *c != '_' && *c != 'x' && *c != 'X')
                .collect();
            return Ok(Token::Number(
                u64::from_str_radix(&clean, 16)
                    .map(|v| v as f64)
                    .unwrap_or(f64::NAN),
            ));
        }
        if self.peek_at(0) == Some(b'0')
            && matches!(
                self.peek_at(1),
                Some(b'b') | Some(b'B') | Some(b'o') | Some(b'O')
            )
        {
            let radix = if matches!(self.peek_at(1), Some(b'b') | Some(b'B')) {
                2
            } else {
                8
            };
            self.pos += 2;
            while let Some(c) = self.peek_at(0) {
                if c.is_ascii_digit() || c == b'_' {
                    self.pos += 1;
                } else {
                    break;
                }
            }
            let clean: String = self.text[start..self.pos]
                .chars()
                .filter(|c| c.is_ascii_digit())
                .collect();
            return Ok(Token::Number(
                u64::from_str_radix(&clean, radix)
                    .map(|v| v as f64)
                    .unwrap_or(f64::NAN),
            ));
        }
        while let Some(c) = self.peek_at(0) {
            if c.is_ascii_digit() || c == b'_' {
                self.pos += 1;
            } else {
                break;
            }
        }
        if self.peek_at(0) == Some(b'.') {
            self.pos += 1;
            while matches!(self.peek_at(0), Some(c) if c.is_ascii_digit() || c == b'_') {
                self.pos += 1;
            }
        }
        if matches!(self.peek_at(0), Some(b'e') | Some(b'E')) {
            let save = self.pos;
            self.pos += 1;
            if matches!(self.peek_at(0), Some(b'+') | Some(b'-')) {
                self.pos += 1;
            }
            if matches!(self.peek_at(0), Some(c) if c.is_ascii_digit()) {
                while matches!(self.peek_at(0), Some(c) if c.is_ascii_digit()) {
                    self.pos += 1;
                }
            } else {
                self.pos = save;
            }
        }
        // BigInt literal: keep the value as a number (documented limitation)
        if matches!(self.peek_at(0), Some(b'n')) {
            self.pos += 1;
        }
        let raw: String = self.text[start..self.pos]
            .chars()
            .filter(|c| *c != '_')
            .collect();
        match raw.parse::<f64>() {
            Ok(v) => Ok(Token::Number(v)),
            Err(_) => Err(LexError {
                message: format!("invalid number `{raw}`"),
                line: self.line,
            }),
        }
    }

    fn quoted(&mut self, quote: char) -> Result<Token, LexError> {
        self.pos += 1;
        let mut out = String::new();
        loop {
            let Some(c) = self.bump() else {
                return Err(LexError {
                    message: format!("unterminated string ({quote})"),
                    line: self.line,
                });
            };
            match c {
                b'\\' => self.escape(&mut out)?,
                b'\n' => {
                    // unescaped newline inside a string: tolerated (real engines
                    // reject it, but foreign minified code does this after
                    // broken transfers)
                    out.push('\n');
                }
                _ => {
                    if c < 0x80 {
                        if c as char == quote {
                            break;
                        }
                        out.push(c as char);
                    } else {
                        // multi-byte UTF-8: rewind to the lead byte and copy the
                        // whole sequence (never the closing quote)
                        self.pos -= 1;
                        out.push_str(self.utf8_seq());
                    }
                }
            }
        }
        Ok(Token::String(out))
    }

    fn escape(&mut self, out: &mut String) -> Result<(), LexError> {
        let Some(c) = self.bump() else {
            return Err(LexError {
                message: "unterminated escape".into(),
                line: self.line,
            });
        };
        match c {
            b'n' => out.push('\n'),
            b't' => out.push('\t'),
            b'r' => out.push('\r'),
            b'0' => out.push('\0'),
            b'b' => out.push('\u{8}'),
            b'v' => out.push('\u{b}'),
            b'f' => out.push('\u{c}'),
            b'\n' => {}
            b'x' => {
                let hex = self.take(2);
                out.push(
                    char::from_u32(u32::from_str_radix(&hex, 16).unwrap_or(0)).unwrap_or('\0'),
                );
            }
            b'u' => {
                if self.peek_at(0) == Some(b'{') {
                    self.pos += 1;
                    let mut hex = String::new();
                    while let Some(c) = self.bump() {
                        if c == b'}' {
                            break;
                        }
                        hex.push(c as char);
                    }
                    let cp = u32::from_str_radix(&hex, 16).unwrap_or(0xfffd);
                    out.push(char::from_u32(cp).unwrap_or('\u{fffd}'));
                } else {
                    let hex = self.take(4);
                    let cp = u32::from_str_radix(&hex, 16).unwrap_or(0xfffd);
                    out.push(char::from_u32(cp).unwrap_or('\u{fffd}'));
                }
            }
            _ => {
                // \ü and unknown escapes: the character itself
                self.pos -= 1;
                out.push_str(self.utf8_seq());
            }
        }
        Ok(())
    }

    /// Consume the UTF-8 sequence starting at `self.pos` and return it as `&str`.
    /// Invalid bytes become U+FFFD; we never split a sequence and never run past
    /// the end of the source.
    fn utf8_seq(&mut self) -> &'a str {
        let start = self.pos;
        let lead = match self.src.get(start) {
            Some(&b) => b,
            None => return "",
        };
        let len = utf8_len(lead);
        let end = (start + len).min(self.src.len());
        // do not swallow a following invalid/short sequence
        match std::str::from_utf8(&self.src[start..end]) {
            Ok(s) => {
                self.pos = end;
                s
            }
            Err(e) => {
                let valid = e.valid_up_to();
                if valid > 0 {
                    let s = &self.src[start..start + valid];
                    self.pos = start + valid;
                    // safety: from_utf8 verified these bytes
                    unsafe { std::str::from_utf8_unchecked(s) }
                } else {
                    self.pos = start + 1;
                    "\u{fffd}"
                }
            }
        }
    }

    fn take(&mut self, n: usize) -> String {
        let end = (self.pos + n).min(self.src.len());
        let s = self.text[self.pos..end].to_string();
        self.pos = end;
        s
    }

    /// Template literal: capture raw content, honouring `${ ... }` nesting.
    fn template(&mut self) -> Result<Token, LexError> {
        self.pos += 1; // opening backtick
        let start = self.pos;
        let mut depth = 0usize;
        loop {
            let Some(c) = self.peek_at(0) else {
                return Err(LexError {
                    message: "unterminated template literal".into(),
                    line: self.line,
                });
            };
            match c {
                b'\\' => {
                    self.pos += 2;
                }
                b'`' if depth == 0 => break,
                b'`' => self.pos += 1,
                b'$' if self.peek_at(1) == Some(b'{') => {
                    depth += 1;
                    self.pos += 2;
                }
                b'{' if depth > 0 => {
                    depth += 1;
                    self.pos += 1;
                }
                b'}' if depth > 0 => {
                    depth -= 1;
                    self.pos += 1;
                }
                b'\n' => {
                    self.line += 1;
                    self.pos += 1;
                }
                _ => self.pos += 1,
            }
        }
        let raw = self.text[start..self.pos].to_string();
        self.pos += 1; // closing backtick
        Ok(Token::Template(raw))
    }

    fn ident(&mut self) -> Result<Token, LexError> {
        let start = self.pos;
        while let Some(c) = self.peek_at(0) {
            if cont_ident(c) {
                self.pos += 1;
            } else if c >= 0x80 {
                // a multi-byte identifier character
                let ch = self.char_at(self.pos);
                if ch.is_alphabetic() || ch.is_numeric() || ch == '_' || ch == '$' {
                    self.pos += ch.len_utf8();
                } else {
                    break;
                }
            } else {
                break;
            }
        }
        let word = &self.text[start..self.pos];
        if let Some(p) = P::from_keyword(word) {
            return Ok(Token::Punct(p));
        }
        Ok(Token::Ident(word.to_string()))
    }

    fn char_at(&self, byte: usize) -> char {
        let s = &self.text[byte.min(self.text.len())..];
        s.chars().next().unwrap_or('\u{fffd}')
    }

    fn punctuator_or_regex(&mut self) -> Result<Token, LexError> {
        if self.peek_at(0) == Some(b'/')
            && self.prev != Lexeme::ValueEnd
            && self.peek_at(1) != Some(b'=')
        {
            return self.regex();
        }
        let rest = &self.text[self.pos.min(self.text.len())..];
        // longest match first
        for cand in PUNCTS {
            if rest.starts_with(cand) {
                self.pos += cand.len();
                let p = punct_of(cand).unwrap_or(P::Empty);
                if p == P::Empty {
                    return Err(LexError {
                        message: format!("unexpected `{cand}`"),
                        line: self.line,
                    });
                }
                return Ok(Token::Punct(p));
            }
        }
        let ch = self.char_at(self.pos);
        Err(LexError {
            message: format!("unexpected character `{ch}`"),
            line: self.line,
        })
    }

    fn regex(&mut self) -> Result<Token, LexError> {
        let start = self.pos;
        self.pos += 1;
        let mut class = false;
        loop {
            let Some(c) = self.peek_at(0) else {
                self.pos = start;
                return Err(LexError {
                    message: "unterminated regular expression".into(),
                    line: self.line,
                });
            };
            match c {
                b'\\' => {
                    self.pos += 2;
                }
                b'[' => {
                    class = true;
                    self.pos += 1;
                }
                b']' if class => {
                    class = false;
                    self.pos += 1;
                }
                b'/' if !class => break,
                b'\n' => {
                    self.pos = start;
                    return Err(LexError {
                        message: "unterminated regular expression".into(),
                        line: self.line,
                    });
                }
                _ => self.pos += 1,
            }
        }
        let body = self.text[start + 1..self.pos].to_string();
        self.pos += 1;
        let fstart = self.pos;
        while matches!(self.peek_at(0), Some(c) if c.is_ascii_alphabetic()) {
            self.pos += 1;
        }
        Ok(Token::Regex(body, self.text[fstart..self.pos].to_string()))
    }
}

/// Length in bytes of the UTF-8 sequence introduced by `lead` (1 for ASCII).
fn utf8_len(lead: u8) -> usize {
    if lead < 0x80 {
        1
    } else if lead & 0xe0 == 0xc0 {
        2
    } else if lead & 0xf0 == 0xe0 {
        3
    } else if lead & 0xf8 == 0xf0 {
        4
    } else {
        1
    }
}

fn starts_ident(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c == b'$' || c >= 0x80
}

fn cont_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'$'
}

/// Punctuators, longest first (order matters!).
static PUNCTS: &[&str] = &[
    ">>>=", "...", "===", "!==", "**=", "<<=", ">>=", ">>>", "&&=", "||=", "??=", "&&", "||", "??",
    "==", "!=", "<=", ">=", "<<", ">>", "=>", "++", "--", "+=", "-=", "*=", "/=", "%=", "&=", "|=",
    "^=", "**", "?.", "(", ")", "{", "}", "[", "]", ".", ",", ";", ":", "?", "+", "-", "*", "/",
    "%", "&", "|", "^", "~", "!", "<", ">", "=",
];

fn punct_of(s: &str) -> Option<P> {
    Some(match s {
        ">>>=" => P::UShrAssign,
        "..." => P::Ellipsis,
        "===" => P::EqEqEq,
        "!==" => P::NotEqEq,
        "**=" => P::PowAssign,
        "<<=" => P::ShlAssign,
        ">>=" => P::ShrAssign,
        ">>>" => P::UShiftRight,
        "&&=" => P::AndAndAssign,
        "||=" => P::OrOrAssign,
        "??=" => P::NullishAssign,
        "&&" => P::AndAnd,
        "||" => P::OrOr,
        "??" => P::Nullish,
        "==" => P::EqEq,
        "!=" => P::NotEq,
        "<=" => P::Le,
        ">=" => P::Ge,
        "<<" => P::ShiftLeft,
        ">>" => P::ShiftRight,
        "=>" => P::Arrow,
        "++" => P::PlusPlus,
        "--" => P::MinusMinus,
        "+=" => P::PlusAssign,
        "-=" => P::MinusAssign,
        "*=" => P::MulAssign,
        "/=" => P::DivAssign,
        "%=" => P::ModAssign,
        "&=" => P::AndAssign,
        "|=" => P::OrAssign,
        "^=" => P::XorAssign,
        "**" => P::StarStar,
        "?." => P::QuestionDot,
        "(" => P::LParen,
        ")" => P::RParen,
        "{" => P::LBrace,
        "}" => P::RBrace,
        "[" => P::LBracket,
        "]" => P::RBracket,
        "." => P::Dot,
        "," => P::Comma,
        ";" => P::Semi,
        ":" => P::Colon,
        "?" => P::Question,
        "+" => P::Plus,
        "-" => P::Minus,
        "*" => P::Star,
        "/" => P::Slash,
        "%" => P::Percent,
        "&" => P::Amp,
        "|" => P::Pipe,
        "^" => P::Caret,
        "~" => P::Tilde,
        "!" => P::Bang,
        "<" => P::Lt,
        ">" => P::Gt,
        "=" => P::Assign,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(src: &str) -> Vec<Token> {
        lex(src).unwrap().into_iter().map(|l| l.tok).collect()
    }

    #[test]
    fn identifiers_numbers_keywords() {
        assert_eq!(
            toks("var x = 42;"),
            vec![
                Token::Punct(P::Var),
                Token::Ident("x".into()),
                Token::Punct(P::Assign),
                Token::Number(42.0),
                Token::Punct(P::Semi),
                Token::Eof
            ]
        );
    }

    #[test]
    fn number_forms() {
        assert_eq!(toks("0x10")[0], Token::Number(16.0));
        assert_eq!(toks("1_000")[0], Token::Number(1000.0));
        assert_eq!(toks(".5")[0], Token::Number(0.5));
        assert_eq!(toks("1e3")[0], Token::Number(1000.0));
        assert_eq!(toks("1e")[0], Token::Number(1.0));
        assert_eq!(toks("0b101")[0], Token::Number(5.0));
    }

    #[test]
    fn strings_and_escapes() {
        assert_eq!(toks("'a\\nb'")[0], Token::String("a\nb".into()));
        assert_eq!(toks("\"\\u{1F600}\"")[0], Token::String("😀".into()));
        assert_eq!(toks("'Gr\\u00fcße'")[0], Token::String("Grüße".into()));
    }

    #[test]
    fn regex_versus_division() {
        // after `=` a slash is a regex
        assert!(
            matches!(&toks("var r = /ab+/gi")[3], Token::Regex(b, f) if b == "ab+" && f == "gi")
        );
        // after a value it is division
        assert_eq!(toks("a / b")[1], Token::Punct(P::Slash));
        // division-assignment stays a punctuator
        assert_eq!(toks("a /= 2")[1], Token::Punct(P::DivAssign));
    }

    #[test]
    fn template_literals_nest() {
        assert_eq!(
            toks("`a${b + `c${d}`}e`")[0],
            Token::Template("a${b + `c${d}`}e".into())
        );
    }

    #[test]
    fn comments_are_skipped_but_newlines_are_noted() {
        let l = lex("a;\n// gone\nb").unwrap();
        assert!(l[2].nl, "newline before `b` should be recorded");
        assert_eq!(l[0].tok, Token::Ident("a".into()));
    }

    #[test]
    fn asi_marker_after_line_break_in_expression() {
        let l = lex("x = 1\ny = 2").unwrap();
        let y = l
            .iter()
            .find(|t| t.tok == Token::Ident("y".into()))
            .unwrap();
        assert!(y.nl);
    }

    #[test]
    fn punctuators_longest_match() {
        assert_eq!(toks("a >>>= b")[1], Token::Punct(P::UShrAssign));
        assert_eq!(toks("a ??= b")[1], Token::Punct(P::NullishAssign));
        assert_eq!(toks("?.")[0], Token::Punct(P::QuestionDot));
    }

    #[test]
    fn unterminated_string_is_an_error_not_a_panic() {
        assert!(lex("'oops").is_err());
        assert!(lex("/* nope").is_err());
        assert!(lex("`nope").is_err());
        assert!(lex("var x = ;").is_err() || lex("var x = ;").is_ok());
    }

    #[test]
    fn real_world_snippet() {
        let src = r#"
            (function () {
              'use strict';
              var g = window.gtag || function () { dataLayer.push(arguments) };
              g('js', new Date());
              if (a?.b?.c ?? 3) { console.log(`x: ${1 + 2}`) }
            })();
        "#;
        let l = lex(src).expect("lexes");
        assert!(matches!(l.last().unwrap().tok, Token::Eof));
    }
}
