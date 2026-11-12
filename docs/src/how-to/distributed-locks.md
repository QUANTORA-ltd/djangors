# How to Use Distributed Locks

Distributed locks ensure that only one worker or request handler mutates a resource or runs a critical job at a time.

## Database advisory locks (`djangors-db`)

`with_lock` (non-blocking) and `with_lock_wait` (blocking) run a closure inside a database transaction while holding a lock for a string key:

```rust,compile
# use djangors_db::{with_lock, Database, DbError};
# async fn example(db: &Database) -> Result<(), DbError> {
let result = with_lock(db, "billing:invoice:42", |_conn| {
    Box::pin(async move {
        // Perform exclusive work inside the transaction.
        Ok::<_, DbError>("charged")
    })
})
.await?;

match result {
    Some(value) => println!("acquired lock: {value}"),
    None => println!("another instance already holds the lock"),
}
# Ok(())
# }
```

- **PostgreSQL**: hashes the key (first 8 bytes of SHA-256 as a big-endian `i64`) and calls `pg_try_advisory_xact_lock` (or `pg_advisory_xact_lock` for `with_lock_wait`) inside `transaction_conn`. The lock is released automatically when the transaction commits or rolls back, so it never leaks onto a pooled connection.
- **SQLite**: uses a process-wide per-key `tokio::sync::Mutex` around `transaction_conn`. This covers all concurrent tasks in the single-process SQLite deployment.

## Redis locks (`djangors-cache`, `redis` feature)

When your lock does not span a database transaction—or when you use SQLite across multiple processes or want an explicit TTL—use `RedisLock`:

```rust,illustrative
use std::time::Duration;
use djangors_cache::RedisLock;

async fn run_exclusive(client: &redis::Client) -> Result<(), djangors_cache::CacheError> {
    let Some(guard) = RedisLock::acquire(client, "lock:nightly-export", Duration::from_secs(60)).await? else {
        return Ok(());
    };

    // Do the work...
    let _released = guard.release().await?;
    Ok(())
}
```

`RedisLock::acquire` runs `SET key token NX PX ttl` with a random token, and `guard.release()` runs a Lua script that deletes the key only if its value still matches that token. Always choose a `ttl` that **exceeds** the maximum expected work time; otherwise the lock can expire before the work finishes and another instance can acquire it.
