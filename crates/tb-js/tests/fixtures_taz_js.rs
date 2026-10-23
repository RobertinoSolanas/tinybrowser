//! The real page's own JavaScript: everything taz.de ships inline has to go
//! through our parser. This is the acceptance test for T7.7 — third-party code
//! must not panic us, and it must not be *rejected* either, because a script we
//! cannot parse is a feature the page loses.

use std::path::PathBuf;
use std::time::Instant;
use tb_js::Parser;

fn fixture(name: &str) -> String {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("../../fixtures");
    p.push(name);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("fixture {name}: {e}"))
}

/// Inline `<script>` bodies, plus their (lowercased) `type` attribute — `None`
/// for plain classic scripts. Deliberately a small scanner: tb-js must not
/// depend on tb-html, and this only has to survive one known snapshot.
fn inline_scripts(html: &str) -> Vec<(Option<String>, String)> {
    // ASCII lower-casing keeps byte offsets, so spans stay valid for `html`
    let lower = html.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut at = 0usize;
    while let Some(i) = lower[at..].find("<script") {
        let open = at + i;
        let Some(g) = tag_end(&lower, open) else {
            break;
        };
        let attrs = &lower[open + "<script".len()..g];
        let body_start = g + 1;
        let body_end = lower[body_start..]
            .find("</script")
            .map(|e| body_start + e)
            .unwrap_or(html.len());
        // a script with `src` has no inline body
        if !attrs.contains("src=") && !html[body_start..body_end].trim().is_empty() {
            out.push((
                attr(attrs, "type").map(|t| t.trim().to_ascii_lowercase()),
                html[body_start..body_end].to_string(),
            ));
        }
        at = body_end + "</script".len();
    }
    out
}

/// Scripts the engine would actually execute as classic JavaScript.
fn is_classic(t: Option<&str>) -> bool {
    matches!(
        t,
        None | Some("") | Some("javascript") | Some("text/javascript")
    )
}

/// End of the opening tag: the first `>` outside a quoted attribute value.
/// (Real pages put `>` inside `onload="…"`, so a plain `find('>')` is wrong.)
fn tag_end(lower: &str, from: usize) -> Option<usize> {
    let bytes = lower.as_bytes();
    let mut i = from + "<script".len();
    let mut quote: Option<u8> = None;
    while i < bytes.len() {
        match bytes[i] {
            q @ (b'"' | b'\'') if Some(q) == quote => quote = None,
            q @ (b'"' | b'\'') if quote.is_none() => quote = Some(q),
            b'>' if quote.is_none() => return Some(i),
            _ => {}
        }
        i += 1;
    }
    None
}

/// `name="value"` or `name=value` inside a tag's attribute text.
fn attr(attrs: &str, name: &str) -> Option<String> {
    let i = attrs.find(&format!("{name}="))?;
    let rest = &attrs[i + name.len() + 1..];
    let first = rest.as_bytes().first().copied().unwrap_or(b' ');
    if first == b'"' || first == b'\'' {
        let inner = &rest[1..];
        let end = inner.find(first as char)?;
        Some(inner[..end].to_string())
    } else {
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        Some(rest[..end].to_string())
    }
}

#[test]
fn every_inline_script_of_the_page_parses() {
    let html = fixture("taz.de.html");
    let scripts = inline_scripts(&html);
    assert!(
        scripts.len() >= 10,
        "the snapshot should carry a dozen inline scripts, found {}",
        scripts.len()
    );
    // JSON-LD and friends are not JavaScript — the loader must not even try
    let data = scripts
        .iter()
        .filter(|(t, _)| {
            matches!(
                t.as_deref(),
                Some("application/ld+json") | Some("application/json")
            )
        })
        .count();

    let mut total = 0usize;
    let mut modules = 0usize;
    let mut failed: Vec<(usize, String)> = Vec::new();
    for (ty, body) in &scripts {
        // must never panic — for every kind of script
        let mut problems = Vec::new();
        let tolerant = Parser::parse_tolerant(body, &mut problems);
        if !is_classic(ty.as_deref()) {
            // ES modules (`type="module"`) are not executed as classic scripts
            // (the loader skips them, see todolist T8.4); they still must not
            // take the parser down.
            modules += 1;
            std::hint::black_box(&tolerant);
            continue;
        }
        total += body.len();
        match Parser::parse_program(body) {
            Ok(p) => {
                std::hint::black_box(&p);
            }
            Err(e) => failed.push((body.len(), format!("{e}"))),
        }
    }
    assert!(
        failed.is_empty(),
        "{} classic inline scripts failed to parse (data blocks skipped: {data}, \
         modules skipped: {modules}): {:#?}",
        failed.len(),
        failed
    );
    assert!(
        total > 20_000,
        "expected tens of KB of real JS, got {total}"
    );
}

