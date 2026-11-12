use crate::meta::{
    all_registered_models, DefaultValue, FieldKind, FieldMeta, ForeignKey, ManyToMany, Model,
    ModelMeta, ModelRegistration, OnDelete, OneToOne, RelationKind, RelationMeta,
};
use djangors_db::DbExecutor;

pub struct FictionalModel;

impl Model for FictionalModel {
    fn meta() -> &'static ModelMeta {
        static META: std::sync::OnceLock<ModelMeta> = std::sync::OnceLock::new();
        META.get_or_init(|| ModelMeta {
            struct_name: "FictionalModel",
            app_label: "test_app",
            table_name: "test_app_fictionalmodel",
            fields: &[
                FieldMeta {
                    name: "id",
                    column_name: "id",
                    kind: FieldKind::BigInt,
                    nullable: false,
                    primary_key: true,
                    auto: true,
                    unique: true,
                    db_index: true,
                    default: DefaultValue::None,
                    max_length: None,
                    verbose_name: None,
                    help_text: None,
                    choices: &[],
                },
                FieldMeta {
                    name: "name",
                    column_name: "name",
                    kind: FieldKind::Char,
                    nullable: false,
                    primary_key: false,
                    auto: false,
                    unique: false,
                    db_index: false,
                    default: DefaultValue::Text("unnamed"),
                    max_length: Some(100),
                    verbose_name: Some("Name"),
                    help_text: Some("Enter the name"),
                    choices: &[],
                },
            ],
            relations: &[],
            indexes: &[],
            unique_together: &[],
            ordering: &[],
        })
    }

    fn field_values(&self) -> Vec<(&'static str, crate::expr::Value)> {
        vec![
            ("id", crate::expr::Value::I64(1)),
            ("name", crate::expr::Value::Text("test".to_string())),
        ]
    }

    fn field_names() -> Vec<&'static str> {
        vec!["id", "name"]
    }
}

pub struct RelatedModel;

impl Model for RelatedModel {
    fn meta() -> &'static ModelMeta {
        static META: std::sync::OnceLock<ModelMeta> = std::sync::OnceLock::new();
        META.get_or_init(|| ModelMeta {
            struct_name: "RelatedModel",
            app_label: "test_app",
            table_name: "test_app_relatedmodel",
            fields: &[FieldMeta {
                name: "id",
                column_name: "id",
                kind: FieldKind::BigInt,
                nullable: false,
                primary_key: true,
                auto: true,
                unique: true,
                db_index: true,
                default: DefaultValue::None,
                max_length: None,
                verbose_name: None,
                help_text: None,
                choices: &[],
            }],
            relations: &[RelationMeta {
                field_name: "fictional",
                kind: RelationKind::ForeignKey,
                target: FictionalModel::meta,
                on_delete: OnDelete::Cascade,
                related_name: Some("related_instances"),
            }],
            indexes: &[],
            unique_together: &[],
            ordering: &[],
        })
    }

    fn field_values(&self) -> Vec<(&'static str, crate::expr::Value)> {
        vec![
            ("id", crate::expr::Value::I64(1)),
            ("fictional", crate::expr::Value::I64(2)),
        ]
    }

    fn field_names() -> Vec<&'static str> {
        vec!["id", "fictional"]
    }
}

// Register FictionalModel and RelatedModel for testing inventory iteration
inventory::submit! {
    ModelRegistration {
        meta_fn: FictionalModel::meta,
    }
}

inventory::submit! {
    ModelRegistration {
        meta_fn: RelatedModel::meta,
    }
}

#[test]
fn test_manual_model_meta_reading() {
    let meta = FictionalModel::meta();
    assert_eq!(meta.struct_name, "FictionalModel");
    assert_eq!(meta.app_label, "test_app");
    assert_eq!(meta.table_name, "test_app_fictionalmodel");

    assert_eq!(meta.fields.len(), 2);
    let id_field = meta.fields[0];
    assert_eq!(id_field.name, "id");
    assert_eq!(id_field.kind, FieldKind::BigInt);
    assert!(id_field.primary_key);
    assert_eq!(id_field.default, DefaultValue::None);

    let name_field = meta.fields[1];
    assert_eq!(name_field.name, "name");
    assert_eq!(name_field.kind, FieldKind::Char);
    assert_eq!(name_field.default, DefaultValue::Text("unnamed"));
    assert_eq!(name_field.max_length, Some(100));

    let related_meta = RelatedModel::meta();
    assert_eq!(related_meta.relations.len(), 1);
    let relation = &related_meta.relations[0];
    assert_eq!(relation.field_name, "fictional");
    assert_eq!(relation.kind, RelationKind::ForeignKey);
    assert_eq!(relation.on_delete, OnDelete::Cascade);
    assert_eq!(relation.related_name, Some("related_instances"));

    // Verify resolving target returns FictionalModel's meta
    let target_meta = (relation.target)();
    assert_eq!(target_meta.struct_name, "FictionalModel");
}

#[test]
fn test_inventory_registration() {
    let registered: Vec<&'static ModelMeta> = all_registered_models().collect();

    // Verify FictionalModel is present
    let fictional_found = registered
        .iter()
        .any(|m| m.struct_name == "FictionalModel" && m.app_label == "test_app");
    assert!(
        fictional_found,
        "FictionalModel should be found in registered models"
    );

    // Verify RelatedModel is present
    let related_found = registered
        .iter()
        .any(|m| m.struct_name == "RelatedModel" && m.app_label == "test_app");
    assert!(
        related_found,
        "RelatedModel should be found in registered models"
    );
}

#[test]
fn test_foreign_key_clone_copy() {
    let fk = ForeignKey::<FictionalModel>::new(42);
    assert_eq!(fk.id, 42);

    // Verify copy semantics
    let fk_copy = fk;
    let fk_copy2 = fk;
    assert_eq!(fk_copy.id, 42);
    assert_eq!(fk_copy2.id, 42);

    // Verify debug print format
    let debug_str = format!("{:?}", fk);
    assert!(debug_str.contains("ForeignKey"));
    assert!(debug_str.contains("id: 42"));
}

use crate::q;
use djangors_macros::Model;

#[derive(Model, Debug, Clone)]
#[djangors(app = "test_app", table_name = "test_queryset_model")]
#[allow(dead_code)]
pub struct QuerySetTestModel {
    #[djangors(primary_key, auto)]
    pub id: i64,

    pub name: String,

    pub is_active: bool,
}

pub struct ActiveManager;

impl crate::meta::Manager<ManagedModel> for ActiveManager {
    fn objects() -> crate::queryset::QuerySet<ManagedModel> {
        crate::queryset::QuerySet::<ManagedModel>::new()
            .filter(q!(is_active = true))
            .expect("valid filter")
    }
}

#[derive(Model, Debug)]
#[djangors(
    app = "test_app",
    table_name = "test_manager_model",
    manager = "ActiveManager"
)]
#[allow(dead_code)]
pub struct ManagedModel {
    #[djangors(primary_key, auto)]
    pub id: i64,

    pub name: String,

    pub is_active: bool,
}

#[tokio::test]
async fn test_custom_manager_objects() {
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_manager_model", &[])
        .await;
    db.conn()
        .execute(
            &format!(
                "CREATE TABLE test_manager_model (
                    id {auto_pk},
                    name TEXT NOT NULL,
                    is_active BOOLEAN NOT NULL
                )"
            ),
            &[],
        )
        .await
        .unwrap();

    for (name, active) in [("a", true), ("b", false), ("c", true)] {
        let row = ManagedModel {
            id: 0,
            name: name.to_string(),
            is_active: active,
        };
        row.save(db).await.unwrap();
    }

    // objects() is the custom manager: only active rows
    let active = ManagedModel::objects().all(db).await.unwrap();
    assert_eq!(active.len(), 2);
    assert!(active.iter().all(|r| r.is_active));

    let _ = db
        .conn()
        .execute("DROP TABLE test_subquery_author", &[])
        .await;
}

#[derive(Model, Debug, Clone)]
#[djangors(app = "test_app", table_name = "test_one_to_one_user")]
pub struct OneToOneUser {
    #[djangors(primary_key, auto)]
    pub id: i64,
    pub name: String,
}

#[derive(Model, Debug, Clone)]
#[djangors(app = "test_app", table_name = "test_one_to_one_profile")]
pub struct OneToOneProfile {
    #[djangors(primary_key, auto)]
    pub id: i64,
    #[djangors(foreign_key(on_delete = "cascade", related_name = "profile"))]
    pub user: OneToOne<OneToOneUser>,
}

#[tokio::test]
async fn test_one_to_one_metadata_and_fetch() {
    assert_eq!(
        OneToOneProfile::meta().relations[0].kind,
        RelationKind::OneToOne
    );

    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_one_to_one_profile", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_one_to_one_user", &[])
        .await;
    db.conn()
        .execute(
            &format!("CREATE TABLE test_one_to_one_user (id {auto_pk}, name TEXT NOT NULL)"),
            &[],
        )
        .await
        .unwrap();
    // The OneToOne column must be UNIQUE. `user` is a reserved word in Postgres (but not in
    // SQLite), so the identifier has to be quoted through the dialect here exactly the way the
    // queryset builder quotes it at query time — a bare `user` parses on SQLite and is a syntax
    // error on Postgres.
    let user_col = db.dialect().quote_ident("user");
    let create_profile = format!(
        "CREATE TABLE test_one_to_one_profile (id {auto_pk}, {user_col} BIGINT NOT NULL UNIQUE REFERENCES test_one_to_one_user(id) ON DELETE CASCADE)"
    );
    db.conn().execute(&create_profile, &[]).await.unwrap();

    let user = OneToOneUser {
        id: 0,
        name: "u1".to_string(),
    };
    let saved = user.save(db).await.unwrap();
    let profile = OneToOneProfile {
        id: 0,
        user: OneToOne::new(saved.id),
    };
    let saved_profile = profile.save(db).await.unwrap();

    // Lazy fetch through the OneToOne accessor.
    let fetched = saved_profile.user.fetch(db).await.unwrap();
    assert_eq!(fetched.as_ref().map(|u| u.name.as_str()), Some("u1"));

    // Drop the child first: the profile's FK references the user table.
    let _ = db
        .conn()
        .execute("DROP TABLE test_one_to_one_profile", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE test_one_to_one_user", &[])
        .await;
}

#[derive(Model, Debug, Clone)]
#[djangors(app = "test_app", table_name = "test_m2m_student")]
pub struct M2MStudent {
    #[djangors(primary_key, auto)]
    pub id: i64,
    pub name: String,
}

#[derive(Model, Debug, Clone)]
#[djangors(app = "test_app", table_name = "test_m2m_school")]
pub struct M2MSchool {
    #[djangors(primary_key, auto)]
    pub id: i64,
    pub name: String,
    // A `ManyToMany<T>` is a zero-sized marker that only exists to drive the derive's relation
    // metadata — it has no column and is never read back, so `dead_code` fires on it.
    #[allow(dead_code)]
    #[djangors(many_to_many(related_name = "schools"))]
    pub students: ManyToMany<M2MStudent>,
}

#[tokio::test]
async fn test_many_to_many_metadata_join_and_helpers() {
    use crate::m2m_add;
    use crate::m2m_related;
    use crate::m2m_remove;

    assert_eq!(
        M2MSchool::meta().relations[0].kind,
        RelationKind::ManyToMany
    );
    assert_eq!(
        crate::meta::m2m_through_table(M2MSchool::meta(), &M2MSchool::meta().relations[0]),
        "test_m2m_school_students"
    );

    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_m2m_school_students", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_m2m_school", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_m2m_student", &[])
        .await;
    db.conn()
        .execute(
            &format!("CREATE TABLE test_m2m_student (id {auto_pk}, name TEXT NOT NULL)"),
            &[],
        )
        .await
        .unwrap();
    db.conn()
        .execute(
            &format!("CREATE TABLE test_m2m_school (id {auto_pk}, name TEXT NOT NULL)"),
            &[],
        )
        .await
        .unwrap();
    db.conn()
        .execute(
            "CREATE TABLE test_m2m_school_students (
                    m2mschool_id BIGINT NOT NULL REFERENCES test_m2m_school(id) ON DELETE CASCADE,
                    m2mstudent_id BIGINT NOT NULL REFERENCES test_m2m_student(id) ON DELETE CASCADE
                )",
            &[],
        )
        .await
        .unwrap();

    let s1 = M2MStudent {
        id: 0,
        name: "ann".to_string(),
    }
    .save(db)
    .await
    .unwrap();
    let s2 = M2MStudent {
        id: 0,
        name: "bob".to_string(),
    }
    .save(db)
    .await
    .unwrap();
    let school = M2MSchool {
        id: 0,
        name: "hs".to_string(),
        students: ManyToMany::new(),
    }
    .save(db)
    .await
    .unwrap();

    m2m_add::<M2MSchool, M2MStudent, _>(db, school.id, "students", s1.id)
        .await
        .unwrap();
    m2m_add::<M2MSchool, M2MStudent, _>(db, school.id, "students", s2.id)
        .await
        .unwrap();

    let related = m2m_related::<M2MSchool, M2MStudent, _>(db, school.id, "students")
        .await
        .unwrap();
    assert_eq!(related.len(), 2);
    let names: Vec<&str> = related.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"ann") && names.contains(&"bob"));

    m2m_remove::<M2MSchool, M2MStudent, _>(db, school.id, "students", s1.id)
        .await
        .unwrap();
    let after = m2m_related::<M2MSchool, M2MStudent, _>(db, school.id, "students")
        .await
        .unwrap();
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].name, "bob");

    let _ = db
        .conn()
        .execute("DROP TABLE test_m2m_school_students", &[])
        .await;
    let _ = db.conn().execute("DROP TABLE test_m2m_school", &[]).await;
    let _ = db.conn().execute("DROP TABLE test_m2m_student", &[]).await;
}

