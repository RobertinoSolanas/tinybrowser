//! The X11 front end: `tiny --ui-backend x11`.
//!
//! Written from scratch, like the rest of this browser: the X11 protocol is a
//! stream of bytes, so the client is a socket, a request builder and a reply
//! parser. No Xlib, no wayland, no C bindings.
//!
//! It does what a browser window needs:
//!
//! * connects to `$DISPLAY` (unix socket or TCP), with the `$XAUTHORITY` cookie
//! * creates one `InputOutput` window and asks the window manager to close it
//!   properly (`WM_PROTOCOLS` / `WM_DELETE_WINDOW`)
//! * paints the app's framebuffer with `PutImage`, sending only the rows that
//!   changed, and repaints everything after an `Expose`
//! * turns key, button, motion, wheel and resize events into input
//! * asks the server for the keyboard map (`GetKeyboardMapping`), so the
//!   keyboard layout is the user's, not ours
//!
//! Not here: clipboard selections, XInput2 hi-res scrolling, XShm, XKB, RGBA
//! visual selection / compositing, multiple monitors. The internal clipboard
//! still works inside the browser.

pub mod auth;
pub mod keysyms;
pub mod setup;
pub mod xproto;

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use tb_gfx::Framebuffer;
use tb_ui::{App, Backend, BackendKind, Button, Event, Mods, PointerKind};

pub use auth::{parse as parse_xauthority, Entry as XauthEntry};
pub use keysyms::{keysym_to_key, Keymap};
pub use xproto::{parse_event, parse_setup, pixel, Setup, XError, XEvent};

/// Event mask for our window: keys, buttons, motion, exposure, structure.
///
/// The bit numbers are `X.h`'s; a made-up bit is silently ignored, which is a
/// quiet way of never receiving resize notifications at all.
const EVENT_MASK: u32 = (1 << 0) // KeyPressMask
    | (1 << 1) // KeyReleaseMask
    | (1 << 2) // ButtonPressMask
    | (1 << 3) // ButtonReleaseMask
    | (1 << 6) // PointerMotionMask
    | (1 << 15) // ExposureMask
    | (1 << 16) // VisibilityChangeMask
    | (1 << 17); // StructureNotifyMask

/// Something off the wire.
enum Item {
    Reply(Vec<u8>),
    Error(XError),
    Event([u8; 32]),
    Closed,
}

/// The socket, whichever way we reached the server.
enum Sock {
    Unix(std::os::unix::net::UnixStream),
    Tcp(std::net::TcpStream),
}

impl Sock {
    fn try_clone(&self) -> std::io::Result<Sock> {
        Ok(match self {
            Sock::Unix(s) => Sock::Unix(s.try_clone()?),
            Sock::Tcp(s) => Sock::Tcp(s.try_clone()?),
        })
    }
}

impl Read for Sock {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Sock::Unix(s) => s.read(buf),
            Sock::Tcp(s) => s.read(buf),
        }
    }
}

impl Write for Sock {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Sock::Unix(s) => s.write(buf),
            Sock::Tcp(s) => s.write(buf),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Sock::Unix(s) => s.flush(),
            Sock::Tcp(s) => s.flush(),
        }
    }
}

/// An X11 window showing the browser.
pub struct X11 {
    sock: Sock,
    rx: Receiver<Item>,
    /// Events we have already decoded but not handed to the app.
    events: VecDeque<Event>,
    /// Replies that arrived while we were waiting for an event.
    replies: VecDeque<Vec<u8>>,
    setup: Setup,
    next_id: u32,
    window: u32,
    gc: u32,
    width: u32,
    height: u32,
    title: String,
    keymap: Keymap,
    atoms: Atoms,
    /// The last frame, so the next one can be sent as a diff.
    last: Option<Framebuffer>,
    /// The window contents were lost (an `Expose`, a resize).
    damaged: bool,
    quit: bool,
}

