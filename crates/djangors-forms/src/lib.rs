#![deny(missing_docs)]
//! Forms and validation for the Djangors web framework.
//!
//! Provides the core traits, fields, widgets, renderers, formsets, and error structures.

/// CAPTCHA verification backends and form field (the `VerifiedCaptcha` extractor lives in `djangors-core`).
#[cfg(feature = "captcha")]
pub mod captcha;
/// Form error structures and per-field error maps.
pub mod error;
/// Form field types and field validation traits.
pub mod fields;
/// FormSets for managing multiple form instances in a single request.
pub mod formsets;
/// Bot-mitigation helpers (honeypot fields, minimum submit time, UA checks).
pub mod honeypot;
/// Static CSS and JavaScript media asset declarations for forms and widgets.
pub mod media;
/// Form rendering engines (`as_div`, `as_table`, `as_p`) and `BoundField`.
pub mod renderers;
/// Form widgets and HTML rendering.
pub mod widgets;

#[cfg(feature = "captcha")]
pub use captcha::{
    AlwaysFail, AlwaysPass, CaptchaConfig, CaptchaError, CaptchaField, CaptchaOutcome,
    CaptchaProvider, CaptchaVerifier, HCaptchaVerifier, RecaptchaV2Verifier, RecaptchaV3Verifier,
    TurnstileVerifier,
};
pub use djangors_macros::Form;
pub use error::{FieldError, FormErrors};
pub use fields::{
    BooleanField, CharField, ChoiceField, DateField, DateTimeField, DecimalField, DurationField,
    EmailField, FileField, FloatField, FormField, GenericIpAddressField, ImageField, IntegerField,
    IpProtocol, JsonField, ModelChoiceField, ModelMultipleChoiceField, MultipleChoiceField,
    SlugField, TimeField, TypedChoiceField, UrlField, UuidField,
};
pub use formsets::*;
pub use honeypot::{
    is_suspicious_user_agent, BotGuard, HoneypotError, HoneypotField, MinSubmitTime,
    SubmissionCheck,
};
pub use media::FormMedia;
pub use renderers::*;
pub use widgets::*;

#[cfg(test)]
mod tests;
