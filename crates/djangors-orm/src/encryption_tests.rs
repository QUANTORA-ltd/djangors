use super::{decrypt_str, encrypt_str, encrypt_value, parse_key, seal, set_keys_for_tests, PREFIX};
use crate::error::OrmError;
use crate::expr::{SetExpr, Value};
use crate::meta::Model;
use crate::q;
use crate::queryset::QuerySet;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use djangors_db::{BindValue, Database, DatabaseConfig};
use djangors_macros::Model;

const CURRENT_KEY: [u8; 32] = [7; 32];
const OLD_KEY: [u8; 32] = [9; 32];
const WRONG_KEY: [u8; 32] = [3; 32];

#[derive(Model, Debug, Clone)]
#[djangors(app = "test_app", table_name = "encrypted_test_model")]
#[allow(dead_code)]
pub struct EncryptedNote {
    #[djangors(primary_key, auto)]
    pub id: i64,

    #[djangors(encrypted)]
    pub secret: String,

    #[djangors(encrypted)]
    pub optional_secret: Option<String>,

    pub title: String,
}

fn init_keys() {
    set_keys_for_tests(CURRENT_KEY, vec![OLD_KEY]);
}

async fn connect_memory() -> Database {
    let config = DatabaseConfig::new("sqlite::memory:")
        .max_connections(1)
        .min_connections(1);
    let db = Database::connect(&config).await.unwrap();
    db.conn()
        .execute(
            "CREATE TABLE encrypted_test_model (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                secret TEXT NOT NULL,
                optional_secret TEXT,
                title TEXT NOT NULL
            )",
            &[],
        )
        .await
        .unwrap();
    db
}

fn note(secret: &str) -> EncryptedNote {
    EncryptedNote {
        id: 0,
        secret: secret.to_string(),
        optional_secret: None,
        title: "title".to_string(),
    }
}

/// Reads one column of a row exactly as it is stored, bypassing `from_row`.
async fn raw_column(db: &Database, column: &str, id: i64) -> Option<String> {
    let sql = format!(
        "SELECT {column} FROM encrypted_test_model WHERE id = {}",
        db.dialect().placeholder(1)
    );
    let row = db
        .conn()
        .fetch_one(&sql, &[BindValue::I64(id)])
        .await
        .unwrap();
    row.try_string(0).unwrap()
}

/// Inserts a row whose `secret` column holds `stored` verbatim.
async fn insert_raw_secret(db: &Database, stored: &str) {
    let sql =
        format!("INSERT INTO encrypted_test_model (secret, title) VALUES ('{stored}', 'raw')");
    db.conn().execute(&sql, &[]).await.unwrap();
}

fn ciphertext_under(key: &[u8; 32], plain: &str) -> String {
    let blob = seal(key, plain.as_bytes()).unwrap();
    format!("{PREFIX}{}", STANDARD.encode(blob))
}

#[test]
fn encrypt_then_decrypt_round_trip() {
    init_keys();
    let stored = encrypt_str("hunter2").unwrap();
    assert!(stored.starts_with(PREFIX));
    assert!(!stored.contains("hunter2"));
    assert_eq!(decrypt_str(&stored).unwrap(), "hunter2");
}

#[test]
fn encrypting_same_text_twice_gives_different_ciphertext() {
    init_keys();
    let first = encrypt_str("same").unwrap();
    let second = encrypt_str("same").unwrap();
    assert_ne!(first, second);
    assert_eq!(decrypt_str(&first).unwrap(), "same");
    assert_eq!(decrypt_str(&second).unwrap(), "same");
}

#[test]
fn decrypt_accepts_old_key_for_rotation() {
    init_keys();
    let stored = ciphertext_under(&OLD_KEY, "rotated");
    assert_eq!(decrypt_str(&stored).unwrap(), "rotated");
}

#[test]
fn decrypt_with_unknown_key_is_an_error() {
    init_keys();
    let stored = ciphertext_under(&WRONG_KEY, "nope");
    let result = decrypt_str(&stored);
    assert!(matches!(result, Err(OrmError::InvalidQuery(_))));
}

#[test]
fn decrypt_returns_legacy_plaintext_unchanged() {
    init_keys();
    assert_eq!(
        decrypt_str("plain legacy value").unwrap(),
        "plain legacy value"
    );
}

#[test]
fn encrypt_value_passes_null_and_rejects_non_text() {
    init_keys();
    assert!(matches!(encrypt_value(Value::Null), Ok(Value::Null)));
    assert!(matches!(
        encrypt_value(Value::I64(1)),
        Err(OrmError::InvalidQuery(_))
    ));
}

