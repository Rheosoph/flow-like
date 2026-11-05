mod backend;
mod local;
mod sync;

use std::time::Duration;

use flow_like_types::create_id;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::env::ToolEnv;
use crate::error::{ToolError, ToolResult, code};
use crate::outcome::ToolOutput;
use crate::secret_text::Scrubber;
use crate::spec::{Approval, ToolSpec, schema_of};

pub use backend::{ShellBackend, ShellRequest, ShellResponse};
pub use local::{LocalShell, bubblewrap_args, scrubbed_env};
pub use sync::{Skipped, confine_cwd, mirrored};

#[cfg(test)]
mod tests;

pub const SHELL_RUN: &str = "shell_run";

const MAX_ARGS: usize = 64;
const MAX_ARG_BYTES: usize = 8 * 1024;
const MAX_STDIN_BYTES: usize = 256 * 1024;
const MAX_SECRETS: usize = 8;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ShellRunArgs {
    /// The program to run (`git`, `python3`, `./build.sh`), not a command line. With `shell` true: the
    /// command line for `sh -c`.
    pub command: String,
    /// Arguments, one per entry; no quoting needed. Not allowed together with `shell`.
    #[serde(default)]
    pub args: Vec<String>,
    /// Run `command` through a shell (pipes, redirects, `&&`). Needs its own approval; prefer `args`.
    #[serde(default)]
    pub shell: bool,
    /// Working directory inside the bot's working directory (created when missing). Default: its root.
    #[serde(default)]
    pub cwd: Option<String>,
    /// Text for the program's standard input.
    #[serde(default)]
    pub stdin: Option<String>,
    /// Wall time limit in seconds (default 60, at most 170). The process tree is killed after it.
    #[serde(default)]
    pub timeout_s: Option<u64>,
    /// Names of the bot's secrets to pass as environment variables of the same name. Nothing else
    /// of the host's environment reaches the program.
    #[serde(default)]
    pub secrets: Vec<String>,
}

pub fn specs() -> Vec<ToolSpec> {
    vec![ToolSpec {
        name: SHELL_RUN,
        description: "Run a program on this device in the bot's working directory, a copy of its workspace files. Files the program creates or changes there are saved back to the workspace and returned as handles. Output is capped (start and end are kept); the exit code, duration and whether it timed out are returned. The owner approves each new program.",
        capability: Some("command_line"),
        module: "shell",
        approval: Approval::Ask,
        approval_args: &[],
        mutating: true,
        input_schema: schema_of::<ShellRunArgs>(),
    }]
}

/// The lowercase program name a call is approved under: `git` for `/usr/bin/git`, `sh` for any
/// `shell: true` call.
pub fn program_name(command: &str, shell: bool) -> Option<String> {
    if shell {
        return Some("sh".to_string());
    }
    let base = command.trim().rsplit(['/', '\\']).next()?.to_lowercase();
    let base = ["exe", "cmd", "bat", "com"]
        .iter()
        .find_map(|extension| base.strip_suffix(&format!(".{extension}")))
        .unwrap_or(&base);
    (!base.is_empty()).then(|| base.to_string())
}

/// The grant scope of one call: `command:<program>`.
pub fn grant_scope(args: &Value) -> Option<String> {
    let command = args.get("command")?.as_str()?;
    let shell = args.get("shell").and_then(Value::as_bool).unwrap_or(false);
    program_name(command, shell).map(|name| format!("command:{name}"))
}

fn valid_secret_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && name.len() <= 64
}

fn check_args(args: &ShellRunArgs) -> ToolResult<()> {
    if args.command.trim().is_empty() || args.command.contains('\0') {
        return Err(ToolError::invalid("There is no command to run"));
    }
    if args.shell && !args.args.is_empty() {
        return Err(ToolError::invalid(
            "With shell true the whole command line goes in 'command'; leave 'args' empty",
        ));
    }
    if !args.shell && args.command.trim().contains(char::is_whitespace) {
        return Err(ToolError::invalid(
            "'command' is a program name, not a command line: pass the arguments in 'args', or set shell true",
        ));
    }
    if args.args.len() > MAX_ARGS
        || args
            .args
            .iter()
            .any(|arg| arg.len() > MAX_ARG_BYTES || arg.contains('\0'))
    {
        return Err(ToolError::invalid(format!(
            "Pass at most {MAX_ARGS} arguments of at most {MAX_ARG_BYTES} bytes each"
        )));
    }
    if args
        .stdin
        .as_ref()
        .is_some_and(|stdin| stdin.len() > MAX_STDIN_BYTES)
    {
        return Err(ToolError::invalid(format!(
            "stdin is limited to {MAX_STDIN_BYTES} bytes; write larger input to a file"
        )));
    }
    if args.secrets.len() > MAX_SECRETS || args.secrets.iter().any(|n| !valid_secret_name(n)) {
        return Err(ToolError::invalid(format!(
            "Name at most {MAX_SECRETS} secrets, each a plain identifier (letters, digits, '_')"
        )));
    }
    Ok(())
}

