//! `<script>` execution: order, types, external sources, ready state, timers.

mod common;

use common::Page;
use tb_dom_bindings::ScriptOptions;

const X: &str = "http://example.test/p/x.js";

#[test]
fn inline_scripts_run_in_document_order() {
    let mut p = Page::html(
        "<script>window.o = []; window.o.push(1);</script>\
         <script>window.o.push(2)</script><script>window.o.push(3);</script>",
    );
    let report = p.run(&[]);
    assert_eq!(report.executed, 3);
    assert!(report.clean());
    assert_eq!(p.text("window.o.join(',')"), "1,2,3");
}

#[test]
fn external_scripts_are_fetched_and_run_in_order() {
    let mut p = Page::html(
        "<script src=a.js></script><script>window.o.push('inline');</script><script src=b.js></script>",
    );
    let report = p.run(&[
        ("http://example.test/p/a.js", "window.o = ['a'];"),
        ("http://example.test/p/b.js", "window.o.push('b');"),
    ]);
    assert_eq!(report.executed, 3);
    assert!(report.clean());
    assert_eq!(p.text("window.o.join(',')"), "a,inline,b");
}

#[test]
fn a_missing_script_is_reported_and_carries_on() {
    let mut p = Page::html("<script src=gone.js></script><script>window.after = true;</script>");
    let report = p.run(&[]);
    assert_eq!(report.skipped, 1);
    assert_eq!(report.executed, 1);
    assert!(p.yes("window.after"));
    let problems = p.b.problems();
    assert!(
        problems
            .iter()
            .any(|p| p.message.contains("script not loaded")),
        "{problems:?}"
    );
}

#[test]
fn script_errors_are_counted_not_fatal() {
    let mut p = Page::html(
        "<script>window.x = 1; undefinedFunction();</script><script>window.y = 2;</script>",
    );
    let report = p.run(&[]);
    assert_eq!(report.executed, 2);
    assert_eq!(report.failed, 1);
    assert!(!report.clean());
    assert_eq!(p.num("window.y"), 2.0);
}

#[test]
fn script_types() {
    let mut p = Page::html(
        "<script type='text/javascript'>window.a = 1;</script>\
         <script type='application/json'>{ \"x\": 1 }</script>\
         <script type='module'>window.mod = true;</script>\
         <script type='text/template'>not code</script>\
         <script type='application/javascript'>window.b = 2;</script>",
    );
    let report = p.run(&[]);
    assert_eq!(report.executed, 2, "only the two classic scripts");
    assert_eq!(p.num("window.a"), 1.0);
    assert_eq!(p.num("window.b"), 2.0);
    assert!(
        p.yes("typeof window.mod === 'undefined'"),
        "no module loader"
    );
    let problems = p.b.problems();
    assert!(
        problems.iter().any(|p| p.message.contains("module")),
        "the skipped module must be reported: {problems:?}"
    );
}

#[test]
fn scripts_that_add_scripts_run_in_the_next_pass() {
    let mut p = Page::html("<div id=d></div>");
    p.js("const s = document.createElement('script');\
         s.textContent = \"window.added = true;\";\
         document.body.appendChild(s);");
    let report = p.run(&[]);
    assert!(report.executed >= 1);
    assert!(p.yes("window.added === true"));
}

#[test]
fn ready_state_and_load_events() {
    let mut p = Page::html(
        "<script>window.seq = []; document.addEventListener('DOMContentLoaded', () => window.seq.push('dcl:' + document.readyState));\
         window.addEventListener('load', () => window.seq.push('load:' + document.readyState));\
         window.seq.push('body:' + document.readyState);</script>",
    );
    p.run(&[]);
    assert_eq!(
        p.text("window.seq.join(',')"),
        "body:loading,dcl:interactive,load:complete"
    );
    assert_eq!(p.text("document.readyState"), "complete");
}

#[test]
fn document_write_appends_to_the_body() {
    let mut p = Page::html("<div id=d>x</div><script>document.write('<p>written</p>');</script>");
    p.run(&[]);
    assert_eq!(p.text_of("p"), "written");
    assert!(p.body_html().contains("<p>written</p>"));
}

#[test]
fn timers_run_on_a_virtual_clock() {
    let mut p = Page::html(
        "<script>window.order = [];\
         setTimeout(() => window.order.push('t10'), 10);\
         setTimeout(() => window.order.push('t0'), 0);\
         setTimeout(() => window.order.push('t20'), 20);</script>",
    );
    p.run(&[]);
    // the default budget covers `setTimeout(…, 0)` of this page load
    assert_eq!(p.text("window.order.join(',')"), "t0");
    p.pump(50.0);
    assert_eq!(p.text("window.order.join(',')"), "t0,t10,t20");
}

#[test]
fn microtasks_beat_timers() {
    let mut p = Page::html(
        "<script>window.order = [];\
         setTimeout(() => window.order.push('timer'), 0);\
         Promise.resolve().then(() => window.order.push('micro'));\
         queueMicrotask(() => window.order.push('queued'));</script>",
    );
    let report = p.run(&[]);
    assert!(report.clean());
    assert_eq!(p.text("window.order.join(',')"), "micro,queued,timer");
}

#[test]
fn interval_repeats_until_cleared() {
    let mut p = Page::html(
        "<script>window.n = 0;\
         const id = setInterval(() => { window.n++; if (window.n === 3) clearInterval(id); }, 10);</script>",
    );
    p.run(&[]);
    p.pump(1_000.0);
    assert_eq!(p.num("window.n"), 3.0);
}

#[test]
fn animation_frames_are_timers() {
    let mut p = Page::html(
        "<script>window.framesSeen = 0; requestAnimationFrame(() => { window.framesSeen++; });</script>",
    );
    p.run(&[]);
    p.pump(100.0);
    assert_eq!(p.num("window.framesSeen"), 1.0);
}

#[test]
fn script_sees_the_finished_parse() {
    // a classic script at the end of <body> already sees the whole document,
    // which is what every "script at the bottom of the page" layout relies on
    let mut p = Page::html(
        "<div id=a>1</div><div id=b>2</div><script>window.count = document.querySelectorAll('div').length;</script>",
    );
    p.run(&[]);
    assert_eq!(p.num("window.count"), 2.0);
}

#[test]
fn option_flags_can_silence_the_events() {
    let mut p = Page::html(
        "<script>window.fired = 0; document.addEventListener('DOMContentLoaded', () => window.fired++);</script>",
    );
    p.run_in(
        ScriptOptions {
            fire_events: false,
            ..Default::default()
        },
        &[],
    );
    assert_eq!(p.num("window.fired"), 0.0);
    assert_eq!(p.text("document.readyState"), "loading");
}

#[test]
fn script_writes_are_visible_to_a_second_run() {
    let mut p = Page::html("<div id=d></div><script src=x.js></script>");
    let first = p.run(&[(X, "document.getElementById('d').innerHTML = '<b>1</b>';")]);
    assert!(first.clean());
    assert_eq!(p.text_of("b"), "1");
}

#[test]
fn async_and_defer_attributes_are_accepted() {
    // we run every classic script in document order; the attributes must not
    // make a script vanish or throw
    let mut p = Page::html("<script defer src=a.js></script><script async src=b.js></script>");
    let report = p.run(&[
        ("http://example.test/p/a.js", "window.a = true;"),
        ("http://example.test/p/b.js", "window.b = true;"),
    ]);
    assert_eq!(report.executed, 2);
    assert!(p.yes("window.a && window.b"));
}
