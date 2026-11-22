use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::time::Duration;

use crate::error::FieldError;

/// Trait implemented by all form fields to parse and validate input data.
pub trait FormField {
    /// The resulting typed value after parsing and validation.
    type Value;

    /// Parse and validate a raw string form value (as it would arrive
    /// from an HTTP POST body — always Option<&str>, since a field might
    /// be entirely absent from the submitted data) into a typed value,
    /// or a field-level error.
    fn clean(&self, raw: Option<&str>) -> Result<Self::Value, FieldError>;
}

/// A text field that validates string input.
///
/// # Examples
///
/// ```
/// # use djangors_forms::{CharField, FormField};
/// let field = CharField { max_length: Some(5), required: true };
///
/// // Required and present
/// assert_eq!(field.clean(Some("hello")), Ok("hello".to_string()));
///
/// // Required but empty or absent leads to an error
/// assert!(field.clean(None).is_err());
/// assert!(field.clean(Some("")).is_err());
///
/// // Max length constraint
/// assert!(field.clean(Some("too_long")).is_err());
///
/// // Whitespace is preserved (no automatic trimming)
/// let not_required = CharField { max_length: None, required: false };
/// assert_eq!(not_required.clean(Some("  spaces  ")), Ok("  spaces  ".to_string()));
/// ```
pub struct CharField {
    /// The maximum allowed length in characters.
    pub max_length: Option<usize>,
    /// Whether the field is required to have a non-empty value.
    pub required: bool,
}

impl FormField for CharField {
    type Value = String;

    fn clean(&self, raw: Option<&str>) -> Result<Self::Value, FieldError> {
        match raw {
            None | Some("") => {
                if self.required {
                    Err(FieldError(vec!["This field is required.".to_string()]))
                } else {
                    Ok(String::new())
                }
            }
            Some(s) => {
                if let Some(limit) = self.max_length {
                    if s.chars().count() > limit {
                        return Err(FieldError(vec![format!(
                            "Ensure this value has at most {} characters.",
                            limit
                        )]));
                    }
                }
                Ok(s.to_string())
            }
        }
    }
}

/// An integer field that validates signed 64-bit integer values.
///
/// # Examples
///
/// ```
/// # use djangors_forms::{IntegerField, FormField};
/// let field = IntegerField { min: Some(1), max: Some(10), required: true };
///
/// // Valid integer in range
/// assert_eq!(field.clean(Some("5")), Ok(Some(5)));
///
/// // Valid integer at boundaries (boundary-inclusive)
/// assert_eq!(field.clean(Some("1")), Ok(Some(1)));
/// assert_eq!(field.clean(Some("10")), Ok(Some(10)));
///
/// // Below minimum or above maximum
/// assert!(field.clean(Some("0")).is_err());
/// assert!(field.clean(Some("11")).is_err());
///
/// // Invalid integer format
/// assert!(field.clean(Some("not-a-number")).is_err());
///
/// // Absent/optional handling
/// let optional = IntegerField { min: None, max: None, required: false };
/// assert_eq!(optional.clean(None), Ok(None));
/// assert_eq!(optional.clean(Some("")), Ok(None));
/// ```
pub struct IntegerField {
    /// The minimum allowed value (inclusive).
    pub min: Option<i64>,
    /// The maximum allowed value (inclusive).
    pub max: Option<i64>,
    /// Whether the field is required.
    pub required: bool,
}

impl FormField for IntegerField {
    type Value = Option<i64>;

    fn clean(&self, raw: Option<&str>) -> Result<Self::Value, FieldError> {
        match raw {
            None | Some("") => {
                if self.required {
                    Err(FieldError(vec!["This field is required.".to_string()]))
                } else {
                    Ok(None)
                }
            }
            Some(s) => {
                let val = s
                    .parse::<i64>()
                    .map_err(|_| FieldError(vec!["Enter a whole number.".to_string()]))?;

                if let Some(minimum) = self.min {
                    if val < minimum {
                        return Err(FieldError(vec![format!(
                            "Ensure this value is greater than or equal to {}.",
                            minimum
                        )]));
                    }
                }

                if let Some(maximum) = self.max {
                    if val > maximum {
                        return Err(FieldError(vec![format!(
                            "Ensure this value is less than or equal to {}.",
                            maximum
                        )]));
                    }
                }

                Ok(Some(val))
            }
        }
    }
}

/// A boolean field that validates boolean states (typically checkbox inputs).
///
/// # Django-Specific Semantics
///
/// HTML checkboxes are omitted from HTTP POST submissions if unchecked.
/// Thus, `clean(None)` represents `false`.
///
/// If `required` is true, the field validates that the value is `true` (e.g. for "I agree to terms" checkboxes).
///
/// # Examples
///
/// ```
/// # use djangors_forms::{BooleanField, FormField};
/// let field = BooleanField { required: true };
///
/// // Truthy values are accepted
/// assert_eq!(field.clean(Some("on")), Ok(true));
/// assert_eq!(field.clean(Some("true")), Ok(true));
/// assert_eq!(field.clean(Some("1")), Ok(true));
///
/// // Falsy/absent values error if required: true
/// assert!(field.clean(None).is_err());
/// assert!(field.clean(Some("false")).is_err());
///
/// // If not required, false/absent value is accepted and returns false
/// let optional = BooleanField { required: false };
/// assert_eq!(optional.clean(None), Ok(false));
/// assert_eq!(optional.clean(Some("false")), Ok(false));
/// ```
pub struct BooleanField {
    /// Whether the field value must clean to `true`.
    pub required: bool,
}

impl FormField for BooleanField {
    type Value = bool;

    fn clean(&self, raw: Option<&str>) -> Result<Self::Value, FieldError> {
        let truthy = matches!(raw, Some("on" | "true" | "1"));

        if self.required && !truthy {
            Err(FieldError(vec!["This field is required.".to_string()]))
        } else {
            Ok(truthy)
        }
    }
}

/// An email field that validates a simple email format.
///
/// # Examples
///
/// ```
/// # use djangors_forms::{EmailField, FormField};
/// let field = EmailField { required: true };
///
/// // Valid emails
/// assert_eq!(field.clean(Some("user@example.com")), Ok("user@example.com".to_string()));
/// assert_eq!(field.clean(Some("a@b.c")), Ok("a@b.c".to_string()));
///
/// // Invalid email formats
/// assert!(field.clean(Some("no-at-sign")).is_err());
/// assert!(field.clean(Some("@domain.com")).is_err());
/// assert!(field.clean(Some("user@.com")).is_err());
/// assert!(field.clean(Some("user@domain.")).is_err());
/// assert!(field.clean(Some("user@domain..com")).is_err());
/// ```
pub struct EmailField {
    /// Whether the field is required.
    pub required: bool,
}

impl FormField for EmailField {
    type Value = String;

