# djangors-test

Testing utilities for the Djangors web framework

Part of [Djangors](https://github.com/Chidi09/djangors), a batteries-included web framework for Rust in the spirit of Django.

```toml
[dependencies]
djangors-test = "0.7"
```

Provides in-process testing helpers and assertions for Djangors applications to unit test routes, views, authentication, sessions, and database operations.

`TestClient` supports JSON bodies, arbitrary HTTP methods and headers, and can wrap a cloneable Tower service with `TestClient::from_service` to exercise middleware stacks.

- Documentation: https://docs.rs/djangors-test
- Guides: https://djangors.vercel.app/docs/
- Repository: https://github.com/Chidi09/djangors

Licensed under either of MIT or Apache-2.0 at your option.
