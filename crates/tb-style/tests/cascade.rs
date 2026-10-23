//! Cascade integration tests: specificity, origins, inheritance, shorthands,
//! media queries, custom properties, `::before`/`::after`.

use tb_html::parse_document;
use tb_style::*;

fn vp() -> Viewport {
    Viewport {
        width: 1000.0,
        height: 800.0,
        ..Default::default()
    }
}

/// Style `html` with `css` and return the computed style of the element with
/// `id=` (or the n-th element of tag `tag`).
fn style_one(html: &str, css: &str, find: Find) -> ComputedStyle {
    let dom = parse_document(html);
    let mut problems = tb_core::Problems::new();
    let sheets = resolve_sources(
        &[CssSource::Inline {
            text: css.to_string(),
        }],
        vp(),
        |_| None,
        &mut problems,
    );
    let out = StyleEngine::new(vp()).style_document(&dom, &sheets);
    let node = match find {
        Find::Id(id) => dom.get_element_by_id(id).expect("element with id"),
        Find::TagN(tag, n) => {
            let all = dom.get_elements_by_tag(tag);
            *all.get(n)
                .unwrap_or_else(|| panic!("no <{tag}> #{n} in {html}"))
        }
    };
    out.styles
        .get(node)
        .cloned()
        .unwrap_or_else(|| panic!("no computed style for {:?}", node))
}

