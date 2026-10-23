//! The pipeline as a user sees it: load a document, ask questions about it.
//!
//! Everything here runs offline (`Engine::offline`, `file:`, `data:`), so the
//! tests say something about the pipeline itself rather than about a network.

use std::path::{Path, PathBuf};
use tb_browser::{Engine, EngineConfig, NavState, PaintOptions};

fn tmp(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("tinybrowser-{name}"));
    std::fs::create_dir_all(&p).expect("temp dir");
    p
}

/// Write a page to a temp file and return a `file:` URL for it.
fn page_file(name: &str, html: &str) -> String {
    let path = tmp(name).join(format!("{name}.html"));
    std::fs::write(&path, html).expect("write page");
    format!("file://{}", path.to_string_lossy())
}

fn offline() -> Engine {
    Engine::offline(EngineConfig::default())
}

const PAGE: &str = r#"<!doctype html>
<html>
<head>
  <title>Tiny | Home</title>
  <style>
    body { font-family: sans-serif; color: #222; }
    h1 { color: #bb0000; }
    .card { border: 2px solid #3366cc; padding: 8px; background: #eef; }
    .gone { display: none; }
    .invisible { visibility: hidden; }
  </style>
</head>
<body>
  <header><h1>Welcome</h1></header>
  <nav>
    <a href="/first">First</a>
    <a href="second.html">Second</a>
    <a href="https://example.com/ext">External</a>
  </nav>
  <div class="card"><p>Card text with <strong>emphasis</strong>.</p></div>
  <ul><li>alpha</li><li>beta</li><li>gamma</li></ul>
  <p class="gone">not rendered at all</p>
  <p class="invisible">rendered space, no paint</p>
  <h2>Sections</h2>
  <p>Body copy.</p>
  <img src="photo.png" alt="A photo">
</body>
</html>"#;

#[test]
fn loads_a_document_and_reports_what_it_is() {
    let url = page_file("basic", PAGE);
    let mut browser = offline();
    let page = browser.load(&url).expect("load");
    assert_eq!(page.status, 200);
    assert_eq!(page.title(), "Tiny | Home");
    let s = page.stats();
    assert!(s.elements > 15, "elements {}", s.elements);
    assert!(s.nodes > s.elements, "nodes {}", s.nodes);
    assert_eq!(s.links, 3);
    assert_eq!(s.images, 1);
    assert_eq!(s.stylesheets, 1, "the <style> element is one sheet");
    assert!(s.css_rules >= 5);
    assert!(s.matched_elements > 10, "matched {}", s.matched_elements);
    assert!(s.boxes > 10, "boxes {}", s.boxes);
    assert_eq!(browser.history.len(), 1);
}

#[test]
fn text_is_the_visible_words_in_order() {
    let url = page_file("text", PAGE);
    let mut browser = offline();
    let page = browser.load(&url).unwrap();
    let text = page.text();
    assert!(text.contains("Welcome"), "{text}");
    assert!(text.contains("Card text with emphasis."), "{text}");
    assert!(
        text.contains("alpha\n• beta\n• gamma") || text.contains("alpha\nbeta\ngamma"),
        "{text}"
    );
    assert!(text.contains("Sections"), "{text}");
    // display:none and visibility:hidden are not visible, so they are not text
    assert!(!text.contains("not rendered at all"), "{text}");
    assert!(!text.contains("no paint"), "{text}");
    // and neither is the style sheet or the head
    assert!(!text.contains("sans-serif"), "{text}");
}

#[test]
fn links_are_resolved_against_the_page() {
    let url = page_file("links", PAGE);
    let mut browser = offline();
    let page = browser.load(&url).unwrap();
    let links = page.links();
    let urls: Vec<&str> = links.iter().map(|l| l.url.as_str()).collect();
    let base = url.rsplit_once('/').unwrap().0;
    assert!(urls.contains(&"https://example.com/ext"), "{urls:?}");
    assert!(
        urls.contains(&"file:///first"),
        "root relative resolves against the origin: {urls:?}"
    );
    assert!(
        urls.iter().any(|u| u.ends_with("/second.html")),
        "relative resolved against the page: {urls:?}"
    );
    assert!(base.starts_with("file://"));
    let texts: Vec<&str> = links.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(texts, vec!["First", "Second", "External"]);
}

#[test]
fn headings_and_title_describe_the_page() {
    let url = page_file("headings", PAGE);
    let mut browser = offline();
    let page = browser.load(&url).unwrap();
    let heads = page.headings();
    assert_eq!(heads.len(), 2);
    assert_eq!(heads[0].level, 1);
    assert_eq!(heads[0].text, "Welcome");
    assert_eq!(heads[1].text, "Sections");
    assert_eq!(page.titles()[0], "Tiny | Home");
}

#[test]
fn css_reaches_the_computed_styles() {
    let url = page_file("css", PAGE);
    let mut browser = offline();
    let page = browser.load(&url).unwrap();
    let h1 = page
        .dom
        .descendants(page.dom.document())
        .into_iter()
        .find(|n| page.dom.tag(*n) == Some("h1"))
        .expect("h1");
    let st = page.style_of(h1).expect("style for h1");
    assert_eq!(st.color, tb_core::Color::rgb(0xbb, 0x00, 0x00));
    assert_eq!(st.font_weight, 700, "h1 is bold from the UA sheet");

    let card = page
        .dom
        .descendants(page.dom.document())
        .into_iter()
        .find(|n| page.dom.element(*n).is_some_and(|e| e.has_class("card")))
        .expect(".card");
    let st = page.style_of(card).expect("style for .card");
    assert_eq!(st.background_color, tb_core::Color::rgb(0xee, 0xee, 0xff));
    assert_eq!(st.border_width[0], 2.0);

    let none = page
        .dom
        .descendants(page.dom.document())
        .into_iter()
        .find(|n| page.dom.element(*n).is_some_and(|e| e.has_class("gone")))
        .expect(".gone");
    assert_eq!(
        page.style_of(none).unwrap().display,
        tb_style::Display::None
    );
}

#[test]
fn no_css_falls_back_to_the_user_agent_stylesheet() {
    let url = page_file("nocs", PAGE);
    let mut config = EngineConfig::default();
    config.caps.apply_css = false;
    let mut browser = Engine::offline(config);
    let page = browser.load(&url).unwrap();
    let h1 = page
        .dom
        .descendants(page.dom.document())
        .into_iter()
        .find(|n| page.dom.tag(*n) == Some("h1"))
        .unwrap();
    let st = page.style_of(h1).unwrap();
    // author colour is gone, UA bold and big are not
    assert_ne!(st.color, tb_core::Color::rgb(0xbb, 0x00, 0x00));
    assert!(st.font_size > 16.0, "h1 stays big: {}", st.font_size);
}

#[test]
fn layout_geometry_is_sane() {
    let url = page_file("layout", PAGE);
    let mut browser = offline();
    let page = browser.load(&url).unwrap();
    let layout = page.layout.as_ref().expect("a box tree");
    assert_eq!(layout.viewport.width, 1280.0);
    let body = page
        .dom
        .descendants(page.dom.document())
        .into_iter()
        .find(|n| page.dom.tag(*n) == Some("body"))
        .unwrap();
    let rect = page.rect_of(body).expect("body box");
    assert!(rect.width > 1200.0, "body width {}", rect.width);
    assert!(rect.height > 100.0, "body height {}", rect.height);
    // the hidden paragraph takes no space: its box is not in the tree
    assert!(tb_layout::find_box(&layout.root, {
        page.dom
            .descendants(page.dom.document())
            .into_iter()
            .find(|n| page.dom.element(*n).is_some_and(|e| e.has_class("gone")))
            .unwrap()
    })
    .is_none());
    // every coordinate finite
    fn walk(b: &tb_layout::LayoutBox) {
        assert!(b.content.width.is_finite() && b.content.height.is_finite());
        b.children.iter().for_each(walk);
    }
    walk(&layout.root);
}

#[test]
fn hit_testing_finds_the_thing_under_the_pointer() {
    let url = page_file("hit", PAGE);
    let mut browser = offline();
    let page = browser.load(&url).unwrap();
    let h1 = page
        .dom
        .descendants(page.dom.document())
        .into_iter()
        .find(|n| page.dom.tag(*n) == Some("h1"))
        .unwrap();
    let r = page.rect_of(h1).expect("h1 rect");
    assert_eq!(page.element_at(r.left() + 5.0, r.top() + 2.0), Some(h1));
    assert_eq!(
        page.tag_at(r.left() + 5.0, r.top() + 2.0).as_deref(),
        Some("h1")
    );
}

#[test]
fn screenshots_are_pixels_not_promises() {
    let url = page_file("shot", PAGE);
    let mut browser = offline();
    let mut page = browser.load(&url).unwrap();
    let path = tmp("shot").join("page.png");
    let _ = std::fs::remove_file(&path);
    let bytes = page
        .screenshot(&path, &PaintOptions::default())
        .expect("png");
    assert!(bytes > 500, "png size {bytes}");
    let raw = std::fs::read(&path).expect("file written");
    assert_eq!(&raw[..8], tb_gfx::PNG_SIGNATURE);
    // and it actually painted something: not one flat colour
    let img = tb_gfx::png_decode(&raw).expect("valid png");
    assert!(img.width >= 1280, "width {}", img.width);
    let ink = img
        .pixels
        .iter()
        .filter(|c| **c != tb_core::Color::WHITE)
        .count();
    assert!(ink > 500, "the screenshot is empty ({ink} ink pixels)");
}

#[test]
fn a_full_height_viewport_captures_the_whole_document() {
    let mut tall = String::from("<!doctype html><style>p{margin:0}</style><body>");
    for i in 0..400 {
        tall.push_str(&format!("<p>line {i}</p>"));
    }
    tall.push_str("</body>");
    let url = page_file("tall", &tall);
    let mut browser = offline();
    let mut page = browser.load(&url).unwrap();
    let content = page.content_height();
    assert!(content > 4000.0, "content height {content}");
    assert!(content > page.config.viewport.height);
    page.set_viewport(600.0, content);
    assert_eq!(page.config.viewport.width, 600.0);
    let fb = page.render(&PaintOptions::default());
    assert_eq!(fb.height, content.round() as u32);
    assert!(!fb.is_blank());
}

#[test]
fn scripts_run_and_change_the_document() {
    let html = r#"<!doctype html><html><head><title>before</title></head><body>
      <div id="box"><p>one</p><p>two</p></div>
      <script>
        document.title = 'after';
        const p = document.createElement('p');
        p.className = 'made';
        p.textContent = 'three';
        document.getElementById('box').appendChild(p);
        console.log('count', document.querySelectorAll('#box p').length);
      </script>
    </body></html>"#;
    let mut browser = offline();
    let page = browser.load_html("file:///js.html", html).unwrap();
    assert_eq!(page.title(), "after");
    assert_eq!(page.stats().scripts_run, 1);
    assert_eq!(page.stats().script_failures, 0);
    assert_eq!(page.console(), vec!["count 3".to_string()]);
    // the page object reflects the post-script document
    assert_eq!(page.text().matches("three").count(), 1);
    let made = page
        .dom
        .descendants(page.dom.document())
        .into_iter()
        .filter(|n| page.dom.element(*n).is_some_and(|e| e.has_class("made")))
        .count();
    assert_eq!(made, 1);
}

#[test]
fn eval_after_load_sees_the_live_page() {
    let url = page_file("eval", PAGE);
    let mut browser = offline();
    let mut page = browser.load(&url).unwrap();
    let v = page.eval("document.querySelectorAll('li').length").unwrap();
    assert_eq!(v.to_string(), "3");
    page.eval("document.querySelector('h1').textContent = 'Changed'")
        .unwrap();
    assert_eq!(page.text().matches("Changed").count(), 1);
    // a style written from JS is in the computed style of the page too
    page.eval("document.querySelector('h1').style.color = 'rgb(0, 128, 0)';")
        .unwrap();
    let h1 = page
        .dom
        .descendants(page.dom.document())
        .into_iter()
        .find(|n| page.dom.tag(*n) == Some("h1"))
        .unwrap();
    assert_eq!(
        page.style_of(h1).unwrap().color,
        tb_core::Color::rgb(0, 128, 0)
    );
}

#[test]
fn a_script_can_create_a_stylesheet_that_then_applies() {
    let html = "<!doctype html><body><p id=x>plain</p><script>\
        const s = document.createElement('style');\
        s.textContent = '#x { color: rgb(1, 2, 3); }';\
        document.head.appendChild(s);\
    </script></body>";
    let mut browser = offline();
    let page = browser.load_html("file:///dyn.html", html).unwrap();
    let x = page
        .dom
        .descendants(page.dom.document())
        .into_iter()
        .find(|n| page.dom.element(*n).and_then(|e| e.element_id()) == Some("x"))
        .unwrap();
    assert_eq!(
        page.style_of(x).unwrap().color,
        tb_core::Color::rgb(1, 2, 3),
        "a sheet added by script must be in the cascade"
    );
}

#[test]
fn navigation_requests_are_the_embedders_to_act_on() {
    let html = "<!doctype html><body><a id=a href=next.html>go</a>\
        <script>document.getElementById('a').click();</script></body>";
    let mut browser = offline();
    let page = browser.load_html("file:///nav.html", html).unwrap();
    assert!(
        page.nav_requests
            .iter()
            .any(|n| matches!(n, tb_browser::NavRequest::Load(u) if u.ends_with("/next.html"))),
        "anchor click should ask for a navigation: {:?}",
        page.nav_requests
    );
}

#[test]
fn timers_run_during_a_pump() {
    let html = "<!doctype html><body><script>\
        window.out = 'start';\
        setTimeout(() => { window.out = 'timer'; }, 50);\
    </script></body>";
    let mut browser = offline();
    let mut page = browser.load_html("file:///timer.html", html).unwrap();
    assert_eq!(page.eval("window.out").unwrap().to_string(), "start");
    page.pump(100.0);
    assert_eq!(page.eval("window.out").unwrap().to_string(), "timer");
}

#[test]
fn storage_and_cookies_survive_across_evals() {
    let mut browser = offline();
    let mut page = browser
        .load_html("file:///store.html", "<!doctype html><body>hi</body>")
        .unwrap();
    page.eval("localStorage.setItem('a', '1'); localStorage.b = '2';")
        .unwrap();
    assert_eq!(page.eval("localStorage.length").unwrap().to_string(), "2");
    page.eval("document.cookie = 'sess=abc; path=/'").unwrap();
    assert!(
        page.cookie_header().contains("sess=abc"),
        "{}",
        page.cookie_header()
    );
}

#[test]
fn history_moves_through_loaded_documents() {
    let a = page_file("hist-a", "<!doctype html><title>A</title>");
    let b = page_file("hist-b", "<!doctype html><title>B</title>");
    let mut browser = offline();
    browser.load(&a).unwrap();
    browser.load(&b).unwrap();
    assert_eq!(browser.history.len(), 2);
    let back = browser.back().unwrap().expect("back works");
    assert_eq!(back.title(), "A");
    let fwd = browser.forward().unwrap().expect("forward works");
    assert_eq!(fwd.title(), "B");
    assert!(browser
        .history
        .current()
        .unwrap()
        .to_string()
        .ends_with("hist-b.html"));
}

#[test]
fn a_missing_file_is_an_error_not_a_panic() {
    let mut browser = offline();
    let err = match browser.load("/definitely/not/here.html") {
        Ok(_) => panic!("a missing file must not load"),
        Err(e) => e,
    };
    let msg = err.to_string();
    assert!(msg.contains("not/here.html"), "{msg}");
    assert_eq!(browser.state, NavState::Failed);
}

#[test]
fn data_and_about_urls_work_without_a_network() {
    let mut browser = offline();
    let page = browser
        .load("data:text/html,<title>inline</title><p>from a data url</p>")
        .expect("data url");
    assert_eq!(page.title(), "inline");
    assert!(page.text().contains("from a data url"));
}

#[test]
fn dumps_are_readable_text() {
    let url = page_file("dumps", PAGE);
    let mut browser = offline();
    let page = browser.load(&url).unwrap();
    let dom = page.dump_dom();
    assert!(dom.contains("<html>"), "{dom}");
    assert!(dom.contains("<h1>"), "{dom}");
    assert!(dom.contains("a href=/first"), "{dom}");
    let css = page.dump_css();
    assert!(css.contains("<style>"), "{css}");
    let layout = page.dump_layout();
    assert!(layout.contains("<body>"), "{layout}");
    assert!(!Path::new("/nope").exists());
}
