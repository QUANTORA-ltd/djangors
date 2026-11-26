//! Live Task Queue Dashboard (`GET /tasks/dashboard/`, retry & purge endpoints).

use djangors_orm::Model;
use serde::Serialize;

use crate::{QueuedTask, RecurringTask};

/// Summary counters for the task queue dashboard.
#[derive(Debug, Clone, Serialize)]
pub struct QueueStats {
    /// Count of `"pending"` tasks.
    pub pending: i64,
    /// Count of `"running"` tasks.
    pub running: i64,
    /// Count of `"completed"` tasks.
    pub completed: i64,
    /// Count of `"failed"` tasks.
    pub failed: i64,
    /// Count of enabled [`RecurringTask`] schedules.
    pub recurring_enabled: i64,
}

/// Collect current queue counters from `db`.
pub async fn collect_queue_stats(
    db: &djangors_db::Database,
) -> Result<QueueStats, djangors_orm::OrmError> {
    let pending = QueuedTask::objects()
        .filter(djangors_orm::q!(status = "pending"))?
        .count(db)
        .await?;
    let running = QueuedTask::objects()
        .filter(djangors_orm::q!(status = "running"))?
        .count(db)
        .await?;
    let completed = QueuedTask::objects()
        .filter(djangors_orm::q!(status = "completed"))?
        .count(db)
        .await?;
    let failed = QueuedTask::objects()
        .filter(djangors_orm::q!(status = "failed"))?
        .count(db)
        .await?;
    let recurring_enabled = RecurringTask::objects()
        .filter(djangors_orm::q!(enabled = true))?
        .count(db)
        .await?;
    Ok(QueueStats {
        pending,
        running,
        completed,
        failed,
        recurring_enabled,
    })
}

/// Retry a failed (or stuck) task by ID, resetting status to `"pending"` and `attempts` to `0`.
pub async fn retry_task(
    db: &djangors_db::Database,
    task_id: i64,
) -> Result<u64, djangors_orm::OrmError> {
    let mut conn = db.conn();
    let dialect = conn.dialect();
    let sql = format!(
        "UPDATE djangors_task_queue SET status = 'pending', attempts = 0, scheduled_at = {}, error_message = NULL WHERE id = {}",
        dialect.current_timestamp(),
        dialect.placeholder(1),
    );
    let rows = conn
        .execute(&sql, &[djangors_db::BindValue::I64(task_id)])
        .await?;
    Ok(rows)
}

/// Delete all `"completed"` tasks from `djangors_task_queue`, returning the deleted row count.
pub async fn purge_completed_tasks(
    db: &djangors_db::Database,
) -> Result<u64, djangors_orm::OrmError> {
    let mut conn = db.conn();
    let rows = conn
        .execute(
            "DELETE FROM djangors_task_queue WHERE status = 'completed'",
            &[],
        )
        .await?;
    Ok(rows)
}

