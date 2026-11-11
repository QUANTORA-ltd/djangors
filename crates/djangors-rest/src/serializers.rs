//! Model serialization for REST endpoints.

use std::collections::HashMap;

use djangors_orm::expr::Value;
use djangors_orm::meta::{FieldKind, Model};

use crate::*;

/// Serializes any [`Model`]'s `field_values()` into a `serde_json::Value` object.
/// Relation fields serialize as their raw related id integer/null.
pub fn serialize<M: Model>(instance: &M) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for (name, value) in instance.field_values() {
        map.insert(name.to_string(), value_to_json(value));
    }
    serde_json::Value::Object(map)
}

/// Serializes a [`Model`] directly into a UTF-8 JSON byte buffer without
/// allocating an intermediate `Vec<(&'static str, Value)>` or `serde_json::Map`.
pub fn serialize_to_vec<M: Model>(instance: &M) -> Vec<u8> {
    let mut buf = Vec::with_capacity(128);
    instance.write_json(&mut buf);
    buf
}

/// Serializes a slice of [`Model`] instances directly into a JSON array byte buffer
/// without intermediate `serde_json::Value` allocations.
pub fn serialize_many_to_vec<M: Model>(instances: &[M]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(instances.len().saturating_mul(128).saturating_add(2));
    buf.push(b'[');
    for (idx, item) in instances.iter().enumerate() {
        if idx > 0 {
            buf.push(b',');
        }
        item.write_json(&mut buf);
    }
    buf.push(b']');
    buf
}

/// Converts a single ORM [`Value`] into JSON.
///
/// Split out from [`serialize`] so the list case can recurse; a `Value::List`
/// only reaches here through a projection such as
/// [`QuerySet::values`](djangors_orm::queryset::QuerySet::values), not from a
/// plain model field.
pub fn value_to_json(value: Value) -> serde_json::Value {
    match value {
        Value::I64(n) => serde_json::Value::Number(n.into()),
        Value::F64(f) => serde_json::Number::from_f64(f)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        Value::Text(s) => serde_json::Value::String(s),
        Value::Bool(b) => serde_json::Value::Bool(b),
        Value::DateTime(dt) => serde_json::Value::String(dt.to_rfc3339()),
        Value::Null => serde_json::Value::Null,
        Value::List(items) => {
            serde_json::Value::Array(items.into_iter().map(value_to_json).collect())
        }
    }
}

/// Deserializes a JSON object into field values suitable for `QuerySet::insert_raw` / `update`,
/// using per-`FieldKind` parsing conventions. Collects ALL validation errors at once into a
/// `HashMap<String, String>` (mapping field names to error messages).
pub fn deserialize<M: Model>(
    json: &serde_json::Value,
) -> Result<Vec<(&'static str, Value)>, HashMap<String, String>> {
    deserialize_inner::<M>(json, false)
}

/// Parses only the fields the body actually supplied.
///
/// [`deserialize`] is whole-object: it walks every column, defaults the absent
/// ones, and reports the rest as required. That is right for a full write and
/// wrong for `PATCH`, where an omitted field means "leave it alone" — so
/// partial writes skip absent keys entirely instead of defaulting them.
///
/// An explicit `null` is still honoured (and still rejected on a non-nullable
/// column); only *absent* keys are skipped.
pub fn deserialize_partial<M: Model>(
    json: &serde_json::Value,
) -> Result<Vec<(&'static str, Value)>, HashMap<String, String>> {
    deserialize_inner::<M>(json, true)
}

