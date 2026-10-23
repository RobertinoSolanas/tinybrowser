//! Statement execution, hoisting and binding patterns.

use std::cell::RefCell;
use std::rc::Rc;

use super::ast as js;
use super::env::{self, Env, EnvRef, ScopeKind};
use super::runtime::{Flow, FnData, FnKind, Interp, JsResult};
use super::value::{Key, Kind, Obj, Value};

impl Interp {
    /// Hoist `var`s and function declarations of a scope.
    ///
    /// `let`/`const` are created when their statement runs (no temporal-dead-
    /// zone errors — a documented simplification), but function declarations
    /// must exist before the body runs, because real code calls helpers defined
    /// further down all the time.
    pub fn hoist(&mut self, block: &js::Block, env: &EnvRef) {
        let target = env::var_scope(env);
        self.hoist_walk(&block.body, env, &target);
    }

    fn hoist_walk(&mut self, body: &[Rc<js::Stmt>], env: &EnvRef, target: &EnvRef) {
        for s in body {
            match &s.kind {
                js::StmtKind::Decl { kind, decls } => {
                    if *kind == js::VarKind::Var {
                        for d in decls {
                            self.hoist_pattern(&d.pattern, target);
                        }
                    }
                }
                js::StmtKind::Fn(f) => {
                    let name = match &f.name {
                        Some(n) => n.clone(),
                        None => continue,
                    };
                    let val = self.make_function(f, env, None);
                    target.borrow_mut().vars.insert(name, val);
                }
                js::StmtKind::Block(b) => self.hoist_walk(&b.body, env, target),
                js::StmtKind::If { cons, els, .. } => {
                    self.hoist_stmt(cons, env, target);
                    if let Some(e) = els {
                        self.hoist_stmt(e, env, target);
                    }
                }
                js::StmtKind::For { body, init, .. } => {
                    if let Some(js::ForInit::Decl(js::VarKind::Var, ds)) = init {
                        for d in ds {
                            self.hoist_pattern(&d.pattern, target);
                        }
                    }
                    self.hoist_stmt(body, env, target);
                }
                js::StmtKind::ForIn { body, left, .. } | js::StmtKind::ForOf { body, left, .. } => {
                    if let js::ForInit::Decl(js::VarKind::Var, ds) = left {
                        for d in ds {
                            self.hoist_pattern(&d.pattern, target);
                        }
                    }
                    self.hoist_stmt(body, env, target);
                }
                js::StmtKind::While { body, .. } | js::StmtKind::Do { body, .. } => {
                    self.hoist_stmt(body, env, target)
                }
                js::StmtKind::Labeled(_, s) => self.hoist_stmt(s, env, target),
                js::StmtKind::Try {
                    block,
                    catch,
                    finalizer,
                } => {
                    self.hoist_walk(&block.body, env, target);
                    if let Some(c) = catch {
                        self.hoist_walk(&c.body.body, env, target);
                    }
                    if let Some(f) = finalizer {
                        self.hoist_walk(&f.body, env, target);
                    }
                }
                js::StmtKind::Switch { cases, .. } => {
                    for c in cases {
                        self.hoist_walk(&c.body, env, target);
                    }
                }
                _ => {}
            }
        }
    }

    fn hoist_stmt(&mut self, s: &Rc<js::Stmt>, env: &EnvRef, target: &EnvRef) {
        match &s.kind {
            js::StmtKind::Block(b) => self.hoist_walk(&b.body, env, target),
            other => {
                let tmp = js::Block {
                    body: vec![s.clone()],
                };
                self.hoist_walk(&tmp.body, env, target);
                let _ = other;
            }
        }
    }