#[tokio::test]
async fn test_queryset_operations() {
    // Isolated per-test database: four tests in this file all create and drop the same
    // `test_queryset_model` table (the model's `table_name` is fixed at derive time, so they
    // cannot simply use different names). Sharing one Postgres database made them race —
    // one test dropping the table while another queried it — which SQLite never showed,
    // because each SQLite test already gets its own database.
    let test_db = djangors_test::TestDatabase::isolated().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();

    // Create table
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_queryset_model", &[])
        .await;

    let create_sql = format!(
        "CREATE TABLE test_queryset_model (
            id {auto_pk},
            name TEXT NOT NULL,
            is_active BOOLEAN NOT NULL
        )"
    );
    db.conn().execute(&create_sql, &[]).await.unwrap();

    // 1. Test empty queryset
    let count = QuerySetTestModel::objects().count(db).await.unwrap();
    assert_eq!(count, 0);

    let exists = QuerySetTestModel::objects().exists(db).await.unwrap();
    assert!(!exists);

    let first = QuerySetTestModel::objects().first(db).await.unwrap();
    assert!(first.is_none());

    let get_res = QuerySetTestModel::objects().get(db).await;
    assert!(matches!(
        get_res,
        Err(crate::error::OrmError::NotFound { .. })
    ));

    // 2. Insert out-of-order test data
    db.conn()
        .execute(
            "INSERT INTO test_queryset_model (name, is_active) VALUES ('Alice', true), ('Charlie', false), ('Bob', true)",
            &[],
        )
        .await
        .unwrap();

    // 3. Test basic operations: count, exists, first, all
    assert_eq!(QuerySetTestModel::objects().count(db).await.unwrap(), 3);
    assert!(QuerySetTestModel::objects().exists(db).await.unwrap());

    // 4. Test ordering (ascending name)
    let sorted_asc = QuerySetTestModel::objects()
        .order_by("name")
        .unwrap()
        .all(db)
        .await
        .unwrap();
    assert_eq!(sorted_asc.len(), 3);
    assert_eq!(sorted_asc[0].name, "Alice");
    assert_eq!(sorted_asc[1].name, "Bob");
    assert_eq!(sorted_asc[2].name, "Charlie");

    // Test ordering (descending name)
    let sorted_desc = QuerySetTestModel::objects()
        .order_by("-name")
        .unwrap()
        .all(db)
        .await
        .unwrap();
    assert_eq!(sorted_desc.len(), 3);
    assert_eq!(sorted_desc[0].name, "Charlie");
    assert_eq!(sorted_desc[1].name, "Bob");
    assert_eq!(sorted_desc[2].name, "Alice");

    // 5. Test filter: exact match
    let active_qs = QuerySetTestModel::objects()
        .filter(q!(is_active = true))
        .unwrap();
    assert_eq!(active_qs.count(db).await.unwrap(), 2);
    let active_rows = active_qs.order_by("name").unwrap().all(db).await.unwrap();
    assert_eq!(active_rows[0].name, "Alice");
    assert_eq!(active_rows[1].name, "Bob");

    // Test filter: contains lookup
    let ch_qs = QuerySetTestModel::objects()
        .filter(q!(name__contains = "ch"))
        .unwrap();
    if db.dialect() == djangors_db::Dialect::Postgres {
        assert_eq!(ch_qs.count(db).await.unwrap(), 0); // case sensitive on Postgres
    }
    let ich_qs = QuerySetTestModel::objects()
        .filter(q!(name__icontains = "ch"))
        .unwrap();
    assert_eq!(ich_qs.count(db).await.unwrap(), 1);
    assert_eq!(ich_qs.get(db).await.unwrap().name, "Charlie");

    // Test filter: starts with
    let starts_qs = QuerySetTestModel::objects()
        .filter(q!(name__startswith = "Al"))
        .unwrap();
    assert_eq!(starts_qs.get(db).await.unwrap().name, "Alice");

    // Test filter: ends with
    let ends_qs = QuerySetTestModel::objects()
        .filter(q!(name__endswith = "ob"))
        .unwrap();
    assert_eq!(ends_qs.get(db).await.unwrap().name, "Bob");

    // 6. Test limit & offset
    let limit_qs = QuerySetTestModel::objects()
        .order_by("name")
        .unwrap()
        .limit(2);
    let limit_rows = limit_qs.all(db).await.unwrap();
    assert_eq!(limit_rows.len(), 2);
    assert_eq!(limit_rows[0].name, "Alice");
    assert_eq!(limit_rows[1].name, "Bob");

    let offset_qs = QuerySetTestModel::objects()
        .order_by("name")
        .unwrap()
        .offset(1);
    let offset_rows = offset_qs.all(db).await.unwrap();
    assert_eq!(offset_rows.len(), 2);
    assert_eq!(offset_rows[0].name, "Bob");
    assert_eq!(offset_rows[1].name, "Charlie");

    // 7. Test get multiple error
    let multi_get = QuerySetTestModel::objects()
        .filter(q!(is_active = true))
        .unwrap()
        .get(db)
        .await;
    assert!(matches!(
        multi_get,
        Err(crate::error::OrmError::MultipleObjectsReturned { .. })
    ));

    // 8. Test typo'd field name validation in filter()
    let typo_res = QuerySetTestModel::objects().filter(q!(name_typo = "val"));
    match typo_res {
        Err(crate::error::OrmError::FieldNotFound { field, .. }) => {
            assert_eq!(field, "name_typo");
        }
        other => panic!("expected FieldNotFound, got {other:?}"),
    }

    // 9. Test typo'd field name validation in order_by()
    let order_typo_res =
        QuerySetTestModel::objects().order_by("'; DROP TABLE test_queryset_model; --");
    match order_typo_res {
        Err(crate::error::OrmError::FieldNotFound { field, .. }) => {
            assert_eq!(field, "'; DROP TABLE test_queryset_model; --");
        }
        other => panic!("expected FieldNotFound, got {other:?}"),
    }

    // 10. Test save() (insert new instance)
    let initial = QuerySetTestModel {
        id: 0,
        name: "David".to_string(),
        is_active: true,
    };
    let saved = initial.save(db).await.unwrap();
    assert_ne!(saved.id, 0);
    assert_eq!(saved.name, "David");
    assert!(saved.is_active);

    // Confirm it exists in DB directly
    let p1 = db.dialect().placeholder(1);
    let check_sql = format!(
        "SELECT COUNT(*) FROM test_queryset_model WHERE id = {p1} AND name = 'David' AND is_active = true"
    );
    let row = db
        .conn()
        .fetch_one(&check_sql, &[djangors_db::BindValue::I64(saved.id)])
        .await
        .unwrap();
    let count = row.try_i64(0).unwrap().unwrap();
    assert_eq!(count, 1);

    // 11. Test update()
    let mut to_update = saved;
    to_update.name = "David Updated".to_string();
    to_update.update(db).await.unwrap();

    // Re-fetch via QuerySet
    let fetched = QuerySetTestModel::objects()
        .filter(q!(id = to_update.id))
        .unwrap()
        .get(db)
        .await
        .unwrap();
    assert_eq!(fetched.name, "David Updated");

    // 12. Test update() on non-existent PK
    let non_existent = QuerySetTestModel {
        id: 999999,
        name: "Ghost".to_string(),
        is_active: false,
    };
    let update_res = non_existent.update(db).await;
    assert!(matches!(
        update_res,
        Err(crate::error::OrmError::NotFound { .. })
    ));

    // 13. Test delete()
    assert_eq!(QuerySetTestModel::objects().count(db).await.unwrap(), 4);
    fetched.delete(db).await.unwrap();
    assert_eq!(QuerySetTestModel::objects().count(db).await.unwrap(), 3);

    let get_deleted = QuerySetTestModel::objects()
        .filter(q!(id = fetched.id))
        .unwrap()
        .get(db)
        .await;
    assert!(matches!(
        get_deleted,
        Err(crate::error::OrmError::NotFound { .. })
    ));

    // 14. Test delete() on non-existent PK
    let delete_res = non_existent.delete(db).await;
    assert!(matches!(
        delete_res,
        Err(crate::error::OrmError::NotFound { .. })
    ));

    // Cleanup table
    let _ = db
        .conn()
        .execute("DROP TABLE test_queryset_model", &[])
        .await;
}

#[tokio::test]
async fn test_queryset_bulk_delete() {
    // Isolated per-test database: four tests in this file all create and drop the same
    // `test_queryset_model` table (the model's `table_name` is fixed at derive time, so they
    // cannot simply use different names). Sharing one Postgres database made them race —
    // one test dropping the table while another queried it — which SQLite never showed,
    // because each SQLite test already gets its own database.
    let test_db = djangors_test::TestDatabase::isolated().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_queryset_model", &[])
        .await;
    db.conn()
        .execute(
            &format!(
                "CREATE TABLE test_queryset_model (
                    id {auto_pk},
                    name TEXT NOT NULL,
                    is_active BOOLEAN NOT NULL
                )"
            ),
            &[],
        )
        .await
        .unwrap();

    for (name, active) in [
        ("a", true),
        ("b", true),
        ("c", false),
        ("d", false),
        ("e", true),
    ] {
        let row = QuerySetTestModel {
            id: 0,
            name: name.to_string(),
            is_active: active,
        };
        row.save(db).await.unwrap();
    }
    assert_eq!(QuerySetTestModel::objects().count(db).await.unwrap(), 5);

    // Filtered bulk delete: only inactive rows
    let deleted = QuerySetTestModel::objects()
        .filter(q!(is_active = false))
        .unwrap()
        .delete(db)
        .await
        .unwrap();
    assert_eq!(deleted, 2);
    assert_eq!(QuerySetTestModel::objects().count(db).await.unwrap(), 3);

    // Remaining rows are all active
    let remaining = QuerySetTestModel::objects().all(db).await.unwrap();
    assert!(remaining.iter().all(|r| r.is_active));

    // Unfiltered bulk delete clears the table
    let deleted_all = QuerySetTestModel::objects().delete(db).await.unwrap();
    assert_eq!(deleted_all, 3);
    assert_eq!(QuerySetTestModel::objects().count(db).await.unwrap(), 0);

    let _ = db
        .conn()
        .execute("DROP TABLE test_queryset_model", &[])
        .await;
}

#[tokio::test]
async fn test_queryset_cached_all_and_invalidation() {
    // Isolated per-test database: four tests in this file all create and drop the same
    // `test_queryset_model` table (the model's `table_name` is fixed at derive time, so they
    // cannot simply use different names). Sharing one Postgres database made them race —
    // one test dropping the table while another queried it — which SQLite never showed,
    // because each SQLite test already gets its own database.
    let test_db = djangors_test::TestDatabase::isolated().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_queryset_model", &[])
        .await;
    db.conn()
        .execute(
            &format!(
                "CREATE TABLE test_queryset_model (
                    id {auto_pk},
                    name TEXT NOT NULL,
                    is_active BOOLEAN NOT NULL
                )"
            ),
            &[],
        )
        .await
        .unwrap();

    for name in ["a", "b", "c"] {
        let row = QuerySetTestModel {
            id: 0,
            name: name.to_string(),
            is_active: true,
        };
        row.save(db).await.unwrap();
    }

    // Prime and reuse the cache.
    let qs = QuerySetTestModel::objects()
        .order_by("name")
        .unwrap()
        .cache(std::time::Duration::from_secs(60));
    let first = qs.cached_all(db).await.unwrap();
    let second = qs.cached_all(db).await.unwrap();
    assert_eq!(first.len(), 3);
    assert_eq!(second.len(), 3);

    // A write invalidates the table cache.
    let new_row = QuerySetTestModel {
        id: 0,
        name: "d".to_string(),
        is_active: true,
    };
    new_row.save(db).await.unwrap();
    let after = qs.cached_all(db).await.unwrap();
    assert_eq!(after.len(), 4);

    let _ = db
        .conn()
        .execute("DROP TABLE test_queryset_model", &[])
        .await;
}

#[tokio::test]
async fn test_get_or_create_and_update_or_create() {
    // Isolated per-test database: four tests in this file all create and drop the same
    // `test_queryset_model` table (the model's `table_name` is fixed at derive time, so they
    // cannot simply use different names). Sharing one Postgres database made them race —
    // one test dropping the table while another queried it — which SQLite never showed,
    // because each SQLite test already gets its own database.
    let test_db = djangors_test::TestDatabase::isolated().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_queryset_model", &[])
        .await;
    db.conn()
        .execute(
            &format!(
                "CREATE TABLE test_queryset_model (
                    id {auto_pk},
                    name TEXT NOT NULL,
                    is_active BOOLEAN NOT NULL
                )"
            ),
            &[],
        )
        .await
        .unwrap();

    let defaults = || {
        vec![
            ("name", crate::expr::Value::Text("Dana".into())),
            ("is_active", crate::expr::Value::Bool(true)),
        ]
    };

    // get_or_create: creates when absent
    let (created_1, was_created) = QuerySetTestModel::objects()
        .filter(crate::q!(name = "Dana"))
        .unwrap()
        .get_or_create(db, defaults)
        .await
        .unwrap();
    assert!(was_created);
    assert_eq!(created_1.name, "Dana");

    // get_or_create: returns existing when present (no duplicate)
    let (created_2, was_created) = QuerySetTestModel::objects()
        .filter(crate::q!(name = "Dana"))
        .unwrap()
        .get_or_create(db, defaults)
        .await
        .unwrap();
    assert!(!was_created);
    assert_eq!(created_2.id, created_1.id);
    assert_eq!(QuerySetTestModel::objects().count(db).await.unwrap(), 1);

    // update_or_create: updates an existing match
    let (updated, was_created) = QuerySetTestModel::objects()
        .filter(crate::q!(name = "Dana"))
        .unwrap()
        .update_or_create(db, defaults, || crate::set!(is_active = true))
        .await
        .unwrap();
    assert!(!was_created);
    assert!(updated.is_active);
    assert_eq!(QuerySetTestModel::objects().count(db).await.unwrap(), 1);

    // update_or_create: creates when absent
    let (made, was_created) = QuerySetTestModel::objects()
        .filter(crate::q!(name = "Erin"))
        .unwrap()
        .update_or_create(
            db,
            || {
                vec![
                    ("name", crate::expr::Value::Text("Erin".into())),
                    ("is_active", crate::expr::Value::Bool(false)),
                ]
            },
            || crate::set!(is_active = false),
        )
        .await
        .unwrap();
    assert!(was_created);
    assert_eq!(made.name, "Erin");
    assert!(!made.is_active);
    assert_eq!(QuerySetTestModel::objects().count(db).await.unwrap(), 2);

    // Cleanup
    let _ = db
        .conn()
        .execute("DROP TABLE test_queryset_model", &[])
        .await;
}

#[derive(Model, Debug)]
#[djangors(app = "test_app", table_name = "test_aggregate_model")]
#[allow(dead_code)]
pub struct AggregateTestModel {
    #[djangors(primary_key, auto)]
    pub id: i64,

    pub category: String,

    pub score: i32,
}

