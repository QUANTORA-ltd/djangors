//! Typed HTTP endpoint metadata, JSON Schema generation, and router mount tracking.
//!
//! Used by `#[endpoint]` and `#[router]` in `djangors-macros`, tracked across
//! nested [`Router::mount`](crate::Router::mount) calls, and consumed by
//! `djangors-rest`'s OpenAPI 3.1 builder and `dj openapi --typescript`.

use std::any::TypeId;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use schemars::generate::{SchemaGenerator, SchemaSettings};
use schemars::JsonSchema;

static NEXT_ROUTE_ID: AtomicU64 = AtomicU64::new(1);
static MOUNTED_ROUTES: OnceLock<Mutex<Vec<MountedRouteEntry>>> = OnceLock::new();

fn mounted_routes_store() -> &'static Mutex<Vec<MountedRouteEntry>> {
    MOUNTED_ROUTES.get_or_init(|| Mutex::new(Vec::new()))
}

pub(crate) fn next_route_id() -> u64 {
    NEXT_ROUTE_ID.fetch_add(1, Ordering::Relaxed)
}

fn default_type_id() -> TypeId {
    TypeId::of::<()>()
}

/// JSON Schema representation for a single request body, query struct, or response payload,
/// together with any referenced component definitions.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EndpointSchema {
    /// Component schema name when `T` is a named struct or enum (`!T::inline_schema()`).
    pub name: Option<String>,
    /// OpenAPI 3.1 JSON Schema (or `{"$ref": "#/components/schemas/<Name>"}`) for the type.
    pub schema: serde_json::Value,
    /// Named subschemas referenced by `schema`, keyed by schema name.
    pub definitions: BTreeMap<String, serde_json::Value>,
}

/// Generates an OpenAPI 3.1-compatible [`EndpointSchema`] for `T` using its serialization contract
/// (for response payloads).
pub fn schema_for_serialize<T: JsonSchema>() -> EndpointSchema {
    generate_endpoint_schema::<T>(SchemaSettings::openapi3().for_serialize().into_generator())
}

/// Generates an OpenAPI 3.1-compatible [`EndpointSchema`] for `T` using its deserialization
/// contract (for request bodies and query parameters).
pub fn schema_for_deserialize<T: JsonSchema>() -> EndpointSchema {
    generate_endpoint_schema::<T>(
        SchemaSettings::openapi3()
            .for_deserialize()
            .into_generator(),
    )
}

/// Alias for [`schema_for_serialize`].
pub fn schema_for<T: JsonSchema>() -> EndpointSchema {
    schema_for_serialize::<T>()
}

fn generate_endpoint_schema<T: JsonSchema>(mut generator: SchemaGenerator) -> EndpointSchema {
    let mut sub = generator.subschema_for::<T>();
    for transform in generator.transforms_mut() {
        transform.transform(&mut sub);
    }

    let mut schema_val = sub.to_value();
    rewrite_component_refs(&mut schema_val);

    let mut definitions = BTreeMap::new();
    for (k, mut v) in generator.take_definitions(true) {
        rewrite_component_refs(&mut v);
        definitions.insert(k, v);
    }

    let name = if T::inline_schema() {
        None
    } else {
        Some(T::schema_name().into_owned())
    };

    EndpointSchema {
        name,
        schema: schema_val,
        definitions,
    }
}

/// Rewrites JSON Schema `$defs` / `definitions` pointers into OpenAPI `#/components/schemas/` pointers.
fn rewrite_component_refs(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(serde_json::Value::String(r)) = map.get_mut("$ref") {
                if let Some(rest) = r.strip_prefix("#/$defs/") {
                    *r = format!("#/components/schemas/{rest}");
                } else if let Some(rest) = r.strip_prefix("#/definitions/") {
                    *r = format!("#/components/schemas/{rest}");
                }
            }
            for v in map.values_mut() {
                rewrite_component_refs(v);
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr {
                rewrite_component_refs(v);
            }
        }
        _ => {}
    }
}

