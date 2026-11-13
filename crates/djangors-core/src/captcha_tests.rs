use std::sync::Arc;

use djangors_forms::captcha::{AlwaysFail, AlwaysPass, CaptchaVerifier};

use super::VerifiedCaptcha;
use crate::extract::FromRequest;
use crate::AppState;

fn request_with(verifier: Arc<dyn CaptchaVerifier>, token: &str) -> crate::Request {
    let mut headers = hyper::HeaderMap::new();
    headers.insert("x-captcha-token", token.parse().unwrap());
    crate::Request::new(
        hyper::Method::POST,
        "/submit".parse().unwrap(),
        headers,
        Default::default(),
    )
    .with_state(AppState::new().insert(verifier))
}

#[tokio::test]
async fn verified_captcha_accepts_passing_token() {
    let req = request_with(Arc::new(AlwaysPass), "ok-token");
    let VerifiedCaptcha(outcome) = VerifiedCaptcha::from_request(&req).await.unwrap();
    assert!(outcome.success);
}

#[tokio::test]
async fn verified_captcha_rejects_failing_token() {
    let req = request_with(Arc::new(AlwaysFail), "bad-token");
    assert!(VerifiedCaptcha::from_request(&req).await.is_err());
}
