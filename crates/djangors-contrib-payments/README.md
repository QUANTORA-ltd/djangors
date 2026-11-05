# djangors-contrib-payments

Payment provider integration (Paystack) for Djangors, with idempotent transaction recording

Part of [Djangors](https://github.com/QUANTORA-ltd/djangors), a batteries-included web framework for Rust in the spirit of Django.

```toml
[dependencies]
djangors-contrib-payments = "0.6"
```

Provides payment provider integration for Djangors with a `PaymentProvider` trait and Paystack implementation. Monetary amounts are strictly handled as integer minor units (kobo/cents) with idempotent transaction recording for reliability.

- Documentation: https://docs.rs/djangors-contrib-payments
- Guides: https://djangors.vercel.app/docs/
- Repository: https://github.com/QUANTORA-ltd/djangors

Licensed under either of MIT or Apache-2.0 at your option.
