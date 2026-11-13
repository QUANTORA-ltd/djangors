//! Secrets providers and `${secret:NAME}` configuration interpolation.
//!
//! Secret values are wrapped in [`SecretString`] (`secrecy::SecretString`) so they
//! are zeroized on drop and never printed by `Debug`.

use crate::error::DjangorsError;
#[cfg(feature = "secrets-remote")]
use hmac::{Hmac, Mac};
pub use secrecy::{ExposeSecret, SecretString};
#[cfg(feature = "secrets-remote")]
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
#[cfg(feature = "secrets-remote")]
use std::time::{SystemTime, UNIX_EPOCH};
#[cfg(feature = "secrets-remote")]
use tokio::sync::Mutex;
use tokio::sync::RwLock;

#[cfg(feature = "secrets-remote")]
type HmacSha256 = Hmac<Sha256>;

/// Errors returned by [`SecretsProvider`] implementations and placeholder interpolation.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SecretsError {
    /// A referenced secret could not be found during interpolation.
    #[error("secret `{0}` not found")]
    NotFound(String),
    /// Provider configuration is invalid.
    #[error("invalid secrets provider configuration: {0}")]
    InvalidConfig(String),
    /// Reading a local file (such as `.env`) failed.
    #[error("I/O error reading secrets: {0}")]
    Io(#[from] std::io::Error),
    /// Parsing a `.env` file failed.
    #[error("failed to parse dotenv file: {0}")]
    Dotenv(String),
    /// An HTTP request to a remote secrets backend failed.
    #[error("secrets HTTP error: {0}")]
    Http(String),
    /// The remote backend returned an unexpected payload or status.
    #[error("secrets provider error: {0}")]
    Provider(String),
    /// A `${secret:...}` placeholder was malformed or could not be resolved.
    #[error("secret placeholder `{placeholder}` error: {reason}")]
    Interpolation {
        /// The placeholder expression or name.
        placeholder: String,
        /// Why interpolation failed.
        reason: String,
    },
}

impl From<SecretsError> for DjangorsError {
    fn from(err: SecretsError) -> Self {
        DjangorsError::Internal(err.to_string())
    }
}

/// Asynchronous source of secret values, keyed by name.
#[async_trait::async_trait]
pub trait SecretsProvider: Send + Sync {
    /// Looks up the secret named `name`. Returns `Ok(None)` when absent.
    async fn get(&self, name: &str) -> Result<Option<SecretString>, SecretsError>;
}

/// Reads secrets from process environment variables (`std::env::var`).
#[derive(Debug, Clone, Default)]
pub struct EnvProvider {
    prefix: String,
}

impl EnvProvider {
    /// Creates an [`EnvProvider`] that reads variable names verbatim.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an [`EnvProvider`] that prepends `prefix` to every requested name.
    pub fn with_prefix(prefix: impl Into<String>) -> Self {
        Self {
            prefix: prefix.into(),
        }
    }
}

#[async_trait::async_trait]
impl SecretsProvider for EnvProvider {
    async fn get(&self, name: &str) -> Result<Option<SecretString>, SecretsError> {
        let key = if self.prefix.is_empty() {
            name.to_string()
        } else {
            format!("{}{}", self.prefix, name)
        };
        match std::env::var(&key) {
            Ok(val) => Ok(Some(SecretString::from(val))),
            Err(std::env::VarError::NotPresent) => Ok(None),
            Err(std::env::VarError::NotUnicode(_)) => Err(SecretsError::Provider(format!(
                "environment variable `{key}` is not valid UTF-8"
            ))),
        }
    }
}

/// Reads secrets from a `.env` file via `dotenvy` without mutating process environment variables.
#[derive(Debug, Clone)]
pub struct DotenvProvider {
    path: PathBuf,
    optional: bool,
}

impl DotenvProvider {
    /// Creates a [`DotenvProvider`] reading from `path`. Missing files return an error.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            optional: false,
        }
    }

    /// Creates a [`DotenvProvider`] that returns `Ok(None)` when `path` does not exist.
    pub fn optional(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            optional: true,
        }
    }
}

