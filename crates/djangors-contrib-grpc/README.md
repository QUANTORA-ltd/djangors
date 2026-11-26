# djangors-contrib-grpc

gRPC integration (`tonic`) for Djangors with shared `AppState` and a `HybridServer` supporting dual-port or single-port multiplexed HTTP + gRPC.

Part of [Djangors](https://github.com/QUANTORA-ltd/djangors), a batteries-included web framework for Rust in the spirit of Django.

```toml
[dependencies]
djangors-contrib-grpc = "1.0"
```

Provides:

- `AppStateInterceptor` and `extract_app_state` to share `djangors_core::AppState` (database pools, caches, configuration) with `tonic` gRPC handlers.
- `is_grpc_request` to detect `application/grpc` requests by `Content-Type`.
- `HybridServer` to serve a `djangors_core::Router` alongside `tonic::service::Routes` either on separate ports (`serve_dual`) or multiplexed on a single port (`serve_multiplexed` / `serve_multiplexed_listener`).

- Documentation: https://docs.rs/djangors-contrib-grpc
- Guides: https://djangors.vercel.app/docs/
- Repository: https://github.com/QUANTORA-ltd/djangors

Licensed under either of MIT or Apache-2.0 at your option.
