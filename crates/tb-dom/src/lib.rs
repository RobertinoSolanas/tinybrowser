//! # tb-dom — the DOM aggregate
//!
//! The document tree is an arena (a `Vec` of slots) addressed through
//! [`NodeId`] handles that carry a generation counter. That gives us
//!
//! * O(1) child/sibling navigation (`parent`, `first_child`, `next`),
//! * cheap stable handles that JavaScript can hold on to,
//! * safe reuse of removed nodes: a stale handle never points at a different
//!   node, it simply stops resolving.
//!
//! There is no reference counting and no garbage collector: owning the `Dom`
//! owns the document.

use std::fmt;

/// Handle to a node in the arena. `{idx, gen}` — never reuse a stale handle.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId {
    idx: u32,
    gen: u32,
}

impl NodeId {
    /// A handle that never resolves.
    pub const NONE: NodeId = NodeId {
        idx: u32::MAX,
        gen: 0,
    };

    pub fn is_none(self) -> bool {
        self.idx == u32::MAX
    }

    /// Arena slot index — use as a dense array index (style maps, layout boxes).
    pub fn idx_usize(self) -> usize {
        self.idx as usize
    }

    /// Raw handle parts (for debugging / serialisation).
    pub fn raw(self) -> (u32, u32) {
        (self.idx, self.gen)
    }

    /// Rebuild a handle from its raw parts. Used by host bindings that have to
    /// park a node reference inside a JavaScript value; a handle that was freed
    /// simply stops resolving, it never points at a different node.
    pub const fn from_raw(idx: u32, gen: u32) -> NodeId {
        NodeId { idx, gen }
    }
}

impl fmt::Debug for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_none() {
            f.write_str("NodeId(none)")
        } else {
            write!(f, "#{}:{}", self.idx, self.gen)
        }
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

/// The namespaces we distinguish (self-closing rules differ).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Namespace {
    Html,
    Svg,
    MathMl,
    Xml,
}

/// An element node.
#[derive(Debug, Clone)]
pub struct Element {
    /// lower case local name (`div`)
    pub tag: String,
    pub namespace: Namespace,
    /// lower case attribute names, original values
    pub attrs: Vec<Attr>,
}

/// An attribute (`name="value"`).
#[derive(Debug, Clone, PartialEq)]
pub struct Attr {
    pub name: String,
    pub value: String,
}

impl Element {
    pub fn new(tag: &str) -> Element {
        Element {
            tag: tag.to_ascii_lowercase(),
            namespace: Namespace::Html,
            attrs: Vec::new(),
        }
    }

    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|a| a.name == name)
            .map(|a| a.value.as_str())
    }

    pub fn has_attr(&self, name: &str) -> bool {
        self.attrs.iter().any(|a| a.name == name)
    }

    pub fn set_attr(&mut self, name: &str, value: impl Into<String>) {
        let name = name.to_ascii_lowercase();
        let value = value.into();
        match self.attrs.iter_mut().find(|a| a.name == name) {
            Some(a) => a.value = value,
            None => self.attrs.push(Attr { name, value }),
        }
    }

    pub fn remove_attr(&mut self, name: &str) {
        let name = name.to_ascii_lowercase();
        self.attrs.retain(|a| a.name != name);
    }

    /// Split the `class` attribute.
    pub fn classes(&self) -> Vec<&str> {
        self.attr("class")
            .map(|c| c.split_whitespace().collect())
            .unwrap_or_default()
    }

    pub fn has_class(&self, class: &str) -> bool {
        self.attr("class")
            .map(|c| c.split_whitespace().any(|k| k == class))
            .unwrap_or(false)
    }

    /// The `id` attribute.
    pub fn element_id(&self) -> Option<&str> {
        self.attr("id").filter(|s| !s.is_empty())
    }

    /// `data-*` attributes as camelCase map (for `dataset`).
    pub fn dataset(&self) -> Vec<(String, String)> {
        self.attrs
            .iter()
            .filter(|a| a.name.starts_with("data-"))
            .map(|a| (kebab_to_camel(&a.name[5..]), a.value.clone()))
            .collect()
    }
}

pub fn kebab_to_camel(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut upper = false;
    for c in s.chars() {
        if c == '-' {
            upper = true;
        } else if upper {
            out.extend(c.to_uppercase());
            upper = false;
        } else {
            out.push(c);
        }
    }
    out
}

