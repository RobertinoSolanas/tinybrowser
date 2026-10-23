//! A small backtracking regular-expression engine.
//!
//! Enough of the syntax real pages use: literals, `.`, character classes with
//! ranges, the usual escapes (`\d \w \s \b \n \uXXXX`), greedy and lazy
//! quantifiers (`* + ? {n,m}`), capturing groups, `(?:…)`, look-ahead
//! `(?= …) (?! …)`, alternation, `^ $` with the `m` flag, and the `i` flag.
//!
//! Positions are **char** indices, not bytes; `RegExp` results are converted to
//! the byte offsets our string helpers need. Surrogate pairs are counted as one
//! character (a documented difference from real JS, which counts UTF-16 units).

use super::value::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Flags {
    pub global: bool,
    pub ignore_case: bool,
    pub multiline: bool,
    pub dot_all: bool,
    pub sticky: bool,
    pub unicode: bool,
}

impl Flags {
    pub fn parse(s: &str) -> Flags {
        let mut f = Flags {
            global: false,
            ignore_case: false,
            multiline: false,
            dot_all: false,
            sticky: false,
            unicode: false,
        };
        for c in s.chars() {
            match c {
                'g' => f.global = true,
                'i' => f.ignore_case = true,
                'm' => f.multiline = true,
                's' => f.dot_all = true,
                'y' => f.sticky = true,
                'u' => f.unicode = true,
                _ => {}
            }
        }
        f
    }
}

/// Compiled pattern node.
#[derive(Debug, Clone)]
enum Node {
    Empty,
    Char(char),
    Any,
    Class {
        negated: bool,
        items: Vec<Item>,
    },
    /// start/end of the whole subject, or of a line with `m`
    Start,
    End,
    WordBoundary(bool),
    /// capture group index (1-based), inner
    Group(usize, Box<Node>),
    /// non-capturing group / look-around
    /// `(?=…)` / `(?!…)` — forward only, lookbehind is rejected when parsing
    Look {
        neg: bool,
        inner: Box<Node>,
    },
    Concat(Vec<Node>),
    Alt(Vec<Node>),
    /// greedy / lazy repetition
    Repeat {
        min: u32,
        max: Option<u32>,
        lazy: bool,
        inner: Box<Node>,
    },
    /// back reference \1 … \9
    BackRef(usize),
}

#[derive(Debug, Clone)]
enum Item {
    Ch(char),
    Range(char, char),
    Digit,
    NotDigit,
    Word,
    NotWord,
    Space,
    NotSpace,
}

impl Item {
    fn matches(&self, c: char, fold: bool) -> bool {
        let (a, b) = if fold {
            (
                c.to_lowercase().next().unwrap_or(c),
                c.to_uppercase().next().unwrap_or(c),
            )
        } else {
            (c, c)
        };
        let eq = |x: char| a == x || b == x;
        match self {
            Item::Ch(x) => eq(*x),
            Item::Range(lo, hi) => (a >= *lo && a <= *hi) || (b >= *lo && b <= *hi),
            Item::Digit => c.is_ascii_digit(),
            Item::NotDigit => !c.is_ascii_digit(),
            Item::Word => c.is_alphanumeric() || c == '_',
            Item::NotWord => !(c.is_alphanumeric() || c == '_'),
            Item::Space => c.is_whitespace(),
            Item::NotSpace => !c.is_whitespace(),
        }
    }
}

/// A compiled regular expression.
#[derive(Debug, Clone)]
pub struct RegExp {
    pub source: String,
    pub flags: String,
    root: Node,
    n_groups: usize,
    fl: Flags,
}

/// One match: text plus char offsets and captures.
#[derive(Debug, Clone)]
pub struct MatchInfo {
    pub text: String,
    pub start: usize,
    pub end: usize,
    pub groups: Vec<Option<String>>,
}

#[derive(Debug)]
pub struct CompileError(pub String);

#[derive(Clone)]
struct Compiled {
    flags: String,
    code: Vec<Ins>,
}

thread_local! {
    /// Compiled programs by pattern source (regex literals are evaluated per
    /// evaluation, so caching keeps hot loops cheap).
    static PROGRAM: std::cell::RefCell<std::collections::HashMap<String, Compiled>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

type Caps = Vec<(usize, usize)>;

impl CompileError {
    pub fn to_value_msg(&self) -> String {
        format!("Invalid regular expression: /{}/: {}", self.0, self.0)
    }
}

struct PatternParser<'a> {
    chars: Vec<char>,
    pos: usize,
    groups: usize,
    src: &'a str,
}

