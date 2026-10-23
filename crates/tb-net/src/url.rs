//! A from-scratch URL implementation.
//!
//! Covers what a browser actually needs from RFC&nbsp;3986 plus the few
//! WHATWG-specific habits of real HTML: protocol relative references
//! (`//cdn.example/x.js`), query only (`?page=2`), fragment only (`#top`),
//! backslashes used as separators on the wire, and `data:` URLs.

use std::fmt;
use tb_core::Result;
use tb_core::TbError;

/// Percent-decode a byte string (UTF-8 lossy at the end).
pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hi = (bytes[i + 1] as char).to_digit(16);
                let lo = bytes.get(i + 2).and_then(|b| (*b as char).to_digit(16));
                match (hi, lo) {
                    (Some(h), Some(l)) => {
                        out.push((h * 16 + l) as u8);
                        i += 3;
                    }
                    _ => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b'+');
                i += 1;
            }
            _ => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Percent-encode everything that is not in the URL "path" set.
pub fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'~'
            | b'/'
            | b'!'
            | b'$'
            | b'&'
            | b'\''
            | b'('
            | b')'
            | b'*'
            | b'+'
            | b','
            | b';'
            | b'='
            | b':'
            | b'@' => out.push(*b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Percent-encode for `application/x-www-form-urlencoded`.
pub fn form_urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'*' => {
                out.push(*b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Decode `application/x-www-form-urlencoded` (`+` means space here).
pub fn form_urldecode(s: &str) -> String {
    percent_decode(&s.replace('+', "%20"))
}

/// Parsed URL.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Url {
    /// lower case scheme without `:`, e.g. `https`, `http`, `data`, `about`.
    pub scheme: String,
    pub username: String,
    pub password: Option<String>,
    /// lower case host name (or IP literal) without port; empty for `data:`.
    pub host: String,
    /// explicit port only (use [`Url::effective_port`] for the real one).
    pub port: Option<u16>,
    /// path, always starts with `/` for http(s); may be empty for `data:`.
    pub path: String,
    pub query: Option<String>,
    pub fragment: Option<String>,
}

impl Url {
    /// Parse an absolute URL.
    pub fn parse(input: &str) -> Result<Url> {
        let s = input.trim();
        if s.is_empty() {
            return Err(TbError::Network("empty url".into()));
        }
        // Strip fragment first.
        let (rest, fragment) = match s.find('#') {
            Some(i) => (&s[..i], Some(s[i + 1..].to_string())),
            None => (s, None),
        };
        let (rest, query) = match rest.find('?') {
            Some(i) => (&rest[..i], Some(rest[i + 1..].to_string())),
            None => (rest, None),
        };
        let Some(colon) = rest.find(':') else {
            return Err(TbError::Network(format!("no scheme: {input}")));
        };
        let scheme = rest[..colon].to_ascii_lowercase();
        if scheme.is_empty()
            || !scheme
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'-' || b == b'.')
        {
            return Err(TbError::Network(format!("bad scheme: {input}")));
        }
        let rest = &rest[colon + 1..];

        if scheme == "data" {
            return Ok(Url {
                scheme,
                username: String::new(),
                password: None,
                host: String::new(),
                port: None,
                path: rest.to_string(),
                query,
                fragment,
            });
        }
        if scheme == "about" || scheme == "javascript" {
            return Ok(Url {
                scheme,
                username: String::new(),
                password: None,
                host: String::new(),
                port: None,
                path: rest.to_string(),
                query,
                fragment,
            });
        }

        let body = rest.strip_prefix("//").unwrap_or(rest);
        // authority ends at the first `/`
        let (authority, path) = match body.find('/') {
            Some(i) => (&body[..i], &body[i..]),
            None => (body, ""),
        };
        let (userinfo, hostport) = match authority.rfind('@') {
            Some(i) => (Some(&authority[..i]), &authority[i + 1..]),
            None => (None, authority),
        };
        let (username, password) = match userinfo {
            Some(ui) => match ui.find(':') {
                Some(i) => (percent_decode(&ui[..i]), Some(percent_decode(&ui[i + 1..]))),
                None => (percent_decode(ui), None),
            },
            None => (String::new(), None),
        };
        let (host, port) = split_host_port(hostport)?;
        if host.is_empty() {
            return Err(TbError::Network(format!("missing host: {input}")));
        }
        Ok(Url {
            scheme,
            username,
            password,
            host: host.to_ascii_lowercase(),
            port,
            path: normalize_path(&path.replace('\\', "/")),
            query,
            fragment,
        })
    }

    /// Parse `input` relative to `base` (browser `new URL(href, base)`).
    ///
    /// Never fails for the shapes real pages use; falls back to the base URL
    /// when the reference is nonsensical.
    pub fn join(base: &Url, input: &str) -> Result<Url> {
        let raw = input.trim();
        if raw.is_empty() {
            let mut u = base.clone();
            u.fragment = None;
            return Ok(u);
        }
        // Ignore control characters and newlines, like browsers do.
        let cleaned: String = raw
            .chars()
            .filter(|c| !matches!(c, '\u{0}' | '\t' | '\n' | '\r'))
            .collect();
        let s = cleaned.as_str();

        if let Some(hash) = s.strip_prefix('#') {
            let mut u = base.clone();
            u.fragment = Some(hash.to_string());
            return Ok(u);
        }
        if s.starts_with('?') {
            let mut u = base.clone();
            u.query = Some(s[1..].to_string());
            u.fragment = None;
            return Ok(u);
        }
        // scheme present?
        if let Some(colon) = s.find(':') {
            let scheme = &s[..colon];
            let scheme_ok = !scheme.is_empty()
                && scheme.bytes().next().map(|b| b.is_ascii_alphabetic()) == Some(true)
                && scheme
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'-' || b == b'.');
            if scheme_ok {
                if scheme.eq_ignore_ascii_case("javascript")
                    || scheme.eq_ignore_ascii_case("mailto")
                {
                    // keep verbatim, they are not fetchable but must round-trip
                    return Ok(Url {
                        scheme: scheme.to_ascii_lowercase(),
                        username: String::new(),
                        password: None,
                        host: String::new(),
                        port: None,
                        path: s[colon + 1..].to_string(),
                        query: None,
                        fragment: None,
                    });
                }
                return Url::parse(s);
            }
        }
        if s.starts_with("//") {
            let mut u = Url::parse(&format!("{}:{s}", base.scheme))?;
            if u.scheme.is_empty() {
                u.scheme = base.scheme.clone();
            }
            return Ok(u);
        }
        // relative path reference
        let joined = if s.starts_with('/') {
            s.to_string()
        } else {
            let dir = match base.path.rfind('/') {
                Some(i) => &base.path[..=i],
                None => "/",
            };
            format!("{dir}{s}")
        };
        Ok(Url {
            scheme: base.scheme.clone(),
            username: base.username.clone(),
            password: base.password.clone(),
            host: base.host.clone(),
            port: base.port,
            path: normalize_path(&joined.replace('\\', "/")),
            query: None,
            fragment: None,
        })
    }

    /// Default port for the scheme.
    pub fn default_port(scheme: &str) -> Option<u16> {
        match scheme {
            "http" => Some(80),
            "https" => Some(443),
            "ws" => Some(80),
            "wss" => Some(443),
            "ftp" => Some(21),
            _ => None,
        }
    }

    pub fn effective_port(&self) -> Option<u16> {
        self.port.or_else(|| Url::default_port(&self.scheme))
    }

    pub fn is_https(&self) -> bool {
        self.scheme == "https"
    }

    pub fn is_http(&self) -> bool {
        self.scheme == "http" || self.scheme == "https"
    }

    /// `scheme://host[:port]`.
    pub fn origin(&self) -> String {
        if !self.is_http() {
            return format!("{}:", self.scheme);
        }
        match self.port {
            Some(p) if Some(p) != Url::default_port(&self.scheme) => {
                format!("{}://{}:{}", self.scheme, self.host, p)
            }
            _ => format!("{}://{}", self.scheme, self.host),
        }
    }

    /// Same-origin check (used for cookie and referrer policy).
    pub fn same_origin(&self, other: &Url) -> bool {
        self.scheme == other.scheme
            && self.host == other.host
            && self.effective_port() == other.effective_port()
    }

    /// Path + query + fragment.
    pub fn path_with_query(&self) -> String {
        let mut out = if self.path.is_empty() {
            "/".to_string()
        } else {
            self.path.clone()
        };
        if let Some(q) = &self.query {
            out.push('?');
            out.push_str(q);
        }
        if let Some(f) = &self.fragment {
            out.push('#');
            out.push_str(f);
        }
        out
    }

    /// `host` with a leading dot stripped (cookie domain normalisation).
    pub fn domain(&self) -> String {
        self.host.trim_start_matches('.').to_string()
    }

    /// Decoded query parameters (`a=1&b=2`).
    pub fn query_pairs(&self) -> Vec<(String, String)> {
        let Some(q) = &self.query else {
            return Vec::new();
        };
        q.split('&')
            .filter(|p| !p.is_empty())
            .map(|p| match p.split_once('=') {
                Some((k, v)) => (percent_decode(k), percent_decode(v)),
                None => (percent_decode(p), String::new()),
            })
            .collect()
    }

    /// The last path segment (`/a/b.html` -> `b.html`).
    pub fn basename(&self) -> String {
        self.path
            .rsplit('/')
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or(&self.path)
            .to_string()
    }

    pub fn extension(&self) -> String {
        match self.basename().rsplit_once('.') {
            Some((_, ext)) => ext.to_ascii_lowercase(),
            None => String::new(),
        }
    }

    /// URL with the fragment removed.
    pub fn without_fragment(&self) -> Url {
        let mut u = self.clone();
        u.fragment = None;
        u
    }
}

