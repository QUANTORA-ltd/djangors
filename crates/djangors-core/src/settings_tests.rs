use super::*;
use std::sync::Mutex;

// Mutex to ensure environment variable tests do not run concurrently.
static ENV_MUTEX: Mutex<()> = Mutex::new(());

struct EnvGuard<'a> {
    _lock: std::sync::MutexGuard<'a, ()>,
    vars: Vec<&'static str>,
}

impl<'a> EnvGuard<'a> {
    fn new(vars: Vec<&'static str>) -> Self {
        let lock = ENV_MUTEX.lock().unwrap();
        for var in &vars {
            std::env::remove_var(var);
        }
        Self { _lock: lock, vars }
    }
}

impl<'a> Drop for EnvGuard<'a> {
    fn drop(&mut self) {
        for var in &self.vars {
            std::env::remove_var(var);
        }
    }
}

#[test]
fn test_defaults() {
    let settings = DjangorsSettings::default();
    assert!(settings.debug);
    assert_eq!(
        settings.allowed_hosts,
        vec!["127.0.0.1".to_string(), "localhost".to_string()]
    );
    assert_eq!(settings.secret_key, "");
    assert_eq!(settings.host, "127.0.0.1");
    assert_eq!(settings.port, 8000);
}

#[test]
fn settings_builder_sets_public_configuration() {
    let settings = DjangorsSettings::new()
        .with_debug(false)
        .with_allowed_hosts(["example.com", "api.example.com"])
        .with_secret_key("a-long-production-secret-key-value")
        .with_host("0.0.0.0")
        .with_port(9000);
    assert!(!settings.debug);
    assert_eq!(settings.allowed_hosts, ["example.com", "api.example.com"]);
    assert_eq!(settings.secret_key, "a-long-production-secret-key-value");
    assert_eq!(settings.host, "0.0.0.0");
    assert_eq!(settings.port, 9000);
}

#[test]
fn test_from_toml_str_partial() {
    let toml_content = r#"
            debug = false
            port = 9000
        "#;
    let settings = DjangorsSettings::from_toml_str(toml_content).unwrap();
    assert!(!settings.debug);
    assert_eq!(settings.port, 9000);
    // Untouched fields should be default
    assert_eq!(
        settings.allowed_hosts,
        vec!["127.0.0.1".to_string(), "localhost".to_string()]
    );
    assert_eq!(settings.secret_key, "");
    assert_eq!(settings.host, "127.0.0.1");
}

#[test]
fn test_from_env_clean() {
    let _guard = EnvGuard::new(vec![
        "DJANGORS_DEBUG",
        "DJANGORS_ALLOWED_HOSTS",
        "DJANGORS_SECRET_KEY",
        "DJANGORS_HOST",
        "DJANGORS_PORT",
    ]);

    std::env::set_var("DJANGORS_DEBUG", "false");
    std::env::set_var("DJANGORS_ALLOWED_HOSTS", "example.com,test.org");
    std::env::set_var("DJANGORS_SECRET_KEY", "super-secret-key-123");
    std::env::set_var("DJANGORS_HOST", "0.0.0.0");
    std::env::set_var("DJANGORS_PORT", "8888");

    let (settings, warnings) = DjangorsSettings::from_env();
    assert!(warnings.is_empty());
    assert!(!settings.debug);
    assert_eq!(
        settings.allowed_hosts,
        vec!["example.com".to_string(), "test.org".to_string()]
    );
    assert_eq!(settings.secret_key, "super-secret-key-123");
    assert_eq!(settings.host, "0.0.0.0");
    assert_eq!(settings.port, 8888);
}

#[test]
fn test_from_env_malformed() {
    let _guard = EnvGuard::new(vec!["DJANGORS_DEBUG", "DJANGORS_PORT"]);

    std::env::set_var("DJANGORS_DEBUG", "not-a-bool");
    std::env::set_var("DJANGORS_PORT", "not-a-port");

    let (settings, warnings) = DjangorsSettings::from_env();
    assert_eq!(warnings.len(), 2);
    assert!(warnings[0].contains("DJANGORS_DEBUG"));
    assert!(warnings[1].contains("DJANGORS_PORT"));

    // Defaults should be preserved
    assert!(settings.debug);
    assert_eq!(settings.port, 8000);
}

