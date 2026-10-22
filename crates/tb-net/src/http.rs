//! HTTP/1.1 messages, written from scratch: header table, chunked transfer
//! decoding, response framing and status helpers.

use tb_core::TbError;
use tb_core::Result;

/// Case-insensitive header multimap (insertion ordered).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HeaderMap {
    entries: Vec<(String, String)>,
}

impl HeaderMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, name: &str, value: impl Into<String>) {
        let name = name.to_ascii_lowercase();
        for e in self.entries.iter_mut() {
            if e.0 == name {
                e.1 = value.into();
                return;
            }
        }
        self.entries.push((name, value.into()));
    }

    pub fn append(&mut self, name: &str, value: impl Into<String>) {
        self.entries
            .push((name.to_ascii_lowercase(), value.into()));
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.entries
            .iter()
            .find(|(k, _)| k == &name)
            .map(|(_, v)| v.as_str())
    }

    pub fn get_lossy(&self, name: &str) -> Option<&str> {
        self.get(name)
    }

    pub fn values(&self, name: &str) -> Vec<&str> {
        let name = name.to_ascii_lowercase();
        self.entries
            .iter()
            .filter(|(k, _)| k == &name)
            .map(|(_, v)| v.as_str())
            .collect()
    }

    /// Comma separated values of a repeated header, trimmed.
    pub fn list(&self, name: &str) -> Vec<String> {
        self.values(name)
            .iter()
            .flat_map(|v| v.split(','))
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .collect()
    }

    pub fn contains(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    pub fn remove(&mut self, name: &str) {
        let name = name.to_ascii_lowercase();
        self.entries.retain(|(k, _)| k != &name);
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.entries.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Parse one `Name: value` line.
    pub fn parse_line(&mut self, line: &str) -> bool {
        match line.split_once(':') {
            Some((n, v)) => {
                self.append(n.trim(), v.trim());
                true
            }
            None => false,
        }
    }
}

/// An HTTP request (only what a browser needs).
#[derive(Clone, Debug)]
pub struct Request {
    pub method: String,
    pub url: crate::url::Url,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

impl Request {
    pub fn get(url: crate::url::Url) -> Request {
        Request {
            method: "GET".into(),
            url,
            headers: HeaderMap::new(),
            body: Vec::new(),
        }
    }

    /// The request target (`/path?query`).
    pub fn target(&self) -> String {
        if self.url.scheme == "data" {
            return self.url.path.clone();
        }
        let mut t = if self.url.path.is_empty() {
            "/".to_string()
        } else {
            self.url.path.clone()
        };
        if let Some(q) = &self.url.query {
            t.push('?');
            t.push_str(q);
        }
        t
    }

    /// Serialise to the wire (HTTP/1.1).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = format!("{} {} HTTP/1.1\r\n", self.method, self.target());
        if !self.url.host.is_empty() {
            let host = match self.url.port {
                Some(p) if Some(p) != crate::url::Url::default_port(&self.url.scheme) => {
                    format!("{}:{}", self.url.host, p)
                }
                _ => self.url.host.clone(),
            };
            out.push_str(&format!("Host: {host}\r\n"));
        }
        for (k, v) in self.headers.iter() {
            out.push_str(&format!("{}: {v}\r\n", header_name(k)));
        }
        if !self.body.is_empty() {
            out.push_str(&format!("Content-Length: {}\r\n", self.body.len()));
        }
        out.push_str("\r\n");
        let mut bytes = out.into_bytes();
        bytes.extend_from_slice(&self.body);
        bytes
    }
}

/// Capitalise a header name for the wire (`content-type` -> `Content-Type`).
fn header_name(k: &str) -> String {
    let mut out = String::with_capacity(k.len());
    let mut upper = true;
    for c in k.chars() {
        if upper {
            out.extend(c.to_uppercase());
        } else {
            out.push(c);
        }
        upper = c == '-';
    }
    out
}

/// An HTTP response with the body already de-framed.
#[derive(Clone, Debug)]
pub struct Response {
    pub status: u16,
    pub reason: String,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
    /// Final URL after redirects (set by the client).
    pub url: String,
}

impl Response {
    pub fn ok(body: Vec<u8>) -> Response {
        let mut headers = HeaderMap::new();
        headers.insert("content-type", "text/html; charset=utf-8");
        Response {
            status: 200,
            reason: "OK".into(),
            headers,
            body,
            url: String::new(),
        }
    }

    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    pub fn is_redirect(&self) -> bool {
        matches!(self.status, 301 | 302 | 303 | 307 | 308)
    }

    pub fn content_type(&self) -> ContentType {
        ContentType::parse(self.headers.get("content-type").unwrap_or(""))
    }

    /// Character set declared by the `Content-Type` header.
    pub fn charset(&self) -> Option<String> {
        self.content_type().charset
    }
}

/// Very small `Content-Type` parser (media type + parameters).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ContentType {
    pub ty: String,
    pub subtype: String,
    pub charset: Option<String>,
    pub params: Vec<(String, String)>,
}

