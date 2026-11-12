use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

static TEST_TASK_EXECUTED: AtomicBool = AtomicBool::new(false);

// All DB-touching tests below share the single real `djangors_test` database and a
// fixed `djangors_task_queue` table name (djangors-test's TestDatabase does not yet
// provide per-test isolation - see docs/design's TestDatabase rollback/fixtures item).
// `cargo test` runs `#[tokio::test]` functions concurrently by default, so without this
// lock two tests racing on the same table produce nondeterministic row counts. Every
// test that touches `djangors_task_queue` must acquire this guard before doing so.
static TEST_DB_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(serde::Serialize, serde::Deserialize)]
struct SamplePayload {
    message: String,
}

#[task]
async fn sample_task_fn(payload: SamplePayload) -> Result<(), TaskError> {
    if payload.message == "hello_tasks" {
        TEST_TASK_EXECUTED.store(true, Ordering::SeqCst);
    }
    Ok(())
}

#[task(name = "custom_registered_task")]
async fn custom_task_fn(payload: SamplePayload) -> Result<(), TaskError> {
    if payload.message == "custom" {
        TEST_TASK_EXECUTED.store(true, Ordering::SeqCst);
    }
    Ok(())
}

#[task]
async fn failing_task_fn(_payload: SamplePayload) -> Result<(), TaskError> {
    Err(TaskError::TaskExecution("simulated error".to_string()))
}

#[task]
async fn panicking_task_fn(_payload: SamplePayload) -> Result<(), TaskError> {
    panic!("simulated task panic!");
}

#[test]
fn test_task_macro_discovery_and_execution() {
    let mut found = false;
    for reg in inventory::iter::<TaskRegistration>() {
        if reg.name == "sample_task_fn" {
            found = true;
            TEST_TASK_EXECUTED.store(false, Ordering::SeqCst);
            let payload = serde_json::json!({ "message": "hello_tasks" });
            let fut = (reg.handler)(payload);
            tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(fut)
                .unwrap();
            assert!(TEST_TASK_EXECUTED.load(Ordering::SeqCst));
        }
    }
    assert!(found, "sample_task_fn should be discovered via inventory");
}

#[test]
fn test_custom_name_macro() {
    let found =
        inventory::iter::<TaskRegistration>().any(|reg| reg.name == "custom_registered_task");
    assert!(
        found,
        "custom_registered_task should be registered with explicit name"
    );
}

#[test]
fn test_admin_registration() {
    let site = djangors_admin::AdminSite::new();
    register_admin(&site);
}

#[tokio::test]
async fn test_concurrent_claim_skip_locked() -> Result<(), Box<dyn std::error::Error>> {
    let test_db = djangors_test::TestDatabase::connect().await?;
    let _guard = TEST_DB_LOCK.lock().await;
    let db = test_db.database();
    create_task_table(db).await?;

    // Clear queue
    db.conn()
        .execute("DELETE FROM djangors_task_queue", &[])
        .await?;

    // Enqueue ONE task
    let task_id = enqueue(
        db,
        "sample_task_fn",
        &SamplePayload {
            message: "hello_tasks".into(),
        },
    )
    .await?;

    // Spawn two concurrent claims against the same row
    let db1 = db.clone();
    let db2 = db.clone();

    let handle1 = tokio::spawn(async move { claim_next_task(&db1).await });
    let handle2 = tokio::spawn(async move { claim_next_task(&db2).await });

    let (res1, res2) = tokio::join!(handle1, handle2);
    let claim1 = res1??;
    let claim2 = res2??;

    // Exactly one should get Some(task_id), the other None
    let claimed_count =
        (if claim1.is_some() { 1 } else { 0 }) + (if claim2.is_some() { 1 } else { 0 });
    assert_eq!(
        claimed_count, 1,
        "Exactly one worker should claim the single pending task"
    );

    let claimed_task = claim1.or(claim2).unwrap();
    assert_eq!(claimed_task.id, task_id);
    assert_eq!(claimed_task.status, "running");
    assert_eq!(claimed_task.attempts, 1);

    Ok(())
}

