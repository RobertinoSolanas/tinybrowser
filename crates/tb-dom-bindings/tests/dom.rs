//! Node / Element / query / serialization behaviour of the DOM bindings.

mod common;

use common::Page;

#[test]
fn document_basics() {
    let mut p = Page::html("<!doctype html><title>T</title><body><p id=a>hi</p></body>");
    assert_eq!(p.text("document.documentElement.tagName"), "HTML");
    assert_eq!(p.text("document.head.tagName"), "HEAD");
    assert_eq!(p.text("document.getElementById('a').tagName"), "P");
    assert_eq!(p.text("document.title"), "T");
    assert_eq!(p.num("document.nodeType"), 9.0);
    assert_eq!(p.text("document.nodeName"), "#document");
    assert_eq!(p.text("document.readyState"), "loading");
    assert!(p.yes("document.body === document.querySelector('body')"));
    assert_eq!(p.text("String(document.body)"), "[object HTMLBodyElement]");
    assert_eq!(p.text("String(document)"), "[object HTMLDocument]");
}

#[test]
fn wrapper_identity_is_stable() {
    let mut p = Page::html("<div id=x></div>");
    p.ids(&["x"]);
    assert!(p.yes("document.getElementById('x') === document.getElementById('x')"));
    assert!(p.yes("document.getElementById('x') === document.querySelector('#x')"));
    assert!(p.yes("x === document.getElementById('x')"));
    assert!(p.yes("document.body === document.body"));
    assert!(p.yes("document.body.ownerDocument === document"));
    assert!(p.yes("document.getElementById('x').parentElement === document.body"));
    assert!(p.yes("document.getElementById('nope') === null"));
}

#[test]
fn node_names_and_types() {
    let mut p = Page::html("<div id=d data-x=1>text<!--c--></div>");
    p.ids(&["d"]);
    assert_eq!(p.num("d.nodeType"), 1.0);
    assert_eq!(p.text("d.nodeName"), "DIV");
    assert_eq!(p.text("d.tagName"), "DIV");
    assert_eq!(p.text("d.localName"), "div");
    assert_eq!(p.num("d.firstChild.nodeType"), 3.0);
    assert_eq!(p.text("d.firstChild.nodeName"), "#text");
    assert_eq!(p.text("d.firstChild.nodeValue"), "text");
    assert_eq!(p.text("d.lastChild.nodeName"), "#comment");
    assert_eq!(p.text("d.lastChild.nodeValue"), "c");
}

#[test]
fn text_content_read_write() {
    let mut p = Page::html("<div id=d>a<b>B</b>c</div>");
    p.ids(&["d"]);
    assert_eq!(p.text("d.textContent"), "aBc");
    p.js("d.textContent = 'x<em>y</em>'");
    assert_eq!(p.text("d.textContent"), "x<em>y</em>");
    assert_eq!(p.num("d.childNodes.length"), 1.0);
    assert_eq!(p.num("d.children.length"), 0.0);
}

#[test]
fn inner_html_roundtrip() {
    let mut p = Page::html("<div id=d></div>");
    p.ids(&["d"]);
    p.js("d.innerHTML = '<p class=x>one</p><p>two</p>'");
    assert_eq!(p.text("d.innerHTML"), "<p class=\"x\">one</p><p>two</p>");
    assert_eq!(p.num("d.querySelectorAll('p').length"), 2.0);
    assert_eq!(p.text("d.querySelector('.x').textContent"), "one");
    p.js("d.innerHTML = ''");
    assert_eq!(p.num("d.childNodes.length"), 0.0);
}

#[test]
fn outer_html_and_insert_adjacent() {
    let mut p = Page::html("<div id=d><b>bold</b></div>");
    p.ids(&["d"]);
    assert_eq!(p.text("d.outerHTML"), "<div id=\"d\"><b>bold</b></div>");
    p.js("d.insertAdjacentHTML('afterbegin', '<i>i</i>')");
    p.js("d.insertAdjacentHTML('beforeend', '<u>u</u>')");
    assert_eq!(p.text("d.innerHTML"), "<i>i</i><b>bold</b><u>u</u>");
    p.js("d.insertAdjacentHTML('beforebegin', '<hr>')");
    assert!(p.yes("document.body.firstElementChild.tagName === 'HR'"));
    assert!(p.yes("d.previousElementSibling === document.body.firstElementChild"));
    p.js("d.insertAdjacentElement('beforeend', document.createElement('s'))");
    assert!(p.yes("d.lastElementChild.tagName === 'S'"));
}

