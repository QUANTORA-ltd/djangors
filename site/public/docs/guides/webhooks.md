# Outbound Webhooks

`djangors-contrib-webhooks` (`djangors::webhooks`, enabled by default) provides customer-facing outbound webhook endpoints with HMAC-SHA256 signing, replay-window protection, exponential backoff retries, and 1-click replay.

## Registering Endpoints & Emitting Events

```rust,illustrative
use djangors::webhooks::{create_webhooks_tables, WebhookDispatcher};

create_webhooks_tables(&db).await?;
let dispatcher = WebhookDispatcher::new(db.clone()).with_max_attempts(5);

dispatcher
    .register_endpoint(
        "org-1",
        "https://customer.example.com/webhooks",
        "whsec_super_secret",
        &["invoice.paid", "subscription.*"],
        "Billing events",
    )
    .await?;

// Enqueue deliveries for matching enabled endpoints
let delivery_ids = dispatcher
    .emit("org-1", "invoice.paid", &serde_json::json!({ "id": "inv_123" }))
    .await?;

// Process due deliveries (run periodically from a worker or cron task)
let report = dispatcher.deliver_due().await?;
```

## Signature Verification

Every request includes `X-Djangors-Signature: t=<unix_ts>,v1=<hex_hmac_sha256>` and `X-Djangors-Event`. Receivers can verify payloads with `verify_signature`:

```rust,illustrative
use djangors::webhooks::verify_signature;

verify_signature("whsec_super_secret", header_value, raw_body, now_unix, 300)?;
```