#[async_trait::async_trait]
impl SecretsProvider for DotenvProvider {
    async fn get(&self, name: &str) -> Result<Option<SecretString>, SecretsError> {
        let iter = match dotenvy::from_path_iter(&self.path) {
            Ok(iter) => iter,
            Err(dotenvy::Error::Io(e))
                if self.optional && e.kind() == std::io::ErrorKind::NotFound =>
            {
                return Ok(None);
            }
            Err(dotenvy::Error::Io(e)) => return Err(SecretsError::Io(e)),
            Err(e) => return Err(SecretsError::Dotenv(e.to_string())),
        };
        let mut found = None;
        for item in iter {
            let (k, v) = item.map_err(|e| SecretsError::Dotenv(e.to_string()))?;
            if k == name {
                found = Some(SecretString::from(v));
            }
        }
        Ok(found)
    }
}

/// Tries multiple [`SecretsProvider`]s in order and returns the first `Some` value.
#[derive(Default)]
pub struct ChainSecretsProvider {
    providers: Vec<Arc<dyn SecretsProvider>>,
}

impl std::fmt::Debug for ChainSecretsProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChainSecretsProvider")
            .field("providers_count", &self.providers.len())
            .finish()
    }
}

impl ChainSecretsProvider {
    /// Creates an empty provider chain.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends `provider` to the lookup chain.
    pub fn push(mut self, provider: Arc<dyn SecretsProvider>) -> Self {
        self.providers.push(provider);
        self
    }
}

#[async_trait::async_trait]
impl SecretsProvider for ChainSecretsProvider {
    async fn get(&self, name: &str) -> Result<Option<SecretString>, SecretsError> {
        for provider in &self.providers {
            if let Some(secret) = provider.get(name).await? {
                return Ok(Some(secret));
            }
        }
        Ok(None)
    }
}

struct CachedSecret {
    value: Option<SecretString>,
    fetched_at: Instant,
}

/// Wraps any [`SecretsProvider`] with an in-memory TTL cache.
pub struct CachedSecretsProvider {
    inner: Arc<dyn SecretsProvider>,
    ttl: Duration,
    cache: RwLock<HashMap<String, CachedSecret>>,
}

impl std::fmt::Debug for CachedSecretsProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CachedSecretsProvider")
            .field("ttl", &self.ttl)
            .finish_non_exhaustive()
    }
}

impl CachedSecretsProvider {
    /// Caches lookups from `inner` for `ttl`.
    pub fn new(inner: Arc<dyn SecretsProvider>, ttl: Duration) -> Self {
        Self {
            inner,
            ttl,
            cache: RwLock::new(HashMap::new()),
        }
    }

    /// Removes a single cached entry so the next `get` fetches fresh data.
    pub async fn invalidate(&self, name: &str) {
        self.cache.write().await.remove(name);
    }

    /// Clears all cached entries.
    pub async fn clear(&self) {
        self.cache.write().await.clear();
    }
}

#[async_trait::async_trait]
impl SecretsProvider for CachedSecretsProvider {
    async fn get(&self, name: &str) -> Result<Option<SecretString>, SecretsError> {
        if !self.ttl.is_zero() {
            let guard = self.cache.read().await;
            if let Some(entry) = guard.get(name) {
                if entry.fetched_at.elapsed() < self.ttl {
                    return Ok(entry.value.clone());
                }
            }
        }
        let fresh = self.inner.get(name).await?;
        if !self.ttl.is_zero() {
            let mut guard = self.cache.write().await;
            guard.insert(
                name.to_string(),
                CachedSecret {
                    value: fresh.clone(),
                    fetched_at: Instant::now(),
                },
            );
        }
        Ok(fresh)
    }
}

#[cfg(feature = "secrets-remote")]
fn default_http_client() -> Result<reqwest::Client, SecretsError> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| SecretsError::Http(e.to_string()))
}

/// Authentication method for [`VaultProvider`].
#[cfg(feature = "secrets-remote")]
#[derive(Clone)]
pub enum VaultAuth {
    /// Static or renewable Vault token.
    Token(SecretString),
    /// AppRole login exchanging `(role_id, secret_id)` for a renewable client token.
    AppRole {
        /// Vault AppRole `role_id`.
        role_id: String,
        /// Vault AppRole `secret_id`.
        secret_id: SecretString,
        /// Auth mount path (defaults to `"approle"`).
        mount: String,
    },
}

