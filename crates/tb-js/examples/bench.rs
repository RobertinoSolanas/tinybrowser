//! Parse benchmark for the JS front end.
//!
//! ```console
//! cargo run --release -p tb-js --example bench -- fixtures/script.js 20
//! cargo run --release -p tb-js --example bench -- "" 200   # synthetic workload
//! ```
//!
//! Prints lexing, strict parsing and tolerant parsing timings plus how many
//! syntax errors the tolerant pass had to skip — the number that says whether
//! real vendor bundles go through our parser.

use std::time::Instant;
use tb_js::Parser;

const SYNTHETIC: &str = r#"
    'use strict';
    function build(n) {
      const out = [];
      for (let i = 0; i < n; i++) {
        out.push({ id: i, title: 'Article ' + i, tags: ['a', 'b'].slice(0, (i % 2) + 1) });
      }
      return out;
    }
    const list = build(200);
    const byTag = list.reduce(function (acc, item) {
      item.tags.forEach(function (t) { (acc[t] = acc[t] || []).push(item.id) });
      return acc;
    }, {});
    Object.keys(byTag).map(k => k + ':' + byTag[k].length).join(' ');
"#;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut src = String::new();
    match args.first().map(|s| s.as_str()) {
        Some("") | None => src.push_str(SYNTHETIC),
        Some(path) => match std::fs::read_to_string(path) {
            Ok(s) => src = s,
            Err(e) => {
                eprintln!("cannot read {path}: {e}");
                std::process::exit(1);
            }
        },
    }
    let iters: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(50);

    let t = Instant::now();
    for _ in 0..iters {
        match tb_js::lex(&src) {
            Ok(toks) => std::hint::black_box(toks.len()),
            Err(e) => {
                std::hint::black_box(e.message);
                0
            }
        };
    }
    let lex = t.elapsed().as_secs_f64() * 1000.0 / iters as f64;

    let t = Instant::now();
    let mut strict_err = None;
    for _ in 0..iters {
        match Parser::parse_program(&src) {
            Ok(p) => std::hint::black_box(p.body.len()),
            Err(e) => {
                strict_err = Some(e.to_string());
                0
            }
        };
    }
    let parse = t.elapsed().as_secs_f64() * 1000.0 / iters as f64;

    let t = Instant::now();
    let mut problems = Vec::new();
    let mut kept = 0;
    for _ in 0..iters {
        let prog = Parser::parse_tolerant(&src, &mut problems);
        kept = prog.body.len();
    }
    let tolerant = t.elapsed().as_secs_f64() * 1000.0 / iters as f64;
    problems.dedup();

    println!(
        "{:>8} KB · lex {:>6.2} ms · parse {:>6.2} ms · tolerant {:>6.2} ms · {} statements kept",
        src.len() / 1024,
        lex,
        parse,
        tolerant,
        kept
    );
    match strict_err {
        None => println!("strict parse: clean"),
        Some(e) => println!(
            "strict parse: {e} — tolerant pass skipped {} statement(s), e.g. {:?}",
            problems.len(),
            problems.first()
        ),
    }
}
