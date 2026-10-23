//! Scope chain: environments.
//!
//! `var` lives in the nearest *function* (or global) scope, `let`/`const` in the
//! nearest block scope — which is why block scopes exist at all. Lookups walk
//! the chain; assignments find and update the owning scope, and undeclared
//! writes become globals (sloppy mode) instead of failing.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use super::value::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeKind {
    Global,
    Function,
    Block,
}

pub struct Env {
    pub kind: ScopeKind,
    pub vars: HashMap<Rc<str>, Value>,
    pub parent: Option<EnvRef>,
    /// `this` for the current function (arrow functions read their parent's).
    pub this: Option<Value>,
    /// method definition object — what `super.x` looks at
    pub home: Option<Value>,
    /// `new.target` inside a constructor
    pub new_target: Option<Value>,
    /// set when inside an async function so `await` knows it may suspend
    pub is_async: bool,
    /// inside a generator/class method where `super()` is legal
    pub derived_constructor: bool,
    /// class whose field initialisers belong to the current constructor, so
    /// `super()` can run them once the parent has initialised `this`
    pub class_fields: Option<std::rc::Rc<crate::ast::ClassDecl>>,
}

pub type EnvRef = Rc<RefCell<Env>>;

impl Env {
    pub fn new(kind: ScopeKind, parent: Option<EnvRef>) -> EnvRef {
        Rc::new(RefCell::new(Env {
            kind,
            vars: HashMap::new(),
            parent,
            this: None,
            home: None,
            new_target: None,
            is_async: false,
            derived_constructor: false,
            class_fields: None,
        }))
    }

    pub fn global() -> EnvRef {
        Env::new(ScopeKind::Global, None)
    }
}

/// Helpers that need no interpreter.
pub fn lookup(env: &EnvRef, name: &str) -> Option<Value> {
    let mut cur = Some(env.clone());
    while let Some(e) = cur {
        let next = {
            let b = e.borrow();
            if let Some(v) = b.vars.get(name) {
                return Some(v.clone());
            }
            b.parent.clone()
        };
        cur = next;
    }
    None
}

/// Find the scope that owns `name`.
pub fn owner(env: &EnvRef, name: &str) -> Option<EnvRef> {
    let mut cur = Some(env.clone());
    while let Some(e) = cur {
        let next = {
            let b = e.borrow();
            if b.vars.contains_key(name) {
                return Some(e.clone());
            }
            b.parent.clone()
        };
        cur = next;
    }
    None
}

/// Declare a variable in `env` (idempotent: `var` re-declaration keeps value).
pub fn declare(env: &EnvRef, name: &str, value: Value) {
    let e = env.borrow();
    let key: Rc<str> = Rc::from(name);
    if e.vars.contains_key(&key) {
        return;
    }
    drop(e);
    env.borrow_mut().vars.insert(key, value);
}

/// A copy of `src`'s own bindings on the same parent — one per loop iteration,
/// so closures created inside the body capture *that* iteration's values.
/// (`let i` in a `for` header, see `Interp::exec`.)
pub fn fork(src: &EnvRef) -> EnvRef {
    let src_b = src.borrow();
    let out = Rc::new(RefCell::new(Env {
        kind: ScopeKind::Block,
        vars: src_b.vars.clone(),
        parent: src_b.parent.clone(),
        this: src_b.this.clone(),
        home: src_b.home.clone(),
        new_target: src_b.new_target.clone(),
        is_async: src_b.is_async,
        derived_constructor: src_b.derived_constructor,
        class_fields: src_b.class_fields.clone(),
    }));
    out
}

/// Copy the values of `from`'s own bindings back into `dst` — after a loop body
/// ran in its forked scope, the loop needs the new values for test/update.
pub fn copy_values(from: &EnvRef, dst: &EnvRef) {
    let vars = from.borrow().vars.clone();
    let mut d = dst.borrow_mut();
    for (k, v) in vars {
        d.vars.insert(k, v);
    }
}

/// The root (global) scope of `env`.
pub fn global_scope(env: &EnvRef) -> EnvRef {
    let mut cur = env.clone();
    loop {
        let parent = cur.borrow().parent.clone();
        match parent {
            Some(p) => cur = p,
            None => break cur,
        }
    }
}

/// Write a global binding so that **both** look-up paths see it: identifiers
/// resolve through the global scope's variables first, `window.x` through the
/// global object. A host embedding (DOM bindings!) that only writes one of the
/// two gets a binding that works for `window.x` and throws for the bare name —
/// so `set_global` always writes both.
pub fn set_global(env: &EnvRef, name: &str, value: Value) {
    global_scope(env)
        .borrow_mut()
        .vars
        .insert(Rc::from(name), value);
}