#[cfg(feature = "secrets-remote")]
impl std::fmt::Debug for VaultAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VaultAuth::Token(_) => f.write_str("VaultAuth::Token([REDACTED])"),
            VaultAuth::AppRole { role_id, mount, .. } => f
                .debug_struct("VaultAuth::AppRole")
                .field("role_id", role_id)
                .field("secret_id", &"[REDACTED]")
                .field("mount", mount)
                .finish(),
        }
    }
}

#[cfg(feature = "secrets-remote")]
#[derive(Default)]
struct VaultState {
    token: Option<SecretString>,
    token_expires_at: Option<Instant>,
    /// Tracks active dynamic secret leases `(lease_id, expires_at)` by secret name.
    leases: HashMap<String, (String, Instant)>,
}

/// HashiCorp Vault KV v2 secrets provider (`GET /v1/{mount}/data/{path}`).
///
/// Supports token authentication and AppRole login, renewing both auth tokens and
/// renewable secret leases before expiry.
///
/// Lookup names accept `"path/to/secret#field"` or `"path/to/secret"` (which reads
/// the `"value"` key in `data.data`).
#[cfg(feature = "secrets-remote")]
pub struct VaultProvider {
    addr: String,
    mount: String,
    namespace: Option<String>,
    auth: VaultAuth,
    client: reqwest::Client,
    state: Mutex<VaultState>,
}

#[cfg(feature = "secrets-remote")]
impl std::fmt::Debug for VaultProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VaultProvider")
            .field("addr", &self.addr)
            .field("mount", &self.mount)
            .field("namespace", &self.namespace)
            .field("auth", &self.auth)
            .finish_non_exhaustive()
    }
}

#[cfg(feature = "secrets-remote")]
impl VaultProvider {
    /// Creates a [`VaultProvider`] authenticated with a Vault token.
    pub fn with_token(
        addr: impl Into<String>,
        token: impl Into<SecretString>,
    ) -> Result<Self, SecretsError> {
        let token = token.into();
        if token.expose_secret().is_empty() {
            return Err(SecretsError::InvalidConfig(
                "Vault token must not be empty".to_string(),
            ));
        }
        Ok(Self {
            addr: addr.into().trim_end_matches('/').to_string(),
            mount: "secret".to_string(),
            namespace: None,
            auth: VaultAuth::Token(token.clone()),
            client: default_http_client()?,
            state: Mutex::new(VaultState {
                token: Some(token),
                token_expires_at: None,
                leases: HashMap::new(),
            }),
        })
    }

    /// Creates a [`VaultProvider`] that logs in via Vault AppRole.
    pub fn with_approle(
        addr: impl Into<String>,
        role_id: impl Into<String>,
        secret_id: impl Into<SecretString>,
    ) -> Result<Self, SecretsError> {
        let role_id = role_id.into();
        let secret_id = secret_id.into();
        if role_id.is_empty() || secret_id.expose_secret().is_empty() {
            return Err(SecretsError::InvalidConfig(
                "Vault AppRole role_id and secret_id must not be empty".to_string(),
            ));
        }
        Ok(Self {
            addr: addr.into().trim_end_matches('/').to_string(),
            mount: "secret".to_string(),
            namespace: None,
            auth: VaultAuth::AppRole {
                role_id,
                secret_id,
                mount: "approle".to_string(),
            },
            client: default_http_client()?,
            state: Mutex::new(VaultState::default()),
        })
    }

    /// Overrides the KV v2 mount name (defaults to `"secret"`).
    pub fn mount(mut self, mount: impl Into<String>) -> Self {
        self.mount = mount.into().trim_matches('/').to_string();
        self
    }

    /// Sets the `X-Vault-Namespace` header for Vault Enterprise namespaces.
    pub fn namespace(mut self, ns: impl Into<String>) -> Self {
        self.namespace = Some(ns.into());
        self
    }

    /// Overrides the AppRole auth mount path (defaults to `"approle"`).
    pub fn approle_mount(mut self, auth_mount: impl Into<String>) -> Self {
        if let VaultAuth::AppRole { mount, .. } = &mut self.auth {
            *mount = auth_mount.into().trim_matches('/').to_string();
        }
        self
    }

    fn apply_ns(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.namespace {
            Some(ns) => req.header("X-Vault-Namespace", ns),
            None => req,
        }
    }