pub fn camel_to_kebab(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for c in s.chars() {
        if c.is_ascii_uppercase() {
            out.push('-');
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// What a node is.
#[derive(Debug, Clone)]
pub enum NodeKind {
    Document,
    Doctype {
        name: String,
        public_id: String,
        system_id: String,
    },
    Element(Element),
    Text(String),
    Comment(String),
    ProcessingInstruction {
        target: String,
        data: String,
    },
}

impl NodeKind {
    pub fn type_name(&self) -> &'static str {
        match self {
            NodeKind::Document => "#document",
            NodeKind::Doctype { .. } => "#doctype",
            NodeKind::Element(_) => "element",
            NodeKind::Text(_) => "text",
            NodeKind::Comment(_) => "comment",
            NodeKind::ProcessingInstruction { .. } => "processing-instruction",
        }
    }
}

/// One node of the tree.
#[derive(Debug, Clone)]
pub struct Node {
    pub kind: NodeKind,
    pub parent: Option<NodeId>,
    pub first: Option<NodeId>,
    pub last: Option<NodeId>,
    pub prev: Option<NodeId>,
    pub next: Option<NodeId>,
    /// Number of children (kept in sync).
    pub child_count: u32,
    /// Marks detached nodes (removed from the tree but still addressable).
    pub attached: bool,
}

impl Node {
    pub fn new(kind: NodeKind) -> Node {
        Node {
            kind,
            parent: None,
            first: None,
            last: None,
            prev: None,
            next: None,
            child_count: 0,
            attached: false,
        }
    }

    pub fn is_element(&self) -> bool {
        matches!(self.kind, NodeKind::Element(_))
    }
    pub fn is_text(&self) -> bool {
        matches!(self.kind, NodeKind::Text(_))
    }
    pub fn is_comment(&self) -> bool {
        matches!(self.kind, NodeKind::Comment(_))
    }
    pub fn is_document(&self) -> bool {
        matches!(self.kind, NodeKind::Document)
    }
    pub fn as_element(&self) -> Option<&Element> {
        match &self.kind {
            NodeKind::Element(e) => Some(e),
            _ => None,
        }
    }
    pub fn as_element_mut(&mut self) -> Option<&mut Element> {
        match &mut self.kind {
            NodeKind::Element(e) => Some(e),
            _ => None,
        }
    }
    pub fn tag(&self) -> Option<&str> {
        self.as_element().map(|e| e.tag.as_str())
    }
    pub fn text(&self) -> Option<&str> {
        match &self.kind {
            NodeKind::Text(t) => Some(t),
            _ => None,
        }
    }
}

#[derive(Clone)]
struct Slot {
    node: Node,
    gen: u32,
    alive: bool,
}

/// The document tree.
#[derive(Clone)]
pub struct Dom {
    slots: Vec<Slot>,
    free: Vec<u32>,
    document: NodeId,
    /// `<!doctype ...>` info, kept next to the document for quick access.
    pub doctype: String,
    /// Whether the document is in quirks mode (no/legacy doctype).
    pub quirks: bool,
}

impl Default for Dom {
    fn default() -> Self {
        Self::new()
    }
}

impl Dom {
    /// An empty document (just the `#document` node).
    pub fn new() -> Dom {
        let mut dom = Dom {
            slots: Vec::new(),
            free: Vec::new(),
            document: NodeId::NONE,
            doctype: String::new(),
            // No doctype means quirks mode; the parser clears this.
            quirks: true,
        };
        let doc = dom.allocate(NodeKind::Document);
        dom.slots[doc.idx as usize].node.attached = true;
        dom.document = doc;
        dom
    }

    fn allocate(&mut self, kind: NodeKind) -> NodeId {
        let node = Node::new(kind);
        if let Some(idx) = self.free.pop() {
            let slot = &mut self.slots[idx as usize];
            slot.node = node;
            slot.alive = true;
            NodeId { idx, gen: slot.gen }
        } else {
            let idx = self.slots.len() as u32;
            self.slots.push(Slot {
                node,
                gen: 0,
                alive: true,
            });
            NodeId { idx, gen: 0 }
        }
    }

    /// The `#document` node.
    pub fn document(&self) -> NodeId {
        self.document
    }

    /// Number of arena slots (>= live node count; freed slots are reused).
    pub fn max_index(&self) -> usize {
        self.slots.len()
    }

    /// Resolve a handle.
    pub fn get(&self, id: NodeId) -> Option<&Node> {
        let slot = self.slots.get(id.idx as usize)?;
        if slot.alive && slot.gen == id.gen {
            Some(&slot.node)
        } else {
            None
        }
    }

    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        let slot = self.slots.get_mut(id.idx as usize)?;
        if slot.alive && slot.gen == id.gen {
            Some(&mut slot.node)
        } else {
            None
        }
    }

    /// `true` when the handle still refers to a live node.
    pub fn exists(&self, id: NodeId) -> bool {
        self.get(id).is_some()
    }

    pub fn element(&self, id: NodeId) -> Option<&Element> {
        self.get(id)?.as_element()
    }

    pub fn element_mut(&mut self, id: NodeId) -> Option<&mut Element> {
        self.get_mut(id)?.as_element_mut()
    }

    pub fn tag(&self, id: NodeId) -> Option<&str> {
        self.element(id).map(|e| e.tag.as_str())
    }

    /// Total nodes in the arena (including detached ones).
    pub fn node_count(&self) -> usize {
        self.slots.iter().filter(|s| s.alive).count()
    }

    pub fn element_count(&self) -> usize {
        self.slots
            .iter()
            .filter(|s| s.alive && s.node.is_element())
            .count()
    }

    // ------------------------------------------------------------- factory --

    pub fn create_element(&mut self, tag: &str) -> NodeId {
        self.allocate(NodeKind::Element(Element::new(tag)))
    }

    pub fn create_element_ns(&mut self, tag: &str, ns: Namespace) -> NodeId {
        let mut e = Element::new(tag);
        e.namespace = ns;
        self.allocate(NodeKind::Element(e))
    }

    pub fn create_text(&mut self, text: &str) -> NodeId {
        self.allocate(NodeKind::Text(text.to_string()))
    }

    pub fn create_comment(&mut self, text: &str) -> NodeId {
        self.allocate(NodeKind::Comment(text.to_string()))
    }

    pub fn append_doctype(&mut self, name: &str, public_id: &str, system_id: &str) {
        self.doctype = name.to_string();
        let id = self.allocate(NodeKind::Doctype {
            name: name.to_string(),
            public_id: public_id.to_string(),
            system_id: system_id.to_string(),
        });
        self.appendChild(self.document, id);
    }

    // ------------------------------------------------------------- tree ops --

    /// Append `child` to `parent` (moves it if it already has a parent).
    #[allow(non_snake_case)]
    pub fn appendChild(&mut self, parent: NodeId, child: NodeId) {
        self.detach(child);
        let Some(pn) = self.get_mut(parent) else {
            return;
        };
        let last = pn.last;
        pn.last = Some(child);
        if pn.first.is_none() {
            pn.first = Some(child);
        }
        pn.child_count += 1;
        if let Some(l) = last {
            if let Some(ln) = self.get_mut(l) {
                ln.next = Some(child);
            }
        }
        if let Some(cn) = self.get_mut(child) {
            cn.parent = Some(parent);
            cn.prev = last;
            cn.next = None;
            cn.attached = true;
        }
    }

    /// Insert `child` before `reference` (appends when `reference` is `None`).
    #[allow(non_snake_case)]
    pub fn insertBefore(&mut self, parent: NodeId, child: NodeId, reference: Option<NodeId>) {
        let Some(reference) = reference.filter(|r| self.exists(*r)) else {
            self.appendChild(parent, child);
            return;
        };
        self.detach(child);
        let prev = self.get(reference).and_then(|n| n.prev);
        {
            let Some(rn) = self.get_mut(reference) else {
                self.appendChild(parent, child);
                return;
            };
            rn.prev = Some(child);
            if let Some(p) = prev {
                if let Some(pn) = self.get_mut(p) {
                    pn.next = Some(child);
                }
            }
        }
        {
            let Some(pn) = self.get_mut(parent) else {
                return;
            };
            pn.child_count += 1;
            if pn.first == Some(reference) || pn.first.is_none() {
                pn.first = Some(child);
            }
            if pn.last.is_none() {
                pn.last = Some(reference);
            }
        }
        if let Some(cn) = self.get_mut(child) {
            cn.parent = Some(parent);
            cn.prev = prev;
            cn.next = Some(reference);
            cn.attached = true;
        }
    }

    /// Unlink a node from its parent (keeps its subtree intact).
    pub fn detach(&mut self, child: NodeId) {
        let parent = match self.get(child).and_then(|n| n.parent) {
            Some(p) => p,
            None => return,
        };
        let (prev, next) = {
            let Some(cn) = self.get(child) else {
                return;
            };
            (cn.prev, cn.next)
        };
        if let Some(p) = prev {
            if let Some(pn) = self.get_mut(p) {
                pn.next = next;
            }
        }
        if let Some(n) = next {
            if let Some(nn) = self.get_mut(n) {
                nn.prev = prev;
            }
        }
        if let Some(pn) = self.get_mut(parent) {
            if pn.first == Some(child) {
                pn.first = next;
            }
            if pn.last == Some(child) {
                pn.last = prev;
            }
            pn.child_count = pn.child_count.saturating_sub(1);
        }
        if let Some(cn) = self.get_mut(child) {
            cn.parent = None;
            cn.prev = None;
            cn.next = None;
            cn.attached = false;
        }
    }

    /// Remove a node from the document.
    pub fn remove(&mut self, child: NodeId) {
        self.detach(child);
    }

    /// Remove all children of `node`.
    pub fn clear_children(&mut self, node: NodeId) {
        let kids: Vec<NodeId> = self.children(node);
        for k in kids {
            self.detach(k);
            self.free_subtree(k);
        }
    }

    fn free_subtree(&mut self, node: NodeId) {
        let kids: Vec<NodeId> = self.children(node);
        for k in kids {
            self.detach(k);
            self.free_subtree(k);
        }
        if let Some(n) = self.get_mut(node) {
            n.first = None;
            n.last = None;
            n.child_count = 0;
        }
        if let Some(slot) = self.slots.get_mut(node.idx as usize) {
            if slot.alive && slot.gen == node.gen {
                slot.alive = false;
                slot.gen = slot.gen.wrapping_add(1);
                self.free.push(node.idx);
            }
        }
    }

    /// `replaceChild(newChild, oldChild)`
    pub fn replace_child(&mut self, parent: NodeId, new: NodeId, old: NodeId) {
        self.insertBefore(parent, new, Some(old));
        self.remove(old);
    }

    // ------------------------------------------------------------ accessors --

    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.get(id)?.parent
    }

    pub fn children(&self, id: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut cur = self.get(id).and_then(|n| n.first);
        while let Some(c) = cur {
            out.push(c);
            cur = self.get(c).and_then(|n| n.next);
        }
        out
    }

    pub fn child_elements(&self, id: NodeId) -> Vec<NodeId> {
        self.children(id)
            .into_iter()
            .filter(|c| self.tag(*c).is_some())
            .collect()
    }

    pub fn first_child(&self, id: NodeId) -> Option<NodeId> {
        self.get(id)?.first
    }

    pub fn last_child(&self, id: NodeId) -> Option<NodeId> {
        self.get(id)?.last
    }

    pub fn next_sibling(&self, id: NodeId) -> Option<NodeId> {
        self.get(id)?.next
    }

    pub fn prev_sibling(&self, id: NodeId) -> Option<NodeId> {
        self.get(id)?.prev
    }

    pub fn next_element_sibling(&self, id: NodeId) -> Option<NodeId> {
        let mut cur = self.next_sibling(id);
        while let Some(c) = cur {
            if self.tag(c).is_some() {
                return Some(c);
            }
            cur = self.next_sibling(c);
        }
        None
    }

    pub fn previous_element_sibling(&self, id: NodeId) -> Option<NodeId> {
        let mut cur = self.prev_sibling(id);
        while let Some(c) = cur {
            if self.tag(c).is_some() {
                return Some(c);
            }
            cur = self.prev_sibling(c);
        }
        None
    }

    pub fn index_in_parent(&self, id: NodeId) -> usize {
        let mut i = 0;
        let mut cur = self.parent(id).and_then(|p| self.first_child(p));
        while let Some(c) = cur {
            if c == id {
                return i;
            }
            i += 1;
            cur = self.next_sibling(c);
        }
        0
    }

    /// 1-based index among *element* siblings (`:nth-child`).
    pub fn element_index(&self, id: NodeId) -> usize {
        let mut i = 0;
        let mut cur = self.parent(id).and_then(|p| self.first_child(p));
        while let Some(c) = cur {
            if self.tag(c).is_some() {
                i += 1;
                if c == id {
                    return i;
                }
            }
            cur = self.next_sibling(c);
        }
        0
    }

    /// Number of element siblings (`:last-child` maths).
    pub fn element_sibling_count(&self, id: NodeId) -> usize {
        let Some(p) = self.parent(id) else {
            return 1;
        };
        self.children(p)
            .into_iter()
            .filter(|c| self.tag(*c).is_some())
            .count()
    }

    // ----------------------------------------------------------- traversal --

    /// Pre-order walk over `id` and everything below it.
    pub fn descendants(&self, id: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut stack = vec![id];
        while let Some(c) = stack.pop() {
            out.push(c);
            let kids = self.children(c);
            for k in kids.into_iter().rev() {
                stack.push(k);
            }
        }
        out
    }

    /// Every element in document order.
    pub fn elements(&self) -> Vec<NodeId> {
        self.descendants(self.document)
            .into_iter()
            .filter(|n| self.tag(*n).is_some())
            .collect()
    }

    /// Depth of a node (document = 0).
    pub fn depth(&self, id: NodeId) -> usize {
        let mut d = 0;
        let mut cur = self.parent(id);
        while cur.is_some() {
            d += 1;
            cur = self.parent(cur.unwrap());
        }
        d
    }

    /// Ancestor chain (parent first), optionally including the node itself.
    pub fn ancestors(&self, id: NodeId, include_self: bool) -> Vec<NodeId> {
        let mut out = Vec::new();
        if include_self {
            out.push(id);
        }
        let mut cur = self.parent(id);
        while let Some(c) = cur {
            out.push(c);
            cur = self.parent(c);
        }
        out
    }

    /// Is `ancestor` an ancestor of `id`?
    pub fn contains(&self, ancestor: NodeId, id: NodeId) -> bool {
        self.ancestors(id, false).contains(&ancestor)
    }

    // --------------------------------------------------------- attributes ---

    pub fn attr(&self, id: NodeId, name: &str) -> Option<String> {
        self.element(id)
            .and_then(|e| e.attr(name))
            .map(|s| s.to_string())
    }

    pub fn set_attr(&mut self, id: NodeId, name: &str, value: impl Into<String>) {
        if let Some(e) = self.element_mut(id) {
            e.set_attr(name, value);
        }
    }

    pub fn remove_attr(&mut self, id: NodeId, name: &str) {
        if let Some(e) = self.element_mut(id) {
            e.remove_attr(name);
        }
    }

    pub fn has_attr(&self, id: NodeId, name: &str) -> bool {
        self.element(id).map(|e| e.has_attr(name)).unwrap_or(false)
    }

    /// `element.id`
    pub fn element_id(&self, id: NodeId) -> Option<String> {
        self.element(id)
            .and_then(|e| e.element_id())
            .map(|s| s.to_string())
    }

    /// `document.getElementById(id)` — first match in document order.
    pub fn get_element_by_id(&self, id: &str) -> Option<NodeId> {
        self.elements()
            .into_iter()
            .find(|n| self.element_id(*n).as_deref() == Some(id))
    }

    pub fn get_elements_by_tag(&self, tag: &str) -> Vec<NodeId> {
        let tag = tag.to_ascii_lowercase();
        if tag == "*" {
            return self.elements();
        }
        self.elements()
            .into_iter()
            .filter(|n| self.tag(*n) == Some(tag.as_str()))
            .collect()
    }

    pub fn get_elements_by_class(&self, class: &str) -> Vec<NodeId> {
        self.elements()
            .into_iter()
            .filter(|n| {
                self.element(*n)
                    .map(|e| e.has_class(class))
                    .unwrap_or(false)
            })
            .collect()
    }

    // --------------------------------------------------------------- text ---

    /// `textContent`: concatenated text of the whole subtree (DOM semantics:
    /// script and style content included).
    /// Text of a text node (`None` for any other node type).
    pub fn text(&self, id: NodeId) -> Option<String> {
        match self.get(id).map(|n| &n.kind) {
            Some(NodeKind::Text(t)) => Some(t.clone()),
            _ => None,
        }
    }

    pub fn text_content(&self, id: NodeId) -> String {
        let mut out = String::new();
        self.collect_text(id, &mut out, false);
        out
    }

    /// Text a human would read: `script`, `style` and `template` content is
    /// skipped (used for layout and for `tiny --text`).
    pub fn visible_text(&self, id: NodeId) -> String {
        let mut out = String::new();
        self.collect_text(id, &mut out, true);
        out
    }

    fn collect_text(&self, id: NodeId, out: &mut String, skip_invisible: bool) {
        if let Some(node) = self.get(id) {
            match &node.kind {
                NodeKind::Text(t) => {
                    out.push_str(t);
                    return;
                }
                NodeKind::Element(e)
                    if skip_invisible
                        && matches!(
                            e.tag.as_str(),
                            "script" | "style" | "template" | "head" | "title"
                        ) =>
                {
                    return;
                }
                _ => {}
            }
        }
        for c in self.children(id) {
            self.collect_text(c, out, skip_invisible);
        }
    }

    /// Replace the children of `id` with a single text node.
    pub fn set_text_content(&mut self, id: NodeId, text: &str) {
        self.clear_children(id);
        if !text.is_empty() {
            let t = self.create_text(text);
            self.appendChild(id, t);
        }
    }

    /// Direct text children only (used by inline layout).
    pub fn text_children(&self, id: NodeId) -> Vec<(NodeId, String)> {
        self.children(id)
            .into_iter()
            .filter_map(|c| {
                self.get(c)
                    .and_then(|n| n.text())
                    .map(|t| (c, t.to_string()))
            })
            .collect()
    }

    /// Merge adjacent text nodes (after DOM surgery).
    pub fn normalize(&mut self, id: NodeId) {
        let kids = self.children(id);
        let mut i = 0;
        while i + 1 < kids.len() {
            let a = kids[i];
            let b = kids[i + 1];
            let both_text = self.get(a).map(|n| n.is_text()) == Some(true)
                && self.get(b).map(|n| n.is_text()) == Some(true);
            if both_text {
                let merged = {
                    let mut s = self.get(a).and_then(|n| n.text()).unwrap_or("").to_string();
                    s.push_str(self.get(b).and_then(|n| n.text()).unwrap_or(""));
                    s
                };
                if let Some(n) = self.get_mut(a) {
                    n.kind = NodeKind::Text(merged);
                }
                self.detach(b);
                self.free_subtree(b);
                continue;
            }
            i += 1;
        }
    }

    // -------------------------------------------------------------- clone ---

    /// Deep copy a subtree (not attached to any parent).
    pub fn clone_node(&mut self, id: NodeId, deep: bool) -> NodeId {
        let Some(node) = self.get(id) else {
            return NodeId::NONE;
        };
        let kind = node.kind.clone();
        let copy = self.allocate(kind);
        if deep {
            for c in self.children(id) {
                let cc = self.clone_node(c, true);
                self.appendChild(copy, cc);
            }
        }
        copy
    }

    // -------------------------------------------------------- serialisation --

    /// `outerHTML` of a node.
    pub fn outer_html(&self, id: NodeId) -> String {
        let mut out = String::new();
        self.serialize(id, &mut out);
        out
    }

    /// `innerHTML` of a node.
    pub fn inner_html(&self, id: NodeId) -> String {
        let mut out = String::new();
        for c in self.children(id) {
            self.serialize(c, &mut out);
        }
        out
    }

    fn serialize(&self, id: NodeId, out: &mut String) {
        let Some(node) = self.get(id) else {
            return;
        };
        match &node.kind {
            NodeKind::Document => {
                for c in self.children(id) {
                    self.serialize(c, out);
                }
            }
            NodeKind::Doctype { name, .. } => {
                out.push_str(&format!("<!DOCTYPE {name}>"));
            }
            NodeKind::Text(t) => {
                let parent_tag = self.parent(id).and_then(|p| self.tag(p));
                match parent_tag {
                    // raw text parents must not be escaped
                    Some("script") | Some("style") | Some("textarea") | Some("title") => {
                        out.push_str(t)
                    }
                    _ => out.push_str(&escape_text(t)),
                }
            }
            NodeKind::Comment(c) => out.push_str(&format!("<!--{c}-->")),
            NodeKind::ProcessingInstruction { target, data } => {
                out.push_str(&format!("<?{target}{data}>"))
            }
            NodeKind::Element(e) => {
                out.push('<');
                out.push_str(&e.tag);
                for a in &e.attrs {
                    out.push(' ');
                    out.push_str(&a.name);
                    out.push_str("=\"");
                    out.push_str(&escape_attr(&a.value));
                    out.push('"');
                }
                out.push('>');
                if crate::void::is_void(&e.tag) {
                    return;
                }
                for c in self.children(id) {
                    self.serialize(c, out);
                }
                out.push_str("</");
                out.push_str(&e.tag);
                out.push('>');
            }
        }
    }

    /// A debug tree, e.g. for `--dom`.
    pub fn debug_tree(&self) -> String {
        fn walk(dom: &Dom, id: NodeId, depth: usize, out: &mut String) {
            let pad = "  ".repeat(depth);
            match dom.get(id).map(|n| &n.kind) {
                Some(NodeKind::Document) => out.push_str(&format!("{pad}#document\n")),
                Some(NodeKind::Doctype { name, .. }) => {
                    out.push_str(&format!("{pad}<!DOCTYPE {name}>\n"))
                }
                Some(NodeKind::Comment(c)) => out.push_str(&format!(
                    "{pad}<!-- {} -->\n",
                    tb_core::text::truncate(c, 40)
                )),
                Some(NodeKind::Text(t)) => {
                    let t = tb_core::text::collapse_whitespace(t);
                    if !t.is_empty() {
                        out.push_str(&format!(
                            "{pad}#text \"{}\"\n",
                            tb_core::text::truncate(&t, 60)
                        ));
                    }
                }
                Some(NodeKind::Element(e)) => {
                    out.push_str(&format!("{pad}<{}", e.tag));
                    if let Some(id) = e.element_id() {
                        out.push_str(&format!("#{id}"));
                    }
                    for c in e.classes().into_iter().take(4) {
                        out.push_str(&format!(".{c}"));
                    }
                    if let Some(href) = e.attr("href").or_else(|| e.attr("src")) {
                        out.push_str(&format!(" \"{}\"", tb_core::text::truncate(href, 40)));
                    }
                    out.push_str(">\n");
                }
                _ => {}
            }
            for c in dom.children(id) {
                walk(dom, c, depth + 1, out);
            }
        }
        let mut out = String::new();
        walk(self, self.document, 0, &mut out);
        out
    }
}

