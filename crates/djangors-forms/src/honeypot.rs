//! Bot-mitigation form helpers: [`HoneypotField`], [`MinSubmitTime`],
//! User-Agent checks, and suspicious-submission escalation to CAPTCHA.

use base64::Engine as _;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use thiserror::Error;

type HmacSha256 = Hmac<Sha256>;

/// Domain-separation prefix: callers usually pass the app `SECRET_KEY`, so the MAC
/// must not be valid for any other token type signed with that key.
const MIN_SUBMIT_PURPOSE: &[u8] = b"djangors.forms.min-submit-time:";

/// Errors produced by honeypot or minimum-submit-time checks.
#[derive(Debug, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum HoneypotError {
    /// The hidden honeypot field was filled in (likely an automated bot).
    #[error("honeypot field must be empty")]
    HoneypotFilled,
    /// The signed timestamp token was missing, malformed, or tampered with.
    #[error("invalid or missing form timestamp")]
    InvalidTimestamp,
    /// The form was submitted faster than the configured minimum duration.
    #[error("form submitted too quickly")]
    SubmittedTooFast,
    /// The request had an empty or obvious-bot User-Agent header.
    #[error("suspicious user agent")]
    SuspiciousUserAgent,
}

/// Hidden form input that must remain empty on submission.
#[derive(Debug, Clone)]
pub struct HoneypotField {
    /// HTML input `name` attribute (defaults to `"website"`).
    pub name: String,
}

impl Default for HoneypotField {
    fn default() -> Self {
        Self::new("website")
    }
}

impl HoneypotField {
    /// Creates a [`HoneypotField`] with the given input `name`.
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }

    /// Renders a visually hidden `<input>` element.
    pub fn render(&self) -> String {
        let escaped = self
            .name
            .replace('&', "&amp;")
            .replace('"', "&quot;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        format!(
            "<div style=\"position:absolute;left:-9999px;\" aria-hidden=\"true\">\
             <input type=\"text\" name=\"{escaped}\" value=\"\" tabindex=\"-1\" autocomplete=\"off\">\
             </div>"
        )
    }

    /// Validates that the submitted value is absent or empty.
    pub fn validate(&self, submitted: Option<&str>) -> Result<(), HoneypotError> {
        match submitted {
            None | Some("") => Ok(()),
            Some(_) => Err(HoneypotError::HoneypotFilled),
        }
    }
}

/// Signed timestamp field that rejects submissions faster than `min_duration`.
#[derive(Debug, Clone)]
pub struct MinSubmitTime {
    /// HTML hidden input `name` attribute (defaults to `"_form_ts"`).
    pub field_name: String,
    /// Minimum elapsed time required between rendering and submission.
    pub min_duration: Duration,
    /// HMAC secret key used to sign timestamps.
    secret: Vec<u8>,
}

impl MinSubmitTime {
    /// Creates a [`MinSubmitTime`] guard requiring at least `min_duration` before submit.
    pub fn new(secret: impl AsRef<[u8]>, min_duration: Duration) -> Self {
        Self {
            field_name: "_form_ts".to_string(),
            min_duration,
            secret: secret.as_ref().to_vec(),
        }
    }

    /// Overrides the hidden input name.
    pub fn with_field_name(mut self, name: impl Into<String>) -> Self {
        self.field_name = name.into();
        self
    }

    /// Generates a signed timestamp token for the current Unix timestamp.
    pub fn generate_token(&self) -> String {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_secs();
        self.sign_timestamp(now)
    }

