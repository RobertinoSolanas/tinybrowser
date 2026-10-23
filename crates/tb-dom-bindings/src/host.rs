//! The host side of the bindings: one [`Host`] per browsing context.
//!
//! The `Host` owns the [`Dom`] the scripts mutate plus everything a browsing
//! context keeps around it — document URL, cookies, storage, event listeners,
//! navigation requests and the style/layout caches that `getBoundingClientRect`
//! and `getComputedStyle` need. Every native function in this crate closes over
//! one [`HostRef`], so there is exactly one DOM per interpreter.
//!
//! ## The borrow rule (read this before adding a binding)
//!
//! The host lives in an `Rc<RefCell<_>>` and JavaScript can re-enter it at any
//! moment: an event handler may mutate the DOM, a setter may call a script.
//! Therefore **no borrow of the host may be alive across a call into JS**.
//! Two consequences:
//!
//! * the helper constructors below use `try_borrow_mut()` and answer with a
//!   `TypeError` instead of panicking, and
//! * [`crate::events::dispatch`] collects its listener list, drops the borrow
//!   and only then invokes callbacks.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use tb_core::{EngineConfig, Phase, Problems};
use tb_css::media::Viewport;
use tb_css::StyleSheet;
use tb_dom::{Dom, NodeId};
use tb_js::builtins::obj_of;
use tb_js::value::{Key, Obj, ObjRef, Prop, Value};
use tb_js::{Interp, JsResult};
use tb_layout::LayoutDocument;
use tb_net::Url;
use tb_style::{compute, MatchState, StyleMap};

/// One interpreter's DOM, shared with every binding closure.
pub type HostRef = Rc<RefCell<Host>>;

/// A DOM node as it survives inside a JavaScript value: the arena handle
/// `{idx, gen}` rendered as `"idx:gen"`. A freed node stops resolving — the
/// wrapper becomes a harmless dead object instead of pointing at a stranger.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct NodeHandle {
    pub idx: u32,
    pub gen: u32,
}

impl NodeHandle {
    pub fn of(id: NodeId) -> NodeHandle {
        let (idx, gen) = id.raw();
        NodeHandle { idx, gen }
    }

    pub fn id(self) -> NodeId {
        NodeId::from_raw(self.idx, self.gen)
    }

    fn encoded(self) -> String {
        format!("{}:{}", self.idx, self.gen)
    }

    fn decode(s: &str) -> Option<NodeHandle> {
        let (idx, gen) = s.split_once(':')?;
        Some(NodeHandle {
            idx: idx.parse().ok()?,
            gen: gen.parse().ok()?,
        })
    }
}

/// What an event listener can be attached to. Elements and the document are
/// nodes; the window is not (it has no arena slot).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum EvtTarget {
    Window,
    Node(NodeHandle),
}

impl EvtTarget {
    pub fn node(id: NodeId) -> EvtTarget {
        EvtTarget::Node(NodeHandle::of(id))
    }
}

/// `document.readyState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReadyState {
    #[default]
    Loading,
    Interactive,
    Complete,
}

impl ReadyState {
    pub fn as_str(self) -> &'static str {
        match self {
            ReadyState::Loading => "loading",
            ReadyState::Interactive => "interactive",
            ReadyState::Complete => "complete",
        }
    }
}

/// What a script asked the browser to do to the *navigation*. The application
/// layer (`tb-browser`, T9.1) drains [`Host::nav`] after each script run —
/// a real browser does not navigate in the middle of a script either.
#[derive(Debug, Clone, PartialEq)]
pub enum NavRequest {
    Load(String),
    Replace(String),
    Reload,
    Back(i32),
    Scroll(f64, f64),
}

/// A registered `addEventListener` entry.
#[derive(Debug, Clone)]
pub struct Listener {
    pub target: EvtTarget,
    pub kind: String,
    pub capture: bool,
    pub once: bool,
    pub callback: Value,
}

/// Everything a DOM object can ask about the browsing context.
#[derive(Debug, Clone)]
pub struct PageContext {
    pub config: EngineConfig,
    pub url: Option<Url>,
    pub referrer: String,
    /// `document.title` before any `<title>` existed.
    pub title: String,
    /// `navigator.platform`.
    pub platform: String,
    /// `navigator.language`.
    pub language: String,
    /// `navigator.userAgent` (defaults to `config.user_agent`).
    pub user_agent: String,
}

impl Default for PageContext {
    fn default() -> Self {
        PageContext {
            config: EngineConfig::default(),
            url: None,
            referrer: String::new(),
            title: String::new(),
            platform: "Linux x86_64".into(),
            language: "en-US".into(),
            user_agent: tb_core::USER_AGENT.to_string(),
        }
    }
}

impl PageContext {
    pub fn viewport(&self) -> Viewport {
        Viewport {
            width: self.config.viewport.width,
            height: self.config.viewport.height,
            color_scheme: self.config.color_scheme,
            reduced_motion: false,
        }
    }

    /// Resolve a relative reference against the document URL.
    pub fn resolve(&self, href: &str) -> Option<Url> {
        let base = self.url.as_ref()?;
        Url::join(base, href).ok()
    }
}

