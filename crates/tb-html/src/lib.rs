//! # tb-html — the `markup` context's parser
//!
//! A hand written HTML5 tokenizer plus tree builder producing
//! [`tb_dom::Dom`]. No external parser is involved.
//!
//! ```
//! let dom = tb_html::parse_document("<h1 id=t>Hello <em>world</em></h1>");
//! let h = dom.get_element_by_id("t").unwrap();
//! assert_eq!(dom.tag(h), Some("h1"));
//! assert_eq!(dom.text_content(h), "Hello world");
//! ```

pub mod entities;
pub mod tokenizer;
pub mod tree_builder;

pub use tokenizer::{RawKind, Token, Tokenizer};
pub use tree_builder::{parse_document, parse_fragment, TreeBuilder};
