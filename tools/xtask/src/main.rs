//! # xtask — the workspace's own tools
//!
//! `cargo junit` (see `.cargo/config.toml`) runs the whole test suite and
//! writes `target/junit.xml` in the JUnit XML format CI tools expect.
//!
//! It shells out to `cargo test` per package — the crate names come from the
//! workspace `Cargo.toml`, which is what makes the suite names in the report
//! mean something (`tb_css::tests::parser` rather than a binary with a hash) —
//! and parses cargo's own output. No extra dependencies, stable Rust only.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None | Some("junit") => junit(&args[1..]),
        Some("help") | Some("--help") | Some("-h") => {
            usage();
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("xtask: unknown task {other}\n");
            usage();
            ExitCode::from(2)
        }
    }
}

fn usage() {
    println!(
        "\
xtask — tinybrowser workspace tooling

    cargo junit [cargo test args]
        run the whole workspace test suite and write target/junit.xml
        (the run's exit status is preserved: a failing test fails the command)

    cargo junit -- --test dom
        arguments after `--` are passed on to `cargo test`"
    );
}

/// One `<testcase>`.
#[derive(Debug)]
struct Case {
    name: String,
    status: Status,
    output: String,
}

#[derive(Debug, PartialEq, Clone, Copy)]
enum Status {
    Passed,
    Failed,
    Skipped,
}

/// One `<testsuite>`: a cargo test target of one package.
#[derive(Debug)]
struct Suite {
    name: String,
    package: String,
    seconds: f64,
    cases: Vec<Case>,
}

impl Suite {
    fn count(&self, s: Status) -> usize {
        self.cases.iter().filter(|c| c.status == s).count()
    }
}

fn junit(extra: &[String]) -> ExitCode {
    let root = match workspace_root() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("xtask: {e}");
            return ExitCode::from(2);
        }
    };
    let packages = workspace_members(&root);
    if packages.is_empty() {
        eprintln!(
            "xtask: no workspace members found in {}",
            root.join("Cargo.toml").display()
        );
        return ExitCode::from(2);
    }

    let started = std::time::Instant::now();
    let mut suites: Vec<Suite> = Vec::new();
    let mut failed_run = false;

    for pkg in &packages {
        let mut cmd = Command::new("cargo");
        cmd.arg("test").arg("-p").arg(pkg).arg("--");
        cmd.args(extra);
        let out = match cmd.current_dir(&root).output() {
            Ok(o) => o,
            Err(e) => {
                eprintln!("xtask: cannot run cargo test for {pkg}: {e}");
                return ExitCode::from(1);
            }
        };
        // cargo prints its own status lines ("Running tests/dom.rs (…)") to
        // stderr and the test harness's lines to stdout. Capturing the two
        // separately loses their relative order, so the target names are taken
        // from stderr as an ordered list and matched to the harness's
        // "running N tests" blocks in stdout.
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        let text = format!("{stdout}{stderr}");
        // let the human watch the same run the XML is built from
        let _ = std::io::stdout().write_all(text.as_bytes());
        let _ = std::io::stdout().flush();
        let headers = target_headers(&stderr);
        let parsed = parse_run(pkg, &stdout, &headers);
        if parsed.is_empty() {
            // a package with no tests at all is not a failure
            if !out.status.success() {
                failed_run = true;
                suites.push(Suite {
                    name: format!("{pkg}::build"),
                    package: pkg.clone(),
                    seconds: 0.0,
                    cases: vec![Case {
                        name: "cargo test".into(),
                        status: Status::Failed,
                        output: text,
                    }],
                });
            }
        } else {
            if !out.status.success() {
                failed_run = true;
            }
            suites.extend(parsed);
        }
    }

    let tests: usize = suites.iter().map(|s| s.cases.len()).sum();
    let failures: usize = suites.iter().map(|s| s.count(Status::Failed)).sum();
    let skipped: usize = suites.iter().map(|s| s.count(Status::Skipped)).sum();
    let xml = render(&suites, started.elapsed().as_secs_f64());
    let path = root.join("target").join("junit.xml");
    if let Err(e) = write(&path, &xml) {
        eprintln!("xtask: {e}");
        return ExitCode::from(1);
    }
    println!(
        "\njunit: {} ({} suites, {tests} tests, {failures} failures, {skipped} skipped)",
        path.display(),
        suites.len()
    );
    if failed_run || failures > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

