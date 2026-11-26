# djangors-contrib-notifications

Multi-channel notifications (Email, In-App DB Inbox, Webhook, Slack, Discord) with per-user preferences for Djangors.

Part of [Djangors](https://github.com/QUANTORA-ltd/djangors), a batteries-included web framework for Rust in the spirit of Django.

```toml
[dependencies]
djangors-contrib-notifications = "1.0"
```

Provides `InAppNotification` and `NotificationPreference` ORM models, the `Notification` trait, `NotificationDispatcher` for routing across channels while respecting per-user opt-out preferences, inbox helpers (`unread_count`, `list_for_user`, `mark_read`, `mark_all_read`), and optional `djangors-admin` registration (`admin` feature).

- Documentation: https://docs.rs/djangors-contrib-notifications
- Guides: https://djangors.vercel.app/docs/
- Repository: https://github.com/QUANTORA-ltd/djangors

Licensed under either of MIT or Apache-2.0 at your option.
