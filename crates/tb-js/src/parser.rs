//! JavaScript parser: recursive descent, precedence climbing for expressions.
//!
//! Covered: `var/let/const` + destructuring (array/object, defaults, rest,
//! nested), functions (declaration, expression, arrow, async, methods,
//! generators parsed), classes (`extends`, `super`, static, getters/setters,
//! fields), `if/else`, `for`/`for-in`/`for-of`/`while`/`do`, labeled
//! `break`/`continue`, `switch`, `try/catch/finally`, `throw`, `return`,
//! optional chaining, nullish coalescing, logical assignment, exponent, template
//! literals (incl. tagged), regex literals, spread/rest, `new.target`,
//! `import()`, ASI.
//!
//! [`Parser::parse_program`] is strict like a browser: one syntax error fails
//! the script. [`Parser::parse_tolerant`] keeps going by skipping to the next
//! statement boundary — useful for third-party vendor scripts we do not fully
//! support, so one exotic file cannot kill a page.

use super::ast::*;
use super::lexer::{lex, Lexed, Token, P};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Debug, Clone)]
pub struct ParseError {
    pub message: String,
    pub line: u32,
}

impl ParseError {
    pub fn new(message: impl Into<String>, line: u32) -> ParseError {
        ParseError {
            message: message.into(),
            line,
        }
    }
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SyntaxError: {} (line {})", self.message, self.line)
    }
}

pub struct Parser {
    toks: Vec<Lexed>,
    pos: usize,
    /// current recursion depth, see [`Parser::enter`] (`RefCell` so the guard
    /// can release its slot while `&mut self` is otherwise in use)
    depth: Rc<RefCell<u32>>,
}

/// How deeply statements, blocks and expressions may nest. Minified vendor code
/// nests a lot, and a stack overflow would take the whole browser down with it,
/// so deep input becomes a syntax error instead (V8: "Maximum call stack size
/// exceeded"). 200 is far above anything a real page uses.
pub const MAX_NEST: u32 = 200;

/// Frees the depth slot taken by [`Parser::enter`] when the call returns.
struct Depth(Rc<RefCell<u32>>);

impl Drop for Depth {
    fn drop(&mut self) {
        let mut d = self.0.borrow_mut();
        *d -= 1;
    }
}

type PR<T> = Result<T, ParseError>;

impl Parser {
    /// Parse a whole script (strict: first syntax error wins).
    pub fn parse_program(src: &str) -> PR<Program> {
        let toks = lex(src).map_err(|e| ParseError::new(e.message, e.line))?;
        let mut p = Parser {
            toks,
            pos: 0,
            depth: Rc::new(RefCell::new(0)),
        };
        let body = p.parse_statements_until_eof()?;
        let strict = matches!(
            body.first().map(|s| &s.kind),
            Some(StmtKind::Expr(e)) if matches!(&e.node, Node::Str(s) if &**s == "use strict")
        );
        Ok(Program { body, strict })
    }

    /// Parse as much as possible; collect the syntax errors that were skipped.
    pub fn parse_tolerant(src: &str, problems: &mut Vec<String>) -> Program {
        let toks = match lex(src) {
            Ok(t) => t,
            Err(e) => {
                problems.push(format!("{e}"));
                return Program::default();
            }
        };
        let mut p = Parser {
            toks,
            pos: 0,
            depth: Rc::new(RefCell::new(0)),
        };
        let mut body: Vec<S> = Vec::new();
        while !p.at(Token::Eof) {
            match p.parse_statement() {
                Ok(s) => body.push(s),
                Err(e) => {
                    problems.push(format!("{e}"));
                    p.recover();
                }
            }
        }
        Program {
            body,
            strict: false,
        }
    }

    /// Skip forward to a plausible statement boundary after an error.
    fn recover(&mut self) {
        let mut depth = 0i32;
        while !self.at(Token::Eof) {
            let t = self.cur().clone();
            match t {
                Token::Punct(P::LBrace | P::LBracket | P::LParen) => depth += 1,
                Token::Punct(P::RBrace | P::RBracket | P::RParen) => {
                    depth -= 1;
                    if depth < 0 {
                        depth = 0;
                    }
                }
                Token::Punct(P::Semi) => {
                    self.bump();
                    return;
                }
                _ => {}
            }
            let nl_ahead = self.peek_lex(1).nl;
            self.bump();
            if depth == 0 && nl_ahead {
                return;
            }
        }
    }

    // ------------------------------------------------------------- plumbing --

    fn cur(&self) -> &Token {
        &self.toks[self.pos.min(self.toks.len() - 1)].tok
    }

    fn peek(&self, n: usize) -> &Token {
        &self.toks[self.pos.saturating_add(n).min(self.toks.len() - 1)].tok
    }

    fn peek_lex(&self, n: usize) -> &Lexed {
        &self.toks[self.pos.saturating_add(n).min(self.toks.len() - 1)]
    }

    fn at(&self, t: Token) -> bool {
        *self.cur() == t
    }

    fn at_p(&self, p: P) -> bool {
        self.cur().is_punct(p)
    }

    fn line(&self) -> u32 {
        self.peek_lex(0).span.line
    }

    fn bump(&mut self) -> Lexed {
        let t = self.peek_lex(0).clone();
        if !matches!(t.tok, Token::Eof) {
            self.pos += 1;
        }
        t
    }

    fn eat(&mut self, p: P) -> bool {
        if self.at_p(p) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, p: P) -> PR<()> {
        if self.at_p(p) {
            self.bump();
            Ok(())
        } else {
            Err(ParseError::new(
                format!("expected `{p}`, found `{}`", self.describe()),
                self.line(),
            ))
        }
    }

    fn expect_ident(&mut self) -> PR<Rc<str>> {
        match self.cur().clone() {
            Token::Ident(s) => {
                self.bump();
                Ok(Rc::from(s))
            }
            _ => Err(ParseError::new(
                format!("expected identifier, found `{}`", self.describe()),
                self.line(),
            )),
        }
    }

    fn describe(&self) -> String {
        match self.cur() {
            Token::Ident(s) => s.clone(),
            Token::Number(n) => n.to_string(),
            Token::String(s) => format!("{s:?}"),
            Token::Template(_) => "`…`".into(),
            Token::Regex(_, _) => "/…/".into(),
            Token::Punct(p) => p.to_string(),
            Token::Eof => "end of input".into(),
        }
    }

    /// `;`, end of a block, end of input **or** a newline before the next token
    /// (automatic semicolon insertion).
    fn semicolon(&mut self) -> PR<()> {
        if self.eat(P::Semi) || self.at_p(P::RBrace) || self.at(Token::Eof) {
            return Ok(());
        }
        if self.peek_lex(0).nl {
            return Ok(());
        }
        Err(ParseError::new(
            format!("expected `;`, found `{}`", self.describe()),
            self.line(),
        ))
    }

    fn parse_statements_until_eof(&mut self) -> PR<Vec<S>> {
        let mut out = Vec::new();
        while !self.at(Token::Eof) {
            out.push(self.parse_statement()?);
        }
        Ok(out)
    }

    fn parse_block(&mut self) -> PR<Rc<Block>> {
        let _deep = self.enter()?;
        let line = self.line();
        self.expect(P::LBrace)?;
        let mut body = Vec::new();
        while !self.at_p(P::RBrace) {
            if self.at(Token::Eof) {
                return Err(ParseError::new("unterminated block", line));
            }
            body.push(self.parse_statement()?);
        }
        self.expect(P::RBrace)?;
        Ok(Rc::new(Block { body }))
    }

    // ----------------------------------------------------------- statements --