async fn secret_env(env: &dyn ToolEnv, names: &[String]) -> ToolResult<Vec<(String, String)>> {
    let mut found = Vec::new();
    for name in names {
        let value = env
            .secret(name)
            .await?
            .ok_or_else(|| {
                ToolError::not_found(format!(
                    "The bot has no secret '{name}'; call secret_list for the stored names, or secret_request to ask the owner for it"
                ))
            })?;
        found.push((name.clone(), value));
    }
    Ok(found)
}

/// Runs the command with the named secrets in its environment; whatever it prints of them comes
/// back as `{{secret:NAME}}`.
pub async fn shell_run(env: &dyn ToolEnv, args: ShellRunArgs) -> ToolResult<ToolOutput> {
    check_args(&args)?;
    let secrets = secret_env(env, &args.secrets).await?;
    let mut scrubber = Scrubber::default();
    for (name, value) in &secrets {
        scrubber.add(name, value.clone());
    }
    match run_command(env, args, secrets).await {
        Ok(mut output) => {
            scrubber.scrub_value(&mut output.result);
            for warning in &mut output.warnings {
                *warning = scrubber.scrub(warning);
            }
            Ok(output)
        }
        Err(error) => Err(scrubber.scrub_error(error)),
    }
}

async fn run_command(
    env: &dyn ToolEnv,
    args: ShellRunArgs,
    secrets: Vec<(String, String)>,
) -> ToolResult<ToolOutput> {
    let backend = env.shell().ok_or_else(|| {
        ToolError::new(
            code::SHELL_UNAVAILABLE,
            "This host cannot run commands: only the desktop app and a bot's own device can",
        )
    })?;
    let limits = env.limits().clone();
    let root = backend.root();
    let cwd = confine_cwd(&root, args.cwd.as_deref())?;
    let wall = args
        .timeout_s
        .unwrap_or(limits.shell_wall_time_secs)
        .clamp(1, limits.shell_max_wall_time_secs);

    sync::pull(env, &root).await?;
    let before = sync::snapshot(&root).await;
    let response = backend
        .run(
            ShellRequest {
                program: args.command.trim().to_string(),
                args: args.args.clone(),
                script: args.shell,
                cwd: cwd.clone(),
                stdin: args.stdin.clone(),
                wall_time: Duration::from_secs(wall),
                output_bytes: limits.shell_output_bytes,
                env: secrets,
            },
            &|| env.is_cancelled(),
        )
        .await;
    if let Some(error) = &response.spawn_error {
        return Err(ToolError::new(code::FAILED, error.clone()));
    }
    let after = sync::snapshot(&root).await;
    let pushed = sync::push(env, &root, &before, &after).await;
    if response.cancelled {
        return Err(ToolError::new(code::CANCELLED, "The command was cancelled").with_details(
            json!({ "stdout": response.stdout, "stderr": response.stderr, "files": pushed.files }),
        ));
    }

    let run = format!("run-{}", create_id().chars().take(8).collect::<String>());
    let mut output = ToolOutput::new(Value::Null);
    if response.timed_out {
        output.warnings.push(format!(
            "The command ran past {wall} s and its process tree was killed"
        ));
    }
    if response.sandboxed {
        output
            .warnings
            .push("Ran sandboxed: only the working directory is writable".to_string());
    }
    output.truncated = response.stdout_omitted + response.stderr_omitted > 0;
    output.result = json!({
        "run": run,
        "command": args.command,
        "args": args.args,
        "shell": args.shell,
        "cwd": cwd,
        "exit_code": response.exit_code,
        "timed_out": response.timed_out,
        "stdout": response.stdout,
        "stderr": response.stderr,
        "stdout_omitted_bytes": response.stdout_omitted,
        "stderr_omitted_bytes": response.stderr_omitted,
        "duration_ms": response.duration_ms,
        "files": pushed.files,
        "files_not_saved": pushed.skipped,
    });
    output.saved = pushed.files;
    Ok(output)
}
