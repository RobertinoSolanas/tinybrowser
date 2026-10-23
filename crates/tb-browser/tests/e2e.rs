//! End-to-end against a real HTTP server (`tb-testkit`): the whole browser,
//! network included, from a URL to an answer.

use tb_browser::{Engine, EngineConfig, NavRequest};
use tb_testkit::{StubResponse, TestServer};

fn browser() -> Engine {
    Engine::new(EngineConfig::default())
}

#[test]
fn an_external_stylesheet_is_fetched_and_applied() {
    let mut server = TestServer::start();
    server
        .route(
            "/",
            StubResponse::html(
                "<!doctype html><link rel=stylesheet href=/site.css><h1 id=t>Title</h1>",
            ),
        )
        .route(
            "/site.css",
            StubResponse::css("#t { color: rgb(10, 20, 30); }"),
        );
    let url = server.url("/");

    let mut engine = browser();
    let page = engine.load(&url).expect("load");
    assert!(
        server.paths_requested().iter().any(|p| p == "/site.css"),
        "the sheet should have been fetched: {:?}",
        server.paths_requested()
    );
    let t = page
        .dom
        .descendants(page.dom.document())
        .into_iter()
        .find(|n| page.dom.element(*n).and_then(|e| e.element_id()) == Some("t"))
        .expect("#t");
    assert_eq!(
        page.style_of(t).unwrap().color,
        tb_core::Color::rgb(10, 20, 30),
        "the external sheet must be in the cascade"
    );
    assert_eq!(page.stats().stylesheets, 1);
}

#[test]
fn a_script_is_fetched_and_executed_in_document_order() {
    let mut server = TestServer::start();
    server.route(
        "/",
        StubResponse::html(
            "<!doctype html><body><div id=out></div>\
             <script src=/a.js></script>\
             <script>window.order = (window.order||'') + 'inline';</script>\
             </body>",
        ),
    );
    server.route(
        "/a.js",
        StubResponse::js("window.order = (window.order||'') + 'external';"),
    );
    let url = server.url("/");

    let mut engine = browser();
    let mut page = engine.load(&url).expect("load");
    assert_eq!(
        page.eval("window.order").unwrap().to_string(),
        "externalinline",
        "scripts run in document order"
    );
    assert_eq!(page.stats().scripts_run, 2);
    assert_eq!(page.stats().script_failures, 0);
}

#[test]
fn imports_inside_a_sheet_are_followed() {
    let mut server = TestServer::start();
    server
        .route(
            "/",
            StubResponse::html("<link rel=stylesheet href=/main.css><p class=deep>x</p>"),
        )
        .route(
            "/main.css",
            StubResponse::css("@import url(base.css); p { color: red; }"),
        )
        .route(
            "/base.css",
            StubResponse::css(".deep { background: rgb(1, 1, 1); }"),
        );
    let url = server.url("/");

    let mut engine = browser();
    let page = engine.load(&url).unwrap();
    let paths = server.paths_requested();
    assert!(paths.iter().any(|p| p == "/base.css"), "{paths:?}");
    let p = page
        .dom
        .descendants(page.dom.document())
        .into_iter()
        .find(|n| page.dom.element(*n).is_some_and(|e| e.has_class("deep")))
        .unwrap();
    assert_eq!(
        page.style_of(p).unwrap().background_color,
        tb_core::Color::rgb(1, 1, 1)
    );
}

#[test]
fn the_same_resource_is_not_fetched_twice() {
    let mut server = TestServer::start();
    server.route(
        "/",
        StubResponse::html(
            "<link rel=stylesheet href=/one.css><link rel=stylesheet href=/one.css><p>y</p>",
        ),
    );
    server.route("/one.css", StubResponse::css("p{color:red}"));
    let url = server.url("/");

    let mut engine = browser();
    let _page = engine.load(&url).unwrap();
    let hits = server
        .paths_requested()
        .iter()
        .filter(|p| p.as_str() == "/one.css")
        .count();
    assert_eq!(
        hits,
        1,
        "one sheet, one request: {:?}",
        server.paths_requested()
    );
}

