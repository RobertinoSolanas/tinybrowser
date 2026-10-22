//! Tiny monotonic stopwatch used for timeouts and benchmarks.

use std::time::Instant;

#[derive(Debug, Clone)]
pub struct Stopwatch {
    start: Instant,
}

impl Default for Stopwatch {
    fn default() -> Self {
        Self::new()
    }
}

impl Stopwatch {
    pub fn new() -> Self {
        Stopwatch {
            start: Instant::now(),
        }
    }

    pub fn elapsed_ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    pub fn elapsed_micros(&self) -> u128 {
        self.start.elapsed().as_micros()
    }

    pub fn restart(&mut self) {
        self.start = Instant::now();
    }

    /// `true` once `ms` milliseconds have passed since start/restart.
    pub fn exceeded(&self, ms: u64) -> bool {
        self.elapsed_ms() >= ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measures_a_sleep() {
        let s = Stopwatch::new();
        std::thread::sleep(std::time::Duration::from_millis(25));
        assert!(s.elapsed_ms() >= 20);
        assert!(s.elapsed_micros() > 20_000);
    }

    #[test]
    fn exceeded_flag() {
        let s = Stopwatch::new();
        assert!(!s.exceeded(500));
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert!(s.exceeded(5));
    }
}
