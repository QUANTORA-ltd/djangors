# How to Send Email

## Problem
You want to send plain-text or HTML emails in your Djangors application using SMTP, or write outgoing messages to local disk during development.

## Solution
Use `djangors_mail::Message` and configure a backend implementing the `MailBackend` trait, such as `SmtpBackend` (powered by `lettre` with `rustls` TLS — zero OpenSSL or `native-tls` C dependencies), `FileBackend`, `ConsoleBackend`, or `InMemoryBackend`. Note that `Message` is `#[non_exhaustive]`; construct it via `Message::new(to, from, subject, text_body)` and attach optional HTML bodies with `.html_body(...)`.

## Code Example

### 1. Sending via SMTP (Production)

```rust,compile
use djangors_mail::{Message, SmtpConfig, SmtpBackend, MailBackend};

pub async fn send_welcome_email(user_email: &str) -> Result<(), djangors_mail::MailError> {
    // Construct email message
    let message = Message::new(
        [user_email],
        "noreply@example.com",
        "Welcome to our platform!",
        "Hello, welcome to our service!",
    )
    .html_body("<h1>Hello!</h1><p>Welcome to our service!</p>");

    // Configure SMTP backend
    let config = SmtpConfig::new("smtp.example.com")
        .port(587)
        .credentials("smtp_user", "smtp_pass")
        .use_tls(true);

    let backend = SmtpBackend::new(config)?;
    backend.send(&message).await?;

    Ok(())
}
```

### 2. Saving Emails to Disk (Development)

```rust,compile
use djangors_mail::{Message, FileBackend, MailBackend};
use std::path::PathBuf;

pub async fn send_dev_email(message: &Message) -> Result<(), djangors_mail::MailError> {
    // Writes emails as `.eml` files in the specified directory
    let backend = FileBackend::new(PathBuf::from("/tmp/app_emails"));
    backend.send(message).await?;
    Ok(())
}
```

### 3. Asserting Sent Emails in Tests (`InMemoryBackend`)

`InMemoryBackend` keeps every sent `Message` in memory and exposes
`sent_messages()` — pair it with `djangors-test` or a plain `#[tokio::test]`
when your test asserts that "an email was sent with this subject":

```rust,compile
# use djangors_mail::{Message, InMemoryBackend, MailBackend};
# fn build_message() -> Message { Message::new(["a@b.c"], "noreply@example.com", "reset", "hi") }
# async fn run() -> Result<(), djangors_mail::MailError> {
let backend = InMemoryBackend::default();
let sent: Result<(), _> = backend.send(&build_message()).await;
assert!(sent.is_ok());

let messages = backend.sent_messages();
assert_eq!(messages.len(), 1);
assert_eq!(messages[0].subject, "reset");
# Ok(())
# }
```

`ConsoleBackend` is the fourth backend: it logs the message body via `tracing`
to stdout/stderr, useful for local development when you want to *see* emails
without an SMTP server or files on disk.
