#![deny(missing_docs)]
//! Static file collection, hashing, and serving for Djangors.

/// Optional malware/virus scanning of uploaded file bytes via a `clamd` daemon.
/// Requires the `clamav` Cargo feature.
#[cfg(feature = "clamav")]
pub mod clamav;
/// Static file collection and manifest creation.
pub mod collect;
/// Static file processing and loading errors.
pub mod error;
/// Static file serving handler logic.
pub mod serve;
/// Pluggable static-file storage backends.
pub mod storage;
/// Helpers for writing uploaded files through a storage backend.
pub mod upload;

pub use collect::Manifest;
pub use error::StaticFilesError;
pub use serve::{StaticFiles, StaticFilesHandler};
#[cfg(feature = "s3")]
pub use storage::S3Storage;
pub use storage::{LocalDiskStorage, Storage};