fn split_host_port(hostport: &str) -> Result<(String, Option<u16>)> {
    let hp = hostport.trim();
    if let Some(rest) = hp.strip_prefix('[') {
        // IPv6 literal
        let Some(i) = rest.find(']') else {
            return Err(TbError::Network(format!("bad ipv6 host: {hostport}")));
        };
        let host = &rest[..i];
        let after = &rest[i + 1..];
        let port = match after.strip_prefix(':') {
            Some(p) if !p.is_empty() => Some(
                p.parse::<u16>()
                    .map_err(|_| TbError::Network(format!("bad port: {p}")))?,
            ),
            _ => None,
        };
        return Ok((format!("[{host}]"), port));
    }
    match hp.rfind(':') {
        Some(i) => {
            let port_str = &hp[i + 1..];
            if port_str.is_empty() {
                Ok((hp[..i].to_string(), None))
            } else {
                let port: u16 = port_str
                    .parse()
                    .map_err(|_| TbError::Network(format!("bad port: {port_str}")))?;
                Ok((hp[..i].to_string(), Some(port)))
            }
        }
        None => Ok((hp.to_string(), None)),
    }
}

/// Resolve `.`/`..` segments, ensure a leading `/`.
pub fn normalize_path(path: &str) -> String {
    let path = if path.is_empty() { "/" } else { path };
    let mut parts: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(seg),
        }
    }
    let mut out = parts.join("/");
    if path.starts_with('/') && !out.starts_with('/') {
        out.insert(0, '/');
    }
    if path.ends_with('/') && !out.ends_with('/') {
        out.push('/');
    }
    if out.is_empty() {
        out.push('/');
    }
    out
}

