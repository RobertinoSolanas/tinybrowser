//! Selector matching against the DOM arena.
//!
//! Matching runs right-to-left: the right-most compound of a selector is the
//! cheap filter, and the [`crate::cascade`] rule index buckets rules by exactly
//! that compound. Matching is allocation free — `parts` is
//! `Compound (Combinator Compound)*`, so we walk it with indices.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use tb_css::selector::{AttrOp, Combinator, ComplexSelector, SelectorPart, Simple};
use tb_dom::{Dom, NodeId};

/// Interaction state used by `:hover`, `:focus`, `:active`.
#[derive(Debug, Clone, Copy, Default)]
pub struct MatchState {
    pub hover: Option<NodeId>,
    pub focus: Option<NodeId>,
    pub active: Option<NodeId>,
}

/// Selector lists inside `:not()`/`:is()`/`:has()` are parsed once per distinct
/// argument; the cascade would otherwise re-parse them for every element.
thread_local! {
    static ARGS_CACHE: RefCell<HashMap<String, Rc<Vec<ComplexSelector>>>> =
        RefCell::new(HashMap::new());
}

fn args_for(args: &str) -> Rc<Vec<ComplexSelector>> {
    ARGS_CACHE.with(|c| {
        if let Some(v) = c.borrow().get(args) {
            return v.clone();
        }
        let v = Rc::new(tb_css::selector::parse_selector_list(args));
        c.borrow_mut().insert(args.to_string(), v.clone());
        v
    })
}

fn compound_of(part: &SelectorPart) -> Option<&Vec<Simple>> {
    match part {
        SelectorPart::Compound(c) => Some(c),
        _ => None,
    }
}

/// Does `node` match the selector?
pub fn matches(dom: &Dom, node: NodeId, sel: &ComplexSelector, state: &MatchState) -> bool {
    if dom.tag(node).is_none() {
        return false;
    }
    let parts = &sel.parts;
    let mut ci = match parts.iter().rposition(|p| compound_of(p).is_some()) {
        Some(i) => i,
        None => return false,
    };
    if !matches_compound(dom, node, compound_of(&parts[ci]).unwrap(), state) {
        return false;
    }
    let mut current = node;
    while ci > 0 {
        // the combinator directly left of the current compound (implicit descendant)
        let mut j = ci - 1;
        let combinator = match &parts[j] {
            SelectorPart::Combinator(k) => {
                if j == 0 {
                    // malformed selector: combinator with nothing on the left
                    return false;
                }
                j -= 1;
                while j > 0 && compound_of(&parts[j]).is_none() {
                    j -= 1;
                }
                *k
            }
            _ => Combinator::Descendant,
        };
        let target = match compound_of(&parts[j]) {
            Some(c) => c,
            None => return false,
        };
        match find_ancestor_match(dom, current, target, combinator, state) {
            Some(n) => {
                current = n;
                ci = j;
            }
            None => return false,
        }
    }
    true
}

fn find_ancestor_match(
    dom: &Dom,
    from: NodeId,
    target: &[Simple],
    combinator: Combinator,
    state: &MatchState,
) -> Option<NodeId> {
    match combinator {
        Combinator::Descendant => {
            let mut cur = dom.parent(from);
            while let Some(c) = cur {
                if matches_compound(dom, c, target, state) {
                    return Some(c);
                }
                cur = dom.parent(c);
            }
            None
        }
        Combinator::Child => {
            let p = dom.parent(from)?;
            matches_compound(dom, p, target, state).then_some(p)
        }
        Combinator::NextSibling => {
            let s = dom.previous_element_sibling(from)?;
            matches_compound(dom, s, target, state).then_some(s)
        }
        Combinator::SubsequentSibling => {
            let mut s = dom.previous_element_sibling(from);
            while let Some(x) = s {
                if matches_compound(dom, x, target, state) {
                    return Some(x);
                }
                s = dom.previous_element_sibling(x);
            }
            None
        }
    }
}

/// Every simple selector of a compound must hold.
pub fn matches_compound(dom: &Dom, node: NodeId, compound: &[Simple], state: &MatchState) -> bool {
    compound
        .iter()
        .all(|s| matches_simple(dom, node, s, state))
}

