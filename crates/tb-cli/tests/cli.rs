//! The CLI: what a person typing `tiny` gets.
//!
//! The binary is invoked for real (`CARGO_BIN_EXE_tiny`) against pages written
//! into a temp directory, so exit codes, stdout shape and the PNG file are all
//! covered the way a user experiences them.

use std::io::BufRead;
use std::path::PathBuf;
use std::process::{Command, Output};

fn tiny() -> Command {
    Command::new(env!("CARGO_BIN_EXE_tiny"))
}

fn dir() -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("tiny-cli-{}", std::process::id()));
    std::fs::create_dir_all(&p).expect("temp dir");
    p
}

/// Write a page into the temp dir and return its path as a string.
fn page(name: &str, html: &str) -> String {
    let path = dir().join(name);
    std::fs::write(&path, html).expect("write page");
    path.to_string_lossy().to_string()
}

fn run(args: &[&str]) -> Output {
    tiny().args(args).output().expect("tiny runs")
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).to_string()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).to_string()
}

const PAGE: &str = r#"<!doctype html>
<html><head><title>Cli page</title>
<style>h1 { color: rgb(1, 2, 3); } .hide { display: none }</style>
</head>
<body>
<h1>Head</h1>
<p>Some <a href="/deep/place">inner</a> and <a href="https://example.com/x">outer</a> text.</p>
<p class=hide>secret</p>
<script>console.log('cli-script-ran', document.querySelectorAll('a').length);</script>
</body></html>"#;

#[test]
fn a_bare_run_summarises_the_page() {
    let path = page("summary.html", PAGE);
    let out = run(&[&path]);
    assert!(out.status.success(), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("Cli page"), "{text}");
    assert!(text.contains("elements:"), "{text}");
    assert!(text.contains("links: 2"), "{text}");
    assert!(text.contains("css:"), "{text}");
    assert!(text.contains("js:"), "{text}");
    // the summary must not swallow the page itself
    assert!(!text.contains("Some inner"), "{text}");
}

#[test]
fn text_gives_the_visible_words_only() {
    let path = page("text.html", PAGE);
    let out = run(&["--text", &path]);
    let text = stdout(&out);
    assert!(text.contains("Head"), "{text}");
    assert!(text.contains("inner"), "{text}");
    assert!(!text.contains("secret"), "display:none is not text: {text}");
    assert!(!text.contains("elements:"), "--text alone: {text}");
    assert!(!text.contains("rgb(1, 2, 3)"), "{text}");
}

#[test]
fn links_are_listed_resolved() {
    let path = page("links.html", PAGE);
    let out = run(&["--links", &path]);
    let text = stdout(&out);
    assert!(text.contains("https://example.com/x"), "{text}");
    assert!(text.contains("inner"), "{text}");
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines.len(), 2, "{text}");
}

#[test]
fn dom_shows_the_tree_and_css_the_sheets() {
    let path = page("dom.html", PAGE);
    let out = run(&["--dom", &path]);
    let text = stdout(&out);
    assert!(text.contains("<html>"), "{text}");
    assert!(text.contains("<h1>"), "{text}");
    assert!(text.contains("a href=/deep/place"), "{text}");

    let out = run(&["--css", &path]);
    let text = stdout(&out);
    assert!(text.contains("<style>"), "{text}");
    assert!(text.contains("rules"), "{text}");
}

#[test]
fn layout_prints_the_box_tree() {
    let path = page("layout.html", PAGE);
    let out = run(&["--layout", &path]);
    let text = stdout(&out);
    assert!(text.contains("<body>"), "{text}");
    assert!(text.contains("1280"), "viewport width in geometry: {text}");
}

#[test]
fn console_output_comes_through() {
    let path = page("console.html", PAGE);
    let out = run(&["--console", &path]);
    let text = stdout(&out);
    assert!(text.contains("[log] cli-script-ran 2"), "{text}");
}

#[test]
fn no_js_leaves_scripts_alone() {
    let html = "<title>untouched</title><p>x</p><script>document.title='changed';</script>";
    let path = page("nojs.html", html);
    let out = run(&["--no-js", "--quiet", &path]);
    let text = stdout(&out);
    assert!(text.is_empty(), "--no-js --quiet prints nothing: {text:?}");

    let out = run(&["--no-js", &path]);
    let text = stdout(&out);
    assert!(text.contains("untouched"), "{text}");
    assert!(text.contains("js:       off"), "{text}");
}

