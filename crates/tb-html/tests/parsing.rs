//! Tokenizer and tree builder tests — including the markup shapes that
//! taz.de actually serves.

use tb_html::tokenizer::{RawKind, Token, Tokenizer};
use tb_html::{parse_document, parse_fragment};

fn tokens(input: &str) -> Vec<Token> {
    let mut t = Tokenizer::new(input);
    let mut out = Vec::new();
    loop {
        let tok = t.next_token();
        if tok.is_eof() {
            break;
        }
        out.push(tok);
    }
    out
}

#[test]
fn tokenizes_simple_tags_and_text() {
    let t = tokens("<p class=\"a\">hi</p>");
    assert_eq!(
        t,
        vec![
            Token::StartTag {
                name: "p".into(),
                attrs: vec![("class".into(), "a".into())],
                self_closing: false
            },
            Token::Text { text: "hi".into() },
            Token::EndTag { name: "p".into() },
        ]
    );
}

#[test]
fn tokenizes_single_quoted_and_unquoted_and_uppercase() {
    let t = tokens("<DIV ID='x' HIDDEN data-n=2>");
    match &t[0] {
        Token::StartTag {
            name,
            attrs,
            self_closing,
        } => {
            assert_eq!(name, "div");
            assert_eq!(attrs[0], ("id".into(), "x".into()));
            assert_eq!(attrs[1], ("hidden".into(), "".into()));
            assert_eq!(attrs[2], ("data-n".into(), "2".into()));
            assert!(!self_closing);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn self_closing_and_void_elements() {
    let t = tokens("<img src=a.png /><br>");
    match &t[0] {
        Token::StartTag {
            name, self_closing, ..
        } => {
            assert_eq!(name, "img");
            assert!(self_closing);
        }
        o => panic!("{o:?}"),
    }
    assert_eq!(t[1], Token::StartTag { name: "br".into(), attrs: vec![], self_closing: false });
}

#[test]
fn comments_and_doctype() {
    let t = tokens("<!DOCTYPE html><!-- note --><p>");
    assert_eq!(
        t[0],
        Token::Doctype {
            name: "html".into(),
            public_id: "".into(),
            system_id: "".into()
        }
    );
    assert_eq!(t[1], Token::Comment { text: " note ".into() });
}

#[test]
fn unterminated_comment_ends_at_eof() {
    let t = tokens("<!-- never closed");
    assert_eq!(t[0], Token::Comment { text: " never closed".into() });
}

#[test]
fn entities_in_text_and_attributes() {
    let t = tokens("<p title=\"a&amp;b\">caf&eacute; &lt;x&gt;</p>");
    match &t[1] {
        Token::Text { text } => assert_eq!(text, "café <x>"),
        o => panic!("{o:?}"),
    }
    match &t[0] {
        Token::StartTag { attrs, .. } => assert_eq!(attrs[0].1, "a&b"),
        o => panic!("{o:?}"),
    }
}

#[test]
fn raw_text_in_script_keeps_markup_characters() {
    let src = "<script>if (a < b && c > d) { document.write('</p>'); } var x = 1;</script>";
    let t = tokens(src);
    // the tree builder will drive the raw text switch; here we only check that
    // a plain tokenizer sees the script tag and then text
    assert_eq!(t[0], Token::StartTag { name: "script".into(), attrs: vec![], self_closing: false });
}

#[test]
fn raw_text_mode_stops_at_the_matching_end_tag_only() {
    let mut t = Tokenizer::new("<script>var s = '</div>'; var y = 2;</script><p>x</p>");
    assert_eq!(
        t.next_token(),
        Token::StartTag { name: "script".into(), attrs: vec![], self_closing: false }
    );
    t.switch_to_raw_text(RawKind::Script, "script");
    assert_eq!(
        t.next_token(),
        Token::Text {
            text: "var s = '</div>'; var y = 2;".into()
        }
    );
    assert_eq!(t.next_token(), Token::EndTag { name: "script".into() });
    assert_eq!(t.next_token(), Token::StartTag { name: "p".into(), attrs: vec![], self_closing: false });
}

#[test]
fn style_and_title_are_raw_text() {
    let dom = parse_document("<title>a < b</title><style>p{content:'}</p>'}</style><p>real</p>");
    let title = dom.get_elements_by_tag("title").remove(0);
    assert_eq!(dom.text_content(title), "a < b");
    let style = dom.get_elements_by_tag("style").remove(0);
    assert_eq!(dom.text_content(style), "p{content:'}</p>'}");
    let p = dom.get_elements_by_tag("p").remove(0);
    assert_eq!(dom.text_content(p), "real");
}

#[test]
fn builds_implicit_html_head_body() {
    let dom = parse_document("<title>t</title><p>x</p>");
    let html = dom.get_elements_by_tag("html").remove(0);
    let head = dom.get_elements_by_tag("head").remove(0);
    let body = dom.get_elements_by_tag("body").remove(0);
    assert_eq!(dom.parent(html), Some(dom.document()));
    assert_eq!(dom.parent(head), Some(html));
    assert_eq!(dom.parent(body), Some(html));
    let title = dom.get_elements_by_tag("title").remove(0);
    assert_eq!(dom.parent(title), Some(head));
    let p = dom.get_elements_by_tag("p").remove(0);
    assert_eq!(dom.parent(p), Some(body));
}

#[test]
fn doctype_and_quirks() {
    let dom = parse_document("<!DOCTYPE html><p>x");
    assert_eq!(dom.doctype, "html");
    assert!(!dom.quirks);
    let dom2 = parse_document("<p>x");
    assert!(dom2.quirks, "no doctype means quirks mode");
    let dom3 = parse_document("<!DOCTYPE HTML PUBLIC \"-//W3C//DTD HTML 4.01//EN\"><p>");
    assert_eq!(dom3.doctype, "html");
}

#[test]
fn paragraphs_are_not_nested() {
    let dom = parse_document("<p>one<p>two<p>three");
    let ps = dom.get_elements_by_tag("p");
    assert_eq!(ps.len(), 3);
    for p in &ps {
        assert_ne!(dom.parent(*p), Some(ps[0]), "p must not contain p");
    }
    assert_eq!(dom.text_content(ps[0]), "one");
}

#[test]
fn block_elements_close_paragraphs() {
    let dom = parse_document("<p>text<div>block</div>after");
    let p = dom.get_elements_by_tag("p").remove(0);
    let div = dom.get_elements_by_tag("div").remove(0);
    assert_eq!(dom.parent(div), Some(dom.get_elements_by_tag("body").remove(0)));
    assert_eq!(dom.text_content(p), "text");
}

#[test]
fn list_items_close_each_other() {
    let dom = parse_document("<ul><li>a<li>b<li>c</ul>");
    let ul = dom.get_elements_by_tag("ul").remove(0);
    let kids = dom.child_elements(ul);
    assert_eq!(kids.len(), 3);
    for k in kids {
        assert_eq!(dom.tag(k), Some("li"));
    }
}

#[test]
fn table_gets_an_implicit_tbody() {
    let dom = parse_document("<table><tr><td>1<td>2<tr><td>3</table>");
    let table = dom.get_elements_by_tag("table").remove(0);
    let tbody = dom.get_elements_by_tag("tbody").remove(0);
    assert_eq!(dom.parent(tbody), Some(table));
    let trs = dom.get_elements_by_tag("tr");
    assert_eq!(trs.len(), 2);
    let tds = dom.get_elements_by_tag("td");
    assert_eq!(tds.len(), 3);
    assert_eq!(dom.parent(tds[0]), Some(trs[0]));
}

#[test]
fn stray_end_tags_are_ignored() {
    let dom = parse_document("<div>a</span></div></p></body></html>more");
    let div = dom.get_elements_by_tag("div").remove(0);
    assert_eq!(dom.text_content(div), "a");
    let body = dom.get_elements_by_tag("body").remove(0);
    assert!(dom.text_content(body).contains("more"));
}

#[test]
fn taz_style_broken_end_tags_on_void_elements() {
    // taz.de really emits </meta> and </link>
    let src = r#"<!DOCTYPE html><html lang="de"><head>
<title>taz.de | taz.de </title>
<meta name="viewport" content="width=device-width"></meta>
<link rel="canonical" href="https://taz.de/"></link>
</head><body><div id="main">content</div></body></html>"#;
    let dom = parse_document(src);
    let metas = dom.get_elements_by_tag("meta");
    assert_eq!(metas.len(), 1);
    assert_eq!(dom.attr(metas[0], "name").as_deref(), Some("viewport"));
    let links = dom.get_elements_by_tag("link");
    assert_eq!(links.len(), 1);
    assert_eq!(
        dom.attr(links[0], "href").as_deref(),
        Some("https://taz.de/")
    );
    // the broken end tags must not have closed the document early
    let main = dom.get_element_by_id("main").expect("div#main survives");
    assert_eq!(dom.text_content(main), "content");
    assert_eq!(dom.get_elements_by_tag("head").len(), 1);
    assert_eq!(dom.get_elements_by_tag("body").len(), 1);
}

#[test]
fn unclosed_tags_close_at_the_parent_end() {
    let dom = parse_document("<div><span>a<b>c</div><p>x</p>");
    let div = dom.get_elements_by_tag("div").remove(0);
    let p = dom.get_elements_by_tag("p").remove(0);
    assert_eq!(dom.parent(div), Some(dom.get_elements_by_tag("body").remove(0)));
    // </div> closed div, span and b
    assert_eq!(dom.text_content(div), "ac");
    assert_eq!(dom.parent(p), Some(dom.get_elements_by_tag("body").remove(0)));
}

#[test]
fn misnested_tags_recover_without_panics() {
    for src in [
        "<b><i>x</b></i>",
        "<table><td>a</table>",
        "<div><p><span></div></p>",
        "<a href=1><div>y</div></a>",
        "<ul><p><li>a</p></ul>",
        "<<>>",
        "<",
        "</>",
        "<>",
        "<3",
        "<!--",
        "<script>",
        "<div id=>",
        "<img src=\"unterminated>",
        "<a href='a\"b'>x</a>",
    ] {
        let dom = parse_document(src);
        // must always produce a document with a body
        assert!(
            !dom.get_elements_by_tag("html").is_empty(),
            "no html element for {src:?}"
        );
    }
}

#[test]
fn svg_content_keeps_self_closing_and_namespace() {
    let dom = parse_document(
        "<div><svg width='20'><path d='M0 0'/><circle cx='5'></circle></svg><p>after</p></div>",
    );
    let svg = dom.get_elements_by_tag("svg").remove(0);
    let path = dom.get_elements_by_tag("path").remove(0);
    assert_eq!(dom.parent(path), Some(svg));
    let circle = dom.get_elements_by_tag("circle").remove(0);
    assert_eq!(dom.parent(circle), Some(svg));
    // <p> breaks out of svg
    let p = dom.get_elements_by_tag("p").remove(0);
    assert_eq!(dom.tag(dom.parent(p).unwrap()), Some("div"));
    assert_eq!(
        dom.element(svg).unwrap().namespace,
        tb_dom::Namespace::Svg
    );
}

#[test]
fn attributes_keep_their_case_in_values() {
    let dom = parse_document("<a href=\"/Foo/Bar.html\" VIEWBOX=\"x\">l</a>");
    let a = dom.get_elements_by_tag("a").remove(0);
    assert_eq!(dom.attr(a, "href").as_deref(), Some("/Foo/Bar.html"));
    assert_eq!(dom.attr(a, "viewbox").as_deref(), Some("x"));
}

#[test]
fn duplicate_attributes_first_wins() {
    let dom = parse_document("<div class=\"first\" class=\"second\">x</div>");
    let d = dom.get_elements_by_tag("div").remove(0);
    assert_eq!(dom.attr(d, "class").as_deref(), Some("first"));
}

#[test]
fn script_content_is_preserved_verbatim() {
    // Note: `</script>` inside a string literal ends the script - in a real
    // document the author has to write "<\/script>"; we test that rule.
    let src = "<script type=\"text/javascript\">\nif (1<2 && 3>2) { var s = \"<\\/script>\"; }\n</script>";
    let dom = parse_document(src);
    let s = dom.get_elements_by_tag("script").remove(0);
    let code = dom.text_content(s);
    assert!(code.contains("if (1<2 && 3>2)"), "{code}");
    assert!(code.contains("<\\/script>"), "{code}");
}

#[test]
fn inline_event_handler_attributes_survive() {
    let dom = parse_document(r#"<button onclick="doIt('a<b')">go</button>"#);
    let b = dom.get_elements_by_tag("button").remove(0);
    assert_eq!(
        dom.attr(b, "onclick").as_deref(),
        Some("doIt('a<b')")
    );
}

#[test]
fn fragment_parsing_for_innerhtml() {
    let mut dom = parse_document("<div id=host></div>");
    let host = dom.get_element_by_id("host").unwrap();
    parse_fragment(&mut dom, host, "<p>a</p><span>b</span>");
    let kids = dom.child_elements(host);
    assert_eq!(kids.len(), 2);
    assert_eq!(dom.tag(kids[0]), Some("p"));
    assert_eq!(dom.text_content(host), "ab");
}

#[test]
fn whitespace_text_nodes_are_kept() {
    let dom = parse_document("<p>a <b>b</b> c</p>");
    let p = dom.get_elements_by_tag("p").remove(0);
    // "a ", <b>, " c"
    assert_eq!(dom.children(p).len(), 3);
    assert_eq!(dom.text_content(p), "a b c");
}

#[test]
fn deep_nesting_does_not_overflow() {
    // an arena based tree builder has no recursion while parsing
    let src = "<div>".repeat(2000) + "x" + &"</div>".repeat(2000);
    let dom = parse_document(&src);
    assert_eq!(dom.get_elements_by_tag("div").len(), 2000);
}

#[test]
fn fuzz_random_markup_never_panics() {
    // deterministic pseudo random generator (xorshift)
    let mut seed: u64 = 0x243f_6a88_85a3_08d3;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let alphabet: Vec<char> = "<>/!-a=' \"b\r\n\t;p&c#01xscriptstylediv".chars().collect();
    for i in 0..300 {
        let len = 5 + (next() as usize % 120);
        let s: String = (0..len)
            .map(|_| alphabet[(next() as usize) % alphabet.len()])
            .collect();
        let dom = parse_document(&s);
        assert!(!dom.get_elements_by_tag("html").is_empty(), "case {i}");
        // every node reachable from document must exist
        for n in dom.descendants(dom.document()) {
            assert!(dom.exists(n));
        }
    }
}

#[test]
fn big_page_parses_in_one_pass() {
    let mut src = String::from("<!DOCTYPE html><html><body>");
    for i in 0..2000 {
        src.push_str(&format!(
            "<div class=\"item i{i}\" id=\"n{i}\"><a href=\"/a/{i}\" title=\"t &amp; {i}\">Item {i}</a><br></div>"
        ));
    }
    src.push_str("</body></html>");
    let dom = parse_document(&src);
    assert_eq!(dom.get_elements_by_tag("div").len(), 2000);
    assert_eq!(dom.get_elements_by_tag("a").len(), 2000);
    let n1999 = dom.get_element_by_id("n1999").unwrap();
    let a = dom.child_elements(n1999)[0];
    assert_eq!(dom.tag(a), Some("a"));
    assert_eq!(dom.attr(a, "title").as_deref(), Some("t & 1999"));
    assert_eq!(dom.attr(a, "href").as_deref(), Some("/a/1999"));
}
