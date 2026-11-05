#![deny(missing_docs)]
//! Database backends and connection pool management for the Djangors web framework.
//!
//! This crate provides connection pooling, config-driven database setup,
//! and transaction support with explicit isolation levels, built on top of SQLx.

/// Bind parameter values and NULL kinds.
pub mod bind;
/// Connection pool and database configuration options.
pub mod config;
/// Core database connection wrapper and transaction helpers.
pub mod database;
/// Database dialects for SQL syntax.
pub mod dialect;
/// Database errors and failure types.
pub mod error;
/// Query execution targets: run against the pool or inside a transaction.
pub mod executor;
/// Backend-agnostic row abstraction.
pub mod row;

pub use bind::{BindValue, NullKind};
pub use config::DatabaseConfig;
#[doc(hidden)]
pub use database::{isolation_level_sql, BoxFuture};
pub use database::{Database, IsolationLevel};
pub use dialect::{DatePart, Dialect};
pub use error::DbError;
pub use executor::{Conn, DbExecutor, QueryEvent};
pub use row::DbRow;
