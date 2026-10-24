//! The X11 protocol layer, tested byte by byte — no server needed.

use tb_ui::{Key, NamedKey};
use tb_x11::xproto::{
    change_property, change_property_atoms, connect_request, create_gc, create_window,
    get_keyboard_mapping, intern_atom, map_window, parse_error, parse_event, parse_setup, pixel,
    put_image, round_trip, set_input_focus, unmap_window, Setup, XEvent,
};
use tb_x11::{keysym_to_key, Keymap};

fn le32(v: u32) -> [u8; 4] {
    v.to_le_bytes()
}

/// A server reply, laid out the way a real Xorg one is (captured from `Xvfb`
/// 1280x800x24 and shrunk to one screen, one depth, one visual).
fn setup_reply(screens: u16) -> Vec<u8> {
    let mut b = Vec::new();
    // ---- 8-byte prefix
    b.extend_from_slice(&[1, 0]);
    b.extend_from_slice(&11u16.to_le_bytes());
    b.extend_from_slice(&0u16.to_le_bytes());
    b.extend_from_slice(&[0, 0]); // length of what follows, in 4-byte units
                                  // ---- 32-byte header
    b.extend_from_slice(&le32(12_009_104)); // release number
    b.extend_from_slice(&le32(0x0020_0000)); // resource id base
    b.extend_from_slice(&le32(0x001f_ffff)); // resource id mask
    b.extend_from_slice(&le32(256)); // motion buffer
    let vendor = b"TinyX";
    b.extend_from_slice(&(vendor.len() as u16).to_le_bytes());
    b.extend_from_slice(&[0xff, 0xff]); // padding the real server leaves here
    b.push(screens as u8); // number of screens
    b.push(if screens == 0 { 0 } else { 6 }); // number of pixmap formats
    b.extend_from_slice(&[0, 0]);
    b.extend_from_slice(&[32, 32, 8, 0xff]); // scanline unit, pad, bits per RGB
    b.extend_from_slice(&[0, 0, 0, 0]);
    // ---- the vendor name, padded
    b.extend_from_slice(vendor);
    while b.len() % 4 != 0 {
        b.push(0);
    }
    // ---- pixmap formats: eight bytes each on a real server
    for (depth, bpp) in [(1u8, 1u8), (4, 8), (8, 8), (16, 16), (24, 32), (32, 32)] {
        b.extend_from_slice(&[depth, bpp, 32]);
        b.extend_from_slice(&[0, 0, 0, 0, 0]);
    }
    // ---- the screen definition
    b.extend_from_slice(&le32(0x0000_039f)); // root window
    b.extend_from_slice(&le32(0x0000_0020)); // default colormap
    b.extend_from_slice(&le32(0xff_ffff)); // white pixel
    b.extend_from_slice(&le32(0)); // black pixel
    b.extend_from_slice(&le32(0)); // current input mask
    b.extend_from_slice(&1280u16.to_le_bytes());
    b.extend_from_slice(&800u16.to_le_bytes());
    b.extend_from_slice(&340u16.to_le_bytes());
    b.extend_from_slice(&203u16.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&le32(0x0000_0021)); // root visual
    b.extend_from_slice(&[1, 0, 24, 1]); // backing stores, save-unders, depth, depths
                                         // ---- one depth record, then the visual it contains
    b.extend_from_slice(&[24, 0, 1, 0, 0, 0, 0, 0]);
    b.extend_from_slice(&le32(0x0000_0021)); // visual id
    b.extend_from_slice(&[2, 8]); // class TrueColor, 8 bits per RGB value
    b.extend_from_slice(&0u16.to_le_bytes());
    b.extend_from_slice(&le32(0x00ff_0000));
    b.extend_from_slice(&le32(0x0000_ff00));
    b.extend_from_slice(&le32(0x0000_00ff));
    b.extend_from_slice(&[0, 0, 0, 0]);
    // a second screen, for the tests that ask for one
    if screens > 1 {
        let one = b.clone();
        let start = 96;
        let screen = &one[start..];
        for _ in 1..screens {
            b.extend_from_slice(screen);
        }
    }
    b
}

