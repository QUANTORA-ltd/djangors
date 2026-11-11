use std::fmt;
use std::marker::PhantomData;

/// Represents the database field type.
///
/// Analogous to the various field classes in Django (e.g., `CharField`, `IntegerField`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum FieldKind {
    /// Text field with a maximum length. Django's `CharField`.
    Char,
    /// Unbounded text field. Django's `TextField`.
    Text,
    /// A path/key stored in a configured file storage backend. Django's `FileField`.
    FileField,
    /// Standard 32-bit signed integer. Django's `IntegerField`.
    Integer,
    /// 64-bit signed integer. Django's `BigIntegerField`.
    BigInt,
    /// Floating point number. Django's `FloatField`.
    Float,
    /// Fixed-precision decimal number. Django's `DecimalField`.
    Decimal {
        /// The maximum number of digits allowed in the number.
        max_digits: u32,
        /// The number of decimal places to store with the number.
        decimal_places: u32,
    },
    /// Boolean field. Django's `BooleanField`.
    Boolean,
    /// Date field. Django's `DateField`.
    Date,
    /// Date and time field. Django's `DateTimeField`.
    DateTime,
    /// Time field. Django's `TimeField`.
    Time,
    /// Duration / delta time field. Django's `DurationField`.
    Duration,
    /// Universally unique identifier. Django's `UUIDField`.
    Uuid,
    /// Email address field. Django's `EmailField`.
    Email,
    /// URL field. Django's `URLField`.
    Url,
    /// Slug field (for URLs). Django's `SlugField`.
    Slug,
    /// IP address field. Django's `GenericIPAddressField`.
    Ip,
    /// Binary data field. Django's `BinaryField`.
    Binary,
    /// JSON-encoded data field. Django's `JSONField`.
    Json,
}

/// Represents the default value of a model field.
///
/// Analogous to the `default` argument in Django model fields.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum DefaultValue {
    /// Integer default value.
    I64(i64),
    /// Floating point default value.
    F64(f64),
    /// Text default value.
    Text(&'static str),
    /// Boolean default value.
    Bool(bool),
    /// No default value.
    None,
}

/// Metadata for a single model field.
///
/// Contains information about the field's name, type, database column, nullability,
/// constraints, and defaults. Analogous to attributes on a Django Field instance.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FieldMeta {
    /// The name of the field on the Rust struct.
    pub name: &'static str,
    /// The database column name for this field.
    pub column_name: &'static str,
    /// The database field type.
    pub kind: FieldKind,
    /// Whether the field is allowed to be NULL. Django's `null` parameter.
    pub nullable: bool,
    /// Whether this field is the primary key. Django's `primary_key` parameter.
    pub primary_key: bool,
    /// Whether the field is auto-incremented or auto-generated. Django's `AutoField`.
    pub auto: bool,
    /// Whether the field values must be unique across the table. Django's `unique` parameter.
    pub unique: bool,
    /// Whether a database index should be created for this field. Django's `db_index` parameter.
    pub db_index: bool,
    /// The default value for the field. Django's `default` parameter.
    pub default: DefaultValue,
    /// The maximum length in characters (useful for Char fields). Django's `max_length` parameter.
    pub max_length: Option<u32>,
    /// A human-readable name for the field. Django's `verbose_name` parameter.
    pub verbose_name: Option<&'static str>,
    /// Extra helper text for forms. Django's `help_text` parameter.
    pub help_text: Option<&'static str>,
    /// Choices for field values, as (db_value, human_readable_label) pairs. Django's `choices` parameter.
    pub choices: &'static [(&'static str, &'static str)],
}

/// The kind of relation between models.
///
/// Matches Django's relational field types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RelationKind {
    /// Many-to-one relation. Django's `ForeignKey`.
    ForeignKey,
    /// One-to-one relation. Django's `OneToOneField`.
    OneToOne,
    /// Many-to-many relation. Django's `ManyToManyField`.
    ManyToMany,
}

/// Action to take when a referenced object is deleted.
///
/// Analogous to Django's `on_delete` behaviors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum OnDelete {
    /// Cascade deletes. Django's `CASCADE`.
    Cascade,
    /// Prevent deletion of referenced object. Django's `PROTECT`.
    Protect,
    /// Set the reference to NULL. Django's `SET_NULL`.
    SetNull,
    /// Prevent deletion of referenced object (SQL standard). Django's `RESTRICT`.
    Restrict,
    /// Take no action. Django's `DO_NOTHING`.
    DoNothing,
}