/// Compile-time / link-time registration record emitted by `#[endpoint(...)]`.
pub struct EndpointRegistration {
    /// Rust identifier name of the handler function.
    pub fn_name: &'static str,
    /// Rust `module_path!()` where the handler was defined.
    pub module_path: &'static str,
    /// Uppercase HTTP method (e.g. `"GET"`, `"POST"`).
    pub method: &'static str,
    /// Route path declared on `#[endpoint(...)]` (e.g. `"/payments/{id}/receipt"`).
    pub path: &'static str,
    /// Optional explicit prefix override declared on `#[endpoint(..., prefix = "...")]`.
    pub prefix: Option<&'static str>,
    /// Optional client/tag group declared on `#[endpoint(..., tag = "...")]`.
    pub tag: Option<&'static str>,
    /// Optional client method name override declared on `#[endpoint(..., name = "...")]`.
    pub operation_name: Option<&'static str>,
    /// Optional short summary or first doc-comment line from the handler.
    pub summary: Option<&'static str>,
    /// HTTP status code for the primary response (defaults to `200`, or `204` when untyped).
    pub status: u16,
    /// Returns the concrete `TypeId` of the annotated handler function item.
    pub handler_type_id: fn() -> TypeId,
    /// Returns `std::any::type_name` for the annotated handler function item.
    pub handler_type_name: fn() -> &'static str,
    /// Optional JSON Schema generator for the request body type.
    pub request_schema: Option<fn() -> EndpointSchema>,
    /// Optional JSON Schema generator for the query parameter struct type.
    pub query_schema: Option<fn() -> EndpointSchema>,
    /// Optional JSON Schema generator for the response payload type.
    pub response_schema: Option<fn() -> EndpointSchema>,
}

inventory::collect!(EndpointRegistration);

/// Registration record emitted by `#[router]` on a root `fn() -> Router` builder so `dj openapi`
/// can execute the route composition tree during introspection.
pub struct RouterRegistration {
    /// Builder function returning the composed application [`Router`](crate::Router).
    pub build: fn() -> crate::Router,
}

inventory::collect!(RouterRegistration);

/// Returns `TypeId::of::<T>()` given a reference to a handler function item.
#[doc(hidden)]
pub fn handler_type_id_of_val<T: 'static>(_: &T) -> TypeId {
    TypeId::of::<T>()
}

/// Returns `std::any::type_name::<T>()` given a reference to a handler function item.
#[doc(hidden)]
pub fn handler_type_name_of_val<T: 'static>(_: &T) -> &'static str {
    std::any::type_name::<T>()
}

/// Runtime route mount record maintained by [`Router`](crate::Router) across `route` and `mount`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MountedRouteEntry {
    /// Monotonic route identifier assigned at initial registration.
    pub route_id: u64,
    /// Concrete `TypeId` of the handler function when registered as a typed `H: Handler`.
    #[serde(skip, default = "default_type_id")]
    pub handler_type_id: TypeId,
    /// Rust `type_name` of the handler function.
    pub handler_type_name: String,
    /// Uppercase HTTP method.
    pub method: String,
    /// Local path registered on the inner router (e.g. `"/payments/{id:i64}/receipt"`).
    pub local_path: String,
    /// Combined mount prefix accumulated through `Router::mount` calls (e.g. `"/api/v1/billing"`).
    pub mount_prefix: String,
    /// Full mounted path including prefix and local path (e.g. `"/api/v1/billing/payments/{id:i64}/receipt"`).
    pub full_path: String,
    /// Ordered prefix segments added by outer `Router::mount` calls (e.g. `["api", "v1", "billing"]`).
    pub mount_segments: Vec<String>,
}

pub(crate) fn format_slash_path(pattern: &str) -> String {
    let trimmed = pattern.trim_matches('/');
    if trimmed.is_empty() {
        "/".to_string()
    } else {
        format!("/{trimmed}")
    }
}

pub(crate) fn join_mount_prefix(outer_prefix: &str, inner_prefix: &str) -> String {
    let outer = outer_prefix.trim_matches('/');
    let inner = inner_prefix.trim_matches('/');
    match (outer.is_empty(), inner.is_empty()) {
        (true, true) => String::new(),
        (false, true) => format!("/{outer}"),
        (true, false) => format!("/{inner}"),
        (false, false) => format!("/{outer}/{inner}"),
    }
}