#[test]
fn test_validate() {
    let mut settings = DjangorsSettings::default();

    // Dev/debug mode with empty secret key should be valid (this is
    // exactly Default::default()'s state — debug: true, secret_key: "").
    assert!(settings.validate().is_ok());

    // Production mode with empty secret key should fail validation
    settings.debug = false;
    settings.secret_key = "".to_string();
    assert!(settings.validate().is_err());

    // Production mode with non-empty secret key should be valid
    settings.secret_key = "some-key".to_string();
    assert!(settings.validate().is_ok());

    // Port 0 should fail validation
    settings.port = 0;
    assert!(settings.validate().is_err());
}

#[derive(djangors_macros::Settings, Debug, PartialEq)]
#[djangors(prefix = "TESTAPP")]
struct DeriveSettingsFixture {
    api_key: String,
    #[djangors(default = "https://api.example.com".to_string())]
    base_url: String,
    #[djangors(default = 30)]
    timeout_secs: u64,
    feature_flag: Option<bool>,
    allowed_origins: Option<Vec<String>>,
}

#[test]
fn derive_settings_required_field_missing_errors() {
    let _guard = EnvGuard::new(vec![
        "TESTAPP_API_KEY",
        "TESTAPP_BASE_URL",
        "TESTAPP_TIMEOUT_SECS",
        "TESTAPP_FEATURE_FLAG",
        "TESTAPP_ALLOWED_ORIGINS",
    ]);

    let err = DeriveSettingsFixture::load().unwrap_err();
    assert_eq!(
        err,
        SettingsError::MissingRequired {
            field: "api_key",
            env_var: "TESTAPP_API_KEY".to_string(),
        }
    );
}

#[test]
fn derive_settings_applies_defaults_and_parses_option_fields() {
    let _guard = EnvGuard::new(vec![
        "TESTAPP_API_KEY",
        "TESTAPP_BASE_URL",
        "TESTAPP_TIMEOUT_SECS",
        "TESTAPP_FEATURE_FLAG",
        "TESTAPP_ALLOWED_ORIGINS",
    ]);
    std::env::set_var("TESTAPP_API_KEY", "sk_live_123");

    let settings = DeriveSettingsFixture::load().unwrap();
    assert_eq!(settings.api_key, "sk_live_123");
    assert_eq!(settings.base_url, "https://api.example.com");
    assert_eq!(settings.timeout_secs, 30);
    assert_eq!(settings.feature_flag, None);
    assert_eq!(settings.allowed_origins, None);
}

#[test]
fn derive_settings_env_vars_override_defaults_and_parse_every_type() {
    let _guard = EnvGuard::new(vec![
        "TESTAPP_API_KEY",
        "TESTAPP_BASE_URL",
        "TESTAPP_TIMEOUT_SECS",
        "TESTAPP_FEATURE_FLAG",
        "TESTAPP_ALLOWED_ORIGINS",
    ]);
    std::env::set_var("TESTAPP_API_KEY", "sk_live_456");
    std::env::set_var("TESTAPP_BASE_URL", "https://staging.example.com");
    std::env::set_var("TESTAPP_TIMEOUT_SECS", "90");
    std::env::set_var("TESTAPP_FEATURE_FLAG", "true");
    std::env::set_var("TESTAPP_ALLOWED_ORIGINS", "a.com, b.com,c.com");

    let settings = DeriveSettingsFixture::load().unwrap();
    assert_eq!(settings.api_key, "sk_live_456");
    assert_eq!(settings.base_url, "https://staging.example.com");
    assert_eq!(settings.timeout_secs, 90);
    assert_eq!(settings.feature_flag, Some(true));
    assert_eq!(
        settings.allowed_origins,
        Some(vec![
            "a.com".to_string(),
            "b.com".to_string(),
            "c.com".to_string()
        ])
    );
}

