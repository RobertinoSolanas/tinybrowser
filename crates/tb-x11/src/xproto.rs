//! Just enough of the X11 wire protocol to draw a window and hear the user.
//!
//! Everything here is pure: bytes in, bytes out, no socket. That means the
//! protocol part of the back end can be tested without an X server, which is
//! more than can be said for the parts that need one.
//!
//! The protocol reference is *X Protocol Reference Manual, Volume 1* (the X
//! Consortium / Open Group version, protocol 11.0). Request numbers, field
//! offsets and event types below are from it.

/// A request the client sends; the vector is the whole request.
pub type Request = Vec<u8>;

/// Little-endian writer, which is what the X wire uses when we say so.
#[derive(Default)]
pub struct Writer {
    pub bytes: Vec<u8>,
}

impl Writer {
    pub fn new(op: u8) -> Writer {
        let mut w = Writer { bytes: Vec::new() };
        w.u8(op);
        w
    }

    pub fn u8(&mut self, v: u8) -> &mut Writer {
        self.bytes.push(v);
        self
    }

    pub fn u16(&mut self, v: u16) -> &mut Writer {
        self.bytes.extend_from_slice(&v.to_le_bytes());
        self
    }

    pub fn u32(&mut self, v: u32) -> &mut Writer {
        self.bytes.extend_from_slice(&v.to_le_bytes());
        self
    }

    pub fn i16(&mut self, v: i16) -> &mut Writer {
        self.bytes.extend_from_slice(&v.to_le_bytes());
        self
    }

    pub fn raw(&mut self, v: &[u8]) -> &mut Writer {
        self.bytes.extend_from_slice(v);
        self
    }

    /// Pad to a multiple of four bytes, as every X request must be.
    pub fn pad(&mut self) -> &mut Writer {
        while self.bytes.len() % 4 != 0 {
            self.bytes.push(0);
        }
        self
    }

    /// Write the length field (in 4-byte units) at `at`, now that the request
    /// is complete.
    pub fn fix_length(&mut self, at: usize) -> &mut Writer {
        let units = (self.bytes.len() / 4) as u16;
        self.bytes[at..at + 2].copy_from_slice(&units.to_le_bytes());
        self
    }
}

pub fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

pub fn i16_at(b: &[u8], at: usize) -> i16 {
    i16::from_le_bytes([b[at], b[at + 1]])
}

pub fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

// ------------------------------------------------------------------ setup ----

/// What the server tells us about itself when we connect.
#[derive(Debug, Clone, PartialEq)]
pub struct Setup {
    pub major: u16,
    pub minor: u16,
    pub vendor: String,
    /// First resource id we may use.
    pub resource_base: u32,
    /// …and how many of them before wrapping.
    pub resource_mask: u32,
    pub max_request_length: u32,
    /// Byte order the server wants image data in (`false` = MSB first).
    pub image_lsb_first: bool,
    pub root: u32,
    pub default_colormap: u32,
    pub root_visual: u32,
    /// Pixel value of `whitePixel` / `blackPixel`.
    pub white: u32,
    pub black: u32,
    pub width: u16,
    pub height: u16,
    /// True-colour masks of the root visual, which is what we draw into.
    pub red_mask: u32,
    pub green_mask: u32,
    pub blue_mask: u32,
    pub depth: u8,
    pub bits_per_pixel: u8,
}

/// The connection reply is parsed in [`crate::setup`], because finding the
/// records in it is more than a couple of field reads.
pub use crate::setup::{bpp_of_depth, parse_setup};

/// The connection setup request.
pub fn connect_request(lsb: bool, auth_name: &str, auth_data: &[u8]) -> Vec<u8> {
    let mut w = Writer::default();
    w.u8(if lsb { b'l' } else { b'B' });
    w.u8(0);
    w.u16(11);
    w.u16(0);
    w.u16(auth_name.len() as u16);
    w.u16(auth_data.len() as u16);
    w.u16(0);
    w.raw(auth_name.as_bytes());
    w.pad();
    w.raw(auth_data);
    w.pad();
    w.bytes
}

// ---------------------------------------------------------------- requests ----

/// `InternAtom` (16).
pub fn intern_atom(name: &str, only_if_exists: bool) -> Request {
    let mut w = Writer::new(16);
    w.u8(if only_if_exists { 1 } else { 0 });
    w.u16(0); // length, filled in below
    w.u16(name.len() as u16); // `nbytes` is 16 bits, with 16 bits of pad
    w.u16(0);
    w.raw(name.as_bytes());
    w.pad();
    w.fix_length(2);
    w.bytes
}