/// `var` declarations belong to the nearest function/global scope.
pub fn var_scope(env: &EnvRef) -> EnvRef {
    let mut cur = env.clone();
    loop {
        let parent = {
            let b = cur.borrow();
            if b.kind != ScopeKind::Block {
                return cur.clone();
            }
            b.parent.clone()
        };
        match parent {
            Some(p) => cur = p,
            None => return cur,
        }
    }
}

/// Assign `name`, walking up the chain; undeclared names land on the global.
/// Returns `false` only for cases we cannot fix (never, today).
pub fn assign(env: &EnvRef, name: &str, value: Value) {
    if let Some(owner) = owner(env, name) {
        owner.borrow_mut().vars.insert(Rc::from(name), value);
        return;
    }
    // implicit global, like sloppy-mode JS
    let mut cur = env.clone();
    let global = loop {
        let parent = cur.borrow().parent.clone();
        match parent {
            Some(p) => cur = p,
            None => break cur,
        }
    };
    global.borrow_mut().vars.insert(Rc::from(name), value);
}

/// Nearest scope providing `this` (arrow functions capture their parent's).
pub fn this_value(env: &EnvRef) -> Value {
    let mut cur = Some(env.clone());
    while let Some(e) = cur {
        let next = {
            let b = e.borrow();
            if let Some(t) = &b.this {
                return t.clone();
            }
            if b.kind == ScopeKind::Global {
                return Value::Undefined;
            }
            b.parent.clone()
        };
        cur = next;
    }
    Value::Undefined
}

/// Nearest scope with a home object (for `super.x` in methods).
pub fn home_object(env: &EnvRef) -> Option<Value> {
    let mut cur = Some(env.clone());
    while let Some(e) = cur {
        let next = {
            let b = e.borrow();
            if let Some(h) = &b.home {
                return Some(h.clone());
            }
            b.parent.clone()
        };
        cur = next;
    }
    None
}

pub fn new_target(env: &EnvRef) -> Option<Value> {
    let mut cur = Some(env.clone());
    while let Some(e) = cur {
        let next = {
            let b = e.borrow();
            if b.new_target.is_some() {
                return b.new_target.clone();
            }
            if b.kind == ScopeKind::Function {
                return None;
            }
            b.parent.clone()
        };
        cur = next;
    }
    None
}

pub fn in_async(env: &EnvRef) -> bool {
    let mut cur = Some(env.clone());
    while let Some(e) = cur {
        let next = {
            let b = e.borrow();
            if b.is_async {
                return true;
            }
            if b.kind == ScopeKind::Function {
                return false;
            }
            b.parent.clone()
        };
        cur = next;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shadowing_and_lookup() {
        let g = Env::global();
        declare(&g, "x", Value::Number(1.0));
        let b = Env::new(ScopeKind::Block, Some(g.clone()));
        assert_eq!(lookup(&b, "x").unwrap().to_number(), 1.0);
        declare(&b, "x", Value::Number(2.0));
        assert_eq!(lookup(&b, "x").unwrap().to_number(), 2.0, "block shadows");
        assert_eq!(
            lookup(&g, "x").unwrap().to_number(),
            1.0,
            "global unchanged"
        );
    }

    #[test]
    fn var_scope_skips_blocks() {
        let g = Env::global();
        let f = Env::new(ScopeKind::Function, Some(g.clone()));
        let b = Env::new(ScopeKind::Block, Some(f.clone()));
        let target = var_scope(&b);
        assert!(Rc::ptr_eq(&target, &f));
        let target2 = var_scope(&f);
        assert!(Rc::ptr_eq(&target2, &f));
    }

    #[test]
    fn assign_updates_owner_or_becomes_global() {
        let g = Env::global();
        let f = Env::new(ScopeKind::Function, Some(g.clone()));
        declare(&g, "counter", Value::Number(0.0));
        assign(&f, "counter", Value::Number(7.0));
        assert_eq!(lookup(&g, "counter").unwrap().to_number(), 7.0);
        assign(&f, "brand_new", Value::string("global"));
        assert_eq!(lookup(&g, "brand_new").unwrap().to_string(), "global");
    }

    #[test]
    fn this_search_stops_at_function_scope() {
        let g = Env::global();
        let f = Env::new(ScopeKind::Function, Some(g.clone()));
        f.borrow_mut().this = Some(Value::string("obj"));
        let arrow = Env::new(ScopeKind::Block, Some(f.clone()));
        assert_eq!(this_value(&arrow).to_string(), "obj");
    }
}
