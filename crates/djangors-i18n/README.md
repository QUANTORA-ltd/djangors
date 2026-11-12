# djangors-i18n

Runtime internationalization for Djangors

Part of [Djangors](https://github.com/Chidi09/djangors), a batteries-included web framework for Rust in the spirit of Django.

```toml
[dependencies]
djangors-i18n = "0.9"
```

Provides runtime internationalization and locale catalog lookup for Djangors using Fluent (`.ftl`) sources. The `LocaleLayer` middleware resolves `Accept-Language` headers or session locale overrides and provides resolved locales to request extensions and template contexts.

- Documentation: https://docs.rs/djangors-i18n
- Guides: https://djangors.vercel.app/docs/
- Repository: https://github.com/Chidi09/djangors

Licensed under either of MIT or Apache-2.0 at your option.