/// Prototypes shared by every wrapper of a kind. Bindings live on the
/// prototype (reading the hidden `"@@node"` from `this`), which is why a page
/// with 3 000 elements costs 3 000 small objects and not 3 000 × 60 closures.
#[derive(Debug, Clone, Default)]
pub struct Protos {
    pub event_target: Option<ObjRef>,
    pub node: Option<ObjRef>,
    pub element: Option<ObjRef>,
    pub html_element: Option<ObjRef>,
    pub document: Option<ObjRef>,
    pub text: Option<ObjRef>,
    pub comment: Option<ObjRef>,
    pub event: Option<ObjRef>,
    pub token_list: Option<ObjRef>,
    pub style_decl: Option<ObjRef>,
    pub dom_rect: Option<ObjRef>,
    pub location: Option<ObjRef>,
    pub navigator: Option<ObjRef>,
    pub history: Option<ObjRef>,
    pub screen: Option<ObjRef>,
    pub storage: Option<ObjRef>,
    pub media: Option<ObjRef>,
}

/// Fetch hook signature for `fetch()` — `tb-browser` supplies the real one.
/// Reply is `(status, body)`.
pub type FetchHook = dyn Fn(&str) -> Option<(u16, String)>;

/// The DOM plus everything around it.
pub struct Host {
    /// The document. Scripts mutate this in place.
    pub dom: Dom,
    pub ctx: PageContext,
    pub problems: Problems,
    pub protos: Protos,
    pub ready: ReadyState,
    /// `window` (`== interp.globals`) and `document`, for identity.
    pub window: Option<ObjRef>,
    pub document: Option<Value>,
    /// Wrapper cache — weak, so a page dropping a node also drops its JS side.
    wrappers: HashMap<NodeHandle, Weak<RefCell<Obj>>>,
    /// Second-order wrappers (`el.style`, `el.classList`, `el.dataset`), one per
    /// node and kind — held **strongly**, so `el.style === el.style` holds and a
    /// `dataset` write survives the statement that made it.
    aux_wrappers: HashMap<(NodeHandle, &'static str), Rc<RefCell<Obj>>>,
    /// Registered listeners, in registration order (dispatch order matters).
    pub listeners: Vec<Listener>,
    /// `onclick=""`-style handler *properties*.
    pub handlers: HashMap<EvtTarget, HashMap<String, Value>>,
    /// Pending navigation requests.
    pub nav: Vec<NavRequest>,
    /// `history` stack (URLs) and their `pushState` values.
    pub history: Vec<String>,
    pub history_pos: usize,
    pub states: Vec<Value>,
    /// `document.cookie` (name → value, last write wins).
    pub cookies: Vec<(String, String)>,
    /// The `.value` a script gave a form control. A browser keeps this apart
    /// from the `value` attribute ("dirty value flag"), and so do we:
    /// `i.value = 'x'` must not change `i.defaultValue`.
    pub values: HashMap<NodeId, String>,
    /// `localStorage` / `sessionStorage`.
    pub local: HashMap<String, String>,
    /// JS mirror objects of the two storages (index 0 = local, 1 = session),
    /// so `localStorage.x = 1` — a plain property write — is picked up.
    pub storage_mirrors: [Option<ObjRef>; 2],
    pub session: HashMap<String, String>,
    /// Scroll offset in CSS px.
    pub scroll: (f64, f64),
    /// Author stylesheets the embedder supplied (external `<link>` styles).
    /// `<style>` elements in the document are added on top of these.
    pub sheets: Vec<StyleSheet>,
    /// Interaction state for `:hover`/`:focus` matching.
    pub match_state: MatchState,
    version: u64,
    style_cache: Option<(u64, Rc<StyleMap>)>,
    doc_sheets_cache: Option<(u64, Vec<StyleSheet>)>,
    layout_cache: Option<(u64, Rc<LayoutDocument>)>,
    pub fetch_hook: Option<Rc<FetchHook>>,
    /// How many scripts ran (diagnostics, tests).
    pub scripts_run: usize,
    /// `true` once any mirror (`dataset`, storage) exists: without one there is
    /// nothing to sync, and `host_mut` must not walk a map on every property access.
    mirrors_seen: bool,
}

impl Host {
    pub fn new(dom: Dom, ctx: PageContext) -> Host {
        let history = ctx
            .url
            .as_ref()
            .map(|u| vec![u.to_string()])
            .unwrap_or_default();
        Host {
            dom,
            ctx,
            problems: Problems::new(),
            protos: Protos::default(),
            ready: ReadyState::Loading,
            window: None,
            document: None,
            wrappers: HashMap::new(),
            aux_wrappers: HashMap::new(),
            listeners: Vec::new(),
            handlers: HashMap::new(),
            nav: Vec::new(),
            history,
            history_pos: 0,
            states: vec![Value::Null],
            cookies: Vec::new(),
            values: HashMap::new(),
            local: HashMap::new(),
            storage_mirrors: [None, None],
            session: HashMap::new(),
            scroll: (0.0, 0.0),
            sheets: Vec::new(),
            match_state: MatchState::default(),
            version: 0,
            style_cache: None,
            doc_sheets_cache: None,
            layout_cache: None,
            fetch_hook: None,
            scripts_run: 0,
            mirrors_seen: false,
        }
    }

    // ------------------------------------------------------------- identity --

