use super::*;
use base64::engine::general_purpose::STANDARD;
use chrono::Utc;
use djangors_cache::InMemoryCache;

const RSA_CERT_PEM: &str = include_str!("../tests/fixtures/idp_rsa_cert.pem");
const EC_CERT_PEM: &str = include_str!("../tests/fixtures/idp_ec_cert.pem");

const RSA_SHA256_XML: &str = include_str!("../tests/fixtures/rsa_sha256_signed_assertion.xml");
const RSA_SHA512_XML: &str = include_str!("../tests/fixtures/rsa_sha512_signed_assertion.xml");
const ECDSA_P256_XML: &str = include_str!("../tests/fixtures/ecdsa_p256_signed_assertion.xml");
const RSA_RESPONSE_ONLY_XML: &str = include_str!("../tests/fixtures/rsa_signed_response_only.xml");
const RSA_RESPONSE_AND_ASSERTION_XML: &str =
    include_str!("../tests/fixtures/rsa_signed_response_and_assertion.xml");
const ROGUE_KEY_XML: &str = include_str!("../tests/fixtures/rogue_key_signed_assertion.xml");
const COMMENT_INJECTION_XML: &str = include_str!("../tests/fixtures/comment_injection_nameid.xml");

fn fixture_idp(cert_pem: &str) -> SamlIdentityProvider {
    SamlIdentityProvider {
        id: 1,
        slug: "okta".to_string(),
        name: "Fixture Okta".to_string(),
        entity_id: "https://idp.example.com".to_string(),
        sso_url: "https://idp.example.com/sso".to_string(),
        x509_cert_pem: cert_pem.to_string(),
        metadata_xml: String::new(),
        attribute_map_json: "{}".to_string(),
        jit_create_users: true,
        is_active: true,
        created_at: Utc::now(),
    }
}

fn fixture_sp() -> SamlSpConfig {
    SamlSpConfig::new("https://app.example.com/sp", "https://app.example.com")
}

fn assert_alice_claims(claims: &SamlAssertionClaims) {
    assert_eq!(claims.name_id, "alice@example.com");
    assert_eq!(
        claims.attributes.get("uid").map(Vec::as_slice),
        Some(&["fixture-alice".to_string()][..])
    );
    assert_eq!(
        claims.attributes.get("mail").map(Vec::as_slice),
        Some(&["alice@example.com".to_string()][..])
    );
    assert_eq!(claims.username.as_deref(), Some("fixture-alice"));
    assert_eq!(claims.email.as_deref(), Some("alice@example.com"));
    assert_eq!(claims.in_response_to.as_deref(), Some("_req_fixture_1"));
}

#[tokio::test]
async fn verifies_rsa_sha256_signed_assertion_fixture() {
    let cache = InMemoryCache::new(1000);
    let claims = verify_and_extract_saml_response(
        &STANDARD.encode(RSA_SHA256_XML),
        &fixture_idp(RSA_CERT_PEM),
        &fixture_sp(),
        Some("_req_fixture_1"),
        &cache,
    )
    .await
    .unwrap();
    assert_alice_claims(&claims);
}

#[tokio::test]
async fn verifies_rsa_sha512_signed_assertion_fixture() {
    let cache = InMemoryCache::new(1000);
    let claims = verify_and_extract_saml_response(
        &STANDARD.encode(RSA_SHA512_XML),
        &fixture_idp(RSA_CERT_PEM),
        &fixture_sp(),
        Some("_req_fixture_1"),
        &cache,
    )
    .await
    .unwrap();
    assert_alice_claims(&claims);
}

#[tokio::test]
async fn verifies_ecdsa_p256_signed_assertion_fixture() {
    let cache = InMemoryCache::new(1000);
    let claims = verify_and_extract_saml_response(
        &STANDARD.encode(ECDSA_P256_XML),
        &fixture_idp(EC_CERT_PEM),
        &fixture_sp(),
        Some("_req_fixture_1"),
        &cache,
    )
    .await
    .unwrap();
    assert_alice_claims(&claims);
}

#[tokio::test]
async fn verifies_rsa_signed_response_only_fixture() {
    let cache = InMemoryCache::new(1000);
    let claims = verify_and_extract_saml_response(
        &STANDARD.encode(RSA_RESPONSE_ONLY_XML),
        &fixture_idp(RSA_CERT_PEM),
        &fixture_sp(),
        Some("_req_fixture_1"),
        &cache,
    )
    .await
    .unwrap();
    assert_alice_claims(&claims);
}

#[tokio::test]
async fn verifies_rsa_signed_response_and_assertion_fixture() {
    let cache = InMemoryCache::new(1000);
    let claims = verify_and_extract_saml_response(
        &STANDARD.encode(RSA_RESPONSE_AND_ASSERTION_XML),
        &fixture_idp(RSA_CERT_PEM),
        &fixture_sp(),
        Some("_req_fixture_1"),
        &cache,
    )
    .await
    .unwrap();
    assert_alice_claims(&claims);
}