    fn hoist_pattern(&mut self, pat: &js::Pattern, target: &EnvRef) {
        match pat {
            js::Pattern::Binding(n) => {
                target
                    .borrow_mut()
                    .vars
                    .entry(n.clone())
                    .or_insert(Value::Undefined);
            }
            js::Pattern::Array(els) => {
                for e in els {
                    match e {
                        js::ArrayEl::Item(p, _) => self.hoist_pattern(p, target),
                        js::ArrayEl::Rest(n) => {
                            target
                                .borrow_mut()
                                .vars
                                .entry(n.clone())
                                .or_insert(Value::Undefined);
                        }
                        js::ArrayEl::Hole => {}
                    }
                }
            }
            js::Pattern::Object(els) => {
                for e in els {
                    match &e.pattern {
                        js::Pattern::Binding(n) if e.rest || &**n != "" => {
                            target
                                .borrow_mut()
                                .vars
                                .entry(n.clone())
                                .or_insert(Value::Undefined);
                        }
                        p => self.hoist_pattern(p, target),
                    }
                }
            }
        }
    }

    // ---------------------------------------------------------------- blocks

    /// Run a block. `fresh=false` means the caller already made the scope
    /// (a function body), so declarations land directly in `env`.
    pub fn exec_block(&mut self, block: &js::Block, env: &EnvRef, fresh: bool) -> JsResult<Flow> {
        let scope = if fresh {
            Env::new(ScopeKind::Block, Some(env.clone()))
        } else {
            env.clone()
        };
        for s in &block.body {
            match self.exec(s, &scope)? {
                Flow::Normal => {}
                other => return Ok(other),
            }
        }
        Ok(Flow::Normal)
    }

    /// Run a list of statements in a fresh block scope (switch cases, …).
    pub fn exec_stmts(&mut self, body: &[Rc<js::Stmt>], env: &EnvRef) -> JsResult<Flow> {
        let b = js::Block {
            body: body.to_vec(),
        };
        self.exec_block(&b, env, true)
    }

    // ------------------------------------------------------------- statements