    /// The JS object for a node — the same object every time, so that
    /// `document.getElementById('x') === document.querySelector('#x')` holds.
    pub fn wrap(&mut self, id: NodeId) -> Value {
        let h = NodeHandle::of(id);
        if let Some(existing) = self.wrappers.get(&h).and_then(|w| w.upgrade()) {
            return Value::Object(existing);
        }
        let class = dom_class(&self.dom, id);
        let proto = match self.dom.get(id).map(|n| &n.kind) {
            Some(tb_dom::NodeKind::Document) => self.protos.document.clone(),
            Some(tb_dom::NodeKind::Element(_)) => self.protos.html_element.clone(),
            Some(tb_dom::NodeKind::Text(_)) => self.protos.text.clone(),
            Some(tb_dom::NodeKind::Comment(_)) => self.protos.comment.clone(),
            _ => self.protos.node.clone(),
        };
        let mut o = Obj::plain();
        o.proto = proto.or(self.protos.node.clone());
        let obj = Rc::new(RefCell::new(o));
        {
            let mut b = obj.borrow_mut();
            b.define_internal("@@node", Value::string(h.encoded()));
            b.define_internal("@@class", Value::string(class));
        }
        self.wrappers.insert(h, Rc::downgrade(&obj));
        Value::Object(obj)
    }

    /// A cached helper object for a node (`style`, `classList`).
    pub fn aux(&mut self, id: NodeId, kind: &'static str, proto: Option<ObjRef>) -> Value {
        let h = NodeHandle::of(id);
        if let Some(existing) = self.aux_wrappers.get(&(h, kind)) {
            return Value::Object(existing.clone());
        }
        let mut o = Obj::plain();
        o.proto = proto;
        let obj = Rc::new(RefCell::new(o));
        {
            let mut b = obj.borrow_mut();
            b.define_internal("@@node", Value::string(h.encoded()));
            b.define_internal("@@class", Value::string(kind));
        }
        if kind == "dataset" {
            self.mirrors_seen = true;
        }
        self.aux_wrappers.insert((h, kind), obj.clone());
        Value::Object(obj)
    }

    /// The JS object of an event target (`window` included).
    pub fn wrap_target(&mut self, target: EvtTarget) -> Value {
        match target {
            EvtTarget::Window => self
                .window
                .clone()
                .map(Value::Object)
                .unwrap_or(Value::Undefined),
            EvtTarget::Node(h) => self.wrap(h.id()),
        }
    }

    pub fn document_value(&mut self) -> Value {
        let doc = self.dom.document();
        self.wrap(doc)
    }

    // ------------------------------------------------------------ mutation ---

    /// Every DOM mutation bumps the version; style and layout caches notice.
    /// The `.value` of a form control: what a script last wrote, or the
    /// attribute / text content when nobody has touched it.
    pub fn form_value(&self, id: NodeId, fallback: impl FnOnce() -> String) -> String {
        match self.values.get(&id) {
            Some(v) => v.clone(),
            None => fallback(),
        }
    }

    /// Forget the script-written value (a `reset()` goes back to the attribute).
    pub fn clear_form_value(&mut self, id: NodeId) {
        self.values.remove(&id);
    }

    /// Push every live mirror (`dataset`, `localStorage`, `sessionStorage`) back
    /// into the state it stands for.
    ///
    /// A page writing `el.dataset.x = 1` changes a plain JS property; nothing
    /// tells us about it, so we sync at the points where control comes back to
    /// the engine: after each script, timer and event dispatch, and before any
    /// style or layout work.
    pub fn flush_mirrors(&mut self) {
        if !self.mirrors_seen {
            return;
        }
        let live: Vec<(NodeId, tb_js::value::ObjRef)> = self
            .aux_wrappers
            .iter()
            .filter(|((_, kind), _)| *kind == "dataset")
            .map(|((h, _), o)| (h.id(), o.clone()))
            .collect();
        let mut changed = false;
        for (id, obj) in live {
            if self.dom.exists(id) && crate::style_api::sync_dataset(&mut self.dom, id, &obj) {
                changed = true;
            }
        }
        for which in 0..2 {
            if self.sync_storage(which) {
                changed = true;
            }
        }
        if changed {
            self.touch();
        }
    }

    /// Sync one storage mirror (0 = local, 1 = session) with its map.
    pub fn sync_storage(&mut self, which: usize) -> bool {
        let Some(obj) = self.storage_mirrors[which].clone() else {
            return false;
        };
        self.mirrors_seen = true;
        let map = if which == 0 {
            &mut self.local
        } else {
            &mut self.session
        };
        sync_storage(map, &obj)
    }

