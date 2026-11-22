//! FormSets for managing multiple form instances submitted together in an HTTP request.
//!
//! Provides [`ManagementForm`], [`FormSet`], [`FormSetResult`], and [`InlineFormSet`] for
//! prefixing (`form-0-title`, ...), deletion and ordering tracking, min/max count validation,
//! parent foreign-key injection, and security caps on `TOTAL_FORMS`.

use std::collections::HashMap;
use std::fmt;

use crate::error::FormErrors;
use crate::widgets::{html_escape, CheckboxInput, HiddenInput, NumberInput, Widget, WidgetAttrs};

/// Error produced during legacy [`FormSet::clean_with`] validation or management form demultiplexing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormSetError {
    /// Errors originating from the management form (e.g. missing `TOTAL_FORMS` or exceeding `max_num`).
    pub management_errors: Vec<String>,
    /// Per-form validation errors across the formset.
    pub form_errors: Vec<Option<FormErrors>>,
}

impl fmt::Display for FormSetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.management_errors.is_empty() {
            write!(
                f,
                "Management form errors: {}",
                self.management_errors.join(", ")
            )
        } else {
            write!(f, "FormSet validation errors in individual forms")
        }
    }
}

impl std::error::Error for FormSetError {}

/// Cleaned result for an individual form within a [`FormSet`] (used by [`FormSet::clean_with`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormSetFormResult<C> {
    /// Typed cleaned data produced by form validation.
    pub cleaned: C,
    /// Whether the `DELETE` checkbox was submitted for this form.
    pub delete: bool,
}

/// Django-style management form tracking formset counts in hidden HTML inputs.
///
/// Manages `{prefix}-TOTAL_FORMS`, `{prefix}-INITIAL_FORMS`, `{prefix}-MIN_NUM_FORMS`,
/// and `{prefix}-MAX_NUM_FORMS`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagementForm {
    /// Formset prefix (e.g. `"form"`).
    pub prefix: String,
    /// Total number of forms submitted (`TOTAL_FORMS`).
    pub total_forms: usize,
    /// Number of initial pre-populated forms (`INITIAL_FORMS`).
    pub initial_forms: usize,
    /// Minimum number of forms (`MIN_NUM_FORMS`).
    pub min_num_forms: usize,
    /// Maximum number of forms (`MAX_NUM_FORMS`).
    pub max_num_forms: usize,
}

impl Default for ManagementForm {
    fn default() -> Self {
        Self {
            prefix: "form".to_string(),
            total_forms: 0,
            initial_forms: 0,
            min_num_forms: 0,
            max_num_forms: 1000,
        }
    }
}

impl ManagementForm {
    /// Creates a new `ManagementForm` with explicit counts.
    pub fn new(
        prefix: impl Into<String>,
        total_forms: usize,
        initial_forms: usize,
        min_num_forms: usize,
        max_num_forms: usize,
    ) -> Self {
        Self {
            prefix: prefix.into(),
            total_forms,
            initial_forms,
            min_num_forms,
            max_num_forms,
        }
    }

    /// Parses management form fields from submitted POST data for the given `prefix`.
    ///
    /// `TOTAL_FORMS` and `INITIAL_FORMS` are required; `MIN_NUM_FORMS` defaults to `0`
    /// and `MAX_NUM_FORMS` defaults to `1000` when omitted.
    pub fn parse(data: &HashMap<String, String>, prefix: &str) -> Result<Self, Vec<String>> {
        let mut errors = Vec::new();

        let total_key = format!("{}-TOTAL_FORMS", prefix);
        let initial_key = format!("{}-INITIAL_FORMS", prefix);
        let min_key = format!("{}-MIN_NUM_FORMS", prefix);
        let max_key = format!("{}-MAX_NUM_FORMS", prefix);

        let total_forms = match data.get(&total_key) {
            Some(s) => match s.trim().parse::<usize>() {
                Ok(n) => n,
                Err(_) => {
                    errors.push("TOTAL_FORMS must be a valid non-negative integer.".to_string());
                    0
                }
            },
            None => {
                errors.push("ManagementForm missing TOTAL_FORMS field.".to_string());
                0
            }
        };

        let initial_forms = match data.get(&initial_key) {
            Some(s) => match s.trim().parse::<usize>() {
                Ok(n) => n,
                Err(_) => {
                    errors.push("INITIAL_FORMS must be a valid non-negative integer.".to_string());
                    0
                }
            },
            None => {
                errors.push("ManagementForm missing INITIAL_FORMS field.".to_string());
                0
            }
        };

        let min_num_forms = match data.get(&min_key) {
            Some(s) if !s.trim().is_empty() => match s.trim().parse::<usize>() {
                Ok(n) => n,
                Err(_) => {
                    errors.push("MIN_NUM_FORMS must be a valid non-negative integer.".to_string());
                    0
                }
            },
            _ => 0,
        };

        let max_num_forms = match data.get(&max_key) {
            Some(s) if !s.trim().is_empty() => match s.trim().parse::<usize>() {
                Ok(n) => n,
                Err(_) => {
                    errors.push("MAX_NUM_FORMS must be a valid non-negative integer.".to_string());
                    1000
                }
            },
            _ => 1000,
        };

        if errors.is_empty() {
            Ok(Self {
                prefix: prefix.to_string(),
                total_forms,
                initial_forms,
                min_num_forms,
                max_num_forms,
            })
        } else {
            Err(errors)
        }
    }

