#![deny(missing_docs)]
//! Django-style ORM with querysets and model metadata for Djangors.
//!
//! This crate defines runtime metadata types (`ModelMeta`, `FieldMeta`, etc.)
//! which describe database models. These metadata structures serve as the
//! single source of truth for migrations, serializers, admin interfaces, and more.

/// Aggregate function definitions and result types.
pub mod aggregate;
/// Error types and row-mapping trait for ORM operations.
pub mod error;
/// Query expression tree nodes, operators, and macros.
pub mod expr;
/// Zero-intermediate-allocation JSON byte-buffer writers for models.
pub mod json;
/// Model runtime metadata structures and field definitions.
pub mod meta;
/// Process-local query result cache and invalidation.
pub mod query_cache;
/// Type-safe fluent QuerySet query builder.
pub mod queryset;
/// Model lifecycle signals.
pub mod signals;

pub use djangors_db;
pub use djangors_forms;
pub use inventory;
pub use sqlx;

pub use djangors_db::{BindValue, DbRow, Dialect, NullKind};
pub use meta::{
    all_registered_models, m2m_through_table, DefaultValue, FieldKind, FieldMeta, FieldSnapshot,
    ForeignKey, IndexMeta, IndexSnapshot, Manager, ManyToMany, Model, ModelDatabaseMap, ModelForm,
    ModelMeta, ModelRegistration, ModelSnapshot, OnDelete, OneToOne, RelationKind, RelationMeta,
    RelationSnapshot, SnapshotDefault,
};

pub use aggregate::{AggExpr, AggResult, FuncExpr};
pub use error::{FromRow, OrmError};
pub use expr::{
    ArithOp, CompareOp, Exists, Expr, IntoSetExpr, NotExists, OuterRef, SetExpr, UnresolvedCompare,
    UnresolvedExpr, UnresolvedFieldCompare, UnresolvedOuterCompare, Value, F,
};
pub use queryset::{m2m_add, m2m_related, m2m_remove, prefetch_related, GroupRow, QuerySet};

#[cfg(test)]
#[allow(unused_extern_crates)]
extern crate self as djangors_orm;

#[cfg(test)]
mod tests;
