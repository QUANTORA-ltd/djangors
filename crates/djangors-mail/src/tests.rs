use super::*;
fn message() -> Message {
    Message::new(
        ["recipient@example.com"],
        "sender@example.com",
        "Subject",
        "plain body",
    )
    .html_body("<p>html body</p>")
}
#[tokio::test]
async fn console_send() {
    ConsoleBackend.send(&message()).await.unwrap();
}
#[tokio::test]
async fn in_memory_send_and_inspect() {
    let b = InMemoryBackend::default();
    b.send(&message()).await.unwrap();
    assert_eq!(b.sent_messages(), vec![message()]);
}
#[tokio::test]
async fn file_send_can_be_read_back() {
    let dir = std::env::temp_dir().join(format!("djangors-mail-{}", std::process::id()));
    let b = FileBackend::new(&dir);
    b.send(&message()).await.unwrap();
    let mut entries = tokio::fs::read_dir(&dir).await.unwrap();
    let p = entries.next_entry().await.unwrap().unwrap().path();
    let text = tokio::fs::read_to_string(p).await.unwrap();
    assert!(text.contains("plain body") && text.contains("html body"));
    let _ = tokio::fs::remove_dir_all(dir).await;
}
#[test]
fn smtp_config_rejects_empty_host() {
    assert!(SmtpBackend::new(SmtpConfig::new(" ")).is_err());
}
#[test]
fn html_message_is_multipart_alternative() {
    let raw = SmtpBackend::serialize(&message()).unwrap();
    let text = String::from_utf8(raw).unwrap();
    assert!(text.contains("multipart/alternative"));
    assert!(text.contains("plain body") && text.contains("html body"));
}

#[test]
fn message_builder_adds_copy_and_reply_to_headers() {
    let message = Message::new(["to@example.com"], "from@example.com", "Subject", "body")
        .cc("cc@example.com")
        .bcc("bcc@example.com")
        .reply_to("reply@example.com");
    let raw = SmtpBackend::serialize(&message).unwrap();
    let text = String::from_utf8(raw).unwrap();
    assert!(text.contains("Cc: cc@example.com"));
    // Lettre deliberately removes Bcc from the serialized message headers.
    assert_eq!(message.bcc, ["bcc@example.com"]);
    assert!(text.contains("Reply-To: reply@example.com"));
}