    async fn ensure_token(&self) -> Result<SecretString, SecretsError> {
        let mut state = self.state.lock().await;
        let now = Instant::now();
        let needs_refresh = match (&state.token, state.token_expires_at) {
            (Some(_), Some(exp)) => exp <= now + Duration::from_secs(30),
            (Some(_), None) => false,
            (None, _) => true,
        };

        if !needs_refresh {
            if let Some(tok) = &state.token {
                return Ok(tok.clone());
            }
        }

        // Try renewing an existing token before falling back to AppRole re-login.
        if let Some(existing) = state.token.clone() {
            if let Ok(new_ttl) = self.renew_token_inner(&existing).await {
                if new_ttl > 0 {
                    state.token_expires_at = Some(now + Duration::from_secs(new_ttl));
                    return Ok(existing);
                }
            }
        }

        match &self.auth {
            VaultAuth::Token(tok) => {
                state.token = Some(tok.clone());
                Ok(tok.clone())
            }
            VaultAuth::AppRole {
                role_id,
                secret_id,
                mount,
            } => {
                let url = format!("{}/v1/auth/{mount}/login", self.addr);
                let resp = self
                    .apply_ns(self.client.post(&url))
                    .json(&serde_json::json!({
                        "role_id": role_id,
                        "secret_id": secret_id.expose_secret(),
                    }))
                    .send()
                    .await
                    .map_err(|e| SecretsError::Http(e.to_string()))?;
                if !resp.status().is_success() {
                    return Err(SecretsError::Provider(format!(
                        "Vault AppRole login failed with status {}",
                        resp.status()
                    )));
                }
                let body: serde_json::Value = resp
                    .json()
                    .await
                    .map_err(|e| SecretsError::Provider(e.to_string()))?;
                let auth = body.get("auth").ok_or_else(|| {
                    SecretsError::Provider("Vault AppRole login response missing `auth`".into())
                })?;
                let client_token = auth
                    .get("client_token")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| {
                        SecretsError::Provider(
                            "Vault AppRole login response missing `client_token`".into(),
                        )
                    })?;
                let lease_secs = auth
                    .get("lease_duration")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let token = SecretString::from(client_token.to_string());
                state.token = Some(token.clone());
                state.token_expires_at = if lease_secs > 0 {
                    Some(now + Duration::from_secs(lease_secs))
                } else {
                    None
                };
                Ok(token)
            }
        }
    }

    async fn renew_token_inner(&self, token: &SecretString) -> Result<u64, SecretsError> {
        let url = format!("{}/v1/auth/token/renew-self", self.addr);
        let resp = self
            .apply_ns(self.client.post(&url))
            .header("X-Vault-Token", token.expose_secret())
            .send()
            .await
            .map_err(|e| SecretsError::Http(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(SecretsError::Provider(format!(
                "Vault token renewal failed with status {}",
                resp.status()
            )));
        }
        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| SecretsError::Provider(e.to_string()))?;
        Ok(body
            .get("auth")
            .and_then(|a| a.get("lease_duration"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0))
    }

    /// Renews any tracked secret leases that expire within `within`.
    pub async fn renew_expiring_leases(&self, within: Duration) -> Result<(), SecretsError> {
        let token = self.ensure_token().await?;
        let now = Instant::now();
        let to_renew: Vec<(String, String)> = {
            let state = self.state.lock().await;
            state
                .leases
                .iter()
                .filter(|(_, (_, exp))| *exp <= now + within)
                .map(|(name, (lease_id, _))| (name.clone(), lease_id.clone()))
                .collect()
        };
        for (name, lease_id) in to_renew {
            let url = format!("{}/v1/sys/leases/renew", self.addr);
            let resp = self
                .apply_ns(self.client.put(&url))
                .header("X-Vault-Token", token.expose_secret())
                .json(&serde_json::json!({ "lease_id": lease_id }))
                .send()
                .await
                .map_err(|e| SecretsError::Http(e.to_string()))?;
            if !resp.status().is_success() {
                self.state.lock().await.leases.remove(&name);
                continue;
            }
            let body: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| SecretsError::Provider(e.to_string()))?;
            let new_ttl = body
                .get("lease_duration")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            if new_ttl > 0 {
                self.state.lock().await.leases.insert(
                    name,
                    (lease_id, Instant::now() + Duration::from_secs(new_ttl)),
                );
            }
        }
        Ok(())
    }
}