#[tokio::test]
async fn test_worker_run_once_success() -> Result<(), Box<dyn std::error::Error>> {
    let test_db = djangors_test::TestDatabase::connect().await?;
    let _guard = TEST_DB_LOCK.lock().await;
    let db = test_db.database();
    create_task_table(db).await?;

    db.conn()
        .execute("DELETE FROM djangors_task_queue", &[])
        .await?;

    TEST_TASK_EXECUTED.store(false, Ordering::SeqCst);
    let task_id = enqueue(
        db,
        "sample_task_fn",
        &SamplePayload {
            message: "hello_tasks".into(),
        },
    )
    .await?;

    let worker = Worker::new(db.clone());
    let claimed = worker.run_once().await?;
    assert!(claimed);

    assert!(TEST_TASK_EXECUTED.load(Ordering::SeqCst));

    // Check task status in DB
    let ph = db.dialect().placeholder(1);
    let sql = format!("SELECT status, attempts FROM djangors_task_queue WHERE id = {ph}");
    let row = db
        .conn()
        .fetch_one(&sql, &[djangors_db::BindValue::I64(task_id)])
        .await?;

    let status = row.try_string_by_name("status")?.unwrap_or_default();
    let attempts = row.try_i64_by_name("attempts")?.unwrap_or_default() as i32;

    assert_eq!(status, "completed");
    assert_eq!(attempts, 1);

    Ok(())
}

#[tokio::test]
async fn test_worker_retry_under_and_at_max_attempts() -> Result<(), Box<dyn std::error::Error>> {
    let test_db = djangors_test::TestDatabase::connect().await?;
    let _guard = TEST_DB_LOCK.lock().await;
    let db = test_db.database();
    create_task_table(db).await?;

    db.conn()
        .execute("DELETE FROM djangors_task_queue", &[])
        .await?;

    let task_id = enqueue(
        db,
        "failing_task_fn",
        &SamplePayload {
            message: "fail".into(),
        },
    )
    .await?;

    let worker = Worker::new(db.clone());

    // 1st attempt: attempts = 0 -> 1. max_attempts = 3. 1 < 3 => status requeued to 'pending'
    let claimed = worker.run_once().await?;
    assert!(claimed);

    let ph = db.dialect().placeholder(1);
    let sql1 =
        format!("SELECT status, attempts, error_message FROM djangors_task_queue WHERE id = {ph}");
    let row1 = db
        .conn()
        .fetch_one(&sql1, &[djangors_db::BindValue::I64(task_id)])
        .await?;
    let status1 = row1.try_string_by_name("status")?.unwrap_or_default();
    let attempts1 = row1.try_i64_by_name("attempts")?.unwrap_or_default() as i32;
    let err1 = row1.try_string_by_name("error_message")?;

    assert_eq!(status1, "pending");
    assert_eq!(attempts1, 1);
    assert!(err1.unwrap().contains("simulated error"));

    // 2nd attempt: attempts = 1 -> 2. 2 < 3 => status requeued to 'pending'
    let claimed2 = worker.run_once().await?;
    assert!(claimed2);

    let sql2 = format!("SELECT status, attempts FROM djangors_task_queue WHERE id = {ph}");
    let row2 = db
        .conn()
        .fetch_one(&sql2, &[djangors_db::BindValue::I64(task_id)])
        .await?;
    let status2 = row2.try_string_by_name("status")?.unwrap_or_default();
    let attempts2 = row2.try_i64_by_name("attempts")?.unwrap_or_default() as i32;
    assert_eq!(status2, "pending");
    assert_eq!(attempts2, 2);

    // 3rd attempt: attempts = 2 -> 3. 3 >= 3 => status set to 'failed' permanently
    let claimed3 = worker.run_once().await?;
    assert!(claimed3);

    let row3 = db
        .conn()
        .fetch_one(&sql2, &[djangors_db::BindValue::I64(task_id)])
        .await?;
    let status3 = row3.try_string_by_name("status")?.unwrap_or_default();
    let attempts3 = row3.try_i64_by_name("attempts")?.unwrap_or_default() as i32;
    assert_eq!(status3, "failed");
    assert_eq!(attempts3, 3);

    // 4th run_once should claim nothing because status is now 'failed'
    let claimed4 = worker.run_once().await?;
    assert!(!claimed4);

    Ok(())
}