    /// One level of recursion. Every entry point that can call itself through
    /// the AST (statements, blocks, expressions, patterns) takes a slot.
    fn enter(&self) -> PR<Depth> {
        if *self.depth.borrow() >= MAX_NEST {
            return Err(ParseError::new("nests too deeply", self.line()));
        }
        *self.depth.borrow_mut() += 1;
        Ok(Depth(self.depth.clone()))
    }

    fn parse_statement(&mut self) -> PR<S> {
        let _deep = self.enter()?;
        let line = self.line();
        // labeled statement: `name:` (but not `default:`)
        if let Token::Ident(name) = self.cur().clone() {
            if self.peek(1).is_punct(P::Colon) && !P::from_keyword_shaped(&name) {
                self.bump();
                self.bump();
                let inner = self.parse_statement()?;
                return Ok(stmt(StmtKind::Labeled(Rc::from(name), inner), line));
            }
        }
        let kind = match self.cur().clone() {
            Token::Punct(p) => match p {
                P::LBrace => StmtKind::Block(self.parse_block()?),
                P::Semi => {
                    self.bump();
                    StmtKind::Empty
                }
                P::Var => {
                    self.bump();
                    StmtKind::Decl {
                        kind: VarKind::Var,
                        decls: self.parse_declarators()?,
                    }
                }
                P::Let => {
                    self.bump();
                    let kind = self.parse_let_or_const_after_keyword(VarKind::Let)?;
                    kind
                }
                P::Const => {
                    self.bump();
                    StmtKind::Decl {
                        kind: VarKind::Const,
                        decls: self.parse_declarators()?,
                    }
                }
                P::If => {
                    self.bump();
                    self.expect(P::LParen)?;
                    let test = self.parse_expression()?;
                    self.expect(P::RParen)?;
                    let cons = self.parse_statement()?;
                    let els = if self.eat(P::Else) {
                        Some(self.parse_statement()?)
                    } else {
                        None
                    };
                    StmtKind::If { test, cons, els }
                }
                P::While => {
                    self.bump();
                    self.expect(P::LParen)?;
                    let test = self.parse_expression()?;
                    self.expect(P::RParen)?;
                    let body = self.parse_statement()?;
                    StmtKind::While { test, body }
                }
                P::Do => {
                    self.bump();
                    let body = self.parse_statement()?;
                    self.expect(P::While)?;
                    self.expect(P::LParen)?;
                    let test = self.parse_expression()?;
                    self.expect(P::RParen)?;
                    let _ = self.eat(P::Semi);
                    StmtKind::Do { body, test }
                }
                P::For => return self.parse_for(line),
                P::Switch => {
                    self.bump();
                    self.expect(P::LParen)?;
                    let disc = self.parse_expression()?;
                    self.expect(P::RParen)?;
                    self.expect(P::LBrace)?;
                    let mut cases = Vec::new();
                    while !self.at_p(P::RBrace) {
                        if self.at(Token::Eof) {
                            return Err(ParseError::new("unterminated switch", line));
                        }
                        let test = if self.eat(P::Case) {
                            Some(self.parse_expression()?)
                        } else if self.eat(P::Default) {
                            None
                        } else {
                            return Err(ParseError::new(
                                "expected `case`/`default` in switch",
                                self.line(),
                            ));
                        };
                        self.expect(P::Colon)?;
                        let mut body = Vec::new();
                        while !self.at_p(P::RBrace) && !self.at_p(P::Case) && !self.at_p(P::Default)
                        {
                            body.push(self.parse_statement()?);
                        }
                        cases.push(Case { test, body });
                    }
                    self.expect(P::RBrace)?;
                    StmtKind::Switch { disc, cases }
                }
                P::Return => {
                    self.bump();
                    let value = if self.at_p(P::Semi)
                        || self.at_p(P::RBrace)
                        || self.at(Token::Eof)
                        || self.peek_lex(0).nl
                    {
                        None
                    } else {
                        Some(self.parse_expression()?)
                    };
                    self.semicolon()?;
                    StmtKind::Return(value)
                }
                P::Break | P::Continue => {
                    self.bump();
                    let label = if self.at_p(P::Semi)
                        || self.at_p(P::RBrace)
                        || self.at(Token::Eof)
                        || self.peek_lex(0).nl
                    {
                        None
                    } else {
                        Some(self.expect_ident()?)
                    };
                    self.semicolon()?;
                    if p == P::Break {
                        StmtKind::Break(label)
                    } else {
                        StmtKind::Continue(label)
                    }
                }
                P::Throw => {
                    self.bump();
                    if self.peek_lex(0).nl {
                        return Err(ParseError::new("illegal newline after `throw`", line));
                    }
                    let value = self.parse_expression()?;
                    self.semicolon()?;
                    StmtKind::Throw(value)
                }
                P::Try => {
                    self.bump();
                    let block = self.parse_block()?;
                    let catch = if self.eat(P::Catch) {
                        let param = if self.eat(P::LParen) {
                            let pat = self.parse_binding_pattern()?;
                            self.expect(P::RParen)?;
                            Some(pat)
                        } else {
                            None
                        };
                        Some(Catch {
                            param,
                            body: self.parse_block()?,
                        })
                    } else {
                        None
                    };
                    let finalizer = if self.eat(P::Finally) {
                        Some(self.parse_block()?)
                    } else {
                        None
                    };
                    if catch.is_none() && finalizer.is_none() {
                        return Err(ParseError::new("`try` needs `catch` or `finally`", line));
                    }
                    StmtKind::Try {
                        block,
                        catch,
                        finalizer,
                    }
                }
                P::Function => {
                    let f = self.parse_function(false, false)?;
                    StmtKind::Fn(f)
                }
                P::Async => {
                    if self.peek(1).is_punct(P::Function) {
                        self.bump();
                        let f = self.parse_function(true, false)?;
                        StmtKind::Fn(f)
                    } else {
                        let e = self.parse_expression()?;
                        self.semicolon()?;
                        StmtKind::Expr(e)
                    }
                }
                P::Class => {
                    let c = self.parse_class()?;
                    StmtKind::Class(c)
                }
                P::Debugger => {
                    self.bump();
                    self.semicolon()?;
                    StmtKind::Debugger
                }
                _ => {
                    let e = self.parse_expression()?;
                    self.semicolon()?;
                    StmtKind::Expr(e)
                }
            },
            Token::Ident(_)
            | Token::Number(_)
            | Token::String(_)
            | Token::Template(_)
            | Token::Regex(_, _) => {
                let e = self.parse_expression()?;
                self.semicolon()?;
                StmtKind::Expr(e)
            }
            Token::Eof => return Err(ParseError::new("unexpected end of input", line)),
        };
        Ok(stmt(kind, line))
    }

    fn parse_let_or_const_after_keyword(&mut self, kind: VarKind) -> PR<StmtKind> {
        // `let` can also be an identifier (`let = 3`), and `let` followed by a
        // newline is not a declaration
        if self.at_p(P::Assign) || (self.peek_lex(0).nl && !matches!(self.cur(), Token::Ident(_))) {
            let e = self.parse_assignment()?;
            self.semicolon()?;
            return Ok(StmtKind::Expr(e));
        }
        Ok(StmtKind::Decl {
            kind,
            decls: self.parse_declarators()?,
        })
    }

