use super::*;
use serde::{Deserialize, Serialize};

const KEY: &str = "unit-test-secret-key-0123456789";

#[test]
fn signer_roundtrip_returns_original_value() {
    let signer = Signer::new(KEY, "test.salt").unwrap();
    let signed = signer.sign("hello:world");
    assert_eq!(signer.unsign(&signed).unwrap(), "hello:world");
}

#[test]
fn signer_rejects_tampered_value() {
    let signer = Signer::new(KEY, "test.salt").unwrap();
    let signed = signer.sign("alice");
    let forged = signed.replacen("alice", "mallory", 1);
    assert_eq!(signer.unsign(&forged), Err(SigningError::BadSignature));
}

#[test]
fn signer_salt_is_domain_separation() {
    let a = Signer::new(KEY, "purpose.one").unwrap();
    let b = Signer::new(KEY, "purpose.two").unwrap();
    let signed = a.sign("value");
    assert_eq!(b.unsign(&signed), Err(SigningError::BadSignature));
}

#[test]
fn signer_rejects_missing_separator() {
    let signer = Signer::new(KEY, "test.salt").unwrap();
    assert_eq!(signer.unsign("no-separator"), Err(SigningError::Malformed));
}

#[test]
fn empty_key_fails_closed() {
    assert_eq!(
        Signer::new("", "salt").unwrap_err(),
        SigningError::MissingKey
    );
    let ok = Signer::new(KEY, "salt").unwrap();
    assert_eq!(
        ok.with_fallbacks(vec![""]).unwrap_err(),
        SigningError::MissingKey
    );
}

#[test]
fn fallback_key_verifies_but_primary_signs() {
    let old = Signer::new("old-secret-key-aaaaaaaaaaaa", "rot").unwrap();
    let signed_by_old = old.sign("payload");

    let rotated = Signer::new("new-secret-key-bbbbbbbbbbbb", "rot")
        .unwrap()
        .with_fallbacks(vec!["old-secret-key-aaaaaaaaaaaa"])
        .unwrap();
    assert_eq!(rotated.unsign(&signed_by_old).unwrap(), "payload");

    let signed_by_new = rotated.sign("payload");
    assert_eq!(old.unsign(&signed_by_new), Err(SigningError::BadSignature));
}

#[test]
fn from_settings_uses_secret_and_fallbacks() {
    let mut settings = DjangorsSettings {
        secret_key: "current-secret-key-cccccccccc".to_string(),
        secret_key_fallbacks: vec!["previous-secret-key-dddddddddd".to_string()],
        ..Default::default()
    };
    let signer = Signer::from_settings(&settings, "s").unwrap();
    assert_eq!(signer.unsign(&signer.sign("x")).unwrap(), "x");

    settings.secret_key = String::new();
    assert_eq!(
        Signer::from_settings(&settings, "s").unwrap_err(),
        SigningError::MissingKey
    );
}

#[test]
fn timestamp_signer_accepts_fresh_and_rejects_expired() {
    let ts = TimestampSigner::new(KEY, "ts.salt", Duration::from_secs(60)).unwrap();
    let signed = ts.sign_at("user-9", 1_000);
    assert_eq!(ts.unsign_at(&signed, 1_030).unwrap(), "user-9");
    assert_eq!(ts.unsign_at(&signed, 1_061), Err(SigningError::Expired));
}

#[test]
fn timestamp_signer_rejects_far_future_timestamp() {
    let ts = TimestampSigner::new(KEY, "ts.salt", Duration::from_secs(60)).unwrap();
    let signed = ts.sign_at("user-9", 10_000);
    assert_eq!(ts.unsign_at(&signed, 1_000), Err(SigningError::Malformed));
}

