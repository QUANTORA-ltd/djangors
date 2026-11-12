use djangors_core::DjangorsError;
use djangors_macros::Model as DeriveModel;
use djangors_orm::meta::ModelMeta;

/// Definition of a custom bulk admin action.
#[derive(Clone)]
pub struct AdminAction {
    /// Internal action key name.
    pub name: &'static str,
    /// Human-readable label displayed in the action dropdown.
    pub label: &'static str,
    /// Whether executing this action requires a confirmation page step.
    pub requires_confirm: bool,
    /// Async handler executing the action against selected primary keys.
    #[allow(clippy::type_complexity)]
    pub handler: for<'a> fn(
        &'a djangors_db::Database,
        &'a [i64],
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<(), DjangorsError>> + Send + 'a>,
    >,
}

/// Serialized row for rendering action options in templates.
#[derive(serde::Serialize)]
pub struct AdminActionRow {
    /// Internal action key name.
    pub name: String,
    /// Human-readable action label.
    pub label: String,
}

/// Log entry action flag for object addition.
pub const ACTION_ADDITION: i32 = 1;
/// Log entry action flag for object modification.
pub const ACTION_CHANGE: i32 = 2;
/// Log entry action flag for object deletion.
pub const ACTION_DELETION: i32 = 3;

/// Struct capturing a field change diff item.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FieldDiffItem {
    /// Field name that changed.
    pub field: String,
    /// Value before modification.
    pub old: String,
    /// Value after modification.
    pub new: String,
}

/// Audit log entry model recording staff admin actions.
#[derive(DeriveModel, Debug, Clone, serde::Serialize)]
#[djangors(app = "admin", table_name = "djangors_admin_log")]
pub struct LogEntry {
    /// Primary key log entry ID.
    #[djangors(primary_key, auto)]
    pub id: i64,
    /// ID of the staff user who performed the action.
    pub user_id: i64,
    /// Timestamp when the action was logged.
    pub action_time: chrono::DateTime<chrono::Utc>,
    /// Application label of the target model.
    pub app_label: String,
    /// Struct name of the target model.
    pub model_name: String,
    /// Primary key ID of the target object.
    pub object_id: i64,
    /// String representation of the target object.
    pub object_repr: String,
    /// Action flag (`ACTION_ADDITION`, `ACTION_CHANGE`, `ACTION_DELETION`).
    pub action_flag: i32,
    /// Human-readable summary of changes.
    pub change_message: String,
    /// Optional JSON string detailing field-level diffs.
    pub field_diff: Option<String>,
}

pub(crate) async fn log_action(
    db: &djangors_db::Database,
    user_id: i64,
    meta: &'static ModelMeta,
    object_id: i64,
    action_flag: i32,
    change_message: &str,
    field_diff: Option<String>,
) -> Result<(), DjangorsError> {
    let res = LogEntry {
        id: 0,
        user_id,
        action_time: chrono::Utc::now(),
        app_label: meta.app_label.to_string(),
        model_name: meta.struct_name.to_string(),
        object_id,
        object_repr: format!("{} object ({})", meta.struct_name, object_id),
        action_flag,
        change_message: change_message.to_string(),
        field_diff,
    }
    .save(db)
    .await;

    match res {
        Ok(_) => Ok(()),
        Err(e) => {
            let err_str = e.to_string();
            if err_str.contains("djangors_admin_log")
                && (err_str.contains("does not exist") || err_str.contains("no such table"))
            {
                return Ok(());
            }
            Err(DjangorsError::Internal(err_str))
        }
    }
}