enum Find<'a> {
    Id(&'a str),
    TagN(&'a str, usize),
}

#[test]
fn ua_stylesheet_provides_html_defaults() {
    let s = style_one(
        "<html><head></head><body><h1>T</h1><p>P</p><span>S</span><ul><li>x</li></ul></body></html>",
        "",
        Find::TagN("h1", 0),
    );
    assert_eq!(s.display, Display::Block);
    assert_eq!(s.font_size, 32.0, "h1 = 2em");
    assert_eq!(s.font_weight, 700);
    let p = style_one("<p>x</p>", "", Find::TagN("p", 0));
    assert_eq!(p.display, Display::Block);
    assert_eq!(p.margin[ComputedStyle::TOP], L::Px(16.0), "1em");
    let span = style_one("<span>x</span>", "", Find::TagN("span", 0));
    assert_eq!(span.display, Display::Inline);
    let li = style_one("<ul><li>x</li></ul>", "", Find::TagN("li", 0));
    assert_eq!(li.display, Display::ListItem);
    assert!(li.list_style_bullet);
    let hidden = style_one("<style>x</style>", "", Find::TagN("style", 0));
    assert_eq!(hidden.display, Display::None);
}

#[test]
fn specificity_decides() {
    let html = "<div id=a class=b><p class=c>d</p></div>";
    // element selector loses to class loses to id
    let s = style_one(
        html,
        "p { color: red } .c { color: green } div p { color: blue }",
        Find::TagN("p", 0),
    );
    assert_eq!(s.color, tb_core::Color::parse("green").unwrap());
    let s = style_one(
        html,
        ".b .c { color: green } #a p { color: blue }",
        Find::TagN("p", 0),
    );
    assert_eq!(s.color, tb_core::Color::parse("blue").unwrap());
    // id outranks class+type no matter the order
    let s = style_one(
        html,
        "div.a { color: black } #a { color: red }",
        Find::Id("a"),
    );
    assert_eq!(s.color, tb_core::Color::parse("red").unwrap());
    // equal specificity: later wins
    // type+class (0,1,1) beats class alone (0,1,0) even when written first
    let s = style_one(
        html,
        "div.b { color: red } .b { color: green }",
        Find::Id("a"),
    );
    assert_eq!(s.color, tb_core::Color::parse("red").unwrap());
    let s = style_one(
        html,
        ".b { color: red } div.b { color: black }",
        Find::Id("a"),
    );
    assert_eq!(s.color, tb_core::Color::parse("black").unwrap());
}

#[test]
fn important_and_style_attribute_origins() {
    let html = "<p id=x style=\"color: purple\">t</p>";
    // style attribute beats any selector
    let s = style_one(html, "#x { color: red }", Find::Id("x"));
    assert_eq!(s.color, tb_core::Color::parse("purple").unwrap());
    // !important author beats the style attribute
    let s = style_one(html, "p { color: red !important }", Find::Id("x"));
    assert_eq!(s.color, tb_core::Color::parse("red").unwrap());
    // but UA !important would beat author !important — we have none, so the
    // author sheet always wins over the UA sheet:
    let s = style_one(
        "<div>x</div>",
        "div { display: inline }",
        Find::TagN("div", 0),
    );
    assert_eq!(s.display, Display::Inline);
}

#[test]
fn inheritance_flows_down_and_can_be_stopped() {
    let html = "<div id=a style=\"color:#ff0000\"><section><p id=b>x</p></section></div>";
    let s = style_one(html, "", Find::Id("b"));
    assert_eq!(s.color, tb_core::Color::rgb(255, 0, 0));
    // `display` does not inherit; `all: initial`-like resets do apply per element
    assert_eq!(s.display, Display::Block);
    // explicit inherit keyword
    let s = style_one(
        "<div style=\"font-size:30px\"><em id=b>x</em></div>",
        "#b { font-size: 50% }",
        Find::Id("b"),
    );
    assert_eq!(s.font_size, 15.0);
}

#[test]
fn shorthands_expand_to_sides() {
    let s = style_one(
        "<div id=a>x</div>",
        "#a{margin:1px 2px 3px 4px}",
        Find::Id("a"),
    );
    assert_eq!(s.margin, [L::Px(1.0), L::Px(2.0), L::Px(3.0), L::Px(4.0)]);
    let s = style_one("<div id=a>x</div>", "#a{margin:10px 5px}", Find::Id("a"));
    assert_eq!(s.margin, [L::Px(10.0), L::Px(5.0), L::Px(10.0), L::Px(5.0)]);
    let s = style_one("<div id=a>x</div>", "#a{padding:4px}", Find::Id("a"));
    assert_eq!(s.padding, [L::Px(4.0); 4]);
    let s = style_one(
        "<div id=a>x</div>",
        "#a{border:2px solid #00ff00}",
        Find::Id("a"),
    );
    assert_eq!(s.border_width, [2.0; 4]);
    assert_eq!(s.border_style, [BorderStyle::Solid; 4]);
    assert_eq!(s.border_color, [tb_core::Color::rgb(0, 255, 0); 4]);
    let s = style_one(
        "<div id=a>x</div>",
        "#a{border-top:1px dotted red;border-left:3px solid blue}",
        Find::Id("a"),
    );
    assert_eq!(s.border_width, [1.0, 0.0, 0.0, 3.0]);
    assert_eq!(s.border_style[ComputedStyle::TOP], BorderStyle::Dotted);
    assert_eq!(
        s.border_color[ComputedStyle::LEFT],
        tb_core::Color::rgb(0, 0, 255)
    );
}

#[test]
fn font_shorthand_and_line_height() {
    let s = style_one(
        "<div id=a>x</div>",
        "#a{font:bold italic 12px/1.5 Georgia, serif}",
        Find::Id("a"),
    );
    assert_eq!(s.font_size, 12.0);
    assert_eq!(s.font_weight, 700);
    assert!(s.italic);
    assert_eq!(s.line_height, 18.0);
    // family names are lower-cased with every other CSS identifier
    assert_eq!(s.font_family.first().map(|s| s.as_str()), Some("georgia"));
    assert!(s.font_family.iter().any(|f| f == "serif"));
    // em/rem resolution
    let s = style_one(
        "<div style=\"font-size:20px\"><p id=a>x</p></div>",
        "#a{font-size:1.5em}",
        Find::Id("a"),
    );
    assert_eq!(s.font_size, 30.0);
    let s = style_one("<div id=a>x</div>", "#a{width:2rem}", Find::Id("a"));
    assert_eq!(s.width, L::Px(32.0));
    let s = style_one("<div id=a>x</div>", "#a{width:50vw}", Find::Id("a"));
    assert_eq!(s.width, L::Px(500.0));
}

#[test]
fn media_queries_filter_rules_by_viewport() {
    let css = "p{color:red} @media (max-width:600px){p{color:blue}} @media (min-width:900px){p{font-size:20px}}";
    let s = style_one("<p id=a>x</p>", css, Find::Id("a"));
    assert_eq!(s.color, tb_core::Color::parse("red").unwrap());
    assert_eq!(s.font_size, 20.0);
    // narrow viewport flips the first query
    let dom = parse_document("<p id=a>x</p>");
    let mut problems = tb_core::Problems::new();
    let sheets = resolve_sources(
        &[CssSource::Inline {
            text: css.to_string(),
        }],
        Viewport {
            width: 400.0,
            height: 600.0,
            ..Default::default()
        },
        |_| None,
        &mut problems,
    );
    let out = StyleEngine::new(Viewport {
        width: 400.0,
        height: 600.0,
        ..Default::default()
    })
    .style_document(&dom, &sheets);
    let p = dom.get_element_by_id("a").unwrap();
    assert_eq!(
        out.styles.get(p).unwrap().color,
        tb_core::Color::parse("blue").unwrap()
    );
}

#[test]
fn custom_properties_resolve_via_var() {
    let s = style_one(
        "<div id=wrap><p id=a>x</p></div>",
        "#wrap{--c:#123456;--pad:7px} #a{color:var(--c);padding:var(--pad)}",
        Find::Id("a"),
    );
    assert_eq!(s.color, tb_core::Color::rgb(0x12, 0x34, 0x56));
    assert_eq!(s.padding[ComputedStyle::TOP], L::Px(7.0));
    // fallbacks
    let s = style_one(
        "<p id=a>x</p>",
        "#a{color:var(--nope, teal)}",
        Find::Id("a"),
    );
    assert_eq!(s.color, tb_core::Color::parse("teal").unwrap());
    // redefinition closer to the element wins
    let s = style_one(
        "<div id=wrap style=\"--c:black\"><p id=a>x</p></div>",
        "html{--c:red} #a{color:var(--c)}",
        Find::Id("a"),
    );
    assert_eq!(s.color, tb_core::Color::parse("black").unwrap());
}

#[test]
fn calc_reduces_to_linear_lengths() {
    let s = style_one(
        "<div id=a>x</div>",
        "#a{width:calc(100% - 32px)}",
        Find::Id("a"),
    );
    assert_eq!(
        s.width,
        L::Linear {
            pct: 100.0,
            fixed: -32.0
        }
    );
    assert_eq!(s.width.resolve(800.0), 768.0);
    let s = style_one(
        "<div id=a>x</div>",
        "#a{width:calc(2em + 10px)}",
        Find::Id("a"),
    );
    assert_eq!(s.width, L::Px(42.0));
    let s = style_one(
        "<div id=a style='--w:25%'>x</div>",
        "#a{width:calc(var(--w) * 2 + 8px)}",
        Find::Id("a"),
    );
    assert_eq!(
        s.width,
        L::Linear {
            pct: 50.0,
            fixed: 8.0
        }
    );
}

#[test]
fn pseudo_element_content() {
    let s = style_one(
        "<p id=a>x</p>",
        "#a::before{content:\"«\"} #a::after{content:\"»\"}",
        Find::Id("a"),
    );
    assert_eq!(s.before_content.as_deref(), Some("«"));
    assert_eq!(s.after_content.as_deref(), Some("»"));
    // `content: none` produces nothing visible but the property was seen
    let s = style_one("<p id=a>x</p>", "#a::before{content:none}", Find::Id("a"));
    assert_eq!(s.before_content.as_deref(), Some(""));
}

#[test]
fn pseudo_class_state_and_negation() {
    let dom = parse_document("<ul><li class=x><a href=/1>one</a></li><li><a>two</a></li></ul>");
    let mut problems = tb_core::Problems::new();
    let sheets = resolve_sources(
        &[CssSource::Inline {
            text: ".x a { color: #ff0000 } li:not(.x) { opacity: 0.5 }".into(),
        }],
        vp(),
        |_| None,
        &mut problems,
    );
    let out = StyleEngine::new(vp()).style_document(&dom, &sheets);
    let links = dom.get_elements_by_tag("a");
    assert_eq!(
        out.styles.get(links[0]).unwrap().color,
        tb_core::Color::rgb(255, 0, 0)
    );
    assert_eq!(
        out.styles.get(links[1]).unwrap().color,
        tb_core::Color::parse("#00e").unwrap()
    );
    let lis = dom.get_elements_by_tag("li");
    assert_eq!(out.styles.get(lis[0]).unwrap().opacity, 1.0);
    assert_eq!(out.styles.get(lis[1]).unwrap().opacity, 0.5);
}

#[test]
fn display_none_hides_and_flex_values_are_computed() {
    let s = style_one("<div id=a>x</div>", "#a{display:none}", Find::Id("a"));
    assert_eq!(s.display, Display::None);
    let s = style_one(
        "<div id=a>x</div>",
        "#a{display:flex;flex-direction:column;justify-content:center;gap:8px}",
        Find::Id("a"),
    );
    assert_eq!(s.display, Display::Flex);
    assert_eq!(s.flex_direction, FlexDirection::Column);
    assert_eq!(s.justify_content, JustifyContent::Center);
    assert_eq!(s.row_gap, 8.0);
    assert_eq!(s.column_gap, 8.0);
    let s = style_one("<div id=a>x</div>", "#a{flex:1 0 auto}", Find::Id("a"));
    assert_eq!(s.flex_grow, 1.0);
    assert_eq!(s.flex_shrink, 0.0);
    assert!(s.flex_basis.is_auto());
}

#[test]
fn text_and_background_values() {
    let s = style_one(
        "<div id=a>x</div>",
        "#a{text-align:center;text-transform:uppercase;white-space:pre-wrap;background:#eee url(bg.png) no-repeat}",
        Find::Id("a"),
    );
    assert_eq!(s.text_align, TextAlign::Center);
    assert_eq!(s.text_transform, TextTransform::Uppercase);
    assert_eq!(s.white_space, WhiteSpace::PreWrap);
    assert_eq!(s.background_color, tb_core::Color::parse("#eee").unwrap());
    assert!(s.background_image);
    let s = style_one("<a id=a href=/x>x</a>", "", Find::Id("a"));
    assert!(s.text_decoration, "links are underlined by the UA sheet");
}

#[test]
fn position_offsets_and_z_index() {
    let s = style_one(
        "<div id=a>x</div>",
        "#a{position:absolute;top:10px;left:20px;z-index:3}",
        Find::Id("a"),
    );
    assert_eq!(s.position, Position::Absolute);
    assert_eq!(s.inset[ComputedStyle::TOP], L::Px(10.0));
    assert_eq!(s.inset[ComputedStyle::LEFT], L::Px(20.0));
    assert_eq!(s.z_index, Some(3));
    let s = style_one("<div id=a>x</div>", "#a{z-index:auto}", Find::Id("a"));
    assert_eq!(s.z_index, None);
}

#[test]
fn malformed_css_does_not_break_the_rest() {
    let s = style_one(
        "<p id=a>x</p>",
        "#a{color: ; font-size: ; margin: 5px; font-weight: bold}",
        Find::Id("a"),
    );
    assert_eq!(s.margin[ComputedStyle::TOP], L::Px(5.0));
    assert_eq!(s.font_weight, 700);
    assert_eq!(
        s.font_size, 16.0,
        "invalid font-size is dropped, not applied"
    );
    // an unterminated function swallows the rest of the block — like browsers
    let s = style_one(
        "<p id=a>x</p>",
        "#a{color: rgb(1,2,3; margin: 5px} #a{font-weight: bold}",
        Find::Id("a"),
    );
    assert_eq!(s.font_weight, 700);
}

#[test]
fn stylesheet_order_beats_specificity_only_when_equal() {
    // later sheet, equal specificity
    let dom = parse_document("<p class=a>x</p>");
    let mut problems = tb_core::Problems::new();
    let sheets = resolve_sources(
        &[
            CssSource::Inline {
                text: ".a{color:red}".into(),
            },
            CssSource::Inline {
                text: ".a{color:blue}".into(),
            },
        ],
        vp(),
        |_| None,
        &mut problems,
    );
    let out = StyleEngine::new(vp()).style_document(&dom, &sheets);
    let p = dom.get_elements_by_tag("p")[0];
    assert_eq!(
        out.styles.get(p).unwrap().color,
        tb_core::Color::parse("blue").unwrap()
    );
}

#[test]
fn deep_tree_styling_is_fast_and_complete() {
    let mut html = String::from("<div id=root>");
    for i in 0..2000 {
        html.push_str(&format!("<p class='i{i}'>t{i}</p>"));
    }
    html.push_str("</div>");
    let dom = parse_document(&html);
    let mut problems = tb_core::Problems::new();
    let sheets = resolve_sources(
        &[CssSource::Inline {
            text: "#root p{color:#111} .i7{font-weight:bold}".into(),
        }],
        vp(),
        |_| None,
        &mut problems,
    );
    let t0 = std::time::Instant::now();
    let out = StyleEngine::new(vp()).style_document(&dom, &sheets);
    let ms = t0.elapsed().as_millis();
    let ps = dom.get_elements_by_tag("p");
    assert_eq!(ps.len(), 2000);
    assert_eq!(
        out.styles.get(ps[0]).unwrap().color,
        tb_core::Color::parse("#111").unwrap()
    );
    assert_eq!(out.styles.get(ps[7]).unwrap().font_weight, 700);
    // 2000 elements with a rule index must stay well under a second
    assert!(ms < 900, "styling 2000 elements took {ms}ms");
}