#[test]
fn a_screenshot_is_written_and_reported() {
    let path = page("shot.html", PAGE);
    let png = dir().join("page.png");
    let _ = std::fs::remove_file(&png);
    let out = run(&["--screenshot", png.to_str().unwrap(), &path]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(png.exists(), "the PNG must exist");
    let bytes = std::fs::read(&png).expect("read png");
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "a real PNG");
    let text = stdout(&out);
    assert!(text.contains("screenshot:"), "{text}");
    assert!(text.contains("1280x1024"), "{text}");
    assert!(
        !text.contains("BLANK"),
        "a rendered page is not blank: {text}"
    );
}

#[test]
fn viewport_flags_change_the_canvas() {
    let path = page("vp.html", PAGE);
    let png = dir().join("vp.png");
    let out = run(&[
        "--screenshot",
        png.to_str().unwrap(),
        "--width",
        "600",
        "--height",
        "400",
        "--quiet",
        &path,
    ]);
    assert!(out.status.success(), "{}", stderr(&out));
    let bytes = std::fs::read(&png).expect("png");
    // IHDR: width and height are the 4-byte big-endian fields at 16 and 20
    let w = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
    let h = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
    assert_eq!((w, h), (600, 400), "PNG header says {w}x{h}");
}

#[test]
fn problems_are_reportable() {
    let html = "<p>ok</p><script>this is not javascript (((</script>";
    let path = page("broken.html", html);
    let out = run(&["--problems", "--quiet", &path]);
    let text = stdout(&out);
    assert!(
        text.contains("script") || text.contains("Syntax") || text.contains("SyntaxError"),
        "a broken script should show up: {text}"
    );
    // and the page itself still loads
    let out = run(&["--text", &path]);
    assert!(stdout(&out).contains("ok"), "{}", stdout(&out));
}

#[test]
fn unknown_options_are_a_usage_error() {
    let out = run(&["--frobnicate", "x"]);
    assert_eq!(out.status.code(), Some(2), "usage errors exit 2");
    assert!(stderr(&out).contains("unknown option"), "{}", stderr(&out));
    assert!(stderr(&out).contains("USAGE"), "{}", stderr(&out));
}

#[test]
fn help_explains_the_flags() {
    let out = run(&["--help"]);
    assert!(out.status.success());
    let text = stdout(&out);
    for flag in [
        "--dom",
        "--text",
        "--links",
        "--css",
        "--layout",
        "--screenshot",
        "--no-js",
        "--repl",
    ] {
        assert!(text.contains(flag), "{flag} missing from help");
    }
}

#[test]
fn a_missing_file_exits_nonzero_with_a_message() {
    let out = run(&["/definitely/not/here.html"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("cannot load"), "{}", stderr(&out));
}

#[test]
fn no_arguments_prints_the_usage() {
    let out = tiny().output().expect("run");
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr(&out).contains("USAGE"), "{}", stderr(&out));
}

#[test]
fn a_local_site_can_be_served_and_browsed() {
    // `--serve` exists so demos work without a web server; check it serves a
    // file, then load that file through the browser itself.
    let site = dir().join("site");
    std::fs::create_dir_all(&site).expect("site dir");
    std::fs::write(
        site.join("index.html"),
        "<title>Served</title><p>from the local server</p>",
    )
    .expect("write index");

    let server = Command::new(env!("CARGO_BIN_EXE_tiny"))
        .args(["--serve", site.to_str().unwrap()])
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("serve starts");
    let mut server = server;
    let url = {
        let out = server.stdout.take().expect("stdout piped");
        let mut reader = std::io::BufReader::new(out);
        let mut line = String::new();
        reader.read_line(&mut line).expect("line");
        line.split_whitespace()
            .find(|w| w.starts_with("http://"))
            .unwrap_or_default()
            .to_string()
    };
    assert!(!url.is_empty(), "the server should announce its URL");

    let mut got = None;
    for _ in 0..50 {
        let out = run(&["--text", &url]);
        if stdout(&out).contains("from the local server") {
            got = Some(stdout(&out));
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let _ = server.kill();
    assert!(got.is_some(), "served page should load from {url}");
}
