//! Error taxonomy shared by every tinybrowser component.
//!
//! The taxonomy follows the pipeline phases of the browser so that a caller can
//! tell *where* in `fetch -> parse -> style -> layout -> script -> render`
//! something went wrong.

use std::fmt;

/// Phases of the rendering/navigation pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Phase {
    /// URL handling / resource retrieval.
    Network,
    /// HTML tokenizing + tree building.
    Html,
    /// CSS tokenizing + parsing.
    Css,
    /// Selector matching / cascade.
    Style,
    /// Box construction and layout.
    Layout,
    /// JavaScript lexing, parsing, interpretation.
    Script,
    /// Painting / rasterizing.
    Render,
    /// Orchestration (navigation, sub-resource policy).
    Browser,
}

impl fmt::Display for Phase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Phase::Network => "network",
            Phase::Html => "html",
            Phase::Css => "css",
            Phase::Style => "style",
            Phase::Layout => "layout",
            Phase::Script => "script",
            Phase::Render => "render",
            Phase::Browser => "browser",
        };
        f.write_str(s)
    }
}

/// Every recoverable/unrecoverable failure of the engine.
#[derive(Debug, Clone, PartialEq)]
pub enum TbError {
    /// I/O failure (socket, file, dns).
    Io(String),
    /// Protocol level network failure (TLS, HTTP framing, timeout).
    Network(String),
    /// Server answered with a non-success status.
    HttpStatus { status: u16, url: String },
    /// Too many redirects while navigating.
    TooManyRedirects { url: String },
    /// A parser gave up on its input (usually *not* fatal for a page).
    Parse {
        phase: Phase,
        message: String,
        line: usize,
        column: usize,
    },
    /// A script raised at runtime.
    Script(String),
    /// A feature is deliberately not implemented (yet).
    Unsupported(&'static str),
    /// Operation exceeded its time budget.
    Timeout(String),
    /// A policy (sandboxing / resource limits) refused an operation.
    Policy(String),
}

impl TbError {
    /// Convenience constructor for a parse error.
    pub fn parse(phase: Phase, message: impl Into<String>) -> Self {
        TbError::Parse {
            phase,
            message: message.into(),
            line: 0,
            column: 0,
        }
    }

    /// Convenience constructor with position.
    pub fn parse_at(phase: Phase, message: impl Into<String>, line: usize, column: usize) -> Self {
        TbError::Parse {
            phase,
            message: message.into(),
            line,
            column,
        }
    }

    /// The pipeline phase this error belongs to.
    pub fn phase(&self) -> Phase {
        match self {
            TbError::Io(_)
            | TbError::Network(_)
            | TbError::HttpStatus { .. }
            | TbError::TooManyRedirects { .. }
            | TbError::Timeout(_) => Phase::Network,
            TbError::Parse { phase, .. } => *phase,
            TbError::Script(_) => Phase::Script,
            TbError::Unsupported(_) | TbError::Policy(_) => Phase::Browser,
        }
    }

    /// `true` when a page can still be shown although this error happened.
    pub fn is_recoverable(&self) -> bool {
        matches!(
            self,
            TbError::Parse { .. } | TbError::Unsupported(_) | TbError::Script(_)
        )
    }
}

impl fmt::Display for TbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TbError::Io(m) => write!(f, "io error: {m}"),
            TbError::Network(m) => write!(f, "network error: {m}"),
            TbError::HttpStatus { status, url } => write!(f, "HTTP {status} for {url}"),
            TbError::TooManyRedirects { url } => write!(f, "too many redirects for {url}"),
            TbError::Parse {
                phase,
                message,
                line,
                column,
            } => {
                if *line == 0 {
                    write!(f, "{phase} parse error: {message}")
                } else {
                    write!(f, "{phase} parse error at {line}:{column}: {message}")
                }
            }
            TbError::Script(m) => write!(f, "script error: {m}"),
            TbError::Unsupported(m) => write!(f, "unsupported: {m}"),
            TbError::Timeout(m) => write!(f, "timeout: {m}"),
            TbError::Policy(m) => write!(f, "policy: {m}"),
        }
    }
}

impl std::error::Error for TbError {}

impl From<std::io::Error> for TbError {
    fn from(e: std::io::Error) -> Self {
        TbError::Io(e.to_string())
    }
}

impl From<std::string::FromUtf8Error> for TbError {
    fn from(e: std::string::FromUtf8Error) -> Self {
        TbError::Io(format!("invalid utf-8: {e}"))
    }
}

impl From<std::net::AddrParseError> for TbError {
    fn from(e: std::net::AddrParseError) -> Self {
        TbError::Network(format!("invalid address: {e}"))
    }
}

/// crate-wide result alias.
pub type Result<T> = std::result::Result<T, TbError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_errors_are_recoverable_and_carry_phase() {
        let e = TbError::parse_at(Phase::Css, "unexpected token", 3, 7);
        assert!(e.is_recoverable());
        assert_eq!(e.phase(), Phase::Css);
        assert_eq!(e.to_string(), "css parse error at 3:7: unexpected token");
    }

    #[test]
    fn http_status_error_maps_to_network_phase() {
        let e = TbError::HttpStatus {
            status: 404,
            url: "https://taz.de/nope".into(),
        };
        assert_eq!(e.phase(), Phase::Network);
        assert!(!e.is_recoverable());
        assert_eq!(e.to_string(), "HTTP 404 for https://taz.de/nope");
    }

    #[test]
    fn io_error_converts() {
        let io = std::io::Error::new(std::io::ErrorKind::NotFound, "missing");
        let e: TbError = io.into();
        assert!(matches!(e, TbError::Io(_)));
    }
}