impl ContentType {
    pub fn parse(s: &str) -> ContentType {
        let mut parts = s.split(';');
        let media = parts.next().unwrap_or("").trim().to_ascii_lowercase();
        let (ty, subtype) = match media.split_once('/') {
            Some((a, b)) => (a.trim().to_string(), b.trim().to_string()),
            None => (media, String::new()),
        };
        let mut ct = ContentType {
            ty,
            subtype,
            charset: None,
            params: Vec::new(),
        };
        for p in parts {
            if let Some((k, v)) = p.split_once('=') {
                let k = k.trim().to_ascii_lowercase();
                let v = v.trim().trim_matches('"').to_string();
                if k == "charset" {
                    ct.charset = Some(v.to_ascii_lowercase());
                }
                ct.params.push((k, v));
            }
        }
        ct
    }

    pub fn is_html(&self) -> bool {
        self.ty == "text" && self.subtype == "html"
            || self.ty == "application" && self.subtype == "xhtml+xml"
    }

    pub fn is_css(&self) -> bool {
        self.subtype == "css" || self.subtype.ends_with("+css")
    }

    pub fn is_javascript(&self) -> bool {
        matches!(
            self.subtype.as_str(),
            "javascript" | "ecmascript" | "x-javascript" | "module"
        ) || self.subtype.ends_with("+javascript")
    }

    pub fn is_json(&self) -> bool {
        self.subtype == "json" || self.subtype.ends_with("+json")
    }

    pub fn is_image(&self) -> bool {
        self.ty == "image"
    }

    pub fn is_text(&self) -> bool {
        self.ty == "text" || self.is_css() || self.is_javascript() || self.is_json()
    }
}

impl std::fmt::Display for ContentType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.ty, self.subtype)
    }
}

/// Split `HTTP/1.1 200 OK\r\nheaders\r\n\r\nbody` into its parts and de-frame
/// the body (chunked / content-length / read-to-eof).
///
/// Returns `Ok(None)` when more bytes are needed.
pub fn parse_response(raw: &[u8]) -> Result<Option<Response>> {
    let sep = find_double_newline(raw).ok_or_else(|| {
        TbError::Network("malformed HTTP response: no header terminator".into())
    })?;
    let head = String::from_utf8_lossy(&raw[..sep]).replace("\r\n", "\n");
    let mut lines = head.split('\n');
    let status_line = lines
        .next()
        .ok_or_else(|| TbError::Network("empty status line".into()))?;
    let mut it = status_line.splitn(3, ' ');
    let _version = it.next().unwrap_or("");
    let status: u16 = it
        .next()
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(|| TbError::Network(format!("bad status line: {status_line}")))?;
    let reason = it.next().unwrap_or("").to_string();

    let mut headers = HeaderMap::new();
    for l in lines {
        if l.is_empty() {
            continue;
        }
        headers.parse_line(l);
    }

    let body_start = sep + 4;
    let body = decode_body(status, &headers, &raw[body_start..])?;
    match body {
        None => Ok(None),
        Some(b) => Ok(Some(Response {
            status,
            reason,
            headers,
            body: b,
            url: String::new(),
        })),
    }
}

fn find_double_newline(raw: &[u8]) -> Option<usize> {
    if raw.len() < 4 {
        return None;
    }
    (0..raw.len() - 3).find(|&i| {
        raw[i] == b'\r' && raw[i + 1] == b'\n' && raw[i + 2] == b'\r' && raw[i + 3] == b'\n'
    })
}

