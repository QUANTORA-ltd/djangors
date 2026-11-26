use std::sync::{Arc, Mutex};

use djangors_mail::InMemoryBackend;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::*;

async fn setup_db() -> Database {
    let config = djangors_db::DatabaseConfig::new("sqlite::memory:")
        .max_connections(1)
        .min_connections(1);
    let db = Database::connect(&config).await.unwrap();
    create_notifications_tables(&db).await.unwrap();
    db
}

struct OrderShipped {
    order_id: i64,
    channels: Vec<NotificationChannel>,
}

impl Notification for OrderShipped {
    fn kind(&self) -> &str {
        "order.shipped"
    }

    fn channels(&self) -> Vec<NotificationChannel> {
        self.channels.clone()
    }

    fn subject(&self) -> String {
        format!("Order #{} shipped", self.order_id)
    }

    fn body(&self) -> String {
        format!("Your order #{} is on the way.", self.order_id)
    }

    fn html_body(&self) -> Option<String> {
        Some(format!("<p>Your order #{} is on the way.</p>", self.order_id))
    }

    fn data(&self) -> serde_json::Value {
        serde_json::json!({ "order_id": self.order_id })
    }
}

#[derive(Debug, Clone)]
struct CapturedRequest {
    path: String,
    body: serde_json::Value,
}

async fn spawn_mock_webhook_server(
    expected_requests: usize,
) -> (String, Arc<Mutex<Vec<CapturedRequest>>>, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let captured = Arc::new(Mutex::new(Vec::new()));
    let captured_clone = Arc::clone(&captured);

    let handle = tokio::spawn(async move {
        for _ in 0..expected_requests {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = Vec::new();
            let mut tmp = [0u8; 4096];
            loop {
                let n = stream.read(&mut tmp).await.unwrap();
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&tmp[..n]);
                if let Some(header_end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&buf[..header_end]);
                    let content_length = headers
                        .lines()
                        .find_map(|line| {
                            let (k, v) = line.split_once(':')?;
                            if k.eq_ignore_ascii_case("content-length") {
                                v.trim().parse::<usize>().ok()
                            } else {
                                None
                            }
                        })
                        .unwrap_or(0);
                    let body_start = header_end + 4;
                    if buf.len() >= body_start + content_length {
                        let request_line = headers.lines().next().unwrap_or("");
                        let path = request_line
                            .split_whitespace()
                            .nth(1)
                            .unwrap_or("/")
                            .to_string();
                        let body_bytes = &buf[body_start..body_start + content_length];
                        let body: serde_json::Value =
                            serde_json::from_slice(body_bytes).unwrap();
                        captured_clone
                            .lock()
                            .unwrap()
                            .push(CapturedRequest { path, body });
                        break;
                    }
                }
            }
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .await
                .unwrap();
        }
    });

    (format!("http://{addr}"), captured, handle)
}

#[tokio::test]
async fn test_multi_channel_delivery_and_webhook_payloads() {
    let db = setup_db().await;
    let mail = Arc::new(InMemoryBackend::default());
    let (base_url, captured, server_handle) = spawn_mock_webhook_server(3).await;

    let dispatcher = NotificationDispatcher::new(db.clone())
        .with_mail(mail.clone(), "alerts@example.com");

    let recipient = NotificationRecipient::new("user-42")
        .email("user42@example.com")
        .webhook(format!("{base_url}/webhook"))
        .slack(format!("{base_url}/slack"))
        .discord(format!("{base_url}/discord"));

    let notif = OrderShipped {
        order_id: 101,
        channels: vec![
            NotificationChannel::InApp,
            NotificationChannel::Email,
            NotificationChannel::Webhook,
            NotificationChannel::Slack,
            NotificationChannel::Discord,
        ],
    };

    let report = dispatcher.send(&recipient, &notif).await.unwrap();
    server_handle.await.unwrap();

    assert_eq!(report.delivered, notif.channels());
    assert!(report.skipped_by_preference.is_empty());

    // Email verified
    let sent_emails = mail.sent_messages();
    assert_eq!(sent_emails.len(), 1);
    assert_eq!(sent_emails[0].to, vec!["user42@example.com"]);
    assert_eq!(sent_emails[0].from, "alerts@example.com");
    assert_eq!(sent_emails[0].subject, "Order #101 shipped");
    assert_eq!(
        sent_emails[0].html_body.as_deref(),
        Some("<p>Your order #101 is on the way.</p>")
    );

    // Webhook, Slack, Discord payloads verified
    let reqs = captured.lock().unwrap().clone();
    assert_eq!(reqs.len(), 3);
    assert_eq!(reqs[0].path, "/webhook");
    assert_eq!(reqs[0].body["kind"], "order.shipped");
    assert_eq!(reqs[0].body["title"], "Order #101 shipped");
    assert_eq!(reqs[0].body["data"]["order_id"], 101);

    assert_eq!(reqs[1].path, "/slack");
    assert_eq!(
        reqs[1].body["text"],
        "*Order #101 shipped*\nYour order #101 is on the way."
    );

    assert_eq!(reqs[2].path, "/discord");
    assert_eq!(
        reqs[2].body["content"],
        "**Order #101 shipped**\nYour order #101 is on the way."
    );

    // In-app inbox verified
    assert_eq!(dispatcher.unread_count("user-42").await.unwrap(), 1);
    let inbox = dispatcher.list_for_user("user-42", true, 10).await.unwrap();
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0].title, "Order #101 shipped");
    assert_eq!(inbox[0].data_json, r#"{"order_id":101}"#);
}

