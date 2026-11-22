//! Extensible System Check Framework.
//!
//! Mirrors Django's system check framework (`django.core.checks`): crates and
//! applications can register check functions via `inventory::submit!` with
//! [`SystemCheckRegistration`], and [`run_system_checks`] collects and filters
//! all reported [`CheckMessage`]s against `silenced_system_checks`.

/// Severity level for a system check finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CheckLevel {
    /// Diagnostic message useful during development.
    Debug,
    /// Informational note.
    Info,
    /// Potential misconfiguration that should be reviewed.
    Warning,
    /// Configuration error that may break functionality.
    Error,
    /// Severe misconfiguration or security hazard preventing safe startup.
    Critical,
}

/// A single diagnostic finding produced by a system check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckMessage {
    /// Unique identifier for this check (e.g. `"security.W009"`), used to silence it.
    pub id: String,
    /// Severity level of the finding.
    pub level: CheckLevel,
    /// Human-readable description of the problem.
    pub message: String,
    /// Optional actionable hint explaining how to resolve the finding.
    pub hint: Option<String>,
}

impl CheckMessage {
    /// Creates a new [`CheckMessage`] with no hint.
    pub fn new(id: impl Into<String>, level: CheckLevel, message: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            level,
            message: message.into(),
            hint: None,
        }
    }

    /// Creates a [`CheckLevel::Debug`] message.
    pub fn debug(id: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(id, CheckLevel::Debug, message)
    }

    /// Creates a [`CheckLevel::Info`] message.
    pub fn info(id: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(id, CheckLevel::Info, message)
    }

    /// Creates a [`CheckLevel::Warning`] message.
    pub fn warning(id: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(id, CheckLevel::Warning, message)
    }

    /// Creates a [`CheckLevel::Error`] message.
    pub fn error(id: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(id, CheckLevel::Error, message)
    }

    /// Creates a [`CheckLevel::Critical`] message.
    pub fn critical(id: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(id, CheckLevel::Critical, message)
    }

    /// Attaches an actionable remediation hint to this message.
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
}

/// A compile-time registration record for a system check function.
///
/// Register checks from any crate using:
/// ```rust
/// djangors_core::inventory::submit! {
///     djangors_core::checks::SystemCheckRegistration {
///         name: "my_app.check_config",
///         check: || vec![],
///     }
/// }
/// ```
pub struct SystemCheckRegistration {
    /// Descriptive name of the registered check.
    pub name: &'static str,
    /// Function returning any [`CheckMessage`]s discovered.
    pub check: fn() -> Vec<CheckMessage>,
}

inventory::collect!(SystemCheckRegistration);

/// Executes all registered [`SystemCheckRegistration`] checks and returns
/// all messages whose `id` is not listed in `silenced`.
pub fn run_system_checks(silenced: &[String]) -> Vec<CheckMessage> {
    let mut results = Vec::new();
    for reg in inventory::iter::<SystemCheckRegistration>() {
        for msg in (reg.check)() {
            if !silenced.iter().any(|s| s == &msg.id) {
                results.push(msg);
            }
        }
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_check() -> Vec<CheckMessage> {
        vec![
            CheckMessage::warning("test.W001", "sample warning").with_hint("fix it"),
            CheckMessage::error("test.E001", "sample error"),
        ]
    }

    inventory::submit! {
        SystemCheckRegistration {
            name: "djangors_core.tests.sample_check",
            check: sample_check,
        }
    }

    #[test]
    fn run_system_checks_collects_and_silences() {
        let all = run_system_checks(&[]);
        assert!(all
            .iter()
            .any(|m| m.id == "test.W001" && m.hint.as_deref() == Some("fix it")));
        assert!(all
            .iter()
            .any(|m| m.id == "test.E001" && m.level == CheckLevel::Error));

        let filtered = run_system_checks(&["test.W001".to_string()]);
        assert!(!filtered.iter().any(|m| m.id == "test.W001"));
        assert!(filtered.iter().any(|m| m.id == "test.E001"));
    }
}
