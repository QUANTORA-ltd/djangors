//! OpenAPI 3.1 schema generation from model metadata.

use std::collections::{BTreeMap, HashMap};

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

/// Builder for constructing OpenAPI 3.1 specifications from registered Djangors models.
#[derive(Debug, Clone, Default)]
pub struct OpenApiBuilder {
    title: String,
    version: String,
    schemas: HashMap<String, serde_json::Value>,
    paths: serde_json::Map<String, serde_json::Value>,
}

impl OpenApiBuilder {
    /// Creates a new `OpenApiBuilder` with specified title and API version.
    pub fn new(title: &str, version: &str) -> Self {
        Self {
            title: title.to_string(),
            version: version.to_string(),
            schemas: HashMap::new(),
            paths: serde_json::Map::new(),
        }
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
        self.register_crud_paths(schema_name, base_path);
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
        self.register_crud_paths(schema_name, base_path);
        self
    }

    fn register_crud_paths(&mut self, schema_name: &str, base_path: &str) {
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
                                "required": ["count", "page", "total_pages", "results"]
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
        list_create_item["get"] = list_op;
        list_create_item["post"] = create_op;
        self.paths.insert(list_create_path, list_create_item);

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
        detail_item["get"] = retrieve_op;
        detail_item["put"] = update_put_op;
        detail_item["patch"] = update_patch_op;
        detail_item["delete"] = destroy_op;
        self.paths.insert(detail_path, detail_item);
    }

    /// Builds and returns the complete OpenAPI 3.1 specification JSON Value.
    pub fn build(&self) -> serde_json::Value {
        serde_json::json!({
            "openapi": "3.1.0",
            "info": {
                "title": self.title,
                "version": self.version
            },
            "paths": self.paths,
            "components": {
                "schemas": self.schemas
            }
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
         export interface PaginatedList<T> {\n\
         \x20 count: number;\n\
         \x20 page: number;\n\
         \x20 total_pages: number;\n\
         \x20 results: T[];\n\
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

    // Collect base paths per model from `paths`
    let mut model_paths: BTreeMap<String, String> = BTreeMap::new();
    if let Some(paths) = spec.get("paths").and_then(|p| p.as_object()) {
        for (path, item) in paths {
            if path.ends_with("/{pk}") {
                continue;
            }
            if let Some(op_id) = item
                .get("post")
                .and_then(|post| post.get("operationId"))
                .and_then(|id| id.as_str())
            {
                if let Some(lower_model) = op_id.strip_prefix("create_") {
                    if let Some((schema_name, _)) = sorted_schemas
                        .iter()
                        .find(|(k, _)| k.to_lowercase() == lower_model)
                    {
                        model_paths.insert(schema_name.clone(), path.clone());
                    }
                }
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

    for (model, base_path) in &model_paths {
        let clean = base_path.trim_end_matches('/');
        out.push_str(&format!(
            "\n  async list{model}(params?: Record<string, string | number | boolean>): Promise<PaginatedList<{model}>> {{\n\
             \x20   const qs = params ? \"?\" + new URLSearchParams(Object.entries(params).map(([k, v]) => [k, String(v)])).toString() : \"\";\n\
             \x20   return this.request<PaginatedList<{model}>>(`{clean}${{qs}}`, {{ method: \"GET\" }});\n\
             \x20 }}\n\n\
             \x20 async retrieve{model}(pk: number | string): Promise<{model}> {{\n\
             \x20   return this.request<{model}>(`{clean}/${{encodeURIComponent(String(pk))}}`, {{ method: \"GET\" }});\n\
             \x20 }}\n\n\
             \x20 async create{model}(data: {model}CreateInput): Promise<{model}> {{\n\
             \x20   return this.request<{model}>(\"{clean}\", {{ method: \"POST\", body: JSON.stringify(data) }});\n\
             \x20 }}\n\n\
             \x20 async update{model}(pk: number | string, data: {model}CreateInput): Promise<{model}> {{\n\
             \x20   return this.request<{model}>(`{clean}/${{encodeURIComponent(String(pk))}}`, {{ method: \"PUT\", body: JSON.stringify(data) }});\n\
             \x20 }}\n\n\
             \x20 async partialUpdate{model}(pk: number | string, data: {model}UpdateInput): Promise<{model}> {{\n\
             \x20   return this.request<{model}>(`{clean}/${{encodeURIComponent(String(pk))}}`, {{ method: \"PATCH\", body: JSON.stringify(data) }});\n\
             \x20 }}\n\n\
             \x20 async destroy{model}(pk: number | string): Promise<void> {{\n\
             \x20   return this.request<void>(`{clean}/${{encodeURIComponent(String(pk))}}`, {{ method: \"DELETE\" }});\n\
             \x20 }}\n"
        ));
    }

    out.push_str("}\n");
    out
}

fn json_schema_to_ts_type(schema: &serde_json::Value) -> String {
    let base = match schema.get("type").and_then(|t| t.as_str()) {
        Some("integer") | Some("number") => "number",
        Some("boolean") => "boolean",
        Some("string") => "string",
        Some("object") => "Record<string, unknown>",
        Some("array") => "unknown[]",
        _ => "unknown",
    };
    if schema
        .get("nullable")
        .and_then(|n| n.as_bool())
        .unwrap_or(false)
    {
        format!("{base} | null")
    } else {
        base.to_string()
    }
}
