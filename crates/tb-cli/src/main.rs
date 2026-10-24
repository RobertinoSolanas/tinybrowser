//! `tiny` — the command line face of the browser.
//!
//! Everything the engine can do has a flag here, and the output is meant to be
//! read by a human *and* grepped by a test. No argument-parsing crate: the flag
//! set is small, explicit, and matches `README.md` exactly.

use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use tb_browser::{Engine, EngineConfig, Page, PaintOptions};

const USAGE: &str = "\
tiny — a small browser with hand-written HTML, CSS and JS engines

USAGE:
    tiny [OPTIONS] [URL|FILE]

OUTPUT (default: a one-screen summary):
    --dom              DOM tree (tags, ids, classes, key attributes)
    --text             visible text (script/style/head skipped)
    --links            every link, resolved against the page
    --css              which stylesheets applied and how many rules
    --layout           box tree with measurements
    --js               run scripts (on by default) and print console output
    --console          print console output
    --problems         report parse/network/script problems
    --screenshot FILE  write a PNG (own rasterizer + PNG encoder)

VIEWPORT:
    --width N          viewport width in CSS px (default 1280)
    --height N         viewport height in CSS px (default 1024)
    --scale N          device pixels per CSS px (default 1)
    --full-height      screenshot the whole document, not just the viewport
    --scroll-y N       scroll down before painting

SWITCHES:
    --no-js            do not run scripts
    --no-css           do not apply author CSS (UA stylesheet only)
    --no-layout        do not build a box tree
    --no-net           no subresources, no network for the document either
    --timeout MS       per-request timeout
    --verbose, -v      extra detail (timings, host, problem list)
    --quiet, -q        only what was asked for
    --repl             interactive JS console against the loaded page
    --serve DIR        serve DIR on 127.0.0.1 (for demos and tests)
    --help, -h         this text

GUI (a window you can click and type in):
    --ui               open the browser: tabs, address bar, mouse, keyboard
    --ui-backend KIND  auto (default) | x11 | term | headless
    --theme NAME       light | dark
    --zoom N           GUI zoom, device px per CSS px (default 1)
    --new-tab URL      open a second tab (repeatable)
    --home URL         start page and the `H` key
    --search-url URL   search template for the address bar (`{}` = query)
    --bookmarks FILE   bookmark file ('' to keep none)
    --hover-css        restyle on mouse-over (pretty, slower on big pages)
    --no-chrome        no tab strip, toolbar, bookmarks or status line
    --ui-script FILE   drive the GUI from a script (see README, headless)

EXAMPLES:
    tiny https://example.com
    tiny --text https://example.com
    tiny --links --console https://taz.de
    tiny --screenshot /tmp/taz.png --full-height https://taz.de
    tiny --repl fixtures/taz.de.html
    tiny --ui https://example.com
    tiny --ui-backend term --theme dark https://taz.de
";

/// Parsed command line.
#[derive(Debug, Clone)]
pub struct Args {
    pub input: Option<String>,
    pub dom: bool,
    pub text: bool,
    pub links: bool,
    pub css: bool,
    pub layout: bool,
    pub js: bool,
    pub console: bool,
    pub problems: bool,
    pub screenshot: Option<PathBuf>,
    pub width: f32,
    pub height: f32,
    pub scale: f32,
    pub full_height: bool,
    pub scroll_y: f32,
    pub no_js: bool,
    pub no_css: bool,
    pub no_layout: bool,
    pub no_net: bool,
    pub timeout_ms: u64,
    pub verbose: bool,
    pub quiet: bool,
    pub repl: bool,
    pub serve: Option<PathBuf>,
    pub ui: bool,
    pub ui_backend: String,
    pub theme: String,
    pub zoom: f32,
    pub new_tab: Vec<String>,
    pub home: Option<String>,
    pub search_url: Option<String>,
    pub bookmarks: Option<PathBuf>,
    pub hover_css: bool,
    pub no_chrome: bool,
    pub ui_script: Option<PathBuf>,
}

