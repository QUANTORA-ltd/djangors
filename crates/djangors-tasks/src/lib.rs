#![deny(missing_docs)]
//! Background task queue, `#[task]` attribute macro, and worker loop for Djangors.
//!
//! This crate provides a distributed background task worker system:
//! - [`QueuedTask`]: Database model representing an individual task instance queued for execution.
//! - [`RecurringTask`]: Database model representing a schedule configuration for cron-based recurring tasks.
//! - [`Worker`]: A worker instance that polls the database queue, claims tasks using database-level
//!   concurrency controls (e.g. `SKIP LOCKED` or advisory locks), runs them, and handles retries or failures.
//! - Tasks are registered globally at compile-time using the [`task`] attribute macro, which submits task metadata
//!   via the `inventory` crate to be discovered by the worker at runtime.

extern crate self as djangors_tasks;

use djangors_db::DbExecutor;
use djangors_macros::Model;
use std::future::Future;
use std::pin::Pin;
use std::str::FromStr;

// Helper to parse standard 5-field cron expressions. The underlying `cron` crate expects
// a 6-field expression (with seconds as the first field), so this prefixes "0 " before parsing.
fn parse_schedule(expression: &str) -> Result<cron::Schedule, String> {
    if expression.split_whitespace().count() != 5 {
        return Err("cron expression must contain exactly five fields".into());
    }
    cron::Schedule::from_str(&format!("0 {expression}")).map_err(|e| e.to_string())
}

pub use djangors_macros::task;
pub use inventory;
pub use serde_json;

/// Boxed pinned sendable future returned by background task handlers.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Error types for task queue operations and task execution.
#[derive(thiserror::Error, Debug)]
pub enum TaskError {
    /// A database operation failed.
    #[error("Database error: {0}")]
    Db(#[from] djangors_db::DbError),

    /// An ORM operation failed.
    #[error("ORM error: {0}")]
    Orm(#[from] djangors_orm::OrmError),

    /// A JSON serialization or deserialization error occurred.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// Task payload deserialization failed.
    #[error("Task payload deserialization failed: {0}")]
    PayloadDeserialization(String),

    /// Task serialization failed.
    #[error("Task serialization failed: {0}")]
    Serialization(String),

    /// Requested task handler was not found in the inventory registry.
    #[error("Task handler not found: {0}")]
    TaskNotFound(String),

    /// Task execution encountered an error.
    #[error("Task execution failed: {0}")]
    TaskExecution(String),

    /// Task execution panicked.
    #[error("Task panicked: {0}")]
    TaskPanicked(String),
}

/// Registration record for background tasks registered via the `#[task]` macro.
pub struct TaskRegistration {
    /// Unique identifier name of the task.
    pub name: &'static str,
    /// Function pointer executing the task handler logic given JSON payload value.
    pub handler: fn(serde_json::Value) -> BoxFuture<'static, Result<(), TaskError>>,
}

inventory::collect!(TaskRegistration);

/// Database model representing a task in the queue table.
#[derive(Model, Debug, Clone)]
#[djangors(app = "djangors_tasks", table_name = "djangors_task_queue")]
pub struct QueuedTask {
    /// Primary key task identifier.
    #[djangors(primary_key, auto)]
    pub id: i64,
    /// Name of the registered task handler.
    #[djangors(max_length = 255)]
    pub task_name: String,
    /// JSON payload parameter string.
    pub payload: String,
    /// Status string (`"pending"`, `"running"`, `"completed"`, `"failed"`).
    #[djangors(max_length = 50)]
    pub status: String,
    /// Number of execution attempts attempted so far.
    pub attempts: i32,
    /// Maximum allowed execution attempts before marking failed.
    pub max_attempts: i32,
    /// Record creation timestamp.
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Earliest scheduled execution timestamp.
    pub scheduled_at: chrono::DateTime<chrono::Utc>,
    /// Last error or panic message if execution failed.
    pub error_message: Option<String>,
}

/// Database model representing a task scheduled on a recurring cron expression.
#[derive(Model, Debug, Clone)]
#[djangors(app = "djangors_tasks", table_name = "djangors_recurring_task")]
pub struct RecurringTask {
    /// Primary key recurring task identifier.
    #[djangors(primary_key, auto)]
    pub id: i64,
    /// Name of the registered task handler.
    pub task_name: String,
    /// JSON payload parameter string.
    pub payload: String,
    /// Standard five-field cron expression.
    pub cron_expr: String,
    /// Next scheduled execution timestamp.
    pub next_run_at: chrono::DateTime<chrono::Utc>,
    /// Previous scheduled execution timestamp.
    pub last_run_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Whether this recurring task is active.
    pub enabled: bool,
}

/// Creates the database table `djangors_task_queue` if it does not already exist.
pub async fn create_task_table(db: &djangors_db::Database) -> Result<(), djangors_db::DbError> {
    let mut conn = db.conn();
    let dialect = conn.dialect();
    let pk_type = dialect.auto_pk_type();
    let ts_type = dialect.timestamp_type();
    let sql = format!(
        "CREATE TABLE IF NOT EXISTS djangors_task_queue (
            id {pk_type},
            task_name VARCHAR(255) NOT NULL,
            payload TEXT NOT NULL,
            status VARCHAR(50) NOT NULL,
            attempts INT NOT NULL DEFAULT 0,
            max_attempts INT NOT NULL DEFAULT 3,
            created_at {ts_type} NOT NULL,
            scheduled_at {ts_type} NOT NULL,
            error_message TEXT
        );"
    );
    conn.execute(&sql, &[])
        .await
        .map_err(djangors_db::DbError::QueryFailed)?;
    Ok(())
}

/// Creates the database table for recurring tasks if it does not already exist.
pub async fn create_recurring_task_table(
    db: &djangors_db::Database,
) -> Result<(), djangors_db::DbError> {
    let mut conn = db.conn();
    let dialect = conn.dialect();
    let pk_type = dialect.auto_pk_type();
    let ts_type = dialect.timestamp_type();
    let sql = format!(
        "CREATE TABLE IF NOT EXISTS djangors_recurring_task (
            id {pk_type},
            task_name TEXT NOT NULL,
            payload TEXT NOT NULL,
            cron_expr TEXT NOT NULL,
            next_run_at {ts_type} NOT NULL,
            last_run_at {ts_type},
            enabled BOOLEAN NOT NULL DEFAULT TRUE
        );"
    );
    conn.execute(&sql, &[])
        .await
        .map_err(djangors_db::DbError::QueryFailed)?;
    Ok(())
}

