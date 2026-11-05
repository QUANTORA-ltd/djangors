use super::*;

const TEST_DB_URL: &str = "postgres://postgres:postgres@localhost/djangors_test";

// Each test uses its own uniquely-named table, but all of them share the single
// `djangors_migrations` bookkeeping table via `ensure_history()`'s
// `CREATE TABLE IF NOT EXISTS`. That statement is not atomic under real concurrency -
// two tests creating it at the same instant can hit Postgres's own catalog-level
// uniqueness constraints (pg_class_relname_nsp_index / pg_type_typname_nsp_index),
// the same class of cross-test DDL race documented in PLAN.md's Phase 11 (task #61).
static TEST_DB_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Every test gets its own migrations directory and its own uniquely-named table, so
/// tests can run concurrently against the one real shared `djangors_test` database
/// without colliding (djangors-test's `TestDatabase` does not yet provide per-test
/// isolation - see PLAN.md Phase 11, "TestDatabase transactional rollback-per-test").
/// The shared `djangors_migrations` tracking table is safe to share too, since every
/// migration name embeds the same unique suffix.
fn unique_suffix() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{nanos}_{:?}", std::thread::current().id())
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect()
}

async fn connect() -> djangors_db::Database {
    match djangors_test::TestBackend::current() {
        djangors_test::TestBackend::Sqlite => {
            let config = djangors_db::config::DatabaseConfig::new("sqlite::memory:".to_string())
                .max_connections(1)
                .min_connections(1);
            djangors_db::Database::connect(&config)
                .await
                .expect("connect sqlite")
        }
        djangors_test::TestBackend::Postgres => {
            let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| TEST_DB_URL.to_string());
            let config = djangors_db::config::DatabaseConfig::new(&url);
            djangors_db::Database::connect(&config)
                .await
                .expect("connect postgres")
        }
    }
}

fn write_migration(dir: &std::path::Path, n: u32, name: &str, up: &str, down: Option<&str>) {
    let body = match down {
        Some(down) => format!("-- up\n{up}\n-- down\n{down}\n"),
        None => format!("-- up\n{up}\n-- down\n-- no-down\n"),
    };
    std::fs::write(dir.join(format!("{n:04}_{name}.sql")), body).unwrap();
}

#[tokio::test]
async fn multi_migration_sequence_add_column_actually_applies() {
    let db = connect().await;
    let _guard = TEST_DB_LOCK.lock().await;
    let suffix = unique_suffix();
    let table = format!("mig_test_{suffix}");
    let dir = std::env::temp_dir().join(format!("djangors_migtest_{suffix}"));
    std::fs::create_dir_all(&dir).unwrap();

    let auto_pk = db.dialect().auto_pk_type();
    let ph = db.dialect().placeholder(1);

    write_migration(
        &dir,
        1,
        &format!("create_{suffix}"),
        &format!("CREATE TABLE {table} (id {auto_pk})"),
        Some(&format!("DROP TABLE {table}")),
    );
    write_migration(
        &dir,
        2,
        &format!("addcol_{suffix}"),
        &format!("ALTER TABLE {table} ADD COLUMN label TEXT"),
        Some(&format!("ALTER TABLE {table} DROP COLUMN label")),
    );

    migrate_from_dir(&db, &dir)
        .await
        .expect("migrate_from_dir should apply both migrations");

    // The column must genuinely exist now - this is exactly what the old code never did.
    db.conn()
        .execute(
            &format!("INSERT INTO {table} (label) VALUES ('hello')"),
            &[],
        )
        .await
        .expect("label column should exist and accept a value");

    // Cleanup.
    db.conn()
        .execute(&format!("DROP TABLE {table}"), &[])
        .await
        .ok();
    let delete_sql = format!("DELETE FROM djangors_migrations WHERE name LIKE {ph}");
    db.conn()
        .execute(&delete_sql, &[BindValue::Text(format!("%{suffix}%"))])
        .await
        .ok();
}

