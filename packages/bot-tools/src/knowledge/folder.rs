use std::borrow::Cow;
use std::collections::BTreeSet;

use chrono::Utc;
use flow_like_types::data_classification::DataClassification;
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer};
use serde_json::{Value, json};

use super::store::{FolderRecord, Knowledge, SourceRecord, folder_record_id, source_record_id};
use super::{Added, index_file};
use crate::embed::KnowledgeEmbedder;
use crate::env::{FileEntry, Progress, ToolEnv};
use crate::error::{ToolError, ToolResult, code};
use crate::files::{FileRef, require_storage_read, resolve, sha256_hex, workspace_of};
use crate::layout::normalize_path;

const REPORTED: usize = 20;

/// A folder to index: a workspace folder, `app:<app_id>/<prefix>/` or `{app_id, prefix}`.
#[derive(Debug, Clone, PartialEq)]
pub struct FolderRef {
    pub app_id: Option<String>,
    pub prefix: String,
}

impl FolderRef {
    pub fn parse(text: &str) -> ToolResult<Self> {
        let text = text.trim();
        let (app_id, rest) = match text.strip_prefix("app:") {
            Some(rest) => {
                let (app_id, path) = rest.split_once('/').unwrap_or((rest, ""));
                if app_id.is_empty() {
                    return Err(ToolError::invalid(format!(
                        "'{text}' is not 'app:<app_id>/<folder>/'"
                    )));
                }
                (Some(app_id.to_string()), path)
            }
            None => (None, text),
        };
        Ok(Self {
            app_id,
            prefix: folder_prefix(rest)?,
        })
    }

    pub fn from_value(value: Value) -> ToolResult<Self> {
        match value {
            Value::String(text) => Self::parse(&text),
            Value::Object(fields) => {
                let text = |key: &str| fields.get(key).and_then(Value::as_str);
                Ok(Self {
                    app_id: text("app_id").map(str::to_string),
                    prefix: folder_prefix(text("prefix").or(text("path")).unwrap_or_default())?,
                })
            }
            other => Err(ToolError::invalid(format!(
                "A folder reference is a string or an object, not {other}"
            ))),
        }
    }

    fn resolve(&self, env: &dyn ToolEnv) -> ToolResult<(String, String)> {
        let app_id = match &self.app_id {
            Some(app_id) => app_id.clone(),
            None => workspace_of(env, "read workspace folders")?,
        };
        require_storage_read(env, &app_id)?;
        Ok((app_id, self.prefix.clone()))
    }
}

fn folder_prefix(text: &str) -> ToolResult<String> {
    let trimmed = text.trim().trim_matches('/');
    if trimmed.is_empty() {
        Ok(String::new())
    } else {
        normalize_path(trimmed)
    }
}

impl<'de> Deserialize<'de> for FolderRef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Self::from_value(value).map_err(|error| serde::de::Error::custom(error.message))
    }
}

impl JsonSchema for FolderRef {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("FolderRef")
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "A folder, indexed recursively: a workspace folder ('bot/notes/'), another app's folder ('app:<app_id>/<folder>/') or {app_id, prefix}.",
            "oneOf": [
                { "type": "string" },
                {
                    "type": "object",
                    "properties": {
                        "app_id": { "type": "string" },
                        "prefix": { "type": "string" }
                    },
                    "required": ["app_id", "prefix"]
                }
            ]
        })
    }
}

#[derive(Default)]
pub struct Totals {
    pub failed: Vec<Value>,
    pub warnings: Vec<String>,
    pub classification: Option<DataClassification>,
    pub chars: u64,
}

struct Tally {
    listed: usize,
    added: usize,
    updated: usize,
    unchanged: usize,
    removed: usize,
    changed: Vec<String>,
    skipped: Vec<Value>,
    skipped_total: usize,
}

impl Tally {
    fn skip(&mut self, path: &str, reason: &str) {
        self.skipped_total += 1;
        if self.skipped.len() < REPORTED {
            self.skipped.push(json!({ "file": path, "reason": reason }));
        }
    }