fn deserialize_inner<M: Model>(
    json: &serde_json::Value,
    partial: bool,
) -> Result<Vec<(&'static str, Value)>, HashMap<String, String>> {
    let obj = match json.as_object() {
        Some(o) => o,
        None => {
            let mut errs = HashMap::new();
            errs.insert(
                "non_field_errors".to_string(),
                "Expected JSON object".to_string(),
            );
            return Err(errs);
        }
    };

    let meta = M::meta();
    let mut errors = HashMap::new();
    let mut values = Vec::new();

    for field in meta.fields {
        if field.auto {
            continue;
        }

        let val_opt = obj.get(field.name);

        if partial && val_opt.is_none() {
            continue;
        }

        if field.kind == FieldKind::Boolean {
            match val_opt {
                None | Some(serde_json::Value::Null) => {
                    if field.nullable {
                        values.push((field.name, Value::Null));
                    } else {
                        values.push((field.name, Value::Bool(false)));
                    }
                }
                Some(serde_json::Value::Bool(b)) => {
                    values.push((field.name, Value::Bool(*b)));
                }
                Some(serde_json::Value::String(s)) => match s.as_str() {
                    "true" | "1" => values.push((field.name, Value::Bool(true))),
                    "false" | "0" => values.push((field.name, Value::Bool(false))),
                    _ => {
                        errors.insert(
                            field.name.to_string(),
                            format!("Field '{}' must be a valid boolean.", field.name),
                        );
                    }
                },
                _ => {
                    errors.insert(
                        field.name.to_string(),
                        format!("Field '{}' must be a valid boolean.", field.name),
                    );
                }
            }
            continue;
        }

        match val_opt {
            None | Some(serde_json::Value::Null) => {
                if field.nullable {
                    values.push((field.name, Value::Null));
                } else {
                    errors.insert(
                        field.name.to_string(),
                        format!("Field '{}' is required.", field.name),
                    );
                }
            }
            Some(v) => match field.kind {
                FieldKind::Char
                | FieldKind::Text
                | FieldKind::Email
                | FieldKind::Url
                | FieldKind::Slug
                | FieldKind::Ip
                | FieldKind::Binary
                | FieldKind::Json => {
                    if let Some(s) = v.as_str() {
                        values.push((field.name, Value::Text(s.to_string())));
                    } else {
                        errors.insert(
                            field.name.to_string(),
                            format!("Field '{}' must be a string.", field.name),
                        );
                    }
                }
                FieldKind::FileField => {
                    if let Some(s) = v.as_str() {
                        values.push((field.name, Value::Text(s.to_string())));
                    } else {
                        errors.insert(
                            field.name.to_string(),
                            format!("Field '{}' must be a string.", field.name),
                        );
                    }
                }
                FieldKind::Integer | FieldKind::BigInt => {
                    if let Some(n) = v.as_i64() {
                        values.push((field.name, Value::I64(n)));
                    } else if let Some(s) = v.as_str() {
                        match s.parse::<i64>() {
                            Ok(n) => values.push((field.name, Value::I64(n))),
                            Err(_) => {
                                errors.insert(
                                    field.name.to_string(),
                                    format!("Field '{}' must be a valid integer.", field.name),
                                );
                            }
                        }
                    } else {
                        errors.insert(
                            field.name.to_string(),
                            format!("Field '{}' must be a valid integer.", field.name),
                        );
                    }
                }
                FieldKind::Float => {
                    if let Some(n) = v.as_f64() {
                        values.push((field.name, Value::F64(n)));
                    } else if let Some(s) = v.as_str() {
                        match s.parse::<f64>() {
                            Ok(n) => values.push((field.name, Value::F64(n))),
                            Err(_) => {
                                errors.insert(
                                    field.name.to_string(),
                                    format!("Field '{}' must be a valid float.", field.name),
                                );
                            }
                        }
                    } else {
                        errors.insert(
                            field.name.to_string(),
                            format!("Field '{}' must be a valid float.", field.name),
                        );
                    }
                }
                FieldKind::DateTime => {
                    if let Some(s) = v.as_str() {
                        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
                            values.push((
                                field.name,
                                Value::DateTime(dt.with_timezone(&chrono::Utc)),
                            ));
                        } else if let Ok(naive) =
                            chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
                        {
                            let dt = chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(
                                naive,
                                chrono::Utc,
                            );
                            values.push((field.name, Value::DateTime(dt)));
                        } else {
                            errors.insert(
                                field.name.to_string(),
                                format!(
                                    "Field '{}' must be in YYYY-MM-DD HH:MM:SS format.",
                                    field.name
                                ),
                            );
                        }
                    } else {
                        errors.insert(
                            field.name.to_string(),
                            format!(
                                "Field '{}' must be in YYYY-MM-DD HH:MM:SS format.",
                                field.name
                            ),
                        );
                    }
                }
                FieldKind::Decimal { .. }
                | FieldKind::Date
                | FieldKind::Time
                | FieldKind::Duration
                | FieldKind::Uuid => {
                    errors.insert(
                        field.name.to_string(),
                        format!(
                            "Unsupported FieldKind {:?} for field '{}'.",
                            field.kind, field.name
                        ),
                    );
                }
                FieldKind::Boolean => unreachable!(),
            },
        }
    }

    for rel in meta.relations {
        let val_opt = obj.get(rel.field_name);
        if partial && val_opt.is_none() {
            continue;
        }
        match val_opt {
            None | Some(serde_json::Value::Null) => {
                values.push((rel.field_name, Value::Null));
            }
            Some(v) => {
                if let Some(n) = v.as_i64() {
                    values.push((rel.field_name, Value::I64(n)));
                } else if let Some(s) = v.as_str() {
                    match s.parse::<i64>() {
                        Ok(n) => values.push((rel.field_name, Value::I64(n))),
                        Err(_) => {
                            errors.insert(
                                rel.field_name.to_string(),
                                format!("Field '{}' must be a valid integer ID.", rel.field_name),
                            );
                        }
                    }
                } else {
                    errors.insert(
                        rel.field_name.to_string(),
                        format!("Field '{}' must be a valid integer ID.", rel.field_name),
                    );
                }
            }
        }
    }

    if !errors.is_empty() {
        Err(errors)
    } else {
        Ok(values)
    }
}

