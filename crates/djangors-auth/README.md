# djangors-auth

Authentication, users, and permissions for Djangors

Part of [Djangors](https://github.com/QUANTORA-ltd/djangors), a batteries-included web framework for Rust in the spirit of Django.

```toml
[dependencies]
djangors-auth = "0.9"
```

Provides authentication, user management, and permissions for Djangors. It uses compile-time genericity over the `AuthUser` trait as the Rust-idiomatic equivalent of Django's `AUTH_USER_MODEL` setting, enabling customizable user identity types across applications.

- Documentation: https://docs.rs/djangors-auth
- Guides: https://djangors.vercel.app/docs/
- Repository: https://github.com/QUANTORA-ltd/djangors

Licensed under either of MIT or Apache-2.0 at your option.