#[test]
fn derive_settings_invalid_value_is_a_distinct_error_from_missing() {
    let _guard = EnvGuard::new(vec![
        "TESTAPP_API_KEY",
        "TESTAPP_BASE_URL",
        "TESTAPP_TIMEOUT_SECS",
        "TESTAPP_FEATURE_FLAG",
        "TESTAPP_ALLOWED_ORIGINS",
    ]);
    std::env::set_var("TESTAPP_API_KEY", "sk_live_789");
    std::env::set_var("TESTAPP_TIMEOUT_SECS", "not-a-number");

    let err = DeriveSettingsFixture::load().unwrap_err();
    match err {
        SettingsError::InvalidValue { field, env_var, .. } => {
            assert_eq!(field, "timeout_secs");
            assert_eq!(env_var, "TESTAPP_TIMEOUT_SECS");
        }
        other => panic!("expected InvalidValue, got {other:?}"),
    }
}

#[test]
fn test_custom_debug_redacts_secret_key() {
    let settings = DjangorsSettings {
        debug: true,
        allowed_hosts: vec![],
        secret_key: "my-super-secret-unredacted-key".to_string(),
        host: "127.0.0.1".to_string(),
        port: 8000,
        max_body_bytes: 1024,
        cors_allowed_origins: vec![],
        ..Default::default()
    };

    let debug_str = format!("{:?}", settings);
    assert!(!debug_str.contains("my-super-secret-unredacted-key"));
    assert!(debug_str.contains("[redacted]"));
}

#[test]
fn settings_builder_sets_max_body_bytes() {
    let settings = DjangorsSettings::new().with_max_body_bytes(4096);
    assert_eq!(settings.max_body_bytes, 4096);
}

#[test]
fn max_body_bytes_defaults_to_ten_mib_and_appears_in_debug() {
    let settings = DjangorsSettings::default();
    assert_eq!(settings.max_body_bytes, 10 * 1024 * 1024);
    let debug_str = format!("{:?}", settings);
    assert!(debug_str.contains("max_body_bytes"));
}

#[test]
fn test_from_env_max_body_bytes() {
    let _guard = EnvGuard::new(vec!["DJANGORS_MAX_BODY_BYTES"]);

    std::env::set_var("DJANGORS_MAX_BODY_BYTES", "2048");
    let (settings, warnings) = DjangorsSettings::from_env();
    assert!(warnings.is_empty());
    assert_eq!(settings.max_body_bytes, 2048);
}

#[test]
fn test_from_env_invalid_max_body_bytes_keeps_default() {
    let _guard = EnvGuard::new(vec!["DJANGORS_MAX_BODY_BYTES"]);

    std::env::set_var("DJANGORS_MAX_BODY_BYTES", "ten-megabytes");
    let (settings, warnings) = DjangorsSettings::from_env();
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("DJANGORS_MAX_BODY_BYTES"));
    assert_eq!(settings.max_body_bytes, 10 * 1024 * 1024);
}

#[test]
fn toml_sets_max_body_bytes() {
    let settings = DjangorsSettings::from_toml_str("max_body_bytes = 2048\n").unwrap();
    assert_eq!(settings.max_body_bytes, 2048);
    let defaults = DjangorsSettings::from_toml_str("debug = true\n").unwrap();
    assert_eq!(defaults.max_body_bytes, 10 * 1024 * 1024);
}

#[test]
fn test_from_env_cors_allowed_origins() {
    let _guard = EnvGuard::new(vec!["DJANGORS_CORS_ALLOWED_ORIGINS"]);

    std::env::set_var(
        "DJANGORS_CORS_ALLOWED_ORIGINS",
        "https://app.example.com, https://admin.example.com ",
    );
    let (settings, warnings) = DjangorsSettings::from_env();
    assert!(warnings.is_empty());
    assert_eq!(
        settings.cors_allowed_origins,
        vec![
            "https://app.example.com".to_string(),
            "https://admin.example.com".to_string(),
        ]
    );
}

#[test]
fn toml_sets_cors_allowed_origins_and_appears_in_debug() {
    let settings =
        DjangorsSettings::from_toml_str("cors_allowed_origins = [\"https://app.example.com\"]\n")
            .unwrap();
    assert_eq!(
        settings.cors_allowed_origins,
        vec!["https://app.example.com".to_string()]
    );
    let debug_str = format!("{:?}", settings);
    assert!(debug_str.contains("cors_allowed_origins"));
}
