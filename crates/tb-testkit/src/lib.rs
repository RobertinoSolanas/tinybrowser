//! # tb-testkit — automatic browser tests without the internet
//!
//! A multi threaded HTTP/1.1 server built on `std::net` only. Every browser
//! level test spins one of these up, points the browser at it and can then
//! assert on what the server saw (headers, cookies, number of requests).

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// What the server recorded for one request.
#[derive(Debug, Clone)]
pub struct RecordedRequest {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl RecordedRequest {
    pub fn header(&self, name: &str) -> Option<&str> {
        let n = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(k, _)| k.to_ascii_lowercase() == n)
            .map(|(_, v)| v.as_str())
    }
}

/// A canned response.
#[derive(Debug, Clone)]
pub struct StubResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// Send the body with `transfer-encoding: chunked`.
    pub chunked: bool,
    /// Compress the body with our own gzip encoder before sending.
    pub gzipped: bool,
    /// Artificial delay before answering (timeout tests).
    pub delay: Duration,
}

impl Default for StubResponse {
    fn default() -> Self {
        StubResponse {
            status: 200,
            headers: Vec::new(),
            body: Vec::new(),
            chunked: false,
            gzipped: false,
            delay: Duration::from_millis(0),
        }
    }
}

impl StubResponse {
    pub fn html(body: impl Into<String>) -> Self {
        StubResponse {
            status: 200,
            headers: vec![("content-type".into(), "text/html; charset=utf-8".into())],
            body: body.into().into_bytes(),
            ..Default::default()
        }
    }

    pub fn bytes(content_type: &str, body: impl Into<Vec<u8>>) -> Self {
        StubResponse {
            status: 200,
            headers: vec![("content-type".into(), content_type.into())],
            body: body.into(),
            ..Default::default()
        }
    }

    pub fn css(body: impl Into<String>) -> Self {
        StubResponse {
            status: 200,
            headers: vec![("content-type".into(), "text/css; charset=utf-8".into())],
            body: body.into().into_bytes(),
            ..Default::default()
        }
    }

    pub fn js(body: impl Into<String>) -> Self {
        StubResponse {
            status: 200,
            headers: vec![(
                "content-type".into(),
                "application/javascript; charset=utf-8".into(),
            )],
            body: body.into().into_bytes(),
            ..Default::default()
        }
    }

    pub fn redirect(location: &str) -> Self {
        StubResponse {
            status: 302,
            headers: vec![("location".into(), location.into())],
            body: Vec::new(),
            ..Default::default()
        }
    }

    pub fn not_found() -> Self {
        StubResponse {
            status: 404,
            headers: vec![("content-type".into(), "text/plain".into())],
            body: b"not found".to_vec(),
            ..Default::default()
        }
    }

    pub fn with_header(mut self, k: &str, v: &str) -> Self {
        self.headers.push((k.to_ascii_lowercase(), v.to_string()));
        self
    }

    pub fn chunked(mut self) -> Self {
        self.chunked = true;
        self
    }

    pub fn gzipped(mut self) -> Self {
        self.gzipped = true;
        self
    }

    pub fn delayed(mut self, ms: u64) -> Self {
        self.delay = Duration::from_millis(ms);
        self
    }
}

/// The running test server. Shuts down when dropped.
pub struct TestServer {
    addr: SocketAddr,
    routes: Arc<RwLock<HashMap<String, StubResponse>>>,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    connections: Arc<AtomicUsize>,
}

impl TestServer {
    /// Bind a random free port on loopback and serve in the background.
    pub fn start() -> TestServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let addr = listener.local_addr().expect("local addr");
        let routes: Arc<RwLock<HashMap<String, StubResponse>>> =
            Arc::new(RwLock::new(HashMap::new()));
        let requests: Arc<Mutex<Vec<RecordedRequest>>> = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let connections = Arc::new(AtomicUsize::new(0));

        let (r2, q2, s2, c2) = (
            routes.clone(),
            requests.clone(),
            stop.clone(),
            connections.clone(),
        );
        let worker = thread::spawn(move || {
            let _ = listener.set_nonblocking(true);
            loop {
                if s2.load(Ordering::Relaxed) {
                    break;
                }
                match listener.accept() {
                    Ok((sock, _peer)) => {
                        c2.fetch_add(1, Ordering::Relaxed);
                        let (routes, requests, stop) = (r2.clone(), q2.clone(), s2.clone());
                        thread::spawn(move || {
                            let _ = handle_connection(sock, routes, requests, stop);
                        });
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                    }
                    Err(_) => break,
                }
            }
        });

