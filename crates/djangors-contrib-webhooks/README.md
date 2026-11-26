# djangors-contrib-webhooks

Customer-facing outbound webhooks for Djangors with HMAC-SHA256 signing, replay-window verification, exponential backoff retries, and 1-click replay.

Part of [Djangors](https://github.com/QUANTORA-ltd/djangors), a batteries-included web framework for Rust in the spirit of Django.

```toml
[dependencies]
djangors-contrib-webhooks = "1.0"
```

Adds `WebhookEndpoint` and `WebhookDelivery` models, `sign_payload` / `verify_signature` HMAC-SHA256 helpers, `WebhookDispatcher` (`register_endpoint`, `emit`, `deliver_due`, `replay`), `create_webhooks_tables`, and optional `djangors-admin` registration.

- Documentation: https://docs.rs/djangors-contrib-webhooks
- Guides: https://djangors.vercel.app/docs/
- Repository: https://github.com/QUANTORA-ltd/djangors

Licensed under either of MIT or Apache-2.0 at your option.
