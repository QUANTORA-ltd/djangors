# djangors-contrib-otp

TOTP enrollment and verification for Djangors

Part of [Djangors](https://github.com/Chidi09/djangors), a batteries-included web framework for Rust in the spirit of Django.

```toml
[dependencies]
djangors-contrib-otp = "0.6"
```

Provides TOTP-based two-factor authentication primitives for Djangors applications. It handles device enrollment, secret generation, QR code provisioning URIs, and code verification before establishing authenticated sessions.

By default, the `models` feature is enabled and registers the `OtpDevice` model. To use only
the TOTP helpers without adding the OTP device table to your model registry, disable default
features:

```toml
djangors-contrib-otp = { version = "0.7", default-features = false }
```

`OtpDevice.secret` is stored as plaintext. If credentials must be encrypted at rest, use the
helpers with your own encrypted storage and disable the default `models` feature.

- Documentation: https://docs.rs/djangors-contrib-otp
- Guides: https://djangors.vercel.app/docs/
- Repository: https://github.com/Chidi09/djangors

Licensed under either of MIT or Apache-2.0 at your option.