#[tokio::test]
async fn test_preference_opt_out_and_re_enable() {
    let db = setup_db().await;
    let mail = Arc::new(InMemoryBackend::default());
    let dispatcher = NotificationDispatcher::new(db.clone())
        .with_mail(mail.clone(), "alerts@example.com");

    dispatcher
        .set_preference("user-1", "order.shipped", NotificationChannel::Email, false)
        .await
        .unwrap();

    let recipient = NotificationRecipient::new("user-1").email("user1@example.com");
    let notif = OrderShipped {
        order_id: 202,
        channels: vec![NotificationChannel::InApp, NotificationChannel::Email],
    };

    let report = dispatcher.send(&recipient, &notif).await.unwrap();
    assert_eq!(report.delivered, vec![NotificationChannel::InApp]);
    assert_eq!(
        report.skipped_by_preference,
        vec![NotificationChannel::Email]
    );
    assert!(mail.sent_messages().is_empty());

    // Re-enable email preference
    let updated = dispatcher
        .set_preference("user-1", "order.shipped", NotificationChannel::Email, true)
        .await
        .unwrap();
    assert!(updated.enabled);

    let report2 = dispatcher.send(&recipient, &notif).await.unwrap();
    assert_eq!(
        report2.delivered,
        vec![NotificationChannel::InApp, NotificationChannel::Email]
    );
    assert!(report2.skipped_by_preference.is_empty());
    assert_eq!(mail.sent_messages().len(), 1);
}

#[tokio::test]
async fn test_inbox_list_and_mark_read() {
    let db = setup_db().await;
    let dispatcher = NotificationDispatcher::new(db.clone());
    let recipient = NotificationRecipient::new("user-9");

    for order_id in [1, 2, 3] {
        let notif = OrderShipped {
            order_id,
            channels: vec![NotificationChannel::InApp],
        };
        dispatcher.send(&recipient, &notif).await.unwrap();
    }

    assert_eq!(dispatcher.unread_count("user-9").await.unwrap(), 3);
    let all = dispatcher.list_for_user("user-9", false, 10).await.unwrap();
    assert_eq!(all.len(), 3);

    // Mark newest one as read
    let updated = dispatcher.mark_read(all[0].id, "user-9").await.unwrap();
    assert_eq!(updated, 1);
    assert_eq!(dispatcher.unread_count("user-9").await.unwrap(), 2);

    // Wrong user cannot mark someone else's notification read
    let wrong_user = dispatcher.mark_read(all[1].id, "other-user").await.unwrap();
    assert_eq!(wrong_user, 0);
    assert_eq!(dispatcher.unread_count("user-9").await.unwrap(), 2);

    // Unread filter only returns 2
    let unread = dispatcher.list_for_user("user-9", true, 10).await.unwrap();
    assert_eq!(unread.len(), 2);

    // Mark all read
    let marked_all = dispatcher.mark_all_read("user-9").await.unwrap();
    assert_eq!(marked_all, 2);
    assert_eq!(dispatcher.unread_count("user-9").await.unwrap(), 0);
    assert!(dispatcher
        .list_for_user("user-9", true, 10)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn test_missing_target_errors() {
    let db = setup_db().await;
    let dispatcher = NotificationDispatcher::new(db);
    let recipient = NotificationRecipient::new("user-missing");

    let email_notif = OrderShipped {
        order_id: 1,
        channels: vec![NotificationChannel::Email],
    };
    assert!(matches!(
        dispatcher.send(&recipient, &email_notif).await,
        Err(NotificationError::MissingTarget(_))
    ));

    let webhook_notif = OrderShipped {
        order_id: 1,
        channels: vec![NotificationChannel::Webhook],
    };
    assert!(matches!(
        dispatcher.send(&recipient, &webhook_notif).await,
        Err(NotificationError::MissingTarget(_))
    ));
}

#[cfg(feature = "admin")]
#[test]
fn test_register_admin() {
    let site = djangors_admin::AdminSite::new();
    register_admin(&site);
}
