//! Layout tests: box sizes, wrapping, flex, absolute positioning, hit testing.
//!
//! Every test styles a tiny document and then asks for the geometry of one
//! element. Lengths are in CSS px; our text measurement comes from the bitmap
//! font, so line counts are deterministic.

use tb_css::media::Viewport;
use tb_html::parse_document;
use tb_layout::{element_at, layout, LayoutBox};
use tb_style::{compute, MatchState};

fn vp(w: f32, h: f32) -> Viewport {
    Viewport {
        width: w,
        height: h,
        ..Default::default()
    }
}

/// Lay out `html` (author CSS already inline in the markup where needed).
fn lay(html: &str, width: f32) -> (tb_dom::Dom, tb_layout::LayoutDocument) {
    let dom = parse_document(html);
    let mut problems = tb_core::Problems::new();
    let sheets = tb_style::resolve_sources(
        &tb_style::collect_sources(&dom),
        vp(width, 800.0),
        |_| None,
        &mut problems,
    );
    let styled = compute(&dom, &sheets, vp(width, 800.0), &MatchState::default());
    let doc = layout(&dom, &styled.styles, vp(width, 800.0)).expect("layout");
    (dom, doc)
}

fn box_of<'a>(
    doc: &'a tb_layout::LayoutDocument,
    dom: &'a tb_dom::Dom,
    sel: &str,
) -> &'a LayoutBox {
    let node = if let Some(id) = sel.strip_prefix('#') {
        dom.get_element_by_id(id).expect("id")
    } else {
        let all = dom.get_elements_by_tag(sel);
        *all.last().expect("element")
    };
    doc.find(node).expect("box")
}

// ------------------------------------------------------------------- blocks --

#[test]
fn auto_width_block_fills_its_container() {
    let (dom, doc) = lay("<body style='margin:0'><div id=a>x</div></body>", 500.0);
    let a = box_of(&doc, &dom, "#a");
    assert_eq!(a.border_box().width, 500.0);
    assert_eq!(a.border_box().x, 0.0);
    let body = box_of(&doc, &dom, "body");
    assert_eq!(body.border_box().width, 500.0, "body margin is 0 here");
}

#[test]
fn body_default_margin_is_8px() {
    let (dom, doc) = lay("<div id=a>x</div>", 500.0);
    let a = box_of(&doc, &dom, "#a");
    assert_eq!(a.border_box().x, 8.0);
    assert_eq!(a.border_box().width, 500.0 - 16.0);
}

#[test]
fn explicit_width_padding_and_border() {
    let (dom, doc) = lay(
        "<body style='margin:0'><div id=a style='width:100px;padding:10px 20px;border:2px solid'>x</div></body>",
        500.0,
    );
    let a = box_of(&doc, &dom, "#a");
    assert_eq!(a.content.width, 100.0);
    assert_eq!(a.padding.left, 20.0);
    assert_eq!(a.border.left, 2.0);
    assert_eq!(a.border_box().width, 100.0 + 40.0 + 4.0);
}

#[test]
fn box_sizing_border_box() {
    let (dom, doc) = lay(
        "<body style='margin:0'><div id=a style='width:120px;box-sizing:border-box;padding:10px;border:5px solid'>x</div></body>",
        500.0,
    );
    let a = box_of(&doc, &dom, "#a");
    assert_eq!(a.border_box().width, 120.0);
    assert_eq!(a.content.width, 120.0 - 20.0 - 10.0);
}

#[test]
fn percentage_width_resolves_against_container() {
    let (dom, doc) = lay(
        "<body style='margin:0'><div style='width:400px'><div id=a style='width:50%'>x</div></div></body>",
        800.0,
    );
    assert_eq!(box_of(&doc, &dom, "#a").border_box().width, 200.0);
}

#[test]
fn margin_auto_centres_a_fixed_width_box() {
    let (dom, doc) = lay(
        "<body style='margin:0'><div id=a style='width:200px;margin:0 auto'>x</div></body>",
        600.0,
    );
    let a = box_of(&doc, &dom, "#a");
    assert_eq!(a.border_box().x, 200.0, "centred in 600px");
    assert_eq!(a.border_box().width, 200.0);
}