pub(crate) fn split_segments(path: &str) -> Vec<String> {
    path.split('/')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

pub(crate) fn record_route_registration(
    route_id: u64,
    handler_type_id: TypeId,
    handler_type_name: &str,
    method: &str,
    pattern: &str,
) {
    let local_path = format_slash_path(pattern);
    let mut store = mounted_routes_store()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    store.push(MountedRouteEntry {
        route_id,
        handler_type_id,
        handler_type_name: handler_type_name.to_string(),
        method: method.to_ascii_uppercase(),
        local_path: local_path.clone(),
        mount_prefix: String::new(),
        full_path: local_path,
        mount_segments: Vec::new(),
    });
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn record_route_mount(
    route_id: u64,
    handler_type_id: Option<TypeId>,
    handler_type_name: Option<&str>,
    method: &str,
    prev_mount_prefix: &str,
    new_mount_prefix: &str,
    new_mount_segments: &[String],
    full_pattern: &str,
) {
    let Some(type_id) = handler_type_id else {
        return;
    };
    let full_path = format_slash_path(full_pattern);
    let mut store = mounted_routes_store()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if let Some(entry) = store
        .iter_mut()
        .rev()
        .find(|e| e.route_id == route_id && e.mount_prefix == prev_mount_prefix)
    {
        entry.mount_prefix = new_mount_prefix.to_string();
        entry.mount_segments = new_mount_segments.to_vec();
        entry.full_path = full_path;
    } else {
        let local_path = handler_type_name
            .and_then(|_| {
                store
                    .iter()
                    .find(|e| e.route_id == route_id)
                    .map(|e| e.local_path.clone())
            })
            .unwrap_or_else(|| full_path.clone());
        store.push(MountedRouteEntry {
            route_id,
            handler_type_id: type_id,
            handler_type_name: handler_type_name.unwrap_or_default().to_string(),
            method: method.to_ascii_uppercase(),
            local_path,
            mount_prefix: new_mount_prefix.to_string(),
            full_path,
            mount_segments: new_mount_segments.to_vec(),
        });
    }
}

/// Infers an app/group name from accumulated router mount segments.
///
/// For `["api", "v1", "billing"]` this returns `Some("billing")`; for `["api", "v1"]`
/// it returns `None` because `"api"` and version tags (`v1`, `v2`, ...) are API version
/// prefixes rather than app namespaces.
pub(crate) fn infer_app_from_segments(segments: &[String]) -> Option<String> {
    segments
        .iter()
        .rev()
        .find(|seg| !is_api_or_version_segment(seg))
        .cloned()
}

fn is_api_or_version_segment(seg: &str) -> bool {
    let lower = seg.to_ascii_lowercase();
    if lower == "api" {
        return true;
    }
    if let Some(rest) = lower.strip_prefix('v') {
        return !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit() || c == '.');
    }
    false
}

/// Serializable snapshot of a registered typed endpoint, combining compile-time metadata
/// from `#[endpoint]` with any mounted prefix resolved through [`Router::mount`](crate::Router::mount).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EndpointSnapshot {
    /// Rust handler function identifier.
    pub fn_name: String,
    /// Rust `module_path!()` where the handler was declared.
    pub module_path: String,
    /// Uppercase HTTP method (e.g. `"GET"`, `"POST"`).
    pub method: String,
    /// Path declared on `#[endpoint]` before outer router prefixes.
    pub path: String,
    /// Optional prefix explicitly supplied on `#[endpoint(..., prefix = "...")]`.
    pub prefix: Option<String>,
    /// Optional explicit tag/group supplied on `#[endpoint(..., tag = "...")]`.
    pub tag: Option<String>,
    /// Optional explicit method name override supplied on `#[endpoint(..., name = "...")]`.
    pub operation_name: Option<String>,
    /// Optional summary / docstring line for OpenAPI documentation.
    pub summary: Option<String>,
    /// HTTP success status code (e.g. `200`, `201`, `204`).
    pub status: u16,
    /// Rust `type_name` of the handler function.
    pub handler_type_name: String,
    /// Exact mount prefix discovered from `Router::mount` (e.g. `"/api/v1/billing"`), if mounted.
    pub mounted_prefix: Option<String>,
    /// Exact full mounted path discovered from `Router::mount`, if mounted.
    pub mounted_full_path: Option<String>,
    /// App/namespace segment inferred from `mounted_prefix` (e.g. `"billing"`), if mounted.
    pub mounted_app: Option<String>,
    /// Schema for the JSON request body, if declared.
    pub request_schema: Option<EndpointSchema>,
    /// Schema for query parameters, if declared.
    pub query_schema: Option<EndpointSchema>,
    /// Schema for the response body, if declared.
    pub response_schema: Option<EndpointSchema>,
}

/// Complete OpenAPI introspection payload emitted when `DJANGORS_INTROSPECT_OPENAPI=1` is set.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct OpenApiIntrospection {
    /// Registered `#[derive(Model)]` snapshots.
    pub models: Vec<djangors_orm::ModelSnapshot>,
    /// Registered `#[endpoint]` snapshots (enriched with router mount prefixes when available).
    pub endpoints: Vec<EndpointSnapshot>,
}

