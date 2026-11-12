# djangors-deploy

dj deploy - multi-provider deployment for the Djangors web framework

Part of [Djangors](https://github.com/QUANTORA-ltd/djangors), a batteries-included web framework for Rust in the spirit of Django.

```toml
[dependencies]
djangors-deploy = "0.11"
```

Provides `dj deploy` multi-provider deployment support for Djangors. It defines a `DeployProvider` trait covering provision, deploy, status, logs, and destroy operations with built-in support for Render and extensible provider implementations.

- Documentation: https://docs.rs/djangors-deploy
- Guides: https://djangors.vercel.app/docs/
- Repository: https://github.com/QUANTORA-ltd/djangors

Licensed under either of MIT or Apache-2.0 at your option.
