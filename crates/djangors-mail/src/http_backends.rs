//! Modern HTTP API email transports: Resend, Postmark, SendGrid, and AWS SES v2.

use async_trait::async_trait;
use chrono::Utc;
use hmac::{Hmac, Mac};
use reqwest::Client;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::{MailBackend, MailError, Message};

type HmacSha256 = Hmac<Sha256>;

fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Result<Vec<u8>, MailError> {
    let mut mac =
        HmacSha256::new_from_slice(key).map_err(|e| MailError::Configuration(e.to_string()))?;
    mac.update(data);
    Ok(mac.finalize().into_bytes().to_vec())
}

async fn check_response(resp: reqwest::Response, provider: &str) -> Result<(), MailError> {
    let status = resp.status();
    if status.is_success() {
        Ok(())
    } else {
        let body = resp.text().await.unwrap_or_default();
        Err(MailError::Send(format!("{provider} HTTP {status}: {body}")))
    }
}

/// Resend HTTP API mail transport (`POST /emails`).
#[derive(Debug, Clone)]
pub struct ResendBackend {
    api_key: String,
    base_url: String,
    client: Client,
}

impl ResendBackend {
    /// Create a new [`ResendBackend`] with a Resend API key (`re_...`).
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: "https://api.resend.com".to_string(),
            client: Client::new(),
        }
    }

    /// Override the base URL (useful for local mock servers or proxies).
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into().trim_end_matches('/').to_string();
        self
    }
}

#[async_trait]
impl MailBackend for ResendBackend {
    async fn send(&self, message: &Message) -> Result<(), MailError> {
        let mut payload = json!({
            "from": message.from,
            "to": message.to,
            "subject": message.subject,
            "text": message.body,
        });
        if let Some(obj) = payload.as_object_mut() {
            if let Some(html) = &message.html_body {
                obj.insert("html".to_string(), Value::String(html.clone()));
            }
            if !message.cc.is_empty() {
                obj.insert("cc".to_string(), json!(message.cc));
            }
            if !message.bcc.is_empty() {
                obj.insert("bcc".to_string(), json!(message.bcc));
            }
            if let Some(reply_to) = &message.reply_to {
                obj.insert("reply_to".to_string(), Value::String(reply_to.clone()));
            }
        }
        let resp = self
            .client
            .post(format!("{}/emails", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&payload)
            .send()
            .await
            .map_err(|e| MailError::Send(format!("resend request failed: {e}")))?;
        check_response(resp, "Resend").await
    }
}

/// Postmark HTTP API mail transport (`POST /email`).
#[derive(Debug, Clone)]
pub struct PostmarkBackend {
    server_token: String,
    base_url: String,
    client: Client,
}

impl PostmarkBackend {
    /// Create a new [`PostmarkBackend`] with a Postmark server token.
    pub fn new(server_token: impl Into<String>) -> Self {
        Self {
            server_token: server_token.into(),
            base_url: "https://api.postmarkapp.com".to_string(),
            client: Client::new(),
        }
    }

    /// Override the base URL (useful for local mock servers or proxies).
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into().trim_end_matches('/').to_string();
        self
    }
}

#[async_trait]
impl MailBackend for PostmarkBackend {
    async fn send(&self, message: &Message) -> Result<(), MailError> {
        let mut payload = json!({
            "From": message.from,
            "To": message.to.join(","),
            "Subject": message.subject,
            "TextBody": message.body,
        });
        if let Some(obj) = payload.as_object_mut() {
            if let Some(html) = &message.html_body {
                obj.insert("HtmlBody".to_string(), Value::String(html.clone()));
            }
            if !message.cc.is_empty() {
                obj.insert("Cc".to_string(), Value::String(message.cc.join(",")));
            }
            if !message.bcc.is_empty() {
                obj.insert("Bcc".to_string(), Value::String(message.bcc.join(",")));
            }
            if let Some(reply_to) = &message.reply_to {
                obj.insert("ReplyTo".to_string(), Value::String(reply_to.clone()));
            }
        }
        let resp = self
            .client
            .post(format!("{}/email", self.base_url))
            .header("X-Postmark-Server-Token", &self.server_token)
            .header("Accept", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|e| MailError::Send(format!("postmark request failed: {e}")))?;
        check_response(resp, "Postmark").await
    }
}

/// SendGrid v3 HTTP API mail transport (`POST /v3/mail/send`).
#[derive(Debug, Clone)]
pub struct SendGridBackend {
    api_key: String,
    base_url: String,
    client: Client,
}

impl SendGridBackend {
    /// Create a new [`SendGridBackend`] with a SendGrid API key.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: "https://api.sendgrid.com".to_string(),
            client: Client::new(),
        }
    }

    /// Override the base URL (useful for local mock servers or proxies).
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into().trim_end_matches('/').to_string();
        self
    }
}

