//! OpenAPI 3.1 schema generation from model metadata.

use std::collections::{BTreeMap, HashMap};

use djangors_core::{EndpointSchema, EndpointSnapshot};
use djangors_orm::meta::{FieldKind, Model, ModelSnapshot, RelationKind};

/// Generates an OpenAPI 3.1 JSON Schema component schema from a serializable [`ModelSnapshot`].
pub fn openapi_schema_for_snapshot(snapshot: &ModelSnapshot) -> serde_json::Value {
    let mut props = serde_json::Map::new();
    let mut required = Vec::new();

    for field in &snapshot.fields {
        let mut field_schema = match field.kind {
            FieldKind::Char => {
                let mut s = serde_json::json!({ "type": "string" });
                if let Some(max_len) = field.max_length {
                    s["maxLength"] = serde_json::json!(max_len);
                }
                s
            }
            FieldKind::Text | FieldKind::Slug | FieldKind::FileField => {
                serde_json::json!({ "type": "string" })
            }
            FieldKind::Email => serde_json::json!({ "type": "string", "format": "email" }),
            FieldKind::Url => serde_json::json!({ "type": "string", "format": "uri" }),
            FieldKind::Ip => serde_json::json!({ "type": "string", "format": "ipv4" }),
            FieldKind::Binary => serde_json::json!({ "type": "string", "format": "binary" }),
            FieldKind::Uuid => serde_json::json!({ "type": "string", "format": "uuid" }),
            FieldKind::Date => serde_json::json!({ "type": "string", "format": "date" }),
            FieldKind::DateTime => serde_json::json!({ "type": "string", "format": "date-time" }),
            FieldKind::Time => serde_json::json!({ "type": "string", "format": "time" }),
            FieldKind::Duration => serde_json::json!({ "type": "string" }),
            FieldKind::Json => serde_json::json!({ "type": "object" }),
            FieldKind::Vector { dimensions } => serde_json::json!({
                "type": "array",
                "items": { "type": "number", "format": "float" },
                "minItems": dimensions,
                "maxItems": dimensions,
            }),
            FieldKind::Integer => serde_json::json!({ "type": "integer" }),
            FieldKind::BigInt => serde_json::json!({ "type": "integer", "format": "int64" }),
            FieldKind::Float => serde_json::json!({ "type": "number", "format": "float" }),
            FieldKind::Decimal {
                max_digits,
                decimal_places,
            } => serde_json::json!({
                "type": "string",
                "description": format!("Decimal number with max {} digits and {} decimal places", max_digits, decimal_places)
            }),
            FieldKind::Boolean => serde_json::json!({ "type": "boolean" }),
        };

        if field.nullable {
            field_schema["nullable"] = serde_json::json!(true);
        } else if !field.auto {
            required.push(field.name.clone());
        }

        props.insert(field.name.clone(), field_schema);
    }

    for rel in &snapshot.relations {
        if rel.kind == RelationKind::ManyToMany {
            continue;
        }
        let rel_schema = serde_json::json!({
            "type": "integer",
            "format": "int64",
            "description": format!("Foreign key ID for relation {}", rel.field_name)
        });
        props.insert(rel.field_name.clone(), rel_schema);
    }

    let mut schema = serde_json::json!({
        "type": "object",
        "title": snapshot.struct_name,
        "properties": props
    });

    if !required.is_empty() {
        schema["required"] = serde_json::json!(required);
    }

    schema
}

/// Generates an OpenAPI 3.1 JSON Schema component schema for model `M`.
pub fn openapi_schema_for<M: Model>() -> serde_json::Value {
    let meta = M::meta();
    let mut props = serde_json::Map::new();
    let mut required = Vec::new();

    for field in meta.fields {
        let mut field_schema = match field.kind {
            FieldKind::Char => {
                let mut s = serde_json::json!({ "type": "string" });
                if let Some(max_len) = field.max_length {
                    s["maxLength"] = serde_json::json!(max_len);
                }
                s
            }
            FieldKind::Text | FieldKind::Slug | FieldKind::FileField => {
                serde_json::json!({ "type": "string" })
            }
            FieldKind::Email => serde_json::json!({ "type": "string", "format": "email" }),
            FieldKind::Url => serde_json::json!({ "type": "string", "format": "uri" }),
            FieldKind::Ip => serde_json::json!({ "type": "string", "format": "ipv4" }),
            FieldKind::Binary => serde_json::json!({ "type": "string", "format": "binary" }),
            FieldKind::Uuid => serde_json::json!({ "type": "string", "format": "uuid" }),
            FieldKind::Date => serde_json::json!({ "type": "string", "format": "date" }),
            FieldKind::DateTime => serde_json::json!({ "type": "string", "format": "date-time" }),
            FieldKind::Time => serde_json::json!({ "type": "string", "format": "time" }),
            FieldKind::Duration => serde_json::json!({ "type": "string" }),
            FieldKind::Json => serde_json::json!({ "type": "object" }),
            FieldKind::Vector { dimensions } => serde_json::json!({
                "type": "array",
                "items": { "type": "number", "format": "float" },
                "minItems": dimensions,
                "maxItems": dimensions,
            }),
            FieldKind::Integer => serde_json::json!({ "type": "integer" }),
            FieldKind::BigInt => serde_json::json!({ "type": "integer", "format": "int64" }),
            FieldKind::Float => serde_json::json!({ "type": "number", "format": "float" }),
            FieldKind::Decimal {
                max_digits,
                decimal_places,
            } => {
                // Fixed-precision decimal fields serialize as JSON strings (matching Stripe/financial API conventions)
                // because fixed-precision decimals cannot be safely represented as IEEE-754 JSON numbers without precision loss.
                serde_json::json!({
                    "type": "string",
                    "description": format!("Decimal number with max {} digits and {} decimal places", max_digits, decimal_places)
                })
            }
            FieldKind::Boolean => serde_json::json!({ "type": "boolean" }),
        };

        if field.nullable {
            field_schema["nullable"] = serde_json::json!(true);
        } else if !field.auto {
            required.push(field.name);
        }

        if let Some(verbose) = field.verbose_name {
            if field_schema.get("description").is_none() {
                field_schema["description"] = serde_json::json!(verbose);
            }
        } else if let Some(help) = field.help_text {
            if field_schema.get("description").is_none() {
                field_schema["description"] = serde_json::json!(help);
            }
        }

        props.insert(field.name.to_string(), field_schema);
    }

    for rel in meta.relations {
        let rel_schema = serde_json::json!({
            "type": "integer",
            "format": "int64",
            "description": format!("Foreign key ID for relation {}", rel.field_name)
        });
        props.insert(rel.field_name.to_string(), rel_schema);
    }

    let mut schema = serde_json::json!({
        "type": "object",
        "title": meta.struct_name,
        "properties": props
    });

    if !required.is_empty() {
        schema["required"] = serde_json::json!(required);
    }

    schema
}

