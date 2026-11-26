#![deny(missing_docs)]
//! Multi-channel notifications (Email, In-App DB Inbox, Webhook, Slack, Discord)
//! with per-user preferences for Djangors.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use djangors_db::Database;
use djangors_macros::Model;
use djangors_mail::{MailBackend, Message as MailMessage};
use djangors_orm::{q, Model, SetExpr, Value};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Errors returned by notification dispatch, inbox queries, or schema setup.
#[derive(Error, Debug)]
pub enum NotificationError {
    /// Database layer error.
    #[error("database error: {0}")]
    Db(#[from] djangors_db::DbError),
    /// ORM query or model error.
    #[error("orm error: {0}")]
    Orm(#[from] djangors_orm::OrmError),
    /// Email backend delivery error.
    #[error("mail error: {0}")]
    Mail(#[from] djangors_mail::MailError),
    /// HTTP webhook delivery error.
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    /// JSON serialization error.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    /// Required target address, URL, or backend was not provided for a channel.
    #[error("missing notification target: {0}")]
    MissingTarget(String),
}

/// Supported notification delivery channels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationChannel {
    /// Email delivery via [`djangors_mail::MailBackend`].
    Email,
    /// Persistent in-app database inbox ([`InAppNotification`]).
    InApp,
    /// Generic JSON HTTP POST webhook.
    Webhook,
    /// Slack incoming webhook.
    Slack,
    /// Discord incoming webhook.
    Discord,
}

impl NotificationChannel {
    /// Returns the canonical string identifier stored in [`NotificationPreference`].
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Email => "email",
            Self::InApp => "in_app",
            Self::Webhook => "webhook",
            Self::Slack => "slack",
            Self::Discord => "discord",
        }
    }
}

/// Persistent in-app notification stored in the database inbox.
#[derive(Model, Debug, Clone, Serialize, Deserialize)]
#[djangors(
    app = "djangors_notifications",
    table_name = "djangors_inapp_notification"
)]
pub struct InAppNotification {
    /// Auto-incrementing primary key.
    #[djangors(primary_key, auto)]
    pub id: i64,
    /// Identifier of the recipient user.
    #[djangors(max_length = 128, db_index)]
    pub recipient_id: String,
    /// Notification kind/type identifier (e.g., `"invoice.paid"`).
    #[djangors(max_length = 128)]
    pub kind: String,
    /// Short notification title or subject.
    #[djangors(max_length = 255)]
    pub title: String,
    /// Notification body text.
    pub body: String,
    /// Arbitrary structured payload serialized as a JSON string.
    pub data_json: String,
    /// UTC timestamp when the user marked the notification as read, if read.
    pub read_at: Option<DateTime<Utc>>,
    /// UTC timestamp when the notification was created.
    pub created_at: DateTime<Utc>,
}

/// Per-user, per-kind, per-channel notification opt-in/opt-out preference.
#[derive(Model, Debug, Clone, Serialize, Deserialize)]
#[djangors(
    app = "djangors_notifications",
    table_name = "djangors_notification_preference"
)]
pub struct NotificationPreference {
    /// Auto-incrementing primary key.
    #[djangors(primary_key, auto)]
    pub id: i64,
    /// Identifier of the user this preference belongs to.
    #[djangors(max_length = 128, db_index)]
    pub user_id: String,
    /// Notification kind/type identifier.
    #[djangors(max_length = 128)]
    pub kind: String,
    /// Channel identifier matching [`NotificationChannel::as_str`].
    #[djangors(max_length = 32)]
    pub channel: String,
    /// Whether delivery on this channel is enabled (`true`) or opted out (`false`).
    pub enabled: bool,
}

/// Target addressing information for delivering a notification to a recipient.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationRecipient {
    /// Unique identifier for the recipient user.
    pub user_id: String,
    /// Optional email address for [`NotificationChannel::Email`].
    pub email: Option<String>,
    /// Optional HTTP endpoint URL for [`NotificationChannel::Webhook`].
    pub webhook_url: Option<String>,
    /// Optional Slack incoming webhook URL for [`NotificationChannel::Slack`].
    pub slack_webhook_url: Option<String>,
    /// Optional Discord incoming webhook URL for [`NotificationChannel::Discord`].
    pub discord_webhook_url: Option<String>,
}