/// Window attribute bits (`CW*` in the protocol): which of the values after the
/// mask are present. The bit numbers are the ones the server checks, not the
/// order the names come in — getting them wrong makes the server read the event
/// mask as a window gravity and complain about a bad value.
pub const CW_BACK_PIXEL: u32 = 1 << 0;
pub const CW_EVENT_MASK: u32 = 1 << 11;
pub const CW_COLORMAP: u32 = 1 << 13;

/// `CreateWindow` (1) — an `InputOutput` window with the events we want.
///
/// Ten arguments because the request has ten fields: an argument per field is
/// the clearest form of this particular function.
#[allow(clippy::too_many_arguments)]
pub fn create_window(
    wid: u32,
    parent: u32,
    x: i16,
    y: i16,
    width: u16,
    height: u16,
    visual: u32,
    colormap: u32,
    background: u32,
    event_mask: u32,
) -> Request {
    let mut w = Writer::new(1);
    w.u8(0); // depth: CopyFromParent
    w.u16(0); // request length, filled in at the end
    w.u32(wid);
    w.u32(parent);
    w.i16(x);
    w.i16(y);
    w.u16(width);
    w.u16(height);
    w.u16(0); // border width
    w.u16(1); // class: InputOutput
    w.u32(visual);
    // The value mask, then the values in order of their bit numbers:
    // background-pixel, event-mask, colormap.
    let mask: u32 = CW_BACK_PIXEL | CW_EVENT_MASK | CW_COLORMAP;
    w.u32(mask);
    w.u32(background);
    w.u32(event_mask);
    w.u32(colormap);
    w.fix_length(2);
    w.bytes
}

/// `MapWindow` (8).
pub fn map_window(wid: u32) -> Request {
    let mut w = Writer::new(8);
    w.u8(0);
    w.u16(2);
    w.u32(wid);
    w.bytes
}

/// `UnmapWindow` (10).
pub fn unmap_window(wid: u32) -> Request {
    let mut w = Writer::new(10);
    w.u8(0);
    w.u16(2);
    w.u32(wid);
    w.bytes
}

/// `DestroyWindow` (4).
pub fn destroy_window(wid: u32) -> Request {
    let mut w = Writer::new(4);
    w.u8(0);
    w.u16(2);
    w.u32(wid);
    w.bytes
}

/// `CreateGC` (55) with no attributes: it is only needed as an argument to
/// `PutImage`.
pub fn create_gc(gc: u32, drawable: u32) -> Request {
    let mut w = Writer::new(55);
    w.u8(0);
    w.u16(4);
    w.u32(gc);
    w.u32(drawable);
    w.u16(0);
    w.pad();
    w.bytes
}

/// `ChangeProperty` (18), mode Replace.
pub fn change_property(window: u32, property: u32, kind: u32, format: u8, items: &[u8]) -> Request {
    let mut w = Writer::new(18);
    w.u8(0); // mode: Replace
    w.u16(0); // length
    w.u32(window);
    w.u32(property);
    w.u32(kind);
    w.u8(format);
    w.u8(0);
    w.u16(0);
    let units = match format {
        8 => items.len(),
        16 => items.len() / 2,
        _ => items.len() / 4,
    };
    w.u32(units as u32);
    w.raw(items);
    w.pad();
    w.fix_length(2);
    w.bytes
}

/// `ChangeProperty` for a list of 32-bit atoms.
pub fn change_property_atoms(window: u32, property: u32, kind: u32, atoms: &[u32]) -> Request {
    let mut items = Vec::with_capacity(atoms.len() * 4);
    for a in atoms {
        items.extend_from_slice(&a.to_le_bytes());
    }
    let mut w = Writer::new(18);
    w.u8(0);
    w.u16(0);
    w.u32(window);
    w.u32(property);
    w.u32(kind);
    w.u8(32);
    w.u8(0);
    w.u16(0);
    w.u32(atoms.len() as u32);
    w.raw(&items);
    w.pad();
    w.fix_length(2);
    w.bytes
}

/// `SetInputFocus` (42), so the keys come to us.
pub fn set_input_focus(window: u32, time: u32) -> Request {
    let mut w = Writer::new(42);
    w.u8(0); // revert-to: None
    w.u16(3);
    w.u32(window);
    w.u32(time);
    w.bytes
}

/// A request that gets a reply, so the server has finished everything before it
/// by the time the reply arrives — that is what "flush and wait" needs.
///
/// `GetInputFocus` (43) does it in four bytes. `Sync` does not: this Xorg answers
/// opcode 120 with `BadRequest`, so we use the request that is actually there.
/// (Checked against a live server, not from a table.)
pub fn round_trip() -> Request {
    vec![43, 0, 1, 0]
}

/// `GetKeyboardMapping` (101).
pub fn get_keyboard_mapping(first: u8, count: u8) -> Request {
    let mut w = Writer::new(101);
    w.u8(first);
    w.u16(2);
    w.u8(count);
    w.u8(0);
    w.u16(0);
    w.bytes
}

