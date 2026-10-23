//! Events: capture → target → bubble, the `on*` handlers, propagation control.

mod common;

use common::Page;
use tb_dom_bindings::EvtTarget;

#[test]
fn listener_fires_and_sees_the_event() {
    let mut p = Page::html("<button id=b>Go</button>");
    p.js("window.log = [];\
         document.getElementById('b').addEventListener('click', e => {\
           window.log.push(e.type + ':' + e.target.tagName);\
         });");
    let t = p.target("#b");
    p.fire(t, "click");
    assert_eq!(p.text("window.log.join('|')"), "click:BUTTON");
    assert_eq!(p.b.listener_count(), 1);
}

#[test]
fn capture_then_target_then_bubble() {
    let mut p = Page::html("<div id=outer><div id=inner><span id=leaf>x</span></div></div>");
    p.js(
        "window.log = [];\
         const push = (s) => window.log.push(s);\
         document.getElementById('outer').addEventListener('click', () => push('outer-capture'), true);\
         document.getElementById('outer').addEventListener('click', () => push('outer-bubble'));\
         document.getElementById('inner').addEventListener('click', () => push('inner-capture'), true);\
         document.getElementById('inner').addEventListener('click', () => push('inner-bubble'));\
         document.getElementById('leaf').addEventListener('click', () => push('leaf-target'));",
    );
    let leaf = p.target("#leaf");
    p.fire(leaf, "click");
    assert_eq!(
        p.text("window.log.join(',')"),
        "outer-capture,inner-capture,leaf-target,inner-bubble,outer-bubble"
    );
}

#[test]
fn target_and_current_target() {
    let mut p = Page::html("<div id=a><b id=b>x</b></div>");
    p.js("window.seen = [];\
         document.getElementById('a').addEventListener('click', e => {\
           window.seen.push(e.target.id + '/' + e.currentTarget.id + '/' + e.eventPhase);\
         });");
    let b = p.target("#b");
    p.fire(b, "click");
    assert_eq!(p.text("window.seen.join(',')"), "b/a/3");
}

#[test]
fn registration_is_deduped_and_removal_works() {
    let mut p = Page::html("<div id=d>x</div>");
    p.js("window.n = 0;\
         window.h = () => window.n++;\
         const d = document.getElementById('d');\
         d.addEventListener('click', window.h);\
         d.addEventListener('click', window.h);\
         window.count1 = window.n;");
    let d = p.target("#d");
    p.fire(d, "click");
    assert_eq!(p.num("window.n"), 1.0);
    p.js("document.getElementById('d').removeEventListener('click', window.h);");
    p.fire(d, "click");
    assert_eq!(p.num("window.n"), 1.0);
    assert_eq!(p.num("window.count1"), 0.0);
}

#[test]
fn capture_and_bubble_are_separate_slots() {
    let mut p = Page::html("<div id=d>x</div>");
    p.js("window.n = 0;\
         window.h = () => window.n++;\
         const d = document.getElementById('d');\
         d.addEventListener('click', window.h, true);\
         d.addEventListener('click', window.h, false);");
    let d = p.target("#d");
    p.fire(d, "click");
    assert_eq!(p.num("window.n"), 2.0);
    p.js("document.getElementById('d').removeEventListener('click', window.h, true);");
    p.fire(d, "click");
    assert_eq!(p.num("window.n"), 3.0);
}

#[test]
fn once_listener_runs_once() {
    let mut p = Page::html("<div id=d>x</div>");
    p.js(
        "window.n = 0;\
         document.getElementById('d').addEventListener('click', () => window.n++, { once: true });",
    );
    let d = p.target("#d");
    p.fire(d, "click");
    p.fire(d, "click");
    assert_eq!(p.num("window.n"), 1.0);
}

#[test]
fn stop_propagation_and_immediate() {
    let mut p = Page::html("<div id=o><div id=i>x</div></div>");
    p.js(
        "window.log = [];\
         document.getElementById('o').addEventListener('click', () => window.log.push('o'));\
         document.getElementById('i').addEventListener('click', e => { window.log.push('i'); e.stopPropagation(); });",
    );
    let i = p.target("#i");
    p.fire(i, "click");
    assert_eq!(p.text("window.log.join(',')"), "i");

    p.js("window.log2 = []; document.getElementById('i').addEventListener('click', e => { window.log2.push('first'); e.stopImmediatePropagation(); }); document.getElementById('i').addEventListener('click', () => window.log2.push('second'));");
    p.fire(i, "click");
    assert_eq!(p.text("window.log2.join(',')"), "first");
}

#[test]
fn prevent_default_is_reported_to_the_dispatcher() {
    let mut p = Page::html("<a id=a href='#x'>go</a>");
    p.js("document.getElementById('a').addEventListener('click', e => e.preventDefault());");
    let a = p.target("#a");
    let allow = p.fire(a, "click");
    assert!(
        !allow.to_boolean(),
        "a prevented click must not default-navigate"
    );
    p.js("document.getElementById('a').addEventListener('click', () => {});");
    let a2 = p.target("#a");
    // the first handler still prevents; removing one of two does not help
    assert!(!p.fire(a2, "click").to_boolean());
    assert!(p.yes("typeof Event !== 'undefined'"));
}