#[tokio::test]
async fn test_worker_panic_isolation() -> Result<(), Box<dyn std::error::Error>> {
    let test_db = djangors_test::TestDatabase::connect().await?;
    let _guard = TEST_DB_LOCK.lock().await;
    let db = test_db.database();
    create_task_table(db).await?;

    db.conn()
        .execute("DELETE FROM djangors_task_queue", &[])
        .await?;

    let task_id = enqueue(
        db,
        "panicking_task_fn",
        &SamplePayload {
            message: "panic".into(),
        },
    )
    .await?;

    let worker = Worker::new(db.clone());

    // Worker should NOT panic or crash when running panicking task
    let claimed = worker.run_once().await?;
    assert!(claimed);

    let ph = db.dialect().placeholder(1);
    let sql = format!("SELECT status, error_message FROM djangors_task_queue WHERE id = {ph}");
    let row = db
        .conn()
        .fetch_one(&sql, &[djangors_db::BindValue::I64(task_id)])
        .await?;
    let status = row.try_string_by_name("status")?.unwrap_or_default();
    let err_msg = row.try_string_by_name("error_message")?;

    assert_eq!(status, "pending");
    assert!(err_msg
        .unwrap()
        .contains("Task panicked: simulated task panic!"));

    Ok(())
}

#[tokio::test]
async fn test_register_recurring_rejects_invalid_cron_expression(
) -> Result<(), Box<dyn std::error::Error>> {
    let test_db = djangors_test::TestDatabase::connect().await?;
    let _guard = TEST_DB_LOCK.lock().await;
    let db = test_db.database();
    create_recurring_task_table(db).await?;
    db.conn()
        .execute("DELETE FROM djangors_recurring_task", &[])
        .await?;

    let bad_syntax = register_recurring(
        db,
        "sample_task_fn",
        &SamplePayload {
            message: "x".into(),
        },
        "not a cron expression",
    )
    .await;
    assert!(matches!(bad_syntax, Err(TaskError::Serialization(_))));

    // Wrong field count (4 instead of the required standard 5).
    let wrong_fields = register_recurring(
        db,
        "sample_task_fn",
        &SamplePayload {
            message: "x".into(),
        },
        "* * * *",
    )
    .await;
    assert!(matches!(wrong_fields, Err(TaskError::Serialization(_))));

    Ok(())
}

#[tokio::test]
async fn test_disabled_recurring_task_never_enqueues() -> Result<(), Box<dyn std::error::Error>> {
    let test_db = djangors_test::TestDatabase::connect().await?;
    let _guard = TEST_DB_LOCK.lock().await;
    let db = test_db.database();
    create_recurring_task_table(db).await?;
    db.conn()
        .execute("DELETE FROM djangors_recurring_task", &[])
        .await?;
    create_task_table(db).await?;

    let name = "sample_task_fn_disabled_test";
    let recurring_id = register_recurring(
        db,
        name,
        &SamplePayload {
            message: "disabled".into(),
        },
        "*/5 * * * *",
    )
    .await?;

    // Mark it disabled and force it overdue.
    let p1 = db.dialect().placeholder(1);
    let p2 = db.dialect().placeholder(2);
    let update_sql = format!(
        "UPDATE djangors_recurring_task SET enabled = false, next_run_at = {p1} WHERE id = {p2}"
    );
    db.conn()
        .execute(
            &update_sql,
            &[
                djangors_db::BindValue::DateTime(
                    chrono::Utc::now() - chrono::Duration::minutes(20),
                ),
                djangors_db::BindValue::I64(recurring_id),
            ],
        )
        .await?;

    let enqueued = tick_recurring_tasks(db).await?;
    assert_eq!(
        enqueued, 0,
        "a disabled recurring task must never enqueue, even overdue"
    );

    let sql = format!("SELECT COUNT(*) FROM djangors_task_queue WHERE task_name = {p1}");
    let row = db
        .conn()
        .fetch_one(&sql, &[djangors_db::BindValue::Text(name.to_string())])
        .await?;
    let count = row.try_i64(0)?.unwrap_or_default();
    assert_eq!(count, 0);

    Ok(())
}

