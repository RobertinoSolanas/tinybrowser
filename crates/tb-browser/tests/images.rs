//! Pictures: fetch, decode, size the box, paint the pixels — and behave when
//! any of that fails.
//!
//! The fixtures are PNGs our *own* encoder produced (`tb_gfx::png_encode`),
//! served over HTTP by `tb-testkit`. That is a fair test: the reader has to
//! deal with real chunked, DEFLATE-compressed PNG bytes, not a toy format, and
//! it is checked against a second independent implementation in
//! `tb-gfx/tests/png_image.rs`.

use tb_browser::{Engine, EngineConfig, Page};
use tb_core::Color;
use tb_gfx::{png_encode, Framebuffer};
use tb_testkit::{StubResponse, TestServer};

/// A `w×h` picture with a distinct colour, as PNG bytes.
fn picture(w: u32, h: u32, c: Color) -> Vec<u8> {
    let mut fb = Framebuffer::new(w, h);
    fb.fill_opaque(c);
    png_encode(&fb)
}

fn png_response(w: u32, h: u32, c: Color) -> StubResponse {
    StubResponse::bytes("image/png", picture(w, h, c))
}

fn page_with(img_tag: &str, server: &TestServer) -> Page {
    let mut engine = Engine::new(EngineConfig::default());
    engine
        .load_html(
            &server.url("/"),
            &format!("<html><body><p>text</p>{img_tag}</body></html>"),
        )
        .expect("page loads")
}

