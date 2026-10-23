//! The environment around the document: `window`, `location`, storage,
//! cookies, computed style, geometry, forms and `fetch`.

mod common;

use common::Page;
use tb_dom_bindings::NavRequest;

#[test]
fn window_is_global_this() {
    let mut p = Page::html("<p>x</p>");
    assert!(p.yes("window === globalThis"));
    assert!(p.yes("window.window === window"));
    assert!(p.yes("self === window"));
    assert!(p.yes("top === window"));
    assert!(p.yes("parent === window"));
    assert!(p.yes("String(window) === '[object Window]'"));
    p.js("var fromScript = 42;");
    assert_eq!(p.num("window.fromScript"), 42.0);
    let vp = tb_core::EngineConfig::default().viewport;
    assert_eq!(p.num("window.innerWidth"), f64::from(vp.width));
    assert_eq!(p.num("window.innerHeight"), f64::from(vp.height));
    assert_eq!(p.num("screen.width"), f64::from(vp.width));
    assert_eq!(p.num("navigator.hardwareConcurrency"), 4.0);
    assert!(p.text("navigator.userAgent").starts_with("tinybrowser"));
    assert_eq!(p.text("navigator.platform"), "Linux x86_64");
    assert_eq!(p.text("navigator.language"), "en-US");
    assert_eq!(p.text("document.documentElement.lang + ''"), "");
}

#[test]
fn location_reads_the_current_url() {
    let mut p = Page::at(
        "https://example.com:8443/docs/index.html?a=1#frag",
        "<p id=p>x</p>",
    );
    assert_eq!(
        p.text("location.href"),
        "https://example.com:8443/docs/index.html?a=1#frag"
    );
    assert_eq!(p.text("location.protocol"), "https:");
    assert_eq!(p.text("location.host"), "example.com:8443");
    assert_eq!(p.text("location.hostname"), "example.com");
    assert_eq!(p.text("location.port"), "8443");
    assert_eq!(p.text("location.pathname"), "/docs/index.html");
    assert_eq!(p.text("location.search"), "?a=1");
    assert_eq!(p.text("location.hash"), "#frag");
    assert_eq!(p.text("location.origin"), "https://example.com:8443");
    assert_eq!(
        p.text("document.URL"),
        "https://example.com:8443/docs/index.html?a=1#frag"
    );
    assert_eq!(
        p.text("String(location)"),
        "https://example.com:8443/docs/index.html?a=1#frag"
    );
}

#[test]
fn relative_urls_are_resolved() {
    let mut p = Page::at(
        "http://example.test/a/b/index.html",
        "<a id=l href=c.html>c</a><img id=i src=../x/y.png>",
    );
    p.ids(&["l", "i"]);
    assert_eq!(p.text("l.href"), "http://example.test/a/b/c.html");
    assert_eq!(p.text("l.getAttribute('href')"), "c.html");
    assert_eq!(p.text("l.pathname"), "/a/b/c.html");
    assert_eq!(p.text("i.src"), "http://example.test/a/x/y.png");
    p.js("l.href = 'd/e.html';");
    assert_eq!(p.text("l.getAttribute('href')"), "d/e.html");
    assert_eq!(p.text("l.hostname"), "example.test");
}

#[test]
fn navigation_requests_reach_the_browser() {
    let mut p = Page::at(
        "http://example.test/start.html",
        "<a id=a href=next.html>go</a>",
    );
    p.js("window.location.href = 'other/page.html';");
    assert_eq!(
        p.b.take_navigation(),
        Some(NavRequest::Load(
            "http://example.test/other/page.html".into()
        ))
    );
    p.js("location.assign('x.html');");
    assert_eq!(
        p.b.take_navigation(),
        Some(NavRequest::Load("http://example.test/x.html".into()))
    );
    p.js("location.replace('y.html');");
    assert_eq!(
        p.b.take_navigation(),
        Some(NavRequest::Replace("http://example.test/y.html".into()))
    );
    p.js("location.reload();");
    assert_eq!(p.b.take_navigation(), Some(NavRequest::Reload));
    // a plain click on a link asks for the same thing
    p.js("document.getElementById('a').click();");
    assert_eq!(
        p.b.take_navigation(),
        Some(NavRequest::Load("http://example.test/next.html".into()))
    );
    // a hash change is not a navigation, it is a fragment jump
    p.js("location.hash = '#top';");
    assert_eq!(
        p.b.take_navigation(),
        Some(NavRequest::Load(
            "http://example.test/start.html#top".into()
        ))
    );
}

