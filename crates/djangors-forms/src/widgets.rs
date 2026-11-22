//! HTML form widget rendering layer.
//!
//! Provides the [`Widget`] trait and built-in widget implementations for rendering
//! HTML form controls. All string inputs (field names, values, option labels, and
//! attributes) are HTML-escaped using [`html_escape`].

use std::collections::BTreeMap;

/// Escapes HTML special characters in string input.
///
/// Converts `<`, `>`, `&`, `"`, `'`, and `/` into HTML entities.
///
/// # Why this duplicates `djangors_core::html_escape`
///
/// It is character-for-character identical to the one in `djangors-core`, and that is
/// deliberate rather than an oversight. `djangors-forms` **cannot** depend on
/// `djangors-core`: the dependency chain runs `djangors-core` -> `djangors-orm` ->
/// `djangors-forms`, so importing core here would close a cycle and the workspace would
/// stop building.
///
/// If you are tempted to unify these, the only safe route is to extract escaping into a
/// third crate that both depend on. Do not "simplify" this by adding a core dependency.
///
/// Both copies are security-relevant: every widget below reflects previously-submitted
/// values back into HTML attributes, so a change to one must be mirrored in the other.
pub fn html_escape(input: &str) -> String {
    let mut escaped = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '&' => escaped.push_str("&amp;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#x27;"),
            '/' => escaped.push_str("&#x2F;"),
            _ => escaped.push(c),
        }
    }
    escaped
}

/// Attributes applied to HTML widget elements (e.g. `class`, `id`, `placeholder`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WidgetAttrs {
    map: BTreeMap<String, String>,
}

impl WidgetAttrs {
    /// Creates a new, empty set of widget attributes.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets an attribute key and value.
    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.map.insert(key.into(), value.into());
    }

    /// Gets an attribute value by key.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.map.get(key).map(|s| s.as_str())
    }

    /// Removes an attribute by key.
    pub fn remove(&mut self, key: &str) -> Option<String> {
        self.map.remove(key)
    }

    /// Checks if the attribute map is empty.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Iterates over key-value attribute pairs.
    pub fn iter(&self) -> std::collections::btree_map::Iter<'_, String, String> {
        self.map.iter()
    }
}

impl<K: Into<String>, V: Into<String>> FromIterator<(K, V)> for WidgetAttrs {
    fn from_iter<T: IntoIterator<Item = (K, V)>>(iter: T) -> Self {
        let mut attrs = WidgetAttrs::new();
        for (k, v) in iter {
            attrs.set(k, v);
        }
        attrs
    }
}

impl<K: Into<String>, V: Into<String>, const N: usize> From<[(K, V); N]> for WidgetAttrs {
    fn from(arr: [(K, V); N]) -> Self {
        arr.into_iter().collect()
    }
}

/// Trait implemented by form widgets that render as HTML controls.
pub trait Widget: Send + Sync {
    /// Render this widget as HTML for the given field name and current value.
    fn render(&self, name: &str, value: Option<&str>, attrs: &WidgetAttrs) -> String;
}

fn render_attrs(name: &str, attrs: &WidgetAttrs) -> String {
    let mut out = String::new();

    let id_already_present = attrs.get("id").is_some();
    if !id_already_present && !name.is_empty() {
        let default_id = format!("id_{}", name);
        out.push_str(&format!(" id=\"{}\"", html_escape(&default_id)));
    }

    for (k, v) in attrs.iter() {
        out.push_str(&format!(" {}=\"{}\"", html_escape(k), html_escape(v)));
    }

    out
}

/// Single-line plain text input (`<input type="text">`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextInput;

impl Widget for TextInput {
    fn render(&self, name: &str, value: Option<&str>, attrs: &WidgetAttrs) -> String {
        let esc_name = html_escape(name);
        let attrs_str = render_attrs(name, attrs);
        let val_attr = match value {
            Some(val) => format!(" value=\"{}\"", html_escape(val)),
            None => String::new(),
        };
        format!(
            "<input type=\"text\" name=\"{}\"{}{}>",
            esc_name, val_attr, attrs_str
        )
    }
}

