#![deny(missing_docs)]
//! Runtime internationalization for Djangors.
//!
//! v1 deliberately provides runtime catalog loading and lookup only.  Catalogs are hand-written
//! Fluent (`.ftl`) sources; a future `dj makemessages`-style extractor is out of scope here.
//!
//! `LocaleLayer` resolves the first `Accept-Language` tag, or the session's `_locale` override,
//! and inserts [`ResolvedLocale`] into the request extensions.  Template filters cannot access a
//! request, so pass an opaque [`locales_value`] and the resolved locale in the template context:
//!
//! ```ignore
//! let ctx = minijinja::context! { locales => locales_value(std::sync::Arc::new(locales)), locale => "en-US" };
//! // Template: {{ "welcome" | trans(locales, locale) }}
//! ```
//!
//! Date formatting is intentionally a small convention mapping, not a CLDR implementation.

use chrono::{DateTime, NaiveDate, Utc};
use fluent_bundle::{concurrent::FluentBundle, FluentArgs, FluentResource};
use minijinja::{value::Object, Error as MiniJinjaError, ErrorKind as MiniJinjaErrorKind};
use std::{collections::HashMap, fmt, sync::Arc};
use thiserror::Error;
use tower::{Layer, Service};
use unic_langid::LanguageIdentifier;

/// Error types for internationalization operations.
#[derive(Debug, Error)]
pub enum I18nError {
    /// An invalid locale string identifier was provided.
    #[error("invalid locale '{0}'")]
    InvalidLocale(String),
    /// A Fluent bundle error occurred.
    #[error("invalid Fluent source: {0:?}")]
    Fluent(Vec<fluent_bundle::FluentError>),
    /// A Fluent resource parsing error occurred.
    #[error("invalid Fluent resource: {0}")]
    Resource(String),
}

/// A parsed Fluent message catalog for a specific locale.
pub struct Catalog {
    bundle: FluentBundle<FluentResource>,
}

impl Catalog {
    /// Parses a Fluent (`.ftl`) source string into a `Catalog` for `locale`.
    pub fn from_ftl(locale: &str, ftl_source: &str) -> Result<Self, I18nError> {
        let language: LanguageIdentifier = locale
            .parse()
            .map_err(|_| I18nError::InvalidLocale(locale.to_string()))?;
        let resource = FluentResource::try_new(ftl_source.to_string())
            .map_err(|(_, errors)| I18nError::Resource(format!("{errors:?}")))?;
        let mut bundle = FluentBundle::new_concurrent(vec![language]);
        bundle.add_resource(resource).map_err(I18nError::Fluent)?;
        Ok(Self { bundle })
    }

    /// Looks up and formats a translated message by ID, passing optional arguments.
    pub fn get(&self, message_id: &str, args: Option<&FluentArgs<'_>>) -> Option<String> {
        let message = self.bundle.get_message(message_id)?;
        let pattern = message.value()?;
        let mut errors = Vec::new();
        Some(
            self.bundle
                .format_pattern(pattern, args, &mut errors)
                .into_owned(),
        )
    }
}

/// A collection of locale catalogs mapped by locale identifier.
pub struct Locales {
    default_locale: String,
    catalogs: HashMap<String, Catalog>,
}

