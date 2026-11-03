mod backend;
#[cfg(feature = "code")]
mod catalog;

use std::net::{IpAddr, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

use flow_like_types::bot_message::WorkspaceHandle;
use flow_like_types::egress::is_public_ip;
use flow_like_types::{Bytes, create_id};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::env::{Environment, ToolEnv};
use crate::error::{ToolError, ToolResult, code};
use crate::files::{FileRef, handle_for, resolve};
use crate::layout::{CODE_DIR, normalize_path, sanitize_name};
use crate::outcome::{ToolImage, ToolOutput, shrink_string};
use crate::spec::{Approval, ToolSpec, schema_of};

pub use backend::{LanguageSupport, RunRequest, RunResponse, RunStatus, SandboxBackend};
#[cfg(feature = "code")]
pub use catalog::CatalogSandbox;

pub const CODE_RUN: &str = "code_run";

const MAX_RESULT_IMAGES: usize = 4;
const MAX_RESULT_IMAGE_BYTES: usize = 4 * 1024 * 1024;

pub(crate) fn default_backend() -> Option<Arc<dyn SandboxBackend>> {
    #[cfg(feature = "code")]
    return catalog::default_backend();
    #[cfg(not(feature = "code"))]
    None
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CodeRunArgs {
    /// `python` (3.12 standard library; with `packages` the scientific build) or `javascript`
    /// (QuickJS, ES2023, no Node or browser APIs).
    pub language: String,
    /// The program. It finds `inputs` (a dict) and fills `outputs` (a JSON-serialisable dict); it runs in
    /// `/flow/out` and reads the given files from `/flow/in/<name>`. Print to see results.
    pub code: String,
    /// Files to stage under `/flow/in/<file name>`: workspace paths, attachments, handles, other apps' files.
    #[serde(default)]
    pub files: Vec<FileRef>,
    /// Hosts the program may reach. None (the default) means no network at all. Needs the owner's approval.
    #[serde(default)]
    pub network_hosts: Vec<String>,
    /// Python only: library sets to load, e.g. `["numpy", "pandas"]` (also scipy, scikit-learn, matplotlib, ...). The first use downloads a large runtime.
    #[serde(default)]
    pub packages: Vec<String>,
}

pub fn specs() -> Vec<ToolSpec> {
    vec![ToolSpec {
        name: CODE_RUN,
        description: "Run a Python or JavaScript program in a sandbox (no network, no host files). Staged input files are under /flow/in, the working directory is /flow/out; files you write there (csv, json, png, ...) are saved in the workspace and returned as handles, and png/jpg/gif/webp images are shown to you. Python has `inputs`/`outputs` dicts; for numpy, pandas or matplotlib pass packages.",
        capability: Some("code_execution"),
        module: "code",
        approval: Approval::None,
        approval_args: &["network_hosts"],
        mutating: true,
        input_schema: schema_of::<CodeRunArgs>(),
    }]
}

fn language_id(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "py" | "python3" => "python".to_string(),
        "js" | "node" | "nodejs" | "quickjs" => "javascript".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
pub(crate) fn language_id_for_tests(raw: &str) -> String {
    language_id(raw)
}

fn valid_host(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']'))
}

async fn ensure_public(host: &str) -> ToolResult<()> {
    let bare = host.trim_matches(['[', ']']);
    let refused = |why: &str| {
        ToolError::refused(format!(
            "The host '{host}' {why}; on a server the sandbox may only reach public internet hosts"
        ))
    };
    if let Ok(ip) = bare.parse::<IpAddr>() {
        return if is_public_ip(ip) {
            Ok(())
        } else {
            Err(refused("is not a public address"))
        };
    }
    let lowered = bare.to_ascii_lowercase();
    if lowered == "localhost" || lowered.ends_with(".local") || lowered.ends_with(".internal") {
        return Err(refused("is an internal name"));
    }
    let lookup = lowered.clone();
    let resolved = flow_like_types::tokio::task::spawn_blocking(move || {
        (lookup.as_str(), 443)
            .to_socket_addrs()
            .map(|addresses| addresses.map(|a| a.ip()).collect::<Vec<_>>())
    })
    .await
    .map_err(|error| ToolError::new(code::FAILED, format!("Resolving '{host}' failed: {error}")))?
    .map_err(|error| ToolError::new(code::FAILED, format!("Resolving '{host}' failed: {error}")))?;
    if resolved.is_empty() || resolved.iter().any(|ip| !is_public_ip(*ip)) {
        return Err(refused("does not resolve to public addresses only"));
    }
    Ok(())
}

async fn checked_hosts(env: &dyn ToolEnv, hosts: &[String]) -> ToolResult<Vec<String>> {
    let max = env.limits().code_max_hosts;
    if hosts.len() > max {
        return Err(ToolError::invalid(format!(
            "Name at most {max} network hosts"
        )));
    }
    let mut clean: Vec<String> = Vec::new();
    for host in hosts {
        let host = host.trim().to_ascii_lowercase();
        if !valid_host(&host) {
            return Err(ToolError::invalid(format!(
                "'{host}' is not a plain host name or address (no scheme, path or spaces)"
            )));
        }
        if env.environment() == Environment::Server {
            ensure_public(&host).await?;
        }
        if !clean.contains(&host) {
            clean.push(host);
        }
    }
    Ok(clean)
}

fn checked_packages(
    supported: &[LanguageSupport],
    language: &str,
    requested: &[String],
) -> ToolResult<Vec<String>> {
    if requested.is_empty() {
        return Ok(Vec::new());
    }
    let offered: &[String] = supported
        .iter()
        .find(|s| s.id == language)
        .map(|s| s.packages.as_slice())
        .unwrap_or_default();
    if offered.is_empty() {
        return Err(ToolError::invalid(format!(
            "The {language} sandbox on this host has no packages; leave 'packages' empty"
        )));
    }
    let mut clean: Vec<String> = Vec::new();
    for name in requested {
        let name = name.trim().to_ascii_lowercase();
        if name.is_empty() || name.len() > 64 {
            return Err(ToolError::invalid(
                "A package name is empty or too long".to_string(),
            ));
        }
        if !clean.contains(&name) {
            clean.push(name);
        }
    }
    Ok(clean)
}

fn is_model_image(mime: &str) -> bool {
    matches!(
        mime,
        "image/png" | "image/jpeg" | "image/gif" | "image/webp"
    )
}

fn attach_images(output: &mut ToolOutput, files: &[(String, Bytes)]) {
    let mut left_out = 0usize;
    let mut svgs = 0usize;
    for (relative, bytes) in files {
        let mime = crate::files::guess_mime(relative);
        if mime == "image/svg+xml" {
            svgs += 1;
        } else if is_model_image(&mime) {
            if output.images.len() < MAX_RESULT_IMAGES && bytes.len() <= MAX_RESULT_IMAGE_BYTES {
                output.images.push(ToolImage {
                    label: relative.clone(),
                    mime,
                    data: bytes.clone(),
                });
            } else {
                left_out += 1;
            }
        }
    }
    if left_out > 0 {
        output.warnings.push(format!(
            "{left_out} image(s) were not shown to you (at most {MAX_RESULT_IMAGES} images of {} MiB each); they are saved as files",
            MAX_RESULT_IMAGE_BYTES / (1024 * 1024)
        ));
    }
    if svgs > 0 {
        output.warnings.push(
            "SVG files are saved but cannot be shown to you; write a png (e.g. matplotlib savefig) to see a figure"
                .to_string(),
        );
    }
}

fn staged_name(taken: &[String], name: &str) -> String {
    let name = sanitize_name(name);
    if !taken.contains(&name) {
        return name;
    }
    (2..)
        .map(|n| match name.rsplit_once('.') {
            Some((stem, ext)) => format!("{stem}-{n}.{ext}"),
            None => format!("{name}-{n}"),
        })
        .find(|candidate| !taken.contains(candidate))
        .unwrap_or(name)
}

fn trimmed(text: &str, budget: usize) -> (Value, bool) {
    if text.len() > budget {
        (json!(shrink_string(text, budget)), true)
    } else {
        (json!(text), false)
    }
}

fn failure(status: RunStatus, response: &RunResponse, budget: usize) -> ToolError {
    let (code, headline) = match status {
        RunStatus::Timeout => (code::CODE_TIMEOUT, "The program ran out of time"),
        RunStatus::OutOfMemory => (code::CODE_OUT_OF_MEMORY, "The program ran out of memory"),
        RunStatus::FuelExhausted => (
            code::CODE_FUEL_EXHAUSTED,
            "The program used up its compute budget",
        ),
        RunStatus::Unavailable => (
            code::CODE_UNAVAILABLE,
            "The code runtime is not available here",
        ),
        _ => (code::CODE_ERROR, "The program failed"),
    };
    let detail = response.error.as_deref().unwrap_or_default();
    ToolError::new(
        code,
        format!("{headline}: {detail}")
            .trim_end_matches([':', ' '])
            .to_string(),
    )
    .with_details(json!({
        "stdout": trimmed(&response.stdout, budget).0,
        "stderr": trimmed(&response.stderr, budget).0,
        "duration_ms": response.duration_ms,
    }))
}

pub async fn code_run(env: &dyn ToolEnv, args: CodeRunArgs) -> ToolResult<ToolOutput> {
    if args.code.trim().is_empty() {
        return Err(ToolError::invalid("There is no code to run"));
    }
    let sandbox = env.sandbox().ok_or_else(|| {
        ToolError::new(
            code::CODE_UNAVAILABLE,
            "This host cannot run code: the sandbox is not available here",
        )
    })?;
    let language = language_id(&args.language);
    let supported = sandbox.languages();
    if !supported.iter().any(|s| s.id == language) {
        let available: Vec<&str> = supported.iter().map(|s| s.id.as_str()).collect();
        return Err(ToolError::new(
            code::CODE_UNAVAILABLE,
            format!(
                "Language '{}' cannot run here; available: {}",
                args.language,
                if available.is_empty() {
                    "none".to_string()
                } else {
                    available.join(", ")
                }
            ),
        ));
    }
    let packages = checked_packages(&supported, &language, &args.packages)?;
    let limits = env.limits().clone();
    let hosts = checked_hosts(env, &args.network_hosts).await?;

    let mut staged: Vec<(String, Bytes)> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    let mut classification = None;
    let mut total = 0u64;
    for reference in &args.files {
        let file = resolve(env, reference, limits.code_max_input_bytes).await?;
        total += file.bytes.len() as u64;
        if total > limits.code_max_input_bytes {
            return Err(ToolError::new(
                code::TOO_LARGE,
                format!(
                    "The input files add up to more than {} bytes",
                    limits.code_max_input_bytes
                ),
            ));
        }
        classification = classification.max(file.classification);
        let name = staged_name(&names, &file.name);
        names.push(name.clone());
        staged.push((name, file.bytes));
    }
    let staged_paths: Vec<Value> = args
        .files
        .iter()
        .zip(&names)
        .map(|(reference, name)| json!({ "file": reference.label(), "path": format!("/flow/in/{name}") }))
        .collect();

    let response = sandbox
        .run(RunRequest {
            language: language.clone(),
            code: args.code.clone(),
            files: staged,
            network_hosts: hosts,
            wall_time: Duration::from_secs(limits.code_wall_time_secs),
            memory_bytes: limits.code_memory_bytes,
            max_input_bytes: limits.code_max_input_bytes,
            packages,
        })
        .await;

    let budget = limits.inline_cap_bytes.saturating_sub(2048).max(1024) / 3;
    if response.status != RunStatus::Ok {
        return Err(failure(response.status, &response, budget));
    }

    let run = format!("run-{}", create_id().chars().take(8).collect::<String>());
    let mut output = ToolOutput::new(Value::Null).classified(classification);
    let mut saved: Vec<WorkspaceHandle> = Vec::new();
    if let Some(workspace) = env.scope().workspace.clone() {
        let mut keep = vec![(
            format!("script.{}", extension_of(&language)),
            Bytes::from(args.code.clone().into_bytes()),
        )];
        keep.extend(response.files.iter().cloned());
        for (relative, bytes) in keep {
            let Ok(path) = normalize_path(&format!("{CODE_DIR}/{run}/{relative}")) else {
                output.warnings.push(format!(
                    "The output '{relative}' has an unusable path and was not saved"
                ));
                continue;
            };
            let mime = crate::files::guess_mime(&path);
            env.write_workspace(&path, bytes.clone(), &mime).await?;
            saved.push(handle_for(&workspace, &path, &bytes, &mime));
        }
    } else if !response.files.is_empty() {
        output.warnings.push(
            "The bot has no workspace, so the files the program wrote were not saved".to_string(),
        );
    }
    attach_images(&mut output, &response.files);
    let (stdout, cut_out) = trimmed(&response.stdout, budget);
    let (stderr, cut_err) = trimmed(&response.stderr, budget);
    if response.truncated {
        output
            .warnings
            .push("The sandbox cut some output or files at its limits".to_string());
    }
    output.truncated = cut_out || cut_err || response.truncated;
    output.result = json!({
        "run": run,
        "language": language,
        "outputs": response.outputs,
        "stdout": stdout,
        "stderr": stderr,
        "files": saved,
        "inputs": staged_paths,
        "duration_ms": response.duration_ms,
    });
    output.saved = saved;
    Ok(output)
}

fn extension_of(language: &str) -> &'static str {
    match language {
        "python" => "py",
        "javascript" => "js",
        _ => "txt",
    }
}