        TestServer {
            addr,
            routes,
            requests,
            stop,
            worker: Some(worker),
            connections,
        }
    }

    /// Register the response for an exact path (`/index.html`).
    pub fn route(&mut self, path: &str, resp: StubResponse) -> &mut Self {
        self.routes.write().unwrap().insert(path.to_string(), resp);
        self
    }

    /// Base URL, e.g. `http://127.0.0.1:41234`.
    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Full URL for a path.
    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url(), path)
    }

    pub fn requests(&self) -> Vec<RecordedRequest> {
        self.requests.lock().unwrap().clone()
    }

    pub fn request_count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }

    pub fn paths_requested(&self) -> Vec<String> {
        self.requests().iter().map(|r| r.path.clone()).collect()
    }

    pub fn connections(&self) -> usize {
        self.connections.load(Ordering::Relaxed)
    }

    pub fn clear_requests(&self) {
        self.requests.lock().unwrap().clear();
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.worker.take() {
            let _ = h.join();
        }
    }
}

fn handle_connection(
    mut sock: TcpStream,
    routes: Arc<RwLock<HashMap<String, StubResponse>>>,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    stop: Arc<AtomicBool>,
) -> std::io::Result<()> {
    let _ = sock.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = sock.set_write_timeout(Some(Duration::from_secs(5)));
    let mut buf: Vec<u8> = Vec::new();
    let mut tmp = [0u8; 4096];
    // read headers
    loop {
        let n = sock.read(&mut tmp)?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&tmp[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") || stop.load(Ordering::Relaxed) {
            break;
        }
        if buf.len() > 1 << 20 {
            return Ok(());
        }
    }
    let head_end = buf
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "no header end"))?;
    let head = String::from_utf8_lossy(&buf[..head_end]).replace("\r\n", "\n");
    let mut lines = head.split('\n');
    let request_line = lines.next().unwrap_or("");
    let mut it = request_line.split_whitespace();
    let method = it.next().unwrap_or("GET").to_string();
    let target = it.next().unwrap_or("/").to_string();
    let mut headers = Vec::new();
    for l in lines {
        if let Some((k, v)) = l.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    // body (content-length)
    let mut body = buf[head_end + 4..].to_vec();
    if let Some(cl) = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse::<usize>().ok())
    {
        while body.len() < cl {
            let n = sock.read(&mut tmp)?;
            if n == 0 {
                break;
            }
            body.extend_from_slice(&tmp[..n]);
        }
        body.truncate(cl);
    }
    let path = target.split('#').next().unwrap_or("/").to_string();
    requests.lock().unwrap().push(RecordedRequest {
        method,
        path: path.clone(),
        headers,
        body,
    });

    let resp = routes
        .read()
        .unwrap()
        .get(&path)
        .cloned()
        .unwrap_or_else(StubResponse::not_found);
    write_response(&mut sock, &resp)?;
    let _ = sock.shutdown(Shutdown::Write);
    Ok(())
}

fn write_response(sock: &mut TcpStream, resp: &StubResponse) -> std::io::Result<()> {
    if !resp.delay.is_zero() {
        thread::sleep(resp.delay);
    }
    let body = if resp.gzipped {
        tb_core::gzip_deflate(&resp.body)
    } else {
        resp.body.clone()
    };
    let mut head = format!("HTTP/1.1 {} {}\r\n", resp.status, reason(resp.status));
    let mut sent_content_length = false;
    for (k, v) in &resp.headers {
        if k.eq_ignore_ascii_case("content-length") {
            sent_content_length = true;
            continue;
        }
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    if resp.chunked {
        head.push_str("transfer-encoding: chunked\r\n");
    } else if !sent_content_length {
        head.push_str(&format!("content-length: {}\r\n", body.len()));
    }
    head.push_str("connection: close\r\n\r\n");
    sock.write_all(head.as_bytes())?;
    if resp.chunked {
        for chunk in body.chunks(64) {
            sock.write_all(format!("{:x}\r\n", chunk.len()).as_bytes())?;
            sock.write_all(chunk)?;
            sock.write_all(b"\r\n")?;
        }
        sock.write_all(b"0\r\n\r\n")?;
    } else {
        sock.write_all(&body)?;
    }
    sock.flush()
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        204 => "No Content",
        301 => "Moved Permanently",
        302 => "Found",
        304 => "Not Modified",
        400 => "Bad Request",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "Status",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serves_registered_routes() {
        let mut s = TestServer::start();
        s.route("/a", StubResponse::html("<p>hi</p>"));
        let mut sock = TcpStream::connect(s.addr).unwrap();
        sock.write_all(b"GET /a HTTP/1.1\r\nHost: x\r\n\r\n")
            .unwrap();
        let mut out = String::new();
        sock.read_to_string(&mut out).unwrap();
        assert!(out.starts_with("HTTP/1.1 200 OK"), "{out}");
        assert!(out.ends_with("<p>hi</p>"), "{out}");
        assert_eq!(s.request_count(), 1);
        assert_eq!(s.paths_requested(), vec!["/a".to_string()]);
    }

    #[test]
    fn unknown_path_is_404() {
        let s = TestServer::start();
        let mut sock = TcpStream::connect(s.addr).unwrap();
        sock.write_all(b"GET /nope HTTP/1.1\r\nHost: x\r\n\r\n")
            .unwrap();
        let mut out = String::new();
        sock.read_to_string(&mut out).unwrap();
        assert!(out.starts_with("HTTP/1.1 404"));
    }
}
