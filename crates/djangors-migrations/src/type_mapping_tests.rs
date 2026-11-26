use super::*;

#[test]
fn file_field_maps_to_the_same_sql_type_as_text() {
    let file_field_sql = sql_type_for(
        &FieldKind::FileField,
        None,
        false,
        "attachment",
        Dialect::Postgres,
    )
    .unwrap();
    let text_sql = sql_type_for(&FieldKind::Text, None, false, "body", Dialect::Postgres).unwrap();
    assert_eq!(file_field_sql, text_sql);
    assert_eq!(file_field_sql, "TEXT");
}

#[test]
fn sqlite_auto_primary_key_yields_integer_primary_key_autoincrement() {
    let sql_int = sql_type_for(&FieldKind::Integer, None, true, "id", Dialect::Sqlite).unwrap();
    assert_eq!(sql_int, "INTEGER PRIMARY KEY AUTOINCREMENT");

    let sql_bigint = sql_type_for(&FieldKind::BigInt, None, true, "id", Dialect::Sqlite).unwrap();
    assert_eq!(sql_bigint, "INTEGER PRIMARY KEY AUTOINCREMENT");
}

#[test]
fn vector_field_and_pgvector_index_sql() {
    let pg = sql_type_for(
        &FieldKind::Vector { dimensions: 1536 },
        None,
        false,
        "embedding",
        Dialect::Postgres,
    )
    .unwrap();
    assert_eq!(pg, "VECTOR(1536)");

    let sq = sql_type_for(
        &FieldKind::Vector { dimensions: 1536 },
        None,
        false,
        "embedding",
        Dialect::Sqlite,
    )
    .unwrap();
    assert_eq!(sq, "TEXT");

    let ext = crate::Operation::EnableExtension {
        name: "vector".to_string(),
    };
    assert_eq!(
        ext.to_sql(Dialect::Postgres).unwrap(),
        "CREATE EXTENSION IF NOT EXISTS \"vector\";"
    );

    let idx = crate::Operation::CreateVectorIndex {
        index_name: "docs_embedding_hnsw".to_string(),
        table_name: "docs".to_string(),
        column_name: "embedding".to_string(),
        method: crate::VectorIndexMethod::Hnsw,
        opclass: crate::VectorOpClass::Cosine,
    };
    assert_eq!(
        idx.to_sql(Dialect::Postgres).unwrap(),
        "CREATE INDEX IF NOT EXISTS \"docs_embedding_hnsw\" ON \"docs\" USING hnsw (\"embedding\" vector_cosine_ops);"
    );
    assert!(matches!(
        idx.reverse(),
        Some(crate::Operation::DropIndex { index_name }) if index_name == "docs_embedding_hnsw"
    ));
}
