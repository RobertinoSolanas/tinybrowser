//! The terminal front end: `tiny --ui-backend term`.
//!
//! It runs anywhere there is a terminal, which is to say everywhere — over ssh,
//! in tmux, inside GNU screen, in `xterm` on a machine with no window manager.
//! It is written from scratch like the rest of this browser: the terminal is
//! driven with escape sequences, and the only external program it uses is
//! [`stty`], to put the tty into raw mode and put it back.
//!
//! What it does:
//!
//! * raw mode (`stty raw -echo min 0 time 1`), restored on exit — even if the
//!   process is killed, because the settings are restored from `Drop`
//! * the alternate screen, so the browser leaves your shell history behind
//! * a diffing renderer: only the cells that changed are written
//! * mouse reporting (SGR, `1006`) with drag and wheel, so clicking, selecting
//!   text and scrolling all work
//! * bracketed paste, so pasting a URL does not run it
//! * the kitty keyboard protocol when the terminal has it (real `Ctrl+Shift+…`
//!   and functional keys), legacy decoding when it does not
//! * window title through OSC 0, clipboard through OSC 52
//! * geometry through `CSI 18 t`, so a resize of the window follows through to
//!   a re-layout
//!
//! What it does not do: show images. A cell grid is the display here, so page
//! pixels are not available — `--ui-backend x11` is the pixel path.

mod ansi;

use std::collections::VecDeque;
use std::io::{IsTerminal, Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, TryRecvError};
use std::time::{Duration, Instant};

use tb_ui::chrome::{CELL_H, CELL_W};
use tb_ui::{Backend, BackendKind, Event, Screen};

pub use ansi::{
    clipboard_set, enter_mode, key_from_byte, leave_mode, query_size, set_cursor, set_title,
    Decoded, Decoder, Painter,
};

/// The terminal as a browser display.
pub struct Term {
    cols: usize,
    rows: usize,
    painter: Painter,
    decoder: Decoder,
    /// Bytes read from stdin, from the reader thread.
    bytes: Receiver<Vec<u8>>,
    /// Decoded events waiting to be handed to the app.
    pending: VecDeque<Event>,
    out: std::io::Stdout,
    /// `stty -g`, so we can put the tty back exactly as it was.
    saved: Option<String>,
    title: String,
    last_query: Instant,
    /// `true` until the terminal has answered our geometry query once.
    sized: bool,
    quit: bool,
}

impl Term {
    /// Take over the terminal. Fails with a readable message when there is no
    /// terminal to take over (a pipe, a cron job, a test harness).
    pub fn new() -> Result<Term, String> {
        if !std::io::stdin().is_terminal() {
            return Err(
                "stdin is not a terminal; use --ui-backend headless (or a script) instead"
                    .to_string(),
            );
        }
        let saved = stty(&["-g"]);
        // `raw` alone: `min 1 time 0`, so a read blocks until a byte arrives.
        // `min 0 time 1` would make `read()` return *zero bytes* every hundred
        // milliseconds, and a zero-length read is how EOF is reported — the
        // reader below would quit before the user ever pressed a key.
        if stty(&["raw", "-echo"]).is_none() {
            return Err(
                "could not put the terminal into raw mode (is `stty` available?)".to_string(),
            );
        }

        let (tx, bytes) = mpsc::channel::<Vec<u8>>();
        std::thread::spawn(move || {
            let mut stdin = std::io::stdin();
            let mut buf = [0u8; 4096];
            let mut errors = 0;
            loop {
                match stdin.read(&mut buf) {
                    // The terminal is gone (the window closed, the shell died).
                    Ok(0) => {
                        let _ = tx.send(Vec::new());
                        break;
                    }
                    Ok(n) => {
                        errors = 0;
                        if tx.send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                    // An interrupted read (a resize, a signal) is not an error.
                    Err(_) => {
                        errors += 1;
                        if errors > 16 {
                            let _ = tx.send(Vec::new());
                            break;
                        }
                    }
                }
            }
        });

        let mut term = Term {
            cols: 80,
            rows: 24,
            painter: Painter::new(),
            decoder: Decoder::new(),
            bytes,
            pending: VecDeque::new(),
            out: std::io::stdout(),
            saved,
            title: String::new(),
            last_query: Instant::now(),
            sized: false,
            quit: false,
        };

        // A wrapper that knows the size (tmux, `script`, an editor's terminal
        // widget) says so in the environment; believe it before we ask.
        let env_size = || -> Option<(usize, usize)> {
            let c = std::env::var("COLUMNS").ok()?.parse::<usize>().ok()?;
            let r = std::env::var("LINES").ok()?.parse::<usize>().ok()?;
            Some((c, r))
        };
        if let Some((cols, rows)) = env_size() {
            if cols > 1 && rows > 1 {
                term.set_size(cols, rows);
                term.sized = true;
            }
        }

        let mut init = String::new();
        ansi::enter_mode(&mut init);
        term.write(&init)
            .map_err(|e| format!("cannot talk to the terminal: {e}"))?;

        // The replies to the geometry and kitty queries arrive straight away;
        // collect them before the first paint so the first frame is the right
        // size.
        for _ in 0..8 {
            if term.sized {
                break;
            }
            term.read(Some(60));
        }
        if !term.sized {
            term.fallback_size();
        }
        Ok(term)
    }

    /// Ask for the size again if it has been a while — a terminal tells us
    /// nothing until we ask.
    fn maybe_query_size(&mut self) {
        if self.last_query.elapsed() < Duration::from_millis(300) {
            return;
        }
        self.last_query = Instant::now();
        let mut q = String::new();
        ansi::query_size(&mut q);
        let _ = self.write(&q);
    }

    /// `stty size`, for terminals that do not answer `CSI 18 t`.
    fn fallback_size(&mut self) {
        if let Some(out) = stty(&["size"]) {
            let mut it = out.split_whitespace();
            if let (Some(rows), Some(cols)) = (
                it.next().and_then(|s| s.parse::<usize>().ok()),
                it.next().and_then(|s| s.parse::<usize>().ok()),
            ) {
                if cols > 1 && rows > 1 {
                    self.set_size(cols, rows);
                    self.sized = true;
                }
            }
        }
    }

    fn set_size(&mut self, cols: usize, rows: usize) {
        if cols == self.cols && rows == self.rows {
            return;
        }
        self.cols = cols;
        self.rows = rows;
        self.pending.push_back(Event::Resize {
            width: cols as u32 * CELL_W as u32,
            height: rows as u32 * CELL_H as u32,
        });
    }

    /// Wait for bytes, decode them, and queue whatever they turned into.
    fn read(&mut self, timeout: Option<u64>) {
        let chunk = match timeout {
            None => match self.bytes.try_recv() {
                Ok(c) => c,
                // `Empty` is nothing yet; `Disconnected` is a terminal that went
                // away while we were looking the other way.
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    self.quit = true;
                    return;
                }
            },
            Some(ms) => match self.bytes.recv_timeout(Duration::from_millis(ms)) {
                Ok(c) => c,
                Err(RecvTimeoutError::Timeout) => return,
                Err(RecvTimeoutError::Disconnected) => {
                    self.quit = true;
                    return;
                }
            },
        };
        if chunk.is_empty() {
            // the reader thread's EOF marker: hang up like the terminal did
            self.quit = true;
            return;
        }
        let mut decoded = Decoded::default();
        self.decoder.feed(&chunk, &mut decoded);
        for ev in decoded.events {
            self.pending.push_back(ev);
        }
        if let Some((cols, rows)) = decoded.size {
            self.sized = true;
            self.set_size(cols, rows);
        }
    }