/// Controls which fields cross the wire, in each direction.
///
/// This is the read/write split the review found missing: previously
/// [`serialize`] emitted every column and [`deserialize`] accepted every
/// column, so a password hash or an internal flag could only be hidden by
/// post-processing the JSON.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FieldSet {
    /// When non-empty, only these fields are emitted or accepted. Empty means
    /// "every model field", subject to the exclusions below.
    pub only: Vec<String>,
    /// Fields never emitted and never accepted, in either direction.
    pub exclude: Vec<String>,
    /// Fields emitted on read but silently dropped from writes — server-owned
    /// values such as `id` or `created_at`.
    pub read_only: Vec<String>,
    /// Fields accepted on write but never emitted — secrets such as `password`.
    pub write_only: Vec<String>,
}

impl FieldSet {
    /// An unrestricted field set: every model field, both directions.
    pub fn all() -> Self {
        Self::default()
    }

    /// Restrict to exactly these fields.
    pub fn only(fields: &[&str]) -> Self {
        Self {
            only: fields.iter().map(|f| f.to_string()).collect(),
            ..Self::default()
        }
    }

    /// Exclude these fields entirely.
    pub fn excluding(mut self, fields: &[&str]) -> Self {
        self.exclude = fields.iter().map(|f| f.to_string()).collect();
        self
    }

    /// Mark these fields read-only: emitted, never accepted.
    pub fn read_only(mut self, fields: &[&str]) -> Self {
        self.read_only = fields.iter().map(|f| f.to_string()).collect();
        self
    }

    /// Mark these fields write-only: accepted, never emitted.
    pub fn write_only(mut self, fields: &[&str]) -> Self {
        self.write_only = fields.iter().map(|f| f.to_string()).collect();
        self
    }

    fn in_scope(&self, field: &str) -> bool {
        if self.exclude.iter().any(|f| f == field) {
            return false;
        }
        self.only.is_empty() || self.only.iter().any(|f| f == field)
    }

    /// Whether the field appears in serialized output.
    pub fn is_readable(&self, field: &str) -> bool {
        self.in_scope(field) && !self.write_only.iter().any(|f| f == field)
    }

    /// Returns `true` when every model field is readable on output (no `only`,
    /// `exclude`, or `write_only` restrictions), allowing [`ModelSerializer`] to
    /// take the unbranched compile-time [`Model::write_json`] fast path.
    pub fn is_all_readable(&self) -> bool {
        self.only.is_empty() && self.exclude.is_empty() && self.write_only.is_empty()
    }

    /// Whether the field is accepted from client input.
    pub fn is_writable(&self, field: &str) -> bool {
        self.in_scope(field) && !self.read_only.iter().any(|f| f == field)
    }
}

