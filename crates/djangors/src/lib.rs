#![deny(missing_docs)]
//! The Django of Rust: batteries-included web framework — ORM, migrations, admin, forms, auth, background tasks.

/// Background task execution and scheduling.
#[cfg(feature = "tasks")]
pub use djangors_tasks as tasks;

/// Django-style administration site and model administration.
#[cfg(feature = "admin")]
pub use djangors_admin as admin;
/// Authentication backends and user authentication events.
#[cfg(feature = "auth")]
pub use djangors_auth as auth;
/// In-memory, database, and Redis cache backends.
#[cfg(feature = "cache")]
pub use djangors_cache as cache;
/// Payment provider integration (Paystack, Bachs) with idempotent transaction recording.
#[cfg(feature = "payments")]
pub use djangors_contrib_payments as payments;
/// Multi-tenancy: tenant model, membership-verified request scoping.
#[cfg(feature = "tenancy")]
pub use djangors_contrib_tenancy as tenancy;
/// Core HTTP types: Router, Request, Response, middleware, and settings.
pub use djangors_core as core;
/// Database configuration and connection abstractions.
pub use djangors_db as db;
/// Form fields, validation, and form errors.
#[cfg(feature = "forms")]
pub use djangors_forms as forms;
/// Internationalization catalogs, locales, and middleware.
#[cfg(feature = "i18n")]
pub use djangors_i18n as i18n;
/// Mail messages and delivery backends.
#[cfg(feature = "mail")]
pub use djangors_mail as mail;
/// Database migration operations and planning.
#[cfg(feature = "migrations")]
pub use djangors_migrations as migrations;
/// ORM models, querysets, expressions, and metadata.
pub use djangors_orm as orm;
/// Typed PDF generation for report cards, invoices, and receipts.
#[cfg(feature = "pdf")]
pub use djangors_pdf as pdf;
/// REST serialization, authentication, and permissions.
#[cfg(feature = "rest")]
pub use djangors_rest as rest;
/// Session storage and signed cookie sessions.
#[cfg(feature = "sessions")]
pub use djangors_sessions as sessions;
/// Static-file collection, storage, and serving.
#[cfg(feature = "staticfiles")]
pub use djangors_staticfiles as staticfiles;
/// Template rendering engine and filters.
#[cfg(feature = "template")]
pub use djangors_template as template;

#[cfg(test)]
mod tests;