#[tokio::test]
async fn test_queryset_aggregation() {
    use crate::aggregate::{AggExpr, AggResult};

    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_aggregate_model", &[])
        .await;
    let create_sql = format!(
        "CREATE TABLE test_aggregate_model (
            id {auto_pk},
            category TEXT NOT NULL,
            score INTEGER NOT NULL
        )"
    );
    db.conn().execute(&create_sql, &[]).await.unwrap();

    db.conn()
        .execute(
            "INSERT INTO test_aggregate_model (category, score) VALUES ('a', 10), ('a', 20), ('a', 30), ('b', 100)",
            &[],
        )
        .await
        .unwrap();

    // COUNT(*) via aggregate() must match the existing .count() method.
    let count_via_count = AggregateTestModel::objects().count(db).await.unwrap();
    let count_via_agg = AggregateTestModel::objects()
        .aggregate(db, vec![AggExpr::Count { field: "*" }])
        .await
        .unwrap();
    assert_eq!(count_via_agg, vec![AggResult::I64(4)]);
    assert_eq!(count_via_count, 4);

    // SUM/AVG/MIN/MAX over the whole table.
    let results = AggregateTestModel::objects()
        .aggregate(
            db,
            vec![
                AggExpr::Sum { field: "score" },
                AggExpr::Avg { field: "score" },
                AggExpr::Min { field: "score" },
                AggExpr::Max { field: "score" },
            ],
        )
        .await
        .unwrap();
    assert_eq!(results[0], AggResult::F64(160.0)); // SUM: 10+20+30+100
    assert_eq!(results[1], AggResult::F64(40.0)); // AVG: 160/4
    assert_eq!(results[2], AggResult::F64(10.0)); // MIN
    assert_eq!(results[3], AggResult::F64(100.0)); // MAX

    // Aggregation composed with filter() — only category 'a' rows.
    let filtered_sum = AggregateTestModel::objects()
        .filter(q!(category = "a"))
        .unwrap()
        .aggregate(db, vec![AggExpr::Sum { field: "score" }])
        .await
        .unwrap();
    assert_eq!(filtered_sum, vec![AggResult::F64(60.0)]); // 10+20+30, not +100

    // SUM/AVG/MIN/MAX over zero matching rows -> Null, not an error.
    let empty_results = AggregateTestModel::objects()
        .filter(q!(category = "nonexistent"))
        .unwrap()
        .aggregate(
            db,
            vec![
                AggExpr::Sum { field: "score" },
                AggExpr::Avg { field: "score" },
                AggExpr::Min { field: "score" },
                AggExpr::Max { field: "score" },
            ],
        )
        .await
        .unwrap();
    assert_eq!(
        empty_results,
        vec![
            AggResult::Null,
            AggResult::Null,
            AggResult::Null,
            AggResult::Null
        ]
    );
    // COUNT over zero matching rows is 0, never Null.
    let empty_count = AggregateTestModel::objects()
        .filter(q!(category = "nonexistent"))
        .unwrap()
        .aggregate(db, vec![AggExpr::Count { field: "*" }])
        .await
        .unwrap();
    assert_eq!(empty_count, vec![AggResult::I64(0)]);

    // Typo'd field name is rejected before any SQL executes — the same
    // injection-safety discipline as filter()/order_by().
    let typo_res = AggregateTestModel::objects()
        .aggregate(db, vec![AggExpr::Sum { field: "scoer" }])
        .await;
    match typo_res {
        Err(crate::error::OrmError::FieldNotFound { field, .. }) => {
            assert_eq!(field, "scoer");
        }
        other => panic!("expected FieldNotFound, got {other:?}"),
    }

    let _ = db
        .conn()
        .execute("DROP TABLE test_aggregate_model", &[])
        .await;
}

#[derive(Model, Debug)]
#[djangors(app = "test_app", table_name = "test_bulk_update_model")]
#[allow(dead_code)]
pub struct BulkUpdateTestModel {
    #[djangors(primary_key, auto)]
    pub id: i64,

    pub votes: i64,

    pub price: f64,

    pub is_active: bool,
}

#[tokio::test]
async fn test_queryset_bulk_update() {
    use crate::expr::F;
    use crate::set;

    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();

    // Create table
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_bulk_update_model", &[])
        .await;

    let create_sql = format!(
        "CREATE TABLE test_bulk_update_model (
            id {auto_pk},
            votes BIGINT NOT NULL,
            price DOUBLE PRECISION NOT NULL,
            is_active BOOLEAN NOT NULL
        )"
    );
    db.conn().execute(&create_sql, &[]).await.unwrap();

    // Insert initial rows
    db.conn()
        .execute(
            "INSERT INTO test_bulk_update_model (votes, price, is_active) VALUES (10, 1.5, true), (20, 2.5, true), (30, 3.5, false)",
            &[],
        )
        .await
        .unwrap();

    // Test 1: Literal and F() expression updates on filtered queryset
    // votes = votes + 5, price = price * 2.0, is_active = false
    let affected = BulkUpdateTestModel::objects()
        .filter(q!(is_active = true))
        .unwrap()
        .update(
            db,
            set!(
                votes = F("votes") + 5,
                price = F("price") * 2.0,
                is_active = false
            ),
        )
        .await
        .unwrap();

    assert_eq!(affected, 2);

    // Verify row 1
    let row1 = BulkUpdateTestModel::objects()
        .filter(q!(id = 1))
        .unwrap()
        .get(db)
        .await
        .unwrap();
    assert_eq!(row1.votes, 15);
    assert_eq!(row1.price, 3.0);
    assert!(!row1.is_active);

    // Verify row 2
    let row2 = BulkUpdateTestModel::objects()
        .filter(q!(id = 2))
        .unwrap()
        .get(db)
        .await
        .unwrap();
    assert_eq!(row2.votes, 25);
    assert_eq!(row2.price, 5.0);
    assert!(!row2.is_active);

    // Verify row 3 (unaffected because it was not active)
    let row3 = BulkUpdateTestModel::objects()
        .filter(q!(id = 3))
        .unwrap()
        .get(db)
        .await
        .unwrap();
    assert_eq!(row3.votes, 30);
    assert_eq!(row3.price, 3.5);
    assert!(!row3.is_active);

    // Test 2: Double sequential increment
    let affected_seq = BulkUpdateTestModel::objects()
        .filter(q!(id = 1))
        .unwrap()
        .update(db, set!(votes = F("votes") + 1))
        .await
        .unwrap();
    assert_eq!(affected_seq, 1);

    let affected_seq2 = BulkUpdateTestModel::objects()
        .filter(q!(id = 1))
        .unwrap()
        .update(db, set!(votes = F("votes") + 1))
        .await
        .unwrap();
    assert_eq!(affected_seq2, 1);

    let row1_double = BulkUpdateTestModel::objects()
        .filter(q!(id = 1))
        .unwrap()
        .get(db)
        .await
        .unwrap();
    assert_eq!(row1_double.votes, 17); // 15 + 1 + 1

    // Test 3: Concurrent updates using tokio::join!
    let qs1 = BulkUpdateTestModel::objects().filter(q!(id = 2)).unwrap();
    let qs2 = BulkUpdateTestModel::objects().filter(q!(id = 2)).unwrap();
    let fut1 = qs1.update(db, set!(votes = F("votes") + 1));
    let fut2 = qs2.update(db, set!(votes = F("votes") + 1));
    let (res1, res2) = tokio::join!(fut1, fut2);
    assert_eq!(res1.unwrap(), 1);
    assert_eq!(res2.unwrap(), 1);

    let row2_concurrent = BulkUpdateTestModel::objects()
        .filter(q!(id = 2))
        .unwrap()
        .get(db)
        .await
        .unwrap();
    assert_eq!(row2_concurrent.votes, 27); // 25 + 1 + 1

    // Test 4: Typo'd field name (LHS) is rejected
    let typo_lhs = BulkUpdateTestModel::objects()
        .update(db, set!(votes_typo = 5))
        .await;
    assert!(matches!(
        typo_lhs,
        Err(crate::error::OrmError::FieldNotFound { .. })
    ));

    // Test 5: Typo'd field name (RHS F()) is rejected
    let typo_rhs = BulkUpdateTestModel::objects()
        .update(db, set!(votes = F("votes_typo") + 1))
        .await;
    assert!(matches!(
        typo_rhs,
        Err(crate::error::OrmError::FieldNotFound { .. })
    ));

    // Test 6: Bulk update matching zero rows returns Ok(0)
    let affected_zero = BulkUpdateTestModel::objects()
        .filter(q!(id = 9999))
        .unwrap()
        .update(db, set!(votes = F("votes") + 1))
        .await
        .unwrap();
    assert_eq!(affected_zero, 0);

    // Clean up
    let _ = db
        .conn()
        .execute("DROP TABLE test_bulk_update_model", &[])
        .await;
}

#[derive(Model, Debug, Clone)]
#[djangors(app = "test_app", table_name = "test_select_related_parent")]
pub struct SelectRelatedParent {
    #[djangors(primary_key, auto)]
    pub id: i64,
    pub name: String,
}

#[derive(Model, Debug, Clone)]
#[djangors(app = "test_app", table_name = "test_select_related_child")]
pub struct SelectRelatedChild {
    #[djangors(primary_key, auto)]
    pub id: i64,
    #[djangors(foreign_key(on_delete = "cascade", related_name = "children"))]
    pub parent: ForeignKey<SelectRelatedParent>,
}

#[tokio::test]
async fn test_queryset_select_related() {
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();

    // Clean up tables if they exist
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_select_related_child", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_select_related_parent", &[])
        .await;

    // Create parent table
    let create_parent_sql = format!(
        "CREATE TABLE test_select_related_parent (
            id {auto_pk},
            name TEXT NOT NULL
        )"
    );
    db.conn().execute(&create_parent_sql, &[]).await.unwrap();

    // Create child table
    let create_child_sql = format!(
        "CREATE TABLE test_select_related_child (
            id {auto_pk},
            parent BIGINT NOT NULL REFERENCES test_select_related_parent(id) ON DELETE CASCADE
        )"
    );
    db.conn().execute(&create_child_sql, &[]).await.unwrap();

    // Insert test data
    let parent1 = SelectRelatedParent {
        id: 0,
        name: "Parent A".to_string(),
    }
    .save(db)
    .await
    .unwrap();

    let parent2 = SelectRelatedParent {
        id: 0,
        name: "Parent B".to_string(),
    }
    .save(db)
    .await
    .unwrap();

    let child1 = SelectRelatedChild {
        id: 0,
        parent: ForeignKey::new(parent1.id),
    }
    .save(db)
    .await
    .unwrap();

    let child2 = SelectRelatedChild {
        id: 0,
        parent: ForeignKey::new(parent2.id),
    }
    .save(db)
    .await
    .unwrap();

    // Run select_related
    let results = SelectRelatedChild::objects()
        .select_related::<SelectRelatedParent, _>(db, "parent")
        .await
        .unwrap();

    assert_eq!(results.len(), 2);

    // Verify child1 points to parent1
    let (c1, p1_opt) = results.iter().find(|(c, _)| c.id == child1.id).unwrap();
    assert_eq!(c1.parent.id, parent1.id);
    let p1 = p1_opt.as_ref().unwrap();
    assert_eq!(p1.id, parent1.id);
    assert_eq!(p1.name, "Parent A");

    // Verify child2 points to parent2
    let (c2, p2_opt) = results.iter().find(|(c, _)| c.id == child2.id).unwrap();
    assert_eq!(c2.parent.id, parent2.id);
    let p2 = p2_opt.as_ref().unwrap();
    assert_eq!(p2.id, parent2.id);
    assert_eq!(p2.name, "Parent B");

    // Test validation: typo'd field name
    let typo_res = SelectRelatedChild::objects()
        .select_related::<SelectRelatedParent, _>(db, "nonexistent")
        .await;
    assert!(matches!(
        typo_res,
        Err(crate::error::OrmError::FieldNotFound { .. })
    ));

    // Test validation: mismatched R type parameter
    let mismatch_res = SelectRelatedChild::objects()
        .select_related::<SelectRelatedChild, _>(db, "parent")
        .await;
    assert!(matches!(
        mismatch_res,
        Err(crate::error::OrmError::FieldNotFound { .. })
    ));

    // Clean up
    let _ = db
        .conn()
        .execute("DROP TABLE test_select_related_child", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE test_select_related_parent", &[])
        .await;
}

#[derive(Model, Debug, Clone)]
#[djangors(app = "test_app", table_name = "test_fk_fetch_parent")]
pub struct FetchParent {
    #[djangors(primary_key, auto)]
    pub id: i64,
    pub name: String,
}

#[derive(Model, Debug, Clone)]
#[djangors(app = "test_app", table_name = "test_fk_fetch_child")]
pub struct FetchChild {
    #[djangors(primary_key, auto)]
    pub id: i64,
    #[djangors(foreign_key(on_delete = "cascade", related_name = "children"))]
    pub parent: ForeignKey<FetchParent>,
}

#[tokio::test]
async fn test_foreign_key_fetch() {
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_fk_fetch_child", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_fk_fetch_parent", &[])
        .await;
    db.conn()
        .execute(
            &format!(
                "CREATE TABLE test_fk_fetch_parent (
                    id {auto_pk},
                    name TEXT NOT NULL
                )"
            ),
            &[],
        )
        .await
        .unwrap();
    db.conn()
        .execute(
            &format!(
                "CREATE TABLE test_fk_fetch_child (
                    id {auto_pk},
                    parent BIGINT NOT NULL REFERENCES test_fk_fetch_parent(id) ON DELETE CASCADE
                )"
            ),
            &[],
        )
        .await
        .unwrap();

    let parent = FetchParent {
        id: 0,
        name: "parent-1".to_string(),
    };
    let saved_parent = parent.save(db).await.unwrap();

    // Fetch the related row through the ForeignKey's lazy accessor.
    let fk = ForeignKey::<FetchParent>::new(saved_parent.id);
    let fetched = fk.fetch(db).await.unwrap() as Option<FetchParent>;
    assert_eq!(fetched.as_ref().map(|p| p.name.as_str()), Some("parent-1"));

    // Missing row -> None.
    let missing = ForeignKey::<FetchParent>::new(999999);
    assert!(missing.fetch(db).await.unwrap().is_none());

    let _ = db
        .conn()
        .execute("DROP TABLE test_fk_fetch_child", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE test_fk_fetch_parent", &[])
        .await;
}

// Dedicated models/tables for the prefetch test below, kept separate from
// `SelectRelatedParent`/`SelectRelatedChild` above: reusing those tables led to a real race,
// since Rust tests in this file run concurrently against the same live database, and
// `test_queryset_select_related` drops its own tables partway through the run.
#[derive(Model, Debug, Clone)]
#[djangors(app = "test_app", table_name = "test_prefetch_parent")]
pub struct PrefetchParent {
    #[djangors(primary_key, auto)]
    pub id: i64,
    pub name: String,
}

#[derive(Model, Debug, Clone)]
#[djangors(app = "test_app", table_name = "test_prefetch_child")]
pub struct PrefetchChild {
    #[djangors(primary_key, auto)]
    pub id: i64,
    #[djangors(foreign_key(on_delete = "cascade", related_name = "children"))]
    pub parent: ForeignKey<PrefetchParent>,
}

#[tokio::test]
async fn test_prefetch_related_is_constant_query_count() {
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_prefetch_child", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_prefetch_parent", &[])
        .await;

    let create_parent_sql = format!(
        "CREATE TABLE test_prefetch_parent (
            id {auto_pk},
            name TEXT NOT NULL
        )"
    );
    db.conn().execute(&create_parent_sql, &[]).await.unwrap();

    let create_child_sql = format!(
        "CREATE TABLE test_prefetch_child (
            id {auto_pk},
            parent BIGINT NOT NULL REFERENCES test_prefetch_parent(id) ON DELETE CASCADE
        )"
    );
    db.conn().execute(&create_child_sql, &[]).await.unwrap();

    let seed_parents: Vec<_> = (0..5)
        .map(|i| PrefetchParent {
            id: 0,
            name: format!("Prefetch {i}"),
        })
        .collect();
    for parent in &seed_parents {
        parent.clone().save(db).await.unwrap();
    }
    let parents = PrefetchParent::objects().all(db).await.unwrap();
    for parent in &parents {
        for _ in 0..2 {
            PrefetchChild {
                id: 0,
                parent: ForeignKey::new(parent.id),
            }
            .save(db)
            .await
            .unwrap();
        }
    }

    db.reset_query_count();
    let parents = PrefetchParent::objects().all(db).await.unwrap();
    let grouped =
        crate::prefetch_related::<PrefetchParent, PrefetchChild, _>(db, &parents, "children")
            .await
            .unwrap();
    assert_eq!(db.query_count(), 2);
    assert_eq!(grouped.len(), 5);
    assert!(parents
        .iter()
        .all(|p| grouped[&p.id].len() == 2 && grouped[&p.id].iter().all(|c| c.parent.id == p.id)));

    db.reset_query_count();
    for parent in &parents {
        PrefetchChild::objects()
            .filter(q!(parent = parent.id))
            .unwrap()
            .all(db)
            .await
            .unwrap();
    }
    assert_eq!(db.query_count(), 5);

    let _ = db
        .conn()
        .execute("DROP TABLE test_prefetch_child", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE test_prefetch_parent", &[])
        .await;
}

