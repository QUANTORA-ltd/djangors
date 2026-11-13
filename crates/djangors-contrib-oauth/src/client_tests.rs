use super::*;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use djangors_auth::hash_password;
use djangors_core::AppState;
use djangors_db::config::DatabaseConfig;
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::http::{Extensions, HeaderMap};
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use jsonwebtoken::{DecodingKey, Validation};
use p256::pkcs8::EncodePrivateKey as _;
use rsa::traits::PublicKeyParts as _;
use rsa::RsaPrivateKey;
use sha2::{Digest, Sha256};
use std::convert::Infallible;
use std::sync::Mutex;
use tokio::net::TcpListener;

async fn setup_oauth_db() -> Database {
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

    create_social_account_table(&db).await.unwrap();
    db
}

#[derive(Clone)]
struct MockIdpState {
    issuer: String,
    rsa_pem: String,
    n_b64: String,
    e_b64: String,
    expected_code_verifier: Arc<Mutex<Option<String>>>,
    next_nonce: Arc<Mutex<Option<String>>>,
    next_email: Arc<Mutex<Option<String>>>,
    next_email_verified: Arc<Mutex<bool>>,
    next_subject: Arc<Mutex<String>>,
    next_issuer_override: Arc<Mutex<Option<String>>>,
    next_audience_override: Arc<Mutex<Option<String>>>,
}

#[derive(Serialize)]
struct MockIdTokenClaims {
    iss: String,
    sub: String,
    aud: String,
    nonce: String,
    iat: i64,
    exp: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    email: Option<String>,
    email_verified: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}

