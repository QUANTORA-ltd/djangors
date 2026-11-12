use super::*;
use crate::groups::Groups;
use djangors_orm::djangors_db::DatabaseConfig;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio_stream::StreamExt;

fn postgres_url() -> Option<String> {
    std::env::var("DATABASE_URL")
        .ok()
        .filter(|url| url.starts_with("postgres"))
}

fn unique_channel() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is after the Unix epoch")
        .as_nanos();
    format!("djangors_groups_test_{nanos}")
}

#[test]
fn encode_payload_accepts_small_messages() {
    let payload = encode_payload("room", "hello").expect("small message encodes");
    let value: serde_json::Value = serde_json::from_str(&payload).expect("payload is JSON");
    assert_eq!(value["group"], "room");
    assert_eq!(value["message"], "hello");
}

#[test]
fn encode_payload_rejects_oversized_messages() {
    let too_big = "x".repeat(MAX_NOTIFY_PAYLOAD_BYTES);
    assert!(encode_payload("room", &too_big).is_err());
}

#[tokio::test]
async fn deliver_forwards_valid_payloads_and_skips_invalid_ones() {
    let groups = Groups::new();
    let mut stream = groups.group("room").subscribe();

    deliver(&groups, "not json");
    deliver(&groups, r#"{"group":"other","message":"ignored"}"#);

    let payload = encode_payload("room", "hi \"there\"").expect("message encodes");
    deliver(&groups, &payload);

    assert_eq!(stream.next().await, Some("hi \"there\"".to_string()));
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn start_rejects_non_postgres_database() {
    let config = DatabaseConfig::new("sqlite::memory:")
        .max_connections(1)
        .min_connections(1);
    let db = Database::connect(&config).await.expect("connect sqlite");

    let result = PgGroups::start(&db, Groups::new(), "room_events").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn pg_groups_delivers_across_instances() {
    let url = match postgres_url() {
        Some(url) => url,
        None => return,
    };
    let config = DatabaseConfig::new(url)
        .max_connections(5)
        .min_connections(1);

    // Two connection pools stand in for two server processes.
    let db_a = Database::connect(&config).await.expect("connect server A");
    let db_b = Database::connect(&config).await.expect("connect server B");
    let channel = unique_channel();

    let groups_a = Groups::new();
    let groups_b = Groups::new();
    let server_a = PgGroups::start(&db_a, groups_a.clone(), &channel)
        .await
        .expect("start PgGroups on server A");
    let _server_b = PgGroups::start(&db_b, groups_b.clone(), &channel)
        .await
        .expect("start PgGroups on server B");

    // Subscribe both sides before sending: server A also receives its own
    // notifications through LISTEN, and they arrive asynchronously.
    let mut stream_a = groups_a.group("room").subscribe();
    let mut stream_b = groups_b.group("room").subscribe();
    for message in ["hello", "second"] {
        server_a
            .send("room", message)
            .await
            .expect("send from server A");
    }

    for (name, stream) in [("server B", &mut stream_b), ("server A", &mut stream_a)] {
        for expected in ["hello", "second"] {
            let received = tokio::time::timeout(Duration::from_secs(5), stream.next())
                .await
                .unwrap_or_else(|_| panic!("{name} received {expected:?} in time"));
            assert_eq!(received, Some(expected.to_string()), "{name}");
        }
    }
}

#[tokio::test]
async fn pg_groups_send_rejects_oversized_message() {
    let url = match postgres_url() {
        Some(url) => url,
        None => return,
    };
    let config = DatabaseConfig::new(url)
        .max_connections(5)
        .min_connections(1);
    let db = Database::connect(&config).await.expect("connect postgres");
    let channel = unique_channel();

    let server = PgGroups::start(&db, Groups::new(), &channel)
        .await
        .expect("start PgGroups");

    let too_big = "x".repeat(MAX_NOTIFY_PAYLOAD_BYTES);
    assert!(server.send("room", &too_big).await.is_err());
}
