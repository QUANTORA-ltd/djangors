# Security

Djangors includes built-in security middleware covering CSRF protection, security response headers, host header validation, and HMAC-signed session cookie storage.

## CSRF Protection (`CsrfLayer`)

`CsrfLayer` (`djangors_core::middleware::csrf_layer()`) implements CSRF protection using a double-submit cookie scheme with header and form-body validation fallbacks.

```rust,compile
# fn main() {
# let router_service = djangors_core::router::RouterService::new(djangors_core::Router::new(), false);
use djangors_core::middleware::csrf_layer;
use tower::ServiceBuilder;

let service = ServiceBuilder::new()
    .layer(csrf_layer())
    .service(router_service);
# }
```

### Mechanism & Verification Flow
1. **Cookie Generation**: On safe requests (`GET`, `HEAD`, `OPTIONS`), if no `csrftoken` cookie exists, a cryptographically secure random 32-byte hex token is generated and returned in a `csrftoken` cookie (`SameSite=Lax; Path=/; Max-Age=31536000`). Note: The cookie is intentionally *not* `HttpOnly` so client-side JavaScript can read it to populate request headers.
2. **Unsafe Method Validation**: For unsafe HTTP methods (`POST`, `PUT`, `PATCH`, `DELETE`), the token in the `X-CSRFToken` request header is compared against the `csrftoken` cookie value using constant-time comparison (`constant_time_eq`).
3. **Form Fallback**: If the header is missing or mismatched, `CsrfLayer` sets `CsrfPendingFormCheck` on the request extensions. Form submission handlers verify the `csrfmiddlewaretoken` form body field against the pending token, returning `403 Forbidden` if missing or invalid.

**Production checklist**: `csrf_layer()` defaults to `Secure` **off** on the `csrftoken` cookie, the
same way `SignedCookieStore::new()` defaults `Secure` off (see below). This keeps local HTTP
development working out of the box. In production, enable it the same way the session store does:
`csrf_layer().with_secure(!settings.debug)`. Both `examples/school` and `examples/polls` wire this
correctly; if you're copying from an older reference, check that your own `main.rs` does too. An
internal security review (2026-07-27) found neither example app had actually done this before,
despite the session-cookie guidance below already documenting the correct pattern.

---

## Security Headers (`SecurityHeadersLayer`)

`SecurityHeadersLayer` (`djangors_core::middleware::security_headers_layer()`) sets standard security response headers mimicking Django's `SecurityMiddleware`:

```rust,compile
# fn main() {
# let router_service = djangors_core::router::RouterService::new(djangors_core::Router::new(), false);
use tower::ServiceBuilder;
use djangors_core::middleware::security_headers_layer;

let service = ServiceBuilder::new()
    .layer(security_headers_layer())
    .service(router_service);
# }
```

### Headers Set
- **`X-Content-Type-Options: nosniff`**: Prevents browsers from MIME-sniffing responses away from the declared content type.
- **`X-Frame-Options: DENY`**: Protects against clickjacking by preventing the app from being embedded in `<frame>`, `<iframe>`, or `<object>` elements.
- **`Referrer-Policy: same-origin`**: Restricts referrer information sent in HTTP headers to same-origin requests.

---

## Strict Transport Security (`HstsLayer`)

Enforces HTTPS via HTTP Strict Transport Security (HSTS):

```rust,compile
# fn main() {
use djangors_core::middleware::hsts_layer;

// Sets Strict-Transport-Security: max-age=31536000; includeSubDomains
let hsts = hsts_layer(31536000).with_include_subdomains(true);
# }
```

---

## Host Header Validation (`HostValidationLayer`)

`HostValidationLayer` validates incoming HTTP `Host` headers against the project's `ALLOWED_HOSTS` setting:

```rust,compile
# fn main() {
use djangors_core::middleware::HostValidationLayer;

let layer = HostValidationLayer::new(vec!["example.com".to_string(), "api.example.com".to_string()]);
# }
```

- Strip trailing port numbers before matching.
- Requests with disallowed Host headers are immediately rejected with `400 Bad Request: Disallowed Host` before reaching application handlers.

---

## Content Security Policy (`CspBuilder`)

`CspBuilder` (`djangors_core::middleware::CspBuilder`) is the `django-csp` equivalent: a builder
for the `Content-Security-Policy` response header, with one method per directive:

```rust,compile
# fn main() {
use djangors_core::middleware::CspBuilder;

let csp_layer = CspBuilder::new()
    .default_src(&["'self'"])
    .script_src(&["'self'", "https://cdn.example.com"])
    .style_src(&["'self'", "'unsafe-inline'"])
    .img_src(&["'self'", "data:"])
    .frame_ancestors(&["'none'"])
    .upgrade_insecure_requests()
    .build();
# let _ = csp_layer;
# }
```

Supported directives: `default_src`, `script_src`, `style_src`, `img_src`, `connect_src`,
`font_src`, `frame_ancestors`, `form_action`, and `object_src` (each taking a source list), plus
`upgrade_insecure_requests()` (a bare flag, no source list). Call `.report_only(true)` before
`.build()` to send `Content-Security-Policy-Report-Only` instead of enforcing the policy. This is
useful for testing a new policy against real traffic before turning on enforcement. Directive
values pass through verbatim (no source-keyword validation), matching `HstsLayer`'s own
deliberately minimal scope, so you're responsible for getting the CSP syntax right.

---

## Signed Cookie Session Store (`SignedCookieStore`)

`SignedCookieStore` (`djangors_sessions::SignedCookieStore`) stores session data in client-side cookies signed with HMAC-SHA256 using `settings.SECRET_KEY`:

```rust,compile
# fn main() {
# let secret_key_bytes = b"01234567890123456789012345678901";
use djangors_sessions::{SessionLayer, SignedCookieStore};

let store = SignedCookieStore::new(secret_key_bytes)
    .with_cookie_name("djangors_sessionid".into())
    .with_secure(true); // Enforce Secure attribute in production

let session_layer = SessionLayer::new(store);
# }
```

### Security Features
- **HMAC Signature**: Cookies are encoded as `b64_json.b64_expiry.b64_mac`. Any tampering with payload or expiration invalidates the MAC signature, causing `decode()` to return `None` (clearing session state).
- **`HttpOnly` & `SameSite=Lax`**: Session cookies are marked `HttpOnly` (inaccessible to client JavaScript) and `SameSite=Lax`.
- **Session Fixation Prevention (`cycle_key`)**: Calling `session.cycle_key()` during authentication (`login()`) rotates the session key string, preventing session fixation attacks.

---

## Malware/AV Scanning of Uploads (`clamav` feature)

`djangors-staticfiles`'s optional `clamav` Cargo feature (off by default, so zero cost/deps unless
enabled) scans uploaded file bytes against a real `clamd` daemon before they're ever written to
disk or a `Storage` backend, using `clamd`'s `INSTREAM` wire protocol directly.

```rust,illustrative
use djangors_staticfiles::clamav::{ClamAvScanner, ScanResult};

async fn scan_upload(bytes: &[u8]) -> Result<(), &'static str> {
    let scanner = ClamAvScanner::unix("/var/run/clamav/clamd.ctl");
    match scanner.scan(bytes).await {
        Ok(ScanResult::Clean) => Ok(()),
        Ok(ScanResult::Infected(signature)) => {
            Err(Box::leak(format!("rejected upload: {signature}").into_boxed_str()))
        }
        Err(_) => Err("could not reach the malware scanner"),
    }
}
```

`ClamAvScanner::unix(path)` connects over a Unix domain socket (the common case when `clamd` runs
on the same host); `ClamAvScanner::tcp(host, port)` connects over TCP instead. Scanning happens
**in-memory** rather than by handing `clamd` a file path, since `clamd` runs as its own OS user and
generally can't read arbitrary application-owned paths. Scanning in-memory bytes also means
the check can happen before anything touches disk at all. This requires a real `clamd` daemon
already running and reachable; nothing in this module starts one for you.

---

## CORS

To let browser front ends on another origin call your API, configure `CorsConfig` and build a
Tower layer with `cors_layer(&config)`.

```rust,compile
# use djangors_core::middleware::{cors_layer, csrf_layer, CorsConfig};
# use djangors_core::{DjangorsError, Router};
# use tower::ServiceBuilder;
# fn example(settings: &djangors_core::DjangorsSettings, router: Router) -> Result<(), DjangorsError> {
// Origins come from `cors_allowed_origins` in djangors.toml or
// DJANGORS_CORS_ALLOWED_ORIGINS=https://app.example.com,https://admin.example.com
let cors = cors_layer(&CorsConfig::from_settings(settings).allow_credentials(true))?;

let service = ServiceBuilder::new()
    .layer(cors)
    .layer(csrf_layer())
    .service(router);
# let _ = service;
# Ok(())
# }
```

Each origin is `scheme://host[:port]` exactly as browsers send it, with no path or trailing
slash; `cors_layer` returns an error for anything else. With `allow_credentials(true)`, cookies
are sent cross-origin, so state-changing requests still go through CSRF: the front end must send
the `X-CSRFToken` header.

Always place `cors_layer` **outside** `csrf_layer()` so preflight `OPTIONS` requests are answered
before CSRF checks run. `cors_layer` rejects `"*"` combined with `allow_credentials(true)` because
browsers forbid wildcard origins with credentials, and `dj check --deploy` warns if
`cors_allowed_origins` contains `"*"` when `debug` is `false`.

## Cross-Origin Isolation & Permissions-Policy

`SecurityHeadersLayer` sets `Cross-Origin-Opener-Policy: same-origin` and
`Cross-Origin-Resource-Policy: same-origin` by default (while leaving
`Cross-Origin-Embedder-Policy` off so third-party embeds continue to work).
Use `SecurityHeadersBuilder` and `PermissionsPolicyBuilder` to customize them:

```rust,compile
# use djangors_core::middleware::{PermissionsPolicyBuilder, SecurityHeadersBuilder};
# fn example() {
let layer = SecurityHeadersBuilder::new()
    .permissions_policy(
        PermissionsPolicyBuilder::new()
            .feature("camera", &[])
            .feature("geolocation", &["self"]),
    )
    .coep(Some("require-corp"))
    .build();
# let _ = layer;
# }
```

## Per-Request CSP Nonces

Call `.with_nonce()` on `CspBuilder` or `CspLayer` to generate a fresh 16-byte cryptographic
nonce per request, store `CspNonce` in request extensions, and add `'nonce-...'` to
`script-src` and `style-src`. Live-reload `<script>` tags receive the nonce automatically,
and templates can read it with `{{ csp_nonce() }}` when `_csp_nonce` is passed in the template
context.

## Honeypot and Minimum Submit Time

In `djangors-forms`, `HoneypotField` renders a hidden field that must stay empty,
`MinSubmitTime` embeds an HMAC-signed timestamp to reject submissions faster than N seconds,
and `BotGuard` combines both with `is_suspicious_user_agent` so you can escalate to
`CaptchaField` only when a submission looks suspicious.
