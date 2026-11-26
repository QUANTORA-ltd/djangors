use super::*;
use djangors_db::DatabaseConfig;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

async fn setup_db() -> Database {
    let config = DatabaseConfig::new("sqlite::memory:")
        .max_connections(1)
        .min_connections(1);
    let db = Database::connect(&config).await.unwrap();
    create_webhooks_tables(&db).await.unwrap();
    db
}

#[derive(Debug, Clone)]
struct CapturedRequest {
    headers: std::collections::HashMap<String, String>,
    body: String,
}

async fn spawn_mock_webhook_server(
    statuses: Vec<u16>,
) -> (String, Arc<Mutex<Vec<CapturedRequest>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let captured = Arc::new(Mutex::new(Vec::new()));
    let captured_clone = Arc::clone(&captured);
    let counter = Arc::new(AtomicUsize::new(0));

    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            let captured_inner = Arc::clone(&captured_clone);
            let idx = counter.fetch_add(1, Ordering::SeqCst);
            let status = statuses.get(idx).copied().unwrap_or(200);

            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut chunk = [0_u8; 4096];
                let header_end;
                loop {
                    let n = socket.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        header_end = pos + 4;
                        break;
                    }
                }

                let header_text = String::from_utf8_lossy(&buf[..header_end]);
                let mut headers = std::collections::HashMap::new();
                let mut content_length = 0_usize;
                for line in header_text.lines().skip(1) {
                    if let Some((k, v)) = line.split_once(':') {
                        let key = k.trim().to_ascii_lowercase();
                        let val = v.trim().to_string();
                        if key == "content-length" {
                            content_length = val.parse().unwrap_or(0);
                        }
                        headers.insert(key, val);
                    }
                }

                while buf.len() < header_end + content_length {
                    let n = socket.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                }

                let body_bytes = &buf[header_end..(header_end + content_length).min(buf.len())];
                let body = String::from_utf8_lossy(body_bytes).to_string();

                captured_inner
                    .lock()
                    .unwrap()
                    .push(CapturedRequest { headers, body });

                let reason = if (200..300).contains(&status) {
                    "OK"
                } else {
                    "Internal Server Error"
                };
                let resp_body = format!("status={status}");
                let resp = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{resp_body}",
                    resp_body.len()
                );
                let _ = socket.write_all(resp.as_bytes()).await;
                let _ = socket.shutdown().await;
            });
        }
    });

    (format!("http://{addr}/webhook"), captured)
}

#[test]
fn signature_roundtrip_and_tamper_and_replay_window() {
    let secret = "whsec_test_secret";
    let payload = br#"{"id":"evt_1","type":"invoice.paid"}"#;
    let now = 1_700_000_000_i64;

    let header = sign_payload(secret, now, payload);
    assert!(verify_signature(secret, &header, payload, now + 10, 300).is_ok());

    // Tampered payload is rejected.
    let tampered = br#"{"id":"evt_1","type":"invoice.refunded"}"#;
    assert!(matches!(
        verify_signature(secret, &header, tampered, now + 10, 300),
        Err(WebhookError::InvalidSignature)
    ));

    // Wrong secret is rejected.
    assert!(matches!(
        verify_signature("wrong_secret", &header, payload, now + 10, 300),
        Err(WebhookError::InvalidSignature)
    ));

    // Malformed header is rejected.
    assert!(matches!(
        verify_signature(secret, "malformed", payload, now, 300),
        Err(WebhookError::InvalidSignature)
    ));

    // Expired timestamp outside replay window is rejected.
    assert!(matches!(
        verify_signature(secret, &header, payload, now + 301, 300),
        Err(WebhookError::ExpiredTimestamp)
    ));
    assert!(matches!(
        verify_signature(secret, &header, payload, now - 301, 300),
        Err(WebhookError::ExpiredTimestamp)
    ));
}