#[cfg(feature = "secrets-remote")]
#[async_trait::async_trait]
impl SecretsProvider for VaultProvider {
    async fn get(&self, name: &str) -> Result<Option<SecretString>, SecretsError> {
        let (path, field) = match name.split_once('#') {
            Some((p, f)) if !p.is_empty() && !f.is_empty() => (p, f),
            Some(_) => {
                return Err(SecretsError::InvalidConfig(format!(
                    "invalid Vault secret name `{name}`"
                )));
            }
            None => (name, "value"),
        };
        let clean_path = path.trim_start_matches('/');
        if clean_path.is_empty() || clean_path.contains("..") {
            return Err(SecretsError::InvalidConfig(format!(
                "invalid Vault secret path `{path}`"
            )));
        }

        let _ = self.renew_expiring_leases(Duration::from_secs(30)).await;
        let token = self.ensure_token().await?;
        let url = format!("{}/v1/{}/data/{clean_path}", self.addr, self.mount);
        let resp = self
            .apply_ns(self.client.get(&url))
            .header("X-Vault-Token", token.expose_secret())
            .send()
            .await
            .map_err(|e| SecretsError::Http(e.to_string()))?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !resp.status().is_success() {
            return Err(SecretsError::Provider(format!(
                "Vault KV read failed with status {}",
                resp.status()
            )));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| SecretsError::Provider(e.to_string()))?;

        if let (Some(lease_id), Some(ttl)) = (
            body.get("lease_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty()),
            body.get("lease_duration")
                .and_then(|v| v.as_u64())
                .filter(|&d| d > 0),
        ) {
            let renewable = body
                .get("renewable")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if renewable {
                self.state.lock().await.leases.insert(
                    name.to_string(),
                    (
                        lease_id.to_string(),
                        Instant::now() + Duration::from_secs(ttl),
                    ),
                );
            }
        }

        let val = body
            .get("data")
            .and_then(|d| d.get("data"))
            .and_then(|m| m.get(field));
        match val {
            Some(serde_json::Value::String(s)) => Ok(Some(SecretString::from(s.clone()))),
            Some(other) if !other.is_null() => Ok(Some(SecretString::from(other.to_string()))),
            _ => Ok(None),
        }
    }
}

/// Google Cloud Secret Manager REST provider using GCE/Cloud Run metadata-server tokens.
#[cfg(feature = "secrets-remote")]
pub struct GcpSecretManagerProvider {
    project: String,
    endpoint: String,
    metadata_url: String,
    static_token: Option<SecretString>,
    cached_token: Mutex<Option<(SecretString, Instant)>>,
    client: reqwest::Client,
}

#[cfg(feature = "secrets-remote")]
impl std::fmt::Debug for GcpSecretManagerProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GcpSecretManagerProvider")
            .field("project", &self.project)
            .field("endpoint", &self.endpoint)
            .finish_non_exhaustive()
    }
}

#[cfg(feature = "secrets-remote")]
impl GcpSecretManagerProvider {
    /// Creates a GCP Secret Manager provider for `project`.
    pub fn new(project: impl Into<String>) -> Result<Self, SecretsError> {
        let project = project.into();
        if project.is_empty() {
            return Err(SecretsError::InvalidConfig(
                "GCP project must not be empty".to_string(),
            ));
        }
        Ok(Self {
            project,
            endpoint: "https://secretmanager.googleapis.com".to_string(),
            metadata_url: "http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token".to_string(),
            static_token: None,
            cached_token: Mutex::new(None),
            client: default_http_client()?,
        })
    }