impl Default for Args {
    fn default() -> Self {
        Args {
            input: None,
            dom: false,
            text: false,
            links: false,
            css: false,
            layout: false,
            js: false,
            console: false,
            problems: false,
            screenshot: None,
            width: 1280.0,
            height: 1024.0,
            scale: 1.0,
            full_height: false,
            scroll_y: 0.0,
            no_js: false,
            no_css: false,
            no_layout: false,
            no_net: false,
            timeout_ms: 20_000,
            verbose: false,
            quiet: false,
            repl: false,
            serve: None,
            ui: false,
            ui_backend: "auto".to_string(),
            theme: "light".to_string(),
            zoom: 1.0,
            new_tab: Vec::new(),
            home: None,
            search_url: None,
            bookmarks: None,
            hover_css: false,
            no_chrome: false,
            ui_script: None,
        }
    }
}

impl Args {
    /// True when the user asked for something specific; then the summary is
    /// skipped, so `tiny --text url | head` gives text and nothing else.
    fn wants(&self) -> bool {
        self.dom
            || self.text
            || self.links
            || self.css
            || self.layout
            || self.console
            || self.js
            || self.problems
            || self.screenshot.is_some()
    }

    fn config(&self) -> EngineConfig {
        let mut cfg = EngineConfig::default();
        cfg.viewport.width = self.width.max(1.0);
        cfg.viewport.height = self.height.max(1.0);
        cfg.scale = self.scale.max(0.05);
        cfg.timeout_ms = self.timeout_ms;
        cfg.caps.run_scripts = !self.no_js;
        cfg.caps.apply_css = !self.no_css;
        cfg.caps.layout = !self.no_layout;
        if self.no_net {
            cfg.caps.fetch_subresources = false;
        }
        cfg
    }
}