impl fmt::Debug for Locales {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Locales")
            .field("default_locale", &self.default_locale)
            .field("catalogs", &self.catalogs.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl Locales {
    /// Creates a new `Locales` container with `default_locale`.
    pub fn new(default_locale: &str) -> Self {
        Self {
            default_locale: default_locale.to_string(),
            catalogs: HashMap::new(),
        }
    }

    /// Parses and adds a Fluent (`.ftl`) translation source for `locale`.
    pub fn add_locale(&mut self, locale: &str, ftl_source: &str) -> Result<(), I18nError> {
        self.catalogs
            .insert(locale.to_string(), Catalog::from_ftl(locale, ftl_source)?);
        Ok(())
    }

    /// Translates `message_id` for `locale`, falling back to the default locale, then `message_id`.
    pub fn translate(
        &self,
        locale: &str,
        message_id: &str,
        args: Option<&FluentArgs<'_>>,
    ) -> String {
        self.catalogs
            .get(locale)
            .and_then(|catalog| catalog.get(message_id, args))
            .or_else(|| {
                self.catalogs
                    .get(&self.default_locale)
                    .and_then(|c| c.get(message_id, args))
            })
            .unwrap_or_else(|| message_id.to_string())
    }

    /// Returns the configured default locale string.
    pub fn default_locale(&self) -> &str {
        &self.default_locale
    }
}

/// Request extension wrapper carrying the resolved locale string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedLocale(pub String);

/// Request extension wrapper carrying the active locale string set by [`LocaleLayer`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveLocale(pub String);

/// Tower middleware layer resolving the request locale into extensions.
///
/// Inserts both [`ResolvedLocale`] and [`ActiveLocale`] into the request's
/// extensions map.
#[derive(Clone)]
pub struct LocaleLayer {
    default_locale: String,
    supported_locales: Vec<String>,
    cookie_name: String,
}

impl LocaleLayer {
    /// Creates a new `LocaleLayer` with `default_locale`.
    pub fn new(default_locale: impl Into<String>) -> Self {
        Self {
            default_locale: default_locale.into(),
            supported_locales: Vec::new(),
            cookie_name: "djangors_language".to_string(),
        }
    }

    /// Restrict locale detection to the given supported locale identifiers.
    pub fn with_supported_locales(mut self, locales: Vec<String>) -> Self {
        self.supported_locales = locales;
        self
    }

    /// Configure the cookie name inspected for a user locale preference.
    ///
    /// Defaults to `"djangors_language"`.
    pub fn with_cookie_name(mut self, name: impl Into<String>) -> Self {
        self.cookie_name = name.into();
        self
    }
}

impl Default for LocaleLayer {
    fn default() -> Self {
        Self::new("en-US")
    }
}

/// Tower service middleware performing locale resolution per request.
#[derive(Clone)]
pub struct LocaleService<S> {
    inner: S,
    default_locale: String,
    supported_locales: Vec<String>,
    cookie_name: String,
}

impl<S> Layer<S> for LocaleLayer {
    type Service = LocaleService<S>;
    fn layer(&self, inner: S) -> Self::Service {
        LocaleService {
            inner,
            default_locale: self.default_locale.clone(),
            supported_locales: self.supported_locales.clone(),
            cookie_name: self.cookie_name.clone(),
        }
    }
}

fn extract_cookie_value(cookie_header: &str, name: &str) -> Option<String> {
    for part in cookie_header.split(';') {
        if let Some((k, v)) = part.trim().split_once('=') {
            if k.trim() == name {
                let val = v.trim();
                if !val.is_empty() {
                    return Some(val.to_string());
                }
            }
        }
    }
    None
}

fn extract_url_prefix(path: &str) -> Option<&str> {
    let trimmed = path.strip_prefix('/')?;
    let seg = trimmed.split('/').next()?;
    if seg.is_empty() {
        None
    } else {
        Some(seg)
    }
}

impl<S, B> Service<hyper::Request<B>> for LocaleService<S>
where
    S: Service<hyper::Request<B>> + Clone + Send + 'static,
    S::Future: Send + 'static,
    B: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>,
    >;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: hyper::Request<B>) -> Self::Future {
        let mut inner = self.inner.clone();
        let accept_header = req
            .headers()
            .get("accept-language")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let cookie_locale = req
            .headers()
            .get("cookie")
            .and_then(|v| v.to_str().ok())
            .and_then(|hdr| extract_cookie_value(hdr, &self.cookie_name));
        let session_locale = req
            .extensions()
            .get::<djangors_sessions::Session>()
            .and_then(|s| s.get::<String>("_locale"));

        let locale = if self.supported_locales.is_empty() {
            cookie_locale
                .or_else(|| accept_header.as_deref().and_then(first_locale_tag))
                .or(session_locale)
                .unwrap_or_else(|| self.default_locale.clone())
        } else {
            let url_prefix = extract_url_prefix(req.uri().path());
            let supported_refs: Vec<&str> =
                self.supported_locales.iter().map(String::as_str).collect();
            let effective_cookie = cookie_locale.or(session_locale);
            detect_locale(
                accept_header.as_deref(),
                effective_cookie.as_deref(),
                url_prefix,
                &supported_refs,
                &self.default_locale,
            )
        };

        req.extensions_mut().insert(ResolvedLocale(locale.clone()));
        req.extensions_mut().insert(ActiveLocale(locale));
        Box::pin(async move { inner.call(req).await })
    }
}

