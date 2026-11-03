use std::borrow::Cow;

use flow_like_types::bot_message::WorkspaceHandle;
use flow_like_types::data_classification::DataClassification;
use flow_like_types::{Bytes, mime_guess};
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::env::{FileData, ToolEnv};
use crate::error::{ToolError, ToolResult, code};
use crate::layout::{file_name, inbox_path, normalize_path};

/// One reference for every source of a file.
#[derive(Debug, Clone, PartialEq)]
pub enum FileRef {
    Workspace(String),
    App { app_id: String, path: String },
    Attachment { seq: u64, part: String },
    Handle(WorkspaceHandle),
}

impl FileRef {
    pub fn parse(text: &str) -> ToolResult<Self> {
        let text = text.trim();
        if let Some(rest) = text.strip_prefix('#') {
            return Self::attachment(rest);
        }
        if let Some(rest) = text.strip_prefix("app:") {
            let (app_id, path) = rest.split_once('/').ok_or_else(|| {
                ToolError::invalid(format!(
                    "'{text}' is not 'app:<app_id>/<path>'; a workspace path needs no prefix"
                ))
            })?;
            return Ok(Self::App {
                app_id: app_id.to_string(),
                path: normalize_path(path)?,
            });
        }
        Ok(Self::Workspace(normalize_path(text)?))
    }

    fn attachment(rest: &str) -> ToolResult<Self> {
        let invalid = || {
            ToolError::invalid(format!(
                "'#{rest}' is not a message attachment; expected '#<seq>.a<N>', e.g. '#12.a1'"
            ))
        };
        let (seq, part) = rest.split_once('.').ok_or_else(invalid)?;
        let numbered = part
            .strip_prefix('a')
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
        match (seq.parse::<u64>(), numbered) {
            (Ok(seq), true) => Ok(Self::Attachment {
                seq,
                part: part.to_string(),
            }),
            _ => Err(invalid()),
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::Workspace(path) => path.clone(),
            Self::App { app_id, path } => format!("app:{app_id}/{path}"),
            Self::Attachment { seq, part } => format!("#{seq}.{part}"),
            Self::Handle(handle) => format!("{}/{}", handle.app_id, handle.path),
        }
    }

    pub fn from_value(value: Value) -> ToolResult<Self> {
        match value {
            Value::String(text) => Self::parse(&text),
            Value::Object(ref fields) if fields.contains_key("sha256") => {
                serde_json::from_value::<WorkspaceHandle>(value)
                    .map(Self::Handle)
                    .map_err(|error| {
                        ToolError::invalid(format!(
                            "A handle needs app_id, path, bytes, sha256 and mime: {error}"
                        ))
                    })
            }
            Value::Object(fields) => {
                let text = |key: &str| fields.get(key).and_then(Value::as_str);
                match (text("app_id"), text("path")) {
                    (Some(app_id), Some(path)) => Ok(Self::App {
                        app_id: app_id.to_string(),
                        path: normalize_path(path)?,
                    }),
                    _ => Err(ToolError::invalid(
                        "A file object needs app_id and path (or a whole handle)",
                    )),
                }
            }
            other => Err(ToolError::invalid(format!(
                "A file reference is a string or an object, not {other}"
            ))),
        }
    }
}

impl serde::Serialize for FileRef {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Handle(handle) => handle.serialize(serializer),
            other => serializer.serialize_str(&other.label()),
        }
    }
}

impl<'de> Deserialize<'de> for FileRef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Self::from_value(value).map_err(|error| serde::de::Error::custom(error.message))
    }
}

impl JsonSchema for FileRef {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("FileRef")
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "A file: a workspace path ('bot/notes/a.md'), another app's storage path ('app:<app_id>/<path>' or {app_id, path}), a message attachment ('#12.a1') or a handle {app_id, path, bytes, sha256, mime}.",
            "oneOf": [
                { "type": "string" },
                {
                    "type": "object",
                    "properties": {
                        "app_id": { "type": "string" },
                        "path": { "type": "string" },
                        "bytes": { "type": "integer" },
                        "sha256": { "type": "string" },
                        "mime": { "type": "string" }
                    },
                    "required": ["app_id", "path"]
                }
            ]
        })
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub fn guess_mime(path: &str) -> String {
    mime_guess::from_path(path)
        .first_raw()
        .unwrap_or("application/octet-stream")
        .to_string()
}

pub fn handle_for(app_id: &str, path: &str, bytes: &[u8], mime: &str) -> WorkspaceHandle {
    WorkspaceHandle {
        app_id: app_id.to_string(),
        path: path.to_string(),
        bytes: bytes.len() as u64,
        sha256: sha256_hex(bytes),
        mime: mime.to_string(),
    }
}

#[derive(Debug, Clone)]
pub struct ResolvedFile {
    pub app_id: String,
    pub path: String,
    pub name: String,
    pub bytes: Bytes,
    pub mime: String,
    pub classification: Option<DataClassification>,
}