/// The atoms we need by name.
#[derive(Debug, Clone, Copy, Default)]
struct Atoms {
    wm_protocols: u32,
    wm_delete: u32,
    wm_name: u32,
    net_wm_name: u32,
    utf8_string: u32,
    string: u32,
    wm_class: u32,
}

impl X11 {
    /// Open the display, map the window. `Err` says why in one line.
    pub fn open(width: u32, height: u32, title: &str) -> Result<X11, String> {
        let display = std::env::var("DISPLAY").unwrap_or_default();
        if display.is_empty() {
            return Err(
                "DISPLAY is not set; start an X server, use --ui-backend term, \
                        or --ui-backend headless"
                    .to_string(),
            );
        }
        let (host, number, screen) = parse_display(&display)?;
        let mut sock = connect_socket(&host, number)?;

        // The authorisation the server may or may not want.
        let entries = auth::from_env();
        let (name, data) = match auth::lookup(&entries, &display, number, &host) {
            Some(e) => (e.name.clone(), e.data.clone()),
            None => (String::new(), Vec::new()),
        };
        let hello = xproto::connect_request(true, &name, &data);
        sock.write_all(&hello)
            .map_err(|e| format!("cannot send the hello: {e}"))?;
        sock.flush()
            .map_err(|e| format!("cannot flush the hello: {e}"))?;

        // The reply arrives before any event, so it is safe to read it here —
        // the reader thread has not started yet.
        let mut header = [0u8; 8];
        sock.read_exact(&mut header)
            .map_err(|e| format!("cannot read the reply: {e}"))?;
        // The prefix is 8 bytes and its length counts everything after them —
        // reading more than that would eat the replies to requests we have not
        // sent yet, and the reader thread would never get them.
        let setup_len = u16::from_le_bytes([header[6], header[7]]) as usize * 4;
        let mut body = vec![0u8; setup_len];
        sock.read_exact(&mut body)
            .map_err(|e| format!("cannot read the setup: {e}"))?;
        match header[0] {
            0 => {
                let msg = String::from_utf8_lossy(&body[8..]).trim().to_string();
                return Err(format!("the X server refused the connection: {msg}"));
            }
            1 => {}
            2 => {
                return Err(
                    "the X server wants to authenticate us, which this client cannot do"
                        .to_string(),
                )
            }
            other => return Err(format!("unexpected reply from the X server: {other}")),
        }
        // `parse_setup` takes the whole reply, prefix included.
        let mut setup_bytes = Vec::with_capacity(8 + body.len());
        setup_bytes.extend_from_slice(&header);
        setup_bytes.extend_from_slice(&body);
        // `TINY_X11_DEBUG=1` dumps the setup bytes: the one thing you want
        // when the server's layout does not match what you expected.
        if std::env::var("TINY_X11_DEBUG").is_ok() {
            let n = setup_bytes.len().min(160);
            eprint!("x11 setup[{n}]:");
            for (i, b) in setup_bytes[..n].iter().enumerate() {
                if i % 16 == 0 {
                    eprint!("\n  {i:3}:");
                }
                eprint!(" {b:02x}");
            }
            eprintln!();
        }
        let setup = parse_setup(&setup_bytes, screen)?;

        // Now hand the socket to a reader thread: replies, errors and events all
        // arrive on the same stream and must be sorted.
        let (tx, rx) = mpsc::channel::<Item>();
        let reader = sock.try_clone().map_err(|e| e.to_string())?;
        std::thread::spawn(move || {
            let mut r = reader;
            loop {
                let mut head = [0u8; 32];
                if r.read_exact(&mut head).is_err() {
                    let _ = tx.send(Item::Closed);
                    return;
                }
                match head[0] {
                    0 => {
                        let _ = tx.send(Item::Error(xproto::parse_error(&head)));
                    }
                    1 => {
                        let extra =
                            u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as usize * 4;
                        let mut body = vec![0u8; extra];
                        if r.read_exact(&mut body).is_err() {
                            let _ = tx.send(Item::Closed);
                            return;
                        }
                        let mut msg = head.to_vec();
                        msg.extend_from_slice(&body);
                        let _ = tx.send(Item::Reply(msg));
                    }
                    _ => {
                        let _ = tx.send(Item::Event(head));
                    }
                }
            }
        });

        let mut x = X11 {
            sock,
            rx,
            events: VecDeque::new(),
            replies: VecDeque::new(),
            setup,
            next_id: 0,
            window: 0,
            gc: 0,
            width: width.max(320),
            height: height.max(200),
            title: String::new(),
            keymap: Keymap::default(),
            atoms: Atoms::default(),
            last: None,
            damaged: true,
            quit: false,
        };
        x.next_id = x.setup.resource_base;
        x.atoms = x.intern_atoms()?;
        x.create_window()?;
        x.fetch_keymap()?;
        x.set_title(title);
        Ok(x)
    }