async fn spawn_mock_oidc_server() -> (String, MockIdpState) {
    let mut rng = rand::thread_rng();
    let private_key = RsaPrivateKey::new(&mut rng, 2048).unwrap();
    let rsa_pem = private_key
        .to_pkcs8_pem(rsa::pkcs8::LineEnding::LF)
        .unwrap()
        .to_string();
    let pub_key = private_key.to_public_key();
    let n_b64 = URL_SAFE_NO_PAD.encode(pub_key.n().to_bytes_be());
    let e_b64 = URL_SAFE_NO_PAD.encode(pub_key.e().to_bytes_be());

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{addr}");

    let state = MockIdpState {
        issuer: base_url.clone(),
        rsa_pem,
        n_b64,
        e_b64,
        expected_code_verifier: Arc::new(Mutex::new(None)),
        next_nonce: Arc::new(Mutex::new(None)),
        next_email: Arc::new(Mutex::new(Some("oidc.user@example.com".to_string()))),
        next_email_verified: Arc::new(Mutex::new(true)),
        next_subject: Arc::new(Mutex::new("sub-oidc-123".to_string())),
        next_issuer_override: Arc::new(Mutex::new(None)),
        next_audience_override: Arc::new(Mutex::new(None)),
    };

    let srv_state = state.clone();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let st = srv_state.clone();
            tokio::spawn(async move {
                let io = TokioIo::new(stream);
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(
                        io,
                        service_fn(move |req: hyper::Request<hyper::body::Incoming>| {
                            let st = st.clone();
                            async move {
                                let path = req.uri().path().to_string();
                                let body_bytes = req
                                    .into_body()
                                    .collect()
                                    .await
                                    .map(|b| b.to_bytes())
                                    .unwrap_or_default();
                                let (status, json_body) = match path.as_str() {
                                    "/.well-known/openid-configuration" => (
                                        hyper::StatusCode::OK,
                                        serde_json::json!({
                                            "issuer": st.issuer,
                                            "authorization_endpoint": format!("{}/authorize", st.issuer),
                                            "token_endpoint": format!("{}/token", st.issuer),
                                            "jwks_uri": format!("{}/jwks", st.issuer),
                                            "response_types_supported": ["code"],
                                            "subject_types_supported": ["public"],
                                            "id_token_signing_alg_values_supported": ["RS256"]
                                        }),
                                    ),
                                    "/jwks" => (
                                        hyper::StatusCode::OK,
                                        serde_json::json!({
                                            "keys": [{
                                                "kty": "RSA",
                                                "use": "sig",
                                                "alg": "RS256",
                                                "kid": "mock-key-1",
                                                "n": st.n_b64,
                                                "e": st.e_b64
                                            }]
                                        }),
                                    ),
                                    "/token" => {
                                        let form: HashMap<String, String> =
                                            serde_urlencoded::from_bytes(&body_bytes)
                                                .unwrap_or_default();
                                        if let Some(expected_v) =
                                            st.expected_code_verifier.lock().unwrap().as_ref()
                                        {
                                            if form.get("code_verifier").map(String::as_str)
                                                != Some(expected_v.as_str())
                                            {
                                                let body = serde_json::json!({
                                                    "error": "invalid_grant",
                                                    "error_description": "pkce verifier mismatch"
                                                });
                                                return Ok::<_, Infallible>(
                                                    hyper::Response::builder()
                                                        .status(hyper::StatusCode::BAD_REQUEST)
                                                        .header("content-type", "application/json")
                                                        .body(Full::new(Bytes::from(body.to_string())))
                                                        .unwrap(),
                                                );
                                            }
                                        }
                                        let now = Utc::now().timestamp();
                                        let nonce = st
                                            .next_nonce
                                            .lock()
                                            .unwrap()
                                            .clone()
                                            .unwrap_or_default();
                                        let iss = st
                                            .next_issuer_override
                                            .lock()
                                            .unwrap()
                                            .clone()
                                            .unwrap_or_else(|| st.issuer.clone());
                                        let aud = st
                                            .next_audience_override
                                            .lock()
                                            .unwrap()
                                            .clone()
                                            .unwrap_or_else(|| "mock-client".to_string());
                                        let claims = MockIdTokenClaims {
                                            iss,
                                            sub: st.next_subject.lock().unwrap().clone(),
                                            aud,
                                            nonce,
                                            iat: now - 5,
                                            exp: now + 300,
                                            email: st.next_email.lock().unwrap().clone(),
                                            email_verified: *st.next_email_verified.lock().unwrap(),
                                            name: Some("Mock OIDC User".to_string()),
                                        };
                                        let mut hdr = Header::new(Algorithm::RS256);
                                        hdr.kid = Some("mock-key-1".to_string());
                                        let enc_key =
                                            EncodingKey::from_rsa_pem(st.rsa_pem.as_bytes())
                                                .unwrap();
                                        let id_token =
                                            jsonwebtoken::encode(&hdr, &claims, &enc_key).unwrap();
                                        (
                                            hyper::StatusCode::OK,
                                            serde_json::json!({
                                                "access_token": "mock-access-token",
                                                "token_type": "Bearer",
                                                "expires_in": 3600,
                                                "id_token": id_token
                                            }),
                                        )
                                    }
                                    _ => (
                                        hyper::StatusCode::NOT_FOUND,
                                        serde_json::json!({"error": "not_found"}),
                                    ),
                                };
                                Ok::<_, Infallible>(
                                    hyper::Response::builder()
                                        .status(status)
                                        .header("content-type", "application/json")
                                        .body(Full::new(Bytes::from(json_body.to_string())))
                                        .unwrap(),
                                )
                            }
                        }),
                    )
                    .await;
            });
        }
    });

    (base_url, state)
}

fn extract_query_param(url: &str, key: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    parsed
        .query_pairs()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.into_owned())
}

