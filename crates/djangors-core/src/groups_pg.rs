use std::sync::Arc;
use std::time::Duration;

use djangors_orm::djangors_db::{Database, Dialect};
use djangors_orm::sqlx::postgres::PgListener;
use djangors_orm::sqlx::PgPool;
use serde::Deserialize;
use tokio::task::JoinHandle;

use crate::error::DjangorsError;
use crate::groups::Groups;

/// PostgreSQL rejects `NOTIFY` payloads of this many bytes or more.
const MAX_NOTIFY_PAYLOAD_BYTES: usize = 8000;

/// How long the listener waits before retrying after an unexpected `LISTEN` error.
const LISTEN_RETRY_DELAY: Duration = Duration::from_secs(1);

/// The JSON envelope sent through `NOTIFY`.
#[derive(Deserialize)]
struct Envelope {
    group: String,
    message: String,
}

/// Cross-process broadcast groups backed by PostgreSQL `LISTEN`/`NOTIFY`.
///
/// [`Groups`] only reaches subscribers in the same process. `PgGroups` forwards
/// every [`send`](Self::send) through `pg_notify`, and each process that called
/// [`start`](Self::start) delivers the notification to its local [`Groups`].
/// Because the sending process receives its own notification through `LISTEN`,
/// call sites use only `send` and never deliver locally as well.
///
/// Cloning is cheap. The background listener task stops when the last clone is
/// dropped.
#[derive(Clone)]
pub struct PgGroups {
    inner: Arc<Inner>,
}

struct Inner {
    pool: PgPool,
    channel: String,
    listener_task: JoinHandle<()>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.listener_task.abort();
    }
}

impl PgGroups {
    /// Start listening on `channel` and forwarding its notifications into `groups`.
    ///
    /// The `LISTEN` command has completed when this returns, so a `send` made after
    /// `start` is delivered. Returns an error if `db` is not a PostgreSQL database
    /// or if the listener cannot be created.
    pub async fn start(
        db: &Database,
        groups: Groups,
        channel: &str,
    ) -> Result<Self, DjangorsError> {
        if !matches!(db.dialect(), Dialect::Postgres) {
            return Err(DjangorsError::Internal(
                "PgGroups requires a PostgreSQL database".to_string(),
            ));
        }

        let pool = db.pool().clone();

        let mut listener = PgListener::connect_with(&pool).await.map_err(|e| {
            DjangorsError::Internal(format!("PgGroups could not open a listener: {e}"))
        })?;
        listener.listen(channel).await.map_err(|e| {
            DjangorsError::Internal(format!("PgGroups could not LISTEN on {channel}: {e}"))
        })?;

        let listener_task = tokio::spawn(run_listener(listener, groups));

        Ok(Self {
            inner: Arc::new(Inner {
                pool,
                channel: channel.to_string(),
                listener_task,
            }),
        })
    }

    /// Send `message` to every process whose [`PgGroups`] listens on this channel,
    /// including this one.
    ///
    /// Returns an error if the encoded payload is 8000 bytes or more, since
    /// PostgreSQL cannot carry it. The message is never truncated.
    pub async fn send(&self, group: &str, message: &str) -> Result<(), DjangorsError> {
        let payload = encode_payload(group, message)?;

        djangors_orm::sqlx::query("SELECT pg_notify($1, $2)")
            .bind(self.inner.channel.as_str())
            .bind(payload.as_str())
            .execute(&self.inner.pool)
            .await
            .map_err(|e| DjangorsError::Internal(format!("PgGroups NOTIFY failed: {e}")))?;

        Ok(())
    }
}

/// Encode a group message as the JSON payload sent through `NOTIFY`.
fn encode_payload(group: &str, message: &str) -> Result<String, DjangorsError> {
    let payload = serde_json::json!({ "group": group, "message": message }).to_string();
    if payload.len() >= MAX_NOTIFY_PAYLOAD_BYTES {
        return Err(DjangorsError::Internal(format!(
            "PgGroups message is {} bytes; PostgreSQL NOTIFY payloads must be under {} bytes",
            payload.len(),
            MAX_NOTIFY_PAYLOAD_BYTES
        )));
    }
    Ok(payload)
}

/// Deliver one raw notification payload to the local `groups`.
///
/// Invalid payloads are logged and skipped.
fn deliver(groups: &Groups, payload: &str) {
    match serde_json::from_str::<Envelope>(payload) {
        Ok(envelope) => groups.group(&envelope.group).send(envelope.message),
        Err(err) => {
            tracing::warn!(error = %err, "skipping invalid PgGroups notification payload");
        }
    }
}

/// Forward notifications to `groups` until the pool closes.
///
/// `PgListener::recv` reconnects and re-LISTENs by itself after a dropped
/// connection. Other errors are logged and retried after a short delay.
async fn run_listener(mut listener: PgListener, groups: Groups) {
    loop {
        match listener.recv().await {
            Ok(notification) => deliver(&groups, notification.payload()),
            Err(djangors_orm::sqlx::Error::PoolClosed) => return,
            Err(err) => {
                tracing::warn!(error = %err, "PgGroups listener error; retrying");
                tokio::time::sleep(LISTEN_RETRY_DELAY).await;
            }
        }
    }
}

#[cfg(test)]
#[path = "groups_pg_tests.rs"]
mod tests;
