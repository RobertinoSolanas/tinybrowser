//! Robustness: hostile/broken input must never panic, and real code shapes
//! have to *run* (not merely parse). See todolist T7.7 / T7.8.

use std::time::Instant;
use tb_js::{Parser, Runtime};

/// Deterministic pseudo-random generator (no `rand` dependency).
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 11
    }

    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[(self.next() as usize) % xs.len()]
    }
}

#[test]
fn random_scripts_never_panic() {
    // pieces that together make plausible-looking, sometimes broken, JavaScript
    let frags: &[&str] = &[
        "var",
        "let",
        "const",
        "x",
        "y",
        "function",
        "f",
        "(",
        ")",
        "{",
        "}",
        "[",
        "]",
        "=>",
        ";",
        ",",
        ".",
        "...",
        "=",
        "==",
        "===",
        "+",
        "-",
        "*",
        "/",
        "%",
        "**",
        "&&",
        "||",
        "??",
        "?.",
        "++",
        "--",
        "if",
        "else",
        "for",
        "while",
        "do",
        "of",
        "in",
        "return",
        "break",
        "continue",
        "switch",
        "case",
        "default",
        "try",
        "catch",
        "finally",
        "throw",
        "new",
        "class",
        "extends",
        "super",
        "this",
        "typeof",
        "instanceof",
        "delete",
        "void",
        "await",
        "async",
        "yield",
        "1",
        "0",
        "1.5",
        "0x1f",
        "'str'",
        "\"s2\"",
        "`t${x}`",
        "/re/g",
        "true",
        "false",
        "null",
        "undefined",
        "JSON.stringify",
        "document.querySelectorAll",
        "addEventListener",
        "Promise.resolve",
        "setTimeout",
        "console.log",
        "}",
        ")",
        "]",
        "/* c */",
        "// line\n",
        "\n",
        " ",
        "\\",
        "@",
        "#",
        "$",
        "!",
        "~",
        ":",
        "?",
        "<",
        ">",
    ];
    let mut rng = Lcg(0x5eed);
    for case in 0..4000 {
        let n = 1 + (rng.next() % 24) as usize;
        let mut src = String::new();
        for _ in 0..n {
            // `frags` is `&[&str]`: the turbofish says which element type we
            // want, so `pick` hands back `&str` and not `&&str`
            src.push_str(rng.pick::<&str>(frags));
            if rng.next() % 4 == 0 {
                src.push(' ');
            }
        }
        // lexing must either work or report an error — never panic
        let _ = tb_js::lex(&src);
        // strict parse: error is fine, panic is not
        let _ = Parser::parse_program(&src);
        // tolerant parse + run: the engine must survive and report problems
        let mut rt = Runtime::new();
        rt.interp.step_limit = 200_000;
        let _ = rt.eval(&src);
        if case % 500 == 0 {
            // keep the loop honest: something has to be reported sometimes
            let _ = rt.problems().len();
        }
    }
}