#[tokio::test]
async fn rejects_wrong_key_type_for_signature() {
    let sp = fixture_sp();

    // ECDSA fixture verified against RSA cert -> Err.
    let cache1 = InMemoryCache::new(1000);
    let err1 = verify_and_extract_saml_response(
        &STANDARD.encode(ECDSA_P256_XML),
        &fixture_idp(RSA_CERT_PEM),
        &sp,
        Some("_req_fixture_1"),
        &cache1,
    )
    .await
    .unwrap_err();
    assert!(matches!(err1, SamlError::SignatureVerification(_)));

    // RSA fixture verified against EC cert -> Err.
    let cache2 = InMemoryCache::new(1000);
    let err2 = verify_and_extract_saml_response(
        &STANDARD.encode(RSA_SHA256_XML),
        &fixture_idp(EC_CERT_PEM),
        &sp,
        Some("_req_fixture_1"),
        &cache2,
    )
    .await
    .unwrap_err();
    assert!(matches!(err2, SamlError::SignatureVerification(_)));
}

#[tokio::test]
async fn rejects_rogue_key_ignoring_embedded_keyinfo() {
    let cache = InMemoryCache::new(1000);
    let err = verify_and_extract_saml_response(
        &STANDARD.encode(ROGUE_KEY_XML),
        &fixture_idp(RSA_CERT_PEM),
        &fixture_sp(),
        Some("_req_fixture_1"),
        &cache,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, SamlError::SignatureVerification(_)));
}

#[tokio::test]
async fn comment_injection_nameid_never_truncates_at_comment() {
    let cache = InMemoryCache::new(1000);
    let res = verify_and_extract_saml_response(
        &STANDARD.encode(COMMENT_INJECTION_XML),
        &fixture_idp(RSA_CERT_PEM),
        &fixture_sp(),
        Some("_req_fixture_1"),
        &cache,
    )
    .await;
    // The comment is dropped by c14n, so the signature is valid; the SP must read the
    // full text, never just the part before the comment.
    let claims = res.expect("comment-split NameID has a valid signature");
    assert_eq!(claims.name_id, "admin@example.com.evil.test");
}

#[tokio::test]
async fn rejects_tampered_signed_assertion_and_response() {
    let sp = fixture_sp();
    let idp = fixture_idp(RSA_CERT_PEM);

    let tampered_assertion = RSA_SHA256_XML.replacen("fixture-alice", "fixture-admin", 1);
    assert_ne!(tampered_assertion, RSA_SHA256_XML);
    let cache1 = InMemoryCache::new(1000);
    let err1 = verify_and_extract_saml_response(
        &STANDARD.encode(&tampered_assertion),
        &idp,
        &sp,
        Some("_req_fixture_1"),
        &cache1,
    )
    .await
    .unwrap_err();
    assert!(matches!(err1, SamlError::SignatureVerification(_)));

    let tampered_response = RSA_RESPONSE_ONLY_XML.replacen(
        "https://app.example.com/sp",
        "https://evil.example.com/sp",
        1,
    );
    assert_ne!(tampered_response, RSA_RESPONSE_ONLY_XML);
    let cache2 = InMemoryCache::new(1000);
    let err2 = verify_and_extract_saml_response(
        &STANDARD.encode(&tampered_response),
        &idp,
        &sp,
        Some("_req_fixture_1"),
        &cache2,
    )
    .await
    .unwrap_err();
    assert!(matches!(err2, SamlError::SignatureVerification(_)));
}

#[tokio::test]
async fn rejects_replayed_assertion_with_same_cache() {
    let cache = InMemoryCache::new(1000);
    let idp = fixture_idp(RSA_CERT_PEM);
    let sp = fixture_sp();
    let b64 = STANDARD.encode(RSA_SHA256_XML);

    let claims = verify_and_extract_saml_response(&b64, &idp, &sp, Some("_req_fixture_1"), &cache)
        .await
        .unwrap();
    assert_alice_claims(&claims);

    let err = verify_and_extract_saml_response(&b64, &idp, &sp, Some("_req_fixture_1"), &cache)
        .await
        .unwrap_err();
    assert!(matches!(err, SamlError::ReplayDetected(_)));
}

#[tokio::test]
async fn rejects_wrong_in_response_to() {
    let cache = InMemoryCache::new(1000);
    let err = verify_and_extract_saml_response(
        &STANDARD.encode(RSA_SHA256_XML),
        &fixture_idp(RSA_CERT_PEM),
        &fixture_sp(),
        Some("_other"),
        &cache,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, SamlError::InResponseToMismatch(_)));
}

#[tokio::test]
async fn verifies_base64_wrapped_at_76_chars() {
    let cache = InMemoryCache::new(1000);
    let raw_b64 = STANDARD.encode(RSA_SHA256_XML);
    let wrapped_b64 = raw_b64
        .as_bytes()
        .chunks(76)
        .map(|chunk| std::str::from_utf8(chunk).unwrap())
        .collect::<Vec<_>>()
        .join("\r\n");

    let claims = verify_and_extract_saml_response(
        &wrapped_b64,
        &fixture_idp(RSA_CERT_PEM),
        &fixture_sp(),
        Some("_req_fixture_1"),
        &cache,
    )
    .await
    .unwrap();
    assert_alice_claims(&claims);
}