/// Registers a recurring task after immediately validating its standard cron expression.
pub async fn register_recurring(
    db: &djangors_db::Database,
    task_name: &str,
    payload: &impl serde::Serialize,
    cron_expr: &str,
) -> Result<i64, TaskError> {
    let schedule = parse_schedule(cron_expr)
        .map_err(|e| TaskError::Serialization(format!("invalid cron expression: {e}")))?;
    let next_run_at = schedule.upcoming(chrono::Utc).next().ok_or_else(|| {
        TaskError::Serialization("cron expression has no future occurrence".into())
    })?;
    let payload =
        serde_json::to_string(payload).map_err(|e| TaskError::Serialization(e.to_string()))?;
    let mut conn = db.conn();
    let dialect = conn.dialect();
    let p1 = dialect.placeholder(1);
    let p2 = dialect.placeholder(2);
    let p3 = dialect.placeholder(3);
    let p4 = dialect.placeholder(4);
    let sql = format!(
        "INSERT INTO djangors_recurring_task (task_name, payload, cron_expr, next_run_at, last_run_at, enabled) \
         VALUES ({p1}, {p2}, {p3}, {p4}, NULL, TRUE) RETURNING id"
    );
    let params = vec![
        djangors_db::BindValue::Text(task_name.to_string()),
        djangors_db::BindValue::Text(payload),
        djangors_db::BindValue::Text(cron_expr.to_string()),
        djangors_db::BindValue::DateTime(next_run_at),
    ];
    let row = conn
        .fetch_one(&sql, &params)
        .await
        .map_err(djangors_db::DbError::QueryFailed)?;
    let id = row
        .try_i64(0)
        .map_err(djangors_db::DbError::QueryFailed)?
        .ok_or_else(|| djangors_db::DbError::QueryFailed(sqlx::Error::RowNotFound))?;
    Ok(id)
}

