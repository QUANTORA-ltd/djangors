# How to Send Events Reliably

When your application writes a domain record (such as an order) and enqueues a background task (such as sending a confirmation email), doing those in two separate operations can leave them out of sync if the process crashes or the database rolls back.

The **transactional outbox** pattern solves this by inserting the `QueuedTask` row inside the **same** database transaction as the domain write. If the transaction rolls back, neither the domain change nor the task is persisted; if it commits, the worker is guaranteed to see the task.

## Enqueuing inside `transaction_conn`

Use `enqueue_in` (or `enqueue_scheduled_in`) with the `&mut Conn<'_>` provided by `Database::transaction_conn`:

```rust,compile
# use djangors_db::{BindValue, Database, DbError, DbExecutor};
# use djangors_tasks::enqueue_in;
# async fn place_order(db: &Database, order_id: i64, email: &str) -> Result<i64, DbError> {
let email = email.to_string();
let task_id = db
    .transaction_conn(|conn| {
        Box::pin(async move {
            let p1 = conn.dialect().placeholder(1);
            let sql = format!("INSERT INTO orders (id) VALUES ({p1})");
            conn.execute(&sql, &[BindValue::I64(order_id)])
                .await
                .map_err(DbError::QueryFailed)?;

            let id = enqueue_in(
                conn,
                "send_order_confirmation",
                &serde_json::json!({ "order_id": order_id, "email": email }),
            )
            .await
            .map_err(|e| DbError::TransactionFailed(e.to_string()))?;

            Ok::<_, DbError>(id)
        })
    })
    .await?;

Ok(task_id)
# }
```

## Idempotency and at-least-once delivery

Because a worker can crash after executing a task's side effect (such as sending an email or calling a webhook) but before marking the row `"completed"`, background task delivery is **at-least-once**. Design your task handlers to be **idempotent**—for example, by checking an idempotency key or recording a "sent" flag before repeating external side effects.