    fn clean(&self, raw: Option<&str>) -> Result<Self::Value, FieldError> {
        match raw {
            None | Some("") => {
                if self.required {
                    Err(FieldError(vec!["This field is required.".to_string()]))
                } else {
                    Ok(String::new())
                }
            }
            Some(s) => {
                let parts: Vec<&str> = s.split('@').collect();
                if parts.len() != 2 {
                    return Err(FieldError(vec!["Enter a valid email address.".to_string()]));
                }
                let local = parts[0];
                let domain = parts[1];

                if local.is_empty()
                    || domain.is_empty()
                    || !domain.contains('.')
                    || domain.starts_with('.')
                    || domain.ends_with('.')
                    || domain.contains("..")
                {
                    return Err(FieldError(vec!["Enter a valid email address.".to_string()]));
                }

                Ok(s.to_string())
            }
        }
    }
}

/// A text field that validates input against a fixed set of choices.
///
/// # Examples
///
/// ```
/// # use djangors_forms::{ChoiceField, FormField};
/// let field = ChoiceField { choices: &["draft", "published", "archived"], required: true };
///
/// assert_eq!(field.clean(Some("draft")), Ok("draft".to_string()));
/// assert!(field.clean(Some("invalid")).is_err());
/// assert!(field.clean(None).is_err());
/// ```
pub struct ChoiceField {
    /// The set of allowed values.
    pub choices: &'static [&'static str],
    /// Whether the field is required.
    pub required: bool,
}

impl FormField for ChoiceField {
    type Value = String;

