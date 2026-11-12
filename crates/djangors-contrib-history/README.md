# djangors-contrib-history

Model history for Djangors: records every create, update, and delete of chosen models.

Part of [Djangors](https://github.com/QUANTORA-ltd/djangors), a batteries-included web framework for Rust in the spirit of Django.

```toml
[dependencies]
djangors-contrib-history = "0.12"
```

Records the `saved` and `deleted` model signals of tracked models into a shared `djangors_model_history` table, with each row's field values stored as JSON. Read a row's timeline back with `history_for`.

- Documentation: https://docs.rs/djangors-contrib-history
- Guides: https://djangors.vercel.app/docs/
- Repository: https://github.com/QUANTORA-ltd/djangors

Licensed under either of MIT or Apache-2.0 at your option.
