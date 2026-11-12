//! Task-local SQL query recording and N+1 / duplicate-query detection.
//!
//! When a request is wrapped in [`with_query_log`] (for example by the
//! `debug-panel` middleware in `djangors-core`), every query executed through
//! [`Conn`](crate::Conn) records its SQL text, parameter count, and execution
//! duration in microseconds. Outside a [`with_query_log`] scope, the check is
//! a single thread-local read with zero heap allocation.

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::{Arc, Mutex};

/// A single recorded SQL query execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedQuery {
    /// The SQL statement executed.
    pub sql: String,
    /// Number of bound parameters passed to the query.
    pub param_count: usize,
    /// Execution time in microseconds.
    pub duration_micros: u64,
}

/// Warning emitted when an identical SQL query executes multiple times within a single request
/// (a classic indicator of an N+1 query loop).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateQueryWarning {
    /// The repeated SQL statement.
    pub sql: String,
    /// Number of times this exact SQL statement was executed in the scope.
    pub count: usize,
    /// Total time spent across all executions of this query in microseconds.
    pub total_duration_micros: u64,
}

/// Aggregated SQL query statistics for a request or test scope.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QueryStats {
    /// Every query executed in chronological order.
    pub queries: Vec<RecordedQuery>,
    /// Total execution time across all recorded queries in microseconds.
    pub total_duration_micros: u64,
    /// Queries executed two or more times in the same scope (N+1 loop candidates).
    pub duplicates: Vec<DuplicateQueryWarning>,
}

impl QueryStats {
    /// Builds [`QueryStats`] from a list of recorded queries, computing total time
    /// and flagging any SQL statement executed `>= 2` times.
    pub fn from_queries(queries: Vec<RecordedQuery>) -> Self {
        let mut total_duration_micros = 0u64;
        let mut counts: BTreeMap<String, (usize, u64)> = BTreeMap::new();
        for q in &queries {
            total_duration_micros = total_duration_micros.saturating_add(q.duration_micros);
            let entry = counts.entry(q.sql.clone()).or_insert((0, 0));
            entry.0 += 1;
            entry.1 = entry.1.saturating_add(q.duration_micros);
        }

        let mut duplicates: Vec<DuplicateQueryWarning> = counts
            .into_iter()
            .filter(|(_, (count, _))| *count >= 2)
            .map(|(sql, (count, total_dur))| DuplicateQueryWarning {
                sql,
                count,
                total_duration_micros: total_dur,
            })
            .collect();
        duplicates.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.sql.cmp(&b.sql)));

        Self {
            queries,
            total_duration_micros,
            duplicates,
        }
    }

    /// Total number of SQL queries executed in this scope.
    pub fn total_queries(&self) -> usize {
        self.queries.len()
    }

    /// Returns `true` if any SQL statement was executed two or more times.
    pub fn has_duplicates(&self) -> bool {
        !self.duplicates.is_empty()
    }
}

tokio::task_local! {
    static ACTIVE_QUERY_LOG: Arc<Mutex<Vec<RecordedQuery>>>;
}

/// Returns `true` if the current async task has an active query log scope.
#[inline]
pub fn is_logging_active() -> bool {
    ACTIVE_QUERY_LOG.try_with(|_| ()).is_ok()
}

/// Records a completed SQL query if a [`with_query_log`] scope is active on the current task.
#[inline]
pub fn record_query(sql: &str, param_count: usize, duration_micros: u64) {
    let _ = ACTIVE_QUERY_LOG.try_with(|log| {
        if let Ok(mut guard) = log.lock() {
            guard.push(RecordedQuery {
                sql: sql.to_string(),
                param_count,
                duration_micros,
            });
        }
    });
}

/// Returns a clone of the active task-local query log handle, if any.
///
/// Used by `Router::handle_dispatch` when spawning a handler task via `tokio::spawn`
/// so queries executed inside the spawned task continue recording into the parent
/// request's query log.
#[inline]
pub fn current_query_log_handle() -> Option<Arc<Mutex<Vec<RecordedQuery>>>> {
    ACTIVE_QUERY_LOG.try_with(Arc::clone).ok()
}

/// Runs `fut` attached to an existing query log handle (or runs `fut` directly if `None`).
pub async fn with_inherited_query_log<F, T>(
    handle: Option<Arc<Mutex<Vec<RecordedQuery>>>>,
    fut: F,
) -> T
where
    F: Future<Output = T>,
{
    match handle {
        Some(log) => ACTIVE_QUERY_LOG.scope(log, fut).await,
        None => fut.await,
    }
}

/// Runs `fut` inside a task-local query logging scope and returns both the future's output
/// and the collected [`QueryStats`] (including N+1 duplicate query warnings).
pub async fn with_query_log<F, T>(fut: F) -> (T, QueryStats)
where
    F: Future<Output = T>,
{
    let log = Arc::new(Mutex::new(Vec::new()));
    let log_clone = Arc::clone(&log);
    let output = ACTIVE_QUERY_LOG.scope(log, fut).await;
    let queries = match log_clone.lock() {
        Ok(mut guard) => std::mem::take(&mut *guard),
        Err(_) => Vec::new(),
    };
    (output, QueryStats::from_queries(queries))
}
