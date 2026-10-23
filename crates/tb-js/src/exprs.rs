//! Expression evaluation.

use std::rc::Rc;

use super::ast as js;
use super::env::{self, Env, EnvRef};
use super::regexp::RegExp;
use super::runtime::{Interp, JsResult};
use super::value::{Key, Value};

impl Interp {
    pub fn eval(&mut self, e: &Rc<js::Expr>, env: &EnvRef) -> JsResult<Value> {
        self.tick()?;
        match &e.node {
            js::Node::Num(n) => Ok(Value::Number(*n)),
            js::Node::Str(s) => Ok(Value::Str(s.clone())),
            js::Node::Bool(b) => Ok(Value::Boolean(*b)),
            js::Node::Null => Ok(Value::Null),
            js::Node::Undefined => Ok(Value::Undefined),
            js::Node::This => Ok(env::this_value(env)),
            js::Node::NewTarget => Ok(env::new_target(env).unwrap_or(Value::Undefined)),

            js::Node::Ident(name) => match env::lookup(env, name) {
                Some(v) => Ok(v),
                None => {
                    // globals are properties of globalThis, so host bindings and
                    // `var` at top level both show up here
                    if let Some(o) = env::lookup(env, "globalThis") {
                        if self.has_prop(&o, &Key::Name(name.clone())) {
                            return self.get(&o, &Key::Name(name.clone()));
                        }
                    }
                    let g = self.globals_value();
                    if self.has_prop(&g, &Key::Name(name.clone())) {
                        return self.get(&g, &Key::Name(name.clone()));
                    }
                    Err(self.reference_error(&format!("{name} is not defined")))
                }
            },

            js::Node::Super => {
                // `super.x` — handled by Member; a bare `super` is the prototype
                // of the current method's home object
                match env::home_object(env) {
                    Some(h) => {
                        let proto = h.as_object().and_then(|o| o.borrow().proto.clone());
                        Ok(match proto {
                            Some(p) => Value::Object(p),
                            None => Value::Undefined,
                        })
                    }
                    None => Err(self.syntax_error("'super' keyword unexpected here")),
                }
            }

            js::Node::Array(items) => {
                let mut out = Vec::with_capacity(items.len());
                for it in items {
                    match it {
                        None => out.push(Value::Undefined),
                        Some(x) => {
                            if let js::Node::Spread(inner) = &x.node {
                                let v = self.eval(inner, env)?;
                                out.extend(self.iterate(&v)?);
                            } else {
                                out.push(self.eval(x, env)?);
                            }
                        }
                    }
                }
                Ok(self.array(out))
            }

            js::Node::Object(props) => {
                let obj = self.plain_object();
                let mut protos: Vec<Value> = Vec::new();
                // `__proto__: x` in a literal sets the prototype
                let mut literal_proto: Option<Value> = None;
                for p in props {
                    if let js::Prop::Value { key, value } = p {
                        if let js::Key::Name(n) = key {
                            if &**n == "__proto__" {
                                literal_proto = Some(self.eval(value, env)?);
                                continue;
                            }
                        }
                    }
                    match p {
                        js::Prop::Shorthand(n) => {
                            let v =
                                self.eval(&js::expr(js::Node::Ident(n.clone()), e.line), env)?;
                            self.set(&obj, &Key::Name(n.clone()), v)?;
                        }
                        js::Prop::Value { key, value } => {
                            let k = self.prop_key(key, env)?;
                            let v = self.eval(value, env)?;
                            self.set(&obj, &k, v)?;
                        }
                        js::Prop::Method { key, func } => {
                            let k = self.prop_key(key, env)?;
                            let f = self.make_function(func, env, Some(obj.clone()));
                            self.set(&obj, &k, f)?;
                        }
                        js::Prop::Accessor {
                            key,
                            getter,
                            setter,
                        } => {
                            let k = self.prop_key(key, env)?;
                            let g = match getter {
                                Some(g) => Some(
                                    self.make_function(g, env, Some(obj.clone()))
                                        .as_object()
                                        .cloned()
                                        .unwrap(),
                                ),
                                None => None,
                            };
                            let s = match setter {
                                Some(s) => Some(
                                    self.make_function(s, env, Some(obj.clone()))
                                        .as_object()
                                        .cloned()
                                        .unwrap(),
                                ),
                                None => None,
                            };
                            let target = obj.as_object().unwrap().clone();
                            target
                                .borrow_mut()
                                .define(&k.name(), super::value::Prop::accessor(g, s));
                        }
                        js::Prop::Spread(src) => {
                            let v = self.eval(src, env)?;
                            protos.push(v.clone());
                            let keys = self.enumerable_keys(&v);
                            for kk in keys {
                                let val = self.get(&v, &Key::from_str(&kk))?;
                                self.set(&obj, &Key::from_str(&kk), val)?;
                            }
                        }
                    }
                }
                if let Some(Value::Object(p)) = literal_proto {
                    obj.as_object().unwrap().borrow_mut().proto = Some(p);
                }
                let _ = protos;
                Ok(obj)
            }

            js::Node::Template { chunks, exprs } => {
                let mut out = String::new();
                for (i, chunk) in chunks.iter().enumerate() {
                    out.push_str(chunk);
                    if let Some(x) = exprs.get(i) {
                        let v = self.eval(x, env)?;
                        out.push_str(&self.to_string_val(&v)?);
                    }
                }
                Ok(Value::string(out))
            }

            js::Node::Tagged { tag, chunks, exprs } => {
                // build the strings array, then call the tag
                let mut strs: Vec<Value> = Vec::new();
                for c in chunks {
                    strs.push(Value::string(c.as_ref()));
                }
                let arr = self.array(strs.clone());
                let raw = self.array(strs);
                self.set(&arr, &Key::Name(Rc::from("raw")), raw)?;
                let mut args = vec![arr];
                for x in exprs {
                    args.push(self.eval(x, env)?);
                }
                let t = self.eval(tag, env)?;
                self.call_value(&t, Value::Undefined, args)
            }

            js::Node::Regex(src, flags) => {
                let re = RegExp::compile(src, flags).map_err(|err| self.syntax_error(&err.0))?;
                Ok(self.regexp_value(re))
            }

            js::Node::Member {
                obj,
                prop,
                optional,
            } => {
                let base = self.eval(obj, env)?;
                if *optional && base.is_nullish() {
                    return Ok(Value::Undefined);
                }
                let key = match prop {
                    js::MemberProp::Name(n) => Key::Name(n.clone()),
                    js::MemberProp::Expr(ke) => {
                        let k = self.eval(ke, env)?;
                        self.to_key(&k)?
                    }
                };
                self.get(&base, &key)
            }

            js::Node::Call {
                callee,
                args,
                optional,
            } => {
                // `super(...)` inside a derived constructor
                if let js::Node::Super = &callee.node {
                    return self.super_call(env, args);
                }
                let (func, this) = self.eval_callee(callee, env, *optional)?;
                if *optional && func.is_nullish() {
                    return Ok(Value::Undefined);
                }
                let argv = self.eval_args(args, env)?;
                match func.as_object() {
                    Some(o) => self.call(o, this, argv),
                    None => {
                        let desc = self.describe(&func);
                        Err(self.type_error(&format!("{desc} is not a function")))
                    }
                }
            }

            js::Node::New { callee, args } => {
                let f = self.eval(callee, env)?;
                let argv = self.eval_args(args, env)?;
                self.construct_value(&f, argv)
            }

            js::Node::Import(_) => Err(self.syntax_error("dynamic import() is not supported")),

            js::Node::Unary { op, arg } => self.eval_unary(*op, arg, env),

            js::Node::Update { op, target, prefix } => {
                let old = {
                    let v = self.eval(target, env)?;
                    self.to_number_val(&v)?
                };
                let new = match op {
                    js::UpdateOp::Inc => old + 1.0,
                    js::UpdateOp::Dec => old - 1.0,
                };
                self.assign_to(target, &Value::Number(new), env)?;
                Ok(Value::Number(if *prefix { new } else { old }))
            }

            js::Node::Binary { op, lhs, rhs } => self.eval_binary(*op, lhs, rhs, env),

            js::Node::Logical { op, lhs, rhs } => {
                let l = self.eval(lhs, env)?;
                match op {
                    js::LogicOp::And => {
                        if !l.to_boolean() {
                            Ok(l)
                        } else {
                            self.eval(rhs, env)
                        }
                    }
                    js::LogicOp::Or => {
                        if l.to_boolean() {
                            Ok(l)
                        } else {
                            self.eval(rhs, env)
                        }
                    }
                    js::LogicOp::Nullish => {
                        if l.is_nullish() {
                            self.eval(rhs, env)
                        } else {
                            Ok(l)
                        }
                    }
                }
            }

            js::Node::Assign { target, op, value } => {
                let v = match op {
                    None => self.eval(value, env)?,
                    Some(js::AssignOp::Assign) => self.eval(value, env)?,
                    Some(op) => {
                        let cur = self.eval(target, env)?;
                        let rhs = self.eval(value, env)?;
                        match op {
                            js::AssignOp::Add => self.add(&cur, &rhs)?,
                            js::AssignOp::Sub => {
                                Value::Number(self.to_number_val(&cur)? - self.to_number_val(&rhs)?)
                            }
                            js::AssignOp::Mul => {
                                Value::Number(self.to_number_val(&cur)? * self.to_number_val(&rhs)?)
                            }
                            js::AssignOp::Div => {
                                Value::Number(self.to_number_val(&cur)? / self.to_number_val(&rhs)?)
                            }
                            js::AssignOp::Mod => {
                                Value::Number(self.to_number_val(&cur)? % self.to_number_val(&rhs)?)
                            }
                            js::AssignOp::Pow => Value::Number(
                                self.to_number_val(&cur)?.powf(self.to_number_val(&rhs)?),
                            ),
                            // logical assignment: the right side is only
                            // evaluated (and stored) when the test says so
                            js::AssignOp::And => {
                                if cur.to_boolean() {
                                    rhs
                                } else {
                                    return Ok(cur);
                                }
                            }
                            js::AssignOp::Or => {
                                if cur.to_boolean() {
                                    return Ok(cur);
                                } else {
                                    rhs
                                }
                            }
                            js::AssignOp::BitAnd => Value::Number(
                                (cur.to_int32() & self.to_number_val(&rhs)?.to_int32_f()) as f64,
                            ),
                            js::AssignOp::BitOr => Value::Number(
                                (cur.to_int32() | self.to_number_val(&rhs)?.to_int32_f()) as f64,
                            ),
                            js::AssignOp::BitXor => Value::Number(
                                (cur.to_int32() ^ self.to_number_val(&rhs)?.to_int32_f()) as f64,
                            ),
                            js::AssignOp::Shl => Value::Number(
                                (cur.to_int32() << (self.to_number_val(&rhs)?.to_uint32_f() & 31))
                                    as f64,
                            ),
                            js::AssignOp::Shr => Value::Number(
                                (cur.to_int32() >> (self.to_number_val(&rhs)?.to_uint32_f() & 31))
                                    as f64,
                            ),
                            js::AssignOp::UShr => Value::Number(
                                (cur.to_uint32() >> (self.to_number_val(&rhs)?.to_uint32_f() & 31))
                                    as f64,
                            ),
                            js::AssignOp::Nullish => {
                                if cur.is_nullish() {
                                    rhs
                                } else {
                                    cur
                                }
                            }
                            js::AssignOp::Assign => unreachable!(),
                        }
                    }
                };
                self.assign_to(target, &v, env)?;
                Ok(v)
            }

            js::Node::Cond { test, then, els } => {
                if self.eval(test, env)?.to_boolean() {
                    self.eval(then, env)
                } else {
                    self.eval(els, env)
                }
            }

            js::Node::Seq(items) => {
                let mut last = Value::Undefined;
                for i in items {
                    last = self.eval(i, env)?;
                }
                Ok(last)
            }

            js::Node::Spread(inner) => self.eval(inner, env),

            js::Node::Fn(f) => Ok(self.make_function(f, env, None)),

            js::Node::Arrow {
                params,
                body,
                is_async,
            } => {
                // an arrow has no `this` of its own: pin the one in scope now
                let fake = Rc::new(js::Function {
                    name: None,
                    params: params.clone(),
                    body: match body {
                        js::FnBody::Block(b) => b.clone(),
                        js::FnBody::Expr(e) => Rc::new(js::Block {
                            body: vec![js::stmt(js::StmtKind::Return(Some(e.clone())), e.line)],
                        }),
                    },
                    is_async: *is_async,
                    is_generator: false,
                    is_method: false,
                    is_arrow: true,
                    line: e.line,
                });
                let pin = Env::new(env::ScopeKind::Block, Some(env.clone()));
                pin.borrow_mut().this = Some(env::this_value(env));
                pin.borrow_mut().home = env::home_object(env);
                pin.borrow_mut().is_async = env::in_async(env);
                Ok(self.make_function(&fake, &pin, None))
            }

            js::Node::Class(decl) => self.define_class(decl, env),

            js::Node::Await(inner) => {
                let v = self.eval(inner, env)?;
                if !env::in_async(env) {
                    return Err(self.syntax_error("await is only valid in async functions"));
                }
                self.await_value(&v)
            }

            js::Node::Yield(_) => {
                self.note("yield is not supported");
                Ok(Value::Undefined)
            }
        }
    }