    fn change(&mut self, path: &str) {
        if self.changed.len() < REPORTED {
            self.changed.push(path.to_string());
        }
    }
}

fn under(path: &str, prefix: &str) -> bool {
    prefix.is_empty()
        || path
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with('/'))
}

fn unchanged_by_listing(record: &SourceRecord, entry: &FileEntry) -> bool {
    entry.modified_ms == Some(record.modified_ms) && entry.bytes as i64 == record.bytes
}

fn record_for(
    source: &str,
    index: &str,
    entry: &FileEntry,
    sha256: String,
    chunks: usize,
) -> SourceRecord {
    SourceRecord {
        id: source_record_id(index, source),
        kb_index: index.to_string(),
        source: source.to_string(),
        bytes: entry.bytes as i64,
        modified_ms: entry.modified_ms.unwrap_or(-1),
        sha256,
        chunks: chunks as i64,
        indexed_at: Utc::now().timestamp_millis(),
    }
}

fn file_ref(app_id: &str, workspace: &str, path: &str) -> FileRef {
    if app_id == workspace {
        FileRef::Workspace(path.to_string())
    } else {
        FileRef::App {
            app_id: app_id.to_string(),
            path: path.to_string(),
        }
    }
}

pub struct Index<'a> {
    pub env: &'a dyn ToolEnv,
    pub store: &'a Knowledge,
    pub embedder: &'a dyn KnowledgeEmbedder,
    pub name: &'a str,
    pub dimension: usize,
}

