//! # tb-js — the JavaScript engine
//!
//! Lexer → AST → tree-walking interpreter with objects, prototypes, closures,
//! `this` binding, exceptions, promises (microtask queue), timers and a console.
//!
//! ```
//! let mut rt = tb_js::Runtime::new();
//! let v = rt.eval("const a = [3,1,2].sort((x,y)=>x-y); a.join('-')").unwrap();
//! assert_eq!(v.to_string(), "1-2-3");
//! ```

pub mod ast;
pub mod builtins;
pub mod env;
pub mod exprs;
pub mod jobs;
pub mod json;
pub mod lexer;
pub mod parser;
pub mod regexp;
pub mod runtime;
pub mod std_array;
pub mod std_misc;
pub mod std_string;
pub mod stmts;
pub mod value;

pub use builtins::{arg, def};
pub use env::Env;
pub use jobs::{is_promise, STATUS as PROMISE_STATUS};
pub use lexer::{lex, LexError, Token};
pub use parser::{parse, ParseError, Parser};
pub use regexp::{MatchInfo, RegExp};
pub use runtime::{Flow, FnData, FnKind, Interp, JsResult, NativeFn};
pub use value::{Key, Kind, Obj, ObjRef, Prop, Value};

/// A ready-to-use script engine with a global scope.
///
/// ```
/// let mut rt = tb_js::Runtime::new();
/// let v = rt.eval("const a = [3,1,2].sort((x,y)=>x-y); a.join('-')").unwrap();
/// assert_eq!(v.to_string(), "1-2-3");
/// ```
pub struct Runtime {
    pub interp: Interp,
}

impl Runtime {
    pub fn new() -> Runtime {
        Runtime {
            interp: Interp::new(),
        }
    }

    /// Run a script; the value of its last expression statement is returned.
    pub fn eval(&mut self, src: &str) -> Result<Value, String> {
        self.interp.run_script(src)
    }

    /// Run a script and then drain microtasks and timers up to `ms`.
    pub fn eval_settled(&mut self, src: &str, ms: f64) -> Result<Value, String> {
        let v = self.interp.run_script(src)?;
        self.interp.run_timers(ms);
        Ok(v)
    }

    /// Everything `console.*` printed, oldest first.
    pub fn console(&self) -> Vec<String> {
        self.interp.console.iter().map(|(_, t)| t.clone()).collect()
    }

    pub fn console_level(&self, level: &str) -> Vec<String> {
        self.interp
            .console
            .iter()
            .filter(|(l, _)| l == level)
            .map(|(_, t)| t.clone())
            .collect()
    }

    pub fn problems(&self) -> &[String] {
        &self.interp.problems
    }

    /// Print console output to stdout as it happens.
    pub fn set_echo_console(&mut self, on: bool) {
        self.interp.echo_console = on;
    }

    /// The global object, for host bindings.
    pub fn globals(&self) -> ObjRef {
        self.interp.globals.clone()
    }

    /// Install a native function on the global object.
    pub fn register<F>(&mut self, name: &str, len: u32, f: F)
    where
        F: Fn(&mut Interp, Value, Vec<Value>) -> JsResult<Value> + 'static,
    {
        let v = self.interp.native_fn(name, len, f);
        self.interp.globals.borrow_mut().define_internal(name, v);
    }

    /// Let pending microtasks and timers run.
    pub fn tick(&mut self, ms: f64) {
        self.interp.run_jobs(100_000);
        self.interp.run_timers(ms);
    }
}

impl Default for Runtime {
    fn default() -> Self {
        Runtime::new()
    }
}
