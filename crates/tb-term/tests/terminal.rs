//! The terminal protocol, tested without a terminal.

use tb_term::{Decoded, Decoder, Painter};
use tb_ui::screen::{Cell, Screen, Style};
use tb_ui::{Button, Event, Key, Mods, NamedKey, PointerKind};

fn decode(bytes: &[&[u8]]) -> Decoded {
    let mut dec = Decoder::new();
    let mut out = Decoded::default();
    for chunk in bytes {
        dec.feed(chunk, &mut out);
    }
    out
}

fn keys(out: &Decoded) -> Vec<(Key, Mods)> {
    out.events
        .iter()
        .map(|e| match e {
            Event::Key { key, mods } => (*key, *mods),
            _ => panic!("not a key: {e:?}"),
        })
        .collect()
}

#[test]
fn plain_text_and_control_codes() {
    let out = decode(&[b"aB \r\t\x7f"]);
    assert_eq!(
        keys(&out),
        vec![
            (Key::Char('a'), Mods::none()),
            (Key::Char('B'), Mods::none()),
            (Key::Char(' '), Mods::none()),
            (Key::Named(NamedKey::Enter), Mods::none()),
            (Key::Named(NamedKey::Tab), Mods::none()),
            (Key::Named(NamedKey::Backspace), Mods::none()),
        ]
    );
}

#[test]
fn control_bytes_are_ctrl_letters() {
    let out = decode(&[&[0x03], &[0x18]]);
    let k = keys(&out);
    assert_eq!(k[0].0, Key::Char('c'));
    assert!(k[0].1.ctrl);
    assert_eq!(k[1].0, Key::Char('x'));
    assert!(k[1].1.ctrl);
}

#[test]
fn utf8_survives_a_split_read() {
    // "küß" in UTF-8, one byte per read: a naive decoder would drop the
    // characters that straddle a read boundary
    let bytes = "küß".as_bytes();
    let chunks: Vec<&[u8]> = bytes.chunks(1).collect();
    let out = decode(&chunks);
    let text: String = keys(&out)
        .iter()
        .filter_map(|(k, _)| match k {
            Key::Char(c) => Some(*c),
            _ => None,
        })
        .collect();
    assert_eq!(text, "küß");
}

#[test]
fn arrows_and_modified_arrows() {
    let out = decode(&[b"\x1b[A", b"\x1b[1;5C", b"\x1b[1;2D"]);
    let k = keys(&out);
    assert_eq!(k[0], (Key::Named(NamedKey::Up), Mods::none()));
    assert_eq!(k[1].0, Key::Named(NamedKey::Right));
    assert!(k[1].1.ctrl);
    assert_eq!(k[2].0, Key::Named(NamedKey::Left));
    assert!(k[2].1.shift);
}

#[test]
fn function_keys_and_navigation() {
    let out = decode(&[b"\x1b[3~\x1b[5~\x1b[H\x1bOF\x1bOP"]);
    let k = keys(&out);
    assert_eq!(
        k.iter().map(|(key, _)| *key).collect::<Vec<_>>(),
        vec![
            Key::Named(NamedKey::Delete),
            Key::Named(NamedKey::PageUp),
            Key::Named(NamedKey::Home),
            Key::Named(NamedKey::End),
            Key::Named(NamedKey::F(1)),
        ]
    );
}

#[test]
fn alt_is_escape_prefix_and_escape_is_escape() {
    let out = decode(&[b"\x1bd", b"\x1b\x1b"]);
    let k = keys(&out);
    assert_eq!(k[0].0, Key::Char('d'));
    assert!(k[0].1.alt);
    assert_eq!(k[1], (Key::Named(NamedKey::Escape), Mods::none()));
}

#[test]
fn shift_tab_is_tab_with_shift() {
    let out = decode(&[b"\x1b[Z"]);
    let k = keys(&out);
    assert_eq!(k[0].0, Key::Named(NamedKey::Tab));
    assert!(k[0].1.shift);
}

#[test]
fn mouse_click_release_and_drag() {
    let out = decode(&[b"\x1b[<0;41;12M", b"\x1b[<0;41;12m", b"\x1b[<32;44;12M"]);
    let ptr = |i: usize| match &out.events[i] {
        Event::Pointer {
            kind, button, x, y, ..
        } => (*kind, *button, *x, *y),
        other => panic!("not a pointer: {other:?}"),
    };
    assert_eq!(
        ptr(0),
        (
            PointerKind::Press,
            Some(Button::Left),
            40,
            11 // SGR coordinates are 1-based
        )
    );
    assert_eq!(ptr(1), (PointerKind::Release, Some(Button::Left), 40, 11));
    assert_eq!(
        ptr(2),
        (PointerKind::Drag, Some(Button::Left), 43, 11) // bit 32: a button is held
    );
}

