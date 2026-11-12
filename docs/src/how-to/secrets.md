# Secrets providers

`djangors_core::secrets` (enabled by the default `secrets` feature) loads secrets from environment variables, `.env` files, HashiCorp Vault, GCP Secret Manager, and AWS Secrets Manager, and interpolates `${secret:NAME}` placeholders in settings.

All secret values are returned as `SecretString` (`secrecy::SecretString`), which redacts `Debug` output and zeroizes memory on drop.

## Built-in providers

- `EnvProvider`: reads `std::env::var`, optionally with a prefix (`EnvProvider::with_prefix("MYAPP_")`).
- `DotenvProvider`: parses a `.env` file with `dotenvy` without mutating process-wide environment variables.
- `VaultProvider`: reads HashiCorp Vault KV v2 secrets (`/v1/{mount}/data/{path}`) with token or AppRole authentication and automatic token/lease renewal.
- `GcpSecretManagerProvider`: reads GCP Secret Manager versions using GCE/Cloud Run metadata-server tokens.
- `AwsSecretsManagerProvider`: calls AWS Secrets Manager (`GetSecretValue`) using pure-Rust SigV4 signing (`hmac`/`sha2`).
- `ChainSecretsProvider`: tries multiple providers in order.
- `CachedSecretsProvider`: caches lookups in memory for a configurable TTL.

## Interpolating settings

```rust,illustrative
use djangors_core::secrets::{CachedSecretsProvider, ChainSecretsProvider, DotenvProvider, EnvProvider};
use djangors_core::settings::DjangorsSettings;
use std::sync::Arc;
use std::time::Duration;

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let chain = Arc::new(
    ChainSecretsProvider::new()
        .push(Arc::new(EnvProvider::new()))
        .push(Arc::new(DotenvProvider::optional(".env"))),
);
let cached = CachedSecretsProvider::new(chain, Duration::from_secs(300));

let (settings, _warnings) = DjangorsSettings::load_with_secrets(&cached).await?;
settings.validate()?;
# Ok(())
# }
```

In `djangors.toml`:

```toml
debug = false
secret_key = "${secret:APP_SECRET_KEY}"
```
