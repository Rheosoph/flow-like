//! Owner-defined fork policy.
//!
//! The app **owner** decides what a fork of their app contains; the person
//! forking gets no choice. The policy is loaded from the source `App` row
//! inside each fork engine and passed as an explicit parameter to every
//! function that can drop something — it is deliberately **not** a
//! [`super::ForkOptions`] field, so a caller can neither supply it nor have
//! it silently ignored.
//!
//! This is hygiene, not confidentiality. Every caller that passes
//! `check_can_fork` holds `RolePermissions::ReadFiles`, which alone guards
//! the file download endpoints and satisfies the project-database routes
//! (including arbitrary SQL). A forker can already read anything the policy
//! excludes — excluding it keeps the *copy* clean, it does not protect the
//! data.

use crate::entity::app;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// How the project LanceDB (`apps/{id}/storage/db/**`) is handled.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ForkDatabaseMode {
    /// No project database at all — nothing under `storage/db/` is copied.
    None,
    /// User tables are recreated empty from the source schema. Reserved
    /// artifact tables (`__graph_overlays__`, `__saved_queries_manifest__`,
    /// …) are Data Studio *configuration* rather than user data and ride
    /// along whole, so ontologies and saved queries still resolve.
    SchemaOnly,
    /// Pre-#756 behavior: the whole project LanceDB is mirrored.
    #[default]
    WithData,
}

fn enabled() -> bool {
    true
}

/// Owner-defined, per-app. Loaded from `App.forkPolicy`; a NULL column or an
/// absent field defaults to permissive so every pre-#756 app forks
/// byte-identically.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ForkPolicy {
    /// Boards and the versioned board archives events pin to. Excluding
    /// flows cascades: pages and events are board-gated in both engines, so
    /// a fork without flows is an app shell with no runnable logic.
    #[serde(default = "enabled")]
    pub flows: bool,
    /// User files under `apps/{id}/upload/**`.
    #[serde(default = "enabled")]
    pub files: bool,
    #[serde(default)]
    pub databases: ForkDatabaseMode,
    /// Source role rows copied verbatim. When false the destination is given
    /// freshly minted Owner / Admin / User roles instead — a fork always
    /// needs an owner role and a default role to be usable.
    ///
    /// No-op on the offline path: an offline bundle ships no role rows at all.
    #[serde(default = "enabled")]
    pub roles: bool,
    #[serde(default = "enabled")]
    pub widgets: bool,
    #[serde(default = "enabled")]
    pub templates: bool,
}

impl Default for ForkPolicy {
    fn default() -> Self {
        Self {
            flows: true,
            files: true,
            databases: ForkDatabaseMode::WithData,
            roles: true,
            widgets: true,
            templates: true,
        }
    }
}

