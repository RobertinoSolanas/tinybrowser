//! Promises, microtasks, timers and the console.
//!
//! `await` is implemented by pumping the event loop synchronously: the async
//! body runs to completion while `await` drains microtasks (and, if nothing is
//! queued, advances the virtual clock to the next timer). That makes the very
//! common `await new Promise(r => setTimeout(r, 10))` work without green
//! threads; ordering can differ from a real engine in exotic cases.

use std::cell::RefCell;
use std::rc::Rc;

use super::runtime::{Interp, JsResult};
use super::value::{Key, Kind, Obj, ObjRef, Value};

/// Status/reaction slots live as non-enumerable internal properties.
pub const STATUS: &str = "@@status";
pub const VALUE: &str = "@@value";
pub const REACTIONS: &str = "@@reactions";

pub fn is_promise(o: &ObjRef) -> bool {
    o.borrow().get_own(&Key::Name(Rc::from(STATUS))).is_some()
}

fn status_of(p: &ObjRef) -> String {
    p.borrow()
        .get_own(&Key::Name(Rc::from(STATUS)))
        .and_then(|pr| pr.value().cloned())
        .map(|v| v.to_string())
        .unwrap_or_else(|| "pending".into())
}

fn value_of(p: &ObjRef) -> Value {
    p.borrow()
        .get_own(&Key::Name(Rc::from(VALUE)))
        .and_then(|pr| pr.value().cloned())
        .unwrap_or(Value::Undefined)
}

fn reactions_of(p: &ObjRef) -> Vec<Value> {
    let slot = p
        .borrow()
        .get_own(&Key::Name(Rc::from(REACTIONS)))
        .and_then(|pr| pr.value().cloned());
    match slot.and_then(|v| v.as_object().cloned()) {
        Some(o) => o.borrow().items().cloned().unwrap_or_default(),
        None => Vec::new(),
    }
}

fn push_reaction(p: &ObjRef, reaction: ObjRef) {
    let arr = {
        let existing = p
            .borrow()
            .get_own(&Key::Name(Rc::from(REACTIONS)))
            .and_then(|pr| pr.value().cloned());
        match existing.and_then(|v| v.as_object().cloned()) {
            Some(a) => a,
            None => {
                let a = Rc::new(RefCell::new(Obj::array()));
                p.borrow_mut()
                    .define_internal(REACTIONS, Value::Object(a.clone()));
                a
            }
        }
    };
    {
        let mut b = arr.borrow_mut();
        if let Kind::Array { items } = &mut b.kind {
            items.push(Value::Object(reaction));
        }
    }
}

/// Attach a reaction object from another module (`Promise.all` & friends).
pub fn push_reaction_public(p: &ObjRef, reaction: ObjRef) {
    push_reaction(p, reaction)
}

fn drop_reactions(p: &ObjRef) {
    let key = Key::Name(Rc::from(REACTIONS));
    // NOTE: keep the `borrow()` temporary out of the `if let` scrutinee — in
    // Rust 2021 it lives through the whole body and would make `borrow_mut()`
    // panic with "already borrowed".
    let at = p.borrow().find(&key);
    if let Some(i) = at {
        p.borrow_mut().props.remove(i);
    }
}

impl Interp {
    /// A pending promise plus its `resolve`/`reject` functions.
    pub fn new_promise(&mut self) -> (ObjRef, ObjRef, ObjRef) {
        let mut o = Obj::plain();
        o.kind = Kind::Plain;
        o.proto = self.natives.promise_proto.clone();
        let p = Rc::new(RefCell::new(o));
        Self::init_promise(&p);
        let (res, rej) = self.settle_hooks(&p);
        (p, res, rej)
    }

    /// Mark an object as a promise (so `new Promise()` can reuse its `this`).
    pub fn init_promise(p: &ObjRef) {
        let mut b = p.borrow_mut();
        b.define_internal(STATUS, Value::string("pending"));
        b.define_internal(VALUE, Value::Undefined);
    }

    /// `resolve`/`reject` functions bound to an existing promise object.
    pub fn settle_hooks(&mut self, p: &ObjRef) -> (ObjRef, ObjRef) {
        let res = {
            let pp = p.clone();
            self.native_fn("resolve", 1, move |it, _this, mut args| {
                let v = args.drain(..).next().unwrap_or(Value::Undefined);
                it.resolve(&pp, &v);
                Ok(Value::Undefined)
            })
        };
        let rej = {
            let pp = p.clone();
            self.native_fn("reject", 1, move |it, _this, mut args| {
                let v = args.drain(..).next().unwrap_or(Value::Undefined);
                it.reject(&pp, &v);
                Ok(Value::Undefined)
            })
        };
        (
            res.as_object().unwrap().clone(),
            rej.as_object().unwrap().clone(),
        )
    }

