# Signing values and signed URLs

`djangors_core::signing` provides Django-style cryptographic signing (`Signer`, `TimestampSigner`, `dumps`/`loads`) and signed URLs (`sign_url`, `sign_url_for`, `SignedUrl`).

Every signature uses HMAC-SHA256 with a key derived from the secret and a caller-supplied **salt** for domain separation. Tokens signed under one salt never verify under another, even when both share `SECRET_KEY`. Comparisons are constant time, and an empty key fails closed with `SigningError::MissingKey`.

## Signing strings and payloads

```rust,illustrative
use djangors_core::signing::{dumps, loads, Signer, TimestampSigner};
use std::time::Duration;

# fn example() -> Result<(), Box<dyn std::error::Error>> {
let signer = Signer::new("your-secret-key", "myapp.invite")?;
let signed = signer.sign("user-42");
assert_eq!(signer.unsign(&signed)?, "user-42");

// Time-limited signing
let ts = TimestampSigner::new("your-secret-key", "myapp.reset", Duration::from_secs(3600))?;
let token = ts.sign("user-42");
let value = ts.unsign(&token)?;

// Structured JSON payloads (with optional zlib compression)
let packed = dumps(&serde_json::json!({"org": 7}), &signer, true)?;
let decoded: serde_json::Value = loads(&packed, &signer)?;
# Ok(())
# }
```

## Key rotation (`SECRET_KEY_FALLBACKS`)

`Signer::from_settings` and `TimestampSigner::from_settings` sign new values with `settings.secret_key` and accept signatures made with any key in `settings.secret_key_fallbacks`.

Configure fallbacks in `djangors.toml`:

```toml
secret_key = "current-secret-key"
secret_key_fallbacks = ["previous-secret-key"]
```

Or via `DJANGORS_SECRET_KEY_FALLBACKS` (comma-separated).

## Signed URLs

`sign_url` and `sign_url_for` append `exp` and `sig` query parameters covering the HTTP method, path, sorted query pairs, and expiry timestamp. Use the `SignedUrl` extractor in handlers to reject tampered or expired links with `403 Forbidden`:

```rust,illustrative
use djangors_core::signing::{sign_url_for, SignedUrl, Signer, SIGNED_URL_SALT};
use djangors_core::{DjangorsError, PathParams, Request, Response, StatusCode};
use std::time::Duration;

async fn download(req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
    let _verified = SignedUrl::from_request(&req).await?;
    Ok(Response::text(StatusCode::OK, "secret file contents"))
}

# fn make_link(secret: &str) -> Result<String, Box<dyn std::error::Error>> {
let signer = Signer::new(secret, SIGNED_URL_SALT)?;
let url = sign_url_for(&signer, "GET", "/files/report.pdf?v=1", Duration::from_secs(300))?;
# Ok(url)
# }
```
