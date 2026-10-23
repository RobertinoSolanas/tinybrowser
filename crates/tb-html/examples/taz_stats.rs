use std::time::Instant;
fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or("fixtures/taz.de.html".into());
    let html = std::fs::read_to_string(&path).expect("read fixture");
    let t = Instant::now();
    let dom = tb_html::parse_document(&html);
    let ms = t.elapsed().as_millis();
    let els = dom.element_count();
    let nodes = dom.node_count();
    let title = dom
        .get_elements_by_tag("title")
        .first()
        .map(|t| dom.text_content(*t));
    let links = dom
        .get_elements_by_tag("a")
        .into_iter()
        .filter(|a| dom.has_attr(*a, "href"))
        .count();
    let imgs = dom.get_elements_by_tag("img").len();
    let scripts = dom.get_elements_by_tag("script").len();
    let styles = dom.get_elements_by_tag("style").len();
    let divs = dom.get_elements_by_tag("div").len();
    println!("parsed {path} in {ms} ms: {els} elements, {nodes} nodes");
    println!("title: {:?}", title);
    println!("links={links} imgs={imgs} scripts={scripts} inline-style={styles} divs={divs}");
    // sanity: text of body
    let body = dom.get_elements_by_tag("body").remove(0);
    let text = dom.visible_text(body);
    let collapsed = tb_core::text::collapse_whitespace(&text);
    println!(
        "visible text: {} chars, begins: {}",
        collapsed.chars().count(),
        collapsed.chars().take(160).collect::<String>()
    );
}