/// Write `path` with `content`, creating parent directories.
fn write(path: &Path, content: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(path, content).map_err(|e| format!("{}: {e}", path.display()))
}

// ------------------------------------------------------------- cargo output -

/// The ordered list of test targets cargo announced on stderr.
fn target_headers(stderr: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in stderr.lines() {
        let trimmed = line.trim_end();
        if let Some(rest) = extract(trimmed, "Running ") {
            out.push(rest.trim().to_string());
        } else if let Some(rest) = extract(trimmed, "Doc-tests ") {
            out.push(format!("doc {}", rest.trim()));
        }
    }
    out
}

/// Parse one package's harness output (`stdout`) into one suite per target;
/// `headers` (from cargo's stderr) name the targets in the same order.
fn parse_run(package: &str, text: &str, headers: &[String]) -> Vec<Suite> {
    let mut suites: Vec<Suite> = Vec::new();
    let mut current: Option<Suite> = None;
    // captured stdout of a failing test: `---- name stdout ----`
    let mut capturing: Option<String> = None;
    let mut capture_buf = String::new();
    // captured stdout is applied at the end: while we parse, the suite a test
    // belongs to may still be the one we are building
    let mut captured: Vec<(String, String)> = Vec::new();
    let mut header_index = 0usize;

    macro_rules! flush_capture {
        () => {
            if let Some(name) = capturing.take() {
                captured.push((name, std::mem::take(&mut capture_buf)));
            }
        };
    }

    for line in text.lines() {
        let trimmed = line.trim_end();

        // ---- a failing test's captured output
        if let Some(rest) = trimmed.strip_prefix("---- ") {
            if let Some(rest) = rest.strip_suffix(" stdout ----") {
                flush_capture!();
                capturing = Some(rest.trim().to_string());
                continue;
            }
        } else if capturing.is_some() {
            if trimmed.starts_with("failures:")
                || trimmed.starts_with("test result:")
                || trimmed.starts_with("running ")
            {
                flush_capture!();
            } else {
                capture_buf.push_str(trimmed);
                capture_buf.push('\n');
                continue;
            }
        }

        // ---- a new test target starts here ("running 12 tests")
        if extract(trimmed, "running ").is_some() {
            let name = match headers.get(header_index) {
                Some(h) => target_name(package, h),
                None => format!("{package}::tests"),
            };
            header_index += 1;
            current = Some(Suite {
                name,
                package: package.to_string(),
                seconds: 0.0,
                cases: Vec::new(),
            });
            continue;
        }

        // ---- one test result
        if let Some(rest) = extract(trimmed, "test ") {
            if let Some((name, status)) = split_test_line(rest) {
                let suite = current.get_or_insert_with(|| Suite {
                    name: format!("{package}::tests"),
                    package: package.to_string(),
                    seconds: 0.0,
                    cases: Vec::new(),
                });
                suite.cases.push(Case {
                    name: name.to_string(),
                    status,
                    output: String::new(),
                });
                continue;
            }
        }

        // ---- end of a target: totals and wall time
        if trimmed.starts_with("test result:") {
            if let Some(mut suite) = current.take() {
                if let Some(secs) = parse_seconds(trimmed) {
                    suite.seconds = secs;
                }
                suites.push(suite);
            }
            continue;
        }
    }
    flush_capture!();
    if let Some(suite) = current.take() {
        suites.push(suite);
    }
    for (name, out) in captured {
        for suite in suites.iter_mut() {
            if let Some(case) = suite.cases.iter_mut().find(|c| c.name == name) {
                case.output = out;
                break;
            }
        }
    }
    suites
}

/// `tests/dom.rs (target/debug/deps/dom-1234)` → `tb_dom_bindings::tests::dom`
fn target_name(package: &str, running: &str) -> String {
    let crate_name = package.replace('-', "_");
    if let Some(rest) = running.strip_prefix("doc ") {
        return format!("{}::doc", rest.trim().replace('-', "_"));
    }
    let path = running.split('(').next().unwrap_or(running).trim();
    if path.ends_with("src/lib.rs") || path.ends_with("src/main.rs") {
        return format!("{crate_name}::unit");
    }
    if let Some(rest) = path.strip_prefix("tests/") {
        let stem = rest.rsplit('/').next().unwrap_or(rest);
        let stem = stem.strip_suffix(".rs").unwrap_or(stem);
        return format!("{crate_name}::tests::{stem}");
    }
    format!("{crate_name}::{path}")
}