impl RegExp {
    pub fn compile(source: &str, flags: &str) -> Result<RegExp, CompileError> {
        let fl = Flags::parse(flags);
        let mut pp = PatternParser {
            chars: source.chars().collect(),
            pos: 0,
            groups: 0,
            src: source,
        };
        let root = pp.parse_alt()?;
        if pp.pos < pp.chars.len() {
            return Err(CompileError(format!("unmatched `)` in /{}/", source)));
        }
        Ok(RegExp {
            source: source.to_string(),
            flags: flags.to_string(),
            root,
            n_groups: pp.groups,
            fl,
        })
    }

    /// First match at or after char index `from`.
    pub fn find(&self, text: &str, from: usize) -> Option<MatchInfo> {
        let code = self.program();
        let chars: Vec<char> = text.chars().collect();
        let fl = &self.fl;
        let span = |a: usize, b: usize| -> String {
            let a = a.min(chars.len());
            let b = b.min(chars.len()).max(a);
            chars.iter().skip(a).take(b - a).collect()
        };
        let start = from.min(chars.len());
        let mut caps: Caps = vec![(usize::MAX, usize::MAX); self.n_groups + 1];
        for i in start..=chars.len() {
            for c in caps.iter_mut() {
                *c = (usize::MAX, usize::MAX);
            }
            caps[0] = (i, usize::MAX);
            let hit = {
                let mut r = Runner {
                    chars: &chars,
                    fl,
                    caps: &mut caps,
                    steps: 0,
                    seen: std::collections::HashSet::new(),
                };
                // slot 0 wraps the whole program: Save(0) … Match
                r.run(&code, 0, i)
            };
            if hit {
                let (raw_s, raw_e) = caps[0];
                let s = if raw_s == usize::MAX { i } else { raw_s };
                let e = if raw_e == usize::MAX { s } else { raw_e };
                let groups: Vec<Option<String>> = caps
                    .iter()
                    .map(|pair| {
                        let (a, b) = (pair.0, pair.1);
                        if a == usize::MAX || b == usize::MAX || b < a {
                            None
                        } else {
                            Some(span(a, b))
                        }
                    })
                    .collect();
                return Some(MatchInfo {
                    text: span(s, e),
                    start: s,
                    end: e,
                    groups,
                });
            }
            if i == chars.len() {
                break;
            }
        }
        None
    }

    /// The compiled program: `Save(0) <body> Save(0) Match`.
    fn program(&self) -> Vec<Ins> {
        let cached = PROGRAM.with(|p| {
            p.borrow()
                .get(&self.source)
                .filter(|c| c.flags == self.flags)
                .map(|c| c.code.clone())
        });
        if let Some(code) = cached {
            return code;
        }
        let mut code = vec![Ins::SaveStart(0)];
        compile(&self.root, &mut code);
        code.push(Ins::SaveEnd(0));
        code.push(Ins::Match);
        PROGRAM.with(|p| {
            p.borrow_mut().insert(
                self.source.clone(),
                Compiled {
                    flags: self.flags.clone(),
                    code: code.clone(),
                },
            )
        });
        code
    }

    /// First match anywhere (`find(text, 0)`).
    pub fn first(&self, text: &str) -> Option<MatchInfo> {
        self.find(text, 0)
    }

    pub fn test(&self, text: &str) -> bool {
        self.find(text, 0).is_some()
    }

    pub fn group_count(&self) -> usize {
        self.n_groups
    }

    /// All matches (used by the `g` flag).
    pub fn find_all(&self, text: &str) -> Vec<MatchInfo> {
        let mut out = Vec::new();
        let max = text.chars().count();
        let mut pos = 0usize;
        while pos <= max {
            let Some(m) = self.find(text, pos) else { break };
            // a zero-length match must still advance, or we would spin
            pos = if m.end == m.start { m.end + 1 } else { m.end };
            out.push(m);
            if out.len() >= 100_000 {
                break;
            }
        }
        out
    }

    pub fn flags_struct(&self) -> Flags {
        self.fl
    }
}