/// Parses the first valid BCP-47 language tag from an `Accept-Language` header value.
pub fn first_locale_tag(header: &str) -> Option<String> {
    let tag = header
        .split(',')
        .next()
        .and_then(|tag| tag.trim().split(';').next())
        .map(str::trim)
        .filter(|tag| !tag.is_empty() && *tag != "*")?;
    tag.parse::<LanguageIdentifier>()
        .ok()
        .map(|_| tag.to_string())
}

fn match_supported(candidate: &str, supported: &[&str]) -> Option<String> {
    let candidate = candidate.trim();
    if candidate.is_empty() || candidate == "*" {
        return None;
    }
    if supported.is_empty() {
        return candidate
            .parse::<LanguageIdentifier>()
            .ok()
            .map(|_| candidate.to_string());
    }

    let cand_lower = candidate.to_ascii_lowercase().replace('_', "-");
    // Exact case-insensitive match first.
    for &sup in supported {
        if sup.to_ascii_lowercase().replace('_', "-") == cand_lower {
            return Some(sup.to_string());
        }
    }
    // Primary language subtag match (e.g. "fr-CA" -> "fr" or "fr" -> "fr-FR").
    let cand_lang = cand_lower.split('-').next().unwrap_or("");
    if !cand_lang.is_empty() {
        for &sup in supported {
            let sup_lower = sup.to_ascii_lowercase().replace('_', "-");
            let sup_lang = sup_lower.split('-').next().unwrap_or("");
            if sup_lang == cand_lang {
                return Some(sup.to_string());
            }
        }
    }
    None
}

/// Detect the best matching locale from URL prefix, cookie, `Accept-Language` header,
/// and supported locales list, falling back to `default`.
///
/// Precedence follows Django's `LocaleMiddleware`:
/// 1. `url_prefix`
/// 2. `cookie_locale`
/// 3. `accept_language` (ordered by descending `q=` quality factor)
/// 4. `default`
pub fn detect_locale(
    accept_language: Option<&str>,
    cookie_locale: Option<&str>,
    url_prefix: Option<&str>,
    supported: &[&str],
    default: &str,
) -> String {
    if let Some(prefix) = url_prefix {
        if let Some(matched) = match_supported(prefix, supported) {
            return matched;
        }
    }

    if let Some(cookie) = cookie_locale {
        if let Some(matched) = match_supported(cookie, supported) {
            return matched;
        }
    }

    if let Some(header) = accept_language {
        let mut parsed: Vec<(f32, usize, &str)> = Vec::new();
        for (idx, part) in header.split(',').enumerate() {
            let mut segments = part.trim().split(';');
            let tag = match segments.next().map(str::trim) {
                Some(t) if !t.is_empty() && t != "*" => t,
                _ => continue,
            };
            let mut q = 1.0_f32;
            for param in segments {
                let param = param.trim();
                if let Some(q_str) = param
                    .strip_prefix("q=")
                    .or_else(|| param.strip_prefix("Q="))
                {
                    if let Ok(val) = q_str.trim().parse::<f32>() {
                        q = val;
                    }
                }
            }
            if q > 0.0 {
                parsed.push((q, idx, tag));
            }
        }
        parsed.sort_by(|a, b| {
            b.0.partial_cmp(&a.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.1.cmp(&b.1))
        });
        for (_, _, tag) in parsed {
            if let Some(matched) = match_supported(tag, supported) {
                return matched;
            }
        }
    }

    default.to_string()
}