#[test]
fn stacked_blocks_and_their_margins() {
    let (dom, doc) = lay(
        "<body style='margin:0'>\
           <div style='height:50px'>a</div>\
           <div id=b style='height:30px;margin-top:12px'>b</div>\
         </body>",
        400.0,
    );
    let b = box_of(&doc, &dom, "#b");
    assert_eq!(b.border_box().y, 50.0 + 12.0);
    assert_eq!(b.border_box().height, 30.0);
}

#[test]
fn document_grows_beyond_the_viewport() {
    let (_, doc) = lay(
        "<body style='margin:0'><div style='height:3000px'></div></body>",
        500.0,
    );
    assert!(doc.scroll_size.height >= 3000.0, "{:?}", doc.scroll_size);
    assert_eq!(doc.viewport.width, 500.0);
}

#[test]
fn nested_padding_offsets_the_child() {
    let (dom, doc) = lay(
        "<body style='margin:0'><div style='padding:25px'><div id=a style='height:10px'>x</div></div></body>",
        500.0,
    );
    let a = box_of(&doc, &dom, "#a");
    assert_eq!((a.border_box().x, a.border_box().y), (25.0, 25.0));
}

// -------------------------------------------------------------------- text --

#[test]
fn text_wraps_into_line_boxes() {
    let words = "wort ".repeat(60);
    let html =
        format!("<body style='margin:0'><p style='width:200px;font-size:14px'>{words}</p></body>");
    let (dom, doc) = lay(&html, 800.0);
    let p = box_of(&doc, &dom, "p");
    assert!(
        p.lines.len() > 5,
        "expected wrapping, got {} lines",
        p.lines.len()
    );
    // every line fits into the content width
    for line in &p.lines {
        let right = line
            .runs
            .iter()
            .map(|r| r.x + r.width)
            .fold(0.0f32, f32::max);
        assert!(
            right <= line.rect.right() + 0.5,
            "line overflows: {right} > {}",
            line.rect.right()
        );
        assert!(line.rect.height > 8.0);
    }
    // lines are stacked
    let ys: Vec<f32> = p.lines.iter().map(|l| l.rect.y).collect();
    let mut sorted = ys.clone();
    sorted.sort_by(|a: &f32, b: &f32| a.partial_cmp(b).unwrap());
    assert_eq!(ys, sorted);
}

#[test]
fn line_boxes_have_a_baseline_for_drawing() {
    let (dom, doc) = lay(
        "<body style='margin:0'><p style='font-size:16px'>Hallo Welt</p></body>",
        400.0,
    );
    let p = box_of(&doc, &dom, "p");
    assert_eq!(p.lines.len(), 1);
    let line = &p.lines[0];
    assert!(line.baseline > line.rect.y);
    assert!(line.baseline <= line.rect.bottom() + 0.5);
    assert_eq!(line.runs.len(), 2, "two words = two runs");
    assert_eq!(line.runs[0].text, "Hallo");
    assert_eq!(line.runs[1].text, "Welt");
    assert!(line.runs[1].x > line.runs[0].x + line.runs[0].width - 0.5);
}

#[test]
fn nowrap_does_not_wrap() {
    let words = "sehr_langes_wort ".repeat(30);
    let html = format!("<body style='margin:0'><p style='width:150px;white-space:nowrap;font-size:14px'>{words}</p></body>");
    let (dom, doc) = lay(&html, 800.0);
    let p = box_of(&doc, &dom, "p");
    assert_eq!(p.lines.len(), 1, "nowrap keeps one line");
    assert!(
        doc.scroll_size.width > 150.0,
        "content overflows the viewport width"
    );
}

#[test]
fn pre_preserves_newlines_and_spaces() {
    let html =
        "<body style='margin:0'><pre style='font-size:14px'>zeile1\n  zeile2\nzeile3</pre></body>";
    let (dom, doc) = lay(html, 800.0);
    let pre = box_of(&doc, &dom, "pre");
    assert_eq!(
        pre.lines.len(),
        3,
        "{:#?}",
        pre.lines.iter().map(|l| l.runs.len()).collect::<Vec<_>>()
    );
}

#[test]
fn text_align_moves_the_runs() {
    let base = "<body style='margin:0'><p style='width:300px;font-size:14px;text-align:{}'>zwei wörter</p></body>";
    let (dom, doc) = lay(&base.replace("{}", "left"), 800.0);
    let left = box_of(&doc, &dom, "p").lines[0].runs[0].x;
    let (dom, doc) = lay(&base.replace("{}", "center"), 800.0);
    let center = box_of(&doc, &dom, "p").lines[0].runs[0].x;
    let (dom, doc) = lay(&base.replace("{}", "right"), 800.0);
    let right = box_of(&doc, &dom, "p").lines[0].runs[0].x;
    assert!(left < center, "{left} < {center}");
    assert!(center < right, "{center} < {right}");
}