#[async_trait]
impl MailBackend for SendGridBackend {
    async fn send(&self, message: &Message) -> Result<(), MailError> {
        let mut personalization = json!({
            "to": message.to.iter().map(|e| json!({ "email": e })).collect::<Vec<_>>(),
        });
        if let Some(obj) = personalization.as_object_mut() {
            if !message.cc.is_empty() {
                obj.insert(
                    "cc".to_string(),
                    json!(message
                        .cc
                        .iter()
                        .map(|e| json!({ "email": e }))
                        .collect::<Vec<_>>()),
                );
            }
            if !message.bcc.is_empty() {
                obj.insert(
                    "bcc".to_string(),
                    json!(message
                        .bcc
                        .iter()
                        .map(|e| json!({ "email": e }))
                        .collect::<Vec<_>>()),
                );
            }
        }

        let mut content = vec![json!({ "type": "text/plain", "value": message.body })];
        if let Some(html) = &message.html_body {
            content.push(json!({ "type": "text/html", "value": html }));
        }

        let mut payload = json!({
            "personalizations": [personalization],
            "from": { "email": message.from },
            "subject": message.subject,
            "content": content,
        });
        if let Some(reply_to) = &message.reply_to {
            if let Some(obj) = payload.as_object_mut() {
                obj.insert("reply_to".to_string(), json!({ "email": reply_to }));
            }
        }

        let resp = self
            .client
            .post(format!("{}/v3/mail/send", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&payload)
            .send()
            .await
            .map_err(|e| MailError::Send(format!("sendgrid request failed: {e}")))?;
        check_response(resp, "SendGrid").await
    }
}

/// AWS SES v2 HTTP API mail transport (`POST /v2/email/outbound-emails`) signed with AWS SigV4.
#[derive(Debug, Clone)]
pub struct SesBackend {
    access_key: String,
    secret_key: String,
    session_token: Option<String>,
    region: String,
    base_url: String,
    client: Client,
}

impl SesBackend {
    /// Create a new [`SesBackend`] for the given AWS region and IAM credentials.
    pub fn new(
        region: impl Into<String>,
        access_key: impl Into<String>,
        secret_key: impl Into<String>,
    ) -> Self {
        let region = region.into();
        let base_url = format!("https://email.{region}.amazonaws.com");
        Self {
            access_key: access_key.into(),
            secret_key: secret_key.into(),
            session_token: None,
            region,
            base_url,
            client: Client::new(),
        }
    }

    /// Attach an optional temporary STS session token (`X-Amz-Security-Token`).
    pub fn with_session_token(mut self, token: impl Into<String>) -> Self {
        self.session_token = Some(token.into());
        self
    }

    /// Override the base URL (useful for LocalStack or mock HTTP tests).
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into().trim_end_matches('/').to_string();
        self
    }

    /// Build the AWS SigV4 `Authorization` header and `x-amz-date` timestamp.
    pub(crate) fn sign_request(
        &self,
        host: &str,
        canonical_uri: &str,
        body: &[u8],
        amz_date: &str,
    ) -> Result<(String, String), MailError> {
        let date_stamp = &amz_date[..8];
        let payload_hash = sha256_hex(body);

        let canonical_headers = format!(
            "content-type:application/json\nhost:{host}\nx-amz-content-sha256:{payload_hash}\nx-amz-date:{amz_date}\n"
        );
        let signed_headers = "content-type;host;x-amz-content-sha256;x-amz-date";
        let canonical_request = format!(
            "POST\n{canonical_uri}\n\n{canonical_headers}\n{signed_headers}\n{payload_hash}"
        );

        let credential_scope = format!("{date_stamp}/{}/ses/aws4_request", self.region);
        let string_to_sign = format!(
            "AWS4-HMAC-SHA256\n{amz_date}\n{credential_scope}\n{}",
            sha256_hex(canonical_request.as_bytes())
        );

        let k_date = hmac_sha256(
            format!("AWS4{}", self.secret_key).as_bytes(),
            date_stamp.as_bytes(),
        )?;
        let k_region = hmac_sha256(&k_date, self.region.as_bytes())?;
        let k_service = hmac_sha256(&k_region, b"ses")?;
        let k_signing = hmac_sha256(&k_service, b"aws4_request")?;
        let signature = hex::encode(hmac_sha256(&k_signing, string_to_sign.as_bytes())?);

        let authorization = format!(
            "AWS4-HMAC-SHA256 Credential={}/{credential_scope}, SignedHeaders={signed_headers}, Signature={signature}",
            self.access_key
        );
        Ok((authorization, payload_hash))
    }
}