/// Generate locale-prefixed URL pattern pairs `(prefixed_path, original_path)`.
///
/// Analogous to Django's `i18n_patterns()`. When `prefix_default_language` is
/// `false` and `default_locale` matches a locale in `locales`, the unprefixed
/// path is emitted for that locale instead of `/<default_locale><path>`.
pub fn i18n_patterns(
    locales: &[&str],
    paths: &[&str],
    prefix_default_language: bool,
    default_locale: Option<&str>,
) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for &locale in locales {
        let is_default = !prefix_default_language && default_locale == Some(locale);
        for &path in paths {
            let normalized_path = if path.starts_with('/') {
                path.to_string()
            } else {
                format!("/{path}")
            };
            if is_default {
                out.push((normalized_path.clone(), normalized_path));
            } else {
                let prefixed = if normalized_path == "/" {
                    format!("/{locale}/")
                } else {
                    format!("/{locale}{normalized_path}")
                };
                out.push((prefixed, normalized_path));
            }
        }
    }
    out
}

/// Opaque MiniJinja Object wrapper for `Locales`.
#[derive(Clone, Debug)]
pub struct LocalesValue(pub Arc<Locales>);
impl Object for LocalesValue {}

/// Wraps an `Arc<Locales>` into a MiniJinja value suitable for template context.
pub fn locales_value(locales: Arc<Locales>) -> minijinja::Value {
    minijinja::Value::from_object(LocalesValue(locales))
}

/// Template filter function `trans` for translating message IDs.
pub fn trans(
    message_id: minijinja::Value,
    locales: minijinja::Value,
    locale: String,
) -> Result<String, MiniJinjaError> {
    let locales = locales
        .downcast_object_ref::<LocalesValue>()
        .ok_or_else(|| {
            MiniJinjaError::new(
                MiniJinjaErrorKind::InvalidOperation,
                "trans filter expects locales_value(...)",
            )
        })?;
    Ok(locales.0.translate(&locale, &message_id.to_string(), None))
}

/// Formats a `NaiveDate` according to locale date conventions.
pub fn localized_date(date: NaiveDate, locale: &str) -> String {
    let format = match locale {
        "en-US" => "%m/%d/%Y",
        "en-GB" | "en-AU" | "en-NZ" => "%d/%m/%Y",
        _ if locale.starts_with("en-")
            || locale.starts_with("de-")
            || locale.starts_with("fr-") =>
        {
            "%d/%m/%Y"
        }
        _ => "%Y-%m-%d",
    };
    date.format(format).to_string()
}

/// Formats a `DateTime<Utc>` according to locale date conventions.
pub fn localized_datetime(date: DateTime<Utc>, locale: &str) -> String {
    localized_date(date.date_naive(), locale)
}

/// Formats a floating-point number with locale-appropriate thousands and decimal separators.
///
/// - `en*` (and default): `,` thousands, `.` decimal (`1,234,567.89`)
/// - `de*`, `es*`, `it*`, `pt*`, `nl*`: `.` thousands, `,` decimal (`1.234.567,89`)
/// - `fr*`, `sv*`, `fi*`, `pl*`, `ru*`: `\u{00A0}` (non-breaking space) thousands, `,` decimal (`1\u{00A0}234\u{00A0}567,89`)
pub fn format_number(value: f64, locale: &str) -> String {
    if !value.is_finite() {
        return value.to_string();
    }

    let lang = locale
        .to_ascii_lowercase()
        .replace('_', "-")
        .split('-')
        .next()
        .unwrap_or("en")
        .to_string();

    let (thousands_sep, decimal_sep) = match lang.as_str() {
        "de" | "es" | "it" | "pt" | "nl" => (".", ","),
        "fr" | "sv" | "fi" | "pl" | "ru" => ("\u{00A0}", ","),
        _ => (",", "."),
    };

    let negative = value.is_sign_negative() && value != 0.0;
    let abs_val = value.abs();
    let raw = if abs_val.fract() == 0.0 {
        format!("{abs_val:.0}")
    } else {
        format!("{abs_val}")
    };

    let (int_part, frac_part) = match raw.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (raw.as_str(), None),
    };

    let mut grouped = String::with_capacity(int_part.len() + int_part.len() / 3);
    let len = int_part.len();
    for (idx, ch) in int_part.chars().enumerate() {
        if idx > 0 && (len - idx) % 3 == 0 {
            grouped.push_str(thousands_sep);
        }
        grouped.push(ch);
    }

    let mut out = String::new();
    if negative {
        out.push('-');
    }
    out.push_str(&grouped);
    if let Some(frac) = frac_part {
        out.push_str(decimal_sep);
        out.push_str(frac);
    }
    out
}

