# Privacy, GDPR Export, and Account Deletion

`djangors-auth::gdpr` provides user data export as a ZIP archive and scheduled account deletion with a configurable grace period.

## Exporting User Data

`UserDataRegistry::with_builtins()` registers built-in exporters for `user`, `sessions`, `social_accounts`, `api_keys`, and `history`. Apps can register custom exporters under any name:

```rust,illustrative
use djangors::auth::{export_user_data, UserDataRegistry};

# async fn example(db: &djangors::db::Database) -> Result<(), djangors::auth::AuthError> {
let mut registry = UserDataRegistry::with_builtins();
registry.register("orders", |db, user_id| {
    Box::pin(async move {
        let _ = (db, user_id);
        Ok(serde_json::json!([]))
    })
});

let zip_bytes = export_user_data(db, &registry, 1).await?;
# let _ = zip_bytes;
# Ok(())
# }
```

Wire `request_export_view` (`POST /privacy/export`) and `download_export_view` (`GET /privacy/export/download`) to let authenticated users request and download their ZIP archive via a signed link.

## Scheduled Account Deletion

```rust,illustrative
use djangors::auth::{cancel_account_deletion, request_account_deletion, run_scheduled_deletions};

# async fn example(db: &djangors::db::Database) -> Result<(), djangors::auth::AuthError> {
// Schedule deletion 30 days from now (default):
request_account_deletion(db, 1, None).await?;

// User changed their mind:
cancel_account_deletion(db, 1).await?;

// Run in a recurring background task to process due deletions:
let processed = run_scheduled_deletions(db, &[]).await?;
# let _ = processed;
# Ok(())
# }
```
