use super::*;
use crate::config::DatabaseConfig;

#[tokio::test]
async fn sqlite_with_lock_contends_on_same_key_and_allows_different_keys() {
    // The lock holder keeps one connection busy, so the "different key" call needs a second
    // connection to the same database. In-memory SQLite gives each connection its own
    // database, so use a temporary file.
    let path = std::env::temp_dir().join(format!("djangors_locks_{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let config = DatabaseConfig::new(format!("sqlite://{}?mode=rwc", path.display()))
        .max_connections(2)
        .min_connections(1);
    let db = Database::connect(&config).await.unwrap();

    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel::<()>();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();

    let db1 = db.clone();
    let holder = tokio::spawn(async move {
        with_lock(&db1, "sqlite-lock-test-same", |_conn| {
            Box::pin(async move {
                let _ = entered_tx.send(());
                let _ = release_rx.await;
                Ok::<_, DbError>(42)
            })
        })
        .await
    });

    entered_rx.await.unwrap();

    // Contending on the same key returns Ok(None) immediately.
    let contended = with_lock(&db, "sqlite-lock-test-same", |_conn| {
        Box::pin(async move { Ok::<_, DbError>(99) })
    })
    .await
    .unwrap();
    assert_eq!(contended, None);

    // Different key does not block and succeeds.
    let other = with_lock(&db, "sqlite-lock-test-other", |_conn| {
        Box::pin(async move { Ok::<_, DbError>(7) })
    })
    .await
    .unwrap();
    assert_eq!(other, Some(7));

    let _ = release_tx.send(());
    let held_res = holder.await.unwrap().unwrap();
    assert_eq!(held_res, Some(42));
    drop(db);
    let _ = std::fs::remove_file(&path);
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_with_lock_contends_on_same_key_and_allows_different_keys() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        return;
    };
    let config = DatabaseConfig::new(url);
    let db = Database::connect(&config).await.unwrap();

    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel::<()>();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();

    let db1 = db.clone();
    let holder = tokio::spawn(async move {
        with_lock(&db1, "pg-lock-test-same", |_conn| {
            Box::pin(async move {
                let _ = entered_tx.send(());
                let _ = release_rx.await;
                Ok::<_, DbError>(42)
            })
        })
        .await
    });

    entered_rx.await.unwrap();

    let contended = with_lock(&db, "pg-lock-test-same", |_conn| {
        Box::pin(async move { Ok::<_, DbError>(99) })
    })
    .await
    .unwrap();
    assert_eq!(contended, None);

    let other = with_lock(&db, "pg-lock-test-other", |_conn| {
        Box::pin(async move { Ok::<_, DbError>(7) })
    })
    .await
    .unwrap();
    assert_eq!(other, Some(7));

    let _ = release_tx.send(());
    let held_res = holder.await.unwrap().unwrap();
    assert_eq!(held_res, Some(42));
}
