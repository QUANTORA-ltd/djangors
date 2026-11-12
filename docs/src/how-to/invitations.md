# User Invitations

`djangors-auth` provides an `Invitation` model (`auth_invitation` table) and helpers for inviting new or existing users (optionally into an organization).

## Sending and Resending Invitations

```rust,illustrative
use std::time::Duration;
use djangors::auth::{invite, resend_invitation, revoke_invitation};

# async fn example(
#     db: &djangors::db::Database,
#     mailer: &dyn djangors::mail::MailBackend,
# ) -> Result<(), djangors::auth::AuthError> {
let (inv, _raw_token) = invite(
    db,
    mailer,
    1, // inviter user_id
    "teammate@example.com",
    Some(42), // optional organization_id
    "member",
    "https://example.com",
    Duration::from_secs(7 * 24 * 60 * 60),
)
.await?;

// Resending rotates the stored token hash and extends expiry:
let (_inv, _new_token) = resend_invitation(
    db,
    mailer,
    inv.id,
    "https://example.com",
    Duration::from_secs(7 * 24 * 60 * 60),
)
.await?;

// Revoking prevents acceptance:
revoke_invitation(db, inv.id).await?;
# Ok(())
# }
```

## Accepting Invitations

`accept_invitation(db, token, password, current_user)` handles both cases:

- **New user**: creates an active `User` with the supplied `password` (validated against `default_validators()`).
- **Existing user**: requires `current_user` to be `Some(user)` whose email matches the invitation email.

Mount `djangors::auth::invitation_router()` to expose `GET /invitations/accept` and `GET /admin/invitations` (staff only).
