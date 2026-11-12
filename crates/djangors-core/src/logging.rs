use tracing_subscriber::EnvFilter;

/// Initializes a development-friendly tracing subscriber: compact,
/// colored console output, sensible default log level, and support for
/// the standard `RUST_LOG` environment variable to override verbosity.
///
/// Roughly the Djangors equivalent of Django's default `runserver`
/// request logging (e.g. `[16/Jul/2026 10:23:11] "GET /hello HTTP/1.1" 200 15`),
/// but built on the `tracing` ecosystem so any `tracing`-instrumented code
/// (this framework's middleware included) shows up automatically.
///
/// Call this once, early in your application's `main()`, before starting
/// the server. Calling it more than once is a no-op after the first
/// successful call (tracing only allows a single global subscriber).
///
/// Under the hood, this uses the standard Rust ecosystem `RUST_LOG` environment
/// variable (e.g. `RUST_LOG=debug` or `RUST_LOG=djangors_core=trace`) to override
/// the default log level, which defaults to `"info,djangors_core=debug"`.
pub fn init_dev_logging() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,djangors_core=debug"));

    let _ = tracing_subscriber::fmt()
        .compact()
        .with_env_filter(filter)
        .try_init();
}

/// Initializes a production-friendly tracing subscriber: structured JSON output,
/// sensible default log level, and support for the standard `RUST_LOG` environment
/// variable to override verbosity.
///
/// This formatter does not include colors and outputs logs in a structured JSON
/// format, which is ideal for piping into log aggregators (e.g., Elasticsearch,
/// AWS CloudWatch, Datadog).
///
/// Call this once, early in your application's `main()`, before starting
/// the server. Calling it more than once is a no-op after the first
/// successful call.
///
/// Under the hood, this uses the standard Rust ecosystem `RUST_LOG` environment
/// variable to override the default log level, which defaults to `"info,djangors_core=info"`.
pub fn init_production_logging() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,djangors_core=info"));

    let _ = tracing_subscriber::fmt()
        .json()
        .with_env_filter(filter)
        .try_init();
}

/// Initializes Sentry error tracking together with a production-style structured
/// JSON `tracing` subscriber, in one call.
///
/// `tracing` only allows a single global subscriber to be installed process-wide,
/// so Sentry can't be bolted onto an already-initialized subscriber from
/// [`init_production_logging`] after the fact — this function builds the combined
/// layered subscriber (JSON formatting + Sentry event/breadcrumb capture) itself.
///
/// Returns a guard that **must be held for the lifetime of the process** —
/// typically `let _sentry_guard = djangors_core::logging::init_production_logging_with_sentry(&dsn);`
/// at the top of `main()`. Dropping it flushes any queued events before the
/// client shuts down. An empty or invalid `dsn` produces a disabled client
/// (matching the Sentry SDK's own convention across languages) rather than
/// erroring, so this is always safe to call unconditionally with a settings
/// value that may be empty in development.
///
/// `ERROR`-level `tracing` events are captured as Sentry events automatically;
/// `WARN`/`INFO`/`DEBUG`/`TRACE` events become breadcrumbs attached to the next
/// captured event. Panics are captured automatically via Sentry's built-in panic
/// integration (enabled by this crate's `sentry` feature).
///
/// Requires the `sentry` Cargo feature.
#[cfg(feature = "sentry")]
pub fn init_production_logging_with_sentry(dsn: &str) -> sentry::ClientInitGuard {
    let guard = sentry::init((
        dsn,
        sentry::ClientOptions {
            release: sentry::release_name!(),
            ..Default::default()
        },
    ));

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,djangors_core=info"));

    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().json())
        .with(sentry_tracing::layer())
        .try_init();

    guard
}

/// Keeps the OpenTelemetry trace pipeline alive. Returned by
/// [`init_otel_logging`].
///
/// Hold it for the lifetime of the process (`let _otel = ...;` at the top of
/// `main()`). When it is dropped, the tracer provider is shut down, which
/// exports any spans still waiting in the batch queue.
#[cfg(feature = "otel")]
pub struct OtelGuard {
    provider: opentelemetry_sdk::trace::SdkTracerProvider,
}

#[cfg(feature = "otel")]
impl Drop for OtelGuard {
    fn drop(&mut self) {
        if let Err(err) = self.provider.shutdown() {
            tracing::warn!(
                error = %err,
                "failed to shut down the OpenTelemetry tracer provider"
            );
        }
    }
}

/// Initializes the production JSON `tracing` subscriber and also exports every
/// span to an OpenTelemetry collector over OTLP/HTTP (protobuf).
///
/// Spans are sent to the endpoint named by the standard
/// `OTEL_EXPORTER_OTLP_ENDPOINT` environment variable. When it is not set, the
/// exporter uses `http://localhost:4318` and appends `/v1/traces`. The
/// `OTEL_EXPORTER_OTLP_TRACES_*` variables (endpoint, headers, timeout) are
/// honored as well, as defined by the OpenTelemetry specification.
///
/// `service_name` becomes the `service.name` resource attribute, which is how
/// backends such as Jaeger, Grafana Tempo, Honeycomb and Datadog label your
/// service.
///
/// Filtering works like [`init_production_logging`]: `RUST_LOG` overrides the
/// default level. The default also silences the OpenTelemetry SDK's own
/// internal logs, so an unreachable collector does not flood the output.
///
/// Export failures never panic. They are reported by the SDK and the affected
/// batch is dropped. Building the exporter does not connect to the collector,
/// so this works even when no collector is running.
///
/// Call this once in `main()` and keep the returned guard alive. A second
/// call still returns a guard, but the subscriber installed by the first call
/// stays in place.
///
/// Requires the `otel` Cargo feature.
#[cfg(feature = "otel")]
pub fn init_otel_logging(service_name: &str) -> Result<OtelGuard, crate::error::DjangorsError> {
    use opentelemetry::trace::TracerProvider as _;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    // The exporter's HTTP client uses rustls without a built-in crypto provider; install
    // `ring` unless the application already chose one.
    if otel_rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = otel_rustls::crypto::ring::default_provider().install_default();
    }

    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .build()
        .map_err(|err| {
            crate::error::DjangorsError::Internal(format!("failed to build OTLP exporter: {err}"))
        })?;

    let resource = opentelemetry_sdk::Resource::builder()
        .with_service_name(service_name.to_string())
        .build();

    let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_resource(resource)
        .build();
    let tracer = provider.tracer("djangors");

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new("info,djangors_core=info,opentelemetry=off,opentelemetry_sdk=off")
    });

    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().json())
        .with(tracing_opentelemetry::layer().with_tracer(tracer))
        .try_init();

    Ok(OtelGuard { provider })
}

#[cfg(test)]
#[path = "logging_tests.rs"]
mod tests;