    fn clean(&self, raw: Option<&str>) -> Result<Self::Value, FieldError> {
        match raw {
            None | Some("") => {
                if self.required {
                    Err(FieldError(vec!["This field is required.".to_string()]))
                } else {
                    Ok(String::new())
                }
            }
            Some(s) => {
                if self.choices.contains(&s) {
                    Ok(s.to_string())
                } else {
                    let valid = self.choices.join(", ");
                    Err(FieldError(vec![format!(
                        "Select a valid choice. {} is not one of the available choices.",
                        valid
                    )]))
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Standard Django form fields
// ---------------------------------------------------------------------------

/// A floating-point number field that validates finite `f64` values with optional bounds.
#[derive(Debug, Clone, PartialEq)]
pub struct FloatField {
    /// Minimum allowed value (inclusive).
    pub min_value: Option<f64>,
    /// Maximum allowed value (inclusive).
    pub max_value: Option<f64>,
    /// Whether the field is required.
    pub required: bool,
    /// Optional human-readable label for the field.
    pub label: Option<String>,
    /// Optional help text rendered alongside the field.
    pub help_text: Option<String>,
    /// Optional initial value string.
    pub initial: Option<String>,
}

impl Default for FloatField {
    fn default() -> Self {
        Self {
            min_value: None,
            max_value: None,
            required: true,
            label: None,
            help_text: None,
            initial: None,
        }
    }
}

impl FloatField {
    /// Creates a new required `FloatField` with no bounds.
    pub fn new() -> Self {
        Self::default()
    }
}

impl FormField for FloatField {
    type Value = Option<f64>;

    fn clean(&self, raw: Option<&str>) -> Result<Self::Value, FieldError> {
        let s = match raw.map(str::trim) {
            None | Some("") => {
                return if self.required {
                    Err(FieldError(vec!["This field is required.".to_string()]))
                } else {
                    Ok(None)
                };
            }
            Some(v) => v,
        };

        let val = s
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite())
            .ok_or_else(|| FieldError(vec!["Enter a number.".to_string()]))?;

        if let Some(min) = self.min_value {
            if val < min {
                return Err(FieldError(vec![format!(
                    "Ensure this value is greater than or equal to {}.",
                    min
                )]));
            }
        }
        if let Some(max) = self.max_value {
            if val > max {
                return Err(FieldError(vec![format!(
                    "Ensure this value is less than or equal to {}.",
                    max
                )]));
            }
        }

        Ok(Some(val))
    }
}

/// A fixed-precision decimal field that validates decimal strings with optional
/// `max_digits`, `decimal_places`, `min_value`, and `max_value` constraints.
#[derive(Debug, Clone, PartialEq)]
pub struct DecimalField {
    /// Maximum total number of digits allowed (excluding sign and decimal point).
    pub max_digits: Option<usize>,
    /// Maximum number of digits allowed after the decimal point.
    pub decimal_places: Option<usize>,
    /// Minimum allowed numeric value (inclusive).
    pub min_value: Option<f64>,
    /// Maximum allowed numeric value (inclusive).
    pub max_value: Option<f64>,
    /// Whether the field is required.
    pub required: bool,
    /// Optional human-readable label for the field.
    pub label: Option<String>,
    /// Optional help text rendered alongside the field.
    pub help_text: Option<String>,
    /// Optional initial value string.
    pub initial: Option<String>,
}

impl Default for DecimalField {
    fn default() -> Self {
        Self {
            max_digits: None,
            decimal_places: None,
            min_value: None,
            max_value: None,
            required: true,
            label: None,
            help_text: None,
            initial: None,
        }
    }
}

impl DecimalField {
    /// Creates a new required `DecimalField` with no digit or range limits.
    pub fn new() -> Self {
        Self::default()
    }
}

impl FormField for DecimalField {
    type Value = Option<String>;

    fn clean(&self, raw: Option<&str>) -> Result<Self::Value, FieldError> {
        let s = match raw.map(str::trim) {
            None | Some("") => {
                return if self.required {
                    Err(FieldError(vec!["This field is required.".to_string()]))
                } else {
                    Ok(None)
                };
            }
            Some(v) => v,
        };

        let unsigned = s
            .strip_prefix('-')
            .or_else(|| s.strip_prefix('+'))
            .unwrap_or(s);

        let parts: Vec<&str> = unsigned.split('.').collect();
        if parts.len() > 2 {
            return Err(FieldError(vec!["Enter a number.".to_string()]));
        }

        let int_part = parts[0];
        let frac_part = parts.get(1).copied().unwrap_or("");

        if (int_part.is_empty() && frac_part.is_empty())
            || !int_part.chars().all(|c| c.is_ascii_digit())
            || !frac_part.chars().all(|c| c.is_ascii_digit())
        {
            return Err(FieldError(vec!["Enter a number.".to_string()]));
        }

        let frac_digits = frac_part.len();
        let significant_int = int_part.trim_start_matches('0').len();
        let total_digits = (significant_int + frac_digits).max(1);

        if let Some(dp) = self.decimal_places {
            if frac_digits > dp {
                return Err(FieldError(vec![format!(
                    "Ensure that there are no more than {} decimal places.",
                    dp
                )]));
            }
        }

        if let Some(md) = self.max_digits {
            if total_digits > md {
                return Err(FieldError(vec![format!(
                    "Ensure that there are no more than {} digits in total.",
                    md
                )]));
            }
            if let Some(dp) = self.decimal_places {
                if md >= dp && significant_int > md - dp {
                    return Err(FieldError(vec![format!(
                        "Ensure that there are no more than {} digits before the decimal point.",
                        md - dp
                    )]));
                }
            }
        }

        let numeric = s
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite())
            .ok_or_else(|| FieldError(vec!["Enter a number.".to_string()]))?;

        if let Some(min) = self.min_value {
            if numeric < min {
                return Err(FieldError(vec![format!(
                    "Ensure this value is greater than or equal to {}.",
                    min
                )]));
            }
        }
        if let Some(max) = self.max_value {
            if numeric > max {
                return Err(FieldError(vec![format!(
                    "Ensure this value is less than or equal to {}.",
                    max
                )]));
            }
        }

        Ok(Some(s.to_string()))
    }
}

fn is_leap_year(year: u32) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(year) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

fn is_valid_calendar_date(year: u32, month: u32, day: u32) -> bool {
    (1..=9999).contains(&year)
        && (1..=12).contains(&month)
        && day >= 1
        && day <= days_in_month(year, month)
}

fn parse_ymd_iso(s: &str) -> Option<(u32, u32, u32)> {
    let mut parts = s.split('-');
    let y_str = parts.next()?;
    let m_str = parts.next()?;
    let d_str = parts.next()?;
    if parts.next().is_some() || y_str.len() != 4 || m_str.len() != 2 || d_str.len() != 2 {
        return None;
    }
    let y: u32 = y_str.parse().ok()?;
    let m: u32 = m_str.parse().ok()?;
    let d: u32 = d_str.parse().ok()?;
    is_valid_calendar_date(y, m, d).then_some((y, m, d))
}

fn parse_date_with_format(s: &str, fmt: &str) -> Option<(u32, u32, u32)> {
    match fmt {
        "%Y-%m-%d" => parse_ymd_iso(s),
        "%Y/%m/%d" => {
            let parts: Vec<&str> = s.split('/').collect();
            if parts.len() != 3 || parts[0].len() != 4 {
                return None;
            }
            let (y, m, d) = (
                parts[0].parse().ok()?,
                parts[1].parse().ok()?,
                parts[2].parse().ok()?,
            );
            is_valid_calendar_date(y, m, d).then_some((y, m, d))
        }
        "%m/%d/%Y" => {
            let parts: Vec<&str> = s.split('/').collect();
            if parts.len() != 3 || parts[2].len() != 4 {
                return None;
            }
            let (m, d, y) = (
                parts[0].parse().ok()?,
                parts[1].parse().ok()?,
                parts[2].parse().ok()?,
            );
            is_valid_calendar_date(y, m, d).then_some((y, m, d))
        }
        "%d/%m/%Y" => {
            let parts: Vec<&str> = s.split('/').collect();
            if parts.len() != 3 || parts[2].len() != 4 {
                return None;
            }
            let (d, m, y) = (
                parts[0].parse().ok()?,
                parts[1].parse().ok()?,
                parts[2].parse().ok()?,
            );
            is_valid_calendar_date(y, m, d).then_some((y, m, d))
        }
        "%d-%m-%Y" => {
            let parts: Vec<&str> = s.split('-').collect();
            if parts.len() != 3 || parts[2].len() != 4 {
                return None;
            }
            let (d, m, y) = (
                parts[0].parse().ok()?,
                parts[1].parse().ok()?,
                parts[2].parse().ok()?,
            );
            is_valid_calendar_date(y, m, d).then_some((y, m, d))
        }
        "%d.%m.%Y" => {
            let parts: Vec<&str> = s.split('.').collect();
            if parts.len() != 3 || parts[2].len() != 4 {
                return None;
            }
            let (d, m, y) = (
                parts[0].parse().ok()?,
                parts[1].parse().ok()?,
                parts[2].parse().ok()?,
            );
            is_valid_calendar_date(y, m, d).then_some((y, m, d))
        }
        _ => None,
    }
}

fn parse_hms_time(s: &str) -> Option<(u32, u32, u32)> {
    let (core, frac) = match s.split_once('.') {
        Some((c, f)) => {
            if f.is_empty() || f.len() > 9 || !f.chars().all(|ch| ch.is_ascii_digit()) {
                return None;
            }
            (c, Some(f))
        }
        None => (s, None),
    };

    let parts: Vec<&str> = core.split(':').collect();
    if parts.len() != 2 && parts.len() != 3 {
        return None;
    }
    if parts.len() == 2 && frac.is_some() {
        return None;
    }
    if parts.iter().any(|p| p.len() != 2) {
        return None;
    }

    let h: u32 = parts[0].parse().ok()?;
    let m: u32 = parts[1].parse().ok()?;
    let sec: u32 = if parts.len() == 3 {
        parts[2].parse().ok()?
    } else {
        0
    };

    if h <= 23 && m <= 59 && sec <= 59 {
        Some((h, m, sec))
    } else {
        None
    }
}

/// A calendar date field validating `YYYY-MM-DD` (or configured `input_formats`)
/// and normalizing to ISO `YYYY-MM-DD`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DateField {
    /// Accepted date format patterns (default: `["%Y-%m-%d"]`).
    ///
    /// Supported patterns: `"%Y-%m-%d"`, `"%Y/%m/%d"`, `"%m/%d/%Y"`,
    /// `"%d/%m/%Y"`, `"%d-%m-%Y"`, `"%d.%m.%Y"`.
    pub input_formats: Vec<String>,
    /// Whether the field is required.
    pub required: bool,
    /// Optional human-readable label for the field.
    pub label: Option<String>,
    /// Optional help text rendered alongside the field.
    pub help_text: Option<String>,
    /// Optional initial value string.
    pub initial: Option<String>,
}

impl Default for DateField {
    fn default() -> Self {
        Self {
            input_formats: vec!["%Y-%m-%d".to_string()],
            required: true,
            label: None,
            help_text: None,
            initial: None,
        }
    }
}

impl DateField {
    /// Creates a new required `DateField` accepting `YYYY-MM-DD`.
    pub fn new() -> Self {
        Self::default()
    }
}

impl FormField for DateField {
    type Value = Option<String>;

    fn clean(&self, raw: Option<&str>) -> Result<Self::Value, FieldError> {
        let s = match raw.map(str::trim) {
            None | Some("") => {
                return if self.required {
                    Err(FieldError(vec!["This field is required.".to_string()]))
                } else {
                    Ok(None)
                };
            }
            Some(v) => v,
        };

        let formats: &[String] = if self.input_formats.is_empty() {
            return match parse_ymd_iso(s) {
                Some((y, m, d)) => Ok(Some(format!("{:04}-{:02}-{:02}", y, m, d))),
                None => Err(FieldError(vec!["Enter a valid date.".to_string()])),
            };
        } else {
            &self.input_formats
        };

        for fmt in formats {
            if let Some((y, m, d)) = parse_date_with_format(s, fmt) {
                return Ok(Some(format!("{:04}-{:02}-{:02}", y, m, d)));
            }
        }

        Err(FieldError(vec!["Enter a valid date.".to_string()]))
    }
}

/// A date-and-time field validating ISO-8601 (`YYYY-MM-DDTHH:MM[:SS[.fff]][Z|±HH:MM]`)
/// and space-separated (`YYYY-MM-DD HH:MM[:SS]`) timestamps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DateTimeField {
    /// Whether the field is required.
    pub required: bool,
    /// Optional human-readable label for the field.
    pub label: Option<String>,
    /// Optional help text rendered alongside the field.
    pub help_text: Option<String>,
    /// Optional initial value string.
    pub initial: Option<String>,
}

impl Default for DateTimeField {
    fn default() -> Self {
        Self {
            required: true,
            label: None,
            help_text: None,
            initial: None,
        }
    }
}

impl DateTimeField {
    /// Creates a new required `DateTimeField`.
    pub fn new() -> Self {
        Self::default()
    }
}

fn strip_and_validate_tz(time_with_tz: &str) -> Option<&str> {
    if let Some(stripped) = time_with_tz
        .strip_suffix('Z')
        .or_else(|| time_with_tz.strip_suffix('z'))
    {
        return Some(stripped);
    }

    if let Some(idx) = time_with_tz.rfind(['+', '-']) {
        let time_part = &time_with_tz[..idx];
        let tz_part = &time_with_tz[idx + 1..];
        let (tz_h, tz_m) = if let Some((h, m)) = tz_part.split_once(':') {
            if h.len() != 2 || m.len() != 2 {
                return None;
            }
            (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?)
        } else if tz_part.len() == 4 {
            (
                tz_part[..2].parse::<u32>().ok()?,
                tz_part[2..].parse::<u32>().ok()?,
            )
        } else {
            return None;
        };

        if tz_h <= 23 && tz_m <= 59 {
            return Some(time_part);
        }
        return None;
    }

    Some(time_with_tz)
}

impl FormField for DateTimeField {
    type Value = Option<String>;