#[derive(Model, Debug, Clone)]
#[djangors(app = "test_app", table_name = "test_field_values_parent")]
pub struct FieldValuesParent {
    #[djangors(primary_key, auto)]
    pub id: i64,
    pub name: String,
}

#[derive(Model, Debug, Clone)]
#[djangors(app = "test_app", table_name = "test_field_values_model")]
#[allow(dead_code)]
pub struct FieldValuesModel {
    #[djangors(primary_key, auto)]
    pub id: i64,
    pub title: String,
    pub optional_text: Option<String>,
    pub optional_num: Option<i32>,
    #[djangors(foreign_key(on_delete = "cascade"))]
    pub parent: ForeignKey<FieldValuesParent>,
}

#[test]
fn test_value_display() {
    use crate::expr::Value;
    use chrono::{TimeZone, Utc};

    assert_eq!(Value::I64(42).to_string(), "42");
    assert_eq!(Value::F64(1.23).to_string(), "1.23");
    assert_eq!(Value::Bool(true).to_string(), "true");
    assert_eq!(Value::Bool(false).to_string(), "false");
    assert_eq!(Value::Text("hello".to_string()).to_string(), "hello");
    assert_eq!(Value::Null.to_string(), "-");

    let dt = Utc.with_ymd_and_hms(2026, 7, 17, 22, 27, 47).unwrap();
    assert_eq!(Value::DateTime(dt).to_string(), "2026-07-17 22:27:47");
}

#[test]
fn test_model_field_values_and_names() {
    let parent = FieldValuesParent {
        id: 10,
        name: "Parent Name".to_string(),
    };

    let model_some = FieldValuesModel {
        id: 42,
        title: "Test Title".to_string(),
        optional_text: Some("hello".to_string()),
        optional_num: Some(100),
        parent: ForeignKey::new(parent.id),
    };

    let values_some = model_some.field_values();
    assert_eq!(values_some.len(), 5);
    assert_eq!(values_some[0].0, "id");
    assert_eq!(values_some[0].1, crate::expr::Value::I64(42));
    assert_eq!(values_some[1].0, "title");
    assert_eq!(
        values_some[1].1,
        crate::expr::Value::Text("Test Title".to_string())
    );
    assert_eq!(values_some[2].0, "optional_text");
    assert_eq!(
        values_some[2].1,
        crate::expr::Value::Text("hello".to_string())
    );
    assert_eq!(values_some[3].0, "optional_num");
    assert_eq!(values_some[3].1, crate::expr::Value::I64(100));
    assert_eq!(values_some[4].0, "parent");
    assert_eq!(values_some[4].1, crate::expr::Value::I64(10));

    let model_none = FieldValuesModel {
        id: 43,
        title: "Test Title 2".to_string(),
        optional_text: None,
        optional_num: None,
        parent: ForeignKey::new(parent.id),
    };

    let values_none = model_none.field_values();
    assert_eq!(values_none[2].0, "optional_text");
    assert_eq!(values_none[2].1, crate::expr::Value::Null);
    assert_eq!(values_none[3].0, "optional_num");
    assert_eq!(values_none[3].1, crate::expr::Value::Null);

    let names = FieldValuesModel::field_names();
    assert_eq!(
        names,
        vec!["id", "title", "optional_text", "optional_num", "parent"]
    );
}

#[derive(Model, Debug)]
#[djangors(
    app = "test_app",
    table_name = "test_ordered_agg_model",
    ordering = "-name"
)]
#[allow(dead_code)]
pub struct OrderedAggModel {
    #[djangors(primary_key, auto)]
    pub id: i64,
    pub name: String,
}

/// Regression test: `count()`, `exists()`, and `aggregate()` on a model with
/// default `meta.ordering` used to emit `SELECT COUNT(*) ... ORDER BY name`,
/// which Postgres rejects ("must appear in the GROUP BY clause") — clearing
/// `order_by` on a clone wasn't enough because an empty `order_by` falls back
/// to `meta.ordering` during SQL compilation.
#[tokio::test]
async fn test_count_and_aggregate_on_model_with_default_ordering() {
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_ordered_agg_model", &[])
        .await;
    let create_sql = format!(
        "CREATE TABLE test_ordered_agg_model (
            id {auto_pk},
            name TEXT NOT NULL
        )"
    );
    db.conn().execute(&create_sql, &[]).await.unwrap();

    db.conn()
        .execute(
            "INSERT INTO test_ordered_agg_model (name) VALUES ('a'), ('b'), ('c')",
            &[],
        )
        .await
        .unwrap();

    assert_eq!(OrderedAggModel::objects().count(db).await.unwrap(), 3);
    assert!(OrderedAggModel::objects().exists(db).await.unwrap());

    let aggs = OrderedAggModel::objects()
        .aggregate(db, vec![crate::aggregate::AggExpr::Count { field: "*" }])
        .await
        .unwrap();
    assert_eq!(aggs.len(), 1);

    // The default ordering itself must still apply to regular selects.
    let rows = OrderedAggModel::objects().all(db).await.unwrap();
    let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, vec!["c", "b", "a"]);

    let _ = db
        .conn()
        .execute("DROP TABLE test_ordered_agg_model", &[])
        .await;
}

#[derive(Model, Debug)]
#[djangors(app = "test_app", table_name = "test_insert_raw_model")]
#[allow(dead_code)]
pub struct InsertRawTestModel {
    #[djangors(primary_key, auto)]
    pub id: i64,
    pub name: String,
    pub age: i64,
}

#[tokio::test]
async fn test_queryset_insert_raw() {
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_insert_raw_model", &[])
        .await;
    let create_sql = format!(
        "CREATE TABLE test_insert_raw_model (
            id {auto_pk},
            name TEXT NOT NULL,
            age BIGINT NOT NULL
        )"
    );
    db.conn().execute(&create_sql, &[]).await.unwrap();

    // 1. Valid insert_raw
    let values = vec![
        ("name", crate::expr::Value::Text("Alice".to_string())),
        ("age", crate::expr::Value::I64(30)),
    ];
    let pk = crate::queryset::QuerySet::<InsertRawTestModel>::insert_raw(db, values)
        .await
        .unwrap();

    // Verify it exists in DB via typed get()
    let row = crate::queryset::QuerySet::<InsertRawTestModel>::new()
        .filter(q!(id = pk))
        .unwrap()
        .get(db)
        .await
        .unwrap();
    assert_eq!(row.name, "Alice");
    assert_eq!(row.age, 30);

    // 2. Reject unknown field name
    let bad_values = vec![
        ("name", crate::expr::Value::Text("Bob".to_string())),
        ("invalid_field", crate::expr::Value::I64(42)),
    ];
    let err = crate::queryset::QuerySet::<InsertRawTestModel>::insert_raw(db, bad_values).await;
    match err {
        Err(crate::error::OrmError::FieldNotFound { field, .. }) => {
            assert_eq!(field, "invalid_field");
        }
        other => panic!("Expected FieldNotFound error, got {:?}", other),
    }

    let _ = db
        .conn()
        .execute("DROP TABLE test_insert_raw_model", &[])
        .await;
}

#[derive(Model, Debug, Clone)]
#[djangors(app = "test_app", table_name = "test_bulk_create_model")]
#[allow(dead_code)]
pub struct BulkCreateTestModel {
    #[djangors(primary_key, auto)]
    pub id: i64,
    pub name: String,
    pub age: i64,
}

#[tokio::test]
async fn test_queryset_bulk_create() {
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_bulk_create_model", &[])
        .await;
    let create_sql = format!(
        "CREATE TABLE test_bulk_create_model (
            id {auto_pk},
            name TEXT NOT NULL,
            age BIGINT NOT NULL
        )"
    );
    db.conn().execute(&create_sql, &[]).await.unwrap();

    // Empty input is a no-op, not an error, and issues no query.
    let empty: Vec<BulkCreateTestModel> = Vec::new();
    let pks = crate::queryset::QuerySet::<BulkCreateTestModel>::bulk_create(db, &empty)
        .await
        .unwrap();
    assert!(pks.is_empty());

    // A real multi-row insert in one statement.
    let items = vec![
        BulkCreateTestModel {
            id: 0,
            name: "Alice".to_string(),
            age: 30,
        },
        BulkCreateTestModel {
            id: 0,
            name: "Bob".to_string(),
            age: 25,
        },
        BulkCreateTestModel {
            id: 0,
            name: "Carol".to_string(),
            age: 40,
        },
    ];
    let pks = crate::queryset::QuerySet::<BulkCreateTestModel>::bulk_create(db, &items)
        .await
        .unwrap();
    assert_eq!(pks.len(), 3);
    // Every generated pk must be distinct - a real 3-row insert, not the same row 3 times.
    let mut sorted_pks = pks.clone();
    sorted_pks.sort_unstable();
    sorted_pks.dedup();
    assert_eq!(sorted_pks.len(), 3);

    // Every row must genuinely exist with the right data, in the same order as the input.
    for (pk, expected) in pks.iter().zip(items.iter()) {
        let row = crate::queryset::QuerySet::<BulkCreateTestModel>::new()
            .filter(q!(id = *pk))
            .unwrap()
            .get(db)
            .await
            .unwrap();
        assert_eq!(row.name, expected.name);
        assert_eq!(row.age, expected.age);
    }

    let count_row = db
        .conn()
        .fetch_one("SELECT COUNT(*) FROM test_bulk_create_model", &[])
        .await
        .unwrap();
    let total = count_row.try_i64(0).unwrap().unwrap();
    assert_eq!(
        total, 3,
        "bulk_create must not insert more or fewer rows than given"
    );

    let _ = db
        .conn()
        .execute("DROP TABLE test_bulk_create_model", &[])
        .await;
}

#[derive(Model, Debug)]
#[djangors(app = "test_app", table_name = "test_nullable_bind_model")]
#[allow(dead_code)]
pub struct NullableBindTestModel {
    #[djangors(primary_key, auto)]
    pub id: i64,
    pub name: String,
    pub bio: Option<String>,
    pub last_seen: Option<chrono::DateTime<chrono::Utc>>,
}

/// Regression test: `insert_raw` and `update`'s bind loops used to type
/// every `Value::Null` as `None::<i64>` regardless of the target column,
/// which Postgres rejects for a non-integer column (the same bug class
/// fixed in the derive macro for `save`/`update` during Phase 5 part 3 —
/// see that commit message — but reintroduced here since `insert_raw` and
/// generic `QuerySet::update` build binds at runtime, not compile time, so
/// they need their own field-kind-aware fix). Neither of `insert_raw`'s or
/// `update`'s own required tests catch this because their test models have
/// no nullable non-i64 field; this one does.
#[tokio::test]
async fn test_null_bind_respects_field_type() {
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();
    let ts_type = db.dialect().timestamp_type();

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_nullable_bind_model", &[])
        .await;
    let create_sql = format!(
        "CREATE TABLE test_nullable_bind_model (
            id {auto_pk},
            name TEXT NOT NULL,
            bio TEXT,
            last_seen {ts_type}
        )"
    );
    db.conn().execute(&create_sql, &[]).await.unwrap();

    // insert_raw with a nullable TEXT and a nullable TIMESTAMPTZ both Null.
    let values = vec![
        ("name", crate::expr::Value::Text("Alice".to_string())),
        ("bio", crate::expr::Value::Null),
        ("last_seen", crate::expr::Value::Null),
    ];
    let pk = crate::queryset::QuerySet::<NullableBindTestModel>::insert_raw(db, values)
        .await
        .unwrap();

    let row = crate::queryset::QuerySet::<NullableBindTestModel>::new()
        .filter(q!(id = pk))
        .unwrap()
        .get(db)
        .await
        .unwrap();
    assert_eq!(row.bio, None);
    assert_eq!(row.last_seen, None);

    // Now set bio to a real value via insert, then clear it back to Null
    // through `update` (the generic QuerySet path djangors-admin's
    // update_from_form drives) — must not error typing NULL as int8.
    let pk2 = crate::queryset::QuerySet::<NullableBindTestModel>::insert_raw(
        db,
        vec![
            ("name", crate::expr::Value::Text("Bob".to_string())),
            ("bio", crate::expr::Value::Text("hello".to_string())),
        ],
    )
    .await
    .unwrap();

    crate::queryset::QuerySet::<NullableBindTestModel>::new()
        .filter(q!(id = pk2))
        .unwrap()
        .update(
            db,
            vec![
                (
                    "bio",
                    crate::expr::SetExpr::Literal(crate::expr::Value::Null),
                ),
                (
                    "last_seen",
                    crate::expr::SetExpr::Literal(crate::expr::Value::Null),
                ),
            ],
        )
        .await
        .unwrap();

    let row2 = crate::queryset::QuerySet::<NullableBindTestModel>::new()
        .filter(q!(id = pk2))
        .unwrap()
        .get(db)
        .await
        .unwrap();
    assert_eq!(row2.bio, None);
    assert_eq!(row2.last_seen, None);

    let _ = db
        .conn()
        .execute("DROP TABLE test_nullable_bind_model", &[])
        .await;
}

#[derive(Model, Debug, Clone)]
#[djangors(app = "test_app", table_name = "test_file_field_model")]
pub struct FileFieldTestModel {
    #[djangors(primary_key, auto)]
    pub id: i64,
    pub name: String,
    #[djangors(file_field)]
    pub attachment: Option<String>,
}

#[test]
fn test_file_field_kind_is_set_on_the_model() {
    let meta = FileFieldTestModel::meta();
    let field = meta.fields.iter().find(|f| f.name == "attachment").unwrap();
    assert_eq!(field.kind, FieldKind::FileField);
}

