use super::*;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use djangors_auth::email_verification::{create_email_address_table, EmailAddress};
use djangors_auth::{hash_password, SESSION_USER_ID_KEY};
use djangors_core::AppState;
use djangors_db::config::DatabaseConfig;
use djangors_sessions::Session;
use hyper::body::Bytes;
use hyper::http::{Extensions, HeaderMap, Method};
use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{decode, decode_header, DecodingKey, Validation};
use std::sync::Once;

static INIT_ENC_KEY: Once = Once::new();

fn init_test_encryption_key() {
    INIT_ENC_KEY.call_once(|| {
        djangors_orm::encryption::set_keys_for_tests([0x42_u8; 32], vec![]);
    });
}

async fn setup_provider_db() -> Database {
    init_test_encryption_key();
    let config = DatabaseConfig::new("sqlite::memory:")
        .max_connections(1)
        .min_connections(1);
    let db = Database::connect(&config).await.unwrap();
    let d = db.dialect();
    let pk = d.auto_pk_type();
    let ts = d.timestamp_type();
    let b = d.bool_type();

    db.conn()
        .execute(
            &format!(
                "CREATE TABLE auth_user (
                    id {pk},
                    username VARCHAR(150) NOT NULL UNIQUE,
                    email VARCHAR(254) NOT NULL,
                    password TEXT NOT NULL,
                    is_active {b} NOT NULL,
                    is_staff {b} NOT NULL,
                    is_superuser {b} NOT NULL,
                    date_joined {ts} NOT NULL,
                    last_login {ts}
                )"
            ),
            &[],
        )
        .await
        .unwrap();

    create_email_address_table(&db).await.unwrap();
    create_oauth_provider_tables(&db).await.unwrap();
    db
}

async fn seed_user(db: &Database, username: &str, email: &str, verified: bool) -> User {
    let user = User {
        id: 0,
        username: username.to_string(),
        email: email.to_string(),
        password: hash_password("pass-123456!").unwrap(),
        is_active: true,
        is_staff: false,
        is_superuser: false,
        date_joined: Utc::now(),
        last_login: None,
    }
    .save(db)
    .await
    .unwrap();

    if verified {
        EmailAddress {
            id: 0,
            user: ForeignKey::new(user.id),
            email: email.to_string(),
            verified: true,
            primary: true,
            verified_at: Some(Utc::now()),
        }
        .save(db)
        .await
        .unwrap();
    }

    user
}

fn build_state(db: Database, issuer: &str) -> AppState {
    AppState::new()
        .insert(db)
        .insert(OAuthProviderConfig::new(issuer))
}

fn session_extensions(user: Option<&User>) -> Extensions {
    let session = Session::new_empty();
    if let Some(u) = user {
        session.set(SESSION_USER_ID_KEY, u.id);
    }
    let mut ext = Extensions::new();
    ext.insert(session);
    ext
}

fn form_request(
    uri: &str,
    params: &[(&str, &str)],
    headers: HeaderMap,
    state: AppState,
    ext: Extensions,
) -> Request {
    let mut hdrs = headers;
    hdrs.insert(
        "content-type",
        "application/x-www-form-urlencoded".parse().unwrap(),
    );
    let body = serde_urlencoded::to_string(params).unwrap();
    Request::new(Method::POST, uri.parse().unwrap(), hdrs, Bytes::from(body))
        .with_extensions(ext)
        .with_state(state)
}

fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

fn query_param(location: &str, key: &str) -> Option<String> {
    let url = Url::parse(location).ok()?;
    url.query_pairs()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.into_owned())
}

#[derive(Deserialize)]
struct DecodedIdToken {
    iss: String,
    sub: String,
    aud: String,
    nonce: Option<String>,
    at_hash: String,
    preferred_username: Option<String>,
    email: Option<String>,
    email_verified: Option<bool>,
}

