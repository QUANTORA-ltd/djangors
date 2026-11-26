# djangors-contrib-flags

Database and cache-backed feature flags and gradual rollouts for Djangors.

Part of [Djangors](https://github.com/QUANTORA-ltd/djangors), a batteries-included web framework for Rust in the spirit of Django.

```toml
[dependencies]
djangors-contrib-flags = "1.0"
```

Provides the `FeatureFlag` ORM model, deterministic SHA-256 percentage rollouts, user and organization allowlists, optional `djangors-cache` backing with automatic invalidation, and optional `djangors-admin` registration.

- Documentation: https://docs.rs/djangors-contrib-flags
- Guides: https://djangors.vercel.app/docs/
- Repository: https://github.com/QUANTORA-ltd/djangors

Licensed under either of MIT or Apache-2.0 at your option.
