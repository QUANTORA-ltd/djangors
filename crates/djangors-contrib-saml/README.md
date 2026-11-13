# djangors-contrib-saml

SAML 2.0 single sign-on (Service Provider) for Djangors, in pure Rust.

Part of [Djangors](https://github.com/QUANTORA-ltd/djangors), a batteries-included web framework for Rust in the spirit of Django.

```toml
[dependencies]
djangors-contrib-saml = "0.13"
```

Serves SP metadata, starts logins, and verifies responses at the ACS endpoint. Signatures are checked against pinned IdP certificates (RSA-SHA256, RSA-SHA512, ECDSA-P256-SHA256) with exclusive canonicalization. DTDs, entities, duplicate IDs, extra or nested assertions, and encrypted assertions are rejected. Audience, recipient, timing, `InResponseTo` and replay are all checked.

No OpenSSL, libxml2 or xmlsec is needed.

- Documentation: https://docs.rs/djangors-contrib-saml
- Guides: https://djangors.vercel.app/docs/
- Repository: https://github.com/QUANTORA-ltd/djangors

Licensed under either of MIT or Apache-2.0 at your option.