impl PatternParser<'_> {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn parse_alt(&mut self) -> Result<Node, CompileError> {
        let mut branches = vec![self.parse_concat()?];
        while self.eat('|') {
            branches.push(self.parse_concat()?);
        }
        Ok(if branches.len() == 1 {
            branches.pop().unwrap()
        } else {
            Node::Alt(branches)
        })
    }

    fn parse_concat(&mut self) -> Result<Node, CompileError> {
        let mut items = Vec::new();
        loop {
            match self.peek() {
                None | Some('|') | Some(')') => break,
                _ => {}
            }
            let atom = self.parse_atom()?;
            items.push(self.parse_quantifier(atom)?);
        }
        Ok(match items.len() {
            0 => Node::Empty,
            1 => items.pop().unwrap(),
            _ => Node::Concat(items),
        })
    }

    fn parse_atom(&mut self) -> Result<Node, CompileError> {
        let c = self
            .bump()
            .ok_or_else(|| CompileError(format!("unexpected end of pattern /{}/", self.src)))?;
        match c {
            '(' => {
                if self.eat('?') {
                    if self.eat(':') {
                        let inner = self.parse_alt()?;
                        self.expect(')')?;
                        return Ok(inner);
                    }
                    if self.eat('<') {
                        // `(?<=…)` / `(?<!…)`: the backreference machine only
                        // looks forward, so these are a documented "cannot"
                        if matches!(self.peek(), Some('=') | Some('!')) {
                            self.bump();
                            return Err(CompileError("lookbehind is not supported".into()));
                        }
                        // named group `(?<name>…)` — captured like a plain one
                        while let Some(x) = self.bump() {
                            if x == '>' {
                                break;
                            }
                        }
                        self.groups += 1;
                        let inner = self.parse_alt()?;
                        self.expect(')')?;
                        return Ok(Node::Group(self.groups, Box::new(inner)));
                    }
                    // `(?=…)` is a positive, `(?!…)` a negative lookahead; every
                    // other `(?…` form (inline flags, atomic groups) we reject
                    let neg = self.eat('!');
                    if !neg && !self.eat('=') {
                        return Err(CompileError("unsupported group `(?…`".into()));
                    }
                    let inner = self.parse_alt()?;
                    self.expect(')')?;
                    return Ok(Node::Look {
                        neg,
                        inner: Box::new(inner),
                    });
                }
                self.groups += 1;
                let idx = self.groups;
                let inner = self.parse_alt()?;
                self.expect(')')?;
                Ok(Node::Group(idx, Box::new(inner)))
            }
            '[' => self.parse_class(),
            '.' => Ok(Node::Any),
            '^' => Ok(Node::Start),
            '$' => Ok(Node::End),
            '\\' => self.parse_escape(),
            ')' => Err(CompileError(format!("unmatched `)` in /{}/", self.src))),
            other => Ok(Node::Char(other)),
        }
    }

    fn expect(&mut self, c: char) -> Result<(), CompileError> {
        if self.eat(c) {
            Ok(())
        } else {
            Err(CompileError(format!("expected `{c}` in /{}/", self.src)))
        }
    }

    fn parse_escape(&mut self) -> Result<Node, CompileError> {
        let Some(c) = self.bump() else {
            return Err(CompileError(format!(
                "trailing backslash in /{}/",
                self.src
            )));
        };
        Ok(match c {
            'd' => Node::Class {
                negated: false,
                items: vec![Item::Digit],
            },
            'D' => Node::Class {
                negated: true,
                items: vec![Item::Digit],
            },
            'w' => Node::Class {
                negated: false,
                items: vec![Item::Word],
            },
            'W' => Node::Class {
                negated: true,
                items: vec![Item::Word],
            },
            's' => Node::Class {
                negated: false,
                items: vec![Item::Space],
            },
            'S' => Node::Class {
                negated: true,
                items: vec![Item::Space],
            },
            'b' => Node::WordBoundary(true),
            'B' => Node::WordBoundary(false),
            'n' => Node::Char('\n'),
            't' => Node::Char('\t'),
            'r' => Node::Char('\r'),
            'f' => Node::Char('\u{c}'),
            'v' => Node::Char('\u{b}'),
            '0' => Node::Char('\0'),
            c if c.is_ascii_digit() => {
                let mut n = (c as u8 - b'0') as usize;
                while matches!(self.peek(), Some(d) if d.is_ascii_digit()) {
                    let d = self.bump().unwrap();
                    let cand = n * 10 + (d as u8 - b'0') as usize;
                    if cand <= self.groups {
                        n = cand;
                    } else {
                        self.pos -= 1;
                        break;
                    }
                }
                Node::BackRef(n)
            }
            'u' => {
                if self.eat('{') {
                    let mut hex = String::new();
                    while let Some(x) = self.bump() {
                        if x == '}' {
                            break;
                        }
                        hex.push(x);
                    }
                    let cp = u32::from_str_radix(&hex, 16).unwrap_or(0xfffd);
                    Node::Char(char::from_u32(cp).unwrap_or('\u{fffd}'))
                } else {
                    let hex: String = (0..4).filter_map(|_| self.bump()).collect();
                    let cp = u32::from_str_radix(&hex, 16).unwrap_or(0xfffd);
                    Node::Char(char::from_u32(cp).unwrap_or('\u{fffd}'))
                }
            }
            'x' => {
                let hex: String = (0..2).filter_map(|_| self.bump()).collect();
                let cp = u32::from_str_radix(&hex, 16).unwrap_or(0);
                Node::Char(char::from_u32(cp).unwrap_or('\0'))
            }
            other => Node::Char(other),
        })
    }

    fn parse_class(&mut self) -> Result<Node, CompileError> {
        let negated = self.eat('^');
        let mut items = Vec::new();
        loop {
            match self.peek() {
                None => {
                    return Err(CompileError(format!(
                        "unterminated character class in /{}/",
                        self.src
                    )))
                }
                Some(']') if !items.is_empty() => {
                    self.pos += 1;
                    break;
                }
                Some(']') => {
                    self.pos += 1;
                    items.push(Item::Ch(']'));
                    continue;
                }
                _ => {}
            }
            let c = self.bump().unwrap();
            if c == '\\' {
                let e = self.bump().unwrap_or('\\');
                let item = match e {
                    'd' => Item::Digit,
                    'D' => Item::NotDigit,
                    'w' => Item::Word,
                    'W' => Item::NotWord,
                    's' => Item::Space,
                    'S' => Item::NotSpace,
                    'n' => Item::Ch('\n'),
                    't' => Item::Ch('\t'),
                    'r' => Item::Ch('\r'),
                    'u' => {
                        let hex: String = if self.eat('{') {
                            let mut h = String::new();
                            while let Some(x) = self.bump() {
                                if x == '}' {
                                    break;
                                }
                                h.push(x);
                            }
                            h
                        } else {
                            (0..4).filter_map(|_| self.bump()).collect()
                        };
                        let cp = u32::from_str_radix(&hex, 16).unwrap_or(0xfffd);
                        Item::Ch(char::from_u32(cp).unwrap_or('\u{fffd}'))
                    }
                    other => Item::Ch(other),
                };
                // range?  e.g. \x20-\x7e
                if let Item::Ch(a) = item {
                    if self.peek() == Some('-')
                        && self.chars.get(self.pos + 1) != Some(&']')
                        && self.chars.get(self.pos + 1).is_some()
                    {
                        self.pos += 1;
                        let b = self.bump_class_char();
                        items.push(Item::Range(a, b));
                        continue;
                    }
                }
                items.push(item);
                continue;
            }
            // literal, possibly a range a-z
            if self.peek() == Some('-')
                && self.chars.get(self.pos + 1).is_some()
                && self.chars.get(self.pos + 1) != Some(&']')
            {
                self.pos += 1;
                let b = self.bump_class_char();
                items.push(Item::Range(c, b));
                continue;
            }
            items.push(Item::Ch(c));
        }
        Ok(Node::Class { negated, items })
    }

    fn bump_class_char(&mut self) -> char {
        let c = self.bump().unwrap_or('-');
        if c != '\\' {
            return c;
        }
        match self.bump() {
            Some('n') => '\n',
            Some('t') => '\t',
            Some('r') => '\r',
            Some(other) => other,
            None => '\\',
        }
    }

    fn parse_quantifier(&mut self, atom: Node) -> Result<Node, CompileError> {
        let Some(c) = self.peek() else {
            return Ok(atom);
        };
        let (min, max) = match c {
            '*' => {
                self.pos += 1;
                (0u32, None)
            }
            '+' => {
                self.pos += 1;
                (1, None)
            }
            '?' => {
                self.pos += 1;
                (0, Some(1))
            }
            '{' => {
                // only a quantifier when it parses as one
                let save = self.pos;
                self.pos += 1;
                let lo = self.take_digits();
                if lo.is_empty() {
                    self.pos = save;
                    return Ok(atom);
                }
                let hi = if self.eat(',') {
                    let h = self.take_digits();
                    if h.is_empty() {
                        None
                    } else {
                        Some(h.parse::<u32>().unwrap_or(u32::MAX))
                    }
                } else {
                    Some(lo.parse::<u32>().unwrap())
                };
                if !self.eat('}') {
                    self.pos = save;
                    return Ok(atom);
                }
                (lo.parse::<u32>().unwrap(), hi)
            }
            _ => return Ok(atom),
        };
        let lazy = self.eat('?');
        // `++` is possessive in some engines; treat it as greedy
        while self.eat('+') {}
        Ok(Node::Repeat {
            min,
            max,
            lazy,
            inner: Box::new(atom),
        })
    }

    fn take_digits(&mut self) -> String {
        let mut s = String::new();
        while matches!(self.peek(), Some(d) if d.is_ascii_digit()) {
            s.push(self.bump().unwrap());
        }
        s
    }
}