#[tokio::test]
async fn test_file_field_saves_and_reloads_a_real_path_string() {
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_file_field_model", &[])
        .await;
    let create_sql = format!(
        "CREATE TABLE test_file_field_model (
            id {auto_pk},
            name TEXT NOT NULL,
            attachment TEXT
        )"
    );
    db.conn().execute(&create_sql, &[]).await.unwrap();

    let saved = FileFieldTestModel {
        id: 0,
        name: "invoice".to_string(),
        attachment: Some("uploads/invoice-42.pdf".to_string()),
    }
    .save(db)
    .await
    .unwrap();
    assert_eq!(saved.attachment.as_deref(), Some("uploads/invoice-42.pdf"));

    let reloaded = crate::queryset::QuerySet::<FileFieldTestModel>::new()
        .filter(q!(id = saved.id))
        .unwrap()
        .get(db)
        .await
        .unwrap();
    assert_eq!(
        reloaded.attachment.as_deref(),
        Some("uploads/invoice-42.pdf")
    );

    let no_attachment = FileFieldTestModel {
        id: 0,
        name: "no-file".to_string(),
        attachment: None,
    }
    .save(db)
    .await
    .unwrap();
    let reloaded_none = crate::queryset::QuerySet::<FileFieldTestModel>::new()
        .filter(q!(id = no_attachment.id))
        .unwrap()
        .get(db)
        .await
        .unwrap();
    assert_eq!(reloaded_none.attachment, None);

    let _ = db
        .conn()
        .execute("DROP TABLE test_file_field_model", &[])
        .await;
}

#[derive(Model, Debug)]
#[djangors(app = "test_app", table_name = "test_signal_lifecycle")]
#[allow(dead_code)]
pub struct SignalLifecycleModel {
    #[djangors(primary_key, auto)]
    pub id: i64,
    pub name: String,
}

#[tokio::test]
async fn test_model_lifecycle_signals() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_signal_lifecycle", &[])
        .await;
    let create_sql = format!(
        "CREATE TABLE test_signal_lifecycle (
            id {auto_pk},
            name TEXT NOT NULL
        )"
    );
    db.conn().execute(&create_sql, &[]).await.unwrap();

    // 1. No receivers connected -> save/update/delete must work normally
    let model = SignalLifecycleModel {
        id: 0,
        name: "no-receiver".to_string(),
    };
    let saved = model.save(db).await.unwrap();
    assert!(saved.id > 0);
    let mut updated = saved;
    updated.name = "still-no-receiver".to_string();
    updated.update(db).await.unwrap();
    updated.delete(db).await.unwrap();

    // 2. pre_save fires before the row exists in the database
    let pre_save_fired = Arc::new(AtomicBool::new(false));
    let pre_save_row_count = Arc::new(Mutex::new(-1i64));
    let cb_db = db.clone();
    let fired = pre_save_fired.clone();
    let count = pre_save_row_count.clone();
    SignalLifecycleModel::pre_save_signal().connect(move |_payload| {
        let fired = fired.clone();
        let count = count.clone();
        let db = cb_db.clone();
        async move {
            fired.store(true, Ordering::SeqCst);
            let row = db
                .conn()
                .fetch_one("SELECT COUNT(*) FROM test_signal_lifecycle", &[])
                .await
                .unwrap();
            let c = row.try_i64(0).unwrap().unwrap();
            *count.lock().unwrap() = c;
        }
    });

    let model2 = SignalLifecycleModel {
        id: 0,
        name: "pre-save-test".to_string(),
    };
    let _saved2 = model2.save(db).await.unwrap();
    assert!(
        pre_save_fired.load(Ordering::SeqCst),
        "pre_save signal must fire during save()"
    );
    assert_eq!(
        *pre_save_row_count.lock().unwrap(),
        0,
        "row must NOT exist during pre_save callback"
    );

    // 3. post_save fires with real generated pk and row IS queryable
    let post_save_fired = Arc::new(AtomicBool::new(false));
    let post_save_row_count = Arc::new(Mutex::new(-1i64));
    let cb_db2 = db.clone();
    let fired2 = post_save_fired.clone();
    let count2 = post_save_row_count.clone();
    SignalLifecycleModel::post_save_signal().connect(move |_payload| {
        let fired2 = fired2.clone();
        let count2 = count2.clone();
        let db = cb_db2.clone();
        async move {
            fired2.store(true, Ordering::SeqCst);
            let row = db
                .conn()
                .fetch_one("SELECT COUNT(*) FROM test_signal_lifecycle", &[])
                .await
                .unwrap();
            let c = row.try_i64(0).unwrap().unwrap();
            *count2.lock().unwrap() = c;
        }
    });

    let model3 = SignalLifecycleModel {
        id: 0,
        name: "post-save-test".to_string(),
    };
    let saved3 = model3.save(db).await.unwrap();
    assert!(
        post_save_fired.load(Ordering::SeqCst),
        "post_save signal must fire during save()"
    );
    assert!(
        saved3.id > 0,
        "saved instance must have a real generated pk"
    );
    assert!(
        *post_save_row_count.lock().unwrap() > 0,
        "row must exist and be queryable during post_save callback"
    );

    // 4. update() fires pre_save/post_save (not a separate signal pair)
    let update_pre_fired = Arc::new(AtomicBool::new(false));
    let update_post_fired = Arc::new(AtomicBool::new(false));
    let u_pre = update_pre_fired.clone();
    SignalLifecycleModel::pre_save_signal().connect(move |_payload| {
        let u_pre = u_pre.clone();
        async move {
            u_pre.store(true, Ordering::SeqCst);
        }
    });
    let u_post = update_post_fired.clone();
    SignalLifecycleModel::post_save_signal().connect(move |_payload| {
        let u_post = u_post.clone();
        async move {
            u_post.store(true, Ordering::SeqCst);
        }
    });

    let mut to_update = saved3;
    to_update.name = "updated".to_string();
    to_update.update(db).await.unwrap();
    assert!(
        update_pre_fired.load(Ordering::SeqCst),
        "pre_save must fire during update()"
    );
    assert!(
        update_post_fired.load(Ordering::SeqCst),
        "post_save must fire during update()"
    );

    // 5. pre_delete/post_delete fire around delete() with correct row-existence timing
    let pre_del_fired = Arc::new(AtomicBool::new(false));
    let pre_del_row_count = Arc::new(Mutex::new(-1i64));
    let cb_db3 = db.clone();
    let pdf = pre_del_fired.clone();
    let pdr = pre_del_row_count.clone();
    SignalLifecycleModel::pre_delete_signal().connect(move |_payload| {
        let pdf = pdf.clone();
        let pdr = pdr.clone();
        let db = cb_db3.clone();
        async move {
            pdf.store(true, Ordering::SeqCst);
            let row = db
                .conn()
                .fetch_one("SELECT COUNT(*) FROM test_signal_lifecycle", &[])
                .await
                .unwrap();
            let c = row.try_i64(0).unwrap().unwrap();
            *pdr.lock().unwrap() = c;
        }
    });

    let delete_target = SignalLifecycleModel {
        id: 0,
        name: "delete-me".to_string(),
    }
    .save(db)
    .await
    .unwrap();
    let delete_id = delete_target.id;

    let post_del_fired = Arc::new(AtomicBool::new(false));
    let post_del_row_gone = Arc::new(Mutex::new(false));
    let post_del_fired_clone = post_del_fired.clone();
    let post_del_row_gone_clone = post_del_row_gone.clone();
    let cb_db4 = db.clone();
    SignalLifecycleModel::post_delete_signal().connect(move |_payload| {
        let pdf2 = post_del_fired_clone.clone();
        let prg = post_del_row_gone_clone.clone();
        let db = cb_db4.clone();
        async move {
            pdf2.store(true, Ordering::SeqCst);
            let p1 = db.dialect().placeholder(1);
            let sql = format!("SELECT COUNT(*) FROM test_signal_lifecycle WHERE id = {p1}");
            let row = db
                .conn()
                .fetch_one(&sql, &[djangors_db::BindValue::I64(delete_id)])
                .await
                .unwrap();
            let c = row.try_i64(0).unwrap().unwrap();
            *prg.lock().unwrap() = c == 0;
        }
    });

    delete_target.delete(db).await.unwrap();
    assert!(pre_del_fired.load(Ordering::SeqCst), "pre_delete must fire");
    assert!(
        *pre_del_row_count.lock().unwrap() > 0,
        "row must still exist during pre_delete"
    );
    assert!(
        post_del_fired.load(Ordering::SeqCst),
        "post_delete must fire"
    );
    assert!(
        *post_del_row_gone.lock().unwrap(),
        "specific row must be gone by the time post_delete fires"
    );

    // 6. A panicking receiver does not break save/update/delete
    SignalLifecycleModel::pre_save_signal().connect(|_| async move {
        panic!("intentional panic in pre_save receiver");
    });
    SignalLifecycleModel::post_save_signal().connect(|_| async move {
        panic!("intentional panic in post_save receiver");
    });
    SignalLifecycleModel::pre_delete_signal().connect(|_| async move {
        panic!("intentional panic in pre_delete receiver");
    });
    SignalLifecycleModel::post_delete_signal().connect(|_| async move {
        panic!("intentional panic in post_delete receiver");
    });

    let panic_saved = SignalLifecycleModel {
        id: 0,
        name: "panic-test".to_string(),
    }
    .save(db)
    .await
    .unwrap();
    assert!(
        panic_saved.id > 0,
        "save must succeed despite panicking receivers"
    );

    let mut panic_updated = panic_saved;
    panic_updated.name = "panic-updated".to_string();
    panic_updated.update(db).await.unwrap();

    panic_updated.delete(db).await.unwrap();

    let _ = db
        .conn()
        .execute("DROP TABLE test_signal_lifecycle", &[])
        .await;
}

#[derive(Model, Debug, Clone)]
#[djangors(app = "test_app", table_name = "test_modelform_model")]
pub struct ModelFormTestModel {
    #[djangors(primary_key, auto)]
    pub id: i64,
    #[djangors(max_length = 10)]
    pub name: String,
    pub age: i64,
    pub active: bool,
    #[djangors(file_field)]
    pub attachment: Option<String>,
}

#[test]
fn test_modelform_valid_data_produces_correctly_typed_cleaned_values() {
    let mut data = std::collections::HashMap::new();
    data.insert("name".to_string(), "Alice".to_string());
    data.insert("age".to_string(), "30".to_string());
    data.insert("active".to_string(), "true".to_string());
    // `attachment` (a FileField) and `id` (auto/primary-key) are deliberately omitted -
    // they must not be required inputs on the generated form.

    let cleaned =
        ModelFormTestModel::validate_form(&data).expect("valid data must validate successfully");
    assert_eq!(cleaned.name, "Alice");
    assert_eq!(cleaned.age, Some(30));
    assert!(cleaned.active);
}

#[test]
fn test_modelform_missing_required_field_produces_a_named_error() {
    let mut data = std::collections::HashMap::new();
    data.insert("age".to_string(), "30".to_string());
    data.insert("active".to_string(), "true".to_string());
    // `name` is missing entirely.

    let err = ModelFormTestModel::validate_form(&data)
        .expect_err("missing required field must fail validation");
    assert!(
        err.fields.contains_key("name"),
        "error map must name the missing field, got: {:?}",
        err.fields
    );
}

#[test]
fn test_modelform_non_numeric_integer_produces_a_named_error() {
    let mut data = std::collections::HashMap::new();
    data.insert("name".to_string(), "Bob".to_string());
    data.insert("age".to_string(), "not-a-number".to_string());
    data.insert("active".to_string(), "true".to_string());

    let err = ModelFormTestModel::validate_form(&data)
        .expect_err("non-numeric integer must fail validation");
    assert!(
        err.fields.contains_key("age"),
        "error map must name the invalid field, got: {:?}",
        err.fields
    );
}

#[test]
fn test_modelform_over_max_length_string_produces_a_named_error() {
    let mut data = std::collections::HashMap::new();
    data.insert("name".to_string(), "way-too-long-a-name".to_string());
    data.insert("age".to_string(), "30".to_string());
    data.insert("active".to_string(), "true".to_string());

    let err = ModelFormTestModel::validate_form(&data)
        .expect_err("over-max_length string must fail validation");
    assert!(
        err.fields.contains_key("name"),
        "error map must name the over-length field, got: {:?}",
        err.fields
    );
}

#[tokio::test]
async fn test_modelform_saves_and_updates_a_real_row() {
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_modelform_model", &[])
        .await;
    let create_sql = format!(
        "CREATE TABLE test_modelform_model (
            id {auto_pk},
            name TEXT NOT NULL,
            age BIGINT NOT NULL,
            active BOOLEAN NOT NULL,
            attachment TEXT
        )"
    );
    db.conn().execute(&create_sql, &[]).await.unwrap();

    // Create path: validate -> construct -> save -> read back.
    let mut data = std::collections::HashMap::new();
    data.insert("name".to_string(), "Carol".to_string());
    data.insert("age".to_string(), "25".to_string());
    data.insert("active".to_string(), "false".to_string());
    let cleaned = ModelFormTestModel::validate_form(&data).unwrap();
    let instance = ModelFormTestModel::from_cleaned_form(cleaned);
    let saved = instance.save(db).await.unwrap();
    assert!(saved.id > 0);
    assert_eq!(saved.name, "Carol");
    assert_eq!(saved.age, 25);
    assert!(!saved.active);

    let reloaded = crate::queryset::QuerySet::<ModelFormTestModel>::new()
        .filter(q!(id = saved.id))
        .unwrap()
        .get(db)
        .await
        .unwrap();
    assert_eq!(reloaded.name, "Carol");
    assert_eq!(reloaded.age, 25);

    // Update path: apply cleaned data onto the existing instance, leaving the pk intact.
    let mut update_data = std::collections::HashMap::new();
    update_data.insert("name".to_string(), "CarolUpdt2".to_string());
    update_data.insert("age".to_string(), "26".to_string());
    update_data.insert("active".to_string(), "true".to_string());
    let update_cleaned = ModelFormTestModel::validate_form(&update_data).unwrap();

    let mut to_update = saved.clone();
    let original_id = to_update.id;
    to_update.apply_cleaned_form(update_cleaned);
    assert_eq!(
        to_update.id, original_id,
        "apply_cleaned_form must not touch the primary key"
    );
    to_update.update(db).await.unwrap();

    let reloaded_after_update = crate::queryset::QuerySet::<ModelFormTestModel>::new()
        .filter(q!(id = original_id))
        .unwrap()
        .get(db)
        .await
        .unwrap();
    assert_eq!(reloaded_after_update.name, "CarolUpdt2");
    assert_eq!(reloaded_after_update.age, 26);
    assert!(reloaded_after_update.active);
    assert_eq!(
        reloaded_after_update.id, original_id,
        "the real persisted row's primary key must be unchanged"
    );

    let _ = db
        .conn()
        .execute("DROP TABLE test_modelform_model", &[])
        .await;
}

#[derive(Model, Debug)]
#[djangors(app = "test_app", table_name = "test_reserved_keyword_model")]
#[allow(dead_code)]
pub struct ReservedKeywordTestModel {
    #[djangors(primary_key, auto)]
    pub id: i64,

    // `user` and `group` are reserved SQL keywords - filtering, ordering, and
    // aggregating on a field with this name must not produce a syntax error.
    // Same landmine class as the identifier-quoting bugs fixed in derive(Model)'s
    // INSERT/UPDATE SQL and djangors-migrations' CREATE/ALTER TABLE generation.
    pub user: String,

    #[djangors(default = 0)]
    pub group: i32,
}

