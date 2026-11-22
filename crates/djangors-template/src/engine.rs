use minijinja::value::ValueKind;
use minijinja::{AutoEscape, Environment, Value};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::context_processors::ContextProcessor;
use crate::error::TemplateError;
use crate::filters;
use crate::functions;

/// The template rendering engine for Djangors, powered by MiniJinja.
#[derive(Clone)]
pub struct TemplateEngine {
    env: Environment<'static>,
    functions_state: functions::FunctionsState,
    search_dirs: Arc<Vec<PathBuf>>,
    embedded_sources: Arc<HashMap<String, String>>,
    context_processors: Arc<Mutex<Vec<Arc<dyn ContextProcessor>>>>,
}

fn register_filters(env: &mut Environment<'static>) {
    env.add_filter("date", filters::date);
    env.add_filter("floatformat", filters::floatformat);
    env.add_filter("pluralize", filters::pluralize);
    env.add_filter("truncatewords", filters::truncatewords);
    env.add_filter("intcomma", filters::intcomma);
    env.add_filter("filesizeformat", filters::filesizeformat);
    env.add_filter("naturaltime", filters::naturaltime);
    env.add_filter("trans", djangors_i18n::trans);
    env.add_filter("json_script", filters::json_script);
    env.add_filter("timesince", filters::timesince);
    env.add_filter("timeuntil", filters::timeuntil);
    env.add_filter("linebreaks", filters::linebreaks);
    env.add_filter("linebreaksbr", filters::linebreaksbr);
    env.add_filter("slugify", filters::slugify);
    env.add_filter("urlize", filters::urlize);
    env.add_filter("striptags", filters::striptags);
    env.add_filter("yesno", filters::yesno);
    env.add_filter("ordinal", filters::ordinal);
    env.add_filter("naturalday", filters::naturalday);
    env.add_filter("apnumber", filters::apnumber);
    env.add_filter("dictsort", filters::dictsort);
}

fn extract_partial(source: &str, partial_name: &str) -> Option<String> {
    if let Some(body) = extract_comment_partial(source, partial_name) {
        return Some(body);
    }
    extract_block_partial(source, partial_name)
}

fn extract_comment_partial(source: &str, partial_name: &str) -> Option<String> {
    let mut cursor = 0usize;
    let mut depth = 0usize;
    let mut body_start = None;

    while let Some(rel_open) = source[cursor..].find("{#") {
        let open_idx = cursor + rel_open;
        let after_open = open_idx + 2;
        let rel_close = source[after_open..].find("#}")?;
        let close_idx = after_open + rel_close;
        let tag_end = close_idx + 2;
        let inner = source[after_open..close_idx].trim();
        let tokens: Vec<&str> = inner.split_whitespace().collect();

        if depth == 0 {
            if tokens.len() == 2 && tokens[0] == "partialdef" && tokens[1] == partial_name {
                depth = 1;
                body_start = Some(tag_end);
            }
        } else if tokens.first() == Some(&"partialdef") {
            depth += 1;
        } else if tokens.first() == Some(&"endpartialdef")
            && (tokens.len() == 1 || (depth == 1 && tokens.get(1) == Some(&partial_name)))
        {
            depth -= 1;
            if depth == 0 {
                let start = body_start?;
                return Some(source[start..open_idx].to_string());
            }
        }

        cursor = tag_end;
    }

    None
}

fn extract_block_partial(source: &str, partial_name: &str) -> Option<String> {
    let mut cursor = 0usize;
    let mut depth = 0usize;
    let mut body_start = None;

    while let Some(rel_open) = source[cursor..].find("{%") {
        let open_idx = cursor + rel_open;
        let after_open = open_idx + 2;
        let rel_close = source[after_open..].find("%}")?;
        let close_idx = after_open + rel_close;
        let tag_end = close_idx + 2;
        let inner = source[after_open..close_idx]
            .trim()
            .trim_start_matches('-')
            .trim_end_matches('-')
            .trim();
        let tokens: Vec<&str> = inner.split_whitespace().collect();

        if depth == 0 {
            if tokens.len() >= 2 && tokens[0] == "block" && tokens[1] == partial_name {
                depth = 1;
                body_start = Some(tag_end);
            }
        } else if tokens.first() == Some(&"block") {
            depth += 1;
        } else if tokens.first() == Some(&"endblock") {
            depth -= 1;
            if depth == 0 {
                let start = body_start?;
                return Some(source[start..open_idx].to_string());
            }
        }

        cursor = tag_end;
    }

    None
}

impl TemplateEngine {
    /// Create a new `TemplateEngine` with search directories checked in order.
    ///
    /// The search directories are searched in the order they are listed.
    /// The first directory that contains the requested template name wins.
    /// This order implements Django's override precedence (project-level
    /// directory listed first overrides app-level templates).
    pub fn new(search_dirs: Vec<PathBuf>) -> Result<Self, TemplateError> {
        let mut env = Environment::new();

        // Configure auto-escaping callback: enable HTML escaping for .html and .htm files
        env.set_auto_escape_callback(|name| {
            if name.ends_with(".html") || name.ends_with(".htm") {
                AutoEscape::Html
            } else {
                AutoEscape::None
            }
        });

        // Set custom template loader using the provided search directories in order
        let dirs = Arc::new(search_dirs);
        let loader_dirs = Arc::clone(&dirs);
        env.set_loader(move |name| {
            for dir in loader_dirs.as_ref() {
                let path = dir.join(name);
                if path.exists() {
                    match std::fs::read_to_string(&path) {
                        Ok(content) => return Ok(Some(content)),
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                        Err(e) => {
                            return Err(minijinja::Error::new(
                                minijinja::ErrorKind::InvalidOperation,
                                format!("Failed to read template file '{}': {}", path.display(), e),
                            ))
                        }
                    }
                }
            }
            Ok(None)
        });

        register_filters(&mut env);

        // Note: 'default' filter is built-in to minijinja and behaves matching Django's
        // default template filter when the value is undefined or falsy.

        let functions_state = functions::FunctionsState::default();
        functions::register_functions(&mut env, functions_state.clone());

        Ok(TemplateEngine {
            env,
            functions_state,
            search_dirs: dirs,
            embedded_sources: Arc::new(HashMap::new()),
            context_processors: Arc::new(Mutex::new(Vec::new())),
        })
    }