// ---------------------------------------------------------------------------
// matching: the node tree compiles into a flat program, which the matcher runs
// with explicit backtracking. Instructions reference each other by index, so
// nothing borrows the tree and continuations are plain integers.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Ins {
    /// single character
    Char(char),
    Any,
    Class {
        negated: bool,
        items: Vec<Item>,
    },
    Start,
    End,
    WordB(bool),
    BackRef(usize),
    Jmp(usize),
    /// try `a` first, fall back to `b`
    Split(usize, usize),
    /// begin capture slot `n` at the current position
    SaveStart(usize),
    /// finish capture slot `n` at the current position
    SaveEnd(usize),
    /// look-around: run `sub` at the current position, then continue at `next`
    Look {
        neg: bool,
        sub: Vec<Ins>,
        next: usize,
    },
    Match,
}

fn compile(node: &Node, out: &mut Vec<Ins>) {
    match node {
        Node::Empty => {}
        Node::Char(c) => out.push(Ins::Char(*c)),
        Node::Any => out.push(Ins::Any),
        Node::Class { negated, items } => out.push(Ins::Class {
            negated: *negated,
            items: items.clone(),
        }),
        Node::Start => out.push(Ins::Start),
        Node::End => out.push(Ins::End),
        Node::WordBoundary(w) => out.push(Ins::WordB(*w)),
        Node::BackRef(i) => out.push(Ins::BackRef(*i)),
        Node::Group(i, inner) => {
            out.push(Ins::SaveStart(*i));
            compile(inner, out);
            out.push(Ins::SaveEnd(*i));
        }
        Node::Look { neg, inner } => {
            let mut sub = Vec::new();
            compile(inner, &mut sub);
            sub.push(Ins::Match);
            let placeholder = out.len();
            out.push(Ins::Look {
                neg: *neg,
                sub,
                next: 0,
            });
            // patch: continue right after this instruction
            let next = placeholder + 1;
            if let Ins::Look { next: slot, .. } = &mut out[placeholder] {
                *slot = next;
            }
        }
        Node::Concat(items) => {
            for i in items {
                compile(i, out);
            }
        }
        Node::Alt(branches) => {
            // Split(L1, L2) L1: <b1> Jmp End  L2: <b2> ... End:
            let mut ends = Vec::new();
            let n = branches.len();
            for (i, b) in branches.iter().enumerate() {
                if i + 1 == n {
                    compile(b, out);
                } else {
                    let split_at = out.len();
                    out.push(Ins::Split(0, 0));
                    let body_start = out.len();
                    compile(b, out);
                    let jmp_at = out.len();
                    out.push(Ins::Jmp(0));
                    ends.push(jmp_at);
                    let next_alt = out.len();
                    if let Ins::Split(a, _) = &mut out[split_at] {
                        *a = body_start;
                        // second alternative follows right after the jmp
                        let _ = next_alt;
                    }
                    // the fallback of this split is patched below
                    let fallback = out.len();
                    if let Ins::Split(_, bslot) = &mut out[split_at] {
                        *bslot = fallback;
                    }
                }
            }
            let end_at = out.len();
            for e in ends {
                if let Ins::Jmp(slot) = &mut out[e] {
                    *slot = end_at;
                }
            }
        }
        Node::Repeat {
            min,
            max,
            lazy,
            inner,
        } => {
            compile_repeat(*min, *max, *lazy, inner, out);
        }
    }
}