/// Build a [`djangors_core::Router`] serving the live task queue dashboard under `/tasks/dashboard/`.
///
/// Routes:
/// - `GET /tasks/dashboard/` — HTML dashboard with counts, failed jobs + Retry button, Purge Completed, and cron schedules
/// - `GET /tasks/dashboard/stats.json` — JSON [`QueueStats`]
/// - `POST /tasks/dashboard/retry/{id}` — retries a failed job and redirects back to `/tasks/dashboard/`
/// - `POST /tasks/dashboard/purge` — purges completed jobs and redirects back to `/tasks/dashboard/`
pub fn queue_dashboard_router(db: djangors_db::Database) -> djangors_core::Router {
    let db_html = db.clone();
    let db_json = db.clone();
    let db_retry = db.clone();
    let db_purge = db;

    djangors_core::Router::new()
        .get("/tasks/dashboard/", move |_req, _params| {
            let db = db_html.clone();
            async move {
                let stats = collect_queue_stats(&db)
                    .await
                    .map_err(|e| djangors_core::DjangorsError::Internal(e.to_string()))?;
                let failed = QueuedTask::objects()
                    .filter(djangors_orm::q!(status = "failed"))
                    .map_err(|e| djangors_core::DjangorsError::Internal(e.to_string()))?
                    .order_by("-id")
                    .map_err(|e| djangors_core::DjangorsError::Internal(e.to_string()))?
                    .limit(25)
                    .all(&db)
                    .await
                    .map_err(|e| djangors_core::DjangorsError::Internal(e.to_string()))?;
                let recurring = RecurringTask::objects()
                    .order_by("id")
                    .map_err(|e| djangors_core::DjangorsError::Internal(e.to_string()))?
                    .limit(50)
                    .all(&db)
                    .await
                    .unwrap_or_default();

                let failed_rows: String = failed
                    .iter()
                    .map(|t| {
                        let err = djangors_core::html_escape(
                            t.error_message.as_deref().unwrap_or("unknown error"),
                        );
                        let name = djangors_core::html_escape(&t.task_name);
                        format!(
                            "<tr><td>{}</td><td><code>{name}</code></td><td>{}/{}</td>\
                             <td><code>{err}</code></td>\
                             <td><form method=\"post\" action=\"/tasks/dashboard/retry/{}\">\
                             <button type=\"submit\">Retry</button></form></td></tr>",
                            t.id, t.attempts, t.max_attempts, t.id
                        )
                    })
                    .collect();

                let cron_rows: String = recurring
                    .iter()
                    .map(|r| {
                        format!(
                            "<tr><td>{}</td><td><code>{}</code></td><td><code>{}</code></td><td>{}</td><td>{}</td></tr>",
                            r.id,
                            djangors_core::html_escape(&r.task_name),
                            djangors_core::html_escape(&r.cron_expr),
                            r.enabled,
                            r.next_run_at.to_rfc3339()
                        )
                    })
                    .collect();

                let html = format!(
                    "<!DOCTYPE html><html><head><meta charset=\"utf-8\">\
                     <title>Djangors Task Queue Dashboard</title>\
                     <style>body{{font-family:system-ui,sans-serif;max-width:980px;margin:2rem auto;padding:0 1rem;color:#1f2937}}\
                     .cards{{display:grid;grid-template-columns:repeat(5,1fr);gap:.75rem;margin:1rem 0}}\
                     .card{{border:1px solid #e5e7eb;border-radius:8px;padding:.75rem 1rem;background:#f9fafb}}\
                     .card b{{display:block;font-size:1.5rem}}\
                     table{{width:100%;border-collapse:collapse;margin:1rem 0}}\
                     th,td{{text-align:left;padding:.5rem;border-bottom:1px solid #e5e7eb;font-size:14px}}\
                     button{{background:#201e1e;color:#f5f1e7;border:0;padding:.35rem .7rem;border-radius:4px;cursor:pointer}}</style>\
                     </head><body><h1>Task Queue Dashboard</h1>\
                     <div class=\"cards\">\
                     <div class=\"card\"><span>Pending</span><b>{}</b></div>\
                     <div class=\"card\"><span>Running</span><b>{}</b></div>\
                     <div class=\"card\"><span>Completed</span><b>{}</b></div>\
                     <div class=\"card\"><span>Failed</span><b>{}</b></div>\
                     <div class=\"card\"><span>Cron Active</span><b>{}</b></div>\
                     </div>\
                     <form method=\"post\" action=\"/tasks/dashboard/purge\" style=\"margin:1rem 0\">\
                     <button type=\"submit\">Purge Completed Tasks</button></form>\
                     <h2>Recent Failed Tasks</h2>\
                     <table><thead><tr><th>ID</th><th>Task</th><th>Attempts</th><th>Error</th><th>Action</th></tr></thead>\
                     <tbody>{}</tbody></table>\
                     <h2>Recurring Schedules</h2>\
                     <table><thead><tr><th>ID</th><th>Task</th><th>Cron</th><th>Enabled</th><th>Next Run</th></tr></thead>\
                     <tbody>{}</tbody></table></body></html>",
                    stats.pending,
                    stats.running,
                    stats.completed,
                    stats.failed,
                    stats.recurring_enabled,
                    if failed_rows.is_empty() {
                        "<tr><td colspan=\"5\">No failed tasks</td></tr>".to_string()
                    } else {
                        failed_rows
                    },
                    if cron_rows.is_empty() {
                        "<tr><td colspan=\"5\">No recurring tasks</td></tr>".to_string()
                    } else {
                        cron_rows
                    },
                );
                Ok(djangors_core::Response::html(
                    djangors_core::StatusCode::OK,
                    html,
                ))
            }
        })
        .get("/tasks/dashboard/stats.json", move |_req, _params| {
            let db = db_json.clone();
            async move {
                let stats = collect_queue_stats(&db)
                    .await
                    .map_err(|e| djangors_core::DjangorsError::Internal(e.to_string()))?;
                djangors_core::Response::json(djangors_core::StatusCode::OK, &stats)
            }
        })
        .post("/tasks/dashboard/retry/{id:i64}", move |_req: djangors_core::Request, params: djangors_core::PathParams| {
            let db = db_retry.clone();
            async move {
                let id = params.get_as::<i64>("id")?;
                retry_task(&db, id)
                    .await
                    .map_err(|e| djangors_core::DjangorsError::Internal(e.to_string()))?;
                Ok(djangors_core::Response::redirect("/tasks/dashboard/"))
            }
        })
        .post("/tasks/dashboard/purge", move |_req, _params| {
            let db = db_purge.clone();
            async move {
                purge_completed_tasks(&db)
                    .await
                    .map_err(|e| djangors_core::DjangorsError::Internal(e.to_string()))?;
                Ok(djangors_core::Response::redirect("/tasks/dashboard/"))
            }
        })
}