    fn eval_callee(
        &mut self,
        callee: &Rc<js::Expr>,
        env: &EnvRef,
        optional: bool,
    ) -> JsResult<(Value, Value)> {
        match &callee.node {
            js::Node::Member {
                obj,
                prop,
                optional: opt2,
            } => {
                // `super.m()`: the *lookup* starts at the home object's
                // prototype (`eval(Super)` gives exactly that), but the method
                // still runs with the current `this` — otherwise `this.kind`
                // inside a parent method would read the prototype's fields.
                let via_super = matches!(obj.node, js::Node::Super);
                let base = self.eval(obj, env)?;
                if (optional || *opt2) && base.is_nullish() {
                    return Ok((Value::Undefined, Value::Undefined));
                }
                let key = match prop {
                    js::MemberProp::Name(n) => Key::Name(n.clone()),
                    js::MemberProp::Expr(ke) => {
                        let k = self.eval(ke, env)?;
                        self.to_key(&k)?
                    }
                };
                let f = self.get(&base, &key)?;
                let this = if via_super {
                    env::this_value(env)
                } else {
                    base
                };
                Ok((f, this))
            }
            js::Node::Super => {
                // `super.method()`
                let home = env::home_object(env)
                    .ok_or_else(|| self.syntax_error("'super' keyword unexpected here"))?;
                Ok((Value::Undefined, home))
            }
            _ => {
                let f = self.eval(callee, env)?;
                Ok((f, Value::Undefined))
            }
        }
    }

