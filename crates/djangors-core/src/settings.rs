//! Settings and configuration system for Djangors.
//! Mirrors Django's settings approach with environment variable overlays
//! and file-based TOML configuration.

use crate::error::DjangorsError;
use serde::Deserialize;
use std::fmt;

/// Error type produced by `#[derive(Settings)]`-generated `load()` methods.
///
/// Distinguishes a required field with no matching environment variable from a
/// field whose environment variable was present but couldn't be parsed into the
/// field's type, so callers (and error messages) can tell the two apart.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SettingsError {
    /// A field had no `#[djangors(default = ...)]` and wasn't `Option<T>`, but its
    /// environment variable was not set.
    #[error(
        "missing required setting: environment variable `{env_var}` is not set (field `{field}`)"
    )]
    MissingRequired {
        /// The struct field name.
        field: &'static str,
        /// The environment variable name that was checked.
        env_var: String,
    },
    /// A field's environment variable was set but could not be parsed into the
    /// field's declared type.
    #[error("invalid value for setting `{field}` (env var `{env_var}`): {message}")]
    InvalidValue {
        /// The struct field name.
        field: &'static str,
        /// The environment variable name that was checked.
        env_var: String,
        /// A human-readable description of why parsing failed.
        message: String,
    },
}

/// Implemented for every type a `#[derive(Settings)]` field can hold, so the derive
/// macro can generate a single generic `std::env::var(...)` + parse call per field
/// regardless of its concrete type.
///
/// Djangors implements this for `String`, `bool`, every built-in integer and float
/// type, and `Vec<String>` (parsed as a comma-separated list, matching
/// `DjangorsSettings`'s own `DJANGORS_ALLOWED_HOSTS` convention). `Option<T>` fields
/// are handled directly by the derive macro (absent env var -> `None`) rather than
/// through this trait.
pub trait FromSettingsValue: Sized {
    /// Parses a raw environment variable string into `Self`.
    fn parse_settings_value(raw: &str) -> Result<Self, String>;
}

impl FromSettingsValue for String {
    fn parse_settings_value(raw: &str) -> Result<Self, String> {
        Ok(raw.to_string())
    }
}

impl FromSettingsValue for bool {
    fn parse_settings_value(raw: &str) -> Result<Self, String> {
        match raw.to_lowercase().as_str() {
            "true" | "1" => Ok(true),
            "false" | "0" => Ok(false),
            other => Err(format!(
                "expected a boolean (`true`/`false`/`1`/`0`), got `{other}`"
            )),
        }
    }
}

impl FromSettingsValue for Vec<String> {
    fn parse_settings_value(raw: &str) -> Result<Self, String> {
        Ok(raw
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect())
    }
}

macro_rules! impl_from_settings_value_via_parse {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl FromSettingsValue for $ty {
                fn parse_settings_value(raw: &str) -> Result<Self, String> {
                    raw.parse::<$ty>().map_err(|e| e.to_string())
                }
            }
        )+
    };
}

impl_from_settings_value_via_parse!(
    i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize, f32, f64
);

/// Settings structure holding configuration parameters for the Djangors framework.
/// Mirrors Django's `settings.py` structure.
#[derive(Clone)]
pub struct DjangorsSettings {
    /// Mirrors Django's `DEBUG` setting. If true, detailed error reports are shown.
    /// Never set to true in production.
    pub debug: bool,

    /// Mirrors Django's `ALLOWED_HOSTS` setting. A list of host/domain names that
    /// this site can serve.
    pub allowed_hosts: Vec<String>,

    /// Mirrors Django's `SECRET_KEY` setting. A secret key used for cryptographic signing.
    /// Must be kept confidential and never be empty in production.
    pub secret_key: String,

    /// Bind address host name or IP address (e.g., "127.0.0.1").
    pub host: String,

    /// Port number to listen on (e.g., 8000).
    pub port: u16,
}

impl fmt::Debug for DjangorsSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DjangorsSettings")
            .field("debug", &self.debug)
            .field("allowed_hosts", &self.allowed_hosts)
            .field("secret_key", &"[redacted]")
            .field("host", &self.host)
            .field("port", &self.port)
            .finish()
    }
}

impl Default for DjangorsSettings {
    fn default() -> Self {
        Self {
            debug: true,
            allowed_hosts: vec!["127.0.0.1".into(), "localhost".into()],
            secret_key: String::new(),
            host: "127.0.0.1".into(),
            port: 8000,
        }
    }
}