    fn clean(&self, raw: Option<&str>) -> Result<Self::Value, FieldError> {
        let s = match raw.map(str::trim) {
            None | Some("") => {
                return if self.required {
                    Err(FieldError(vec!["This field is required.".to_string()]))
                } else {
                    Ok(None)
                };
            }
            Some(v) => v,
        };

        let (date_str, time_tz_str) = s
            .split_once('T')
            .or_else(|| s.split_once('t'))
            .or_else(|| s.split_once(' '))
            .ok_or_else(|| FieldError(vec!["Enter a valid date/time.".to_string()]))?;

        if parse_ymd_iso(date_str).is_none() {
            return Err(FieldError(vec!["Enter a valid date/time.".to_string()]));
        }

        let time_str = strip_and_validate_tz(time_tz_str)
            .ok_or_else(|| FieldError(vec!["Enter a valid date/time.".to_string()]))?;

        if parse_hms_time(time_str).is_none() {
            return Err(FieldError(vec!["Enter a valid date/time.".to_string()]));
        }

        Ok(Some(s.to_string()))
    }
}

/// A time-of-day field validating `HH:MM` and `HH:MM:SS[.ffffff]` strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeField {
    /// Whether the field is required.
    pub required: bool,
    /// Optional human-readable label for the field.
    pub label: Option<String>,
    /// Optional help text rendered alongside the field.
    pub help_text: Option<String>,
    /// Optional initial value string.
    pub initial: Option<String>,
}

impl Default for TimeField {
    fn default() -> Self {
        Self {
            required: true,
            label: None,
            help_text: None,
            initial: None,
        }
    }
}

impl TimeField {
    /// Creates a new required `TimeField`.
    pub fn new() -> Self {
        Self::default()
    }
}

impl FormField for TimeField {
    type Value = Option<String>;

    fn clean(&self, raw: Option<&str>) -> Result<Self::Value, FieldError> {
        let s = match raw.map(str::trim) {
            None | Some("") => {
                return if self.required {
                    Err(FieldError(vec!["This field is required.".to_string()]))
                } else {
                    Ok(None)
                };
            }
            Some(v) => v,
        };

        if parse_hms_time(s).is_some() {
            Ok(Some(s.to_string()))
        } else {
            Err(FieldError(vec!["Enter a valid time.".to_string()]))
        }
    }
}

/// A duration field parsing seconds (`"90"`, `"1.5"`), `HH:MM:SS` / `[D ]HH:MM:SS`,
/// or ISO-8601 duration strings (`"PT1H30M15S"`, `"P2DT3H"`) into [`std::time::Duration`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurationField {
    /// Whether the field is required.
    pub required: bool,
    /// Optional human-readable label for the field.
    pub label: Option<String>,
    /// Optional help text rendered alongside the field.
    pub help_text: Option<String>,
    /// Optional initial value string.
    pub initial: Option<String>,
}

impl Default for DurationField {
    fn default() -> Self {
        Self {
            required: true,
            label: None,
            help_text: None,
            initial: None,
        }
    }
}

impl DurationField {
    /// Creates a new required `DurationField`.
    pub fn new() -> Self {
        Self::default()
    }
}

fn parse_iso8601_duration(s: &str) -> Option<Duration> {
    let rest = s.strip_prefix('P')?;
    if rest.is_empty() {
        return None;
    }

    let (date_part, time_part) = match rest.split_once('T') {
        Some((d, t)) => {
            if t.is_empty() {
                return None;
            }
            (d, Some(t))
        }
        None => (rest, None),
    };

    let mut total_secs: f64 = 0.0;
    let mut found_any = false;

    if !date_part.is_empty() {
        let mut num_buf = String::new();
        for ch in date_part.chars() {
            if ch.is_ascii_digit() {
                num_buf.push(ch);
            } else if ch == 'D' && !num_buf.is_empty() {
                let days: u64 = num_buf.parse().ok()?;
                total_secs += (days as f64) * 86_400.0;
                num_buf.clear();
                found_any = true;
            } else {
                return None;
            }
        }
        if !num_buf.is_empty() {
            return None;
        }
    }

    if let Some(tp) = time_part {
        let mut num_buf = String::new();
        for ch in tp.chars() {
            if ch.is_ascii_digit() || ch == '.' {
                num_buf.push(ch);
            } else if !num_buf.is_empty() {
                let val: f64 = num_buf.parse().ok()?;
                if !val.is_finite() || val < 0.0 {
                    return None;
                }
                match ch {
                    'H' => total_secs += val * 3600.0,
                    'M' => total_secs += val * 60.0,
                    'S' => total_secs += val,
                    _ => return None,
                }
                num_buf.clear();
                found_any = true;
            } else {
                return None;
            }
        }
        if !num_buf.is_empty() {
            return None;
        }
    }

    if found_any && total_secs.is_finite() && total_secs >= 0.0 {
        Some(Duration::from_secs_f64(total_secs))
    } else {
        None
    }
}