    /// Create a new `TemplateEngine` from templates embedded at compile time
    /// (e.g. via `include_str!`), rather than loaded from the filesystem at
    /// runtime. This is what library crates that ship their own templates
    /// (like `djangors-admin`) should use.
    pub fn from_embedded(
        templates: &[(&'static str, &'static str)],
    ) -> Result<Self, TemplateError> {
        let mut env = Environment::new();

        env.set_auto_escape_callback(|name| {
            if name.ends_with(".html") || name.ends_with(".htm") {
                AutoEscape::Html
            } else {
                AutoEscape::None
            }
        });

        let mut embedded = HashMap::with_capacity(templates.len());
        for (name, source) in templates {
            env.add_template(name, source)
                .map_err(TemplateError::MiniJinja)?;
            embedded.insert((*name).to_string(), (*source).to_string());
        }

        register_filters(&mut env);

        let functions_state = functions::FunctionsState::default();
        functions::register_functions(&mut env, functions_state.clone());

        Ok(TemplateEngine {
            env,
            functions_state,
            search_dirs: Arc::new(Vec::new()),
            embedded_sources: Arc::new(embedded),
            context_processors: Arc::new(Mutex::new(Vec::new())),
        })
    }

    /// Supply the resolver backing `{{ url(...) }}`.
    pub fn set_url_resolver<F>(&mut self, f: F)
    where
        F: Fn(&str, &[(String, String)]) -> Option<String> + Send + Sync + 'static,
    {
        *self.functions_state.url_resolver.lock().unwrap() = Some(Arc::new(f));
    }

    /// Prefix for `{{ static(...) }}`. Defaults to "/static/".
    pub fn set_static_url(&mut self, prefix: impl Into<String>) {
        *self.functions_state.static_url.lock().unwrap() = prefix.into();
    }

    /// Register a [`ContextProcessor`] to be executed by [`TemplateEngine::render_with_request`].
    pub fn add_context_processor(&mut self, processor: Arc<dyn ContextProcessor>) {
        self.context_processors.lock().unwrap().push(processor);
    }

    /// Render a template by name with a serializable context.
    pub fn render(&self, name: &str, ctx: impl Serialize) -> Result<String, TemplateError> {
        let template = self.env.get_template(name).map_err(|e| {
            if e.kind() == minijinja::ErrorKind::TemplateNotFound {
                TemplateError::NotFound {
                    name: name.to_string(),
                    searched: self.search_dirs.as_ref().clone(),
                }
            } else {
                TemplateError::MiniJinja(e)
            }
        })?;

        let output = template.render(ctx).map_err(TemplateError::MiniJinja)?;
        Ok(output)
    }

    /// Render a template by running all registered [`ContextProcessor`]s against `req`
    /// and merging `extra_ctx` on top (view context overrides processor keys).
    pub fn render_with_request(
        &self,
        req: &djangors_core::Request,
        name: &str,
        extra_ctx: impl Serialize,
    ) -> Result<String, TemplateError> {
        let mut merged: BTreeMap<String, Value> = BTreeMap::new();
        {
            let processors = self.context_processors.lock().unwrap();
            for processor in processors.iter() {
                for (k, v) in processor.process(Some(req)) {
                    merged.insert(k, v);
                }
            }
        }

        let extra_val = Value::from_serialize(&extra_ctx);
        if extra_val.kind() == ValueKind::Map {
            if let Ok(iter) = extra_val.try_iter() {
                for key in iter {
                    if let Ok(val) = extra_val.get_item(&key) {
                        merged.insert(key.to_string(), val);
                    }
                }
            }
        }

        self.render(name, Value::from(merged))
    }

    /// Render a named inline partial from a template (Django 6.0 template partials).
    ///
    /// Searches the template source for `{# partialdef <partial_name> #}...{# endpartialdef #}`
    /// first, falling back to `{% block <partial_name> %}...{% endblock %}`.
    pub fn render_partial(
        &self,
        template_name: &str,
        partial_name: &str,
        ctx: impl Serialize,
    ) -> Result<String, TemplateError> {
        let source = self.load_template_source(template_name)?;
        let fragment = extract_partial(&source, partial_name).ok_or_else(|| {
            TemplateError::PartialNotFound {
                template: template_name.to_string(),
                partial: partial_name.to_string(),
            }
        })?;

        self.env
            .render_named_str(template_name, &fragment, ctx)
            .map_err(TemplateError::MiniJinja)
    }

    fn load_template_source(&self, name: &str) -> Result<String, TemplateError> {
        if let Some(src) = self.embedded_sources.get(name) {
            return Ok(src.clone());
        }
        for dir in self.search_dirs.as_ref() {
            let path = dir.join(name);
            if path.exists() {
                return std::fs::read_to_string(&path).map_err(TemplateError::Io);
            }
        }
        Err(TemplateError::NotFound {
            name: name.to_string(),
            searched: self.search_dirs.as_ref().clone(),
        })
    }
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;