#[allow(clippy::comparison_chain)]
fn compile_repeat(min: u32, max: Option<u32>, lazy: bool, inner: &Node, out: &mut Vec<Ins>) {
    // minimum part: simply inline the body `min` times
    for _ in 0..min {
        compile(inner, out);
    }
    match max {
        None => {
            // L: Split(body, exit)  body: <inner> Jmp L
            let l = out.len();
            out.push(Ins::Split(0, 0));
            let body = out.len();
            compile(inner, out);
            out.push(Ins::Jmp(l));
            let exit = out.len();
            if let Ins::Split(a, b) = &mut out[l] {
                if lazy {
                    *a = exit;
                    *b = body;
                } else {
                    *a = body;
                    *b = exit;
                }
            }
        }
        Some(mx) => {
            if mx <= min {
                return;
            }
            let optional = mx - min;
            // chain `optional` optional copies: Split(body, exit_i) … exit_i:
            let mut splits = Vec::new();
            for _ in 0..optional {
                let sp = out.len();
                out.push(Ins::Split(0, 0));
                let body = out.len();
                compile(inner, out);
                splits.push((sp, body));
            }
            let exit = out.len();
            for (sp, body) in splits {
                if let Ins::Split(a, b) = &mut out[sp] {
                    if lazy {
                        *a = exit;
                        *b = body;
                    } else {
                        *a = body;
                        *b = exit;
                    }
                }
            }
        }
    }
}

