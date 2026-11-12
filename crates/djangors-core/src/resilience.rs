//! Resilience primitives for external service calls: retries, timeouts, and circuit breakers.

use std::fmt;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::time::Instant;

/// Exponential-backoff retry configuration (requires the `resilience` feature).
#[cfg(feature = "resilience")]
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct RetryPolicy {
    /// Total number of attempts including the initial call (default 3).
    pub max_attempts: usize,
    /// Delay before the first retry (default 100 ms).
    pub initial_delay: Duration,
    /// Upper bound on delay between retries (default 5 s).
    pub max_delay: Duration,
    /// Whether to add random jitter to backoff delays (default `true`).
    pub jitter: bool,
}

#[cfg(feature = "resilience")]
impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(5),
            jitter: true,
        }
    }
}

#[cfg(feature = "resilience")]
impl RetryPolicy {
    /// Creates a [`RetryPolicy`] with default settings (3 attempts, 100 ms initial, 5 s max, jitter on).
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the total number of attempts (clamped to at least 1).
    pub fn max_attempts(mut self, max_attempts: usize) -> Self {
        self.max_attempts = max_attempts.max(1);
        self
    }

    /// Sets the initial backoff delay before the first retry.
    pub fn initial_delay(mut self, delay: Duration) -> Self {
        self.initial_delay = delay;
        self
    }

    /// Sets the maximum backoff delay between retries.
    pub fn max_delay(mut self, delay: Duration) -> Self {
        self.max_delay = delay;
        self
    }

    /// Enables or disables random jitter on backoff delays.
    pub fn jitter(mut self, jitter: bool) -> Self {
        self.jitter = jitter;
        self
    }
}

/// Retries `op` with exponential backoff according to `policy` while
/// `should_retry(&err)` returns `true`.
#[cfg(feature = "resilience")]
pub async fn retry<T, E, F, Fut>(
    policy: &RetryPolicy,
    op: F,
    should_retry: impl Fn(&E) -> bool,
) -> Result<T, E>
where
    E: fmt::Display,
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    use backon::Retryable as _;

    let mut builder = backon::ExponentialBuilder::new()
        .with_min_delay(policy.initial_delay)
        .with_max_delay(policy.max_delay)
        .with_max_times(policy.max_attempts.saturating_sub(1));

    if policy.jitter {
        builder = builder.with_jitter();
    }

    let mut attempt = 0usize;
    op.retry(builder)
        .sleep(backon::TokioSleeper)
        .when(should_retry)
        .notify(|err, dur| {
            attempt += 1;
            tracing::warn!(
                attempt,
                delay_ms = dur.as_millis() as u64,
                error = %err,
                "retrying failed operation"
            );
        })
        .await
}

/// Error returned by [`with_timeout`] when a future does not complete in time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("operation timed out after {duration:?}")]
pub struct TimedOut {
    /// The configured timeout duration that elapsed.
    pub duration: Duration,
}

/// Awaits `fut`, returning [`TimedOut`] if it does not complete within `duration`.
pub async fn with_timeout<T, Fut>(duration: Duration, fut: Fut) -> Result<T, TimedOut>
where
    Fut: Future<Output = T>,
{
    tokio::time::timeout(duration, fut)
        .await
        .map_err(|_| TimedOut { duration })
}

/// Current state of a [`CircuitBreaker`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    /// Normal operation; calls pass through.
    Closed,
    /// Tripped after consecutive failures; calls are rejected immediately.
    Open,
    /// Cooldown elapsed; a limited number of probe calls are allowed.
    HalfOpen,
}

