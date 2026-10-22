//! Engine configuration — one value object that parameterises the whole
//! pipeline (viewport, capabilities, budgets, user agent).

use crate::geometry::Size;

/// Colour scheme preference (`prefers-color-scheme`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorScheme {
    #[default]
    Light,
    Dark,
}

/// Which pipeline stages are enabled. Tests switch scripts/images off to get
/// deterministic renders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    /// Fetch sub-resources (`<link rel=stylesheet>`, `<script src>`, ...).
    pub fetch_subresources: bool,
    /// Fetch and apply CSS.
    pub apply_css: bool,
    /// Execute `<script>` and inline event handlers.
    pub run_scripts: bool,
    /// Build a box tree and lay it out.
    pub layout: bool,
    /// Decode/lay out images (`<img>` boxes).
    pub images: bool,
}

impl Default for Capabilities {
    fn default() -> Self {
        Capabilities {
            fetch_subresources: true,
            apply_css: true,
            run_scripts: true,
            layout: true,
            images: true,
        }
    }
}

impl Capabilities {
    /// Nothing but HTML parsing.
    pub fn markup_only() -> Self {
        Capabilities {
            fetch_subresources: false,
            apply_css: false,
            run_scripts: false,
            layout: false,
            images: false,
        }
    }
}

/// All knobs of the browser in one place.
#[derive(Debug, Clone, PartialEq)]
pub struct EngineConfig {
    /// Layout viewport (css px).
    pub viewport: Size,
    /// Device pixel ratio used when rasterising screenshots.
    pub scale: f32,
    /// `User-Agent` request header.
    pub user_agent: String,
    /// Per request connect/read timeout in milliseconds.
    pub timeout_ms: u64,
    /// Maximum number of followed redirects.
    pub max_redirects: usize,
    /// Maximum accepted body size in bytes.
    pub max_resource_bytes: usize,
    /// Maximum number of fetched sub-resources per page.
    pub max_subresources: usize,
    /// Maximum number of JavaScript statements executed per page.
    pub max_script_steps: u64,
    /// Feature switches.
    pub caps: Capabilities,
    /// `prefers-color-scheme`.
    pub color_scheme: ColorScheme,
    /// Send/accept cookies.
    pub cookies: bool,
    /// Referrer policy: send `Referer` on navigation.
    pub send_referer: bool,
}

impl Default for EngineConfig {
    fn default() -> Self {
        EngineConfig {
            viewport: Size::new(1280.0, 1024.0),
            scale: 1.0,
            user_agent: crate::USER_AGENT.to_string(),
            timeout_ms: 20_000,
            max_redirects: 10,
            max_resource_bytes: 32 * 1024 * 1024,
            max_subresources: 64,
            max_script_steps: 4_000_000,
            caps: Capabilities::default(),
            color_scheme: ColorScheme::default(),
            cookies: true,
            send_referer: true,
        }
    }
}

impl EngineConfig {
    /// Config with everything but HTML disabled (used by parser unit tests).
    pub fn markup_only() -> Self {
        EngineConfig {
            caps: Capabilities::markup_only(),
            ..Default::default()
        }
    }

    /// Physical pixel size of a screenshot for the current viewport.
    pub fn pixel_size(&self) -> Size {
        Size::new(
            (self.viewport.width * self.scale).round(),
            (self.viewport.height * self.scale).round(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_a_reasonable_desktop() {
        let c = EngineConfig::default();
        assert_eq!(c.viewport.width, 1280.0);
        assert!(c.caps.run_scripts);
        assert_eq!(c.user_agent, crate::USER_AGENT);
    }

    #[test]
    fn markup_only_disables_the_world() {
        let c = EngineConfig::markup_only();
        assert!(!c.caps.run_scripts);
        assert!(!c.caps.apply_css);
        assert!(!c.caps.fetch_subresources);
    }

    #[test]
    fn pixel_size_uses_scale() {
        let mut c = EngineConfig::default();
        c.scale = 2.0;
        assert_eq!(c.pixel_size().width, 2560.0);
    }
}