#[async_trait]
impl MailBackend for SesBackend {
    async fn send(&self, message: &Message) -> Result<(), MailError> {
        let mut body_obj = json!({
            "Text": { "Data": message.body, "Charset": "UTF-8" }
        });
        if let Some(html) = &message.html_body {
            if let Some(obj) = body_obj.as_object_mut() {
                obj.insert(
                    "Html".to_string(),
                    json!({ "Data": html, "Charset": "UTF-8" }),
                );
            }
        }

        let mut destination = json!({ "ToAddresses": message.to });
        if let Some(obj) = destination.as_object_mut() {
            if !message.cc.is_empty() {
                obj.insert("CcAddresses".to_string(), json!(message.cc));
            }
            if !message.bcc.is_empty() {
                obj.insert("BccAddresses".to_string(), json!(message.bcc));
            }
        }

        let mut payload = json!({
            "FromEmailAddress": message.from,
            "Destination": destination,
            "Content": {
                "Simple": {
                    "Subject": { "Data": message.subject, "Charset": "UTF-8" },
                    "Body": body_obj,
                }
            }
        });
        if let Some(reply_to) = &message.reply_to {
            if let Some(obj) = payload.as_object_mut() {
                obj.insert("ReplyToAddresses".to_string(), json!([reply_to]));
            }
        }

        let body_bytes =
            serde_json::to_vec(&payload).map_err(|e| MailError::Send(e.to_string()))?;
        let url_str = format!("{}/v2/email/outbound-emails", self.base_url);
        let parsed = reqwest::Url::parse(&url_str)
            .map_err(|e| MailError::Configuration(format!("invalid SES base_url: {e}")))?;
        let host = match parsed.host_str() {
            Some(h) => {
                if let Some(p) = parsed.port() {
                    format!("{h}:{p}")
                } else {
                    h.to_string()
                }
            }
            None => {
                return Err(MailError::Configuration("SES URL missing host".into()));
            }
        };

        let amz_date = Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
        let (authorization, payload_hash) =
            self.sign_request(&host, parsed.path(), &body_bytes, &amz_date)?;

        let mut req = self
            .client
            .post(url_str)
            .header("Content-Type", "application/json")
            .header("X-Amz-Date", &amz_date)
            .header("X-Amz-Content-Sha256", &payload_hash)
            .header("Authorization", authorization);
        if let Some(token) = &self.session_token {
            req = req.header("X-Amz-Security-Token", token);
        }
        let resp = req
            .body(body_bytes)
            .send()
            .await
            .map_err(|e| MailError::Send(format!("SES request failed: {e}")))?;
        check_response(resp, "AWS SES").await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    async fn spawn_mock_http() -> (String, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 8192];
            let n = sock.read(&mut buf).await.unwrap();
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
            let resp = "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}";
            sock.write_all(resp.as_bytes()).await.unwrap();
            req
        });
        (format!("http://{addr}"), handle)
    }

    #[tokio::test]
    async fn resend_postmark_sendgrid_and_ses_deliver_expected_requests() {
        let msg = Message::new(
            ["to@example.com"],
            "from@example.com",
            "Hello v1.1",
            "Plain body",
        )
        .html_body("<p>HTML</p>");

        let (url, h) = spawn_mock_http().await;
        ResendBackend::new("re_test_key")
            .with_base_url(url)
            .send(&msg)
            .await
            .unwrap();
        let raw = h.await.unwrap();
        assert!(raw.starts_with("POST /emails "));
        assert!(raw
            .to_ascii_lowercase()
            .contains("authorization: bearer re_test_key"));

        let (url, h) = spawn_mock_http().await;
        PostmarkBackend::new("pm_tok")
            .with_base_url(url)
            .send(&msg)
            .await
            .unwrap();
        let raw = h.await.unwrap();
        assert!(raw.starts_with("POST /email "));
        assert!(raw
            .to_ascii_lowercase()
            .contains("x-postmark-server-token: pm_tok"));

        let (url, h) = spawn_mock_http().await;
        SendGridBackend::new("sg_key")
            .with_base_url(url)
            .send(&msg)
            .await
            .unwrap();
        let raw = h.await.unwrap();
        assert!(raw.starts_with("POST /v3/mail/send "));
        assert!(raw
            .to_ascii_lowercase()
            .contains("authorization: bearer sg_key"));

        let (url, h) = spawn_mock_http().await;
        SesBackend::new("us-east-1", "AKIAEXAMPLE", "secret_key_123")
            .with_base_url(url)
            .send(&msg)
            .await
            .unwrap();
        let raw = h.await.unwrap();
        assert!(raw.starts_with("POST /v2/email/outbound-emails "));
        let lower = raw.to_ascii_lowercase();
        assert!(lower.contains("aws4-hmac-sha256 credential=akiaexample/"));
        assert!(lower.contains("/us-east-1/ses/aws4_request"));
    }
}
