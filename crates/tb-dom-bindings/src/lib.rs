//! # tb-dom-bindings — the DOM that JavaScript sees
//!
//! This crate puts [`tb_dom`], [`tb_style`], [`tb_layout`] and [`tb_js`]
//! together into a scripting environment: `window`, `document`, elements,
//! events and the `<script>` execution order of a real page load.
//!
//! ```
//! use tb_dom_bindings::{DomBindings, PageContext};
//!
//! let mut interp = tb_js::Runtime::new();
//! let dom = tb_html::parse_document(
//!     "<ul id=list><li>a</li></ul>\
//!      <script>const li = document.createElement('li');\
//!      li.textContent = 'b'; document.getElementById('list').appendChild(li);\
//!      document.title = 'zwei';</script>",
//! );
//! let mut b = DomBindings::install(dom, PageContext::default(), &mut interp);
//! let report = b.run_scripts(&mut interp, |_| None);
//! assert_eq!(report.executed, 1);
//! assert_eq!(b.eval(&mut interp, "document.querySelectorAll('#list li').length").ok().and_then(|v| v.as_number()), Some(2.0));
//! assert_eq!(b.title(), "zwei");
//! ```
//!
//! ## How a DOM object works
//!
//! A DOM object in JavaScript is a small plain object carrying one hidden,
//! non-enumerable property: `"@@node"`, the arena handle (`"idx:gen"`) of the
//! node it stands for. Everything else — `textContent`, `querySelector`,
//! `classList` — lives on **one** prototype per kind and reads that handle off
//! `this`. Two rules follow from that:
//!
//! * **identity is stable**: `document.getElementById('x') === document.querySelector('#x')`,
//!   because wrappers are cached (weakly, so a dropped node can be collected),
//! * **stale handles are harmless**: after a node is freed its wrapper answers
//!   `undefined`/`null` instead of pointing at whatever now occupies the slot.
//!
//! ## The borrow rule
//!
//! The [`Host`] is a single `Rc<RefCell<_>>`. Scripts re-enter it constantly
//! (an event handler mutates the DOM), so a host borrow must never be alive
//! across a call into JavaScript. [`host::method`] holds the borrow for pure DOM
//! work; [`host::method_unlocked`] exists for bindings that dispatch events, and
//! every helper answers with a `TypeError` — never a panic — if the host is
//! already borrowed.

mod document;
mod element;
mod events;
mod host;
mod html;
mod node;
mod query;
mod scripts;
mod style_api;
mod tokens;
mod window;

pub use events::{dispatch, make_event, EVENT_HANDLERS};
pub use host::{
    evt_target_of, node_of, EvtTarget, FetchHook, Host, HostRef, NavRequest, NodeHandle,
    PageContext, ReadyState,
};
pub use scripts::{pump, run_document, ScriptOptions, ScriptReport};
pub use style_api::STYLE_PROPS;
pub use tb_dom::NodeId;

use std::rc::Rc;
use tb_dom::Dom;
use tb_js::value::{Obj, ObjRef, Value};
use tb_js::{Interp, JsResult, Runtime};

/// The installed scripting environment of one document.
pub struct DomBindings {
    /// Shared host state (DOM, URL, listeners, caches).
    pub host: HostRef,
    /// `window`, which is `globalThis`.
    pub window: ObjRef,
}

impl DomBindings {
    /// Install `window`/`document`/`Element`/… into `rt`'s global scope.
    pub fn install(dom: Dom, ctx: PageContext, rt: &mut Runtime) -> DomBindings {
        Self::install_in(dom, ctx, &mut rt.interp)
    }

    /// The same, for a bare [`Interp`].
    pub fn install_in(dom: Dom, ctx: PageContext, it: &mut Interp) -> DomBindings {
        let host: HostRef = Rc::new(std::cell::RefCell::new(Host::new(dom, ctx)));
        install_prototypes(it, &host);
        window::install(it, &host);

        let window = it.globals.clone();
        // `document` is a value (not a getter): every page compares it, and the
        // wrapper must stay the same object for the whole session
        let document = {
            let mut h = host.borrow_mut();
            let doc = h.dom.document();
            let v = h.wrap(doc);
            h.document = Some(v.clone());
            v
        };
        it.globals
            .borrow_mut()
            .define_internal("document", document);
        if let Ok(mut h) = host.try_borrow_mut() {
            h.window = Some(window.clone());
        }
        DomBindings { host, window }
    }

    // -------------------------------------------------------------- access --

    /// Run `f` with the DOM (no clone of the arena).
    pub fn with_dom<R>(&self, f: impl FnOnce(&Dom) -> R) -> R {
        let h = self.host.borrow();
        f(&h.dom)
    }

    /// A copy of the DOM (for rendering after the scripts had their way).
    pub fn dom(&self) -> Dom {
        self.host.borrow().dom.clone()
    }

    /// `document.title`, without going through JavaScript.
    pub fn title(&self) -> String {
        let h = self.host.borrow();
        document::title_of(&h)
    }

    /// The author stylesheets to cascade with (set by `tb-browser`).
    pub fn set_sheets(&self, sheets: Vec<tb_css::StyleSheet>) {
        if let Ok(mut h) = self.host.try_borrow_mut() {
            h.set_sheets(sheets);
        }
    }

    /// Give `fetch()` a way to the network.
    pub fn set_fetch_hook<F>(&self, hook: F)
    where
        F: Fn(&str) -> Option<(u16, String)> + 'static,
    {
        if let Ok(mut h) = self.host.try_borrow_mut() {
            h.fetch_hook = Some(Rc::new(hook));
        }
    }