    pub fn exec(&mut self, s: &Rc<js::Stmt>, env: &EnvRef) -> JsResult<Flow> {
        self.tick()?;
        match &s.kind {
            js::StmtKind::Empty | js::StmtKind::Debugger => Ok(Flow::Normal),

            js::StmtKind::Expr(e) => {
                // expression statements carry the completion value of a script,
                // including through `try` / `if` / plain blocks
                self.last_value = self.eval(e, env)?;
                Ok(Flow::Normal)
            }

            js::StmtKind::Decl { kind, decls } => {
                let lexical = *kind != js::VarKind::Var;
                for d in decls {
                    let value = match &d.init {
                        Some(e) => self.eval(e, env)?,
                        None => Value::Undefined,
                    };
                    self.bind_pattern(&d.pattern, &value, env, lexical)?;
                }
                Ok(Flow::Normal)
            }

            js::StmtKind::Block(b) => self.exec_block(b, env, true),

            js::StmtKind::If { test, cons, els } => {
                if self.eval(test, env)?.to_boolean() {
                    self.exec(cons, env)
                } else if let Some(e) = els {
                    self.exec(e, env)
                } else {
                    Ok(Flow::Normal)
                }
            }

            js::StmtKind::For {
                init,
                test,
                update,
                body,
            } => {
                let my = self.loop_label.take();
                // `for (let i = …)` gets one scope *per iteration* so closures
                // created inside capture that iteration's value; `var` shares one
                let lexical =
                    matches!(init, Some(js::ForInit::Decl(k, _)) if *k != js::VarKind::Var);
                let outer = Env::new(ScopeKind::Block, Some(env.clone()));
                if let Some(i) = init {
                    match i {
                        js::ForInit::Decl(kind, decls) => {
                            let lexical = *kind != js::VarKind::Var;
                            for d in decls {
                                let v = match &d.init {
                                    Some(e) => self.eval(e, env)?,
                                    None => Value::Undefined,
                                };
                                self.bind_pattern(&d.pattern, &v, &outer, lexical)?;
                            }
                        }
                        js::ForInit::Expr(e) => {
                            self.eval(e, env)?;
                        }
                        js::ForInit::Pattern(_) => {}
                    }
                }
                let mut iter_guard = 0u64;
                loop {
                    iter_guard += 1;
                    if iter_guard > 50_000_000 {
                        return Err(self.range_error("for loop exceeded its iteration budget"));
                    }
                    if let Some(t) = test {
                        if !self.eval(t, &outer)?.to_boolean() {
                            break;
                        }
                    }
                    let iter_env = if lexical {
                        env::fork(&outer)
                    } else {
                        outer.clone()
                    };
                    let flow = self.exec(body, &iter_env)?;
                    if lexical {
                        // the loop's own bookkeeping uses the iteration's result
                        env::copy_values(&iter_env, &outer);
                    }
                    match flow {
                        Flow::Break(None) => break,
                        Flow::Break(Some(l)) if label_is(&l, &my) => break,
                        Flow::Continue(Some(l)) if label_is(&l, &my) => {}
                        Flow::Continue(None) | Flow::Normal => {}
                        other => return Ok(other),
                    }
                    if let Some(u) = update {
                        self.eval(u, &outer)?;
                    }
                }
                Ok(Flow::Normal)
            }

            js::StmtKind::ForIn { left, obj, body } => {
                let my = self.loop_label.take();
                let target = self.eval(obj, env)?;
                let keys = self.enumerable_keys(&target);
                for k in keys {
                    match self.bind_loop_var(left, &Value::string(k), env, body)? {
                        Flow::Break(Some(l)) if label_is(&l, &my) => break,
                        Flow::Continue(Some(l)) if label_is(&l, &my) => continue,
                        Flow::Break(None) => break,
                        Flow::Normal | Flow::Continue(None) => {}
                        other => return Ok(other),
                    }
                }
                Ok(Flow::Normal)
            }

            js::StmtKind::ForOf {
                left, obj, body, ..
            } => {
                let my = self.loop_label.take();
                let target = self.eval(obj, env)?;
                let items = self.iterate(&target)?;
                for v in items {
                    match self.bind_loop_var(left, &v, env, body)? {
                        Flow::Normal => {}
                        Flow::Break(None) => break,
                        Flow::Break(Some(l)) if label_is(&l, &my) => break,
                        Flow::Continue(None) => continue,
                        Flow::Continue(Some(l)) if label_is(&l, &my) => continue,
                        other => return Ok(other),
                    }
                }
                Ok(Flow::Normal)
            }

            js::StmtKind::While { test, body } => {
                let my = self.loop_label.take();
                let mut guard = 0u64;
                while self.eval(test, env)?.to_boolean() {
                    guard += 1;
                    if guard > 50_000_000 {
                        return Err(self.range_error("while loop exceeded its iteration budget"));
                    }
                    match self.exec(body, env)? {
                        Flow::Break(None) => break,
                        Flow::Break(Some(l)) if label_is(&l, &my) => break,
                        Flow::Continue(Some(l)) if label_is(&l, &my) => {}
                        Flow::Continue(None) | Flow::Normal => {}
                        other => return Ok(other),
                    }
                }
                Ok(Flow::Normal)
            }

            js::StmtKind::Do { body, test } => {
                let my = self.loop_label.take();
                let mut guard = 0u64;
                loop {
                    guard += 1;
                    if guard > 50_000_000 {
                        return Err(self.range_error("do loop exceeded its iteration budget"));
                    }
                    match self.exec(body, env)? {
                        Flow::Break(None) => break,
                        Flow::Break(Some(l)) if label_is(&l, &my) => break,
                        Flow::Continue(Some(l)) if label_is(&l, &my) => {}
                        Flow::Continue(None) | Flow::Normal => {}
                        other => return Ok(other),
                    }
                    if !self.eval(test, env)?.to_boolean() {
                        break;
                    }
                }
                Ok(Flow::Normal)
            }

            js::StmtKind::Switch { disc, cases } => {
                let d = self.eval(disc, env)?;
                let scope = Env::new(ScopeKind::Block, Some(env.clone()));
                let mut matched = false;
                let mut flow = Flow::Normal;
                for case in cases {
                    if !matched {
                        if let Some(t) = &case.test {
                            let tv = self.eval(t, &scope)?;
                            if d.strict_eq(&tv) {
                                matched = true;
                            }
                        } else {
                            // `default:` — but only if no `case` matched later
                            let mut any_match = false;
                            for other in cases {
                                if let Some(t) = &other.test {
                                    if d.strict_eq(&self.eval(t, &scope)?) {
                                        any_match = true;
                                        break;
                                    }
                                }
                            }
                            if any_match {
                                continue;
                            }
                            matched = true;
                        }
                    }
                    if matched {
                        match self.exec_stmts(&case.body, &scope)? {
                            Flow::Break(None) => {
                                flow = Flow::Normal;
                                break;
                            }
                            Flow::Normal => {}
                            other => {
                                flow = other;
                                break;
                            }
                        }
                    }
                }
                Ok(flow)
            }

            js::StmtKind::Return(Some(e)) => Ok(Flow::Return(self.eval(e, env)?)),
            js::StmtKind::Return(None) => Ok(Flow::Return(Value::Undefined)),
            js::StmtKind::Break(l) => Ok(Flow::Break(l.clone())),
            js::StmtKind::Continue(l) => Ok(Flow::Continue(l.clone())),

            js::StmtKind::Throw(e) => Err(self.eval(e, env)?),

            js::StmtKind::Try {
                block,
                catch,
                finalizer,
            } => {
                let mut flow = Flow::Normal;
                let mut thrown: Option<Value> = None;
                match self.exec_block(block, env, true) {
                    Ok(f) => flow = f,
                    Err(err) => thrown = Some(err),
                }
                if let Some(err) = thrown.take() {
                    match catch {
                        Some(c) => {
                            let cenv = Env::new(ScopeKind::Block, Some(env.clone()));
                            if let Some(p) = &c.param {
                                self.bind_pattern(p, &err, &cenv, true)?;
                            }
                            match self.exec_block(&c.body, &cenv, false) {
                                Ok(f) => flow = f,
                                Err(e2) => {
                                    thrown = Some(e2);
                                    flow = Flow::Normal;
                                }
                            }
                        }
                        None => thrown = Some(err),
                    }
                }
                if let Some(f) = finalizer {
                    match self.exec_block(f, env, true) {
                        Ok(ff) => {
                            // a `finally` that returns/ breaks wins, per spec
                            if !matches!(ff, Flow::Normal) {
                                thrown = None;
                                flow = ff;
                            }
                        }
                        Err(e3) => {
                            thrown = Some(e3);
                        }
                    }
                }
                match thrown {
                    Some(e) => Err(e),
                    None => Ok(flow),
                }
            }

            js::StmtKind::Fn(f) => {
                let name = f.name.clone().unwrap_or_else(|| Rc::from(""));
                let val = self.make_function(f, env, None);
                env::var_scope(env).borrow_mut().vars.insert(name, val);
                Ok(Flow::Normal)
            }

            js::StmtKind::Class(decl) => {
                let name = decl.name.clone().unwrap_or_else(|| Rc::from(""));
                let val = self.define_class(decl, env)?;
                // class declarations are lexically scoped like `let`
                env.borrow_mut().vars.insert(name, val);
                Ok(Flow::Normal)
            }

            js::StmtKind::Labeled(label, inner) => {
                // A loop has to *know* its label while it runs (`continue outer`),
                // so hand it down before entering; every other statement only
                // needs the label to stop `break`/`continue` when it bubbles out.
                if matches!(
                    inner.kind,
                    js::StmtKind::For { .. }
                        | js::StmtKind::ForIn { .. }
                        | js::StmtKind::ForOf { .. }
                        | js::StmtKind::While { .. }
                        | js::StmtKind::Do { .. }
                ) {
                    self.loop_label = Some(label.clone());
                }
                let f = self.exec(inner, env)?;
                match f {
                    Flow::Break(l) if l.as_deref() == Some(&**label) => Ok(Flow::Normal),
                    Flow::Continue(l) if l.as_deref() == Some(&**label) => Ok(Flow::Normal),
                    other => Ok(other),
                }
            }
        }
    }

