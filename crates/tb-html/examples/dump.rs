fn main() {
    let src = std::env::args()
        .nth(1)
        .unwrap_or("<p>one<p>two<p>three".into());
    let dom = tb_html::parse_document(&src);
    print!("{}", dom.debug_tree());
    println!("--- html: {}", dom.outer_html(dom.document()));
}