    pub fn eval_args(&mut self, args: &[js::Arg], env: &EnvRef) -> JsResult<Vec<Value>> {
        let mut out = Vec::with_capacity(args.len());
        for a in args {
            match a {
                js::Arg::Single(e) => out.push(self.eval(e, env)?),
                js::Arg::Spread(e) => {
                    let v = self.eval(e, env)?;
                    out.extend(self.iterate(&v)?);
                }
            }
        }
        Ok(out)
    }

    /// `a + b` with JS's string-or-number rule.
    pub fn add(&mut self, a: &Value, b: &Value) -> JsResult<Value> {
        let pa = self.to_primitive(a, super::value::PrimitiveHint::Number)?;
        let pb = self.to_primitive(b, super::value::PrimitiveHint::Number)?;
        if pa.is_string() || pb.is_string() {
            return Ok(Value::string(format!(
                "{}{}",
                self.to_string_val(&pa)?,
                self.to_string_val(&pb)?
            )));
        }
        Ok(Value::Number(pa.to_number() + pb.to_number()))
    }

    fn eval_binary(
        &mut self,
        op: js::BinOp,
        lhs: &Rc<js::Expr>,
        rhs: &Rc<js::Expr>,
        env: &EnvRef,
    ) -> JsResult<Value> {
        use js::BinOp::*;
        // `in` / `instanceof` need the raw values
        match op {
            In => {
                let r = self.eval(rhs, env)?;
                let l = self.eval(lhs, env)?;
                let k = self.to_key(&l)?;
                return Ok(Value::Boolean(self.has_prop(&r, &k)));
            }
            Instanceof => {
                let l = self.eval(lhs, env)?;
                let r = self.eval(rhs, env)?;
                return Ok(Value::Boolean(self.instance_of(&l, &r)?));
            }
            Eq | Ne | StrictEq | StrictNe => {}
            _ => {}
        }
        let a = self.eval(lhs, env)?;
        let b = self.eval(rhs, env)?;
        Ok(match op {
            Add => self.add(&a, &b)?,
            Sub => Value::Number(self.to_number_val(&a)? - self.to_number_val(&b)?),
            Mul => Value::Number(self.to_number_val(&a)? * self.to_number_val(&b)?),
            Div => Value::Number(self.to_number_val(&a)? / self.to_number_val(&b)?),
            Mod => Value::Number(self.to_number_val(&a)? % self.to_number_val(&b)?),
            Pow => Value::Number(self.to_number_val(&a)?.powf(self.to_number_val(&b)?)),
            Lt => Value::Boolean(self.compare(&a, &b)?),
            Gt => Value::Boolean(self.compare(&b, &a)?),
            Le => Value::Boolean(!self.compare(&b, &a)?),
            Ge => Value::Boolean(!self.compare(&a, &b)?),
            Eq => Value::Boolean(a.loose_eq(&b)),
            Ne => Value::Boolean(!a.loose_eq(&b)),
            StrictEq => Value::Boolean(a.strict_eq(&b)),
            StrictNe => Value::Boolean(!a.strict_eq(&b)),
            BitAnd => Value::Number((a.to_int32() & b.to_int32()) as f64),
            BitOr => Value::Number((a.to_int32() | b.to_int32()) as f64),
            BitXor => Value::Number((a.to_int32() ^ b.to_int32()) as f64),
            Shl => Value::Number((a.to_int32() << (b.to_uint32() & 31)) as f64),
            Shr => Value::Number((a.to_int32() >> (b.to_uint32() & 31)) as f64),
            UShr => Value::Number((a.to_uint32() >> (b.to_uint32() & 31)) as f64),
            In | Instanceof => unreachable!(),
        })
    }