/// The column values a serializer produces from a request body: pairs of model
/// field name and coerced value, ready for `insert_raw` / `update`.
pub type FieldValues = Vec<(&'static str, Value)>;

/// Converts between a model and its wire representation, with validation.
///
/// This is the seam DRF has and Djangors lacked. A ViewSet no longer hardcodes
/// `serialize`/`deserialize`; it asks a serializer, so a project can control
/// field exposure, coerce values, and enforce business rules in one place.
pub trait Serializer<M: Model>: Send + Sync + 'static {
    /// Render an instance for the response body.
    fn to_representation(&self, instance: &M) -> serde_json::Value;

    /// Parse and validate a request body into column values ready for
    /// `insert_raw` / `update`.
    ///
    /// Implementations should accumulate every problem into
    /// [`ValidationErrors`] rather than returning on the first one.
    fn to_internal_value(
        &self,
        data: &serde_json::Value,
        partial: bool,
    ) -> Result<Vec<(&'static str, Value)>, ValidationErrors>;

    /// Object-level rules, run after fields parse successfully.
    ///
    /// The default accepts everything. Override to express cross-field
    /// constraints; record problems on `errors` instead of returning early so
    /// the client sees them all at once.
    fn validate(&self, _values: &[(&'static str, Value)], _errors: &mut ValidationErrors) {}

    /// Render an instance with pre-fetched related objects embedded in place of
    /// their raw foreign-key ids.
    ///
    /// This trait is synchronous, so it cannot itself fetch relations; pair it
    /// with [`QuerySet::select_related`](djangors_orm::queryset::QuerySet::select_related)
    /// to load the related rows, then pass them here. Only keys already present
    /// in the base representation are replaced, so an unknown relation name is
    /// ignored rather than injecting a field the schema never declared.
    fn to_representation_nested(
        &self,
        instance: &M,
        related: &RelatedObjects,
    ) -> serde_json::Value {
        let mut base = self.to_representation(instance);
        if let Some(obj) = base.as_object_mut() {
            for (field, value) in related {
                if obj.contains_key(*field) {
                    obj.insert((*field).to_string(), value.clone());
                }
            }
        }
        base
    }

    /// Render a collection.
    fn to_representation_many(&self, instances: &[M]) -> Vec<serde_json::Value> {
        instances
            .iter()
            .map(|item| self.to_representation(item))
            .collect()
    }

    /// Appends the serialized JSON representation of `instance` directly into `buf`.
    ///
    /// [`ModelSerializer`] overrides this to call [`Model::write_json`] /
    /// [`Model::write_json_filtered`], bypassing intermediate `serde_json::Value`
    /// allocations entirely. Custom serializers inherit a default implementation
    /// that delegates to [`Serializer::to_representation`].
    fn write_json(&self, instance: &M, buf: &mut Vec<u8>) {
        let val = self.to_representation(instance);
        let _ = serde_json::to_writer(&mut *buf, &val);
    }

    /// Appends a JSON array of `instances` directly into `buf`.
    ///
    /// Custom serializers that override [`Serializer::to_representation_many`]
    /// are honored by the default implementation; [`ModelSerializer`] overrides
    /// this to stream each item via [`Serializer::write_json`] without
    /// allocating a `Vec<serde_json::Value>`.
    fn write_json_many(&self, instances: &[M], buf: &mut Vec<u8>) {
        let vals = self.to_representation_many(instances);
        let _ = serde_json::to_writer(&mut *buf, &vals);
    }

    /// Renders `instance` into a freshly allocated UTF-8 JSON byte vector.
    fn to_json_bytes(&self, instance: &M) -> Vec<u8> {
        let mut buf = Vec::with_capacity(128);
        self.write_json(instance, &mut buf);
        buf
    }

    /// Parse, then apply [`Serializer::validate`]. ViewSets call this rather
    /// than `to_internal_value` directly, so object-level rules are never
    /// accidentally skipped.
    fn parse(
        &self,
        data: &serde_json::Value,
        partial: bool,
    ) -> Result<Vec<(&'static str, Value)>, ValidationErrors> {
        let values = self.to_internal_value(data, partial)?;
        let mut errors = ValidationErrors::new();
        self.validate(&values, &mut errors);
        if errors.is_empty() {
            Ok(values)
        } else {
            Err(errors)
        }
    }
}

/// The default [`Serializer`]: derives its behaviour from [`Model`] metadata,
/// narrowed by a [`FieldSet`].
///
/// Equivalent to DRF's `ModelSerializer`. Constructing one with
/// [`FieldSet::all`] reproduces the historical [`serialize`]/[`deserialize`]
/// behaviour exactly.
pub struct ModelSerializer<M: Model> {
    fields: FieldSet,
    validators: Vec<Box<dyn Validator<FieldValues>>>,
    _marker: std::marker::PhantomData<M>,
}

impl<M: Model> Default for ModelSerializer<M> {
    fn default() -> Self {
        Self::new(FieldSet::all())
    }
}

impl<M: Model> ModelSerializer<M> {
    /// Build a serializer restricted to `fields`.
    pub fn new(fields: FieldSet) -> Self {
        Self {
            fields,
            validators: Vec::new(),
            _marker: std::marker::PhantomData,
        }
    }

    /// Attach an object-level rule. Rules run in registration order, and every
    /// rule runs even if an earlier one failed, so the client sees the full set.
    pub fn with_validator<V>(mut self, validator: V) -> Self
    where
        V: Validator<FieldValues>,
    {
        self.validators.push(Box::new(validator));
        self
    }

    /// The field set in force.
    pub fn field_set(&self) -> &FieldSet {
        &self.fields
    }
}

impl<M: Model + Send + Sync + 'static> Serializer<M> for ModelSerializer<M> {
    fn to_representation(&self, instance: &M) -> serde_json::Value {
        let full = serialize(instance);
        let serde_json::Value::Object(map) = full else {
            return serde_json::Value::Object(serde_json::Map::new());
        };
        serde_json::Value::Object(
            map.into_iter()
                .filter(|(name, _)| self.fields.is_readable(name))
                .collect(),
        )
    }

    fn write_json(&self, instance: &M, buf: &mut Vec<u8>) {
        if self.fields.is_all_readable() {
            instance.write_json(buf);
        } else {
            instance.write_json_filtered(buf, &mut |name| self.fields.is_readable(name));
        }
    }

    fn write_json_many(&self, instances: &[M], buf: &mut Vec<u8>) {
        buf.push(b'[');
        for (idx, item) in instances.iter().enumerate() {
            if idx > 0 {
                buf.push(b',');
            }
            self.write_json(item, buf);
        }
        buf.push(b']');
    }

    fn to_internal_value(
        &self,
        data: &serde_json::Value,
        partial: bool,
    ) -> Result<Vec<(&'static str, Value)>, ValidationErrors> {
        // Reject writes to read-only/excluded fields loudly rather than
        // silently dropping them: a client that thinks it set `id` should be
        // told it did not.
        let mut errors = ValidationErrors::new();
        if let serde_json::Value::Object(map) = data {
            for name in map.keys() {
                let known = M::meta().fields.iter().any(|f| f.name == name)
                    || M::meta().relations.iter().any(|r| r.field_name == name);
                if known && !self.fields.is_writable(name) {
                    errors.add(name.clone(), "field is read-only");
                }
            }
        }

        // A partial (PATCH) body legitimately omits fields; a full (PUT/POST)
        // body must supply every writable, non-auto column.
        //
        // This reads the raw body rather than `deserialize`'s output on
        // purpose. `deserialize` defaults a missing non-nullable boolean to
        // `false` — correct for an HTML checkbox, wrong for a JSON API, where
        // `POST {}` would silently set the flag instead of reporting it
        // missing. Checking the body directly closes that hole, and runs
        // independently of whether `deserialize` also failed, so the client
        // sees every problem at once.
        if !partial {
            let supplied = |name: &str| data.get(name).is_some_and(|value| !value.is_null());
            for field in M::meta().fields {
                if field.auto
                    || field.primary_key
                    || field.nullable
                    || !self.fields.is_writable(field.name)
                {
                    continue;
                }
                if !supplied(field.name) {
                    errors.add(field.name, "this field is required");
                }
            }
            for relation in M::meta().relations {
                if !self.fields.is_writable(relation.field_name) {
                    continue;
                }
                if !supplied(relation.field_name) {
                    errors.add(relation.field_name, "this field is required");
                }
            }
        }

        let parsed = if partial {
            deserialize_partial::<M>(data)
        } else {
            deserialize::<M>(data)
        };

        let mut values = match parsed {
            Ok(values) => values,
            Err(parse_errors) => {
                // Merge rather than replace: a body can be both missing one
                // field and malformed in another, and the client should see
                // both at once.
                errors.merge(ValidationErrors::from(parse_errors));
                return Err(errors);
            }
        };

        errors.into_result()?;

        values.retain(|(name, _)| self.fields.is_writable(name));
        Ok(values)
    }

    fn validate(&self, values: &[(&'static str, Value)], errors: &mut ValidationErrors) {
        let owned = values.to_vec();
        for validator in &self.validators {
            validator.validate(&owned, errors);
        }
    }
}

/// Related objects to embed, keyed by the relation's field name on the parent.
///
/// The key must match the field name as it appears in the parent's
/// representation (i.e. the foreign-key field), because
/// [`Serializer::to_representation_nested`] replaces that key in place.
pub type RelatedObjects = HashMap<&'static str, serde_json::Value>;

/// Composes two serializers so a relation renders as a nested object instead of
/// a bare id — DRF's nested serializer.
///
/// The relation's rows must already be loaded (via
/// [`QuerySet::select_related`](djangors_orm::queryset::QuerySet::select_related));
/// this type decides only how they are *rendered*, never how they are fetched.
/// When no related instance is supplied, the field keeps its raw id, so a
/// missing join degrades to the flat representation rather than to `null`.
///
/// ```ignore
/// let serializer = NestedSerializer::new(
///     ModelSerializer::<Article>::default(),
///     "author_id",
///     ModelSerializer::<User>::new(FieldSet::all().excluding(&["password"])),
/// );
/// let body = serializer.render(&article, Some(&author));
/// ```
pub struct NestedSerializer<M: Model + 'static, R: Model + 'static> {
    base: Arc<dyn Serializer<M>>,
    relation: &'static str,
    inner: Arc<dyn Serializer<R>>,
}

impl<M: Model + 'static, R: Model + 'static> NestedSerializer<M, R> {
    /// Embeds `relation` on `base` using `inner` to render the related object.
    pub fn new<B: Serializer<M>, I: Serializer<R>>(
        base: B,
        relation: &'static str,
        inner: I,
    ) -> Self {
        Self {
            base: Arc::new(base),
            relation,
            inner: Arc::new(inner),
        }
    }

    /// Renders `instance`, embedding `related` at the configured relation field.
    pub fn render(&self, instance: &M, related: Option<&R>) -> serde_json::Value {
        let Some(related) = related else {
            return self.base.to_representation(instance);
        };
        let mut map: RelatedObjects = HashMap::new();
        map.insert(self.relation, self.inner.to_representation(related));
        self.base.to_representation_nested(instance, &map)
    }

    /// Renders a collection of `(instance, related)` pairs, which is exactly the
    /// shape `select_related` returns.
    pub fn render_many(&self, rows: &[(M, Option<R>)]) -> Vec<serde_json::Value> {
        rows.iter()
            .map(|(instance, related)| self.render(instance, related.as_ref()))
            .collect()
    }
}

impl<M: Model + Send + Sync + 'static, R: Model + Send + Sync + 'static> Serializer<M>
    for NestedSerializer<M, R>
{
    /// Without a related instance to embed there is nothing to nest, so this
    /// falls through to the base representation. Use
    /// [`NestedSerializer::render`] when the relation has been loaded.
    fn to_representation(&self, instance: &M) -> serde_json::Value {
        self.base.to_representation(instance)
    }

    fn to_internal_value(
        &self,
        data: &serde_json::Value,
        partial: bool,
    ) -> Result<Vec<(&'static str, Value)>, ValidationErrors> {
        // Writes stay flat: a nested payload would need to create or match the
        // related row, which is a different operation from serializing it.
        self.base.to_internal_value(data, partial)
    }

    fn validate(&self, values: &[(&'static str, Value)], errors: &mut ValidationErrors) {
        self.base.validate(values, errors)
    }
}
