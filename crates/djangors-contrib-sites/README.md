# djangors-contrib-sites

Multi-site domain resolution and `Site` model for Djangors.

Part of [Djangors](https://github.com/QUANTORA-ltd/djangors), a batteries-included web framework for Rust in the spirit of Django.

```toml
[dependencies]
djangors-contrib-sites = "1.0"
```

Mirrors `django.contrib.sites`: stores `Site` rows (`domain`, `name`) in the `django_site` table, resolves the active site from incoming request `Host` headers via [`get_current_site`] or [`CurrentSiteLayer`], and registers `Site` in `djangors-admin`.

- Documentation: https://docs.rs/djangors-contrib-sites
- Guides: https://djangors.vercel.app/docs/
- Repository: https://github.com/QUANTORA-ltd/djangors

Licensed under either of MIT or Apache-2.0 at your option.
