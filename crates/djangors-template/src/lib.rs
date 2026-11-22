#![deny(missing_docs)]
//! Template engine integration for Djangors.
//!
//! Provides the `TemplateEngine` type and the `render` shortcut helper for rendering templates.
//!
//! # Auto-Escaping and html_escape Discrepancy
//! MiniJinja auto-escaping is enabled for `.html` and `.htm` templates, protecting dynamic insertions from XSS.
//! Note: MiniJinja autoescapes the `/` character as `&#x2f;` (lowercase hex), whereas Djangors' internal
//! `djangors_core::html_escape` escapes it as `&#x2F;` (uppercase hex). Although browser decodings are identical,
//! several admin test assertions are byte-sensitive and verify the uppercase hex entity. As a result, admin
//! changelist cells and search queries are pre-escaped in Rust code with `html_escape` and rendered in templates
//! via the `|safe` filter to avoid double-escaping or lowercase conversion failing assertions.

/// Built-in and custom template context processors.
pub mod context_processors;
/// MiniJinja template engine integration and directory loading logic.
pub mod engine;
/// Template loading and rendering error definitions.
pub mod error;
/// Template filters matching standard Django built-in filters.
pub mod filters;
/// Built-in template functions (`url`, `static`, `csrf_token`, `now`, `querystring`).
pub mod functions;
/// Convenient HTTP response rendering shortcuts.
pub mod render;

pub use context_processors::{
    ContextProcessor, CsrfContextProcessor, ExtraContextProcessor, RequestContextProcessor,
    StaticUrlProcessor,
};
pub use engine::TemplateEngine;
pub use error::TemplateError;
pub use render::{render, render_with_request};