#[test]
fn history_keeps_state() {
    let mut p = Page::at("http://example.test/a", "<p>x</p>");
    assert_eq!(p.num("history.length"), 1.0);
    assert!(p.yes("history.state === null"));
    p.js("history.pushState({ n: 1 }, 't', '/b');");
    assert_eq!(p.num("history.length"), 2.0);
    assert_eq!(p.num("history.state.n"), 1.0);
    p.js("history.pushState({ n: 2 }, 't', '/c');");
    assert_eq!(p.num("history.state.n"), 2.0);
    p.js("history.replaceState({ n: 3 }, 't');");
    assert_eq!(p.num("history.state.n"), 3.0);
    assert_eq!(p.num("history.length"), 3.0);
    p.js("history.back();");
    assert_eq!(p.b.take_navigation(), Some(NavRequest::Back(-1)));
    p.js("history.forward();");
    assert_eq!(p.b.take_navigation(), Some(NavRequest::Back(1)));
    p.js("history.go(-2);");
    assert_eq!(p.b.take_navigation(), Some(NavRequest::Back(-2)));
}

#[test]
fn storage_behaves_like_web_storage() {
    let mut p = Page::html("<p>x</p>");
    assert!(p.yes("localStorage !== sessionStorage"));
    p.js(
        "localStorage.setItem('a', '1'); localStorage.b = 'two'; sessionStorage.setItem('a', 'x');",
    );
    assert_eq!(p.text("localStorage.getItem('a')"), "1");
    assert_eq!(p.text("sessionStorage.getItem('a')"), "x");
    assert_eq!(p.text("localStorage.b"), "two");
    assert!(p.yes("localStorage.getItem('missing') === null"));
    assert_eq!(p.num("localStorage.length"), 2.0);
    assert_eq!(p.text("localStorage.key(0)"), "a");
    p.js("localStorage.removeItem('a');");
    assert_eq!(p.num("localStorage.length"), 1.0);
    p.js("localStorage.clear();");
    assert_eq!(p.num("localStorage.length"), 0.0);
    // values are strings, like in a browser
    p.js("localStorage.n = 5;");
    assert_eq!(p.text("typeof localStorage.n"), "string");
    assert_eq!(p.text("localStorage.n"), "5");
}

#[test]
fn cookies_round_trip() {
    let mut p = Page::at("http://example.test/", "<p>x</p>");
    p.js("document.cookie = 'sess=abc; path=/';");
    p.js("document.cookie = 'other=1';");
    assert_eq!(p.text("document.cookie"), "sess=abc; other=1");
    // a cookie already past its expiry is dropped on arrival
    p.js("document.cookie = 'gone=1; max-age=0';");
    assert!(!p.text("document.cookie").contains("gone"));
    // writing the same name again replaces the value
    p.js("document.cookie = 'sess=xyz';");
    assert_eq!(p.text("document.cookie"), "sess=xyz; other=1");
    assert!(p.yes("navigator.cookieEnabled"));
    // and the header the network will send
    assert_eq!(p.b.host.borrow().cookie_header(), "sess=xyz; other=1");
}

