//! The `ResourceLoader`: one page's door to the outside world.
//!
//! Belongs to the application layer on purpose. The retrieval context
//! (`tb_net`) knows how to speak HTTP; *which* things a page needs, in what
//! budget, resolved against *which* base URL — and what a bare filesystem path
//! means when the "URL" is `fixtures/taz.de.html` — is browser policy.
//!
//! A loader travels as `Rc<RefCell<_>>` so the JS host's fetch hook and the
//! script loader can use it while the page itself is being assembled.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use tb_core::{EngineConfig, Phase, Problems, TbError};
use tb_net::{HttpClient, Url};

/// What a sub-resource turned into.
#[derive(Debug, Clone)]
pub struct Loaded {
    pub url: String,
    pub status: u16,
    pub text: String,
    pub from_cache: bool,
}

/// Loads the resources of one page.
///
/// Policy: at most `config.max_subresources` fetches, each URL at most once,
/// answers are remembered so a second `<link>` to the same sheet is free.
/// Anything that fails becomes a `Problem` and `None` — a page with a dead CDN
/// still has to render.
pub struct ResourceLoader {
    client: Option<Arc<HttpClient>>,
    base: Url,
    budget: usize,
    used: usize,
    seen: HashSet<String>,
    memory: HashMap<String, Loaded>,
    /// Test/demo hook: canned bodies by absolute URL, consulted first.
    stubs: HashMap<String, (u16, String)>,
    pub problems: Problems,
}

/// A loader the JS host can hold on to.
pub type SharedLoader = Rc<RefCell<ResourceLoader>>;

impl Default for ResourceLoader {
    fn default() -> Self {
        ResourceLoader::new(
            Url::parse("about:blank").expect("about:blank parses"),
            None,
            &EngineConfig::default(),
        )
    }
}

impl ResourceLoader {
    pub fn new(base: Url, client: Option<Arc<HttpClient>>, config: &EngineConfig) -> Self {
        ResourceLoader {
            client,
            base,
            budget: config.max_subresources,
            used: 0,
            seen: HashSet::new(),
            memory: HashMap::new(),
            stubs: HashMap::new(),
            problems: Problems::new(),
        }
    }

    pub fn base(&self) -> &Url {
        &self.base
    }

    pub fn set_base(&mut self, url: Url) {
        self.base = url;
    }

    /// Offline/test mode: answer these URLs without touching the network.
    pub fn stub(&mut self, absolute_url: &str, status: u16, text: impl Into<String>) {
        self.stubs
            .insert(absolute_url.to_string(), (status, text.into()));
    }

    pub fn fetched(&self) -> usize {
        self.used
    }

    /// The HTTP client this page reaches the world through (`None` offline).
    /// Cookies a script set are pushed back into its jar with it.
    pub fn client(&self) -> Option<Arc<HttpClient>> {
        self.client.clone()
    }

    /// Join `href` against the page URL. `None` for empty, broken, or schemes
    /// we deliberately never fetch (`javascript:`, `mailto:`, plain `#frag`).
    pub fn resolve(&self, href: &str) -> Option<Url> {
        let href = href.trim();
        if href.is_empty() || href.starts_with('#') {
            return None;
        }
        let lowered = href.to_ascii_lowercase();
        if lowered.starts_with("javascript:")
            || lowered.starts_with("mailto:")
            || lowered.starts_with("tel:")
        {
            return None;
        }
        match Url::parse(href) {
            Ok(u) => Some(u),
            Err(_) => Url::join(&self.base, href).ok(),
        }
    }

    /// Fetch `href` (resolved against the page) as text.
    pub fn text(&mut self, href: &str) -> Option<String> {
        self.load(href).map(|l| l.text)
    }