/// `foo::bar ... ok` → `("foo::bar", Status)`
fn split_test_line(rest: &str) -> Option<(&str, Status)> {
    let idx = rest.rfind(" ... ")?;
    let (name, tail) = (&rest[..idx], rest[idx + 5..].trim());
    let status = match tail {
        "ok" => Status::Passed,
        "FAILED" => Status::Failed,
        "ignored" => Status::Skipped,
        _ => return None,
    };
    Some((name, status))
}

/// `... finished in 0.12s` → 0.12
fn parse_seconds(line: &str) -> Option<f64> {
    let after = line.rsplit("finished in ").next()?;
    let num: String = after
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    num.parse().ok()
}

fn extract<'a>(line: &'a str, prefix: &str) -> Option<&'a str> {
    let idx = line.find(prefix)?;
    // only cargo's own status lines, which start with whitespace + prefix
    let before = &line[..idx];
    if !before.chars().all(|c| c == ' ') {
        return None;
    }
    Some(&line[idx + prefix.len()..])
}

// -------------------------------------------------------------------- xml ---

fn render(suites: &[Suite], total_seconds: f64) -> String {
    let tests: usize = suites.iter().map(|s| s.cases.len()).sum();
    let failures: usize = suites.iter().map(|s| s.count(Status::Failed)).sum();
    let skipped: usize = suites.iter().map(|s| s.count(Status::Skipped)).sum();
    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(&format!(
        "<testsuites name=\"tinybrowser\" tests=\"{tests}\" failures=\"{failures}\" skipped=\"{skipped}\" time=\"{total_seconds:.3}\">\n"
    ));
    for suite in suites {
        let f = suite.count(Status::Failed);
        let s = suite.count(Status::Skipped);
        out.push_str(&format!(
            "  <testsuite name=\"{}\" package=\"{}\" tests=\"{}\" failures=\"{f}\" skipped=\"{s}\" time=\"{:.3}\">\n",
            esc(&suite.name),
            esc(&suite.package),
            suite.cases.len(),
            suite.seconds
        ));
        for case in &suite.cases {
            out.push_str(&format!(
                "    <testcase classname=\"{}\" name=\"{}\"",
                esc(&suite.name),
                esc(&case.name)
            ));
            match case.status {
                Status::Passed => {
                    if case.output.is_empty() {
                        out.push_str("/>\n");
                    } else {
                        out.push_str(&format!(
                            ">\n      <system-out>{}</system-out>\n    </testcase>\n",
                            esc(&case.output)
                        ));
                    }
                }
                Status::Skipped => {
                    out.push_str(">\n      <skipped/>\n    </testcase>\n");
                }
                Status::Failed => {
                    out.push_str(">\n");
                    out.push_str(&format!(
                        "      <failure message=\"test {} failed\">{}</failure>\n",
                        esc(&case.name),
                        esc(&case.output)
                    ));
                    out.push_str("    </testcase>\n");
                }
            }
        }
        out.push_str("  </testsuite>\n");
    }
    out.push_str("</testsuites>\n");
    out
}

fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // XML 1.0 has no representation for these
            c if (c as u32) < 0x20 && !matches!(c, '\n' | '\r' | '\t') => {}
            c => out.push(c),
        }
    }
    out
}

// ------------------------------------------------------------------ cargo ---

/// The directory with the workspace `Cargo.toml`.
fn workspace_root() -> Result<PathBuf, String> {
    let out = Command::new("cargo")
        .args(["locate-project", "--workspace", "--message-format=plain"])
        .output()
        .map_err(|e| format!("cargo locate-project: {e}"))?;
    if !out.status.success() {
        return Err("cargo locate-project failed".into());
    }
    let path = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    let root = path
        .parent()
        .ok_or_else(|| "no parent of Cargo.toml".to_string())?
        .to_path_buf();
    Ok(root)
}

