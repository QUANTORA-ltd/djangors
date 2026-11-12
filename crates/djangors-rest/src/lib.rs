#![deny(missing_docs)]
//! REST framework core for Djangors: serializers, ViewSets, permissions,
//! pagination, and router mounting.
//!
//! ViewSet routes require an authenticated user by default. Public routes must opt into
//! [`AllowAny`] explicitly through [`viewset_routes_with_permission`].

/// Machine-to-machine API key authentication, scopes, and admin registration.
pub mod api_keys;
/// API authentication: database-backed tokens and JWT.
pub mod auth;

/// Composable query-string filter backends for ViewSets.
pub mod filters;
/// OpenAPI 3.1 schema generation from model metadata.
pub mod openapi;
/// Pluggable pagination strategies for list endpoints.
pub mod pagination;
/// Permission policies and combinators for ViewSet routes.
pub mod permissions;
/// Model serialization, field sets, and the serializer trait.
pub mod serializers;
/// Field-level validation errors and the validator trait.
pub mod validation;

/// DRF-style request throttling.
pub mod throttling;
/// Generic ViewSet controllers and route mounting.
pub mod viewsets;

pub use api_keys::*;
pub use auth::*;
pub use filters::*;
pub use openapi::*;
pub use pagination::*;
pub use permissions::*;
pub use serializers::*;
pub use throttling::*;
pub use validation::*;
pub use viewsets::*;

use std::sync::Arc;

use djangors_core::error::DjangorsError;
use djangors_core::path_params::PathParams;
use djangors_core::request::Request;
use djangors_core::response::Response;

fn guarded<P, F, Fut>(permission: Arc<P>, handler: F) -> impl djangors_core::Handler
where
    P: Permission,
    F: Fn(Request, PathParams) -> Fut + Copy + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<Response, DjangorsError>> + Send + 'static,
{
    move |req: Request, params: PathParams| {
        let permission = permission.clone();
        async move {
            if !permission.has_permission(&req).await {
                return Err(DjangorsError::Unauthorized("not authenticated".to_string()));
            }
            handler(req, params).await
        }
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

#[cfg(test)]
#[path = "serializer_tests.rs"]
mod serializer_tests;

#[cfg(test)]
#[path = "filter_backend_tests.rs"]
mod filter_backend_tests;

#[cfg(test)]
#[path = "api_keys_tests.rs"]
mod api_keys_tests;