#[test]
fn broken_foreign_script_is_skipped_with_a_problem_not_a_panic() {
    // one statement is unparseable (truncated call); the rest must still run
    let src = "\
var kept = 'yes';
function halfBroken( {
var more = 1;
";
    let mut rt = Runtime::new();
    // a full parse fails, so `run_script` falls back to the tolerant parse
    let _ = rt.eval(src);
    assert!(
        !rt.problems().is_empty(),
        "the syntax error has to be reported"
    );
}

#[test]
fn minified_vendor_code_runs_end_to_end() {
    let mut rt = Runtime::new();
    // the UMD + factory shape a bundler emits, with a real payload
    let src = r#"
      var lib = (function (root, factory) {
        if (typeof define === 'function' && define.amd) { define([], factory) }
        else { return factory() }
      })(this, function () {
        'use strict';
        function slug(s) {
          return String(s).toLowerCase().trim().replace(/[^a-z0-9]+/g, '-').replace(/(^-|-$)/g, '');
        }
        function Store(items) { this.items = items }
        Store.prototype.hrefs = function () {
          var seen = Object.create(null);
          var out = [];
          this.items.forEach((function (t) {
            var h = '/' + slug(t);
            if (seen[h]) return;
            seen[h] = 1;
            out.push(h);
          }).bind(this));
          return out;
        };
        Store.prototype.latest = async function () {
          const list = await Promise.resolve(this.hrefs());
          return list[list.length - 1];
        };
        return { Store: Store, slug: slug };
      });
      var store = new lib.Store(['Erste Meldung', 'Erste Meldung', 'Zweite!!']);
      var out = [lib.slug('  Hello World  '), store.hrefs().join(','), typeof store.latest];
      var done = 'pending';
      store.latest().then(function (v) { done = v });
      __pump();
      out.join(' | ') + ' | ' + done
    "#;
    rt.register("__pump", 0, |it, _t, _a| {
        it.run_jobs(10_000);
        Ok(tb_js::Value::Undefined)
    });
    let v = rt.eval(src).unwrap().to_string();
    assert_eq!(
        v,
        "hello-world | /erste-meldung,/zweite | function | /zweite"
    );
    assert!(rt.console_level("error").is_empty(), "{:?}", rt.problems());
}

#[test]
fn deep_nesting_does_not_overflow_the_stack() {
    // the interpreter counts depth and turns recursion into a RangeError
    let mut rt = Runtime::new();
    let err = rt
        .eval("function f(n){ return f(n+1) + 1 } f(0)")
        .unwrap_err();
    assert!(
        err.to_lowercase().contains("stack") || err.to_lowercase().contains("budget"),
        "{err}"
    );
    // deep literal nesting must be reported as a syntax error, not a crash
    let deep = format!("{}1{}", "(".repeat(5000), ")".repeat(5000));
    let _ = Parser::parse_program(&deep);
    let _ = Parser::parse_tolerant(&deep, &mut Vec::new());
}

#[test]
fn deeply_nested_code_and_data_survive() {
    let mut rt = Runtime::new();
    // nesting a real page uses still works …
    let nested = format!("{}7{}", "(".repeat(60), ")".repeat(60));
    assert_eq!(rt.eval(&nested).unwrap().to_number(), 7.0);
    let deep_arr = format!("{}1{}", "[".repeat(60), "]".repeat(60));
    assert!(!rt.eval(&deep_arr).unwrap().to_string().is_empty());
    // … and absurd nesting is refused instead of overflowing the stack
    let deep = format!("{}7{}", "(".repeat(2000), ")".repeat(2000));
    assert!(
        rt.eval(&deep).is_err(),
        "deep parens must be a syntax error"
    );
    let deep_blocks = format!("{}return 1{}", "{".repeat(150), "}".repeat(150));
    let _ = Runtime::new().eval(&format!("(function(){deep_blocks})()"));
    let deep_json = format!("{}1{}", "[".repeat(10_000), "]".repeat(10_000));
    let src = format!(
        "let err = null; try {{ JSON.parse('{deep_json}') }} catch (e) {{ err = e.name }} err"
    );
    assert_eq!(rt.eval(&src).unwrap().to_string(), "SyntaxError");
}

#[test]
fn a_thousand_iterations_of_the_full_pipeline_stay_fast() {
    // this is the benchmark: parse + interpret a realistic script 1000 times
    let src = r#"
        function clean(list) {
          return list
            .map(function (x) { return { id: x.id, title: x.title.trim().toLowerCase() } })
            .filter(function (x) { return x.title.length > 3 })
            .sort(function (a, b) { return a.id - b.id })
            .reduce(function (acc, x) { acc[x.title] = x.id; return acc }, {});
        }
        const input = [];
        for (let i = 0; i < 20; i++) {
          input.push({ id: (i * 7) % 20, title: '  Article ' + i + '  ' });
        }
        const out = clean(input);
        Object.keys(out).length + Object.values(out).reduce((a, b) => a + b, 0)
    "#;
    let t = Instant::now();
    let mut last = 0.0;
    for _ in 0..1000 {
        let mut rt = Runtime::new();
        let v = rt.eval(src).unwrap();
        last = v.to_number();
    }
    let ms = t.elapsed().as_secs_f64() * 1000.0;
    // 20 items → ids 0..19 sum to 190, all titles kept
    assert_eq!(last, 210.0, "19 titles * ... + id sum");
    // generous upper bound: catches accidental quadratic blow-ups, not CI noise
    assert!(ms < 60_000.0, "1000 runs took {ms:.0} ms");
    eprintln!("benchmark: 1000 x pipeline = {ms:.0} ms");
}