    /// Overrides the Secret Manager REST base URL (useful for testing against a mock server).
    pub fn endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = endpoint.into().trim_end_matches('/').to_string();
        self
    }

    /// Overrides the GCE metadata token URL (useful for testing).
    pub fn metadata_url(mut self, url: impl Into<String>) -> Self {
        self.metadata_url = url.into();
        self
    }

    /// Supplies a static OAuth bearer token instead of querying the metadata server.
    pub fn with_token(mut self, token: impl Into<SecretString>) -> Self {
        self.static_token = Some(token.into());
        self
    }

    async fn bearer_token(&self) -> Result<SecretString, SecretsError> {
        if let Some(tok) = &self.static_token {
            return Ok(tok.clone());
        }
        let mut guard = self.cached_token.lock().await;
        if let Some((tok, expires_at)) = &*guard {
            if Instant::now() + Duration::from_secs(60) < *expires_at {
                return Ok(tok.clone());
            }
        }
        let resp = self
            .client
            .get(&self.metadata_url)
            .header("Metadata-Flavor", "Google")
            .send()
            .await
            .map_err(|e| SecretsError::Http(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(SecretsError::Provider(format!(
                "GCP metadata token request failed with status {}",
                resp.status()
            )));
        }
        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| SecretsError::Provider(e.to_string()))?;
        let access_token = body
            .get("access_token")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                SecretsError::Provider("GCP metadata response missing `access_token`".into())
            })?;
        let expires_in = body
            .get("expires_in")
            .and_then(|v| v.as_u64())
            .unwrap_or(3600);
        let tok = SecretString::from(access_token.to_string());
        *guard = Some((
            tok.clone(),
            Instant::now() + Duration::from_secs(expires_in),
        ));
        Ok(tok)
    }
}

#[cfg(feature = "secrets-remote")]
#[async_trait::async_trait]
impl SecretsProvider for GcpSecretManagerProvider {
    async fn get(&self, name: &str) -> Result<Option<SecretString>, SecretsError> {
        if name.is_empty() || name.contains('/') || name.contains("..") {
            return Err(SecretsError::InvalidConfig(format!(
                "invalid GCP secret name `{name}`"
            )));
        }
        let token = self.bearer_token().await?;
        let url = format!(
            "{}/v1/projects/{}/secrets/{}/versions/latest:access",
            self.endpoint, self.project, name
        );
        let resp = self
            .client
            .get(&url)
            .bearer_auth(token.expose_secret())
            .send()
            .await
            .map_err(|e| SecretsError::Http(e.to_string()))?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !resp.status().is_success() {
            return Err(SecretsError::Provider(format!(
                "GCP Secret Manager failed with status {}",
                resp.status()
            )));
        }
        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| SecretsError::Provider(e.to_string()))?;
        let Some(b64) = body
            .get("payload")
            .and_then(|p| p.get("data"))
            .and_then(|d| d.as_str())
        else {
            return Ok(None);
        };
        use base64::Engine as _;
        let raw = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .map_err(|e| SecretsError::Provider(format!("invalid base64 payload: {e}")))?;
        let text = String::from_utf8(raw)
            .map_err(|_| SecretsError::Provider("GCP secret payload is not valid UTF-8".into()))?;
        Ok(Some(SecretString::from(text)))
    }
}

/// AWS Secrets Manager provider using pure-Rust SigV4 signing over `reqwest`.
#[cfg(feature = "secrets-remote")]
pub struct AwsSecretsManagerProvider {
    region: String,
    access_key_id: String,
    secret_access_key: SecretString,
    session_token: Option<SecretString>,
    endpoint: String,
    client: reqwest::Client,
}

#[cfg(feature = "secrets-remote")]
impl std::fmt::Debug for AwsSecretsManagerProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AwsSecretsManagerProvider")
            .field("region", &self.region)
            .field("endpoint", &self.endpoint)
            .field("access_key_id", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

#[cfg(feature = "secrets-remote")]
impl AwsSecretsManagerProvider {
    /// Creates an [`AwsSecretsManagerProvider`] for `region` using explicit credentials.
    pub fn new(
        region: impl Into<String>,
        access_key_id: impl Into<String>,
        secret_access_key: impl Into<SecretString>,
    ) -> Result<Self, SecretsError> {
        let region = region.into();
        let access_key_id = access_key_id.into();
        let secret_access_key = secret_access_key.into();
        if region.is_empty()
            || access_key_id.is_empty()
            || secret_access_key.expose_secret().is_empty()
        {
            return Err(SecretsError::InvalidConfig(
                "AWS region, access_key_id, and secret_access_key must not be empty".to_string(),
            ));
        }
        let endpoint = format!("https://secretsmanager.{region}.amazonaws.com");
        Ok(Self {
            region,
            access_key_id,
            secret_access_key,
            session_token: None,
            endpoint,
            client: default_http_client()?,
        })
    }

    /// Attaches an AWS STS session token (`X-Amz-Security-Token`).
    pub fn with_session_token(mut self, token: impl Into<SecretString>) -> Self {
        self.session_token = Some(token.into());
        self
    }

    /// Overrides the Secrets Manager endpoint URL (useful for LocalStack or mock tests).
    pub fn endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = endpoint.into().trim_end_matches('/').to_string();
        self
    }
}