#[test]
fn mouse_wheel_and_release_without_button() {
    let out = decode(&[b"\x1b[<64;10;5M", b"\x1b[<65;10;5M", b"\x1b[<35;7;7M"]);
    match &out.events[0] {
        Event::Pointer { kind, dy, .. } => {
            assert_eq!(*kind, PointerKind::Wheel);
            assert_eq!(*dy, -3);
        }
        other => panic!("{other:?}"),
    }
    match &out.events[1] {
        Event::Pointer { dy, .. } => assert_eq!(*dy, 3),
        other => panic!("{other:?}"),
    }
    match &out.events[2] {
        // bit 32 with "no button" (3) is a plain move
        Event::Pointer { kind, .. } => assert_eq!(*kind, PointerKind::Move),
        other => panic!("{other:?}"),
    }
}

#[test]
fn bracketed_paste_does_not_run_keys() {
    let out = decode(&[b"\x1b[200~", b"https://example.com", b"\x1b[201~"]);
    assert_eq!(out.events.len(), 1);
    assert_eq!(out.events[0], Event::Paste("https://example.com".into()));
}

#[test]
fn geometry_reply_sets_the_size() {
    let out = decode(&[b"\x1b[18t", b"\x1b[8;30;120t"]);
    assert_eq!(out.size, Some((120, 30)));
}

#[test]
fn kitty_query_reply_and_csi_u_keys() {
    let mut dec = Decoder::new();
    let mut out = Decoded::default();
    dec.feed(b"\x1b[?1u", &mut out);
    assert_eq!(out.kitty, Some(1));
    assert!(dec.kitty);
    // Ctrl+Shift+I as kitty reports it: codepoint 'I', mods 1+1(shift)+4(ctrl)
    dec.feed(b"\x1b[105;6u", &mut out);
    match out.events.last().unwrap() {
        Event::Key { key, mods } => {
            assert_eq!(*key, Key::Char('i'));
            assert!(mods.ctrl && mods.shift);
        }
        other => panic!("{other:?}"),
    }
    // a key *release* (event type 3) is dropped
    let before = out.events.len();
    dec.feed(b"\x1b[105;6;3u", &mut out);
    assert_eq!(out.events.len(), before);
}

#[test]
fn kitty_functional_keys() {
    let out = decode(&[b"\x1b[57377u"]); // F5 in kitty's functional block
    assert_eq!(keys(&out)[0].0, Key::Named(NamedKey::F(5)));
}

#[test]
fn unknown_sequences_are_ignored_not_mis_read() {
    // an OSC response must not leak into the keys
    let out = decode(&[b"\x1b]10;rgb:0000/0000/0000\x07", b"x"]);
    assert_eq!(keys(&out), vec![(Key::Char('x'), Mods::none())]);
}

// ------------------------------------------------------------------ painter ----

fn screen_with(text: &str) -> Screen {
    let mut s = Screen::new(20, 3);
    s.text(
        0,
        1,
        text,
        Style::new(tb_core::Color::rgb(10, 20, 30), tb_core::Color::WHITE),
    );
    s
}

#[test]
fn first_paint_writes_the_text() {
    let mut p = Painter::new();
    let mut out = String::new();
    p.paint(&screen_with("hello"), &mut out);
    assert!(out.contains("hello"), "{out}");
    assert!(out.contains("\x1b[2J"), "first frame clears: {out}");
    assert!(out.contains("38;2;10;20;30"), "colour is set: {out}");
}

#[test]
fn second_paint_writes_only_what_changed() {
    let mut p = Painter::new();
    let mut first = String::new();
    p.paint(&screen_with("hello"), &mut first);

    let mut changed = screen_with("helloy");
    changed.put(
        0,
        0,
        'T',
        Style::new(tb_core::Color::BLACK, tb_core::Color::WHITE),
    );
    let mut second = String::new();
    p.paint(&changed, &mut second);

    assert!(second.contains('T'));
    assert!(second.contains('y'));
    assert!(
        !second.contains("hello"),
        "unchanged prefix skipped: {second}"
    );
    // a row that did not change at all is skipped entirely
    assert!(
        second.len() < first.len(),
        "diff must be smaller than the full paint: {second} vs {first}"
    );
}

#[test]
fn a_resize_repaints_everything() {
    let mut p = Painter::new();
    let mut first = String::new();
    p.paint(&screen_with("hello"), &mut first);
    let mut bigger = Screen::new(40, 3);
    bigger.text(0, 0, "wide", Style::default());
    let mut second = String::new();
    p.paint(&bigger, &mut second);
    assert!(second.contains("\x1b[2J"));
    assert!(second.contains("wide"));
}

#[test]
fn the_cursor_is_positioned_or_hidden() {
    let mut s = Screen::new(10, 2);
    s.cursor = Some((3, 1));
    let mut out = String::new();
    tb_term::set_cursor(&mut out, s.cursor);
    assert_eq!(out, "\x1b[2;4H\x1b[?25h");
    let mut out = String::new();
    tb_term::set_cursor(&mut out, None);
    assert_eq!(out, "\x1b[?25l");
}

#[test]
fn titles_cannot_break_out_of_the_sequence() {
    let mut out = String::new();
    tb_term::set_title(&mut out, "evil\x07\x1b[2Jtitle");
    assert!(!out.contains("\x1b[2J"), "{out}");
    assert!(out.starts_with("\x1b]0;"), "{out}");
    assert!(out.ends_with('\u{7}'));
}

#[test]
fn cell_defaults_match_the_style_default() {
    assert_eq!(Cell::default().style, Style::default());
}
