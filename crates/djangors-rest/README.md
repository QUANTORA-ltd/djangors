# djangors-rest

REST framework with serializers and ViewSets for Djangors

Part of [Djangors](https://github.com/QUANTORA-ltd/djangors), a batteries-included web framework for Rust in the spirit of Django.

```toml
[dependencies]
djangors-rest = "0.6"
```

Provides a REST framework for Djangors including serializers, ViewSets, permissions, pagination, and router mounting. ViewSet routes default to requiring authenticated users, with explicit `AllowAny` opt-ins for public routes.

- Documentation: https://docs.rs/djangors-rest
- Guides: https://djangors.vercel.app/docs/
- Repository: https://github.com/QUANTORA-ltd/djangors

Licensed under either of MIT or Apache-2.0 at your option.
