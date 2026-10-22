//! A minimal cookie jar implementing the parts of RFC 6265 a browser needs:
//! domain matching, path matching, `Secure`, `HttpOnly` (kept but never sent
//! to scripts), `Max-Age` and `Expires`.

use crate::url::Url;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    /// Empty => session cookie (host only).
    pub domain: String,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    /// `None` => session cookie.
    pub expires_unix: Option<i64>,
}

impl Cookie {
    /// Parse one `Set-Cookie` header value against the requesting URL.
    pub fn parse(header: &str, url: &Url) -> Option<Cookie> {
        let mut parts = header.split(';');
        let nv = parts.next()?.trim();
        let (name, value) = nv.split_once('=')?;
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        let mut c = Cookie {
            name: name.to_string(),
            value: value.trim().to_string(),
            domain: url.domain(),
            path: default_path(&url.path),
            secure: url.is_https(),
            http_only: false,
            expires_unix: None,
        };
        for attr in parts {
            let attr = attr.trim();
            let (k, v) = match attr.split_once('=') {
                Some((k, v)) => (k.trim().to_ascii_lowercase(), v.trim()),
                None => (attr.to_ascii_lowercase(), ""),
            };
            match k.as_str() {
                "domain" => {
                    let d = v.trim_start_matches('.').to_ascii_lowercase();
                    // Reject cookies for a domain that does not cover the URL.
                    if covers(&url.host, &d) {
                        c.domain = d;
                    }
                }
                "path" => {
                    if v.starts_with('/') {
                        c.path = v.to_string();
                    }
                }
                "secure" => c.secure = true,
                "httponly" => c.http_only = true,
                "max-age" => {
                    if let Ok(secs) = v.parse::<i64>() {
                        c.expires_unix = Some(now_unix() + secs);
                    }
                }
                "expires" => {
                    if let Some(t) = parse_http_date(v) {
                        c.expires_unix = Some(t);
                    }
                }
                _ => {}
            }
        }
        Some(c)
    }

    pub fn is_expired(&self, now: i64) -> bool {
        matches!(self.expires_unix, Some(t) if t <= now)
    }
}

fn default_path(path: &str) -> String {
    if path.is_empty() || !path.starts_with('/') {
        return "/".to_string();
    }
    match path.rfind('/') {
        Some(0) | None => "/".to_string(),
        Some(i) => path[..i].to_string(),
    }
}

/// Does `host` satisfy cookie `domain`?
fn covers(host: &str, domain: &str) -> bool {
    let host = host.trim_start_matches('.');
    let domain = domain.trim_start_matches('.');
    host == domain || host.ends_with(&format!(".{domain}"))
}

fn path_matches(req_path: &str, cookie_path: &str) -> bool {
    if cookie_path == "/" || cookie_path.is_empty() {
        return true;
    }
    if req_path == cookie_path {
        return true;
    }
    req_path.starts_with(cookie_path)
        && (req_path.as_bytes().get(cookie_path.len()) == Some(&b'/') || cookie_path.ends_with('/'))
}

/// Broken-down HTTP date (`IMF-fixdate`, RFC 822 and RFC 850 flavours).
pub fn parse_http_date(s: &str) -> Option<i64> {
    let s = s.trim();
    let (_, rest) = match s.split_once(", ") {
        Some((dow, rest)) => (dow, rest),
        None => ("", s),
    };
    let mut it = rest.split_whitespace();
    let day: i64 = it.next()?.parse().ok()?;
    let month = it.next()?.to_ascii_lowercase();
    let year: i64 = it.next()?.parse().ok()?;
    let year = if year < 70 { year + 2000 } else if year < 1000 { year } else { year };
    let month = match month.get(..3)? {
        "jan" => 1,
        "feb" => 2,
        "mar" => 3,
        "apr" => 4,
        "may" => 5,
        "jun" => 6,
        "jul" => 7,
        "aug" => 8,
        "sep" => 9,
        "oct" => 10,
        "nov" => 11,
        "dec" => 12,
        _ => return None,
    };
    let hms = it.next().unwrap_or("00:00:00");
    let mut t = hms.split(':');
    let h: i64 = t.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    let m: i64 = t.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    let sec: i64 = t.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    Some(days_from_civil(year, month, day) * 86_400 + h * 3600 + m * 60 + sec)
}

/// Howard Hinnant's `days_from_civil`.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

pub fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// The cookie store of a browser session.
#[derive(Debug, Default, Clone)]
pub struct CookieJar {
    cookies: Vec<Cookie>,
}

impl CookieJar {
    pub fn new() -> Self {
        Self::default()
    }

    /// Handle all `Set-Cookie` headers of a response.
    pub fn store_from_response(&mut self, headers: &crate::http::HeaderMap, url: &Url) {
        for h in headers.values("set-cookie") {
            if let Some(c) = Cookie::parse(h, url) {
                self.insert(c);
            }
        }
    }