#[test]
fn timestamp_signer_roundtrip_through_trait() {
    let ts = TimestampSigner::new(KEY, "ts.salt", Duration::from_secs(3600)).unwrap();
    let signed = ts.sign_value("abc");
    assert_eq!(ts.unsign_value(&signed).unwrap(), "abc");
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Payload {
    user_id: i64,
    scopes: Vec<String>,
}

#[test]
fn dumps_loads_roundtrip_plain_and_compressed() {
    let signer = Signer::new(KEY, "dumps.salt").unwrap();
    let value = Payload {
        user_id: 42,
        scopes: vec!["read".to_string(); 50],
    };

    let plain = dumps(&value, &signer, false).unwrap();
    let back: Payload = loads(&plain, &signer).unwrap();
    assert_eq!(back, value);

    let packed = dumps(&value, &signer, true).unwrap();
    let back: Payload = loads(&packed, &signer).unwrap();
    assert_eq!(back, value);
}

#[test]
fn loads_rejects_token_from_other_salt() {
    let a = Signer::new(KEY, "a").unwrap();
    let b = Signer::new(KEY, "b").unwrap();
    let token = dumps(
        &Payload {
            user_id: 1,
            scopes: vec![],
        },
        &a,
        false,
    )
    .unwrap();
    let result: Result<Payload, _> = loads(&token, &b);
    assert_eq!(result.unwrap_err(), SigningError::BadSignature);
}

#[test]
fn sign_url_roundtrip_and_query_order_insensitive() {
    let signer = Signer::new(KEY, SIGNED_URL_SALT).unwrap();
    let url = sign_url(&signer, "GET", "/files/report?b=2&a=1", 2_000).unwrap();
    let (path, query) = url.split_once('?').unwrap();
    assert_eq!(path, "/files/report");
    assert!(query.contains("&exp=2000&sig="));

    assert!(verify_url(&signer, "GET", path, Some(query), 1_999).is_ok());

    // Same pairs in a different order still verify, because the signature sorts them.
    let sig = query.rsplit_once("sig=").unwrap().1;
    let reordered = format!("a=1&exp=2000&b=2&sig={sig}");
    assert!(verify_url(&signer, "GET", path, Some(&reordered), 1_999).is_ok());
}

#[test]
fn signed_url_rejects_expired_tampered_and_wrong_method() {
    let signer = Signer::new(KEY, SIGNED_URL_SALT).unwrap();
    let url = sign_url(&signer, "GET", "/files/report?a=1", 2_000).unwrap();
    let (path, query) = url.split_once('?').unwrap();

    assert_eq!(
        verify_url(&signer, "GET", path, Some(query), 2_000),
        Err(SigningError::Expired)
    );

    let tampered = query.replacen("a=1", "a=2", 1);
    assert_eq!(
        verify_url(&signer, "GET", path, Some(&tampered), 1_000),
        Err(SigningError::BadSignature)
    );

    assert_eq!(
        verify_url(&signer, "POST", path, Some(query), 1_000),
        Err(SigningError::BadSignature)
    );

    assert_eq!(
        verify_url(&signer, "GET", "/other", Some(query), 1_000),
        Err(SigningError::BadSignature)
    );
}

#[test]
fn signed_url_requires_exp_and_sig_and_rejects_duplicates() {
    let signer = Signer::new(KEY, SIGNED_URL_SALT).unwrap();
    assert_eq!(
        verify_url(&signer, "GET", "/x", Some("a=1"), 1),
        Err(SigningError::Malformed)
    );
    assert_eq!(
        verify_url(&signer, "GET", "/x", Some("exp=5&exp=6&sig=AA"), 1),
        Err(SigningError::Malformed)
    );
}

#[test]
fn sign_url_replaces_existing_exp_and_sig() {
    let signer = Signer::new(KEY, SIGNED_URL_SALT).unwrap();
    let first = sign_url(&signer, "GET", "/x?exp=1&sig=zzz&k=v", 500).unwrap();
    assert_eq!(first.matches("exp=").count(), 1);
    assert_eq!(first.matches("sig=").count(), 1);
    let (path, query) = first.split_once('?').unwrap();
    assert!(verify_url(&signer, "GET", path, Some(query), 100).is_ok());
}

#[test]
fn sign_url_for_uses_future_expiry() {
    let signer = Signer::new(KEY, SIGNED_URL_SALT).unwrap();
    let url = sign_url_for(&signer, "GET", "/x", Duration::from_secs(600)).unwrap();
    let (_, query) = url.split_once('?').unwrap();
    let exp: u64 = query
        .split('&')
        .find_map(|p| p.strip_prefix("exp="))
        .unwrap()
        .parse()
        .unwrap();
    assert!(exp > now_secs().unwrap());
}

#[test]
fn timestamp_signer_supports_fallbacks_and_inherent_methods() {
    let old =
        TimestampSigner::new("old-ts-key-1111111111111111", "ts", Duration::from_secs(60)).unwrap();
    let signed = old.sign("hello");

    let rotated =
        TimestampSigner::new("new-ts-key-2222222222222222", "ts", Duration::from_secs(60))
            .unwrap()
            .with_fallbacks(["old-ts-key-1111111111111111"])
            .unwrap();
    assert_eq!(rotated.unsign(&signed).unwrap(), "hello");
}

#[tokio::test]
async fn signed_url_extractor_accepts_valid_and_fallback_and_rejects_bad() {
    use crate::state::AppState;
    use bytes::Bytes;
    use hyper::{HeaderMap, Method, StatusCode, Uri};

    let settings = DjangorsSettings::new()
        .with_secret_key("new-url-secret-key-0123456789")
        .with_secret_key_fallbacks(["old-url-secret-key-0123456789"]);

    // Signed with fallback key should still be accepted.
    let old_signer = Signer::new("old-url-secret-key-0123456789", SIGNED_URL_SALT).unwrap();
    let valid_url = sign_url_for(
        &old_signer,
        "GET",
        "/downloads/report.pdf?version=2",
        Duration::from_secs(300),
    )
    .unwrap();
    let req = Request::new(
        Method::GET,
        valid_url.parse::<Uri>().unwrap(),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_state(AppState::new().insert(settings.clone()));
    let extracted = SignedUrl::from_request(&req).await.unwrap();
    assert!(extracted.expires_at > now_secs().unwrap());

    // Tampered query parameter returns 403 Forbidden.
    let tampered = valid_url.replacen("version=2", "version=3", 1);
    let req_tampered = Request::new(
        Method::GET,
        tampered.parse::<Uri>().unwrap(),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_state(AppState::new().insert(settings.clone()));
    let err = SignedUrl::from_request(&req_tampered).await.unwrap_err();
    assert_eq!(err.status_code(), StatusCode::FORBIDDEN);

    // Expired URL returns 403 Forbidden.
    let expired_url = sign_url(&old_signer, "GET", "/downloads/report.pdf", 1).unwrap();
    let req_expired = Request::new(
        Method::GET,
        expired_url.parse::<Uri>().unwrap(),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_state(AppState::new().insert(settings));
    let err = SignedUrl::from_request(&req_expired).await.unwrap_err();
    assert_eq!(err.status_code(), StatusCode::FORBIDDEN);

    // Empty secret_key fails closed with 500 Internal.
    let empty_settings = DjangorsSettings::new();
    let req_empty = Request::new(
        Method::GET,
        valid_url.parse::<Uri>().unwrap(),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_state(AppState::new().insert(empty_settings));
    let err = SignedUrl::from_request(&req_empty).await.unwrap_err();
    assert_eq!(err.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
}