fn parse_duration_str(s: &str) -> Option<Duration> {
    if s.starts_with('P') {
        return parse_iso8601_duration(s);
    }

    // Plain numeric seconds (e.g. "90" or "1.5")
    if !s.contains(':') && !s.contains(' ') {
        let secs: f64 = s.parse().ok()?;
        if secs.is_finite() && secs >= 0.0 {
            return Some(Duration::from_secs_f64(secs));
        }
        return None;
    }

    // Optional leading days: "D HH:MM:SS"
    let (days, hms) = match s.split_once(' ') {
        Some((d_str, rest)) => (d_str.trim().parse::<u64>().ok()?, rest.trim()),
        None => (0u64, s),
    };

    let parts: Vec<&str> = hms.split(':').collect();
    if parts.len() != 3 {
        return None;
    }

    let hours: u64 = parts[0].parse().ok()?;
    let minutes: u64 = parts[1].parse().ok()?;
    let seconds: f64 = parts[2].parse().ok()?;

    if minutes >= 60 || !seconds.is_finite() || seconds < 0.0 || seconds >= 60.0 {
        return None;
    }

    let total =
        (days as f64) * 86_400.0 + (hours as f64) * 3600.0 + (minutes as f64) * 60.0 + seconds;
    total.is_finite().then(|| Duration::from_secs_f64(total))
}

impl FormField for DurationField {
    type Value = Option<Duration>;

    fn clean(&self, raw: Option<&str>) -> Result<Self::Value, FieldError> {
        let s = match raw.map(str::trim) {
            None | Some("") => {
                return if self.required {
                    Err(FieldError(vec!["This field is required.".to_string()]))
                } else {
                    Ok(None)
                };
            }
            Some(v) => v,
        };

        parse_duration_str(s)
            .map(Some)
            .ok_or_else(|| FieldError(vec!["Enter a valid duration.".to_string()]))
    }
}

/// A URL field validating `http://` and `https://` URLs with optional `max_length`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UrlField {
    /// Maximum allowed length in characters.
    pub max_length: Option<usize>,
    /// Whether the field is required.
    pub required: bool,
    /// Optional human-readable label for the field.
    pub label: Option<String>,
    /// Optional help text rendered alongside the field.
    pub help_text: Option<String>,
    /// Optional initial value string.
    pub initial: Option<String>,
}

impl Default for UrlField {
    fn default() -> Self {
        Self {
            max_length: None,
            required: true,
            label: None,
            help_text: None,
            initial: None,
        }
    }
}

impl UrlField {
    /// Creates a new required `UrlField`.
    pub fn new() -> Self {
        Self::default()
    }
}

fn is_valid_http_url(s: &str) -> bool {
    if s.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return false;
    }

    let rest = if let Some(r) = s.strip_prefix("http://") {
        r
    } else if let Some(r) = s.strip_prefix("https://") {
        r
    } else {
        return false;
    };

    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() {
        return false;
    }

    // Strip optional userinfo@
    let host_port = match authority.rsplit_once('@') {
        Some((_, hp)) => hp,
        None => authority,
    };

    if host_port.is_empty() {
        return false;
    }

    // Handle IPv6 literal host like [::1]:8080
    if let Some(after_bracket) = host_port.strip_prefix('[') {
        let Some((ipv6_str, remainder)) = after_bracket.split_once(']') else {
            return false;
        };
        if ipv6_str.parse::<Ipv6Addr>().is_err() {
            return false;
        }
        if let Some(port_str) = remainder.strip_prefix(':') {
            return port_str.parse::<u16>().is_ok();
        }
        return remainder.is_empty();
    }

    let (host, port_opt) = match host_port.rsplit_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (host_port, None),
    };

    if let Some(p) = port_opt {
        if p.parse::<u16>().is_err() {
            return false;
        }
    }

    if host.is_empty() || host.starts_with('.') || host.ends_with('.') || host.contains("..") {
        return false;
    }

    if host.eq_ignore_ascii_case("localhost") || host.parse::<Ipv4Addr>().is_ok() {
        return true;
    }

    host.contains('.')
        && host.split('.').all(|label| {
            !label.is_empty() && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
}

impl FormField for UrlField {
    type Value = String;

    fn clean(&self, raw: Option<&str>) -> Result<Self::Value, FieldError> {
        let s = match raw.map(str::trim) {
            None | Some("") => {
                return if self.required {
                    Err(FieldError(vec!["This field is required.".to_string()]))
                } else {
                    Ok(String::new())
                };
            }
            Some(v) => v,
        };

        if let Some(limit) = self.max_length {
            if s.chars().count() > limit {
                return Err(FieldError(vec![format!(
                    "Ensure this value has at most {} characters.",
                    limit
                )]));
            }
        }

        if is_valid_http_url(s) {
            Ok(s.to_string())
        } else {
            Err(FieldError(vec!["Enter a valid URL.".to_string()]))
        }
    }
}

/// A slug field validating URL-safe identifiers (`[a-zA-Z0-9_-]+`, or Unicode letters/digits
/// when `allow_unicode` is enabled) with optional `max_length`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlugField {
    /// Maximum allowed length in characters.
    pub max_length: Option<usize>,
    /// Whether to permit Unicode alphanumeric characters in addition to ASCII.
    pub allow_unicode: bool,
    /// Whether the field is required.
    pub required: bool,
    /// Optional human-readable label for the field.
    pub label: Option<String>,
    /// Optional help text rendered alongside the field.
    pub help_text: Option<String>,
    /// Optional initial value string.
    pub initial: Option<String>,
}

impl Default for SlugField {
    fn default() -> Self {
        Self {
            max_length: None,
            allow_unicode: false,
            required: true,
            label: None,
            help_text: None,
            initial: None,
        }
    }
}

impl SlugField {
    /// Creates a new required ASCII `SlugField`.
    pub fn new() -> Self {
        Self::default()
    }
}

impl FormField for SlugField {
    type Value = String;

    fn clean(&self, raw: Option<&str>) -> Result<Self::Value, FieldError> {
        let s = match raw {
            None | Some("") => {
                return if self.required {
                    Err(FieldError(vec!["This field is required.".to_string()]))
                } else {
                    Ok(String::new())
                };
            }
            Some(v) => v,
        };

        if let Some(limit) = self.max_length {
            if s.chars().count() > limit {
                return Err(FieldError(vec![format!(
                    "Ensure this value has at most {} characters.",
                    limit
                )]));
            }
        }

        let valid = if self.allow_unicode {
            s.chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
        } else {
            s.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        };

        if valid {
            Ok(s.to_string())
        } else {
            Err(FieldError(vec![
                "Enter a valid 'slug' consisting of letters, numbers, underscores or hyphens."
                    .to_string(),
            ]))
        }
    }
}

/// A UUID field validating standard 36-character hyphenated (`8-4-4-4-12`) or 32-character
/// hexadecimal UUID strings and normalizing to lowercase hyphenated form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UuidField {
    /// Whether the field is required.
    pub required: bool,
    /// Optional human-readable label for the field.
    pub label: Option<String>,
    /// Optional help text rendered alongside the field.
    pub help_text: Option<String>,
    /// Optional initial value string.
    pub initial: Option<String>,
}

impl Default for UuidField {
    fn default() -> Self {
        Self {
            required: true,
            label: None,
            help_text: None,
            initial: None,
        }
    }
}

impl UuidField {
    /// Creates a new required `UuidField`.
    pub fn new() -> Self {
        Self::default()
    }
}

