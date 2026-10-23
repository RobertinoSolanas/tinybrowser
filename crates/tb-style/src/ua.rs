//! The user-agent stylesheet — written in our own CSS dialect and parsed by
//! our own CSS parser (dogfooding: if the CSS parser is broken, the whole
//! browser breaks, and the tests for it will say so).

use tb_css::StyleSheet;

/// The default styling every HTML element gets before author CSS applies.
pub const UA_CSS: &str = r#"
html { display: block; font-family: sans-serif; font-size: 16px; color: #000; background: #fff; }
head, style, script, link, meta, title, base, template, noscript { display: none; }
body { display: block; margin: 8px; }
p { display: block; margin: 1em 0; }
div, section, article, aside, header, footer, nav, main, address, figure, figcaption,
blockquote, details, summary, dialog, search, hgroup, fieldset, legend, form, hr, pre,
dl, dt, dd, ul, ol, menu, li, center { display: block; }
li { display: list-item; margin-left: 0; }
ul, ol { padding-left: 40px; margin: 1em 0; }
ul { list-style-type: disc; }
ol { list-style-type: decimal; }
dd { margin-left: 40px; }
h1 { display: block; font-size: 2em; font-weight: bold; margin: 0.67em 0; }
h2 { display: block; font-size: 1.5em; font-weight: bold; margin: 0.83em 0; }
h3 { display: block; font-size: 1.17em; font-weight: bold; margin: 1em 0; }
h4 { display: block; font-size: 1em; font-weight: bold; margin: 1.33em 0; }
h5 { display: block; font-size: 0.83em; font-weight: bold; margin: 1.67em 0; }
h6 { display: block; font-size: 0.67em; font-weight: bold; margin: 2.33em 0; }
b, strong { font-weight: bold; }
i, em, cite, dfn, var, address { font-style: italic; }
u, ins { text-decoration: underline; }
s, strike, del { text-decoration: line-through; }
mark { background: #ff0; color: #000; }
small { font-size: 0.83em; }
big { font-size: 1.17em; }
code, kbd, samp, tt, pre { font-family: monospace; }
pre { white-space: pre; margin: 1em 0; }
a { color: #00e; text-decoration: underline; cursor: pointer; }
a:link { color: #00e; }
a:visited { color: #551a8b; }
a:hover { color: #e00; }
a:active { color: #e00; }
table { display: table; border-collapse: separate; border-spacing: 2px; }
thead { display: table-header-group; }
tbody { display: table-row-group; }
tr { display: table-row; }
td, th { display: table-cell; padding: 1px; }
th { font-weight: bold; text-align: -internal-center; }
caption { display: block; text-align: center; }
br { display: inline; }
img, video, canvas, svg, embed, object, audio { display: inline-block; }
iframe { display: inline-block; width: 300px; height: 150px; border: 1px solid; }
hr { border-style: solid; border-width: 1px 0 0 0; margin: 0.5em 0; }
button, input, select, textarea { display: inline-block; font-family: sans-serif; }
button, input[type=button], input[type=submit], input[type=reset] {
  border: 2px outset #ddd; background: #efefef; padding: 1px 6px; text-align: center;
}
textarea { border: 1px solid #767676; padding: 2px; }
blockquote { margin: 1em 40px; }
sup { font-size: 0.75em; vertical-align: super; }
sub { font-size: 0.75em; vertical-align: sub; }
noscript { display: none; }
[hidden] { display: none; }
[align=center], .center { text-align: center; }
[align=right] { text-align: right; }
"#;

/// The parsed UA stylesheet (built once, shared).
pub fn ua_stylesheet() -> &'static StyleSheet {
    use std::sync::OnceLock;
    static SHEET: OnceLock<StyleSheet> = OnceLock::new();
    SHEET.get_or_init(|| {
        let mut sheet = tb_css::parse_stylesheet(UA_CSS);
        sheet.href = "ua:/stylesheet.css".to_string();
        sheet
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ua_sheet_parses_cleanly() {
        let sheet = ua_stylesheet();
        assert!(
            sheet.style_rule_count() > 30,
            "rules: {}",
            sheet.style_rule_count()
        );
        assert!(sheet.declaration_count() > 60);
        // nothing of the UA sheet may be silently dropped
        let dump = sheet.debug_dump();
        assert!(dump.contains("h1"), "{dump}");
        assert!(dump.contains("[hidden]"), "{dump}");
    }

    #[test]
    fn ua_sheet_is_shared() {
        let a = ua_stylesheet() as *const StyleSheet;
        let b = ua_stylesheet() as *const StyleSheet;
        assert_eq!(a, b);
    }
}
