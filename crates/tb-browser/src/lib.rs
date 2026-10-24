//! # tb-browser — the application layer
//!
//! Everything to the left of this crate is a bounded context that knows nothing
//! about a browser: `tb_net` retrieves bytes, `tb_dom`/`tb_html` build a tree,
//! `tb_css`/`tb_style` decide what things look like, `tb_layout` where,
//! `tb_gfx` paints, `tb_js` runs scripts and `tb_dom_bindings` lets those
//! scripts touch the tree.
//!
//! `tb-browser` is the only place that is allowed to know all of them, and it
//! uses that privilege in exactly one way: it runs the pipeline
//!
//! ```text
//! fetch → parse → (subresources) → script → style → layout → paint
//! ```
//!
//! and hands back a [`Page`] that can answer questions about the result —
//! text, links, headings, DOM dump, box tree, screenshot, `eval`.
//!
//! ```no_run
//! use tb_browser::Engine;
//! let mut browser = Engine::new(Default::default());
//! let mut page = browser.load("https://example.com").unwrap();
//! println!("{} — {}", page.title(), page.stats().elements);
//! page.screenshot(std::path::Path::new("/tmp/page.png"), &Default::default()).unwrap();
//! ```
//!
//! Styling and layout run *after* the scripts on purpose: what a page looks
//! like is decided by the document the scripts left behind, not by the bytes
//! the server sent.

pub mod engine;
pub mod images;
pub mod page;
pub mod paint;
pub mod resources;

pub use engine::{assemble_sheets, paint_options, Engine, History, NavState};
pub use images::{load_images, ImageStore};
pub use page::{Heading, Link, Modifiers, Page, PageStats, Timings};
pub use paint::{paint, PaintOptions};
pub use resources::{file_path, url_for_input, Loaded, ResourceLoader, SharedLoader};
pub use tb_dom_bindings::EventInit;

/// Everything a page carries that is worth re-exporting: the embedder should
/// not have to reach into four crates to handle a navigation.
pub use tb_core::{Color, EngineConfig, Problem, Problems, Rect, Severity, Size};
pub use tb_css::StyleSheet;
pub use tb_dom::{Dom, NodeId};
pub use tb_dom_bindings::{Host, NavRequest, PageContext, ScriptReport};
pub use tb_gfx::Image;
pub use tb_layout::{IntrinsicSizes, LayoutDocument};
pub use tb_net::Url;