#[test]
fn module_scripts_are_tolerated_without_panic() {
    // `import`/`export` is module syntax we do not execute; a page full of them
    // must still be survivable (and the message should say what is missing)
    let html = fixture("taz.de.html");
    let mut seen = 0;
    for (ty, body) in inline_scripts(&html) {
        if ty.as_deref() != Some("module") {
            continue;
        }
        seen += 1;
        let mut problems = Vec::new();
        let _ = Parser::parse_tolerant(&body, &mut problems);
        match Parser::parse_program(&body) {
            // either we understand it, or we report and move on — never panic
            Ok(_) | Err(_) => {}
        }
    }
    assert!(seen > 0, "the snapshot should contain module scripts");
}

#[test]
fn the_pages_scripts_parse_in_milliseconds() {
    let html = fixture("taz.de.html");
    let scripts = inline_scripts(&html);
    let scripts: Vec<String> = scripts
        .into_iter()
        .filter(|(ty, _)| is_classic(ty.as_deref()))
        .map(|(_, body)| body)
        .collect();
    let bytes: usize = scripts.iter().map(|s| s.len()).sum();
    let t = Instant::now();
    let mut statements = 0usize;
    for _ in 0..20 {
        for body in &scripts {
            let p = Parser::parse_program(body).expect("classic script parses");
            statements += p.body.len();
        }
    }
    let ms = t.elapsed().as_secs_f64() * 1000.0 / 20.0;
    assert!(
        statements > 20,
        "the page's scripts hold statements: {statements}"
    );
    // ~80 KB of real minified JS in well under a frame
    assert!(
        ms < 100.0,
        "one pass over {bytes} bytes of JS took {ms:.1} ms"
    );
    eprintln!("taz.de inline JS: {bytes} bytes → {statements} statements in {ms:.2} ms");
}

#[test]
fn the_committed_js_fixture_parses_clean_and_fast() {
    // the same 12 scripts as one file — the benchmark the README quotes
    let src = fixture("js/taz-inline.js");
    assert!(
        src.len() > 40_000,
        "the fixture is the whole page's JS: {} B",
        src.len()
    );
    let t = Instant::now();
    let mut statements = 0usize;
    for _ in 0..50 {
        let p = Parser::parse_program(&src).expect("the page's JS must parse");
        statements += p.body.len();
    }
    let ms = t.elapsed().as_secs_f64() * 1000.0 / 50.0;
    assert!(statements > 40, "{statements} statements");
    assert!(
        ms < 25.0,
        "{} KB of real JS took {ms:.2} ms",
        src.len() / 1024
    );
    eprintln!(
        "fixtures/js/taz-inline.js: {} KB, {statements} statements, {ms:.2} ms/pass",
        src.len() / 1024
    );
}

#[test]
fn the_scripts_are_small_enough_for_the_depth_limit() {
    // depth is about *nesting*, and minified vendor code nests a lot — our cap
    // has to leave room for what real pages do
    let html = fixture("taz.de.html");
    for (_, body) in inline_scripts(&html) {
        let mut depth = 0u32;
        let mut max = 0u32;
        for c in body.chars() {
            match c {
                '{' | '(' | '[' => {
                    depth += 1;
                    max = max.max(depth);
                }
                '}' | ')' | ']' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
        assert!(
            max < tb_js::parser::MAX_NEST,
            "a script nests {} levels deep, limit is {}",
            max,
            tb_js::parser::MAX_NEST
        );
    }
}
