//! Styling with the real world: the saved taz.de HTML plus its real stylesheets.

use std::path::PathBuf;
use tb_style::*;

fn fixture(name: &str) -> String {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("../../fixtures");
    p.push(name);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("fixture {name}: {e}"))
}

fn dom() -> tb_dom::Dom {
    tb_html::parse_document(&fixture("taz.de.html"))
}

#[test]
fn taz_fixtures_cascade_without_panicking() {
    let d = dom();
    let mut problems = tb_core::Problems::new();
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
    let sheets = resolve_sources(&sources, Viewport::default(), |_| None, &mut problems);
    assert_eq!(sheets.len(), 3);
    let engine = StyleEngine::new(Viewport {
        width: 1280.0,
        height: 1024.0,
        ..Default::default()
    });
    let t0 = std::time::Instant::now();
    let out = engine.style_document(&d, &sheets);
    let ms = t0.elapsed().as_millis();
    // every element got a style
    assert_eq!(out.styles.count(), d.elements().len());
    assert!(out.rule_count > 3000, "rules: {}", out.rule_count);
    assert!(ms < 4000, "styling taz.de took {ms}ms");
}

#[test]
fn taz_elements_get_useful_styles() {
    let d = dom();
    let mut problems = tb_core::Problems::new();
    let sheets = resolve_sources(
        &[CssSource::Inline {
            text: fixture("css/styles__news.min.css"),
        }],
        Viewport::default(),
        |_| None,
        &mut problems,
    );
    let out = StyleEngine::new(Viewport {
        width: 1280.0,
        height: 1024.0,
        ..Default::default()
    })
    .style_document(&d, &sheets);

    let body = d.get_elements_by_tag("body")[0];
    let body_style = out.styles.get(body).unwrap();
    assert_eq!(body_style.display, Display::Block);
    assert!(body_style.font_size > 10.0 && body_style.font_size < 30.0);

    // the page's headlines exist and are styled block-level with a real size
    let mut h3 = d.get_elements_by_tag("h3");
    if h3.is_empty() {
        h3 = d.get_elements_by_tag("h2");
    }
    assert!(!h3.is_empty());
    for h in h3.iter().take(20) {
        let s = out.styles.get(*h).unwrap();
        assert_ne!(s.display, Display::Inline, "headings should not be inline");
        assert!(s.font_size > 8.0);
    }

    // the root html element keeps the UA font size
    let html = d.get_elements_by_tag("html")[0];
    let root = out.styles.get(html).unwrap();
    assert!(root.background_color.a > 0, "html has a background");
}

#[test]
fn taz_display_none_is_used_for_hidden_chrome() {
    let d = dom();
    let mut problems = tb_core::Problems::new();
    let sheets = resolve_sources(
        &[
            CssSource::Inline {
                text: fixture("css/styles__news.min.css"),
            },
            CssSource::Inline {
                text: fixture("css/swiper-bundle.min.css"),
            },
        ],
        Viewport::default(),
        |_| None,
        &mut problems,
    );
    let out = StyleEngine::new(Viewport {
        width: 1280.0,
        height: 1024.0,
        ..Default::default()
    })
    .style_document(&d, &sheets);
    let hidden = d
        .elements()
        .iter()
        .filter(|n| out.styles.get(**n).map(|s| s.display == Display::None).unwrap_or(false))
        .count();
    // <head> children alone guarantee a bunch, and swiper/shariff hide more
    assert!(hidden > 20, "expected some display:none elements, got {hidden}");
    // ...but the page must stay mostly visible
    let total = d.elements().len();
    assert!(
        hidden * 2 < total,
        "too much hidden: {hidden}/{total}"
    );
}

#[test]
fn media_queries_in_taz_css_are_viewport_dependent() {
    let d = dom();
    let mut problems = tb_core::Problems::new();
    let sheets = resolve_sources(
        &[CssSource::Inline {
            text: fixture("css/styles__news.min.css"),
        }],
        Viewport::default(),
        |_| None,
        &mut problems,
    );
    let wide = StyleEngine::new(Viewport {
        width: 1600.0,
        height: 1200.0,
        ..Default::default()
    })
    .style_document(&d, &sheets);
    let narrow = StyleEngine::new(Viewport {
        width: 360.0,
        height: 640.0,
        ..Default::default()
    })
    .style_document(&d, &sheets);
    // at least one element must differ between a desktop and a phone viewport
    let mut differing = 0usize;
    for n in d.elements() {
        let a = wide.styles.get(n).unwrap();
        let b = narrow.styles.get(n).unwrap();
        if a.display != b.display
            || (a.font_size - b.font_size).abs() > 0.5
            || (a.width.resolve(1000.0) - b.width.resolve(1000.0)).abs() > 0.5
        {
            differing += 1;
        }
    }
    assert!(differing > 5, "viewport-dependent rules: {differing}");
}
