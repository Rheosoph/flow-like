use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use super::artifact::sha256_hex;
use super::{SandboxFile, SandboxLimits, SandboxRequest};
use crate::pyodide::network::GuestNetwork;

const MAX_WALK_DEPTH: usize = 16;

pub(super) struct Collected {
    pub files: Vec<SandboxFile>,
    pub truncated: bool,
}

pub fn validate_relative_path(path: &str) -> Result<String> {
    if path.is_empty() || path.contains('\0') || path.contains('\\') || path.starts_with('/') {
        bail!("invalid file path {path:?}: must be a relative path with '/' separators");
    }
    let parts: Vec<&str> = path.split('/').collect();
    if parts
        .iter()
        .any(|p| p.is_empty() || *p == "." || *p == "..")
    {
        bail!("invalid file path {path:?}: empty, '.' and '..' components are not allowed");
    }
    Ok(parts.join("/"))
}

pub(super) fn validate_inputs(
    files: &[SandboxFile],
    max_bytes: u64,
) -> Result<HashMap<String, String>> {
    let mut total = 0u64;
    let mut staged = HashMap::new();
    for file in files {
        let path = validate_relative_path(&file.path)?;
        total += file.bytes.len() as u64;
        if total > max_bytes {
            bail!("input files exceed the {max_bytes} byte limit");
        }
        if staged
            .insert(path.clone(), sha256_hex(&file.bytes))
            .is_some()
        {
            bail!("input file {path:?} was given twice");
        }
    }
    Ok(staged)
}

pub(super) struct Scripts {
    pub bootstrap_name: &'static str,
    pub bootstrap: &'static str,
    pub code_name: &'static str,
}

pub(super) async fn stage(
    root: &Path,
    request: &SandboxRequest,
    network: &GuestNetwork,
    scripts: &Scripts,
) -> Result<()> {
    let write = |name: &'static str, data: Vec<u8>| async move {
        tokio::fs::write(root.join(name), data)
            .await
            .with_context(|| format!("write {name}"))
    };
    write(scripts.bootstrap_name, scripts.bootstrap.as_bytes().to_vec()).await?;
    write(scripts.code_name, request.code.clone().into_bytes()).await?;
    write("inputs.json", serde_json::to_vec(&request.inputs)?).await?;
    write(
        "config.json",
        serde_json::to_vec(&serde_json::json!({
            "packages": [],
            "package_allowlist": [],
            "network_enabled": network.is_enabled(),
            "allowed_hosts": [],
            "max_output_bytes": request.limits.max_output_bytes,
            "workdir": "/flow/out",
        }))?,
    )
    .await?;

    tokio::fs::create_dir_all(root.join("out"))
        .await
        .context("create out/")?;
    for file in &request.files {
        let target = root.join("in").join(validate_relative_path(&file.path)?);
        if let Some(parent) = target.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .with_context(|| format!("create {}", parent.display()))?;
        }
        tokio::fs::write(&target, &file.bytes)
            .await
            .with_context(|| format!("stage {}", file.path))?;
    }
    tokio::fs::create_dir_all(root.join("in"))
        .await
        .context("create in/")?;
    Ok(())
}

pub(super) async fn collect(
    root: &Path,
    staged: &HashMap<String, String>,
    limits: &SandboxLimits,
) -> Collected {
    let mut collected = Collected {
        files: Vec::new(),
        truncated: false,
    };
    let mut budget = limits.max_result_bytes;
    for area in ["out", "in"] {
        for (relative, path, len) in list_regular_files(&root.join(area)).await {
            if collected.files.len() >= limits.max_result_files || len > budget {
                collected.truncated = true;
                continue;
            }
            let Ok(bytes) = tokio::fs::read(&path).await else {
                continue;
            };
            if area == "in"
                && staged
                    .get(&relative)
                    .is_some_and(|h| *h == sha256_hex(&bytes))
            {
                continue;
            }
            budget -= bytes.len() as u64;
            collected.files.push(SandboxFile {
                path: format!("{area}/{relative}"),
                bytes,
            });
        }
    }
    collected
}

async fn list_regular_files(base: &Path) -> Vec<(String, PathBuf, u64)> {
    let mut found = Vec::new();
    let mut stack = vec![(base.to_path_buf(), String::new(), 0usize)];
    while let Some((dir, prefix, depth)) = stack.pop() {
        let Ok(mut entries) = tokio::fs::read_dir(&dir).await else {
            continue;
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let Ok(meta) = tokio::fs::symlink_metadata(entry.path()).await else {
                continue;
            };
            let name = entry.file_name().to_string_lossy().into_owned();
            let relative = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            if meta.is_dir() && depth < MAX_WALK_DEPTH {
                stack.push((entry.path(), relative, depth + 1));
            } else if meta.is_file() {
                found.push((relative, entry.path(), meta.len()));
            }
        }
    }
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found
}
