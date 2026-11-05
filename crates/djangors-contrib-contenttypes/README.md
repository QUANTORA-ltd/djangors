# djangors-contrib-contenttypes

Content types and generic foreign keys for Djangors

Part of [Djangors](https://github.com/QUANTORA-ltd/djangors), a batteries-included web framework for Rust in the spirit of Django.

```toml
[dependencies]
djangors-contrib-contenttypes = "0.6"
```

Provides stable model identities and generic foreign-key pairs for Djangors. It tracks all registered models by storing unique `(app_label, model_name)` pairs in the database via `ContentType`, enabling `GenericForeignKey` dynamic references linking to any row in any database table.

- Documentation: https://docs.rs/djangors-contrib-contenttypes
- Guides: https://djangors.vercel.app/docs/
- Repository: https://github.com/QUANTORA-ltd/djangors

Licensed under either of MIT or Apache-2.0 at your option.
