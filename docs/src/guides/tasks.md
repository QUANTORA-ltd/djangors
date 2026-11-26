# Background Tasks

Djangors features a robust, database-backed background task execution system. Tasks are defined using a macro, stored in the database, claimed by one or more worker processes, and executed asynchronously.

---

## Macro Registration

Background tasks are registered at compile-time using the `#[task]` attribute macro from `djangors_tasks` (re-exported by `djangors`).

```rust,illustrative
use djangors::tasks::{task, TaskError};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct SendEmailPayload {
    pub recipient: String,
    pub body: String,
}

#[task]
pub async fn send_email(payload: SendEmailPayload) -> Result<(), TaskError> {
    // Send email logic...
    Ok(())
}
```

### Under the Hood: Compile-Time Discovery
The `#[task]` macro automatically does the following behind the scenes:
1. Generates a unique task name (defaults to the function name, e.g. `"send_email"`). You can override this using `#[task(name = "custom_name")]`.
2. Generates code that serializes/deserializes your custom payload structure to and from a `serde_json::Value`.
3. Registers the task handler function globally using the `inventory` crate. The worker discovers all registered `TaskRegistration` structs at startup without manual wiring.

---

## Task Queue Table Structure

Pending tasks are stored in the `djangors_task_queue` database table, mapped to the `QueuedTask` model.

The schema comprises:
* **`id`**: Unique identifier (e.g. `BIGSERIAL` / `INTEGER AUTOINCREMENT`).
* **`task_name`**: Name of the registered task handler (e.g. `"send_email"`).
* **`payload`**: JSON string representing the task's input parameters.
* **`status`**: Current state of the task: `"pending"`, `"running"`, `"completed"`, or `"failed"`.
* **`attempts`**: Count of execution attempts.
* **`max_attempts`**: Maximum allowed execution attempts before marking it permanently `"failed"` (defaults to 3).
* **`created_at`**: Timestamp when the task was enqueued.
* **`scheduled_at`**: Earliest timestamp when the task is eligible to run.
* **`error_message`**: Last recorded error or panic trace if the task failed.

### Enqueuing Tasks
To queue a task, use `enqueue` or `enqueue_scheduled`:

```rust,illustrative
// Immediate execution
let task_id = djangors::tasks::enqueue(db, "send_email", &payload).await?;

// Delayed execution
let scheduled_at = chrono::Utc::now() + chrono::Duration::hours(2);
let task_id = djangors::tasks::enqueue_scheduled(db, "send_email", &payload, scheduled_at).await?;
```

---

## The Worker Claiming Strategy

Workers run in a continuous loop to fetch, lock, and execute pending tasks.

```rust,illustrative
use djangors::tasks::Worker;
use std::time::Duration;

let worker = Worker::new(db)
    .with_poll_interval(Duration::from_secs(1))
    .with_recurring_tick_interval(Duration::from_secs(10));
worker.run().await;
```

To run a worker from the command line, use:
```bash
dj runworker
```

### Claiming Concurrency (`SKIP LOCKED`)
To support running multiple parallel worker processes safely without double-executing tasks, Djangors uses database row-level locking during the claiming phase:

* **PostgreSQL**: Claims the next pending task using a `SELECT ... FOR UPDATE SKIP LOCKED LIMIT 1` query inside a transaction. This allows workers to lock a row and skip past any rows already locked by other workers, ensuring high-throughput parallel execution.
* **SQLite**: Since SQLite only supports database-level write locks (single writer), concurrency is naturally serialized. The worker issues a standard query and updates the row, relying on SQLite's transactional lock behavior to serialize worker claims.

### Manual Polling: `run_once` and `claim_next_task`

`Worker::run()` loops forever, but the underlying single-cycle primitives are public so you can
drive the queue yourself (e.g. draining tasks under a fixed budget, or embedding claims in another
loop).

```rust,illustrative
use djangors_tasks::{Worker, TaskError, claim_next_task};

// Claim one due task atomically, without touching the registry or executing it.
match claim_next_task(&db).await {
    Ok(Some(task)) => { /* already marked "running", attempts incremented */ }
    Ok(None) => { /* queue empty */ }
    Err(e) => { /* TaskError */ }
}

// Drive a single poll cycle of the worker loop:
//   Ok(true)  => a task was claimed and executed (or had no handler)
//   Ok(false) => nothing was due; callers usually sleep the poll interval now
//   Err(e)    => the poll failed (returned via `?` as TaskError)
let worker = Worker::new(db);
match worker.run_once().await? {
    true => println!("ran a task"),
    false => println!("queue idle"),
}
```

