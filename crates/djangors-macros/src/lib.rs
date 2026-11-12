#![deny(missing_docs)]
//! Proc macros for the Djangors web framework
//!
//! Provides procedural derive macros `#[derive(Model)]` and `#[derive(Form)]`,
//! as well as attribute macros like `#[task]`.

use proc_macro::TokenStream;
use syn::{parse_macro_input, DeriveInput};

mod factory;
mod form;
mod management_command;
mod model;
mod settings;
mod task;

/// Derive macro for model factories (`#[derive(Factory)]`).
///
/// Generates a `<Model>Factory` builder struct and `Model::factory()` constructor
/// with sensible per-field defaults and `.with_<field>(...)`, `.build()`,
/// `.create(&db)`, and `.create_many(&db, count)` helpers.
#[proc_macro_derive(Factory, attributes(djangors))]
pub fn derive_factory(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    factory::expand_derive_factory(input)
        .unwrap_or_else(|err| err.to_compile_error())
        .into()
}

/// Derive macro for `Model` structs in the Djangors ORM.
///
/// Automatically generates table metadata, column mappings, and `QuerySet` helpers
/// for a struct representing a database table.
///
/// # Requirements and Constraints
/// Every `Model` struct must have an explicit primary key designated with `#[djangors(primary_key)]`,
/// or contain a field named `id` of an integer type.
#[proc_macro_derive(Model, attributes(djangors))]
pub fn derive_model(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    model::expand_derive_model(input)
        .unwrap_or_else(|err| err.to_compile_error())
        .into()
}

/// Derive macro for `Form` structs in Djangors.
///
/// Generates field-level validation and form cleaning methods for HTTP input mapping.
#[proc_macro_derive(Form, attributes(djangors))]
pub fn derive_form(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    form::expand_derive_form(input)
        .unwrap_or_else(|err| err.to_compile_error())
        .into()
}

/// Derive macro for typed, validated application settings structs.
///
/// The Djangors equivalent of `pydantic-settings`/`django-environ`: reads each
/// field from an environment variable, coercing it into the field's declared
/// type, with a compile-time-checked set of supported types (see
/// `djangors_core::settings::FromSettingsValue`) rather than the runtime-only
/// validation those Python tools provide.
///
/// ```ignore
/// #[derive(Settings, Debug)]
/// #[djangors(prefix = "MYAPP")]
/// struct MySettings {
///     api_key: String,                        // required - errors if MYAPP_API_KEY is unset
///     #[djangors(default = "https://api.example.com".to_string())]
///     base_url: String,                        // falls back to the default if unset
///     #[djangors(default = 30)]
///     timeout_secs: u64,
///     feature_flag: Option<bool>,               // None if MYAPP_FEATURE_FLAG is unset
/// }
///
/// let settings = MySettings::load()?;
/// ```
///
/// Field types must implement `djangors_core::settings::FromSettingsValue`
/// (implemented for `String`, `bool`, every built-in integer/float type, and
/// `Vec<String>` as a comma-separated list) or be `Option<T>` where `T` does.
#[proc_macro_derive(Settings, attributes(djangors))]
pub fn derive_settings(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    settings::expand_derive_settings(input)
        .unwrap_or_else(|err| err.to_compile_error())
        .into()
}

/// Attribute macro for registering background tasks.
///
/// Uses `inventory` for distributed task handler registration at link time,
/// allowing tasks to be invoked asynchronously across worker processes.
#[proc_macro_attribute]
pub fn task(attr: TokenStream, item: TokenStream) -> TokenStream {
    task::expand_task(attr.into(), item.into())
        .unwrap_or_else(|err| err.to_compile_error())
        .into()
}

/// Attribute macro for registering custom management commands.
///
/// Mirrors the `#[task]` macro structure: generates a wrapper function plus an
/// `inventory::submit!` block referencing `djangors_core::ManagementCommandRegistration`.
///
/// # Example
/// ```ignore
/// #[management_command]
/// async fn seed_data(args: Vec<String>) {
///     // custom logic here
/// }
///
/// #[management_command(name = "load")]
/// async fn load_fixtures(args: Vec<String>) {
///     // custom logic here
/// }
/// ```
#[proc_macro_attribute]
pub fn management_command(attr: TokenStream, item: TokenStream) -> TokenStream {
    management_command::expand_management_command(attr.into(), item.into())
        .unwrap_or_else(|err| err.to_compile_error())
        .into()
}
