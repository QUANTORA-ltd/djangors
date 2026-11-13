# SAML 2.0 Single Sign-On

Djangors provides pure-Rust SAML 2.0 Service Provider (SP) support in `djangors::saml` (crate `djangors-contrib-saml`). It has **no C library dependencies** (`libxml2` and `xmlsec1` are not needed) and verifies XML-DSig signatures in-house using `quick-xml`, W3C Exclusive Canonical XML 1.0 (`xml-exc-c14n#`), `der`, `rsa`, `p256`, and `sha2`.

## Enabling the feature

Add the `saml` feature to `djangors`:

```toml
[dependencies]
djangors = { version = "0.13", features = ["saml"] }
```

## Database setup and identity provider configuration

Create the `saml_identity_provider` table once at startup (or in a migration), import an IdP's `EntityDescriptor` metadata XML, and insert [`SamlSpConfig`](../../crates/djangors-contrib-saml/src/lib.rs) plus your replay [`Cache`](../../crates/djangors-cache/src/lib.rs) (`Arc<dyn Cache>`) into `AppState`:

```rust,illustrative
use djangors::cache::{Cache, InMemoryCache};
use djangors::orm::Model as _;
use djangors::saml::{
    create_saml_tables, saml_router, SamlIdentityProvider, SamlSpConfig,
};
use djangors::AppState;
use std::sync::Arc;

# async fn setup(db: djangors::db::Database, idp_metadata_xml: String) -> Result<(), Box<dyn std::error::Error>> {
create_saml_tables(&db).await?;

let idp = SamlIdentityProvider::from_metadata_xml("okta", "Okta Enterprise", idp_metadata_xml)?;
idp.save(&db).await?;

let replay_cache: Arc<dyn Cache> = Arc::new(InMemoryCache::new(10_000));
let sp_config = SamlSpConfig::new(
    "https://app.example.com/saml/okta/metadata",
    "https://app.example.com",
);

let state = AppState::new()
    .insert(db)
    .insert(sp_config)
    .insert(replay_cache);

let router = saml_router();
# let _ = (state, router);
# Ok(())
# }
```

## Endpoints

`saml_router()` mounts three routes keyed by `{idp}` slug:

| Route | Purpose |
| --- | --- |
| `GET /saml/{idp}/metadata` | Serves SP SAML 2.0 `EntityDescriptor` metadata (`WantAssertionsSigned="true"`) |
| `GET /saml/{idp}/login` | Initiates SP-initiated AuthnRequest (HTTP-Redirect binding + session `RelayState`) |
| `POST /saml/{idp}/acs` | Assertion Consumer Service: verifies XML signature, claims, replay cache, JIT user creation, and login |

## Admin registration

When the `admin` feature is enabled, IdP configurations can be added and rotated directly in the admin site:

```rust,illustrative
djangors::saml::register_saml_admin(&admin_site);
```

## Attribute mapping & JIT user creation

`SamlIdentityProvider::attribute_map_json` deserializes into `SamlAttributeMap` (candidate SAML attribute names for `username`, `email`, `first_name`, and `last_name`, falling back to `<NameID>`). When `jit_create_users` is `true`, new users are created with an unusable password (`"!"`).

## Security guarantees

- **No unsigned or partially signed assertions**: At least one direct-child `<ds:Signature>` must cover `<samlp:Response>` (`#<response_id>`) and/or the sole `<saml:Assertion>` (`#<assertion_id>`).
- **Pinned certificates only**: Signatures verify strictly against X.509 certificates stored on `SamlIdentityProvider` — embedded `<KeyInfo>` certificates inside untrusted responses are ignored.
- **Anti-XSW and anti-XXE**: `<!DOCTYPE`, entity references, duplicate `ID` attributes across the document, `<EncryptedAssertion>`, nested `<Assertion>` wrappers, and multiple assertions are rejected before claim extraction.
- **Strict claim & replay enforcement**: Validates `StatusCode == Success`, `Destination`, `Audience`, `Recipient`, `NotBefore`, `NotOnOrAfter`, `InResponseTo`, and marks each `Assertion/@ID` once via atomic `Cache::add`.
- **Supported signature algorithms**: `RSA-SHA256`, `RSA-SHA512`, and `ECDSA-P256-SHA256` with `xml-exc-c14n#` and `enveloped-signature` transforms. (IdPs requiring SHA-1, XML encryption, or DSA signatures are rejected.)