    pub fn touch(&mut self) {
        self.version = self.version.wrapping_add(1);
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    // ------------------------------------------------------- style / layout ---

    /// Stylesheets from outside the document (`<link>` the embedder resolved).
    pub fn set_sheets(&mut self, sheets: Vec<StyleSheet>) {
        self.sheets = sheets;
        self.touch();
    }

    /// The sheets to cascade: what the embedder handed us (external `<link>`
    /// styles and `@import`s it resolved) plus every `<style>` element the
    /// document itself carries — so a page that writes a `<style>` tag sees it
    /// apply, and the embedder does not have to re-read the DOM.
    fn all_sheets(&mut self) -> Vec<StyleSheet> {
        let mut out = self.sheets.clone();
        out.extend(self.doc_sheets());
        out
    }

    /// `<style>` elements of the document, parsed once per DOM version.
    fn doc_sheets(&mut self) -> Vec<StyleSheet> {
        if self
            .doc_sheets_cache
            .as_ref()
            .is_some_and(|(v, _)| *v == self.version)
        {
            return self.doc_sheets_cache.as_ref().unwrap().1.clone();
        }
        let mut out = Vec::new();
        for id in self.dom.get_elements_by_tag("style") {
            let text = self.dom.text_content(id);
            if text.trim().is_empty() {
                continue;
            }
            let mut sheet = tb_css::parse_stylesheet(&text);
            sheet.href = "<style>".into();
            out.push(sheet);
        }
        self.doc_sheets_cache = Some((self.version, out.clone()));
        out
    }

    /// Cascade (cached until the DOM or the sheets change).
    pub fn style_map(&mut self) -> Rc<StyleMap> {
        // a `dataset` write is only a JS property until somebody looks
        self.flush_mirrors();
        if self
            .style_cache
            .as_ref()
            .is_some_and(|(v, _)| *v == self.version)
        {
            return self.style_cache.as_ref().unwrap().1.clone();
        }
        let viewport = self.ctx.viewport();
        let sheets = self.all_sheets();
        let out = compute(&self.dom, &sheets, viewport, &self.match_state);
        let map = Rc::new(out.styles);
        self.style_cache = Some((self.version, map.clone()));
        map
    }

    /// Lay the document out (cached like [`Host::style_map`]).
    pub fn layout(&mut self) -> Option<Rc<LayoutDocument>> {
        self.flush_mirrors();
        if self
            .layout_cache
            .as_ref()
            .is_some_and(|(v, _)| *v == self.version)
        {
            return Some(self.layout_cache.as_ref().unwrap().1.clone());
        }
        let styles = self.style_map();
        let viewport = self.ctx.viewport();
        let doc = tb_layout::layout(&self.dom, &styles, viewport)?;
        let doc = Rc::new(doc);
        self.layout_cache = Some((self.version, doc.clone()));
        Some(doc)
    }

    /// Border box of an element in viewport coordinates.
    pub fn rect_of(&mut self, id: NodeId) -> Option<tb_core::Rect> {
        let doc = self.layout()?;
        let b = doc.find(id)?;
        Some(b.border_box())
    }

    // ------------------------------------------------------------- cookies ---

    /// `document.cookie` — read as `name=value; name=value`.
    pub fn cookie_header(&self) -> String {
        self.cookies
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// A `document.cookie = "name=value; path=/; expires=…"` write. An `expires`
    /// in the past (or `max-age<=0`) deletes the cookie.
    pub fn set_cookie(&mut self, raw: &str) {
        let mut parts = raw.split(';');
        let Some(pair) = parts.next() else { return };
        let (name, value) = match pair.split_once('=') {
            Some((k, v)) => (k.trim().to_string(), v.trim().to_string()),
            None => (pair.trim().to_string(), String::new()),
        };
        if name.is_empty() {
            return;
        }
        let mut expired = false;
        for attr in parts {
            let a = attr.trim().to_ascii_lowercase();
            if let Some(v) = a.split_once('=').map(|(_, v)| v.trim()) {
                if a.starts_with("max-age=") {
                    if v.parse::<i64>().unwrap_or(1) <= 0 {
                        expired = true;
                    }
                } else if a.starts_with("expires=") {
                    // "Thu, 01 Jan 1970" is the only date we care about
                    if v.contains("1970") || v.contains("197") {
                        expired = true;
                    }
                }
            }
        }
        match self.cookies.iter_mut().find(|(k, _)| k == &name) {
            // replacing a value keeps the cookie where it was, which is what
            // `document.cookie` reads back in a browser
            Some(slot) => {
                if expired {
                    self.cookies.retain(|(k, _)| k != &name);
                } else {
                    slot.1 = value;
                }
            }
            None => {
                if !expired {
                    self.cookies.push((name, value));
                }
            }
        }
    }

    // ----------------------------------------------------------- navigation --

    /// Change the document URL (after a real navigation), resetting caches.
    pub fn set_url(&mut self, url: &Url) {
        self.ctx.url = Some(url.clone());
        self.history.push(url.to_string());
        self.history_pos = self.history.len() - 1;
        self.states.push(Value::Null);
    }

    pub fn url_string(&self) -> String {
        self.ctx
            .url
            .as_ref()
            .map(|u| u.to_string())
            .unwrap_or_else(|| "about:blank".into())
    }

    pub fn request(&mut self, nav: NavRequest) {
        self.nav.push(nav);
    }

    /// Take the pending navigation request (last one wins, like a browser).
    pub fn take_navigation(&mut self) -> Option<NavRequest> {
        self.nav.pop()
    }

    pub fn log_problem(&mut self, message: impl Into<String>) {
        let m = message.into();
        if !self.problems.all().iter().any(|p| p.message == m) {
            self.problems.warn(Phase::Script, m);
        }
    }
}

/// Sync one `localStorage`/`sessionStorage` mirror object with the map behind
/// it: property writes go in, `setItem` values and deleted keys come out. The
/// methods live on the mirror's *prototype*, so its own properties really are
/// the stored keys, and a `@@k:` marker per key tells us which side changed —
/// a key the map gained on its own (persisted state loaded after load, say) is
/// pulled in rather than mistaken for a deletion.
pub fn sync_storage(map: &mut HashMap<String, String>, obj: &ObjRef) -> bool {
    const MARK: &str = "@@k:";
    let (props, marks): (Vec<(String, Value)>, Vec<(String, Value)>) = {
        let b = obj.borrow();
        let mut props = Vec::new();
        let mut marks = Vec::new();
        for (k, p) in &b.props {
            let Key::Name(n) = k else { continue };
            if let Some(rest) = n.strip_prefix(MARK) {
                if let Some(v) = p.value() {
                    marks.push((rest.to_string(), v.clone()));
                }
            } else if !n.starts_with("@@") {
                if let Some(v) = p.value() {
                    props.push((n.to_string(), v.clone()));
                }
            }
        }
        (props, marks)
    };
    let mark_of = |name: &str| {
        marks
            .iter()
            .find(|(m, _)| m == name)
            .map(|(_, v)| v.clone())
    };
    let mut changed = false;

    // 1. properties the page wrote → storage (values are always strings)
    for (name, value) in &props {
        let key = Key::Name(Rc::from(name.as_str()));
        if value.is_nullish() {
            if map.remove(name).is_some() {
                changed = true;
            }
            obj.borrow_mut().delete(&key);
            obj.borrow_mut()
                .delete(&Key::Name(Rc::from(format!("{MARK}{name}"))));
            continue;
        }
        // unchanged since the last sync: leave it alone, or a key that
        // `removeItem` just dropped would be pushed straight back in
        if mark_of(name).is_some_and(|m| m.to_string() == value.to_string()) {
            continue;
        }
        let text = value.to_string();
        if map.get(name).map(|s| s.as_str()) != Some(text.as_str()) {
            map.insert(name.clone(), text.clone());
            changed = true;
        }
        if !value.is_string() {
            // `localStorage.n = 5` reads back as "5", like a real Storage
            obj.borrow_mut()
                .define(&name, Prop::data(Value::string(&text)));
        }
        obj.borrow_mut()
            .define_internal(&format!("{MARK}{name}"), Value::string(&text));
    }
    // 2. properties the page deleted → key gone (only if we had exposed it)
    for (name, _) in &marks {
        if props.iter().any(|(n, _)| n == name) {
            continue;
        }
        if map.remove(name).is_some() {
            changed = true;
        }
        obj.borrow_mut()
            .delete(&Key::Name(Rc::from(format!("{MARK}{name}"))));
    }
    // 3. storage → properties (`setItem`, `clear`, state loaded from disk)
    let entries: Vec<(String, String)> = map.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    for (k, v) in entries {
        if mark_of(&k).is_some_and(|m| m.to_string() == v) {
            continue;
        }
        obj.borrow_mut().define(&k, Prop::data(Value::string(&v)));
        obj.borrow_mut()
            .define_internal(&format!("{MARK}{k}"), Value::string(&v));
    }
    // keys that left the map (`removeItem`, `clear`) leave the mirror too
    for (name, _) in &marks {
        if !map.contains_key(name) {
            let mut b = obj.borrow_mut();
            b.delete(&Key::Name(Rc::from(name.as_str())));
            b.delete(&Key::Name(Rc::from(format!("{MARK}{name}"))));
        }
    }
    changed
}

// ---------------------------------------------------------------------------
// handles: JS object  <->  DOM node
// ---------------------------------------------------------------------------

fn internal_str(v: &Value, key: &str) -> Option<String> {
    let o = v.as_object()?;
    let s = o
        .borrow()
        .get_own(&Key::Name(Rc::from(key)))
        .and_then(|p| p.value())
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())?;
    Some(s)
}

/// The node behind `this` (a wrapper created by [`Host::wrap`]).
pub fn node_of(v: &Value) -> Option<NodeId> {
    NodeHandle::decode(&internal_str(v, "@@node")?).map(|h| h.id())
}

/// The node behind `this`, *without* checking that it still exists.
pub fn handle_of(v: &Value) -> Option<NodeHandle> {
    NodeHandle::decode(&internal_str(v, "@@node")?)
}

/// What `this` is for `addEventListener` and friends.
pub fn evt_target_of(v: &Value) -> Option<EvtTarget> {
    if let Some(h) = handle_of(v) {
        return Some(EvtTarget::Node(h));
    }
    if v.as_object()
        .map(|o| o.borrow().get_own(&Key::Name(Rc::from("@@win"))).is_some())
        .unwrap_or(false)
    {
        return Some(EvtTarget::Window);
    }
    None
}

/// The class name `String(wrapper)` prints.
fn dom_class(dom: &Dom, id: NodeId) -> String {
    match dom.get(id).map(|n| &n.kind) {
        Some(tb_dom::NodeKind::Document) => "HTMLDocument".into(),
        Some(tb_dom::NodeKind::Text(_)) => "Text".into(),
        Some(tb_dom::NodeKind::Comment(_)) => "Comment".into(),
        Some(tb_dom::NodeKind::Element(e)) => match e.namespace {
            tb_dom::Namespace::Svg => "SVGElement".into(),
            tb_dom::Namespace::MathMl => "MathMLElement".into(),
            _ => html_class(&e.tag).into(),
        },
        _ => "Node".into(),
    }
}

/// `Object.prototype.toString` of an element: the HTML interface for its tag,
/// `HTMLElement` for the tags that have no interface of their own.
fn html_class(tag: &str) -> &'static str {
    match tag {
        "a" => "HTMLAnchorElement",
        "area" => "HTMLAreaElement",
        "article" | "aside" | "footer" | "header" | "main" | "nav" | "section" => "HTMLElement",
        "audio" => "HTMLAudioElement",
        "b" | "i" | "em" | "strong" | "small" | "u" | "s" | "mark" | "bdi" | "bdo" | "cite"
        | "code" | "dfn" | "kbd" | "q" | "samp" | "sub" | "sup" | "time" | "var" | "abbr" => {
            "HTMLElement"
        }
        "base" => "HTMLBaseElement",
        "body" => "HTMLBodyElement",
        "br" => "HTMLBRElement",
        "button" => "HTMLButtonElement",
        "canvas" => "HTMLCanvasElement",
        "caption" => "HTMLTableCaptionElement",
        "col" => "HTMLTableColElement",
        "data" => "HTMLDataElement",
        "datalist" => "HTMLDataListElement",
        "dd" | "dt" => "HTMLElement",
        "del" | "ins" => "HTMLModElement",
        "details" => "HTMLDetailsElement",
        "dialog" => "HTMLDialogElement",
        "div" => "HTMLDivElement",
        "dl" => "HTMLDListElement",
        "embed" => "HTMLEmbedElement",
        "fieldset" => "HTMLFieldSetElement",
        "figcaption" => "HTMLElement",
        "font" => "HTMLFontElement",
        "form" => "HTMLFormElement",
        "frame" => "HTMLFrameElement",
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => "HTMLHeadingElement",
        "head" => "HTMLHeadElement",
        "hgroup" => "HTMLElement",
        "hr" => "HTMLHRElement",
        "html" => "HTMLHtmlElement",
        "iframe" => "HTMLIFrameElement",
        "img" => "HTMLImageElement",
        "input" => "HTMLInputElement",
        "label" => "HTMLLabelElement",
        "legend" => "HTMLLegendElement",
        "li" => "HTMLLIElement",
        "link" => "HTMLLinkElement",
        "map" => "HTMLMapElement",
        "menu" => "HTMLMenuElement",
        "meta" => "HTMLMetaElement",
        "meter" => "HTMLMeterElement",
        "object" => "HTMLObjectElement",
        "ol" => "HTMLOListElement",
        "optgroup" => "HTMLOptGroupElement",
        "option" => "HTMLOptionElement",
        "output" => "HTMLOutputElement",
        "p" => "HTMLParagraphElement",
        "picture" => "HTMLPictureElement",
        "pre" => "HTMLPreElement",
        "progress" => "HTMLProgressElement",
        "script" => "HTMLScriptElement",
        "search" => "HTMLElement",
        "select" => "HTMLSelectElement",
        "slot" => "HTMLSlotElement",
        "source" => "HTMLSourceElement",
        "span" => "HTMLSpanElement",
        "style" => "HTMLStyleElement",
        "summary" => "HTMLElement",
        "table" => "HTMLTableElement",
        "tbody" | "tfoot" | "thead" => "HTMLTableSectionElement",
        "td" | "th" => "HTMLTableCellElement",
        "template" => "HTMLTemplateElement",
        "textarea" => "HTMLTextAreaElement",
        "title" => "HTMLTitleElement",
        "tr" => "HTMLTableRowElement",
        "track" => "HTMLTrackElement",
        "ul" => "HTMLUListElement",
        "video" => "HTMLVideoElement",
        // a custom element is an HTMLElement (or whatever it registered)
        _ => "HTMLElement",
    }
}

// ---------------------------------------------------------------------------
// binding constructors
// ---------------------------------------------------------------------------

/// `borrow_mut()` that never panics: re-entrant script access gets a
/// `TypeError`, not a crash. (A hostile page must not be able to kill us.)
pub fn host_mut<'a>(h: &'a HostRef, it: &mut Interp) -> Result<std::cell::RefMut<'a, Host>, Value> {
    let mut host = h.try_borrow_mut().map_err(|_| {
        it.type_error("DOM is busy (re-entrant host access); work with the object you have")
    })?;
    // `el.dataset.x = 1` is a JS property write we cannot intercept, so every
    // trip through the DOM starts by settling those mirrors into attributes
    host.flush_mirrors();
    Ok(host)
}