    pub fn url(&self) -> String {
        self.host.borrow().url_string()
    }

    pub fn ready_state(&self) -> ReadyState {
        self.host.borrow().ready
    }

    /// Pending navigation a script asked for (`location.href = …`, `form.submit()`).
    pub fn take_navigation(&self) -> Option<NavRequest> {
        self.host.borrow_mut().take_navigation()
    }

    pub fn problems(&self) -> Vec<tb_core::Problem> {
        self.host.borrow().problems.all().to_vec()
    }

    /// How many listeners are registered (diagnostics, tests).
    pub fn listener_count(&self) -> usize {
        self.host.borrow().listeners.len()
    }

    // --------------------------------------------------------------- script ---

    /// Run every `<script>` of the document, then `DOMContentLoaded`/`load`.
    pub fn run_scripts(
        &mut self,
        rt: &mut Runtime,
        fetch: impl FnMut(&str) -> Option<String>,
    ) -> ScriptReport {
        run_document(&self.host, &mut rt.interp, fetch, &ScriptOptions::default())
    }

    /// Like [`DomBindings::run_scripts`], with options.
    pub fn run_scripts_with(
        &mut self,
        rt: &mut Runtime,
        fetch: impl FnMut(&str) -> Option<String>,
        opts: &ScriptOptions,
    ) -> ScriptReport {
        run_document(&self.host, &mut rt.interp, fetch, opts)
    }

    /// Evaluate one script in the page's global scope.
    pub fn eval(&mut self, rt: &mut Runtime, src: &str) -> Result<Value, String> {
        rt.interp.run_script(src)
    }

    /// Let timers and microtasks run.
    pub fn pump(&mut self, rt: &mut Runtime, ms: f64) {
        pump(&self.host, &mut rt.interp, ms);
    }

    /// Dispatch an event from Rust (a real click, a `visibilitychange`, …).
    pub fn fire(&mut self, rt: &mut Runtime, target: EvtTarget, kind: &str) -> JsResult<Value> {
        events::fire(&self.host, &mut rt.interp, target, kind, true)
    }

    /// A fresh event object, ready to hand to `dispatchEvent`.
    pub fn new_event(&mut self, rt: &mut Runtime, kind: &str, bubbles: bool) -> Value {
        let proto = self.host.borrow().protos.event.clone();
        let o = Rc::new(std::cell::RefCell::new(Obj::plain()));
        o.borrow_mut()
            .define_value("bubbles", Value::Boolean(bubbles));
        o.borrow_mut()
            .define_value("cancelable", Value::Boolean(true));
        make_event(&mut rt.interp, kind, Some(Value::Object(o)), proto)
    }

    /// The wrapper object of a node (identity preserved).
    pub fn wrap(&mut self, id: NodeId) -> Value {
        self.host.borrow_mut().wrap(id)
    }
}

/// Build the prototype chain and install every property and method on it.
fn install_prototypes(it: &mut Interp, host: &HostRef) {
    let object_proto = it.natives.object_proto.clone();
    let mk = |_it: &mut Interp, proto: Option<ObjRef>| {
        let o = Obj::with_proto(proto.or(object_proto.clone()));
        Rc::new(std::cell::RefCell::new(o))
    };
    let event_target = mk(it, None);
    let node = mk(it, Some(event_target.clone()));
    let element = mk(it, Some(node.clone()));
    let html_element = mk(it, Some(element.clone()));
    let document = mk(it, Some(node.clone()));
    let text = mk(it, Some(node.clone()));
    let comment = mk(it, Some(node.clone()));
    let event = mk(it, None);
    let token_list = mk(it, None);
    let style_decl = mk(it, None);
    let dom_rect = mk(it, None);
    let location = mk(it, None);
    let navigator = mk(it, None);
    let history = mk(it, None);
    let screen = mk(it, None);
    let storage = mk(it, None);
    let media = mk(it, None);

    {
        let mut h = host.borrow_mut();
        h.protos.event_target = Some(event_target.clone());
        h.protos.node = Some(node.clone());
        h.protos.element = Some(element.clone());
        h.protos.html_element = Some(html_element.clone());
        h.protos.document = Some(document.clone());
        h.protos.text = Some(text.clone());
        h.protos.comment = Some(comment.clone());
        h.protos.event = Some(event.clone());
        h.protos.token_list = Some(token_list.clone());
        h.protos.style_decl = Some(style_decl.clone());
        h.protos.dom_rect = Some(dom_rect.clone());
        h.protos.location = Some(location.clone());
        h.protos.navigator = Some(navigator.clone());
        h.protos.history = Some(history.clone());
        h.protos.screen = Some(screen.clone());
        h.protos.storage = Some(storage.clone());
        h.protos.media = Some(media.clone());
    }

    events::install_event_target(it, host, &event_target);
    events::install_event_proto(it, &event);
    node::install(it, host, &node);
    element::install(it, host, &element);
    html::install(it, host, &html_element);
    events::install_handler_props(it, host, &html_element);
    events::install_handler_props(it, host, &document);
    document::install(it, host, &document);
    tokens::install(it, host, &token_list);
    style_api::install_style(it, host, &style_decl);
    // text/comment nodes only need Node, but a dedicated prototype keeps
    // `Object.getPrototypeOf(textNode)` honest
    let _ = (
        &text, &comment, &dom_rect, &navigator, &history, &screen, &storage, &media,
    );
}