#[tokio::test]
async fn signing_keys_encrypt_at_rest_and_publish_jwks_and_discovery() {
    let db = setup_provider_db().await;
    let rsa_key = SigningKey::generate_rs256()
        .unwrap()
        .save(&db)
        .await
        .unwrap();
    let ec_key = SigningKey::generate_es256()
        .unwrap()
        .save(&db)
        .await
        .unwrap();

    // Verify private_key_pem is stored encrypted with enc:v1: in raw SQLite storage.
    let raw_row = db
        .conn()
        .fetch_one(
            "SELECT private_key_pem FROM oauth_signing_key WHERE id = ?",
            &[djangors_db::BindValue::I64(rsa_key.id)],
        )
        .await
        .unwrap();
    let stored_cipher = raw_row.try_string(0).unwrap().unwrap();
    assert!(
        stored_cipher.starts_with("enc:v1:"),
        "private_key_pem must be encrypted at rest, got: {stored_cipher}"
    );
    assert!(!stored_cipher.contains("BEGIN PRIVATE KEY"));

    // Verify model load transparently decrypts the PEM.
    let loaded = SigningKey::objects()
        .filter(djangors_orm::q!(id = ec_key.id))
        .unwrap()
        .get(&db)
        .await
        .unwrap();
    assert!(loaded.private_key_pem.contains("BEGIN PRIVATE KEY"));

    let state = build_state(db.clone(), "https://auth.example.com/");

    // /.well-known/openid-configuration
    let disc_req = Request::new(
        Method::GET,
        "/.well-known/openid-configuration".parse().unwrap(),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_state(state.clone());
    let disc_resp = openid_configuration_view(disc_req, PathParams::new())
        .await
        .unwrap();
    assert_eq!(disc_resp.status(), StatusCode::OK);
    let disc_json: serde_json::Value = serde_json::from_slice(disc_resp.body()).unwrap();
    assert_eq!(disc_json["issuer"], "https://auth.example.com");
    assert_eq!(
        disc_json["token_endpoint"],
        "https://auth.example.com/o/token"
    );
    assert_eq!(disc_json["jwks_uri"], "https://auth.example.com/o/jwks");

    // /o/jwks must only include public keys.
    let jwks_req = Request::new(
        Method::GET,
        "/o/jwks".parse().unwrap(),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_state(state);
    let jwks_resp = jwks_view(jwks_req, PathParams::new()).await.unwrap();
    assert_eq!(jwks_resp.status(), StatusCode::OK);
    let body_str = std::str::from_utf8(jwks_resp.body()).unwrap();
    assert!(!body_str.contains("PRIVATE KEY"));
    assert!(!body_str.contains("\"d\":"));
    let jwk_set: JwkSet = serde_json::from_slice(jwks_resp.body()).unwrap();
    assert_eq!(jwk_set.keys.len(), 2);
}

#[tokio::test]
async fn authorization_code_pkce_oidc_id_token_and_userinfo_flow() {
    let db = setup_provider_db().await;
    let alice = seed_user(&db, "alice", "alice@example.com", true).await;
    let rsa_key = SigningKey::generate_rs256()
        .unwrap()
        .save(&db)
        .await
        .unwrap();

    // Public client: PKCE S256 mandatory, no client_secret.
    let spa_app = OAuthApplication::create_public(
        &db,
        "Example SPA",
        &["https://app.example.com/cb"],
        &["authorization_code", "refresh_token"],
        &["openid", "profile", "email", "offline_access"],
        false,
        Some(alice.id),
    )
    .await
    .unwrap();

    let state = build_state(db.clone(), "https://auth.example.com");
    let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk-abcdef";
    let challenge = pkce_challenge(verifier);

    // 1. Unregistered redirect_uri returns direct 400 (never redirects!).
    let bad_redir_req = Request::new(
        Method::GET,
        format!(
            "/o/authorize?response_type=code&client_id={}&redirect_uri=https://evil.example/cb&scope=openid&code_challenge={challenge}&code_challenge_method=S256",
            spa_app.client_id
        )
        .parse()
        .unwrap(),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(session_extensions(Some(&alice)))
    .with_state(state.clone());
    let bad_redir_resp = authorize_get_view(bad_redir_req, PathParams::new())
        .await
        .unwrap();
    assert_eq!(bad_redir_resp.status(), StatusCode::BAD_REQUEST);

    // 2. Public client missing PKCE redirects back with error=invalid_request.
    let no_pkce_req = Request::new(
        Method::GET,
        format!(
            "/o/authorize?response_type=code&client_id={}&redirect_uri=https://app.example.com/cb&scope=openid&state=st0",
            spa_app.client_id
        )
        .parse()
        .unwrap(),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(session_extensions(Some(&alice)))
    .with_state(state.clone());
    let no_pkce_resp = authorize_get_view(no_pkce_req, PathParams::new())
        .await
        .unwrap();
    assert_eq!(no_pkce_resp.status(), StatusCode::FOUND);
    let loc = no_pkce_resp
        .headers()
        .get("location")
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(
        query_param(loc, "error").as_deref(),
        Some("invalid_request")
    );

    // 3. Unauthenticated GET /o/authorize redirects to login_url.
    let unauth_req = Request::new(
        Method::GET,
        format!(
            "/o/authorize?response_type=code&client_id={}&redirect_uri=https://app.example.com/cb&scope=openid&code_challenge={challenge}&code_challenge_method=S256",
            spa_app.client_id
        )
        .parse()
        .unwrap(),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(session_extensions(None))
    .with_state(state.clone());
    let unauth_resp = authorize_get_view(unauth_req, PathParams::new())
        .await
        .unwrap();
    assert_eq!(unauth_resp.status(), StatusCode::FOUND);
    assert!(unauth_resp
        .headers()
        .get("location")
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("/accounts/login?next="));

    // 4. Authenticated GET renders consent screen.
    let consent_req = Request::new(
        Method::GET,
        format!(
            "/o/authorize?response_type=code&client_id={}&redirect_uri=https://app.example.com/cb&scope=openid%20profile%20email&state=xyz&nonce=n-123&code_challenge={challenge}&code_challenge_method=S256",
            spa_app.client_id
        )
        .parse()
        .unwrap(),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(session_extensions(Some(&alice)))
    .with_state(state.clone());
    let consent_resp = authorize_get_view(consent_req, PathParams::new())
        .await
        .unwrap();
    assert_eq!(consent_resp.status(), StatusCode::OK);
    let consent_html = std::str::from_utf8(consent_resp.body()).unwrap();
    assert!(consent_html.contains("Example SPA"));
    assert!(consent_html.contains("alice"));

    // 5. Consent approval POST mints single-use code (stored hashed).
    let approve_req = form_request(
        "/o/authorize",
        &[
            ("response_type", "code"),
            ("client_id", &spa_app.client_id),
            ("redirect_uri", "https://app.example.com/cb"),
            ("scope", "openid profile email"),
            ("state", "xyz"),
            ("nonce", "n-123"),
            ("code_challenge", &challenge),
            ("code_challenge_method", "S256"),
            ("decision", "approve"),
        ],
        HeaderMap::new(),
        state.clone(),
        session_extensions(Some(&alice)),
    );
    let approve_resp = authorize_post_view(approve_req, PathParams::new())
        .await
        .unwrap();
    assert_eq!(approve_resp.status(), StatusCode::FOUND);
    let cb_loc = approve_resp
        .headers()
        .get("location")
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(query_param(cb_loc, "state").as_deref(), Some("xyz"));
    let raw_code = query_param(cb_loc, "code").unwrap();

    // Ensure code is hashed in DB (not plaintext).
    let saved_code = AuthorizationCode::objects().all(&db).await.unwrap();
    assert_eq!(saved_code.len(), 1);
    assert_ne!(saved_code[0].code_hash, raw_code);

    // 6. Wrong PKCE verifier fails cleanly and burns the claimed code.
    let bad_pkce_token_req = form_request(
        "/o/token",
        &[
            ("grant_type", "authorization_code"),
            ("client_id", &spa_app.client_id),
            ("code", &raw_code),
            ("redirect_uri", "https://app.example.com/cb"),
            (
                "code_verifier",
                "wrong-verifier-00000000000000000000000000000000",
            ),
        ],
        HeaderMap::new(),
        state.clone(),
        Extensions::new(),
    );
    let bad_pkce_resp = token_view(bad_pkce_token_req, PathParams::new())
        .await
        .unwrap();
    assert_eq!(bad_pkce_resp.status(), StatusCode::BAD_REQUEST);
    let err_json: serde_json::Value = serde_json::from_slice(bad_pkce_resp.body()).unwrap();
    assert_eq!(err_json["error"], "invalid_grant");

    // 7. Re-approve a fresh code and exchange with valid PKCE verifier.
    let approve_req2 = form_request(
        "/o/authorize",
        &[
            ("response_type", "code"),
            ("client_id", &spa_app.client_id),
            ("redirect_uri", "https://app.example.com/cb"),
            ("scope", "openid profile email"),
            ("state", "xyz2"),
            ("nonce", "n-456"),
            ("code_challenge", &challenge),
            ("code_challenge_method", "S256"),
            ("decision", "approve"),
        ],
        HeaderMap::new(),
        state.clone(),
        session_extensions(Some(&alice)),
    );
    let approve_resp2 = authorize_post_view(approve_req2, PathParams::new())
        .await
        .unwrap();
    let raw_code2 = query_param(
        approve_resp2
            .headers()
            .get("location")
            .unwrap()
            .to_str()
            .unwrap(),
        "code",
    )
    .unwrap();

    let token_req = form_request(
        "/o/token",
        &[
            ("grant_type", "authorization_code"),
            ("client_id", &spa_app.client_id),
            ("code", &raw_code2),
            ("redirect_uri", "https://app.example.com/cb"),
            ("code_verifier", verifier),
        ],
        HeaderMap::new(),
        state.clone(),
        Extensions::new(),
    );
    let token_resp = token_view(token_req, PathParams::new()).await.unwrap();
    assert_eq!(token_resp.status(), StatusCode::OK);
    assert_eq!(
        token_resp
            .headers()
            .get("cache-control")
            .unwrap()
            .to_str()
            .unwrap(),
        "no-store"
    );

    let tok: serde_json::Value = serde_json::from_slice(token_resp.body()).unwrap();
    let access_token = tok["access_token"].as_str().unwrap().to_string();
    let id_token = tok["id_token"].as_str().unwrap().to_string();

    // Replaying the same authorization code must be rejected.
    let replay_req = form_request(
        "/o/token",
        &[
            ("grant_type", "authorization_code"),
            ("client_id", &spa_app.client_id),
            ("code", &raw_code2),
            ("redirect_uri", "https://app.example.com/cb"),
            ("code_verifier", verifier),
        ],
        HeaderMap::new(),
        state.clone(),
        Extensions::new(),
    );
    let replay_resp = token_view(replay_req, PathParams::new()).await.unwrap();
    assert_eq!(replay_resp.status(), StatusCode::BAD_REQUEST);

    // 8. Verify id_token signature & claims using public JWK from /o/jwks.
    let jwks_resp = jwks_view(
        Request::new(
            Method::GET,
            "/o/jwks".parse().unwrap(),
            HeaderMap::new(),
            Bytes::new(),
        )
        .with_state(state.clone()),
        PathParams::new(),
    )
    .await
    .unwrap();
    let jwk_set: JwkSet = serde_json::from_slice(jwks_resp.body()).unwrap();
    let hdr = decode_header(&id_token).unwrap();
    assert_eq!(hdr.kid.as_deref(), Some(rsa_key.kid.as_str()));
    let jwk = jwk_set.find(&rsa_key.kid).unwrap();
    let dec_key = DecodingKey::from_jwk(jwk).unwrap();
    let mut val = Validation::new(jsonwebtoken::Algorithm::RS256);
    val.set_audience(&[&spa_app.client_id]);
    val.set_issuer(&["https://auth.example.com"]);
    let verified = decode::<DecodedIdToken>(&id_token, &dec_key, &val).unwrap();
    assert_eq!(verified.claims.iss, "https://auth.example.com");
    assert_eq!(verified.claims.aud, spa_app.client_id);
    assert_eq!(verified.claims.sub, alice.id.to_string());
    assert_eq!(verified.claims.nonce.as_deref(), Some("n-456"));
    assert_eq!(verified.claims.email.as_deref(), Some("alice@example.com"));
    assert_eq!(verified.claims.email_verified, Some(true));
    assert_eq!(verified.claims.preferred_username.as_deref(), Some("alice"));
    let expected_at_hash = URL_SAFE_NO_PAD.encode(&Sha256::digest(access_token.as_bytes())[..16]);
    assert_eq!(verified.claims.at_hash, expected_at_hash);

    // 9. /o/userinfo returns profile & email claims for Bearer token.
    let mut ui_hdrs = HeaderMap::new();
    ui_hdrs.insert(
        "authorization",
        format!("Bearer {access_token}").parse().unwrap(),
    );
    let ui_req = Request::new(
        Method::GET,
        "/o/userinfo".parse().unwrap(),
        ui_hdrs,
        Bytes::new(),
    )
    .with_state(state.clone());
    let ui_resp = userinfo_view(ui_req, PathParams::new()).await.unwrap();
    assert_eq!(ui_resp.status(), StatusCode::OK);
    let ui_json: serde_json::Value = serde_json::from_slice(ui_resp.body()).unwrap();
    assert_eq!(ui_json["sub"], alice.id.to_string());
    assert_eq!(ui_json["preferred_username"], "alice");
    assert_eq!(ui_json["email"], "alice@example.com");
    assert_eq!(ui_json["email_verified"], true);
}

#[tokio::test]
async fn client_credentials_and_oauth_bearer_and_has_scope_work() {
    let db = setup_provider_db().await;
    let (m2m_app, secret) = OAuthApplication::create_confidential(
        &db,
        "Billing Service",
        &[],
        &["client_credentials"],
        &["read", "billing"],
        false,
        None,
    )
    .await
    .unwrap();
    let public_app = OAuthApplication::create_public(
        &db,
        "Mobile App",
        &["myapp://cb"],
        &["authorization_code", "client_credentials"],
        &["read"],
        false,
        None,
    )
    .await
    .unwrap();

    // Ensure client_secret is stored hashed.
    assert_ne!(m2m_app.client_secret_hash, secret);

    let state = build_state(db.clone(), "https://auth.example.com");

    // 1. Public client cannot use client_credentials.
    let pub_req = form_request(
        "/o/token",
        &[
            ("grant_type", "client_credentials"),
            ("client_id", &public_app.client_id),
            ("scope", "read"),
        ],
        HeaderMap::new(),
        state.clone(),
        Extensions::new(),
    );
    let pub_resp = token_view(pub_req, PathParams::new()).await.unwrap();
    assert_eq!(pub_resp.status(), StatusCode::BAD_REQUEST);

    // 2. Wrong secret rejected with 401.
    let bad_sec_req = form_request(
        "/o/token",
        &[
            ("grant_type", "client_credentials"),
            ("client_id", &m2m_app.client_id),
            ("client_secret", "wrong-secret"),
            ("scope", "read"),
        ],
        HeaderMap::new(),
        state.clone(),
        Extensions::new(),
    );
    let bad_sec_resp = token_view(bad_sec_req, PathParams::new()).await.unwrap();
    assert_eq!(bad_sec_resp.status(), StatusCode::UNAUTHORIZED);

    // 3. HTTP Basic client_credentials succeeds without issuing refresh_token.
    let basic = STANDARD.encode(format!("{}:{}", m2m_app.client_id, secret));
    let mut hdrs = HeaderMap::new();
    hdrs.insert("authorization", format!("Basic {basic}").parse().unwrap());
    let ok_req = form_request(
        "/o/token",
        &[("grant_type", "client_credentials"), ("scope", "read")],
        hdrs,
        state.clone(),
        Extensions::new(),
    );
    let ok_resp = token_view(ok_req, PathParams::new()).await.unwrap();
    assert_eq!(ok_resp.status(), StatusCode::OK);
    let tok: serde_json::Value = serde_json::from_slice(ok_resp.body()).unwrap();
    assert!(tok.get("refresh_token").is_none());
    let raw_access = tok["access_token"].as_str().unwrap();

    // 4. OAuthBearer and HasOAuthScope enforcement.
    let mut api_hdrs = HeaderMap::new();
    api_hdrs.insert(
        "authorization",
        format!("Bearer {raw_access}").parse().unwrap(),
    );
    let api_req = Request::new(
        Method::GET,
        "/api/items".parse().unwrap(),
        api_hdrs,
        Bytes::new(),
    )
    .with_state(state);

    let bearer = HasOAuthScope("read").check(&api_req).await.unwrap();
    assert_eq!(bearer.application_id, m2m_app.id);
    assert_eq!(bearer.user_id, None);
    assert!(HasOAuthScope("write").check(&api_req).await.is_err());
}

#[tokio::test]
async fn refresh_token_rotation_and_reuse_detection_revokes_family() {
    let db = setup_provider_db().await;
    let bob = seed_user(&db, "bob", "bob@example.com", true).await;
    SigningKey::generate_es256()
        .unwrap()
        .save(&db)
        .await
        .unwrap();

    let (app, secret) = OAuthApplication::create_confidential(
        &db,
        "Backend Web App",
        &["https://web.example.com/cb"],
        &["authorization_code", "refresh_token"],
        &["openid", "offline_access", "read"],
        true, // skip_consent
        None,
    )
    .await
    .unwrap();
    let state = build_state(db.clone(), "https://auth.example.com");

    // 1. Auto-approved GET /o/authorize issues authorization code.
    let verifier = "verifier-0123456789012345678901234567890123456789";
    let challenge = pkce_challenge(verifier);
    let auth_req = Request::new(
        Method::GET,
        format!(
            "/o/authorize?response_type=code&client_id={}&redirect_uri=https://web.example.com/cb&scope=openid%20offline_access%20read&code_challenge={challenge}&code_challenge_method=S256",
            app.client_id
        )
        .parse()
        .unwrap(),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(session_extensions(Some(&bob)))
    .with_state(state.clone());
    let auth_resp = authorize_get_view(auth_req, PathParams::new())
        .await
        .unwrap();
    assert_eq!(auth_resp.status(), StatusCode::FOUND);
    let code = query_param(
        auth_resp
            .headers()
            .get("location")
            .unwrap()
            .to_str()
            .unwrap(),
        "code",
    )
    .unwrap();

    let token_req = form_request(
        "/o/token",
        &[
            ("grant_type", "authorization_code"),
            ("client_id", &app.client_id),
            ("client_secret", &secret),
            ("code", &code),
            ("redirect_uri", "https://web.example.com/cb"),
            ("code_verifier", verifier),
        ],
        HeaderMap::new(),
        state.clone(),
        Extensions::new(),
    );
    let tok1: serde_json::Value = serde_json::from_slice(
        token_view(token_req, PathParams::new())
            .await
            .unwrap()
            .body(),
    )
    .unwrap();
    let rt1 = tok1["refresh_token"].as_str().unwrap().to_string();

    // 2. Rotate rt1 -> rt2.
    let rot_req = form_request(
        "/o/token",
        &[
            ("grant_type", "refresh_token"),
            ("client_id", &app.client_id),
            ("client_secret", &secret),
            ("refresh_token", &rt1),
        ],
        HeaderMap::new(),
        state.clone(),
        Extensions::new(),
    );
    let rot_resp = token_view(rot_req, PathParams::new()).await.unwrap();
    assert_eq!(rot_resp.status(), StatusCode::OK);
    let tok2: serde_json::Value = serde_json::from_slice(rot_resp.body()).unwrap();
    let rt2 = tok2["refresh_token"].as_str().unwrap().to_string();
    assert_ne!(rt1, rt2);

    // Scope escalation on refresh must be rejected without rotating rt2.
    let esc_req = form_request(
        "/o/token",
        &[
            ("grant_type", "refresh_token"),
            ("client_id", &app.client_id),
            ("client_secret", &secret),
            ("refresh_token", &rt2),
            ("scope", "write"),
        ],
        HeaderMap::new(),
        state.clone(),
        Extensions::new(),
    );
    let esc_resp = token_view(esc_req, PathParams::new()).await.unwrap();
    assert_eq!(esc_resp.status(), StatusCode::BAD_REQUEST);
    let esc_json: serde_json::Value = serde_json::from_slice(esc_resp.body()).unwrap();
    assert_eq!(esc_json["error"], "invalid_scope");

    // 3. Replaying old rt1 triggers reuse detection and revokes the whole family (including rt2!).
    let reuse_req = form_request(
        "/o/token",
        &[
            ("grant_type", "refresh_token"),
            ("client_id", &app.client_id),
            ("client_secret", &secret),
            ("refresh_token", &rt1),
        ],
        HeaderMap::new(),
        state.clone(),
        Extensions::new(),
    );
    let reuse_resp = token_view(reuse_req, PathParams::new()).await.unwrap();
    assert_eq!(reuse_resp.status(), StatusCode::BAD_REQUEST);

    // 4. Subsequent use of rt2 MUST fail because the whole family was revoked!
    let rt2_req = form_request(
        "/o/token",
        &[
            ("grant_type", "refresh_token"),
            ("client_id", &app.client_id),
            ("client_secret", &secret),
            ("refresh_token", &rt2),
        ],
        HeaderMap::new(),
        state,
        Extensions::new(),
    );
    let rt2_resp = token_view(rt2_req, PathParams::new()).await.unwrap();
    assert_eq!(rt2_resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn revoke_introspect_and_revoke_app_tokens_work() {
    let db = setup_provider_db().await;
    let (app, secret) = OAuthApplication::create_confidential(
        &db,
        "Worker",
        &[],
        &["client_credentials"],
        &["read"],
        false,
        None,
    )
    .await
    .unwrap();
    let state = build_state(db.clone(), "https://auth.example.com");

    // Issue two access tokens.
    let mut tokens = Vec::new();
    for _ in 0..2 {
        let req = form_request(
            "/o/token",
            &[
                ("grant_type", "client_credentials"),
                ("client_id", &app.client_id),
                ("client_secret", &secret),
                ("scope", "read"),
            ],
            HeaderMap::new(),
            state.clone(),
            Extensions::new(),
        );
        let resp = token_view(req, PathParams::new()).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(resp.body()).unwrap();
        tokens.push(body["access_token"].as_str().unwrap().to_string());
    }

    // Introspect first token -> active: true.
    let intr1 = introspect_view(
        form_request(
            "/o/introspect",
            &[
                ("client_id", &app.client_id),
                ("client_secret", &secret),
                ("token", &tokens[0]),
            ],
            HeaderMap::new(),
            state.clone(),
            Extensions::new(),
        ),
        PathParams::new(),
    )
    .await
    .unwrap();
    let intr1_json: serde_json::Value = serde_json::from_slice(intr1.body()).unwrap();
    assert_eq!(intr1_json["active"], true);
    assert_eq!(intr1_json["client_id"], app.client_id);

    // Revoke first token -> 200 OK; subsequent introspect returns active: false.
    let rev_resp = revoke_view(
        form_request(
            "/o/revoke",
            &[
                ("client_id", &app.client_id),
                ("client_secret", &secret),
                ("token", &tokens[0]),
            ],
            HeaderMap::new(),
            state.clone(),
            Extensions::new(),
        ),
        PathParams::new(),
    )
    .await
    .unwrap();
    assert_eq!(rev_resp.status(), StatusCode::OK);

    let intr1_after = introspect_view(
        form_request(
            "/o/introspect",
            &[
                ("client_id", &app.client_id),
                ("client_secret", &secret),
                ("token", &tokens[0]),
            ],
            HeaderMap::new(),
            state.clone(),
            Extensions::new(),
        ),
        PathParams::new(),
    )
    .await
    .unwrap();
    let intr1_after_json: serde_json::Value = serde_json::from_slice(intr1_after.body()).unwrap();
    assert_eq!(intr1_after_json["active"], false);

    // Bulk admin revocation clears remaining active token(s).
    let revoked_count = revoke_app_tokens(&db, app.id).await.unwrap();
    assert_eq!(revoked_count, 1);

    let mut bear_hdr = HeaderMap::new();
    bear_hdr.insert(
        "authorization",
        format!("Bearer {}", tokens[1]).parse().unwrap(),
    );
    let bearer_req = Request::new(
        Method::GET,
        "/api/check".parse().unwrap(),
        bear_hdr,
        Bytes::new(),
    )
    .with_state(state);
    assert!(OAuthBearer::from_request(&bearer_req).await.is_err());
}