/// Metadata for a relation between two models.
///
/// Analogous to Django's relational fields.
#[derive(Debug, Clone, Copy)]
pub struct RelationMeta {
    /// The name of the relation field on the Rust struct.
    pub field_name: &'static str,
    /// The relationship kind (e.g. ForeignKey).
    pub kind: RelationKind,
    /// Late-bound target model metadata function. Resolves ordering circularity.
    pub target: fn() -> &'static ModelMeta,
    /// The referential integrity delete rule. Django's `on_delete`.
    pub on_delete: OnDelete,
    /// The name to use for the relation from the related object back to this one. Django's `related_name`.
    pub related_name: Option<&'static str>,
}

/// Metadata for an explicit database index.
///
/// Analogous to entries in Django's `Meta.indexes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexMeta {
    /// The name of the index in the database.
    pub name: &'static str,
    /// The fields included in this index.
    pub fields: &'static [&'static str],
}

/// Runtime metadata for a model.
///
/// This serves as the single source of truth for downstream crates (migrations, admin, forms, etc.).
/// Analogous to Django's `Model._meta` options class.
#[derive(Debug, Clone, Copy)]
pub struct ModelMeta {
    /// The name of the Rust struct representing this model.
    pub struct_name: &'static str,
    /// The app label defining the logical namespace. Django's `app_label`.
    pub app_label: &'static str,
    /// The database table name. Django's `db_table`.
    pub table_name: &'static str,
    /// Fields defined on this model.
    pub fields: &'static [FieldMeta],
    /// Relations defined on this model.
    pub relations: &'static [RelationMeta],
    /// Indexes defined on this model.
    pub indexes: &'static [IndexMeta],
    /// Unique constraints across multiple fields. Django's `unique_together`.
    pub unique_together: &'static [&'static [&'static str]],
    /// The default ordering for querysets. Django's `ordering`.
    pub ordering: &'static [&'static str],
}

/// Trait implemented by all models to expose their metadata.
pub trait Model {
    /// Returns the static `ModelMeta` reference for this model.
    fn meta() -> &'static ModelMeta
    where
        Self: Sized;

    /// Every field's (field_name, current value) in declaration order,
    /// including primary-key/auto fields (unlike save binds, which skip
    /// auto fields) and FK fields (as the related id). Generated by
    /// #[derive(Model)]; the admin changelist renders rows through this.
    fn field_values(&self) -> Vec<(&'static str, crate::expr::Value)>;

    /// Every field's name in declaration order (same order and names as
    /// [`Model::field_values`]). Needed separately from `field_values`
    /// because column headers must render even when a query returns zero
    /// rows, and `ModelMeta` cannot supply this: relations live in a
    /// separate list from plain fields, so declaration order across both
    /// is only known to the derive. Required (no default) so a
    /// hand-written impl can't silently render zero columns.
    fn field_names() -> Vec<&'static str>
    where
        Self: Sized;

    /// Appends this model's complete JSON object representation directly into
    /// `buf` without allocating an intermediate `Vec` or `serde_json::Map`.
    ///
    /// Generated at compile time by `#[derive(Model)]` with pre-baked static
    /// JSON key fragments (`concat!("\"", field, "\":")`), matching Fastify's
    /// `fast-json-stringify` strategy.
    fn write_json(&self, buf: &mut Vec<u8>) {
        self.write_json_filtered(buf, &mut |_| true);
    }

    /// Appends this model's JSON object representation directly into `buf`,
    /// emitting only fields for which `include(field_name)` returns `true`.
    fn write_json_filtered(
        &self,
        buf: &mut Vec<u8>,
        include: &mut dyn FnMut(&'static str) -> bool,
    ) {
        crate::json::write_field_values_filtered(buf, &self.field_values(), include);
    }

    /// Helper to get a QuerySet for this model.
    fn objects() -> crate::queryset::QuerySet<Self>
    where
        Self: Sized + crate::error::FromRow,
    {
        crate::queryset::QuerySet::new()
    }
}

/// A custom manager for a model — Django's custom `Manager`.
///
/// A manager encapsulates reusable query logic that should be available via
/// `Model::objects()`. Implementations typically build on [`QuerySet`] and are
/// referenced from a model with `#[djangors(manager = "MyManager")]`.
pub trait Manager<T: Model> {
    /// Returns the queryset returned by `objects()` for `T`.
    fn objects() -> crate::queryset::QuerySet<T>
    where
        T: crate::error::FromRow;
}

/// Form operations generated for a model by `#[derive(Model)]`.
pub trait ModelForm: Model {
    /// The cleaned representation produced by form validation.
    type FormCleaned;
    /// Validate raw form fields and return cleaned values.
    fn validate_form(
        data: &std::collections::HashMap<String, String>,
    ) -> Result<Self::FormCleaned, djangors_forms::FormErrors>;
    /// Construct a model from cleaned form values.
    fn from_cleaned_form(cleaned: Self::FormCleaned) -> Self;
    /// Apply cleaned form values to an existing model.
    fn apply_cleaned_form(&mut self, cleaned: Self::FormCleaned);
}

/// Entry representing a registered model.
///
/// Used by the `inventory` crate to collect all models at startup.
pub struct ModelRegistration {
    /// Function returning the model's metadata.
    pub meta_fn: fn() -> &'static ModelMeta,
}

inventory::collect!(ModelRegistration);

/// Returns an iterator over all registered model metadata.
///
/// Can be used by downstream tools like migrations to discover all models.
pub fn all_registered_models() -> impl Iterator<Item = &'static ModelMeta> {
    inventory::iter::<ModelRegistration>().map(|r| (r.meta_fn)())
}

/// Serializable model metadata used by the external `dj` process.
#[allow(missing_docs)]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ModelSnapshot {
    pub struct_name: String,
    pub app_label: String,
    pub table_name: String,
    pub fields: Vec<FieldSnapshot>,
    pub relations: Vec<RelationSnapshot>,
    pub unique_together: Vec<Vec<String>>,
    pub ordering: Vec<String>,
    pub indexes: Vec<IndexSnapshot>,
}

