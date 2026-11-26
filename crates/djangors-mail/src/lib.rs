#![deny(missing_docs)]
//! Email messages and pluggable delivery backends.

use async_trait::async_trait;
use lettre::{
    message::{header::ContentType, Mailbox, MultiPart, SinglePart},
    AsyncSmtpTransport, AsyncTransport, Message as LettreMessage, Tokio1Executor,
};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use thiserror::Error;

#[cfg(feature = "http")]
pub mod http_backends;
#[cfg(feature = "http")]
pub use http_backends::{PostmarkBackend, ResendBackend, SendGridBackend, SesBackend};

/// An outgoing email message.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct Message {
    /// Recipient email addresses.
    pub to: Vec<String>,
    /// Sender email address.
    pub from: String,
    /// Email subject line.
    pub subject: String,
    /// Plain text email body.
    pub body: String,
    /// Optional HTML formatted email body.
    pub html_body: Option<String>,
    /// Carbon-copy recipient email addresses.
    pub cc: Vec<String>,
    /// Blind-carbon-copy recipient email addresses.
    pub bcc: Vec<String>,
    /// Optional reply-to email address.
    pub reply_to: Option<String>,
}

impl Message {
    /// Creates a message with required addressing and content fields.
    pub fn new(
        to: impl IntoIterator<Item = impl Into<String>>,
        from: impl Into<String>,
        subject: impl Into<String>,
        body: impl Into<String>,
    ) -> Self {
        Self {
            to: to.into_iter().map(Into::into).collect(),
            from: from.into(),
            subject: subject.into(),
            body: body.into(),
            ..Self::default()
        }
    }

    /// Adds a carbon-copy recipient.
    pub fn cc(mut self, address: impl Into<String>) -> Self {
        self.cc.push(address.into());
        self
    }

    /// Adds a blind-carbon-copy recipient.
    pub fn bcc(mut self, address: impl Into<String>) -> Self {
        self.bcc.push(address.into());
        self
    }

    /// Sets the reply-to address.
    pub fn reply_to(mut self, address: impl Into<String>) -> Self {
        self.reply_to = Some(address.into());
        self
    }

    /// Sets an HTML alternative body.
    pub fn html_body(mut self, body: impl Into<String>) -> Self {
        self.html_body = Some(body.into());
        self
    }
}

/// Errors returned by email backends during delivery or configuration.
#[derive(Error, Debug)]
pub enum MailError {
    /// Email sending failed.
    #[error("failed to send mail: {0}")]
    Send(String),
    /// Invalid mail backend configuration.
    #[error("invalid mail configuration: {0}")]
    Configuration(String),
}

/// Trait defining a pluggable email delivery backend.
#[async_trait]
pub trait MailBackend: Send + Sync {
    /// Delivers the specified email `message`.
    async fn send(&self, message: &Message) -> Result<(), MailError>;
}

/// Email backend that logs outgoing messages via `tracing::info`.
#[derive(Debug, Default, Clone, Copy)]
pub struct ConsoleBackend;

#[async_trait]
impl MailBackend for ConsoleBackend {
    async fn send(&self, message: &Message) -> Result<(), MailError> {
        tracing::info!(to=?message.to, cc=?message.cc, bcc=?message.bcc, from=%message.from, reply_to=?message.reply_to, subject=%message.subject, body=%message.body, html_body=?message.html_body, "ConsoleBackend sending mail");
        Ok(())
    }
}

/// Configuration parameters for SMTP mail delivery.
#[derive(Debug, Clone)]
pub struct SmtpConfig {
    /// Hostname or IP address of the SMTP server.
    pub host: String,
    /// Port number for the SMTP connection.
    pub port: u16,
    /// Optional username for SMTP authentication.
    pub username: Option<String>,
    /// Optional password for SMTP authentication.
    pub password: Option<String>,
    /// Whether to enforce TLS encryption.
    pub use_tls: bool,
}

