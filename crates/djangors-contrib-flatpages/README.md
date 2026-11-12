# djangors-contrib-flatpages

Admin-editable exact-path flatpages for Djangors

Part of [Djangors](https://github.com/QUANTORA-ltd/djangors), a batteries-included web framework for Rust in the spirit of Django.

```toml
[dependencies]
djangors-contrib-flatpages = "0.12"
```

Provides admin-editable flat pages served as trusted HTML, matching Django's flatpages convention where staff-authored page bodies render as-is. Applications register flatpage routes explicitly via `flatpage_routes` to render database-backed static content pages.

- Documentation: https://docs.rs/djangors-contrib-flatpages
- Guides: https://djangors.vercel.app/docs/
- Repository: https://github.com/QUANTORA-ltd/djangors

Licensed under either of MIT or Apache-2.0 at your option.
