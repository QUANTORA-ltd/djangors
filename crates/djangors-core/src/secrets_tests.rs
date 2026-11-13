use super::*;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
#[cfg(feature = "secrets-remote")]
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[cfg(feature = "secrets-remote")]
use tokio::net::TcpListener;

struct MapProvider {
    values: HashMap<String, String>,
    calls: AtomicUsize,
}

impl MapProvider {
    fn new(pairs: &[(&str, &str)]) -> Self {
        Self {
            values: pairs
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
            calls: AtomicUsize::new(0),
        }
    }
}

#[async_trait::async_trait]
impl SecretsProvider for MapProvider {
    async fn get(&self, name: &str) -> Result<Option<SecretString>, SecretsError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.values.get(name).cloned().map(SecretString::from))
    }
}

#[test]
fn secret_string_redacts_debug_output() {
    let secret = SecretString::from("super-secret-value-12345");
    let dbg = format!("{secret:?}");
    assert!(!dbg.contains("super-secret-value-12345"));
    assert_eq!(secret.expose_secret(), "super-secret-value-12345");
}

#[tokio::test]
async fn env_provider_reads_and_respects_prefix() {
    let key = "DJANGORS_SECRETS_UNIT_TEST_KEY";
    std::env::set_var(key, "env-secret-value");
    let plain = EnvProvider::new();
    assert_eq!(
        plain.get(key).await.unwrap().unwrap().expose_secret(),
        "env-secret-value"
    );

    let prefixed = EnvProvider::with_prefix("DJANGORS_SECRETS_UNIT_");
    assert_eq!(
        prefixed
            .get("TEST_KEY")
            .await
            .unwrap()
            .unwrap()
            .expose_secret(),
        "env-secret-value"
    );
    assert!(prefixed.get("NONEXISTENT_XYZ_999").await.unwrap().is_none());
    std::env::remove_var(key);
}