    fn parse_for(&mut self, line: u32) -> PR<S> {
        self.expect(P::For)?;
        // `for await (const x of ys)` — the await only marks it as async iteration
        let is_await = self.eat(P::Await);
        self.expect(P::LParen)?;
        // for (init; …)  |  for (x in y)  |  for (x of y)  |  for (;;)
        if self.at_p(P::Semi) {
            self.bump();
            return self.finish_for(None, line);
        }
        let decl_kind = match self.cur() {
            Token::Punct(P::Var) => Some(VarKind::Var),
            Token::Punct(P::Let) => Some(VarKind::Let),
            Token::Punct(P::Const) => Some(VarKind::Const),
            _ => None,
        };
        if let Some(k) = decl_kind {
            self.bump();
            let decls = self.parse_declarators_one()?;
            if self.eat(P::In) {
                let obj = self.parse_expression()?;
                self.expect(P::RParen)?;
                let body = self.parse_statement()?;
                return Ok(stmt(
                    StmtKind::ForIn {
                        left: ForInit::Decl(k, decls),
                        obj,
                        body,
                    },
                    line,
                ));
            }
            if self.at_p(P::Of) || self.at_ident_of("of") {
                self.bump();
                let obj = self.parse_assignment()?;
                self.expect(P::RParen)?;
                let body = self.parse_statement()?;
                return Ok(stmt(
                    StmtKind::ForOf {
                        left: ForInit::Decl(k, decls),
                        obj,
                        body,
                        await_: is_await,
                    },
                    line,
                ));
            }
            self.expect(P::Semi)?;
            return self.finish_for(Some(ForInit::Decl(k, decls)), line);
        }
        // expression init
        let init = self.parse_expression_start()?;
        if self.at_p(P::In) {
            self.bump();
            let obj = self.parse_expression()?;
            self.expect(P::RParen)?;
            let body = self.parse_statement()?;
            let left = ForInit::Pattern(to_pattern_expr(init)?);
            return Ok(stmt(StmtKind::ForIn { left, obj, body }, line));
        }
        if self.at_p(P::Of) || self.at_ident_of("of") {
            self.bump();
            let obj = self.parse_assignment()?;
            self.expect(P::RParen)?;
            let body = self.parse_statement()?;
            let left = ForInit::Pattern(to_pattern_expr(init)?);
            return Ok(stmt(
                StmtKind::ForOf {
                    left,
                    obj,
                    body,
                    await_: is_await,
                },
                line,
            ));
        }
        self.expect(P::Semi)?;
        self.finish_for(Some(ForInit::Expr(init)), line)
    }

    fn finish_for(&mut self, init: Option<ForInit>, line: u32) -> PR<S> {
        let test = if self.at_p(P::Semi) {
            None
        } else {
            Some(self.parse_expression()?)
        };
        self.expect(P::Semi)?;
        let update = if self.at_p(P::RParen) {
            None
        } else {
            Some(self.parse_expression()?)
        };
        self.expect(P::RParen)?;
        let body = self.parse_statement()?;
        Ok(stmt(
            StmtKind::For {
                init,
                test,
                update,
                body,
            },
            line,
        ))
    }

    fn at_ident_of(&self, name: &str) -> bool {
        matches!(self.cur(), Token::Ident(s) if s == name)
    }

    fn parse_declarators(&mut self) -> PR<Vec<Declarator>> {
        let mut out = vec![self.parse_declarator()?];
        while self.eat(P::Comma) {
            out.push(self.parse_declarator()?);
        }
        self.semicolon()?;
        Ok(out)
    }

    fn parse_declarators_one(&mut self) -> PR<Vec<Declarator>> {
        let mut out = vec![self.parse_declarator()?];
        while self.eat(P::Comma) {
            out.push(self.parse_declarator()?);
        }
        Ok(out)
    }

    fn parse_declarator(&mut self) -> PR<Declarator> {
        let pattern = self.parse_binding_pattern()?;
        let init = if self.eat(P::Assign) {
            Some(self.parse_assignment()?)
        } else {
            None
        };
        Ok(Declarator { pattern, init })
    }

    // ------------------------------------------------------- binding patterns --

    fn parse_binding_pattern(&mut self) -> PR<Pattern> {
        let _deep = self.enter()?;
        match self.cur() {
            Token::Punct(P::LBracket) => self.parse_array_pattern(),
            Token::Punct(P::LBrace) => self.parse_object_pattern(),
            _ => Ok(Pattern::Binding(self.expect_ident()?)),
        }
    }

    fn parse_array_pattern(&mut self) -> PR<Pattern> {
        self.expect(P::LBracket)?;
        let mut items = Vec::new();
        while !self.at_p(P::RBracket) {
            if self.at(Token::Eof) {
                return Err(ParseError::new("unterminated array pattern", self.line()));
            }
            if self.eat(P::Comma) {
                items.push(ArrayEl::Hole);
                continue;
            }
            if self.eat(P::Ellipsis) {
                items.push(ArrayEl::Rest(self.expect_ident()?));
                if !self.eat(P::Comma) {
                    break;
                }
                continue;
            }
            let pat = self.parse_binding_pattern()?;
            let default = if self.eat(P::Assign) {
                Some(self.parse_assignment()?)
            } else {
                None
            };
            items.push(ArrayEl::Item(pat, default));
            if !self.eat(P::Comma) {
                break;
            }
        }
        self.expect(P::RBracket)?;
        Ok(Pattern::Array(items))
    }

    fn parse_object_pattern(&mut self) -> PR<Pattern> {
        self.expect(P::LBrace)?;
        let mut items = Vec::new();
        while !self.at_p(P::RBrace) {
            if self.at(Token::Eof) {
                return Err(ParseError::new("unterminated object pattern", self.line()));
            }
            if self.eat(P::Ellipsis) {
                items.push(ObjectEl {
                    key: Key::Name(Rc::from("")),
                    pattern: Pattern::Binding(self.expect_ident()?),
                    default: None,
                    rest: true,
                });
                if !self.eat(P::Comma) {
                    break;
                }
                continue;
            }
            let (key, computed) = self.parse_property_key()?;
            let (pattern, default) = if self.eat(P::Colon) {
                let pat = self.parse_binding_pattern()?;
                let d = if self.eat(P::Assign) {
                    Some(self.parse_assignment()?)
                } else {
                    None
                };
                (pat, d)
            } else {
                let name = match &key {
                    Key::Name(n) => n.clone(),
                    _ => {
                        return Err(ParseError::new(
                            "shorthand pattern needs a name",
                            self.line(),
                        ))
                    }
                };
                let d = if self.eat(P::Assign) {
                    Some(self.parse_assignment()?)
                } else {
                    None
                };
                (Pattern::Binding(name), d)
            };
            if computed && false {
                // keep the compiler happy about the unused flag
            }
            items.push(ObjectEl {
                key,
                pattern,
                default,
                rest: false,
            });
            if !self.eat(P::Comma) {
                break;
            }
        }
        self.expect(P::RBrace)?;
        Ok(Pattern::Object(items))
    }

    fn parse_property_key(&mut self) -> PR<(Key, bool)> {
        if self.eat(P::LBracket) {
            let e = self.parse_assignment()?;
            self.expect(P::RBracket)?;
            return Ok((Key::Computed(e), true));
        }
        match self.cur().clone() {
            Token::Ident(name) => {
                self.bump();
                Ok((Key::Name(Rc::from(name)), false))
            }
            Token::String(s) => {
                self.bump();
                Ok((Key::Name(Rc::from(s)), false))
            }
            Token::Number(n) => {
                self.bump();
                Ok((Key::Num(n), false))
            }
            Token::Punct(p) => {
                // contextual keywords used as property names: get/set/static/async/of
                if let Some(kw) = keywordish(p) {
                    self.bump();
                    return Ok((Key::Name(Rc::from(kw)), false));
                }
                Err(ParseError::new(
                    format!("bad property key `{p}`"),
                    self.line(),
                ))
            }
            _ => Err(ParseError::new(
                format!("bad property key `{}`", self.describe()),
                self.line(),
            )),
        }
    }
}

