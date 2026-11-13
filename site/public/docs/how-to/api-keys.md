# How to Authenticate Machines with API Keys

API keys let background workers, CLIs, and partner services authenticate without a user account or session cookie. Each key has the form `<prefix>.<secret>`: the 8-character `prefix` is stored in plaintext for lookup, while only the SHA-256 hash of `secret` is stored in the database.

## Creating the table and issuing a key

Create the `djangors_rest_apikey` table at startup (or via migrations), then call `create_api_key`. The full key string is returned once and cannot be recovered later.

```rust,compile
# use djangors_rest::{create_api_key, create_api_key_table};
# async fn example(db: &djangors_db::Database) -> Result<(), Box<dyn std::error::Error>> {
create_api_key_table(db).await?;

let (record, full_key) = create_api_key(
    db,
    "billing-worker",
    &["invoices:read", "invoices:write"],
    None,
)
.await?;

println!("Key prefix: {}", record.prefix);
println!("Give this secret to the client once: {full_key}");
# Ok(())
# }
```

## Sending and verifying a key

Clients can pass the key in either header:

- `X-Api-Key: <prefix>.<secret>`
- `Authorization: Api-Key <prefix>.<secret>`

Use the `ApiKeyAuth` extractor in handlers to require a valid, non-revoked, non-expired key:

```rust,compile
# use djangors_rest::ApiKeyAuth;
# use djangors_core::extract::FromRequest;
# use djangors_core::{DjangorsError, Request, Response, StatusCode};
async fn ingest_event(req: Request) -> Result<Response, DjangorsError> {
    let ApiKeyAuth(key) = ApiKeyAuth::from_request(&req).await?;
    Ok(Response::text(StatusCode::OK, &format!("authenticated as {}", key.name)))
}
```

Every failed check (missing header, unknown prefix, wrong secret, revoked key, or expired key) returns `401 Unauthorized` with `"invalid API key"` so callers cannot probe which check failed. On success, `last_used_at` is updated.

## Checking scopes

Scopes are space-separated strings stored on `ApiKey::scopes`. Check them directly with `key.has_scope("invoices:write")` or enforce them on routes using `HasApiKey` and `HasApiKeyScope`:

```rust,compile
# use djangors_rest::{HasApiKey, HasApiKeyScope, IsAuthenticated, PermissionExt};
# fn example() {
let any_key = HasApiKey;
let write_scope = HasApiKeyScope("invoices:write");
let user_or_key = IsAuthenticated.or(HasApiKey);
# let _ = (any_key, write_scope, user_or_key);
# }
```

## Revoking a key

Revoke a key by its prefix with `revoke_api_key`:

```rust,compile
# use djangors_rest::revoke_api_key;
# async fn example(db: &djangors_db::Database) -> Result<(), djangors_orm::OrmError> {
let revoked = revoke_api_key(db, "aB3dE7gH").await?;
# let _ = revoked;
# Ok(())
# }
```

## Admin and OpenAPI

Enable the `admin` feature on `djangors-rest` and call `djangors_rest::register_admin(&site)` to manage and revoke keys in the admin UI.

To document the `X-Api-Key` header in your OpenAPI 3.1 schema, call `.api_key_security()` on `OpenApiBuilder`:

```rust,compile
# use djangors_rest::OpenApiBuilder;
# fn example() {
let mut builder = OpenApiBuilder::new("My API", "1.0.0");
builder.api_key_security();
let spec = builder.build();
# let _ = spec;
# }
```