/// `PutImage` (72) with a Z-pixmap: the whole image, or one band of it.
///
/// The header is 24 bytes and includes the drawable's `depth`: the server
/// compares it with the drawable and answers `BadMatch` when they differ, so
/// leaving it out is not an option.
#[allow(clippy::too_many_arguments)] // it mirrors the request's own fields
pub fn put_image(
    drawable: u32,
    gc: u32,
    x: u16,
    y: u16,
    width: u16,
    height: u16,
    depth: u8,
    data: &[u8],
    lsb_first: bool,
) -> Request {
    let mut w = Writer::new(72);
    w.u8(2); // format: ZPixmap
    w.u16(0); // length
    w.u32(drawable);
    w.u32(gc);
    w.u16(width);
    w.u16(height);
    w.u16(x);
    w.u16(y);
    w.u8(0); // left pad (always 0 for a Z-pixmap)
    w.u8(depth);
    w.u16(0); // pad — the data starts at byte 24
    if lsb_first {
        w.raw(data);
    } else {
        for chunk in data.chunks(4) {
            // swap each pixel to big-endian order
            let mut p = [0u8; 4];
            p[..chunk.len()].copy_from_slice(chunk);
            w.u32(u32::from_be_bytes(p));
        }
    }
    w.pad();
    w.fix_length(2);
    w.bytes
}

// ------------------------------------------------------------------ events ----

/// Event type numbers (`X.h`).
pub const EV_KEY_PRESS: u8 = 2;
pub const EV_KEY_RELEASE: u8 = 3;
pub const EV_BUTTON_PRESS: u8 = 4;
pub const EV_BUTTON_RELEASE: u8 = 5;
pub const EV_MOTION: u8 = 6;
pub const EV_EXPOSE: u8 = 12;
pub const EV_CONFIGURE: u8 = 22;
pub const EV_CLIENT_MESSAGE: u8 = 33;
pub const EV_MAPPING_NOTIFY: u8 = 34;

/// Key masks in a key or pointer event's `state`.
pub const STATE_SHIFT: u16 = 1 << 0;
pub const STATE_CTRL: u16 = 1 << 2;
pub const STATE_MOD1: u16 = 1 << 3; // Alt on every modern layout
pub const STATE_MOD4: u16 = 1 << 6; // Super

/// A parsed event, in the few shapes the back end cares about.
#[derive(Debug, Clone, PartialEq)]
pub enum XEvent {
    Key {
        press: bool,
        keycode: u8,
        state: u16,
    },
    Button {
        press: bool,
        button: u8,
        x: i32,
        y: i32,
        state: u16,
        time: u32,
    },
    Motion {
        x: i32,
        y: i32,
        state: u16,
    },
    Expose,
    Configure {
        width: u32,
        height: u32,
    },
    /// A `WM_PROTOCOLS` message (the window manager asking us to close).
    ProtocolDelete,
    MappingNotify,
    Other(u8),
}

/// Decode one 32-byte event.
pub fn parse_event(b: &[u8], wm_protocols: u32, wm_delete: u32) -> XEvent {
    let kind = b[0] & 0x7f; // bit 7 means "sent by the client", not a new type
    match kind {
        EV_KEY_PRESS | EV_KEY_RELEASE => XEvent::Key {
            press: kind == EV_KEY_PRESS,
            keycode: b[1],
            state: u16_at(b, 28),
        },
        EV_BUTTON_PRESS | EV_BUTTON_RELEASE => XEvent::Button {
            press: kind == EV_BUTTON_PRESS,
            button: b[1],
            x: i16_at(b, 24) as i32,
            y: i16_at(b, 26) as i32,
            state: u16_at(b, 28),
            time: u32_at(b, 4),
        },
        EV_MOTION => XEvent::Motion {
            x: i16_at(b, 24) as i32,
            y: i16_at(b, 26) as i32,
            state: u16_at(b, 28),
        },
        EV_EXPOSE => XEvent::Expose,
        EV_CONFIGURE => XEvent::Configure {
            width: u16_at(b, 20) as u32,
            height: u16_at(b, 22) as u32,
        },
        EV_CLIENT_MESSAGE => {
            let message_type = u32_at(b, 8);
            let first = u32_at(b, 12);
            if message_type == wm_protocols && first == wm_delete {
                XEvent::ProtocolDelete
            } else {
                XEvent::Other(kind)
            }
        }
        EV_MAPPING_NOTIFY => XEvent::MappingNotify,
        _ => XEvent::Other(kind),
    }
}

