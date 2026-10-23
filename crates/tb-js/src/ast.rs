//! JavaScript AST.
//!
//! Deliberately close to the shape of the source: the interpreter walks this
//! directly (a tree-walking evaluator), so "close to the source" also means
//! "easy to get right". Everything is behind `Rc` so clones are cheap — the
//! interpreter clones `Rc<Expr>` constantly when building closures and loops.

use std::cell::RefCell;
use std::rc::Rc;

pub type E = Rc<Expr>;
pub type S = Rc<Stmt>;

#[derive(Debug)]
pub struct Expr {
    pub node: Node,
    pub line: u32,
}

#[derive(Debug, Clone)]
pub enum Node {
    Num(f64),
    Str(Rc<str>),
    Bool(bool),
    Null,
    Undefined,
    /// `this`
    This,
    /// `super` inside a method (calls the parent constructor / prototype)
    Super,
    NewTarget,
    Ident(Rc<str>),
    /// `[a, , b]` — `None` is a hole
    Array(Vec<Option<E>>),
    Object(Vec<Prop>),
    Template {
        /// literal text chunks; `parts.len() == exprs.len() + 1`
        chunks: Vec<Rc<str>>,
        exprs: Vec<E>,
    },
    /// `` tag`...` `` — a tagged template
    Tagged {
        tag: E,
        chunks: Vec<Rc<str>>,
        exprs: Vec<E>,
    },
    Regex(Rc<str>, Rc<str>),
    Member {
        obj: E,
        prop: MemberProp,
        optional: bool,
    },
    Call {
        callee: E,
        args: Vec<Arg>,
        optional: bool,
    },
    New {
        callee: E,
        args: Vec<Arg>,
    },
    /// `import(...)` — a promise for a module (we resolve to an error object)
    Import(E),
    Unary {
        op: UnOp,
        arg: E,
    },
    Update {
        op: UpdateOp,
        target: E,
        prefix: bool,
    },
    Binary {
        op: BinOp,
        lhs: E,
        rhs: E,
    },
    Logical {
        op: LogicOp,
        lhs: E,
        rhs: E,
    },
    Assign {
        target: E,
        op: Option<AssignOp>,
        value: E,
    },
    Cond {
        test: E,
        then: E,
        els: E,
    },
    /// `a, b, c`
    Seq(Vec<E>),
    /// `...x` inside calls/arrays (also used for spread in literals)
    Spread(E),
    Fn(Rc<Function>),
    Arrow {
        params: Vec<Param>,
        body: FnBody,
        is_async: bool,
    },
    Class(Rc<ClassDecl>),
    Await(E),
    /// `new Promise(...)`-style helper for `yield`-free generators we treat as functions
    Yield(E),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Pos,
    Not,
    BitNot,
    Typeof,
    Void,
    Delete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateOp {
    Inc,
    Dec,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Lt,
    Gt,
    Le,
    Ge,
    Eq,
    Ne,
    StrictEq,
    StrictNe,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    UShr,
    In,
    Instanceof,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogicOp {
    And,
    Or,
    Nullish,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignOp {
    Assign,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    And,
    Or,
    Nullish,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    UShr,
}

#[derive(Debug, Clone)]
pub enum MemberProp {
    Name(Rc<str>),
    Expr(E),
}

#[derive(Debug, Clone)]
pub enum Arg {
    Single(E),
    Spread(E),
}

/// Object literal property.
#[derive(Debug, Clone)]
pub enum Prop {
    /// `{ a }`
    Shorthand(Rc<str>),
    /// `{ a: 1 }`, `{ "a": 1 }`, `{ 1: 1 }`, `{ [k]: 1 }`
    Value { key: Key, value: E },
    /// `{ a() {} }`, `{ async a() {} }`, `{ *gen() {} }`
    Method { key: Key, func: Rc<Function> },
    /// `{ get a() {}, set a(v) {} }`
    Accessor {
        key: Key,
        getter: Option<Rc<Function>>,
        setter: Option<Rc<Function>>,
    },
    /// `{ ...other }`
    Spread(E),
}

#[derive(Debug, Clone)]
pub enum Key {
    Name(Rc<str>),
    Num(f64),
    Computed(E),
}

#[derive(Debug, Clone)]
pub struct Function {
    pub name: Option<Rc<str>>,
    pub params: Vec<Param>,
    pub body: Rc<Block>,
    pub is_async: bool,
    pub is_generator: bool,
    pub is_method: bool,
    /// arrow functions inherit `this` from where they were created
    pub is_arrow: bool,
    pub line: u32,
}

#[derive(Debug, Clone)]
pub enum FnBody {
    /// `(a) => expr`
    Expr(E),
    /// `(a) => { ... }`
    Block(Rc<Block>),
}

#[derive(Debug, Clone)]
pub struct Param {
    pub pattern: Pattern,
    /// default value
    pub default: Option<E>,
    /// `...rest`
    pub rest: bool,
}

/// Destructuring pattern (also plain bindings).
#[derive(Debug, Clone)]
pub enum Pattern {
    Binding(Rc<str>),
    Array(Vec<ArrayEl>),
    Object(Vec<ObjectEl>),
}

#[derive(Debug, Clone)]
pub enum ArrayEl {
    Item(Pattern, Option<E>),
    /// `...rest`
    Rest(Rc<str>),
    /// hole
    Hole,
}

#[derive(Debug, Clone)]
pub struct ObjectEl {
    pub key: Key,
    pub pattern: Pattern,
    pub default: Option<E>,
    /// `...rest`
    pub rest: bool,
}

#[derive(Debug, Clone)]
pub struct ClassDecl {
    pub name: Option<Rc<str>>,
    pub super_class: Option<E>,
    pub members: Vec<Member>,
    pub line: u32,
}

impl ClassDecl {
    /// The `constructor` member, if declared.
    pub fn ctor(&self) -> Option<&Rc<Function>> {
        self.members.iter().find_map(|m| match &m.kind {
            MemberKind::Method(f) if matches!(&m.key, Key::Name(n) if &**n == "constructor") => {
                Some(f)
            }
            _ => None,
        })
    }
}

#[derive(Debug, Clone)]
pub struct Member {
    pub key: Key,
    pub kind: MemberKind,
    pub is_static: bool,
}

#[derive(Debug, Clone)]
pub enum MemberKind {
    Method(Rc<Function>),
    Getter(Rc<Function>),
    Setter(Rc<Function>),
    /// class field: `x = 1` (undefined when no initialiser)
    Field(Option<E>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VarKind {
    Var,
    Let,
    Const,
}

#[derive(Debug, Clone)]
pub struct Declarator {
    pub pattern: Pattern,
    pub init: Option<E>,
}

#[derive(Debug, Clone)]
pub struct Stmt {
    pub kind: StmtKind,
    pub line: u32,
}

#[derive(Debug, Clone)]
pub enum ForInit {
    Expr(E),
    /// `for ([a, b] of pairs)` / `for ({x} of ys)`
    Pattern(Pattern),
    Decl(VarKind, Vec<Declarator>),
}

#[derive(Debug, Clone)]
pub enum StmtKind {
    Decl {
        kind: VarKind,
        decls: Vec<Declarator>,
    },
    Expr(E),
    Block(Rc<Block>),
    If {
        test: E,
        cons: S,
        els: Option<S>,
    },
    For {
        init: Option<ForInit>,
        test: Option<E>,
        update: Option<E>,
        body: S,
    },
    ForIn {
        left: ForInit,
        obj: E,
        body: S,
    },
    ForOf {
        left: ForInit,
        obj: E,
        body: S,
        await_: bool,
    },
    While {
        test: E,
        body: S,
    },
    Do {
        body: S,
        test: E,
    },
    Switch {
        disc: E,
        cases: Vec<Case>,
    },
    Return(Option<E>),
    Break(Option<Rc<str>>),
    Continue(Option<Rc<str>>),
    Throw(E),
    Try {
        block: Rc<Block>,
        catch: Option<Catch>,
        finalizer: Option<Rc<Block>>,
    },
    Fn(Rc<Function>),
    Class(Rc<ClassDecl>),
    Labeled(Rc<str>, S),
    Empty,
    Debugger,
}

#[derive(Debug, Clone)]
pub struct Case {
    /// `None` = `default:`
    pub test: Option<E>,
    pub body: Vec<S>,
}

#[derive(Debug, Clone)]
pub struct Catch {
    pub param: Option<Pattern>,
    pub body: Rc<Block>,
}

#[derive(Debug, Clone, Default)]
pub struct Block {
    pub body: Vec<S>,
}

/// A parsed script.
#[derive(Debug, Clone, Default)]
pub struct Program {
    pub body: Vec<S>,
    /// `true` when the script starts with `'use strict'`
    pub strict: bool,
}

impl Program {
    /// Top-level `function` declarations, hoisted before execution.
    pub fn hoisted_functions(&self) -> Vec<Rc<Function>> {
        let mut out = Vec::new();
        collect_fns(&self.body, &mut out);
        out
    }
}

fn collect_fns(body: &[S], out: &mut Vec<Rc<Function>>) {
    for s in body {
        if let StmtKind::Fn(f) = &s.kind {
            out.push(f.clone());
        }
    }
}

/// Convenience constructors used by the parser.
pub fn expr(node: Node, line: u32) -> E {
    Rc::new(Expr { node, line })
}

pub fn stmt(kind: StmtKind, line: u32) -> S {
    Rc::new(Stmt { kind, line })
}

/// Shared empty block (cheap).
pub fn empty_block() -> Rc<Block> {
    thread_local! {
        static B: RefCell<Rc<Block>> = RefCell::new(Rc::new(Block::default()));
    }
    B.with(|b| b.borrow().clone())
}