    /// Fulfil a promise with a value (adopting thenables).
    pub fn resolve(&mut self, p: &ObjRef, v: &Value) {
        if status_of(p) != "pending" {
            return;
        }
        // thenable? adopt its state through `then`
        if let Some(o) = v.as_object() {
            if is_promise(o) {
                match status_of(o).as_str() {
                    "fulfilled" => {
                        let inner = value_of(o);
                        self.settle(p, "fulfilled", &inner);
                        return;
                    }
                    "rejected" => {
                        let inner = value_of(o);
                        self.settle(p, "rejected", &inner);
                        return;
                    }
                    _ => {
                        let target = p.clone();
                        let ok = {
                            let t = target.clone();
                            Rc::new(move |it: &mut Interp, _th, args: Vec<Value>| {
                                let v = args.into_iter().next().unwrap_or(Value::Undefined);
                                it.resolve(&t, &v);
                                Ok(Value::Undefined)
                            })
                        };
                        let err = {
                            let t = target.clone();
                            Rc::new(move |it: &mut Interp, _th, args: Vec<Value>| {
                                let v = args.into_iter().next().unwrap_or(Value::Undefined);
                                it.reject(&t, &v);
                                Ok(Value::Undefined)
                            })
                        };
                        self.then_call(o, ok, err);
                        return;
                    }
                }
            }
            // non-promise thenable: call `then` with our hooks
            let then = self.get(v, &Key::Name(Rc::from("then"))).ok();
            if let Some(tf) = then.and_then(|t| t.as_object().cloned()) {
                if tf.borrow().is_function() {
                    let target = p.clone();
                    let ok = {
                        let t = target.clone();
                        self.native_fn("resolve", 1, move |it, _th, mut args| {
                            let v = args.drain(..).next().unwrap_or(Value::Undefined);
                            it.resolve(&t, &v);
                            Ok(Value::Undefined)
                        })
                    };
                    let err = {
                        let t = target.clone();
                        self.native_fn("reject", 1, move |it, _th, mut args| {
                            let v = args.drain(..).next().unwrap_or(Value::Undefined);
                            it.reject(&t, &v);
                            Ok(Value::Undefined)
                        })
                    };
                    self.call(&tf, v.clone(), vec![ok, err]).ok();
                    return;
                }
            }
        }
        self.settle(p, "fulfilled", v);
    }

    pub fn reject(&mut self, p: &ObjRef, e: &Value) {
        if status_of(p) != "pending" {
            return;
        }
        self.settle(p, "rejected", e);
    }

    fn settle(&mut self, p: &ObjRef, st: &str, v: &Value) {
        {
            let mut b = p.borrow_mut();
            b.define_internal(STATUS, Value::string(st));
            b.define_internal(VALUE, v.clone());
        }
        let st: String = st.to_string();
        let reactions = reactions_of(p);
        drop_reactions(p);
        for rv in reactions {
            let (on_ok, on_err, target) = match rv.as_object() {
                Some(r) => {
                    let b = r.borrow();
                    let get = |n: &str| {
                        b.get_own(&Key::Name(Rc::from(n)))
                            .and_then(|pr| pr.value().cloned())
                            .unwrap_or(Value::Undefined)
                    };
                    (get("ok"), get("err"), get("target"))
                }
                None => continue,
            };
            let value = v.clone();
            let ok_h = on_ok.clone();
            let err_h = on_err.clone();
            let st2 = st.clone();
            let job = move |it: &mut Interp| {
                let handler = if st2 == "fulfilled" {
                    ok_h.clone()
                } else {
                    err_h.clone()
                };
                let out = match handler.as_object() {
                    Some(ho) if ho.borrow().is_function() => {
                        it.call(ho, Value::Undefined, vec![value.clone()])
                    }
                    _ if st2 == "fulfilled" => Ok(value.clone()),
                    _ => Err(value.clone()),
                };
                match out {
                    Ok(res) => {
                        if let Some(t) = target.as_object() {
                            it.resolve(t, &res);
                        }
                    }
                    Err(err) => {
                        if let Some(t) = target.as_object() {
                            it.reject(t, &err);
                        }
                    }
                }
            };
            self.queue.push_back(Rc::new(job));
        }
    }

