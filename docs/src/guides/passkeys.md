# Passkeys (WebAuthn)

Passkeys let users sign in with a fingerprint, face, device PIN or security key instead
of a password. The private key never leaves the user's device. The server stores only a
public key, so there is nothing to phish and nothing useful to leak.

Djangors implements the server side of WebAuthn (the *relying party*) in pure Rust,
without OpenSSL. Enable it with the `passkeys` feature:

```toml
djangors = { version = "0.11", features = ["passkeys"] }
```

## What is supported

- **Algorithms**: ES256 (P-256) and EdDSA (Ed25519). Every current platform authenticator
  (Apple, Google Password Manager, Windows Hello, Android) and most security keys use one
  of these.
- **User verification is always required.** A passkey sign-in proves the user unlocked
  the device, not just that someone touched it.
- **Discoverable credentials** (`residentKey: "required"`), so users can sign in without
  typing a username.
- **Attestation `"none"`**: Djangors does not restrict which authenticator models are
  allowed, so it does not verify attestation statements.
- **Clone detection**: if an authenticator's signature counter stops increasing, sign-in
  is rejected with `CounterRegression`.

Every check WebAuthn requires is enforced: the challenge, the origin, the relying party
ID hash, the user-present and user-verified flags, the credential ID and user handle,
and the signature. Malformed input returns an error and never panics.

## The relying party

```rust,compile
use djangors_auth::passkeys::RelyingParty;

# fn example() {
// id: the domain passkeys are bound to. origin: the exact page origin.
let rp = RelyingParty::new("example.com", "Example", "https://example.com")
    .with_origin("https://app.example.com");
# let _ = rp;
# }
```

The ID must be the page's domain or a parent of it. Passkeys created for `example.com`
work on `app.example.com` only if that origin is listed with `with_origin`. For local
development, use `RelyingParty::new("localhost", "Dev", "http://localhost:8000")`.
Browsers allow WebAuthn on `localhost` without HTTPS.

## Registering a passkey

Registration takes two requests. Keep the state from the first request in the session
and remove it when the second request uses it:

```rust,compile
use djangors_auth::passkeys::{
    finish_registration, start_registration, user_handle_for, PasskeyStore,
    RegistrationResponse, RegistrationState, RelyingParty,
};

async fn begin(
    rp: &RelyingParty,
    store: &PasskeyStore,
    session: &djangors_sessions::Session,
    user_id: i64,
    username: &str,
) -> Result<serde_json::Value, djangors_core::DjangorsError> {
    let existing = store.for_user(user_id).await?;
    let (options, state) =
        start_registration(rp, &user_handle_for(user_id), username, username, &existing);
    session.set("passkey_registration", state);
    Ok(options) // send to the browser as JSON
}

async fn finish(
    rp: &RelyingParty,
    store: &PasskeyStore,
    session: &djangors_sessions::Session,
    user_id: i64,
    response: RegistrationResponse, // the browser's credential.toJSON()
) -> Result<(), djangors_core::DjangorsError> {
    let state: RegistrationState = session
        .get("passkey_registration")
        .ok_or_else(|| djangors_core::DjangorsError::BadRequest("no registration in progress".into()))?;
    session.remove("passkey_registration");
    let passkey = finish_registration(rp, &state, &response)?;
    store.add(user_id, &passkey, "My device").await?;
    Ok(())
}
```

`user_handle_for(user_id)` turns a numeric user ID into the opaque user handle that
WebAuthn stores on the device. Passing the user's existing passkeys in `exclude` stops
the browser from registering the same authenticator twice.

## Signing in

```rust,compile
use djangors_auth::passkeys::{
    finish_authentication, start_authentication, AuthenticationResponse,
    AuthenticationState, PasskeyStore, RelyingParty,
};

async fn begin(
    rp: &RelyingParty,
    session: &djangors_sessions::Session,
) -> serde_json::Value {
    // An empty list lets the browser offer any passkey for this site.
    let (options, state) = start_authentication(rp, &[]);
    session.set("passkey_authentication", state);
    options
}

/// Returns the signed-in user's ID.
async fn finish(
    rp: &RelyingParty,
    store: &PasskeyStore,
    session: &djangors_sessions::Session,
    response: AuthenticationResponse,
) -> Result<i64, djangors_core::DjangorsError> {
    let state: AuthenticationState = session
        .get("passkey_authentication")
        .ok_or_else(|| djangors_core::DjangorsError::BadRequest("no sign-in in progress".into()))?;
    session.remove("passkey_authentication");
    let (user_id, passkey) = store
        .find(&response.credential_id()?)
        .await?
        .ok_or_else(|| djangors_core::DjangorsError::BadRequest("unknown passkey".into()))?;
    let count = finish_authentication(rp, &state, &response, &passkey)?;
    store.update_sign_count(&passkey.credential_id, count).await?;
    Ok(user_id)
}
```

After `finish` returns, load the user and call `djangors_auth::login(&session, &user)`,
as with a password sign-in. `login` rotates the session key.

`PasskeyError` converts into `DjangorsError`, so `?` works in handlers. Verification
failures become `400` with the code `passkey_invalid`. Storage failures become `500`.

## Storage

`PasskeyStore` keeps passkeys in the `djangors_auth_passkey` table and creates the table
on first use. It works on PostgreSQL and SQLite. Each row holds the owning user, the
credential ID, the public key, the signature counter, a display name, and the creation
and last-use times.

| Method | What it does |
|---|---|
| `add(user_id, &passkey, name)` | Saves a new passkey. A credential ID can be registered once. |
| `find(credential_id)` | Returns the owning user ID and the passkey. |
| `for_user(user_id)` | All of a user's passkeys, for a settings page or for `exclude`. |
| `update_sign_count(credential_id, count)` | Records a sign-in. |
| `remove(user_id, credential_id)` | Deletes a passkey only if it belongs to that user. |

## The browser side

Modern browsers convert between WebAuthn objects and JSON for you:

```javascript
// Registration
const options = await (await fetch("/passkeys/register/begin", { method: "POST" })).json();
const credential = await navigator.credentials.create({
  publicKey: PublicKeyCredential.parseCreationOptionsFromJSON(options.publicKey),
});
await fetch("/passkeys/register/finish", {
  method: "POST",
  headers: { "Content-Type": "application/json" },
  body: JSON.stringify(credential.toJSON()),
});

// Sign-in
const request = await (await fetch("/passkeys/login/begin", { method: "POST" })).json();
const assertion = await navigator.credentials.get({
  publicKey: PublicKeyCredential.parseRequestOptionsFromJSON(request.publicKey),
});
await fetch("/passkeys/login/finish", {
  method: "POST",
  headers: { "Content-Type": "application/json" },
  body: JSON.stringify(assertion.toJSON()),
});
```

`RegistrationResponse` and `AuthenticationResponse` deserialize the output of
`toJSON()` directly. For older browsers without `parseCreationOptionsFromJSON`, decode
the base64url `challenge`, `user.id` and credential `id` fields to `ArrayBuffer`s
yourself.

## Security notes

- Serve the site over HTTPS. Browsers refuse WebAuthn on plain HTTP except on
  `localhost`.
- Use each challenge once. Remove the state from the session before verifying, as in
  the examples, so a failed attempt can't be retried with the same challenge.
- Protect the begin and finish endpoints with CSRF protection like any other POST.
- Passkeys can replace passwords or sit alongside them. Let users register more than
  one passkey, so losing a device doesn't lock them out.
