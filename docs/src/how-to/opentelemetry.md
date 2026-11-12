# How to Export Traces with OpenTelemetry

## Problem
You want to see where time goes in a request: which handler ran, which SQL
statements it executed, and how long each took. You want those traces in a
tracing backend such as Jaeger, Grafana Tempo, Honeycomb, or Datadog.

## Solution
Enable the `otel` Cargo feature and call `djangors_core::logging::init_otel_logging`
once at the top of `main()`. The function keeps the production JSON log output and
also sends every span to an OpenTelemetry collector over OTLP/HTTP.

The feature is off by default. Builds without it do not compile or link any
OpenTelemetry code.

## Enable the feature

Turn on `otel` on the facade crate:

```toml
[dependencies]
djangors = { version = "0.12", features = ["otel"] }
```

If you depend on `djangors-core` directly, enable `otel` there instead.

## Code Example

```rust,illustrative
use djangors::core::logging::init_otel_logging;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Keep the guard alive for the whole process. Dropping it flushes queued spans.
    let _otel = init_otel_logging("my-service")?;

    // ... build the app and serve requests as usual.
    Ok(())
}
```

The guard must stay alive until shutdown. If you drop it early, spans recorded
afterwards are not exported.

## Configuration

Export is configured with the standard OpenTelemetry environment variables:

| Variable | Purpose | Default |
|----------|---------|---------|
| `OTEL_EXPORTER_OTLP_ENDPOINT` | Collector base URL. `/v1/traces` is appended. | `http://localhost:4318` |
| `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` | Full traces URL. Overrides the base endpoint. | unset |
| `OTEL_EXPORTER_OTLP_HEADERS` | Extra headers, such as an API key (`key=value,key2=value2`). | unset |
| `OTEL_EXPORTER_OTLP_TRACES_TIMEOUT` | Export timeout in milliseconds. | 10000 |
| `RUST_LOG` | Log and span filter, as with the other logging setups. | `info,djangors_core=info,opentelemetry=off,opentelemetry_sdk=off` |

The `service_name` argument becomes the `service.name` resource attribute. Use it
to tell services apart in the backend.

The default filter turns off the OpenTelemetry SDK's own internal logs. Without
this, a missing collector would produce a steady stream of export errors. Set
`RUST_LOG` explicitly if you want to see them.

Export never panics. If the collector is down, the SDK reports the failure and
drops that batch, and the application keeps serving requests.

## Running a local collector

Jaeger's all-in-one image accepts OTLP over HTTP on port 4318 and serves its UI
on port 16686:

```bash
docker run --rm -p 4318:4318 -p 16686:16686 jaegertracing/all-in-one:latest
```

Start your application with the default endpoint, then open
`http://localhost:16686` and select your `service_name`.

## What spans exist

| Span | Created by | Fields |
|------|------------|--------|
| `request` | Every request handled by the router (`Router::handle_dispatch`) | `method`, `path` |
| `sql` | Every query run through `Conn` (`fetch_all`, `fetch_one`, `fetch_optional`, `execute`) | `statement` |

The `sql` span records the SQL text only. Bound parameter values are never
recorded, so secrets and personal data do not leave the process through traces.
Queries you write inside a handler run under the `request` span, and the `sql`
spans appear as its children.

The `request` and `sql` spans are plain `tracing` spans. They cost almost nothing
when no subscriber is installed, so they are present in every build. The `otel`
feature only adds the exporter.

Your own `tracing::info_span!` and `#[tracing::instrument]` spans nest inside
them automatically.

## Notes

- The exporter uses HTTP with protobuf encoding and rustls with the `ring` crypto
  provider, so `https://` endpoints work. It needs no OpenSSL, C toolchain or gRPC
  stack. `init_otel_logging` installs `ring` as the process-wide rustls provider
  unless your application installed one first.
- Traces are batched. Spans recorded just before a crash may not be exported.
  Call `init_otel_logging` once and keep the guard until shutdown.
- Calling `init_otel_logging` a second time returns a guard without installing a
  second subscriber. The first call's configuration stays in effect.