    /// `promise.then(ok, err)` with native hooks (used when adopting).
    fn then_call(
        &mut self,
        p: &ObjRef,
        ok: Rc<super::runtime::NativeFn>,
        err: Rc<super::runtime::NativeFn>,
    ) {
        let okv = self.native_fn_rc("onFulfilled", 1, ok);
        let errv = self.native_fn_rc("onRejected", 1, err);
        let reaction = {
            let mut r = Obj::plain();
            r.define_internal("ok", okv);
            r.define_internal("err", errv);
            Rc::new(RefCell::new(r))
        };
        push_reaction(p, reaction);
        let st = status_of(p);
        if st != "pending" {
            let v = value_of(p);
            self.settle(p, &st, &v);
        }
    }

    /// `Promise.prototype.then` implementation.
    pub fn promise_then(&mut self, this: &Value, on_ok: Value, on_err: Value) -> JsResult<Value> {
        let p = match this.as_object() {
            Some(o) => o.clone(),
            None => return Err(self.type_error("then called on a non-promise")),
        };
        let (target, _res, _rej) = self.new_promise();
        let ok = on_ok
            .as_object()
            .cloned()
            .filter(|o| o.borrow().is_function());
        let err = on_err
            .as_object()
            .cloned()
            .filter(|o| o.borrow().is_function());
        let reaction = {
            let mut r = Obj::plain();
            if let Some(o) = ok {
                r.define_internal("ok", Value::Object(o));
            }
            if let Some(e) = err {
                r.define_internal("err", Value::Object(e));
            }
            r.define_internal("target", Value::Object(target.clone()));
            Rc::new(RefCell::new(r))
        };
        let st = status_of(&p);
        if st == "pending" {
            push_reaction(&p, reaction);
        } else {
            let v = value_of(&p);
            let reaction2 = reaction.clone();
            let target2 = target.clone();
            let job = move |it: &mut Interp| {
                let handler = if st == "fulfilled" {
                    reaction2
                        .borrow()
                        .get_own(&Key::Name(Rc::from("ok")))
                        .and_then(|pr| pr.value().cloned())
                } else {
                    reaction2
                        .borrow()
                        .get_own(&Key::Name(Rc::from("err")))
                        .and_then(|pr| pr.value().cloned())
                };
                let out = match handler {
                    Some(h) => match h.as_object() {
                        Some(ho) => it.call(ho, Value::Undefined, vec![v.clone()]),
                        None => Ok(v.clone()),
                    },
                    None if st == "fulfilled" => Ok(v.clone()),
                    None => Err(v.clone()),
                };
                match out {
                    Ok(res) => it.resolve(&target2, &res),
                    Err(e) => it.reject(&target2, &e),
                }
            };
            self.queue.push_back(Rc::new(job));
        }
        Ok(Value::Object(target))
    }

    /// Run microtasks until the queue drains (or the budget is used up).
    pub fn run_jobs(&mut self, budget: usize) {
        let mut n = 0;
        while let Some(job) = self.queue.pop_front() {
            n += 1;
            if n > budget {
                self.note("microtask queue did not drain");
                break;
            }
            job(self);
        }
    }

    pub fn run_one_job(&mut self) -> bool {
        match self.queue.pop_front() {
            Some(job) => {
                job(self);
                true
            }
            None => false,
        }
    }

    /// Advance the virtual clock to the next due timer and run it.
    /// Returns false when no timer is available.
    pub fn run_next_timer(&mut self) -> bool {
        if self.timers.is_empty() {
            return false;
        }
        let (idx, due) = self
            .timers
            .iter()
            .enumerate()
            .map(|(i, t)| (i, t.due))
            .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .unwrap();
        if due > self.clock {
            self.clock = due;
        }
        let timer = self.timers.remove(idx);
        let every = timer.every;
        if let Some(ms) = every {
            self.timers.push(super::runtime::Timer {
                id: timer.id,
                due: self.clock + ms.max(1.0),
                every: Some(ms),
                cb: timer.cb.clone(),
                args: timer.args.clone(),
            });
        }
        let cb = timer.cb.clone();
        let args = timer.args.clone();
        if let Some(o) = cb.as_object() {
            if o.borrow().is_function() {
                if let Err(e) = self.call(o, self.globals_value(), args) {
                    let msg = self.format_thrown(&e);
                    self.console
                        .push(("error".into(), format!("Uncaught {msg}")));
                }
            }
        } else if let Value::Str(src) = &cb {
            // setTimeout("code") — legacy string form
            let s = src.to_string();
            if let Err(msg) = self.run_script(&s) {
                self.console.push(("error".into(), msg));
            }
        }
        // microtasks belong between timer callbacks
        self.run_jobs(10_000);
        true
    }