impl fmt::Display for Url {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.scheme == "data" || self.scheme == "about" || self.scheme == "javascript" {
            write!(f, "{}:{}", self.scheme, self.path)?;
        } else {
            write!(f, "{}://", self.scheme)?;
            if !self.username.is_empty() {
                write!(f, "{}", self.username)?;
                if let Some(p) = &self.password {
                    write!(f, ":{p}")?;
                }
                write!(f, "@")?;
            }
            write!(f, "{}", self.host)?;
            if let Some(p) = self.port {
                if Some(p) != Url::default_port(&self.scheme) {
                    write!(f, ":{p}")?;
                }
            }
            write!(f, "{}", self.path)?;
        }
        if let Some(q) = &self.query {
            write!(f, "?{q}")?;
        }
        if let Some(fr) = &self.fragment {
            write!(f, "#{fr}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for Url {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Url({self})")
    }
}

/// `https://example.com/a/b` style helper for tests.
pub fn parse_or_panic(s: &str) -> Url {
    Url::parse(s).unwrap_or_else(|e| panic!("test url {s} invalid: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_https() {
        let u = Url::parse("https://taz.de/Faustname/").unwrap();
        assert_eq!(u.scheme, "https");
        assert_eq!(u.host, "taz.de");
        assert_eq!(u.path, "/Faustname/");
        assert_eq!(u.effective_port(), Some(443));
        assert_eq!(u.origin(), "https://taz.de");
    }

    #[test]
    fn lowercases_host_and_keeps_case_in_path() {
        let u = Url::parse("HTTPS://Taz.DE/AaB").unwrap();
        assert_eq!(u.host, "taz.de");
        assert_eq!(u.path, "/AaB");
        assert_eq!(u.to_string(), "https://taz.de/AaB");
    }

    #[test]
    fn parses_userinfo_port_query_fragment() {
        let u = Url::parse("http://user:pw@example.com:8080/p/a.html?x=1&y=2#frag").unwrap();
        assert_eq!(u.username, "user");
        assert_eq!(u.password.as_deref(), Some("pw"));
        assert_eq!(u.port, Some(8080));
        assert_eq!(u.query.as_deref(), Some("x=1&y=2"));
        assert_eq!(u.fragment.as_deref(), Some("frag"));
        assert_eq!(u.query_pairs()[1].0, "y");
    }

    #[test]
    fn ipv6_host() {
        let u = Url::parse("http://[::1]:3000/x").unwrap();
        assert_eq!(u.host, "[::1]");
        assert_eq!(u.port, Some(3000));
    }

    #[test]
    fn normalizes_dot_segments() {
        assert_eq!(normalize_path("/a/./b/../c"), "/a/c");
        assert_eq!(normalize_path("/a/b/../../c"), "/c");
        assert_eq!(normalize_path(""), "/");
    }

    #[test]
    fn joins_relative_references() {
        let base = parse_or_panic("https://taz.de/Nachrichten/Index.html?a=1");
        assert_eq!(
            Url::join(&base, "/lib/x.css").unwrap().to_string(),
            "https://taz.de/lib/x.css"
        );
        assert_eq!(
            Url::join(&base, "sub/x.html").unwrap().to_string(),
            "https://taz.de/Nachrichten/sub/x.html"
        );
        assert_eq!(
            Url::join(&base, "../up.html").unwrap().to_string(),
            "https://taz.de/up.html"
        );
        assert_eq!(
            Url::join(&base, "//cdn.example.com/j.js")
                .unwrap()
                .to_string(),
            "https://cdn.example.com/j.js"
        );
        assert_eq!(
            Url::join(&base, "#top").unwrap().fragment.as_deref(),
            Some("top")
        );
        assert_eq!(
            Url::join(&base, "?p=2").unwrap().query.as_deref(),
            Some("p=2")
        );
        assert_eq!(
            Url::join(&base, "https://other.org/x").unwrap().host,
            "other.org"
        );
    }

    #[test]
    fn join_with_base_without_path() {
        let base = parse_or_panic("http://example.com");
        assert_eq!(base.path, "/");
        assert_eq!(
            Url::join(&base, "a/b").unwrap().to_string(),
            "http://example.com/a/b"
        );
    }

    #[test]
    fn javascript_and_mailto_urls_survive_links() {
        let base = parse_or_panic("https://taz.de/");
        let u = Url::join(&base, "javascript:void(0)").unwrap();
        assert_eq!(u.scheme, "javascript");
        let m = Url::join(&base, "mailto:leserbrief@taz.de").unwrap();
        assert_eq!(m.scheme, "mailto");
        assert!(!m.is_http());
    }

    #[test]
    fn percent_coding() {
        assert_eq!(percent_decode("a%20b+c"), "a b+c");
        assert_eq!(form_urldecode("a+b%26c"), "a b&c");
        assert_eq!(percent_decode("%C3%A4"), "ä");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(form_urlencode("a b&c"), "a+b%26c");
    }

    #[test]
    fn rejects_garbage() {
        assert!(Url::parse("").is_err());
        assert!(Url::parse("http://").is_err());
        assert!(Url::parse("://noscheme").is_err());
        assert!(Url::parse("http://host:notaport/").is_err());
    }

    #[test]
    fn default_port_is_omitted_in_display() {
        let u = Url::parse("https://taz.de:443/").unwrap();
        assert_eq!(u.to_string(), "https://taz.de/");
    }

    #[test]
    fn same_origin() {
        let a = parse_or_panic("https://taz.de/a");
        let b = parse_or_panic("https://taz.de/b");
        let c = parse_or_panic("http://taz.de/b");
        assert!(a.same_origin(&b));
        assert!(!a.same_origin(&c));
    }
}
