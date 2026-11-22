use std::path::PathBuf;
use thiserror::Error;

/// Error type for djangors-template.
#[derive(Debug, Error)]
pub enum TemplateError {
    /// The template file was not found in any of the search directories.
    #[error("Template '{name}' not found. Searched in directories: {searched:?}")]
    NotFound {
        /// The name of the missing template.
        name: String,
        /// The directories searched.
        searched: Vec<PathBuf>,
    },

    /// An I/O error occurred while reading a template.
    #[error("I/O error reading template: {0}")]
    Io(#[from] std::io::Error),

    /// A named template partial was not found inside the template.
    #[error("Partial '{partial}' not found in template '{template}'")]
    PartialNotFound {
        /// The template that was searched.
        template: String,
        /// The name of the missing partial.
        partial: String,
    },

    /// A template parsing or rendering error from MiniJinja.
    #[error("MiniJinja error: {0}")]
    MiniJinja(#[from] minijinja::Error),
}
