use super::*;

#[test]
fn minimum_length_accepts_and_rejects() {
    let v = MinimumLength::default();
    assert!(v.validate("12345678", None).is_ok());
    assert!(v.validate("short", None).is_err());
    assert!(!v.help_text().is_empty());
}

#[test]
fn numeric_password_accepts_and_rejects() {
    let v = NumericPassword;
    assert!(v.validate("abc12345", None).is_ok());
    assert!(v.validate("12345678", None).is_err());
    assert!(!v.help_text().is_empty());
}

#[test]
fn user_attribute_similarity_accepts_and_rejects() {
    let v = UserAttributeSimilarity;
    let user = UserAttributes::new("AliceSmith", "alice.dev@example.com");

    assert!(v
        .validate("correct-horse-battery-staple", Some(&user))
        .is_ok());
    assert!(v.validate("my-alicesmith-pass", Some(&user)).is_err());
    assert!(v.validate("pass-htimsecila-1", Some(&user)).is_err());
    assert!(v.validate("pass-alice.dev-1", Some(&user)).is_err());
    assert!(v.validate("pass-ved.ecila-1", Some(&user)).is_err());
}

#[cfg(feature = "password-strength")]
#[test]
fn strength_validator_accepts_strong_and_rejects_weak() {
    let v = StrengthValidator::default();
    assert!(v.validate("correct-horse-battery-staple", None).is_ok());
    assert!(v.validate("password123", None).is_err());
    assert!(!v.help_text().is_empty());
}

#[test]
fn validate_password_collects_all_messages() {
    let validators = default_validators();
    let errs = validate_password("1234", None, &validators).unwrap_err();
    assert!(errs.len() >= 2, "expected multiple errors, got: {errs:?}");
    assert!(errs.iter().any(|e| e.contains("at least 8")));
    assert!(errs.iter().any(|e| e.contains("entirely numeric")));
}
