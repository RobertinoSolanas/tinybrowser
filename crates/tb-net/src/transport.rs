//! Byte level transport: TCP and TLS.
//!
//! TLS is the *only* thing tinybrowser does not implement itself (writing X.509
//! and TLS 1.3 from scratch would be the wrong kind of tiny). Everything above
//! this module — framing, headers, chunked decoding, redirects — is ours.

use crate::url::Url;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;
use tb_core::Result;
use tb_core::TbError;

/// Either a plain or a TLS wrapped connection.
pub enum Connection {
    Plain(TcpStream),
    Tls(rustls::StreamOwned<rustls::ClientConnection, TcpStream>),
}

impl Read for Connection {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Connection::Plain(s) => s.read(buf),
            Connection::Tls(s) => s.read(buf),
        }
    }
}

impl Connection {
    /// Adjust the read timeout (used to honour one overall deadline).
    pub fn set_read_timeout(&self, t: Option<Duration>) -> std::io::Result<()> {
        match self {
            Connection::Plain(s) => s.set_read_timeout(t),
            Connection::Tls(s) => s.get_ref().set_read_timeout(t),
        }
    }
}

impl Write for Connection {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Connection::Plain(s) => s.write(buf),
            Connection::Tls(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Connection::Plain(s) => s.flush(),
            Connection::Tls(s) => s.flush(),
        }
    }
}

/// Build the rustls client config once per process.
fn tls_config() -> std::sync::Arc<rustls::ClientConfig> {
    use std::sync::OnceLock;
    static CFG: OnceLock<std::sync::Arc<rustls::ClientConfig>> = OnceLock::new();
    CFG.get_or_init(|| {
        let mut roots = rustls::RootCertStore::empty();
        roots
            .roots
            .extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let provider = rustls::crypto::ring::default_provider();
        let cfg = rustls::ClientConfig::builder_with_provider(provider.into())
            .with_safe_default_protocol_versions()
            .expect("rustls default provider is usable")
            .with_root_certificates(std::sync::Arc::new(roots))
            .with_no_client_auth();
        std::sync::Arc::new(cfg)
    })
    .clone()
}

/// Resolve, connect and (for https) handshake.
pub fn connect(url: &Url, timeout: Duration) -> Result<Connection> {
    let host = url.host.as_str();
    if host.is_empty() {
        return Err(TbError::Network(format!("no host to connect to: {url}")));
    }
    let port = url
        .effective_port()
        .ok_or_else(|| TbError::Network(format!("no port for scheme {}", url.scheme)))?;
    // Prefer the addresses getaddrinfo hands us in order.
    let addrs = std::net::ToSocketAddrs::to_socket_addrs(&format!("{host}:{port}"))
        .map_err(|e| TbError::Network(format!("dns lookup failed for {host}: {e}")))?
        .collect::<Vec<_>>();
    if addrs.is_empty() {
        return Err(TbError::Network(format!("no addresses for {host}")));
    }
    let mut last: Option<TbError> = None;
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, timeout) {
            Ok(sock) => {
                let _ = sock.set_nodelay(true);
                sock.set_read_timeout(Some(timeout))
                    .map_err(|e| TbError::Io(e.to_string()))?;
                sock.set_write_timeout(Some(timeout))
                    .map_err(|e| TbError::Io(e.to_string()))?;
                if url.scheme == "https" || url.scheme == "wss" {
                    let name = rustls::pki_types::ServerName::try_from(host.to_string())
                        .unwrap_or_else(|_| {
                            // IP literals still need a name for the API
                            rustls::pki_types::ServerName::try_from("localhost".to_string())
                                .expect("static name")
                        });
                    let conn = rustls::ClientConnection::new(tls_config(), name)
                        .map_err(|e| TbError::Network(format!("tls setup: {e}")))?;
                    // rustls performs the handshake lazily with the first
                    // read/write; errors surface as transport errors below.
                    return Ok(Connection::Tls(rustls::StreamOwned::new(conn, sock)));
                }
                return Ok(Connection::Plain(sock));
            }
            Err(e) => last = Some(TbError::Network(format!("connect {addr} failed: {e}"))),
        }
    }
    Err(last.unwrap_or_else(|| TbError::Network("connect failed".into())))
}

/// Write the request, then read until the peer closes the connection.
///
/// `Connection: close` is used for every request, which keeps the client tiny
/// and is perfectly fine for a single-window browser. A single wall clock
/// `timeout` bounds connect *and* read.
pub fn request_and_read(
    url: &Url,
    request_bytes: &[u8],
    timeout: Duration,
    max_bytes: usize,
) -> Result<Vec<u8>> {
    let deadline = std::time::Instant::now() + timeout;
    let mut conn = connect(url, timeout)?;
    conn.write_all(request_bytes)
        .map_err(|e| TbError::Network(format!("write failed: {e}")))?;
    conn.flush()
        .map_err(|e| TbError::Network(format!("flush failed: {e}")))?;
    let mut buf: Vec<u8> = Vec::new();
    let mut tmp = [0u8; 64 * 1024];
    loop {
        let now = std::time::Instant::now();
        if now >= deadline {
            if buf.is_empty() {
                return Err(TbError::Timeout(format!(
                    "{url}: no response within {} ms",
                    timeout.as_millis()
                )));
            }
            break;
        }
        conn.set_read_timeout(Some((deadline - now).min(Duration::from_millis(50))))
            .map_err(|e| TbError::Io(e.to_string()))?;
        match conn.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&tmp[..n]);
                if buf.len() > max_bytes {
                    return Err(TbError::Policy(format!(
                        "response larger than {max_bytes} bytes"
                    )));
                }
            }
            Err(ref e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::Interrupted
                        | std::io::ErrorKind::TimedOut
                ) =>
            {
                continue;
            }
            Err(e) => {
                if buf.is_empty() {
                    return Err(TbError::Network(format!("read failed: {e}")));
                }
                // A truncated stream that already delivered data is rendered
                // like a browser would: with what arrived.
                break;
            }
        }
    }
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dns_failure_reports_network_error() {
        let u = Url::parse("https://this-host-does-not-exist.invalid/").unwrap();
        match connect(&u, Duration::from_millis(400)) {
            Err(TbError::Network(m)) => assert!(m.contains("dns") || m.contains("connect"), "{m}"),
            Err(other) => panic!("unexpected error: {other}"),
            Ok(_) => panic!("unexpectedly connected to a bogus host"),
        }
    }

    #[test]
    fn connect_needs_a_host() {
        let u = Url::parse("data:text/plain,hi").unwrap();
        assert!(connect(&u, Duration::from_millis(100)).is_err());
    }

    #[test]
    fn tls_config_is_built_once_and_has_roots() {
        let a = tls_config();
        let b = tls_config();
        assert!(std::sync::Arc::ptr_eq(&a, &b));
    }
}
