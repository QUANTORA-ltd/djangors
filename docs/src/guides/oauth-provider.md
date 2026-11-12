# Acting as an OAuth 2.0 / OpenID Connect Provider

Djangors includes an OAuth 2.0 authorization server and OpenID Connect (OIDC) provider in `djangors::oauth::provider` (crate `djangors-contrib-oauth`).

## Enabling the feature

Add the `oauth-provider` feature to `djangors`:

```toml
[dependencies]
djangors = { version = "0.13", features = ["oauth-provider"] }
```

## Database setup and signing keys

Create the provider tables once at startup (or in a migration) and generate an asymmetric OIDC signing key (`RS256` or `ES256`). Ensure `DJANGORS_FIELD_ENCRYPTION_KEY` (or `DJANGORS_FIELD_ENCRYPTION_KEYS`) is configured because `SigningKey::private_key_pem` is stored with `#[djangors(encrypted)]`.

```rust,illustrative
use djangors::oauth::{
    create_oauth_provider_tables, oauth_provider_router, OAuthApplication,
    OAuthProviderConfig, SigningKey,
};
use djangors::orm::Model as _;
use djangors::AppState;

# async fn setup(db: djangors::db::Database) -> Result<(), Box<dyn std::error::Error>> {
create_oauth_provider_tables(&db).await?;

// Generate an RS256 signing key if none exists yet.
if SigningKey::objects().all(&db).await?.is_empty() {
    SigningKey::generate_rs256()?.save(&db).await?;
}

// Register a confidential web client (keep `client_secret` — it is only returned once).
let (app, client_secret) = OAuthApplication::create_confidential(
    &db,
    "Partner Portal",
    &["https://partner.example.com/oauth/callback"],
    &["authorization_code", "refresh_token"],
    &["openid", "profile", "email", "offline_access"],
    false,
    None,
)
.await?;

let state = AppState::new()
    .insert(db)
    .insert(OAuthProviderConfig::new("https://auth.example.com"));

let router = oauth_provider_router();
# let _ = (app, client_secret, state, router);
# Ok(())
# }
```

## Endpoints

`oauth_provider_router()` mounts:

| Endpoint | Purpose |
| --- | --- |
| `GET \| POST /o/authorize` | Authorization endpoint with HTML consent form (auto-approves if `skip_consent = true`) |
| `POST /o/token` | Token endpoint (`authorization_code`, `client_credentials`, `refresh_token`) with `Cache-Control: no-store` |
| `POST /o/revoke` | RFC 7009 token revocation (revokes access tokens or entire refresh-token families) |
| `POST /o/introspect` | RFC 7662 token introspection (requires client authentication) |
| `GET \| POST /o/userinfo` | OIDC UserInfo endpoint filtered by granted scopes (`openid`, `profile`, `email`) |
| `GET /.well-known/openid-configuration` | OpenID Provider discovery metadata |
| `GET /o/jwks` | Public JSON Web Key Set for active signing keys |

## Protecting API views

Use [`OAuthBearer`](../../crates/djangors-contrib-oauth/src/provider.rs) to validate incoming `Authorization: Bearer <token>` headers, or [`HasOAuthScope`](../../crates/djangors-contrib-oauth/src/provider.rs) to enforce scopes:

```rust,illustrative
use djangors::oauth::HasOAuthScope;
use djangors::prelude::*;

async fn read_reports(req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
    let bearer = HasOAuthScope("read").check(&req).await?;
    Response::json(StatusCode::OK, &serde_json::json!({
        "client_app_id": bearer.application_id,
        "user_id": bearer.user_id,
    }))
}
```

## Admin and bulk revocation

When the `admin` feature is enabled, register all provider models on your `AdminSite` and revoke tokens programmatically via `revoke_app_tokens`:

```rust,illustrative
djangors::oauth::register_provider_admin(&admin_site);
let revoked = djangors::oauth::revoke_app_tokens(&db, app.id).await?;
```

## Security properties

- **PKCE S256 required for public clients**: Public clients (`confidential = false`) cannot skip PKCE and cannot downgrade `code_challenge_method` to `plain`. Set `require_pkce_for_all_clients(true)` on `OAuthProviderConfig` to enforce PKCE for confidential clients too.
- **Exact `redirect_uri` matching**: Wildcards and prefix matching are rejected before redirecting.
- **Short-lived, single-use authorization codes**: Default TTL is 120 seconds, hashed with SHA-256 at rest, and consumed atomically via a single conditional `UPDATE`.
- **Rotating refresh tokens with reuse detection**: Every refresh exchange issues a new `RefreshToken` in the same `family_id`. Presenting a previously used or revoked refresh token immediately revokes all refresh tokens in that family.
- **Hashed client secrets & tokens**: Only SHA-256 digests of secrets, codes, access tokens, and refresh tokens are stored; comparisons run in constant time.