impl Parser {
    // ----------------------------------------------------- functions/class --

    /// `function name(...) { ... }` — `async`/`generator` flags come from context.
    pub(crate) fn parse_function(&mut self, is_async: bool, is_method: bool) -> PR<Rc<Function>> {
        let line = self.line();
        self.expect(P::Function)?;
        let is_generator = self.eat(P::Star);
        let name = match self.cur() {
            Token::Ident(s) => {
                let s = s.clone();
                self.bump();
                Some(Rc::from(s))
            }
            _ => None,
        };
        let params = self.parse_params()?;
        let body = self.parse_block()?;
        Ok(Rc::new(Function {
            name,
            params,
            body,
            is_async,
            is_generator,
            is_method,
            is_arrow: false,
            line,
        }))
    }

    fn parse_params(&mut self) -> PR<Vec<Param>> {
        self.expect(P::LParen)?;
        let mut params = Vec::new();
        while !self.at_p(P::RParen) {
            if self.at(Token::Eof) {
                return Err(ParseError::new("unterminated parameter list", self.line()));
            }
            if self.eat(P::Ellipsis) {
                let pat = self.parse_binding_pattern()?;
                params.push(Param {
                    pattern: pat,
                    default: None,
                    rest: true,
                });
            } else {
                let pattern = self.parse_binding_pattern()?;
                let default = if self.eat(P::Assign) {
                    Some(self.parse_assignment()?)
                } else {
                    None
                };
                params.push(Param {
                    pattern,
                    default,
                    rest: false,
                });
            }
            if !self.eat(P::Comma) {
                break;
            }
        }
        self.expect(P::RParen)?;
        Ok(params)
    }

    pub(crate) fn parse_class(&mut self) -> PR<Rc<ClassDecl>> {
        let line = self.line();
        self.expect(P::Class)?;
        let name = match self.cur() {
            Token::Ident(s) => {
                let s = s.clone();
                self.bump();
                Some(Rc::from(s))
            }
            _ => None,
        };
        let super_class = if self.eat(P::Extends) {
            Some(self.parse_unary()?)
        } else {
            None
        };
        // the heritage expression may be followed by a call: `class A extends B()`
        let super_class = match super_class {
            Some(sc) => {
                if self.at_p(P::LParen) {
                    Some(self.parse_call_tail(sc)?)
                } else {
                    Some(sc)
                }
            }
            None => None,
        };
        self.expect(P::LBrace)?;
        let mut members = Vec::new();
        while !self.at_p(P::RBrace) {
            if self.at(Token::Eof) {
                return Err(ParseError::new("unterminated class body", line));
            }
            if self.eat(P::Semi) {
                continue;
            }
            members.push(self.parse_class_member()?);
        }
        self.expect(P::RBrace)?;
        Ok(Rc::new(ClassDecl {
            name,
            super_class,
            members,
            line,
        }))
    }

    fn parse_class_member(&mut self) -> PR<Member> {
        let mut is_static = false;
        if self.at_p(P::Static) {
            // `static` is only a modifier when something else follows
            match self.peek(1) {
                Token::Punct(P::LParen) | Token::Punct(P::Semi) | Token::Punct(P::Assign) => {}
                _ => {
                    is_static = true;
                    self.bump();
                }
            }
        }
        let mut kind_marker: Option<P> = None;
        if (self.at_p(P::Get) || self.at_p(P::Set)) && !self.peek(1).is_punct(P::LParen) {
            match self.peek(1) {
                Token::Punct(P::Colon) | Token::Punct(P::Comma) | Token::Punct(P::Assign) => {}
                _ => {
                    kind_marker = match self.cur() {
                        Token::Punct(p) => Some(*p),
                        _ => None,
                    };
                    self.bump();
                }
            }
        }
        let is_async = if self.at_p(P::Async)
            && !matches!(
                self.peek(1),
                Token::Punct(P::LParen) | Token::Punct(P::Semi) | Token::Punct(P::Assign)
            ) {
            self.bump();
            true
        } else {
            false
        };
        let is_generator = self.eat(P::Star);
        let (key, _computed) = self.parse_property_key()?;
        // method?
        if self.at_p(P::LParen) {
            let line = self.line();
            let params = self.parse_params()?;
            let body = self.parse_block()?;
            let f = Rc::new(Function {
                name: match &key {
                    Key::Name(n) => Some(n.clone()),
                    _ => None,
                },
                params,
                body,
                is_async,
                is_generator,
                is_method: true,
                is_arrow: false,
                line,
            });
            let kind = match kind_marker {
                Some(P::Get) => MemberKind::Getter(f),
                Some(P::Set) => MemberKind::Setter(f),
                _ => MemberKind::Method(f),
            };
            return Ok(Member {
                key,
                kind,
                is_static,
            });
        }
        // field: `x = 1` / `x`
        let init = if self.eat(P::Assign) {
            Some(self.parse_assignment()?)
        } else {
            None
        };
        let _ = self.eat(P::Semi);
        Ok(Member {
            key,
            kind: MemberKind::Field(init),
            is_static,
        })
    }

    // --------------------------------------------------------- expressions --

    /// Comma expression: `a, b`.
    pub(crate) fn parse_expression(&mut self) -> PR<E> {
        let _deep = self.enter()?;
        let line = self.line();
        let first = self.parse_assignment()?;
        if !self.at_p(P::Comma) {
            return Ok(first);
        }
        let mut parts = vec![first];
        while self.eat(P::Comma) {
            parts.push(self.parse_assignment()?);
        }
        Ok(expr(Node::Seq(parts), line))
    }

    /// Entry point that allows arrow functions to start (used for `for (x of y)`).
    fn parse_expression_start(&mut self) -> PR<E> {
        self.parse_assignment()
    }

    pub(crate) fn parse_assignment(&mut self) -> PR<E> {
        let line = self.line();
        // arrow function: `(a, b) => …` / `async (a) => …` / `a => …`
        if let Some(e) = self.try_parse_arrow()? {
            return Ok(e);
        }
        let lhs = self.parse_conditional()?;
        if let Some(op) = assign_op(self.cur()) {
            self.bump();
            let value = self.parse_assignment()?;
            return Ok(expr(
                Node::Assign {
                    target: lhs,
                    op: Some(op),
                    value,
                },
                line,
            ));
        }
        Ok(lhs)
    }

