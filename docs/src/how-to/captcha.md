# CAPTCHA Verification

Enable the `captcha` feature on `djangors` (or `djangors-forms`) to use Cloudflare Turnstile, hCaptcha, reCAPTCHA v2, or reCAPTCHA v3. Network failures fail closed.

## Configuration

```rust,illustrative
use std::time::Duration;
use djangors::core::AppState;
use djangors::forms::{CaptchaConfig, CaptchaProvider};

# fn example() {
let config = CaptchaConfig::new(
    CaptchaProvider::Turnstile,
    "your-site-key",
    "your-secret-key",
)
.with_timeout(Duration::from_secs(5));

let state = AppState::new().insert(config.build_verifier());
# let _ = state;
# }
```

## Form Field and JSON Extractor

- Use `CaptchaField::new(provider, site_key).render()` to emit the widget script tag and hidden response input in HTML forms.
- Use `djangors::core::captcha::VerifiedCaptcha` as an extractor in JSON API handlers. It reads the `X-Captcha-Token` header, or a `captcha_token` / `token` field in a JSON body, and rejects with `403` when verification fails. It lives in `djangors-core` (feature `captcha`) because `djangors-forms` sits below the HTTP layer.
- Use `AlwaysPass` and `AlwaysFail` in tests.