pub fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.iter().any(|a| a == "--help" || a == "-h") {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    let args = match parse(&argv) {
        Ok(a) => a,
        Err(msg) => {
            eprintln!("tiny: {msg}");
            eprint!("\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    run(&args)
}

fn run(args: &Args) -> ExitCode {
    if let Some(dir) = &args.serve {
        return serve(dir);
    }
    if args.ui {
        return gui(args);
    }
    let Some(input) = &args.input else {
        eprint!("{USAGE}");
        return ExitCode::from(2);
    };

    let config = args.config();
    let mut browser = if args.no_net {
        Engine::offline(config)
    } else {
        Engine::new(config)
    };

    let mut page = match browser.load(input) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("tiny: cannot load {input}: {e}");
            return ExitCode::from(1);
        }
    };

    // a bare `tiny URL` prints the summary; with `--text` & friends the user
    // asked for exactly that output and nothing else
    if !args.quiet && (!args.wants() || args.verbose) {
        summary(&page);
    }
    if args.dom {
        print!("{}", page.dump_dom());
    }
    if args.text {
        print!("{}", page.text());
    }
    if args.links {
        for l in page.links() {
            let rel = if l.rel.is_empty() {
                String::new()
            } else {
                format!(" [{}]", l.rel)
            };
            let id = if l.id.is_empty() {
                String::new()
            } else {
                format!(" #{}", l.id)
            };
            println!("{}{id}{rel} {url}", truncate(&l.text, 60), url = l.url);
        }
    }
    if args.css {
        print!("{}", page.dump_css());
    }
    if args.layout {
        print!("{}", page.dump_layout());
    }
    if args.console || args.js {
        for (level, line) in page.console_lines() {
            println!("[{level}] {line}");
        }
    }
    if args.problems {
        for p in page.problems().all() {
            // `--verbose` adds the context (which url, which selector)
            if args.verbose {
                println!("{p}");
            } else {
                println!("[{:?} {}] {}", p.severity, p.phase, p.message);
            }
        }
    }
    if let Some(path) = &args.screenshot {
        if args.full_height {
            let h = page.content_height().max(args.height);
            page.set_viewport(args.width, h);
        }
        let opts = PaintOptions {
            scale: args.scale,
            scroll_y: args.scroll_y,
            ..Default::default()
        };
        match page.screenshot(path, &opts) {
            Ok(bytes) => {
                if !args.quiet {
                    let fb = page.render(&opts);
                    println!(
                        "screenshot: {} ({}x{}, {bytes} bytes{})",
                        path.display(),
                        fb.width,
                        fb.height,
                        if fb.is_blank() { ", BLANK" } else { "" }
                    );
                }
            }
            Err(e) => {
                eprintln!("tiny: cannot write {}: {e}", path.display());
                return ExitCode::from(1);
            }
        }
    }
    if args.verbose {
        println!("pipeline:   {}", page.timings);
        println!("state:      {}", browser.state);
    }

    if args.repl {
        repl(args, &mut browser, &mut page);
    }
    ExitCode::SUCCESS
}

/// The one-screen summary `tiny <url>` prints.
fn summary(page: &Page) {
    let s = page.stats();
    println!(
        "→ {}  ({} {}, {} bytes, {} ms)",
        page.url,
        page.status,
        page.content_type,
        page.bytes,
        page.timings.fetch.round() as i64
    );
    println!("title:    {}", page.title());
    println!(
        "elements: {}   nodes: {}   links: {}   images: {} ({} decoded)   scripts: {}",
        s.elements, s.nodes, s.links, s.images, s.images_decoded, s.scripts
    );
    println!(
        "css:      {} sheets, {} rules, {} elements matched   boxes: {}",
        s.stylesheets, s.css_rules, s.matched_elements, s.boxes
    );
    if page.has_js() {
        println!(
            "js:       {} run, {} skipped, {} failed   console: {}   problems: {}",
            s.scripts_run, s.script_skipped, s.script_failures, s.console_lines, s.problems
        );
    } else {
        println!("js:       off (--no-js)");
    }
}

/// A tiny JS console against the loaded page.
fn repl(args: &Args, browser: &mut Engine, page: &mut Page) {
    if !page.has_js() {
        println!("(javascript is off — start without --no-js)");
        return;
    }
    println!("tiny repl — one expression per line, :help for commands");
    let stdin = io::stdin();
    print!("> ");
    let _ = io::stdout().flush();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let cmd = line.trim();
        if cmd.is_empty() {
            print!("> ");
            let _ = io::stdout().flush();
            continue;
        }
        match cmd {
            ":quit" | ":q" | ":exit" => break,
            ":help" => println!(
                ":load URL   navigate\n:dom        DOM tree\n:text       visible text\n:title      document.title\n:eval EXPR  same as typing it\n:quit       leave"
            ),
            ":dom" => print!("{}", page.dump_dom()),
            ":text" => print!("{}", page.text()),
            ":title" => println!("{}", page.title()),
            _ if cmd.starts_with(":load ") => {
                let url = cmd[":load ".len()..].trim();
                match browser.load(url) {
                    Ok(p) => {
                        *page = p;
                        println!("{}", page.url);
                    }
                    Err(e) => println!("cannot load {url}: {e}"),
                }
            }
            src => {
                let src = src.strip_prefix(":eval ").unwrap_or(src);
                match page.eval(src) {
                    Ok(v) => {
                        let out = v.to_string();
                        if !out.is_empty() {
                            println!("{out}");
                        }
                    }
                    Err(e) => println!("! {e}"),
                }
                // scripts may have navigated or changed the tree
                page.pump(0.0);
                if args.verbose {
                    println!("(dom: {} elements)", page.stats().elements);
                }
            }
        }
        print!("> ");
        let _ = io::stdout().flush();
    }
    println!();
}

