# djangors-sessions

Session engines for the Djangors web framework

Part of [Djangors](https://github.com/QUANTORA-ltd/djangors), a batteries-included web framework for Rust in the spirit of Django.

```toml
[dependencies]
djangors-sessions = "0.9"
```

Provides session management engines for Djangors, including signed-cookie sessions. Session state is JSON-serialized, base64-encoded, signed with HMAC-SHA256, and managed in requests via `SessionLayer` middleware and `Session` handles.

- Documentation: https://docs.rs/djangors-sessions
- Guides: https://djangors.vercel.app/docs/
- Repository: https://github.com/QUANTORA-ltd/djangors

Licensed under either of MIT or Apache-2.0 at your option.
