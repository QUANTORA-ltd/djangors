//! [`VerifiedCaptcha`] extractor for JSON APIs.
//!
//! The verifier backends live in `djangors_forms::captcha`; this module only adds the
//! request extractor, because `djangors-forms` sits below `djangors-core` in the crate graph.

use std::sync::Arc;

use djangors_forms::captcha::{CaptchaConfig, CaptchaOutcome, CaptchaVerifier};
use hyper::StatusCode;

use crate::error::DjangorsError;
use crate::extract::FromRequest;
use crate::request::Request;

/// Extractor that verifies the request's CAPTCHA token with the
/// `Arc<dyn CaptchaVerifier>` (or [`CaptchaConfig`]) registered in [`crate::AppState`].
///
/// The token is read from the `X-Captcha-Token` header, falling back to a
/// `captcha_token` (or `token`) field in a JSON body. Verification failures,
/// including network errors talking to the provider, fail closed with `403`.
#[derive(Debug, Clone)]
pub struct VerifiedCaptcha(pub CaptchaOutcome);

#[async_trait::async_trait]
impl FromRequest for VerifiedCaptcha {
    async fn from_request(req: &Request) -> Result<Self, DjangorsError> {
        let verifier: Arc<dyn CaptchaVerifier> =
            if let Some(v) = req.state::<Arc<dyn CaptchaVerifier>>() {
                Arc::clone(v)
            } else if let Some(cfg) = req.state::<CaptchaConfig>() {
                cfg.build_verifier()
            } else {
                return Err(DjangorsError::Internal(
                    "CAPTCHA verifier not configured in AppState".to_string(),
                ));
            };

        let header_token = req
            .header("x-captcha-token")
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_string);
        let token = match header_token {
            Some(t) => t,
            None => {
                let bytes = req.body_bytes().await;
                serde_json::from_slice::<serde_json::Value>(bytes)
                    .ok()
                    .and_then(|val| {
                        val.get("captcha_token")
                            .or_else(|| val.get("token"))
                            .and_then(|v| v.as_str())
                            .map(str::to_string)
                    })
                    .ok_or_else(|| {
                        DjangorsError::api(
                            StatusCode::BAD_REQUEST,
                            "missing_captcha",
                            "Missing CAPTCHA token",
                        )
                    })?
            }
        };

        let remote_ip = req
            .header("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.split(',').next())
            .map(str::trim)
            .filter(|s| !s.is_empty());

        let outcome = verifier.verify(&token, remote_ip).await.map_err(|err| {
            // Provider/network details go to the log, never to the client.
            tracing::warn!(error = %err, "captcha verification error");
            DjangorsError::api(
                StatusCode::FORBIDDEN,
                "captcha_error",
                "CAPTCHA verification failed",
            )
        })?;

        if !outcome.success {
            return Err(DjangorsError::api(
                StatusCode::FORBIDDEN,
                "captcha_rejected",
                "CAPTCHA challenge failed",
            ));
        }

        Ok(VerifiedCaptcha(outcome))
    }
}

#[cfg(test)]
#[path = "captcha_tests.rs"]
mod tests;
