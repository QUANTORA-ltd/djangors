//! CAPTCHA verification (Cloudflare Turnstile, hCaptcha, reCAPTCHA v2/v3),
//! form field rendering, and JSON API extractor (`VerifiedCaptcha`).

use async_trait::async_trait;
use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;

/// Errors returned by [`CaptchaVerifier::verify`].
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CaptchaError {
    /// Missing or empty CAPTCHA response token.
    #[error("missing captcha token")]
    MissingToken,
    /// Network or timeout error contacting the CAPTCHA provider (fails closed).
    #[error("captcha verification request failed: {0}")]
    Network(String),
    /// Provider rejected the CAPTCHA token or score was below threshold.
    #[error("captcha verification failed: {0}")]
    Rejected(String),
}

/// Outcome returned by a [`CaptchaVerifier`].
#[derive(Debug, Clone, PartialEq)]
pub struct CaptchaOutcome {
    /// Whether the CAPTCHA challenge passed.
    pub success: bool,
    /// Optional score (populated by reCAPTCHA v3).
    pub score: Option<f64>,
    /// Optional action name (populated by reCAPTCHA v3 / Turnstile).
    pub action: Option<String>,
    /// Provider error codes, if any.
    pub error_codes: Vec<String>,
}

/// Supported CAPTCHA providers.
#[derive(Debug, Clone, PartialEq)]
pub enum CaptchaProvider {
    /// Cloudflare Turnstile.
    Turnstile,
    /// hCaptcha.
    HCaptcha,
    /// Google reCAPTCHA v2.
    RecaptchaV2,
    /// Google reCAPTCHA v3 with a minimum score threshold (e.g. `0.5`).
    RecaptchaV3 {
        /// Minimum score required to pass (`0.0..=1.0`).
        min_score: f64,
    },
}

/// Settings for configuring a CAPTCHA provider and widget.
#[derive(Debug, Clone)]
pub struct CaptchaConfig {
    /// Provider variant.
    pub provider: CaptchaProvider,
    /// Public site key rendered in HTML widgets.
    pub site_key: String,
    /// Server-side secret key used for siteverify calls.
    pub secret: String,
    /// HTTP request timeout (network failures fail closed).
    pub timeout: Duration,
}

impl CaptchaConfig {
    /// Creates a new [`CaptchaConfig`] with a default 5-second timeout.
    pub fn new(
        provider: CaptchaProvider,
        site_key: impl Into<String>,
        secret: impl Into<String>,
    ) -> Self {
        Self {
            provider,
            site_key: site_key.into(),
            secret: secret.into(),
            timeout: Duration::from_secs(5),
        }
    }

    /// Sets the verification HTTP request timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Builds the corresponding [`CaptchaVerifier`] implementation.
    pub fn build_verifier(&self) -> Arc<dyn CaptchaVerifier> {
        match self.provider {
            CaptchaProvider::Turnstile => {
                Arc::new(TurnstileVerifier::new(self.secret.clone(), self.timeout))
            }
            CaptchaProvider::HCaptcha => {
                Arc::new(HCaptchaVerifier::new(self.secret.clone(), self.timeout))
            }
            CaptchaProvider::RecaptchaV2 => {
                Arc::new(RecaptchaV2Verifier::new(self.secret.clone(), self.timeout))
            }
            CaptchaProvider::RecaptchaV3 { min_score } => Arc::new(RecaptchaV3Verifier::new(
                self.secret.clone(),
                min_score,
                self.timeout,
            )),
        }
    }
}

/// Pluggable CAPTCHA verification backend.
#[async_trait]
pub trait CaptchaVerifier: Send + Sync + 'static {
    /// Verifies `token` with optional `remote_ip`. Network failures must fail closed (`Err`).
    async fn verify(
        &self,
        token: &str,
        remote_ip: Option<&str>,
    ) -> Result<CaptchaOutcome, CaptchaError>;
}

#[derive(Deserialize)]
struct SiteVerifyResponse {
    success: bool,
    score: Option<f64>,
    action: Option<String>,
    #[serde(rename = "error-codes", default)]
    error_codes: Vec<String>,
}