    /// `for (const x of …)` / `for (x in …)`: bind and run the body.
    fn bind_loop_var(
        &mut self,
        left: &js::ForInit,
        value: &Value,
        env: &EnvRef,
        body: &Rc<js::Stmt>,
    ) -> JsResult<Flow> {
        let scope = Env::new(ScopeKind::Block, Some(env.clone()));
        match left {
            js::ForInit::Decl(kind, decls) => {
                let lexical = *kind != js::VarKind::Var;
                for d in decls {
                    self.bind_pattern(&d.pattern, value, &scope, lexical)?;
                }
            }
            js::ForInit::Pattern(p) => self.bind_pattern(p, value, &scope, true)?,
            js::ForInit::Expr(e) => self.assign_to(e, value, env)?,
        }
        // the body may declare things too
        match &body.kind {
            js::StmtKind::Block(b) => self.exec_block(b, &scope, false),
            _ => self.exec(body, &scope),
        }
    }

    // --------------------------------------------------------------- binding

    /// `const` / `let` / `var` binding of a pattern (possibly nested).
    ///
    /// `lexical == true` declares in the current scope, otherwise in the nearest
    /// function scope (and an already-existing name is only assigned).
    pub fn bind_pattern(
        &mut self,
        pat: &js::Pattern,
        value: &Value,
        env: &EnvRef,
        lexical: bool,
    ) -> JsResult<()> {
        match pat {
            js::Pattern::Binding(name) => {
                if lexical {
                    env.borrow_mut().vars.insert(name.clone(), value.clone());
                } else {
                    let target = env::var_scope(env);
                    target.borrow_mut().vars.insert(name.clone(), value.clone());
                }
                Ok(())
            }
            js::Pattern::Array(els) => {
                let items = match self.iterate(value) {
                    Ok(v) => v,
                    Err(_) => Vec::new(),
                };
                let mut i = 0usize;
                for e in els {
                    match e {
                        js::ArrayEl::Hole => i += 1,
                        js::ArrayEl::Item(p, default) => {
                            let mut v = items.get(i).cloned().unwrap_or(Value::Undefined);
                            if v.is_undefined() {
                                if let Some(d) = default {
                                    v = self.eval(d, env)?;
                                }
                            }
                            self.bind_pattern(p, &v, env, lexical)?;
                            i += 1;
                        }
                        js::ArrayEl::Rest(n) => {
                            let rest = self.array(items.iter().skip(i).cloned().collect());
                            if lexical {
                                env.borrow_mut().vars.insert(n.clone(), rest);
                            } else {
                                let target = env::var_scope(env);
                                target.borrow_mut().vars.insert(n.clone(), rest);
                            }
                            i = items.len();
                        }
                    }
                }
                Ok(())
            }
            js::Pattern::Object(els) => {
                let mut used: Vec<String> = Vec::new();
                for e in els {
                    if e.rest {
                        // `...rest` gets every own property not taken above
                        let mut rest = Obj::plain();
                        rest.proto = self.natives.object_proto.clone();
                        let keys = {
                            let b = value.as_object().map(|o| o.borrow().own_keys());
                            b.unwrap_or_default()
                        };
                        for k in keys {
                            if used.contains(&k) {
                                continue;
                            }
                            let v = self.get(value, &Key::from_str(&k))?;
                            rest.props
                                .push((Key::from_str(&k), super::value::Prop::data(v)));
                        }
                        if let js::Pattern::Binding(n) = &e.pattern {
                            if lexical {
                                env.borrow_mut().vars.insert(
                                    n.clone(),
                                    Value::Object(Rc::new(std::cell::RefCell::new(rest))),
                                );
                            } else {
                                let target = env::var_scope(env);
                                target.borrow_mut().vars.insert(
                                    n.clone(),
                                    Value::Object(Rc::new(std::cell::RefCell::new(rest))),
                                );
                            }
                        }
                        continue;
                    }
                    let key = match &e.key {
                        js::Key::Name(n) => {
                            used.push(n.to_string());
                            Key::Name(n.clone())
                        }
                        js::Key::Num(n) => Key::Num(*n),
                        js::Key::Computed(ce) => {
                            let cv = self.eval(ce, env)?;
                            self.to_key(&cv)?
                        }
                    };
                    let mut v = self.get(value, &key)?;
                    if v.is_undefined() {
                        if let Some(d) = &e.default {
                            v = self.eval(d, env)?;
                        }
                    }
                    self.bind_pattern(&e.pattern, &v, env, lexical)?;
                }
                Ok(())
            }
        }
    }