#[test]
fn tree_mutation() {
    let mut p = Page::html("<ul id=u><li>1</li><li id=two>2</li></ul>");
    p.ids(&["u", "two"]);
    p.js("const li = document.createElement('li');\
         li.textContent = '3';\
         u.appendChild(li);");
    assert_eq!(p.num("u.children.length"), 3.0);
    assert_eq!(p.text("u.lastElementChild.textContent"), "3");

    p.js("u.insertBefore(document.createElement('span'), u.firstElementChild);");
    assert_eq!(p.text("u.firstElementChild.tagName"), "SPAN");

    p.js("u.removeChild(u.firstElementChild);");
    assert_eq!(p.num("u.children.length"), 3.0);

    p.js("u.replaceChild(document.createElement('em'), two);");
    assert_eq!(p.text("u.children[1].tagName"), "EM");
    assert!(p.yes("two.parentNode === null"));
    assert!(p.yes("two.isConnected === false"));
}

#[test]
fn remove_and_children_helpers() {
    let mut p = Page::html("<div id=d><span>1</span><span>2</span><b>3</b></div>");
    p.ids(&["d"]);
    p.js("d.querySelectorAll('span').forEach(s => s.remove());");
    assert_eq!(p.text("d.innerHTML"), "<b>3</b>");

    p.js("d.append('t', document.createElement('i'));");
    assert_eq!(p.num("d.childNodes.length"), 3.0);
    p.js("d.prepend('p');");
    assert_eq!(p.text("d.firstChild.nodeValue"), "p");

    p.js("d.replaceChildren(document.createElement('u'));");
    assert_eq!(p.text("d.innerHTML"), "<u></u>");
}

#[test]
fn sibling_and_child_accessors() {
    let mut p = Page::html("<div id=d>txt<b id=b>1</b>txt<i id=i>2</i></div>");
    p.ids(&["d", "b", "i"]);
    assert_eq!(p.num("d.childNodes.length"), 4.0);
    assert_eq!(p.num("d.children.length"), 2.0);
    assert_eq!(p.num("d.childElementCount"), 2.0);
    assert!(p.yes("d.firstElementChild === b"));
    assert!(p.yes("d.lastElementChild === i"));
    assert!(p.yes("b.nextElementSibling === i"));
    assert!(p.yes("i.previousElementSibling === b"));
    assert!(!p.yes("b.nextSibling === i"));
    assert!(!p.yes("b.contains(d)"));
    assert!(p.yes("d.contains(b)"));
    assert!(p.yes("b.contains(b)"));
}

#[test]
fn class_list() {
    let mut p = Page::html("<div id=d class='a b'></div>");
    p.ids(&["d"]);
    assert_eq!(p.num("d.classList.length"), 2.0);
    assert_eq!(p.text("d.classList.value"), "a b");
    assert!(p.yes("d.classList.contains('a')"));
    p.js("d.classList.add('c', 'd');");
    assert_eq!(p.text("d.className"), "a b c d");
    p.js("d.classList.remove('a');");
    assert!(!p.yes("d.classList.contains('a')"));
    // toggle returns "now present"
    assert!(!p.yes("d.classList.toggle('c')")); // removes c
    assert!(p.yes("d.classList.toggle('c')")); // adds it back
    assert!(p.yes("d.classList.replace('b', 'B')"));
    // `toggle` re-added `c` at the end, and `replace` keeps the slot of `b`
    assert_eq!(p.text("d.className"), "B d c");
    assert!(!p.yes("d.classList.replace('zz', 'B')"));
    assert_eq!(p.text("d.classList.item(0)"), "B");
    assert_eq!(p.text("String(d.classList)"), "B d c");
    p.js("window.__seen = []; d.classList.forEach(t => window.__seen.push(t));");
    assert_eq!(p.text("window.__seen.join(',')"), "B,d,c");
}