/// Multiline text area (`<textarea>`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Textarea;

impl Widget for Textarea {
    fn render(&self, name: &str, value: Option<&str>, attrs: &WidgetAttrs) -> String {
        let esc_name = html_escape(name);
        let attrs_str = render_attrs(name, attrs);
        let val_content = value.map(html_escape).unwrap_or_default();
        format!(
            "<textarea name=\"{}\"{}>{}</textarea>",
            esc_name, attrs_str, val_content
        )
    }
}

/// Numeric input widget (`<input type="number">`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NumberInput;

impl Widget for NumberInput {
    fn render(&self, name: &str, value: Option<&str>, attrs: &WidgetAttrs) -> String {
        let esc_name = html_escape(name);
        let attrs_str = render_attrs(name, attrs);
        let val_attr = match value {
            Some(val) => format!(" value=\"{}\"", html_escape(val)),
            None => String::new(),
        };
        format!(
            "<input type=\"number\" name=\"{}\"{}{}>",
            esc_name, val_attr, attrs_str
        )
    }
}

/// Checkbox widget (`<input type="checkbox">`).
///
/// Evaluates boolean-ish truthiness (`"true"`, `"on"`, `"1"`, `"checked"`) to render
/// the boolean `checked` attribute. Does not emit `checked="false"`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CheckboxInput;

impl Widget for CheckboxInput {
    fn render(&self, name: &str, value: Option<&str>, attrs: &WidgetAttrs) -> String {
        let esc_name = html_escape(name);
        let attrs_str = render_attrs(name, attrs);
        let is_checked = matches!(
            value.map(|s| s.trim()),
            Some("true" | "on" | "1" | "checked")
        );
        let checked_str = if is_checked { " checked" } else { "" };
        format!(
            "<input type=\"checkbox\" name=\"{}\"{}{}>",
            esc_name, checked_str, attrs_str
        )
    }
}

/// Dropdown select list (`<select>`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Select {
    /// Vector of (option_value, option_label) pairs.
    pub choices: Vec<(String, String)>,
}

impl Select {
    /// Creates a new `Select` widget with choices.
    pub fn new(choices: Vec<(String, String)>) -> Self {
        Self { choices }
    }
}

impl Widget for Select {
    fn render(&self, name: &str, value: Option<&str>, attrs: &WidgetAttrs) -> String {
        let esc_name = html_escape(name);
        let attrs_str = render_attrs(name, attrs);
        let mut options_html = String::new();

        for (val, label) in &self.choices {
            let esc_val = html_escape(val);
            let esc_label = html_escape(label);
            let selected_str = if value == Some(val.as_str()) {
                " selected"
            } else {
                ""
            };
            options_html.push_str(&format!(
                "<option value=\"{}\"{}>{}</option>",
                esc_val, selected_str, esc_label
            ));
        }

        format!(
            "<select name=\"{}\"{}>{}</select>",
            esc_name, attrs_str, options_html
        )
    }
}

/// Radio button list (`<input type="radio">`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RadioSelect {
    /// Vector of (option_value, option_label) pairs.
    pub choices: Vec<(String, String)>,
}

impl RadioSelect {
    /// Creates a new `RadioSelect` widget with choices.
    pub fn new(choices: Vec<(String, String)>) -> Self {
        Self { choices }
    }
}

impl Widget for RadioSelect {
    fn render(&self, name: &str, value: Option<&str>, attrs: &WidgetAttrs) -> String {
        let esc_name = html_escape(name);
        let base_id = attrs
            .get("id")
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("id_{}", name));
        let esc_base_id = html_escape(&base_id);