    /// Assign to an existing target (`x = 1`, `o.x = 1`, `[a, b] = arr`).
    pub fn assign_to(
        &mut self,
        target: &Rc<js::Expr>,
        value: &Value,
        env: &EnvRef,
    ) -> JsResult<()> {
        match &target.node {
            js::Node::Ident(name) => {
                env::assign(env, name, value.clone());
                Ok(())
            }
            js::Node::Member { obj, prop, .. } => {
                let base = self.eval(obj, env)?;
                let key = match prop {
                    js::MemberProp::Name(n) => Key::Name(n.clone()),
                    js::MemberProp::Expr(e) => {
                        let k = self.eval(e, env)?;
                        self.to_key(&k)?
                    }
                };
                self.set(&base, &key, value.clone())
            }
            js::Node::Array(items) => {
                let list = self.iterate(value).unwrap_or_default();
                let mut i = 0usize;
                for it in items {
                    match it {
                        None => i += 1,
                        Some(e) => {
                            let v = list.get(i).cloned().unwrap_or(Value::Undefined);
                            match &e.node {
                                js::Node::Spread(inner) => {
                                    let pat = pattern_of(inner)?;
                                    let rest = self.array(list[i..].to_vec());
                                    self.bind_pattern(&pat, &rest, env, false)?;
                                    i = list.len();
                                }
                                _ => {
                                    self.assign_to(e, &v, env)?;
                                    i += 1;
                                }
                            }
                        }
                    }
                }
                Ok(())
            }
            js::Node::Object(props) => {
                for p in props {
                    match p {
                        js::Prop::Shorthand(n) => {
                            let v = self.get(value, &Key::Name(n.clone()))?;
                            env::assign(env, n, v);
                        }
                        js::Prop::Value { key, value: pat } => {
                            let k = self.prop_key(key, env)?;
                            let v = self.get(value, &k)?;
                            if let js::Node::Ident(name) = &pat.node {
                                env::assign(env, name, v);
                            } else if let Ok(p) = pattern_of(pat) {
                                self.bind_pattern(&p, &v, env, false)?;
                            } else {
                                self.assign_to(pat, &v, env)?;
                            }
                        }
                        js::Prop::Spread(_) => {
                            // `{ ...rest } = obj` is handled via `pattern_of`
                        }
                        _ => {}
                    }
                }
                Ok(())
            }
            _ => Err(self.syntax_error("Invalid assignment target")),
        }
    }

