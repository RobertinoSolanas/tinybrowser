//! # tb-css — the presentation context's stylesheet parser
//!
//! Hand written: value parser, selector parser, `@media` evaluation and a
//! recovery-oriented document parser. No external CSS library is involved.
//!
//! ```
//! let sheet = tb_css::parse_stylesheet("h1.title { color: #c00; margin: 0 auto }");
//! assert_eq!(sheet.style_rule_count(), 1);
//! assert_eq!(sheet.declaration_count(), 2);
//! ```

pub mod ast;
pub mod calc;
pub mod media;
pub mod parser;
pub mod selector;
pub mod value;

pub use ast::{Declaration, Rule, StyleRule, StyleSheet};
pub use media::{MediaQuery, Viewport};
pub use parser::{extract_url, parse_declarations, parse_rule, parse_stylesheet};
pub use selector::{parse_selector_list, ComplexSelector, Simple, Specificity};
pub use tb_core::ColorScheme;
pub use value::{parse_value, DeclarationValue, Unit, Value};
