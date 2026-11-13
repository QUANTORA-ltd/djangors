# LDAP / Active Directory Authentication

Djangors provides an optional LDAP / Active Directory authentication backend (`LdapBackend`) implementing `AuthBackend`. It runs purely on `rustls` (via `ldap3` with the `ring` crypto provider — zero OpenSSL or C toolchain dependencies).

## Enabling the feature

`ldap` is off by default. Enable it on the `djangors` facade or directly on `djangors-auth`:

```toml
[dependencies]
djangors = { version = "0.12.0", features = ["ldap"] }
```

## Configuration

Build an [`LdapConfig`] and pass it to [`LdapBackend`]:

```rust,illustrative
use djangors_auth::ldap::{LdapAttributeMap, LdapBackend, LdapConfig, LdapGroupMapping};
use std::collections::HashMap;

let mut cfg = LdapConfig::new(
    "ldaps://ldap.example.com:636",
    "cn=service-ro,ou=ServiceAccounts,dc=example,dc=com",
    std::env::var("LDAP_BIND_PASSWORD")?,
    "ou=People,dc=example,dc=com",
)
.with_user_filter("(sAMAccountName={username})");

cfg.attributes = LdapAttributeMap {
    username_attr: "sAMAccountName".into(),
    email_attr: "mail".into(),
    first_name_attr: "givenName".into(),
    last_name_attr: "sn".into(),
    groups_attr: "memberOf".into(),
};

cfg.group_mapping = LdapGroupMapping {
    staff_groups: vec!["Engineering".into()],
    superuser_groups: vec!["CN=Domain Admins,OU=Groups,DC=example,DC=com".into()],
    group_map: HashMap::from([("Editors".into(), "editors".into())]),
    sync_all_groups: false,
};

let backend = LdapBackend::new(cfg);
```

### TLS and custom CA certificates

- By default, `require_tls = true` rejects plaintext `ldap://` URLs unless `starttls = true`.
- `ldap3` 0.12's `tls-rustls-ring` backend validates certificates against the operating system trust store. If your directory uses an internal CA, validate the PEM file path with `cfg.ca_file = Some(...)` and point `SSL_CERT_FILE=/path/to/ca.pem` in your process environment so `rustls` loads it into the root store.

## Security guarantees

- **No anonymous binds**: empty passwords and blank usernames are rejected immediately before dialing the LDAP server.
- **Injection protection**: `{username}` placeholders in `user_filter` are escaped per RFC 4515 (`escape_filter_value`), and `{dn_username}` placeholders are escaped per RFC 4514 (`escape_dn_value`).
- **Unambiguous search**: login fails closed unless the directory search returns exactly one entry.
- **Unusable local passwords**: just-in-time provisioned users receive `"!"` as their password hash so `ModelBackend` cannot authenticate them locally.

## Live integration testing

Unit tests for escaping, attribute mapping, and JIT user/group sync run without a directory server. To exercise live bind + search authentication, set:

```bash
LDAP_URL=ldaps://ldap.example.com:636 \
LDAP_BIND_DN="cn=admin,dc=example,dc=com" \
LDAP_BIND_PASSWORD="..." \
LDAP_BASE_DN="dc=example,dc=com" \
LDAP_TEST_USER=jdoe \
LDAP_TEST_PASSWORD="..." \
cargo test -p djangors-auth --features ldap live_ldap
```