impl NotificationRecipient {
    /// Creates a new recipient with only `user_id` set.
    pub fn new(user_id: impl Into<String>) -> Self {
        Self {
            user_id: user_id.into(),
            email: None,
            webhook_url: None,
            slack_webhook_url: None,
            discord_webhook_url: None,
        }
    }

    /// Sets the recipient's email address.
    pub fn email(mut self, email: impl Into<String>) -> Self {
        self.email = Some(email.into());
        self
    }

    /// Sets the recipient's generic webhook URL.
    pub fn webhook(mut self, url: impl Into<String>) -> Self {
        self.webhook_url = Some(url.into());
        self
    }

    /// Sets the recipient's Slack incoming webhook URL.
    pub fn slack(mut self, url: impl Into<String>) -> Self {
        self.slack_webhook_url = Some(url.into());
        self
    }

    /// Sets the recipient's Discord incoming webhook URL.
    pub fn discord(mut self, url: impl Into<String>) -> Self {
        self.discord_webhook_url = Some(url.into());
        self
    }
}

/// Trait implemented by application notification types.
pub trait Notification: Send + Sync {
    /// Machine-readable kind identifier used for routing and preference lookup.
    fn kind(&self) -> &str;
    /// Channels this notification should be delivered to.
    fn channels(&self) -> Vec<NotificationChannel>;
    /// Short subject or title for the notification.
    fn subject(&self) -> String;
    /// Plain-text body of the notification.
    fn body(&self) -> String;
    /// Optional HTML body used for email delivery.
    fn html_body(&self) -> Option<String> {
        None
    }
    /// Optional structured JSON metadata attached to in-app and webhook payloads.
    fn data(&self) -> serde_json::Value {
        serde_json::json!({})
    }
}

/// Summary of channels delivered to or skipped during [`NotificationDispatcher::send`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryReport {
    /// Channels that were successfully delivered.
    pub delivered: Vec<NotificationChannel>,
    /// Channels skipped because the user disabled them in [`NotificationPreference`].
    pub skipped_by_preference: Vec<NotificationChannel>,
}

/// Dispatches notifications across configured channels and manages the in-app inbox
/// and per-user channel preferences.
#[derive(Clone)]
pub struct NotificationDispatcher {
    db: Database,
    mail: Option<Arc<dyn MailBackend>>,
    from_email: String,
    client: reqwest::Client,
}

impl std::fmt::Debug for NotificationDispatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NotificationDispatcher")
            .field("has_mail", &self.mail.is_some())
            .field("from_email", &self.from_email)
            .finish_non_exhaustive()
    }
}

impl NotificationDispatcher {
    /// Creates a new dispatcher backed by `db` with no email backend configured.
    pub fn new(db: Database) -> Self {
        Self {
            db,
            mail: None,
            from_email: "noreply@localhost".to_string(),
            client: reqwest::Client::new(),
        }
    }

    /// Configures the [`MailBackend`] and sender address used for [`NotificationChannel::Email`].
    pub fn with_mail(mut self, mail: Arc<dyn MailBackend>, from_email: impl Into<String>) -> Self {
        self.mail = Some(mail);
        self.from_email = from_email.into();
        self
    }

    async fn is_channel_enabled(
        &self,
        user_id: &str,
        kind: &str,
        channel: NotificationChannel,
    ) -> Result<bool, NotificationError> {
        let pref = NotificationPreference::objects()
            .filter(q!(
                user_id = user_id,
                kind = kind,
                channel = channel.as_str()
            ))?
            .first(&self.db)
            .await?;
        Ok(pref.map(|p| p.enabled).unwrap_or(true))
    }