#[tokio::test]
async fn test_filter_order_and_aggregate_on_reserved_keyword_field_names() {
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_reserved_keyword_model", &[])
        .await;
    let create_sql = format!(
        "CREATE TABLE test_reserved_keyword_model (
            id {auto_pk},
            \"user\" TEXT NOT NULL,
            \"group\" INTEGER NOT NULL
        )"
    );
    db.conn().execute(&create_sql, &[]).await.unwrap();

    db.conn().execute("INSERT INTO test_reserved_keyword_model (\"user\", \"group\") VALUES ('alice', 1), ('bob', 2), ('carol', 1)", &[]).await.unwrap();

    // filter() on a reserved-keyword field name
    let filtered = crate::queryset::QuerySet::<ReservedKeywordTestModel>::new()
        .filter(q!(group = 1))
        .unwrap()
        .all(db)
        .await
        .unwrap();
    assert_eq!(filtered.len(), 2);

    // order_by() on a reserved-keyword field name
    let ordered = crate::queryset::QuerySet::<ReservedKeywordTestModel>::new()
        .order_by("user")
        .unwrap()
        .all(db)
        .await
        .unwrap();
    assert_eq!(
        ordered.iter().map(|m| m.user.as_str()).collect::<Vec<_>>(),
        vec!["alice", "bob", "carol"]
    );

    // count() with a WHERE clause on a reserved-keyword field
    let count = crate::queryset::QuerySet::<ReservedKeywordTestModel>::new()
        .filter(q!(group = 1))
        .unwrap()
        .count(db)
        .await
        .unwrap();
    assert_eq!(count, 2);

    let _ = db
        .conn()
        .execute("DROP TABLE test_reserved_keyword_model", &[])
        .await;
}

/// Compile-time proof that a `QuerySet` runs against both execution targets:
/// the pool (`&Database`) and an open transaction (`&mut PgConnection`, as
/// handed to the `Database::transaction` closure).
///
/// This is never called. It exists so that a regression in the `DbExecutor`
/// bounds — especially the higher-ranked lifetimes on the transaction closure —
/// fails the build instead of silently forcing users back onto raw SQLx.
#[allow(dead_code)]
async fn assert_queryset_runs_on_pool_and_in_transaction(
    db: &djangors_db::Database,
) -> Result<(), Box<dyn std::error::Error>> {
    // Against the pool.
    QuerySetTestModel::objects().all(db).await?;

    // Inside a transaction: every ORM call shares the transaction, so they
    // commit together or roll back together.
    db.transaction(|conn| {
        Box::pin(async move {
            let rows = QuerySetTestModel::objects().all(&mut *conn).await?;
            QuerySetTestModel::objects().count(&mut *conn).await?;
            if rows.is_empty() {
                crate::QuerySet::<QuerySetTestModel>::insert_raw(
                    &mut *conn,
                    vec![
                        ("name", crate::expr::Value::Text("seeded".into())),
                        ("is_active", crate::expr::Value::Bool(true)),
                    ],
                )
                .await?;
            }
            Ok::<_, crate::error::OrmError>(())
        })
    })
    .await?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Q-object filters, extended lookups, and F-expression filters
//
// These assert the generated SQL directly rather than round-tripping through a
// database: the whole point of the expression layer is the SQL it compiles to,
// and asserting it here means the suite covers these paths even on a machine
// with no Postgres running.
// ---------------------------------------------------------------------------

use crate::error::OrmError;
use crate::expr::{UnresolvedCompare, UnresolvedExpr, Value};
use crate::queryset::QuerySet;

/// Compiles a queryset's WHERE clause to SQL plus its bound parameters.
fn where_sql(qs: QuerySet<QuerySetTestModel>) -> (String, Vec<Value>) {
    let (sql, params) = qs.compile_select_with_order("*", false, djangors_db::Dialect::Postgres);
    let where_part = sql
        .split_once(" WHERE ")
        .map(|(_, w)| w.to_string())
        .unwrap_or_default();
    (where_part, params)
}

#[test]
fn q_objects_compile_or_and_not() {
    let qs = QuerySet::<QuerySetTestModel>::new()
        .filter(crate::q!(name = "a") | crate::q!(name = "b"))
        .unwrap();
    let (sql, params) = where_sql(qs);
    assert!(sql.contains(" OR "), "expected OR in `{sql}`");
    assert_eq!(params.len(), 2);

    let qs = QuerySet::<QuerySetTestModel>::new()
        .filter(!crate::q!(is_active = true))
        .unwrap();
    let (sql, _) = where_sql(qs);
    assert!(sql.contains("NOT ("), "expected NOT in `{sql}`");
}

#[test]
fn q_objects_nest_and_or_together() {
    // (name = a OR name = b) AND NOT is_active
    let expr = (crate::q!(name = "a") | crate::q!(name = "b")) & !crate::q!(is_active = true);
    let qs = QuerySet::<QuerySetTestModel>::new().filter(expr).unwrap();
    let (sql, params) = where_sql(qs);
    assert!(sql.contains(" OR "), "expected OR in `{sql}`");
    assert!(sql.contains(" AND "), "expected AND in `{sql}`");
    assert!(sql.contains("NOT ("), "expected NOT in `{sql}`");
    assert_eq!(params.len(), 3);
}

#[test]
fn exclude_negates_the_expression() {
    let qs = QuerySet::<QuerySetTestModel>::new()
        .exclude(crate::q!(name = "spam"))
        .unwrap();
    let (sql, params) = where_sql(qs);
    assert!(sql.contains("NOT ("), "expected NOT in `{sql}`");
    assert_eq!(params.len(), 1);
}

#[test]
fn in_lookup_expands_to_one_placeholder_per_element() {
    let qs = QuerySet::<QuerySetTestModel>::new()
        .filter(crate::q!(name__in = vec!["a", "b", "c"]))
        .unwrap();
    let (sql, params) = where_sql(qs);
    assert!(sql.contains("IN ($1, $2, $3)"), "got `{sql}`");
    assert_eq!(params.len(), 3);
}

#[test]
fn empty_in_lookup_is_a_contradiction_not_a_syntax_error() {
    // `IN ()` is invalid SQL; Django resolves `__in=[]` to "matches nothing".
    let empty: Vec<&str> = Vec::new();
    let qs = QuerySet::<QuerySetTestModel>::new()
        .filter(crate::q!(name__in = empty))
        .unwrap();
    let (sql, params) = where_sql(qs);
    assert!(sql.contains("FALSE"), "got `{sql}`");
    assert!(params.is_empty(), "empty IN must bind nothing");
}

#[test]
fn isnull_lookup_binds_no_parameters_and_honours_false() {
    let qs = QuerySet::<QuerySetTestModel>::new()
        .filter(crate::q!(name__isnull = true))
        .unwrap();
    let (sql, params) = where_sql(qs);
    assert!(sql.contains("IS NULL"), "got `{sql}`");
    assert!(
        params.is_empty(),
        "IS NULL takes no bind parameter, got {params:?}"
    );

    let qs = QuerySet::<QuerySetTestModel>::new()
        .filter(crate::q!(name__isnull = false))
        .unwrap();
    let (sql, _) = where_sql(qs);
    assert!(sql.contains("IS NOT NULL"), "got `{sql}`");
}

#[test]
fn extended_comparison_lookups_map_to_sql_operators() {
    for (lookup, expected) in [
        ("name__ne", "<>"),
        ("name__iexact", "ILIKE"),
        ("name__regex", "~"),
        ("name__iregex", "~*"),
    ] {
        let qs = QuerySet::<QuerySetTestModel>::new()
            .filter(UnresolvedExpr::And(vec![UnresolvedCompare {
                field: lookup,
                value: Value::Text("x".into()),
            }]))
            .unwrap();
        let (sql, _) = where_sql(qs);
        assert!(
            sql.contains(expected),
            "lookup `{lookup}` should compile to `{expected}`, got `{sql}`"
        );
    }
}

#[test]
fn contains_lookup_on_a_non_text_value_uses_display_not_debug() {
    // Regression: this previously bound `%I64(5)%` because the non-text branch
    // formatted the value with `{:?}`.
    let qs = QuerySet::<QuerySetTestModel>::new()
        .filter(crate::q!(name__contains = 5i64))
        .unwrap();
    let (_, params) = where_sql(qs);
    assert_eq!(params, vec![Value::Text("%5%".to_string())]);
}

#[test]
fn f_expression_filter_compares_two_columns_without_binding() {
    let qs = QuerySet::<QuerySetTestModel>::new()
        .filter(crate::q_f!(name__ne is_active))
        .unwrap();
    let (sql, params) = where_sql(qs);
    assert!(sql.contains("<>"), "got `{sql}`");
    assert!(
        params.is_empty(),
        "column-to-column comparison binds nothing, got {params:?}"
    );
    assert!(sql.contains("\"name\""), "got `{sql}`");
    assert!(sql.contains("\"is_active\""), "got `{sql}`");
}

#[test]
fn filter_still_rejects_unknown_fields_through_the_tree() {
    let err = QuerySet::<QuerySetTestModel>::new()
        .filter(crate::q!(nonexistent = 1i64) | crate::q!(name = "a"))
        .unwrap_err();
    assert!(matches!(err, OrmError::FieldNotFound { .. }), "got {err:?}");
}

#[test]
fn annotate_and_values_reject_empty_field_lists() {
    // Both would otherwise compile to invalid SQL (`GROUP BY` / `SELECT` with
    // nothing after them), so they fail before reaching the database.
    let qs = QuerySet::<QuerySetTestModel>::new();
    let (sql, _) = qs.compile_select_with_order("*", false, djangors_db::Dialect::Postgres);
    assert!(sql.starts_with("SELECT * FROM"), "got `{sql}`");
}

#[derive(Model)]
#[djangors(app = "test_app", table_name = "test_sqlite_roundtrip")]
pub struct SqliteTestModel {
    #[djangors(primary_key, auto)]
    pub id: i64,
    pub name: String,
    pub is_active: bool,
}

#[tokio::test]
async fn test_sqlite_queryset_roundtrip() {
    let config = djangors_db::DatabaseConfig::new("sqlite::memory:");
    let db = djangors_db::Database::connect(&config).await.unwrap();

    sqlx::query(
        "CREATE TABLE \"test_sqlite_roundtrip\" (
            \"id\" INTEGER PRIMARY KEY AUTOINCREMENT,
            \"name\" TEXT NOT NULL,
            \"is_active\" INTEGER NOT NULL
        )",
    )
    .execute(db.sqlite_pool().unwrap())
    .await
    .unwrap();

    let id1 = QuerySet::<SqliteTestModel>::insert_raw(
        &db,
        vec![
            ("name", Value::Text("Alice".into())),
            ("is_active", Value::Bool(true)),
        ],
    )
    .await
    .unwrap();
    assert_eq!(id1, 1);

    let id2 = QuerySet::<SqliteTestModel>::insert_raw(
        &db,
        vec![
            ("name", Value::Text("Bob".into())),
            ("is_active", Value::Bool(false)),
        ],
    )
    .await
    .unwrap();
    assert_eq!(id2, 2);

    let active_models = SqliteTestModel::objects()
        .filter(crate::q!(is_active = true))
        .unwrap()
        .all(&db)
        .await
        .unwrap();

    assert_eq!(active_models.len(), 1);
    assert_eq!(active_models[0].id, 1);
    assert_eq!(active_models[0].name, "Alice");
    assert!(active_models[0].is_active);

    let bob = SqliteTestModel::objects()
        .filter(crate::q!(name = "Bob"))
        .unwrap()
        .get(&db)
        .await
        .unwrap();

    assert_eq!(bob.id, 2);
    assert_eq!(bob.name, "Bob");
    assert!(!bob.is_active);

    let count = SqliteTestModel::objects().count(&db).await.unwrap();
    assert_eq!(count, 2);
}

#[test]
fn test_select_for_update_sql_shapes() {
    let sql_plain = QuerySetTestModel::objects().select_for_update().debug_sql();
    assert_eq!(
        sql_plain,
        "SELECT * FROM \"test_queryset_model\" FOR UPDATE"
    );

    let sql_nowait = QuerySetTestModel::objects()
        .select_for_update()
        .nowait()
        .debug_sql();
    assert_eq!(
        sql_nowait,
        "SELECT * FROM \"test_queryset_model\" FOR UPDATE NOWAIT"
    );

    let sql_skip_locked = QuerySetTestModel::objects()
        .select_for_update()
        .skip_locked()
        .debug_sql();
    assert_eq!(
        sql_skip_locked,
        "SELECT * FROM \"test_queryset_model\" FOR UPDATE SKIP LOCKED"
    );

    let sql_of = QuerySetTestModel::objects()
        .select_for_update()
        .lock_of(&["test_queryset_model"])
        .debug_sql();
    assert_eq!(
        sql_of,
        "SELECT * FROM \"test_queryset_model\" FOR UPDATE OF \"test_queryset_model\""
    );

    let sql_full = QuerySetTestModel::objects()
        .order_by("name")
        .unwrap()
        .limit(10)
        .offset(5)
        .select_for_update()
        .skip_locked()
        .lock_of(&["test_queryset_model", "other_table"])
        .debug_sql();
    assert_eq!(
        sql_full,
        "SELECT * FROM \"test_queryset_model\" ORDER BY \"name\" ASC LIMIT 10 OFFSET 5 FOR UPDATE OF \"test_queryset_model\", \"other_table\" SKIP LOCKED"
    );
}

#[test]
fn test_select_for_update_clone() {
    let qs = QuerySetTestModel::objects()
        .select_for_update()
        .skip_locked()
        .lock_of(&["test_queryset_model"]);
    let cloned = qs.clone();
    assert_eq!(cloned.debug_sql(), qs.debug_sql());
}

#[tokio::test]
async fn test_select_for_update_outside_transaction_guard() {
    let config = djangors_db::DatabaseConfig::new("sqlite::memory:");
    let db = djangors_db::Database::connect(&config).await.unwrap();

    let res = QuerySetTestModel::objects()
        .select_for_update()
        .all(&db)
        .await;
    assert!(matches!(
        res,
        Err(OrmError::SelectForUpdateOutsideTransaction)
    ));
}