/// De-frame a body. `None` = incomplete.
fn decode_body(status: u16, headers: &HeaderMap, rest: &[u8]) -> Result<Option<Vec<u8>>> {
    // 1xx / 204 / 304 never have a body.
    if status < 200 || status == 204 || status == 304 {
        return Ok(Some(Vec::new()));
    }
    let chunked = headers
        .get("transfer-encoding")
        .map(|v| v.to_ascii_lowercase().contains("chunked"))
        .unwrap_or(false);
    if chunked {
        return decode_chunked(rest);
    }
    if let Some(cl) = headers.get("content-length").and_then(|v| v.trim().parse::<usize>().ok()) {
        if rest.len() < cl {
            return Ok(None);
        }
        return Ok(Some(rest[..cl].to_vec()));
    }
    // read until EOF: caller signals completion by passing the whole buffer.
    Ok(Some(rest.to_vec()))
}

/// Decode `transfer-encoding: chunked`. `None` when incomplete.
pub fn decode_chunked(mut input: &[u8]) -> Result<Option<Vec<u8>>> {
    let mut out = Vec::new();
    loop {
        let Some(nl) = find_line_end(input) else {
            return Ok(None);
        };
        let line = String::from_utf8_lossy(&input[..nl]).trim().to_ascii_lowercase();
        let size_str = line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_str, 16)
            .map_err(|_| TbError::Network(format!("bad chunk size: {size_str:?}")))?;
        // consume size line (including CRLF)
        let after_line = if input[nl..].starts_with(b"\r\n") {
            nl + 2
        } else {
            nl + 1
        };
        if input.len() < after_line + size + 2 {
            return Ok(None);
        }
        out.extend_from_slice(&input[after_line..after_line + size]);
        input = &input[after_line + size..];
        // expect CRLF after chunk data
        input = if input.starts_with(b"\r\n") {
            &input[2..]
        } else if input.starts_with(b"\n") {
            &input[1..]
        } else {
            &input[..]
        };
        if size == 0 {
            return Ok(Some(out));
        }
    }
}

fn find_line_end(input: &[u8]) -> Option<usize> {
    (0..input.len()).find(|&i| input[i] == b'\n')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::url::Url;

    fn req(s: &str) -> Request {
        Request::get(Url::parse(s).unwrap())
    }

    #[test]
    fn header_map_is_case_insensitive() {
        let mut h = HeaderMap::new();
        h.insert("Content-Type", "text/html");
        assert_eq!(h.get("content-type"), Some("text/html"));
        assert_eq!(h.get("CONTENT-TYPE"), Some("text/html"));
        h.append("set-cookie", "a=1");
        h.append("Set-Cookie", "b=2");
        assert_eq!(h.values("set-cookie").len(), 2);
        h.remove("SET-COOKIE");
        assert!(h.values("set-cookie").is_empty());
    }

    #[test]
    fn request_serialisation_has_host_and_target() {
        let r = req("http://example.com:8080/a/b?x=1");
        let bytes = r.to_bytes();
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.starts_with("GET /a/b?x=1 HTTP/1.1\r\n"));
        assert!(text.contains("Host: example.com:8080\r\n"));
    }

    #[test]
    fn parses_plain_response() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: 5\r\n\r\nhello";
        let r = parse_response(raw).unwrap().unwrap();
        assert_eq!(r.status, 200);
        assert_eq!(r.reason, "OK");
        assert_eq!(&r.body, b"hello");
        assert_eq!(r.charset().as_deref(), Some("utf-8"));
        assert!(r.content_type().is_html());
    }

    #[test]
    fn parses_chunked_response() {
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n";
        let r = parse_response(raw).unwrap().unwrap();
        assert_eq!(&r.body, b"hello world");
    }

    #[test]
    fn incomplete_body_returns_none() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nshort";
        assert!(parse_response(raw).unwrap().is_none());
    }

    #[test]
    fn no_body_statuses() {
        let raw = b"HTTP/1.1 304 Not Modified\r\n\r\n";
        let r = parse_response(raw).unwrap().unwrap();
        assert_eq!(r.status, 304);
        assert!(r.body.is_empty());
    }

    #[test]
    fn content_type_parsing() {
        let ct = ContentType::parse("TEXT/HTML; charset=\"ISO-8859-1\"");
        assert_eq!(ct.ty, "text");
        assert_eq!(ct.subtype, "html");
        assert_eq!(ct.charset.as_deref(), Some("iso-8859-1"));
        assert!(ContentType::parse("text/css").is_css());
        assert!(ContentType::parse("application/javascript").is_javascript());
        assert!(ContentType::parse("image/png").is_image());
        assert!(!ContentType::parse("application/pdf").is_text());
    }

    #[test]
    fn chunked_decoder_reports_garbage() {
        assert!(decode_chunked(b"not hexadecimal\r\n").is_err());
    }
}
