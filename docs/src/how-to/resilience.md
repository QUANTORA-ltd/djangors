# How to Make External Calls Resilient

Calls to payment gateways, mailers, and third-party APIs can hang or fail transiently. `djangors_core::resilience` provides exponential-backoff retries, timeouts, and a circuit breaker.

## Example: calling a payment provider

Combine `CircuitBreaker`, `retry`, and `with_timeout` so a flaky gateway is retried briefly, slow calls time out, and a downed gateway trips open instead of piling up blocked tasks:

```rust,compile
# use std::time::Duration;
# use djangors_core::{retry, with_timeout, BreakerError, CircuitBreaker, RetryPolicy};
#[derive(Debug)]
enum PaymentError {
    Transient(&'static str),
    Declined,
}

impl std::fmt::Display for PaymentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transient(msg) => write!(f, "transient gateway error: {msg}"),
            Self::Declined => write!(f, "card declined"),
        }
    }
}

async fn charge_card(
    breaker: &CircuitBreaker,
    amount_cents: u64,
) -> Result<String, BreakerError<PaymentError>> {
    let policy = RetryPolicy::new()
        .max_attempts(3)
        .initial_delay(Duration::from_millis(100))
        .max_delay(Duration::from_secs(2));

    breaker
        .call(retry(
            &policy,
            || async {
                match with_timeout(Duration::from_secs(5), async {
                    // Replace with your HTTP client call.
                    let _ = amount_cents;
                    Ok::<_, PaymentError>("ch_123".to_string())
                })
                .await
                {
                    Ok(res) => res,
                    Err(_) => Err(PaymentError::Transient("request timed out")),
                }
            },
            |err| matches!(err, PaymentError::Transient(_)),
        ))
        .await
}
```

## Primitives

- **`RetryPolicy` & `retry`** (behind the default-on `resilience` feature): retries with exponential backoff and optional jitter while `should_retry(&err)` returns `true`, logging each retry attempt and delay with `tracing::warn!`.
- **`with_timeout`**: wraps `tokio::time::timeout` and returns `TimedOut` if the future does not complete in time.
- **`CircuitBreaker`**: cheaply cloneable (`Arc`-backed). Opens after `failure_threshold` consecutive failures (default 5), rejects immediately with `BreakerError::Open` for `open_duration` (default 30 s), then allows `half_open_max_calls` (default 1) probe calls. Call `.state()` to inspect `CircuitState` for metrics.
