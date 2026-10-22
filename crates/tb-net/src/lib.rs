//! # tb-net — the `retrieval` bounded context
//!
//! Own URL implementation, own HTTP/1.1 message handling, TCP/TLS transport,
//! charset detection, cookie jar and HTTP cache.
//!
//! ```
//! use tb_net::{HttpClient, Url};
//! let url = Url::parse("https://taz.de/").unwrap();
//! assert_eq!(url.host, "taz.de");
//! let client = HttpClient::default();
//! assert!(client.config().timeout_ms > 0);
//! ```

pub mod cache;
pub mod charset;
pub mod client;
pub mod cookies;
pub mod http;
pub mod transport;
pub mod url;

pub use cache::{CacheEntry, HttpCache};
pub use charset::{decode_document, Charset};
pub use client::{base64_decode, base64_encode, FetchResult, HttpClient};
pub use cookies::{Cookie, CookieJar};
pub use http::{ContentType, HeaderMap, Request, Response};
pub use url::{form_urldecode, percent_decode, percent_encode, Url};