#[test]
fn setup_reply_is_understood() {
    let s: Setup = parse_setup(&setup_reply(1), 0).expect("setup parses");
    assert_eq!(s.major, 11);
    assert_eq!(s.vendor, "TinyX");
    assert_eq!(s.resource_base, 0x0020_0000);
    assert_eq!(s.resource_mask, 0x001f_ffff);
    assert!(s.image_lsb_first);
    assert_eq!(s.root, 0x39f);
    assert_eq!(s.default_colormap, 0x20);
    assert_eq!(s.root_visual, 0x21);
    assert_eq!((s.width, s.height), (1280, 800));
    assert_eq!(s.red_mask, 0x00ff_0000);
    assert_eq!(s.green_mask, 0x0000_ff00);
    assert_eq!(s.blue_mask, 0x0000_00ff);
    assert_eq!(s.bits_per_pixel, 32);
}

#[test]
fn setup_rejects_a_screen_that_is_not_there() {
    let b = setup_reply(1);
    assert!(parse_setup(&b, 1).is_err());
}

#[test]
fn setup_reports_a_refusal() {
    let mut b = setup_reply(1);
    b[0] = 0; // failed
    assert!(parse_setup(&b, 0).is_err());
}

#[test]
fn hello_carries_the_cookie() {
    let r = connect_request(true, "MIT-MAGIC-COOKIE-1", &[1, 2, 3]);
    assert_eq!(r[0], b'l');
    assert_eq!(&r[6..8], &18u16.to_le_bytes()); // name length
    assert_eq!(&r[8..10], &3u16.to_le_bytes()); // data length
    assert_eq!(&r[12..30], b"MIT-MAGIC-COOKIE-1");
    // both fields are padded to 4-byte units
    assert_eq!(r.len() % 4, 0);
    assert!(r.windows(3).any(|w| w == [1, 2, 3]));
}

#[test]
fn requests_are_padded_and_length_prefixed() {
    let r = intern_atom("WM_PROTOCOLS", false);
    assert_eq!(r[0], 16);
    assert_eq!(r[1], 0);
    // the length field counts 4-byte units, and matches the request exactly
    let units = u16::from_le_bytes([r[2], r[3]]) as usize;
    assert_eq!(units * 4, r.len());
    assert_eq!(&r[4..6], &12u16.to_le_bytes()); // name length (16 bits)

    for r in [
        map_window(0x1a).into_iter().collect::<Vec<u8>>(),
        unmap_window(0x1a),
        round_trip(),
        create_gc(7, 0x1a),
        set_input_focus(0x1a, 0),
        get_keyboard_mapping(8, 240),
    ] {
        assert_eq!(r.len() % 4, 0, "requests must be padded: {r:?}");
    }
}

#[test]
fn create_window_carries_visual_colormap_and_events() {
    let r = create_window(0x40, 0x1a, 10, 20, 800, 600, 0x21, 0x20, 0, 0x84_0047);
    assert_eq!(r[0], 1);
    assert_eq!(u16::from_le_bytes([r[2], r[3]]) * 4, r.len() as u16);
    assert_eq!(&r[4..8], &le32(0x40)); // window id
    assert_eq!(&r[8..12], &le32(0x1a)); // parent
    assert_eq!(i16::from_le_bytes([r[12], r[13]]), 10);
    assert_eq!(i16::from_le_bytes([r[14], r[15]]), 20);
    assert_eq!(u16::from_le_bytes([r[16], r[17]]), 800);
    assert_eq!(u16::from_le_bytes([r[18], r[19]]), 600);
    assert_eq!(u16::from_le_bytes([r[22], r[23]]), 1); // InputOutput
    assert_eq!(&r[24..28], &le32(0x21)); // visual
    assert_eq!(
        &r[28..32],
        &le32((1 << 0) | (1 << 11) | (1 << 13)) // background, event mask, colormap
    ); // value mask
    assert_eq!(&r[32..36], &le32(0)); // background pixel
    assert_eq!(&r[36..40], &le32(0x84_0047)); // event mask
    assert_eq!(&r[40..44], &le32(0x20)); // colormap
}

#[test]
fn properties_carry_the_right_unit_count() {
    let r = change_property(0x1a, 3, 4, 8, b"hello\0\0\0");
    assert_eq!(r[0], 18);
    assert_eq!(r[1], 0, "mode Replace");
    assert_eq!(r[16], 8, "format 8");
    assert_eq!(u32::from_le_bytes([r[20], r[21], r[22], r[23]]), 8); // bytes
    assert_eq!(&r[24..29], b"hello");

    let a = change_property_atoms(0x1a, 1, 1, &[0x123, 0x456]);
    assert_eq!(a[16], 32); // format
    assert_eq!(u32::from_le_bytes([a[20], a[21], a[22], a[23]]), 2); // atoms
    assert_eq!(&a[24..28], &le32(0x123));
}

