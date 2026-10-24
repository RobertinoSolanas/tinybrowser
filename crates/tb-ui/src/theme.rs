//! The chrome palette.
//!
//! Colours here are *chrome* — tabs, bars, selection, menus. The page paints
//! itself from its own CSS; the only thing the theme says about the page is
//! which `prefers-color-scheme` the viewport reports, because that is how a
//! page decides to be dark in the first place.

use tb_core::{Color, ColorScheme};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ThemeKind {
    Light,
    Dark,
}

impl ThemeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ThemeKind::Light => "light",
            ThemeKind::Dark => "dark",
        }
    }

    pub fn parse(s: &str) -> Option<ThemeKind> {
        match s.to_ascii_lowercase().as_str() {
            "light" => Some(ThemeKind::Light),
            "dark" => Some(ThemeKind::Dark),
            _ => None,
        }
    }

    pub fn flipped(self) -> ThemeKind {
        match self {
            ThemeKind::Light => ThemeKind::Dark,
            ThemeKind::Dark => ThemeKind::Light,
        }
    }

    pub fn color_scheme(self) -> ColorScheme {
        match self {
            ThemeKind::Light => ColorScheme::Light,
            ThemeKind::Dark => ColorScheme::Dark,
        }
    }
}

/// Every colour the chrome needs, named by what it is for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    pub kind: ThemeKind,
    /// Page canvas behind everything (also the terminal's background).
    pub bg: Color,
    pub fg: Color,
    /// Secondary text (paths, dim labels).
    pub dim: Color,
    /// Text on the accent colour (buttons, active tab).
    pub accent: Color,
    pub accent_fg: Color,
    /// Links and other things you can act on.
    pub link: Color,
    pub selection_bg: Color,
    pub selection_fg: Color,
    pub bar_bg: Color,
    pub bar_fg: Color,
    pub tab_bg: Color,
    pub tab_fg: Color,
    pub tab_active_bg: Color,
    pub tab_active_fg: Color,
    /// Address bar (and the caret in it).
    pub url_bg: Color,
    pub url_fg: Color,
    pub caret: Color,
    pub border: Color,
    pub status_bg: Color,
    pub status_fg: Color,
    pub warn: Color,
    pub error: Color,
    pub ok: Color,
    pub menu_bg: Color,
    pub menu_fg: Color,
    pub menu_sel_bg: Color,
    pub menu_sel_fg: Color,
    /// Find-in-page: current match vs the other matches.
    pub find_bg: Color,
    pub find_current_bg: Color,
    /// The dim layer behind an overlay (help, menu).
    pub overlay: Color,
}

impl Theme {
    pub fn light() -> Theme {
        Theme {
            kind: ThemeKind::Light,
            bg: Color::rgb(0xff, 0xff, 0xff),
            fg: Color::rgb(0x20, 0x20, 0x20),
            dim: Color::rgb(0x77, 0x77, 0x77),
            accent: Color::rgb(0x1a, 0x73, 0xe8),
            accent_fg: Color::rgb(0xff, 0xff, 0xff),
            link: Color::rgb(0x0b, 0x4d, 0xa8),
            selection_bg: Color::rgb(0xac, 0xcf, 0xf7),
            selection_fg: Color::rgb(0x10, 0x10, 0x10),
            bar_bg: Color::rgb(0xde, 0xe1, 0xe6),
            bar_fg: Color::rgb(0x20, 0x20, 0x20),
            tab_bg: Color::rgb(0xc8, 0xcc, 0xd3),
            tab_fg: Color::rgb(0x33, 0x33, 0x33),
            tab_active_bg: Color::rgb(0xff, 0xff, 0xff),
            tab_active_fg: Color::rgb(0x11, 0x11, 0x11),
            url_bg: Color::rgb(0xff, 0xff, 0xff),
            url_fg: Color::rgb(0x11, 0x11, 0x11),
            caret: Color::rgb(0x20, 0x20, 0x20),
            border: Color::rgb(0x9a, 0xa0, 0xa8),
            status_bg: Color::rgb(0x2b, 0x2f, 0x36),
            status_fg: Color::rgb(0xe8, 0xea, 0xee),
            warn: Color::rgb(0xb0, 0x6a, 0x00),
            error: Color::rgb(0xb3, 0x26, 0x1e),
            ok: Color::rgb(0x1d, 0x7a, 0x33),
            menu_bg: Color::rgb(0xf6, 0xf6, 0xf6),
            menu_fg: Color::rgb(0x18, 0x18, 0x18),
            menu_sel_bg: Color::rgb(0x1a, 0x73, 0xe8),
            menu_sel_fg: Color::rgb(0xff, 0xff, 0xff),
            find_bg: Color::rgb(0xff, 0xe5, 0x66),
            find_current_bg: Color::rgb(0xff, 0x9c, 0x2b),
            overlay: Color::rgb(0x20, 0x24, 0x2a),
        }
    }

    pub fn dark() -> Theme {
        Theme {
            kind: ThemeKind::Dark,
            bg: Color::rgb(0x18, 0x1a, 0x1e),
            fg: Color::rgb(0xdc, 0xde, 0xe2),
            dim: Color::rgb(0x8a, 0x90, 0x99),
            accent: Color::rgb(0x8a, 0xb8, 0xff),
            accent_fg: Color::rgb(0x10, 0x14, 0x1a),
            link: Color::rgb(0x8a, 0xb8, 0xff),
            selection_bg: Color::rgb(0x2c, 0x52, 0x8a),
            selection_fg: Color::rgb(0xf0, 0xf2, 0xf5),
            bar_bg: Color::rgb(0x23, 0x26, 0x2c),
            bar_fg: Color::rgb(0xdc, 0xde, 0xe2),
            tab_bg: Color::rgb(0x1c, 0x1f, 0x24),
            tab_fg: Color::rgb(0xa8, 0xae, 0xb8),
            tab_active_bg: Color::rgb(0x33, 0x38, 0x41),
            tab_active_fg: Color::rgb(0xf2, 0xf4, 0xf7),
            url_bg: Color::rgb(0x14, 0x16, 0x1a),
            url_fg: Color::rgb(0xe6, 0xe8, 0xec),
            caret: Color::rgb(0xe6, 0xe8, 0xec),
            border: Color::rgb(0x4a, 0x50, 0x5a),
            status_bg: Color::rgb(0x10, 0x12, 0x15),
            status_fg: Color::rgb(0xc0, 0xc6, 0xd0),
            warn: Color::rgb(0xe0, 0xa8, 0x4a),
            error: Color::rgb(0xe8, 0x70, 0x66),
            ok: Color::rgb(0x6f, 0xc8, 0x82),
            menu_bg: Color::rgb(0x25, 0x28, 0x2e),
            menu_fg: Color::rgb(0xe4, 0xe6, 0xea),
            menu_sel_bg: Color::rgb(0x2c, 0x52, 0x8a),
            menu_sel_fg: Color::rgb(0xff, 0xff, 0xff),
            find_bg: Color::rgb(0x7a, 0x66, 0x1a),
            find_current_bg: Color::rgb(0xa8, 0x6a, 0x1e),
            overlay: Color::rgb(0x0c, 0x0e, 0x11),
        }
    }

    pub fn of(kind: ThemeKind) -> Theme {
        match kind {
            ThemeKind::Light => Theme::light(),
            ThemeKind::Dark => Theme::dark(),
        }
    }

    /// A frame around a popup or a focused field.
    pub fn frame(&self) -> Theme {
        Theme {
            border: self.accent,
            ..*self
        }
    }
}

impl Default for Theme {
    fn default() -> Theme {
        Theme::light()
    }
}