/// `--ui`: open the browser — a window with tabs, an address bar, a pointer and
/// a keyboard.
///
/// `auto` prefers X11 when `$DISPLAY` is set and falls back to the terminal;
/// with neither, the run is refused rather than showing nothing.
fn gui(args: &Args) -> ExitCode {
    use std::io::IsTerminal;
    use tb_ui::{render, App, Event, Theme, UiConfig};

    let mut cfg = UiConfig::default();
    cfg.scale = args.zoom.clamp(0.25, 4.0);
    cfg.show_chrome = !args.no_chrome;
    cfg.run_scripts = !args.no_js;
    cfg.apply_css = !args.no_css;
    cfg.fetch = !args.no_net;
    cfg.timeout_ms = args.timeout_ms;
    cfg.hover_css = args.hover_css;
    if let Some(home) = &args.home {
        cfg.home = home.clone();
    }
    if let Some(search) = &args.search_url {
        cfg.search_url = search.clone();
    }
    if let Some(file) = &args.bookmarks {
        cfg.bookmark_file = Some(file.clone());
    }
    let theme = match args.theme.to_ascii_lowercase().as_str() {
        "dark" => Theme::dark(),
        "light" => Theme::light(),
        other => {
            eprintln!("tiny: unknown theme {other:?} (light or dark)");
            return ExitCode::from(2);
        }
    };

    let mut urls: Vec<String> = Vec::new();
    if let Some(input) = &args.input {
        urls.push(input.clone());
    }
    urls.extend(args.new_tab.iter().cloned());

    // `--width`/`--height` are CSS pixels; a window is measured in device pixels
    let width = ((args.width.max(320.0) * args.zoom) as u32).max(320);
    let height = ((args.height.max(200.0) * args.zoom) as u32).max(200);
    let mut app = App::new(cfg, theme, width, height, &urls);

    if let Some(path) = &args.ui_script {
        return script(&mut app, path);
    }

    let backend = args.ui_backend.to_ascii_lowercase();
    let display = std::env::var("DISPLAY").unwrap_or_default();
    if matches!(backend.as_str(), "auto" | "x11") && !display.is_empty() {
        match tb_x11::browse(&mut app) {
            Ok(()) => return ExitCode::SUCCESS,
            Err(e) => {
                if backend == "x11" {
                    eprintln!("tiny: {e}");
                    return ExitCode::from(1);
                }
                eprintln!("tiny: no X11 window ({e}); trying the terminal");
            }
        }
    }
    match backend.as_str() {
        "auto" | "term" => {
            if !std::io::stdin().is_terminal() {
                eprintln!(
                    "tiny: --ui needs a terminal or an X display; \
                     try --ui-backend headless or a --ui-script"
                );
                return ExitCode::from(1);
            }
            match tb_term::browse(&mut app) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("tiny: {e}");
                    ExitCode::from(1)
                }
            }
        }
        "headless" => {
            // No display at all: paint once and print what the window would
            // have shown. `--ui-script` is the version of this that a test can
            // drive.
            for _ in 0..8 {
                app.on_event(Event::Tick);
            }
            print!("{}", render::paint_screen(&app).dump());
            ExitCode::SUCCESS
        }
        "x11" => {
            eprintln!("tiny: --ui-backend x11 needs DISPLAY set");
            ExitCode::from(1)
        }
        other => {
            eprintln!("tiny: unknown --ui-backend {other:?} (auto, x11, term, headless)");
            ExitCode::from(2)
        }
    }
}

/// `--ui-script`: drive the same GUI with events from a file and print what the
/// script asks for. It is how the GUI is tested, and how a demo is repeated.
fn script(app: &mut tb_ui::App, path: &Path) -> ExitCode {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("tiny: cannot read {}: {e}", path.display());
            return ExitCode::from(2);
        }
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // a browser command (open, back, newtab, …) is not an input event
        if tb_ui::backend::script_command(app, line) {
            continue;
        }
        // an output verb prints the app's state right here, in script order
        if let Some(out) = tb_ui::backend::script_output(app, line) {
            if !out.is_empty() {
                println!("{out}");
            }
            continue;
        }
        for ev in tb_ui::backend::parse_script(line) {
            app.on_event(ev);
        }
    }
    ExitCode::SUCCESS
}

/// Serve a directory on 127.0.0.1 — enough for a demo (`--serve .` and then
/// `tiny --links http://127.0.0.1:PORT/`).
fn serve(dir: &Path) -> ExitCode {
    if !dir.is_dir() {
        eprintln!("tiny: {} is not a directory", dir.display());
        return ExitCode::from(2);
    }
    let listener = match std::net::TcpListener::bind("127.0.0.1:0") {
        Ok(l) => l,
        Err(e) => {
            eprintln!("tiny: cannot bind: {e}");
            return ExitCode::from(1);
        }
    };
    let addr = listener.local_addr().expect("bound address");
    println!("serving {} on http://{addr}/", dir.display());
    let _ = io::stdout().flush();
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        let mut buf = [0u8; 4096];
        let n = match std::io::Read::read(&mut stream, &mut buf) {
            Ok(n) if n > 0 => n,
            _ => continue,
        };
        let head = String::from_utf8_lossy(&buf[..n]);
        let path = head
            .split_whitespace()
            .nth(1)
            .unwrap_or("/")
            .split('?')
            .next()
            .unwrap_or("/");
        let mut file = dir.to_path_buf();
        for part in path.trim_matches('/').split('/') {
            if part == ".." {
                continue;
            }
            file.push(part);
        }
        if file.is_dir() {
            file.push("index.html");
        }
        let (status, ctype, body) = match std::fs::read(&file) {
            Ok(bytes) => ("200 OK", mime_for(&file), bytes),
            Err(_) => (
                "404 Not Found",
                "text/plain; charset=utf-8",
                b"not found".to_vec(),
            ),
        };
        let resp = format!(
            "HTTP/1.1 {status}\r\ncontent-type: {ctype}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            body.len()
        );
        let _ = std::io::Write::write_all(&mut stream, resp.as_bytes());
        let _ = std::io::Write::write_all(&mut stream, &body);
    }
    ExitCode::SUCCESS
}