    fn write(&mut self, s: &str) -> std::io::Result<()> {
        self.out.write_all(s.as_bytes())?;
        self.out.flush()
    }
}

impl Backend for Term {
    fn kind(&self) -> BackendKind {
        BackendKind::Term
    }

    fn size(&self) -> (u32, u32) {
        (
            self.cols as u32 * CELL_W as u32,
            self.rows as u32 * CELL_H as u32,
        )
    }

    fn present(&mut self, screen: &Screen) -> std::io::Result<()> {
        let mut buf = String::with_capacity(screen.cols * 8);
        self.painter.paint(screen, &mut buf);
        if let Some(cursor) = screen.cursor {
            ansi::set_cursor(&mut buf, Some(cursor));
        } else {
            ansi::set_cursor(&mut buf, None);
        }
        self.write(&buf)
    }

    fn next_event(&mut self, timeout_ms: u64) -> Option<Event> {
        if self.quit {
            return Some(Event::Quit);
        }
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        loop {
            if let Some(ev) = self.pending.pop_front() {
                return Some(ev);
            }
            self.maybe_query_size();
            // Never block longer than the caller asked us to.
            let left = deadline.saturating_duration_since(Instant::now());
            let wait = left.min(Duration::from_millis(250)).as_millis() as u64;
            self.read(Some(wait));
            if Instant::now() >= deadline {
                // The app pumps its timers on a Tick.
                return Some(Event::Tick);
            }
        }
    }

    fn set_title(&mut self, title: &str) {
        if title == self.title {
            return;
        }
        self.title = title.to_string();
        let mut buf = String::new();
        ansi::set_title(&mut buf, title);
        let _ = self.write(&buf);
    }

    fn store_clipboard(&mut self, text: &str) {
        let mut buf = String::new();
        ansi::clipboard_set(&mut buf, &tb_ui::clipboard::base64(text.as_bytes()));
        let _ = self.write(&buf);
    }

    fn shutdown(&mut self) {
        self.quit = true;
        let mut buf = String::new();
        ansi::leave_mode(&mut buf);
        let _ = self.write(&buf);
        if let Some(saved) = self.saved.take() {
            stty(&[&saved]);
        }
    }
}

impl Drop for Term {
    /// A browser that is killed must still give the shell back a working
    /// terminal.
    fn drop(&mut self) {
        if let Some(saved) = self.saved.take() {
            stty(&[&saved]);
        }
    }
}

/// Run `stty` against the inherited stdin (the tty) and return its output.
fn stty(args: &[&str]) -> Option<String> {
    let out = Command::new("stty")
        .args(args)
        .stdin(Stdio::inherit())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Open the browser in the terminal.
pub fn browse(app: &mut tb_ui::App) -> std::io::Result<()> {
    let mut term = Term::new().map_err(std::io::Error::other)?;
    tb_ui::run(app, &mut term)
}