/// The `members = [...]` list of the workspace `Cargo.toml`; the member path's
/// last segment is the package name (true for this workspace).
fn workspace_members(root: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(root.join("Cargo.toml")) else {
        return Vec::new();
    };
    let Some(start) = text.find("members") else {
        return Vec::new();
    };
    let Some(open) = text[start..].find('[') else {
        return Vec::new();
    };
    let Some(close) = text[start + open..].find(']') else {
        return Vec::new();
    };
    let body = &text[start + open + 1..start + open + close];
    body.lines()
        .filter_map(|l| {
            let l = l.trim().trim_end_matches(',').trim();
            let l = l.trim_matches('"').trim_matches('\'');
            if l.is_empty() || l.starts_with('#') {
                return None;
            }
            Path::new(l)
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
        })
        .collect()
}

// -------------------------------------------------------------------- tests -

#[cfg(test)]
mod tests {
    use super::*;

    const STDERR: &str = r#"     Running unittests src/lib.rs (target/debug/deps/tb_core-1a2b3c)
     Running tests/parser.rs (target/debug/deps/parser-9f8e)
   Doc-tests tb_core
"#;

    const SAMPLE: &str = r#"
running 3 tests
test color::tests::parses_hex ... ok
test color::tests::rejects_junk ... ok
test slow::tests::needs_a_moment ... ignored
test result: ok. 2 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.01s

running 2 tests
test reads_selectors ... ok
test breaks_on_nothing ... FAILED

failures:

---- breaks_on_nothing stdout ----

thread 'breaks_on_nothing' panicked at tests/parser.rs:44:5:
assertion failed: false
note: run with RUST_BACKTRACE=1

failures:
    breaks_on_nothing

test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.25s

running 1 test
test src/color.rs - color::Color::parse (line 20) - compile ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
"#;

    #[test]
    fn parses_each_cargo_target_into_a_suite() {
        let suites = parse_run("tb-core", SAMPLE, &target_headers(STDERR));
        assert_eq!(
            suites.len(),
            3,
            "{:?}",
            suites.iter().map(|s| &s.name).collect::<Vec<_>>()
        );
        assert_eq!(suites[0].name, "tb_core::unit");
        assert_eq!(suites[1].name, "tb_core::tests::parser");
        assert_eq!(suites[2].name, "tb_core::doc");
        assert_eq!(suites[0].count(Status::Passed), 2);
        assert_eq!(suites[0].count(Status::Skipped), 1);
        assert_eq!(suites[1].count(Status::Failed), 1);
    }

    #[test]
    fn captures_the_output_of_a_failing_test() {
        let suites = parse_run("tb-core", SAMPLE, &target_headers(STDERR));
        let failed = suites[1]
            .cases
            .iter()
            .find(|c| c.status == Status::Failed)
            .expect("one failure");
        assert_eq!(failed.name, "breaks_on_nothing");
        assert!(
            failed.output.contains("assertion failed"),
            "{:?}",
            failed.output
        );
    }

    #[test]
    fn reads_times_and_totals() {
        let suites = parse_run("tb-core", SAMPLE, &target_headers(STDERR));
        assert!(
            (suites[1].seconds - 0.25).abs() < 0.001,
            "{}",
            suites[1].seconds
        );
        let xml = render(&suites, 1.5);
        assert!(xml.starts_with("<?xml"));
        assert!(xml.contains("tests=\"6\""), "{xml}");
        assert!(xml.contains("failures=\"1\""));
        assert!(xml.contains("<skipped/>"));
        assert!(xml.contains("<failure message=\"test breaks_on_nothing failed\">"));
    }

    #[test]
    fn escaping_keeps_the_xml_well_formed() {
        let suites = vec![Suite {
            name: "a<b&c\"d".into(),
            package: "p".into(),
            seconds: 0.0,
            cases: vec![Case {
                name: "x < y".into(),
                status: Status::Failed,
                output: "1 < 2 & 3 > 2\n".into(),
            }],
        }];
        let xml = render(&suites, 0.0);
        assert!(xml.contains("a&lt;b&amp;c&quot;d"), "{xml}");
        assert!(xml.contains("1 &lt; 2 &amp; 3 &gt; 2"));
        // and it really parses
        assert!(xml.contains("</testsuites>"));
    }

    #[test]
    fn workspace_members_come_from_the_manifest() {
        let root = workspace_root().expect("inside a workspace");
        let members = workspace_members(&root);
        assert!(members.contains(&"tb-core".to_string()), "{members:?}");
        assert!(members.contains(&"xtask".to_string()), "{members:?}");
    }
}
