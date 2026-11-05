# djangors-contrib-messages

Per-session flash message queue for Djangors

Part of [Djangors](https://github.com/QUANTORA-ltd/djangors), a batteries-included web framework for Rust in the spirit of Django.

```toml
[dependencies]
djangors-contrib-messages = "0.6"
```

Provides a per-session flash message queue for one-shot notifications like "Profile updated successfully". Views record notifications with `add` (or `add_success`/`add_error`), and template rendering code consumes them with `take` via session state.

- Documentation: https://docs.rs/djangors-contrib-messages
- Guides: https://djangors.vercel.app/docs/
- Repository: https://github.com/QUANTORA-ltd/djangors

Licensed under either of MIT or Apache-2.0 at your option.