/// Enqueues due recurring tasks atomically while holding row locks, returning the enqueue count.
pub async fn tick_recurring_tasks(db: &djangors_db::Database) -> Result<usize, TaskError> {
    db.transaction_conn(|conn| Box::pin(async move {
        let dialect = conn.dialect();
        // A tick spans several statements (cron evaluation, enqueue, and
        // schedule advancement). On Postgres, serialize that claim-and-advance
        // sequence at the database level so READ COMMITTED cannot let another tick
        // observe the same due row between those statements.
        // On SQLite, a write transaction holds an exclusive database lock for its duration,
        // so the interleaving the advisory lock defends against cannot occur.
        if dialect == djangors_db::Dialect::Postgres {
            conn.execute("SELECT pg_advisory_xact_lock(hashtextextended('djangors.tick_recurring_tasks', 0))", &[])
                .await
                .map_err(djangors_db::DbError::QueryFailed)?;
        }
        let now = chrono::Utc::now();
        let p1 = dialect.placeholder(1);
        let lock_clause = match dialect {
            djangors_db::Dialect::Postgres => " FOR UPDATE SKIP LOCKED",
            djangors_db::Dialect::Sqlite => "",
        };
        let sql_select = format!(
            "SELECT id, task_name, payload, cron_expr, next_run_at FROM djangors_recurring_task WHERE enabled = true AND next_run_at <= {p1} ORDER BY next_run_at, id{lock_clause}"
        );
        let params_select = vec![djangors_db::BindValue::DateTime(now)];
        let rows = conn.fetch_all(&sql_select, &params_select).await.map_err(djangors_db::DbError::QueryFailed)?;

        let p_ins1 = dialect.placeholder(1);
        let p_ins2 = dialect.placeholder(2);
        let p_ins3 = dialect.placeholder(3);
        let p_ins4 = dialect.placeholder(4);
        let sql_insert = format!(
            "INSERT INTO djangors_task_queue (task_name, payload, status, attempts, max_attempts, created_at, scheduled_at, error_message) VALUES ({p_ins1}, {p_ins2}, 'pending', 0, 3, {p_ins3}, {p_ins4}, NULL)"
        );

        let p_up1 = dialect.placeholder(1);
        let p_up2 = dialect.placeholder(2);
        let p_up3 = dialect.placeholder(3);
        let sql_update = format!(
            "UPDATE djangors_recurring_task SET last_run_at = {p_up1}, next_run_at = {p_up2} WHERE id = {p_up3}"
        );

        let mut count = 0;
        for row in rows {
            let id = row.try_i64(0).map_err(djangors_db::DbError::QueryFailed)?.unwrap_or_default();
            let task_name = row.try_string(1).map_err(djangors_db::DbError::QueryFailed)?.unwrap_or_default();
            let payload = row.try_string(2).map_err(djangors_db::DbError::QueryFailed)?.unwrap_or_default();
            let expr = row.try_string(3).map_err(djangors_db::DbError::QueryFailed)?.unwrap_or_default();
            let previous = row.try_datetime(4).map_err(djangors_db::DbError::QueryFailed)?.ok_or_else(|| djangors_db::DbError::QueryFailed(sqlx::Error::Protocol("missing next_run_at".into())))?;

            let schedule = parse_schedule(&expr).map_err(|e| djangors_db::DbError::QueryFailed(sqlx::Error::Protocol(e)))?;
            let next = schedule.after(&previous).next().ok_or_else(|| djangors_db::DbError::QueryFailed(sqlx::Error::Protocol("cron has no next occurrence".into())))?;

            let params_insert = vec![
                djangors_db::BindValue::Text(task_name),
                djangors_db::BindValue::Text(payload),
                djangors_db::BindValue::DateTime(previous),
                djangors_db::BindValue::DateTime(previous),
            ];
            conn.execute(&sql_insert, &params_insert).await.map_err(djangors_db::DbError::QueryFailed)?;

            let params_update = vec![
                djangors_db::BindValue::DateTime(previous),
                djangors_db::BindValue::DateTime(next),
                djangors_db::BindValue::I64(id),
            ];
            conn.execute(&sql_update, &params_update).await.map_err(djangors_db::DbError::QueryFailed)?;
            count += 1;
        }
        Ok::<usize, djangors_db::DbError>(count)
    })).await.map_err(TaskError::Db)
}