| Method | Signature | Notes |
| --- | --- | --- |
| `Worker::run_once` | `async fn(&self) -> Result<bool, TaskError>` | One poll cycle: claim + run a single due task; returns whether one ran |
| `claim_next_task` | `async fn(&Database) -> Result<Option<QueuedTask>, TaskError>` | Atomic claim inside one transaction; Postgres uses `FOR UPDATE SKIP LOCKED`, SQLite a guarded update |

`claim_next_task` already flips the row to `"running"` and increments `attempts` as part of the
claim, so a crash between claim and execution leaves the task parked in `"running"` rather than
double-claiming it.

### `TaskError` Variants

Every task operation bubbles up through this one error type:

| Variant | Meaning |
| --- | --- |
| `Db(DbError)` | A database operation via `djangors_db` failed |
| `Orm(OrmError)` | An ORM operation failed |
| `Json(serde_json::Error)` | Generic JSON serialization/deserialization error |
| `PayloadDeserialization(String)` | Task payload could not be deserialized into the handler's type |
| `Serialization(String)` | Serializing the payload (or compiling a cron expression) failed |
| `TaskNotFound(String)` | No `#[task]` handler with that name is registered in the inventory |
| `TaskExecution(String)` | The handler executed but returned `Err` |
| `TaskPanicked(String)` | The handler panicked inside the worker's `tokio::spawn` isolation |

---

## Error Handling, Retries, and Status Transitions

When a worker executes a task:
1. The task status is updated to `"running"`, and `attempts` is incremented by 1.
2. The handler function is executed inside a `tokio::spawn` wrapper to isolate panics.
3. **Success**: If the handler returns `Ok(())`, the status changes to `"completed"`.
4. **Failure/Panic**: If the handler returns an `Err(TaskError)` or panics:
   * The exception is captured and written to `error_message`.
   * If `attempts < max_attempts`, the status reverts to `"pending"` (scheduled for a retry on the next worker pass).
   * If `attempts >= max_attempts`, the status is marked as `"failed"`.

---

## Recurring (Cron) Tasks

Djangors supports recurring tasks configured with standard 5-field cron expressions. These are managed via the `djangors_recurring_task` database table and mapped to the `RecurringTask` model.

### Registering Recurring Tasks
```rust,illustrative
use djangors::tasks::register_recurring;

// Trigger database cleanup every night at midnight
let schedule_id = register_recurring(
    db,
    "database_cleanup",
    &EmptyPayload {},
    "0 0 * * *"
).await?;
```

### The Ticking Mechanism
The recurring task dispatcher tracks the `next_run_at` timestamp.
1. When configuring a `Worker` with `.with_recurring_tick_interval()`, the worker periodically calls `tick_recurring_tasks`.
2. It claims due recurring tasks using a transactional advisory lock (`pg_advisory_xact_lock` on Postgres) to prevent concurrent ticks.
3. For each due task, it enqueues a new execution instance in `djangors_task_queue` and calculates the `next_run_at` using the cron schedule.

---

## Admin Site Integration

Djangors provides built-in admin configurations to monitor and manage your task queue from the admin web panel.

To register the task queue dashboard, call the `register_admin` helper:

```rust,illustrative
use djangors::tasks::register_admin;

let site = djangors_admin::AdminSite::new("Admin Portal");
register_admin(&site);
```

This registers the `QueuedTask` model under the `djangors_tasks` app label. Admin users can view the task list, search for tasks by `task_name` or `status`, monitor execution attempts, and inspect `error_message` text details for failed background tasks.

### Live Queue Dashboard

Mount `djangors::tasks::queue_dashboard_router(db)` (or `register_dashboard(site, db)`) for a live `/tasks/dashboard/` UI with queue counters, recent failures, 1-click **Retry** (`POST /tasks/dashboard/retry/{id}`), **Purge Completed**, and `/tasks/dashboard/stats.json`.