    /// Run all timers whose due time is within `horizon` ms of the clock.
    pub fn run_timers(&mut self, horizon: f64) -> usize {
        let until = self.clock + horizon;
        let mut n = 0;
        loop {
            self.run_jobs(10_000);
            let next_due = self
                .timers
                .iter()
                .map(|t| t.due)
                .fold(f64::INFINITY, f64::min);
            if next_due > until || self.timers.is_empty() {
                break;
            }
            if !self.run_next_timer() {
                break;
            }
            n += 1;
            if n > 5000 {
                self.note("timer loop hit its limit (setInterval?)");
                break;
            }
        }
        self.run_jobs(10_000);
        n
    }

    /// `await value`: pump the loop until the promise settles.
    pub fn await_value(&mut self, v: &Value) -> JsResult<Value> {
        let p = match v.as_object() {
            Some(o) if is_promise(o) => o.clone(),
            _ => {
                // awaiting a non-promise still yields once
                self.run_jobs(1000);
                return Ok(v.clone());
            }
        };
        // a promise that never settles must not wedge the engine
        const MAX_PUMPS: u32 = 1_000_000;
        let mut guard = 0u32;
        loop {
            match status_of(&p).as_str() {
                "fulfilled" => return Ok(value_of(&p)),
                "rejected" => return Err(value_of(&p)),
                _ => {}
            }
            guard += 1;
            if guard > MAX_PUMPS {
                self.note("await gave up after 1_000_000 pumps");
                break;
            }
            if self.run_one_job() {
                continue;
            }
            if !self.timers.is_empty() && self.run_next_timer() {
                continue;
            }
            break;
        }
        self.note("await could not be settled; continuing with undefined");
        Ok(Value::Undefined)
    }

    /// A promise already resolved with `v`.
    pub fn resolved_promise(&mut self, v: &Value) -> Value {
        let (p, _res, _rej) = self.new_promise();
        self.resolve(&p, v);
        Value::Object(p)
    }

    /// A promise already rejected with `e`.
    pub fn rejected_promise(&mut self, e: &Value) -> Value {
        let (p, _res, _rej) = self.new_promise();
        self.reject(&p, e);
        Value::Object(p)
    }

    pub fn clear_timer(&mut self, id: f64) {
        self.timers.retain(|t| t.id as f64 != id);
    }

    pub fn schedule(&mut self, ms: f64, every: Option<f64>, cb: Value, args: Vec<Value>) -> u32 {
        let id = self.next_timer;
        self.next_timer += 1;
        let delay = if every.is_some() {
            ms.max(1.0)
        } else {
            ms.max(0.0)
        };
        self.timers.push(super::runtime::Timer {
            id,
            due: self.clock + delay,
            every,
            cb,
            args,
        });
        id
    }

    // ------------------------------------------------------------- console

    pub fn log(&mut self, level: &str, text: String) {
        if self.echo_console {
            println!("{text}");
        }
        self.console.push((level.to_string(), text));
        if self.console.len() > 5000 {
            self.console.remove(0);
        }
    }

    /// Render a thrown value the way a browser console would.
    pub fn format_thrown(&mut self, e: &Value) -> String {
        match e.as_object() {
            Some(o) if matches!(o.borrow().kind, Kind::Error) => {
                let name = self
                    .get(e, &Key::Name(Rc::from("name")))
                    .map(|v| v.to_string())
                    .unwrap_or_else(|_| "Error".into());
                let msg = self
                    .get(e, &Key::Name(Rc::from("message")))
                    .map(|v| v.to_string())
                    .unwrap_or_default();
                if msg.is_empty() {
                    name
                } else {
                    format!("{name}: {msg}")
                }
            }
            _ => e.inspect(),
        }
    }
}

impl Interp {
    /// Does this object have a callable `then`?
    pub fn is_thenable(&mut self, v: &Value) -> bool {
        match self.get(v, &Key::Name(Rc::from("then"))) {
            Ok(t) => t
                .as_object()
                .map(|o| o.borrow().is_function())
                .unwrap_or(false),
            Err(_) => false,
        }
    }

    /// Call `value.then(ok, err)` with native hooks (adopting a thenable).
    pub fn resolve_thenable(&mut self, v: &Value, ok: &Value, err: &Value) {
        let f = match self.get(v, &Key::Name(Rc::from("then"))) {
            Ok(f) => f,
            Err(_) => return,
        };
        if let Some(fo) = f.as_object() {
            if fo.borrow().is_function() {
                self.call(fo, v.clone(), vec![ok.clone(), err.clone()]).ok();
            }
        }
    }
}
