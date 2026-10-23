//! The `Engine` facade and the navigation aggregate.
//!
//! This is where the bounded contexts meet — fetch → parse → subresources →
//! script → style → layout → paint — each stage timed, each stage's problems
//! collected into the page. Nothing below this module knows anything to its
//! right; this is the only place allowed to know all of them.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use tb_core::{EngineConfig, Problems, Result, TbError};
use tb_css::{parse_stylesheet, StyleSheet};
use tb_dom::Dom;
use tb_dom_bindings::{DomBindings, PageContext, ScriptOptions, ScriptReport};
use tb_html::parse_document;
use tb_js::Runtime;
use tb_net::{HttpClient, Url};
use tb_style::{collect_sources, compute, resolve_sources, CssSource, MatchState, Viewport};

use crate::page::{Page, Timings};
use crate::paint::PaintOptions;
use crate::resources::{url_for_input, ResourceLoader, SharedLoader};

/// Where a navigation currently stands — the aggregate's state, so a UI (or a
/// test) can assert on progress without peeking at internals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavState {
    Idle,
    Fetching,
    Parsed,
    Styled,
    LaidOut,
    Scripted,
    Rendered,
    Failed,
}

impl std::fmt::Display for NavState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            NavState::Idle => "idle",
            NavState::Fetching => "fetching",
            NavState::Parsed => "parsed",
            NavState::Styled => "styled",
            NavState::LaidOut => "laid out",
            NavState::Scripted => "scripted",
            NavState::Rendered => "rendered",
            NavState::Failed => "failed",
        };
        f.write_str(s)
    }
}

/// Session history: the documents that were loaded, plus the pointer.
#[derive(Debug, Default)]
pub struct History {
    entries: Vec<Url>,
    index: Option<usize>,
}

impl History {
    pub fn push(&mut self, url: Url) {
        // loading after going back drops the forward entries, like a browser
        if let Some(i) = self.index {
            self.entries.truncate(i + 1);
        }
        self.entries.push(url);
        self.index = Some(self.entries.len() - 1);
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn entries(&self) -> &[Url] {
        &self.entries
    }

    pub fn current(&self) -> Option<&Url> {
        self.index.map(|i| &self.entries[i])
    }

    pub fn back(&mut self) -> Option<Url> {
        let i = self.index?;
        if i == 0 {
            return None;
        }
        self.index = Some(i - 1);
        Some(self.entries[i - 1].clone())
    }

    pub fn forward(&mut self) -> Option<Url> {
        let i = self.index?;
        if i + 1 >= self.entries.len() {
            return None;
        }
        self.index = Some(i + 1);
        Some(self.entries[i + 1].clone())
    }
}

/// The browser facade: config, network, session history and the pipeline.
pub struct Engine {
    pub config: EngineConfig,
    client: Option<Arc<HttpClient>>,
    pub history: History,
    pub state: NavState,
}

impl Engine {
    /// A browser with a network.
    pub fn new(config: EngineConfig) -> Engine {
        Engine {
            config: config.clone(),
            client: Some(Arc::new(HttpClient::new(config))),
            history: History::default(),
            state: NavState::Idle,
        }
    }

    /// A browser without one: `file:`, `data:` and [`Engine::load_html`].
    pub fn offline(config: EngineConfig) -> Engine {
        Engine {
            config,
            client: None,
            history: History::default(),
            state: NavState::Idle,
        }
    }

    pub fn client(&self) -> Option<&Arc<HttpClient>> {
        self.client.as_ref()
    }

    /// `tiny https://example.com` — also accepts `file:`, `data:` and a plain
    /// filesystem path (`fixtures/taz.de.html`).
    pub fn load(&mut self, input: &str) -> Result<Page> {
        let url = url_for_input(input)?;
        self.load_url(url)
    }

    /// Fetch and process `url`; the URL lands in the session history.
    pub fn load_url(&mut self, url: Url) -> Result<Page> {
        let page = self.retrieve(&url)?;
        self.history.push(url);
        Ok(page)
    }