    pub fn prop_key(&mut self, key: &js::Key, env: &EnvRef) -> JsResult<Key> {
        match key {
            js::Key::Name(n) => Ok(Key::Name(n.clone())),
            js::Key::Num(n) => Ok(Key::Num(*n)),
            js::Key::Computed(e) => {
                let v = self.eval(e, env)?;
                self.to_key(&v)
            }
        }
    }

    // ----------------------------------------------------------------- classes

    /// Fields of `decl` onto an instance (`x = 1`, `#p = 2`, static handled by
    /// [`Interp::define_class`]).
    pub fn init_class_fields(
        &mut self,
        decl: &Rc<js::ClassDecl>,
        this: &Value,
        env: &EnvRef,
    ) -> JsResult<()> {
        for m in &decl.members {
            if m.is_static {
                continue;
            }
            if let js::MemberKind::Field(init) = &m.kind {
                let key = self.prop_key(&m.key, env)?;
                let v = match init {
                    Some(e) => self.eval(e, env)?,
                    None => Value::Undefined,
                };
                self.set(this, &key, v)?;
            }
        }
        Ok(())
    }

    /// Evaluate a class expression/declaration into a constructor value.
    pub fn define_class(&mut self, decl: &Rc<js::ClassDecl>, env: &EnvRef) -> JsResult<Value> {
        // 1. instance prototype and its methods
        let proto = self.plain_ref();
        let name = decl.name.clone().unwrap_or_else(|| Rc::from(""));
        // class body sees an environment containing the class name
        let body_env = Env::new(ScopeKind::Block, Some(env.clone()));

        let mut ctor_members: Vec<&js::Member> = Vec::new();
        let mut statics: Vec<&js::Member> = Vec::new();
        for m in &decl.members {
            match &m.kind {
                js::MemberKind::Method(_) => {
                    if let js::Key::Name(n) = &m.key {
                        if &**n == "constructor" {
                            ctor_members.push(m);
                            continue;
                        }
                    }
                    if m.is_static {
                        statics.push(m);
                    } else {
                        self.define_member(&Value::Object(proto.clone()), m, &body_env, &name)?;
                    }
                }
                js::MemberKind::Getter(_) | js::MemberKind::Setter(_) => {
                    if m.is_static {
                        statics.push(m);
                    } else {
                        self.define_member(&Value::Object(proto.clone()), m, &body_env, &name)?;
                    }
                }
                js::MemberKind::Field(_) => {
                    if m.is_static {
                        statics.push(m);
                    }
                }
            }
        }

        // 2. the constructor function itself
        let ctor_fn_ast = ctor_members.first().and_then(|m| match &m.kind {
            js::MemberKind::Method(f) => Some(f.clone()),
            _ => None,
        });
        let obj = Rc::new(RefCell::new(Obj::plain()));
        obj.borrow_mut().proto = self.natives.fn_proto.clone();
        obj.borrow_mut()
            .define_internal("name", Value::string(name.as_ref()));
        obj.borrow_mut()
            .define_internal("prototype", Value::Object(proto.clone()));
        let ctor_len = ctor_fn_ast
            .as_ref()
            .map(|f| f.params.len() as u32)
            .unwrap_or(0);
        proto
            .borrow_mut()
            .define_internal("constructor", Value::Object(obj.clone()));
        obj.borrow_mut().kind = Kind::Fn(Rc::new(FnData {
            name: name.clone(),
            length: ctor_len,
            kind: FnKind::Class {
                decl: decl.clone(),
                closure: body_env.clone(),
            },
        }));
        let class_value = Value::Object(obj.clone());

        // 3. heritage: `class A extends B` ⇒ A.__proto__ = B, A.prototype.__proto__ = B.prototype
        if let Some(sc) = &decl.super_class {
            let parent = self.eval(sc, &body_env)?;
            if let Some(p) = parent.as_object() {
                obj.borrow_mut().proto = Some(p.clone());
                let pproto = p
                    .borrow()
                    .get_own(&Key::Name(Rc::from("prototype")))
                    .and_then(|pr| pr.value().cloned())
                    .and_then(|v| v.as_object().cloned());
                if let Some(pp) = pproto {
                    proto.borrow_mut().proto = Some(pp);
                }
            }
        }

        // 4. statics
        for m in statics {
            self.define_member(&class_value, m, &body_env, &name)?;
        }
        // 5. static fields with their initialisers
        for m in &decl.members {
            if !m.is_static {
                continue;
            }
            if let js::MemberKind::Field(init) = &m.kind {
                let key = self.prop_key(&m.key, &body_env)?;
                let v = match init {
                    Some(e) => self.eval(e, &body_env)?,
                    None => Value::Undefined,
                };
                self.set(&class_value, &key, v)?;
            }
        }
        // the name is visible inside the class body (self-recursion)
        if !name.is_empty() {
            body_env.borrow_mut().vars.insert(name, class_value.clone());
        }
        Ok(class_value)
    }