/// Collects all `#[endpoint]` registrations and enriches them with mounted prefixes recorded
/// by [`Router`](crate::Router) (invoking any `#[router]` builder first if present).
pub fn registered_endpoints_snapshot() -> Vec<EndpointSnapshot> {
    let mut built_routers: Vec<crate::Router> = Vec::new();
    for reg in inventory::iter::<RouterRegistration>() {
        built_routers.push((reg.build)());
    }

    if !built_routers.is_empty() {
        let mut combined: Vec<EndpointSnapshot> = Vec::new();
        for router in &built_routers {
            for snap in registered_endpoints_for_router(router) {
                let already = combined.iter().any(|existing| {
                    existing.handler_type_name == snap.handler_type_name
                        && existing.mounted_full_path == snap.mounted_full_path
                        && existing.method == snap.method
                });
                if !already {
                    combined.push(snap);
                }
            }
        }
        return combined;
    }

    let entries = mounted_routes_store()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    snapshots_from_entries(&entries, true)
}

/// Collects all `#[endpoint]` registrations matched against the routes actually mounted on `router`.
///
/// Endpoints mounted on `router` receive their exact `mounted_prefix`, `mounted_full_path`,
/// and `mounted_app` from `router` without guessing paths from source files.
pub fn registered_endpoints_for_router(router: &crate::Router) -> Vec<EndpointSnapshot> {
    let entries = router.mounted_routes();
    snapshots_from_entries(&entries, false)
}

/// Like [`registered_endpoints_for_router`], but only returns endpoints that are actually mounted
/// on `router` (ignoring unmounted `#[endpoint]` functions elsewhere in the binary).
pub fn mounted_endpoints_for_router(router: &crate::Router) -> Vec<EndpointSnapshot> {
    let entries = router.mounted_routes();
    let mut out = Vec::new();
    for reg in inventory::iter::<EndpointRegistration>() {
        let tid = (reg.handler_type_id)();
        let tname = (reg.handler_type_name)();
        let matches: Vec<&MountedRouteEntry> = entries
            .iter()
            .filter(|e| {
                e.handler_type_id == tid || (!tname.is_empty() && e.handler_type_name == tname)
            })
            .collect();
        for entry in matches {
            out.push(build_snapshot(reg, Some(entry)));
        }
    }
    sort_snapshots(&mut out);
    out
}

fn snapshots_from_entries(
    entries: &[MountedRouteEntry],
    prefer_deepest_mount: bool,
) -> Vec<EndpointSnapshot> {
    let mut out = Vec::new();
    for reg in inventory::iter::<EndpointRegistration>() {
        let tid = (reg.handler_type_id)();
        let tname = (reg.handler_type_name)();
        let mut matches: Vec<&MountedRouteEntry> = entries
            .iter()
            .filter(|e| {
                e.handler_type_id == tid || (!tname.is_empty() && e.handler_type_name == tname)
            })
            .collect();

        if prefer_deepest_mount {
            matches.sort_by_key(|e| std::cmp::Reverse((e.mount_segments.len(), e.full_path.len())));
            out.push(build_snapshot(reg, matches.into_iter().next()));
        } else if matches.is_empty() {
            out.push(build_snapshot(reg, None));
        } else {
            for entry in matches {
                out.push(build_snapshot(reg, Some(entry)));
            }
        }
    }
    sort_snapshots(&mut out);
    out
}

fn sort_snapshots(snapshots: &mut [EndpointSnapshot]) {
    snapshots.sort_by(|a, b| {
        let path_a = a.mounted_full_path.as_deref().unwrap_or(&a.path);
        let path_b = b.mounted_full_path.as_deref().unwrap_or(&b.path);
        path_a
            .cmp(path_b)
            .then_with(|| a.method.cmp(&b.method))
            .then_with(|| a.fn_name.cmp(&b.fn_name))
    });
}

fn build_snapshot(
    reg: &EndpointRegistration,
    mounted: Option<&MountedRouteEntry>,
) -> EndpointSnapshot {
    let (mounted_prefix, mounted_full_path, mounted_app) = match mounted {
        Some(entry) => {
            let prefix = if entry.mount_prefix.is_empty() {
                None
            } else {
                Some(entry.mount_prefix.clone())
            };
            let app = infer_app_from_segments(&entry.mount_segments);
            (prefix, Some(entry.full_path.clone()), app)
        }
        None => (None, None, None),
    };

    EndpointSnapshot {
        fn_name: reg.fn_name.to_string(),
        module_path: reg.module_path.to_string(),
        method: reg.method.to_ascii_uppercase(),
        path: reg.path.to_string(),
        prefix: reg.prefix.map(str::to_string),
        tag: reg.tag.map(str::to_string),
        operation_name: reg.operation_name.map(str::to_string),
        summary: reg.summary.map(str::to_string),
        status: reg.status,
        handler_type_name: (reg.handler_type_name)().to_string(),
        mounted_prefix,
        mounted_full_path,
        mounted_app,
        request_schema: reg.request_schema.map(|f| f()),
        query_schema: reg.query_schema.map(|f| f()),
        response_schema: reg.response_schema.map(|f| f()),
    }
}