#[test]
fn dataset_and_attributes() {
    let mut p = Page::html("<div id=d data-foo=1 data-bar-baz=2></div>");
    p.ids(&["d"]);
    assert_eq!(p.text("d.dataset.foo"), "1");
    assert_eq!(p.text("d.dataset.barBaz"), "2");
    p.js("d.dataset.nope = 'x';");
    assert_eq!(p.text("d.getAttribute('data-nope')"), "x");
    p.js("delete d.dataset.foo;");
    assert!(!p.yes("'foo' in d.dataset"));
    assert!(!p.yes("d.hasAttribute('data-foo')"));

    p.js("d.setAttribute('role', 'button'); d.toggleAttribute('hidden');");
    assert_eq!(p.text("d.getAttribute('role')"), "button");
    assert!(p.yes("d.hidden"));
    p.js("d.toggleAttribute('hidden');");
    assert!(!p.yes("d.hidden"));
    assert_eq!(p.num("d.attributes.length"), 4.0); // id + 2 data + role
    assert_eq!(p.text("d.attributes[0].name"), "id");
    assert_eq!(p.text("d.attributes.getNamedItem('role').value"), "button");
    p.js("d.removeAttribute('role');");
    assert!(p.yes("d.getAttribute('role') === null"));
}

#[test]
fn id_and_reflected_properties() {
    let mut p = Page::html("<div id=d></div>");
    p.ids(&["d"]);
    p.js("d.id = 'e';");
    assert!(p.yes("document.getElementById('e') === d"));
    assert_eq!(p.text("d.id"), "e");
    p.js("d.className = 'c1 c2'; d.title = 'T'; d.lang = 'de'; d.dir = 'rtl';");
    assert_eq!(p.text("d.getAttribute('class')"), "c1 c2");
    assert_eq!(p.text("d.title"), "T");
    assert_eq!(p.text("d.lang"), "de");
    assert_eq!(p.text("d.dir"), "rtl");
}

#[test]
fn queries() {
    let mut p =
        Page::html("<section class=wrap><article><h2 id=h>A</h2><p class=p1>1</p><p class='p2' data-n=2>2</p></article><aside><p>3</p></aside></section>");
    p.ids(&["h"]);
    assert_eq!(p.num("document.querySelectorAll('p').length"), 3.0);
    assert_eq!(p.num("document.querySelectorAll('article p').length"), 2.0);
    assert_eq!(
        p.num("document.querySelectorAll('section > p').length"),
        0.0
    );
    // the <aside> is inside the <section>, so its <p> counts too
    assert_eq!(p.num("document.querySelectorAll('section p').length"), 3.0);
    assert_eq!(p.num("document.querySelectorAll('p, h2').length"), 4.0);
    assert_eq!(p.num("document.querySelectorAll('p:not(.p1)').length"), 2.0);
    assert_eq!(p.num("document.querySelectorAll('p[data-n]').length"), 1.0);
    assert_eq!(p.text("document.querySelectorAll('p')[1].className"), "p2");
    assert!(p.yes("h.matches('article > h2')"));
    assert!(p.yes("h.matches('.wrap h2, .nope')"));
    assert!(p.yes("h.closest('section').tagName === 'SECTION'"));
    assert!(p.yes("h.closest('html').tagName === 'HTML'"));
    assert!(p.yes("h.closest('nope') === null"));
    assert_eq!(p.num("document.querySelectorAll('#h, #h').length"), 1.0);
    assert!(p.yes("document.querySelector('aside p').parentElement.tagName === 'ASIDE'"));
}

#[test]
fn collection_lookups() {
    let mut p = Page::html(
        "<div><p class=c>1</p><p class=c>2</p></div><span class=c>3</span><input name=q value=x>",
    );
    assert_eq!(p.num("document.getElementsByTagName('p').length"), 2.0);
    assert_eq!(p.num("document.getElementsByClassName('c').length"), 3.0);
    assert_eq!(p.num("document.getElementsByName('q').length"), 1.0);
    assert_eq!(p.num("document.querySelectorAll('div .c').length"), 2.0);
    assert_eq!(
        p.text("document.getElementsByTagName('p')[0].textContent"),
        "1"
    );
    assert_eq!(p.num("document.links.length"), 0.0);
    assert_eq!(p.num("document.images.length"), 0.0);
    assert_eq!(
        p.text("document.forms.length === 0 ? 'none' : 'some'"),
        "none"
    );
}