enum Indexed {
    Unchanged,
    Added(Added),
    Skipped(&'static str),
}

impl Index<'_> {
    /// Indexes the files of one folder that are new or changed since the last pass; with
    /// `sync`, drops the files that are no longer listed.
    pub async fn sync_folder(
        &self,
        folder: &FolderRef,
        sync: bool,
        totals: &mut Totals,
    ) -> ToolResult<Value> {
        let limits = self.env.limits().clone();
        let (app_id, prefix) = folder.resolve(self.env)?;
        let workspace = self.env.scope().workspace.clone().unwrap_or_default();
        let mut entries = self
            .env
            .list_files(&app_id, &prefix, limits.folder_scan_limit + 1)
            .await?;
        let scan_cut = entries.len() > limits.folder_scan_limit;
        entries.truncate(limits.folder_scan_limit);
        entries.retain(|entry| under(&entry.path, &prefix));
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        if entries.is_empty() && !scan_cut {
            return Err(ToolError::not_found(format!(
                "No files under '{}' in app '{app_id}'",
                if prefix.is_empty() { "/" } else { &prefix }
            )));
        }

        let records = self.store.source_records(self.name).await?;
        let mut tally = Tally {
            listed: entries.len(),
            added: 0,
            updated: 0,
            unchanged: 0,
            removed: 0,
            changed: Vec::new(),
            skipped: Vec::new(),
            skipped_total: 0,
        };
        let mut files_left = limits.max_files_per_folder;
        let mut bytes_left = limits.max_folder_bytes;
        let work = entries.len();
        for (position, entry) in entries.iter().enumerate() {
            let source = format!("{app_id}/{}", entry.path);
            let known = records.get(&source);
            if known.is_some_and(|record| unchanged_by_listing(record, entry)) {
                tally.unchanged += 1;
                continue;
            }
            if entry.bytes > limits.max_ingest_bytes {
                tally.skip(&entry.path, "file too large");
                continue;
            }
            if files_left == 0 || entry.bytes > bytes_left {
                tally.skip(&entry.path, "per-call cap reached");
                continue;
            }
            files_left -= 1;
            bytes_left -= entry.bytes;
            if self.env.is_cancelled() {
                return Err(ToolError::new(code::CANCELLED, "The call was cancelled"));
            }
            self.env.progress(Progress {
                message: format!("Folder file {} of {work}", position + 1),
                fraction: Some(position as f32 / work.max(1) as f32),
            });
            let reference = file_ref(&app_id, &workspace, &entry.path);
            let was_indexed = known.is_some();
            match self.index_entry(&reference, entry, known, &source).await {
                Ok(Indexed::Unchanged) => {
                    files_left += 1;
                    bytes_left += entry.bytes;
                    tally.unchanged += 1;
                }
                Ok(Indexed::Skipped(reason)) => tally.skip(&entry.path, reason),
                Ok(Indexed::Added(done)) => {
                    totals.classification = totals.classification.max(done.classification);
                    totals.chars += done.chars;
                    if done.dropped {
                        totals.warnings.push(format!(
                            "'{}' was cut at {} chunks",
                            entry.path, limits.max_chunks_per_source
                        ));
                    }
                    if was_indexed {
                        tally.updated += 1;
                    } else {
                        tally.added += 1;
                    }
                    tally.change(&entry.path);
                }
                Err(error) if error.code == code::CANCELLED => return Err(error),
                Err(error) => totals.failed.push(json!({
                    "file": reference.label(),
                    "code": error.code,
                    "error": error.message,
                })),
            }
        }

        if sync {
            if scan_cut {
                totals.warnings.push(format!(
                    "Folder '{prefix}' lists more than {} files, so removed files were not cleaned up",
                    limits.folder_scan_limit
                ));
            } else {
                let listed: BTreeSet<String> = entries
                    .iter()
                    .map(|entry| format!("{app_id}/{}", entry.path))
                    .collect();
                let folder_key = format!("{app_id}/{prefix}");
                for source in records.keys() {
                    if under(source, &folder_key) && !listed.contains(source) {
                        self.store.remove_source(self.name, source).await?;
                        tally.removed += 1;
                    }
                }
            }
        }

        self.store
            .put_folder(&FolderRecord {
                id: folder_record_id(self.name, &app_id, &prefix),
                kb_index: self.name.to_string(),
                app_id: app_id.clone(),
                prefix: prefix.clone(),
                last_sync_ms: Utc::now().timestamp_millis(),
                files: tally.listed as i64,
                added: tally.added as i64,
                updated: tally.updated as i64,
                removed: tally.removed as i64,
            })
            .await?;
        Ok(json!({
            "app_id": app_id,
            "prefix": prefix,
            "listed": tally.listed,
            "added": tally.added,
            "updated": tally.updated,
            "unchanged": tally.unchanged,
            "removed": tally.removed,
            "changed": tally.changed,
            "skipped": tally.skipped,
            "skipped_total": tally.skipped_total,
        }))
    }

    async fn index_entry(
        &self,
        reference: &FileRef,
        entry: &FileEntry,
        known: Option<&SourceRecord>,
        source: &str,
    ) -> ToolResult<Indexed> {
        let limits = self.env.limits();
        let file = resolve(self.env, reference, limits.max_ingest_bytes).await?;
        let sha = sha256_hex(&file.bytes);
        if known.is_some_and(|record| record.sha256 == sha) {
            let chunks = known.map_or(0, |record| record.chunks as usize);
            self.store
                .put_source(&record_for(source, self.name, entry, sha, chunks))
                .await?;
            return Ok(Indexed::Unchanged);
        }
        match index_file(
            self.env,
            self.store,
            self.embedder,
            self.name,
            self.dimension,
            file,
        )
        .await
        {
            Ok(done) => {
                self.store
                    .put_source(&record_for(source, self.name, entry, sha, done.chunk_count))
                    .await?;
                Ok(Indexed::Added(done))
            }
            Err(error) if error.code == code::UNSUPPORTED_FILE => {
                self.store
                    .put_source(&record_for(source, self.name, entry, sha, 0))
                    .await?;
                Ok(Indexed::Skipped("not a readable document"))
            }
            Err(error) => Err(error),
        }
    }
}