#[tokio::test]
async fn oidc_full_flow_verifies_jwks_pkce_nonce_and_logs_user_in() {
    let db = setup_oauth_db().await;
    let (issuer, idp_state) = spawn_mock_oidc_server().await;

    let provider = OidcProvider::new(
        OidcConfig::new(
            "oidc",
            issuer,
            "mock-client",
            Some("mock-secret".to_string()),
        )
        .with_allow_insecure_http(true),
    )
    .unwrap();

    let registry = OAuthRegistry::new("http://localhost:8000").register(provider);
    let state = AppState::new().insert(db.clone()).insert(registry);
    let session = Session::new_empty();
    let mut ext = Extensions::new();
    ext.insert(session.clone());

    let login_req = Request::new(
        Method::GET,
        "/auth/oidc/login".parse().unwrap(),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext.clone())
    .with_state(state.clone());

    let mut params = PathParams::new();
    params.insert("provider", "oidc");
    let login_resp = oauth_login_view(login_req, params.clone()).await.unwrap();
    assert_eq!(login_resp.status(), StatusCode::FOUND);
    let auth_url = login_resp
        .headers()
        .get("location")
        .unwrap()
        .to_str()
        .unwrap();

    let state_param = extract_query_param(auth_url, "state").unwrap();
    let nonce_param = extract_query_param(auth_url, "nonce").unwrap();
    let challenge = extract_query_param(auth_url, "code_challenge").unwrap();
    assert_eq!(
        extract_query_param(auth_url, "code_challenge_method").as_deref(),
        Some("S256")
    );

    let pending: PendingOAuthFlow = session
        .get(&pending_session_key(&state_param))
        .expect("pending OAuth state saved in session");
    // Confirm S256 challenge equals SHA256(verifier).
    let expected_challenge =
        URL_SAFE_NO_PAD.encode(Sha256::digest(pending.pkce_verifier.as_bytes()));
    assert_eq!(challenge, expected_challenge);

    *idp_state.expected_code_verifier.lock().unwrap() = Some(pending.pkce_verifier.clone());
    *idp_state.next_nonce.lock().unwrap() = Some(nonce_param);

    let cb_req = Request::new(
        Method::GET,
        format!("/auth/oidc/callback?code=auth-code-1&state={state_param}")
            .parse()
            .unwrap(),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext)
    .with_state(state);

    let cb_resp = oauth_callback_view(cb_req, params).await.unwrap();
    assert_eq!(cb_resp.status(), StatusCode::FOUND);

    // Session now authenticated & pending state was consumed.
    let logged_in_uid = session
        .get::<i64>(SESSION_USER_ID_KEY)
        .expect("user should be logged in");
    assert!(session
        .get::<PendingOAuthFlow>(&pending_session_key(&state_param))
        .is_none());

    let social = SocialAccount::objects().all(&db).await.unwrap();
    assert_eq!(social.len(), 1);
    assert_eq!(social[0].provider, "oidc");
    assert_eq!(social[0].subject, "sub-oidc-123");
    assert_eq!(social[0].user.id, logged_in_uid);
}