#[test]
fn put_image_header_is_right() {
    let data = vec![0u8; 4 * 10]; // 10 pixels
    let r = put_image(0x1a, 7, 0, 12, 5, 2, 24, &data, true);
    assert_eq!(r[0], 72);
    assert_eq!(r[1], 2); // ZPixmap
    assert_eq!(u16::from_le_bytes([r[2], r[3]]) * 4, r.len() as u16);
    assert_eq!(u16::from_le_bytes([r[12], r[13]]), 5); // width
    assert_eq!(u16::from_le_bytes([r[14], r[15]]), 2); // height
    assert_eq!(u16::from_le_bytes([r[16], r[17]]), 0); // dst x
    assert_eq!(u16::from_le_bytes([r[18], r[19]]), 12); // dst y
    assert_eq!(r[20], 0); // left pad
    assert_eq!(r[21], 24, "the drawable's depth, which the server checks");
    assert_eq!(&r[22..24], &[0, 0], "the header is 24 bytes");
    assert_eq!(r[24..24 + data.len()], data[..]);
}

#[test]
fn put_image_can_swap_for_a_big_endian_server() {
    let data = vec![1u8, 2, 3, 4];
    let r = put_image(0x1a, 7, 0, 0, 1, 1, 24, &data, false);
    // the pixel is sent in the server's byte order, so the four bytes flip
    assert_eq!(&r[24..28], &[4, 3, 2, 1]);
}

#[test]
fn colours_pack_through_the_masks() {
    assert_eq!(
        pixel(0x12, 0x34, 0x56, (0xff0000, 0xff00, 0xff)),
        0x0012_3456
    );
    // a BGR visual (some ARM displays) puts the fields the other way round
    assert_eq!(
        pixel(0x12, 0x34, 0x56, (0xff, 0xff00, 0xff0000)),
        0x0056_3412
    );
    // 5-6-5
    assert_eq!(
        pixel(0xf8, 0xfc, 0xf8, (0xf800, 0x7e0, 0x1f)),
        0xf800 | 0x7e0 | 0x1f
    );
}

// ------------------------------------------------------------------ events ----

fn ev(bytes: &mut [u8; 32]) -> XEvent {
    parse_event(bytes, 0xaa, 0xbb)
}

#[test]
fn key_events() {
    let mut b = [0u8; 32];
    b[0] = 2; // KeyPress
    b[1] = 38; // 'a' on a PC layout
    b[28..30].copy_from_slice(&4u16.to_le_bytes()); // ControlMask
    assert_eq!(
        ev(&mut b),
        XEvent::Key {
            press: true,
            keycode: 38,
            state: 4
        }
    );

    let mut b = [0u8; 32];
    b[0] = 3; // KeyRelease
    b[1] = 38;
    assert_eq!(
        ev(&mut b),
        XEvent::Key {
            press: false,
            keycode: 38,
            state: 0
        }
    );
}

#[test]
fn button_and_motion_events_carry_window_coordinates() {
    let mut b = [0u8; 32];
    b[0] = 4; // ButtonPress
    b[1] = 1; // left
    b[24..26].copy_from_slice(&(-3i16).to_le_bytes());
    b[26..28].copy_from_slice(&612i16.to_le_bytes());
    b[28..30].copy_from_slice(&8u16.to_le_bytes()); // Mod1 = Alt
    assert_eq!(
        ev(&mut b),
        XEvent::Button {
            press: true,
            button: 1,
            x: -3,
            y: 612,
            state: 8,
            time: 0
        }
    );

    let mut b = [0u8; 32];
    b[0] = 6; // MotionNotify
    b[24..26].copy_from_slice(&400i16.to_le_bytes());
    b[26..28].copy_from_slice(&20i16.to_le_bytes());
    assert_eq!(
        ev(&mut b),
        XEvent::Motion {
            x: 400,
            y: 20,
            state: 0
        }
    );
}