#[test]
fn computed_style_sees_author_css() {
    let mut p = Page::html(
        "<style>#t { color: rgb(1, 2, 3); font-size: 20px } .box p { color: #ff0000 }</style>\
         <div id=t class=box><p id=q>text</p><span id=s>x</span></div>",
    );
    p.ids(&["t", "q", "s"]);
    assert_eq!(p.text("getComputedStyle(t).color"), "rgb(1, 2, 3)");
    assert_eq!(p.text("getComputedStyle(t).fontSize"), "20px");
    assert_eq!(
        p.text("getComputedStyle(t).getPropertyValue('font-weight')"),
        "400"
    );
    // the `.box p` rule wins over `#t` for the paragraph…
    assert_eq!(p.text("getComputedStyle(q).color"), "rgb(255, 0, 0)");
    // …and the span inherits from #t
    assert_eq!(p.text("getComputedStyle(s).color"), "rgb(1, 2, 3)");
    assert_eq!(p.text("getComputedStyle(s).fontSize"), "20px");
}

#[test]
fn computed_style_follows_a_style_tag_written_by_script() {
    let mut p = Page::html("<div id=d>x</div>");
    p.ids(&["d"]);
    assert_eq!(p.text("getComputedStyle(d).display"), "block");
    p.js("const s = document.createElement('style'); s.textContent = '#d { display: none; color: blue }'; document.head.appendChild(s);");
    assert_eq!(p.text("getComputedStyle(d).display"), "none");
}

#[test]
fn inline_style_and_computed_style() {
    let mut p = Page::html("<div id=d style='color: green; margin: 4px 8px'>x</div>");
    p.ids(&["d"]);
    assert_eq!(p.text("d.style.color"), "green");
    assert_eq!(p.text("d.style.marginTop"), "4px");
    assert_eq!(p.text("d.style.marginLeft"), "8px");
    p.js("d.style.backgroundColor = 'rgb(1, 2, 3)';");
    assert!(p
        .text("d.getAttribute('style')")
        .contains("background-color: rgb(1, 2, 3)"));
    assert_eq!(
        p.text("getComputedStyle(d).backgroundColor"),
        "rgb(1, 2, 3)"
    );
    p.js("d.style.removeProperty('color');");
    assert_eq!(p.text("d.style.color"), "");
    p.js("d.style.setProperty('border-width', '2px');");
    assert_eq!(p.text("getComputedStyle(d).borderTopWidth"), "2px");
    assert!(p.text("d.style.cssText").contains("border-width"));
}

#[test]
fn geometry_comes_from_the_layout_pass() {
    let mut p = Page::html(
        "<style>body { margin: 0 } .row { height: 40px } #box { width: 100px; height: 50px }</style>\
         <div class=row></div><div id=box></div>",
    );
    p.ids(&["box"]);
    assert_eq!(p.num("box.getBoundingClientRect().top"), 40.0);
    assert_eq!(p.num("box.getBoundingClientRect().height"), 50.0);
    assert_eq!(p.num("box.getBoundingClientRect().width"), 100.0);
    assert!(p.yes("box.offsetParent === document.body || box.offsetParent === null"));
    assert_eq!(p.num("box.getClientRects().length"), 1.0);
    // a hidden element has no box at all
    p.js("box.style.display = 'none';");
    assert_eq!(p.num("box.getBoundingClientRect().width"), 0.0);
    assert!(p.yes("box.getBoundingClientRect().height === 0"));
}

#[test]
fn match_media_answers_from_the_viewport() {
    let mut p = Page::html("<p>x</p>");
    assert!(p.yes("matchMedia('(min-width: 800px)').matches"));
    assert!(!p.yes("matchMedia('(max-width: 600px)').matches"));
    assert_eq!(
        p.text("matchMedia('(max-width: 600px)').media"),
        "(max-width: 600px)"
    );
    assert!(p.yes("CSS.supports('display', 'grid')"));
}

#[test]
fn scrolling_is_reported_to_the_browser() {
    let mut p = Page::html("<div style='height: 3000px'>tall</div>");
    p.js("window.scrollTo(0, 250);");
    assert_eq!(p.num("window.scrollY"), 250.0);
    assert_eq!(p.num("window.pageYOffset"), 250.0);
    assert_eq!(p.b.take_navigation(), Some(NavRequest::Scroll(0.0, 250.0)));
    p.js("window.scrollBy(0, 50);");
    assert_eq!(p.num("scrollY"), 300.0);
    p.js("scrollTo({ left: 10, top: 20 });");
    assert_eq!(p.num("scrollX"), 10.0);
    assert_eq!(p.num("scrollY"), 20.0);
}