/// A constructor the page can `new`. Instances get `proto`; `f` fills the new
/// object in and may return a different one (that is how `new Image()` hands
/// back an element wrapper).
pub fn ctor<F>(it: &mut Interp, name: &str, proto: &ObjRef, f: F) -> Value
where
    F: Fn(&mut Interp, Value, Vec<Value>) -> JsResult<Value> + 'static,
{
    it.native_ctor(name, proto, Some(Rc::new(f)))
}

/// Read-only accessor on a prototype; `f` sees the host and `this`'s node.
///
/// `f` must not call into JavaScript while it holds `&mut Host`.
pub fn ro_prop<F>(it: &mut Interp, host: &HostRef, proto: &ObjRef, name: &str, f: F)
where
    F: Fn(&mut Interp, &mut Host, NodeId) -> JsResult<Value> + 'static,
{
    let h = host.clone();
    let body = move |it: &mut Interp, this: Value, _args: Vec<Value>| -> JsResult<Value> {
        let Some(id) = node_of(&this) else {
            return Ok(Value::Undefined);
        };
        let mut h = host_mut(&h, it)?;
        if !h.dom.exists(id) {
            return Ok(Value::Undefined);
        }
        f(it, &mut h, id)
    };
    let g = it.native_fn(name, 0, body);
    proto
        .borrow_mut()
        .define(name, Prop::accessor(Some(obj_of(g)), None));
}

