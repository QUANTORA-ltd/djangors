//! Passkey tests that build real WebAuthn responses signed with real keys.

use super::cbor::Value;
use super::*;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

const ORIGIN: &str = "https://example.com";

fn rp() -> RelyingParty {
    RelyingParty::new("example.com", "Example", ORIGIN)
}

// ---- A minimal CBOR encoder, only for building test inputs ----

fn head(major: u8, n: u64) -> Vec<u8> {
    let m = major << 5;
    match n {
        0..=23 => vec![m | n as u8],
        24..=0xff => vec![m | 24, n as u8],
        0x100..=0xffff => {
            let mut v = vec![m | 25];
            v.extend_from_slice(&(n as u16).to_be_bytes());
            v
        }
        _ => {
            let mut v = vec![m | 26];
            v.extend_from_slice(&(n as u32).to_be_bytes());
            v
        }
    }
}

fn enc(value: &Value) -> Vec<u8> {
    match value {
        Value::Int(i) if *i >= 0 => head(0, *i as u64),
        Value::Int(i) => head(1, (-1 - *i) as u64),
        Value::Bytes(b) => [head(2, b.len() as u64), b.clone()].concat(),
        Value::Text(t) => [head(3, t.len() as u64), t.as_bytes().to_vec()].concat(),
        Value::Array(items) => {
            let mut out = head(4, items.len() as u64);
            for item in items {
                out.extend(enc(item));
            }
            out
        }
        Value::Map(entries) => {
            let mut out = head(5, entries.len() as u64);
            for (k, v) in entries {
                out.extend(enc(k));
                out.extend(enc(v));
            }
            out
        }
        Value::Bool(false) => vec![0xf4],
        Value::Bool(true) => vec![0xf5],
        Value::Null => vec![0xf6],
    }
}

fn int(i: i128) -> Value {
    Value::Int(i)
}

// ---- Test authenticators ----

enum TestKey {
    Es256(p256::ecdsa::SigningKey),
    Ed25519(ed25519_dalek::SigningKey),
}

impl TestKey {
    fn es256() -> Self {
        TestKey::Es256(p256::ecdsa::SigningKey::from_slice(&[7u8; 32]).unwrap())
    }

    fn ed25519() -> Self {
        TestKey::Ed25519(ed25519_dalek::SigningKey::from_bytes(&[9u8; 32]))
    }

    fn cose(&self) -> Vec<u8> {
        match self {
            TestKey::Es256(sk) => {
                let point = sk.verifying_key().to_encoded_point(false);
                enc(&Value::Map(vec![
                    (int(1), int(2)),
                    (int(3), int(-7)),
                    (int(-1), int(1)),
                    (int(-2), Value::Bytes(point.x().unwrap().to_vec())),
                    (int(-3), Value::Bytes(point.y().unwrap().to_vec())),
                ]))
            }
            TestKey::Ed25519(sk) => enc(&Value::Map(vec![
                (int(1), int(1)),
                (int(3), int(-8)),
                (int(-1), int(6)),
                (
                    int(-2),
                    Value::Bytes(sk.verifying_key().to_bytes().to_vec()),
                ),
            ])),
        }
    }

    fn sign(&self, message: &[u8]) -> Vec<u8> {
        match self {
            TestKey::Es256(sk) => {
                use p256::ecdsa::signature::Signer;
                let sig: p256::ecdsa::Signature = sk.sign(message);
                sig.to_der().as_bytes().to_vec()
            }
            TestKey::Ed25519(sk) => {
                use ed25519_dalek::Signer;
                sk.sign(message).to_bytes().to_vec()
            }
        }
    }
}

const CRED_ID: &[u8] = b"credential-0001";
const UP_UV: u8 = FLAG_USER_PRESENT | FLAG_USER_VERIFIED;