/// Error returned by [`CircuitBreaker::call`].
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum BreakerError<E> {
    /// The circuit breaker is currently open and rejected the call without running it.
    #[error("circuit breaker is open")]
    Open,
    /// The underlying operation ran and returned `E`.
    #[error(transparent)]
    Inner(#[from] E),
}

#[derive(Debug)]
enum InnerState {
    Closed { consecutive_failures: usize },
    Open { opened_at: Instant },
    HalfOpen { in_flight: usize },
}

#[derive(Debug)]
struct CircuitBreakerInner {
    failure_threshold: usize,
    open_duration: Duration,
    half_open_max_calls: usize,
    state: Mutex<InnerState>,
}

/// Thread-safe, cheaply cloneable circuit breaker.
#[derive(Debug, Clone)]
pub struct CircuitBreaker {
    inner: Arc<CircuitBreakerInner>,
}

impl Default for CircuitBreaker {
    fn default() -> Self {
        Self::new()
    }
}

impl CircuitBreaker {
    /// Creates a [`CircuitBreaker`] with default thresholds:
    /// 5 consecutive failures, 30 s open duration, 1 half-open probe call.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(CircuitBreakerInner {
                failure_threshold: 5,
                open_duration: Duration::from_secs(30),
                half_open_max_calls: 1,
                state: Mutex::new(InnerState::Closed {
                    consecutive_failures: 0,
                }),
            }),
        }
    }

    /// Sets the number of consecutive failures required to open the circuit.
    pub fn failure_threshold(mut self, threshold: usize) -> Self {
        Arc::make_mut_or_replace(&mut self.inner, |inner| {
            inner.failure_threshold = threshold.max(1);
        });
        self
    }

    /// Sets how long the circuit stays open before transitioning to half-open.
    pub fn open_duration(mut self, duration: Duration) -> Self {
        Arc::make_mut_or_replace(&mut self.inner, |inner| {
            inner.open_duration = duration;
        });
        self
    }

    /// Sets the maximum number of concurrent probe calls allowed while half-open.
    pub fn half_open_max_calls(mut self, max_calls: usize) -> Self {
        Arc::make_mut_or_replace(&mut self.inner, |inner| {
            inner.half_open_max_calls = max_calls.max(1);
        });
        self
    }

    /// Returns the current [`CircuitState`] (for metrics and diagnostics).
    pub fn state(&self) -> CircuitState {
        let guard = self
            .inner
            .state
            .lock()
            .expect("circuit breaker mutex poisoned");
        match *guard {
            InnerState::Closed { .. } => CircuitState::Closed,
            InnerState::Open { opened_at } => {
                if opened_at.elapsed() >= self.inner.open_duration {
                    CircuitState::HalfOpen
                } else {
                    CircuitState::Open
                }
            }
            InnerState::HalfOpen { .. } => CircuitState::HalfOpen,
        }
    }

    /// Executes `fut` if the circuit is closed or allows a half-open probe.
    pub async fn call<T, E, Fut>(&self, fut: Fut) -> Result<T, BreakerError<E>>
    where
        Fut: Future<Output = Result<T, E>>,
    {
        let entered_half_open = {
            let mut guard = self
                .inner
                .state
                .lock()
                .expect("circuit breaker mutex poisoned");
            match &mut *guard {
                InnerState::Closed { .. } => false,
                InnerState::Open { opened_at } => {
                    if opened_at.elapsed() >= self.inner.open_duration {
                        *guard = InnerState::HalfOpen { in_flight: 1 };
                        true
                    } else {
                        return Err(BreakerError::Open);
                    }
                }
                InnerState::HalfOpen { in_flight } => {
                    if *in_flight < self.inner.half_open_max_calls {
                        *in_flight += 1;
                        true
                    } else {
                        return Err(BreakerError::Open);
                    }
                }
            }
        };

        // If this half-open probe is cancelled (its future dropped mid-await), the guard
        // reopens the circuit; otherwise its probe slot would never be released and the
        // breaker would stay half-open, rejecting every call, forever.
        let mut probe = ProbeGuard {
            breaker: self,
            armed: entered_half_open,
        };
        let result = fut.await;
        probe.armed = false;

        let mut guard = self
            .inner
            .state
            .lock()
            .expect("circuit breaker mutex poisoned");
        match result {
            Ok(val) => {
                *guard = InnerState::Closed {
                    consecutive_failures: 0,
                };
                Ok(val)
            }
            Err(err) => {
                if entered_half_open || matches!(*guard, InnerState::HalfOpen { .. }) {
                    *guard = InnerState::Open {
                        opened_at: Instant::now(),
                    };
                } else if let InnerState::Closed {
                    consecutive_failures,
                } = &mut *guard
                {
                    *consecutive_failures += 1;
                    if *consecutive_failures >= self.inner.failure_threshold {
                        *guard = InnerState::Open {
                            opened_at: Instant::now(),
                        };
                    }
                }
                Err(BreakerError::Inner(err))
            }
        }
    }
}

struct ProbeGuard<'a> {
    breaker: &'a CircuitBreaker,
    armed: bool,
}

impl Drop for ProbeGuard<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        if let Ok(mut state) = self.breaker.inner.state.lock() {
            if matches!(*state, InnerState::HalfOpen { .. }) {
                *state = InnerState::Open {
                    opened_at: Instant::now(),
                };
            }
        }
    }
}

trait ArcUpdateExt<T> {
    fn make_mut_or_replace(arc: &mut Arc<T>, f: impl FnOnce(&mut T));
}

impl ArcUpdateExt<CircuitBreakerInner> for Arc<CircuitBreakerInner> {
    fn make_mut_or_replace(
        arc: &mut Arc<CircuitBreakerInner>,
        f: impl FnOnce(&mut CircuitBreakerInner),
    ) {
        if let Some(inner) = Arc::get_mut(arc) {
            f(inner);
        } else {
            let mut cloned = CircuitBreakerInner {
                failure_threshold: arc.failure_threshold,
                open_duration: arc.open_duration,
                half_open_max_calls: arc.half_open_max_calls,
                state: Mutex::new(InnerState::Closed {
                    consecutive_failures: 0,
                }),
            };
            f(&mut cloned);
            *arc = Arc::new(cloned);
        }
    }
}

#[cfg(test)]
#[path = "resilience_tests.rs"]
mod tests;
