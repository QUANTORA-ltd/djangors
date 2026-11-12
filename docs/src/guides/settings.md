# Typed Settings (`#[derive(Settings)]`)

`#[derive(djangors_macros::Settings)]` is the Djangors equivalent of `pydantic-settings`/
`django-environ`: a typed, validated way to load your *own application's* configuration from
environment variables, without hand-writing `std::env::var(...)` plus manual parsing and error
handling for every field.

This is distinct from `DjangorsSettings` (the framework's own fixed settings struct for things
like `SECRET_KEY`/`ALLOWED_HOSTS`/`DEBUG`). `#[derive(Settings)]` is for config your application
defines itself: a third-party API key, a feature flag, a timeout, anything your own code needs
at startup.

## Basic usage

```rust,compile
#[derive(djangors_macros::Settings, Debug)]
#[djangors(prefix = "MYAPP")]
struct AppSettings {
    // Required - missing MYAPP_API_KEY at startup is a load() error, not a panic later.
    api_key: String,

    // Has a default - falls back to this value if MYAPP_BASE_URL isn't set.
    #[djangors(default = "https://api.example.com".to_string())]
    base_url: String,

    #[djangors(default = 30)]
    timeout_secs: u64,

    // Option<T> fields are never required - None if the env var is unset.
    feature_flag: Option<bool>,

    // Vec<String> fields parse as a comma-separated list.
    allowed_origins: Option<Vec<String>>,
}

fn main() {
    match AppSettings::load() {
        Ok(settings) => {
            let _ = settings.api_key;
        }
        Err(e) => {
            eprintln!("config error: {e}");
        }
    }
}
```

## Environment variable naming

Each field maps to `{PREFIX}_{FIELD_NAME_UPPERCASE}`. With `#[djangors(prefix = "MYAPP")]`:

| Field | Environment variable |
|---|---|
| `api_key` | `MYAPP_API_KEY` |
| `base_url` | `MYAPP_BASE_URL` |
| `timeout_secs` | `MYAPP_TIMEOUT_SECS` |

## Supported field types

`String`, `bool`, every built-in integer and float type (`i32`, `i64`, `u32`, `u64`, `f32`, `f64`,
etc.), and `Vec<String>` (comma-separated: `MYAPP_ALLOWED_ORIGINS=a.com,b.com,c.com`). Wrap any of
these in `Option<T>` to make the field optional instead of required.

### Extending with `FromSettingsValue`

All the above types work because they implement `djangors_core::settings::FromSettingsValue`
(one method, `parse_settings_value(raw: &str) -> Result<Self, String>`). Implement it for your own
newtype to get a parsed, validated field for free:

```rust,compile
use djangors_core::settings::FromSettingsValue;

struct Port(u16);

impl FromSettingsValue for Port {
    fn parse_settings_value(raw: &str) -> Result<Self, String> {
        let n = raw.parse::<u16>().map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("port must be non-zero".to_string());
        }
        Ok(Port(n))
    }
}
```

Your type can then appear directly in a `#[derive(Settings)]` struct — with a
`#[djangors(default = ...)]` fallback or `Option<T>` optionality, exactly like
the built-in types.

## Errors

`load()` returns `Result<Self, djangors_core::settings::SettingsError>` with two distinguishable
failure variants:

```rust,compile
# fn show() {
use djangors_core::settings::SettingsError;

let err = SettingsError::MissingRequired {
    field: "api_key",
    env_var: "MYAPP_API_KEY".to_string(),
};
match err {
    SettingsError::MissingRequired { field, env_var } => {
        eprintln!("required setting `{field}` (${env_var}) was not set");
    }
    SettingsError::InvalidValue { field, env_var, .. } => {
        eprintln!("setting `{field}` (${env_var}) had a value that couldn't be parsed");
    }
}
# }
```

- **`MissingRequired`**: a field with no `#[djangors(default = ...)]` and not wrapped in
  `Option<T>` had no environment variable set at all.
- **`InvalidValue`**: the environment variable was set, but its value couldn't be parsed into the
  field's declared type (e.g. `MYAPP_TIMEOUT_SECS=not-a-number`).

Call `load()` once at process startup (in `main()`, before the server starts accepting
connections) and treat any `Err` as a hard failure to boot. This is the same "fail fast on bad
config" principle `dj check --deploy` already applies to the framework's own settings.

## Framework settings (`DjangorsSettings`)

`DjangorsSettings` holds the framework's own settings. Each one can be overridden with an
environment variable when the settings are loaded with `DjangorsSettings::from_env()` or
`DjangorsSettings::load()`. A malformed value is reported as a warning and the default is kept.

| Setting | Environment variable | Default |
|---|---|---|
| `debug` | `DJANGORS_DEBUG` | `true` |
| `allowed_hosts` | `DJANGORS_ALLOWED_HOSTS` | `127.0.0.1,localhost` |
| `secret_key` | `DJANGORS_SECRET_KEY` | empty |
| `host` | `DJANGORS_HOST` | `127.0.0.1` |
| `port` | `DJANGORS_PORT` | `8000` |
| `max_body_bytes` | `DJANGORS_MAX_BODY_BYTES` | `10485760` (10 MiB) |
| `cors_allowed_origins` | `DJANGORS_CORS_ALLOWED_ORIGINS` | empty |

`max_body_bytes` is the largest request body the router will read. See
[Request size limit](http-and-middleware.md#request-size-limit).