#[test]
fn onclick_attribute_and_property() {
    let mut p = Page::html("<div id=d onclick=\"window.hit = (window.hit||0) + 1\">x</div>");
    let d = p.target("#d");
    p.fire(d, "click");
    assert_eq!(p.num("window.hit"), 1.0);

    p.js("document.getElementById('d').onclick = () => { window.hit2 = (window.hit2||0) + 1; };");
    p.fire(d, "click");
    assert_eq!(p.num("window.hit"), 2.0);
    assert_eq!(p.num("window.hit2"), 1.0);

    // assigning null removes the property handler
    p.js("document.getElementById('d').onclick = null;");
    p.fire(d, "click");
    assert_eq!(p.num("window.hit"), 3.0);
    assert_eq!(p.num("window.hit2"), 1.0);
}

#[test]
fn delegation_with_closest() {
    let mut p = Page::html("<ul id=list><li data-i=1>one</li><li data-i=2>two</li></ul>");
    p.js("window.picked = [];\
         document.getElementById('list').addEventListener('click', e => {\
           const li = e.target.closest('li');\
           if (li) window.picked.push(li.dataset.i);\
         });");
    // a click on a <b> inside the second <li> still finds the <li>
    p.js("document.querySelector('#list li:last-child').innerHTML = '<b id=deep>two</b>';");
    let deep = p.target("#deep");
    p.fire(deep, "click");
    assert_eq!(p.text("window.picked.join(',')"), "2");
}

#[test]
fn window_and_document_listeners() {
    let mut p = Page::html("<div id=d>x</div>");
    p.js("window.hits = [];\
         window.addEventListener('resize', () => window.hits.push('win-resize'));\
         document.addEventListener('visibilitychange', () => window.hits.push('doc-vis'));\
         document.addEventListener('click', () => window.hits.push('doc-click'));");
    p.fire(EvtTarget::Window, "resize");
    let doc = EvtTarget::node(p.document_id());
    p.fire(doc, "visibilitychange");
    assert_eq!(p.text("window.hits.join(',')"), "win-resize,doc-vis");
}

#[test]
fn event_objects_from_js() {
    let mut p = Page::html("<div id=d>x</div>");
    p.js(
        "window.got = null;\
         document.getElementById('d').addEventListener('ping', e => { window.got = e.detail.n + '|' + e.bubbles + '|' + e.cancelable; });",
    );
    p.js(
        "document.getElementById('d').dispatchEvent(new CustomEvent('ping', { detail: { n: 7 }, bubbles: true, cancelable: true }));",
    );
    assert_eq!(p.text("window.got"), "7|true|true");
}

#[test]
fn plain_object_as_event() {
    let mut p = Page::html("<div id=d>x</div>");
    p.js("window.seen = ''; document.getElementById('d').addEventListener('hi', e => { window.seen = e.type; });");
    let d = p.target("#d");
    let js = format!(
        "document.getElementById('d').dispatchEvent({{ type: 'hi', bubbles: false, cancelable: false }})"
    );
    let r = p.js(&js);
    assert!(r.to_boolean());
    assert_eq!(p.text("window.seen"), "hi");
    let _ = d;
}

#[test]
fn dispatch_returns_default_prevented_state() {
    let mut p = Page::html("<div id=d>x</div>");
    p.js("document.getElementById('d').addEventListener('go', e => e.preventDefault());");
    assert!(!p
        .js("document.getElementById('d').dispatchEvent(new Event('go', { cancelable: true }))")
        .to_boolean());
    assert!(p
        .js("document.getElementById('d').dispatchEvent(new Event('go2'))")
        .to_boolean());
}

#[test]
fn event_target_is_the_dispatch_target() {
    let mut p = Page::html("<div id=a><span id=b>x</span></div>");
    p.js(
        "window.ids = [];\
         document.getElementById('a').addEventListener('click', e => window.ids.push(e.target.id));",
    );
    let b = p.target("#b");
    p.fire(b, "click");
    let a = p.target("#a");
    p.fire(a, "click");
    assert_eq!(p.text("window.ids.join(',')"), "b,a");
}

#[test]
fn handler_errors_are_reported_not_fatal() {
    let mut p = Page::html("<div id=d>x</div>");
    p.js(
        "window.after = false;\
         document.getElementById('d').addEventListener('click', () => { throw new Error('boom'); });\
         document.getElementById('d').addEventListener('click', () => { window.after = true; });",
    );
    let d = p.target("#d");
    p.fire(d, "click");
    assert!(
        p.yes("window.after"),
        "a throwing handler must not stop the others"
    );
    let problems = p.b.problems();
    assert!(
        problems.iter().any(|p| p.message.contains("boom")),
        "the error must be reported: {problems:?}"
    );
}

#[test]
fn mouse_event_fields() {
    let mut p = Page::html("<div id=d>x</div>");
    p.js(
        "window.m = null;\
         document.getElementById('d').addEventListener('mousedown', e => { window.m = e.clientX + ',' + e.clientY + ',' + e.button; });",
    );
    p.js(
        "const ev = new MouseEvent('mousedown', { clientX: 12, clientY: 34, button: 2, bubbles: true });\
         document.getElementById('d').dispatchEvent(ev);",
    );
    assert_eq!(p.text("window.m"), "12,34,2");
}

#[test]
fn keyboard_event_fields() {
    let mut p = Page::html("<input id=i>");
    p.js(
        "window.k = null;\
         document.getElementById('i').addEventListener('keydown', e => { window.k = e.key + '/' + e.code + '/' + e.shiftKey; });",
    );
    p.js("document.getElementById('i').dispatchEvent(new KeyboardEvent('keydown', { key: 'A', code: 'KeyA', shiftKey: true }));");
    assert_eq!(p.text("window.k"), "A/KeyA/true");
}