#[tokio::test]
async fn oidc_rejects_state_mismatch_nonce_mismatch_and_wrong_issuer() {
    let db = setup_oauth_db().await;
    let (issuer, idp_state) = spawn_mock_oidc_server().await;

    let provider = OidcProvider::new(
        OidcConfig::new(
            "oidc",
            issuer,
            "mock-client",
            Some("mock-secret".to_string()),
        )
        .with_allow_insecure_http(true),
    )
    .unwrap();
    let registry = OAuthRegistry::new("http://localhost:8000").register(provider);
    let state = AppState::new().insert(db).insert(registry);

    // 1. State mismatch rejected.
    let session = Session::new_empty();
    let mut ext = Extensions::new();
    ext.insert(session.clone());
    let mut params = PathParams::new();
    params.insert("provider", "oidc");

    let login_req = Request::new(
        Method::GET,
        "/auth/oidc/login".parse().unwrap(),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext.clone())
    .with_state(state.clone());
    let login_resp = oauth_login_view(login_req, params.clone()).await.unwrap();
    let auth_url = login_resp
        .headers()
        .get("location")
        .unwrap()
        .to_str()
        .unwrap();
    let valid_state = extract_query_param(auth_url, "state").unwrap();

    let bad_state_req = Request::new(
        Method::GET,
        "/auth/oidc/callback?code=c1&state=tampered-state"
            .parse()
            .unwrap(),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext.clone())
    .with_state(state.clone());
    assert!(oauth_callback_view(bad_state_req, params.clone())
        .await
        .is_err());

    // 2. Nonce mismatch rejected (and consumes valid_state so replay also fails!).
    *idp_state.next_nonce.lock().unwrap() = Some("wrong-nonce".to_string());
    let bad_nonce_req = Request::new(
        Method::GET,
        format!("/auth/oidc/callback?code=c2&state={valid_state}")
            .parse()
            .unwrap(),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext.clone())
    .with_state(state.clone());
    assert!(oauth_callback_view(bad_nonce_req, params.clone())
        .await
        .is_err());

    // Replaying the same state after failure is rejected.
    let replay_req = Request::new(
        Method::GET,
        format!("/auth/oidc/callback?code=c2&state={valid_state}")
            .parse()
            .unwrap(),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext.clone())
    .with_state(state.clone());
    assert!(oauth_callback_view(replay_req, params.clone())
        .await
        .is_err());

    // 3. Wrong issuer claim rejected even when signature and nonce match.
    let login_req2 = Request::new(
        Method::GET,
        "/auth/oidc/login".parse().unwrap(),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext.clone())
    .with_state(state.clone());
    let login_resp2 = oauth_login_view(login_req2, params.clone()).await.unwrap();
    let auth_url2 = login_resp2
        .headers()
        .get("location")
        .unwrap()
        .to_str()
        .unwrap();
    let state2 = extract_query_param(auth_url2, "state").unwrap();
    let nonce2 = extract_query_param(auth_url2, "nonce").unwrap();

    *idp_state.next_nonce.lock().unwrap() = Some(nonce2);
    *idp_state.next_issuer_override.lock().unwrap() = Some("https://evil.example.com".to_string());
    let bad_iss_req = Request::new(
        Method::GET,
        format!("/auth/oidc/callback?code=c3&state={state2}")
            .parse()
            .unwrap(),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext)
    .with_state(state);
    assert!(oauth_callback_view(bad_iss_req, params).await.is_err());
}

#[tokio::test]
async fn unverified_email_never_links_existing_user_while_verified_email_links() {
    let db = setup_oauth_db().await;
    let alice = User {
        id: 0,
        username: "alice".to_string(),
        email: "alice@example.com".to_string(),
        password: hash_password("pass12345!").unwrap(),
        is_active: true,
        is_staff: false,
        is_superuser: false,
        date_joined: Utc::now(),
        last_login: None,
    }
    .save(&db)
    .await
    .unwrap();

    // Unverified email claim MUST create a distinct user instead of hijacking `alice`.
    let unverified = OAuthIdentity {
        provider: "github".to_string(),
        subject: "gh-999".to_string(),
        email: Some("alice@example.com".to_string()),
        email_verified: false,
        username_hint: Some("attacker".to_string()),
        name: None,
        extra: serde_json::json!({}),
    };
    let (u1, _) = resolve_or_create_social_user(&db, &unverified, None)
        .await
        .unwrap();
    assert_ne!(u1.id, alice.id);
    assert_eq!(u1.email, "");

    // Verified email claim DOES link to `alice`.
    let verified = OAuthIdentity {
        provider: "google".to_string(),
        subject: "goog-111".to_string(),
        email: Some("alice@example.com".to_string()),
        email_verified: true,
        username_hint: Some("alice".to_string()),
        name: Some("Alice".to_string()),
        extra: serde_json::json!({}),
    };
    let (u2, _) = resolve_or_create_social_user(&db, &verified, None)
        .await
        .unwrap();
    assert_eq!(u2.id, alice.id);
}