#[test]
fn factories() {
    let mut p = Page::html("<div id=d></div>");
    p.ids(&["d"]);
    p.js("window.f = document.createDocumentFragment();\
         f.appendChild(document.createElement('b'));\
         f.appendChild(document.createTextNode('t'));\
         d.appendChild(f);");
    assert_eq!(p.text("d.innerHTML"), "<b></b>t");
    assert_eq!(p.num("f.childNodes.length"), 0.0);
    p.js("d.appendChild(document.createComment('cmt'));");
    assert_eq!(p.text("d.lastChild.nodeValue"), "cmt");
    assert_eq!(p.text("document.createElement('MY-TAG').tagName"), "MY-TAG");
    assert_eq!(p.text("String(document.createTextNode('t').nodeType)"), "3");
}

#[test]
fn clone_and_equal() {
    let mut p = Page::html("<div id=d class=x><b>1</b><b>2</b></div>");
    p.ids(&["d"]);
    p.js("const c = d.cloneNode(true); c.removeAttribute('id'); document.body.appendChild(c); window.c = c;");
    assert_eq!(p.num("document.querySelectorAll('div').length"), 2.0);
    assert_eq!(p.num("c.querySelectorAll('b').length"), 2.0);
    assert!(!p.yes("c === d"));
    assert_eq!(p.text("c.className"), "x");
    p.js("window.s = d.cloneNode(false); document.body.appendChild(s);");
    assert_eq!(p.num("s.childNodes.length"), 0.0);
    assert!(p.yes("d.isEqualNode(document.getElementById('d'))"));
}

#[test]
fn normalize_and_text_nodes() {
    let mut p = Page::html("<div id=d>a<!--x-->b</div>");
    p.ids(&["d"]);
    assert_eq!(p.num("d.childNodes.length"), 3.0);
    p.js("d.normalize();");
    assert_eq!(p.num("d.childNodes.length"), 3.0); // the comment blocks a merge
    p.js("d.appendChild(document.createTextNode('c')); d.normalize();");
    assert_eq!(p.text("d.lastChild.nodeValue"), "bc");
    p.js("d.firstChild.nodeValue = 'A';");
    assert_eq!(p.text("d.textContent"), "Abc");
}

#[test]
fn detach_is_not_dangling() {
    let mut p = Page::html("<div id=d><b id=b>x</b></div>");
    p.ids(&["b"]);
    p.js("const b = document.getElementById('b'); b.remove();");
    assert!(p.yes("b.parentNode === null"));
    assert_eq!(p.text("b.textContent"), "x");
    // re-attaching the very same wrapper works
    p.js("document.body.appendChild(b);");
    assert!(p.yes("document.getElementById('b') === b"));
    p.js("b.remove();");
    assert!(p.yes("b.closest('div') === null"));
}

#[test]
fn has_child_nodes_and_root() {
    let mut p = Page::html("<div id=d><b></b></div><i id=i></i>");
    p.ids(&["d", "i"]);
    assert!(p.yes("d.hasChildNodes()"));
    assert!(!p.yes("i.hasChildNodes()"));
    assert!(p.yes("d.getRootNode() === document"));
    assert!(p.yes("document.documentElement.getRootNode() === document"));
}

#[test]
fn serialization_details() {
    let mut p = Page::html("<div id=d>a&amp;b<img src=x></div>");
    p.ids(&["d"]);
    assert_eq!(p.text("d.textContent"), "a&b");
    assert!(p.text("d.innerHTML").contains("<img src=\"x\">"));
    p.js("d.innerHTML = '<span>1</span>'");
    assert_eq!(p.text("d.innerHTML"), "<span>1</span>");
    assert_eq!(p.text_of("div"), "1");
    assert_eq!(p.outer_of("div"), "<div id=\"d\"><span>1</span></div>");
}

#[test]
fn script_written_dom_is_visible_without_js() {
    // the browser reads the arena after the scripts: what they write must be there
    let mut p = Page::html("<div id=d></div><script src=x.js></script>");
    let report = p.run(&[(
        "http://example.test/p/x.js",
        "document.getElementById('d').innerHTML = '<h1>Hi</h1>';",
    )]);
    assert_eq!(report.executed, 1);
    assert!(report.clean());
    assert_eq!(p.dom_of(|dom| dom.get_elements_by_tag("h1").len()), 1);
    assert_eq!(p.text_of("h1"), "Hi");
}
