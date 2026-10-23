//! Shared test harness: build a page from markup and evaluate JS in it.
#![allow(dead_code)] // not every test file uses every helper

use tb_dom::{Dom, NodeId};
use tb_dom_bindings::{node_of, DomBindings, EvtTarget, PageContext, ScriptOptions, ScriptReport};
use tb_js::value::Value;
use tb_js::Runtime;

pub const URL: &str = "http://example.test/p/index.html";

pub struct Page {
    pub b: DomBindings,
    pub rt: Runtime,
}

impl Page {
    /// A page at [`URL`] with `html` parsed into the DOM.
    pub fn html(html: &str) -> Page {
        Page::at(URL, html)
    }

    pub fn at(url: &str, html: &str) -> Page {
        let mut rt = Runtime::new();
        let ctx = PageContext {
            url: tb_net::Url::parse(url).ok(),
            ..Default::default()
        };
        let dom = tb_html::parse_document(html);
        let b = DomBindings::install(dom, ctx, &mut rt);
        Page { b, rt }
    }

    /// Evaluate in the page's global scope; panics with the JS error.
    pub fn js(&mut self, src: &str) -> Value {
        match self.b.eval(&mut self.rt, src) {
            Ok(v) => v,
            Err(e) => panic!("`{src}` threw: {e}"),
        }
    }

    pub fn num(&mut self, src: &str) -> f64 {
        self.js(src).to_number()
    }

    pub fn text(&mut self, src: &str) -> String {
        self.js(src).to_string()
    }

    pub fn yes(&mut self, src: &str) -> bool {
        self.js(src).to_boolean()
    }

    /// `true` when evaluating `src` throws.
    pub fn throws(&mut self, src: &str) -> bool {
        self.b.eval(&mut self.rt, src).is_err()
    }

    /// Run the document's `<script>`s with `bodies` as the external-script map.
    pub fn run(&mut self, bodies: &[(&str, &str)]) -> ScriptReport {
        self.run_in(ScriptOptions::default(), bodies)
    }

    pub fn run_in(&mut self, opts: ScriptOptions, bodies: &[(&str, &str)]) -> ScriptReport {
        self.b.run_scripts_with(
            &mut self.rt,
            |url| {
                bodies
                    .iter()
                    .find(|(k, _)| *k == url)
                    .map(|(_, v)| v.to_string())
            },
            &opts,
        )
    }

    /// Expose `id`s as globals, the way a browser's named window access does,
    /// so a test can say `d.textContent` instead of a long lookup.
    pub fn ids(&mut self, list: &[&str]) {
        for id in list {
            self.js(&format!(
                "window[\"{id}\"] = document.getElementById(\"{id}\"); 0"
            ));
        }
    }

    /// Let timers and microtasks run.
    pub fn pump(&mut self, ms: f64) {
        self.b.pump(&mut self.rt, ms);
    }

    /// Dispatch a real event from Rust.
    pub fn fire(&mut self, target: EvtTarget, kind: &str) -> Value {
        match self.b.fire(&mut self.rt, target, kind) {
            Ok(v) => v,
            Err(e) => panic!("firing {kind} threw: {}", e.to_string()),
        }
    }

    /// The node `sel` matches, as seen from JS.
    pub fn by_sel(&mut self, sel: &str) -> Option<NodeId> {
        let v = self.js(&format!("document.querySelector('{sel}')"));
        node_of(&v)
    }

    /// The `document` node id.
    pub fn document_id(&mut self) -> NodeId {
        let v = self.js("document");
        node_of(&v).expect("document has no node handle")
    }

    /// Event target of the first `sel` match.
    pub fn target(&mut self, sel: &str) -> EvtTarget {
        let v = self.js(&format!("document.querySelector('{sel}')"));
        node_of(&v).map(EvtTarget::node).expect("no such element")
    }

    pub fn body_html(&mut self) -> String {
        let v = self.js("document.body.innerHTML");
        v.to_string()
    }

    pub fn text_of(&mut self, sel: &str) -> String {
        self.js(&format!("document.querySelector('{sel}').textContent"))
            .to_string()
    }

    pub fn outer_of(&mut self, sel: &str) -> String {
        self.js(&format!("document.querySelector('{sel}').outerHTML"))
            .to_string()
    }

    /// Read the DOM directly (no JS) — what the layout pass would see.
    pub fn dom_of<T>(&self, f: impl FnOnce(&Dom) -> T) -> T {
        self.b.with_dom(f)
    }
}
