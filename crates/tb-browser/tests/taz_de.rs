//! The acceptance page: a committed snapshot of taz.de (422 KB, 2893 elements,
//! 31 script tags) pushed through the whole browser.
//!
//! The stylesheet URLs of the real page are served from `fixtures/css/` by the
//! test server, so this is a genuine network run with a real-world document and
//! real-world CSS — just reproducible and offline.

use std::path::PathBuf;
use std::time::Instant;
use tb_browser::{Engine, EngineConfig};
use tb_testkit::{StubResponse, TestServer};

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

fn read(rel: &str) -> String {
    let path = fixtures().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The snapshot plus the stylesheets it references (those we have locally).
fn taz_server() -> TestServer {
    let mut server = TestServer::start();
    server.route("/", StubResponse::html(read("taz.de.html")));
    let base = "/lib/taz-verlag-node";
    server.route(
        &format!("{base}/styles/css-read-only/styles__news.min.css"),
        StubResponse::css(read("css/styles__news.min.css")),
    );
    server.route(
        &format!("{base}/js/swiper/swiper-bundle.min.css"),
        StubResponse::css(read("css/swiper-bundle.min.css")),
    );
    server.route(
        &format!("{base}/js/shariff/dist/shariff.min.css"),
        StubResponse::css(read("css/shariff.min.css")),
    );
    server
}

/// T10.1: the offline acceptance test over the committed snapshot.
#[test]
fn taz_snapshot_loads() {
    let server = taz_server();
    let url = server.url("/");
    let mut engine = Engine::new(EngineConfig::default());
    let mut page = engine.load(&url).expect("the snapshot loads");

    assert_eq!(page.status, 200);
    assert_eq!(page.title(), "taz.de | taz.de");
    let s = page.stats();
    assert!(s.elements > 2500);
    assert!(s.links > 150);
    assert!(s.boxes > 2000);
    // the snapshot is a big document: the page paints, and the pipeline has a
    // problem list that stays short enough to read
    let png = {
        let mut p = std::env::temp_dir();
        p.push("tinybrowser-taz-snapshot.png");
        p
    };
    page.screenshot(&png, &tb_browser::PaintOptions::default())
        .expect("screenshot");
    assert!(png.exists());
    assert!(
        page.problems().all().len() < 200,
        "problems: {}",
        page.problems().all().len()
    );
}

#[test]
fn the_whole_pipeline_handles_a_newspage() {
    let server = taz_server();
    let url = server.url("/");

    let mut engine = Engine::new(EngineConfig::default());
    let started = Instant::now();
    let page = engine.load(&url).expect("the page loads");
    let total = started.elapsed();

    let s = page.stats();
    assert_eq!(page.status, 200);
    assert!(page.title().contains("taz"), "title: {}", page.title());
    assert!(s.elements > 2500, "elements: {}", s.elements);
    assert!(s.nodes > s.elements, "nodes: {}", s.nodes);
    assert!(s.links > 150, "links: {}", s.links);
    assert!(s.images > 100, "images: {}", s.images);
    assert!(s.scripts >= 20, "script tags: {}", s.scripts);
    assert!(s.stylesheets >= 3, "sheets: {}", s.stylesheets);
    assert!(s.css_rules > 400, "css rules: {}", s.css_rules);
    assert!(s.matched_elements > 1500, "matched: {}", s.matched_elements);
    assert!(s.boxes > 2000, "boxes: {}", s.boxes);

    // the pipeline is not fast by accident: this budget is a regression guard
    let ms = total.as_secs_f64() * 1000.0;
    let budget = if cfg!(debug_assertions) {
        60_000.0
    } else {
        8_000.0
    };
    assert!(ms < budget, "the whole page took {ms:.0} ms");

    // the visible text is real text, and it is a lot of it
    let text = page.text();
    assert!(text.len() > 5_000, "text length {}", text.len());
    assert!(text.chars().any(|c| c.is_alphabetic()));
}

#[test]
fn scripts_of_a_real_page_run_without_taking_the_page_down() {
    let server = taz_server();
    let url = server.url("/");
    let mut engine = Engine::new(EngineConfig::default());
    let page = engine.load(&url).expect("load");

    assert!(page.has_js());
    let s = page.stats();
    assert!(s.scripts_run > 0, "some inline scripts must run");
    // a real page has scripts that fail for real reasons (missing globals);
    // that must be reported and must not stop the load
    if s.script_failures > 0 {
        assert!(
            page.problems()
                .all()
                .iter()
                .any(|p| p.message.contains("script")),
            "script failures should be reported"
        );
    }
    assert!(
        page.problems().all().len() < 200,
        "problems should stay explainable: {}",
        page.problems().all().len()
    );
}

#[test]
fn a_real_page_renders_to_a_picture() {
    let server = taz_server();
    let url = server.url("/");
    let mut engine = Engine::new(EngineConfig::default());
    let mut page = engine.load(&url).unwrap();

    let mut path = std::env::temp_dir();
    path.push("tinybrowser-taz.png");
    let bytes = page
        .screenshot(&path, &tb_browser::PaintOptions::default())
        .expect("png");
    assert!(bytes > 10_000, "png bytes {bytes}");
    let img = tb_gfx::png_decode(&std::fs::read(&path).expect("file")).expect("decode");
    assert_eq!(img.width, 1280);
    let ink = img
        .pixels
        .iter()
        .filter(|c| **c != tb_core::Color::WHITE)
        .count();
    assert!(ink > 10_000, "the page painted {ink} pixels");
}

#[test]
fn geometry_and_links_of_a_real_page_are_sane() {
    let server = taz_server();
    let url = server.url("/");
    let mut engine = Engine::new(EngineConfig::default());
    let page = engine.load(&url).unwrap();

    let layout = page.layout.as_ref().expect("a box tree");
    assert!(layout.scroll_size.height > 3000.0, "page height");
    // every coordinate finite — a NaN here would poison a whole page
    fn walk(b: &tb_layout::LayoutBox) {
        assert!(
            b.content.x.is_finite()
                && b.content.y.is_finite()
                && b.content.width.is_finite()
                && b.content.height.is_finite(),
            "non-finite geometry"
        );
        b.children.iter().for_each(walk);
    }
    walk(&layout.root);

    // links point at taz, resolved to absolute URLs
    let links = page.links();
    assert!(
        links.iter().any(|l| l.url.starts_with("https://taz.de")),
        "expected absolute taz links"
    );
    // the top of the page is not empty space: something is under the header
    assert!(page.tag_at(200.0, 30.0).is_some() || page.tag_at(200.0, 300.0).is_some());
}

#[test]
fn turning_js_off_keeps_the_page_intact() {
    let server = taz_server();
    let url = server.url("/");
    let mut config = EngineConfig::default();
    config.caps.run_scripts = false;
    let mut engine = Engine::new(config);
    let with_js_off = engine.load(&url).unwrap();
    assert!(!with_js_off.has_js());
    assert!(with_js_off.stats().elements > 2500);
    assert!(with_js_off.text().len() > 5_000);
}