#[test]
fn parse_key_requires_base64_of_32_bytes() {
    assert!(parse_key("not base64 !!").is_err());
    let short = STANDARD.encode([1u8; 16]);
    let message = parse_key(&short).unwrap_err();
    assert!(message.contains("32 bytes"));
    assert!(!message.contains(&short));
    assert!(parse_key(&STANDARD.encode([1u8; 32])).is_ok());
}

#[test]
fn encrypted_fields_lists_marked_fields() {
    assert_eq!(
        EncryptedNote::encrypted_fields(),
        &["secret", "optional_secret"]
    );
}

#[test]
fn field_values_stay_plaintext() {
    init_keys();
    let n = note("visible to admin");
    let values = Model::field_values(&n);
    let secret = values.iter().find(|(name, _)| *name == "secret").unwrap();
    assert!(matches!(&secret.1, Value::Text(t) if t == "visible to admin"));
}

#[tokio::test]
async fn encrypted_field_round_trips_and_is_stored_encrypted() {
    init_keys();
    let db = connect_memory().await;
    let saved = note("hunter2").save(&db).await.unwrap();

    let raw = raw_column(&db, "secret", saved.id).await.unwrap();
    assert!(raw.starts_with(PREFIX));
    assert!(!raw.contains("hunter2"));

    let fetched = EncryptedNote::objects()
        .filter(q!(id = saved.id))
        .unwrap()
        .get(&db)
        .await
        .unwrap();
    assert_eq!(fetched.secret, "hunter2");
}

#[tokio::test]
async fn encrypted_optional_field_handles_none_and_some() {
    init_keys();
    let db = connect_memory().await;

    let empty = note("a").save(&db).await.unwrap();
    assert_eq!(raw_column(&db, "optional_secret", empty.id).await, None);
    let fetched_empty = EncryptedNote::objects()
        .filter(q!(id = empty.id))
        .unwrap()
        .get(&db)
        .await
        .unwrap();
    assert_eq!(fetched_empty.optional_secret, None);

    let mut with_value = note("b");
    with_value.optional_secret = Some("maybe".to_string());
    let saved = with_value.save(&db).await.unwrap();
    let raw = raw_column(&db, "optional_secret", saved.id).await.unwrap();
    assert!(raw.starts_with(PREFIX));
    assert!(!raw.contains("maybe"));
    let fetched = EncryptedNote::objects()
        .filter(q!(id = saved.id))
        .unwrap()
        .get(&db)
        .await
        .unwrap();
    assert_eq!(fetched.optional_secret, Some("maybe".to_string()));
}

#[tokio::test]
async fn same_value_saved_twice_gives_different_ciphertext() {
    init_keys();
    let db = connect_memory().await;
    let first = note("same").save(&db).await.unwrap();
    let second = note("same").save(&db).await.unwrap();
    let raw_first = raw_column(&db, "secret", first.id).await;
    let raw_second = raw_column(&db, "secret", second.id).await;
    assert_ne!(raw_first, raw_second);
}

#[tokio::test]
async fn old_key_value_is_readable_after_rotation() {
    init_keys();
    let db = connect_memory().await;
    insert_raw_secret(&db, &ciphertext_under(&OLD_KEY, "from old key")).await;
    let rows = EncryptedNote::objects().all(&db).await.unwrap();
    assert_eq!(rows[0].secret, "from old key");
}

#[tokio::test]
async fn value_under_unknown_key_is_an_error_on_load() {
    init_keys();
    let db = connect_memory().await;
    insert_raw_secret(&db, &ciphertext_under(&WRONG_KEY, "nope")).await;
    let result = EncryptedNote::objects().all(&db).await;
    assert!(matches!(result, Err(OrmError::InvalidQuery(_))));
}

#[tokio::test]
async fn legacy_plaintext_column_is_returned_as_is() {
    init_keys();
    let db = connect_memory().await;
    insert_raw_secret(&db, "legacy value").await;
    let rows = EncryptedNote::objects().all(&db).await.unwrap();
    assert_eq!(rows[0].secret, "legacy value");
}

#[tokio::test]
async fn filtering_on_encrypted_field_is_rejected() {
    init_keys();
    let filtered = EncryptedNote::objects().filter(q!(secret = "x"));
    assert!(matches!(
        filtered,
        Err(OrmError::InvalidQuery(ref msg)) if msg.contains("secret")
    ));

    let excluded = EncryptedNote::objects().exclude(q!(secret = "x"));
    assert!(matches!(excluded, Err(OrmError::InvalidQuery(_))));

    let ordered = EncryptedNote::objects().order_by("secret");
    assert!(matches!(ordered, Err(OrmError::InvalidQuery(_))));

    let icontains = EncryptedNote::objects().filter_or_icontains(&["secret"], "x");
    assert!(matches!(icontains, Err(OrmError::InvalidQuery(_))));
}

