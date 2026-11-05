# djangors-cache

Cache backends and middleware for Djangors

Part of [Djangors](https://github.com/QUANTORA-ltd/djangors), a batteries-included web framework for Rust in the spirit of Django.

```toml
[dependencies]
djangors-cache = "0.6"
```

Provides raw-byte key-value caches, common storage backends, and explicitly opted-in response caching middleware. Features include the core `Cache` trait, `InMemoryCache` using Moka, `DatabaseCache` using SQL tables, `RedisCache` for distributed storage, and Tower middleware `CacheLayer` to serve and cache HTTP GET responses.

- Documentation: https://docs.rs/djangors-cache
- Guides: https://djangors.vercel.app/docs/
- Repository: https://github.com/QUANTORA-ltd/djangors

Licensed under either of MIT or Apache-2.0 at your option.
