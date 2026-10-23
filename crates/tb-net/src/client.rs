//! The HTTP client of the `retrieval` context.
//!
//! Responsibilities: default headers, cookies, caching with revalidation,
//! redirects, `data:` URLs, content decoding and charset detection. It is the
//! only place that talks to the outside world.

use crate::cache::HttpCache;
use crate::charset;
use crate::cookies::CookieJar;
use crate::http::{parse_response, ContentType, HeaderMap, Request, Response};
use crate::transport;
use crate::url::Url;
use std::sync::Mutex;
use std::time::Duration;
use tb_core::{EngineConfig, Phase, Problems, Result, TbError, USER_AGENT};

/// Result of a `fetch` including the metadata the browser layer needs.
#[derive(Debug, Clone)]
pub struct FetchResult {
    pub response: Response,
    /// Decoded text (only for text-like content types).
    pub text: Option<String>,
    /// Character set that was actually used.
    pub charset: Option<String>,
    /// Seconds spent in the network (0 when served from cache).
    pub elapsed_ms: u64,
    /// `true` when the body came out of the HTTP cache.
    pub from_cache: bool,
}

impl FetchResult {
    pub fn content_type(&self) -> ContentType {
        self.response.content_type()
    }
}

/// The browser's HTTP client. Cheap to share (`Arc`), thread safe.
pub struct HttpClient {
    config: EngineConfig,
    jar: Mutex<CookieJar>,
    cache: Mutex<HttpCache>,
}

impl Default for HttpClient {
    fn default() -> Self {
        HttpClient::new(EngineConfig::default())
    }
}

impl HttpClient {
    pub fn new(config: EngineConfig) -> Self {
        HttpClient {
            config,
            jar: Mutex::new(CookieJar::new()),
            cache: Mutex::new(HttpCache::new()),
        }
    }

    pub fn config(&self) -> &EngineConfig {
        &self.config
    }