    /// Process a document that is already in hand (tests, saved copies).
    pub fn load_html(&mut self, base: &str, html: &str) -> Result<Page> {
        let url = url_for_input(base)?;
        let page = self.process(
            url.clone(),
            200,
            "text/html".into(),
            html.to_string(),
            Timings::default(),
        )?;
        self.history.push(url);
        Ok(page)
    }

    /// The same document again, with the HTTP cache flushed.
    pub fn reload(&mut self) -> Result<Option<Page>> {
        let Some(url) = self.history.current().cloned() else {
            return Ok(None);
        };
        if let Some(c) = &self.client {
            c.cache().clear();
        }
        Ok(Some(self.retrieve(&url)?))
    }

    /// One entry back in the session history.
    pub fn back(&mut self) -> Result<Option<Page>> {
        match self.history.back() {
            Some(url) => Ok(Some(self.retrieve(&url)?)),
            None => Ok(None),
        }
    }

    pub fn forward(&mut self) -> Result<Option<Page>> {
        match self.history.forward() {
            Some(url) => Ok(Some(self.retrieve(&url)?)),
            None => Ok(None),
        }
    }

    /// Follow a navigation JS asked for. `href` is resolved against the page.
    pub fn navigate_to(&mut self, page: &Page, href: &str) -> Result<Page> {
        let url = page
            .loader
            .borrow()
            .resolve(href)
            .ok_or_else(|| TbError::Policy(format!("cannot navigate to {href}")))?;
        self.load_url(url)
    }

    /// `fetch()` from Rust: raw bytes + status, no pipeline.
    pub fn fetch(&self, url: &Url) -> Result<tb_net::FetchResult> {
        let Some(client) = &self.client else {
            return Err(TbError::Unsupported("no network in this instance"));
        };
        client.fetch(url)
    }

    fn retrieve(&mut self, url: &Url) -> Result<Page> {
        self.state = NavState::Fetching;
        let started = std::time::Instant::now();
        let fetched = match self.fetch_document(url) {
            Ok(v) => v,
            Err(e) => {
                self.state = NavState::Failed;
                return Err(e);
            }
        };
        let timings = Timings {
            fetch: started.elapsed().as_secs_f64() * 1000.0,
            ..Default::default()
        };
        self.process(fetched.0, fetched.1, fetched.2, fetched.3, timings)
    }

    /// `(final url, status, content type, body)`. A redirect changes the URL —
    /// the page reports where it actually ended up, and so does the history.
    fn fetch_document(&self, url: &Url) -> Result<(Url, u16, String, String)> {
        if url.scheme == "data" {
            let (mediatype, body) = url.path.split_once(',').ok_or_else(|| TbError::Parse {
                phase: tb_core::Phase::Network,
                message: "data url without `,`".into(),
                line: 0,
                column: 0,
            })?;
            let bytes = if mediatype.ends_with(";base64") {
                tb_net::base64_decode(body).unwrap_or_default()
            } else {
                tb_net::percent_decode(body).into_bytes()
            };
            let ct = mediatype.trim_end_matches(";base64").to_string();
            return Ok((
                url.clone(),
                200,
                ct,
                String::from_utf8_lossy(&bytes).into_owned(),
            ));
        }
        if url.scheme == "about" {
            return Ok((url.clone(), 200, "text/html".into(), String::new()));
        }
        if url.scheme == "file" {
            let path = crate::resources::file_path(url);
            let html = std::fs::read_to_string(&path)
                .map_err(|e| TbError::Io(format!("{}: {e}", path.display())))?;
            return Ok((url.clone(), 200, "text/html".into(), html));
        }
        let Some(client) = &self.client else {
            return Err(TbError::Policy(format!(
                "this browser instance has no network: {url}"
            )));
        };
        let r = client.fetch(url)?;
        // the client records where a chain of redirects ended
        let final_url = if r.response.url.is_empty() {
            url.clone()
        } else {
            Url::parse(&r.response.url).unwrap_or_else(|_| url.clone())
        };
        let status = r.response.status;
        let ct = r
            .response
            .headers
            .get("content-type")
            .unwrap_or("text/html")
            .to_string();
        let html = match r.text.clone() {
            Some(t) => t,
            None => String::from_utf8_lossy(&r.response.body).into_owned(),
        };
        Ok((final_url, status, ct, html))
    }