#[cfg(feature = "secrets-remote")]
fn hex_lower(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(feature = "secrets-remote")]
fn sha256_hex(bytes: &[u8]) -> String {
    hex_lower(&Sha256::digest(bytes))
}

#[cfg(feature = "secrets-remote")]
fn hmac_sha256_raw(key: &[u8], msg: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(msg);
    mac.finalize().into_bytes().to_vec()
}

/// Formats `unix_secs` as `(YYYYMMDDTHHMMSSZ, YYYYMMDD)` in UTC.
#[cfg(feature = "secrets-remote")]
pub(crate) fn format_amz_timestamps(unix_secs: u64) -> (String, String) {
    let days = (unix_secs / 86_400) as i64;
    let secs_of_day = unix_secs % 86_400;
    let hour = secs_of_day / 3_600;
    let minute = (secs_of_day % 3_600) / 60;
    let second = secs_of_day % 60;

    // Howard Hinnant's civil-from-days algorithm (UTC, proleptic Gregorian).
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = y + if m <= 2 { 1 } else { 0 };

    let date = format!("{year:04}{m:02}{d:02}");
    let full = format!("{date}T{hour:02}{minute:02}{second:02}Z");
    (full, date)
}

/// Computes an AWS Signature Version 4 `Authorization` header value.
///
/// `headers` must be lowercase `(name, trimmed_value)` pairs sorted by `name`.
#[cfg(feature = "secrets-remote")]
#[allow(clippy::too_many_arguments)]
pub(crate) fn sign_aws_sigv4(
    method: &str,
    canonical_uri: &str,
    canonical_query: &str,
    headers: &[(&str, &str)],
    payload: &[u8],
    amz_date: &str,
    date_stamp: &str,
    region: &str,
    service: &str,
    access_key_id: &str,
    secret_access_key: &str,
) -> String {
    let mut sorted = headers.to_vec();
    sorted.sort_by(|a, b| a.0.cmp(b.0));

    let mut canonical_headers = String::new();
    let mut signed_headers = String::new();
    for (idx, (name, value)) in sorted.iter().enumerate() {
        canonical_headers.push_str(name);
        canonical_headers.push(':');
        canonical_headers.push_str(value.trim());
        canonical_headers.push('\n');
        if idx > 0 {
            signed_headers.push(';');
        }
        signed_headers.push_str(name);
    }

    let payload_hash = sha256_hex(payload);
    let canonical_request = format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        method, canonical_uri, canonical_query, canonical_headers, signed_headers, payload_hash
    );

    let scope = format!("{date_stamp}/{region}/{service}/aws4_request");
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
        sha256_hex(canonical_request.as_bytes())
    );

    let k_secret = format!("AWS4{secret_access_key}");
    let k_date = hmac_sha256_raw(k_secret.as_bytes(), date_stamp.as_bytes());
    let k_region = hmac_sha256_raw(&k_date, region.as_bytes());
    let k_service = hmac_sha256_raw(&k_region, service.as_bytes());
    let k_signing = hmac_sha256_raw(&k_service, b"aws4_request");
    let signature = hex_lower(&hmac_sha256_raw(&k_signing, string_to_sign.as_bytes()));

    format!(
        "AWS4-HMAC-SHA256 Credential={access_key_id}/{scope}, SignedHeaders={signed_headers}, Signature={signature}"
    )
}

#[cfg(feature = "secrets-remote")]
fn host_header_from_url(url: &str) -> Result<String, SecretsError> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .ok_or_else(|| SecretsError::InvalidConfig(format!("invalid URL `{url}`")))?;
    let host_port = rest.split('/').next().unwrap_or("");
    if host_port.is_empty() {
        return Err(SecretsError::InvalidConfig(format!(
            "missing host in `{url}`"
        )));
    }
    Ok(host_port.to_string())
}