    fn try_parse_arrow(&mut self) -> PR<Option<E>> {
        let line = self.line();
        let idx = 0usize;
        let mut is_async = false;
        if self.at_p(P::Async) {
            // `async` only starts an arrow when followed by `(` or an ident on the
            // same line that is followed by `=>`
            match self.peek(1) {
                Token::Punct(P::LParen) => {
                    is_async = true;
                    self.bump(); // consume `async`, then handle the parameter list
                }
                Token::Ident(_) if !self.peek_lex(1).nl => {
                    if self.peek(2).is_punct(P::Arrow) && !self.peek_lex(2).nl {
                        let name = match self.peek(1) {
                            Token::Ident(s) => s.clone(),
                            _ => unreachable!(),
                        };
                        self.bump();
                        self.bump();
                        self.expect(P::Arrow)?;
                        let body = self.parse_arrow_body()?;
                        return Ok(Some(expr(
                            Node::Arrow {
                                params: vec![Param {
                                    pattern: Pattern::Binding(Rc::from(name)),
                                    default: None,
                                    rest: false,
                                }],
                                body,
                                is_async: true,
                            },
                            line,
                        )));
                    }
                    return Ok(None);
                }
                _ => return Ok(None),
            }
        }
        if self.at_p(P::LParen) {
            // find the matching `)` then look for `=>`
            let mut depth = 0i32;
            let mut i = self.pos;
            while i < self.toks.len() {
                match &self.toks[i].tok {
                    Token::Punct(P::LParen) => depth += 1,
                    Token::Punct(P::RParen) => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    Token::Eof => return Ok(None),
                    _ => {}
                }
                i += 1;
            }
            let after = &self.toks.get(i + 1);
            let arrow = after.map(|l| l.tok.is_punct(P::Arrow)).unwrap_or(false);
            if !arrow {
                return Ok(None);
            }
            let save = self.pos;
            let params = match self.parse_params() {
                Ok(p) => p,
                Err(e) => {
                    self.pos = save;
                    return Err(e);
                }
            };
            if !self.eat(P::Arrow) {
                self.pos = save;
                return Ok(None);
            }
            let body = self.parse_arrow_body()?;
            return Ok(Some(expr(
                Node::Arrow {
                    params,
                    body,
                    is_async,
                },
                line,
            )));
        }
        // `x => …`
        if let Token::Ident(name) = self.cur().clone() {
            if self.peek(1).is_punct(P::Arrow) && !self.peek_lex(1).nl {
                self.bump();
                self.bump();
                let body = self.parse_arrow_body()?;
                return Ok(Some(expr(
                    Node::Arrow {
                        params: vec![Param {
                            pattern: Pattern::Binding(Rc::from(name)),
                            default: None,
                            rest: false,
                        }],
                        body,
                        is_async,
                    },
                    line,
                )));
            }
        }
        let _ = idx;
        Ok(None)
    }

    fn parse_arrow_body(&mut self) -> PR<FnBody> {
        if self.at_p(P::LBrace) {
            Ok(FnBody::Block(self.parse_block()?))
        } else {
            Ok(FnBody::Expr(self.parse_assignment()?))
        }
    }

    fn parse_conditional(&mut self) -> PR<E> {
        let line = self.line();
        let test = self.parse_nullish()?;
        if self.eat(P::Question) {
            let then = self.parse_assignment()?;
            self.expect(P::Colon)?;
            let els = self.parse_assignment()?;
            return Ok(expr(Node::Cond { test, then, els }, line));
        }
        Ok(test)
    }

    fn parse_nullish(&mut self) -> PR<E> {
        let line = self.line();
        let mut lhs = self.parse_logical_or()?;
        while self.at_p(P::Nullish) {
            self.bump();
            let rhs = self.parse_logical_or()?;
            lhs = expr(
                Node::Logical {
                    op: LogicOp::Nullish,
                    lhs,
                    rhs,
                },
                line,
            );
        }
        Ok(lhs)
    }

    fn parse_logical_or(&mut self) -> PR<E> {
        let line = self.line();
        let mut lhs = self.parse_logical_and()?;
        while self.at_p(P::OrOr) {
            self.bump();
            let rhs = self.parse_logical_and()?;
            lhs = expr(
                Node::Logical {
                    op: LogicOp::Or,
                    lhs,
                    rhs,
                },
                line,
            );
        }
        Ok(lhs)
    }

    fn parse_logical_and(&mut self) -> PR<E> {
        let line = self.line();
        let mut lhs = self.parse_bitwise()?;
        while self.at_p(P::AndAnd) {
            self.bump();
            let rhs = self.parse_bitwise()?;
            lhs = expr(
                Node::Logical {
                    op: LogicOp::And,
                    lhs,
                    rhs,
                },
                line,
            );
        }
        Ok(lhs)
    }

    fn parse_bitwise(&mut self) -> PR<E> {
        let line = self.line();
        let mut lhs = self.parse_equality()?;
        loop {
            let op = match self.cur() {
                Token::Punct(P::Amp) => BinOp::BitAnd,
                Token::Punct(P::Pipe) => BinOp::BitOr,
                Token::Punct(P::Caret) => BinOp::BitXor,
                _ => break,
            };
            self.bump();
            let rhs = self.parse_equality()?;
            lhs = expr(Node::Binary { op, lhs, rhs }, line);
        }
        Ok(lhs)
    }

    fn parse_equality(&mut self) -> PR<E> {
        let line = self.line();
        let mut lhs = self.parse_relational()?;
        loop {
            let op = match self.cur() {
                Token::Punct(P::EqEq) => BinOp::Eq,
                Token::Punct(P::NotEq) => BinOp::Ne,
                Token::Punct(P::EqEqEq) => BinOp::StrictEq,
                Token::Punct(P::NotEqEq) => BinOp::StrictNe,
                _ => break,
            };
            self.bump();
            let rhs = self.parse_relational()?;
            lhs = expr(Node::Binary { op, lhs, rhs }, line);
        }
        Ok(lhs)
    }

    fn parse_relational(&mut self) -> PR<E> {
        let line = self.line();
        let mut lhs = self.parse_shift()?;
        loop {
            let op = match self.cur() {
                Token::Punct(P::Lt) => BinOp::Lt,
                Token::Punct(P::Gt) => BinOp::Gt,
                Token::Punct(P::Le) => BinOp::Le,
                Token::Punct(P::Ge) => BinOp::Ge,
                Token::Punct(P::Instanceof) => BinOp::Instanceof,
                Token::Punct(P::In) => BinOp::In,
                _ => break,
            };
            self.bump();
            let rhs = self.parse_shift()?;
            lhs = expr(Node::Binary { op, lhs, rhs }, line);
        }
        Ok(lhs)
    }

    fn parse_shift(&mut self) -> PR<E> {
        let line = self.line();
        let mut lhs = self.parse_additive()?;
        loop {
            let op = match self.cur() {
                Token::Punct(P::ShiftLeft) => BinOp::Shl,
                Token::Punct(P::ShiftRight) => BinOp::Shr,
                Token::Punct(P::UShiftRight) => BinOp::UShr,
                _ => break,
            };
            self.bump();
            let rhs = self.parse_additive()?;
            lhs = expr(Node::Binary { op, lhs, rhs }, line);
        }
        Ok(lhs)
    }

    fn parse_additive(&mut self) -> PR<E> {
        let line = self.line();
        let mut lhs = self.parse_multiplicative()?;
        loop {
            let op = match self.cur() {
                Token::Punct(P::Plus) => BinOp::Add,
                Token::Punct(P::Minus) => BinOp::Sub,
                _ => break,
            };
            self.bump();
            let rhs = self.parse_multiplicative()?;
            lhs = expr(Node::Binary { op, lhs, rhs }, line);
        }
        Ok(lhs)
    }

    fn parse_multiplicative(&mut self) -> PR<E> {
        let line = self.line();
        let mut lhs = self.parse_exponent()?;
        loop {
            let op = match self.cur() {
                Token::Punct(P::Star) => BinOp::Mul,
                Token::Punct(P::Slash) => BinOp::Div,
                Token::Punct(P::Percent) => BinOp::Mod,
                _ => break,
            };
            self.bump();
            let rhs = self.parse_exponent()?;
            lhs = expr(Node::Binary { op, lhs, rhs }, line);
        }
        Ok(lhs)
    }