    pub fn cookie_jar(&self) -> std::sync::MutexGuard<'_, CookieJar> {
        self.jar.lock().expect("cookie jar lock")
    }

    pub fn cache(&self) -> std::sync::MutexGuard<'_, HttpCache> {
        self.cache.lock().expect("cache lock")
    }

    pub fn clear_state(&self) {
        self.cookie_jar().clear();
        self.cache().clear();
    }

    /// Turn whatever the user typed / a page provided into a fetchable URL.
    pub fn resolve_url(input: &str, base: Option<&Url>) -> Result<Url> {
        let s = input.trim();
        if s.is_empty() {
            return Err(TbError::Network("empty url".into()));
        }
        // A bare host name (`taz.de`, `localhost:8080/x`) => https/http.
        let has_scheme = s
            .find(':')
            .map(|i| {
                i > 1
                    && s[..i].bytes().next().map(|b| b.is_ascii_alphabetic()) == Some(true)
                    && s[..i]
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'-' || b == b'.')
            })
            .unwrap_or(false);
        if has_scheme {
            return Url::parse(s);
        }
        if s.starts_with('/') || s.starts_with('.') || s.starts_with('?') || s.starts_with('#') {
            if let Some(b) = base {
                return Url::join(b, s);
            }
            return Url::parse(&format!("https://{s}"));
        }
        // looks like a host if it has a dot or a colon-port and no spaces
        if !s.contains(' ') && (s.contains('.') || s.contains(':')) {
            let with_scheme = format!("https://{s}");
            return Url::parse(&with_scheme);
        }
        // otherwise: a search phrase — we use the projects default search page
        let q = crate::url::form_urlencode(s);
        Url::parse(&format!("https://duckduckgo.com/html/?q={q}"))
    }

    /// GET a URL following redirects. Non-2xx statuses are returned as `Err`
    /// unless you use [`HttpClient::send`].
    pub fn fetch(&self, url: &Url) -> Result<FetchResult> {
        let resp = self.send(url, None)?;
        if !resp.response.is_success() {
            return Err(TbError::HttpStatus {
                status: resp.response.status,
                url: url.to_string(),
            });
        }
        Ok(resp)
    }

    /// GET and decode the body as text.
    pub fn fetch_text(&self, url: &Url) -> Result<(String, Response)> {
        let r = self.fetch(url)?;
        let text = r.text.unwrap_or_default();
        Ok((text, r.response))
    }

    /// GET; returns any status (error only for transport failures).
    pub fn send(&self, url: &Url, referer: Option<&Url>) -> Result<FetchResult> {
        self.send_with_accept(
            url,
            referer,
            "text/html,application/xhtml+xml;q=0.9,*/*;q=0.5",
        )
    }

    /// GET with an explicit `Accept` header.
    pub fn send_with_accept(
        &self,
        url: &Url,
        referer: Option<&Url>,
        accept: &str,
    ) -> Result<FetchResult> {
        let mut current = url.clone();
        let mut problems = Problems::new();
        for _ in 0..=self.config.max_redirects {
            let mut result = self.once(&current, referer, accept, &mut problems);
            // follow redirects
            if let Ok(res) = &mut result {
                if res.response.is_redirect() {
                    if let Some(loc) = res.response.headers.get("location") {
                        let next = Url::join(&current, loc).map_err(|_| {
                            TbError::Network(format!("bad redirect target {loc:?}"))
                        })?;
                        current = next;
                        continue;
                    }
                }
            }
            return result;
        }
        Err(TbError::TooManyRedirects {
            url: url.to_string(),
        })
    }

    /// A single request without redirect following (but with cache/cookies).
    fn once(
        &self,
        url: &Url,
        referer: Option<&Url>,
        accept: &str,
        problems: &mut Problems,
    ) -> Result<FetchResult> {
        let started = tb_core::Stopwatch::new();
        if url.scheme == "data" {
            return self.data_url(url);
        }
        if !url.is_http() {
            return Err(TbError::Unsupported("only http/https/data urls"));
        }
        let now = crate::cookies::now_unix();
        let key = url.to_string();

        // Fresh cache entry?
        if let Some(entry) = self.cache().get(&key, now) {
            let headers = entry.headers.clone();
            let body = entry.body.clone();
            let status = entry.status;
            let ct = headers.get("content-type").unwrap_or("").to_string();
            let (text, charset) = decode_if_text(status, &ct, &body);
            return Ok(FetchResult {
                response: Response {
                    status,
                    reason: "OK (cached)".into(),
                    headers,
                    body,
                    url: key,
                },
                text,
                charset,
                elapsed_ms: 0,
                from_cache: true,
            });
        }

        let mut req = Request::get(url.clone());
        req.headers.insert("accept", accept);
        req.headers.insert("accept-language", "de,en;q=0.8");
        req.headers
            .insert("user-agent", self.config.user_agent.as_str());
        // We can decode gzip/deflate ourselves, but `identity` keeps the pipe
        // simple and taz.de answers it with a plain body.
        req.headers
            .insert("accept-encoding", "identity;q=1, gzip;q=0.9, deflate;q=0.5");
        req.headers.insert("connection", "close");
        req.headers.insert("upgrade-insecure-requests", "1");
        if self.config.send_referer {
            if let Some(r) = referer {
                req.headers.insert("referer", r.to_string());
            }
        }
        if self.config.cookies {
            if let Some(c) = self.cookie_jar().header_for(url) {
                req.headers.insert("cookie", c);
            }
            // Conditional revalidation when we have a stale entry.
            if let Some((etag, lm)) = self.cache().validators_for(&key) {
                if let Some(e) = etag {
                    req.headers.insert("if-none-match", e);
                }
                if let Some(l) = lm {
                    req.headers.insert("if-modified-since", l);
                }
            }
        }

        let timeout = Duration::from_millis(self.config.timeout_ms);
        let raw = transport::request_and_read(
            url,
            &req.to_bytes(),
            timeout,
            self.config.max_resource_bytes,
        )?;
        let mut response = parse_response(&raw)?.ok_or_else(|| {
            TbError::Network(format!(
                "connection closed before the response was complete: {key}"
            ))
        })?;
        response.url = key.clone();

        // Content-Encoding
        if let Some(enc) = response
            .headers
            .get("content-encoding")
            .map(|s| s.to_string())
        {
            if !enc.eq_ignore_ascii_case("identity") {
                response.body = tb_core::content_decode(&response.body, &enc).map_err(|e| {
                    problems.warn_ctx(Phase::Network, e.to_string(), enc.clone());
                    TbError::Network(format!("cannot decode {enc}: {e}"))
                })?;
                response.headers.remove("content-length");
            }
        }

        if self.config.cookies {
            self.cookie_jar()
                .store_from_response(&response.headers, url);
            if response.status == 304 {
                if let Some(entry) = self.cache().get_stale(&key).cloned() {
                    self.cache().revalidate(&key, &response, now);
                    let ct = entry.headers.get("content-type").unwrap_or("").to_string();
                    let (text, charset) = decode_if_text(entry.status, &ct, &entry.body);
                    return Ok(FetchResult {
                        response: Response {
                            status: entry.status,
                            reason: "OK (revalidated)".into(),
                            headers: entry.headers.clone(),
                            body: entry.body.clone(),
                            url: key,
                        },
                        text,
                        charset,
                        elapsed_ms: started.elapsed_ms(),
                        from_cache: true,
                    });
                }
            }
            self.cache().put(&key, &response, now);
        }

        let ct = response
            .headers
            .get("content-type")
            .unwrap_or("")
            .to_string();
        let (text, cs) = decode_if_text(response.status, &ct, &response.body);
        Ok(FetchResult {
            response,
            text,
            charset: cs,
            elapsed_ms: started.elapsed_ms(),
            from_cache: false,
        })
    }

    /// `data:` URL support (used by favicons, tests and inline scripts).
    fn data_url(&self, url: &Url) -> Result<FetchResult> {
        let raw = &url.path;
        let (meta, data) = raw.split_once(',').unwrap_or(("", raw));
        let meta_l = meta.to_ascii_lowercase();
        let is_b64 = meta_l.split(';').any(|p| p.trim() == "base64");
        let body = if is_b64 {
            base64_decode(data)?
        } else {
            crate::url::percent_decode(data).into_bytes()
        };
        let mime = meta.split(';').next().unwrap_or("text/plain").trim();
        let mut headers = HeaderMap::new();
        headers.insert("content-type", mime);
        headers.insert("content-length", body.len().to_string());
        let charset = charset::Charset::from_label(mime.split("charset=").nth(1).unwrap_or(""))
            .map(|c| c.label().to_string());
        let response = Response {
            status: 200,
            reason: "OK".into(),
            headers,
            body,
            url: url.to_string(),
        };
        let text =
            if mime.starts_with("text/") || mime.contains("javascript") || mime.contains("json") {
                Some(String::from_utf8_lossy(&response.body).into_owned())
            } else {
                None
            };
        Ok(FetchResult {
            response,
            text,
            charset,
            elapsed_ms: 0,
            from_cache: false,
        })
    }
}