/// One standard CRUD operation a model's routes may expose.
///
/// Used with [`OpenApiBuilder::register_snapshot_with_ops`] so a spec (and the
/// TypeScript client generated from it) lists only operations the API really serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CrudOp {
    /// `GET <base>` — paginated list.
    List,
    /// `POST <base>`.
    Create,
    /// `GET <base>/{pk}`.
    Retrieve,
    /// `PUT <base>/{pk}`.
    Update,
    /// `PATCH <base>/{pk}`.
    PartialUpdate,
    /// `DELETE <base>/{pk}`.
    Destroy,
}

impl CrudOp {
    /// Every operation, in the order `register` emits them.
    pub const ALL: [CrudOp; 6] = [
        CrudOp::List,
        CrudOp::Create,
        CrudOp::Retrieve,
        CrudOp::Update,
        CrudOp::PartialUpdate,
        CrudOp::Destroy,
    ];

    /// Parses `list`, `create`, `retrieve`, `update`, `partial_update`
    /// (or `partialUpdate`) and `destroy`.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "list" => Some(CrudOp::List),
            "create" => Some(CrudOp::Create),
            "retrieve" => Some(CrudOp::Retrieve),
            "update" => Some(CrudOp::Update),
            "partial_update" | "partialUpdate" => Some(CrudOp::PartialUpdate),
            "destroy" => Some(CrudOp::Destroy),
            _ => None,
        }
    }
}

/// Builder for constructing OpenAPI 3.1 specifications from registered Djangors models.
#[derive(Debug, Clone, Default)]
pub struct OpenApiBuilder {
    title: String,
    version: String,
    schemas: HashMap<String, serde_json::Value>,
    paths: serde_json::Map<String, serde_json::Value>,
    security_schemes: serde_json::Map<String, serde_json::Value>,
}

impl OpenApiBuilder {
    /// Creates a new `OpenApiBuilder` with specified title and API version.
    pub fn new(title: &str, version: &str) -> Self {
        Self {
            title: title.to_string(),
            version: version.to_string(),
            schemas: HashMap::new(),
            paths: serde_json::Map::new(),
            security_schemes: serde_json::Map::new(),
        }
    }

    /// Overrides the `info.version` field in the generated OpenAPI specification.
    pub fn version(&mut self, v: &str) -> &mut Self {
        self.version = v.to_string();
        self
    }

    /// Registers an `apiKey` security scheme in `components.securitySchemes`
    /// reading from the `X-Api-Key` header.
    pub fn api_key_security(&mut self) -> &mut Self {
        self.security_schemes.insert(
            "ApiKeyAuth".to_string(),
            serde_json::json!({
                "type": "apiKey",
                "in": "header",
                "name": "X-Api-Key"
            }),
        );
        self
    }

    /// Registers model `M` mounted at `base_path` into the OpenAPI specification,
    /// generating its component schema and standard CRUD path operations.
    pub fn register<M: Model>(&mut self, base_path: &str) -> &mut Self {
        let meta = M::meta();
        let schema_name = meta.struct_name;

        if !self.schemas.contains_key(schema_name) {
            self.schemas
                .insert(schema_name.to_string(), openapi_schema_for::<M>());
        }
        self.register_crud_paths(schema_name, base_path, &CrudOp::ALL);
        self
    }

    /// Registers a serializable [`ModelSnapshot`] mounted at `base_path` into the
    /// OpenAPI specification. Used by `dj openapi` when introspecting a project binary.
    pub fn register_snapshot(&mut self, snapshot: &ModelSnapshot, base_path: &str) -> &mut Self {
        let schema_name = &snapshot.struct_name;
        if !self.schemas.contains_key(schema_name) {
            self.schemas
                .insert(schema_name.clone(), openapi_schema_for_snapshot(snapshot));
        }
        self.register_crud_paths(schema_name, base_path, &CrudOp::ALL);
        self
    }

    /// Like [`register_snapshot`](Self::register_snapshot), but registers only `ops`.
    /// An empty `ops` registers the component schema alone (types, no paths).
    pub fn register_snapshot_with_ops(
        &mut self,
        snapshot: &ModelSnapshot,
        base_path: &str,
        ops: &[CrudOp],
    ) -> &mut Self {
        let schema_name = &snapshot.struct_name;
        if !self.schemas.contains_key(schema_name) {
            self.schemas
                .insert(schema_name.clone(), openapi_schema_for_snapshot(snapshot));
        }
        if !ops.is_empty() {
            self.register_crud_paths(schema_name, base_path, ops);
        }
        self
    }

