# djangors-tasks

Background task queue, #[task] macro, and worker for Djangors

Part of [Djangors](https://github.com/QUANTORA-ltd/djangors), a batteries-included web framework for Rust in the spirit of Django.

```toml
[dependencies]
djangors-tasks = "0.9"
```

Provides a distributed background task queue system, `#[task]` attribute macro, and worker loop for Djangors. Features include database task models (`QueuedTask`, `RecurringTask`), compile-time task registration via `inventory`, and database-level concurrency locking for worker execution.

- Documentation: https://docs.rs/djangors-tasks
- Guides: https://djangors.vercel.app/docs/
- Repository: https://github.com/QUANTORA-ltd/djangors

Licensed under either of MIT or Apache-2.0 at your option.