/// Decode the body when the media type suggests text.
fn decode_if_text(
    _status: u16,
    content_type: &str,
    body: &[u8],
) -> (Option<String>, Option<String>) {
    let ct = ContentType::parse(content_type);
    if ct.is_text() || content_type.is_empty() {
        let (text, used) = charset::decode_document(body, ct.charset.as_deref());
        (Some(text), Some(used))
    } else {
        (None, None)
    }
}

/// Minimal base64 decoder (RFC 4648, tolerates missing padding).
pub fn base64_decode(s: &str) -> Result<Vec<u8>> {
    fn val(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            b'-' => Some(62),
            b'_' => Some(63),
            _ => None,
        }
    }
    let mut out = Vec::with_capacity(s.len() / 4 * 3 + 3);
    let mut acc: u32 = 0;
    let mut nbits = 0u32;
    for &b in s.as_bytes() {
        if b == b'=' || b.is_ascii_whitespace() {
            continue;
        }
        let v = val(b).ok_or_else(|| TbError::Network("bad base64".into()))? as u32;
        acc = (acc << 6) | v;
        nbits += 6;
        if nbits >= 8 {
            nbits -= 8;
            out.push(((acc >> nbits) & 0xff) as u8);
        }
    }
    Ok(out)
}

/// Base64 encoder (for tests and `data:` URL generation).
pub fn base64_encode(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            out.push(T[((n >> 6) & 63) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(T[(n & 63) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

/// Convenience: default user agent for tests that build their own requests.
pub fn default_user_agent() -> &'static str {
    USER_AGENT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_typed_input() {
        assert_eq!(
            HttpClient::resolve_url("taz.de", None).unwrap().to_string(),
            "https://taz.de/"
        );
        assert_eq!(
            HttpClient::resolve_url("https://taz.de/x", None)
                .unwrap()
                .host,
            "taz.de"
        );
        let base = Url::parse("https://taz.de/a/b.html").unwrap();
        assert_eq!(
            HttpClient::resolve_url("/c", Some(&base))
                .unwrap()
                .to_string(),
            "https://taz.de/c"
        );
        assert_eq!(
            HttpClient::resolve_url("http://localhost:8080/x", None)
                .unwrap()
                .port,
            Some(8080)
        );
    }

    #[test]
    fn data_urls_are_decoded() {
        let u = HttpClient::default();
        let r = u
            .fetch(&Url::parse("data:text/plain,Hello%20world").unwrap())
            .unwrap();
        assert_eq!(r.text.as_deref(), Some("Hello world"));
        let b64 = u
            .fetch(&Url::parse("data:text/plain;base64,aGVsbG8=").unwrap())
            .unwrap();
        assert_eq!(b64.text.as_deref(), Some("hello"));
    }

    #[test]
    fn base64_roundtrip() {
        for s in [
            "",
            "a",
            "ab",
            "abc",
            "abcd",
            "sehr langer text mit unicode ünïcode",
        ] {
            let e = base64_encode(s.as_bytes());
            assert_eq!(base64_decode(&e).unwrap(), s.as_bytes(), "{s}");
        }
        assert_eq!(base64_encode(b"Man"), "TWFu");
    }

    #[test]
    fn unsupported_scheme_is_reported() {
        let client = HttpClient::default();
        let e = client.send(&Url::parse("ftp://example.com/x").unwrap(), None);
        assert!(matches!(e, Err(TbError::Unsupported(_))));
    }

    #[test]
    fn connect_failure_is_a_network_error() {
        let cfg = EngineConfig {
            timeout_ms: 500,
            ..Default::default()
        };
        let c = HttpClient::new(cfg);
        let e = c.send(&Url::parse("https://nope.invalid/").unwrap(), None);
        assert!(matches!(e, Err(TbError::Network(_))), "got {e:?}");
    }
}