    /// Registers a typed [`EndpointSnapshot`] (produced by `#[endpoint]` and router mount
    /// introspection) into the OpenAPI specification, adding its request/query/response
    /// component schemas and path operation.
    pub fn register_endpoint(&mut self, endpoint: &EndpointSnapshot) -> &mut Self {
        let full_path = resolve_endpoint_path(endpoint);
        let method_key = endpoint.method.to_ascii_lowercase();
        let op_id = endpoint
            .operation_name
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(&endpoint.fn_name)
            .to_string();

        let mut parameters: Vec<serde_json::Value> = extract_path_params(&full_path)
            .into_iter()
            .map(|name| {
                serde_json::json!({
                    "name": name,
                    "in": "path",
                    "required": true,
                    "schema": { "type": "string" }
                })
            })
            .collect();

        if let Some(query_schema) = &endpoint.query_schema {
            for (def_name, def_val) in &query_schema.definitions {
                self.schemas
                    .entry(def_name.clone())
                    .or_insert_with(|| rewrite_schema_refs(def_val));
            }
            let rewritten = rewrite_schema_refs(&query_schema.schema);
            let resolved = rewritten
                .get("$ref")
                .and_then(|r| r.as_str())
                .and_then(|r| r.strip_prefix("#/components/schemas/"))
                .and_then(|name| self.schemas.get(name))
                .unwrap_or(&rewritten);
            let required_set: Vec<&str> = resolved
                .get("required")
                .and_then(|r| r.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
                .unwrap_or_default();
            if let Some(props) = resolved.get("properties").and_then(|p| p.as_object()) {
                let mut sorted: BTreeMap<&String, &serde_json::Value> = BTreeMap::new();
                for (k, v) in props {
                    sorted.insert(k, v);
                }
                for (name, prop_schema) in sorted {
                    parameters.push(serde_json::json!({
                        "name": name,
                        "in": "query",
                        "required": required_set.contains(&name.as_str()),
                        "schema": prop_schema,
                    }));
                }
            }
        }

        let mut op = serde_json::Map::new();
        op.insert("operationId".to_string(), serde_json::json!(op_id));
        if let Some(summary) = &endpoint.summary {
            op.insert("summary".to_string(), serde_json::json!(summary));
        }
        if let Some(tag) = endpoint.tag.as_deref().or(endpoint.mounted_app.as_deref()) {
            op.insert("tags".to_string(), serde_json::json!([tag]));
        }
        if !parameters.is_empty() {
            op.insert(
                "parameters".to_string(),
                serde_json::Value::Array(parameters),
            );
        }

        if let Some(req_schema) = &endpoint.request_schema {
            let schema_val = self.register_endpoint_schema(req_schema);
            op.insert(
                "requestBody".to_string(),
                serde_json::json!({
                    "required": true,
                    "content": {
                        "application/json": {
                            "schema": schema_val
                        }
                    }
                }),
            );
        }

        let status_key = endpoint.status.to_string();
        let response_entry = if endpoint.status == 204 {
            serde_json::json!({ "description": "No content" })
        } else if let Some(resp_schema) = &endpoint.response_schema {
            let schema_val = self.register_endpoint_schema(resp_schema);
            serde_json::json!({
                "description": "Success",
                "content": {
                    "application/json": {
                        "schema": schema_val
                    }
                }
            })
        } else {
            serde_json::json!({ "description": "Success" })
        };
        let mut responses = serde_json::Map::new();
        responses.insert(status_key, response_entry);
        op.insert(
            "responses".to_string(),
            serde_json::Value::Object(responses),
        );

        let mut path_item = self
            .paths
            .get(&full_path)
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        path_item[method_key] = serde_json::Value::Object(op);
        self.paths.insert(full_path, path_item);
        self
    }

    /// Registers a slice of [`EndpointSnapshot`]s into the OpenAPI specification.
    pub fn register_endpoints(&mut self, endpoints: &[EndpointSnapshot]) -> &mut Self {
        for ep in endpoints {
            self.register_endpoint(ep);
        }
        self
    }

    fn register_endpoint_schema(&mut self, schema: &EndpointSchema) -> serde_json::Value {
        for (def_name, def_val) in &schema.definitions {
            self.schemas
                .entry(def_name.clone())
                .or_insert_with(|| rewrite_schema_refs(def_val));
        }
        let rewritten = rewrite_schema_refs(&schema.schema);
        if let Some(name) = &schema.name {
            let is_named_composite = rewritten.get("properties").is_some()
                || rewritten.get("enum").is_some()
                || rewritten.get("oneOf").is_some()
                || rewritten.get("anyOf").is_some()
                || rewritten.get("allOf").is_some();
            if is_named_composite {
                self.schemas.entry(name.clone()).or_insert(rewritten);
                return serde_json::json!({ "$ref": format!("#/components/schemas/{name}") });
            }
        }
        rewritten
    }

    fn register_crud_paths(&mut self, schema_name: &str, base_path: &str, ops: &[CrudOp]) {
        let clean_base = base_path.trim_end_matches('/');
        let list_create_path = if clean_base.is_empty() {
            "/".to_string()
        } else {
            clean_base.to_string()
        };
        let detail_path = if list_create_path == "/" {
            "/{pk}".to_string()
        } else {
            format!("{list_create_path}/{{pk}}")
        };

        let schema_ref = format!("#/components/schemas/{schema_name}");

        let list_op = serde_json::json!({
            "summary": format!("List {} items", schema_name),
            "operationId": format!("list_{}", schema_name.to_lowercase()),
            "responses": {
                "200": {
                    "description": "Paginated list of items",
                    "content": {
                        "application/json": {
                            "schema": {
                                "type": "object",
                                "properties": {
                                    "count": { "type": "integer" },
                                    "page": { "type": "integer" },
                                    "total_pages": { "type": "integer" },
                                    "results": {
                                        "type": "array",
                                        "items": { "$ref": schema_ref }
                                    }
                                },
                                "required": ["count", "results"]
                            }
                        }
                    }
                }
            }
        });

        let create_op = serde_json::json!({
            "summary": format!("Create a {}", schema_name),
            "operationId": format!("create_{}", schema_name.to_lowercase()),
            "requestBody": {
                "required": true,
                "content": {
                    "application/json": {
                        "schema": { "$ref": schema_ref }
                    }
                }
            },
            "responses": {
                "201": {
                    "description": "Created item",
                    "content": {
                        "application/json": {
                            "schema": { "$ref": schema_ref }
                        }
                    }
                },
                "422": { "description": "Validation error" }
            }
        });

        let mut list_create_item = self
            .paths
            .get(&list_create_path)
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        if ops.contains(&CrudOp::List) {
            list_create_item["get"] = list_op;
        }
        if ops.contains(&CrudOp::Create) {
            list_create_item["post"] = create_op;
        }
        if ops.contains(&CrudOp::List) || ops.contains(&CrudOp::Create) {
            self.paths.insert(list_create_path, list_create_item);
        }

        let pk_param = serde_json::json!([{
            "name": "pk",
            "in": "path",
            "required": true,
            "schema": {
                "type": "integer",
                "format": "int64"
            },
            "description": "Primary key"
        }]);

        let retrieve_op = serde_json::json!({
            "summary": format!("Retrieve a {}", schema_name),
            "operationId": format!("retrieve_{}", schema_name.to_lowercase()),
            "parameters": pk_param,
            "responses": {
                "200": {
                    "description": "Item details",
                    "content": {
                        "application/json": {
                            "schema": { "$ref": schema_ref }
                        }
                    }
                },
                "404": { "description": "Item not found" }
            }
        });

        let update_put_op = serde_json::json!({
            "summary": format!("Update a {}", schema_name),
            "operationId": format!("update_{}", schema_name.to_lowercase()),
            "parameters": pk_param,
            "requestBody": {
                "required": true,
                "content": {
                    "application/json": {
                        "schema": { "$ref": schema_ref }
                    }
                }
            },
            "responses": {
                "200": {
                    "description": "Updated item",
                    "content": {
                        "application/json": {
                            "schema": { "$ref": schema_ref }
                        }
                    }
                },
                "404": { "description": "Item not found" },
                "422": { "description": "Validation error" }
            }
        });

        let update_patch_op = serde_json::json!({
            "summary": format!("Partial update a {}", schema_name),
            "operationId": format!("partial_update_{}", schema_name.to_lowercase()),
            "parameters": pk_param,
            "requestBody": {
                "required": true,
                "content": {
                    "application/json": {
                        "schema": { "$ref": schema_ref }
                    }
                }
            },
            "responses": {
                "200": {
                    "description": "Updated item",
                    "content": {
                        "application/json": {
                            "schema": { "$ref": schema_ref }
                        }
                    }
                },
                "404": { "description": "Item not found" },
                "422": { "description": "Validation error" }
            }
        });

        let destroy_op = serde_json::json!({
            "summary": format!("Delete a {}", schema_name),
            "operationId": format!("destroy_{}", schema_name.to_lowercase()),
            "parameters": pk_param,
            "responses": {
                "204": { "description": "No content" },
                "404": { "description": "Item not found" }
            }
        });

        let mut detail_item = self
            .paths
            .get(&detail_path)
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        let mut any_detail = false;
        for (op, method, value) in [
            (CrudOp::Retrieve, "get", retrieve_op),
            (CrudOp::Update, "put", update_put_op),
            (CrudOp::PartialUpdate, "patch", update_patch_op),
            (CrudOp::Destroy, "delete", destroy_op),
        ] {
            if ops.contains(&op) {
                detail_item[method] = value;
                any_detail = true;
            }
        }
        if any_detail {
            self.paths.insert(detail_path, detail_item);
        }
    }

    /// Builds and returns the complete OpenAPI 3.1 specification JSON Value.
    pub fn build(&self) -> serde_json::Value {
        let mut components = serde_json::json!({
            "schemas": self.schemas
        });
        if !self.security_schemes.is_empty() {
            components["securitySchemes"] =
                serde_json::Value::Object(self.security_schemes.clone());
        }
        serde_json::json!({
            "openapi": "3.1.0",
            "info": {
                "title": self.title,
                "version": self.version
            },
            "paths": self.paths,
            "components": components
        })
    }

    /// Generates a zero-dependency TypeScript client (`types + fetch client + ProblemDetailsError`)
    /// from the registered schemas and paths.
    pub fn to_typescript(&self) -> String {
        generate_typescript_client(&self.build())
    }
}

/// Generates a complete TypeScript definitions and typed `fetch` client module
/// from an OpenAPI 3.1 specification produced by [`OpenApiBuilder`].
///
/// Includes:
/// - `ProblemDetails` (RFC 9457 `application/problem+json`) and `ProblemDetailsError`
/// - `PaginatedList<T>`
/// - Interfaces (`<Model>`, `<Model>CreateInput`, `<Model>UpdateInput`) for every component schema
/// - `DjangorsApiClient` with typed `list<Model>`, `retrieve<Model>`, `create<Model>`,
///   `update<Model>`, `partialUpdate<Model>`, and `destroy<Model>` methods.
pub fn generate_typescript_client(spec: &serde_json::Value) -> String {
    let mut out = String::with_capacity(4096);
    out.push_str(
        "// Auto-generated by Djangors (`dj openapi --typescript`). Do not edit manually.\n\n",
    );
    out.push_str(
        "export interface ProblemDetails {\n\
         \x20 type: string;\n\
         \x20 title: string;\n\
         \x20 status: number;\n\
         \x20 code: string;\n\
         \x20 detail: string;\n\
         \x20 instance?: string;\n\
         \x20 errors?: Record<string, string[]>;\n\
         \x20 details?: unknown;\n\
         }\n\n\
         export class ProblemDetailsError extends Error {\n\
         \x20 readonly problem: ProblemDetails;\n\
         \x20 constructor(problem: ProblemDetails) {\n\
         \x20   super(problem.detail || problem.title);\n\
         \x20   this.name = \"ProblemDetailsError\";\n\
         \x20   this.problem = problem;\n\
         \x20 }\n\
         }\n\n\
         /** List envelope. `count` and `results` are always present; the other fields\n\
         \x20* depend on the pagination style (page number, limit/offset, or cursor). */\n\
         export interface PaginatedList<T> {\n\
         \x20 count: number;\n\
         \x20 results: T[];\n\
         \x20 page?: number;\n\
         \x20 total_pages?: number;\n\
         \x20 page_size?: number;\n\
         \x20 limit?: number;\n\
         \x20 offset?: number;\n\
         \x20 next_cursor?: string | null;\n\
         \x20 previous_cursor?: string | null;\n\
         }\n\n",
    );

    let mut sorted_schemas: BTreeMap<String, &serde_json::Value> = BTreeMap::new();
    if let Some(schemas) = spec
        .get("components")
        .and_then(|c| c.get("schemas"))
        .and_then(|s| s.as_object())
    {
        for (name, schema) in schemas {
            sorted_schemas.insert(name.clone(), schema);
        }
    }

    for (name, schema) in &sorted_schemas {
        let is_non_object_type = schema.get("properties").is_none()
            && (schema.get("enum").is_some()
                || schema.get("oneOf").is_some()
                || schema.get("anyOf").is_some()
                || schema.get("allOf").is_some()
                || schema.get("const").is_some());
        if is_non_object_type {
            let ts_ty = json_schema_to_ts_type(schema);
            out.push_str(&format!("export type {name} = {ts_ty};\n\n"));
            continue;
        }

        let props = schema
            .get("properties")
            .and_then(|p| p.as_object())
            .cloned()
            .unwrap_or_default();
        let required: Vec<&str> = schema
            .get("required")
            .and_then(|r| r.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();

        let mut sorted_props: BTreeMap<String, &serde_json::Value> = BTreeMap::new();
        for (k, v) in &props {
            sorted_props.insert(k.clone(), v);
        }

        // Full read interface
        out.push_str(&format!("export interface {name} {{\n"));
        for (field, fschema) in &sorted_props {
            let ts_ty = json_schema_to_ts_type(fschema);
            out.push_str(&format!("  {field}: {ts_ty};\n"));
        }
        out.push_str("}\n\n");

        // Create input (auto fields become optional)
        out.push_str(&format!("export interface {name}CreateInput {{\n"));
        for (field, fschema) in &sorted_props {
            let ts_ty = json_schema_to_ts_type(fschema);
            let is_req = required.contains(&field.as_str());
            let opt = if is_req { "" } else { "?" };
            out.push_str(&format!("  {field}{opt}: {ts_ty};\n"));
        }
        out.push_str("}\n\n");

        // Update / PATCH input
        out.push_str(&format!(
            "export type {name}UpdateInput = Partial<{name}CreateInput>;\n\n"
        ));
    }

    // Collect each model's base path and the CRUD operations the spec actually
    // declares for it, keyed off the `<op>_<model>` operationIds, plus any
    // custom (non-CRUD) typed endpoint operations.
    let mut model_paths: BTreeMap<String, (String, Vec<&'static str>)> = BTreeMap::new();
    let mut custom_ops: Vec<(String, String, &serde_json::Value)> = Vec::new();
    if let Some(paths) = spec.get("paths").and_then(|p| p.as_object()) {
        let mut sorted_paths: BTreeMap<&String, &serde_json::Value> = BTreeMap::new();
        for (k, v) in paths {
            sorted_paths.insert(k, v);
        }
        for (path, item) in sorted_paths {
            let base = path.strip_suffix("/{pk}").unwrap_or(path).to_string();
            let Some(item) = item.as_object() else {
                continue;
            };
            let mut sorted_methods: BTreeMap<&String, &serde_json::Value> = BTreeMap::new();
            for (m, op) in item {
                sorted_methods.insert(m, op);
            }
            for (method, op) in sorted_methods {
                let Some(op_id) = op.get("operationId").and_then(|id| id.as_str()) else {
                    continue;
                };
                let found = [
                    "partial_update_",
                    "update_",
                    "list_",
                    "create_",
                    "retrieve_",
                    "destroy_",
                ]
                .into_iter()
                .find_map(|prefix| op_id.strip_prefix(prefix).map(|rest| (prefix, rest)));
                if let Some((prefix, lower_model)) = found {
                    if let Some(schema_name) = sorted_schemas
                        .keys()
                        .find(|k| k.to_lowercase() == lower_model)
                    {
                        let entry = model_paths
                            .entry(schema_name.clone())
                            .or_insert_with(|| (base.clone(), Vec::new()));
                        let kind: &'static str = match prefix {
                            "partial_update_" => "partial_update",
                            "update_" => "update",
                            "list_" => "list",
                            "create_" => "create",
                            "retrieve_" => "retrieve",
                            _ => "destroy",
                        };
                        if !entry.1.contains(&kind) {
                            entry.1.push(kind);
                        }
                        continue;
                    }
                }
                custom_ops.push((path.clone(), method.to_ascii_uppercase(), op));
            }
        }
    }

    out.push_str(
        "export interface ClientOptions {\n\
         \x20 baseUrl?: string;\n\
         \x20 headers?: Record<string, string>;\n\
         \x20 fetch?: typeof fetch;\n\
         }\n\n\
         export class DjangorsApiClient {\n\
         \x20 private readonly baseUrl: string;\n\
         \x20 private readonly defaultHeaders: Record<string, string>;\n\
         \x20 private readonly fetchImpl: typeof fetch;\n\n\
         \x20 constructor(options: ClientOptions = {}) {\n\
         \x20   this.baseUrl = (options.baseUrl ?? \"\").replace(/\\/$/, \"\");\n\
         \x20   this.defaultHeaders = options.headers ?? {};\n\
         \x20   this.fetchImpl = options.fetch ?? fetch;\n\
         \x20 }\n\n\
         \x20 private async request<T>(path: string, init: RequestInit = {}): Promise<T> {\n\
         \x20   const res = await this.fetchImpl(`${this.baseUrl}${path}`, {\n\
         \x20     ...init,\n\
         \x20     headers: {\n\
         \x20       Accept: \"application/json, application/problem+json\",\n\
         \x20       \"Content-Type\": \"application/json\",\n\
         \x20       ...this.defaultHeaders,\n\
         \x20       ...(init.headers as Record<string, string> | undefined),\n\
         \x20     },\n\
         \x20   });\n\
         \x20   if (!res.ok) {\n\
         \x20     let body: any = null;\n\
         \x20     try {\n\
         \x20       body = await res.json();\n\
         \x20     } catch {\n\
         \x20       // fallback when response body is not JSON\n\
         \x20     }\n\
         \x20     if (body && typeof body.type === \"string\" && typeof body.status === \"number\") {\n\
         \x20       throw new ProblemDetailsError(body as ProblemDetails);\n\
         \x20     }\n\
         \x20     if (body && body.error) {\n\
         \x20       throw new ProblemDetailsError({\n\
         \x20         type: `urn:djangors:error:${body.error.code ?? \"error\"}`,\n\
         \x20         title: res.statusText || \"Error\",\n\
         \x20         status: body.error.status ?? res.status,\n\
         \x20         code: body.error.code ?? \"error\",\n\
         \x20         detail: body.error.message ?? res.statusText,\n\
         \x20         errors: body.error.details,\n\
         \x20         details: body.error.details,\n\
         \x20       });\n\
         \x20     }\n\
         \x20     throw new ProblemDetailsError({\n\
         \x20       type: \"urn:djangors:error:http_error\",\n\
         \x20       title: res.statusText || \"HTTP Error\",\n\
         \x20       status: res.status,\n\
         \x20       code: \"http_error\",\n\
         \x20       detail: res.statusText || `HTTP ${res.status}`,\n\
         \x20     });\n\
         \x20   }\n\
         \x20   if (res.status === 204) {\n\
         \x20     return undefined as T;\n\
         \x20   }\n\
         \x20   return (await res.json()) as T;\n\
         \x20 }\n",
    );

    for (model, (base_path, ops)) in &model_paths {
        let clean = base_path.trim_end_matches('/');
        let has = |name: &str| ops.contains(&name);
        if has("list") {
            out.push_str(&format!(
                "\n  async list{model}(params?: Record<string, string | number | boolean>): Promise<PaginatedList<{model}>> {{\n\
                 \x20   const qs = params ? \"?\" + new URLSearchParams(Object.entries(params).map(([k, v]) => [k, String(v)])).toString() : \"\";\n\
                 \x20   return this.request<PaginatedList<{model}>>(`{clean}${{qs}}`, {{ method: \"GET\" }});\n\
                 \x20 }}\n"
            ));
        }
        if has("retrieve") {
            out.push_str(&format!(
                "\n  async retrieve{model}(pk: number | string): Promise<{model}> {{\n\
                 \x20   return this.request<{model}>(`{clean}/${{encodeURIComponent(String(pk))}}`, {{ method: \"GET\" }});\n\
                 \x20 }}\n"
            ));
        }
        if has("create") {
            out.push_str(&format!(
                "\n  async create{model}(data: {model}CreateInput): Promise<{model}> {{\n\
                 \x20   return this.request<{model}>(\"{clean}\", {{ method: \"POST\", body: JSON.stringify(data) }});\n\
                 \x20 }}\n"
            ));
        }
        if has("update") {
            out.push_str(&format!(
                "\n  async update{model}(pk: number | string, data: {model}CreateInput): Promise<{model}> {{\n\
                 \x20   return this.request<{model}>(`{clean}/${{encodeURIComponent(String(pk))}}`, {{ method: \"PUT\", body: JSON.stringify(data) }});\n\
                 \x20 }}\n"
            ));
        }
        if has("partial_update") {
            out.push_str(&format!(
                "\n  async partialUpdate{model}(pk: number | string, data: {model}UpdateInput): Promise<{model}> {{\n\
                 \x20   return this.request<{model}>(`{clean}/${{encodeURIComponent(String(pk))}}`, {{ method: \"PATCH\", body: JSON.stringify(data) }});\n\
                 \x20 }}\n"
            ));
        }
        if has("destroy") {
            out.push_str(&format!(
                "\n  async destroy{model}(pk: number | string): Promise<void> {{\n\
                 \x20   return this.request<void>(`{clean}/${{encodeURIComponent(String(pk))}}`, {{ method: \"DELETE\" }});\n\
                 \x20 }}\n"
            ));
        }
    }

    for (path, method, op) in &custom_ops {
        let Some(op_id) = op.get("operationId").and_then(|v| v.as_str()) else {
            continue;
        };
        let ts_method = to_camel_case(op_id);
        let mut args: Vec<String> = Vec::new();
        let mut query_fields: Vec<(String, String, bool)> = Vec::new();

        if let Some(params) = op.get("parameters").and_then(|p| p.as_array()) {
            for p in params {
                let Some(name) = p.get("name").and_then(|v| v.as_str()) else {
                    continue;
                };
                let location = p.get("in").and_then(|v| v.as_str()).unwrap_or("");
                let required = p.get("required").and_then(|v| v.as_bool()).unwrap_or(false);
                let schema = p.get("schema").unwrap_or(&serde_json::Value::Null);
                let ts_ty = json_schema_to_ts_type(schema);
                match location {
                    "path" => args.push(format!("{name}: string | number")),
                    "query" => query_fields.push((name.to_string(), ts_ty, required)),
                    _ => {}
                }
            }
        }

        let body_schema = op
            .get("requestBody")
            .and_then(|rb| rb.get("content"))
            .and_then(|c| c.get("application/json"))
            .and_then(|j| j.get("schema"));
        if let Some(bs) = body_schema {
            args.push(format!("body: {}", json_schema_to_ts_type(bs)));
        }

        if !query_fields.is_empty() {
            let any_req = query_fields.iter().any(|(_, _, r)| *r);
            let fields_str = query_fields
                .iter()
                .map(|(n, ty, req)| {
                    let opt = if *req { "" } else { "?" };
                    format!("{n}{opt}: {ty}")
                })
                .collect::<Vec<_>>()
                .join("; ");
            let q_opt = if any_req { "" } else { "?" };
            args.push(format!("query{q_opt}: {{ {fields_str} }}"));
        }

        let resp_type = op
            .get("responses")
            .and_then(|r| r.as_object())
            .and_then(|map| {
                map.iter()
                    .find(|(code, _)| code.starts_with('2'))
                    .map(|(code, val)| {
                        if code == "204" {
                            "void".to_string()
                        } else if let Some(s) = val
                            .get("content")
                            .and_then(|c| c.get("application/json"))
                            .and_then(|j| j.get("schema"))
                        {
                            json_schema_to_ts_type(s)
                        } else {
                            "void".to_string()
                        }
                    })
            })
            .unwrap_or_else(|| "void".to_string());

        let mut interpolated_path = path.clone();
        for p in extract_path_params(path) {
            interpolated_path = interpolated_path.replace(
                &format!("{{{p}}}"),
                &format!("${{encodeURIComponent(String({p}))}}"),
            );
        }

        let qs_line = if !query_fields.is_empty() {
            "    const qs = query ? \"?\" + new URLSearchParams(Object.entries(query).filter(([, v]) => v !== undefined).map(([k, v]) => [k, String(v)])).toString() : \"\";\n"
        } else {
            ""
        };
        let url_expr = if !query_fields.is_empty() {
            format!("`{interpolated_path}${{qs}}`")
        } else {
            format!("`{interpolated_path}`")
        };
        let init_expr = if body_schema.is_some() {
            format!("{{ method: \"{method}\", body: JSON.stringify(body) }}")
        } else {
            format!("{{ method: \"{method}\" }}")
        };

        out.push_str(&format!(
            "\n  async {ts_method}({}): Promise<{resp_type}> {{\n\
             {qs_line}\
             \x20   return this.request<{resp_type}>({url_expr}, {init_expr});\n\
             \x20 }}\n",
            args.join(", ")
        ));
    }

    out.push_str("}\n");
    out
}

fn resolve_endpoint_path(endpoint: &EndpointSnapshot) -> String {
    if let Some(full) = &endpoint.mounted_full_path {
        if full.starts_with('/') {
            return full.clone();
        }
        return format!("/{full}");
    }
    let raw = match &endpoint.prefix {
        Some(pref) if !pref.trim_matches('/').is_empty() => {
            let p = pref.trim_matches('/');
            let s = endpoint.path.trim_start_matches('/');
            if s.is_empty() {
                format!("/{p}")
            } else {
                format!("/{p}/{s}")
            }
        }
        _ => {
            if endpoint.path.starts_with('/') {
                endpoint.path.clone()
            } else {
                format!("/{}", endpoint.path)
            }
        }
    };
    raw
}

fn extract_path_params(path: &str) -> Vec<String> {
    let mut params = Vec::new();
    let mut rest = path;
    while let Some(start) = rest.find('{') {
        let after = &rest[start + 1..];
        if let Some(end) = after.find('}') {
            let name = after[..end].trim();
            if !name.is_empty() && !params.iter().any(|p| p == name) {
                params.push(name.to_string());
            }
            rest = &after[end + 1..];
        } else {
            break;
        }
    }
    params
}

fn rewrite_schema_refs(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut out = serde_json::Map::with_capacity(map.len());
            for (k, v) in map {
                if k == "$ref" {
                    if let Some(s) = v.as_str() {
                        let rewritten = s
                            .replace("#/$defs/", "#/components/schemas/")
                            .replace("#/definitions/", "#/components/schemas/");
                        out.insert(k.clone(), serde_json::Value::String(rewritten));
                        continue;
                    }
                }
                if k == "$defs" || k == "definitions" {
                    continue;
                }
                out.insert(k.clone(), rewrite_schema_refs(v));
            }
            serde_json::Value::Object(out)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(rewrite_schema_refs).collect())
        }
        other => other.clone(),
    }
}

