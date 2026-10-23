//! CSS selectors against the live DOM — `querySelector(All)`, `matches`,
//! `closest`.
//!
//! Matching itself is [`tb_style::matches`] (right-to-left, allocation free);
//! this module adds the DOM-side plumbing: parse once per distinct selector
//! string, walk the right set of candidates, keep document order.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use tb_css::selector::{parse_selector_list, ComplexSelector};
use tb_dom::{Dom, NodeId};
use tb_style::{matches, MatchState};

thread_local! {
    static CACHE: RefCell<HashMap<String, Rc<Vec<ComplexSelector>>>> = RefCell::new(HashMap::new());
}

/// Parse a selector list, memoised: pages call `querySelectorAll('.x')` in
/// loops and parsing 20 characters 5 000 times is real work.
pub fn selector_list(sel: &str) -> Rc<Vec<ComplexSelector>> {
    CACHE.with(|c| {
        if let Some(v) = c.borrow().get(sel) {
            return v.clone();
        }
        let v = Rc::new(parse_selector_list(sel));
        // keep the cache from growing without bound on generated selectors
        if c.borrow().len() > 4096 {
            c.borrow_mut().clear();
        }
        c.borrow_mut().insert(sel.to_string(), v.clone());
        v
    })
}

/// Every element below `root` (excluding it) matching `sel`, document order.
pub fn query_all(dom: &Dom, state: &MatchState, root: NodeId, sel: &str) -> Vec<NodeId> {
    let list = selector_list(sel);
    if list.is_empty() {
        return Vec::new();
    }
    dom.descendants(root)
        .into_iter()
        .skip(1)
        .filter(|n| dom.tag(*n).is_some())
        .filter(|n| list.iter().any(|s| matches(dom, *n, s, state)))
        .collect()
}

pub fn query_first(dom: &Dom, state: &MatchState, root: NodeId, sel: &str) -> Option<NodeId> {
    let list = selector_list(sel);
    if list.is_empty() {
        return None;
    }
    dom.descendants(root)
        .into_iter()
        .skip(1)
        .filter(|n| dom.tag(*n).is_some())
        .find(|n| list.iter().any(|s| matches(dom, *n, s, state)))
}

/// `element.matches(sel)` — the element itself may match.
pub fn matches_selector(dom: &Dom, state: &MatchState, node: NodeId, sel: &str) -> bool {
    let list = selector_list(sel);
    list.iter().any(|s| matches(dom, node, s, state))
}

/// `element.closest(sel)` — self, then parents, nearest first.
pub fn closest(dom: &Dom, state: &MatchState, node: NodeId, sel: &str) -> Option<NodeId> {
    let list = selector_list(sel);
    dom.ancestors(node, true)
        .into_iter()
        .find(|&a| dom.tag(a).is_some() && list.iter().any(|s| matches(dom, a, s, state)))
}
