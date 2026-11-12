use djangors_db::{Database, DatabaseConfig};
use djangors_macros::Model;

use crate::{ensure_history_table, history_for, track};

#[derive(Model, Debug, Clone)]
#[djangors(app = "history_test", table_name = "history_test_note")]
pub struct Note {
    #[djangors(primary_key, auto)]
    pub id: i64,
    #[djangors(max_length = 255)]
    pub body: String,
}

#[derive(Model, Debug, Clone)]
#[djangors(app = "history_test", table_name = "history_test_ticket")]
pub struct Ticket {
    #[djangors(primary_key, auto)]
    pub id: i64,
    #[djangors(max_length = 255)]
    pub title: String,
}

#[derive(Model, Debug, Clone)]
#[djangors(app = "history_test", table_name = "history_test_quiet")]
pub struct Quiet {
    #[djangors(primary_key, auto)]
    pub id: i64,
    #[djangors(max_length = 255)]
    pub label: String,
}

async fn memory_db() -> Database {
    let config = DatabaseConfig::new("sqlite::memory:")
        .max_connections(1)
        .min_connections(1);
    Database::connect(&config).await.unwrap()
}

async fn create_table(db: &Database, sql: &str) {
    let mut conn = db.conn();
    conn.execute(sql, &[]).await.unwrap();
}

async fn create_note_table(db: &Database) {
    create_table(
        db,
        "CREATE TABLE history_test_note (id INTEGER PRIMARY KEY AUTOINCREMENT, body TEXT NOT NULL)",
    )
    .await;
}

async fn create_ticket_table(db: &Database) {
    create_table(
        db,
        "CREATE TABLE history_test_ticket (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL)",
    )
    .await;
}

#[tokio::test]
async fn save_update_and_delete_are_recorded() {
    let db = memory_db().await;
    ensure_history_table(&db).await.unwrap();
    create_note_table(&db).await;
    track::<Note>(
        db.clone(),
        Note::post_save_signal(),
        Note::post_delete_signal(),
    );

    let saved = Note {
        id: 0,
        body: "first".to_string(),
    }
    .save(&db)
    .await
    .unwrap();

    let mut edited = saved.clone();
    edited.body = "second".to_string();
    edited.update(&db).await.unwrap();

    edited.delete(&db).await.unwrap();

    let history = history_for::<Note>(&db, saved.id).await.unwrap();
    assert_eq!(history.len(), 3);

    assert_eq!(history[0].action, "saved");
    assert_eq!(history[0].data["body"], "first");
    assert_eq!(history[0].data["id"], saved.id);

    assert_eq!(history[1].action, "saved");
    assert_eq!(history[1].data["body"], "second");

    assert_eq!(history[2].action, "deleted");
    assert_eq!(history[2].data["body"], "second");
}

#[tokio::test]
async fn history_is_kept_per_object() {
    let db = memory_db().await;
    ensure_history_table(&db).await.unwrap();
    create_ticket_table(&db).await;
    track::<Ticket>(
        db.clone(),
        Ticket::post_save_signal(),
        Ticket::post_delete_signal(),
    );

    let first = Ticket {
        id: 0,
        title: "login broken".to_string(),
    }
    .save(&db)
    .await
    .unwrap();
    let second = Ticket {
        id: 0,
        title: "slow page".to_string(),
    }
    .save(&db)
    .await
    .unwrap();

    let mut renamed = first.clone();
    renamed.title = "login fixed".to_string();
    renamed.update(&db).await.unwrap();

    let first_history = history_for::<Ticket>(&db, first.id).await.unwrap();
    assert_eq!(first_history.len(), 2);
    assert_eq!(first_history[0].data["title"], "login broken");
    assert_eq!(first_history[1].data["title"], "login fixed");

    let second_history = history_for::<Ticket>(&db, second.id).await.unwrap();
    assert_eq!(second_history.len(), 1);
    assert_eq!(second_history[0].data["title"], "slow page");
}

#[tokio::test]
async fn history_for_unknown_key_is_empty() {
    let db = memory_db().await;
    ensure_history_table(&db).await.unwrap();

    let history = history_for::<Note>(&db, 9999).await.unwrap();
    assert!(history.is_empty());
}

#[tokio::test]
async fn failed_history_insert_does_not_fail_the_save() {
    let db = memory_db().await;
    // The history table is deliberately not created, so every history insert fails. The
    // model operation must still succeed; the failure is only logged.
    create_table(
        &db,
        "CREATE TABLE history_test_quiet (id INTEGER PRIMARY KEY AUTOINCREMENT, label TEXT NOT NULL)",
    )
    .await;
    track::<Quiet>(
        db.clone(),
        Quiet::post_save_signal(),
        Quiet::post_delete_signal(),
    );

    let saved = Quiet {
        id: 0,
        label: "still saved".to_string(),
    }
    .save(&db)
    .await
    .unwrap();
    assert!(saved.id > 0);
}

#[derive(Model, Debug, Clone)]
#[djangors(app = "history_test", table_name = "history_test_secret")]
pub struct Secret {
    #[djangors(primary_key, auto)]
    pub id: i64,
    #[djangors(encrypted)]
    pub token: String,
    #[djangors(max_length = 255)]
    pub label: String,
}

#[tokio::test]
async fn encrypted_fields_are_stored_encrypted_and_read_back_decrypted() {
    djangors_orm::encryption::set_keys_for_tests([5; 32], vec![]);
    let db = memory_db().await;
    ensure_history_table(&db).await.unwrap();
    create_table(
        &db,
        "CREATE TABLE history_test_secret \
         (id INTEGER PRIMARY KEY AUTOINCREMENT, token TEXT NOT NULL, label TEXT NOT NULL)",
    )
    .await;
    track::<Secret>(
        db.clone(),
        Secret::post_save_signal(),
        Secret::post_delete_signal(),
    );

    let saved = Secret {
        id: 0,
        token: "sk-live-123".to_string(),
        label: "stripe".to_string(),
    }
    .save(&db)
    .await
    .unwrap();

    // The history table holds no plaintext.
    let mut conn = db.conn();
    let row = conn
        .fetch_one("SELECT data FROM djangors_model_history", &[])
        .await
        .unwrap();
    let raw = row.try_string(0).unwrap().unwrap();
    assert!(!raw.contains("sk-live-123"), "{raw}");
    assert!(raw.contains("stripe"));

    let entries = history_for::<Secret>(&db, saved.id).await.unwrap();
    assert_eq!(entries[0].data["token"], "sk-live-123");
    assert_eq!(entries[0].data["label"], "stripe");
}
