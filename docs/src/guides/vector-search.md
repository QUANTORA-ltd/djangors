# Vector Search (`pgvector`) & SQLite FTS5

Djangors supports dense vector embeddings (`Vector` + `#[djangors(vector = N)]`) for RAG and semantic search, plus portable full-text search across PostgreSQL (`tsvector`) and SQLite (`FTS5` / multi-term `LIKE`).

## Vector Embeddings (`pgvector`)

```rust,illustrative
use djangors::orm::{Model, Vector};

#[derive(Model, Debug, Clone)]
#[djangors(app = "kb", table_name = "articles")]
pub struct Article {
    #[djangors(primary_key, auto)]
    pub id: i64,
    pub title: String,
    #[djangors(vector = 1536)]
    pub embedding: Vector,
}

let nearest = Article::objects()
    .order_by_cosine_distance("embedding", &query_embedding)?
    .limit(5)
    .all(&db)
    .await?;
```

On PostgreSQL this emits `VECTOR(1536)` and `ORDER BY "embedding" <=> '...'::vector`; on SQLite it stores `[...]` in `TEXT` and computes exact distances in Rust so local tests behave identically.

### `pgvector` Migrations & HNSW / IVFFlat Indexes

```rust,compile
use djangors::migrations::{Operation, VectorIndexMethod, VectorOpClass};

pub fn pgvector_ops() -> (Operation, Operation) {
    let enable_pgvector = Operation::EnableExtension {
        name: "vector".to_string(),
    };
    let hnsw_index = Operation::CreateVectorIndex {
        index_name: "articles_embedding_hnsw".to_string(),
        table_name: "articles".to_string(),
        column_name: "embedding".to_string(),
        method: VectorIndexMethod::Hnsw,
        opclass: VectorOpClass::Cosine,
    };
    (enable_pgvector, hnsw_index)
}
```

## SQLite FTS5 Full-Text Search

`QuerySet::search(query, fields)` automatically uses `to_tsvector` on PostgreSQL and multi-term case-insensitive `LIKE` on SQLite. For dedicated SQLite FTS5 virtual tables:

```rust,illustrative
use djangors::orm::{create_fts5_table, index_fts5_row};

create_fts5_table(&db, "articles_fts", &["title", "body"]).await?;
index_fts5_row(&db, "articles_fts", article.id, &[("title", &article.title), ("body", &article.body)]).await?;

let hits = Article::objects()
    .search_fts5("articles_fts", "rust async")?
    .all(&db)
    .await?;
```