/// A server error reply (`type == 0`), for logging rather than for logic.
///
/// The layout is `code(1 at 1) sequence(2) bad-value(4) minor(2) major(1 at 10)`
/// — the major opcode is *not* where a reply's opcode is, and reading it from
/// offset 2 gives the sequence number instead, which is a confusing thing to see
/// in an error message.
#[derive(Debug, Clone, PartialEq)]
pub struct XError {
    pub code: u8,
    pub bad_value: u32,
    pub minor: u16,
    pub request: u8,
    pub sequence: u16,
}

impl XError {
    /// The one-line form the CLI prints.
    pub fn describe(&self) -> String {
        format!(
            "X error {} ({}) on request {}{} seq {}, bad id {:#x}",
            self.code,
            error_name(self.code),
            self.request,
            request_name(self.request),
            self.sequence,
            self.bad_value
        )
    }
}

/// Error names from the protocol.
pub fn error_name(code: u8) -> &'static str {
    match code {
        1 => "BadRequest",
        2 => "BadValue",
        3 => "BadWindow",
        4 => "BadPixmap",
        5 => "BadAtom",
        6 => "BadCursor",
        7 => "BadFont",
        8 => "BadMatch",
        9 => "BadDrawable",
        10 => "BadAccess",
        11 => "BadAlloc",
        12 => "BadColor",
        13 => "BadGC",
        14 => "BadIDChoice",
        15 => "BadName",
        16 => "BadLength",
        17 => "NotImplemented",
        _ => "unknown",
    }
}

/// The requests this client sends, by number.
/// The requests of the core protocol, by number (`Xproto.h`).
pub fn request_name(op: u8) -> String {
    // The numbers are the ones in `<X11/Xproto.h>`; the ones we send (1, 4, 8,
    // 10, 16, 18, 42, 55, 72, 101, 120) are the ones that matter, the rest are
    // here so an error message names what failed.
    let name = match op {
        1 => "CreateWindow",
        2 => "ChangeWindowAttributes",
        3 => "GetWindowAttributes",
        4 => "DestroyWindow",
        7 => "ReparentWindow",
        8 => "MapWindow",
        9 => "MapSubwindows",
        10 => "UnmapWindow",
        11 => "UnmapSubwindows",
        12 => "ConfigureWindow",
        13 => "CirculateWindow",
        14 => "GetGeometry",
        15 => "QueryTree",
        16 => "InternAtom",
        17 => "GetAtomName",
        18 => "ChangeProperty",
        19 => "DeleteProperty",
        20 => "GetProperty",
        21 => "ListProperties",
        22 => "SetSelectionOwner",
        24 => "ConvertSelection",
        25 => "SendEvent",
        26 => "GrabPointer",
        38 => "QueryPointer",
        40 => "TranslateCoords",
        41 => "WarpPointer",
        42 => "SetInputFocus",
        43 => "GetInputFocus",
        44 => "QueryKeymap",
        45 => "OpenFont",
        53 => "CreatePixmap",
        54 => "FreePixmap",
        55 => "CreateGC",
        56 => "ChangeGC",
        60 => "FreeGC",
        61 => "ClearArea",
        62 => "CopyArea",
        72 => "PutImage",
        73 => "GetImage",
        74 => "PolyText8",
        76 => "ImageText8",
        78 => "CreateColormap",
        98 => "QueryExtension",
        101 => "GetKeyboardMapping",
        120 => "Sync",
        127 => "NoOperation",
        _ => return String::new(),
    };
    format!(" ({name})")
}

/// Decode an error reply.
pub fn parse_error(b: &[u8]) -> XError {
    XError {
        code: b[1],
        bad_value: u32_at(b, 4),
        minor: u16_at(b, 8),
        request: b[10],
        sequence: u16_at(b, 2),
    }
}

/// Turn red/green/blue into a pixel value for the visual's masks.
///
/// The field width comes from the mask, so a 5-6-5 display gets its colours
/// truncated the way the server expects instead of a shifted-overlow mess.
pub fn pixel(r: u8, g: u8, b: u8, masks: (u32, u32, u32)) -> u32 {
    let (rm, gm, bm) = masks;
    (scale(r, rm) << mask_shift(rm))
        | (scale(g, gm) << mask_shift(gm))
        | (scale(b, bm) << mask_shift(bm))
}

/// Fit a byte-wide channel into a colour field of whatever width the mask says.
fn scale(v: u8, mask: u32) -> u32 {
    let bits = mask.count_ones();
    if bits == 0 {
        0
    } else if bits >= 8 {
        v as u32
    } else {
        (v as u32) >> (8 - bits)
    }
}

/// Where a colour field starts in a pixel.
pub fn mask_shift(mask: u32) -> u32 {
    if mask == 0 {
        0
    } else {
        mask.trailing_zeros()
    }
}