/// Read/write accessor on a prototype.
pub fn rw_prop<G, S>(it: &mut Interp, host: &HostRef, proto: &ObjRef, name: &str, get: G, set: S)
where
    G: Fn(&mut Interp, &mut Host, NodeId, Value) -> JsResult<Value> + 'static,
    S: Fn(&mut Interp, &mut Host, NodeId, Value) -> JsResult<()> + 'static,
{
    let hg = host.clone();
    let hs = host.clone();
    let getter = it.native_fn(
        name,
        0,
        move |it: &mut Interp, this: Value, a: Vec<Value>| {
            let Some(id) = node_of(&this) else {
                return Ok(Value::Undefined);
            };
            let mut h = host_mut(&hg, it)?;
            if !h.dom.exists(id) {
                return Ok(Value::Undefined);
            }
            get(
                it,
                &mut h,
                id,
                a.first().cloned().unwrap_or(Value::Undefined),
            )
        },
    );
    let setter = it.native_fn(
        name,
        1,
        move |it: &mut Interp, this: Value, a: Vec<Value>| {
            let Some(id) = node_of(&this) else {
                return Ok(Value::Undefined);
            };
            let mut h = host_mut(&hs, it)?;
            if !h.dom.exists(id) {
                return Ok(Value::Undefined);
            }
            set(
                it,
                &mut h,
                id,
                a.first().cloned().unwrap_or(Value::Undefined),
            )?;
            Ok(Value::Undefined)
        },
    );
    proto.borrow_mut().define(
        name,
        Prop::accessor(Some(obj_of(getter)), Some(obj_of(setter))),
    );
}