#[tokio::test]
async fn rollback_reverses_a_real_schema_change() {
    let db = connect().await;
    let _guard = TEST_DB_LOCK.lock().await;
    let suffix = unique_suffix();
    let table = format!("mig_rb_{suffix}");
    let dir = std::env::temp_dir().join(format!("djangors_migtest_rb_{suffix}"));
    std::fs::create_dir_all(&dir).unwrap();

    let auto_pk = db.dialect().auto_pk_type();
    let ph = db.dialect().placeholder(1);

    write_migration(
        &dir,
        1,
        &format!("create_{suffix}"),
        &format!("CREATE TABLE {table} (id {auto_pk})"),
        Some(&format!("DROP TABLE {table}")),
    );
    write_migration(
        &dir,
        2,
        &format!("addcol_{suffix}"),
        &format!("ALTER TABLE {table} ADD COLUMN label TEXT"),
        Some(&format!("ALTER TABLE {table} DROP COLUMN label")),
    );
    migrate_from_dir(&db, &dir).await.unwrap();

    rollback_from_dir(&db, &dir, 1)
        .await
        .expect("rolling back the last migration should succeed");

    // The column must genuinely be gone now.
    let err = db
        .conn()
        .execute(&format!("INSERT INTO {table} (label) VALUES ('nope')"), &[])
        .await
        .unwrap_err();
    assert!(
        format!("{err}").to_lowercase().contains("label")
            || format!("{err}").to_lowercase().contains("column")
            || format!("{err}").to_lowercase().contains("has no column"),
        "expected a missing-column error after rollback, got: {err}"
    );

    let query_sql = format!("SELECT COUNT(*) FROM djangors_migrations WHERE name LIKE {ph}");
    let row = db
        .conn()
        .fetch_one(&query_sql, &[BindValue::Text(format!("%{suffix}%"))])
        .await
        .unwrap();
    let remaining = row.try_i64(0).unwrap().unwrap();
    assert_eq!(
        remaining, 1,
        "only the create-table migration should remain recorded as applied"
    );

    db.conn()
        .execute(&format!("DROP TABLE {table}"), &[])
        .await
        .ok();
    let delete_sql = format!("DELETE FROM djangors_migrations WHERE name LIKE {ph}");
    db.conn()
        .execute(&delete_sql, &[BindValue::Text(format!("%{suffix}%"))])
        .await
        .ok();
}

#[tokio::test]
async fn rollback_refuses_a_non_invertible_migration_without_partial_effect() {
    let db = connect().await;
    let _guard = TEST_DB_LOCK.lock().await;
    let suffix = unique_suffix();
    let table = format!("mig_ni_{suffix}");
    let dir = std::env::temp_dir().join(format!("djangors_migtest_ni_{suffix}"));
    std::fs::create_dir_all(&dir).unwrap();

    let auto_pk = db.dialect().auto_pk_type();
    let ph = db.dialect().placeholder(1);

    write_migration(
        &dir,
        1,
        &format!("create_{suffix}"),
        &format!("CREATE TABLE {table} (id {auto_pk}, doomed TEXT)"),
        Some(&format!("DROP TABLE {table}")),
    );
    // A DropColumn migration has no safe down - written with no down section at all.
    write_migration(
        &dir,
        2,
        &format!("dropcol_{suffix}"),
        &format!("ALTER TABLE {table} DROP COLUMN doomed"),
        None,
    );
    migrate_from_dir(&db, &dir).await.unwrap();

    let result = rollback_from_dir(&db, &dir, 1).await;
    assert!(
        matches!(result, Err(MigrationError::NonInvertible { .. })),
        "expected NonInvertible, got {result:?}"
    );

    // Both migrations must still be recorded as applied - the refusal must not have
    // partially rolled anything back.
    let query_sql = format!("SELECT COUNT(*) FROM djangors_migrations WHERE name LIKE {ph}");
    let row = db
        .conn()
        .fetch_one(&query_sql, &[BindValue::Text(format!("%{suffix}%"))])
        .await
        .unwrap();
    let remaining = row.try_i64(0).unwrap().unwrap();
    assert_eq!(remaining, 2, "no migration should have been rolled back");

    db.conn()
        .execute(&format!("DROP TABLE {table}"), &[])
        .await
        .ok();
    let delete_sql = format!("DELETE FROM djangors_migrations WHERE name LIKE {ph}");
    db.conn()
        .execute(&delete_sql, &[BindValue::Text(format!("%{suffix}%"))])
        .await
        .ok();
}

