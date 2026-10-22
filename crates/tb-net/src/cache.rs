//! A tiny HTTP cache (memory only) with freshness and revalidation tokens.
//!
//! This is the `Cache` domain service of the `retrieval` context: the client
//! asks it whether a stored response may be reused and which validators to
//! send when not.

use crate::http::{HeaderMap, Response};
use std::collections::HashMap;

/// A stored response plus its freshness bookkeeping.
#[derive(Debug, Clone)]
pub struct CacheEntry {
    pub status: u16,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
    /// Unix seconds at which the entry was stored.
    pub stored_at: i64,
    /// Lifetime in seconds (`max-age`, `s-maxage` or a heuristic).
    pub max_age: i64,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

impl CacheEntry {
    pub fn is_fresh(&self, now: i64) -> bool {
        now.saturating_sub(self.stored_at) < self.max_age
    }

    pub fn age(&self, now: i64) -> i64 {
        (now - self.stored_at).max(0)
    }
}

/// The cache itself. Bounded so a long browsing session cannot run away.
#[derive(Debug, Default, Clone)]
pub struct HttpCache {
    entries: HashMap<String, CacheEntry>,
    order: Vec<String>,
    capacity: usize,
}

impl HttpCache {
    pub fn new() -> Self {
        HttpCache {
            entries: HashMap::new(),
            order: Vec::new(),
            capacity: 128,
        }
    }

    /// Look up a *fresh* entry.
    pub fn get(&self, url: &str, now: i64) -> Option<&CacheEntry> {
        self.entries.get(url).filter(|e| e.is_fresh(now))
    }

    /// Look up any entry (fresh or stale) to use its validators.
    pub fn get_stale(&self, url: &str) -> Option<&CacheEntry> {
        self.entries.get(url)
    }

    /// Validators (`If-None-Match`, `If-Modified-Since`) for a conditional GET.
    pub fn validators_for(&self, url: &str) -> Option<(Option<String>, Option<String>)> {
        let e = self.entries.get(url)?;
        if e.etag.is_none() && e.last_modified.is_none() {
            return None;
        }
        Some((e.etag.clone(), e.last_modified.clone()))
    }

    /// Store a response, computing its freshness lifetime.
    pub fn put(&mut self, url: &str, resp: &Response, now: i64) {
        if !matches!(resp.status, 200 | 203 | 301 | 302 | 304 | 404 | 410) {
            return;
        }
        let cc = resp
            .headers
            .get("cache-control")
            .map(|v| v.to_ascii_lowercase())
            .unwrap_or_default();
        if cc.contains("no-store") || cc.contains("no-cache") {
            return;
        }
        let max_age = parse_max_age(&cc).unwrap_or_else(|| {
            if let Some(expires) = resp.headers.get("expires").and_then(|v| {
                crate::cookies::parse_http_date(v)
            }) {
                (expires - now).max(0)
            } else if let Some(lm) = resp.headers.get("last-modified").and_then(|v| {
                crate::cookies::parse_http_date(v)
            }) {
                // heuristic: 10% of the time since last modification
                ((now - lm) / 10).clamp(0, 3600)
            } else {
                0
            }
        });
        let entry = CacheEntry {
            status: resp.status,
            headers: resp.headers.clone(),
            body: resp.body.clone(),
            stored_at: now,
            max_age,
            etag: resp.headers.get("etag").map(|s| s.to_string()),
            last_modified: resp.headers.get("last-modified").map(|s| s.to_string()),
        };
        if !self.entries.contains_key(url) {
            self.order.push(url.to_string());
            while self.order.len() > self.capacity {
                let old = self.order.remove(0);
                self.entries.remove(&old);
            }
        }
        self.entries.insert(url.to_string(), entry);
    }

    /// Replace the body/headers of a stored entry after a `304`.
    pub fn revalidate(&mut self, url: &str, resp: &Response, now: i64) {
        if let Some(e) = self.entries.get_mut(url) {
            e.stored_at = now;
            for (k, v) in resp.headers.iter() {
                if !matches!(k, "etag" | "last-modified") {
                    e.headers.insert(k, v);
                }
            }
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
    }
}

fn parse_max_age(cc: &str) -> Option<i64> {
    for part in cc.split(',') {
        let part = part.trim();
        if let Some(v) = part.strip_prefix("max-age=") {
            return v.trim().parse::<i64>().ok();
        }
        if let Some(v) = part.strip_prefix("s-maxage=") {
            return v.trim().parse::<i64>().ok();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::Response;

    fn resp(headers: &[(&str, &str)]) -> Response {
        let mut r = Response::ok(b"hello".to_vec());
        for (k, v) in headers {
            r.headers.insert(k, *v);
        }
        r
    }

    #[test]
    fn respects_max_age() {
        let mut c = HttpCache::new();
        c.put("u", &resp(&[("cache-control", "max-age=60")]), 1_000);
        assert!(c.get("u", 1_030).is_some());
        assert!(c.get("u", 1_061).is_none());
        assert!(c.get_stale("u").is_some());
    }

    #[test]
    fn no_store_is_not_cached() {
        let mut c = HttpCache::new();
        c.put("u", &resp(&[("cache-control", "no-store")]), 0);
        assert!(c.is_empty());
    }

    #[test]
    fn taz_style_no_cache_headers_are_respected() {
        let mut c = HttpCache::new();
        let r = resp(&[(
            "cache-control",
            "max-age=0, no-cache, no-store, must-revalidate",
        )]);
        c.put("https://taz.de/", &r, 0);
        assert!(c.is_empty(), "must-not-cache response was stored");
    }

    #[test]
    fn provides_validators() {
        let mut c = HttpCache::new();
        c.put(
            "u",
            &resp(&[("cache-control", "max-age=0"), ("etag", "\"x1\"")]),
            0,
        );
        let (etag, lm) = c.validators_for("u").unwrap();
        assert_eq!(etag.as_deref(), Some("\"x1\""));
        assert!(lm.is_none());
    }

    #[test]
    fn evicts_oldest() {
        let mut c = HttpCache::new();
        c.capacity = 2;
        for i in 0..4 {
            c.put(
                &format!("u{i}"),
                &resp(&[("cache-control", "max-age=100")]),
                0,
            );
        }
        assert_eq!(c.len(), 2);
        assert!(c.get("u0", 0).is_none());
        assert!(c.get("u3", 0).is_some());
    }

    #[test]
    fn heuristic_freshness_from_last_modified() {
        let mut c = HttpCache::new();
        let now = 100_000;
        c.put(
            "u",
            &resp(&[("last-modified", "Thu, 01 Jan 1970 00:00:00 GMT")]),
            now,
        );
        let e = c.get_stale("u").unwrap();
        assert!(e.max_age > 0);
        assert!(e.is_fresh(now + 10));
    }
}