#[tokio::test]
async fn dotenv_provider_reads_file_without_mutating_env() {
    let dir = std::env::temp_dir();
    let path = dir.join(format!("djangors_secrets_test_{}.env", std::process::id()));
    std::fs::write(&path, "DB_PASS=s3cr3t\nAPI_KEY=\"quoted-val\"\n").unwrap();

    let provider = DotenvProvider::new(&path);
    assert_eq!(
        provider
            .get("DB_PASS")
            .await
            .unwrap()
            .unwrap()
            .expose_secret(),
        "s3cr3t"
    );
    assert_eq!(
        provider
            .get("API_KEY")
            .await
            .unwrap()
            .unwrap()
            .expose_secret(),
        "quoted-val"
    );
    assert!(provider.get("MISSING").await.unwrap().is_none());
    assert!(std::env::var("DB_PASS").is_err());

    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn cached_secrets_provider_caches_within_ttl() {
    let inner = Arc::new(MapProvider::new(&[("K", "V")]));
    let cached = CachedSecretsProvider::new(inner.clone(), Duration::from_secs(60));

    assert_eq!(cached.get("K").await.unwrap().unwrap().expose_secret(), "V");
    assert_eq!(cached.get("K").await.unwrap().unwrap().expose_secret(), "V");
    assert_eq!(inner.calls.load(Ordering::SeqCst), 1);

    cached.invalidate("K").await;
    assert_eq!(cached.get("K").await.unwrap().unwrap().expose_secret(), "V");
    assert_eq!(inner.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn resolve_placeholders_substitutes_and_fails_closed() {
    let provider = MapProvider::new(&[("A", "alpha"), ("B", "beta")]);
    let resolved = resolve_placeholders("key=${secret:A}:${secret:B}", &provider)
        .await
        .unwrap();
    assert_eq!(resolved, "key=alpha:beta");

    assert!(matches!(
        resolve_placeholders("${secret:MISSING}", &provider).await,
        Err(SecretsError::NotFound(_))
    ));
    assert!(matches!(
        resolve_placeholders("${secret:UNCLOSED", &provider).await,
        Err(SecretsError::Interpolation { .. })
    ));
    assert!(matches!(
        resolve_placeholders("${secret:}", &provider).await,
        Err(SecretsError::Interpolation { .. })
    ));
}

#[cfg(feature = "secrets-remote")]
#[test]
fn aws_sigv4_matches_official_test_vector() {
    // Official AWS Signature Version 4 test suite (`get-vanilla`):
    // https://docs.aws.amazon.com/IAM/latest/UserGuide/signing-elements.html
    let auth = sign_aws_sigv4(
        "GET",
        "/",
        "",
        &[
            ("host", "example.amazonaws.com"),
            ("x-amz-date", "20150830T123600Z"),
        ],
        b"",
        "20150830T123600Z",
        "20150830",
        "us-east-1",
        "service",
        "AKIDEXAMPLE",
        "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
    );
    assert_eq!(
        auth,
        "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/service/aws4_request, SignedHeaders=host;x-amz-date, Signature=5fa00fa31553b73ebf1942676e86291e8372ff2a2260956d9b8aae1d763fbf31"
    );
}

#[cfg(feature = "secrets-remote")]
#[test]
fn format_amz_timestamps_matches_known_epoch() {
    // 2015-08-30T12:36:00Z = 1440938160
    let (full, date) = format_amz_timestamps(1_440_938_160);
    assert_eq!(full, "20150830T123600Z");
    assert_eq!(date, "20150830");
}

/// Starts a single-use or multi-request local HTTP mock server returning `(status, json_body)`.
#[cfg(feature = "secrets-remote")]
async fn spawn_mock_http(responses: Vec<(u16, String)>) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        for (status, body) in responses {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let _ = sock.read(&mut buf).await;
            let resp = format!(
                "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = sock.write_all(resp.as_bytes()).await;
        }
    });
    (format!("http://{addr}"), handle)
}

#[cfg(feature = "secrets-remote")]
#[tokio::test]
async fn vault_provider_reads_kv_v2_and_gcp_provider_reads_secret() {
    let (vault_url, vh) = spawn_mock_http(vec![(
        200,
        r#"{"data":{"data":{"api_key":"vault-secret-xyz"}}}"#.to_string(),
    )])
    .await;
    let vault = VaultProvider::with_token(&vault_url, "hvs.test-token").unwrap();
    let sec = vault.get("app/prod#api_key").await.unwrap().unwrap();
    assert_eq!(sec.expose_secret(), "vault-secret-xyz");
    vh.await.unwrap();

    // Base64 of "gcp-secret-123" is "Z2NwLXNlY3JldC0xMjM="
    let (gcp_url, gh) = spawn_mock_http(vec![(
        200,
        r#"{"payload":{"data":"Z2NwLXNlY3JldC0xMjM="}}"#.to_string(),
    )])
    .await;
    let gcp = GcpSecretManagerProvider::new("my-proj")
        .unwrap()
        .endpoint(&gcp_url)
        .with_token("ya29.test-token");
    let sec = gcp.get("db-password").await.unwrap().unwrap();
    assert_eq!(sec.expose_secret(), "gcp-secret-123");
    gh.await.unwrap();
}

#[tokio::test]
async fn settings_interpolates_secret_placeholders() {
    let provider = MapProvider::new(&[("SECRET_KEY", "resolved-secret-key-0123456789")]);
    let toml = r#"
debug = false
secret_key = "${secret:SECRET_KEY}"
"#;
    let settings = crate::settings::DjangorsSettings::from_toml_str_with_secrets(toml, &provider)
        .await
        .unwrap();
    assert_eq!(settings.secret_key, "resolved-secret-key-0123456789");
    assert!(settings.validate().is_ok());

    // Unresolved placeholders must fail validate().
    let raw = crate::settings::DjangorsSettings::from_toml_str(toml).unwrap();
    assert!(raw.validate().is_err());
}
