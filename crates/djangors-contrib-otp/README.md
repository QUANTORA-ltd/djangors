# djangors-contrib-otp

TOTP enrollment and verification for Djangors

Part of [Djangors](https://github.com/QUANTORA-ltd/djangors), a batteries-included web framework for Rust in the spirit of Django.

```toml
[dependencies]
djangors-contrib-otp = "0.13"
```

Provides TOTP-based two-factor authentication primitives for Djangors applications. It handles device enrollment, secret generation, QR code provisioning URIs, and code verification before establishing authenticated sessions.

- Documentation: https://docs.rs/djangors-contrib-otp
- Guides: https://djangors.vercel.app/docs/
- Repository: https://github.com/QUANTORA-ltd/djangors

Licensed under either of MIT or Apache-2.0 at your option.