fn parse_and_normalize_uuid(s: &str) -> Option<String> {
    let trimmed = s
        .strip_prefix("urn:uuid:")
        .unwrap_or(s)
        .trim_start_matches('{')
        .trim_end_matches('}');

    let hex: String = if trimmed.len() == 36 {
        let bytes = trimmed.as_bytes();
        if bytes[8] != b'-' || bytes[13] != b'-' || bytes[18] != b'-' || bytes[23] != b'-' {
            return None;
        }
        trimmed.chars().filter(|&c| c != '-').collect()
    } else if trimmed.len() == 32 {
        trimmed.to_string()
    } else {
        return None;
    };

    if hex.len() != 32 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }

    let lower = hex.to_ascii_lowercase();
    Some(format!(
        "{}-{}-{}-{}-{}",
        &lower[0..8],
        &lower[8..12],
        &lower[12..16],
        &lower[16..20],
        &lower[20..32]
    ))
}

impl FormField for UuidField {
    type Value = Option<String>;

    fn clean(&self, raw: Option<&str>) -> Result<Self::Value, FieldError> {
        let s = match raw.map(str::trim) {
            None | Some("") => {
                return if self.required {
                    Err(FieldError(vec!["This field is required.".to_string()]))
                } else {
                    Ok(None)
                };
            }
            Some(v) => v,
        };

        parse_and_normalize_uuid(s)
            .map(Some)
            .ok_or_else(|| FieldError(vec!["Enter a valid UUID.".to_string()]))
    }
}

fn extract_file_extension(filename: &str) -> Option<String> {
    let base = filename.rsplit(['/', '\\']).next().unwrap_or(filename);
    let (_, ext) = base.rsplit_once('.')?;
    if ext.is_empty() {
        None
    } else {
        Some(ext.to_ascii_lowercase())
    }
}

fn validate_allowed_extension(
    filename: &str,
    allowed_extensions: &[String],
) -> Result<(), FieldError> {
    if allowed_extensions.is_empty() {
        return Ok(());
    }
    let normalized_allowed: Vec<String> = allowed_extensions
        .iter()
        .map(|e| e.trim_start_matches('.').to_ascii_lowercase())
        .collect();

    match extract_file_extension(filename) {
        Some(ext) if normalized_allowed.contains(&ext) => Ok(()),
        _ => Err(FieldError(vec![format!(
            "File extension is not allowed. Allowed extensions are: {}.",
            normalized_allowed.join(", ")
        )])),
    }
}

/// A file upload field validating filename presence, `max_length`, `allow_empty_file`,
/// and optional `allowed_extensions`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileField {
    /// Maximum allowed filename length in characters.
    pub max_length: Option<usize>,
    /// Whether zero-byte uploaded files are permitted (default: `false`).
    pub allow_empty_file: bool,
    /// Optional list of allowed lowercase file extensions (e.g. `["pdf", "txt"]`).
    /// Empty means all extensions are permitted.
    pub allowed_extensions: Vec<String>,
    /// Whether the field is required.
    pub required: bool,
    /// Optional human-readable label for the field.
    pub label: Option<String>,
    /// Optional help text rendered alongside the field.
    pub help_text: Option<String>,
    /// Optional initial value string.
    pub initial: Option<String>,
}

impl Default for FileField {
    fn default() -> Self {
        Self {
            max_length: None,
            allow_empty_file: false,
            allowed_extensions: Vec::new(),
            required: true,
            label: None,
            help_text: None,
            initial: None,
        }
    }
}

impl FileField {
    /// Creates a new required `FileField`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Validates both the filename and the raw uploaded file bytes.
    pub fn validate_bytes(&self, filename: &str, bytes: &[u8]) -> Result<String, FieldError> {
        let cleaned = self
            .clean(Some(filename))?
            .ok_or_else(|| FieldError(vec!["This field is required.".to_string()]))?;
        if !self.allow_empty_file && bytes.is_empty() {
            return Err(FieldError(vec!["The submitted file is empty.".to_string()]));
        }
        Ok(cleaned)
    }
}

impl FormField for FileField {
    type Value = Option<String>;

    fn clean(&self, raw: Option<&str>) -> Result<Self::Value, FieldError> {
        let s = match raw.map(str::trim) {
            None | Some("") => {
                return if self.required {
                    Err(FieldError(vec!["This field is required.".to_string()]))
                } else {
                    Ok(None)
                };
            }
            Some(v) => v,
        };

        if let Some(limit) = self.max_length {
            if s.chars().count() > limit {
                return Err(FieldError(vec![format!(
                    "Ensure this filename has at most {} characters.",
                    limit
                )]));
            }
        }

        validate_allowed_extension(s, &self.allowed_extensions)?;
        Ok(Some(s.to_string()))
    }
}

/// An image upload field validating image extensions (defaulting to
/// `png`, `jpg`, `jpeg`, `gif`, `webp`, `svg`) and optionally magic bytes via
/// [`ImageField::validate_bytes`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageField {
    /// Maximum allowed filename length in characters.
    pub max_length: Option<usize>,
    /// Whether zero-byte uploaded files are permitted (default: `false`).
    pub allow_empty_file: bool,
    /// Allowed image extensions (defaults to `["png", "jpg", "jpeg", "gif", "webp", "svg"]`).
    pub allowed_extensions: Vec<String>,
    /// Whether the field is required.
    pub required: bool,
    /// Optional human-readable label for the field.
    pub label: Option<String>,
    /// Optional help text rendered alongside the field.
    pub help_text: Option<String>,
    /// Optional initial value string.
    pub initial: Option<String>,
}

impl Default for ImageField {
    fn default() -> Self {
        Self {
            max_length: None,
            allow_empty_file: false,
            allowed_extensions: vec![
                "png".to_string(),
                "jpg".to_string(),
                "jpeg".to_string(),
                "gif".to_string(),
                "webp".to_string(),
                "svg".to_string(),
            ],
            required: true,
            label: None,
            help_text: None,
            initial: None,
        }
    }
}

impl ImageField {
    /// Creates a new required `ImageField` with standard web image extensions.
    pub fn new() -> Self {
        Self::default()
    }

    /// Checks whether `bytes` start with a recognized PNG, JPEG, GIF, WebP, or SVG signature.
    pub fn has_valid_magic_bytes(bytes: &[u8]) -> bool {
        // PNG: 89 50 4E 47 0D 0A 1A 0A
        if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            return true;
        }
        // JPEG: FF D8 FF
        if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
            return true;
        }
        // GIF: GIF87a or GIF89a
        if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
            return true;
        }
        // WebP: RIFF....WEBP
        if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
            return true;
        }
        // SVG: UTF-8 text containing <svg
        if let Ok(text) = std::str::from_utf8(bytes) {
            let trimmed = text.trim_start_matches('\u{feff}').trim_start();
            if (trimmed.starts_with("<svg") || trimmed.starts_with("<?xml"))
                && trimmed.contains("<svg")
            {
                return true;
            }
        }
        false
    }

    /// Validates both the image filename/extension and image magic bytes.
    pub fn validate_bytes(&self, filename: &str, bytes: &[u8]) -> Result<String, FieldError> {
        let cleaned = self
            .clean(Some(filename))?
            .ok_or_else(|| FieldError(vec!["This field is required.".to_string()]))?;
        if bytes.is_empty() {
            if self.allow_empty_file {
                return Ok(cleaned);
            }
            return Err(FieldError(vec!["The submitted file is empty.".to_string()]));
        }
        if !Self::has_valid_magic_bytes(bytes) {
            return Err(FieldError(vec![
                "Upload a valid image. The file you uploaded was either not an image or a corrupted image."
                    .to_string(),
            ]));
        }
        Ok(cleaned)
    }
}