    fn alloc_id(&mut self) -> u32 {
        // The server gave us a base and a mask of usable ids; wrap inside it.
        let span = self.setup.resource_mask.count_ones().max(1);
        let _ = span;
        self.next_id = self.next_id.wrapping_add(1) & self.setup.resource_mask;
        self.setup.resource_base | self.next_id
    }

    fn send(&mut self, req: &[u8]) -> Result<(), String> {
        if std::env::var("TINY_X11_DEBUG").is_ok() {
            let units = u16::from_le_bytes([req[2], req[3]]);
            let op = xproto::request_name(req[0]);
            let shown = req.len().min(24);
            eprintln!(
                "x11 → {:>3}{} ({units} units, {} bytes): {:02x?}",
                req[0],
                op,
                req.len(),
                &req[..shown]
            );
        }
        self.sock
            .write_all(req)
            .map_err(|e| format!("X write failed: {e}"))?;
        self.sock.flush().map_err(|e| e.to_string())
    }

    /// Wait for the reply to the request we just sent, parking any events that
    /// turn up first.
    fn wait_reply(&mut self) -> Result<Vec<u8>, String> {
        if let Some(r) = self.replies.pop_front() {
            return Ok(r);
        }
        loop {
            match self.rx.recv() {
                Ok(Item::Reply(bytes)) => return Ok(bytes),
                Ok(Item::Error(e)) => return Err(e.describe()),
                Ok(Item::Event(ev)) => {
                    self.translate(&ev);
                }
                Ok(Item::Closed) => return Err("the X server closed the connection".into()),
                Err(_) => return Err("the reader thread is gone".into()),
            }
        }
    }

    fn intern_atoms(&mut self) -> Result<Atoms, String> {
        let mut atoms = Atoms::default();
        for (name, field) in [
            ("WM_PROTOCOLS", 0),
            ("WM_DELETE_WINDOW", 1),
            ("WM_NAME", 2),
            ("_NET_WM_NAME", 3),
            ("UTF8_STRING", 4),
            ("STRING", 5),
            ("WM_CLASS", 6),
        ] {
            self.send(&xproto::intern_atom(name, false))?;
            let reply = self.wait_reply()?;
            let atom = u32::from_le_bytes([reply[8], reply[9], reply[10], reply[11]]);
            match field {
                0 => atoms.wm_protocols = atom,
                1 => atoms.wm_delete = atom,
                2 => atoms.wm_name = atom,
                3 => atoms.net_wm_name = atom,
                4 => atoms.utf8_string = atom,
                5 => atoms.string = atom,
                _ => atoms.wm_class = atom,
            }
        }
        Ok(atoms)
    }