#[tokio::test]
async fn migrate_from_dir_is_idempotent() {
    let db = connect().await;
    let _guard = TEST_DB_LOCK.lock().await;
    let suffix = unique_suffix();
    let table = format!("mig_idem_{suffix}");
    let dir = std::env::temp_dir().join(format!("djangors_migtest_idem_{suffix}"));
    std::fs::create_dir_all(&dir).unwrap();

    let auto_pk = db.dialect().auto_pk_type();
    let ph = db.dialect().placeholder(1);

    write_migration(
        &dir,
        1,
        &format!("create_{suffix}"),
        &format!("CREATE TABLE {table} (id {auto_pk})"),
        Some(&format!("DROP TABLE {table}")),
    );

    migrate_from_dir(&db, &dir).await.unwrap();
    // Running it again must be a clean no-op, not a "relation already exists" error.
    migrate_from_dir(&db, &dir)
        .await
        .expect("re-running migrate_from_dir must be idempotent");

    let query_sql = format!("SELECT COUNT(*) FROM djangors_migrations WHERE name LIKE {ph}");
    let row = db
        .conn()
        .fetch_one(&query_sql, &[BindValue::Text(format!("%{suffix}%"))])
        .await
        .unwrap();
    let applied = row.try_i64(0).unwrap().unwrap();
    assert_eq!(applied, 1, "the migration must be recorded exactly once");

    db.conn()
        .execute(&format!("DROP TABLE {table}"), &[])
        .await
        .ok();
    let delete_sql = format!("DELETE FROM djangors_migrations WHERE name LIKE {ph}");
    db.conn()
        .execute(&delete_sql, &[BindValue::Text(format!("%{suffix}%"))])
        .await
        .ok();
}

#[tokio::test]
async fn test_sqlite_plan_execution_from_snapshots() {
    use djangors_db::{Database, DatabaseConfig, Dialect};
    use djangors_orm::{FieldKind, FieldSnapshot, ModelSnapshot, SnapshotDefault};

    let snapshot = ModelSnapshot {
        app_label: "testapp".to_string(),
        table_name: "testapp_item".to_string(),
        struct_name: "Item".to_string(),
        fields: vec![
            FieldSnapshot {
                name: "id".to_string(),
                column_name: "id".to_string(),
                kind: FieldKind::BigInt,
                nullable: false,
                primary_key: true,
                unique: false,
                db_index: false,
                default: SnapshotDefault::None,
                max_length: None,
                auto: true,
            },
            FieldSnapshot {
                name: "title".to_string(),
                column_name: "title".to_string(),
                kind: FieldKind::Char,
                nullable: false,
                primary_key: false,
                unique: false,
                db_index: false,
                default: SnapshotDefault::None,
                max_length: Some(100),
                auto: false,
            },
        ],
        relations: vec![],
        unique_together: vec![],
        ordering: vec![],
        indexes: vec![],
    };

    let plan = build_create_plan_from_snapshots(&[snapshot], Dialect::Sqlite).unwrap();
    let db = Database::connect(&DatabaseConfig::new("sqlite::memory:"))
        .await
        .unwrap();

    for op in plan {
        let sql = op.to_sql(Dialect::Sqlite).unwrap();
        db.conn().execute(&sql, &[]).await.unwrap();
    }

    db.conn()
        .execute(
            "INSERT INTO testapp_item (title) VALUES ('hello_sqlite')",
            &[],
        )
        .await
        .unwrap();

    let row = db
        .conn()
        .fetch_one("SELECT title FROM testapp_item WHERE id = 1", &[])
        .await
        .unwrap();
    assert_eq!(row.try_string(0).unwrap().unwrap(), "hello_sqlite");
}

#[tokio::test]
async fn schema_validation_reports_missing_tables_and_columns() {
    use djangors_orm::{FieldKind, FieldSnapshot, ModelSnapshot, SnapshotDefault};

    let db = connect().await;
    let _guard = TEST_DB_LOCK.lock().await;
    let table = format!("schema_check_{}", unique_suffix());
    let snapshot = ModelSnapshot {
        app_label: "testapp".to_string(),
        table_name: table.clone(),
        struct_name: "SchemaCheck".to_string(),
        fields: vec![
            FieldSnapshot {
                name: "id".to_string(),
                column_name: "id".to_string(),
                kind: FieldKind::BigInt,
                nullable: false,
                primary_key: true,
                unique: false,
                db_index: false,
                default: SnapshotDefault::None,
                max_length: None,
                auto: true,
            },
            FieldSnapshot {
                name: "label".to_string(),
                column_name: "label".to_string(),
                kind: FieldKind::Char,
                nullable: true,
                primary_key: false,
                unique: false,
                db_index: false,
                default: SnapshotDefault::None,
                max_length: Some(100),
                auto: false,
            },
        ],
        relations: vec![],
        unique_together: vec![],
        ordering: vec![],
        indexes: vec![],
    };

    let error = validate_model_schema(&db, std::slice::from_ref(&snapshot))
        .await
        .expect_err("an absent model table must be reported");
    assert!(matches!(error, MigrationError::SchemaBehind { .. }));

    let id = db.dialect().auto_pk_type();
    db.conn()
        .execute(&format!("CREATE TABLE \"{table}\" (id {id})"), &[])
        .await
        .unwrap();
    let error = validate_model_schema(&db, &[snapshot]).await.unwrap_err();
    match error {
        MigrationError::SchemaBehind {
            table: found,
            missing,
        } => {
            assert_eq!(found, table);
            assert_eq!(missing, vec!["label"]);
        }
        other => panic!("expected SchemaBehind, got {other:?}"),
    }

    db.conn()
        .execute(&format!("DROP TABLE \"{table}\""), &[])
        .await
        .unwrap();
}

