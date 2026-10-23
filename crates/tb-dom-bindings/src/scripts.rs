//! T8.4 — executing `<script>` for a loaded document.
//!
//! Scripts run in document order, inline and `src` alike; `defer`/`async`
//! change nothing (we have no parallel script execution, which is also the
//! deterministic choice for tests). `type="module"`, JSON-LD and unknown types
//! are *not* run as classic JavaScript, and one broken script never stops the
//! next one.

use tb_core::Phase;
use tb_dom::NodeId;
use tb_js::Interp;

use crate::host::{EvtTarget, HostRef, ReadyState};

/// How a script run went.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ScriptReport {
    /// scripts that were executed
    pub executed: usize,
    /// scripts we deliberately did not run (modules, JSON-LD, unknown types)
    pub skipped: usize,
    /// scripts that threw (parse error or runtime error)
    pub failed: usize,
}

impl ScriptReport {
    pub fn clean(&self) -> bool {
        self.failed == 0
    }
}

/// Options for [`run_document`].
#[derive(Debug, Clone)]
pub struct ScriptOptions {
    /// Run timers up to `now + timer_budget_ms` after the last script.
    /// The default of 1 ms covers `setTimeout(…, 0)`, which pages use to hop
    /// out of the current task; longer waits belong to an explicit pump.
    pub timer_budget_ms: f64,
    /// How often to re-scan for scripts that scripts added themselves.
    pub max_passes: usize,
    /// Fire `DOMContentLoaded` / `load`.
    pub fire_events: bool,
}

impl Default for ScriptOptions {
    fn default() -> Self {
        ScriptOptions {
            // enough for `setTimeout(…, 0)`, which pages use to hop out of the
            // current task; anything longer is an explicit pump
            timer_budget_ms: 1.0,
            max_passes: 3,
            fire_events: true,
        }
    }
}

/// Is this `type` classic JavaScript?
fn classic_type(t: &str) -> bool {
    let t = t.trim().to_ascii_lowercase();
    t.is_empty()
        || matches!(
            t.as_str(),
            "text/javascript"
                | "application/javascript"
                | "application/x-javascript"
                | "text/ecmascript"
                | "application/ecmascript"
                | "text/jscript"
                | "text/livescript"
                | "module" // handled below (skipped, but "we would run it")
        )
}

fn is_module(t: &str) -> bool {
    t.trim().eq_ignore_ascii_case("module")
}

/// All `<script>` elements in document order.
fn script_nodes(dom: &tb_dom::Dom) -> Vec<NodeId> {
    dom.get_elements_by_tag("script")
}

