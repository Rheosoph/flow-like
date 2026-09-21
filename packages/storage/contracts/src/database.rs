use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A table reference to open. Tags resolve once to an exact branch and version.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct DatabaseSelector {
    pub branch: String,
    pub version: Option<u64>,
    pub tag: Option<String>,
    pub read_only: bool,
}

impl Default for DatabaseSelector {
    fn default() -> Self {
        Self {
            branch: "main".into(),
            version: None,
            tag: None,
            read_only: false,
        }
    }
}

impl DatabaseSelector {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.branch.trim().is_empty(),
            "Branch name cannot be empty"
        );
        anyhow::ensure!(
            self.version.is_none() || self.tag.is_none(),
            "Choose either a version or a tag"
        );
        anyhow::ensure!(self.version != Some(0), "Version must be greater than zero");
        anyhow::ensure!(
            self.tag.as_ref().is_none_or(|tag| !tag.trim().is_empty()),
            "Tag name cannot be empty"
        );
        Ok(())
    }

    pub fn is_pinned(&self) -> bool {
        self.version.is_some() || self.tag.is_some()
    }

    pub fn is_read_only(&self) -> bool {
        self.read_only || self.is_pinned()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DatabaseReference {
    pub table: String,
    pub branch: String,
    pub version: u64,
    pub read_only: bool,
    pub pinned: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct DatabaseVersion {
    pub version: u64,
    pub timestamp: String,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct DatabaseBranch {
    pub name: String,
    pub parent_branch: Option<String>,
    pub parent_version: Option<u64>,
    pub created_at: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct DatabaseTag {
    pub name: String,
    pub branch: String,
    pub version: u64,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct DatabaseCleanupStats {
    pub bytes_removed: u64,
    pub old_versions: u64,
    pub data_files_removed: u64,
    pub transaction_files_removed: u64,
    pub index_files_removed: u64,
    pub deletion_files_removed: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct DatabaseHistory {
    pub reference: DatabaseReference,
    pub versions: Vec<DatabaseVersion>,
    pub branches: Vec<DatabaseBranch>,
    pub tags: Vec<DatabaseTag>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum DatabaseAction {
    CreateBranch { name: String },
    DeleteBranch { name: String },
    CreateTag { name: String },
    UpdateTag { name: String },
    DeleteTag { name: String },
    Restore,
    Snapshot { name: Option<String> },
    Cleanup { older_than_days: u64 },
    Clone { name: String },
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct DatabaseActionResult {
    pub reference: DatabaseReference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cleanup: Option<DatabaseCleanupStats>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct DatabaseDiffRow {
    pub kind: String,
    pub key: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct DatabaseDiff {
    pub source: DatabaseReference,
    pub target: DatabaseReference,
    pub added: u64,
    pub removed: u64,
    pub changed: u64,
    pub unchanged: u64,
    pub schema_changes: Vec<String>,
    pub rows: Vec<DatabaseDiffRow>,
    pub truncated: bool,
}