#[test]
fn github_email_selection_strictly_requires_verified_email() {
    // Primary unverified, secondary verified -> pick secondary verified.
    let emails = vec![
        GitHubEmailEntry {
            email: "spoof@example.com".to_string(),
            primary: true,
            verified: false,
        },
        GitHubEmailEntry {
            email: "real@example.com".to_string(),
            primary: false,
            verified: true,
        },
    ];
    assert_eq!(
        select_verified_github_email(&emails),
        (Some("real@example.com".to_string()), true)
    );

    // Primary verified -> pick primary.
    let both_verified = vec![
        GitHubEmailEntry {
            email: "secondary@example.com".to_string(),
            primary: false,
            verified: true,
        },
        GitHubEmailEntry {
            email: "primary@example.com".to_string(),
            primary: true,
            verified: true,
        },
    ];
    assert_eq!(
        select_verified_github_email(&both_verified),
        (Some("primary@example.com".to_string()), true)
    );

    // No verified email -> None + false.
    let none_verified = vec![GitHubEmailEntry {
        email: "unverified@example.com".to_string(),
        primary: true,
        verified: false,
    }];
    assert_eq!(select_verified_github_email(&none_verified), (None, false));
}

#[tokio::test]
async fn apple_client_secret_jwt_and_form_post_callback_work() {
    let mut rng = rand::thread_rng();
    let signing_key = p256::ecdsa::SigningKey::random(&mut rng);
    let pkcs8_pem = signing_key
        .to_pkcs8_pem(p256::pkcs8::LineEnding::LF)
        .unwrap()
        .to_string();
    let verifying_key = p256::ecdsa::VerifyingKey::from(&signing_key);
    let spki_pem = p256::pkcs8::EncodePublicKey::to_public_key_pem(
        &verifying_key,
        p256::pkcs8::LineEnding::LF,
    )
    .unwrap();

    let (issuer, idp_state) = spawn_mock_oidc_server().await;
    let mut apple_cfg = AppleConfig::new("com.example.web", "TEAM123456", "KEY1234567", pkcs8_pem);
    apple_cfg.issuer_url = issuer;
    apple_cfg.allow_insecure_http = true;
    let apple = AppleProvider::new(apple_cfg).unwrap();

    // Verify Apple client secret JWT signature & claims.
    let secret_jwt = apple.generate_client_secret_jwt().unwrap();
    let header = jsonwebtoken::decode_header(&secret_jwt).unwrap();
    assert_eq!(header.alg, Algorithm::ES256);
    assert_eq!(header.kid.as_deref(), Some("KEY1234567"));
    let mut val = Validation::new(Algorithm::ES256);
    val.set_audience(&[APPLE_ISSUER]);
    val.set_issuer(&["TEAM123456"]);
    let decoded = jsonwebtoken::decode::<serde_json::Value>(
        &secret_jwt,
        &DecodingKey::from_ec_pem(spki_pem.as_bytes()).unwrap(),
        &val,
    )
    .unwrap();
    assert_eq!(decoded.claims["sub"], "com.example.web");

    // Verify Apple authorization URL requests response_mode=form_post.
    let auth_url = apple
        .authorization_url(
            "https://app.example.com/auth/apple/callback",
            "st",
            "ch",
            Some("nc"),
        )
        .await
        .unwrap();
    assert_eq!(
        extract_query_param(&auth_url, "response_mode").as_deref(),
        Some("form_post")
    );

    // Verify POST form_post callback handles `user` JSON name on first login.
    let db = setup_oauth_db().await;
    let registry = OAuthRegistry::new("http://localhost:8000").register(apple);
    let state = AppState::new().insert(db.clone()).insert(registry);
    let session = Session::new_empty();
    let mut ext = Extensions::new();
    ext.insert(session.clone());
    let mut params = PathParams::new();
    params.insert("provider", "apple");

    let login_req = Request::new(
        Method::GET,
        "/auth/apple/login".parse().unwrap(),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext.clone())
    .with_state(state.clone());
    let login_resp = oauth_login_view(login_req, params.clone()).await.unwrap();
    let redirect_loc = login_resp
        .headers()
        .get("location")
        .unwrap()
        .to_str()
        .unwrap();
    let st_val = extract_query_param(redirect_loc, "state").unwrap();
    let nc_val = extract_query_param(redirect_loc, "nonce").unwrap();

    *idp_state.next_nonce.lock().unwrap() = Some(nc_val);
    *idp_state.next_audience_override.lock().unwrap() = Some("com.example.web".to_string());

    let form_body = serde_urlencoded::to_string([
        ("code", "apple-code"),
        ("state", &st_val),
        (
            "user",
            r#"{"name":{"firstName":"Ada","lastName":"Lovelace"}}"#,
        ),
    ])
    .unwrap();
    let mut hdrs = HeaderMap::new();
    hdrs.insert(
        "content-type",
        "application/x-www-form-urlencoded".parse().unwrap(),
    );
    let cb_req = Request::new(
        Method::POST,
        "/auth/apple/callback".parse().unwrap(),
        hdrs,
        Bytes::from(form_body),
    )
    .with_extensions(ext)
    .with_state(state);

    let cb_resp = oauth_callback_view(cb_req, params).await.unwrap();
    assert_eq!(cb_resp.status(), StatusCode::FOUND);
    assert!(session.get::<i64>(SESSION_USER_ID_KEY).is_some());
}

