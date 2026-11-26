# Feature Flags & Gradual Rollouts

`djangors-contrib-flags` (`djangors::flags`, enabled by default) provides database- and cache-backed feature flags with user/org allowlists and deterministic percentage rollouts.

## Setup

```rust,illustrative
use std::sync::Arc;
use djangors::flags::{create_flags_table, FlagContext, FlagService};

// Create the `djangors_feature_flag` table (PostgreSQL & SQLite)
create_flags_table(&db).await?;

let flags = FlagService::new(db.clone())
    .with_cache(Arc::new(cache), 60); // optional 60s TTL cache
```

## Creating & Evaluating Flags

```rust,illustrative
// Enable "new_billing_ui" for 25% of subjects, plus beta orgs
flags
    .set_flag(
        "new_billing_ui",
        "Roll out redesigned billing portal",
        true,
        25,
        &["user-1"],
        &["org-beta"],
    )
    .await?;

let ctx = FlagContext::new().user("user-42").org("org-acme");
if flags.is_enabled("new_billing_ui", &ctx).await? {
    // serve new UI
}
```

### Evaluation Precedence

1. `enabled == false` → `false`
2. `user_id` in `allowed_users` or `org_id` in `allowed_orgs` → `true`
3. `rollout_percentage >= 100` → `true`; `<= 0` → `false`
4. Deterministic SHA-256 bucket on `"{flag_name}:{subject}"` (`% 100 < rollout_percentage`).

## Admin Integration

Enable the `admin` feature and call `djangors::flags::register_admin(&site)` to manage flags live in the Admin panel.
