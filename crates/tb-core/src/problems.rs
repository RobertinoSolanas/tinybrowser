//! Non-fatal diagnostics.
//!
//! A real browser never aborts on one broken CSS rule or one JS `SyntaxError`;
//! it records the problem and keeps going. `Problems` is that record and is
//! asserted on in the tests ("loading taz.de must not produce panics and only
//! *known* categories of problems").

use crate::error::Phase;
use std::fmt;

/// How bad is it?
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Informational (e.g. "script executed").
    Info,
    /// Something was skipped or repaired.
    Warning,
    /// A stage failed but the page is still usable.
    Error,
}

/// A single diagnostic.
#[derive(Debug, Clone, PartialEq)]
pub struct Problem {
    pub severity: Severity,
    pub phase: Phase,
    pub message: String,
    /// Optional context: url, selector, statement, ...
    pub context: String,
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{:?}/{}] {}", self.severity, self.phase, self.message)?;
        if !self.context.is_empty() {
            write!(f, " ({})", self.context)?;
        }
        Ok(())
    }
}

/// Ordered collection of diagnostics with a hard cap so that a hostile page
/// cannot blow up memory.
#[derive(Debug, Clone, Default)]
pub struct Problems {
    items: Vec<Problem>,
    dropped: usize,
    cap: usize,
}

impl Problems {
    pub fn new() -> Self {
        Problems {
            items: Vec::new(),
            dropped: 0,
            cap: 2000,
        }
    }

    fn push(&mut self, p: Problem) {
        if self.items.len() >= self.cap {
            self.dropped += 1;
            return;
        }
        self.items.push(p);
    }

    pub fn info(&mut self, phase: Phase, message: impl Into<String>) {
        self.push(Problem {
            severity: Severity::Info,
            phase,
            message: message.into(),
            context: String::new(),
        });
    }

    pub fn warn(&mut self, phase: Phase, message: impl Into<String>) {
        self.push(Problem {
            severity: Severity::Warning,
            phase,
            message: message.into(),
            context: String::new(),
        });
    }

    pub fn warn_ctx(
        &mut self,
        phase: Phase,
        message: impl Into<String>,
        context: impl Into<String>,
    ) {
        self.push(Problem {
            severity: Severity::Warning,
            phase,
            message: message.into(),
            context: context.into(),
        });
    }

    pub fn error(&mut self, phase: Phase, message: impl Into<String>) {
        self.push(Problem {
            severity: Severity::Error,
            phase,
            message: message.into(),
            context: String::new(),
        });
    }

    pub fn error_ctx(
        &mut self,
        phase: Phase,
        message: impl Into<String>,
        context: impl Into<String>,
    ) {
        self.push(Problem {
            severity: Severity::Error,
            phase,
            message: message.into(),
            context: context.into(),
        });
    }

    pub fn all(&self) -> &[Problem] {
        &self.items
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn dropped(&self) -> usize {
        self.dropped
    }

    pub fn count(&self, severity: Severity) -> usize {
        self.items.iter().filter(|p| p.severity == severity).count()
    }

    pub fn count_phase(&self, phase: Phase) -> usize {
        self.items.iter().filter(|p| p.phase == phase).count()
    }

    pub fn of_phase(&self, phase: Phase) -> Vec<&Problem> {
        self.items.iter().filter(|p| p.phase == phase).collect()
    }

    pub fn errors(&self) -> Vec<&Problem> {
        self.items
            .iter()
            .filter(|p| p.severity == Severity::Error)
            .collect()
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.dropped = 0;
    }

    /// Append everything from `other`.
    pub fn extend(&mut self, other: Problems) {
        for p in other.items {
            self.push(p);
        }
        self.dropped += other.dropped;
    }

    /// Multi-line human readable report.
    pub fn report(&self) -> String {
        let mut out = String::new();
        for p in &self.items {
            out.push_str(&p.to_string());
            out.push('\n');
        }
        if self.dropped > 0 {
            out.push_str(&format!(
                "... {} further problems suppressed\n",
                self.dropped
            ));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_and_counts() {
        let mut p = Problems::new();
        p.info(Phase::Script, "ran 3 scripts");
        p.warn(Phase::Css, "unknown at-rule @scsssope");
        p.error(Phase::Script, "SyntaxError");
        assert_eq!(p.len(), 3);
        assert_eq!(p.count(Severity::Error), 1);
        assert_eq!(p.count_phase(Phase::Css), 1);
        assert_eq!(p.errors()[0].message, "SyntaxError");
    }

    #[test]
    fn caps_and_counts_dropped() {
        let mut p = Problems::new();
        p.cap = 4;
        for i in 0..10 {
            p.warn(Phase::Html, format!("problem {i}"));
        }
        assert_eq!(p.len(), 4);
        assert_eq!(p.dropped(), 6);
        assert!(p.report().contains("further problems suppressed"));
    }

    #[test]
    fn display_includes_context() {
        let mut p = Problems::new();
        p.warn_ctx(Phase::Css, "bad value", "color: #zz");
        assert!(p.all()[0].to_string().contains("color: #zz"));
    }
}