/// Mounts the live task queue dashboard onto an [`djangors_admin::AdminSite`].
pub fn register_dashboard(
    site: djangors_admin::AdminSite,
    db: djangors_db::Database,
) -> djangors_admin::AdminSite {
    site.extra_route(queue_dashboard_router(db))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    #[tokio::test]
    async fn queue_dashboard_serves_stats_and_retries_failed_task() {
        let cfg = djangors_db::DatabaseConfig::new("sqlite::memory:")
            .max_connections(1)
            .min_connections(1);
        let db = djangors_db::Database::connect(&cfg).await.unwrap();
        crate::create_task_table(&db).await.unwrap();
        crate::create_recurring_task_table(&db).await.unwrap();

        let t = QueuedTask {
            id: 0,
            task_name: "send_invoice".to_string(),
            payload: "{}".to_string(),
            status: "failed".to_string(),
            attempts: 3,
            max_attempts: 3,
            created_at: chrono::Utc::now(),
            scheduled_at: chrono::Utc::now(),
            error_message: Some("timeout".to_string()),
        }
        .save(&db)
        .await
        .unwrap();

        let router = queue_dashboard_router(db.clone());

        let stats_resp = router
            .clone()
            .oneshot(
                hyper::Request::builder()
                    .uri("/tasks/dashboard/stats.json")
                    .body(http_body_util::Full::new(Bytes::new()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(stats_resp.status(), djangors_core::StatusCode::OK);
        let body = stats_resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["failed"], 1);

        let retry_resp = router
            .clone()
            .oneshot(
                hyper::Request::builder()
                    .method("POST")
                    .uri(format!("/tasks/dashboard/retry/{}", t.id))
                    .body(http_body_util::Full::new(Bytes::new()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(retry_resp.status(), djangors_core::StatusCode::FOUND);

        let stats_after = collect_queue_stats(&db).await.unwrap();
        assert_eq!(stats_after.failed, 0);
        assert_eq!(stats_after.pending, 1);
    }
}