#[allow(missing_docs)]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct IndexSnapshot {
    pub name: String,
    pub fields: Vec<String>,
}

#[allow(missing_docs)]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct FieldSnapshot {
    pub name: String,
    pub column_name: String,
    pub kind: FieldKind,
    pub nullable: bool,
    pub primary_key: bool,
    pub auto: bool,
    pub unique: bool,
    pub db_index: bool,
    pub default: SnapshotDefault,
    pub max_length: Option<u32>,
}

#[allow(missing_docs)]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(tag = "kind", content = "value")]
pub enum SnapshotDefault {
    I64(i64),
    F64(f64),
    Text(String),
    Bool(bool),
    None,
}

#[allow(missing_docs)]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct RelationSnapshot {
    pub field_name: String,
    pub kind: RelationKind,
    pub target_struct: String,
    pub on_delete: OnDelete,
    pub related_name: Option<String>,
}

/// Returns the complete registry in a process-boundary-safe representation.
pub fn registered_models_snapshot() -> Vec<ModelSnapshot> {
    all_registered_models()
        .map(|m| ModelSnapshot {
            struct_name: m.struct_name.into(),
            app_label: m.app_label.into(),
            table_name: m.table_name.into(),
            fields: m
                .fields
                .iter()
                .map(|f| FieldSnapshot {
                    name: f.name.into(),
                    column_name: f.column_name.into(),
                    kind: f.kind,
                    nullable: f.nullable,
                    primary_key: f.primary_key,
                    auto: f.auto,
                    unique: f.unique,
                    db_index: f.db_index,
                    default: match f.default {
                        DefaultValue::I64(v) => SnapshotDefault::I64(v),
                        DefaultValue::F64(v) => SnapshotDefault::F64(v),
                        DefaultValue::Text(v) => SnapshotDefault::Text(v.into()),
                        DefaultValue::Bool(v) => SnapshotDefault::Bool(v),
                        DefaultValue::None => SnapshotDefault::None,
                    },
                    max_length: f.max_length,
                })
                .collect(),
            relations: m
                .relations
                .iter()
                .map(|r| RelationSnapshot {
                    field_name: r.field_name.into(),
                    kind: r.kind,
                    target_struct: (r.target)().struct_name.into(),
                    on_delete: r.on_delete,
                    related_name: r.related_name.map(Into::into),
                })
                .collect(),
            unique_together: m
                .unique_together
                .iter()
                .map(|x| x.iter().map(|s| (*s).into()).collect())
                .collect(),
            ordering: m.ordering.iter().map(|s| (*s).into()).collect(),
            indexes: m
                .indexes
                .iter()
                .map(|i| IndexSnapshot {
                    name: i.name.into(),
                    fields: i.fields.iter().map(|f| (*f).into()).collect(),
                })
                .collect(),
        })
        .collect()
}

/// A wrapper representing a foreign key relationship to another model.
///
/// Analogous to Django's `ForeignKey` field.
pub struct ForeignKey<T: Model> {
    /// The ID of the related model instance.
    pub id: i64,
    /// Marker to associate this key with the target model type.
    _marker: PhantomData<T>,
}

impl<T: Model> ForeignKey<T> {
    /// Creates a new `ForeignKey` reference.
    pub fn new(id: i64) -> Self {
        Self {
            id,
            _marker: PhantomData,
        }
    }