fn to_camel_case(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut upper_next = false;
    for ch in input.chars() {
        if ch == '_' || ch == '-' {
            upper_next = true;
        } else if upper_next {
            out.extend(ch.to_uppercase());
            upper_next = false;
        } else {
            out.push(ch);
        }
    }
    out
}

fn json_schema_to_ts_type(schema: &serde_json::Value) -> String {
    if let Some(r) = schema.get("$ref").and_then(|v| v.as_str()) {
        if let Some(name) = r.rsplit('/').next() {
            return name.to_string();
        }
    }
    if let Some(c) = schema.get("const") {
        return serde_json::to_string(c).unwrap_or_else(|_| "unknown".to_string());
    }
    if let Some(variants) = schema.get("enum").and_then(|v| v.as_array()) {
        let parts: Vec<String> = variants
            .iter()
            .filter_map(|v| serde_json::to_string(v).ok())
            .collect();
        if !parts.is_empty() {
            return parts.join(" | ");
        }
    }
    for comb in ["oneOf", "anyOf"] {
        if let Some(arr) = schema.get(comb).and_then(|v| v.as_array()) {
            let parts: Vec<String> = arr.iter().map(json_schema_to_ts_type).collect();
            if !parts.is_empty() {
                return parts.join(" | ");
            }
        }
    }
    if let Some(arr) = schema.get("allOf").and_then(|v| v.as_array()) {
        let parts: Vec<String> = arr.iter().map(json_schema_to_ts_type).collect();
        if !parts.is_empty() {
            return parts.join(" & ");
        }
    }
    if let Some(types) = schema.get("type").and_then(|t| t.as_array()) {
        let parts: Vec<String> = types
            .iter()
            .filter_map(|t| t.as_str())
            .map(|t| match t {
                "integer" | "number" => "number".to_string(),
                "boolean" => "boolean".to_string(),
                "string" => "string".to_string(),
                "null" => "null".to_string(),
                "object" => "Record<string, unknown>".to_string(),
                "array" => {
                    if let Some(items) = schema.get("items") {
                        format!("{}[]", json_schema_to_ts_type(items))
                    } else {
                        "unknown[]".to_string()
                    }
                }
                _ => "unknown".to_string(),
            })
            .collect();
        if !parts.is_empty() {
            return parts.join(" | ");
        }
    }
    let base = match schema.get("type").and_then(|t| t.as_str()) {
        Some("integer") | Some("number") => "number".to_string(),
        Some("boolean") => "boolean".to_string(),
        Some("string") => "string".to_string(),
        Some("null") => "null".to_string(),
        Some("array") => {
            if let Some(items) = schema.get("items") {
                let inner = json_schema_to_ts_type(items);
                if inner.contains(" | ") || inner.contains(" & ") {
                    format!("({inner})[]")
                } else {
                    format!("{inner}[]")
                }
            } else {
                "unknown[]".to_string()
            }
        }
        Some("object") => {
            if let Some(props) = schema.get("properties").and_then(|p| p.as_object()) {
                let req: Vec<&str> = schema
                    .get("required")
                    .and_then(|r| r.as_array())
                    .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
                    .unwrap_or_default();
                let mut sorted: BTreeMap<&String, &serde_json::Value> = BTreeMap::new();
                for (k, v) in props {
                    sorted.insert(k, v);
                }
                let fields = sorted
                    .into_iter()
                    .map(|(k, v)| {
                        let opt = if req.contains(&k.as_str()) { "" } else { "?" };
                        format!("{k}{opt}: {}", json_schema_to_ts_type(v))
                    })
                    .collect::<Vec<_>>()
                    .join("; ");
                format!("{{ {fields} }}")
            } else if let Some(add) = schema.get("additionalProperties") {
                if add.is_object() {
                    format!("Record<string, {}>", json_schema_to_ts_type(add))
                } else {
                    "Record<string, unknown>".to_string()
                }
            } else {
                "Record<string, unknown>".to_string()
            }
        }
        _ => "unknown".to_string(),
    };
    if schema
        .get("nullable")
        .and_then(|n| n.as_bool())
        .unwrap_or(false)
    {
        format!("{base} | null")
    } else {
        base
    }
}