    fn define_member(
        &mut self,
        target: &Value,
        m: &js::Member,
        env: &EnvRef,
        class_name: &str,
    ) -> JsResult<()> {
        let key = self.prop_key(&m.key, env)?;
        match &m.kind {
            js::MemberKind::Method(f) => {
                let v = self.make_function(f, env, Some(target.clone()));
                target
                    .as_object()
                    .unwrap()
                    .borrow_mut()
                    .define(&key.name(), super::value::Prop::internal(v));
                Ok(())
            }
            js::MemberKind::Getter(g) => {
                let g = self.make_function(g, env, Some(target.clone()));
                let existing = target.as_object().unwrap().borrow().get_own(&key).cloned();
                let setter = match existing.map(|p| p.kind) {
                    Some(super::value::PropKind::Access { setter, .. }) => setter,
                    _ => None,
                };
                target.as_object().unwrap().borrow_mut().define(
                    &key.name(),
                    super::value::Prop::accessor(g.as_object().cloned(), setter),
                );
                Ok(())
            }
            js::MemberKind::Setter(s) => {
                let s = self.make_function(s, env, Some(target.clone()));
                let existing = target.as_object().unwrap().borrow().get_own(&key).cloned();
                let getter = match existing.map(|p| p.kind) {
                    Some(super::value::PropKind::Access { getter, .. }) => getter,
                    _ => None,
                };
                target.as_object().unwrap().borrow_mut().define(
                    &key.name(),
                    super::value::Prop::accessor(getter, s.as_object().cloned()),
                );
                Ok(())
            }
            js::MemberKind::Field(_) => Ok(()),
        }
        .map_err(|e: Value| {
            let _ = class_name;
            e
        })
    }