#[test]
fn element_from_point_uses_the_layout() {
    let mut p = Page::html(
        "<style>body{margin:0}</style><div id=a style='width:100px;height:100px'></div>\
         <div id=b style='width:100px;height:100px'></div>",
    );
    let id =
        p.js("document.elementFromPoint(10, 130) ? document.elementFromPoint(10, 130).id : 'none'");
    assert_eq!(id.to_string(), "b");
    let first = p.js("document.elementFromPoint(10, 10).id");
    assert_eq!(first.to_string(), "a");
}

#[test]
fn form_controls_reflect_their_attributes() {
    let mut p = Page::html(
        "<form id=f action=/search method=get>\
           <input id=i name=q value=hello>\
           <input id=c type=checkbox checked>\
           <textarea id=t>multi\nline</textarea>\
           <select id=s><option value=a>A</option><option value=b selected>B</option></select>\
         </form>",
    );
    p.ids(&["f", "i", "c", "t", "s"]);
    assert_eq!(p.text("i.value"), "hello");
    assert_eq!(p.text("i.defaultValue"), "hello");
    p.js("i.value = 'typed';");
    assert_eq!(p.text("i.value"), "typed");
    assert_eq!(p.text("i.defaultValue"), "hello");
    assert!(p.yes("c.checked"));
    p.js("c.checked = false;");
    assert!(!p.yes("c.hasAttribute('checked')"));
    assert_eq!(p.text("t.value"), "multi\nline");
    assert_eq!(p.text("s.value"), "b");
    assert_eq!(p.num("s.selectedIndex"), 1.0);
    assert_eq!(p.num("s.options.length"), 2.0);
    assert_eq!(p.text("s.options[0].text"), "A");
    p.js("s.selectedIndex = 0;");
    assert_eq!(p.text("s.value"), "a");
    assert_eq!(p.text("f.action"), "http://example.test/search");
    assert_eq!(p.text("f.method"), "get");
    assert!(p.yes("document.forms.length === 1"));
    assert!(p.yes("document.getElementById('f') === f"));
}

#[test]
fn form_submit_asks_for_navigation_unless_prevented() {
    let mut p = Page::at(
        "http://example.test/form.html",
        "<form id=f action=/go></form>",
    );
    p.js("document.getElementById('f').submit();");
    assert_eq!(
        p.b.take_navigation(),
        Some(NavRequest::Load("http://example.test/go".into()))
    );
    p.js("document.getElementById('f').addEventListener('submit', e => e.preventDefault());");
    p.js("document.getElementById('f').submit();");
    assert_eq!(p.b.take_navigation(), None);
}

#[test]
fn checkbox_click_toggles_and_fires_change() {
    let mut p = Page::html("<input id=c type=checkbox>");
    p.ids(&["c"]);
    p.js("window.seen = []; c.addEventListener('change', e => window.seen.push('change:' + c.checked));");
    p.js("c.click();");
    assert!(p.yes("c.checked"));
    assert_eq!(p.text("window.seen.join(',')"), "change:true");
    p.js("c.click();");
    assert!(!p.yes("c.checked"));
    assert_eq!(p.text("window.seen.join(',')"), "change:true,change:false");
}

#[test]
fn focus_and_blur_move_the_active_element() {
    let mut p = Page::html(
        "<style>input:focus { border-color: rgb(0, 0, 255) }</style>\
         <input id=a><input id=b>",
    );
    p.ids(&["a", "b"]);
    p.js(
        "window.log = []; a.addEventListener('focus', () => window.log.push('focus'));\
          a.addEventListener('blur', () => window.log.push('blur'));\
          b.addEventListener('focus', () => window.log.push('focus-b'));",
    );
    p.js("a.focus();");
    assert_eq!(p.text("window.log.join(',')"), "focus");
    assert!(p.yes("document.activeElement === a"));
    p.js("b.focus();");
    assert_eq!(p.text("window.log.join(',')"), "focus,blur,focus-b");
    assert!(p.yes("document.activeElement === b"));
}

