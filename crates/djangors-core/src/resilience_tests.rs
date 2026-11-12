use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[cfg(feature = "resilience")]
#[tokio::test]
async fn retry_succeeds_on_third_attempt() {
    let attempts = AtomicUsize::new(0);
    let policy = RetryPolicy::new()
        .max_attempts(3)
        .initial_delay(Duration::from_millis(1))
        .max_delay(Duration::from_millis(5))
        .jitter(false);

    let res: Result<&str, &str> = retry(
        &policy,
        || async {
            let n = attempts.fetch_add(1, Ordering::SeqCst) + 1;
            if n < 3 {
                Err("transient")
            } else {
                Ok("ok")
            }
        },
        |_| true,
    )
    .await;

    assert_eq!(res, Ok("ok"));
    assert_eq!(attempts.load(Ordering::SeqCst), 3);
}

#[cfg(feature = "resilience")]
#[tokio::test]
async fn retry_gives_up_after_max_attempts_with_last_error() {
    let attempts = AtomicUsize::new(0);
    let policy = RetryPolicy::new()
        .max_attempts(3)
        .initial_delay(Duration::from_millis(1))
        .max_delay(Duration::from_millis(5))
        .jitter(false);

    let res: Result<(), String> = retry(
        &policy,
        || async {
            let n = attempts.fetch_add(1, Ordering::SeqCst) + 1;
            Err(format!("error-{n}"))
        },
        |_| true,
    )
    .await;

    assert_eq!(res, Err("error-3".to_string()));
    assert_eq!(attempts.load(Ordering::SeqCst), 3);
}

#[cfg(feature = "resilience")]
#[tokio::test]
async fn retry_stops_immediately_when_should_retry_is_false() {
    let attempts = AtomicUsize::new(0);
    let policy = RetryPolicy::new()
        .max_attempts(5)
        .initial_delay(Duration::from_millis(1))
        .jitter(false);

    let res: Result<(), &str> = retry(
        &policy,
        || async {
            attempts.fetch_add(1, Ordering::SeqCst);
            Err("permanent")
        },
        |_| false,
    )
    .await;

    assert_eq!(res, Err("permanent"));
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn with_timeout_fires_and_succeeds() {
    let fast = with_timeout(Duration::from_millis(50), async { 42 }).await;
    assert_eq!(fast, Ok(42));

    let slow = with_timeout(Duration::from_millis(10), async {
        tokio::time::sleep(Duration::from_secs(1)).await;
        42
    })
    .await;
    assert_eq!(
        slow,
        Err(TimedOut {
            duration: Duration::from_millis(10)
        })
    );
}

#[tokio::test]
async fn circuit_breaker_lifecycle() {
    tokio::time::pause();

    let breaker = CircuitBreaker::new()
        .failure_threshold(2)
        .open_duration(Duration::from_secs(10))
        .half_open_max_calls(1);

    assert_eq!(breaker.state(), CircuitState::Closed);

    // First failure keeps circuit closed
    let r1: Result<(), BreakerError<&str>> = breaker.call(async { Err("fail-1") }).await;
    assert_eq!(r1, Err(BreakerError::Inner("fail-1")));
    assert_eq!(breaker.state(), CircuitState::Closed);

    // Second consecutive failure opens circuit
    let r2: Result<(), BreakerError<&str>> = breaker.call(async { Err("fail-2") }).await;
    assert_eq!(r2, Err(BreakerError::Inner("fail-2")));
    assert_eq!(breaker.state(), CircuitState::Open);

    // Rejects immediately while open without calling inner future
    let called = AtomicUsize::new(0);
    let r_open: Result<(), BreakerError<&str>> = breaker
        .call(async {
            called.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .await;
    assert_eq!(r_open, Err(BreakerError::Open));
    assert_eq!(called.load(Ordering::SeqCst), 0);

    // Advance past open_duration -> HalfOpen
    tokio::time::advance(Duration::from_secs(11)).await;
    assert_eq!(breaker.state(), CircuitState::HalfOpen);

    // Failure in HalfOpen reopens circuit
    let r_half_fail: Result<(), BreakerError<&str>> = breaker.call(async { Err("fail-3") }).await;
    assert_eq!(r_half_fail, Err(BreakerError::Inner("fail-3")));
    assert_eq!(breaker.state(), CircuitState::Open);

    // Advance again -> HalfOpen, then success closes circuit
    tokio::time::advance(Duration::from_secs(11)).await;
    assert_eq!(breaker.state(), CircuitState::HalfOpen);

    let r_ok: Result<&str, BreakerError<&str>> = breaker.call(async { Ok("recovered") }).await;
    assert_eq!(r_ok, Ok("recovered"));
    assert_eq!(breaker.state(), CircuitState::Closed);
}

#[tokio::test(start_paused = true)]
async fn cancelled_half_open_probe_does_not_wedge_the_breaker() {
    let breaker = CircuitBreaker::new()
        .failure_threshold(1)
        .open_duration(Duration::from_secs(10));
    let _ = breaker.call(async { Err::<(), &str>("boom") }).await;
    assert_eq!(breaker.state(), CircuitState::Open);

    tokio::time::advance(Duration::from_secs(11)).await;

    // A half-open probe that never finishes, cancelled by a timeout.
    let probe = breaker.call(std::future::pending::<Result<(), &str>>());
    assert!(tokio::time::timeout(Duration::from_millis(10), probe)
        .await
        .is_err());

    // The cancelled probe reopens the circuit instead of holding the only probe slot.
    assert_eq!(breaker.state(), CircuitState::Open);
    tokio::time::advance(Duration::from_secs(11)).await;
    let res = breaker.call(async { Ok::<_, &str>(7) }).await;
    assert_eq!(res, Ok(7));
    assert_eq!(breaker.state(), CircuitState::Closed);
}
