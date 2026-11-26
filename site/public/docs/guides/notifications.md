# Multi-Channel Notifications

`djangors-contrib-notifications` (`djangors::notifications`, enabled by default) dispatches notifications across **Email**, **In-App DB Inbox**, **Webhooks**, **Slack**, and **Discord**, respecting per-user channel opt-outs.

## Defining a Notification

```rust,illustrative
use djangors::notifications::{
    create_notifications_tables, Notification, NotificationChannel,
    NotificationDispatcher, NotificationRecipient,
};

struct InvoicePaid {
    amount: String,
}

impl Notification for InvoicePaid {
    fn kind(&self) -> &str { "invoice_paid" }
    fn channels(&self) -> Vec<NotificationChannel> {
        vec![NotificationChannel::Email, NotificationChannel::InApp, NotificationChannel::Slack]
    }
    fn subject(&self) -> String { format!("Invoice paid: {}", self.amount) }
    fn body(&self) -> String { format!("Payment of {} succeeded.", self.amount) }
}
```

## Dispatching & In-App Inbox

```rust,illustrative
create_notifications_tables(&db).await?;

let dispatcher = NotificationDispatcher::new(db.clone())
    .with_mail(Arc::new(mail_backend), "billing@example.com");

let recipient = NotificationRecipient::new("user-42")
    .email("ada@example.com")
    .slack("https://hooks.slack.com/services/...");

let report = dispatcher.send(&recipient, &InvoicePaid { amount: "$49.00".into() }).await?;
let unread = dispatcher.unread_count("user-42").await?;
dispatcher.mark_all_read("user-42").await?;
```