#[tokio::test]
async fn bulk_create_encrypts_marked_fields() {
    init_keys();
    let db = connect_memory().await;
    let pks = QuerySet::<EncryptedNote>::bulk_create(&db, &[note("one"), note("two")])
        .await
        .unwrap();
    assert_eq!(pks.len(), 2);

    for pk in &pks {
        let raw = raw_column(&db, "secret", *pk).await.unwrap();
        assert!(raw.starts_with(PREFIX));
    }

    let rows = EncryptedNote::objects()
        .order_by("id")
        .unwrap()
        .all(&db)
        .await
        .unwrap();
    assert_eq!(rows[0].secret, "one");
    assert_eq!(rows[1].secret, "two");
}

#[tokio::test]
async fn update_encrypts_new_value() {
    init_keys();
    let db = connect_memory().await;
    let mut saved = note("before").save(&db).await.unwrap();
    saved.secret = "after".to_string();
    saved.update(&db).await.unwrap();

    let raw = raw_column(&db, "secret", saved.id).await.unwrap();
    assert!(raw.starts_with(PREFIX));
    assert!(!raw.contains("after"));

    let fetched = EncryptedNote::objects()
        .filter(q!(id = saved.id))
        .unwrap()
        .get(&db)
        .await
        .unwrap();
    assert_eq!(fetched.secret, "after");
}

#[tokio::test]
async fn queryset_update_encrypts_literal_value() {
    init_keys();
    let db = connect_memory().await;
    let saved = note("before").save(&db).await.unwrap();

    QuerySet::<EncryptedNote>::new()
        .update(
            &db,
            vec![(
                "secret",
                SetExpr::Literal(Value::Text("via queryset".to_string())),
            )],
        )
        .await
        .unwrap();

    let raw = raw_column(&db, "secret", saved.id).await.unwrap();
    assert!(raw.starts_with(PREFIX));
    assert!(!raw.contains("via queryset"));

    let fetched = EncryptedNote::objects()
        .filter(q!(id = saved.id))
        .unwrap()
        .get(&db)
        .await
        .unwrap();
    assert_eq!(fetched.secret, "via queryset");
}

#[tokio::test]
async fn values_and_values_list_return_decrypted_text() {
    init_keys();
    let db = connect_memory().await;
    let mut with_optional = note("alpha");
    with_optional.optional_secret = Some("beta".to_string());
    with_optional.save(&db).await.unwrap();

    let rows = EncryptedNote::objects()
        .values(&db, &["secret", "optional_secret", "title"])
        .await
        .unwrap();
    assert_eq!(rows[0][0], ("secret", Value::Text("alpha".to_string())));
    assert_eq!(
        rows[0][1],
        ("optional_secret", Value::Text("beta".to_string()))
    );
    assert_eq!(rows[0][2], ("title", Value::Text("title".to_string())));

    let flat = EncryptedNote::objects()
        .values_list(&db, "secret")
        .await
        .unwrap();
    assert_eq!(flat, vec![Value::Text("alpha".to_string())]);
}

#[tokio::test]
async fn grouping_aggregating_and_functions_on_encrypted_fields_are_rejected() {
    use crate::aggregate::{AggExpr, FuncExpr};
    init_keys();
    let db = connect_memory().await;
    note("x").save(&db).await.unwrap();

    let grouped = EncryptedNote::objects()
        .annotate(&db, &["secret"], vec![("n", AggExpr::Count { field: "*" })])
        .await;
    assert!(matches!(grouped, Err(OrmError::InvalidQuery(ref m)) if m.contains("secret")));

    let max = EncryptedNote::objects()
        .aggregate(&db, vec![AggExpr::Max { field: "secret" }])
        .await;
    assert!(matches!(max, Err(OrmError::InvalidQuery(_))));

    let upper = EncryptedNote::objects()
        .annotate_funcs(
            &db,
            &["title"],
            vec![("u", FuncExpr::Upper { field: "secret" })],
        )
        .await;
    assert!(matches!(upper, Err(OrmError::InvalidQuery(_))));

    // Counting rows by an encrypted column is fine: it only checks for NULL.
    let counted = EncryptedNote::objects()
        .aggregate(&db, vec![AggExpr::Count { field: "secret" }])
        .await
        .unwrap();
    assert_eq!(counted.len(), 1);
}