#[test]
fn configure_and_expose() {
    let mut b = [0u8; 32];
    b[0] = 22; // ConfigureNotify
    b[20..22].copy_from_slice(&1024u16.to_le_bytes());
    b[22..24].copy_from_slice(&768u16.to_le_bytes());
    assert_eq!(
        ev(&mut b),
        XEvent::Configure {
            width: 1024,
            height: 768
        }
    );

    let mut b = [0u8; 32];
    b[0] = 12; // Expose
    assert_eq!(ev(&mut b), XEvent::Expose);
}

#[test]
fn the_delete_message_is_recognised_and_others_are_not() {
    let mut b = [0u8; 32];
    b[0] = 33; // ClientMessage
    b[1] = 32; // format
    b[8..12].copy_from_slice(&le32(0xaa)); // WM_PROTOCOLS
    b[12..16].copy_from_slice(&le32(0xbb)); // WM_DELETE_WINDOW
    assert_eq!(ev(&mut b), XEvent::ProtocolDelete);

    // the same message with a different type is not a request to close
    let mut other = b;
    other[8..12].copy_from_slice(&le32(0x99));
    assert_ne!(ev(&mut other), XEvent::ProtocolDelete);
}

#[test]
fn the_sent_event_bit_is_ignored() {
    let mut b = [0u8; 32];
    b[0] = 12 | 0x80; // an Expose the client sent itself
    assert_eq!(ev(&mut b), XEvent::Expose);
}

#[test]
fn errors_say_what_failed() {
    let mut b = [0u8; 32];
    b[0] = 0; // an error reply
    b[1] = 9; // BadDrawable
    b[2..4].copy_from_slice(&12u16.to_le_bytes()); // sequence
    b[4..8].copy_from_slice(&le32(0x1a)); // the resource it complained about
    b[8..10].copy_from_slice(&0u16.to_le_bytes()); // minor opcode
    b[10] = 72; // the request that did it (PutImage)
    let e = parse_error(&b);
    assert_eq!(e.code, 9);
    assert_eq!(e.request, 72);
    assert_eq!(e.bad_value, 0x1a);
    assert_eq!(e.sequence, 12);
    let text = e.describe();
    assert!(text.contains("BadDrawable"), "{text}");
    assert!(text.contains("PutImage"), "{text}");
}

// ------------------------------------------------------------------- keysyms ----

#[test]
fn keysyms_become_keys() {
    assert_eq!(keysym_to_key(0x41), Some(Key::Char('A')));
    assert_eq!(keysym_to_key(0xe9), Some(Key::Char('é'))); // Latin-1
    assert_eq!(keysym_to_key(0x0100_00e9), Some(Key::Char('é'))); // Unicode keysym
    assert_eq!(keysym_to_key(0xff0d), Some(Key::Named(NamedKey::Enter)));
    assert_eq!(keysym_to_key(0xff53), Some(Key::Named(NamedKey::Right)));
    assert_eq!(keysym_to_key(0xffbe), Some(Key::Named(NamedKey::F(1))));
    assert_eq!(keysym_to_key(0xffc9), Some(Key::Named(NamedKey::F(12))));
    assert_eq!(keysym_to_key(0xffb5), Some(Key::Char('5'))); // keypad
    assert_eq!(keysym_to_key(0xffe1), None, "Shift_L is a modifier");
    assert_eq!(keysym_to_key(0), None);
}

#[test]
fn keymap_uses_the_shift_level() {
    // key code 38: 'a' unshifted, 'A' shifted
    let km = Keymap::from_reply(38, 2, &[0x61, 0x41]);
    assert_eq!(km.key(38, false, false), Some(Key::Char('a')));
    assert_eq!(km.key(38, true, false), Some(Key::Char('A')));
    // an unknown key code has no key
    assert_eq!(km.key(9, false, false), None);
}

#[test]
fn keymap_honours_altgr_levels() {
    // a four-level key: a, A, æ, Æ
    let km = Keymap::from_reply(38, 4, &[0x61, 0x41, 0xe6, 0xc6]);
    assert_eq!(km.key(38, false, false), Some(Key::Char('a')));
    assert_eq!(km.key(38, true, false), Some(Key::Char('A')));
    assert_eq!(km.key(38, false, true), Some(Key::Char('æ')));
    assert_eq!(km.key(38, true, true), Some(Key::Char('Æ')));
}

#[test]
fn keymap_falls_back_when_a_level_is_empty() {
    // some layouts leave the shifted slot empty
    let km = Keymap::from_reply(38, 2, &[0x2f, 0]);
    assert_eq!(km.key(38, false, false), Some(Key::Char('/')));
}