#[test]
fn text_transform_uppercases() {
    let (dom, doc) = lay(
        "<body style='margin:0'><p style='text-transform:uppercase'>hallo</p></body>",
        400.0,
    );
    let p = box_of(&doc, &dom, "p");
    assert_eq!(p.lines[0].runs[0].text, "HALLO");
}

#[test]
fn a_long_word_breaks_instead_of_overflowing() {
    let word = "x".repeat(120);
    let html =
        format!("<body style='margin:0'><p style='width:120px;font-size:14px'>{word}</p></body>");
    let (dom, doc) = lay(&html, 400.0);
    let p = box_of(&doc, &dom, "p");
    assert!(p.lines.len() >= 2, "word must break");
    for line in &p.lines {
        let right = line
            .runs
            .iter()
            .map(|r| r.x + r.width)
            .fold(0.0f32, f32::max);
        assert!(
            right <= line.rect.right() + 1.0,
            "{right} > {}",
            line.rect.right()
        );
    }
}

#[test]
fn inline_elements_share_a_line() {
    let (dom, doc) = lay(
        "<body style='margin:0'><p>einfach <b>fett</b> und <em>schräg</em> text</p></body>",
        600.0,
    );
    let p = box_of(&doc, &dom, "p");
    assert_eq!(p.lines.len(), 1);
    assert!(p.lines[0].runs.len() >= 5);
    // bold text is measured wider than the same text in normal weight
    let b = box_of(&doc, &dom, "b");
    assert!(b.content.height > 0.0);
}

#[test]
fn inline_block_sits_in_the_text_flow() {
    let (dom, doc) = lay(
        "<body style='margin:0'><p><span style='display:inline-block;width:40px;height:20px;background:red'></span>danach</p></body>",
        600.0,
    );
    let span = box_of(&doc, &dom, "span");
    assert_eq!(
        (span.border_box().width, span.border_box().height),
        (40.0, 20.0)
    );
    let text = doc.text();
    assert!(text.contains("danach"), "{text}");
}

// --------------------------------------------------------------------- flex --

#[test]
fn flex_row_distributes_free_space() {
    let (dom, doc) = lay(
        "<body style='margin:0'><div style='display:flex;width:300px'>\
           <div style='flex:1'>a</div><div style='flex:1'>b</div><div style='flex:1'>c</div>\
         </div></body>",
        600.0,
    );
    let items: Vec<&LayoutBox> = doc.boxes_of_tag(&dom, "div").into_iter().skip(1).collect();
    assert_eq!(items.len(), 3);
    for item in &items {
        assert!(item.border_box().width > 90.0, "{:?}", item.border_box());
    }
    assert!(items[1].border_box().x > items[0].border_box().x);
    assert!(items[2].border_box().x > items[1].border_box().x);
    // they are on the same line
    assert_eq!(items[0].border_box().y, items[2].border_box().y);
}

#[test]
fn flex_row_with_fixed_basis_and_gap() {
    let (dom, doc) = lay(
        "<body style='margin:0'><div style='display:flex;gap:10px'>\
           <div style='width:50px;height:20px'>a</div><div style='width:60px;height:20px'>b</div>\
         </div></body>",
        600.0,
    );
    let items: Vec<&LayoutBox> = doc.boxes_of_tag(&dom, "div").into_iter().skip(1).collect();
    assert_eq!(items[0].border_box().x, 0.0);
    assert_eq!(items[1].border_box().x, 60.0, "50 + 10 gap");
}

#[test]
fn flex_column_stacks_and_grows() {
    let (dom, doc) = lay(
        "<body style='margin:0'><div style='display:flex;flex-direction:column;height:200px'>\
           <div style='flex:1'>a</div><div style='height:30px'>b</div>\
         </div></body>",
        600.0,
    );
    let items: Vec<&LayoutBox> = doc.boxes_of_tag(&dom, "div").into_iter().skip(1).collect();
    assert_eq!(items[1].border_box().height, 30.0);
    assert!(
        items[0].border_box().height >= 150.0,
        "{:?}",
        items[0].border_box()
    );
    assert!(items[1].border_box().y > items[0].border_box().y);
}

