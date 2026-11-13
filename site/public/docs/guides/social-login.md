# Social Login (OAuth 2.0 & OpenID Connect)

Djangors provides OAuth 2.0 and OpenID Connect social login through the `djangors-contrib-oauth` crate (behind the off-by-default `oauth-client` feature on the `djangors` facade). All HTTP calls use `reqwest` over `rustls` (`ring` crypto provider; zero OpenSSL).

## Enabling the feature

```toml
[dependencies]
djangors = { version = "0.12.0", features = ["oauth-client"] }
```

## Database setup

Create the `oauth_social_account` table once during database initialization or migrations:

```rust,illustrative
use djangors_contrib_oauth::create_social_account_table;

create_social_account_table(&db).await?;
```

When `admin` is enabled, register `SocialAccount` with `register_client_admin(&site)`.

## Configuring providers and mounting routes

```rust,illustrative
use djangors_contrib_oauth::{
    oauth_client_router, AppleConfig, AppleProvider, GitHubConfig, GitHubProvider,
    GoogleProvider, MicrosoftProvider, MicrosoftTenant, OAuthRegistry, OidcConfig, OidcProvider,
};

let registry = OAuthRegistry::new("https://app.example.com")
    .with_post_login_redirect("/dashboard")
    .register(GoogleProvider::new(google_id, google_secret)?)
    .register(MicrosoftProvider::new(
        MicrosoftTenant::Common,
        ms_id,
        ms_secret,
    )?)
    .register(AppleProvider::new(AppleConfig::new(
        apple_services_id,
        apple_team_id,
        apple_key_id,
        apple_p8_pem,
    ))?)
    .register(GitHubProvider::new(GitHubConfig::new(gh_id, gh_secret))?)
    .register(OidcProvider::new(OidcConfig::new(
        "okta",
        "https://example.okta.com/oauth2/default",
        okta_id,
        Some(okta_secret),
    ))?);

let state = AppState::new().insert(db).insert(registry);
let app = Router::new().merge(oauth_client_router()).with_state(state);
```

This mounts:

- `GET /auth/{provider}/login` — redirects to the provider with one-time `state`, PKCE `S256` challenge, and OIDC `nonce` stored in the session.
- `GET|POST /auth/{provider}/callback` — validates `state`, exchanges the code with the PKCE verifier, verifies the ID token's JWKS signature, `iss`, `aud`, `exp`, `nonce`, and `at_hash` (or fetches GitHub `/user` + verified `/user/emails`), resolves the user, and calls `djangors_auth::login`.
- `GET /auth/{provider}/connect` — links a social account to an already logged-in user.
- `POST /auth/{provider}/disconnect` — unlinks a social account; refuses if the user has neither a usable password nor another linked `SocialAccount`.

## Account lookup & security rules

1. Existing `SocialAccount` matching `(provider, subject)`.
2. If in `/auth/{provider}/connect`, link to the logged-in user (rejecting if already linked elsewhere).
3. Existing local `User` with a matching **verified** email (`email_verified == true`). Unverified email claims never link to existing accounts.
4. Otherwise create a new local `User` with an unusable password (`"!"`).

> [!NOTE]
> Apple Sign In uses `response_mode=form_post`, so Apple's browser redirect sends a cross-site `POST` to `/auth/apple/callback`. Exempt `/auth/*/callback` from HTML form CSRF middleware; cross-site request forgery protection is enforced by the single-use `state` token bound to the user's session.