    /// Sends `notification` to `recipient` across all requested channels enabled by user preferences.
    pub async fn send(
        &self,
        recipient: &NotificationRecipient,
        notification: &dyn Notification,
    ) -> Result<DeliveryReport, NotificationError> {
        let mut report = DeliveryReport::default();
        let kind = notification.kind();
        let subject = notification.subject();
        let body = notification.body();
        let data = notification.data();

        for channel in notification.channels() {
            if !self
                .is_channel_enabled(&recipient.user_id, kind, channel)
                .await?
            {
                report.skipped_by_preference.push(channel);
                continue;
            }

            match channel {
                NotificationChannel::InApp => {
                    let item = InAppNotification {
                        id: 0,
                        recipient_id: recipient.user_id.clone(),
                        kind: kind.to_string(),
                        title: subject.clone(),
                        body: body.clone(),
                        data_json: serde_json::to_string(&data)?,
                        read_at: None,
                        created_at: Utc::now(),
                    };
                    item.save(&self.db).await?;
                }
                NotificationChannel::Email => {
                    let mail = self.mail.as_ref().ok_or_else(|| {
                        NotificationError::MissingTarget(
                            "mail backend is not configured".to_string(),
                        )
                    })?;
                    let to = recipient.email.as_deref().ok_or_else(|| {
                        NotificationError::MissingTarget(
                            "recipient email address is not set".to_string(),
                        )
                    })?;
                    let mut msg = MailMessage::new([to], &self.from_email, &subject, &body);
                    if let Some(html) = notification.html_body() {
                        msg = msg.html_body(html);
                    }
                    mail.send(&msg).await?;
                }
                NotificationChannel::Webhook => {
                    let url = recipient.webhook_url.as_deref().ok_or_else(|| {
                        NotificationError::MissingTarget(
                            "recipient webhook_url is not set".to_string(),
                        )
                    })?;
                    let payload = serde_json::json!({
                        "kind": kind,
                        "title": subject,
                        "body": body,
                        "data": data,
                    });
                    self.client
                        .post(url)
                        .json(&payload)
                        .send()
                        .await?
                        .error_for_status()?;
                }
                NotificationChannel::Slack => {
                    let url = recipient.slack_webhook_url.as_deref().ok_or_else(|| {
                        NotificationError::MissingTarget(
                            "recipient slack_webhook_url is not set".to_string(),
                        )
                    })?;
                    let payload = serde_json::json!({
                        "text": format!("*{subject}*\n{body}"),
                    });
                    self.client
                        .post(url)
                        .json(&payload)
                        .send()
                        .await?
                        .error_for_status()?;
                }
                NotificationChannel::Discord => {
                    let url = recipient.discord_webhook_url.as_deref().ok_or_else(|| {
                        NotificationError::MissingTarget(
                            "recipient discord_webhook_url is not set".to_string(),
                        )
                    })?;
                    let payload = serde_json::json!({
                        "content": format!("**{subject}**\n{body}"),
                    });
                    self.client
                        .post(url)
                        .json(&payload)
                        .send()
                        .await?
                        .error_for_status()?;
                }
            }

            report.delivered.push(channel);
        }

        Ok(report)
    }

    /// Returns the number of unread in-app notifications for `user_id`.
    pub async fn unread_count(&self, user_id: &str) -> Result<i64, NotificationError> {
        let count = InAppNotification::objects()
            .filter(q!(recipient_id = user_id, read_at__isnull = true))?
            .count(&self.db)
            .await?;
        Ok(count)
    }

    /// Lists in-app notifications for `user_id`, newest first, up to `limit` rows.
    pub async fn list_for_user(
        &self,
        user_id: &str,
        only_unread: bool,
        limit: i64,
    ) -> Result<Vec<InAppNotification>, NotificationError> {
        let qs = if only_unread {
            InAppNotification::objects()
                .filter(q!(recipient_id = user_id, read_at__isnull = true))?
        } else {
            InAppNotification::objects().filter(q!(recipient_id = user_id))?
        };
        let items = qs
            .order_by("-created_at")?
            .order_by("-id")?
            .limit(limit)
            .all(&self.db)
            .await?;
        Ok(items)
    }

