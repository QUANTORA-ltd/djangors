# Sessions

Djangors provides pluggable session storage through `djangors-sessions` (`djangors::sessions`).

## Stores

- **`SignedCookieStore`** (default): stores session data in an HMAC-SHA256 signed cookie. No database or Redis required.
- **`DatabaseStore`**: stores session data in the `djangors_session` table. Session keys are 32 random bytes, stored as SHA-256 hashes in the database. `last_seen_at` is updated at most once per minute.
- **`RedisStore`** (requires the `redis` feature): stores session data in Redis with automatic TTL expiration and per-user session tracking.

```rust,illustrative
use std::sync::Arc;
use djangors::sessions::{DatabaseStore, SessionLayer};

# async fn example(db: djangors::db::Database) {
let store = DatabaseStore::new(db).with_secure(true);
let layer = SessionLayer::new(store);
# }
```

## Remember Me (`login_with`)

Use `login_with` to control whether a session persists across browser restarts:

```rust,illustrative
use djangors::auth::{login_with, LoginOptions, User};
use djangors::core::Request;

# async fn example(req: Request, user: User) -> Result<(), djangors::auth::AuthError> {
// Browser session: omits Max-Age on Set-Cookie, expires when the browser closes.
login_with(&req, &user, LoginOptions::new(false)).await?;

// Persistent session: sets Max-Age = settings.session_remember_age (default 30 days).
login_with(&req, &user, LoginOptions::new(true)).await?;
# Ok(())
# }
```

## Managing Active Sessions

With `DatabaseStore` or `RedisStore`, users can inspect and revoke their active sessions:

- `list_sessions(store, &user)` returns active `SessionRecord` entries (IP, user agent, timestamps).
- `revoke_session(store, &user, key_or_hash)` deletes a specific session owned by `user`.
- `logout_other_sessions(&req)` rotates the current session key and invalidates all other sessions (also works in signed-cookie mode via the user's `session_generation` counter).

## Cleaning Up Expired Sessions

Run `dj clearsessions` periodically to remove expired rows from `djangors_session`:

```bash
dj clearsessions
```
