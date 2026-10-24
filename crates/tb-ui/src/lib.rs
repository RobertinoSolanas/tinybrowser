//! # tb-ui — the browser shell a human drives
//!
//! Everything to the left of this crate knows about documents; this crate knows
//! about *people*. Tabs, an address bar, back and forward, a status line, a page
//! view that scrolls, links that react to a click, keys that do what the shortcut
//! list says, selection you can drag, find that highlights, hints you can type,
//! an inspector you can read — and the same state rendered either as a **cell
//! grid** (a terminal, or an X11 window drawn with our own bitmap font) or as
//! **real pixels** (our rasterizer painting the page, the chrome drawn around it).
//!
//! ```text
//!   Event ──▶ input ──▶ Command ──▶ App(run) ──▶ tb-browser ──▶ dirty
//!                                                      │
//!                            render::paint_screen ◀────┘   → Backend (term, x11, headless)
//!                            render::paint_frame  ───────▶ → Backend (x11: page pixels)
//! ```
//!
//! ## The pieces
//!
//! | module | what it is |
//! |---|---|
//! | [`event`] | device-independent input: keys, modifiers, pointer, resize, paste |
//! | [`keymap`] | `(mode, key, mods) → Command`; the help overlay prints this table |
//! | [`mode`] | who has the keyboard: page, address bar, find, hints, menu, … |
//! | [`chrome`] | the geometry of tab strip, toolbar, bookmark bar, status, scrollbar |
//! | [`editor`] | the one-line text editor of the address/find/command fields |
//! | [`tab`] | one document plus its view state (scroll, history, selection, caret) |
//! | [`app`] | the application state and navigation; [`App::run`] executes commands |
//! | [`input`] | events → commands, pointer → hit tests → DOM events |
//! | [`pagetext`] | the page as text with geometry (hit testing, find, cell rendering) |
//! | [`selection`] / [`search`] | drag/word/line selection, find in page |
//! | [`hints`] | labelled links for keyboard activation |
//! | [`devtools`] | the inspector drawer: Elements, Console, Problems, CSS, Layout, Info |
//! | [`screen`] | the cell grid every backend ultimately shows |
//! | [`render`] | `App` → `Screen`, and `App` → `Framebuffer` (page pixels + chrome) |
//! | [`backend`] | the display contract, the head-less test rig, the main loop |
//!
//! ## Driving it from a test
//!
//! ```
//! use tb_ui::{App, UiConfig, Theme, render};
//!
//! let app = App::new(
//!     UiConfig::default(),
//!     Theme::dark(),
//!     1200,
//!     640,
//!     &["data:text/html,<h1>Hello%20tiny</h1>".to_string()],
//! );
//! let screen = render::paint_screen(&app);
//! // the page is on the screen, and so is the chrome around it
//! assert!(screen.contains("Hello"), "{}", screen.dump());
//! assert!(screen.contains("data:text/html"), "{}", screen.dump());
//! ```
//!
//! Real interaction lives in `tests/ui.rs`: keys are built with [`event::Key`],
//! clicks with [`Event::press`], and assertions with [`screen::Screen::contains`]
//! — text is laid out in words, so assert on a word rather than a whole sentence.
//!
#![forbid(unsafe_code)]

pub mod app;
pub mod backend;
pub mod bookmarks;
pub mod chrome;
pub mod clipboard;
pub mod commands;
pub mod devtools;
pub mod editor;
pub mod event;
pub mod hints;
pub mod input;
pub mod keymap;
pub mod keys;
pub mod mode;
pub mod pagetext;
pub mod render;
pub mod screen;
pub mod search;
pub mod selection;
pub mod tab;
pub mod theme;

pub use app::{App, Target, UiConfig};
pub use backend::{run, Backend, BackendKind, Headless};
pub use bookmarks::{Bookmark, Bookmarks};
pub use chrome::{Chrome, Widget};
pub use clipboard::Clipboard;
pub use devtools::{Devtools, LineKind, Panel};
pub use editor::Editor;
pub use event::{Button, Event, Key, Mods, NamedKey, PointerKind, Shortcut};
pub use hints::{HintState, HintStep};
pub use keymap::{Command, SHORTCUTS};
pub use mode::{Caret, Mode};
pub use pagetext::{Entry, PageText};
pub use screen::{Cell, Screen, Style};
pub use search::Find;
pub use selection::Selection;
pub use tab::Tab;
pub use theme::{Theme, ThemeKind};