/// Enqueues a task for immediate execution.
pub async fn enqueue(
    db: &djangors_db::Database,
    task_name: &str,
    payload: &impl serde::Serialize,
) -> Result<i64, TaskError> {
    enqueue_scheduled(db, task_name, payload, chrono::Utc::now()).await
}

/// Enqueues a task with a specific scheduled_at time.
pub async fn enqueue_scheduled(
    db: &djangors_db::Database,
    task_name: &str,
    payload: &impl serde::Serialize,
    scheduled_at: chrono::DateTime<chrono::Utc>,
) -> Result<i64, TaskError> {
    let payload_str =
        serde_json::to_string(payload).map_err(|e| TaskError::Serialization(e.to_string()))?;
    let task = QueuedTask {
        id: 0,
        task_name: task_name.to_string(),
        payload: payload_str,
        status: "pending".to_string(),
        attempts: 0,
        max_attempts: 3,
        created_at: chrono::Utc::now(),
        scheduled_at,
        error_message: None,
    };
    let saved = task.save(db).await?;
    Ok(saved.id)
}

/// Atomically claims the next pending, due task using `SELECT ... FOR UPDATE SKIP LOCKED`
/// within a single transaction.
pub async fn claim_next_task(db: &djangors_db::Database) -> Result<Option<QueuedTask>, TaskError> {
    let claimed = db
        .transaction_conn(|conn| {
            Box::pin(async move {
                let dialect = conn.dialect();
                let now = chrono::Utc::now();
                let p1 = dialect.placeholder(1);
                let lock_clause = match dialect {
                    djangors_db::Dialect::Postgres => " FOR UPDATE SKIP LOCKED",
                    djangors_db::Dialect::Sqlite => "",
                };
                let sql_sel = format!(
                    "SELECT id, task_name, payload, status, attempts, max_attempts, created_at, scheduled_at, error_message \
                     FROM djangors_task_queue \
                     WHERE status = 'pending' AND scheduled_at <= {p1} \
                     ORDER BY scheduled_at ASC, id ASC{lock_clause} \
                     LIMIT 1"
                );
                let params_sel = vec![djangors_db::BindValue::DateTime(now)];
                let row_opt = conn
                    .fetch_optional(&sql_sel, &params_sel)
                    .await
                    .map_err(djangors_db::DbError::QueryFailed)?;

                let row = match row_opt {
                    Some(r) => r,
                    None => return Ok::<Option<QueuedTask>, djangors_db::DbError>(None),
                };

                let mut task = QueuedTask {
                    id: row.try_i64(0).map_err(djangors_db::DbError::QueryFailed)?.unwrap_or_default(),
                    task_name: row.try_string(1).map_err(djangors_db::DbError::QueryFailed)?.unwrap_or_default(),
                    payload: row.try_string(2).map_err(djangors_db::DbError::QueryFailed)?.unwrap_or_default(),
                    status: row.try_string(3).map_err(djangors_db::DbError::QueryFailed)?.unwrap_or_default(),
                    attempts: row.try_i64(4).map_err(djangors_db::DbError::QueryFailed)?.unwrap_or_default() as i32,
                    max_attempts: row.try_i64(5).map_err(djangors_db::DbError::QueryFailed)?.unwrap_or_default() as i32,
                    created_at: row.try_datetime(6).map_err(djangors_db::DbError::QueryFailed)?.ok_or_else(|| djangors_db::DbError::QueryFailed(sqlx::Error::Protocol("missing created_at".into())))?,
                    scheduled_at: row.try_datetime(7).map_err(djangors_db::DbError::QueryFailed)?.ok_or_else(|| djangors_db::DbError::QueryFailed(sqlx::Error::Protocol("missing scheduled_at".into())))?,
                    error_message: row.try_string(8).map_err(djangors_db::DbError::QueryFailed)?,
                };

                task.status = "running".to_string();
                task.attempts += 1;

                let p_up1 = dialect.placeholder(1);
                let p_up2 = dialect.placeholder(2);
                let sql_up = format!(
                    "UPDATE djangors_task_queue SET status = 'running', attempts = {p_up1} WHERE id = {p_up2}"
                );
                let params_up = vec![
                    djangors_db::BindValue::I64(task.attempts as i64),
                    djangors_db::BindValue::I64(task.id),
                ];
                conn.execute(&sql_up, &params_up)
                    .await
                    .map_err(djangors_db::DbError::QueryFailed)?;

                Ok(Some(task))
            })
        })
        .await?;

    Ok(claimed)
}