    /// Everything that happens to a document once its bytes have arrived.
    ///
    /// Style and layout run *after* the scripts on purpose: the host builds its
    /// own style/layout on demand while JS runs, and the page must report the
    /// document JS left behind, not the one the server sent.
    fn process(
        &mut self,
        url: Url,
        status: u16,
        content_type: String,
        html: String,
        mut timings: Timings,
    ) -> Result<Page> {
        let config = self.config.clone();
        let mut problems = Problems::new();
        let mut nav_requests = Vec::new();

        // ---- parse --------------------------------------------------------
        let started = std::time::Instant::now();
        let mut dom = parse_document(&html);
        timings.parse = started.elapsed().as_secs_f64() * 1000.0;
        self.state = NavState::Parsed;

        let loader: SharedLoader = Rc::new(RefCell::new(ResourceLoader::new(
            url.clone(),
            self.client.clone(),
            &config,
        )));

        // ---- external css --------------------------------------------------
        // `<style>` elements are deliberately not fetched here: `author_sheets`
        // reads them out of the DOM later, so a sheet a script injected is part
        // of the cascade too.
        let mut external: Vec<StyleSheet> = Vec::new();
        if config.caps.apply_css && config.caps.fetch_subresources {
            let started = std::time::Instant::now();
            let links: Vec<CssSource> = collect_sources(&dom)
                .into_iter()
                .filter(|s| !matches!(s, CssSource::Inline { .. }))
                .collect();
            external = resolve_sources(
                &links,
                style_viewport(&config),
                crate::resources::hook(&loader),
                &mut problems,
            );
            timings.style += started.elapsed().as_secs_f64() * 1000.0;
        }

        // ---- scripts -------------------------------------------------------
        let mut js = None;
        let mut report = ScriptReport {
            executed: 0,
            skipped: 0,
            failed: 0,
        };
        if config.caps.run_scripts {
            let started = std::time::Instant::now();
            let mut rt = Runtime::new();
            rt.set_echo_console(false);
            let mut bindings = DomBindings::install(
                std::mem::replace(&mut dom, Dom::new()),
                PageContext {
                    config: config.clone(),
                    url: Some(url.clone()),
                    ..Default::default()
                },
                &mut rt,
            );
            // ---- cookies ----------------------------------------------------
            // The jar lives in the retrieval context; the page gets a copy for
            // `document.cookie` and hands back what it changed, so a cookie a
            // script sets is sent on the next request, and a `Set-Cookie` from
            // the server is readable in the page.
            if let Some(client) = &self.client {
                let sent = client.cookie_jar().document_cookie(&url);
                bindings.host.borrow_mut().cookies = cookie_pairs(&sent);
            }
            // the host parses `<style>` itself — hand it only the outside sheets
            bindings.set_sheets(external.clone());
            bindings.set_fetch_hook(crate::resources::fetch_hook(&loader));
            report = bindings.run_scripts_with(
                &mut rt,
                crate::resources::hook(&loader),
                &ScriptOptions::default(),
            );
            if let Some(client) = &self.client {
                let pairs = bindings.host.borrow().cookies.clone();
                let mut jar = client.cookie_jar();
                for (k, v) in pairs {
                    if let Some(c) = tb_net::Cookie::parse(&format!("{k}={v}; path=/"), &url) {
                        jar.insert(c);
                    }
                }
            }
            let host = bindings.host.clone();
            {
                let mut h = host.borrow_mut();
                problems.extend(std::mem::take(&mut h.problems));
                // the host's document is the live one now
                dom = h.dom.clone();
                let mut navs = std::mem::take(&mut h.nav);
                if !navs.is_empty() {
                    nav_requests.append(&mut navs);
                }
            }
            timings.script += started.elapsed().as_secs_f64() * 1000.0;
            self.state = NavState::Scripted;
            js = Some((rt, bindings));
        }

        // ---- style + layout of the final document ---------------------------
        let sheets = assemble_sheets(&dom, &external);
        let started = std::time::Instant::now();
        let styled = compute(
            &dom,
            if config.caps.apply_css { &sheets } else { &[] },
            style_viewport(&config),
            &MatchState::default(),
        );
        problems.extend(styled.problems);
        timings.style += started.elapsed().as_secs_f64() * 1000.0;
        self.state = NavState::Styled;

        let layout = if config.caps.layout {
            let started = std::time::Instant::now();
            let l = tb_layout::layout(&dom, &styled.styles, style_viewport(&config));
            timings.layout += started.elapsed().as_secs_f64() * 1000.0;
            self.state = NavState::LaidOut;
            l
        } else {
            None
        };

        // whatever the loader complained about (budget, 404s, dead hosts) is a
        // problem of this page
        problems.extend(std::mem::take(&mut loader.borrow_mut().problems));
        let page = Page::new(
            url,
            status,
            content_type,
            html.len(),
            dom,
            sheets,
            styled.styles,
            layout,
            problems,
            timings,
            styled.matched_elements,
            report,
            loader,
            config,
            js,
        );
        let mut page = page;
        page.nav_requests = nav_requests;
        Ok(page)
    }
}

// ---------------------------------------------------------------- helpers --

/// `a=1; b=2` → `[("a","1"), ("b","2")]`.
fn cookie_pairs(header: &str) -> Vec<(String, String)> {
    header
        .split(';')
        .filter_map(|p| {
            let (k, v) = p.trim().split_once('=')?;
            if k.is_empty() {
                return None;
            }
            Some((k.to_string(), v.to_string()))
        })
        .collect()
}

fn style_viewport(config: &EngineConfig) -> Viewport {
    Viewport {
        width: config.viewport.width,
        height: config.viewport.height,
        color_scheme: config.color_scheme,
        reduced_motion: false,
    }
}

/// Author CSS for the document as it is *now*: the `<link>` sheets we resolved
/// (with the `@import`s they dragged in) placed in the slots the DOM's
/// `<link>`/`<style>` elements occupy.
///
/// `tb_style::collect_sources` owns the document order, so a sheet a script
/// appended participates correctly; imported sheets follow the sheet that
/// imported them, which is the order `resolve_sources` handed them back in.
pub fn assemble_sheets(dom: &Dom, resolved: &[StyleSheet]) -> Vec<StyleSheet> {
    let link_hrefs: Vec<String> = collect_sources(dom)
        .into_iter()
        .filter_map(|s| match s {
            CssSource::External { href, .. } => Some(href),
            _ => None,
        })
        .collect();
    let is_link = |href: &str| !href.is_empty() && link_hrefs.iter().any(|h| h == href);

    let mut out: Vec<StyleSheet> = Vec::new();
    let mut taken = vec![false; resolved.len()];
    for src in collect_sources(dom) {
        match src {
            CssSource::Inline { text } => {
                let mut sheet = parse_stylesheet(&text);
                sheet.href = String::new();
                out.push(sheet);
            }
            CssSource::External { href, .. } => {
                let Some(i) = resolved.iter().position(|s| s.href == href) else {
                    continue;
                };
                taken[i] = true;
                out.push(resolved[i].clone());
                // what this sheet imported follows it in `resolved`
                let mut j = i + 1;
                while j < resolved.len() && !taken[j] && !is_link(&resolved[j].href) {
                    taken[j] = true;
                    out.push(resolved[j].clone());
                    j += 1;
                }
            }
            // an @import is placed together with the sheet that imported it
            CssSource::Import { .. } => {}
        }
    }
    // anything left over (a `<link>` that was removed, a sheet of an import of
    // an import) keeps its order at the end of the cascade
    for (i, sheet) in resolved.iter().enumerate() {
        if !taken[i] {
            out.push(sheet.clone());
        }
    }
    out
}

/// Paint options that match the engine's viewport and scale.
pub fn paint_options(config: &EngineConfig) -> PaintOptions {
    PaintOptions {
        scale: config.scale,
        ..Default::default()
    }
}