        let mut items_html = String::new();
        for (i, (val, label)) in self.choices.iter().enumerate() {
            let esc_val = html_escape(val);
            let esc_label = html_escape(label);
            let opt_id = format!("{}_{}", base_id, i);
            let esc_opt_id = html_escape(&opt_id);
            let checked_str = if value == Some(val.as_str()) {
                " checked"
            } else {
                ""
            };

            items_html.push_str(&format!(
                "<div><label for=\"{}\"><input type=\"radio\" name=\"{}\" value=\"{}\" id=\"{}\"{}> {}</label></div>",
                esc_opt_id, esc_name, esc_val, esc_opt_id, checked_str, esc_label
            ));
        }

        format!("<div id=\"{}\">{}</div>", esc_base_id, items_html)
    }
}

/// Date input widget (`<input type="date">`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DateInput;

impl Widget for DateInput {
    fn render(&self, name: &str, value: Option<&str>, attrs: &WidgetAttrs) -> String {
        let esc_name = html_escape(name);
        let attrs_str = render_attrs(name, attrs);
        let val_attr = match value {
            Some(val) => format!(" value=\"{}\"", html_escape(val)),
            None => String::new(),
        };
        format!(
            "<input type=\"date\" name=\"{}\"{}{}>",
            esc_name, val_attr, attrs_str
        )
    }
}

/// Email input widget (`<input type="email">`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EmailInput;

impl Widget for EmailInput {
    fn render(&self, name: &str, value: Option<&str>, attrs: &WidgetAttrs) -> String {
        let esc_name = html_escape(name);
        let attrs_str = render_attrs(name, attrs);
        let val_attr = match value {
            Some(val) => format!(" value=\"{}\"", html_escape(val)),
            None => String::new(),
        };
        format!(
            "<input type=\"email\" name=\"{}\"{}{}>",
            esc_name, val_attr, attrs_str
        )
    }
}

/// Password input widget (`<input type="password">`).
///
/// **Security Notice:** By default, `PasswordInput` does NOT render the submitted
/// password back into the `value` attribute (`render_value = false`). This prevents
/// sensitive cleartext passwords from being exposed in rendered HTML forms or cached responses.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PasswordInput {
    /// Whether to render the password value into the `value` attribute.
    /// Defaults to `false` for security.
    pub render_value: bool,
}

impl PasswordInput {
    /// Creates a new `PasswordInput` widget with default secure settings (`render_value = false`).
    pub fn new() -> Self {
        Self::default()
    }
}

impl Widget for PasswordInput {
    fn render(&self, name: &str, value: Option<&str>, attrs: &WidgetAttrs) -> String {
        let esc_name = html_escape(name);
        let attrs_str = render_attrs(name, attrs);
        let val_attr = if self.render_value {
            match value {
                Some(val) => format!(" value=\"{}\"", html_escape(val)),
                None => String::new(),
            }
        } else {
            String::new()
        };
        format!(
            "<input type=\"password\" name=\"{}\"{}{}>",
            esc_name, val_attr, attrs_str
        )
    }
}

/// Hidden input widget (`<input type="hidden">`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HiddenInput;

impl Widget for HiddenInput {
    fn render(&self, name: &str, value: Option<&str>, attrs: &WidgetAttrs) -> String {
        let esc_name = html_escape(name);
        let attrs_str = render_attrs(name, attrs);
        let val_attr = match value {
            Some(val) => format!(" value=\"{}\"", html_escape(val)),
            None => String::new(),
        };
        format!(
            "<input type=\"hidden\" name=\"{}\"{}{}>",
            esc_name, val_attr, attrs_str
        )
    }
}

/// File upload input widget (`<input type="file">`).
///
/// For HTML security and specification compliance, `<input type="file">` does not
/// populate a `value` attribute.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileInput;

impl Widget for FileInput {
    fn render(&self, name: &str, _value: Option<&str>, attrs: &WidgetAttrs) -> String {
        let esc_name = html_escape(name);
        let attrs_str = render_attrs(name, attrs);
        format!("<input type=\"file\" name=\"{}\"{}>", esc_name, attrs_str)
    }
}