    fn parse_exponent(&mut self) -> PR<E> {
        let line = self.line();
        let lhs = self.parse_unary()?;
        if self.at_p(P::StarStar) {
            self.bump();
            let rhs = self.parse_exponent()?;
            return Ok(expr(
                Node::Binary {
                    op: BinOp::Pow,
                    lhs,
                    rhs,
                },
                line,
            ));
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> PR<E> {
        let line = self.line();
        let op = match self.cur() {
            Token::Punct(P::Minus) => Some(UnOp::Neg),
            Token::Punct(P::Plus) => Some(UnOp::Pos),
            Token::Punct(P::Bang) => Some(UnOp::Not),
            Token::Punct(P::Tilde) => Some(UnOp::BitNot),
            Token::Punct(P::Typeof) => Some(UnOp::Typeof),
            Token::Punct(P::Void) => Some(UnOp::Void),
            Token::Punct(P::Delete) => Some(UnOp::Delete),
            _ => None,
        };
        if let Some(op) = op {
            self.bump();
            let arg = self.parse_unary()?;
            // `**` binds tighter than a unary operator, so `-2 ** 2` is
            // `- (2 ** 2)`. A real engine rejects that spelling outright; we
            // accept it with the mathematical reading instead.
            if self.at_p(P::StarStar) {
                self.bump();
                let rhs = self.parse_exponent()?;
                let pow = expr(
                    Node::Binary {
                        op: BinOp::Pow,
                        lhs: arg,
                        rhs,
                    },
                    line,
                );
                return Ok(expr(Node::Unary { op, arg: pow }, line));
            }
            return Ok(expr(Node::Unary { op, arg }, line));
        }
        if self.at_p(P::Await) {
            self.bump();
            let arg = self.parse_unary()?;
            return Ok(expr(Node::Await(arg), line));
        }
        if self.at_p(P::PlusPlus) || self.at_p(P::MinusMinus) {
            let op = if self.at_p(P::PlusPlus) {
                UpdateOp::Inc
            } else {
                UpdateOp::Dec
            };
            self.bump();
            let target = self.parse_unary()?;
            return Ok(expr(
                Node::Update {
                    op,
                    target,
                    prefix: true,
                },
                line,
            ));
        }
        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> PR<E> {
        let line = self.line();
        let target = self.parse_call_chain()?;
        if (self.at_p(P::PlusPlus) || self.at_p(P::MinusMinus)) && !self.peek_lex(0).nl {
            let op = if self.at_p(P::PlusPlus) {
                UpdateOp::Inc
            } else {
                UpdateOp::Dec
            };
            self.bump();
            return Ok(expr(
                Node::Update {
                    op,
                    target,
                    prefix: false,
                },
                line,
            ));
        }
        Ok(target)
    }

    fn parse_call_chain(&mut self) -> PR<E> {
        let mut e = self.parse_primary()?;
        loop {
            e = match self.cur() {
                Token::Punct(P::Dot) => {
                    self.bump();
                    let name = self.property_name()?;
                    expr(
                        Node::Member {
                            obj: e,
                            prop: MemberProp::Name(name),
                            optional: false,
                        },
                        self.line(),
                    )
                }
                Token::Punct(P::QuestionDot) => {
                    self.bump();
                    if self.at_p(P::LParen) {
                        let args = self.parse_args()?;
                        expr(
                            Node::Call {
                                callee: e,
                                args,
                                optional: true,
                            },
                            self.line(),
                        )
                    } else if self.at_p(P::LBracket) {
                        self.bump();
                        let prop = self.parse_expression()?;
                        self.expect(P::RBracket)?;
                        expr(
                            Node::Member {
                                obj: e,
                                prop: MemberProp::Expr(prop),
                                optional: true,
                            },
                            self.line(),
                        )
                    } else {
                        let name = self.property_name()?;
                        expr(
                            Node::Member {
                                obj: e,
                                prop: MemberProp::Name(name),
                                optional: true,
                            },
                            self.line(),
                        )
                    }
                }
                Token::Punct(P::LBracket) => {
                    self.bump();
                    let prop = self.parse_expression()?;
                    self.expect(P::RBracket)?;
                    expr(
                        Node::Member {
                            obj: e,
                            prop: MemberProp::Expr(prop),
                            optional: false,
                        },
                        self.line(),
                    )
                }
                Token::Punct(P::LParen) => {
                    let args = self.parse_args()?;
                    expr(
                        Node::Call {
                            callee: e,
                            args,
                            optional: false,
                        },
                        self.line(),
                    )
                }
                Token::Template(_) => {
                    let (chunks, exprs) = self.take_template()?;
                    expr(
                        Node::Tagged {
                            tag: e,
                            chunks,
                            exprs,
                        },
                        self.line(),
                    )
                }
                _ => break,
            };
        }
        Ok(e)
    }

    /// Everything after a callee that already parsed: `(...)` chains, used for
    /// `class A extends B()`.
    fn parse_call_tail(&mut self, callee: E) -> PR<E> {
        let line = self.line();
        let args = self.parse_args()?;
        Ok(expr(
            Node::Call {
                callee,
                args,
                optional: false,
            },
            line,
        ))
    }

    fn property_name(&mut self) -> PR<Rc<str>> {
        match self.cur().clone() {
            Token::Ident(s) => {
                self.bump();
                Ok(Rc::from(s))
            }
            Token::Punct(p) => {
                if let Some(kw) = keywordish(p) {
                    self.bump();
                    return Ok(Rc::from(kw));
                }
                if let Some(kw) = p.keyword() {
                    self.bump();
                    return Ok(Rc::from(kw));
                }
                Err(ParseError::new(
                    format!("expected property name, found `{p}`"),
                    self.line(),
                ))
            }
            Token::String(s) => {
                self.bump();
                Ok(Rc::from(s))
            }
            Token::Number(n) => {
                self.bump();
                Ok(Rc::from(format!("{n}")))
            }
            _ => Err(ParseError::new(
                format!("expected property name, found `{}`", self.describe()),
                self.line(),
            )),
        }
    }

    fn parse_args(&mut self) -> PR<Vec<Arg>> {
        self.expect(P::LParen)?;
        let mut args = Vec::new();
        while !self.at_p(P::RParen) {
            if self.at(Token::Eof) {
                return Err(ParseError::new("unterminated argument list", self.line()));
            }
            if self.eat(P::Ellipsis) {
                args.push(Arg::Spread(self.parse_assignment()?));
            } else {
                args.push(Arg::Single(self.parse_assignment()?));
            }
            if !self.eat(P::Comma) {
                break;
            }
        }
        self.expect(P::RParen)?;
        Ok(args)
    }

    fn parse_primary(&mut self) -> PR<E> {
        let _deep = self.enter()?;
        let line = self.line();
        match self.cur().clone() {
            Token::Number(n) => {
                self.bump();
                Ok(expr(Node::Num(n), line))
            }
            Token::String(s) => {
                self.bump();
                Ok(expr(Node::Str(Rc::from(s)), line))
            }
            Token::Template(_) => {
                let (chunks, exprs) = self.take_template()?;
                Ok(expr(Node::Template { chunks, exprs }, line))
            }
            Token::Regex(body, flags) => {
                self.bump();
                Ok(expr(Node::Regex(Rc::from(body), Rc::from(flags)), line))
            }
            Token::Ident(name) => {
                self.bump();
                Ok(expr(Node::Ident(Rc::from(name)), line))
            }
            Token::Punct(p) => match p {
                P::True => {
                    self.bump();
                    Ok(expr(Node::Bool(true), line))
                }
                P::False => {
                    self.bump();
                    Ok(expr(Node::Bool(false), line))
                }
                P::Null => {
                    self.bump();
                    Ok(expr(Node::Null, line))
                }
                P::Undefined => {
                    self.bump();
                    Ok(expr(Node::Undefined, line))
                }
                P::This => {
                    self.bump();
                    Ok(expr(Node::This, line))
                }
                P::Super => {
                    self.bump();
                    Ok(expr(Node::Super, line))
                }
                P::New => {
                    self.bump();
                    if self.at_p(P::Dot) {
                        self.bump();
                        self.property_name()?;
                        return Ok(expr(Node::NewTarget, line));
                    }
                    let callee = self.parse_unary_no_call()?;
                    let args = if self.at_p(P::LParen) {
                        self.parse_args()?
                    } else {
                        Vec::new()
                    };
                    Ok(expr(Node::New { callee, args }, line))
                }
                P::LParen => {
                    self.bump();
                    let e = self.parse_expression()?;
                    self.expect(P::RParen)?;
                    Ok(e)
                }
                P::LBracket => {
                    self.bump();
                    let mut items: Vec<Option<E>> = Vec::new();
                    while !self.at_p(P::RBracket) {
                        if self.at(Token::Eof) {
                            return Err(ParseError::new("unterminated array literal", line));
                        }
                        if self.at_p(P::Comma) {
                            self.bump();
                            items.push(None);
                            continue;
                        }
                        let item = if self.eat(P::Ellipsis) {
                            Some(expr(Node::Spread(self.parse_assignment()?), self.line()))
                        } else {
                            Some(self.parse_assignment()?)
                        };
                        items.push(item);
                        if !self.eat(P::Comma) {
                            break;
                        }
                    }
                    self.expect(P::RBracket)?;
                    Ok(expr(Node::Array(items), line))
                }
                P::LBrace => self.parse_object_literal(),
                P::Function => {
                    let f = self.parse_function(false, false)?;
                    Ok(expr(Node::Fn(f), line))
                }
                P::Async => {
                    if self.peek(1).is_punct(P::Function) {
                        self.bump();
                        let f = self.parse_function(true, false)?;
                        return Ok(expr(Node::Fn(f), line));
                    }
                    self.bump();
                    Ok(expr(Node::Ident(Rc::from("async")), line))
                }
                P::Class => {
                    let c = self.parse_class()?;
                    Ok(expr(Node::Class(c), line))
                }
                P::Star => {
                    // `function*` handled above; a bare `*` here is a syntax error
                    Err(ParseError::new("unexpected `*`", line))
                }
                _ => Err(ParseError::new(format!("unexpected `{p}`"), line)),
            },
            Token::Eof => Err(ParseError::new("unexpected end of input", line)),
        }
    }

    /// `new X` without a trailing call chain (`new a.b.c()` binds the member).
    fn parse_unary_no_call(&mut self) -> PR<E> {
        let mut e = self.parse_primary()?;
        loop {
            match self.cur() {
                Token::Punct(P::Dot) => {
                    self.bump();
                    let name = self.property_name()?;
                    e = expr(
                        Node::Member {
                            obj: e,
                            prop: MemberProp::Name(name),
                            optional: false,
                        },
                        self.line(),
                    );
                }
                Token::Punct(P::LBracket) => {
                    self.bump();
                    let prop = self.parse_expression()?;
                    self.expect(P::RBracket)?;
                    e = expr(
                        Node::Member {
                            obj: e,
                            prop: MemberProp::Expr(prop),
                            optional: false,
                        },
                        self.line(),
                    );
                }
                _ => break,
            }
        }
        Ok(e)
    }

    fn parse_object_literal(&mut self) -> PR<E> {
        let line = self.line();
        self.expect(P::LBrace)?;
        let mut props: Vec<Prop> = Vec::new();
        while !self.at_p(P::RBrace) {
            if self.at(Token::Eof) {
                return Err(ParseError::new("unterminated object literal", line));
            }
            // spread
            if self.eat(P::Ellipsis) {
                props.push(Prop::Spread(self.parse_assignment()?));
                if !self.eat(P::Comma) {
                    break;
                }
                continue;
            }
            // getter / setter — `get() {}` is a *method* called "get"
            if (self.at_p(P::Get) || self.at_p(P::Set))
                && !matches!(
                    self.peek(1),
                    Token::Punct(P::Colon)
                        | Token::Punct(P::Comma)
                        | Token::Punct(P::RParen)
                        | Token::Punct(P::RBrace)
                        | Token::Punct(P::LParen)
                        | Token::Punct(P::Assign)
                )
            {
                let is_get = self.at_p(P::Get);
                self.bump();
                let (key, _) = self.parse_property_key()?;
                let f = self.parse_method_body(Some(&key))?;
                let existing = props
                    .iter_mut()
                    .find(|p| matches!(p, Prop::Accessor{key: k, ..} if key_eq(k, &key)));
                if let Some(Prop::Accessor { getter, setter, .. }) = existing {
                    if is_get {
                        *getter = Some(f);
                    } else {
                        *setter = Some(f);
                    }
                    if !self.eat(P::Comma) {
                        break;
                    }
                    continue;
                }
                let f2 = f.clone();
                props.push(Prop::Accessor {
                    key,
                    getter: is_get.then_some(f),
                    setter: if is_get { None } else { Some(f2) },
                });
                if !self.eat(P::Comma) {
                    break;
                }
                continue;
            }
            let is_async = self.at_p(P::Async)
                && matches!(
                    self.peek(1),
                    Token::Punct(P::LParen) | Token::Punct(P::LBracket) | Token::Punct(P::Star)
                );
            if is_async {
                self.bump();
            }
            let is_gen = self.eat(P::Star);
            let (key, _) = self.parse_property_key()?;
            if self.at_p(P::LParen) {
                let f = self.parse_method_body(Some(&key))?;
                let f = Rc::new(Function {
                    is_async: is_async || f.is_async,
                    is_generator: is_gen || f.is_generator,
                    ..(&*f).clone()
                });
                props.push(Prop::Method { key, func: f });
                if !self.eat(P::Comma) {
                    break;
                }
                continue;
            }
            if self.eat(P::Colon) {
                let value = self.parse_assignment()?;
                props.push(Prop::Value { key, value });
                if !self.eat(P::Comma) {
                    break;
                }
                continue;
            }
            // shorthand: `{ a }` or `{ a = 1 }` (default in destructuring)
            match &key {
                Key::Name(n) => {
                    if self.eat(P::Assign) {
                        let value = self.parse_assignment()?;
                        props.push(Prop::Value {
                            key: key.clone(),
                            value: expr(
                                Node::Assign {
                                    target: expr(Node::Ident(n.clone()), self.line()),
                                    op: Some(AssignOp::Assign),
                                    value,
                                },
                                self.line(),
                            ),
                        });
                    } else {
                        props.push(Prop::Shorthand(n.clone()));
                    }
                    if !self.eat(P::Comma) {
                        break;
                    }
                }
                _ => {
                    return Err(ParseError::new(
                        "expected `:` in object literal",
                        self.line(),
                    ))
                }
            }
        }
        self.expect(P::RBrace)?;
        Ok(expr(Node::Object(props), line))
    }

    fn parse_method_body(&mut self, key: Option<&Key>) -> PR<Rc<Function>> {
        let line = self.line();
        let params = self.parse_params()?;
        let body = self.parse_block()?;
        Ok(Rc::new(Function {
            name: match key {
                Some(Key::Name(n)) => Some(n.clone()),
                _ => None,
            },
            params,
            body,
            is_async: false,
            is_generator: false,
            is_method: true,
            is_arrow: false,
            line,
        }))
    }

    /// Split a raw template literal into text chunks + interpolated expressions.
    fn take_template(&mut self) -> PR<(Vec<Rc<str>>, Vec<E>)> {
        let Token::Template(raw) = self.cur().clone() else {
            return Err(ParseError::new("expected template literal", self.line()));
        };
        self.bump();
        let mut chunks: Vec<Rc<str>> = Vec::new();
        let mut exprs: Vec<E> = Vec::new();
        let mut cur = String::new();
        let bytes: Vec<char> = raw.chars().collect();
        let mut i = 0usize;
        while i < bytes.len() {
            let c = bytes[i];
            if c == '\\' && i + 1 < bytes.len() {
                // keep escapes simple but correct for the usual cases
                let n = bytes[i + 1];
                match n {
                    'n' => cur.push('\n'),
                    't' => cur.push('\t'),
                    'r' => cur.push('\r'),
                    '`' => cur.push('`'),
                    '$' => cur.push('$'),
                    '\\' => cur.push('\\'),
                    'u' if i + 2 < bytes.len() && bytes[i + 2] == '{' => {
                        let mut j = i + 3;
                        let mut hex = String::new();
                        while j < bytes.len() && bytes[j] != '}' {
                            hex.push(bytes[j]);
                            j += 1;
                        }
                        let cp = u32::from_str_radix(&hex, 16).unwrap_or(0xfffd);
                        cur.push(char::from_u32(cp).unwrap_or('\u{fffd}'));
                        i = j + 1;
                        continue;
                    }
                    other => cur.push(other),
                }
                i += 2;
                continue;
            }
            if c == '$' && i + 1 < bytes.len() && bytes[i + 1] == '{' {
                // collect until the matching `}` (strings and templates counted)
                let mut depth = 1usize;
                let mut src = String::new();
                let mut j = i + 2;
                let mut quote: Option<char> = None;
                while j < bytes.len() {
                    let d = bytes[j];
                    match quote {
                        Some(q) => {
                            if d == '\\' && j + 1 < bytes.len() {
                                src.push(d);
                                src.push(bytes[j + 1]);
                                j += 2;
                                continue;
                            }
                            if d == q {
                                quote = None;
                            }
                            src.push(d);
                        }
                        None => match d {
                            '\'' | '"' | '`' => {
                                quote = Some(d);
                                src.push(d);
                            }
                            '{' => {
                                depth += 1;
                                src.push(d);
                            }
                            '}' => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                                src.push(d);
                            }
                            _ => src.push(d),
                        },
                    }
                    j += 1;
                }
                chunks.push(Rc::from(std::mem::take(&mut cur)));
                let sub = Parser::parse_program(&(src + "\n0"))?;
                // parse_program wraps in statements; take the first expression
                let e = expr_from_program(sub, self.line())?;
                exprs.push(e);
                i = j + 1;
                continue;
            }
            cur.push(c);
            i += 1;
        }
        chunks.push(Rc::from(cur));
        Ok((chunks, exprs))
    }
}

fn expr_from_program(p: Program, line: u32) -> PR<E> {
    if p.body.is_empty() {
        return Ok(expr(Node::Undefined, line));
    }
    match &p.body[0].kind {
        StmtKind::Expr(e) => Ok(e.clone()),
        _ => Err(ParseError::new("bad template interpolation", line)),
    }
}

fn key_eq(a: &Key, b: &Key) -> bool {
    match (a, b) {
        (Key::Name(x), Key::Name(y)) => x == y,
        (Key::Num(x), Key::Num(y)) => x == y,
        _ => false,
    }
}

fn assign_op(t: &Token) -> Option<AssignOp> {
    Some(match t {
        Token::Punct(P::Assign) => AssignOp::Assign,
        Token::Punct(P::PlusAssign) => AssignOp::Add,
        Token::Punct(P::MinusAssign) => AssignOp::Sub,
        Token::Punct(P::MulAssign) => AssignOp::Mul,
        Token::Punct(P::DivAssign) => AssignOp::Div,
        Token::Punct(P::ModAssign) => AssignOp::Mod,
        Token::Punct(P::PowAssign) => AssignOp::Pow,
        Token::Punct(P::AndAndAssign) => AssignOp::And,
        Token::Punct(P::OrOrAssign) => AssignOp::Or,
        Token::Punct(P::NullishAssign) => AssignOp::Nullish,
        Token::Punct(P::AndAssign) => AssignOp::BitAnd,
        Token::Punct(P::OrAssign) => AssignOp::BitOr,
        Token::Punct(P::XorAssign) => AssignOp::BitXor,
        Token::Punct(P::ShlAssign) => AssignOp::Shl,
        Token::Punct(P::ShrAssign) => AssignOp::Shr,
        Token::Punct(P::UShrAssign) => AssignOp::UShr,
        _ => return None,
    })
}

/// Parse a script; on syntax error return the error (callers log and skip).
pub fn parse(src: &str) -> Result<Program, ParseError> {
    Parser::parse_program(src)
}

/// Words that are keywords in some positions but fine as property names.
fn keywordish(p: P) -> Option<&'static str> {
    Some(match p {
        P::Get => "get",
        P::Set => "set",
        P::Static => "static",
        P::Async => "async",
        P::Of => "of",
        P::Let => "let",
        _ => return None,
    })
}

/// `true` for reserved words that can never be labels.
trait KeywordShaped {
    fn from_keyword_shaped(s: &str) -> bool;
}

impl KeywordShaped for P {
    fn from_keyword_shaped(s: &str) -> bool {
        matches!(
            s,
            "if" | "for"
                | "while"
                | "do"
                | "switch"
                | "return"
                | "throw"
                | "try"
                | "var"
                | "const"
                | "break"
                | "continue"
                | "function"
                | "class"
        )
    }
}

/// Turn `[a, b] = c` / `{a} = c` style expressions into a binding pattern.
fn to_pattern_expr(e: E) -> PR<Pattern> {
    Ok(match &e.node {
        Node::Ident(name) => Pattern::Binding(name.clone()),
        Node::Array(items) => {
            let mut out = Vec::new();
            for it in items {
                match it {
                    None => out.push(ArrayEl::Hole),
                    Some(x) => match &x.node {
                        Node::Spread(inner) => match &inner.node {
                            Node::Ident(n) => out.push(ArrayEl::Rest(n.clone())),
                            _ => return Err(ParseError::new("bad rest target", x.line)),
                        },
                        other => out.push(ArrayEl::Item(
                            to_pattern_expr(Rc::new(Expr {
                                node: other.clone(),
                                line: x.line,
                            }))?,
                            None,
                        )),
                    },
                }
            }
            Pattern::Array(out)
        }
        Node::Object(props) => {
            let mut out = Vec::new();
            for p in props {
                match p {
                    Prop::Shorthand(name) => out.push(ObjectEl {
                        key: Key::Name(name.clone()),
                        pattern: Pattern::Binding(name.clone()),
                        default: None,
                        rest: false,
                    }),
                    Prop::Value { key, value } => {
                        let inner = match &value.node {
                            Node::Assign {
                                target: _,
                                op: None,
                                value,
                            } => value.clone(),
                            _ => value.clone(),
                        };
                        out.push(ObjectEl {
                            key: key.clone(),
                            pattern: to_pattern_expr(inner)?,
                            default: None,
                            rest: false,
                        })
                    }
                    Prop::Spread(e) => match &e.node {
                        Node::Ident(n) => out.push(ObjectEl {
                            key: Key::Name(Rc::from("")),
                            pattern: Pattern::Binding(n.clone()),
                            default: None,
                            rest: true,
                        }),
                        _ => return Err(ParseError::new("bad rest target", e.line)),
                    },
                    _ => return Err(ParseError::new("bad destructuring target", e.line)),
                }
            }
            Pattern::Object(out)
        }
        // member expressions (`obj.x = …`) are handled by the interpreter
        _ => return Err(ParseError::new("invalid destructuring target", e.line)),
    })
}