    fn create_window(&mut self) -> Result<(), String> {
        self.window = self.alloc_id();
        self.gc = self.alloc_id();
        let bg = self.setup.black;
        self.send(&xproto::create_window(
            self.window,
            self.setup.root,
            50,
            50,
            self.width as u16,
            self.height as u16,
            self.setup.root_visual,
            self.setup.default_colormap,
            bg,
            EVENT_MASK,
        ))?;
        self.send(&xproto::create_gc(self.gc, self.window))?;
        // Ask to be told when the window manager wants us gone.
        self.send(&xproto::change_property_atoms(
            self.window,
            self.atoms.wm_protocols,
            self.atoms.wm_protocols,
            &[self.atoms.wm_delete],
        ))?;
        self.send(&xproto::map_window(self.window))?;
        self.send(&xproto::set_input_focus(self.window, 0))?;
        self.send(&xproto::round_trip())?;
        // The Sync reply means everything before it has been processed.
        self.wait_reply()?;
        Ok(())
    }

    fn fetch_keymap(&mut self) -> Result<(), String> {
        // Key codes run 8…255.
        self.send(&xproto::get_keyboard_mapping(8, 248 - 8))?;
        let reply = self.wait_reply()?;
        let per = reply[1] as usize;
        let syms: Vec<u32> = reply[32..]
            .chunks(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        self.keymap = Keymap::from_reply(8, per, &syms);
        Ok(())
    }

    // ------------------------------------------------------------- painting ----

    /// Encode rows `r0..r1` of the framebuffer into Z-pixmap bytes.
    fn encode(&self, fb: &Framebuffer, r0: usize, r1: usize) -> Vec<u8> {
        let masks = (
            self.setup.red_mask,
            self.setup.green_mask,
            self.setup.blue_mask,
        );
        let bpp = (self.setup.bits_per_pixel / 8).max(1) as usize;
        let mut out = Vec::with_capacity((r1 - r0) * fb.width as usize * bpp);
        for y in r0..r1 {
            for x in 0..fb.width as usize {
                let c = fb.pixels[y * fb.width as usize + x];
                let v = pixel(c.r, c.g, c.b, masks);
                out.extend_from_slice(&v.to_le_bytes()[..bpp]);
            }
        }
        out
    }

    /// How many rows fit in one `PutImage` request.
    fn rows_per_request(&self, fb: &Framebuffer) -> usize {
        // The setup says how long a request may be, in 4-byte units; a server
        // that says "0" is lying, so 64 KB of pixels at a time is the floor.
        let units = self.setup.max_request_length;
        let max_bytes = if units > 1 && units <= 0x0010_0000 {
            ((units as usize + 1) * 4).saturating_sub(64)
        } else {
            64 * 1024
        };
        let bytes_per_row = (fb.width as usize).max(1) * ((self.setup.bits_per_pixel / 8) as usize);
        (max_bytes / bytes_per_row.max(1)).clamp(1, fb.height.max(1) as usize)
    }

    /// The bands of rows that differ from the last frame.
    fn dirty_bands(&self, fb: &Framebuffer) -> Vec<(usize, usize)> {
        let prev = match &self.last {
            Some(p) if p.width == fb.width && p.height == fb.height => p,
            _ => return vec![(0, fb.height as usize)],
        };
        let w = fb.width as usize;
        let mut bands = Vec::new();
        let mut start: Option<usize> = None;
        for y in 0..fb.height as usize {
            let row = &fb.pixels[y * w..(y + 1) * w];
            let before = &prev.pixels[y * w..(y + 1) * w];
            if row != before {
                if start.is_none() {
                    start = Some(y);
                }
            } else if let Some(s) = start.take() {
                bands.push((s, y));
            }
        }
        if let Some(s) = start {
            bands.push((s, fb.height as usize));
        }
        bands
    }

    // ---------------------------------------------------------------- events ----

    fn mods(state: u16) -> Mods {
        Mods {
            ctrl: state & xproto::STATE_CTRL != 0,
            alt: state & xproto::STATE_MOD1 != 0,
            shift: state & xproto::STATE_SHIFT != 0,
            meta: state & xproto::STATE_MOD4 != 0,
        }
    }

    /// Decode one X event into the app's events.
    fn translate(&mut self, raw: &[u8; 32]) {
        let ev = parse_event(raw, self.atoms.wm_protocols, self.atoms.wm_delete);
        match ev {
            XEvent::Key {
                press,
                keycode,
                state,
            } => {
                if !press {
                    return; // the app acts on the press
                }
                let shift = state & xproto::STATE_SHIFT != 0;
                let Some(key) = self.keymap.key(keycode, shift, false) else {
                    return; // a modifier, or a key with no symbol
                };
                self.events.push_back(Event::Key {
                    key,
                    mods: Self::mods(state),
                });
            }
            XEvent::Button {
                press,
                button,
                x,
                y,
                state,
                ..
            } => {
                let mods = Self::mods(state);
                // The wheel is buttons 4/5 (vertical) and 6/7 (horizontal).
                if (4..=7).contains(&button) {
                    if !press {
                        return;
                    }
                    let (dx, dy) = match button {
                        4 => (0, -3),
                        5 => (0, 3),
                        6 => (-3, 0),
                        _ => (3, 0),
                    };
                    self.events.push_back(Event::Pointer {
                        kind: PointerKind::Wheel,
                        button: None,
                        x,
                        y,
                        dx,
                        dy,
                        mods,
                    });
                    return;
                }
                let which = match button {
                    1 => Button::Left,
                    2 => Button::Middle,
                    3 => Button::Right,
                    _ => Button::Left,
                };
                self.events.push_back(Event::Pointer {
                    kind: if press {
                        PointerKind::Press
                    } else {
                        PointerKind::Release
                    },
                    button: Some(which),
                    x,
                    y,
                    dx: 0,
                    dy: 0,
                    mods,
                });
            }
            XEvent::Motion { x, y, state } => {
                let mods = Self::mods(state);
                // A button held down makes it a drag, which is how text
                // selection arrives.
                // Button1..Button3 masks live at bits 8, 9 and 10
                let dragging = state & (0b111 << 8) != 0;
                self.events.push_back(Event::Pointer {
                    kind: if dragging {
                        PointerKind::Drag
                    } else {
                        PointerKind::Move
                    },
                    button: if dragging { Some(Button::Left) } else { None },
                    x,
                    y,
                    dx: 0,
                    dy: 0,
                    mods,
                });
            }
            XEvent::Expose => {
                // The server threw our pixels away.
                self.damaged = true;
            }
            XEvent::Configure { width, height } => {
                if width != self.width || height != self.height {
                    self.width = width;
                    self.height = height;
                    self.damaged = true;
                    self.events.push_back(Event::Resize {
                        width: width.max(1),
                        height: height.max(1),
                    });
                }
            }
            XEvent::ProtocolDelete => self.events.push_back(Event::Quit),
            XEvent::MappingNotify => {
                // The user changed the keyboard layout; ask again.
                let _ = self.fetch_keymap();
            }
            XEvent::Other(_) => {}
        }
    }
}

impl Backend for X11 {
    fn kind(&self) -> BackendKind {
        BackendKind::X11
    }

    fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn wants_pixels(&self) -> bool {
        true
    }

    fn present(&mut self, _screen: &tb_ui::Screen) -> std::io::Result<()> {
        // This backend asks for pixels (`wants_pixels`), so the shell never
        // calls this. A window that can show 24-bit colour has no use for a
        // cell grid it could not read anyway.
        Ok(())
    }

    fn present_frame(&mut self, fb: &Framebuffer) -> std::io::Result<()> {
        let bands = if self.damaged {
            vec![(0, fb.height as usize)]
        } else {
            self.dirty_bands(fb)
        };
        // A request may not exceed the server's maximum length (256 KB on a
        // default Xorg), so a full screen goes out as several `PutImage`s.
        let rows_per_request = self.rows_per_request(fb);
        for (r0, r1) in bands {
            let mut from = r0;
            while from < r1 {
                let to = (from + rows_per_request).min(r1);
                let data = self.encode(fb, from, to);
                let req = xproto::put_image(
                    self.window,
                    self.gc,
                    0,
                    from as u16,
                    fb.width as u16,
                    (to - from) as u16,
                    self.setup.depth,
                    &data,
                    self.setup.image_lsb_first,
                );
                if self.send(&req).is_err() {
                    // A lost connection is reported by the event loop.
                    return Ok(());
                }
                from = to;
            }
        }
        self.damaged = false;
        self.last = Some(fb.clone());
        Ok(())
    }

