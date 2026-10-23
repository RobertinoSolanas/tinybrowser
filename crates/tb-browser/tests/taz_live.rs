//! The acceptance run against the real https://taz.de.
//!
//! These tests need internet, so they are `#[ignore]`d by default:
//!
//! ```sh
//! cargo test -p tb-browser --test taz_live -- --ignored --nocapture
//! ```
//!
//! They exist because a browser that only ever sees fixtures is a browser that
//! has not been tested. Everything asserted here is the shape of a real, live,
//! third-party news page: charset from the HTTP header, redirect to the trailing
//! slash, megabytes of CSS, scripts that fail for their own reasons.

use tb_browser::{Engine, EngineConfig};

const TAZ: &str = "https://taz.de";

#[test]
#[ignore = "needs internet (fetches https://taz.de)"]
fn taz_de_loads_correctly() {
    let mut engine = Engine::new(EngineConfig::default());
    let started = std::time::Instant::now();
    let page = engine.load(TAZ).expect("taz.de loads");
    let total = started.elapsed();

    // ---- transport
    assert_eq!(page.status, 200, "status {}", page.status);
    assert!(
        page.content_type.contains("text/html"),
        "content type {}",
        page.content_type
    );
    assert!(
        page.content_type.contains("utf-8") || page.content_type.contains("UTF-8"),
        "charset should be utf-8: {}",
        page.content_type
    );
    assert_eq!(
        page.url.path, "/",
        "taz.de redirects to its trailing slash: {}",
        page.url
    );
    assert!(page.bytes > 100_000, "body size {}", page.bytes);

    // ---- document
    assert_eq!(page.title(), "taz.de | taz.de", "title");
    let s = page.stats();
    assert!(s.elements > 300, "elements {}", s.elements);
    assert!(s.links > 150, "links {}", s.links);
    assert!(s.nodes > s.elements);

    // every link is absolute — a fragment link resolves to the page plus its
    // fragment, a `mailto:` keeps its scheme (`a.href`, not "can we fetch it")
    let has_scheme = |u: &str| {
        u.split_once('/')
            .map(|(head, _)| head.contains(':'))
            .unwrap_or_else(|| u.contains(':'))
    };
    let absolute = page.links().iter().filter(|l| has_scheme(&l.url)).count();
    assert_eq!(absolute, s.links, "every link resolves to an absolute URL");
    assert!(
        page.links()
            .iter()
            .filter(|l| l.url.contains("taz.de"))
            .count()
            > 100,
        "most links stay on taz.de"
    );

    // ---- css
    assert!(s.stylesheets > 0, "sheets {}", s.stylesheets);
    assert!(s.css_rules > 0, "rules {}", s.css_rules);
    assert!(
        s.matched_elements > s.elements / 2,
        "matched {} of {} elements",
        s.matched_elements,
        s.elements
    );

    // ---- layout
    let layout = page.layout.as_ref().expect("a box tree");
    assert!(
        layout.scroll_size.height > layout.viewport.height,
        "a newspaper page is taller than the viewport: {}",
        layout.scroll_size.height
    );
    assert!(layout.box_count > 500, "boxes {}", layout.box_count);

    // ---- text and links as a reader would use them
    let text = page.text();
    assert!(text.len() > 2_000, "text length {}", text.len());
    assert!(
        text.to_lowercase().contains("taz"),
        "the page talks about itself"
    );
    assert!(page.headings().iter().any(|h| h.level == 1), "an h1 exists");

    // ---- pixels
    let mut path = std::env::temp_dir();
    path.push("tinybrowser-taz-live.png");
    let mut page = page;
    let bytes = page
        .screenshot(&path, &tb_browser::PaintOptions::default())
        .expect("screenshot");
    let img = tb_gfx::png_decode(&std::fs::read(&path).expect("png")).expect("decode");
    assert_eq!(img.width, 1280);
    let ink = img
        .pixels
        .iter()
        .filter(|c| **c != tb_core::Color::WHITE)
        .count();
    assert!(
        ink > 20_000,
        "the page painted {ink} non-white pixels ({bytes} PNG bytes)"
    );

    // ---- the whole thing in reasonable time
    let ms = total.as_secs_f64() * 1000.0;
    let budget = if cfg!(debug_assertions) {
        120_000.0
    } else {
        30_000.0
    };
    assert!(ms < budget, "taz.de took {ms:.0} ms");
    println!(
        "taz.de: {} elements, {} links, {} rules, {} boxes, {} ms\n{:?}",
        s.elements, s.links, s.css_rules, s.boxes, ms, page.timings
    );
}

#[test]
#[ignore = "needs internet (fetches https://taz.de)"]
fn scripts_of_the_live_page_report_rather_than_break() {
    let mut engine = Engine::new(EngineConfig::default());
    let page = engine.load(TAZ).expect("load");
    // the live page has trackers and consent wrappers that do not work in a
    // console browser; the load must survive them and say what happened
    let s = page.stats();
    assert!(s.scripts >= 10, "script tags {}", s.scripts);
    assert!(s.scripts_run > 0, "some inline scripts run");
    assert!(
        s.problems < 200,
        "problems stay explainable: {}",
        s.problems
    );
    for p in page.problems().all().iter().take(20) {
        println!("problem: {p}");
    }
}

#[test]
#[ignore = "needs internet"]
fn a_second_page_and_the_history_behave() {
    // taz answers some paths with 403 to a console user agent, so the second
    // document is a page that always answers: what matters is that a browser
    // session can hold two documents and move between them.
    let mut engine = Engine::new(EngineConfig::default());
    let first = engine.load(TAZ).expect("taz loads");
    let first_url = first.url.to_string();
    let second = engine
        .load("https://example.com/")
        .expect("example.com loads");
    assert_eq!(second.status, 200);
    assert!(
        second.title().contains("Example"),
        "title {}",
        second.title()
    );
    assert_eq!(engine.history.len(), 2);

    let back = engine.back().unwrap().expect("back to taz");
    assert_eq!(back.title(), "taz.de | taz.de");
    assert_eq!(back.url.to_string(), first_url);
    let fwd = engine.forward().unwrap().expect("forward again");
    assert!(fwd.title().contains("Example"));
}