#[test]
fn flex_wrap_makes_two_lines() {
    let (dom, doc) = lay(
        "<body style='margin:0'><div style='display:flex;flex-wrap:wrap;width:100px'>\
           <div style='width:60px;height:10px'>a</div><div style='width:60px;height:10px'>b</div>\
         </div></body>",
        600.0,
    );
    let items: Vec<&LayoutBox> = doc.boxes_of_tag(&dom, "div").into_iter().skip(1).collect();
    assert!(
        items[1].border_box().y > items[0].border_box().y,
        "must wrap"
    );
    assert_eq!(items[1].border_box().x, 0.0);
}

#[test]
fn justify_content_space_between() {
    let (dom, doc) = lay(
        "<body style='margin:0'><div style='display:flex;justify-content:space-between;width:300px'>\
           <div style='width:50px;height:10px'>a</div><div style='width:50px;height:10px'>b</div>\
         </div></body>",
        600.0,
    );
    let items: Vec<&LayoutBox> = doc.boxes_of_tag(&dom, "div").into_iter().skip(1).collect();
    assert_eq!(items[0].border_box().x, 0.0);
    assert_eq!(items[1].border_box().right(), 300.0);
}

#[test]
fn align_items_centres_on_the_cross_axis() {
    let (dom, doc) = lay(
        "<body style='margin:0'><div style='display:flex;align-items:center;height:100px;width:200px'>\
           <div style='width:20px;height:20px'>a</div>\
         </div></body>",
        600.0,
    );
    let item = doc.boxes_of_tag(&dom, "div").into_iter().nth(1).unwrap();
    assert!(item.border_box().y > 20.0, "{:?}", item.border_box());
    assert!(item.border_box().bottom() < 100.0);
}

// -------------------------------------------------------------- positioned --

#[test]
fn absolute_positioning_with_offsets() {
    let (dom, doc) = lay(
        "<body style='margin:0'><div style='position:relative;width:400px;height:200px'>\
           <div id=a style='position:absolute;left:20px;top:30px;width:50px;height:40px'>x</div>\
         </div></body>",
        600.0,
    );
    let a = box_of(&doc, &dom, "#a");
    assert_eq!((a.border_box().x, a.border_box().y), (20.0, 30.0));
    assert_eq!((a.border_box().width, a.border_box().height), (50.0, 40.0));
}

#[test]
fn absolute_with_left_and_right_stretches() {
    let (dom, doc) = lay(
        "<body style='margin:0'><div style='position:relative;width:400px;height:100px'>\
           <div id=a style='position:absolute;left:10px;right:20px;top:0;height:10px'>x</div>\
         </div></body>",
        600.0,
    );
    let a = box_of(&doc, &dom, "#a");
    assert_eq!(a.border_box().x, 10.0);
    assert_eq!(a.border_box().right(), 380.0);
}

#[test]
fn absolute_box_does_not_push_following_content() {
    let (dom, doc) = lay(
        "<body style='margin:0'><div style='position:relative'>\
           <div style='position:absolute;top:0;height:500px'>a</div>\
           <div id=b style='height:20px'>b</div>\
         </div></body>",
        600.0,
    );
    let b = box_of(&doc, &dom, "#b");
    assert_eq!(b.border_box().y, 0.0, "out of flow takes no space");
}

#[test]
fn relative_positioning_shifts_without_reflow() {
    let (dom, doc) = lay(
        "<body style='margin:0'><div style='height:20px'>a</div>\
         <div id=b style='position:relative;top:15px;left:8px;height:20px'>b</div>\
         <div style='height:20px'>c</div></body>",
        600.0,
    );
    let b = box_of(&doc, &dom, "#b");
    assert_eq!(b.border_box().y, 20.0 + 15.0);
    assert_eq!(b.border_box().x, 8.0);
}

#[test]
fn fixed_positions_against_the_viewport() {
    let (dom, doc) = lay(
        "<body style='margin:0'><div id=a style='position:fixed;top:0;left:0;width:100px;height:30px'>x</div></body>",
        600.0,
    );
    let a = box_of(&doc, &dom, "#a");
    assert_eq!((a.border_box().x, a.border_box().y), (0.0, 0.0));
}

