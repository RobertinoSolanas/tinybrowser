//! # tb-core
//!
//! Shared kernel of the tinybrowser workspace: geometry and colour value
//! objects, the error taxonomy, engine configuration and the non-fatal
//! `Problems` collector.
//!
//! Everything here is dependency free and used by every other crate.

pub mod color;
pub mod config;
pub mod deflate;
pub mod error;
pub mod geometry;
pub mod problems;
pub mod text;
pub mod time;

pub use color::Color;
pub use config::{Capabilities, ColorScheme, EngineConfig};
pub use deflate::{
    adler32, content_decode, crc32, deflate, gzip_deflate, gzip_inflate, inflate, zlib_deflate,
    zlib_inflate,
};
pub use error::{Phase, Result, TbError};
pub use geometry::{Point, Rect, Sides, Size};
pub use problems::{Problem, Problems, Severity};
pub use time::Stopwatch;

/// The `User-Agent` string tinybrowser sends. It deliberately identifies the
/// project (many sites, including taz.de, serve cleaner markup to bots).
pub const USER_AGENT: &str = "tinybrowser/0.1 (Rust; HTML/CSS/JS engines written from scratch)";

/// Short banner for the CLI.
pub const BANNER: &str = "tinybrowser — a tiny web browser with hand written HTML/CSS/JS engines";