fn mime_for(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" => "application/json",
        "png" => "image/png",
        "svg" => "image/svg+xml",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn truncate(s: &str, max: usize) -> String {
    tb_core::text::truncate(s, max)
}

/// Hand-written argument parsing: clear errors beat clever errors.
pub fn parse(argv: &[String]) -> Result<Args, String> {
    let mut a = Args::default();
    let mut i = 0;
    while i < argv.len() {
        let arg = argv[i].as_str();
        // `--flag value` and `--flag=value` both work
        let (name, inline) = match arg.split_once('=') {
            Some((n, v)) => (n, Some(v.to_string())),
            None => (arg, None),
        };
        let mut next = |what: &str| -> Result<String, String> {
            if let Some(v) = &inline {
                return Ok(v.clone());
            }
            i += 1;
            argv.get(i)
                .cloned()
                .ok_or_else(|| format!("{name} needs {what}"))
        };
        let mut number = |what: &str| -> Result<f64, String> {
            let v = next(what)?;
            v.parse::<f64>()
                .map_err(|_| format!("{name}: expected {what}, got {v:?}"))
        };
        match name {
            "--help" | "-h" => {}
            "--dom" => a.dom = true,
            "--text" => a.text = true,
            "--links" => a.links = true,
            "--css" => a.css = true,
            "--layout" => a.layout = true,
            "--js" => a.js = true,
            "--console" => a.console = true,
            "--problems" => a.problems = true,
            "--screenshot" => a.screenshot = Some(PathBuf::from(next("a file name")?)),
            "--width" => a.width = number("a width")? as f32,
            "--height" => a.height = number("a height")? as f32,
            "--scale" => a.scale = number("a scale factor")? as f32,
            "--scroll-y" => a.scroll_y = number("a scroll offset")? as f32,
            "--full-height" => a.full_height = true,
            "--no-js" => a.no_js = true,
            "--no-css" => a.no_css = true,
            "--no-layout" => a.no_layout = true,
            "--no-net" => a.no_net = true,
            "--timeout" => a.timeout_ms = number("a timeout in ms")? as u64,
            "--verbose" | "-v" => a.verbose = true,
            "--quiet" | "-q" => a.quiet = true,
            "--repl" => a.repl = true,
            "--serve" => a.serve = Some(PathBuf::from(next("a directory")?)),
            "--ui" => a.ui = true,
            "--ui-backend" => a.ui_backend = next("x11, term or headless")?,
            "--theme" => a.theme = next("light or dark")?,
            "--zoom" => a.zoom = number("a zoom factor")? as f32,
            "--new-tab" => a.new_tab.push(next("a URL")?),
            "--home" => a.home = Some(next("a URL")?),
            "--search-url" => a.search_url = Some(next("a template")?),
            "--bookmarks" => a.bookmarks = Some(PathBuf::from(next("a file name")?)),
            "--hover-css" => a.hover_css = true,
            "--no-chrome" => a.no_chrome = true,
            "--ui-script" => {
                a.ui_script = Some(PathBuf::from(next("a script file")?));
                a.ui = true;
            }
            other if other.starts_with('-') => return Err(format!("unknown option {other}")),
            _ => {
                if a.input.is_some() {
                    return Err("only one URL or file can be loaded".into());
                }
                a.input = Some(arg.to_string());
            }
        }
        i += 1;
    }
    Ok(a)
}