#[derive(Deserialize)]
struct PartialSettings {
    debug: Option<bool>,
    allowed_hosts: Option<Vec<String>>,
    secret_key: Option<String>,
    host: Option<String>,
    port: Option<u16>,
}

impl DjangorsSettings {
    /// Overlays environment variables onto the settings struct.
    /// Returns a list of warning messages for any malformed environment variables.
    fn merge_env(&mut self) -> Vec<String> {
        let mut warnings = Vec::new();

        if let Ok(val) = std::env::var("DJANGORS_DEBUG") {
            match val.to_lowercase().as_str() {
                "true" | "1" => self.debug = true,
                "false" | "0" => self.debug = false,
                _ => warnings.push(format!("Ignored invalid env var DJANGORS_DEBUG: '{}'", val)),
            }
        }

        if let Ok(val) = std::env::var("DJANGORS_ALLOWED_HOSTS") {
            self.allowed_hosts = val
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }

        if let Ok(val) = std::env::var("DJANGORS_SECRET_KEY") {
            self.secret_key = val;
        }

        if let Ok(val) = std::env::var("DJANGORS_HOST") {
            self.host = val;
        }

        if let Ok(val) = std::env::var("DJANGORS_PORT") {
            match val.parse::<u16>() {
                Ok(port) => self.port = port,
                Err(_) => {
                    warnings.push(format!("Ignored invalid env var DJANGORS_PORT: '{}'", val))
                }
            }
        }

        warnings
    }

    /// Loads settings entirely from the environment variables, starting from defaults.
    /// Returns the constructed settings along with warning messages for malformed environment variables.
    pub fn from_env() -> (Self, Vec<String>) {
        let mut settings = Self::default();
        let warnings = settings.merge_env();
        (settings, warnings)
    }

    /// Parses a TOML configuration string, overlaying specified fields onto defaults.
    pub fn from_toml_str(s: &str) -> Result<Self, DjangorsError> {
        let partial: PartialSettings = toml::from_str(s).map_err(|e| {
            DjangorsError::Internal(format!("Failed to parse TOML settings: {}", e))
        })?;

        let mut settings = Self::default();
        if let Some(debug) = partial.debug {
            settings.debug = debug;
        }
        if let Some(allowed_hosts) = partial.allowed_hosts {
            settings.allowed_hosts = allowed_hosts;
        }
        if let Some(secret_key) = partial.secret_key {
            settings.secret_key = secret_key;
        }
        if let Some(host) = partial.host {
            settings.host = host;
        }
        if let Some(port) = partial.port {
            settings.port = port;
        }

        Ok(settings)
    }

    /// The standard entry point for loading framework settings.
    /// Starts from defaults, overlays values from `djangors.toml` if it exists in the current working directory,
    /// and finally overlays any environment variables.
    /// Returns the loaded settings and warning messages.
    pub fn load() -> Result<(Self, Vec<String>), DjangorsError> {
        let mut settings = if std::path::Path::new("djangors.toml").exists() {
            let content = std::fs::read_to_string("djangors.toml").map_err(|e| {
                DjangorsError::Internal(format!("Failed to read djangors.toml: {}", e))
            })?;
            Self::from_toml_str(&content)?
        } else {
            Self::default()
        };

        let warnings = settings.merge_env();
        Ok((settings, warnings))
    }

    /// Validates the loaded settings structure.
    /// Returns an error if the configuration is invalid or unsafe for startup.
    pub fn validate(&self) -> Result<(), DjangorsError> {
        if !self.debug {
            let weak_key = self.secret_key.len() < 32
                || self.secret_key.to_ascii_lowercase().contains("insecure")
                || self.secret_key.to_ascii_lowercase().contains("dev");
            if weak_key {
                return Err(DjangorsError::Internal(
                    "SECRET_KEY must be at least 32 bytes and must not contain 'dev' or 'insecure' when DEBUG is false".into(),
                ));
            }
            if self.allowed_hosts.is_empty() {
                return Err(DjangorsError::Internal(
                    "ALLOWED_HOSTS must contain at least one host when DEBUG is false".into(),
                ));
            }
        }
        if self.port == 0 {
            return Err(DjangorsError::Internal("Port cannot be 0".into()));
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "settings_tests.rs"]
mod tests;
