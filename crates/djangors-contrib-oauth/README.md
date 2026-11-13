# djangors-contrib-oauth

Social login (OAuth 2 / OpenID Connect client) and an OAuth 2 / OIDC provider for Djangors.

Part of [Djangors](https://github.com/QUANTORA-ltd/djangors), a batteries-included web framework for Rust in the spirit of Django.

```toml
[dependencies]
djangors-contrib-oauth = "0.13"
```

- **`client`** (default): sign in with Google, Microsoft, Apple, GitHub or any OIDC provider. Uses `state`, PKCE (S256) and `nonce`, verifies ID tokens against the provider's JWKS, and links existing accounts by email only when the provider says the email is verified.
- **`provider`** (default): let other apps sign in with yours. Authorization code with PKCE, client credentials, rotating refresh tokens with reuse detection, revocation, introspection, UserInfo, JWKS and OIDC discovery. Generate ID-token signing keys with `SigningKey::generate_es256()` (ES256 is the default); `RS256` (`OAuthProviderConfig::id_token_signing_alg("RS256")` with `SigningKey::generate_rs256()`) is also available for relying parties that require RSA, though it uses the `rsa` crate which has RUSTSEC-2023-0071 (Marvin timing side channel).

HTTPS uses rustls with the `ring` provider; there is no OpenSSL dependency.

- Documentation: https://docs.rs/djangors-contrib-oauth
- Guides: https://djangors.vercel.app/docs/
- Repository: https://github.com/QUANTORA-ltd/djangors

Licensed under either of MIT or Apache-2.0 at your option.
