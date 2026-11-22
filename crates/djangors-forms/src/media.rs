//! Form and widget media asset management (`CSS` and `JS`).
//!
//! Provides [`FormMedia`] for declaring, merging, deduplicating, and rendering
//! stylesheet and script tags required by form widgets.

use std::collections::BTreeMap;

use crate::widgets::html_escape;

/// Static CSS and JavaScript assets required to render a form or widget.
///
/// Mirrors Django's `Media` class: CSS files are grouped by CSS media type
/// (e.g. `"all"`, `"screen"`, `"print"`), while JavaScript files are stored in
/// inclusion order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FormMedia {
    /// Mapping of CSS media type (e.g. `"all"`, `"screen"`) to stylesheet URLs.
    pub css: BTreeMap<String, Vec<String>>,
    /// Ordered list of JavaScript asset URLs.
    pub js: Vec<String>,
}

impl FormMedia {
    /// Creates an empty `FormMedia` instance.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a CSS stylesheet URL under the given media type if not already present.
    pub fn add_css(&mut self, media_type: impl Into<String>, url: impl Into<String>) {
        let urls = self.css.entry(media_type.into()).or_default();
        let url = url.into();
        if !urls.contains(&url) {
            urls.push(url);
        }
    }

    /// Appends a JavaScript asset URL if not already present.
    pub fn add_js(&mut self, url: impl Into<String>) {
        let url = url.into();
        if !self.js.contains(&url) {
            self.js.push(url);
        }
    }

    /// Builder helper to add a CSS stylesheet URL under the given media type.
    pub fn with_css(mut self, media_type: impl Into<String>, url: impl Into<String>) -> Self {
        self.add_css(media_type, url);
        self
    }

    /// Builder helper to add a JavaScript asset URL.
    pub fn with_js(mut self, url: impl Into<String>) -> Self {
        self.add_js(url);
        self
    }

    /// Returns `true` if no CSS or JavaScript assets are registered.
    pub fn is_empty(&self) -> bool {
        self.css.values().all(Vec::is_empty) && self.js.is_empty()
    }

    /// Merges `self` with `other`, deduplicating assets while preserving order.
    pub fn merge(&self, other: &FormMedia) -> FormMedia {
        let mut merged = self.clone();

        for (media_type, urls) in &other.css {
            for url in urls {
                merged.add_css(media_type.clone(), url.clone());
            }
        }

        for url in &other.js {
            merged.add_js(url.clone());
        }

        merged
    }

    /// Renders all registered CSS `<link>` tags followed by `<script>` tags as HTML.
    ///
    /// All URLs and media types are HTML-escaped.
    pub fn render_html(&self) -> String {
        let mut tags = Vec::new();

        for (media_type, urls) in &self.css {
            let esc_media = html_escape(media_type);
            for url in urls {
                let esc_url = html_escape(url);
                tags.push(format!(
                    "<link href=\"{}\" media=\"{}\" rel=\"stylesheet\">",
                    esc_url, esc_media
                ));
            }
        }

        for url in &self.js {
            let esc_url = html_escape(url);
            tags.push(format!("<script src=\"{}\"></script>", esc_url));
        }

        tags.join("\n")
    }
}