    /// Lazily loads the related model instance by its primary key — Django's
    /// `fk.field` accessor. Returns `None` if no row matches.
    pub async fn fetch<E: djangors_db::DbExecutor>(
        &self,
        db: E,
    ) -> Result<Option<T>, crate::error::OrmError>
    where
        T: crate::error::FromRow,
    {
        let qs = crate::queryset::QuerySet::<T>::new()
            .filter(crate::q!(id = self.id))
            .map_err(|e| crate::error::OrmError::InvalidQuery(e.to_string()))?;
        qs.first(db).await
    }
}

impl<T: Model> Clone for ForeignKey<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: Model> Copy for ForeignKey<T> {}

impl<T: Model> fmt::Debug for ForeignKey<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ForeignKey").field("id", &self.id).finish()
    }
}

/// A wrapper representing a one-to-one relationship to another model.
///
/// Analogous to Django's `OneToOneField`. Like [`ForeignKey`], it is stored as a
/// column holding the related object's primary key, but the column is constrained
/// to be unique so each related row can be referenced by at most one row on this
/// model.
pub struct OneToOne<T: Model> {
    /// The ID of the related model instance.
    pub id: i64,
    /// Marker to associate this key with the target model type.
    _marker: PhantomData<T>,
}

impl<T: Model> OneToOne<T> {
    /// Creates a new `OneToOne` reference.
    pub fn new(id: i64) -> Self {
        Self {
            id,
            _marker: PhantomData,
        }
    }

    /// Lazily loads the related model instance by its primary key — Django's
    /// `onetoone.accessor`. Returns `None` if no row matches.
    pub async fn fetch<E: djangors_db::DbExecutor>(
        &self,
        db: E,
    ) -> Result<Option<T>, crate::error::OrmError>
    where
        T: crate::error::FromRow,
    {
        let qs = crate::queryset::QuerySet::<T>::new()
            .filter(crate::q!(id = self.id))
            .map_err(|e| crate::error::OrmError::InvalidQuery(e.to_string()))?;
        qs.first(db).await
    }
}

impl<T: Model> Clone for OneToOne<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: Model> Copy for OneToOne<T> {}

impl<T: Model> fmt::Debug for OneToOne<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OneToOne").field("id", &self.id).finish()
    }
}

/// A marker representing a many-to-many relationship to another model.
///
/// Analogous to Django's `ManyToManyField`. A M2M field is **not a column** on
/// the hosting model; the relationship lives in a dedicated join (through) table
/// generated by migrations. The value on the struct is a zero-size marker used
/// solely to declare and name the relation.
pub struct ManyToMany<T: Model> {
    _marker: PhantomData<T>,
}

impl<T: Model> ManyToMany<T> {
    /// Creates the zero-size M2M marker.
    pub fn new() -> Self {
        Self {
            _marker: PhantomData,
        }
    }
}

impl<T: Model> Default for ManyToMany<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Model> Clone for ManyToMany<T> {
    fn clone(&self) -> Self {
        // `ManyToMany<T>` is a zero-sized marker (`PhantomData`), so this must agree with the
        // `Copy` impl below — clippy::non_canonical_clone_impl rejects anything else.
        *self
    }
}

impl<T: Model> Copy for ManyToMany<T> {}

impl<T: Model> fmt::Debug for ManyToMany<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ManyToMany").finish()
    }
}

/// Computes the deterministic join-table name for a many-to-many relation.
///
/// Convention mirrors Django's default: `{source_table}_{field_name}`.
pub fn m2m_through_table(source: &ModelMeta, relation: &RelationMeta) -> String {
    format!("{}_{}", source.table_name, relation.field_name)
}

/// A per-model database map — Django's `DATABASES` and `DATABASE_ROUTERS` combined.
///
/// Register model-specific databases and retrieve the right one with [`db_for`].
pub struct ModelDatabaseMap {
    default: djangors_db::Database,
    per_model: std::collections::HashMap<&'static str, djangors_db::Database>,
}

impl ModelDatabaseMap {
    /// Creates a new map from a default database and an optional list of
    /// `(model_struct_name, database)` pairs.
    pub fn new(
        default: djangors_db::Database,
        routes: Vec<(&'static str, djangors_db::Database)>,
    ) -> Self {
        let mut map = std::collections::HashMap::new();
        for (name, db) in routes {
            map.insert(name, db);
        }
        Self {
            default,
            per_model: map,
        }
    }

    /// Returns the database configured for `model`, or the default if none was
    /// registered. Use via `ModelDatabaseMap::db_for::<MyModel>(&map)`.
    pub fn db_for<T: Model>(&self) -> &djangors_db::Database {
        let name = T::meta().struct_name;
        self.per_model.get(name).unwrap_or(&self.default)
    }
}
