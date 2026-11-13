//! Built-in template functions (`url`, `static`, `csrf_token`, `now`).

use minijinja::value::Kwargs;
use minijinja::{Error, ErrorKind, State, Value};
use std::sync::{Arc, Mutex};

/// Type alias for the URL resolver closure.
pub type UrlResolver =
    Arc<dyn Fn(&str, &[(String, String)]) -> Option<String> + Send + Sync + 'static>;

/// Shared state backing template functions (`url` resolver and `static` URL prefix).
#[derive(Clone)]
pub struct FunctionsState {
    /// The injected URL resolver closure backing `{{ url(...) }}`.
    pub url_resolver: Arc<Mutex<Option<UrlResolver>>>,
    /// The static URL prefix backing `{{ static(...) }}`.
    pub static_url: Arc<Mutex<String>>,
}

impl Default for FunctionsState {
    fn default() -> Self {
        Self {
            url_resolver: Arc::new(Mutex::new(None)),
            static_url: Arc::new(Mutex::new("/static/".to_string())),
        }
    }
}

/// Escape HTML attribute value characters (`&`, `"`, `<`, `>`, `'`).
pub fn escape_html_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\'' => out.push_str("&#x27;"),
            _ => out.push(c),
        }
    }
    out
}

/// Register `url`, `static`, `csrf_token`, and `now` functions onto the MiniJinja Environment.
pub fn register_functions(env: &mut minijinja::Environment<'static>, state: FunctionsState) {
    let state_url = state.clone();
    env.add_function(
        "url",
        move |_state: &State, name: &str, kwargs: Kwargs| -> Result<Value, Error> {
            let resolver_guard = state_url.url_resolver.lock().unwrap();
            let resolver = resolver_guard.as_ref().ok_or_else(|| {
                Error::new(
                    ErrorKind::InvalidOperation,
                    "No URL resolver configured on TemplateEngine. Call set_url_resolver() first.",
                )
            })?;

            let mut params = Vec::new();
            for key in kwargs.args() {
                let val: Value = kwargs.get(key)?;
                params.push((key.to_string(), val.to_string()));
            }
            kwargs.assert_all_used()?;

            let path = resolver(name, &params).ok_or_else(|| {
                Error::new(
                    ErrorKind::InvalidOperation,
                    format!("No route matching name '{name}' with parameters {params:?}"),
                )
            })?;

            Ok(Value::from_safe_string(path))
        },
    );

    let state_static = state.clone();
    env.add_function("static", move |path: &str| -> Result<Value, Error> {
        let prefix_guard = state_static.static_url.lock().unwrap();
        let prefix = prefix_guard.as_str();
        let p_trimmed = prefix.trim_end_matches('/');
        let path_trimmed = path.trim_start_matches('/');
        let res = format!("{p_trimmed}/{path_trimmed}");
        Ok(Value::from_safe_string(res))
    });

    env.add_function("csrf_token", move |state: &State| -> Result<Value, Error> {
        let token_val = state
            .lookup("csrf_token")
            .filter(|v| !v.is_undefined() && v.as_str().is_some())
            .or_else(|| {
                state
                    .lookup("_csrf_token")
                    .filter(|v| !v.is_undefined() && v.as_str().is_some())
            })
            .or_else(|| {
                state
                    .lookup("csrf_token_val")
                    .filter(|v| !v.is_undefined() && v.as_str().is_some())
            });

        let token_str = token_val.as_ref().and_then(|v| v.as_str()).ok_or_else(|| {
            Error::new(
                ErrorKind::UndefinedError,
                "csrf_token context variable missing",
            )
        })?;

        let escaped_token = escape_html_attr(token_str);
        let html = format!(
            r#"<input type="hidden" name="csrfmiddlewaretoken" value="{}">"#,
            escaped_token
        );
        Ok(Value::from_safe_string(html))
    });

    env.add_function("now", move |fmt: Option<&str>| -> Result<String, Error> {
        let format_str = fmt.unwrap_or("%Y-%m-%d %H:%M:%S");
        let now = chrono::Utc::now();
        Ok(now.format(format_str).to_string())
    });

    // Reads `_csp_nonce` (like `csrf_token()` reads `_csrf_token`): a context variable
    // named `csp_nonce` would shadow this function.
    env.add_function("csp_nonce", move |state: &State| -> Result<Value, Error> {
        let nonce = state
            .lookup("_csp_nonce")
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default();
        Ok(Value::from(nonce))
    });
}
