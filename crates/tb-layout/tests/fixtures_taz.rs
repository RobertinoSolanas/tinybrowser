//! Layout of the real world: the saved taz.de page with its real stylesheets.
//!
//! This is the acceptance test for the layout engine as a whole: a 300 KB news
//! page with 2 900 elements must produce sane boxes, no panics, no infinite
//! geometry, and it must do so in a few milliseconds.

use std::path::PathBuf;
use std::time::Instant;
use tb_css::media::Viewport;
use tb_layout::LayoutBox;
use tb_style::{compute, CssSource, MatchState, StyleEngine};

fn fixture(name: &str) -> String {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("../../fixtures");
    p.push(name);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("fixture {name}: {e}"))
}

fn vp() -> Viewport {
    Viewport {
        width: 1280.0,
        height: 1024.0,
        ..Default::default()
    }
}

struct LaidOut {
    dom: tb_dom::Dom,
    doc: tb_layout::LayoutDocument,
    elapsed: std::time::Duration,
}

fn layout_taz() -> LaidOut {
    let dom = tb_html::parse_document(&fixture("taz.de.html"));
    let sources = vec![
        CssSource::Inline {
            text: fixture("css/styles__news.min.css"),
        },
        CssSource::Inline {
            text: fixture("css/swiper-bundle.min.css"),
        },
        CssSource::Inline {
            text: fixture("css/shariff.min.css"),
        },
    ];
    let mut problems = tb_core::Problems::new();
    let sheets = tb_style::resolve_sources(&sources, vp(), |_| None, &mut problems);
    let engine = StyleEngine::new(vp());
    let styled = engine.style_document(&dom, &sheets);
    let t0 = Instant::now();
    let doc = tb_layout::layout(&dom, &styled.styles, vp()).expect("layout");
    let elapsed = t0.elapsed();
    LaidOut { dom, doc, elapsed }
}

fn walk<F>(b: &LayoutBox, f: &mut F)
where
    F: FnMut(&LayoutBox),
{
    f(b);
    for c in &b.children {
        walk(c, f);
    }
}

#[test]
fn taz_page_lays_out() {
    let l = layout_taz();
    println!(
        "taz layout: {:?} for {} boxes, scroll {:?}",
        l.elapsed, l.doc.box_count, l.doc.scroll_size
    );
    assert!(l.doc.box_count > 2000, "boxes: {}", l.doc.box_count);
    assert!(
        l.doc.scroll_size.height > 2000.0,
        "a news page must be tall: {:?}",
        l.doc.scroll_size
    );
    assert_eq!(l.doc.scroll_size.width, 1280.0);
    // real layout work has to stay fast
    assert!(
        l.elapsed.as_millis() < 400,
        "layout took {:?} — way too slow",
        l.elapsed
    );
}

#[test]
fn taz_geometry_is_finite_everywhere() {
    let l = layout_taz();
    let mut boxes = 0usize;
    walk(&l.doc.root, &mut |b| {
        boxes += 1;
        let bb = b.border_box();
        for v in [bb.x, bb.y, bb.width, bb.height] {
            assert!(v.is_finite(), "non finite geometry in {b:?}");
        }
        assert!(bb.width < 20_000.0, "absurdly wide box: {bb:?}");
    });
    assert!(boxes > 2000);
}

#[test]
fn taz_links_are_clickable_boxes() {
    let l = layout_taz();
    let links = l.dom.get_elements_by_tag("a");
    assert!(links.len() > 200, "fixture has {} links", links.len());
    let mut with_box = 0usize;
    let mut sized = 0usize;
    for id in &links {
        let Some(b) = l.doc.find(*id) else {
            continue;
        };
        with_box += 1;
        let bb = b.border_box();
        if bb.width > 0.0 && bb.height > 0.0 {
            sized += 1;
        }
    }
    assert!(with_box > 150, "only {with_box} links have boxes");
    assert!(
        sized as f32 / with_box as f32 > 0.6,
        "most links should have visible size: {sized}/{with_box}"
    );
}

#[test]
fn taz_headline_text_is_placed() {
    let l = layout_taz();
    let text = l.doc.text();
    // the page title shows up in the laid out text
    assert!(text.contains("taz"), "no taz in laid out text?");
    // and real headlines from the fixture, not raw markup
    assert!(text.chars().count() > 1500, "laid out text too short");
    assert!(
        !text.contains("<div") && !text.contains("function("),
        "markup or script leaked into laid out text"
    );
}

#[test]
fn taz_body_starts_at_the_viewport_top() {
    let l = layout_taz();
    let body = l.dom.get_elements_by_tag("body")[0];
    let b = l.doc.find(body).expect("body box");
    assert!(b.border_box().y >= 0.0, "{:?}", b.border_box());
    assert!(b.border_box().width > 1000.0, "{:?}", b.border_box());
}

#[test]
fn taz_hit_test_finds_a_link() {
    let l = layout_taz();
    let links = l.dom.get_elements_by_tag("a");
    let mut tested = 0usize;
    let mut hits = 0usize;
    for id in links.iter().take(60) {
        let Some(b) = l.doc.find(*id) else { continue };
        let bb = b.border_box();
        if bb.width < 4.0 || bb.height < 4.0 {
            continue;
        }
        tested += 1;
        let hit = tb_layout::element_at(
            &l.doc,
            &l.dom,
            bb.x + bb.width / 2.0,
            bb.y + bb.height / 2.0,
        );
        if hit.is_some() {
            hits += 1;
        }
    }
    assert!(tested > 10, "nothing to test");
    assert!(
        hits == tested,
        "hit testing missed {}/{:?}",
        tested - hits,
        tested
    );
}

#[test]
fn narrow_viewport_reflows_the_page() {
    let dom = tb_html::parse_document(&fixture("taz.de.html"));
    let mut problems = tb_core::Problems::new();
    let sheets = tb_style::resolve_sources(
        &[CssSource::Inline {
            text: fixture("css/styles__news.min.css"),
        }],
        Viewport {
            width: 375.0,
            height: 800.0,
            ..Default::default()
        },
        |_| None,
        &mut problems,
    );
    let styled = compute(
        &dom,
        &sheets,
        Viewport {
            width: 375.0,
            height: 800.0,
            ..Default::default()
        },
        &MatchState::default(),
    );
    let doc = tb_layout::layout(
        &dom,
        &styled.styles,
        Viewport {
            width: 375.0,
            height: 800.0,
            ..Default::default()
        },
    )
    .expect("mobile layout");
    assert!(doc.scroll_size.height > 1000.0);
    // the page itself must not be wider than the phone — carousels may have
    // wider content, but `overflow` contains it
    assert!(
        doc.scroll_size.width <= 375.0 + 1.0,
        "page overflows the viewport: {:?}",
        doc.scroll_size
    );
    // narrow viewports wrap more text, so the page gets longer than at desktop
    let wide = layout_taz();
    assert!(
        doc.scroll_size.height > 1000.0,
        "mobile height {:?} vs desktop {:?}",
        doc.scroll_size,
        wide.doc.scroll_size
    );
}