    fn next_event(&mut self, timeout_ms: u64) -> Option<Event> {
        if self.quit {
            return Some(Event::Quit);
        }
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        loop {
            if let Some(ev) = self.events.pop_front() {
                return Some(ev);
            }
            let left = deadline.saturating_duration_since(Instant::now());
            match self.rx.recv_timeout(left) {
                Ok(Item::Event(raw)) => self.translate(&raw),
                Ok(Item::Reply(r)) => self.replies.push_back(r),
                Ok(Item::Error(e)) => {
                    // Not fatal: one bad request does not end a session, but it
                    // must not be silent either.
                    eprintln!("tiny: {}", e.describe());
                }
                Ok(Item::Closed) => {
                    self.quit = true;
                    return Some(Event::Quit);
                }
                Err(_) => {
                    // Out of time. A Tick keeps the page's timers running.
                    return Some(Event::Tick);
                }
            }
        }
    }

    fn set_title(&mut self, title: &str) {
        if title == self.title {
            return;
        }
        self.title = title.to_string();
        let bytes = title.as_bytes();
        let mut padded = bytes.to_vec();
        while padded.len() % 4 != 0 {
            padded.push(0);
        }
        let _ = self.send(&xproto::change_property(
            self.window,
            self.atoms.net_wm_name,
            self.atoms.utf8_string,
            8,
            &padded,
        ));
        let _ = self.send(&xproto::change_property(
            self.window,
            self.atoms.wm_name,
            self.atoms.string,
            8,
            &padded,
        ));
    }

    fn shutdown(&mut self) {
        self.quit = true;
        // Unmap, then destroy: a window that just vanishes leaves the window
        // manager with a stale entry.
        let _ = self.send(&xproto::unmap_window(self.window));
        let _ = self.send(&xproto::destroy_window(self.window));
    }
}

/// `[host]:number.screen`
pub fn parse_display(d: &str) -> Result<(String, u8, usize), String> {
    let (host, rest) = match d.find(':') {
        Some(i) => (d[..i].to_string(), &d[i + 1..]),
        None => return Err(format!("cannot understand DISPLAY={d}")),
    };
    let (num, screen) = match rest.find('.') {
        Some(i) => (&rest[..i], &rest[i + 1..]),
        None => (rest, ""),
    };
    let number: u8 = num
        .parse()
        .map_err(|_| format!("bad display number in DISPLAY={d}"))?;
    let screen: usize = if screen.is_empty() {
        0
    } else {
        screen
            .parse()
            .map_err(|_| format!("bad screen number in DISPLAY={d}"))?
    };
    Ok((host, number, screen))
}

fn connect_socket(host: &str, number: u8) -> Result<Sock, String> {
    if host.is_empty() {
        let path = format!("/tmp/.X11-unix/X{number}");
        std::os::unix::net::UnixStream::connect(&path)
            .map(Sock::Unix)
            .map_err(|e| format!("cannot connect to {path}: {e}"))
    } else {
        let addr = format!("{host}:{}", 6000 + number as u16);
        std::net::TcpStream::connect(&addr)
            .map(Sock::Tcp)
            .map_err(|e| format!("cannot connect to {addr}: {e}"))
    }
}

/// Open the browser in an X11 window.
pub fn browse(app: &mut App) -> std::io::Result<()> {
    let mut x = X11::open(app.width, app.height, &app.title()).map_err(std::io::Error::other)?;
    tb_ui::run(app, &mut x)
}