struct Runner<'a> {
    chars: &'a [char],
    fl: &'a Flags,
    caps: &'a mut [(usize, usize)],
    steps: u32,
    seen: std::collections::HashSet<(usize, usize)>,
}

impl<'a> Runner<'a> {
    fn run(&mut self, code: &'a [Ins], pc: usize, sp: usize) -> bool {
        self.steps += 1;
        if self.steps > 3_000_000 {
            return false;
        }
        // loop detection: the same (pc, position) can never succeed twice
        if !self.seen.insert((pc, sp)) {
            return false;
        }
        let Some(ins) = code.get(pc) else { return true };
        match ins {
            Ins::Match => true,
            Ins::Char(c) => {
                let Some(x) = self.chars.get(sp) else {
                    return false;
                };
                if char_eq(*x, *c, self.fl.ignore_case) {
                    self.run(code, pc + 1, sp + 1)
                } else {
                    false
                }
            }
            Ins::Any => {
                let Some(x) = self.chars.get(sp) else {
                    return false;
                };
                if self.fl.dot_all || *x != '\n' {
                    self.run(code, pc + 1, sp + 1)
                } else {
                    false
                }
            }
            Ins::Class { negated, items } => {
                let Some(&x) = self.chars.get(sp) else {
                    return false;
                };
                if items.iter().any(|i| i.matches(x, self.fl.ignore_case)) != *negated {
                    self.run(code, pc + 1, sp + 1)
                } else {
                    false
                }
            }
            Ins::Start => {
                let hit = sp == 0 || (self.fl.multiline && self.chars.get(sp - 1) == Some(&'\n'));
                hit && self.run(code, pc + 1, sp)
            }
            Ins::End => {
                let hit = sp == self.chars.len()
                    || (self.fl.multiline && self.chars.get(sp) == Some(&'\n'));
                hit && self.run(code, pc + 1, sp)
            }
            Ins::WordB(want) => {
                let before = sp > 0 && is_word(self.chars[sp - 1]);
                let after = sp < self.chars.len() && is_word(self.chars[sp]);
                ((before != after) == *want) && self.run(code, pc + 1, sp)
            }
            Ins::BackRef(i) => {
                let Some(&(a, b)) = self.caps.get(*i) else {
                    return self.run(code, pc + 1, sp);
                };
                if a == usize::MAX || b == usize::MAX || b <= a {
                    return self.run(code, pc + 1, sp);
                }
                let len = b - a;
                if sp + len > self.chars.len() {
                    return false;
                }
                for j in 0..len {
                    if !char_eq(self.chars[a + j], self.chars[sp + j], self.fl.ignore_case) {
                        return false;
                    }
                }
                self.run(code, pc + 1, sp + len)
            }
            Ins::Jmp(t) => self.run(code, *t, sp),
            Ins::Split(a, b) => self.run(code, *a, sp) || self.run(code, *b, sp),
            Ins::SaveStart(n) => {
                let n = *n;
                let old = self.caps.get(n).copied();
                if n < self.caps.len() {
                    self.caps[n] = (sp, usize::MAX);
                }
                if self.run(code, pc + 1, sp) {
                    return true;
                }
                if let Some(prev) = old {
                    self.caps[n] = prev;
                }
                false
            }
            Ins::SaveEnd(n) => {
                let n = *n;
                let old = self.caps.get(n).copied();
                let begin = old.map(|o| o.0).unwrap_or(sp);
                if n < self.caps.len() {
                    self.caps[n] = (begin, sp);
                }
                if self.run(code, pc + 1, sp) {
                    return true;
                }
                if let Some(prev) = old {
                    self.caps[n] = prev;
                }
                false
            }
            Ins::Look { neg, sub, next } => {
                let mut saved = self.caps.to_vec();
                let found = {
                    let mut r = Runner {
                        chars: self.chars,
                        fl: self.fl,
                        caps: &mut saved,
                        steps: self.steps,
                        seen: std::collections::HashSet::new(),
                    };
                    let r2 = r.run(sub, 0, sp);
                    self.steps = r.steps;
                    r2
                };
                if found != *neg {
                    if !*neg {
                        for (i, c) in saved.iter().enumerate() {
                            if c.0 != usize::MAX && self.caps[i].0 == usize::MAX {
                                self.caps[i] = *c;
                            }
                        }
                    }
                    self.run(code, *next, sp)
                } else {
                    false
                }
            }
        }
    }
}