#[tokio::test]
async fn test_recurring_task_advances_from_previous_scheduled_time_not_now(
) -> Result<(), Box<dyn std::error::Error>> {
    let test_db = djangors_test::TestDatabase::connect().await?;
    let _guard = TEST_DB_LOCK.lock().await;
    let db = test_db.database();
    create_recurring_task_table(db).await?;
    db.conn()
        .execute("DELETE FROM djangors_recurring_task", &[])
        .await?;
    create_task_table(db).await?;

    let name = "sample_task_fn_advance_test";
    let recurring_id = register_recurring(
        db,
        name,
        &SamplePayload {
            message: "advance".into(),
        },
        "*/5 * * * *",
    )
    .await?;

    // Force it 20 minutes overdue (well past several missed 5-minute occurrences).
    let overdue_since = chrono::Utc::now() - chrono::Duration::minutes(20);
    let p1 = db.dialect().placeholder(1);
    let p2 = db.dialect().placeholder(2);
    let update_sql =
        format!("UPDATE djangors_recurring_task SET next_run_at = {p1} WHERE id = {p2}");
    db.conn()
        .execute(
            &update_sql,
            &[
                djangors_db::BindValue::DateTime(overdue_since),
                djangors_db::BindValue::I64(recurring_id),
            ],
        )
        .await?;

    let enqueued = tick_recurring_tasks(db).await?;
    assert_eq!(enqueued, 1);

    let sel_sql = format!("SELECT next_run_at FROM djangors_recurring_task WHERE id = {p1}");
    let row = db
        .conn()
        .fetch_one(&sel_sql, &[djangors_db::BindValue::I64(recurring_id)])
        .await?;
    let new_next_run_at = row
        .try_datetime_by_name("next_run_at")?
        .ok_or_else(|| djangors_db::DbError::QueryFailed(sqlx::Error::RowNotFound))?;

    assert!(
        new_next_run_at < chrono::Utc::now(),
        "next_run_at must advance from the previous overdue schedule, not from now() \
             (got {new_next_run_at}, which is not in the past as expected)"
    );
    assert!(new_next_run_at > overdue_since);

    Ok(())
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_tick_recurring_tasks_dual_claim_race() -> Result<(), Box<dyn std::error::Error>> {
    let test_db = djangors_test::TestDatabase::connect().await?;
    let _guard = TEST_DB_LOCK.lock().await;
    let db = test_db.database();
    create_recurring_task_table(db).await?;
    db.conn()
        .execute("DELETE FROM djangors_recurring_task", &[])
        .await?;
    create_task_table(db).await?;
    db.conn()
        .execute(
            "DELETE FROM djangors_task_queue WHERE task_name = 'sample_task_fn_race_test'",
            &[],
        )
        .await?;

    let name = "sample_task_fn_race_test";
    let cron_expr = "*/5 * * * *";
    let recurring_id = register_recurring(
        db,
        name,
        &SamplePayload {
            message: "race".into(),
        },
        cron_expr,
    )
    .await?;
    let schedule = parse_schedule(cron_expr)?;
    let now = chrono::Utc::now();
    let mut most_recent_boundary = schedule
        .after(&(now - chrono::Duration::minutes(6)))
        .next()
        .expect("cron has a next occurrence");
    loop {
        let candidate = schedule
            .after(&most_recent_boundary)
            .next()
            .expect("cron has a next occurrence");
        if candidate > now {
            break;
        }
        most_recent_boundary = candidate;
    }

    let p1 = db.dialect().placeholder(1);
    let p2 = db.dialect().placeholder(2);
    let update_sql =
        format!("UPDATE djangors_recurring_task SET next_run_at = {p1} WHERE id = {p2}");
    db.conn()
        .execute(
            &update_sql,
            &[
                djangors_db::BindValue::DateTime(most_recent_boundary),
                djangors_db::BindValue::I64(recurring_id),
            ],
        )
        .await?;

    let db1 = db.clone();
    let db2 = db.clone();
    let handle1 = tokio::spawn(async move { tick_recurring_tasks(&db1).await });
    let handle2 = tokio::spawn(async move { tick_recurring_tasks(&db2).await });
    let (res1, res2) = tokio::join!(handle1, handle2);
    let enqueued1 = res1??;
    let enqueued2 = res2??;

    assert_eq!(
            enqueued1 + enqueued2,
            1,
            "exactly one of the two concurrent ticks must enqueue the single due row, never zero or two"
        );

    let sel_sql = format!("SELECT COUNT(*) FROM djangors_task_queue WHERE task_name = {p1}");
    let row = db
        .conn()
        .fetch_one(&sel_sql, &[djangors_db::BindValue::Text(name.to_string())])
        .await?;
    let count = row.try_i64(0)?.unwrap_or_default();
    assert_eq!(count, 1);

    Ok(())
}

#[tokio::test]
async fn test_recurring_task_end_to_end_enqueue_and_execute(
) -> Result<(), Box<dyn std::error::Error>> {
    let test_db = djangors_test::TestDatabase::connect().await?;
    let _guard = TEST_DB_LOCK.lock().await;
    let db = test_db.database();
    create_recurring_task_table(db).await?;
    db.conn()
        .execute("DELETE FROM djangors_recurring_task", &[])
        .await?;
    create_task_table(db).await?;
    db.conn()
        .execute("DELETE FROM djangors_task_queue", &[])
        .await?;

    TEST_TASK_EXECUTED.store(false, Ordering::SeqCst);

    let recurring_id = register_recurring(
        db,
        "sample_task_fn",
        &SamplePayload {
            message: "hello_tasks".into(),
        },
        "*/5 * * * *",
    )
    .await?;

    let p1 = db.dialect().placeholder(1);
    let p2 = db.dialect().placeholder(2);
    let update_sql =
        format!("UPDATE djangors_recurring_task SET next_run_at = {p1} WHERE id = {p2}");
    db.conn()
        .execute(
            &update_sql,
            &[
                djangors_db::BindValue::DateTime(chrono::Utc::now() - chrono::Duration::minutes(1)),
                djangors_db::BindValue::I64(recurring_id),
            ],
        )
        .await?;

    let enqueued = tick_recurring_tasks(db).await?;
    assert_eq!(enqueued, 1);

    let sel_sql = format!("SELECT COUNT(*) FROM djangors_task_queue WHERE task_name = {p1}");
    let row = db
        .conn()
        .fetch_one(
            &sel_sql,
            &[djangors_db::BindValue::Text("sample_task_fn".to_string())],
        )
        .await?;
    let queued_count = row.try_i64(0)?.unwrap_or_default();
    assert_eq!(queued_count, 1);

    let worker = Worker::new(db.clone());
    let claimed = worker.run_once().await?;
    assert!(claimed);
    assert!(
        TEST_TASK_EXECUTED.load(Ordering::SeqCst),
        "the task enqueued by tick_recurring_tasks must actually execute via the normal worker"
    );

    Ok(())
}

#[tokio::test]
async fn transactional_outbox_rolls_back_and_commits_atomically(
) -> Result<(), Box<dyn std::error::Error>> {
    let config = djangors_db::DatabaseConfig::new("sqlite::memory:")
        .max_connections(1)
        .min_connections(1);
    let db = djangors_db::Database::connect(&config).await?;
    create_task_table(&db).await?;
    db.conn()
        .execute(
            "CREATE TABLE outbox_orders (id INTEGER PRIMARY KEY, item TEXT NOT NULL)",
            &[],
        )
        .await?;

    // 1. Transaction returns an error -> neither order nor task exists.
    let rollback_res: Result<(), djangors_db::DbError> = db
        .transaction_conn(|conn| {
            Box::pin(async move {
                let p1 = conn.dialect().placeholder(1);
                let sql = format!("INSERT INTO outbox_orders (id, item) VALUES (1, {p1})");
                conn.execute(&sql, &[djangors_db::BindValue::Text("book".to_string())])
                    .await
                    .map_err(djangors_db::DbError::QueryFailed)?;
                enqueue_in(conn, "sample_task_fn", &serde_json::json!({"order": 1}))
                    .await
                    .map_err(|e| djangors_db::DbError::TransactionFailed(e.to_string()))?;
                Err(djangors_db::DbError::TransactionFailed("abort".to_string()))
            })
        })
        .await;
    assert!(rollback_res.is_err());

    let order_count = db
        .conn()
        .fetch_one("SELECT COUNT(*) FROM outbox_orders", &[])
        .await?
        .try_i64(0)?
        .unwrap_or_default();
    assert_eq!(order_count, 0);
    assert!(claim_next_task(&db).await?.is_none());

    // 2. Transaction succeeds -> both order and task exist, and claim_next_task picks it up.
    let task_id = db
        .transaction_conn(|conn| {
            Box::pin(async move {
                let p1 = conn.dialect().placeholder(1);
                let sql = format!("INSERT INTO outbox_orders (id, item) VALUES (2, {p1})");
                conn.execute(&sql, &[djangors_db::BindValue::Text("pen".to_string())])
                    .await
                    .map_err(djangors_db::DbError::QueryFailed)?;
                let id = enqueue_in(conn, "sample_task_fn", &serde_json::json!({"order": 2}))
                    .await
                    .map_err(|e| djangors_db::DbError::TransactionFailed(e.to_string()))?;
                Ok::<_, djangors_db::DbError>(id)
            })
        })
        .await?;

    let order_count_after = db
        .conn()
        .fetch_one("SELECT COUNT(*) FROM outbox_orders", &[])
        .await?
        .try_i64(0)?
        .unwrap_or_default();
    assert_eq!(order_count_after, 1);

    let claimed = claim_next_task(&db).await?.expect("task should be claimed");
    assert_eq!(claimed.id, task_id);
    assert_eq!(claimed.task_name, "sample_task_fn");

    Ok(())
}
