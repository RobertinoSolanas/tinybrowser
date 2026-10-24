//! Parsing of the connection reply (`xConnSetupPrefix`, `xConnSetup`,
//! `xPixmapFormat`, `xWindowRoot`, `xDepth`, `xVisualType`).
//!
//! The field layout below is copied from `<X11/Xproto.h>`, whose sizes are
//! `sz_xConnSetupPrefix 8`, `sz_xConnSetup 32`, `sz_xPixmapFormat 8`,
//! `sz_xDepth 8`, `sz_xVisualType 24`, `xWindowRoot 40`. Both the format and
//! depth records are padded to eight bytes, which is easy to miss and produces
//! a screen definition that is not where the textbook offsets say it is.

use super::xproto::{u16_at, u32_at, Setup};

/// Bytes of the fixed prefix before `xConnSetup`.
const PREFIX: usize = 8;
/// Offsets inside `xConnSetup`.
const SETUP_RID_BASE: usize = PREFIX + 4;
const SETUP_RID_MASK: usize = PREFIX + 8;
const SETUP_VENDOR_LEN: usize = PREFIX + 16;
const SETUP_MAX_REQUEST: usize = PREFIX + 18;
const SETUP_NUM_ROOTS: usize = PREFIX + 20;
const SETUP_NUM_FORMATS: usize = PREFIX + 21;
const SETUP_IMAGE_ORDER: usize = PREFIX + 22;
/// Offsets inside `xWindowRoot`.
const ROOT_WINDOW: usize = 0;
const ROOT_COLORMAP: usize = 4;
const ROOT_WHITE: usize = 8;
const ROOT_BLACK: usize = 12;
const ROOT_WIDTH: usize = 20;
const ROOT_HEIGHT: usize = 22;
const ROOT_VISUAL: usize = 32;
const ROOT_DEPTH: usize = 38;
const ROOT_NUM_DEPTHS: usize = 39;

const SIZE_ROOT: usize = 40;
const SIZE_DEPTH: usize = 8;
const SIZE_VISUAL: usize = 24;
const SIZE_FORMAT: usize = 8;

/// Parse a connection reply. `screen` selects one of the screens a
/// `[host]:n.screen` display can name.
pub fn parse_setup(b: &[u8], screen: usize) -> Result<Setup, String> {
    if b.len() < PREFIX + 32 {
        return Err("connection reply is too short".into());
    }
    match b[0] {
        0 => {
            // `lengthReason` bytes of text follow the prefix
            let reason = b[8..].to_vec();
            let reason = String::from_utf8_lossy(&reason).trim().to_string();
            return Err(if reason.is_empty() {
                "the X server refused the connection".to_string()
            } else {
                format!("the X server refused the connection: {reason}")
            });
        }
        1 => {}
        2 => {
            return Err(
                "the X server wants to authenticate us, which this client cannot do".to_string(),
            )
        }
        other => return Err(format!("unexpected reply from the X server: {other}")),
    }

    let roots = b[SETUP_NUM_ROOTS] as usize;
    if screen >= roots {
        return Err(format!(
            "display asks for screen {screen}, the server has {roots}"
        ));
    }
    let mut s = Setup {
        major: u16_at(b, 2),
        minor: u16_at(b, 4),
        vendor: String::new(),
        resource_base: u32_at(b, SETUP_RID_BASE),
        resource_mask: u32_at(b, SETUP_RID_MASK),
        max_request_length: u16_at(b, SETUP_MAX_REQUEST) as u32,
        // `MSBFirst` is 1; every little-endian machine's server says 0
        image_lsb_first: b[SETUP_IMAGE_ORDER] == 0,
        root: 0,
        default_colormap: 0,
        root_visual: 0,
        white: 0,
        black: 0,
        width: 0,
        height: 0,
        red_mask: 0,
        green_mask: 0,
        blue_mask: 0,
        depth: 0,
        bits_per_pixel: 0,
    };

    // Vendor name, then the pixmap formats.
    let vendor_len = u16_at(b, SETUP_VENDOR_LEN) as usize;
    let mut at = PREFIX + 32;
    if at + vendor_len > b.len() {
        return Err("setup: truncated vendor name".into());
    }
    s.vendor = String::from_utf8_lossy(&b[at..at + vendor_len]).to_string();
    at += vendor_len.div_ceil(4) * 4;
    let formats = b[SETUP_NUM_FORMATS] as usize;
    if at + formats * SIZE_FORMAT > b.len() {
        return Err("setup: truncated pixmap formats".into());
    }
    let formats_at = at;
    let bpp_of = |depth: u8| -> Option<u8> {
        (0..formats).find_map(|i| {
            let o = formats_at + i * SIZE_FORMAT;
            (b[o] == depth).then(|| b[o + 1])
        })
    };
    at += formats * SIZE_FORMAT;

    // Walk the roots to the one we want, stepping over each one's depth and
    // visual records.
    for which in 0..=screen {
        if at + SIZE_ROOT > b.len() {
            return Err("setup: truncated screen definition".into());
        }
        let take = which == screen;
        let root_visual = u32_at(b, at + ROOT_VISUAL);
        let depth = b[at + ROOT_DEPTH];
        if take {
            s.root = u32_at(b, at + ROOT_WINDOW);
            s.default_colormap = u32_at(b, at + ROOT_COLORMAP);
            s.white = u32_at(b, at + ROOT_WHITE);
            s.black = u32_at(b, at + ROOT_BLACK);
            s.width = u16_at(b, at + ROOT_WIDTH);
            s.height = u16_at(b, at + ROOT_HEIGHT);
            s.root_visual = root_visual;
            s.depth = depth;
            // Bits per pixel normally comes from the format list; a server that
            // does not list our depth still means the Z-pixmap the protocol
            // fixes for it.
            s.bits_per_pixel = bpp_of(depth).unwrap_or_else(|| bpp_of_depth(depth));
        }
        let depths = b[at + ROOT_NUM_DEPTHS] as usize;
        let mut d = at + SIZE_ROOT;
        for _ in 0..depths {
            if d + SIZE_DEPTH > b.len() {
                return Err("setup: truncated depth record".into());
            }
            let visuals = u16_at(b, d + 2) as usize;
            let mut v = d + SIZE_DEPTH;
            for _ in 0..visuals {
                if v + SIZE_VISUAL > b.len() {
                    return Err("setup: truncated visual record".into());
                }
                if take && u32_at(b, v) == root_visual {
                    // VISUALTYPE: id(4) class(1) bits-per-rgb(1) entries(2)
                    // red(4) green(4) blue(4) pad(4)
                    s.red_mask = u32_at(b, v + 8);
                    s.green_mask = u32_at(b, v + 12);
                    s.blue_mask = u32_at(b, v + 16);
                }
                v += SIZE_VISUAL;
            }
            d = v;
        }
        at = d;
    }

    if s.red_mask == 0 && s.depth > 8 {
        // A visual list that did not name the root visual: the 24-bit layout
        // every modern server uses is the only sensible guess.
        s.red_mask = 0x00ff_0000;
        s.green_mask = 0x0000_ff00;
        s.blue_mask = 0x0000_00ff;
    }
    Ok(s)
}

/// Bits per pixel of a Z-pixmap of this depth.
pub fn bpp_of_depth(depth: u8) -> u8 {
    match depth {
        0..=8 => 8,
        9..=16 => 16,
        _ => 32,
    }
}