    fn eval_unary(&mut self, op: js::UnOp, arg: &Rc<js::Expr>, env: &EnvRef) -> JsResult<Value> {
        match op {
            js::UnOp::Typeof => {
                // `typeof undeclared` must not throw
                if let js::Node::Ident(name) = &arg.node {
                    if env::lookup(env, name).is_none()
                        && !self.has_prop(&self.globals_value(), &Key::Name(name.clone()))
                    {
                        return Ok(Value::string("undefined"));
                    }
                }
                let v = self.eval(arg, env)?;
                Ok(Value::string(v.type_of()))
            }
            js::UnOp::Void => {
                self.eval(arg, env)?;
                Ok(Value::Undefined)
            }
            js::UnOp::Delete => {
                // delete needs the base object, not the value
                if let js::Node::Member { obj, prop, .. } = &arg.node {
                    let base = self.eval(obj, env)?;
                    let key = match prop {
                        js::MemberProp::Name(n) => Key::Name(n.clone()),
                        js::MemberProp::Expr(ke) => {
                            let k = self.eval(ke, env)?;
                            self.to_key(&k)?
                        }
                    };
                    return Ok(Value::Boolean(self.delete_prop(&base, &key)));
                }
                self.eval(arg, env)?;
                Ok(Value::Boolean(true))
            }
            js::UnOp::Neg => {
                let v = self.eval(arg, env)?;
                Ok(Value::Number(-self.to_number_val(&v)?))
            }
            js::UnOp::Pos => {
                let v = self.eval(arg, env)?;
                Ok(Value::Number(self.to_number_val(&v)?))
            }
            js::UnOp::Not => {
                let v = self.eval(arg, env)?;
                Ok(Value::Boolean(!v.to_boolean()))
            }
            js::UnOp::BitNot => {
                let v = self.eval(arg, env)?;
                Ok(Value::Number((!v.to_int32()) as f64))
            }
        }
    }

