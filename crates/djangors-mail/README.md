# djangors-mail

Minimal mail library for Djangors

Part of [Djangors](https://github.com/QUANTORA-ltd/djangors), a batteries-included web framework for Rust in the spirit of Django.

```toml
[dependencies]
djangors-mail = "0.13"
```

Provides email messaging utilities (`#[non_exhaustive]` `Message` builder) and pluggable delivery backends (`SmtpBackend` over `rustls`, `FileBackend`, `ConsoleBackend`, and `InMemoryBackend`) for Djangors applications.

- Documentation: https://docs.rs/djangors-mail
- Guides: https://djangors.vercel.app/docs/
- Repository: https://github.com/QUANTORA-ltd/djangors

Licensed under either of MIT or Apache-2.0 at your option.