#[cfg(feature = "secrets-remote")]
#[async_trait::async_trait]
impl SecretsProvider for AwsSecretsManagerProvider {
    async fn get(&self, name: &str) -> Result<Option<SecretString>, SecretsError> {
        if name.is_empty() {
            return Err(SecretsError::InvalidConfig(
                "AWS secret name must not be empty".to_string(),
            ));
        }
        let body = serde_json::to_vec(&serde_json::json!({ "SecretId": name }))
            .map_err(|e| SecretsError::Provider(e.to_string()))?;
        let unix_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| SecretsError::Provider("system clock unavailable".to_string()))?
            .as_secs();
        let (amz_date, date_stamp) = format_amz_timestamps(unix_secs);
        let host = host_header_from_url(&self.endpoint)?;
        let target = "secretsmanager.GetSecretValue";
        let content_type = "application/x-amz-json-1.1";

        let mut sig_headers = vec![
            ("content-type", content_type),
            ("host", host.as_str()),
            ("x-amz-date", amz_date.as_str()),
            ("x-amz-target", target),
        ];
        if let Some(tok) = &self.session_token {
            sig_headers.push(("x-amz-security-token", tok.expose_secret()));
        }

        let auth_header = sign_aws_sigv4(
            "POST",
            "/",
            "",
            &sig_headers,
            &body,
            &amz_date,
            &date_stamp,
            &self.region,
            "secretsmanager",
            &self.access_key_id,
            self.secret_access_key.expose_secret(),
        );

        let url = format!("{}/", self.endpoint);
        let mut req = self
            .client
            .post(&url)
            .header("Content-Type", content_type)
            .header("X-Amz-Date", &amz_date)
            .header("X-Amz-Target", target)
            .header("Authorization", auth_header)
            .body(body);
        if let Some(tok) = &self.session_token {
            req = req.header("X-Amz-Security-Token", tok.expose_secret());
        }

        let resp = req
            .send()
            .await
            .map_err(|e| SecretsError::Http(e.to_string()))?;
        let status = resp.status();
        let payload: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| SecretsError::Provider(e.to_string()))?;
        if !status.is_success() {
            let err_type = payload.get("__type").and_then(|v| v.as_str()).unwrap_or("");
            if err_type.ends_with("ResourceNotFoundException") {
                return Ok(None);
            }
            return Err(SecretsError::Provider(format!(
                "AWS Secrets Manager failed with status {status}"
            )));
        }
        if let Some(secret_str) = payload.get("SecretString").and_then(|v| v.as_str()) {
            return Ok(Some(SecretString::from(secret_str.to_string())));
        }
        if let Some(b64) = payload.get("SecretBinary").and_then(|v| v.as_str()) {
            use base64::Engine as _;
            let raw = base64::engine::general_purpose::STANDARD
                .decode(b64)
                .map_err(|e| SecretsError::Provider(format!("invalid SecretBinary base64: {e}")))?;
            let text = String::from_utf8(raw).map_err(|_| {
                SecretsError::Provider("AWS SecretBinary is not valid UTF-8".into())
            })?;
            return Ok(Some(SecretString::from(text)));
        }
        Ok(None)
    }
}

/// Resolves all `${secret:NAME}` placeholders in `input` using `provider`.
///
/// Fails closed if a placeholder is malformed or if `provider` returns `None`.
pub async fn resolve_placeholders(
    input: &str,
    provider: &dyn SecretsProvider,
) -> Result<String, SecretsError> {
    const PREFIX: &str = "${secret:";
    if !input.contains(PREFIX) {
        return Ok(input.to_string());
    }
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find(PREFIX) {
        out.push_str(&rest[..start]);
        let after_prefix = &rest[start + PREFIX.len()..];
        let Some(end) = after_prefix.find('}') else {
            return Err(SecretsError::Interpolation {
                placeholder: rest[start..].to_string(),
                reason: "unclosed `${secret:...}` placeholder".to_string(),
            });
        };
        let name = after_prefix[..end].trim();
        if name.is_empty() {
            return Err(SecretsError::Interpolation {
                placeholder: "${secret:}".to_string(),
                reason: "secret name must not be empty".to_string(),
            });
        }
        let secret = provider
            .get(name)
            .await?
            .ok_or_else(|| SecretsError::NotFound(name.to_string()))?;
        out.push_str(secret.expose_secret());
        rest = &after_prefix[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

#[cfg(test)]
#[path = "secrets_tests.rs"]
mod secrets_tests;