fn auth_data(rp_id: &str, flags: u8, count: u32, attested: Option<(&[u8], Vec<u8>)>) -> Vec<u8> {
    let mut data = Sha256::digest(rp_id.as_bytes()).to_vec();
    data.push(flags);
    data.extend_from_slice(&count.to_be_bytes());
    if let Some((cred_id, cose)) = attested {
        data.extend_from_slice(&[0u8; 16]);
        data.extend_from_slice(&(cred_id.len() as u16).to_be_bytes());
        data.extend_from_slice(cred_id);
        data.extend(cose);
    }
    data
}

fn client_data(kind: &str, challenge: &str, origin: &str) -> Vec<u8> {
    serde_json::json!({ "type": kind, "challenge": challenge, "origin": origin })
        .to_string()
        .into_bytes()
}

fn b64(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

fn registration(
    state: &RegistrationState,
    key: &TestKey,
    flags: u8,
    origin: &str,
    rp_id: &str,
) -> RegistrationResponse {
    let data = auth_data(
        rp_id,
        flags | FLAG_ATTESTED_DATA,
        0,
        Some((CRED_ID, key.cose())),
    );
    let attestation = enc(&Value::Map(vec![
        (Value::Text("fmt".into()), Value::Text("none".into())),
        (Value::Text("attStmt".into()), Value::Map(vec![])),
        (Value::Text("authData".into()), Value::Bytes(data)),
    ]));
    RegistrationResponse {
        raw_id: b64(CRED_ID),
        kind: "public-key".into(),
        response: AttestationResponse {
            client_data_json: b64(&client_data("webauthn.create", &state.challenge, origin)),
            attestation_object: b64(&attestation),
        },
    }
}

fn register(key: &TestKey) -> Passkey {
    let (_, state) = start_registration(&rp(), &user_handle_for(42), "ada", "Ada", &[]);
    finish_registration(
        &rp(),
        &state,
        &registration(&state, key, UP_UV, ORIGIN, "example.com"),
    )
    .expect("registration succeeds")
}

fn assertion(
    state: &AuthenticationState,
    key: &TestKey,
    flags: u8,
    count: u32,
    origin: &str,
) -> AuthenticationResponse {
    let data = auth_data("example.com", flags, count, None);
    let client = client_data("webauthn.get", &state.challenge, origin);
    let mut signed = data.clone();
    signed.extend_from_slice(&Sha256::digest(&client));
    AuthenticationResponse {
        raw_id: b64(CRED_ID),
        kind: "public-key".into(),
        response: AssertionResponse {
            client_data_json: b64(&client),
            authenticator_data: b64(&data),
            signature: b64(&key.sign(&signed)),
            user_handle: Some(b64(&user_handle_for(42))),
        },
    }
}

// ---- Registration ----

#[test]
fn registration_options_request_user_verification_and_supported_algorithms() {
    let (options, state) = start_registration(&rp(), &user_handle_for(42), "ada", "Ada", &[]);
    let pk = &options["publicKey"];
    assert_eq!(pk["rp"]["id"], "example.com");
    assert_eq!(pk["challenge"], state.challenge.as_str());
    assert_eq!(pk["authenticatorSelection"]["userVerification"], "required");
    assert_eq!(pk["pubKeyCredParams"][0]["alg"], -7);
    assert_eq!(pk["pubKeyCredParams"][1]["alg"], -8);
    assert_eq!(URL_SAFE_NO_PAD.decode(&state.challenge).unwrap().len(), 32);
    let (_, other) = start_registration(&rp(), &user_handle_for(42), "ada", "Ada", &[]);
    assert_ne!(
        state.challenge, other.challenge,
        "challenges must be random"
    );
}

#[test]
fn es256_and_ed25519_passkeys_register() {
    for key in [TestKey::es256(), TestKey::ed25519()] {
        let passkey = register(&key);
        assert_eq!(passkey.credential_id, CRED_ID);
        assert_eq!(passkey.user_handle, user_handle_for(42));
        assert_eq!(passkey.sign_count, 0);
    }
}

#[test]
fn registration_rejects_wrong_challenge_origin_rp_and_flags() {
    let key = TestKey::es256();
    let (_, state) = start_registration(&rp(), &user_handle_for(42), "ada", "Ada", &[]);
    let (_, other_state) = start_registration(&rp(), &user_handle_for(42), "ada", "Ada", &[]);

    let response = registration(&other_state, &key, UP_UV, ORIGIN, "example.com");
    assert!(matches!(
        finish_registration(&rp(), &state, &response),
        Err(PasskeyError::ChallengeMismatch)
    ));

    let response = registration(&state, &key, UP_UV, "https://evil.example", "example.com");
    assert!(matches!(
        finish_registration(&rp(), &state, &response),
        Err(PasskeyError::OriginMismatch(_))
    ));

    let response = registration(&state, &key, UP_UV, ORIGIN, "evil.example");
    assert!(matches!(
        finish_registration(&rp(), &state, &response),
        Err(PasskeyError::RpIdMismatch)
    ));

    let response = registration(&state, &key, FLAG_USER_PRESENT, ORIGIN, "example.com");
    assert!(matches!(
        finish_registration(&rp(), &state, &response),
        Err(PasskeyError::UserNotVerified)
    ));

    let response = registration(&state, &key, FLAG_USER_VERIFIED, ORIGIN, "example.com");
    assert!(matches!(
        finish_registration(&rp(), &state, &response),
        Err(PasskeyError::UserNotPresent)
    ));

    let mut response = registration(&state, &key, UP_UV, ORIGIN, "example.com");
    response.raw_id = b64(b"some-other-id");
    assert!(matches!(
        finish_registration(&rp(), &state, &response),
        Err(PasskeyError::CredentialMismatch)
    ));
}

#[test]
fn registration_rejects_unsupported_algorithms() {
    let (_, state) = start_registration(&rp(), &user_handle_for(42), "ada", "Ada", &[]);
    // RS256 (-257) key: not supported.
    let cose = enc(&Value::Map(vec![
        (int(1), int(3)),
        (int(3), int(-257)),
        (int(-1), Value::Bytes(vec![1; 256])),
        (int(-2), Value::Bytes(vec![1, 0, 1])),
    ]));
    let data = auth_data(
        "example.com",
        UP_UV | FLAG_ATTESTED_DATA,
        0,
        Some((CRED_ID, cose)),
    );
    let attestation = enc(&Value::Map(vec![
        (Value::Text("fmt".into()), Value::Text("none".into())),
        (Value::Text("attStmt".into()), Value::Map(vec![])),
        (Value::Text("authData".into()), Value::Bytes(data)),
    ]));
    let response = RegistrationResponse {
        raw_id: b64(CRED_ID),
        kind: "public-key".into(),
        response: AttestationResponse {
            client_data_json: b64(&client_data("webauthn.create", &state.challenge, ORIGIN)),
            attestation_object: b64(&attestation),
        },
    };
    assert!(matches!(
        finish_registration(&rp(), &state, &response),
        Err(PasskeyError::UnsupportedAlgorithm(-257))
    ));
}

// ---- Sign-in ----

#[test]
fn es256_and_ed25519_sign_in_and_return_the_new_counter() {
    for key in [TestKey::es256(), TestKey::ed25519()] {
        let passkey = register(&key);
        let (options, state) = start_authentication(&rp(), std::slice::from_ref(&passkey));
        assert_eq!(options["publicKey"]["rpId"], "example.com");
        assert_eq!(
            options["publicKey"]["allowCredentials"][0]["id"],
            b64(CRED_ID)
        );
        let response = assertion(&state, &key, UP_UV, 5, ORIGIN);
        assert_eq!(response.credential_id().unwrap(), CRED_ID);
        assert_eq!(
            finish_authentication(&rp(), &state, &response, &passkey).unwrap(),
            5
        );
    }
}

#[test]
fn sign_in_rejects_a_signature_from_another_key() {
    let passkey = register(&TestKey::es256());
    let (_, state) = start_authentication(&rp(), &[]);
    let response = assertion(&state, &TestKey::ed25519(), UP_UV, 1, ORIGIN);
    assert!(matches!(
        finish_authentication(&rp(), &state, &response, &passkey),
        Err(PasskeyError::BadSignature)
    ));
}

#[test]
fn sign_in_rejects_tampered_data() {
    let key = TestKey::es256();
    let passkey = register(&key);
    let (_, state) = start_authentication(&rp(), &[]);

    // Flipping the counter after signing breaks the signature.
    let mut response = assertion(&state, &key, UP_UV, 1, ORIGIN);
    let mut data = URL_SAFE_NO_PAD
        .decode(&response.response.authenticator_data)
        .unwrap();
    data[36] = 99;
    response.response.authenticator_data = b64(&data);
    assert!(matches!(
        finish_authentication(&rp(), &state, &response, &passkey),
        Err(PasskeyError::BadSignature)
    ));

    // A response to a different challenge.
    let (_, other_state) = start_authentication(&rp(), &[]);
    let response = assertion(&other_state, &key, UP_UV, 1, ORIGIN);
    assert!(matches!(
        finish_authentication(&rp(), &state, &response, &passkey),
        Err(PasskeyError::ChallengeMismatch)
    ));

    // From another site.
    let response = assertion(&state, &key, UP_UV, 1, "https://evil.example");
    assert!(matches!(
        finish_authentication(&rp(), &state, &response, &passkey),
        Err(PasskeyError::OriginMismatch(_))
    ));

    // Without user verification.
    let response = assertion(&state, &key, FLAG_USER_PRESENT, 1, ORIGIN);
    assert!(matches!(
        finish_authentication(&rp(), &state, &response, &passkey),
        Err(PasskeyError::UserNotVerified)
    ));

    // A registration-type client data replayed as a sign-in.
    let mut response = assertion(&state, &key, UP_UV, 1, ORIGIN);
    response.response.client_data_json =
        b64(&client_data("webauthn.create", &state.challenge, ORIGIN));
    assert!(finish_authentication(&rp(), &state, &response, &passkey).is_err());
}

#[test]
fn sign_in_rejects_wrong_credential_and_user_handle() {
    let key = TestKey::es256();
    let passkey = register(&key);
    let (_, state) = start_authentication(&rp(), &[]);

    let mut response = assertion(&state, &key, UP_UV, 1, ORIGIN);
    response.raw_id = b64(b"another-credential");
    assert!(matches!(
        finish_authentication(&rp(), &state, &response, &passkey),
        Err(PasskeyError::CredentialMismatch)
    ));

    let mut response = assertion(&state, &key, UP_UV, 1, ORIGIN);
    response.response.user_handle = Some(b64(&user_handle_for(7)));
    assert!(matches!(
        finish_authentication(&rp(), &state, &response, &passkey),
        Err(PasskeyError::CredentialMismatch)
    ));
}

#[test]
fn sign_in_detects_cloned_authenticators_by_counter() {
    let key = TestKey::es256();
    let mut passkey = register(&key);
    passkey.sign_count = 10;
    let (_, state) = start_authentication(&rp(), &[]);
    for stale in [10, 3] {
        let response = assertion(&state, &key, UP_UV, stale, ORIGIN);
        assert!(matches!(
            finish_authentication(&rp(), &state, &response, &passkey),
            Err(PasskeyError::CounterRegression)
        ));
    }
    // Authenticators without a counter always send 0; that's allowed.
    passkey.sign_count = 0;
    let response = assertion(&state, &key, UP_UV, 0, ORIGIN);
    assert_eq!(
        finish_authentication(&rp(), &state, &response, &passkey).unwrap(),
        0
    );
}

// ---- Malformed input never panics ----

#[test]
fn cbor_reader_rejects_malformed_input() {
    assert!(cbor::decode(&[]).is_err());
    assert!(cbor::decode(&[0x5a, 0xff, 0xff, 0xff, 0xff]).is_err()); // bytes longer than input
    assert!(cbor::decode(&[0x9b, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]).is_err()); // huge array
    assert!(cbor::decode(&[0x5f]).is_err()); // indefinite length
    assert!(cbor::decode(&[0xc0, 0x00]).is_err()); // tag
    assert!(cbor::decode(&[0xf9, 0x00, 0x00]).is_err()); // float
    assert!(cbor::decode(&[0xa2, 0x01, 0x00, 0x01, 0x00]).is_err()); // duplicate key
    assert!(cbor::decode(&[0x00, 0x00]).is_err()); // trailing bytes
    assert!(cbor::decode(&[0x63, 0xff, 0xfe, 0xfd]).is_err()); // invalid UTF-8
    let deep = [vec![0x81u8; 64], vec![0x00]].concat();
    assert!(cbor::decode(&deep).is_err()); // nesting too deep
    assert_eq!(cbor::decode(&[0x38, 0x18]).unwrap(), Value::Int(-25));
}

#[test]
fn every_truncation_of_a_valid_response_is_rejected_without_panicking() {
    let key = TestKey::es256();
    let (_, state) = start_registration(&rp(), &user_handle_for(42), "ada", "Ada", &[]);
    let full = registration(&state, &key, UP_UV, ORIGIN, "example.com");
    let attestation = URL_SAFE_NO_PAD
        .decode(&full.response.attestation_object)
        .unwrap();
    for len in 0..attestation.len() {
        let mut response = full.clone();
        response.response.attestation_object = b64(&attestation[..len]);
        assert!(
            finish_registration(&rp(), &state, &response).is_err(),
            "length {len}"
        );
    }
    let data = auth_data(
        "example.com",
        UP_UV | FLAG_ATTESTED_DATA,
        0,
        Some((CRED_ID, key.cose())),
    );
    for len in 0..data.len() {
        assert!(parse_auth_data(&data[..len]).is_err(), "length {len}");
    }
}

#[test]
fn errors_convert_to_client_and_server_errors() {
    let client: djangors_core::DjangorsError = PasskeyError::BadSignature.into();
    assert_eq!(client.status_code(), djangors_core::StatusCode::BAD_REQUEST);
    let server: djangors_core::DjangorsError = PasskeyError::Database("down".into()).into();
    assert!(matches!(server, djangors_core::DjangorsError::Internal(_)));
}

// ---- Storage ----

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn passkey_store_round_trip() {
    let config = djangors_db::DatabaseConfig::new("sqlite::memory:")
        .max_connections(1)
        .min_connections(1);
    let db = djangors_db::Database::connect(&config).await.unwrap();
    let store = PasskeyStore::new(db);
    let passkey = register(&TestKey::es256());

    store.add(42, &passkey, "Laptop").await.unwrap();
    let (user_id, found) = store.find(CRED_ID).await.unwrap().expect("stored");
    assert_eq!(user_id, 42);
    assert_eq!(found, passkey);
    assert!(store.find(b"missing").await.unwrap().is_none());

    store.update_sign_count(CRED_ID, 9).await.unwrap();
    assert_eq!(store.for_user(42).await.unwrap()[0].sign_count, 9);

    // The same credential can't be registered twice.
    assert!(store.add(42, &passkey, "Again").await.is_err());

    assert!(
        !store.remove(7, CRED_ID).await.unwrap(),
        "other users can't remove it"
    );
    assert!(store.remove(42, CRED_ID).await.unwrap());
    assert!(store.for_user(42).await.unwrap().is_empty());
}