#[cfg(test)]
#[allow(dead_code)]
mod endpoint_openapi_tests {
    use super::*;
    use djangors_core::{
        mounted_endpoints_for_router, DjangorsError, PathParams, Request, Response, Router,
        StatusCode,
    };
    use djangors_macros::{endpoint, JsonSchema};
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize, JsonSchema)]
    #[serde(rename_all = "camelCase")]
    struct ReceiptOut {
        receipt_id: i64,
        status: PaymentStatus,
        note: Option<String>,
        items: Vec<String>,
    }

    #[derive(Serialize, Deserialize, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    enum PaymentStatus {
        Paid,
        Refunded,
    }

    #[derive(Deserialize, JsonSchema)]
    struct RefundInput {
        reason: String,
        amount_cents: i64,
    }

    #[derive(Deserialize, JsonSchema)]
    struct ReceiptQuery {
        include_lines: Option<bool>,
    }

    /// Fetch a payment receipt.
    #[endpoint(GET, "/payments/{id}/receipt", query = ReceiptQuery, response = ReceiptOut, tag = "billing")]
    async fn get_receipt(_req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
        Ok(Response::text(StatusCode::OK, "ok"))
    }

    #[endpoint(POST, "/payments/{id}/refund", request = RefundInput, response = ReceiptOut, status = 201, name = "refund_payment")]
    async fn refund_handler(_req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
        Ok(Response::text(StatusCode::OK, "ok"))
    }

    #[test]
    fn typed_endpoints_register_into_openapi_and_typescript() {
        let billing = Router::new().endpoint(get_receipt).endpoint(refund_handler);
        let root = Router::new().mount("/api/v1/billing", billing);

        let snapshots = mounted_endpoints_for_router(&root);
        assert_eq!(snapshots.len(), 2);

        let mut builder = OpenApiBuilder::new("Billing API", "1.0.0");
        builder.register_endpoints(&snapshots);
        let spec = builder.build();

        let receipt_op = &spec["paths"]["/api/v1/billing/payments/{id}/receipt"]["get"];
        assert_eq!(receipt_op["operationId"], "get_receipt");
        assert_eq!(receipt_op["summary"], "Fetch a payment receipt.");
        assert_eq!(receipt_op["tags"][0], "billing");
        assert_eq!(
            receipt_op["responses"]["200"]["content"]["application/json"]["schema"]["$ref"],
            "#/components/schemas/ReceiptOut"
        );
        let params = receipt_op["parameters"].as_array().unwrap();
        assert!(params
            .iter()
            .any(|p| p["name"] == "id" && p["in"] == "path"));
        assert!(params
            .iter()
            .any(|p| p["name"] == "include_lines" && p["in"] == "query"));

        let refund_op = &spec["paths"]["/api/v1/billing/payments/{id}/refund"]["post"];
        assert_eq!(refund_op["operationId"], "refund_payment");
        assert_eq!(
            refund_op["requestBody"]["content"]["application/json"]["schema"]["$ref"],
            "#/components/schemas/RefundInput"
        );
        assert!(refund_op["responses"]["201"].is_object());
        assert!(spec["components"]["schemas"]["PaymentStatus"].is_object());

        let ts = builder.to_typescript();
        assert!(ts.contains("export interface ReceiptOut {"));
        assert!(ts.contains("  receiptId: number;"));
        assert!(ts.contains("  status: PaymentStatus;"));
        assert!(ts.contains("export type PaymentStatus = \"paid\" | \"refunded\";"));
        assert!(ts.contains("async getReceipt(id: string | number, query?: { include_lines?: boolean | null }): Promise<ReceiptOut>"));
        assert!(ts.contains(
            "async refundPayment(id: string | number, body: RefundInput): Promise<ReceiptOut>"
        ));
    }
}
