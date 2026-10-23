//! Pictures: fetch `<img src>`, decode it, remember what it turned into.
//!
//! A browser does not paint a picture where the author wrote one — it paints
//! the picture *it managed to get*. So this module answers three questions and
//! remembers the answers for the rest of the pipeline:
//!
//! * **Which natural size** does each `<img>` have? → [`tb_layout::IntrinsicSizes`],
//!   so layout stops guessing 300×150 and a page stops jumping around when the
//!   first picture lands.
//! * **Which pixels** go into which box? → [`Page::image_nodes`][crate::Page],
//!   consulted by the painter; a missing or broken picture becomes the
//!   placeholder, exactly like a picture that never arrived.
//! * **What went wrong?** → a `Problem` per failed URL, never an error.
//!
//! Decoding is PNG only ([`tb_gfx::image`]); JPEG/WebP need their own decoders
//! and are reported as such instead of silently becoming nothing.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use tb_core::{Phase, Problems};
use tb_dom::{Dom, NodeId};
use tb_gfx::Image;

use crate::resources::SharedLoader;

/// The decoded pictures of one page, keyed by absolute URL.
///
/// `None` means "we tried and it is not a picture" — remembered so a page with
/// 40 broken thumbnails asks once each, not once per paint.
#[derive(Clone, Default)]
pub struct ImageStore {
    by_url: Rc<RefCell<HashMap<String, Option<Rc<Image>>>>>,
}

impl std::fmt::Debug for ImageStore {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let b = self.by_url.borrow();
        let ok = b.values().filter(|v| v.is_some()).count();
        write!(f, "ImageStore({ok} decoded, {} failed)", b.len() - ok)
    }
}

impl ImageStore {
    pub fn new() -> ImageStore {
        ImageStore::default()
    }

    pub fn get(&self, url: &str) -> Option<Rc<Image>> {
        self.by_url.borrow().get(url).and_then(|v| v.clone())
    }

    pub fn decoded(&self) -> usize {
        self.by_url
            .borrow()
            .values()
            .filter(|v| v.is_some())
            .count()
    }

    /// Did we already try this absolute URL? (So "no picture" can be told
    /// apart from "never asked" — a lazy image is not a broken one.)
    pub fn knows_url(&self, url: &str) -> bool {
        self.by_url.borrow().contains_key(url)
    }

    pub fn failed(&self) -> usize {
        self.by_url
            .borrow()
            .values()
            .filter(|v| v.is_none())
            .count()
    }

    fn remember(&self, url: &str, img: Option<Rc<Image>>) {
        self.by_url.borrow_mut().insert(url.to_string(), img);
    }
}

/// Fetch and decode every `<img src>` in `dom` that we do not have yet.
///
/// Returns the natural sizes keyed by node (for layout) and the picture for
/// each node (for painting). Both are cheap to rebuild — already-decoded URLs
/// come from the store, nothing is fetched twice.
pub fn load_images(
    dom: &Dom,
    loader: &SharedLoader,
    store: &ImageStore,
    problems: &mut Problems,
    want: bool,
) -> (tb_layout::IntrinsicSizes, HashMap<NodeId, Rc<Image>>) {
    let mut intrinsic = tb_layout::IntrinsicSizes::new();
    let mut nodes: HashMap<NodeId, Rc<Image>> = HashMap::new();
    if !want {
        return (intrinsic, nodes);
    }
    for id in dom.elements() {
        if dom.tag(id) != Some("img") {
            continue;
        }
        let Some(src) = dom.attr(id, "src") else {
            continue;
        };
        let src = src.trim();
        if src.is_empty() {
            continue;
        }
        let Some(url) = loader.borrow().resolve(src) else {
            continue;
        };
        let key = url.to_string();
        // looked up and released before anything else runs: the borrow must not
        // outlive this statement, `remember` needs the store to itself
        let known = store.by_url.borrow().get(&key).cloned();
        let img = match known {
            Some(known) => known,
            None => {
                let loaded = loader.borrow_mut().load_binary(src);
                let decoded = loaded.and_then(|l| match tb_gfx::image::from_png(&l.bytes) {
                    Ok(img) => Some(Rc::new(img)),
                    Err(e) => {
                        problems.warn(Phase::Browser, format!("picture {key}: {e}"));
                        None
                    }
                });
                store.remember(&key, decoded.clone());
                decoded
            }
        };
        if let Some(img) = img {
            intrinsic.insert(id, img.width as f32, img.height as f32);
            nodes.insert(id, img);
        }
    }
    (intrinsic, nodes)
}