impl SmtpConfig {
    /// Creates a new `SmtpConfig` targeting `host` on default port 587 with TLS enabled.
    pub fn new(host: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            port: 587,
            username: None,
            password: None,
            use_tls: true,
        }
    }

    /// Sets the SMTP server port.
    pub fn port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    /// Sets the username and password for SMTP authentication.
    pub fn credentials(mut self, username: impl Into<String>, password: impl Into<String>) -> Self {
        self.username = Some(username.into());
        self.password = Some(password.into());
        self
    }

    /// Configures whether TLS encryption should be required.
    pub fn use_tls(mut self, use_tls: bool) -> Self {
        self.use_tls = use_tls;
        self
    }

    fn validate(&self) -> Result<(), MailError> {
        if self.host.trim().is_empty() {
            return Err(MailError::Configuration("SMTP host cannot be empty".into()));
        }
        if (self.username.is_some()) != (self.password.is_some()) {
            return Err(MailError::Configuration(
                "SMTP username and password must be supplied together".into(),
            ));
        }
        Ok(())
    }
}

/// SMTP email delivery backend using Lettre.
pub struct SmtpBackend {
    transport: AsyncSmtpTransport<Tokio1Executor>,
}

impl SmtpBackend {
    /// Constructs a new `SmtpBackend` from `config`.
    pub fn new(config: SmtpConfig) -> Result<Self, MailError> {
        config.validate()?;
        // `relay` uses implicit TLS; when TLS is disabled, `builder_dangerous` is explicit.
        // No TLS downgrade is performed when `use_tls` is true.
        let mut builder = if config.use_tls {
            AsyncSmtpTransport::<Tokio1Executor>::relay(&config.host)
                .map_err(|e| MailError::Configuration(e.to_string()))?
        } else {
            AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&config.host)
        };
        builder = builder.port(config.port);
        if let (Some(user), Some(pass)) = (config.username, config.password) {
            builder = builder.credentials(
                lettre::transport::smtp::authentication::Credentials::new(user, pass),
            );
        }
        Ok(Self {
            transport: builder.build(),
        })
    }

    fn build_message(message: &Message) -> Result<LettreMessage, MailError> {
        let from: Mailbox = message
            .from
            .parse()
            .map_err(|e| MailError::Send(format!("invalid sender: {e}")))?;
        let mut builder = LettreMessage::builder()
            .from(from)
            .subject(&message.subject);
        for to in &message.to {
            builder = builder.to(to
                .parse()
                .map_err(|e| MailError::Send(format!("invalid recipient: {e}")))?);
        }
        for cc in &message.cc {
            builder = builder.cc(cc
                .parse()
                .map_err(|e| MailError::Send(format!("invalid cc recipient: {e}")))?);
        }
        for bcc in &message.bcc {
            builder = builder.bcc(
                bcc.parse()
                    .map_err(|e| MailError::Send(format!("invalid bcc recipient: {e}")))?,
            );
        }
        if let Some(reply_to) = &message.reply_to {
            builder = builder.reply_to(
                reply_to
                    .parse()
                    .map_err(|e| MailError::Send(format!("invalid reply-to address: {e}")))?,
            );
        }
        if let Some(html) = &message.html_body {
            builder
                .multipart(MultiPart::alternative_plain_html(
                    message.body.clone(),
                    html.clone(),
                ))
                .map_err(|e| MailError::Send(e.to_string()))
        } else {
            builder
                .singlepart(
                    SinglePart::builder()
                        .header(ContentType::TEXT_PLAIN)
                        .body(message.body.clone()),
                )
                .map_err(|e| MailError::Send(e.to_string()))
        }
    }

    #[cfg(test)]
    fn serialize(message: &Message) -> Result<Vec<u8>, MailError> {
        Ok(Self::build_message(message)?.formatted())
    }
}

#[async_trait]
impl MailBackend for SmtpBackend {
    async fn send(&self, message: &Message) -> Result<(), MailError> {
        self.transport
            .send(Self::build_message(message)?)
            .await
            .map(|_| ())
            .map_err(|e| MailError::Send(e.to_string()))
    }
}