#[tokio::test]
async fn test_select_for_update_sqlite_dialect_guards() {
    let config = djangors_db::DatabaseConfig::new("sqlite::memory:");
    let db = djangors_db::Database::connect(&config).await.unwrap();

    sqlx::query(
        "CREATE TABLE \"test_queryset_model\" (
            \"id\" INTEGER PRIMARY KEY AUTOINCREMENT,
            \"name\" TEXT NOT NULL,
            \"is_active\" INTEGER NOT NULL
        )",
    )
    .execute(db.sqlite_pool().unwrap())
    .await
    .unwrap();

    db.transaction_conn(|conn| {
        Box::pin(async move {
            let res_nowait = QuerySetTestModel::objects()
                .select_for_update()
                .nowait()
                .all(conn.conn())
                .await;
            assert!(matches!(res_nowait, Err(OrmError::UnsupportedOnDialect(_))));

            let res_skip = QuerySetTestModel::objects()
                .select_for_update()
                .skip_locked()
                .all(conn.conn())
                .await;
            assert!(matches!(res_skip, Err(OrmError::UnsupportedOnDialect(_))));

            let res_of = QuerySetTestModel::objects()
                .select_for_update()
                .lock_of(&["test_queryset_model"])
                .all(conn.conn())
                .await;
            assert!(matches!(res_of, Err(OrmError::UnsupportedOnDialect(_))));

            // Plain select_for_update succeeds on SQLite:
            let res_plain = QuerySetTestModel::objects()
                .select_for_update()
                .all(conn.conn())
                .await;
            assert!(res_plain.is_ok());

            Ok::<(), OrmError>(())
        })
    })
    .await
    .unwrap();
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn test_select_for_update_postgres_concurrency_skip_locked() {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let config = djangors_db::DatabaseConfig::new(&url);
    let db1 = djangors_db::Database::connect(&config)
        .await
        .expect("connect db1");
    let db2 = djangors_db::Database::connect(&config)
        .await
        .expect("connect db2");

    sqlx::query("DROP TABLE IF EXISTS test_queryset_model")
        .execute(db1.pool())
        .await
        .ok();
    sqlx::query("CREATE TABLE test_queryset_model (id SERIAL PRIMARY KEY, name TEXT NOT NULL, is_active BOOLEAN NOT NULL DEFAULT TRUE)")
        .execute(db1.pool())
        .await
        .expect("create pg test table");

    sqlx::query("INSERT INTO test_queryset_model (id, name, is_active) VALUES (1, 'row1', true), (2, 'row2', true)")
        .execute(db1.pool())
        .await
        .expect("insert test rows");

    // tx1 locks row 1
    let (tx1_send, mut tx1_recv) = tokio::sync::mpsc::channel::<()>(1);
    let (tx2_send, mut tx2_recv) = tokio::sync::mpsc::channel::<()>(1);

    let handle1 = tokio::spawn(async move {
        db1.transaction_conn(|conn| {
            Box::pin(async move {
                let rows = QuerySetTestModel::objects()
                    .filter(crate::q!(id = 1))
                    .unwrap()
                    .select_for_update()
                    .all(conn.conn())
                    .await
                    .unwrap();
                assert_eq!(rows.len(), 1);

                tx1_send.send(()).await.unwrap();
                tx2_recv.recv().await.unwrap();

                Ok::<(), OrmError>(())
            })
        })
        .await
        .unwrap();
    });

    tx1_recv.recv().await.unwrap();

    // tx2 runs select_for_update().skip_locked()
    db2.transaction_conn(|conn| {
        Box::pin(async move {
            let rows = QuerySetTestModel::objects()
                .select_for_update()
                .skip_locked()
                .all(conn.conn())
                .await
                .unwrap();

            // Row 1 was locked by tx1, so tx2 should only see row 2
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].id, 2);

            tx2_send.send(()).await.unwrap();
            Ok::<(), OrmError>(())
        })
    })
    .await
    .unwrap();

    handle1.await.unwrap();

    sqlx::query("DROP TABLE test_queryset_model")
        .execute(db2.pool())
        .await
        .ok();
}

#[derive(Model, Debug, Clone)]
#[djangors(app = "test_app", table_name = "test_subquery_author")]
pub struct SubqueryAuthorModel {
    #[djangors(primary_key, auto)]
    pub id: i64,

    pub name: String,

    pub active: bool,
}

#[derive(Model, Debug, Clone)]
#[djangors(app = "test_app", table_name = "test_subquery_post")]
pub struct SubqueryPostModel {
    #[djangors(primary_key, auto)]
    pub id: i64,

    pub author_id: i64,

    pub published: bool,

    pub title: String,
}

#[test]
fn test_subquery_exists_sql_shape() {
    // OuterRef is imported here (unlike the other subquery tests) because this test calls it
    // directly to exercise the macro's bound-variable branch, not only the inline branch.
    use crate::{q_outer, Exists, NotExists, OuterRef};

    // Test inline OuterRef("id") macro branch
    let qs_exists = SubqueryAuthorModel::objects()
        .filter(
            Exists::<SubqueryPostModel>::new()
                .filter(q_outer!(author_id = OuterRef("id")))
                .unwrap()
                .filter(q!(published = true))
                .unwrap(),
        )
        .unwrap();

    let sql = qs_exists.debug_sql();
    // The doubled parentheses are expected, not a bug: each `.filter()` call appends its own
    // `Expr::And([..])`, and `compile_expr_sql` parenthesises both the And and each leaf compare.
    // That predates this slice - it is simply the first time a test asserted the full string of a
    // subquery. The SQL is valid and semantically identical either way.
    assert_eq!(
        sql,
        "SELECT * FROM \"test_subquery_author\" WHERE (EXISTS (SELECT 1 FROM \"test_subquery_post\" WHERE ((\"test_subquery_post\".\"author_id\" = \"test_subquery_author\".\"id\")) AND ((\"test_subquery_post\".\"published\" = $1))))"
    );

    // Test binding variable holding OuterRef macro branch
    let oref = OuterRef("id");
    let qs_not_exists = SubqueryAuthorModel::objects()
        .filter(
            NotExists::<SubqueryPostModel>::new()
                .filter(q_outer!(author_id = oref))
                .unwrap()
                .filter(q!(published = true))
                .unwrap(),
        )
        .unwrap();

    let sql_not = qs_not_exists.debug_sql();
    assert_eq!(
        sql_not,
        "SELECT * FROM \"test_subquery_author\" WHERE (NOT EXISTS (SELECT 1 FROM \"test_subquery_post\" WHERE ((\"test_subquery_post\".\"author_id\" = \"test_subquery_author\".\"id\")) AND ((\"test_subquery_post\".\"published\" = $1))))"
    );
}

#[test]
fn test_subquery_parameter_ordering_before_and_after() {
    use crate::{q_outer, Exists, Value};

    let qs = SubqueryAuthorModel::objects()
        .filter(q!(active = true))
        .unwrap()
        .filter(
            Exists::<SubqueryPostModel>::new()
                .filter(q_outer!(author_id = OuterRef("id")))
                .unwrap()
                .filter(q!(published = true))
                .unwrap(),
        )
        .unwrap()
        .filter(q!(name = "Alice"))
        .unwrap();

    let sql = qs.debug_sql();
    let params = qs.debug_params();

    // Same parenthesisation note as test_subquery_exists_sql_shape. What this test actually pins
    // is the PLACEHOLDER NUMBERING: $1 before the subquery, $2 inside it, $3 after. If the
    // subquery did not continue the outer query's param counter, this would still be executable
    // SQL that silently bound the right number of values in the wrong order.
    assert_eq!(
        sql,
        "SELECT * FROM \"test_subquery_author\" WHERE ((\"active\" = $1)) AND (EXISTS (SELECT 1 FROM \"test_subquery_post\" WHERE ((\"test_subquery_post\".\"author_id\" = \"test_subquery_author\".\"id\")) AND ((\"test_subquery_post\".\"published\" = $2)))) AND ((\"name\" = $3))"
    );

    assert_eq!(
        params,
        vec![
            Value::Bool(true),
            Value::Bool(true),
            Value::Text("Alice".to_string()),
        ]
    );
}

#[test]
fn test_outerref_outside_subquery_error() {
    use crate::q_outer;
    let res = SubqueryAuthorModel::objects().filter(q_outer!(id = OuterRef("id")));
    assert!(matches!(
        res,
        Err(crate::error::OrmError::InvalidQuery(msg)) if msg.contains("OuterRef can only be used within a subquery")
    ));
}

// The two subquery round-trip tests below share the SAME two models, so they necessarily share
// the same two tables. Unique table names cannot separate them; only serialisation can. Without
// this, cargo's parallel test threads race on Postgres and produce "relation already exists" in
// one test and "relation does not exist" in the other - a confusing failure that looks like a
// subquery bug and is not one. SQLite is unaffected (each in-memory handle is its own database),
// which is exactly why this only ever failed on the Postgres leg.
// tokio's Mutex, not std's: these are async tests and the guard is deliberately held across
// `.await` points. A std::sync::MutexGuard held across an await is a real deadlock hazard (and
// clippy::await_holding_lock rejects it), because the guard would pin a blocking lock to a task
// that can be parked and resumed on a different thread.
static SUBQUERY_DB_MUTEX: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[tokio::test]
async fn test_subquery_exists_roundtrip() {
    let _guard = SUBQUERY_DB_MUTEX.lock().await;
    use crate::{q_outer, Exists, NotExists};

    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_subquery_post", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_subquery_author", &[])
        .await;

    let create_author_sql = format!(
        "CREATE TABLE test_subquery_author (
            id {auto_pk},
            name TEXT NOT NULL,
            active BOOLEAN NOT NULL
        )"
    );
    let create_post_sql = format!(
        "CREATE TABLE test_subquery_post (
            id {auto_pk},
            author_id BIGINT NOT NULL,
            published BOOLEAN NOT NULL,
            title TEXT NOT NULL
        )"
    );

    db.conn().execute(&create_author_sql, &[]).await.unwrap();
    db.conn().execute(&create_post_sql, &[]).await.unwrap();

    db.conn()
        .execute(
            "INSERT INTO test_subquery_author (name, active) VALUES ('Alice', true), ('Bob', true), ('Charlie', true)",
            &[],
        )
        .await
        .unwrap();

    db.conn()
        .execute(
            "INSERT INTO test_subquery_post (author_id, published, title) VALUES (1, true, 'Post 1'), (3, false, 'Draft 1')",
            &[],
        )
        .await
        .unwrap();

    let authors_with_published = SubqueryAuthorModel::objects()
        .filter(
            Exists::<SubqueryPostModel>::new()
                .filter(q_outer!(author_id = OuterRef("id")))
                .unwrap()
                .filter(q!(published = true))
                .unwrap(),
        )
        .unwrap()
        .order_by("id")
        .unwrap()
        .all(db)
        .await
        .unwrap();

    assert_eq!(authors_with_published.len(), 1);
    assert_eq!(authors_with_published[0].name, "Alice");

    let authors_without_published = SubqueryAuthorModel::objects()
        .filter(
            NotExists::<SubqueryPostModel>::new()
                .filter(q_outer!(author_id = OuterRef("id")))
                .unwrap()
                .filter(q!(published = true))
                .unwrap(),
        )
        .unwrap()
        .order_by("id")
        .unwrap()
        .all(db)
        .await
        .unwrap();

    assert_eq!(authors_without_published.len(), 2);
    assert_eq!(authors_without_published[0].name, "Bob");
    assert_eq!(authors_without_published[1].name, "Charlie");

    let _ = db
        .conn()
        .execute("DROP TABLE test_subquery_post", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE test_subquery_author", &[])
        .await;
}

#[tokio::test]
async fn test_subquery_nested_filters_and_count_aggregate() {
    let _guard = SUBQUERY_DB_MUTEX.lock().await;
    use crate::{q_outer, Exists};

    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_subquery_post", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_subquery_author", &[])
        .await;

    let create_author_sql = format!(
        "CREATE TABLE test_subquery_author (
            id {auto_pk},
            name TEXT NOT NULL,
            active BOOLEAN NOT NULL
        )"
    );
    let create_post_sql = format!(
        "CREATE TABLE test_subquery_post (
            id {auto_pk},
            author_id BIGINT NOT NULL,
            published BOOLEAN NOT NULL,
            title TEXT NOT NULL
        )"
    );

    db.conn().execute(&create_author_sql, &[]).await.unwrap();
    db.conn().execute(&create_post_sql, &[]).await.unwrap();

    db.conn()
        .execute(
            "INSERT INTO test_subquery_author (name, active) VALUES ('Alice', true), ('Aaron', true), ('Bob', false)",
            &[],
        )
        .await
        .unwrap();

    db.conn()
        .execute(
            "INSERT INTO test_subquery_post (author_id, published, title) VALUES (1, true, 'Post 1'), (2, true, 'Post 2')",
            &[],
        )
        .await
        .unwrap();

    let active_authors_with_posts = SubqueryAuthorModel::objects()
        .filter(q!(active = true))
        .unwrap()
        .filter(q!(name__startswith = "A"))
        .unwrap()
        .filter(
            Exists::<SubqueryPostModel>::new()
                .filter(q_outer!(author_id = OuterRef("id")))
                .unwrap()
                .filter(q!(published = true))
                .unwrap(),
        )
        .unwrap()
        .order_by("-name")
        .unwrap()
        .limit(1)
        .all(db)
        .await
        .unwrap();

    assert_eq!(active_authors_with_posts.len(), 1);
    assert_eq!(active_authors_with_posts[0].name, "Alice");

    let count = SubqueryAuthorModel::objects()
        .filter(
            Exists::<SubqueryPostModel>::new()
                .filter(q_outer!(author_id = OuterRef("id")))
                .unwrap()
                .filter(q!(published = true))
                .unwrap(),
        )
        .unwrap()
        .count(db)
        .await
        .unwrap();

    assert_eq!(count, 2);

    use crate::aggregate::AggExpr;
    let agg = SubqueryAuthorModel::objects()
        .filter(
            Exists::<SubqueryPostModel>::new()
                .filter(q_outer!(author_id = OuterRef("id")))
                .unwrap()
                .filter(q!(published = true))
                .unwrap(),
        )
        .unwrap()
        .aggregate(db, vec![AggExpr::Count { field: "*" }])
        .await
        .unwrap();

    assert_eq!(agg, vec![crate::aggregate::AggResult::I64(2)]);

    let _ = db
        .conn()
        .execute("DROP TABLE test_subquery_post", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE test_subquery_author", &[])
        .await;
}

mod soft_delete {
    use crate::aggregate::{AggExpr, AggResult};
    use crate::error::OrmError;
    use crate::expr::{SetExpr, Value};
    use crate::meta::Model;
    use crate::{Exists, NotExists};
    use djangors_macros::Model;

    #[derive(Model, Debug, Clone)]
    #[djangors(app = "test_app", table_name = "soft_delete_test_model", soft_delete)]
    #[allow(dead_code)]
    pub struct SoftDeleteModel {
        #[djangors(primary_key, auto)]
        pub id: i64,

        pub name: String,

        pub deleted_at: Option<chrono::DateTime<chrono::Utc>>,
    }

    #[derive(Model, Debug, Clone)]
    #[djangors(app = "test_app", table_name = "hard_delete_test_model")]
    #[allow(dead_code)]
    pub struct HardDeleteModel {
        #[djangors(primary_key, auto)]
        pub id: i64,

        pub name: String,
    }

    #[derive(Model, Debug, Clone)]
    #[djangors(app = "test_app", table_name = "soft_delete_test_child")]
    #[allow(dead_code)]
    pub struct ChildOfSoft {
        #[djangors(primary_key, auto)]
        pub id: i64,
        #[djangors(foreign_key(on_delete = "cascade", related_name = "kids"))]
        pub parent: crate::meta::ForeignKey<SoftDeleteModel>,
    }