// Internally marks a claimed queued task as successfully completed.
async fn mark_task_completed(db: &djangors_db::Database, task_id: i64) -> Result<(), TaskError> {
    let mut conn = db.conn();
    let p1 = conn.dialect().placeholder(1);
    let sql = format!(
        "UPDATE djangors_task_queue SET status = 'completed', error_message = NULL WHERE id = {p1}"
    );
    let params = vec![djangors_db::BindValue::I64(task_id)];
    conn.execute(&sql, &params)
        .await
        .map_err(djangors_db::DbError::QueryFailed)?;
    Ok(())
}

// Internally updates a claimed queued task after execution failure, either requeueing it as
// "pending" (if attempts < max_attempts) or permanently marking it as "failed".
async fn mark_task_failed(
    db: &djangors_db::Database,
    task_id: i64,
    error_message: &str,
    attempts: i32,
    max_attempts: i32,
) -> Result<(), TaskError> {
    let next_status = if attempts < max_attempts {
        "pending"
    } else {
        "failed"
    };
    let mut conn = db.conn();
    let p1 = conn.dialect().placeholder(1);
    let p2 = conn.dialect().placeholder(2);
    let p3 = conn.dialect().placeholder(3);
    let sql = format!(
        "UPDATE djangors_task_queue SET status = {p1}, error_message = {p2} WHERE id = {p3}"
    );
    let params = vec![
        djangors_db::BindValue::Text(next_status.to_string()),
        djangors_db::BindValue::Text(error_message.to_string()),
        djangors_db::BindValue::I64(task_id),
    ];
    conn.execute(&sql, &params)
        .await
        .map_err(djangors_db::DbError::QueryFailed)?;
    Ok(())
}

/// Worker that claims and executes background tasks from the queue.
pub struct Worker {
    // Database connection pool shared by the worker for query operations.
    db: djangors_db::Database,
    // Idle sleep duration between query polling passes.
    poll_interval: std::time::Duration,
    // How frequently this worker will wake up to run the recurring tasks check.
    recurring_tick_interval: Option<std::time::Duration>,
}

impl Worker {
    /// Creates a new `Worker` backed by database `db`.
    pub fn new(db: djangors_db::Database) -> Self {
        Self {
            db,
            poll_interval: std::time::Duration::from_secs(1),
            recurring_tick_interval: None,
        }
    }

    /// Configures how often due recurring tasks are enqueued.
    pub fn with_recurring_tick_interval(mut self, interval: std::time::Duration) -> Self {
        self.recurring_tick_interval = Some(interval);
        self
    }

    /// Configures the polling interval for checking due tasks when idle.
    pub fn with_poll_interval(mut self, interval: std::time::Duration) -> Self {
        self.poll_interval = interval;
        self
    }

    /// Runs the worker loop indefinitely, processing tasks as they become due.
    pub async fn run(&self) -> ! {
        let result = self.run_until(std::future::pending::<()>()).await;
        match result {
            Ok(()) => unreachable!("a pending shutdown future cannot resolve"),
            Err(error) => panic!("worker loop failed: {error}"),
        }
    }