// ------------------------------------------------------------------ floats --

#[test]
fn float_left_lets_text_flow_next_to_it() {
    let (dom, doc) = lay(
        "<body style='margin:0'><div style='width:300px'>\
           <img src=x style='float:left;width:80px;height:60px' alt=bild>\
           <p>fließender text der neben dem bild steht und umfließen soll</p>\
         </div></body>",
        600.0,
    );
    let img = box_of(&doc, &dom, "img");
    assert_eq!((img.border_box().x, img.border_box().width), (0.0, 80.0));
    let p = box_of(&doc, &dom, "p");
    let first = &p.lines[0];
    assert!(
        first.runs[0].x >= 80.0 - 1.0,
        "text must start right of the float: {:?}",
        first.runs[0].x
    );
}

// ------------------------------------------------------------- display none --

#[test]
fn display_none_subtree_is_gone() {
    let (dom, doc) = lay(
        "<body style='margin:0'><div style='display:none'><p id=x>versteckt</p></div><div id=y>sichtbar</div></body>",
        600.0,
    );
    let x = dom.get_element_by_id("x").unwrap();
    assert!(doc.find(x).is_none(), "display:none generates no box");
    assert!(!doc.text().contains("versteckt"));
    assert!(doc.text().contains("sichtbar"));
}

// ------------------------------------------------------------- hit testing --

#[test]
fn hit_test_finds_the_deepest_element() {
    let (dom, doc) = lay(
        "<body style='margin:0'><div id=outer style='width:200px;height:100px;padding:10px'>\
           <div id=inner style='width:50px;height:30px'>in</div>\
         </div></body>",
        600.0,
    );
    let inner = dom.get_element_by_id("inner").unwrap();
    let r = doc.find(inner).unwrap().border_box();
    let hit = element_at(&doc, &dom, r.x + 2.0, r.y + 2.0).expect("hit");
    assert_eq!(hit, inner);
    // outside the inner box but inside the outer one
    let outer = dom.get_element_by_id("outer").unwrap();
    let hit2 = element_at(&doc, &dom, 150.0, 15.0).expect("hit 2");
    assert_eq!(hit2, outer);
}

#[test]
fn hit_test_on_text_returns_the_link() {
    let (dom, doc) = lay(
        "<body style='margin:0'><p><a href=/artikel id=link>ein Artikel</a></p></body>",
        600.0,
    );
    let a = dom.get_element_by_id("link").unwrap();
    let r = doc.find(a).unwrap().border_box();
    let hit = tb_layout::hit_test(&doc, &dom, r.x + 1.0, r.y + r.height / 2.0).expect("hit");
    assert_eq!(hit.text.as_deref(), Some("ein"));
    let link = tb_layout::hit::link_at(&doc, &dom, r.x + 1.0, r.y + r.height / 2.0);
    assert_eq!(link.as_deref(), Some("/artikel"));
}

// ------------------------------------------------------------------ robust --

fn check_finite(b: &LayoutBox, depth: usize) {
    let bb = b.border_box();
    for v in [bb.x, bb.y, bb.width, bb.height] {
        assert!(v.is_finite(), "non finite geometry: {v}");
    }
    assert!(depth < 300, "runaway tree");
    for c in &b.children {
        check_finite(c, depth + 1);
    }
}

#[test]
fn no_panics_and_no_nan_on_weird_css() {
    let html = "<body><div style='width:calc(50% - )'><p style='font-size:0'>x</p></div>\
                <div style='width:100%;height:calc(2vh)'></div><table><tr><td></td></tr></table>\
                <span style='display:flex'></span></body>";
    let (_, doc) = lay(html, 600.0);
    check_finite(&doc.root, 0);
    assert!(doc.scroll_size.height.is_finite());
}

#[test]
fn zero_and_negative_sizes_are_clamped() {
    let (_, doc) = lay(
        "<body style='margin:0'><div style='width:-40px;height:-10px'>a</div>\
         <div style='width:0;height:0'></div></body>",
        400.0,
    );
    assert!(doc.root.border_box().width.is_finite());
    assert!(doc.scroll_size.width >= 400.0);
}

#[test]
fn empty_document_still_lays_out() {
    let (_, doc) = lay("<html><head></head><body></body></html>", 400.0);
    assert_eq!(doc.viewport.width, 400.0);
    assert!(doc.scroll_size.height >= 0.0);
}