    /// Status + body — which is what `window.fetch` reports.
    pub fn load(&mut self, href: &str) -> Option<Loaded> {
        let url = self.resolve(href)?;
        let key = url.to_string();

        if let Some((status, text)) = self.stubs.get(&key).cloned() {
            return Some(Loaded {
                url: key,
                status,
                text,
                from_cache: false,
            });
        }
        if let Some(hit) = self.memory.get(&key) {
            return Some(hit.clone());
        }
        if !self.seen.insert(key.clone()) {
            // asked for something that already failed: stay quiet about it
            return None;
        }
        if self.used >= self.budget {
            self.problems
                .warn(Phase::Browser, format!("subresource budget used up: {key}"));
            return None;
        }
        self.used += 1;

        if url.scheme == "file" {
            return self.read_file(&url).map(|text| {
                let out = Loaded {
                    url: key.clone(),
                    status: 200,
                    text,
                    from_cache: false,
                };
                self.memory.insert(key, out.clone());
                out
            });
        }

        let Some(client) = self.client.clone() else {
            self.problems
                .info(Phase::Browser, format!("offline page, not fetched: {key}"));
            return None;
        };
        match client.fetch(&url) {
            Ok(r) => {
                let status = r.response.status;
                let Some(text) = r.text.clone() else {
                    // binary subresource (image, font): we note it and move on
                    self.problems
                        .info(Phase::Browser, format!("binary resource skipped: {key}"));
                    return None;
                };
                if status >= 400 {
                    self.problems
                        .warn(Phase::Network, format!("{status} for {key}"));
                }
                let out = Loaded {
                    url: key.clone(),
                    status,
                    text,
                    from_cache: r.from_cache,
                };
                self.memory.insert(key, out.clone());
                Some(out)
            }
            Err(e) => {
                self.problems.warn(Phase::Network, format!("{key}: {e}"));
                None
            }
        }
    }

    fn read_file(&mut self, url: &Url) -> Option<String> {
        let path = file_path(url);
        match read_to_string_max(&path, 64 * 1024 * 1024) {
            Ok(text) => Some(text),
            Err(e) => {
                self.problems.warn(Phase::Network, format!("{path:?}: {e}"));
                None
            }
        }
    }
}

/// `FnMut(href) -> Option<String>` — what `tb_style::resolve_sources` and the
/// `<script src>` loader want.
pub fn hook(me: &SharedLoader) -> impl FnMut(&str) -> Option<String> {
    let me = me.clone();
    move |href: &str| me.borrow_mut().text(href)
}

/// `(status, body)` for `window.fetch`. The loader borrow ends before the body
/// is handed back: the caller is about to run JavaScript.
pub fn fetch_hook(me: &SharedLoader) -> Box<dyn Fn(&str) -> Option<(u16, String)>> {
    let me = me.clone();
    Box::new(move |href: &str| {
        let mut l = me.borrow_mut();
        l.load(href).map(|r| (r.status, r.text))
    })
}

/// `file:///abs/path` → filesystem path.
pub fn file_path(url: &Url) -> PathBuf {
    let p = if url.path.is_empty() { "/" } else { &url.path };
    PathBuf::from(tb_net::percent_decode(p))
}

/// A page can be loaded from a plain path (`tiny fixtures/x.html`); turn it
/// into a `file:` URL so everything downstream only has to deal with URLs.
pub fn url_for_input(input: &str) -> tb_core::Result<Url> {
    let trimmed = input.trim();
    if let Ok(u) = Url::parse(trimmed) {
        if matches!(
            u.scheme.as_str(),
            "http" | "https" | "data" | "about" | "file"
        ) {
            return Ok(u);
        }
    }
    let path = PathBuf::from(trimmed);
    let abs = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .map_err(|e| TbError::Io(format!("cwd: {e}")))?
            .join(path)
    };
    let abs = abs.to_string_lossy().replace('\\', "/");
    Url::parse(&format!("file://{abs}"))
}

fn read_to_string_max(path: &std::path::Path, max: u64) -> tb_core::Result<String> {
    let mut f = std::fs::File::open(path).map_err(|e| TbError::Io(format!("{path:?}: {e}")))?;
    let mut buf = Vec::new();
    f.by_ref()
        .take(max)
        .read_to_end(&mut buf)
        .map_err(|e| TbError::Io(format!("{path:?}: {e}")))?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}