/// Method on a prototype whose `this` is a node.
pub fn method<F>(it: &mut Interp, host: &HostRef, proto: &ObjRef, name: &str, len: u32, f: F)
where
    F: Fn(&mut Interp, &mut Host, NodeId, Vec<Value>) -> JsResult<Value> + 'static,
{
    let h = host.clone();
    let label = name.to_string();
    let body = move |it: &mut Interp, this: Value, args: Vec<Value>| -> JsResult<Value> {
        let Some(id) = node_of(&this) else {
            return Err(it.type_error(&format!("{label}(): `this` is not a DOM node")));
        };
        let mut h = host_mut(&h, it)?;
        if !h.dom.exists(id) {
            return Err(it.type_error(&format!("{label}(): detached node")));
        }
        f(it, &mut h, id, args)
    };
    let v = it.native_fn(name, len, body);
    proto.borrow_mut().define_internal(name, v);
}

/// Method that needs `this` itself (window objects, event targets).
pub fn this_method<F>(it: &mut Interp, host: &HostRef, proto: &ObjRef, name: &str, len: u32, f: F)
where
    F: Fn(&mut Interp, &mut Host, Value, Vec<Value>) -> JsResult<Value> + 'static,
{
    let h = host.clone();
    let body = move |it: &mut Interp, this: Value, args: Vec<Value>| -> JsResult<Value> {
        let mut h = host_mut(&h, it)?;
        f(it, &mut h, this, args)
    };
    let v = it.native_fn(name, len, body);
    proto.borrow_mut().define_internal(name, v);
}

/// Like [`this_method`], but the borrow is released first: for bindings that
/// call into JavaScript (`dispatchEvent`).
pub fn this_method_unlocked<F>(
    it: &mut Interp,
    host: &HostRef,
    proto: &ObjRef,
    name: &str,
    len: u32,
    f: F,
) where
    F: Fn(&mut Interp, &HostRef, Value, Vec<Value>) -> JsResult<Value> + 'static,
{
    let h = host.clone();
    let body = move |it: &mut Interp, this: Value, args: Vec<Value>| -> JsResult<Value> {
        f(it, &h, this, args)
    };
    let v = it.native_fn(name, len, body);
    proto.borrow_mut().define_internal(name, v);
}

/// Define (or extend) an accessor, keeping whichever side is already there.
/// Location builds its getter and setter in two passes, so the second one must
/// not throw the first away.
pub fn merge_accessor(obj: &ObjRef, name: &str, getter: Option<ObjRef>, setter: Option<ObjRef>) {
    let mut b = obj.borrow_mut();
    let key = Key::from_str(name);
    let (mut g, mut s, enumerable) = match b.get_own(&key) {
        Some(Prop {
            kind: tb_js::value::PropKind::Access { getter, setter },
            enumerable,
            ..
        }) => (getter.clone(), setter.clone(), *enumerable),
        _ => (None, None, true),
    };
    if getter.is_some() {
        g = getter;
    }
    if setter.is_some() {
        s = setter;
    }
    let prop = Prop {
        kind: tb_js::value::PropKind::Access {
            getter: g,
            setter: s,
        },
        enumerable,
        writable: true,
    };
    b.define(name, prop);
}

/// Accessor whose body works off `this` rather than a node (window.location …).
pub fn rw_this_prop<G, S>(
    it: &mut Interp,
    host: &HostRef,
    proto: &ObjRef,
    name: &str,
    get: G,
    set: S,
) where
    G: Fn(&mut Interp, &mut Host, Value, Value) -> JsResult<Value> + 'static,
    S: Fn(&mut Interp, &mut Host, Value, Value) -> JsResult<()> + 'static,
{
    let hg = host.clone();
    let hs = host.clone();
    let getter = it.native_fn(
        name,
        0,
        move |it: &mut Interp, this: Value, a: Vec<Value>| {
            let mut h = host_mut(&hg, it)?;
            get(
                it,
                &mut h,
                this,
                a.first().cloned().unwrap_or(Value::Undefined),
            )
        },
    );
    let setter = it.native_fn(
        name,
        1,
        move |it: &mut Interp, this: Value, a: Vec<Value>| {
            let mut h = host_mut(&hs, it)?;
            set(
                it,
                &mut h,
                this,
                a.first().cloned().unwrap_or(Value::Undefined),
            )?;
            Ok(Value::Undefined)
        },
    );
    proto.borrow_mut().define(
        name,
        Prop::accessor(Some(obj_of(getter)), Some(obj_of(setter))),
    );
}