/// Formats a `NaiveDate` according to locale conventions.
///
/// - `en-US`: `MM/DD/YYYY` (`07/22/2026`)
/// - `de` / `de-*`: `DD.MM.YYYY` (`22.07.2026`)
/// - `en-GB`, `en-AU`, `en-NZ`, `fr*`, `es*`, `it*`, `pt*`: `DD/MM/YYYY` (`22/07/2026`)
/// - Other locales (`ja*`, `zh*`, `ko*`, etc.): ISO `YYYY-MM-DD` (`2026-07-22`)
pub fn format_date(date: NaiveDate, locale: &str) -> String {
    let norm = locale.to_ascii_lowercase().replace('_', "-");
    let fmt = if norm == "en-us" || norm == "en" {
        "%m/%d/%Y"
    } else if norm == "de" || norm.starts_with("de-") {
        "%d.%m.%Y"
    } else if norm == "en-gb"
        || norm == "en-au"
        || norm == "en-nz"
        || norm == "fr"
        || norm.starts_with("fr-")
        || norm == "es"
        || norm.starts_with("es-")
        || norm == "it"
        || norm.starts_with("it-")
        || norm == "pt"
        || norm.starts_with("pt-")
    {
        "%d/%m/%Y"
    } else {
        "%Y-%m-%d"
    };
    date.format(fmt).to_string()
}

/// Formats a `DateTime<Utc>` with locale-appropriate date and 24-hour time.
pub fn format_datetime(dt: DateTime<Utc>, locale: &str) -> String {
    format!(
        "{} {}",
        format_date(dt.date_naive(), locale),
        dt.format("%H:%M:%S")
    )
}

tokio::task_local! {
    static TASK_TIMEZONE: std::cell::RefCell<Option<String>>;
}

thread_local! {
    static THREAD_TIMEZONE: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// Execute an async future with `tz` active as the task-local timezone.
///
/// Inside `fut`, [`current_timezone`] returns `Some(tz)`. When `fut` completes,
/// the previous timezone context is automatically restored.
pub async fn with_timezone<F: std::future::Future>(tz: impl Into<String>, fut: F) -> F::Output {
    let cell = std::cell::RefCell::new(Some(tz.into()));
    TASK_TIMEZONE.scope(cell, fut).await
}

/// Activate `tz` as the current timezone in the active task scope (if inside
/// [`with_timezone`]) or thread-local fallback otherwise.
pub fn activate_timezone(tz: impl Into<String>) {
    let tz = tz.into();
    if TASK_TIMEZONE
        .try_with(|cell| {
            *cell.borrow_mut() = Some(tz.clone());
        })
        .is_err()
    {
        THREAD_TIMEZONE.with(|cell| {
            *cell.borrow_mut() = Some(tz);
        });
    }
}

/// Deactivate the current timezone in the active task or thread-local scope.
pub fn deactivate_timezone() {
    if TASK_TIMEZONE
        .try_with(|cell| {
            *cell.borrow_mut() = None;
        })
        .is_err()
    {
        THREAD_TIMEZONE.with(|cell| {
            *cell.borrow_mut() = None;
        });
    }
}

/// Return the currently active timezone, if one has been set via
/// [`with_timezone`] or [`activate_timezone`].
pub fn current_timezone() -> Option<String> {
    if let Ok(tz) = TASK_TIMEZONE.try_with(|cell| cell.borrow().clone()) {
        return tz;
    }
    THREAD_TIMEZONE.with(|cell| cell.borrow().clone())
}

#[cfg(test)]
mod tests;