    pub fn insert(&mut self, c: Cookie) {
        self.cookies.retain(|x| !(x.name == c.name && x.domain == c.domain && x.path == c.path));
        if !c.is_expired(now_unix()) {
            self.cookies.push(c);
        }
    }

    /// `Cookie: ` header value for a request, or `None` when nothing matches.
    pub fn header_for(&self, url: &Url) -> Option<String> {
        let now = now_unix();
        let mut parts = Vec::new();
        for c in self.cookies.iter() {
            if c.is_expired(now) {
                continue;
            }
            if c.secure && !url.is_https() {
                continue;
            }
            if !covers(&url.host, &c.domain) {
                continue;
            }
            if !path_matches(&url.path, &c.path) {
                continue;
            }
            parts.push(format!("{}={}", c.name, c.value));
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join("; "))
        }
    }

    /// `document.cookie` view (skips `HttpOnly`).
    pub fn document_cookie(&self, url: &Url) -> String {
        let now = now_unix();
        self.cookies
            .iter()
            .filter(|c| !c.http_only && !c.is_expired(now) && covers(&url.host, &c.domain))
            .map(|c| format!("{}={}", c.name, c.value))
            .collect::<Vec<_>>()
            .join("; ")
    }

    pub fn len(&self) -> usize {
        self.cookies.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cookies.is_empty()
    }

    pub fn remove_expired(&mut self) {
        let now = now_unix();
        self.cookies.retain(|c| !c.is_expired(now));
    }

    pub fn clear(&mut self) {
        self.cookies.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        crate::url::parse_or_panic(s)
    }

    #[test]
    fn stores_and_sends_matching_cookies() {
        let mut jar = CookieJar::new();
        let u = url("https://taz.de/Nachrichten/Index.html");
        jar.store_from_response(
            &header(&[("set-cookie", "sid=abc; Path=/; Secure")]),
            &u,
        );
        assert_eq!(jar.len(), 1);
        assert_eq!(jar.header_for(&u).as_deref(), Some("sid=abc"));
        // other domain does not get it
        assert_eq!(jar.header_for(&url("https://other.org/")), None);
    }

    #[test]
    fn path_scoping() {
        let mut jar = CookieJar::new();
        let set = url("https://example.com/admin/login");
        jar.insert(Cookie::parse("k=v; Path=/admin", &set).unwrap());
        assert!(jar
            .header_for(&url("https://example.com/admin/x"))
            .is_some());
        assert!(jar.header_for(&url("https://example.com/other")).is_none());
    }

    #[test]
    fn domain_scope_is_checked() {
        let u = url("https://www.taz.de/");
        // cannot set a cookie for a parent domain we are not under
        let c = Cookie::parse("a=1; Domain=.example.com", &u).unwrap();
        assert_eq!(c.domain, "www.taz.de", "foreign domain must be ignored");
        let c2 = Cookie::parse("b=2; Domain=.taz.de", &u).unwrap();
        assert_eq!(c2.domain, "taz.de");
        assert!(covers("www.taz.de", "taz.de"));
        assert!(!covers("evilde", "taz.de"));
    }

    #[test]
    fn max_age_expiry_and_removal() {
        let mut jar = CookieJar::new();
        let u = url("https://example.com/");
        jar.insert(Cookie::parse("gone=1; Max-Age=-5", &u).unwrap());
        assert!(jar.is_empty());
        jar.insert(Cookie::parse("live=1; Max-Age=600", &u).unwrap());
        assert_eq!(jar.header_for(&u).as_deref(), Some("live=1"));
    }

    #[test]
    fn http_date_parsing() {
        // Thu, 01 Jan 1970 00:00:00 GMT
        assert_eq!(parse_http_date("Thu, 01 Jan 1970 00:00:00 GMT"), Some(0));
        assert_eq!(parse_http_date("Mon, 02 Jan 2024 15:04:05 GMT"), Some(1_704_207_845));
        assert_eq!(parse_http_date("nonsense"), None);
    }

    #[test]
    fn httponly_is_hidden_from_scripts() {
        let mut jar = CookieJar::new();
        let u = url("https://example.com/");
        jar.insert(Cookie::parse("secret=1; HttpOnly", &u).unwrap());
        jar.insert(Cookie::parse("visible=2", &u).unwrap());
        assert_eq!(jar.document_cookie(&u), "visible=2");
        assert!(jar.header_for(&u).unwrap().contains("secret=1"));
    }

    fn header(pairs: &[(&str, &str)]) -> crate::http::HeaderMap {
        let mut h = crate::http::HeaderMap::new();
        for (k, v) in pairs {
            h.append(k, *v);
        }
        h
    }
}