    /// Generates a signed timestamp token for an explicit Unix timestamp (useful in tests).
    pub fn sign_timestamp(&self, issued_at_unix_secs: u64) -> String {
        let mut mac =
            HmacSha256::new_from_slice(&self.secret).expect("HMAC accepts any key length");
        mac.update(MIN_SUBMIT_PURPOSE);
        mac.update(issued_at_unix_secs.to_string().as_bytes());
        let sig = mac.finalize().into_bytes();
        let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sig);
        format!("{issued_at_unix_secs}.{b64}")
    }

    /// Renders the hidden `<input type="hidden">` containing the signed timestamp.
    pub fn render(&self) -> String {
        let token = self.generate_token();
        format!(
            "<input type=\"hidden\" name=\"{}\" value=\"{token}\">",
            self.field_name
        )
    }

    /// Validates that `token` has a valid HMAC signature and was issued at least
    /// `self.min_duration` ago.
    pub fn validate(&self, token: Option<&str>) -> Result<(), HoneypotError> {
        let raw = token.ok_or(HoneypotError::InvalidTimestamp)?;
        let (ts_str, b64_sig) = raw.split_once('.').ok_or(HoneypotError::InvalidTimestamp)?;
        let issued_at: u64 = ts_str
            .parse()
            .map_err(|_| HoneypotError::InvalidTimestamp)?;
        let sig_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(b64_sig)
            .map_err(|_| HoneypotError::InvalidTimestamp)?;

        let mut mac = HmacSha256::new_from_slice(&self.secret)
            .map_err(|_| HoneypotError::InvalidTimestamp)?;
        mac.update(MIN_SUBMIT_PURPOSE);
        mac.update(ts_str.as_bytes());
        mac.verify_slice(&sig_bytes)
            .map_err(|_| HoneypotError::InvalidTimestamp)?;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_secs();
        if now.saturating_sub(issued_at) < self.min_duration.as_secs() {
            return Err(HoneypotError::SubmittedTooFast);
        }
        Ok(())
    }
}

/// Returns `true` if `user_agent` is missing, empty, or matches an obvious automated client.
pub fn is_suspicious_user_agent(user_agent: Option<&str>) -> bool {
    let Some(ua) = user_agent.map(str::trim) else {
        return true;
    };
    if ua.is_empty() {
        return true;
    }
    let lower = ua.to_ascii_lowercase();
    const BOT_MARKERS: &[&str] = &[
        "curl/",
        "wget/",
        "python-requests",
        "python-urllib",
        "httpclient",
        "scrapy",
        "bot/",
        "headlesschrome",
    ];
    BOT_MARKERS.iter().any(|m| lower.contains(m))
}

/// Outcome of combining [`HoneypotField`], [`MinSubmitTime`], and User-Agent checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubmissionCheck {
    /// No bot signals tripped.
    Clean,
    /// One or more bot signals tripped; callers can escalate by showing a CAPTCHA.
    Suspicious {
        /// Reasons that triggered suspicion.
        reasons: Vec<String>,
        /// Whether a CAPTCHA challenge should be shown on retry.
        show_captcha: bool,
    },
}

/// Combines honeypot, minimum-submit-time, and optional User-Agent checks, allowing callers
/// to show a `CaptchaField` only when a submission looks suspicious.
#[derive(Debug, Clone)]
pub struct BotGuard {
    /// Honeypot field.
    pub honeypot: HoneypotField,
    /// Minimum submit time guard.
    pub min_time: MinSubmitTime,
    /// Whether to flag empty/bot User-Agent headers.
    pub check_user_agent: bool,
}

impl BotGuard {
    /// Creates a [`BotGuard`] with default [`HoneypotField`], [`MinSubmitTime`], and UA checking enabled.
    pub fn new(secret: impl AsRef<[u8]>, min_duration: Duration) -> Self {
        Self {
            honeypot: HoneypotField::default(),
            min_time: MinSubmitTime::new(secret, min_duration),
            check_user_agent: true,
        }
    }

    /// Evaluates the submission and returns [`SubmissionCheck::Suspicious`] with
    /// `show_captcha: true` if any check trips.
    pub fn evaluate(
        &self,
        honeypot_value: Option<&str>,
        timestamp_token: Option<&str>,
        user_agent: Option<&str>,
    ) -> SubmissionCheck {
        let mut reasons = Vec::new();
        if let Err(e) = self.honeypot.validate(honeypot_value) {
            reasons.push(e.to_string());
        }
        if let Err(e) = self.min_time.validate(timestamp_token) {
            reasons.push(e.to_string());
        }
        if self.check_user_agent && is_suspicious_user_agent(user_agent) {
            reasons.push(HoneypotError::SuspiciousUserAgent.to_string());
        }
        if reasons.is_empty() {
            SubmissionCheck::Clean
        } else {
            SubmissionCheck::Suspicious {
                reasons,
                show_captcha: true,
            }
        }
    }
}