/// Email backend that writes outgoing messages as `.eml` files to a target directory.
#[derive(Debug, Clone)]
pub struct FileBackend {
    directory: PathBuf,
}

impl FileBackend {
    /// Creates a new `FileBackend` that outputs emails into `directory`.
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }

    fn format(message: &Message) -> String {
        format!(
            "From: {}\nTo: {}\nCc: {}\nBcc: {}\nReply-To: {}\nSubject: {}\n\n{}{}",
            message.from,
            message.to.join(", "),
            message.cc.join(", "),
            message.bcc.join(", "),
            message.reply_to.as_deref().unwrap_or_default(),
            message.subject,
            message.body,
            message
                .html_body
                .as_ref()
                .map(|h| format!("\n\n[HTML body]\n{h}"))
                .unwrap_or_default()
        )
    }
}

#[async_trait]
impl MailBackend for FileBackend {
    async fn send(&self, message: &Message) -> Result<(), MailError> {
        tokio::fs::create_dir_all(&self.directory)
            .await
            .map_err(|e| MailError::Send(e.to_string()))?;
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = self.directory.join(format!("{}.eml", nanos));
        tokio::fs::write(path, Self::format(message))
            .await
            .map_err(|e| MailError::Send(e.to_string()))
    }
}

/// An in-memory email backend for testing that records sent messages.
#[derive(Debug, Clone, Default)]
pub struct InMemoryBackend {
    messages: Arc<Mutex<Vec<Message>>>,
}

impl InMemoryBackend {
    /// Returns a vector clone of all email messages sent through this backend.
    pub fn sent_messages(&self) -> Vec<Message> {
        self.messages
            .lock()
            .expect("in-memory mail mutex poisoned")
            .clone()
    }
}
#[async_trait]
impl MailBackend for InMemoryBackend {
    async fn send(&self, message: &Message) -> Result<(), MailError> {
        self.messages
            .lock()
            .map_err(|_| MailError::Send("in-memory mail mutex poisoned".into()))?
            .push(message.clone());
        Ok(())
    }
}

/// An email recorded by [`CapturingBackend`] for local inspection or dev dashboards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedEmail {
    /// Captured outgoing message.
    pub message: Message,
    /// Unix epoch timestamp (seconds) when the message was captured.
    pub sent_at_unix: u64,
}

/// Wrapper [`MailBackend`] that records a bounded history of sent emails before delegating to `inner`.
#[derive(Debug, Clone)]
pub struct CapturingBackend<B> {
    inner: B,
    captured: Arc<Mutex<VecDeque<CapturedEmail>>>,
    max_entries: usize,
}

impl<B> CapturingBackend<B> {
    /// Wrap `inner`, keeping up to `max_entries` most recent emails in memory.
    pub fn new(inner: B, max_entries: usize) -> Self {
        Self {
            inner,
            captured: Arc::new(Mutex::new(VecDeque::new())),
            max_entries: max_entries.max(1),
        }
    }

    /// Return a snapshot of captured emails (newest last).
    pub fn captured(&self) -> Vec<CapturedEmail> {
        self.captured
            .lock()
            .expect("capturing mail mutex poisoned")
            .iter()
            .cloned()
            .collect()
    }

    /// Clear all captured emails.
    pub fn clear(&self) {
        self.captured
            .lock()
            .expect("capturing mail mutex poisoned")
            .clear();
    }
}

#[async_trait]
impl<B: MailBackend> MailBackend for CapturingBackend<B> {
    async fn send(&self, message: &Message) -> Result<(), MailError> {
        self.inner.send(message).await?;
        let sent_at_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let mut guard = self
            .captured
            .lock()
            .map_err(|_| MailError::Send("capturing mail mutex poisoned".into()))?;
        if guard.len() >= self.max_entries {
            guard.pop_front();
        }
        guard.push_back(CapturedEmail {
            message: message.clone(),
            sent_at_unix,
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests;
