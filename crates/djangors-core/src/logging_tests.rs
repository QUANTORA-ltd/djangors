use super::*;

#[test]
fn test_init_logging_does_not_panic() {
    // First initialization call should succeed.
    init_dev_logging();

    // Subsequent initialization calls should be harmless no-ops.
    init_dev_logging();
    init_production_logging();
}

#[cfg(feature = "otel")]
#[test]
fn test_init_otel_logging_builds_without_a_collector() {
    // No collector listens on the default OTLP endpoint in unit tests. Building
    // the exporter and the tracer provider must still succeed, and recording a
    // span must not panic when the batch export fails.
    let guard = init_otel_logging("djangors-test").expect("otel logging should build");

    tracing::info_span!("otel_test_span").in_scope(|| {
        tracing::info!("span recorded without a collector");
    });

    // Dropping the guard shuts the provider down; export errors are logged, not raised.
    drop(guard);
}

#[cfg(feature = "otel")]
#[test]
fn test_init_otel_logging_second_call_is_harmless() {
    let first = init_otel_logging("djangors-test").expect("first call should succeed");
    let second = init_otel_logging("djangors-test").expect("second call should succeed");
    drop(second);
    drop(first);
}

#[cfg(feature = "sentry")]
#[test]
fn test_init_sentry_logging_with_empty_dsn_does_not_panic_or_reach_the_network() {
    // An empty DSN produces a disabled Sentry client (the SDK's own convention
    // across languages, not a Djangors-specific fallback) - safe to call in a
    // unit test with no real Sentry project configured, and confirms the
    // combined layered subscriber (JSON fmt + sentry-tracing) actually builds.
    let guard = init_production_logging_with_sentry("");
    assert!(!guard.is_enabled());
}