impl ResolvedFile {
    pub fn handle(&self) -> WorkspaceHandle {
        handle_for(&self.app_id, &self.path, &self.bytes, &self.mime)
    }

    pub fn source_key(&self) -> String {
        format!("{}/{}", self.app_id, self.path)
    }
}

pub fn workspace_of(env: &dyn ToolEnv, action: &str) -> ToolResult<String> {
    env.scope()
        .workspace
        .clone()
        .ok_or_else(|| ToolError::no_workspace(action))
}

pub fn require_storage_read(env: &dyn ToolEnv, app_id: &str) -> ToolResult<()> {
    let scope = env.scope();
    if scope.can_read_storage(app_id) {
        return Ok(());
    }
    let reason = if !scope.storage {
        "the bot does not have the storage capability"
    } else {
        "the app is not one of the bot's apps"
    };
    Err(ToolError::refused(format!(
        "Cannot read app '{app_id}': {reason}. Files in the workspace, in messages sent to the bot and handles given to it are always readable"
    )))
}

async fn read_whole(
    env: &dyn ToolEnv,
    app_id: &str,
    path: &str,
    max_bytes: u64,
) -> ToolResult<FileData> {
    let data = env
        .read_file(app_id, path, max_bytes)
        .await?
        .ok_or_else(|| ToolError::not_found(format!("No file '{path}' in app '{app_id}'")))?;
    if data.size > max_bytes || data.bytes.len() as u64 > max_bytes {
        return Err(ToolError::new(
            code::TOO_LARGE,
            format!(
                "'{path}' is {} bytes; this tool reads at most {max_bytes}",
                data.size
            ),
        ));
    }
    Ok(data)
}

fn resolved(app_id: &str, path: &str, data: FileData) -> ResolvedFile {
    let mime = data.mime.unwrap_or_else(|| guess_mime(path));
    ResolvedFile {
        app_id: app_id.to_string(),
        path: path.to_string(),
        name: file_name(path).to_string(),
        bytes: data.bytes,
        mime,
        classification: data.classification,
    }
}

pub async fn resolve(
    env: &dyn ToolEnv,
    reference: &FileRef,
    max_bytes: u64,
) -> ToolResult<ResolvedFile> {
    match reference {
        FileRef::Workspace(path) => {
            let workspace = workspace_of(env, "read workspace files")?;
            let data = read_whole(env, &workspace, path, max_bytes).await?;
            Ok(resolved(&workspace, path, data))
        }
        FileRef::App { app_id, path } => {
            require_storage_read(env, app_id)?;
            let data = read_whole(env, app_id, path, max_bytes).await?;
            Ok(resolved(app_id, path, data))
        }
        FileRef::Handle(handle) => resolve_handle(env, handle, max_bytes).await,
        FileRef::Attachment { seq, part } => resolve_attachment(env, *seq, part, max_bytes).await,
    }
}

async fn resolve_handle(
    env: &dyn ToolEnv,
    handle: &WorkspaceHandle,
    max_bytes: u64,
) -> ToolResult<ResolvedFile> {
    if !env.handle_granted(handle) {
        require_storage_read(env, &handle.app_id)?;
    }
    let data = read_whole(env, &handle.app_id, &handle.path, max_bytes).await?;
    if !handle.sha256.is_empty() && sha256_hex(&data.bytes) != handle.sha256.to_lowercase() {
        return Err(ToolError::new(
            code::HANDLE_MISMATCH,
            format!(
                "'{}' changed since the handle was made (sha256 differs); ask for a fresh handle",
                handle.path
            ),
        ));
    }
    Ok(resolved(&handle.app_id, &handle.path, data))
}

async fn resolve_attachment(
    env: &dyn ToolEnv,
    seq: u64,
    part: &str,
    max_bytes: u64,
) -> ToolResult<ResolvedFile> {
    let attachment = env.message_attachment(seq, part).await?.ok_or_else(|| {
        ToolError::not_found(format!("Message #{seq} has no attachment '{part}'"))
    })?;
    let source = &attachment.handle;
    let workspace = env.scope().workspace.clone();
    let Some(workspace) = workspace else {
        let data = read_whole(env, &source.app_id, &source.path, max_bytes).await?;
        return Ok(resolved(&source.app_id, &source.path, data));
    };
    if source.app_id == workspace {
        let data = read_whole(env, &workspace, &source.path, max_bytes).await?;
        return Ok(resolved(&workspace, &source.path, data));
    }
    let copy = inbox_path(seq, part, &attachment.name);
    if env.stat_file(&workspace, &copy).await?.is_some() {
        let data = read_whole(env, &workspace, &copy, max_bytes).await?;
        return Ok(resolved(&workspace, &copy, data));
    }
    let data = read_whole(env, &source.app_id, &source.path, max_bytes).await?;
    let mime = data.mime.clone().unwrap_or_else(|| source.mime.clone());
    env.write_workspace(&copy, data.bytes.clone(), &mime)
        .await?;
    Ok(resolved(
        &workspace,
        &copy,
        FileData {
            mime: Some(mime),
            ..data
        },
    ))
}
