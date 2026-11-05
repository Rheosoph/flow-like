//! The playground worker: one process per bot, so each has its own X connection and input
//! queue. The agent starts it with [`ProcessSpawner`]; the worker itself is [`run_worker`].

use std::path::{Path, PathBuf};
use std::process::Stdio;

use async_trait::async_trait;
use flow_like_types::{Result, anyhow};
use tokio::process::{Child, Command};

use crate::display::{DisplayKind, VirtualDisplay};
use crate::local::{LocalPlayground, bot_dir};
use crate::rpc::{Spawner, Transport, serve};

pub const WORKER_SUBCOMMAND: &str = "playground-worker";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkerArgs {
    pub bot: String,
    pub data_dir: PathBuf,
    pub display: Option<String>,
    pub headful: bool,
}

impl WorkerArgs {
    /// The command line after the program name: `playground-worker --bot <id> --data-dir <dir> [--display :N] [--headful]`.
    pub fn to_cli(&self) -> Vec<String> {
        let mut cli = vec![
            WORKER_SUBCOMMAND.to_string(),
            "--bot".into(),
            self.bot.clone(),
            "--data-dir".into(),
            self.data_dir.display().to_string(),
        ];
        if let Some(display) = &self.display {
            cli.push("--display".into());
            cli.push(display.clone());
        }
        if self.headful {
            cli.push("--headful".into());
        }
        cli
    }
}

/// Serves the bot's playground on stdin/stdout until the agent closes the pipe.
pub async fn run_worker(args: WorkerArgs) -> Result<()> {
    let playground = LocalPlayground::new(&args.data_dir, &args.bot, !args.headful);
    serve(tokio::io::stdin(), tokio::io::stdout(), playground).await;
    Ok(())
}

pub fn worker_command(program: &Path, args: &WorkerArgs, env: &[(String, String)]) -> Command {
    let mut command = Command::new(program);
    command
        .args(args.to_cli())
        .envs(env.iter().map(|(key, value)| (key, value)))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);
    if args.display.is_some() {
        command.env_remove("WAYLAND_DISPLAY");
    }
    #[cfg(unix)]
    command.process_group(0);
    command
}

/// Ends the worker's whole process group, then its virtual display.
struct WorkerGuard {
    pid: Option<u32>,
    _child: Child,
    _display: Option<VirtualDisplay>,
}

impl Drop for WorkerGuard {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.pid {
            unsafe {
                libc::killpg(pid as libc::pid_t, libc::SIGKILL);
            }
        }
        #[cfg(not(unix))]
        let _ = self.pid;
    }
}

pub struct ProcessSpawner {
    pub program: PathBuf,
    pub data_dir: PathBuf,
    pub bot_id: String,
    pub display: DisplayKind,
}

#[async_trait]
impl Spawner for ProcessSpawner {
    async fn spawn(&self) -> Result<Transport> {
        let virtual_display = match self.display {
            DisplayKind::Virtual => Some(
                VirtualDisplay::start(&bot_dir(&self.data_dir, &self.bot_id).join("x11")).await?,
            ),
            _ => None,
        };
        let args = WorkerArgs {
            bot: self.bot_id.clone(),
            data_dir: self.data_dir.clone(),
            display: virtual_display.as_ref().map(|d| format!(":{}", d.number)),
            headful: virtual_display.is_some(),
        };
        let env = virtual_display
            .as_ref()
            .map(VirtualDisplay::env)
            .unwrap_or_default();
        let mut child = worker_command(&self.program, &args, &env)
            .spawn()
            .map_err(|e| {
                anyhow!(
                    "Failed to start the playground worker {} for bot {}: {e}",
                    self.program.display(),
                    self.bot_id
                )
            })?;
        let (stdin, stdout) =
            child.stdin.take().zip(child.stdout.take()).ok_or_else(|| {
                anyhow!("The playground worker for bot {} has no pipes", self.bot_id)
            })?;
        Ok(Transport {
            reader: Box::new(stdout),
            writer: Box::new(stdin),
            guard: Box::new(GuardBox(std::sync::Mutex::new(WorkerGuard {
                pid: child.id(),
                _child: child,
                _display: virtual_display,
            }))),
        })
    }
}

#[allow(dead_code)]
struct GuardBox(std::sync::Mutex<WorkerGuard>);

#[cfg(test)]
mod tests {
    use super::*;

    fn args(display: Option<&str>, headful: bool) -> WorkerArgs {
        WorkerArgs {
            bot: "bot-1".into(),
            data_dir: PathBuf::from("/data"),
            display: display.map(str::to_string),
            headful,
        }
    }

    #[test]
    fn the_worker_command_line_names_the_bot_the_data_dir_and_the_display() {
        assert_eq!(
            args(None, false).to_cli(),
            ["playground-worker", "--bot", "bot-1", "--data-dir", "/data"]
        );
        assert_eq!(
            args(Some(":91"), true).to_cli(),
            [
                "playground-worker",
                "--bot",
                "bot-1",
                "--data-dir",
                "/data",
                "--display",
                ":91",
                "--headful"
            ]
        );
    }

    #[test]
    fn a_virtual_worker_gets_its_display_and_loses_wayland() {
        let env = vec![("DISPLAY".to_string(), ":91".to_string())];
        let command = worker_command(Path::new("/bin/agent"), &args(Some(":91"), true), &env);
        let std = command.as_std();
        let envs: Vec<_> = std.get_envs().collect();
        assert!(envs.contains(&(
            std::ffi::OsStr::new("DISPLAY"),
            Some(std::ffi::OsStr::new(":91"))
        )));
        assert!(envs.contains(&(std::ffi::OsStr::new("WAYLAND_DISPLAY"), None)));
        assert_eq!(std.get_program(), "/bin/agent");
    }

    #[test]
    fn a_session_worker_keeps_the_environment_it_inherits() {
        let command = worker_command(Path::new("/bin/agent"), &args(None, false), &[]);
        assert_eq!(command.as_std().get_envs().count(), 0);
    }
}