    /// Runs until `shutdown` resolves, finishing any task already in progress before returning.
    ///
    /// The shutdown future is also observed while the worker is idle, so an idle worker exits
    /// promptly instead of waiting for the next poll interval.
    pub async fn run_until<F>(&self, shutdown: F) -> Result<(), TaskError>
    where
        F: Future<Output = ()>,
    {
        let mut shutdown = Box::pin(shutdown);
        let mut last_recurring_tick = tokio::time::Instant::now();
        loop {
            tokio::select! {
                biased;
                _ = &mut shutdown => return Ok(()),
                _ = std::future::ready(()) => {}
            }
            if let Some(interval) = self.recurring_tick_interval {
                if last_recurring_tick.elapsed() >= interval {
                    let _ = tick_recurring_tasks(&self.db).await;
                    last_recurring_tick = tokio::time::Instant::now();
                }
            }
            let task_run = self.run_once();
            tokio::pin!(task_run);
            let result = tokio::select! {
                result = &mut task_run => result,
                _ = &mut shutdown => {
                    // Do not cancel a claimed task. Let the current run finish and persist its result.
                    task_run.await?;
                    return Ok(());
                }
            };
            match result {
                Ok(true) => {}
                Ok(false) => {
                    tokio::select! {
                        _ = &mut shutdown => return Ok(()),
                        _ = tokio::time::sleep(self.poll_interval) => {}
                    }
                }
                Err(_) => {
                    tokio::select! {
                        _ = &mut shutdown => return Ok(()),
                        _ = tokio::time::sleep(self.poll_interval) => {}
                    }
                }
            }
        }
    }

    /// Claims and attempts to execute a single task from the queue.
    pub async fn run_once(&self) -> Result<bool, TaskError> {
        let task = match claim_next_task(&self.db).await? {
            Some(t) => t,
            None => return Ok(false),
        };

        let handler = inventory::iter::<TaskRegistration>()
            .find(|reg| reg.name == task.task_name)
            .map(|reg| reg.handler);

        let handler = match handler {
            Some(h) => h,
            None => {
                let err_msg = format!("Task handler '{}' not found in registry", task.task_name);
                mark_task_failed(
                    &self.db,
                    task.id,
                    &err_msg,
                    task.attempts,
                    task.max_attempts,
                )
                .await?;
                return Ok(true);
            }
        };

        let payload_json: serde_json::Value = match serde_json::from_str(&task.payload) {
            Ok(v) => v,
            Err(e) => {
                let err_msg = format!("Failed to parse payload JSON: {}", e);
                mark_task_failed(
                    &self.db,
                    task.id,
                    &err_msg,
                    task.attempts,
                    task.max_attempts,
                )
                .await?;
                return Ok(true);
            }
        };

        let join_handle = tokio::spawn(async move { handler(payload_json).await });

        match join_handle.await {
            Ok(Ok(())) => {
                mark_task_completed(&self.db, task.id).await?;
            }
            Ok(Err(task_err)) => {
                let err_msg = task_err.to_string();
                mark_task_failed(
                    &self.db,
                    task.id,
                    &err_msg,
                    task.attempts,
                    task.max_attempts,
                )
                .await?;
            }
            Err(join_err) => {
                let err_msg = if join_err.is_panic() {
                    let payload = join_err.into_panic();
                    let msg = if let Some(s) = payload.downcast_ref::<&str>() {
                        s.to_string()
                    } else if let Some(s) = payload.downcast_ref::<String>() {
                        s.clone()
                    } else {
                        "unknown panic message".to_string()
                    };
                    format!("Task panicked: {}", msg)
                } else {
                    format!("Task execution failed: {}", join_err)
                };
                mark_task_failed(
                    &self.db,
                    task.id,
                    &err_msg,
                    task.attempts,
                    task.max_attempts,
                )
                .await?;
            }
        }

        Ok(true)
    }
}

/// Registers the `QueuedTask` model with an admin site instance.
/// Registers `QueuedTask` with the given admin site for real add/change/delete/changelist
/// views. `status` is a plain text field (not a boolean), so it can't be used as a
/// `list_filter` entry yet — this ORM's `list_filter` only supports Boolean fields today
/// (choices-based filtering is a separate, not-yet-built feature, tracked in
/// `docs/design/phase-5-roadmap.md`'s deferred-items ledger). `search_fields` gives staff a
/// working way to find tasks by name/status in the meantime.
pub fn register_admin(site: &djangors_admin::AdminSite) {
    site.register_with::<QueuedTask>(djangors_admin::ModelAdminConfig {
        list_display: Some(&["id", "task_name", "status", "attempts", "scheduled_at"]),
        search_fields: Some(&["task_name", "status"]),
        ..Default::default()
    });
}

#[cfg(test)]
mod tests;