    /// `super(...)` inside a derived constructor.
    fn super_call(&mut self, env: &EnvRef, args: &[js::Arg]) -> JsResult<Value> {
        let argv = self.eval_args(args, env)?;
        let new_target = env::new_target(env)
            .ok_or_else(|| self.syntax_error("'super' keyword unexpected here"))?;
        let this = env::this_value(env);
        // parent constructor = prototype of the class object
        let parent = match new_target
            .as_object()
            .and_then(|o| o.borrow().proto.clone())
        {
            Some(p) => Value::Object(p),
            None => return Err(self.type_error("class has no parent constructor")),
        };
        let result = self.run_parent_ctor(&parent, &this, argv, &new_target, env)?;
        // field initialisers of *this* class run right after super()
        let decl = env.borrow().class_fields.clone();
        if let Some(decl) = decl {
            self.init_class_fields(&decl, &result, env)?;
        }
        Ok(Value::Undefined)
    }

    /// Run a parent constructor against an existing `this`.
    pub fn run_parent_ctor(
        &mut self,
        parent: &Value,
        this: &Value,
        argv: Vec<Value>,
        new_target: &Value,
        _env: &EnvRef,
    ) -> JsResult<Value> {
        let obj = match parent.as_object() {
            Some(o) => o.clone(),
            None => return Err(self.type_error("super class is not a constructor")),
        };
        let data = match obj.borrow().fn_data().cloned() {
            Some(d) => d,
            None => return Err(self.type_error("super class is not a constructor")),
        };
        match &data.kind {
            super::runtime::FnKind::Class { decl, closure } => {
                let decl = decl.clone();
                let penv =
                    super::env::Env::new(super::env::ScopeKind::Function, Some(closure.clone()));
                penv.borrow_mut().this = Some(this.clone());
                penv.borrow_mut().new_target = Some(new_target.clone());
                penv.borrow_mut().home = Some(Value::Object(obj.clone()));
                penv.borrow_mut().class_fields = Some(decl.clone());
                match decl.ctor() {
                    Some(ctor) => {
                        let ctor = ctor.clone();
                        self.bind_params(&ctor, &penv, &argv, false)?;
                        self.run_body(&ctor, &penv)?;
                    }
                    None => {
                        if let Some(sc) = &decl.super_class {
                            let gp = self.eval(sc, closure)?;
                            self.run_parent_ctor(&gp, this, argv, new_target, &penv)?;
                        }
                        self.init_class_fields(&decl, this, &penv)?;
                    }
                }
                Ok(this.clone())
            }
            super::runtime::FnKind::User { f, closure, home } => {
                let f = f.clone();
                let penv =
                    super::env::Env::new(super::env::ScopeKind::Function, Some(closure.clone()));
                penv.borrow_mut().this = Some(this.clone());
                penv.borrow_mut().new_target = Some(new_target.clone());
                penv.borrow_mut().home = home.clone();
                self.bind_params(&f, &penv, &argv, false)?;
                self.run_body(&f, &penv)?;
                Ok(this.clone())
            }
            _ => {
                // native parent (`class X extends Array`): construct and graft
                let made = self.construct(&obj, argv)?;
                if let (Some(dst), Some(src)) = (this.as_object(), made.as_object()) {
                    let (kind_items, props) = {
                        let b = src.borrow();
                        let items = match &b.kind {
                            super::value::Kind::Array { items } => Some(items.clone()),
                            _ => None,
                        };
                        (items, b.props.clone())
                    };
                    {
                        let mut db = dst.borrow_mut();
                        if let Some(items) = kind_items {
                            db.kind = super::value::Kind::Array { items };
                        }
                        for (k, p) in props {
                            if k.name() == "length" {
                                continue;
                            }
                            db.define(&k.name(), p);
                        }
                    }
                }
                Ok(this.clone())
            }
        }
    }

    /// A short description of a value for error messages.
    pub fn describe(&mut self, v: &Value) -> String {
        match v {
            Value::Object(o) => {
                let b = o.borrow();
                if let Some(f) = b.fn_data() {
                    format!("{} ({})", f.name, "function")
                } else {
                    b.default_string()
                }
            }
            other => other.inspect(),
        }
    }
}

/// Small numeric helpers used by compound assignment.
trait IntOps {
    fn to_int32_f(self) -> i32;
    fn to_uint32_f(self) -> u32;
}
impl IntOps for f64 {
    fn to_int32_f(self) -> i32 {
        super::value::to_int32(self)
    }
    fn to_uint32_f(self) -> u32 {
        super::value::to_uint32(self)
    }
}