impl FormField for ImageField {
    type Value = Option<String>;

    fn clean(&self, raw: Option<&str>) -> Result<Self::Value, FieldError> {
        let s = match raw.map(str::trim) {
            None | Some("") => {
                return if self.required {
                    Err(FieldError(vec!["This field is required.".to_string()]))
                } else {
                    Ok(None)
                };
            }
            Some(v) => v,
        };

        if let Some(limit) = self.max_length {
            if s.chars().count() > limit {
                return Err(FieldError(vec![format!(
                    "Ensure this filename has at most {} characters.",
                    limit
                )]));
            }
        }

        validate_allowed_extension(s, &self.allowed_extensions)?;
        Ok(Some(s.to_string()))
    }
}

/// A multi-choice field validating one or more selections against `(value, label)` choices.
///
/// [`FormField::clean`] splits comma-separated values; use [`MultipleChoiceField::clean_many`]
/// when you already have a slice of submitted values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultipleChoiceField {
    /// Allowed `(value, label)` choice pairs.
    pub choices: Vec<(String, String)>,
    /// Whether at least one choice must be selected.
    pub required: bool,
    /// Optional human-readable label for the field.
    pub label: Option<String>,
    /// Optional help text rendered alongside the field.
    pub help_text: Option<String>,
    /// Optional initial value string.
    pub initial: Option<String>,
}

impl Default for MultipleChoiceField {
    fn default() -> Self {
        Self {
            choices: Vec::new(),
            required: true,
            label: None,
            help_text: None,
            initial: None,
        }
    }
}

impl MultipleChoiceField {
    /// Creates a new required `MultipleChoiceField` with the given `(value, label)` choices.
    pub fn new(choices: Vec<(String, String)>) -> Self {
        Self {
            choices,
            ..Self::default()
        }
    }

    /// Validates a slice of submitted choice values against `self.choices`.
    pub fn clean_many(&self, values: &[&str]) -> Result<Vec<String>, FieldError> {
        let non_empty: Vec<&str> = values
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();

        if non_empty.is_empty() {
            return if self.required {
                Err(FieldError(vec!["This field is required.".to_string()]))
            } else {
                Ok(Vec::new())
            };
        }

        let mut cleaned = Vec::with_capacity(non_empty.len());
        let mut errors = Vec::new();
        for val in non_empty {
            if self.choices.iter().any(|(k, _)| k == val) {
                cleaned.push(val.to_string());
            } else {
                errors.push(format!(
                    "Select a valid choice. {} is not one of the available choices.",
                    val
                ));
            }
        }

        if errors.is_empty() {
            Ok(cleaned)
        } else {
            Err(FieldError(errors))
        }
    }
}

impl FormField for MultipleChoiceField {
    type Value = Vec<String>;

    fn clean(&self, raw: Option<&str>) -> Result<Self::Value, FieldError> {
        match raw {
            None | Some("") => self.clean_many(&[]),
            Some(s) => {
                let parts: Vec<&str> = s.split(',').collect();
                self.clean_many(&parts)
            }
        }
    }
}

/// A choice field that validates against `choices` and coerces the selected string into `T`
/// using a user-supplied parser/closure.
pub struct TypedChoiceField<T, F>
where
    F: Fn(&str) -> Result<T, String>,
{
    /// Allowed `(value, label)` choice pairs.
    pub choices: Vec<(String, String)>,
    /// Closure converting a validated choice string into `T`.
    pub coerce: F,
    /// Whether the field is required.
    pub required: bool,
    /// Optional human-readable label for the field.
    pub label: Option<String>,
    /// Optional help text rendered alongside the field.
    pub help_text: Option<String>,
    /// Optional initial value string.
    pub initial: Option<String>,
}

impl<T, F> TypedChoiceField<T, F>
where
    F: Fn(&str) -> Result<T, String>,
{
    /// Creates a new required `TypedChoiceField` with `choices` and coercion function `coerce`.
    pub fn new(choices: Vec<(String, String)>, coerce: F) -> Self {
        Self {
            choices,
            coerce,
            required: true,
            label: None,
            help_text: None,
            initial: None,
        }
    }
}

impl<T, F> FormField for TypedChoiceField<T, F>
where
    F: Fn(&str) -> Result<T, String>,
{
    type Value = Option<T>;

    fn clean(&self, raw: Option<&str>) -> Result<Self::Value, FieldError> {
        let s = match raw.map(str::trim) {
            None | Some("") => {
                return if self.required {
                    Err(FieldError(vec!["This field is required.".to_string()]))
                } else {
                    Ok(None)
                };
            }
            Some(v) => v,
        };

        if !self.choices.iter().any(|(k, _)| k == s) {
            return Err(FieldError(vec![format!(
                "Select a valid choice. {} is not one of the available choices.",
                s
            )]));
        }

        (self.coerce)(s)
            .map(Some)
            .map_err(|msg| FieldError(vec![msg]))
    }
}

/// A JSON field validating raw text via `serde_json::from_str`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonField {
    /// Whether the field is required.
    pub required: bool,
    /// Optional human-readable label for the field.
    pub label: Option<String>,
    /// Optional help text rendered alongside the field.
    pub help_text: Option<String>,
    /// Optional initial value string.
    pub initial: Option<String>,
}

impl Default for JsonField {
    fn default() -> Self {
        Self {
            required: true,
            label: None,
            help_text: None,
            initial: None,
        }
    }
}

impl JsonField {
    /// Creates a new required `JsonField`.
    pub fn new() -> Self {
        Self::default()
    }
}

impl FormField for JsonField {
    type Value = Option<serde_json::Value>;

    fn clean(&self, raw: Option<&str>) -> Result<Self::Value, FieldError> {
        let s = match raw.map(str::trim) {
            None | Some("") => {
                return if self.required {
                    Err(FieldError(vec!["This field is required.".to_string()]))
                } else {
                    Ok(None)
                };
            }
            Some(v) => v,
        };

        serde_json::from_str::<serde_json::Value>(s)
            .map(Some)
            .map_err(|_| FieldError(vec!["Enter a valid JSON.".to_string()]))
    }
}

/// IP protocol selector for [`GenericIpAddressField`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum IpProtocol {
    /// Accept both IPv4 and IPv6 addresses.
    #[default]
    Both,
    /// Accept only IPv4 addresses.
    Ipv4,
    /// Accept only IPv6 addresses.
    Ipv6,
}