/// Escape text content for HTML.
pub fn escape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
    out
}

/// Escape attribute values for HTML.
pub fn escape_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
    out
}

/// Void elements: they never have children or an end tag.
pub mod void {
    const VOID: [&str; 16] = [
        "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param",
        "source", "track", "wbr", "basefont", "bgsound",
    ];

    pub fn is_void(tag: &str) -> bool {
        VOID.contains(&tag)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_and_links_a_tree() {
        let mut dom = Dom::new();
        let doc = dom.document();
        let html = dom.create_element("html");
        let body = dom.create_element("body");
        let p = dom.create_element("p");
        let t = dom.create_text("hello");
        dom.appendChild(doc, html);
        dom.appendChild(html, body);
        dom.appendChild(body, p);
        dom.appendChild(p, t);
        assert_eq!(dom.parent(p), Some(body));
        assert_eq!(dom.children(body), vec![p]);
        assert_eq!(dom.first_child(p), Some(t));
        assert_eq!(dom.next_sibling(html), None);
        assert_eq!(dom.text_content(p), "hello");
        assert_eq!(dom.text_content(doc), "hello");
        assert_eq!(dom.element_count(), 3);
    }

    #[test]
    fn move_reparents_correctly() {
        let mut dom = Dom::new();
        let a = dom.create_element("div");
        let b = dom.create_element("span");
        let c = dom.create_element("em");
        dom.appendChild(dom.document(), a);
        dom.appendChild(dom.document(), b);
        dom.appendChild(a, c);
        assert_eq!(dom.children(a), vec![c]);
        assert_eq!(dom.children(b), vec![]);
        dom.appendChild(b, c);
        assert_eq!(dom.children(a), vec![]);
        assert_eq!(dom.children(b), vec![c]);
        assert_eq!(dom.parent(c), Some(b));
    }

    #[test]
    fn insert_before_keeps_order() {
        let mut dom = Dom::new();
        let p = dom.create_element("p");
        let a = dom.create_text("a");
        let b = dom.create_text("b");
        let c = dom.create_text("c");
        dom.appendChild(p, b);
        dom.insertBefore(p, a, Some(b));
        dom.appendChild(p, c);
        assert_eq!(dom.children(p), vec![a, b, c]);
        assert_eq!(dom.text_content(p), "abc");
        dom.detach(a);
        assert_eq!(dom.children(p), vec![b, c]);
        assert_eq!(dom.first_child(p), Some(b));
    }

    #[test]
    fn stale_handles_do_not_resolve() {
        let mut dom = Dom::new();
        let a = dom.create_element("div");
        dom.appendChild(dom.document(), a);
        dom.remove(a);
        assert!(dom.exists(a), "detached nodes stay addressable");
        let gen = a;
        // removing frees nothing, but clear_children does
        let p = dom.create_element("p");
        let kid = dom.create_text("x");
        dom.appendChild(p, kid);
        dom.clear_children(p);
        assert!(!dom.exists(kid), "freed nodes must not resolve");
        let new = dom.create_element("b");
        assert_ne!(new, gen);
    }

    #[test]
    fn attributes_and_classes() {
        let mut dom = Dom::new();
        let d = dom.create_element("div");
        dom.set_attr(d, "id", "main");
        dom.set_attr(d, "CLASS", "a b  c");
        assert_eq!(dom.attr(d, "id").as_deref(), Some("main"));
        assert_eq!(dom.element_id(d).as_deref(), Some("main"));
        let e = dom.element(d).unwrap();
        assert_eq!(e.classes(), vec!["a", "b", "c"]);
        assert!(e.has_class("b"));
        assert!(!e.has_class("bb"));
        dom.set_attr(d, "data-test-case", "1");
        assert_eq!(dom.element(d).unwrap().dataset()[0].0, "testCase");
        dom.remove_attr(d, "class");
        assert!(!dom.has_attr(d, "class"));
    }

    #[test]
    fn lookups() {
        let mut dom = Dom::new();
        let body = dom.create_element("body");
        dom.appendChild(dom.document(), body);
        let d1 = dom.create_element("div");
        let d2 = dom.create_element("div");
        let sp = dom.create_element("span");
        dom.set_attr(d1, "id", "x");
        dom.set_attr(d2, "class", "hit");
        dom.appendChild(body, d1);
        dom.appendChild(body, d2);
        dom.appendChild(d2, sp);
        assert_eq!(dom.get_element_by_id("x"), Some(d1));
        assert_eq!(dom.get_element_by_id("nope"), None);
        assert_eq!(dom.get_elements_by_tag("div"), vec![d1, d2]);
        assert_eq!(dom.get_elements_by_class("hit"), vec![d2]);
        assert_eq!(dom.get_elements_by_tag("*").len(), 4);
    }

    #[test]
    fn sibling_element_helpers() {
        let mut dom = Dom::new();
        let p = dom.create_element("p");
        let a = dom.create_element("a");
        let t = dom.create_text(" ");
        let b = dom.create_element("b");
        dom.appendChild(p, a);
        dom.appendChild(p, t);
        dom.appendChild(p, b);
        assert_eq!(dom.next_element_sibling(a), Some(b));
        assert_eq!(dom.previous_element_sibling(b), Some(a));
        assert_eq!(dom.element_index(b), 2);
        assert_eq!(dom.element_sibling_count(a), 2);
    }

    #[test]
    fn serialization_round_trips() {
        let mut dom = Dom::new();
        let div = dom.create_element("div");
        dom.set_attr(div, "class", "a\"b");
        let p = dom.create_element("p");
        let txt = dom.create_text("<hi> & bye");
        let br = dom.create_element("br");
        dom.appendChild(dom.document(), div);
        dom.appendChild(div, p);
        dom.appendChild(p, txt);
        dom.appendChild(p, br);
        assert_eq!(
            dom.outer_html(div),
            "<div class=\"a&quot;b\"><p>&lt;hi&gt; &amp; bye<br></p></div>"
        );
        assert_eq!(dom.inner_html(div), "<p>&lt;hi&gt; &amp; bye<br></p>");
    }

    #[test]
    fn clone_is_deep_and_detached() {
        let mut dom = Dom::new();
        let ul = dom.create_element("ul");
        let li = dom.create_element("li");
        dom.appendChild(ul, li);
        dom.appendChild(dom.document(), ul);
        let copy = dom.clone_node(ul, true);
        assert_eq!(dom.children(copy), dom.child_elements(copy));
        assert_eq!(dom.children(copy).len(), 1);
        assert_eq!(dom.parent(copy), None);
        assert_ne!(copy, ul);
        let shallow = dom.clone_node(ul, false);
        assert!(dom.children(shallow).is_empty());
    }

    #[test]
    fn normalize_merges_text_nodes() {
        let mut dom = Dom::new();
        let p = dom.create_element("p");
        let a = dom.create_text("foo");
        let b = dom.create_text("bar");
        dom.appendChild(p, a);
        dom.appendChild(p, b);
        dom.normalize(p);
        assert_eq!(dom.children(p).len(), 1);
        assert_eq!(dom.text_content(p), "foobar");
    }

    #[test]
    fn visible_text_skips_script_and_style_but_text_content_does_not() {
        let mut dom = Dom::new();
        let body = dom.create_element("body");
        dom.appendChild(dom.document(), body);
        let s = dom.create_element("script");
        let st = dom.create_element("style");
        let code = dom.create_text("var x = 1;");
        let css = dom.create_text("p{}");
        dom.appendChild(s, code);
        dom.appendChild(st, css);
        let p = dom.create_element("p");
        let visible = dom.create_text("visible");
        dom.appendChild(p, visible);
        dom.appendChild(body, s);
        dom.appendChild(body, st);
        dom.appendChild(body, p);
        assert_eq!(dom.visible_text(body), "visible");
        assert_eq!(dom.text_content(body), "var x = 1;p{}visible");
        assert_eq!(dom.text_content(s), "var x = 1;");
    }
}