#[test]
fn a_broken_stylesheet_is_a_problem_not_a_failure() {
    let mut server = TestServer::start();
    server
        .route(
            "/",
            StubResponse::html("<link rel=stylesheet href=/gone.css><p>x</p>"),
        )
        .route("/gone.css", StubResponse::not_found());
    let url = server.url("/");

    let mut engine = browser();
    let page = engine.load(&url).expect("the page still loads");
    assert_eq!(page.status, 200);
    assert_eq!(page.stats().stylesheets, 0);
    assert!(
        page.problems()
            .all()
            .iter()
            .any(|p| p.message.contains("gone.css")),
        "{:?}",
        page.problems().all()
    );
}

#[test]
fn clicking_a_link_asks_for_a_navigation_and_history_follows() {
    let mut server = TestServer::start();
    server.route(
        "/a",
        StubResponse::html("<title>A</title><a id=go href=/b>go</a>"),
    );
    server.route("/b", StubResponse::html("<title>B</title><p>elsewhere</p>"));
    let url = server.url("/a");

    let mut engine = browser();
    let mut page = engine.load(&url).unwrap();
    assert_eq!(page.title(), "A");
    let link = page
        .dom
        .descendants(page.dom.document())
        .into_iter()
        .find(|n| page.dom.element(*n).and_then(|e| e.element_id()) == Some("go"))
        .expect("#go");
    page.click(link);
    assert!(
        page.nav_requests
            .iter()
            .any(|n| matches!(n, NavRequest::Load(_))),
        "a click on an anchor must ask to navigate: {:?}",
        page.nav_requests
    );
    let href = match &page.nav_requests[0] {
        NavRequest::Load(u) => u.clone(),
        other => panic!("expected Load, got {other:?}"),
    };
    let second = engine.navigate_to(&page, &href).expect("navigate");
    assert_eq!(second.title(), "B");
    assert_eq!(engine.history.len(), 2);

    let back = engine.back().unwrap().expect("back");
    assert_eq!(back.title(), "A");
    assert!(engine.history.current().unwrap().path.ends_with("/a"));
}

#[test]
fn cookies_go_both_ways() {
    let mut server = TestServer::start();
    server.route(
        "/",
        StubResponse::html("<p>hi</p>").with_header("set-cookie", "fromServer=yes; Path=/"),
    );
    server.route("/second", StubResponse::html("<p>2</p>"));
    let url = server.url("/");

    let mut engine = browser();
    let mut page = engine.load(&url).unwrap();
    assert!(
        page.eval("document.cookie")
            .unwrap()
            .to_string()
            .contains("fromServer=yes"),
        "a Set-Cookie must reach document.cookie"
    );
    page.eval("document.cookie = 'fromScript=1; path=/'")
        .unwrap();
    assert!(page.cookie_header().contains("fromScript=1"));

    let second = engine.navigate_to(&page, "/second").unwrap();
    let recorded = server.requests();
    let second_req = recorded
        .iter()
        .find(|r| r.path == "/second")
        .expect("second request recorded");
    let cookie = second_req.header("cookie").unwrap_or("");
    assert!(
        cookie.contains("fromScript=1"),
        "the script's cookie must be sent: {cookie:?}"
    );
    assert!(
        cookie.contains("fromServer=yes"),
        "the server's cookie must survive: {cookie:?}"
    );
    assert_eq!(second.status, 200);
}

#[test]
fn location_href_from_script_becomes_a_navigation_request() {
    let mut server = TestServer::start();
    server.route(
        "/",
        StubResponse::html("<script>location.href = '/moved';</script>"),
    );
    server.route("/moved", StubResponse::html("<title>moved</title>"));
    let url = server.url("/");

    let mut engine = browser();
    let page = engine.load(&url).unwrap();
    let href = page
        .nav_requests
        .iter()
        .find_map(|n| match n {
            NavRequest::Load(u) => Some(u.clone()),
            _ => None,
        })
        .expect("location.href should request a navigation");
    let next = engine.navigate_to(&page, &href).unwrap();
    assert_eq!(next.title(), "moved");
}

