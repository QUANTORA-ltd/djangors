//! Password complexity validators modeled after Django's `password_validation`.

/// User attributes passed to password validators so they can reject passwords
/// that are too similar to the user's own details.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct UserAttributes {
    /// The user's username.
    pub username: String,
    /// The user's email address.
    pub email: String,
}

impl UserAttributes {
    /// Creates a [`UserAttributes`] value from `username` and `email`.
    pub fn new(username: impl Into<String>, email: impl Into<String>) -> Self {
        Self {
            username: username.into(),
            email: email.into(),
        }
    }
}

/// Validates a candidate password, optionally against the target user's attributes.
pub trait PasswordValidator: Send + Sync {
    /// Returns `Ok(())` if `password` satisfies this validator, or an error
    /// message explaining why it was rejected.
    fn validate(&self, password: &str, user: Option<&UserAttributes>) -> Result<(), String>;

    /// Human-readable requirement description suitable for UI help text.
    fn help_text(&self) -> String;
}

/// Rejects passwords shorter than the configured minimum character count (default 8).
#[derive(Debug, Clone, Copy)]
pub struct MinimumLength(pub usize);

impl Default for MinimumLength {
    fn default() -> Self {
        Self(8)
    }
}

impl MinimumLength {
    /// Creates a [`MinimumLength`] validator requiring at least `min_length` characters.
    pub fn new(min_length: usize) -> Self {
        Self(min_length)
    }
}

impl PasswordValidator for MinimumLength {
    fn validate(&self, password: &str, _user: Option<&UserAttributes>) -> Result<(), String> {
        if password.chars().count() < self.0 {
            return Err(format!(
                "This password is too short. It must contain at least {} characters.",
                self.0
            ));
        }
        Ok(())
    }

    fn help_text(&self) -> String {
        format!("Your password must contain at least {} characters.", self.0)
    }
}

/// Rejects passwords consisting entirely of ASCII digits.
#[derive(Debug, Clone, Copy, Default)]
pub struct NumericPassword;

impl PasswordValidator for NumericPassword {
    fn validate(&self, password: &str, _user: Option<&UserAttributes>) -> Result<(), String> {
        if !password.is_empty() && password.chars().all(|c| c.is_ascii_digit()) {
            return Err("This password is entirely numeric.".to_string());
        }
        Ok(())
    }

    fn help_text(&self) -> String {
        "Your password can't be entirely numeric.".to_string()
    }
}

/// Rejects passwords containing the username or email local-part (or their reverse),
/// case-insensitively, for values of 3 or more characters.
#[derive(Debug, Clone, Copy, Default)]
pub struct UserAttributeSimilarity;

impl PasswordValidator for UserAttributeSimilarity {
    fn validate(&self, password: &str, user: Option<&UserAttributes>) -> Result<(), String> {
        let Some(attrs) = user else {
            return Ok(());
        };
        let password_lower = password.to_lowercase();
        let email_local = attrs.email.split('@').next().unwrap_or("");
        let candidates = [attrs.username.as_str(), email_local];

        for candidate in candidates {
            let trimmed = candidate.trim();
            if trimmed.chars().count() < 3 {
                continue;
            }
            let lower = trimmed.to_lowercase();
            let reversed: String = lower.chars().rev().collect();
            if password_lower.contains(&lower) || password_lower.contains(&reversed) {
                return Err("The password is too similar to your personal information.".to_string());
            }
        }

        Ok(())
    }

    fn help_text(&self) -> String {
        "Your password can't be too similar to your other personal information.".to_string()
    }
}

/// Estimates password strength with `zxcvbn` (score 0..=4) and rejects passwords
/// below `min_score` (default 3).
#[cfg(feature = "password-strength")]
#[derive(Debug, Clone, Copy)]
pub struct StrengthValidator {
    /// Minimum acceptable `zxcvbn` score (`0..=4`).
    pub min_score: u8,
}

#[cfg(feature = "password-strength")]
impl Default for StrengthValidator {
    fn default() -> Self {
        Self { min_score: 3 }
    }
}

#[cfg(feature = "password-strength")]
impl StrengthValidator {
    /// Creates a [`StrengthValidator`] requiring at least `min_score` (0..=4).
    pub fn new(min_score: u8) -> Self {
        Self { min_score }
    }
}

#[cfg(feature = "password-strength")]
impl PasswordValidator for StrengthValidator {
    fn validate(&self, password: &str, user: Option<&UserAttributes>) -> Result<(), String> {
        let mut inputs = Vec::new();
        if let Some(attrs) = user {
            if !attrs.username.is_empty() {
                inputs.push(attrs.username.as_str());
            }
            if !attrs.email.is_empty() {
                inputs.push(attrs.email.as_str());
            }
        }

        let estimate = zxcvbn::zxcvbn(password, &inputs);
        let score = u8::from(estimate.score());

        if score < self.min_score {
            if let Some(warning) = estimate.feedback().and_then(|feedback| feedback.warning()) {
                return Err(format!("This password is too weak: {warning}"));
            }
            return Err("This password is too weak or too common.".to_string());
        }

        Ok(())
    }

    fn help_text(&self) -> String {
        "Your password must be strong and not easily guessable.".to_string()
    }
}

/// Returns the default set of password validators: [`MinimumLength`],
/// [`NumericPassword`], [`UserAttributeSimilarity`], and (when the
/// `password-strength` feature is enabled) [`StrengthValidator`].
pub fn default_validators() -> Vec<Box<dyn PasswordValidator>> {
    let mut validators: Vec<Box<dyn PasswordValidator>> = vec![
        Box::new(MinimumLength::default()),
        Box::new(NumericPassword),
        Box::new(UserAttributeSimilarity),
    ];
    #[cfg(feature = "password-strength")]
    {
        validators.push(Box::new(StrengthValidator::default()));
    }
    validators
}

/// Runs all `validators` against `password` and collects every failure message.
pub fn validate_password(
    password: &str,
    user: Option<&UserAttributes>,
    validators: &[Box<dyn PasswordValidator>],
) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();
    for validator in validators {
        if let Err(msg) = validator.validate(password, user) {
            errors.push(msg);
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

#[cfg(test)]
#[path = "password_validation_tests.rs"]
mod tests;