/// Clearable file upload widget displaying the current file with a "Clear" checkbox.
///
/// When `value` is `Some(non_empty)`, renders the current file indicator, a
/// `{name}-clear` checkbox, and the `<input type="file">` control. When `value` is
/// `None` or empty, renders only the `<input type="file">` control.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClearableFileInput {
    /// Label displayed before the current file value (default: `"Currently"`).
    pub initial_text: String,
    /// Label displayed next to the clear checkbox (default: `"Clear"`).
    pub clear_checkbox_label: String,
    /// Label displayed before the file input when a current value exists (default: `"Change"`).
    pub input_text: String,
}

impl Default for ClearableFileInput {
    fn default() -> Self {
        Self {
            initial_text: "Currently".to_string(),
            clear_checkbox_label: "Clear".to_string(),
            input_text: "Change".to_string(),
        }
    }
}

impl ClearableFileInput {
    /// Creates a new `ClearableFileInput` with default Django labels.
    pub fn new() -> Self {
        Self::default()
    }
}

impl Widget for ClearableFileInput {
    fn render(&self, name: &str, value: Option<&str>, attrs: &WidgetAttrs) -> String {
        let file_html = FileInput.render(name, None, attrs);
        match value {
            Some(current) if !current.is_empty() => {
                let esc_initial = html_escape(&self.initial_text);
                let esc_current = html_escape(current);
                let clear_name = format!("{}-clear", name);
                let clear_id = format!("{}-clear_id", name);
                let esc_clear_name = html_escape(&clear_name);
                let esc_clear_id = html_escape(&clear_id);
                let esc_clear_label = html_escape(&self.clear_checkbox_label);
                let esc_input_text = html_escape(&self.input_text);

                format!(
                    "{}: <a href=\"{}\">{}</a> <input type=\"checkbox\" name=\"{}\" id=\"{}\"> <label for=\"{}\">{}</label><br>{}: {}",
                    esc_initial,
                    esc_current,
                    esc_current,
                    esc_clear_name,
                    esc_clear_id,
                    esc_clear_id,
                    esc_clear_label,
                    esc_input_text,
                    file_html
                )
            }
            _ => file_html,
        }
    }
}

fn parse_multi_values(value: Option<&str>) -> Vec<&str> {
    match value {
        Some(v) if !v.is_empty() => v
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

/// Multi-select dropdown list (`<select multiple>`).
///
/// Accepts comma-separated selected values in `value`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SelectMultiple {
    /// Vector of `(option_value, option_label)` pairs.
    pub choices: Vec<(String, String)>,
}

impl SelectMultiple {
    /// Creates a new `SelectMultiple` widget with the given choices.
    pub fn new(choices: Vec<(String, String)>) -> Self {
        Self { choices }
    }
}

impl Widget for SelectMultiple {
    fn render(&self, name: &str, value: Option<&str>, attrs: &WidgetAttrs) -> String {
        let esc_name = html_escape(name);
        let attrs_str = render_attrs(name, attrs);
        let selected_values = parse_multi_values(value);
        let mut options_html = String::new();

        for (val, label) in &self.choices {
            let esc_val = html_escape(val);
            let esc_label = html_escape(label);
            let selected_str = if selected_values.contains(&val.as_str()) {
                " selected"
            } else {
                ""
            };
            options_html.push_str(&format!(
                "<option value=\"{}\"{}>{}</option>",
                esc_val, selected_str, esc_label
            ));
        }

        format!(
            "<select name=\"{}\" multiple{}>{}</select>",
            esc_name, attrs_str, options_html
        )
    }
}

/// Multiple checkbox list widget (`<input type="checkbox">` per choice).
///
/// Accepts comma-separated selected values in `value`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CheckboxSelectMultiple {
    /// Vector of `(option_value, option_label)` pairs.
    pub choices: Vec<(String, String)>,
}

impl CheckboxSelectMultiple {
    /// Creates a new `CheckboxSelectMultiple` widget with the given choices.
    pub fn new(choices: Vec<(String, String)>) -> Self {
        Self { choices }
    }
}