/// Run every script of the document.
///
/// `fetch(href)` returns the (already charset-decoded) body of an external
/// script or `None`; `tb-browser` wires it to `tb-net`, tests to a `&str` map.
pub fn run_document(
    host: &HostRef,
    it: &mut Interp,
    mut fetch: impl FnMut(&str) -> Option<String>,
    opts: &ScriptOptions,
) -> ScriptReport {
    let mut report = ScriptReport::default();
    let mut done: Vec<(u32, u32)> = Vec::new();
    for _pass in 0..opts.max_passes.max(1) {
        let nodes = {
            let h = match host.try_borrow() {
                Ok(h) => h,
                Err(_) => return report,
            };
            script_nodes(&h.dom)
        };
        let mut ran_this_pass = 0usize;
        for node in nodes {
            let key = node.raw();
            if done.contains(&key) {
                continue;
            }
            done.push(key);
            let (kind, src_text, module) = {
                let h = match host.try_borrow() {
                    Ok(h) => h,
                    Err(_) => return report,
                };
                if !h.dom.exists(node) {
                    continue;
                }
                let ty = h.dom.attr(node, "type").unwrap_or_default();
                if !classic_type(&ty) {
                    continue;
                }
                let module = is_module(&ty);
                if let Some(src) = h.dom.attr(node, "src") {
                    let url = crate::html::resolve_display(&h, &src);
                    (Some(url), None, module)
                } else {
                    (None, Some(h.dom.text_content(node)), module)
                }
            };
            if module {
                // ES modules need a loader, and `import`/`export` need a module
                // record: reported once per script and left alone (see NOTES)
                let url = kind.clone().unwrap_or_else(|| "inline".to_string());
                if let Ok(mut h) = host.try_borrow_mut() {
                    h.problems.warn_ctx(
                        Phase::Script,
                        "type=module skipped (no module loader)",
                        url,
                    );
                }
                report.skipped += 1;
                continue;
            }
            let source = match (kind.clone(), src_text) {
                (Some(url), None) => match fetch(&url) {
                    Some(text) => text,
                    None => {
                        if let Ok(mut h) = host.try_borrow_mut() {
                            h.problems.warn_ctx(Phase::Script, "script not loaded", url);
                        }
                        report.skipped += 1;
                        continue;
                    }
                },
                (None, Some(text)) => text,
                // a script with both `src` and inline text is `src`-only, and
                // an empty one is nothing at all
                (Some(url), Some(_)) => match fetch(&url) {
                    Some(text) => text,
                    None => continue,
                },
                (None, None) => continue,
            };
            if source.trim().is_empty() {
                continue;
            }
            ran_this_pass += 1;
            report.executed += 1;
            match it.run_script(&source) {
                Ok(_) => {}
                Err(msg) => {
                    report.failed += 1;
                    it.log("error", format!("[script] {msg}"));
                    // where the failure happened: an external url, or the first
                    // line of an inline script — "script error" alone is useless
                    let context = match &kind {
                        Some(url) => url.clone(),
                        None => {
                            let first = source
                                .lines()
                                .find(|l| !l.trim().is_empty())
                                .unwrap_or("")
                                .trim();
                            tb_core::text::truncate(first, 80).to_string()
                        }
                    };
                    if let Ok(mut h) = host.try_borrow_mut() {
                        h.problems
                            .warn_ctx(Phase::Script, format!("script error: {msg}"), context);
                    }
                }
            }
            if let Ok(mut h) = host.try_borrow_mut() {
                h.scripts_run += 1;
                // the script's `dataset` writes become attributes here
                h.flush_mirrors();
            }
            // a browser drains the microtask queue after each script
            it.run_jobs(20_000);
            if let Ok(mut h) = host.try_borrow_mut() {
                h.flush_mirrors();
            }
        }
        if ran_this_pass == 0 {
            break;
        }
    }
    if opts.fire_events {
        finish(host, it, opts);
    }
    report
}

/// `interactive` → `DOMContentLoaded` → `complete` → `load`.
fn finish(host: &HostRef, it: &mut Interp, opts: &ScriptOptions) {
    {
        let Ok(mut h) = host.try_borrow_mut() else {
            return;
        };
        h.ready = ReadyState::Interactive;
    }
    let doc_target = {
        let Ok(h) = host.try_borrow() else { return };
        EvtTarget::node(h.dom.document())
    };
    let _ = crate::events::fire(host, it, doc_target, "readystatechange", false);
    let _ = crate::events::fire(host, it, doc_target, "DOMContentLoaded", true);
    {
        let Ok(mut h) = host.try_borrow_mut() else {
            return;
        };
        h.ready = ReadyState::Complete;
    }
    let _ = crate::events::fire(host, it, doc_target, "readystatechange", false);
    let _ = crate::events::fire(host, it, EvtTarget::Window, "load", false);
    it.run_jobs(50_000);
    if opts.timer_budget_ms > 0.0 {
        pump(host, it, opts.timer_budget_ms);
    }
}

/// Let pending microtasks and timers run for `ms` of virtual time.
pub fn pump(host: &HostRef, it: &mut Interp, ms: f64) {
    it.run_jobs(100_000);
    let horizon = it.clock + ms.max(0.0);
    it.run_timers(horizon);
    it.run_jobs(100_000);
    if let Ok(mut h) = host.try_borrow_mut() {
        h.flush_mirrors();
    }
}