    /// Marks a single in-app notification `id` belonging to `user_id` as read.
    ///
    /// Returns the number of rows updated (`0` or `1`).
    pub async fn mark_read(&self, id: i64, user_id: &str) -> Result<u64, NotificationError> {
        let updated = InAppNotification::objects()
            .filter(q!(id = id, recipient_id = user_id, read_at__isnull = true))?
            .update(
                &self.db,
                vec![("read_at", SetExpr::Literal(Value::DateTime(Utc::now())))],
            )
            .await?;
        Ok(updated)
    }

    /// Marks all unread in-app notifications for `user_id` as read.
    ///
    /// Returns the number of rows updated.
    pub async fn mark_all_read(&self, user_id: &str) -> Result<u64, NotificationError> {
        let updated = InAppNotification::objects()
            .filter(q!(recipient_id = user_id, read_at__isnull = true))?
            .update(
                &self.db,
                vec![("read_at", SetExpr::Literal(Value::DateTime(Utc::now())))],
            )
            .await?;
        Ok(updated)
    }

    /// Sets or updates a user's delivery preference for `(user_id, kind, channel)`.
    pub async fn set_preference(
        &self,
        user_id: &str,
        kind: &str,
        channel: NotificationChannel,
        enabled: bool,
    ) -> Result<NotificationPreference, NotificationError> {
        if let Some(mut existing) = NotificationPreference::objects()
            .filter(q!(
                user_id = user_id,
                kind = kind,
                channel = channel.as_str()
            ))?
            .first(&self.db)
            .await?
        {
            NotificationPreference::objects()
                .filter(q!(id = existing.id))?
                .update(
                    &self.db,
                    vec![("enabled", SetExpr::Literal(Value::Bool(enabled)))],
                )
                .await?;
            existing.enabled = enabled;
            return Ok(existing);
        }

        let pref = NotificationPreference {
            id: 0,
            user_id: user_id.to_string(),
            kind: kind.to_string(),
            channel: channel.as_str().to_string(),
            enabled,
        };
        let saved = pref.save(&self.db).await?;
        Ok(saved)
    }
}

/// Ensures the `djangors_inapp_notification` and `djangors_notification_preference` tables
/// and their indexes exist.
pub async fn create_notifications_tables(db: &Database) -> Result<(), NotificationError> {
    let mut conn = db.conn();
    let dialect = conn.dialect();
    let pk = dialect.auto_pk_type();
    let ts = dialect.timestamp_type();
    let bool_t = dialect.bool_type();

    let stmts = [
        format!(
            "CREATE TABLE IF NOT EXISTS djangors_inapp_notification (
                id {pk},
                recipient_id VARCHAR(128) NOT NULL,
                kind VARCHAR(128) NOT NULL,
                title VARCHAR(255) NOT NULL,
                body TEXT NOT NULL,
                data_json TEXT NOT NULL,
                read_at {ts},
                created_at {ts} NOT NULL
            )"
        ),
        "CREATE INDEX IF NOT EXISTS djangors_inapp_notification_recipient_idx
            ON djangors_inapp_notification (recipient_id)"
            .to_string(),
        format!(
            "CREATE TABLE IF NOT EXISTS djangors_notification_preference (
                id {pk},
                user_id VARCHAR(128) NOT NULL,
                kind VARCHAR(128) NOT NULL,
                channel VARCHAR(32) NOT NULL,
                enabled {bool_t} NOT NULL,
                UNIQUE (user_id, kind, channel)
            )"
        ),
        "CREATE INDEX IF NOT EXISTS djangors_notification_preference_user_idx
            ON djangors_notification_preference (user_id)"
            .to_string(),
    ];

    for sql in &stmts {
        conn.execute(sql, &[])
            .await
            .map_err(djangors_db::DbError::from)?;
    }
    Ok(())
}

/// Registers [`InAppNotification`] and [`NotificationPreference`] with [`djangors_admin::AdminSite`].
#[cfg(feature = "admin")]
pub fn register_admin(site: &djangors_admin::AdminSite) {
    site.register::<InAppNotification>();
    site.register::<NotificationPreference>();
}

#[cfg(test)]
mod tests;