    /// Turn a [`js::Function`] into a callable value closed over `env`.
    pub fn make_function(
        &mut self,
        f: &Rc<js::Function>,
        env: &EnvRef,
        home: Option<Value>,
    ) -> Value {
        let mut o = Obj::plain();
        o.proto = self.natives.fn_proto.clone();
        let name = f.name.clone().unwrap_or_else(|| Rc::from(""));
        o.kind = Kind::Fn(Rc::new(FnData {
            name: name.clone(),
            length: f.params.len() as u32,
            kind: FnKind::User {
                f: f.clone(),
                closure: env.clone(),
                home,
            },
        }));
        let obj = Rc::new(std::cell::RefCell::new(o));
        obj.borrow_mut()
            .define_internal("name", Value::string(name.as_ref()));
        obj.borrow_mut()
            .define_internal("length", Value::Number(f.params.len() as f64));
        // functions carry a prototype object so `new fn()` works
        let proto = self.plain_ref();
        proto
            .borrow_mut()
            .define_internal("constructor", Value::Object(obj.clone()));
        obj.borrow_mut()
            .define_internal("prototype", Value::Object(proto));
        Value::Object(obj)
    }
}

/// Destructuring target as a pattern (`[a, b] = x` in an expression).
fn pattern_of(e: &Rc<js::Expr>) -> Result<js::Pattern, Value> {
    Ok(match &e.node {
        js::Node::Ident(n) => js::Pattern::Binding(n.clone()),
        js::Node::Array(items) => js::Pattern::Array(
            items
                .iter()
                .map(|it| match it {
                    None => Ok(js::ArrayEl::Hole),
                    Some(inner) => Ok(js::ArrayEl::Item(pattern_of(inner)?, None)),
                })
                .collect::<Result<Vec<_>, Value>>()?,
        ),
        js::Node::Object(props) => {
            let mut out = Vec::new();
            for p in props {
                match p {
                    js::Prop::Shorthand(n) => out.push(js::ObjectEl {
                        key: js::Key::Name(n.clone()),
                        pattern: js::Pattern::Binding(n.clone()),
                        default: None,
                        rest: false,
                    }),
                    js::Prop::Value { key, value } => out.push(js::ObjectEl {
                        key: key.clone(),
                        pattern: pattern_of(value)?,
                        default: None,
                        rest: false,
                    }),
                    js::Prop::Spread(src) => {
                        let name = match &src.node {
                            js::Node::Ident(n) => n.clone(),
                            _ => return Err(Value::string("unsupported spread target")),
                        };
                        out.push(js::ObjectEl {
                            key: js::Key::Name(Rc::from("")),
                            pattern: js::Pattern::Binding(name),
                            default: None,
                            rest: true,
                        });
                    }
                    _ => {
                        return Err(Value::string("unsupported destructuring target"));
                    }
                }
            }
            js::Pattern::Object(out)
        }
        _ => return Err(Value::string("unsupported destructuring target")),
    })
}

/// Does a `break <l>` / `continue <l>` target the loop we are currently in?
fn label_is(l: &Rc<str>, my: &Option<Rc<str>>) -> bool {
    my.as_deref() == Some(&**l)
}