/// A JS `Array` of node wrappers with `item()`/`namedItem()` on top — our
/// stand-in for `NodeList`/`HTMLCollection` (documented deviation: it *is* a
/// real array, so `Array.isArray()` says `true` and spreading works).
pub fn node_list(it: &mut Interp, host: &mut Host, nodes: Vec<NodeId>) -> Value {
    let items: Vec<Value> = nodes.iter().map(|n| host.wrap(*n)).collect();
    let list = it.array(items);
    let Some(obj) = list.as_object().cloned() else {
        return list.clone();
    };
    let item = it.native_fn(
        "item",
        1,
        move |it: &mut Interp, this: Value, a: Vec<Value>| {
            let i = a.first().map(|v| v.to_integer()).unwrap_or(0.0);
            it.get(&this, &Key::Num(i))
        },
    );
    // `namedItem` deliberately asks the elements themselves (through their own
    // accessors) instead of borrowing the host — it may run at any later time.
    let named = it.native_fn(
        "namedItem",
        1,
        move |it: &mut Interp, this: Value, a: Vec<Value>| {
            let want = a
                .first()
                .map(|v| it.to_string_val(v).unwrap_or_default())
                .unwrap_or_default();
            let len = it.get(&this, &Key::Name(Rc::from("length")))?.to_integer();
            for i in 0..len.max(0.0) as usize {
                let el = it.get(&this, &Key::Num(i as f64))?;
                let id_attr = it.get(&el, &Key::Name(Rc::from("id")))?.to_string();
                if id_attr == want {
                    return Ok(el);
                }
                if let Some(name) = it
                    .call_method(&el, "getAttribute", vec![Value::string("name")])?
                    .filter(|v| !v.is_nullish())
                {
                    if it.to_string_val(&name)? == want {
                        return Ok(el);
                    }
                }
            }
            Ok(Value::Null)
        },
    );
    obj.borrow_mut().define_internal("item", item);
    obj.borrow_mut().define_internal("namedItem", named);
    list
}

/// Read-only accessor that also receives the `HostRef`: for properties handing
/// out objects whose own methods need to find the host again (`attributes`).
pub fn ro_prop_h<F>(it: &mut Interp, host: &HostRef, proto: &ObjRef, name: &str, f: F)
where
    F: Fn(&mut Interp, &HostRef, &mut Host, NodeId) -> JsResult<Value> + 'static,
{
    let h = host.clone();
    let body = move |it: &mut Interp, this: Value, _args: Vec<Value>| -> JsResult<Value> {
        let Some(id) = node_of(&this) else {
            return Ok(Value::Undefined);
        };
        let mut hh = host_mut(&h, it)?;
        if !hh.dom.exists(id) {
            return Ok(Value::Undefined);
        }
        f(it, &h, &mut hh, id)
    };
    let g = it.native_fn(name, 0, body);
    proto
        .borrow_mut()
        .define(name, Prop::accessor(Some(obj_of(g)), None));
}

/// `object[name] = function` — built first, then stored (the borrow checker
/// insists, and so does clarity).
pub fn def_native<F>(it: &mut Interp, obj: &Value, name: &str, len: u32, f: F)
where
    F: Fn(&mut Interp, Value, Vec<Value>) -> JsResult<Value> + 'static,
{
    let v = it.native_fn(name, len, f);
    if let Some(o) = obj.as_object() {
        o.borrow_mut().define(name, Prop::data(v));
    }
}

/// Method that is **not** allowed to hold the host borrow, because it calls
/// back into JavaScript (`click()` dispatches an event, and that handler may
/// mutate the DOM). `f` gets the handle plus the `HostRef` and must borrow
/// short itself.
pub fn method_unlocked<F>(
    it: &mut Interp,
    host: &HostRef,
    proto: &ObjRef,
    name: &str,
    len: u32,
    f: F,
) where
    F: Fn(&mut Interp, &HostRef, NodeId, Vec<Value>) -> JsResult<Value> + 'static,
{
    let h = host.clone();
    let check = host.clone();
    let label = name.to_string();
    let body = move |it: &mut Interp, this: Value, args: Vec<Value>| -> JsResult<Value> {
        let Some(id) = node_of(&this) else {
            return Err(it.type_error(&format!("{label}(): `this` is not a DOM node")));
        };
        if let Ok(h) = check.try_borrow() {
            if !h.dom.exists(id) {
                return Err(it.type_error(&format!("{label}(): detached node")));
            }
        }
        f(it, &h, id, args)
    };
    let v = it.native_fn(name, len, body);
    proto.borrow_mut().define_internal(name, v);
}

/// Numbers as CSS/HTML writes them: `12`, not `12.0`.
pub fn num_string(n: f64) -> String {
    if n.is_finite() && n == n.round() {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}