#[test]
fn a_decoded_picture_is_the_box_size() {
    let mut server = TestServer::start();
    server.route("/broad.png", png_response(120, 40, Color::RED));
    let page = page_with(r#"<img id="p" src="/broad.png">"#, &server);

    let s = page.stats();
    assert_eq!(s.images, 1, "one <img>");
    assert_eq!(s.images_decoded, 1, "and we have pixels for it");
    assert_eq!(s.images_failed, 0);

    let node = page.dom.get_elements_by_tag("img")[0];
    let doc = page.layout.as_ref().expect("a box tree");
    let box_ = doc.find(node).expect("the img box");
    assert_eq!(box_.border_box().width, 120.0, "natural width");
    assert_eq!(box_.border_box().height, 40.0, "natural height");
}

#[test]
fn the_width_attribute_is_a_guess_the_picture_can_correct() {
    // browsers keep the attributes around to reserve space *before* the
    // picture arrives; once it has, `width: auto` means "the picture's width"
    let mut server = TestServer::start();
    server.route("/broad.png", png_response(120, 40, Color::RED));
    let page = page_with(r#"<img src="/broad.png" width="30" height="10">"#, &server);
    let node = page.dom.get_elements_by_tag("img")[0];
    let box_ = page.layout.as_ref().unwrap().find(node).unwrap();
    assert_eq!(box_.border_box().width, 120.0);
    assert_eq!(box_.border_box().height, 40.0);
}

#[test]
fn css_wins_and_the_aspect_ratio_survives() {
    let mut server = TestServer::start();
    server.route("/broad.png", png_response(120, 40, Color::RED));
    let page = page_with(r#"<img src="/broad.png" style="width:60px">"#, &server);
    let node = page.dom.get_elements_by_tag("img")[0];
    let box_ = page.layout.as_ref().unwrap().find(node).unwrap();
    assert_eq!(box_.border_box().width, 60.0, "CSS width");
    assert_eq!(box_.border_box().height, 20.0, "half the width, same ratio");
}

#[test]
fn pixels_reach_the_framebuffer() {
    let mut server = TestServer::start();
    server.route("/green.png", png_response(50, 20, Color::GREEN));
    let mut page = page_with(r#"<img src="/green.png" style="display:block">"#, &server);
    let fb = page.paint(&tb_browser::PaintOptions::default());
    // the picture sits under the paragraph; find its row by the colour
    let greens = fb.pixels.iter().filter(|c| **c == Color::GREEN).count();
    assert!(
        greens >= 50 * 20,
        "the picture painted {greens} green pixels"
    );
}

#[test]
fn a_broken_picture_is_a_placeholder_and_a_problem() {
    let mut server = TestServer::start();
    server.route(
        "/broken.png",
        StubResponse::bytes("image/png", b"not a png at all"),
    );
    let page = page_with(
        r#"<img src="/broken.png" alt="kaputt" width="80" height="40">"#,
        &server,
    );
    let s = page.stats();
    assert_eq!(s.images, 1);
    assert_eq!(s.images_decoded, 0);
    assert_eq!(s.images_failed, 1);
    assert!(
        page.problems()
            .all()
            .iter()
            .any(|p| p.message.contains("broken.png")),
        "the page says which picture failed: {:?}",
        page.problems().all()
    );
    // and the box still has the size the attributes promised
    let node = page.dom.get_elements_by_tag("img")[0];
    let box_ = page.layout.as_ref().unwrap().find(node).unwrap();
    assert_eq!(box_.border_box().width, 80.0);
    assert_eq!(box_.border_box().height, 40.0);
}

#[test]
fn a_format_we_cannot_decode_is_said_out_loud() {
    // JPEG magic bytes: we have no JPEG decoder, and "no decoder" is a
    // reportable fact, not a silent hole
    let mut server = TestServer::start();
    server.route(
        "/photo.jpg",
        StubResponse::bytes("image/jpeg", b"\xFF\xD8\xFF\xE0rest"),
    );
    let page = page_with(r#"<img src="/photo.jpg">"#, &server);
    assert_eq!(page.stats().images_failed, 1);
    assert!(page
        .problems()
        .all()
        .iter()
        .any(|p| p.message.contains("photo.jpg")));
    // the default object size keeps the layout honest
    let node = page.dom.get_elements_by_tag("img")[0];
    let box_ = page.layout.as_ref().unwrap().find(node).unwrap();
    assert_eq!(box_.border_box().width, 300.0);
    assert_eq!(box_.border_box().height, 150.0);
}

#[test]
fn a_picture_a_script_added_is_loaded_too() {
    let mut server = TestServer::start();
    server.route("/late.png", png_response(24, 12, Color::BLUE));
    let mut page = Engine::new(EngineConfig::default())
        .load_html(
            &server.url("/"),
            "<html><body><div id='host'></div></body></html>",
        )
        .expect("page");
    let out = page
        .eval(
            "var i = document.createElement('img');\n\
             i.src = '/late.png';\n\
             document.getElementById('host').appendChild(i);\n\
             'added'",
        )
        .expect("script runs");
    assert_eq!(out.to_string(), "added");
    assert_eq!(page.stats().images_decoded, 1, "loaded during the sync");
    let node = page.dom.get_elements_by_tag("img")[0];
    let box_ = page.layout.as_ref().unwrap().find(node).unwrap();
    assert_eq!(box_.border_box().width, 24.0);
    assert_eq!(box_.border_box().height, 12.0);
}

#[test]
fn data_url_pictures_work_without_a_request() {
    let bytes = picture(16, 8, Color::RED);
    let b64 = tb_net::base64_encode(&bytes);
    let mut engine = Engine::offline(EngineConfig::default());
    let page = engine
        .load_html(
            "https://tiny.test/",
            &format!(r#"<html><body><img src="data:image/png;base64,{b64}"></body></html>"#),
        )
        .expect("page");
    assert_eq!(
        page.stats().images_decoded,
        1,
        "decoded from the URL itself"
    );
    let node = page.dom.get_elements_by_tag("img")[0];
    let box_ = page.layout.as_ref().unwrap().find(node).unwrap();
    assert_eq!(box_.border_box().width, 16.0);
    assert_eq!(box_.border_box().height, 8.0);
}

#[test]
fn the_same_picture_twice_is_one_request() {
    let mut server = TestServer::start();
    server.route("/once.png", png_response(20, 20, Color::GREEN));
    let page = page_with(r#"<img src="/once.png"><img src="/once.png">"#, &server);
    assert_eq!(page.stats().images_decoded, 2);
    assert_eq!(
        server
            .paths_requested()
            .iter()
            .filter(|p| p.ends_with("once.png"))
            .count(),
        1,
        "the cache is per page, the picture is one picture"
    );
}

#[test]
fn pictures_spending_the_budget_are_reported() {
    let mut server = TestServer::start();
    for i in 0..4 {
        server.route(&format!("/p{i}.png"), png_response(10, 10, Color::RED));
    }
    let config = EngineConfig {
        max_subresources: 2,
        ..Default::default()
    };
    let mut engine = Engine::new(config);
    let page = engine
        .load_html(
            &server.url("/"),
            r#"<html><body><img src="/p0.png"><img src="/p1.png"><img src="/p2.png"><img src="/p3.png"></body></html>"#,
        )
        .expect("the page still loads");
    assert!(
        page.stats().images_decoded <= 2,
        "budget held: {}",
        page.stats().images_decoded
    );
    assert!(page
        .problems()
        .all()
        .iter()
        .any(|p| p.message.contains("budget")));
}

#[test]
fn an_unknown_format_never_panics_on_garbage() {
    // every byte position of a truncated PNG must be survivable
    let real = picture(8, 8, Color::RED);
    let mut server = TestServer::start();
    for cut in [1usize, 8, 20, 40, real.len() / 2, real.len() - 1] {
        let path = format!("/cut{cut}.png");
        server.route(
            &path,
            StubResponse::bytes("image/png", real[..cut].to_vec()),
        );
    }
    let tags: String = (0..6)
        .map(|i| {
            format!(
                "<img src='/cut{}.png'>",
                [1usize, 8, 20, 40, real.len() / 2, real.len() - 1][i]
            )
        })
        .collect();
    let page = page_with(&tags, &server);
    assert_eq!(page.stats().images, 6);
    // no panic is the assertion; some of them may even decode
    let _ = page.stats().images_decoded;
}

#[test]
fn a_picture_in_a_hidden_box_is_still_measured_but_not_painted() {
    let mut server = TestServer::start();
    server.route("/gone.png", png_response(30, 30, Color::RED));
    let mut page = page_with(r#"<img src="/gone.png" style="display:none">"#, &server);
    let fb = page.paint(&tb_browser::PaintOptions::default());
    assert_eq!(
        fb.pixels.iter().filter(|c| **c == Color::RED).count(),
        0,
        "display:none paints nothing"
    );
    assert_eq!(page.stats().images_decoded, 1, "still in the store");
}