async fn post_siteverify(
    endpoint: &str,
    secret: &str,
    token: &str,
    remote_ip: Option<&str>,
    timeout: Duration,
) -> Result<SiteVerifyResponse, CaptchaError> {
    if token.trim().is_empty() {
        return Err(CaptchaError::MissingToken);
    }
    let client = reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|e| CaptchaError::Network(e.to_string()))?;

    let mut params = vec![("secret", secret), ("response", token)];
    if let Some(ip) = remote_ip {
        params.push(("remoteip", ip));
    }

    let resp = client
        .post(endpoint)
        .form(&params)
        .send()
        .await
        .map_err(|e| CaptchaError::Network(e.to_string()))?;

    if !resp.status().is_success() {
        return Err(CaptchaError::Network(format!(
            "unexpected status {}",
            resp.status()
        )));
    }

    resp.json::<SiteVerifyResponse>()
        .await
        .map_err(|e| CaptchaError::Network(e.to_string()))
}

/// Cloudflare Turnstile verifier.
pub struct TurnstileVerifier {
    secret: String,
    timeout: Duration,
    endpoint: String,
}

impl TurnstileVerifier {
    /// Creates a new [`TurnstileVerifier`].
    pub fn new(secret: impl Into<String>, timeout: Duration) -> Self {
        Self {
            secret: secret.into(),
            timeout,
            endpoint: "https://challenges.cloudflare.com/turnstile/v0/siteverify".to_string(),
        }
    }

    /// Overrides the verification endpoint URL (useful for local mock servers).
    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = endpoint.into();
        self
    }
}

#[async_trait]
impl CaptchaVerifier for TurnstileVerifier {
    async fn verify(
        &self,
        token: &str,
        remote_ip: Option<&str>,
    ) -> Result<CaptchaOutcome, CaptchaError> {
        let res =
            post_siteverify(&self.endpoint, &self.secret, token, remote_ip, self.timeout).await?;
        Ok(CaptchaOutcome {
            success: res.success,
            score: res.score,
            action: res.action,
            error_codes: res.error_codes,
        })
    }
}

/// hCaptcha verifier.
pub struct HCaptchaVerifier {
    secret: String,
    timeout: Duration,
    endpoint: String,
}

impl HCaptchaVerifier {
    /// Creates a new [`HCaptchaVerifier`].
    pub fn new(secret: impl Into<String>, timeout: Duration) -> Self {
        Self {
            secret: secret.into(),
            timeout,
            endpoint: "https://api.hcaptcha.com/siteverify".to_string(),
        }
    }

    /// Overrides the verification endpoint URL.
    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = endpoint.into();
        self
    }
}

#[async_trait]
impl CaptchaVerifier for HCaptchaVerifier {
    async fn verify(
        &self,
        token: &str,
        remote_ip: Option<&str>,
    ) -> Result<CaptchaOutcome, CaptchaError> {
        let res =
            post_siteverify(&self.endpoint, &self.secret, token, remote_ip, self.timeout).await?;
        Ok(CaptchaOutcome {
            success: res.success,
            score: res.score,
            action: res.action,
            error_codes: res.error_codes,
        })
    }
}

/// Google reCAPTCHA v2 verifier.
pub struct RecaptchaV2Verifier {
    secret: String,
    timeout: Duration,
    endpoint: String,
}

impl RecaptchaV2Verifier {
    /// Creates a new [`RecaptchaV2Verifier`].
    pub fn new(secret: impl Into<String>, timeout: Duration) -> Self {
        Self {
            secret: secret.into(),
            timeout,
            endpoint: "https://www.google.com/recaptcha/api/siteverify".to_string(),
        }
    }

    /// Overrides the verification endpoint URL.
    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = endpoint.into();
        self
    }
}

#[async_trait]
impl CaptchaVerifier for RecaptchaV2Verifier {
    async fn verify(
        &self,
        token: &str,
        remote_ip: Option<&str>,
    ) -> Result<CaptchaOutcome, CaptchaError> {
        let res =
            post_siteverify(&self.endpoint, &self.secret, token, remote_ip, self.timeout).await?;
        Ok(CaptchaOutcome {
            success: res.success,
            score: res.score,
            action: res.action,
            error_codes: res.error_codes,
        })
    }
}

