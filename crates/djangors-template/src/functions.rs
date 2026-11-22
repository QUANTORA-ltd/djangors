//! Built-in template functions (`url`, `static`, `csrf_token`, `now`, `querystring`).

use minijinja::value::{Kwargs, ValueKind};
use minijinja::{Error, ErrorKind, State, Value};
use std::collections::BTreeMap;
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

    // Django 5.1 `querystring(base_query, updates, **kwargs)` helper.
    env.add_function(
        "querystring",
        move |base_query: Option<Value>,
              updates: Option<Value>,
              kwargs: Kwargs|
              -> Result<Value, Error> {
            let mut params: BTreeMap<String, String> = BTreeMap::new();

            if let Some(base) = base_query {
                if !base.is_none() && !base.is_undefined() {
                    if let Some(s) = base.as_str() {
                        let trimmed = s.strip_prefix('?').unwrap_or(s);
                        for pair in trimmed.split('&') {
                            if pair.is_empty() {
                                continue;
                            }
                            let (k_raw, v_raw) = pair.split_once('=').unwrap_or((pair, ""));
                            let k = percent_decode_component(k_raw);
                            let v = percent_decode_component(v_raw);
                            if !k.is_empty() {
                                params.insert(k, v);
                            }
                        }
                    } else if base.kind() == ValueKind::Map {
                        for key in base.try_iter()? {
                            let val = base.get_item(&key)?;
                            if !val.is_none() && !val.is_undefined() {
                                params.insert(key.to_string(), val.to_string());
                            }
                        }
                    } else {
                        return Err(Error::new(
                            ErrorKind::InvalidOperation,
                            "querystring: base_query must be a string or mapping",
                        ));
                    }
                }
            }

            if let Some(upd) = updates {
                if !upd.is_none() && !upd.is_undefined() {
                    if upd.kind() == ValueKind::Map {
                        for key in upd.try_iter()? {
                            let val = upd.get_item(&key)?;
                            if val.is_none() || val.is_undefined() {
                                params.remove(&key.to_string());
                            } else {
                                params.insert(key.to_string(), val.to_string());
                            }
                        }
                    } else {
                        return Err(Error::new(
                            ErrorKind::InvalidOperation,
                            "querystring: updates must be a mapping",
                        ));
                    }
                }
            }

            for key in kwargs.args() {
                let val: Value = kwargs.get(key)?;
                if val.is_none() || val.is_undefined() {
                    params.remove(key);
                } else {
                    params.insert(key.to_string(), val.to_string());
                }
            }
            kwargs.assert_all_used()?;

            if params.is_empty() {
                return Ok(Value::from_safe_string(String::new()));
            }

            let encoded: Vec<String> = params
                .into_iter()
                .map(|(k, v)| {
                    format!(
                        "{}={}",
                        percent_encode_component(&k),
                        percent_encode_component(&v)
                    )
                })
                .collect();
            Ok(Value::from_safe_string(format!("?{}", encoded.join("&"))))
        },
    );
}

fn percent_encode_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            b => {
                use std::fmt::Write;
                let _ = write!(&mut out, "%{b:02X}");
            }
        }
    }
    out
}

fn percent_decode_component(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'+' {
            out.push(b' ');
            i += 1;
        } else if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
            out.push(bytes[i]);
            i += 1;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}