#[test]
fn fetch_uses_the_hook_and_resolves_a_response() {
    let mut p = Page::at("http://example.test/page.html", "<p>x</p>");
    p.b.set_fetch_hook(|url| match url {
        "http://example.test/data.json" => Some((200, "{\"ok\": true, \"n\": 3}".into())),
        "http://example.test/missing" => Some((404, "nope".into())),
        _ => None,
    });
    p.js(
        "window.out = [];\
         fetch('/data.json').then(r => r.json()).then(j => { window.out.push('json:' + j.ok + ':' + j.n); });\
         fetch('/missing').then(r => r.text()).then(t => window.out.push('text:' + t));\
         fetch('/gone').then(() => window.out.push('resolved')).catch(e => window.out.push('rejected'));\
         fetch('/data.json').then(r => window.out.push('status:' + r.status + ':' + r.ok));",
    );
    p.pump(10.0);
    // `fetch` here resolves as soon as the hook answered, so the *order* of the
    // four chains is the interpreter's; what matters is that all of them ran.
    let out = p.text("window.out.join('|')");
    for want in ["json:true:3", "text:nope", "status:200:true", "rejected"] {
        assert!(out.contains(want), "{want} missing from {out}");
    }
}

#[test]
fn small_helpers() {
    let mut p = Page::html("<!doctype html><p>x</p>");
    assert_eq!(p.text("btoa('hi')"), "aGk=");
    assert_eq!(p.text("atob('aGk=')"), "hi");
    assert!(p.yes("performance.now() >= 0"));
    assert_eq!(p.text("crypto.randomUUID().split('-').length"), "5");
    assert!(p.yes("typeof document.hidden === 'boolean' || typeof document.hidden === 'undefined'"));
    assert!(p.yes("document.hasFocus()"));
    assert_eq!(p.text("document.characterSet"), "UTF-8");
    assert!(p.yes("document.compatMode === 'CSS1Compat'"));
    assert!(p.yes("new DOMParser().parseFromString('<b>hi</b>', 'text/html').firstElementChild.tagName === 'B'"));
    assert!(p.yes("typeof MutationObserver === 'function'"));
    assert!(p.yes("new MutationObserver(() => {}).observe(document.body, {}) === undefined"));
    assert_eq!(p.text("String(new Image(60, 40).tagName)"), "IMG");
    assert_eq!(p.text("String(new Image(60, 40).width)"), "60");
}

#[test]
fn dialogs_do_not_stop_the_page() {
    let mut p = Page::html("<script>alert('hi'); window.afterAlert = confirm('sure');</script>");
    let report = p.run(&[]);
    assert!(report.clean());
    assert!(!p.yes("window.afterAlert"));
    assert!(
        p.b.problems()
            .iter()
            .any(|p| p.message.contains("automatically")),
        "a dialog that was answered silently must be logged"
    );
}

#[test]
fn title_writes_back_to_the_head() {
    let mut p = Page::at("http://example.test/", "<title>old</title><p>x</p>");
    assert_eq!(p.text("document.title"), "old");
    p.js("document.title = 'new';");
    assert_eq!(p.text("document.querySelector('title').textContent"), "new");
    assert_eq!(p.b.title(), "new");
    p.js("document.title = '';");
    assert_eq!(p.text("document.title"), "");
}

#[test]
fn document_element_and_document_meta() {
    let mut p = Page::at(
        "http://example.test/x/y.html",
        "<html lang=en><body>x</body></html>",
    );
    assert_eq!(p.text("document.baseURI"), "http://example.test/x/y.html");
    assert_eq!(p.text("document.referrer"), "");
    assert_eq!(p.text("document.contentType"), "text/html");
    assert!(p.yes("document.xmlVersion === null"));
    assert!(p.yes("document.implementation.hasFeature('', '') === true"));
    assert_eq!(
        p.text("document.documentElement.getAttribute('lang')"),
        "en"
    );
}