    /// Renders all four management form hidden inputs as HTML.
    pub fn render_html(&self) -> String {
        let hidden = HiddenInput;
        let attrs = WidgetAttrs::new();
        let fields = [
            (
                format!("{}-TOTAL_FORMS", self.prefix),
                self.total_forms.to_string(),
            ),
            (
                format!("{}-INITIAL_FORMS", self.prefix),
                self.initial_forms.to_string(),
            ),
            (
                format!("{}-MIN_NUM_FORMS", self.prefix),
                self.min_num_forms.to_string(),
            ),
            (
                format!("{}-MAX_NUM_FORMS", self.prefix),
                self.max_num_forms.to_string(),
            ),
        ];

        fields
            .iter()
            .map(|(name, val)| hidden.render(name, Some(val), &attrs))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Complete validation result returned by [`FormSet::validate`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormSetResult<C> {
    /// Cleaned data for each valid, non-deleted form.
    pub cleaned_forms: Vec<C>,
    /// Indices of forms marked for deletion via `DELETE` (when `can_delete` is enabled).
    pub deleted_indices: Vec<usize>,
    /// Indices of non-deleted forms ordered by `ORDER` (when `can_order` is enabled)
    /// or in submission order otherwise.
    pub ordered_indices: Vec<usize>,
    /// Per-form validation errors indexed by form index (`None` when that form is valid or deleted).
    pub form_errors: Vec<Option<FormErrors>>,
    /// Formset-level errors (management form errors, `min_num` / `max_num` violations, etc.).
    pub non_form_errors: Vec<String>,
}

impl<C> FormSetResult<C> {
    /// Returns `true` if there are no non-form errors and no per-form errors.
    pub fn is_valid(&self) -> bool {
        self.non_form_errors.is_empty() && self.form_errors.iter().all(Option::is_none)
    }
}

/// A container managing N instances of a form.
#[derive(Debug, Clone)]
pub struct FormSet<F = ()> {
    /// Prefix for form field names in HTTP submissions (default: `"form"`).
    pub prefix: String,
    /// Total number of form instances (for rendering).
    pub total_forms: usize,
    /// Number of initial pre-populated forms (for rendering).
    pub initial_forms: usize,
    /// Number of extra empty forms to display (default: `1`).
    pub extra: usize,
    /// Minimum number of forms required when `validate_min` is enabled (default: `0`).
    pub min_num: usize,
    /// Maximum allowed forms for security and validation (default: `1000`).
    pub max_num: usize,
    /// Whether to enforce `min_num` during [`FormSet::validate`] (default: `false`).
    pub validate_min: bool,
    /// Whether to enforce `max_num` on active forms during [`FormSet::validate`] (default: `false`).
    pub validate_max: bool,
    /// Whether to render and parse a `DELETE` checkbox field per form.
    pub can_delete: bool,
    /// Whether to render and parse an `ORDER` integer field per form.
    pub can_order: bool,
    _marker: std::marker::PhantomData<F>,
}

impl<F> Default for FormSet<F> {
    fn default() -> Self {
        Self {
            prefix: "form".to_string(),
            total_forms: 0,
            initial_forms: 0,
            extra: 1,
            min_num: 0,
            max_num: 1000,
            validate_min: false,
            validate_max: false,
            can_delete: false,
            can_order: false,
            _marker: std::marker::PhantomData,
        }
    }
}

impl<F> FormSet<F> {
    /// Creates a new `FormSet` with default configuration (`prefix = "form"`, `extra = 1`, `max_num = 1000`).
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets form prefix (e.g. `"inline_author"`).
    pub fn with_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.prefix = prefix.into();
        self
    }

    /// Sets total and initial form counts for rendering.
    pub fn with_counts(mut self, total: usize, initial: usize) -> Self {
        self.total_forms = total;
        self.initial_forms = initial;
        self
    }

    /// Sets the number of extra empty forms to display.
    pub fn with_extra(mut self, extra: usize) -> Self {
        self.extra = extra;
        self
    }

    /// Sets minimum required forms.
    pub fn with_min_num(mut self, min_num: usize) -> Self {
        self.min_num = min_num;
        self
    }

    /// Sets maximum allowed total forms.
    pub fn with_max_num(mut self, max_num: usize) -> Self {
        self.max_num = max_num;
        self
    }

    /// Enables or disables `min_num` enforcement during validation.
    pub fn with_validate_min(mut self, validate_min: bool) -> Self {
        self.validate_min = validate_min;
        self
    }

    /// Enables or disables `max_num` enforcement during validation.
    pub fn with_validate_max(mut self, validate_max: bool) -> Self {
        self.validate_max = validate_max;
        self
    }

    /// Enables or disables `can_delete` support.
    pub fn with_can_delete(mut self, can_delete: bool) -> Self {
        self.can_delete = can_delete;
        self
    }

    /// Enables or disables `can_order` support.
    pub fn with_can_order(mut self, can_order: bool) -> Self {
        self.can_order = can_order;
        self
    }

    /// Generates field name for form at index `i` (e.g. `"form-0-title"`).
    pub fn add_prefix(&self, index: usize, field_name: &str) -> String {
        format!("{}-{}-{}", self.prefix, index, field_name)
    }

    /// Builds a [`ManagementForm`] snapshot reflecting this formset's configuration.
    pub fn management_form(&self) -> ManagementForm {
        ManagementForm::new(
            self.prefix.clone(),
            self.total_forms,
            self.initial_forms,
            self.min_num,
            self.max_num,
        )
    }

    /// Renders management form hidden inputs (`TOTAL_FORMS`, `INITIAL_FORMS`,
    /// `MIN_NUM_FORMS`, and `MAX_NUM_FORMS`).
    pub fn render_management_form(&self) -> String {
        self.management_form().render_html()
    }

    /// Renders `DELETE` checkbox for form at index `i` if `can_delete` is enabled.
    pub fn render_delete_checkbox(&self, index: usize, checked: bool) -> String {
        if !self.can_delete {
            return String::new();
        }
        let name = self.add_prefix(index, "DELETE");
        let esc_name = html_escape(&name);
        let val_str = if checked { Some("on") } else { None };
        let checkbox = CheckboxInput.render(&name, val_str, &WidgetAttrs::new());
        format!("<label for=\"id_{}\">{} Delete</label>", esc_name, checkbox)
    }

    /// Renders `ORDER` number input for form at index `i` if `can_order` is enabled.
    pub fn render_order_input(&self, index: usize, value: Option<i64>) -> String {
        if !self.can_order {
            return String::new();
        }
        let name = self.add_prefix(index, "ORDER");
        let esc_name = html_escape(&name);
        let val_owned = value.map(|v| v.to_string());
        let input = NumberInput.render(&name, val_owned.as_deref(), &WidgetAttrs::new());
        format!("<label for=\"id_{}\">Order {}</label>", esc_name, input)
    }

    /// Validates submitted POST data with `row_validator` and returns a rich [`FormSetResult`].
    ///
    /// Enforces management form parsing, `max_num` security cap, `DELETE` tracking,
    /// `ORDER` sorting, and `validate_min` / `validate_max` bounds.
    pub fn validate<C, RowValidator>(
        &self,
        data: &HashMap<String, String>,
        row_validator: RowValidator,
    ) -> FormSetResult<C>
    where
        RowValidator: Fn(&HashMap<String, String>) -> Result<C, FormErrors>,
    {
        let mgmt = match ManagementForm::parse(data, &self.prefix) {
            Ok(m) => m,
            Err(errs) => {
                return FormSetResult {
                    cleaned_forms: Vec::new(),
                    deleted_indices: Vec::new(),
                    ordered_indices: Vec::new(),
                    form_errors: Vec::new(),
                    non_form_errors: errs,
                };
            }
        };

        // Hard security cap before allocating per-form buffers
        let hard_cap = self.max_num.max(mgmt.max_num_forms);
        if mgmt.total_forms > hard_cap && !self.validate_max {
            return FormSetResult {
                cleaned_forms: Vec::new(),
                deleted_indices: Vec::new(),
                ordered_indices: Vec::new(),
                form_errors: Vec::new(),
                non_form_errors: vec![format!(
                    "TOTAL_FORMS ({}) exceeds maximum allowed ({}).",
                    mgmt.total_forms, hard_cap
                )],
            };
        }
        // Always prevent unbounded allocation even when validate_max is true
        if mgmt.total_forms > self.max_num.saturating_add(1000) {
            return FormSetResult {
                cleaned_forms: Vec::new(),
                deleted_indices: Vec::new(),
                ordered_indices: Vec::new(),
                form_errors: Vec::new(),
                non_form_errors: vec![format!("Please submit at most {} forms.", self.max_num)],
            };
        }

        let mut cleaned_forms = Vec::new();
        let mut deleted_indices = Vec::new();
        let mut form_errors = Vec::with_capacity(mgmt.total_forms);
        let mut non_form_errors = Vec::new();
        let mut order_entries: Vec<(usize, Option<i64>)> = Vec::new();

        let prefix_dash = format!("{}-", self.prefix);

        for i in 0..mgmt.total_forms {
            let form_prefix = format!("{}{}-", prefix_dash, i);
            let mut sub_map = HashMap::new();

            for (k, v) in data {
                if let Some(stripped) = k.strip_prefix(&form_prefix) {
                    sub_map.insert(stripped.to_string(), v.clone());
                }
            }

            let delete_checked = self.can_delete
                && matches!(
                    sub_map.get("DELETE").map(String::as_str),
                    Some("on" | "true" | "1")
                );

            if delete_checked {
                deleted_indices.push(i);
                form_errors.push(None);
                continue;
            }

            let mut order_error: Option<String> = None;
            let order_val = if self.can_order {
                match sub_map.get("ORDER").map(|s| s.trim()) {
                    None | Some("") => None,
                    Some(raw_ord) => match raw_ord.parse::<i64>() {
                        Ok(n) => Some(n),
                        Err(_) => {
                            order_error = Some("Enter a whole number.".to_string());
                            None
                        }
                    },
                }
            } else {
                None
            };

            order_entries.push((i, order_val));

            match row_validator(&sub_map) {
                Ok(cleaned) => {
                    if let Some(ord_err) = order_error {
                        let mut errs = FormErrors::new();
                        errs.add_field_error("ORDER", ord_err);
                        form_errors.push(Some(errs));
                    } else {
                        cleaned_forms.push(cleaned);
                        form_errors.push(None);
                    }
                }
                Err(mut errs) => {
                    if let Some(ord_err) = order_error {
                        errs.add_field_error("ORDER", ord_err);
                    }
                    form_errors.push(Some(errs));
                }
            }
        }

        let active_count = mgmt.total_forms.saturating_sub(deleted_indices.len());
        if self.validate_min && active_count < self.min_num {
            non_form_errors.push(format!("Please submit at least {} forms.", self.min_num));
        }
        if self.validate_max && active_count > self.max_num {
            non_form_errors.push(format!("Please submit at most {} forms.", self.max_num));
        }

        if self.can_order {
            order_entries.sort_by_key(|&(idx, ord)| (ord.is_none(), ord.unwrap_or(i64::MAX), idx));
        }
        let ordered_indices = order_entries.into_iter().map(|(idx, _)| idx).collect();

        FormSetResult {
            cleaned_forms,
            deleted_indices,
            ordered_indices,
            form_errors,
            non_form_errors,
        }
    }

    /// Demultiplexes POST submission data into N form maps and cleans each using `clean_fn`.
    ///
    /// Checks `TOTAL_FORMS` against `self.max_num` before allocating form instances to prevent
    /// memory exhaustion attacks.
    pub fn clean_with<C, CleanFn>(
        &self,
        data: &HashMap<String, String>,
        clean_fn: CleanFn,
    ) -> Result<Vec<FormSetFormResult<C>>, FormSetError>
    where
        CleanFn: Fn(&HashMap<String, String>) -> Result<C, FormErrors>,
    {
        let total_key = format!("{}-TOTAL_FORMS", self.prefix);
        let initial_key = format!("{}-INITIAL_FORMS", self.prefix);

        let mut mgmt_errors = Vec::new();

        let total_forms = match data.get(&total_key) {
            Some(s) => match s.parse::<usize>() {
                Ok(n) => n,
                Err(_) => {
                    mgmt_errors
                        .push("TOTAL_FORMS must be a valid non-negative integer.".to_string());
                    0
                }
            },
            None => {
                mgmt_errors.push("ManagementForm missing TOTAL_FORMS field.".to_string());
                0
            }
        };

        let _initial_forms = match data.get(&initial_key) {
            Some(s) => s.parse::<usize>().unwrap_or(0),
            None => 0,
        };

        if total_forms > self.max_num {
            mgmt_errors.push(format!(
                "TOTAL_FORMS ({}) exceeds maximum allowed ({})",
                total_forms, self.max_num
            ));
        }

        if !mgmt_errors.is_empty() {
            return Err(FormSetError {
                management_errors: mgmt_errors,
                form_errors: Vec::new(),
            });
        }

        let mut cleaned_forms = Vec::with_capacity(total_forms);
        let mut form_errors = Vec::with_capacity(total_forms);
        let mut has_errors = false;

        let prefix_dash = format!("{}-", self.prefix);

        for i in 0..total_forms {
            let form_prefix = format!("{}{}-", prefix_dash, i);
            let mut sub_map = HashMap::new();

            for (k, v) in data {
                if let Some(stripped) = k.strip_prefix(&form_prefix) {
                    sub_map.insert(stripped.to_string(), v.clone());
                }
            }

            let delete_checked = if self.can_delete {
                matches!(
                    sub_map.get("DELETE").map(|s| s.as_str()),
                    Some("on" | "true" | "1")
                )
            } else {
                false
            };

            match clean_fn(&sub_map) {
                Ok(cleaned) => {
                    cleaned_forms.push(FormSetFormResult {
                        cleaned,
                        delete: delete_checked,
                    });
                    form_errors.push(None);
                }
                Err(errs) => {
                    has_errors = true;
                    form_errors.push(Some(errs));
                }
            }
        }

        if has_errors {
            Err(FormSetError {
                management_errors: Vec::new(),
                form_errors,
            })
        } else {
            Ok(cleaned_forms)
        }
    }
}

/// Inline formset helper wrapping [`FormSet`] to inject a parent foreign key into every row.
///
/// Mirrors Django's `BaseInlineFormSet`: automatically populates `parent_fk_name` with
/// `parent_pk` in each child form's data map prior to validation.
#[derive(Debug, Clone)]
pub struct InlineFormSet<F = ()> {
    /// Underlying formset managing the child forms.
    pub formset: FormSet<F>,
    /// Foreign-key field name on the child form (e.g. `"author_id"` or `"order"`).
    pub parent_fk_name: String,
    /// Primary key value of the parent object, injected into every child form.
    pub parent_pk: String,
}

impl<F> InlineFormSet<F> {
    /// Creates a new `InlineFormSet` bound to a parent foreign key field name and primary key.
    pub fn new(parent_fk_name: impl Into<String>, parent_pk: impl Into<String>) -> Self {
        Self {
            formset: FormSet::new(),
            parent_fk_name: parent_fk_name.into(),
            parent_pk: parent_pk.into(),
        }
    }

    /// Replaces the underlying [`FormSet`] configuration.
    pub fn with_formset(mut self, formset: FormSet<F>) -> Self {
        self.formset = formset;
        self
    }

    /// Sets the formset prefix.
    pub fn with_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.formset = self.formset.with_prefix(prefix);
        self
    }

    /// Enables or disables `can_delete` on the inner formset.
    pub fn with_can_delete(mut self, can_delete: bool) -> Self {
        self.formset = self.formset.with_can_delete(can_delete);
        self
    }

    /// Enables or disables `can_order` on the inner formset.
    pub fn with_can_order(mut self, can_order: bool) -> Self {
        self.formset = self.formset.with_can_order(can_order);
        self
    }

    /// Renders the management form HTML for the inner formset.
    pub fn render_management_form(&self) -> String {
        self.formset.render_management_form()
    }

    /// Validates submitted POST data, injecting `(parent_fk_name, parent_pk)` into every
    /// child form's data map before invoking `row_validator`.
    pub fn validate<C, RowValidator>(
        &self,
        data: &HashMap<String, String>,
        row_validator: RowValidator,
    ) -> FormSetResult<C>
    where
        RowValidator: Fn(&HashMap<String, String>) -> Result<C, FormErrors>,
    {
        self.formset.validate(data, |sub_map| {
            let mut enriched = sub_map.clone();
            enriched.insert(self.parent_fk_name.clone(), self.parent_pk.clone());
            row_validator(&enriched)
        })
    }
}
