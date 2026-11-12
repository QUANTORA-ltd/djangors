# djangors-core

HTTP kernel for the Djangors web framework

Part of [Djangors](https://github.com/QUANTORA-ltd/djangors), a batteries-included web framework for Rust in the spirit of Django.

```toml
[dependencies]
djangors-core = "0.12"
```

Provides the core HTTP kernel for Djangors, including `Request`, `Response`, `Router`, and `Handler` types. It includes CSRF protection middleware for unsafe HTTP requests (POST, PUT, PATCH, DELETE) using a double-submit cookie pattern.

- Documentation: https://docs.rs/djangors-core
- Guides: https://djangors.vercel.app/docs/
- Repository: https://github.com/QUANTORA-ltd/djangors

Licensed under either of MIT or Apache-2.0 at your option.
