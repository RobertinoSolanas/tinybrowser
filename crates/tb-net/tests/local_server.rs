//! Integration tests of the retrieval context against the in-process server.

use tb_net::{HttpClient, Url};
use tb_testkit::{StubResponse, TestServer};

fn client() -> HttpClient {
    HttpClient::new(tb_core::EngineConfig {
        timeout_ms: 3_000,
        ..Default::default()
    })
}

#[test]
fn gets_a_plain_page() {
    let mut server = TestServer::start();
    server.route(
        "/index.html",
        StubResponse::html("<html><body>hi</body></html>"),
    );
    let c = client();
    let url = Url::parse(&server.url("/index.html")).unwrap();
    let r = c.fetch(&url).unwrap();
    assert_eq!(r.response.status, 200);
    assert_eq!(r.text.as_deref(), Some("<html><body>hi</body></html>"));
    assert!(!r.from_cache);
    let req = &server.requests()[0];
    assert_eq!(req.method, "GET");
    assert_eq!(req.path, "/index.html");
    assert_eq!(req.header("user-agent"), Some(tb_core::USER_AGENT));
}

#[test]
fn follows_redirect_chains() {
    let mut server = TestServer::start();
    server.route("/", StubResponse::redirect("/start"));
    server.route("/start", StubResponse::redirect("/final.html"));
    server.route("/final.html", StubResponse::html("<p>final</p>"));
    let c = client();
    let url = Url::parse(&server.url("/")).unwrap();
    let r = c.fetch(&url).unwrap();
    assert_eq!(r.response.status, 200);
    assert_eq!(r.text.as_deref(), Some("<p>final</p>"));
    assert_eq!(r.response.url, server.url("/final.html"));
    assert_eq!(server.request_count(), 3);
}

#[test]
fn stops_after_too_many_redirects() {
    let mut server = TestServer::start();
    server.route("/loop", StubResponse::redirect("/loop"));
    let c = client();
    let url = Url::parse(&server.url("/loop")).unwrap();
    let e = c.fetch(&url).unwrap_err();
    assert!(
        matches!(e, tb_core::TbError::TooManyRedirects { .. }),
        "{e}"
    );
}

#[test]
fn decodes_chunked_bodies() {
    let mut server = TestServer::start();
    let big = "x".repeat(500);
    server.route("/chunked", StubResponse::html(big.clone()).chunked());
    let c = client();
    let r = c
        .fetch(&Url::parse(&server.url("/chunked")).unwrap())
        .unwrap();
    assert_eq!(r.text.as_deref().unwrap().len(), big.len());
}

#[test]
fn inflates_gzip_content_encoding() {
    let mut server = TestServer::start();
    let body = "taz.de gzip test ".repeat(50);
    server.route(
        "/gz",
        StubResponse::html(body.clone())
            .gzipped()
            .with_header("content-encoding", "gzip"),
    );
    let c = client();
    let r = c.fetch(&Url::parse(&server.url("/gz")).unwrap()).unwrap();
    assert_eq!(r.text.as_deref(), Some(body.as_str()));
}

#[test]
fn non_success_status_is_an_error_with_context() {
    let server = TestServer::start();
    let c = client();
    let e = c
        .fetch(&Url::parse(&server.url("/missing")).unwrap())
        .unwrap_err();
    match e {
        tb_core::TbError::HttpStatus { status, url } => {
            assert_eq!(status, 404);
            assert!(url.ends_with("/missing"));
        }
        other => panic!("expected http status error, got {other}"),
    }
}

#[test]
fn sends_cookies_back_and_forth() {
    let mut server = TestServer::start();
    server.route(
        "/login",
        StubResponse::html("ok").with_header("set-cookie", "session=xyz; Path=/"),
    );
    server.route(
        "/profile",
        StubResponse::html("<p id=u>...</p>").with_header("cache-control", "no-store"),
    );
    let c = client();
    c.fetch(&Url::parse(&server.url("/login")).unwrap())
        .unwrap();
    c.fetch(&Url::parse(&server.url("/profile")).unwrap())
        .unwrap();
    let profile = server
        .requests()
        .into_iter()
        .find(|r| r.path == "/profile")
        .unwrap();
    assert_eq!(profile.header("cookie"), Some("session=xyz"));
}

#[test]
fn second_request_is_served_from_cache() {
    let mut server = TestServer::start();
    server.route(
        "/style.css",
        StubResponse::css("body{color:red}").with_header("cache-control", "max-age=600"),
    );
    let c = client();
    let url = Url::parse(&server.url("/style.css")).unwrap();
    let first = c.fetch(&url).unwrap();
    let second = c.fetch(&url).unwrap();
    assert!(!first.from_cache);
    assert!(second.from_cache);
    assert_eq!(
        server.request_count(),
        1,
        "cached response must not hit the wire"
    );
}

#[test]
fn revalidates_with_etag_and_handles_304() {
    let mut server = TestServer::start();
    server.route(
        "/fresh.css",
        StubResponse::css("a{color:blue}")
            .with_header("cache-control", "max-age=0")
            .with_header("etag", "\"v1\""),
    );
    let c = client();
    let url = Url::parse(&server.url("/fresh.css")).unwrap();
    let first = c.fetch(&url).unwrap();
    assert_eq!(first.text.as_deref(), Some("a{color:blue}"));
    let second = c.fetch(&url).unwrap();
    assert_eq!(second.text.as_deref(), Some("a{color:blue}"));
    let reqs = server.requests();
    assert_eq!(reqs.len(), 2);
    assert_eq!(reqs[1].header("if-none-match"), Some("\"v1\""));
}

#[test]
fn timeout_is_reported_as_error() {
    let mut server = TestServer::start();
    server.route("/slow", StubResponse::html("late").delayed(1_200));
    let c = HttpClient::new(tb_core::EngineConfig {
        timeout_ms: 150,
        ..Default::default()
    });
    let e = c.fetch(&Url::parse(&server.url("/slow")).unwrap());
    assert!(e.is_err(), "slow response must time out");
}

#[test]
fn referer_header_is_sent_when_configured() {
    let mut server = TestServer::start();
    server.route("/a", StubResponse::html("<a href=/b>b</a>"));
    server.route("/b", StubResponse::html("<p>b</p>"));
    let c = client();
    let base = Url::parse(&server.url("/a")).unwrap();
    c.send(&Url::parse(&server.url("/b")).unwrap(), Some(&base))
        .unwrap();
    let req = server.requests().pop().unwrap();
    let expected = server.url("/a");
    assert_eq!(req.header("referer"), Some(expected.as_str()));
}
