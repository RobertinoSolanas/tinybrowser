//! Load a page, ask it questions, paint it — the whole `Page` API in one run.
//!
//! `cargo run -p tb-browser --example page_demo`

use std::path::Path;
use tb_browser::{Engine, EngineConfig};

fn main() {
    let html = r#"<!doctype html>
<html><head><title>Smoke | test</title>
<style>body{font-family:sans-serif;color:#222} h1{color:#b00} .card{border:2px solid #36c;padding:8px;background:#efefff}
#hidden{visibility:hidden}</style></head>
<body>
<header><h1>Hello tinybrowser</h1></header>
<nav><a href="/one">One</a> <a href="https://example.com/two">Two</a></nav>
<div class="card"><p>Card paragraph with <strong>bold</strong> text.</p><ul><li>alpha</li><li>beta</li></ul></div>
<p id="hidden">invisible</p>
<img src="pic.png" alt="A picture">
<script>
  document.title = document.title.toUpperCase();
  const p = document.createElement('p');
  p.id = 'made-by-js';
  p.textContent = 'created by script';
  document.body.appendChild(p);
  console.log('scripts said hi', 1 + 1);
</script>
</body></html>"#;
    let mut browser = Engine::offline(EngineConfig::default());
    let mut page = browser.load_html("file:///tmp/smoke.html", html).unwrap();
    println!("title      = {}", page.title());
    println!("status     = {} {}", page.status, page.content_type);
    let s = page.stats();
    println!(
        "stats      = elements {} rules {} matched {} boxes {} scripts {} console {}",
        s.elements, s.css_rules, s.matched_elements, s.boxes, s.scripts_run, s.console_lines
    );
    println!("--- text ---\n{}", page.text());
    println!("--- links ---");
    for l in page.links() {
        println!("  {:?} -> {}", l.text, l.url);
    }
    println!("--- titles --- {:?}", page.titles());
    println!(
        "--- js --- {:?}",
        page.eval("document.querySelectorAll('li').length").unwrap()
    );
    println!(
        "made-by-js present: {}",
        page.eval("!!document.getElementById('made-by-js')")
            .unwrap()
            .to_string()
    );
    println!(
        "rect of h1: {:?}",
        page.eval("JSON.stringify(document.querySelector('h1').getBoundingClientRect())")
            .unwrap()
    );
    println!("timings: {}", page.timings);
    let n = page
        .screenshot(Path::new("/tmp/smoke.png"), &Default::default())
        .unwrap();
    println!("png bytes = {n}");
    println!("--- dom ---\n{}", page.dump_dom());
    println!("--- boxes ---\n{}", page.dump_layout());
}