/// An IP address field validating IPv4, IPv6, or both via [`std::net::IpAddr`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenericIpAddressField {
    /// Allowed IP protocol family (default: [`IpProtocol::Both`]).
    pub protocol: IpProtocol,
    /// When `true` and `protocol` is [`IpProtocol::Both`], unpacks IPv4-mapped IPv6
    /// addresses (e.g. `"::ffff:192.0.2.1"`) into canonical IPv4 form (`"192.0.2.1"`).
    pub unpack_ipv4: bool,
    /// Whether the field is required.
    pub required: bool,
    /// Optional human-readable label for the field.
    pub label: Option<String>,
    /// Optional help text rendered alongside the field.
    pub help_text: Option<String>,
    /// Optional initial value string.
    pub initial: Option<String>,
}

impl Default for GenericIpAddressField {
    fn default() -> Self {
        Self {
            protocol: IpProtocol::Both,
            unpack_ipv4: false,
            required: true,
            label: None,
            help_text: None,
            initial: None,
        }
    }
}

impl GenericIpAddressField {
    /// Creates a new required `GenericIpAddressField` accepting both IPv4 and IPv6.
    pub fn new() -> Self {
        Self::default()
    }
}

impl FormField for GenericIpAddressField {
    type Value = Option<String>;

    fn clean(&self, raw: Option<&str>) -> Result<Self::Value, FieldError> {
        let s = match raw.map(str::trim) {
            None | Some("") => {
                return if self.required {
                    Err(FieldError(vec!["This field is required.".to_string()]))
                } else {
                    Ok(None)
                };
            }
            Some(v) => v,
        };

        match self.protocol {
            IpProtocol::Ipv4 => s
                .parse::<Ipv4Addr>()
                .map(|ip| Some(ip.to_string()))
                .map_err(|_| FieldError(vec!["Enter a valid IPv4 address.".to_string()])),
            IpProtocol::Ipv6 => s
                .parse::<Ipv6Addr>()
                .map(|ip| Some(ip.to_string()))
                .map_err(|_| FieldError(vec!["Enter a valid IPv6 address.".to_string()])),
            IpProtocol::Both => s
                .parse::<IpAddr>()
                .map(|ip| {
                    if self.unpack_ipv4 {
                        if let IpAddr::V6(v6) = ip {
                            if let Some(v4) = v6.to_ipv4_mapped() {
                                return Some(v4.to_string());
                            }
                        }
                    }
                    Some(ip.to_string())
                })
                .map_err(|_| FieldError(vec!["Enter a valid IPv4 or IPv6 address.".to_string()])),
        }
    }
}

/// A foreign-key choice field backed by a list of `(pk_string, label)` pairs.
///
/// Decouples form validation from `djangors-orm` while preserving Django's
/// `ModelChoiceField` ergonomics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelChoiceField {
    /// Backing `(pk_string, display_label)` choices populated from a queryset.
    pub choices: Vec<(String, String)>,
    /// Optional placeholder label for the empty option (default: `Some("---------")`).
    pub empty_label: Option<String>,
    /// Whether a selection is required.
    pub required: bool,
    /// Optional human-readable label for the field.
    pub label: Option<String>,
    /// Optional help text rendered alongside the field.
    pub help_text: Option<String>,
    /// Optional initial primary-key string.
    pub initial: Option<String>,
}

impl Default for ModelChoiceField {
    fn default() -> Self {
        Self {
            choices: Vec::new(),
            empty_label: Some("---------".to_string()),
            required: true,
            label: None,
            help_text: None,
            initial: None,
        }
    }
}

impl ModelChoiceField {
    /// Creates a new required `ModelChoiceField` backed by `(pk_string, label)` pairs.
    pub fn new(choices: Vec<(String, String)>) -> Self {
        Self {
            choices,
            ..Self::default()
        }
    }
}

impl FormField for ModelChoiceField {
    type Value = Option<String>;

    fn clean(&self, raw: Option<&str>) -> Result<Self::Value, FieldError> {
        let s = match raw.map(str::trim) {
            None | Some("") => {
                return if self.required {
                    Err(FieldError(vec!["This field is required.".to_string()]))
                } else {
                    Ok(None)
                };
            }
            Some(v) => v,
        };

        if self.choices.iter().any(|(pk, _)| pk == s) {
            Ok(Some(s.to_string()))
        } else {
            Err(FieldError(vec![
                "Select a valid choice. That choice is not one of the available choices."
                    .to_string(),
            ]))
        }
    }
}

/// A many-to-many choice field backed by a list of `(pk_string, label)` pairs.
///
/// Decouples form validation from `djangors-orm` while preserving Django's
/// `ModelMultipleChoiceField` ergonomics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelMultipleChoiceField {
    /// Backing `(pk_string, display_label)` choices populated from a queryset.
    pub choices: Vec<(String, String)>,
    /// Whether at least one selection is required.
    pub required: bool,
    /// Optional human-readable label for the field.
    pub label: Option<String>,
    /// Optional help text rendered alongside the field.
    pub help_text: Option<String>,
    /// Optional initial comma-separated primary keys.
    pub initial: Option<String>,
}

impl Default for ModelMultipleChoiceField {
    fn default() -> Self {
        Self {
            choices: Vec::new(),
            required: true,
            label: None,
            help_text: None,
            initial: None,
        }
    }
}

impl ModelMultipleChoiceField {
    /// Creates a new required `ModelMultipleChoiceField` backed by `(pk_string, label)` pairs.
    pub fn new(choices: Vec<(String, String)>) -> Self {
        Self {
            choices,
            ..Self::default()
        }
    }

    /// Validates a slice of submitted primary-key strings against `self.choices`.
    pub fn clean_many(&self, values: &[&str]) -> Result<Vec<String>, FieldError> {
        let non_empty: Vec<&str> = values
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();

        if non_empty.is_empty() {
            return if self.required {
                Err(FieldError(vec!["This field is required.".to_string()]))
            } else {
                Ok(Vec::new())
            };
        }

        let mut cleaned = Vec::with_capacity(non_empty.len());
        let mut errors = Vec::new();
        for pk in non_empty {
            if self.choices.iter().any(|(choice_pk, _)| choice_pk == pk) {
                cleaned.push(pk.to_string());
            } else {
                errors.push(format!(
                    "Select a valid choice. {} is not one of the available choices.",
                    pk
                ));
            }
        }

        if errors.is_empty() {
            Ok(cleaned)
        } else {
            Err(FieldError(errors))
        }
    }
}

impl FormField for ModelMultipleChoiceField {
    type Value = Vec<String>;

    fn clean(&self, raw: Option<&str>) -> Result<Self::Value, FieldError> {
        match raw {
            None | Some("") => self.clean_many(&[]),
            Some(s) => {
                let parts: Vec<&str> = s.split(',').collect();
                self.clean_many(&parts)
            }
        }
    }
}