#[test]
fn no_js_leaves_the_document_as_delivered() {
    let mut server = TestServer::start();
    server.route(
        "/",
        StubResponse::html("<title>real</title><p>text</p><script>document.title='lied'</script>"),
    );
    let url = server.url("/");

    let mut config = EngineConfig::default();
    config.caps.run_scripts = false;
    let mut engine = Engine::new(config);
    let page = engine.load(&url).unwrap();
    assert_eq!(page.title(), "real");
    assert!(!page.has_js());
    assert_eq!(page.stats().scripts_run, 0);
}

#[test]
fn the_subresource_budget_is_honoured() {
    let mut server = TestServer::start();
    let mut html = String::from("<!doctype html><body><p>x</p>");
    for i in 0..10 {
        html.push_str(&format!("<link rel=stylesheet href=/s{i}.css>"));
    }
    html.push_str("</body>");
    server.route("/", StubResponse::html(&html));
    for i in 0..10 {
        server.route(&format!("/s{i}.css"), StubResponse::css("p{color:red}"));
    }
    let url = server.url("/");

    let config = EngineConfig {
        max_subresources: 3,
        ..Default::default()
    };
    let mut engine = Engine::new(config);
    let page = engine.load(&url).unwrap();
    let css_hits = server
        .paths_requested()
        .iter()
        .filter(|p| p.starts_with("/s"))
        .count();
    assert!(css_hits <= 3, "budget exceeded: {css_hits} requests");
    assert!(
        page.problems()
            .all()
            .iter()
            .any(|p| p.message.contains("budget")),
        "running out of budget should be reported"
    );
}

#[test]
fn a_redirect_ends_up_at_the_final_url() {
    let mut server = TestServer::start();
    server.route("/old", StubResponse::redirect("/new"));
    server.route("/new", StubResponse::html("<title>new place</title>"));
    let url = server.url("/old");

    let mut engine = browser();
    let page = engine.load(&url).unwrap();
    assert_eq!(page.title(), "new place");
    assert!(
        page.url.to_string().ends_with("/new"),
        "final url: {}",
        page.url
    );
}

#[test]
fn a_real_page_shape_survives_the_round_trip() {
    // The shape of an article page: header, nav, article with sections, a form,
    // a script that wires a menu. More realistic than the fixture pages.
    let mut server = TestServer::start();
    server.route(
        "/",
        StubResponse::html(
            r#"<!doctype html>
<html lang=en><head><title>News | Tiny</title>
<link rel=stylesheet href=/style.css></head>
<body>
<header><h1>Tiny News</h1><nav><a href=/a>Local</a><a href=/b>World</a></nav></header>
<main>
 <article><h2>Story one</h2><p>Body of the first story.</p></article>
 <article><h2>Story two</h2><p>Body of the second story.</p></article>
</main>
<form action=/search><input name=q value=""><button>Search</button></form>
<script src=/app.js></script>
</body></html>"#,
        ),
    );
    server.route(
        "/style.css",
        StubResponse::css("h1{font-size:32px} article{border:1px solid rgb(0,0,0);padding:4px} nav a{margin-right:8px}"),
    );
    server.route(
        "/app.js",
        StubResponse::js(
            "const links=[...document.querySelectorAll('nav a')].map(a=>a.textContent);\n\
             console.log('nav:' + links.join(','));\n\
             document.querySelector('h1').id = 'title';",
        ),
    );
    let url = server.url("/");

    let mut engine = browser();
    let page = engine.load(&url).unwrap();
    assert_eq!(page.title(), "News | Tiny");
    assert_eq!(page.headings().len(), 3);
    assert_eq!(page.links().len(), 2);
    assert_eq!(page.console(), vec!["nav:Local,World".to_string()]);
    let s = page.stats();
    assert!(s.boxes > 20, "boxes {}", s.boxes);
    assert!(s.matched_elements > 10);
    assert!(page.text().contains("Body of the first story."));
    // the script's DOM change is visible in the page's own DOM
    assert!(page.dump_dom().contains("#title"), "{}", page.dump_dom());
}