/// One simple selector.
pub fn matches_simple(dom: &Dom, node: NodeId, s: &Simple, state: &MatchState) -> bool {
    let Some(el) = dom.element(node) else {
        return false;
    };
    match s {
        Simple::Universal => true,
        Simple::Type(t) => el.tag == *t,
        Simple::Id(id) => el.element_id() == Some(id.as_str()),
        Simple::Class(c) => el.has_class(c),
        Simple::Attr {
            name,
            op,
            value,
            insensitive,
        } => {
            let Some(actual) = el.attr(name) else {
                return false;
            };
            let (a, v) = if *insensitive {
                (
                    actual.to_ascii_lowercase(),
                    value.to_ascii_lowercase(),
                )
            } else {
                (actual.to_string(), value.clone())
            };
            match op {
                AttrOp::Exists => true,
                AttrOp::Equals => a == v,
                AttrOp::Tilde => a.split_whitespace().any(|p| p == v),
                AttrOp::Caret => !v.is_empty() && a.starts_with(v.as_str()),
                AttrOp::Dollar => !v.is_empty() && a.ends_with(v.as_str()),
                AttrOp::Star => !v.is_empty() && a.contains(v.as_str()),
                AttrOp::Hyphen => a == v || a.starts_with(&format!("{v}-")),
            }
        }
        // pseudo elements never match an element directly (the cascade strips them)
        Simple::PseudoElement { .. } => false,
        Simple::PseudoClass { name, args } => matches_pseudo(dom, node, name, args, state),
    }
}

fn matches_pseudo(dom: &Dom, node: NodeId, name: &str, args: &str, state: &MatchState) -> bool {
    let el = match dom.element(node) {
        Some(e) => e,
        None => return false,
    };
    match name {
        "root" => dom
            .parent(node)
            .map(|p| dom.tag(p).is_none())
            .unwrap_or(false),
        "first-child" => dom.previous_element_sibling(node).is_none(),
        "last-child" => dom.next_element_sibling(node).is_none(),
        "only-child" => {
            dom.previous_element_sibling(node).is_none() && dom.next_element_sibling(node).is_none()
        }
        "first-of-type" => {
            let tag = el.tag.clone();
            let mut cur = dom.previous_element_sibling(node);
            while let Some(c) = cur {
                if dom.tag(c) == Some(tag.as_str()) {
                    return false;
                }
                cur = dom.previous_element_sibling(c);
            }
            true
        }
        "last-of-type" => {
            let tag = el.tag.clone();
            let mut cur = dom.next_element_sibling(node);
            while let Some(c) = cur {
                if dom.tag(c) == Some(tag.as_str()) {
                    return false;
                }
                cur = dom.next_element_sibling(c);
            }
            true
        }
        "only-of-type" => {
            let tag = el.tag.clone();
            let parent = match dom.parent(node) {
                Some(p) => p,
                None => return false,
            };
            dom.children(parent)
                .iter()
                .filter(|c| dom.tag(**c) == Some(tag.as_str()))
                .count()
                == 1
        }
        "nth-child" => nth(args).is_some_and(|(a, b)| nth_hit(a, b, dom.element_index(node) as i32)),
        "nth-last-child" => nth(args).is_some_and(|(a, b)| {
            let total = dom.element_sibling_count(node) as i32;
            let i = total - dom.element_index(node) as i32 + 1;
            nth_hit(a, b, i)
        }),
        "nth-of-type" => nth(args).is_some_and(|(a, b)| {
            let tag = el.tag.clone();
            let mut i = 1;
            let mut cur = dom.previous_element_sibling(node);
            while let Some(c) = cur {
                if dom.tag(c) == Some(tag.as_str()) {
                    i += 1;
                }
                cur = dom.previous_element_sibling(c);
            }
            nth_hit(a, b, i)
        }),
        "empty" => dom.children(node).into_iter().all(|c| match dom.get(c) {
            // Selectors 4: any text (even whitespace) makes it non-empty;
            // comments are ignored.
            Some(n) => match &n.kind {
                tb_dom::NodeKind::Text(t) => t.is_empty(),
                tb_dom::NodeKind::Comment(_) => true,
                _ => false,
            },
            None => true,
        }),
        "not" => !args_for(args)
            .iter()
            .any(|s| matches(dom, node, s, state)),
        "is" | "where" | "matches" => args_for(args)
            .iter()
            .any(|s| matches(dom, node, s, state)),
        // `:has()` accepts relative selectors: `:has(> img)` is the common case.
        "has" => {
            let a = args.trim();
            let (comb, rest) = if let Some(r) = a.strip_prefix('>') {
                (Some(Combinator::Child), r)
            } else if let Some(r) = a.strip_prefix('+') {
                (Some(Combinator::NextSibling), r)
            } else if let Some(r) = a.strip_prefix('~') {
                (Some(Combinator::SubsequentSibling), r)
            } else {
                (None, a)
            };
            let list = args_for(rest);
            let candidates: Vec<NodeId> = match comb {
                None => dom
                    .descendants(node)
                    .into_iter()
                    .skip(1)
                    .filter(|d| dom.tag(*d).is_some())
                    .collect(),
                Some(Combinator::Child) => dom.children(node),
                Some(Combinator::NextSibling) => {
                    dom.next_element_sibling(node).into_iter().collect()
                }
                _ => {
                    let mut v = Vec::new();
                    let mut cur = dom.next_element_sibling(node);
                    while let Some(c) = cur {
                        v.push(c);
                        cur = dom.next_element_sibling(c);
                    }
                    v
                }
            };
            candidates
                .iter()
                .any(|d| list.iter().any(|s| matches(dom, *d, s, state)))
        }
        "link" | "any-link" => {
            matches!(el.tag.as_str(), "a" | "area" | "link") && el.has_attr("href")
        }
        "visited" => false,
        "hover" => state.hover == Some(node),
        "active" => state.active == Some(node),
        "focus" => state.focus == Some(node),
        "focus-within" => state.focus.map(|f| dom.contains(node, f)).unwrap_or(false),
        "checked" => {
            (matches!(el.tag.as_str(), "input" | "textarea") && el.has_attr("checked"))
                || (el.tag == "option" && el.has_attr("selected"))
        }
        "disabled" => el.has_attr("disabled") && is_form_el(&el.tag),
        "enabled" => !el.has_attr("disabled") && is_form_el(&el.tag),
        "read-only" => el.has_attr("readonly"),
        "read-write" => !el.has_attr("readonly"),
        "required" => el.has_attr("required"),
        "optional" => !el.has_attr("required"),
        "target" => false,
        "lang" => {
            let want = args.trim().trim_matches('"').to_ascii_lowercase();
            dom.ancestors(node, true)
                .into_iter()
                .filter_map(|a| dom.attr(a, "lang"))
                .find(|l| !l.is_empty())
                .map(|l| {
                    let l = l.to_ascii_lowercase();
                    l == want || l.starts_with(&format!("{want}-"))
                })
                .unwrap_or(false)
        }
        // Unknown pseudo classes never match — safer than matching everything.
        _ => false,
    }
}

