use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use flow_like_types::tokio;
use serde::Serialize;

use crate::env::{Limits, ToolEnv};
use crate::error::{ToolError, ToolResult};
use crate::layout::{EXPORTS_DIR, FILES_DIR, normalize_path};

const MAX_DEPTH: usize = 16;
const MAX_SCANNED: usize = 20_000;

pub(super) type Snapshot = BTreeMap<String, (u64, Option<SystemTime>)>;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Skipped {
    pub path: String,
    pub reason: String,
}

/// Workspace paths that travel into the working directory and back: the user's files, not the
/// bot's own bookkeeping under `bot/`.
pub fn mirrored(path: &str) -> bool {
    !path.starts_with("bot/")
        || path.starts_with(&format!("{FILES_DIR}/"))
        || path.starts_with(&format!("{EXPORTS_DIR}/"))
}

/// The local file for `relative`, with its parent created, only when the real location is still
/// inside `root` (a symlink the program planted cannot lead a write out).
fn confined_target(root: &Path, relative: &str) -> Option<PathBuf> {
    let target = root.join(relative);
    let parent = target.parent()?;
    std::fs::create_dir_all(parent).ok()?;
    let real_root = root.canonicalize().ok()?;
    if !parent.canonicalize().ok()?.starts_with(&real_root) {
        return None;
    }
    match std::fs::symlink_metadata(&target) {
        Ok(meta) if meta.file_type().is_symlink() => None,
        _ => Some(target),
    }
}

pub(super) async fn pull(env: &dyn ToolEnv, root: &Path) -> ToolResult<usize> {
    let Some(workspace) = env.scope().workspace.clone() else {
        return Ok(0);
    };
    let limits = env.limits().clone();
    let entries = env
        .list_files(&workspace, "", limits.shell_pull_max_files)
        .await?;
    let mut pulled = 0;
    for entry in entries {
        if !mirrored(&entry.path) || entry.bytes > limits.shell_sync_file_bytes {
            continue;
        }
        let Ok(relative) = normalize_path(&entry.path) else {
            continue;
        };
        let local = root.join(&relative);
        if std::fs::metadata(&local).is_ok_and(|meta| meta.is_file() && meta.len() == entry.bytes) {
            continue;
        }
        let Some(data) = env
            .read_file(&workspace, &relative, limits.shell_sync_file_bytes)
            .await?
        else {
            continue;
        };
        if data.size > limits.shell_sync_file_bytes {
            continue;
        }
        let Some(target) = confined_target(root, &relative) else {
            continue;
        };
        if tokio::fs::write(&target, &data.bytes).await.is_ok() {
            pulled += 1;
        }
    }
    Ok(pulled)
}

fn walk(dir: &Path, prefix: &str, depth: usize, found: &mut Snapshot) {
    if depth > MAX_DEPTH || found.len() >= MAX_SCANNED {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(meta) = std::fs::symlink_metadata(entry.path()) else {
            continue;
        };
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let relative = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        if meta.is_dir() {
            walk(&entry.path(), &relative, depth + 1, found);
        } else if meta.is_file() {
            found.insert(relative, (meta.len(), meta.modified().ok()));
        }
    }
}

pub(super) async fn snapshot(root: &Path) -> Snapshot {
    let root = root.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let mut found = Snapshot::new();
        walk(&root, "", 0, &mut found);
        found
    })
    .await
    .unwrap_or_default()
}

pub(super) struct Pushed {
    pub files: Vec<flow_like_types::bot_message::WorkspaceHandle>,
    pub skipped: Vec<Skipped>,
}

pub(super) async fn push(
    env: &dyn ToolEnv,
    root: &Path,
    before: &Snapshot,
    after: &Snapshot,
) -> Pushed {
    let mut pushed = Pushed {
        files: Vec::new(),
        skipped: Vec::new(),
    };
    let Some(workspace) = env.scope().workspace.clone() else {
        return pushed;
    };
    let limits: Limits = env.limits().clone();
    let skip = |pushed: &mut Pushed, path: &str, reason: &str| {
        pushed.skipped.push(Skipped {
            path: path.to_string(),
            reason: reason.to_string(),
        });
    };
    for (path, state) in after {
        if before.get(path) == Some(state) {
            continue;
        }
        if pushed.files.len() >= limits.shell_sync_max_files {
            skip(&mut pushed, path, "more changed files than are kept");
            continue;
        }
        if !mirrored(path) || normalize_path(path).is_err() {
            skip(&mut pushed, path, "not a workspace path");
            continue;
        }
        if state.0 > limits.shell_sync_file_bytes {
            skip(&mut pushed, path, "larger than the sync limit");
            continue;
        }
        let Ok(bytes) = tokio::fs::read(root.join(path)).await else {
            skip(&mut pushed, path, "could not be read");
            continue;
        };
        let mime = crate::files::guess_mime(path);
        let bytes = flow_like_types::Bytes::from(bytes);
        if env
            .write_workspace(path, bytes.clone(), &mime)
            .await
            .is_err()
        {
            skip(&mut pushed, path, "could not be saved to the workspace");
            continue;
        }
        pushed
            .files
            .push(crate::files::handle_for(&workspace, path, &bytes, &mime));
    }
    pushed
}

pub(super) fn ensure_root(root: &Path) -> ToolResult<PathBuf> {
    std::fs::create_dir_all(root).map_err(|error| {
        ToolError::new(
            crate::error::code::FAILED,
            format!("Could not create the working directory: {error}"),
        )
    })?;
    root.canonicalize().map_err(|error| {
        ToolError::new(
            crate::error::code::FAILED,
            format!("Could not open the working directory: {error}"),
        )
    })
}

/// `cwd` as a path relative to the working directory, created when missing. Anything that
/// would leave `root` (absolute, `..`, a symlink that points out) is refused.
pub fn confine_cwd(root: &Path, cwd: Option<&str>) -> ToolResult<String> {
    let raw = cwd.map(str::trim).unwrap_or_default();
    if raw.is_empty() || raw == "." {
        return Ok(String::new());
    }
    if raw.starts_with('/') || raw.contains(':') {
        return Err(ToolError::refused(format!(
            "The working directory '{raw}' must be relative to the bot's working directory"
        )));
    }
    let relative = normalize_path(raw).map_err(|error| ToolError::refused(error.message))?;
    let real_root = ensure_root(root)?;
    let target = real_root.join(&relative);
    std::fs::create_dir_all(&target).map_err(|error| {
        ToolError::invalid(format!(
            "The working directory '{relative}' cannot be used: {error}"
        ))
    })?;
    let real = target.canonicalize().map_err(|error| {
        ToolError::invalid(format!(
            "The working directory '{relative}' cannot be used: {error}"
        ))
    })?;
    if !real.starts_with(&real_root) {
        return Err(ToolError::refused(format!(
            "The working directory '{relative}' leaves the bot's working directory"
        )));
    }
    Ok(relative)
}
