//! Bookmarks.
//!
//! A JSON array of `{title, url}` in `$XDG_CONFIG_HOME/tiny/bookmarks.json`
//! (or whatever `--bookmarks` says). Written with `tb-js`'s own JSON parser on
//! the way in and a hand-written writer on the way out — no serde anywhere in
//! this workspace, and a file a human can fix with an editor.

use std::path::PathBuf;

use tb_js::value::{Key, Value};

#[derive(Clone, Debug, PartialEq)]
pub struct Bookmark {
    pub title: String,
    pub url: String,
}

#[derive(Clone, Debug, Default)]
pub struct Bookmarks {
    pub items: Vec<Bookmark>,
    path: Option<PathBuf>,
}

impl Bookmarks {
    /// Open (and read, when it exists) the file; `Some("")` disables saving.
    pub fn open(path: Option<PathBuf>) -> Bookmarks {
        let path = match path {
            Some(p) if p.as_os_str().is_empty() => None,
            Some(p) => Some(p),
            None => default_path(),
        };
        let mut b = Bookmarks {
            items: Vec::new(),
            path,
        };
        b.load();
        b
    }

    pub fn path(&self) -> Option<&PathBuf> {
        self.path.as_ref()
    }

    pub fn load(&mut self) {
        let Some(path) = self.path.clone() else {
            return;
        };
        if let Ok(text) = std::fs::read_to_string(path) {
            self.items = parse_bookmarks(&text);
        }
    }

    /// Write the file; errors are returned so the caller can say so in the
    /// status line instead of failing silently.
    pub fn save(&self) -> Result<(), String> {
        let Some(path) = self.path.as_ref() else {
            return Err("bookmarks are disabled (--bookmarks '')".into());
        };
        if let Some(dir) = path.parent() {
            if !dir.as_os_str().is_empty() {
                let _ = std::fs::create_dir_all(dir);
            }
        }
        std::fs::write(path, to_json(&self.items))
            .map_err(|e| format!("cannot write {}: {e}", path.display()))
    }

    pub fn add(&mut self, title: &str, url: &str) -> bool {
        if self.items.iter().any(|b| b.url == url) {
            return false;
        }
        self.items.push(Bookmark {
            title: if title.trim().is_empty() {
                url.to_string()
            } else {
                title.to_string()
            },
            url: url.to_string(),
        });
        let _ = self.save();
        true
    }

    pub fn remove(&mut self, url: &str) -> bool {
        let before = self.items.len();
        self.items.retain(|b| b.url != url);
        let changed = self.items.len() != before;
        if changed {
            let _ = self.save();
        }
        changed
    }

    pub fn has(&self, url: &str) -> bool {
        self.items.iter().any(|b| b.url == url)
    }

    pub fn find(&self, index: usize) -> Option<&Bookmark> {
        self.items.get(index)
    }
}

fn default_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("tiny").join("bookmarks.json"))
}

/// `[{"title":"…","url":"…"}, …]` — written by hand so the file stays pretty
/// and stable (no floating-point or key-order surprises in a config file).
pub fn to_json(items: &[Bookmark]) -> String {
    let mut out = String::from("[\n");
    for (i, b) in items.iter().enumerate() {
        out.push_str("  {\"title\": ");
        out.push_str(&json_string(&b.title));
        out.push_str(", \"url\": ");
        out.push_str(&json_string(&b.url));
        out.push_str(if i + 1 == items.len() { "}" } else { "}," });
        out.push('\n');
    }
    out.push_str("]\n");
    out
}

fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Read the file back with our JS engine's JSON parser (`tb-js` is the one JSON
/// implementation in this workspace; using it beats writing a second one).
pub fn parse_bookmarks(text: &str) -> Vec<Bookmark> {
    let Ok(value) = tb_js::json::parse(text) else {
        return Vec::new();
    };
    let Some(obj) = value.as_object() else {
        return Vec::new();
    };
    let items: Vec<Value> = obj.borrow().items().cloned().unwrap_or_default();
    let mut out = Vec::new();
    for it in items {
        let Some(o) = it.as_object() else { continue };
        let get = |name: &str| -> String {
            o.borrow()
                .get_own(&Key::Name(std::rc::Rc::from(name)))
                .and_then(|p| p.value())
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        };
        let url = get("url");
        if url.is_empty() {
            continue;
        }
        out.push(Bookmark {
            title: get("title"),
            url,
        });
    }
    out
}
