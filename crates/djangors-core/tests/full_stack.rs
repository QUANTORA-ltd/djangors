//! Full-stack integration test: a real HTTP request, over a real TCP socket,
//! dispatched through a real `Router`, handled by a real `async fn` handler
//! that pulls a real `djangors_db::Database` out of `Request::state`, runs a
//! real ORM query via `djangors_orm::QuerySet` against real Postgres, and
//! returns a response reflecting real data — every layer built so far in
//! this project, exercised together for the first time, not just each layer
//! independently.

use djangors_core::{
    Djangors, DjangorsError, DjangorsSettings, PathParams, Request, Response, Router, StatusCode,
};
use djangors_db::{config::DatabaseConfig, Database};
use djangors_macros::Model;
use djangors_orm::{q, Model};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[derive(Model, Debug)]
#[djangors(app = "full_stack_test", table_name = "test_full_stack_item")]
#[allow(dead_code)]
pub struct Item {
    #[djangors(primary_key, auto)]
    pub id: i64,

    #[djangors(max_length = 100)]
    pub name: String,
}

/// Reads the `id` path param, pulls the `Database` out of app state, runs a
/// real `QuerySet::get` against it, and returns the item's name — or a plain
/// 404 (via `DjangorsError::NotFound`) if nothing matches.
async fn get_item(req: Request, params: PathParams) -> Result<Response, DjangorsError> {
    let id: i64 = params.get_as("id")?;
    let db = req
        .state::<Database>()
        .expect("Database must be attached via Router::with_state");

    let item = Item::objects()
        .filter(q!(id = id))
        .map_err(|e| DjangorsError::Internal(e.to_string()))?
        .get(db)
        .await
        .map_err(|_| DjangorsError::NotFound)?;

    Ok(Response::text(StatusCode::OK, &item.name))
}

#[tokio::test]
async fn full_stack_request_hits_real_database() {
    let sqlite = matches!(
        std::env::var("TEST_BACKEND").as_deref(),
        Ok(value) if value.eq_ignore_ascii_case("sqlite")
    );
    let db_url = if sqlite {
        "sqlite::memory:".to_owned()
    } else {
        std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://postgres:postgres@localhost/djangors_test".to_owned())
    };
    let mut config = DatabaseConfig::new(db_url);
    if sqlite {
        config = config.max_connections(1).min_connections(1);
    }
    let db = Database::connect(&config)
        .await
        .expect("failed to connect to the selected test database");

    let mut conn = db.conn();
    conn.execute("DROP TABLE IF EXISTS test_full_stack_item", &[])
        .await
        .unwrap();
    let id_type = if sqlite {
        "INTEGER PRIMARY KEY AUTOINCREMENT"
    } else {
        "BIGSERIAL PRIMARY KEY"
    };
    let create_table = format!(
        "CREATE TABLE test_full_stack_item (
            id {id_type},
            name VARCHAR(100) NOT NULL
        )"
    );
    conn.execute(&create_table, &[]).await.unwrap();
    conn.execute(
        "INSERT INTO test_full_stack_item (name) VALUES ('Widget')",
        &[],
    )
    .await
    .unwrap();

    let db_for_cleanup = db.clone();
    let router = Router::new()
        .with_state(db)
        .get("/items/{id:i64}", get_item);

    // Bind to an OS-assigned free port directly (settings.validate() rejects
    // a literal port of 0, so — mirroring the same pattern already used in
    // app.rs's own real_socket_request_response test — bind first, then
    // build settings carrying the port the OS actually handed back).
    let listener: TcpListener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let settings = DjangorsSettings {
        host: "127.0.0.1".to_string(),
        port: addr.port(),
        ..Default::default()
    };
    let app = Djangors::new(settings, router);

    tokio::spawn(async move {
        app.serve(listener).await.unwrap();
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Real HTTP request over a real socket for the item that exists.
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    stream
        .write_all(b"GET /items/1 HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    let response = String::from_utf8(buf).unwrap();
    assert!(response.contains("200 OK"), "expected 200, got: {response}");
    assert!(
        response.contains("Widget"),
        "expected body 'Widget', got: {response}"
    );

    // A second real request over a NEW connection for an id that doesn't
    // exist — proves the NotFound path (a real ORM miss) round-trips
    // correctly all the way back out as an HTTP 404, not a 500 or a hang.
    let mut stream2 = tokio::net::TcpStream::connect(addr).await.unwrap();
    stream2
        .write_all(b"GET /items/999 HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut buf2 = Vec::new();
    stream2.read_to_end(&mut buf2).await.unwrap();
    let response2 = String::from_utf8(buf2).unwrap();
    assert!(response2.contains("404"), "expected 404, got: {response2}");

    db_for_cleanup
        .conn()
        .execute("DROP TABLE test_full_stack_item", &[])
        .await
        .unwrap();
}