impl Widget for CheckboxSelectMultiple {
    fn render(&self, name: &str, value: Option<&str>, attrs: &WidgetAttrs) -> String {
        let esc_name = html_escape(name);
        let base_id = attrs
            .get("id")
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("id_{}", name));
        let esc_base_id = html_escape(&base_id);
        let selected_values = parse_multi_values(value);

        let mut items_html = String::new();
        for (i, (val, label)) in self.choices.iter().enumerate() {
            let esc_val = html_escape(val);
            let esc_label = html_escape(label);
            let opt_id = format!("{}_{}", base_id, i);
            let esc_opt_id = html_escape(&opt_id);
            let checked_str = if selected_values.contains(&val.as_str()) {
                " checked"
            } else {
                ""
            };

            items_html.push_str(&format!(
                "<div><label for=\"{}\"><input type=\"checkbox\" name=\"{}\" value=\"{}\" id=\"{}\"{}> {}</label></div>",
                esc_opt_id, esc_name, esc_val, esc_opt_id, checked_str, esc_label
            ));
        }

        format!("<div id=\"{}\">{}</div>", esc_base_id, items_html)
    }
}

/// Date-time input widget (`<input type="datetime-local">`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DateTimeInput;

impl Widget for DateTimeInput {
    fn render(&self, name: &str, value: Option<&str>, attrs: &WidgetAttrs) -> String {
        let esc_name = html_escape(name);
        let attrs_str = render_attrs(name, attrs);
        let val_attr = match value {
            Some(val) => format!(" value=\"{}\"", html_escape(val)),
            None => String::new(),
        };
        format!(
            "<input type=\"datetime-local\" name=\"{}\"{}{}>",
            esc_name, val_attr, attrs_str
        )
    }
}

/// Time input widget (`<input type="time">`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TimeInput;

impl Widget for TimeInput {
    fn render(&self, name: &str, value: Option<&str>, attrs: &WidgetAttrs) -> String {
        let esc_name = html_escape(name);
        let attrs_str = render_attrs(name, attrs);
        let val_attr = match value {
            Some(val) => format!(" value=\"{}\"", html_escape(val)),
            None => String::new(),
        };
        format!(
            "<input type=\"time\" name=\"{}\"{}{}>",
            esc_name, val_attr, attrs_str
        )
    }
}

/// URL input widget (`<input type="url">`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UrlInput;

impl Widget for UrlInput {
    fn render(&self, name: &str, value: Option<&str>, attrs: &WidgetAttrs) -> String {
        let esc_name = html_escape(name);
        let attrs_str = render_attrs(name, attrs);
        let val_attr = match value {
            Some(val) => format!(" value=\"{}\"", html_escape(val)),
            None => String::new(),
        };
        format!(
            "<input type=\"url\" name=\"{}\"{}{}>",
            esc_name, val_attr, attrs_str
        )
    }
}

/// Composite widget splitting a date-time value into separate date (`{name}_0`)
/// and time (`{name}_1`) inputs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SplitDateTimeWidget;

impl Widget for SplitDateTimeWidget {
    fn render(&self, name: &str, value: Option<&str>, attrs: &WidgetAttrs) -> String {
        let (date_val, time_val) = match value {
            Some(v) if !v.trim().is_empty() => {
                let trimmed = v.trim();
                if let Some((d, t)) = trimmed.split_once('T').or_else(|| trimmed.split_once(' ')) {
                    (Some(d), Some(t))
                } else {
                    (Some(trimmed), None)
                }
            }
            _ => (None, None),
        };

        let date_name = format!("{}_0", name);
        let time_name = format!("{}_1", name);

        let mut date_attrs = attrs.clone();
        let mut time_attrs = attrs.clone();
        if let Some(base_id) = attrs.get("id") {
            date_attrs.set("id", format!("{}_0", base_id));
            time_attrs.set("id", format!("{}_1", base_id));
        }

        let date_html = DateInput.render(&date_name, date_val, &date_attrs);
        let time_html = TimeInput.render(&time_name, time_val, &time_attrs);
        format!("{}{}", date_html, time_html)
    }
}