/// Google reCAPTCHA v3 verifier enforcing a minimum score threshold.
pub struct RecaptchaV3Verifier {
    secret: String,
    min_score: f64,
    timeout: Duration,
    endpoint: String,
}

impl RecaptchaV3Verifier {
    /// Creates a new [`RecaptchaV3Verifier`].
    pub fn new(secret: impl Into<String>, min_score: f64, timeout: Duration) -> Self {
        Self {
            secret: secret.into(),
            min_score,
            timeout,
            endpoint: "https://www.google.com/recaptcha/api/siteverify".to_string(),
        }
    }

    /// Overrides the verification endpoint URL.
    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = endpoint.into();
        self
    }
}

#[async_trait]
impl CaptchaVerifier for RecaptchaV3Verifier {
    async fn verify(
        &self,
        token: &str,
        remote_ip: Option<&str>,
    ) -> Result<CaptchaOutcome, CaptchaError> {
        let res =
            post_siteverify(&self.endpoint, &self.secret, token, remote_ip, self.timeout).await?;
        let score_ok = res.score.is_some_and(|s| s >= self.min_score);
        Ok(CaptchaOutcome {
            success: res.success && score_ok,
            score: res.score,
            action: res.action,
            error_codes: res.error_codes,
        })
    }
}

/// Test verifier that always succeeds.
#[derive(Debug, Default, Clone, Copy)]
pub struct AlwaysPass;

#[async_trait]
impl CaptchaVerifier for AlwaysPass {
    async fn verify(
        &self,
        _token: &str,
        _remote_ip: Option<&str>,
    ) -> Result<CaptchaOutcome, CaptchaError> {
        Ok(CaptchaOutcome {
            success: true,
            score: Some(1.0),
            action: None,
            error_codes: Vec::new(),
        })
    }
}

/// Test verifier that always fails.
#[derive(Debug, Default, Clone, Copy)]
pub struct AlwaysFail;

#[async_trait]
impl CaptchaVerifier for AlwaysFail {
    async fn verify(
        &self,
        _token: &str,
        _remote_ip: Option<&str>,
    ) -> Result<CaptchaOutcome, CaptchaError> {
        Ok(CaptchaOutcome {
            success: false,
            score: Some(0.0),
            action: None,
            error_codes: vec!["always-fail".to_string()],
        })
    }
}

/// HTML form field rendering the provider widget script tag and hidden response input.
#[derive(Debug, Clone)]
pub struct CaptchaField {
    /// CAPTCHA provider.
    pub provider: CaptchaProvider,
    /// Public site key.
    pub site_key: String,
    /// Hidden input name submitted with the form.
    pub field_name: String,
}

impl CaptchaField {
    /// Creates a [`CaptchaField`] with the provider's standard response field name.
    pub fn new(provider: CaptchaProvider, site_key: impl Into<String>) -> Self {
        let field_name = match provider {
            CaptchaProvider::Turnstile => "cf-turnstile-response",
            CaptchaProvider::HCaptcha => "h-captcha-response",
            CaptchaProvider::RecaptchaV2 | CaptchaProvider::RecaptchaV3 { .. } => {
                "g-recaptcha-response"
            }
        }
        .to_string();
        Self {
            provider,
            site_key: site_key.into(),
            field_name,
        }
    }

    /// Renders the provider script tag, widget container, and hidden input element.
    pub fn render(&self) -> String {
        let escaped_key = self
            .site_key
            .replace('&', "&amp;")
            .replace('"', "&quot;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        let (script_src, widget_class) = match self.provider {
            CaptchaProvider::Turnstile => (
                "https://challenges.cloudflare.com/turnstile/v0/api.js",
                "cf-turnstile",
            ),
            CaptchaProvider::HCaptcha => ("https://js.hcaptcha.com/1/api.js", "h-captcha"),
            CaptchaProvider::RecaptchaV2 | CaptchaProvider::RecaptchaV3 { .. } => {
                ("https://www.google.com/recaptcha/api.js", "g-recaptcha")
            }
        };
        format!(
            "<script src=\"{script_src}\" async defer></script>\
             <div class=\"{widget_class}\" data-sitekey=\"{escaped_key}\"></div>\
             <input type=\"hidden\" name=\"{}\" value=\"\">",
            self.field_name
        )
    }
}