fn char_eq(a: char, b: char, fold: bool) -> bool {
    a == b || (fold && a.to_lowercase().eq(b.to_lowercase()))
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Convenience for `String.prototype.replace`.
pub fn replace_with<F>(re: &RegExp, text: &str, mut f: F) -> String
where
    F: FnMut(&MatchInfo) -> Value,
{
    let flags = re.flags_struct();
    let matches = if flags.global {
        re.find_all(text)
    } else {
        re.find(text, 0).into_iter().collect::<Vec<_>>()
    };
    if matches.is_empty() {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut i = 0usize;
    for m in &matches {
        while i < m.start {
            out.push(chars[i]);
            i += 1;
        }
        out.push_str(&f(m).to_string());
        i = m.end.max(i);
    }
    while i < chars.len() {
        out.push(chars[i]);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn re(src: &str, flags: &str) -> RegExp {
        RegExp::compile(src, flags).unwrap_or_else(|e| panic!("{:?}", e.0))
    }

    #[test]
    fn literals_and_dot() {
        assert!(re("abc", "").test("xxabcxx"));
        assert!(!re("abc", "").test("ab"));
        assert!(re("a.c", "").test("a-c"));
        assert!(!re("a.c", "").test("a\nc"));
        assert!(re("a.c", "s").test("a\nc"));
    }

    #[test]
    fn classes_and_escapes() {
        assert!(re(r#"[a-c]+"#, "").test("bbb"));
        assert!(re(r#"[^0-9]+"#, "").test("xyz"));
        // `[^0-9]+` still matches the leading letter, exactly like JS
        assert_eq!(re(r#"[^0-9]+"#, "").first("x1").unwrap().text, "x");
        assert!(re(r"\d{3}-\d{4}", "").test("tel 555-1234"));
        assert!(re(r"\w+@\w+\.\w+", "").test("mail me@taz.de now"));
        assert!(re(r"\s+", "").test("a b"));
        assert_eq!(re(r"é", "").first("café au").unwrap().text, "é");
    }

    #[test]
    fn quantifiers_greedy_and_lazy() {
        let m = re("<(.+)>", "").first("<a><b>").unwrap();
        assert_eq!(m.text, "<a><b>", "greedy takes the longest");
        let m = re("<(.+?)>", "").first("<a><b>").unwrap();
        assert_eq!(m.text, "<a>", "lazy takes the shortest");
        assert!(re("ab{2,3}c", "").test("abbc"));
        assert!(!re("ab{2,3}c", "").test("ac"));
    }

    #[test]
    fn groups_and_alternation() {
        let m = re(r"(2024)-(0[1-9]|1[0-2])-(\d{2})", "")
            .first("am 2024-07-19 ok")
            .unwrap();
        assert_eq!(m.text, "2024-07-19");
        assert_eq!(m.groups[1].as_deref(), Some("2024"));
        assert_eq!(m.groups[2].as_deref(), Some("07"));
        assert_eq!(m.groups[3].as_deref(), Some("19"));
        assert!(re("cat|dog", "").test("my dog"));
        let m = re("(?:foo)(bar)", "").first("foobar").unwrap();
        assert_eq!(m.groups[1].as_deref(), Some("bar"));
    }

    #[test]
    fn anchors_and_multiline() {
        assert!(re("^abc", "").test("abcdef"));
        assert!(!re("^abc", "").test("xabc"));
        let text = "one\ntwo\nabc";
        assert!(re("^abc", "m").test(text));
        assert!(re("abc$", "m").test(text));
        assert!(!re("abc$", "").test("abc\ndef"));
    }

    #[test]
    fn case_insensitive() {
        assert!(re("taz", "i").test("TAZ.de"));
        assert!(!re("taz", "").test("TAZ.de"));
        assert!(re("[A-Z]+", "i").test("abc"));
    }

    #[test]
    fn global_find_all() {
        let all = re("\\d+", "g").find_all("a1 bb22 ccc333");
        let got: Vec<&str> = all.iter().map(|m| m.text.as_str()).collect();
        assert_eq!(got, vec!["1", "22", "333"]);
    }

    #[test]
    fn zero_length_matches_terminate() {
        // `x*` matches empty everywhere; find_all must not hang
        let all = re("x*", "g").find_all("abc");
        assert_eq!(all.len(), 4);
    }

    #[test]
    fn lookaround() {
        assert!(re("foo(?=bar)", "").test("foobar"));
        assert!(!re("foo(?!bar)", "").test("foobar"));
        assert!(re("foo(?!bar)", "").test("foobaz"));
        // the look-ahead demands the trailing dash, just like in JS
        assert!(re("(?=\\d{4}-)(\\d{4})", "").first("2024").is_none());
        let m = re("(?=\\d{4}-)(\\d{4})", "").first("2024-07").unwrap();
        assert_eq!(m.groups[1].as_deref(), Some("2024"));
        // named groups capture like plain ones, lookbehind is refused (not
        // silently mis-compiled into a lookahead)
        assert!(re("(?<y>\\d{4})", "").test("2024"));
        assert!(RegExp::compile("(?<=a)b", "").is_err());
        assert!(RegExp::compile("(?<!a)b", "").is_err());
        assert!(RegExp::compile("(?i)a", "").is_err());
    }

    #[test]
    fn backreference() {
        assert!(re(r"(.)\1", "").test("aa"));
        assert!(!re(r"(.)\1", "").test("ab"));
        assert!(re(r"<(\w+)>\1", "").test("<a>a"));
    }

    #[test]
    fn word_boundary() {
        assert!(re(r"\bword\b", "").test("a word here"));
        assert!(!re(r"\bword\b", "").test("words"));
    }

    #[test]
    fn replace_helper() {
        let out = replace_with(&re("a", "g"), "banana", |_| Value::string("o"));
        assert_eq!(out, "bonono");
        let out = replace_with(&re("(\\w)(\\w*)", "g"), "hello world", |m| {
            let g = m.groups[1].clone().unwrap_or_default();
            Value::string(g.to_uppercase() + &m.groups[2].clone().unwrap_or_default())
        });
        assert_eq!(out, "Hello World");
    }

    #[test]
    fn bad_patterns_error_not_panic() {
        for src in ["(", ")", "[a-", "\\", "*", "a{2,1}?", "(?P<", "?"] {
            let _ = RegExp::compile(src, "");
        }
    }
}
