//! Modern versioned REST API example using the `djangors` facade crate.

pub use djangors::orm as djangors_orm;

pub mod models;
pub mod urls;
pub mod views;
