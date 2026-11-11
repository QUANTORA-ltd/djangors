#![deny(missing_docs)]
//! Database backends and connection pool management for the Djangors web framework.
//!
//! This crate provides connection pooling, config-driven database setup,
//! and transaction support with explicit isolation levels, built on top of SQLx.

#[cfg(not(any(feature = "postgres", feature = "sqlite")))]
compile_error!("djangors-db requires at least one backend feature: enable `postgres` or `sqlite`.");

/// Bind parameter values and NULL kinds.
pub mod bind;
/// Connection pool and database configuration options.
pub mod config;
/// Core database connection wrapper and transaction helpers.
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub mod database;
/// Database dialects for SQL syntax.
pub mod dialect;
/// Database errors and failure types.
pub mod error;
/// Query execution targets: run against the pool or inside a transaction.
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub mod executor;
/// Backend-agnostic row abstraction.
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub mod row;

pub use bind::{BindValue, NullKind};
pub use config::DatabaseConfig;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
#[doc(hidden)]
pub use database::{isolation_level_sql, BoxFuture};
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use database::{Database, IsolationLevel};
pub use dialect::{DatePart, Dialect};
pub use error::DbError;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use executor::{Conn, DbExecutor};
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub use row::DbRow;