#[tokio::test]
async fn test_sqlite_migrate_from_dir_cycle() {
    use djangors_db::{Database, DatabaseConfig};

    let db = Database::connect(&DatabaseConfig::new("sqlite::memory:"))
        .await
        .unwrap();

    let dir = std::env::temp_dir().join(format!(
        "djangors_sqlite_migtest_{:?}",
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let migration_body = "-- up\nCREATE TABLE test_cycle_sqlite (id INTEGER PRIMARY KEY AUTOINCREMENT, val TEXT);\n-- down\nDROP TABLE test_cycle_sqlite;\n";
    std::fs::write(dir.join("0001_initial.sql"), migration_body).unwrap();

    migrate_from_dir(&db, &dir).await.unwrap();

    let row = db
        .conn()
        .fetch_one(
            "SELECT name FROM djangors_migrations WHERE name = '0001_initial'",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(row.try_string(0).unwrap().unwrap(), "0001_initial");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_alter_column_type_dialect_support() {
    use djangors_db::Dialect;

    let op = Operation::AlterColumnType {
        table_name: "users".to_string(),
        column_name: "age".to_string(),
        new_sql_type: "BIGINT".to_string(),
    };

    let pg_sql = op.to_sql(Dialect::Postgres).unwrap();
    assert_eq!(
        pg_sql,
        "ALTER TABLE \"users\" ALTER COLUMN \"age\" TYPE BIGINT USING \"age\"::BIGINT;"
    );

    let sqlite_res = op.to_sql(Dialect::Sqlite);
    assert!(matches!(
        sqlite_res,
        Err(MigrationError::UnsupportedOnDialect { operation, dialect })
        if operation == "AlterColumnType" && dialect == "Sqlite"
    ));
}

#[tokio::test]
async fn test_record_and_get_applied_migrations() {
    use djangors_db::{Database, DatabaseConfig};

    let db = Database::connect(&DatabaseConfig::new("sqlite::memory:"))
        .await
        .unwrap();

    record_migration(&db, "0001_initial").await.unwrap();
    record_migration(&db, "0002_add_field").await.unwrap();

    let applied = get_applied_migrations(&db).await.unwrap();
    assert_eq!(applied, vec!["0001_initial", "0002_add_field"]);
}

#[test]
fn test_create_and_drop_index_operations() {
    use crate::operation::Operation;
    use djangors_db::Dialect;

    let create = Operation::CreateIndex {
        index_name: "idx_testapp_item_slug".to_string(),
        table_name: "testapp_item".to_string(),
        columns: vec!["slug".to_string(), "author".to_string()],
    };
    assert_eq!(
        create.to_sql(Dialect::Postgres).unwrap(),
        "CREATE INDEX IF NOT EXISTS \"idx_testapp_item_slug\" ON \"testapp_item\" (\"slug\", \"author\");"
    );

    // Reverse of CreateIndex is DropIndex.
    let drop = create.reverse().unwrap();
    assert_eq!(
        drop.to_sql(Dialect::Sqlite).unwrap(),
        "DROP INDEX IF EXISTS \"idx_testapp_item_slug\";"
    );
}

#[test]
fn test_create_plan_from_snapshots_emits_indexes() {
    use djangors_db::Dialect;
    use djangors_orm::{FieldKind, FieldSnapshot, IndexSnapshot, ModelSnapshot, SnapshotDefault};

    let snapshot = ModelSnapshot {
        app_label: "testapp".to_string(),
        table_name: "testapp_item".to_string(),
        struct_name: "Item".to_string(),
        fields: vec![FieldSnapshot {
            name: "id".to_string(),
            column_name: "id".to_string(),
            kind: FieldKind::BigInt,
            nullable: false,
            primary_key: true,
            unique: false,
            db_index: false,
            default: SnapshotDefault::None,
            max_length: None,
            auto: true,
        }],
        relations: vec![],
        unique_together: vec![],
        ordering: vec![],
        indexes: vec![IndexSnapshot {
            name: "idx_item_slug".to_string(),
            fields: vec!["slug".to_string()],
        }],
    };

    let plan = crate::plan::build_create_plan_from_snapshots(&[snapshot], Dialect::Sqlite).unwrap();
    let index_ops: Vec<_> = plan
        .iter()
        .filter(|op| matches!(op, crate::operation::Operation::CreateIndex { .. }))
        .collect();
    assert_eq!(index_ops.len(), 1);
}

#[test]
fn test_create_plan_from_snapshots_emits_m2m_join_table() {
    use djangors_db::Dialect;
    use djangors_orm::{
        FieldKind, FieldSnapshot, ModelSnapshot, OnDelete, RelationKind, RelationSnapshot,
        SnapshotDefault,
    };

    let pk = FieldSnapshot {
        name: "id".to_string(),
        column_name: "id".to_string(),
        kind: FieldKind::BigInt,
        nullable: false,
        primary_key: true,
        unique: false,
        db_index: false,
        default: SnapshotDefault::None,
        max_length: None,
        auto: true,
    };
    let school = ModelSnapshot {
        app_label: "app".to_string(),
        table_name: "app_school".to_string(),
        struct_name: "School".to_string(),
        fields: vec![pk.clone()],
        relations: vec![RelationSnapshot {
            field_name: "students".to_string(),
            kind: RelationKind::ManyToMany,
            target_struct: "Student".to_string(),
            on_delete: OnDelete::Cascade,
            related_name: Some("schools".to_string()),
        }],
        unique_together: vec![],
        ordering: vec![],
        indexes: vec![],
    };
    let student = ModelSnapshot {
        app_label: "app".to_string(),
        table_name: "app_student".to_string(),
        struct_name: "Student".to_string(),
        fields: vec![pk],
        relations: vec![],
        unique_together: vec![],
        ordering: vec![],
        indexes: vec![],
    };

    let plan =
        crate::plan::build_create_plan_from_snapshots(&[school, student], Dialect::Sqlite).unwrap();

    // Join table `app_school_students` must be created with both FK columns.
    let join: Vec<_> = plan
        .iter()
        .filter(|op| {
            matches!(op, crate::operation::Operation::CreateTable { table_name, .. }
                if table_name == "app_school_students")
        })
        .collect();
    assert_eq!(join.len(), 1);
    if let crate::operation::Operation::CreateTable { columns, .. } = &join[0] {
        let names: Vec<&str> = columns.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"school_id"));
        assert!(names.contains(&"student_id"));
    } else {
        panic!("expected CreateTable");
    }
}

#[test]
fn test_detect_changes_new_and_removed_columns() {
    use djangors_db::Dialect;
    use djangors_orm::{FieldKind, FieldSnapshot, ModelSnapshot, SnapshotDefault};

    let pk = FieldSnapshot {
        name: "id".to_string(),
        column_name: "id".to_string(),
        kind: FieldKind::BigInt,
        nullable: false,
        primary_key: true,
        unique: false,
        db_index: false,
        default: SnapshotDefault::None,
        max_length: None,
        auto: true,
    };
    let prev = ModelSnapshot {
        app_label: "a".to_string(),
        table_name: "a_m".to_string(),
        struct_name: "M".to_string(),
        fields: vec![pk.clone()],
        relations: vec![],
        unique_together: vec![],
        ordering: vec![],
        indexes: vec![],
    };
    let mut curr = prev.clone();
    curr.fields.push(FieldSnapshot {
        name: "extra".to_string(),
        column_name: "extra".to_string(),
        kind: FieldKind::Char,
        nullable: true,
        primary_key: false,
        unique: false,
        db_index: false,
        default: SnapshotDefault::None,
        max_length: Some(100),
        auto: false,
    });

    let ops = crate::plan::detect_changes(
        std::slice::from_ref(&prev),
        std::slice::from_ref(&curr),
        Dialect::Postgres,
    )
    .unwrap();
    let add = ops
        .iter()
        .find(|o| matches!(o, crate::operation::Operation::AddColumn { .. }));
    assert!(add.is_some());

    let prev = prev.clone();
    let ops_rev = crate::plan::detect_changes(&[curr], &[prev], Dialect::Postgres).unwrap();
    let drop = ops_rev
        .iter()
        .find(|o| matches!(o, crate::operation::Operation::DropColumn { .. }));
    assert!(drop.is_some());
}