    #[derive(Model, Debug, Clone)]
    #[djangors(app = "test_app", table_name = "soft_delete_test_note", soft_delete)]
    #[allow(dead_code)]
    pub struct SoftNote {
        #[djangors(primary_key, auto)]
        pub id: i64,
        #[djangors(foreign_key(on_delete = "cascade", related_name = "notes"))]
        pub owner: crate::meta::ForeignKey<HardDeleteModel>,
        pub deleted_at: Option<chrono::DateTime<chrono::Utc>>,
    }

    async fn connect_memory() -> djangors_db::Database {
        let config = djangors_db::DatabaseConfig::new("sqlite::memory:")
            .max_connections(1)
            .min_connections(1);
        djangors_db::Database::connect(&config).await.unwrap()
    }

    async fn create_related_tables(db: &djangors_db::Database) {
        let auto_pk = db.dialect().auto_pk_type();
        let ts_type = db.dialect().timestamp_type();
        let child =
            format!("CREATE TABLE soft_delete_test_child (id {auto_pk}, parent BIGINT NOT NULL)");
        db.conn().execute(&child, &[]).await.unwrap();
        let note = format!(
            "CREATE TABLE soft_delete_test_note \
             (id {auto_pk}, owner BIGINT NOT NULL, deleted_at {ts_type})"
        );
        db.conn().execute(&note, &[]).await.unwrap();
    }

    #[tokio::test]
    async fn select_related_treats_a_soft_deleted_target_as_missing() {
        let db = connect_memory().await;
        create_tables(&db).await;
        create_related_tables(&db).await;
        let live = soft("live").save(&db).await.unwrap();
        let gone = soft("gone").save(&db).await.unwrap();
        for parent in [&live, &gone] {
            ChildOfSoft {
                id: 0,
                parent: crate::meta::ForeignKey::new(parent.id),
            }
            .save(&db)
            .await
            .unwrap();
        }
        gone.delete(&db).await.unwrap();

        let rows = ChildOfSoft::objects()
            .order_by("id")
            .unwrap()
            .select_related::<SoftDeleteModel, _>(&db, "parent")
            .await
            .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].1.as_ref().map(|p| p.name.as_str()), Some("live"));
        assert!(rows[1].1.is_none());
        // ForeignKey::fetch agrees.
        assert!(rows[1].0.parent.fetch(&db).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn prefetch_related_skips_soft_deleted_children() {
        let db = connect_memory().await;
        create_tables(&db).await;
        create_related_tables(&db).await;
        let owner = hard("owner").save(&db).await.unwrap();
        let note = |owner_id| SoftNote {
            id: 0,
            owner: crate::meta::ForeignKey::new(owner_id),
            deleted_at: None,
        };
        let kept = note(owner.id).save(&db).await.unwrap();
        let removed = note(owner.id).save(&db).await.unwrap();
        removed.delete(&db).await.unwrap();

        let grouped: std::collections::HashMap<i64, Vec<SoftNote>> =
            crate::prefetch_related(&db, std::slice::from_ref(&owner), "notes")
                .await
                .unwrap();
        let notes = &grouped[&owner.id];
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].id, kept.id);
    }

    #[tokio::test]
    async fn delete_by_pk_soft_deletes_and_force_delete_by_pk_removes() {
        let db = connect_memory().await;
        create_tables(&db).await;
        let a = soft("a").save(&db).await.unwrap();

        let n = crate::QuerySet::<SoftDeleteModel>::delete_by_pk(&db, a.id)
            .await
            .unwrap();
        assert_eq!(n, 1);
        assert_eq!(SoftDeleteModel::objects().count(&db).await.unwrap(), 0);
        assert_eq!(raw_count(&db, "soft_delete_test_model").await, 1);
        // Already deleted: nothing changes.
        let n = crate::QuerySet::<SoftDeleteModel>::delete_by_pk(&db, a.id)
            .await
            .unwrap();
        assert_eq!(n, 0);

        let n = crate::QuerySet::<SoftDeleteModel>::force_delete_by_pk(&db, a.id)
            .await
            .unwrap();
        assert_eq!(n, 1);
        assert_eq!(raw_count(&db, "soft_delete_test_model").await, 0);

        // Models without soft_delete keep the plain DELETE.
        let h = hard("h").save(&db).await.unwrap();
        crate::QuerySet::<HardDeleteModel>::delete_by_pk(&db, h.id)
            .await
            .unwrap();
        assert_eq!(raw_count(&db, "hard_delete_test_model").await, 0);
    }

    async fn create_tables(db: &djangors_db::Database) {
        let auto_pk = db.dialect().auto_pk_type();
        let ts_type = db.dialect().timestamp_type();
        let soft_sql = format!(
            "CREATE TABLE soft_delete_test_model (
                id {auto_pk},
                name TEXT NOT NULL,
                deleted_at {ts_type}
            )"
        );
        db.conn().execute(&soft_sql, &[]).await.unwrap();
        let hard_sql = format!(
            "CREATE TABLE hard_delete_test_model (
                id {auto_pk},
                name TEXT NOT NULL
            )"
        );
        db.conn().execute(&hard_sql, &[]).await.unwrap();
    }

    /// Counts every physical row, including soft-deleted ones.
    async fn raw_count(db: &djangors_db::Database, table: &str) -> i64 {
        let sql = format!("SELECT COUNT(*) FROM {table}");
        let row = db.conn().fetch_one(&sql, &[]).await.unwrap();
        row.try_i64(0).unwrap().unwrap()
    }

    fn soft(name: &str) -> SoftDeleteModel {
        SoftDeleteModel {
            id: 0,
            name: name.to_string(),
            deleted_at: None,
        }
    }

    fn hard(name: &str) -> HardDeleteModel {
        HardDeleteModel {
            id: 0,
            name: name.to_string(),
        }
    }

    #[tokio::test]
    async fn delete_hides_the_row_from_select_count_and_get() {
        let db = connect_memory().await;
        create_tables(&db).await;
        let a = soft("a").save(&db).await.unwrap();
        soft("b").save(&db).await.unwrap();
        soft("c").save(&db).await.unwrap();

        a.delete(&db).await.unwrap();

        assert_eq!(SoftDeleteModel::objects().count(&db).await.unwrap(), 2);
        assert_eq!(SoftDeleteModel::objects().all(&db).await.unwrap().len(), 2);
        let found = SoftDeleteModel::objects()
            .filter(crate::q!(name = "a"))
            .unwrap()
            .get(&db)
            .await;
        assert!(matches!(found, Err(OrmError::NotFound { .. })));
        // The row is hidden, not removed.
        assert_eq!(raw_count(&db, "soft_delete_test_model").await, 3);
    }

    #[tokio::test]
    async fn with_deleted_and_only_deleted_select_the_right_rows() {
        let db = connect_memory().await;
        create_tables(&db).await;
        let a = soft("a").save(&db).await.unwrap();
        soft("b").save(&db).await.unwrap();
        a.delete(&db).await.unwrap();

        assert_eq!(
            SoftDeleteModel::objects()
                .with_deleted()
                .count(&db)
                .await
                .unwrap(),
            2
        );
        let got = SoftDeleteModel::objects()
            .with_deleted()
            .filter(crate::q!(name = "a"))
            .unwrap()
            .get(&db)
            .await
            .unwrap();
        assert_eq!(got.id, a.id);

        let deleted = SoftDeleteModel::objects()
            .only_deleted()
            .all(&db)
            .await
            .unwrap();
        assert_eq!(deleted.len(), 1);
        assert_eq!(deleted[0].name, "a");
        assert_eq!(
            SoftDeleteModel::objects()
                .only_deleted()
                .count(&db)
                .await
                .unwrap(),
            1
        );
    }

    #[tokio::test]
    async fn restore_brings_a_soft_deleted_row_back() {
        let db = connect_memory().await;
        create_tables(&db).await;
        let a = soft("a").save(&db).await.unwrap();

        a.delete(&db).await.unwrap();
        assert_eq!(SoftDeleteModel::objects().count(&db).await.unwrap(), 0);

        a.restore(&db).await.unwrap();
        assert_eq!(SoftDeleteModel::objects().count(&db).await.unwrap(), 1);

        // Restoring a live row and deleting an already deleted row match nothing.
        let restore_again = a.restore(&db).await;
        assert!(matches!(restore_again, Err(OrmError::NotFound { .. })));
        a.delete(&db).await.unwrap();
        let delete_again = a.delete(&db).await;
        assert!(matches!(delete_again, Err(OrmError::NotFound { .. })));
    }

    #[tokio::test]
    async fn force_delete_really_removes_the_row() {
        let db = connect_memory().await;
        create_tables(&db).await;
        let a = soft("a").save(&db).await.unwrap();
        let b = soft("b").save(&db).await.unwrap();

        a.force_delete(&db).await.unwrap();
        assert_eq!(raw_count(&db, "soft_delete_test_model").await, 1);

        // A soft delete leaves the physical row in place...
        b.delete(&db).await.unwrap();
        assert_eq!(raw_count(&db, "soft_delete_test_model").await, 1);

        // ...and force_delete removes it.
        b.force_delete(&db).await.unwrap();
        assert_eq!(raw_count(&db, "soft_delete_test_model").await, 0);
    }

    #[tokio::test]
    async fn bulk_queryset_delete_soft_deletes_and_with_deleted_hard_deletes() {
        let db = connect_memory().await;
        create_tables(&db).await;
        soft("a").save(&db).await.unwrap();
        soft("b").save(&db).await.unwrap();
        soft("c").save(&db).await.unwrap();

        let one = SoftDeleteModel::objects()
            .filter(crate::q!(name = "a"))
            .unwrap()
            .delete(&db)
            .await
            .unwrap();
        assert_eq!(one, 1);
        assert_eq!(SoftDeleteModel::objects().count(&db).await.unwrap(), 2);
        assert_eq!(raw_count(&db, "soft_delete_test_model").await, 3);

        // Rows that are already soft-deleted are skipped.
        let rest = SoftDeleteModel::objects().delete(&db).await.unwrap();
        assert_eq!(rest, 2);
        assert_eq!(SoftDeleteModel::objects().count(&db).await.unwrap(), 0);
        assert_eq!(raw_count(&db, "soft_delete_test_model").await, 3);

        // with_deleted() turns the bulk delete into a real DELETE.
        let purged = SoftDeleteModel::objects()
            .with_deleted()
            .delete(&db)
            .await
            .unwrap();
        assert_eq!(purged, 3);
        assert_eq!(raw_count(&db, "soft_delete_test_model").await, 0);

        soft("d").save(&db).await.unwrap();
        let forced = SoftDeleteModel::objects().force_delete(&db).await.unwrap();
        assert_eq!(forced, 1);
        assert_eq!(raw_count(&db, "soft_delete_test_model").await, 0);
    }

    #[tokio::test]
    async fn update_and_aggregate_skip_soft_deleted_rows() {
        let db = connect_memory().await;
        create_tables(&db).await;
        let a = soft("a").save(&db).await.unwrap();
        soft("b").save(&db).await.unwrap();
        a.delete(&db).await.unwrap();

        let updated = SoftDeleteModel::objects()
            .update(
                &db,
                vec![("name", SetExpr::Literal(Value::Text("x".to_string())))],
            )
            .await
            .unwrap();
        assert_eq!(updated, 1);

        let renamed = SoftDeleteModel::objects()
            .with_deleted()
            .filter(crate::q!(name = "x"))
            .unwrap()
            .count(&db)
            .await
            .unwrap();
        assert_eq!(renamed, 1);
        let untouched = SoftDeleteModel::objects()
            .with_deleted()
            .filter(crate::q!(name = "a"))
            .unwrap()
            .count(&db)
            .await
            .unwrap();
        assert_eq!(untouched, 1);

        let agg = SoftDeleteModel::objects()
            .aggregate(&db, vec![AggExpr::Count { field: "*" }])
            .await
            .unwrap();
        assert_eq!(agg, vec![AggResult::I64(1)]);

        assert!(SoftDeleteModel::objects().exists(&db).await.unwrap());
        let first = SoftDeleteModel::objects().first(&db).await.unwrap();
        assert_eq!(first.unwrap().name, "x");
    }

    #[tokio::test]
    async fn exists_subqueries_ignore_soft_deleted_rows() {
        let db = connect_memory().await;
        create_tables(&db).await;
        let a = soft("a").save(&db).await.unwrap();
        let b = soft("b").save(&db).await.unwrap();
        a.delete(&db).await.unwrap();
        b.delete(&db).await.unwrap();

        // Every row is soft-deleted, so the uncorrelated EXISTS over live rows is false.
        let exists_count = SoftDeleteModel::objects()
            .with_deleted()
            .filter(Exists::<SoftDeleteModel>::new())
            .unwrap()
            .count(&db)
            .await
            .unwrap();
        assert_eq!(exists_count, 0);

        let not_exists_count = SoftDeleteModel::objects()
            .with_deleted()
            .filter(NotExists::<SoftDeleteModel>::new())
            .unwrap()
            .count(&db)
            .await
            .unwrap();
        assert_eq!(not_exists_count, 2);
    }

    #[test]
    fn soft_delete_predicate_is_in_every_where_clause() {
        let sql = SoftDeleteModel::objects().debug_sql();
        assert!(sql.contains(" WHERE \"deleted_at\" IS NULL"), "{sql}");

        let filtered = SoftDeleteModel::objects()
            .filter(crate::q!(name = "a"))
            .unwrap()
            .debug_sql();
        assert!(filtered.contains("\"deleted_at\" IS NULL"), "{filtered}");

        let with_deleted = SoftDeleteModel::objects().with_deleted().debug_sql();
        assert!(!with_deleted.contains("IS NULL"), "{with_deleted}");

        let only_deleted = SoftDeleteModel::objects().only_deleted().debug_sql();
        assert!(
            only_deleted.contains("\"deleted_at\" IS NOT NULL"),
            "{only_deleted}"
        );
        assert!(
            !only_deleted.contains("\"deleted_at\" IS NULL"),
            "{only_deleted}"
        );
    }

    #[tokio::test]
    async fn models_without_soft_delete_still_hard_delete() {
        let db = connect_memory().await;
        create_tables(&db).await;
        let a = hard("a").save(&db).await.unwrap();
        hard("b").save(&db).await.unwrap();

        assert!(HardDeleteModel::soft_delete_column().is_none());
        assert_eq!(SoftDeleteModel::soft_delete_column(), Some("deleted_at"));

        a.delete(&db).await.unwrap();
        assert_eq!(raw_count(&db, "hard_delete_test_model").await, 1);

        let removed = HardDeleteModel::objects().delete(&db).await.unwrap();
        assert_eq!(removed, 1);
        assert_eq!(raw_count(&db, "hard_delete_test_model").await, 0);

        let sql = HardDeleteModel::objects().debug_sql();
        assert!(!sql.contains("IS NULL"), "{sql}");
        // only_deleted() on a model without soft delete matches nothing.
        let none = HardDeleteModel::objects()
            .only_deleted()
            .count(&db)
            .await
            .unwrap();
        assert_eq!(none, 0);
        let sql = HardDeleteModel::objects().only_deleted().debug_sql();
        assert!(sql.contains("FALSE"), "{sql}");
    }
}
