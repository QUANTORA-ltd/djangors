# djangors-contrib-guardian

Object-level permissions for Djangors

Part of [Djangors](https://github.com/Chidi09/djangors), a batteries-included web framework for Rust in the spirit of Django.

```toml
[dependencies]
djangors-contrib-guardian = "0.9"
```

Provides granular object-level permission checking and management for Djangors, layered on top of `djangors_auth::has_perm`. Helpers like `has_perm_for_object` allow authorization checks against individual model instances in custom views and handlers.

- Documentation: https://docs.rs/djangors-contrib-guardian
- Guides: https://djangors.vercel.app/docs/
- Repository: https://github.com/Chidi09/djangors

Licensed under either of MIT or Apache-2.0 at your option.