impl ForkPolicy {
    /// Reads the policy off an `App` row. A NULL column — or a payload that
    /// no longer parses — falls back to "copy everything" so a malformed
    /// value can never silently produce an empty fork.
    pub fn from_app_row(row: &app::Model) -> Self {
        row.fork_policy
            .clone()
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default()
    }

    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// Extracts the LanceDB table directory from a path relative to
/// `apps/{id}/storage` — the suffix `copy_object_prefix` yields at the
/// `"app storage"` call site.
///
/// `db/foo.lance/data/x.lance` → `Some("foo")`, `db/loose.txt` → `None`.
pub fn lance_table_of(relative_under_storage: &str) -> Option<&str> {
    relative_under_storage
        .strip_prefix("db/")?
        .split('/')
        .next()?
        .strip_suffix(".lance")
}

/// Decides, per source-relative path, whether an object stays out of the
/// destination.
pub type SkipPredicate = Box<dyn Fn(&str) -> bool + Send + Sync>;

/// Skip predicate for the `apps/{id}/storage` mirror. `None` copies
/// everything.
///
/// Exclusion is whole-table: a LanceDB table is only openable with its
/// `data/`, `_versions/`, `_transactions/` and `_indices/` subtrees intact,
/// so the predicate never splits a `{table}.lance/` directory.
pub fn storage_skip(policy: &ForkPolicy) -> Option<SkipPredicate> {
    match policy.databases {
        ForkDatabaseMode::WithData => None,
        ForkDatabaseMode::None => Some(Box::new(|relative: &str| relative.starts_with("db/"))),
        ForkDatabaseMode::SchemaOnly => Some(Box::new(|relative: &str| {
            if !relative.starts_with("db/") {
                return false;
            }
            match lance_table_of(relative) {
                Some(table) => !flow_like_catalog_core::is_reserved_table(table),
                // Loose objects directly under `db/` that aren't a `.lance`
                // directory carry no schema — they go with the data.
                None => true,
            }
        })),
    }
}

/// Content-store prefixes (relative to `apps/{src_app_id}/`) the desktop
/// must not mirror on an offline fork.
///
/// Advisory: the caller already holds `ReadFiles` on the source, so this is
/// hygiene rather than a boundary. `skip_tables` are the source's
/// non-reserved table names, needed because a blanket `storage/db/` would
/// also drop the reserved artifact tables schema-only mode carries.
pub fn offline_content_exclude_prefixes(
    policy: &ForkPolicy,
    skip_tables: &[String],
) -> Vec<String> {
    let mut prefixes = Vec::new();
    if !policy.files {
        prefixes.push("upload/".to_string());
    }
    match policy.databases {
        ForkDatabaseMode::WithData => {}
        ForkDatabaseMode::None => prefixes.push("storage/db/".to_string()),
        ForkDatabaseMode::SchemaOnly => {
            for table in skip_tables {
                prefixes.push(format!("storage/db/{table}.lance/"));
            }
        }
    }
    if !policy.widgets {
        prefixes.push("metadata/widgets/".to_string());
    }
    if !policy.templates {
        prefixes.push("metadata/templates/".to_string());
    }
    prefixes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row_with(policy: Option<serde_json::Value>) -> app::Model {
        use crate::entity::sea_orm_active_enums::{ExecutionMode, Status, Visibility};
        app::Model {
            id: "app".to_string(),
            status: Status::Active,
            visibility: Visibility::Private,
            changelog: None,
            default_role_id: None,
            owner_role_id: None,
            primary_category: None,
            secondary_category: None,
            rating_sum: 0,
            rating_count: 0,
            download_count: 0,
            interactions_count: 0,
            avg_rating: None,
            relevance_score: None,
            total_size: 0,
            price: 0,
            version: None,
            execution_mode: ExecutionMode::Any,
            bits: None,
            created_at: Default::default(),
            updated_at: Default::default(),
            allow_forking: true,
            fork_policy: policy,
            forked_at: None,
            forked_from: None,
            app_type: None,
        }
    }

    #[test]
    fn fork_policy_defaults_to_everything_when_column_is_null() {
        let policy = ForkPolicy::from_app_row(&row_with(None));
        assert!(policy.flows);
        assert!(policy.files);
        assert!(policy.roles);
        assert!(policy.widgets);
        assert!(policy.templates);
        assert_eq!(policy.databases, ForkDatabaseMode::WithData);
        assert!(policy.is_default());
    }

    #[test]
    fn fork_policy_deserializes_partial_json_as_permissive() {
        let policy = ForkPolicy::from_app_row(&row_with(Some(serde_json::json!({"files": false}))));
        assert!(!policy.files);
        assert!(policy.flows);
        assert!(policy.widgets);
        assert_eq!(policy.databases, ForkDatabaseMode::WithData);
    }

    #[test]
    fn malformed_fork_policy_falls_back_to_everything() {
        let policy = ForkPolicy::from_app_row(&row_with(Some(serde_json::json!("nonsense"))));
        assert!(policy.is_default());
    }

    #[test]
    fn lance_table_of_extracts_the_table_directory() {
        assert_eq!(lance_table_of("db/foo.lance/data/x.lance"), Some("foo"));
        assert_eq!(lance_table_of("db/foo.lance"), Some("foo"));
        assert_eq!(lance_table_of("db/loose.txt"), None);
        assert_eq!(lance_table_of("notdb/foo.lance"), None);
        assert_eq!(lance_table_of("db/"), None);
    }

    #[test]
    fn schema_only_skip_keeps_reserved_artifact_tables() {
        let policy = ForkPolicy {
            databases: ForkDatabaseMode::SchemaOnly,
            ..Default::default()
        };
        let skip = storage_skip(&policy).expect("schema-only must filter");
        assert!(skip("db/tables.lance/data/0.lance"));
        assert!(skip("db/tables.lance/_versions/1.manifest"));
        assert!(!skip("db/__graph_overlays__.lance/data/0.lance"));
        assert!(!skip("db/__saved_queries_manifest__.lance/data/0.lance"));
        assert!(!skip("notes.txt"));
        assert!(!skip("node_scratch/out.json"));
    }

    #[test]
    fn database_none_skips_every_object_under_db() {
        let policy = ForkPolicy {
            databases: ForkDatabaseMode::None,
            ..Default::default()
        };
        let skip = storage_skip(&policy).expect("none must filter");
        assert!(skip("db/tables.lance/data/0.lance"));
        assert!(skip("db/__graph_overlays__.lance/data/0.lance"));
        assert!(!skip("notes.txt"));
    }

    #[test]
    fn with_data_never_filters() {
        assert!(storage_skip(&ForkPolicy::default()).is_none());
    }

    #[test]
    fn excluded_content_prefixes_match_the_policy() {
        let tables = vec!["tables".to_string(), "notes".to_string()];

        assert!(offline_content_exclude_prefixes(&ForkPolicy::default(), &tables).is_empty());

        let no_files = ForkPolicy {
            files: false,
            ..Default::default()
        };
        assert_eq!(
            offline_content_exclude_prefixes(&no_files, &tables),
            vec!["upload/".to_string()]
        );

        let no_db = ForkPolicy {
            databases: ForkDatabaseMode::None,
            ..Default::default()
        };
        assert_eq!(
            offline_content_exclude_prefixes(&no_db, &tables),
            vec!["storage/db/".to_string()]
        );

        let schema_only = ForkPolicy {
            databases: ForkDatabaseMode::SchemaOnly,
            ..Default::default()
        };
        assert_eq!(
            offline_content_exclude_prefixes(&schema_only, &tables),
            vec![
                "storage/db/tables.lance/".to_string(),
                "storage/db/notes.lance/".to_string(),
            ]
        );

        let bare = ForkPolicy {
            widgets: false,
            templates: false,
            ..Default::default()
        };
        assert_eq!(
            offline_content_exclude_prefixes(&bare, &tables),
            vec![
                "metadata/widgets/".to_string(),
                "metadata/templates/".to_string(),
            ]
        );
    }
}