#[tokio::test]
async fn disconnect_refuses_when_it_would_lock_out_user_without_password() {
    let db = setup_oauth_db().await;
    let social_only_user = User {
        id: 0,
        username: "sso_user".to_string(),
        email: "sso@example.com".to_string(),
        password: "!".to_string(),
        is_active: true,
        is_staff: false,
        is_superuser: false,
        date_joined: Utc::now(),
        last_login: None,
    }
    .save(&db)
    .await
    .unwrap();

    SocialAccount {
        id: 0,
        provider: "google".to_string(),
        subject: "g-1".to_string(),
        user: ForeignKey::new(social_only_user.id),
        email: "sso@example.com".to_string(),
        email_verified: true,
        extra_json: "{}".to_string(),
        created_at: Utc::now(),
    }
    .save(&db)
    .await
    .unwrap();

    let session = Session::new_empty();
    session.set(SESSION_USER_ID_KEY, social_only_user.id);
    let mut ext = Extensions::new();
    ext.insert(session);
    let state = AppState::new().insert(db.clone());
    let mut params = PathParams::new();
    params.insert("provider", "google");

    let req1 = Request::new(
        Method::POST,
        "/auth/google/disconnect".parse().unwrap(),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext.clone())
    .with_state(state.clone());
    assert!(disconnect_provider_view(req1, params.clone())
        .await
        .is_err());

    // Linking a second provider lets the user disconnect Google safely.
    SocialAccount {
        id: 0,
        provider: "github".to_string(),
        subject: "gh-1".to_string(),
        user: ForeignKey::new(social_only_user.id),
        email: "sso@example.com".to_string(),
        email_verified: true,
        extra_json: "{}".to_string(),
        created_at: Utc::now(),
    }
    .save(&db)
    .await
    .unwrap();

    let req2 = Request::new(
        Method::POST,
        "/auth/google/disconnect".parse().unwrap(),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext)
    .with_state(state);
    let ok = disconnect_provider_view(req2, params).await.unwrap();
    assert_eq!(ok.status(), StatusCode::OK);
    let left = SocialAccount::objects()
        .filter(q!(user = social_only_user.id))
        .unwrap()
        .all(&db)
        .await
        .unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].provider, "github");
}

#[test]
fn microsoft_multi_tenant_issuer_policy_validates_tenant_url() {
    assert!(OidcProvider::verify_microsoft_multi_tenant_issuer(
        "https://login.microsoftonline.com/9188040d-6c67-4c5b-b112-36a304b66dad/v2.0"
    )
    .is_ok());
    assert!(OidcProvider::verify_microsoft_multi_tenant_issuer(
        "https://login.microsoftonline.com/{tenantid}/v2.0"
    )
    .is_err());
    assert!(OidcProvider::verify_microsoft_multi_tenant_issuer(
        "https://evil.example.com/123/v2.0"
    )
    .is_err());
}