fn is_form_el(tag: &str) -> bool {
    matches!(
        tag,
        "input" | "select" | "textarea" | "button" | "option" | "optgroup" | "fieldset"
    )
}

/// Parse `2n+1`, `odd`, `even`, `3`, `-n+2`.
pub fn nth(arg: &str) -> Option<(i32, i32)> {
    let s = arg.trim().to_ascii_lowercase().replace(' ', "");
    if s.is_empty() {
        return None;
    }
    match s.as_str() {
        "odd" => return Some((2, 1)),
        "even" => return Some((2, 0)),
        _ => {}
    }
    if let Some(npos) = s.find('n') {
        let a_raw = &s[..npos];
        let a = match a_raw {
            "" => 1,
            "-" => -1,
            "+" => 1,
            other => other.parse::<i32>().ok()?,
        };
        let rest = &s[npos + 1..];
        let b = if rest.is_empty() {
            0
        } else {
            rest.parse::<i32>().ok()?
        };
        Some((a, b))
    } else {
        Some((0, s.parse::<i32>().ok()?))
    }
}

/// `an+b` hit test (1-based index).
pub fn nth_hit(a: i32, b: i32, index: i32) -> bool {
    if index < 1 {
        return false;
    }
    if a == 0 {
        return index == b;
    }
    let k = index - b;
    k % a == 0 && k / a >= 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use tb_html::parse_document;

    fn dom(html: &str) -> Dom {
        parse_document(html)
    }

    fn sel(s: &str) -> ComplexSelector {
        tb_css::selector::parse_selector_list(s).remove(0)
    }

    fn matching(html: &str, selector: &str) -> Vec<NodeId> {
        let dom = dom(html);
        let s = sel(selector);
        dom.elements()
            .into_iter()
            .filter(|n| matches(&dom, *n, &s, &MatchState::default()))
            .collect()
    }

    #[test]
    fn type_id_class_and_descendant() {
        let html = "<div id=a class=x><p class=y>1</p><span><p class=y>2</p></span></div>";
        assert_eq!(matching(html, "p").len(), 2);
        assert_eq!(matching(html, "#a").len(), 1);
        assert_eq!(matching(html, ".y").len(), 2);
        assert_eq!(matching(html, "div .y").len(), 2);
        assert_eq!(matching(html, "div > p").len(), 1);
        assert_eq!(matching(html, "span > p").len(), 1);
        assert_eq!(matching(html, "div p.y").len(), 2);
        assert_eq!(matching(html, "p.z").len(), 0);
        assert_eq!(matching(html, "div.x > span > p").len(), 1);
    }

    #[test]
    fn sibling_combinators() {
        let html = "<ul><li>a</li><li>b</li><li class=t>c</li><li>d</li></ul>";
        assert_eq!(matching(html, ".t + li").len(), 1);
        assert_eq!(matching(html, "li ~ .t").len(), 1);
        assert_eq!(matching(html, ".t ~ li").len(), 1);
        assert_eq!(matching(html, "ul > li + li").len(), 3);
    }

    #[test]
    fn attribute_operators() {
        let html = r#"<a href="https://x.de/a">1</a><a href="/b">2</a><a download>x</a><a lang="de-DE">3</a>"#;
        assert_eq!(matching(html, "a[href]").len(), 2);
        assert_eq!(matching(html, "a[href^='https']").len(), 1);
        assert_eq!(matching(html, "a[href$='.de/a']").len(), 1);
        assert_eq!(matching(html, "a[href*='b']").len(), 1);
        assert_eq!(matching(html, "a[download]").len(), 1);
        assert_eq!(matching(html, "a[lang|='de']").len(), 1);
        assert_eq!(matching(html, "a[href|='https']").len(), 0, "|= needs `https-`");
    }

    #[test]
    fn structural_pseudo_classes() {
        let html = "<ul><li>1</li><li>2</li><li>3</li><li>4</li></ul>";
        assert_eq!(matching(html, "li:first-child").len(), 1);
        assert_eq!(matching(html, "li:last-child").len(), 1);
        assert_eq!(matching(html, "li:nth-child(2)").len(), 1);
        assert_eq!(matching(html, "li:nth-child(odd)").len(), 2);
        assert_eq!(matching(html, "li:nth-child(even)").len(), 2);
        assert_eq!(matching(html, "li:nth-child(2n+1)").len(), 2);
        assert_eq!(matching(html, "li:nth-last-child(1)").len(), 1);
        assert_eq!(matching(html, "li:first-of-type").len(), 1);
        let empty = dom("<p></p><p> </p><p>x</p><p><!--c--></p>");
        let s = sel("p:empty");
        let hits: Vec<_> = empty
            .elements()
            .into_iter()
            .filter(|n| matches(&empty, *n, &s, &MatchState::default()))
            .collect();
        assert_eq!(hits.len(), 2, "text (even whitespace) is not empty, comments are ignored");
    }

    #[test]
    fn nth_math() {
        assert_eq!(nth("2n+1"), Some((2, 1)));
        assert_eq!(nth("odd"), Some((2, 1)));
        assert_eq!(nth("3"), Some((0, 3)));
        assert_eq!(nth("-n+3"), Some((-1, 3)));
        assert!(nth_hit(2, 1, 1));
        assert!(!nth_hit(2, 1, 2));
        assert!(nth_hit(0, 3, 3));
        assert!(nth_hit(-1, 3, 1));
        assert!(!nth_hit(-1, 3, 5));
    }

    #[test]
    fn functional_pseudo_classes() {
        let html = "<div class=a><span>x</span></div><div class=b><em>y</em></div>";
        assert_eq!(matching(html, "div:not(.a)").len(), 1);
        assert_eq!(matching(html, ".a > span").len(), 1);
        assert_eq!(matching(html, "div:has(> span)").len(), 1);
        assert_eq!(matching(html, ":is(.a, .b)").len(), 2);
        assert_eq!(matching(html, ":where(.a)").len(), 1);
    }

    #[test]
    fn link_and_form_pseudo_classes() {
        let html = r#"<a href="/x">l</a><a>n</a><input checked><input disabled>"#;
        assert_eq!(matching(html, "a:link").len(), 1);
        assert_eq!(matching(html, "input:checked").len(), 1);
        assert_eq!(matching(html, "input:disabled").len(), 1);
        assert_eq!(matching(html, "input:enabled").len(), 1);
    }

    #[test]
    fn hover_state_is_configurable() {
        let dom = dom("<p id=x>hi</p>");
        let p = dom.get_element_by_id("x").unwrap();
        let s = sel("p:hover");
        assert!(!matches(&dom, p, &s, &MatchState::default()));
        let st = MatchState {
            hover: Some(p),
            ..Default::default()
        };
        assert!(matches(&dom, p, &s, &st));
    }

    #[test]
    fn root_and_lang() {
        let html = "<html lang='de-DE'><body><p>x</p></body></html>";
        assert_eq!(matching(html, ":root").len(), 1);
        assert_eq!(matching(html, "p:lang(de)").len(), 1);
        assert_eq!(matching(html, "p:lang(fr)").len(), 0);
    }

    #[test]
    fn pseudo_elements_do_not_match_elements() {
        assert_eq!(matching("<p>x</p>", "p::before").len(), 0);
        assert_eq!(matching("<p>x</p>", "p:first-line").len(), 0);
    }

    #[test]
    fn never_panics_on_weird_selectors() {
        let d = dom("<div>x</div>");
        for s in [
            ":not(", "nth-child(", "a:::b", "[[[]]", ":lang()", "###", "*+*", ">", "+", ":is(:not())",
        ] {
            let sel = tb_css::selector::parse_selector_list(s);
            for c in sel {
                for n in d.elements() {
                    let _ = matches(&d, n, &c, &MatchState::default());
                }
            }
        }
    }
}