#[tokio::test]
async fn end_to_end_delivery_retry_and_replay() {
    let db = setup_db().await;
    let (url, captured) = spawn_mock_webhook_server(vec![500, 500, 200]).await;

    let dispatcher = WebhookDispatcher::new(db.clone()).with_max_attempts(2);
    let secret = "whsec_live_123";
    let endpoint = dispatcher
        .register_endpoint("tenant_1", &url, secret, &["invoice.*".trim(), "invoice.paid"], "Billing")
        .await
        .unwrap();
    assert!(endpoint.enabled);

    // Non-matching event should produce no deliveries.
    let none_ids = dispatcher
        .emit("tenant_1", "user.created", &serde_json::json!({"id": 1}))
        .await
        .unwrap();
    assert!(none_ids.is_empty());

    // Matching event enqueues one delivery.
    let payload = serde_json::json!({"invoice_id": 42});
    let ids = dispatcher
        .emit("tenant_1", "invoice.paid", &payload)
        .await
        .unwrap();
    assert_eq!(ids.len(), 1);
    let delivery_id = ids[0];

    // Attempt 1 -> 500 -> scheduled for retry (attempts = 1, status = pending, next_retry_at > now).
    let report1 = dispatcher.deliver_due().await.unwrap();
    assert_eq!(
        report1,
        DeliveryBatchReport {
            attempted: 1,
            delivered: 0,
            scheduled_retry: 1,
            failed: 0,
        }
    );
    let after1 = WebhookDelivery::objects()
        .filter(q!(id = delivery_id))
        .unwrap()
        .get(&db)
        .await
        .unwrap();
    assert_eq!(after1.status, "pending");
    assert_eq!(after1.attempts, 1);
    assert_eq!(after1.last_status_code, Some(500));
    assert!(after1.next_retry_at.unwrap() > Utc::now());

    // Calling deliver_due immediately before backoff elapses should skip it.
    let skipped = dispatcher.deliver_due().await.unwrap();
    assert_eq!(skipped, DeliveryBatchReport::default());

    // Fast-forward next_retry_at to simulate backoff elapsed.
    let mut due_again = after1.clone();
    due_again.next_retry_at = Some(Utc::now() - Duration::seconds(1));
    due_again.update(&db).await.unwrap();

    // Attempt 2 -> 500 -> hits max_attempts (2) -> marked "failed".
    let report2 = dispatcher.deliver_due().await.unwrap();
    assert_eq!(
        report2,
        DeliveryBatchReport {
            attempted: 1,
            delivered: 0,
            scheduled_retry: 0,
            failed: 1,
        }
    );
    let after2 = WebhookDelivery::objects()
        .filter(q!(id = delivery_id))
        .unwrap()
        .get(&db)
        .await
        .unwrap();
    assert_eq!(after2.status, "failed");
    assert_eq!(after2.attempts, 2);

    // 1-click replay resets status to pending and attempts to 0.
    dispatcher.replay(delivery_id).await.unwrap();
    let report3 = dispatcher.deliver_due().await.unwrap();
    assert_eq!(
        report3,
        DeliveryBatchReport {
            attempted: 1,
            delivered: 1,
            scheduled_retry: 0,
            failed: 0,
        }
    );
    let after3 = WebhookDelivery::objects()
        .filter(q!(id = delivery_id))
        .unwrap()
        .get(&db)
        .await
        .unwrap();
    assert_eq!(after3.status, "delivered");
    assert_eq!(after3.attempts, 1);
    assert_eq!(after3.last_status_code, Some(200));
    assert!(after3.delivered_at.is_some());

    // Verify captured HTTP request headers & HMAC signature on the server.
    {
        let reqs = captured.lock().unwrap();
        assert_eq!(reqs.len(), 3);
        let last_req = &reqs[2];
        assert_eq!(
            last_req
                .headers
                .get(&EVENT_HEADER.to_ascii_lowercase())
                .map(String::as_str),
            Some("invoice.paid")
        );
        let sig_header = last_req
            .headers
            .get(&SIGNATURE_HEADER.to_ascii_lowercase())
            .expect("missing signature header");
        assert!(verify_signature(
            secret,
            sig_header,
            last_req.body.as_bytes(),
            Utc::now().timestamp(),
            300
        )
        .is_ok());
    }

    // Replaying a non-existent delivery returns NotFound.
    assert!(matches!(
        dispatcher.replay(999_999).await,
        Err(WebhookError::NotFound)
    ));
}

#[cfg(feature = "admin")]
#[test]
fn register_admin_registers_webhook_models() {
    let site = djangors_admin::AdminSite::new();
    register_admin(&site);
}
