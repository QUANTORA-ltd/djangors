# djangors-contrib-payments

Payment provider integrations (Paystack and Bachs) for Djangors, with idempotent transaction recording

Part of [Djangors](https://github.com/QUANTORA-ltd/djangors), a batteries-included web framework for Rust in the spirit of Django.

```toml
[dependencies]
djangors-contrib-payments = "0.12"
```

Provides payment provider integration for Djangors with a `PaymentProvider` trait and built-in **Paystack** (`PaystackProvider`) and **Bachs** (`BachsProvider`) implementations. Monetary amounts are strictly handled as integer minor units (kobo/cents) with HMAC-SHA512 webhook verification and idempotent transaction recording for reliability.

- Documentation: https://docs.rs/djangors-contrib-payments
- Guides: https://djangors.vercel.app/docs/
- Repository: https://github.com/QUANTORA-ltd/djangors

Licensed under either of MIT or Apache-2.0 at your option.
