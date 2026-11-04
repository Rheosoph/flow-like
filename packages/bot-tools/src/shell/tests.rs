use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use serde_json::{Value, json};

use super::*;
use crate::testing::{FakeEnv, WORKSPACE};
use crate::{ToolOutcome, run};

struct Rig {
    env: FakeEnv,
    dir: tempfile::TempDir,
}

impl Rig {
    async fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let env = FakeEnv::new().await;
        *env.shell.lock().expect("shell") =
            Some(Arc::new(LocalShell::new(dir.path().join("work"))));
        Self { env, dir }
    }

    fn root(&self) -> std::path::PathBuf {
        self.dir.path().join("work")
    }

    async fn call(&self, args: Value) -> ToolOutcome {
        run(SHELL_RUN, args, &self.env).await
    }

    async fn sh(&self, script: &str) -> ToolOutcome {
        self.call(json!({ "command": script, "shell": true })).await
    }
}

fn result(outcome: &ToolOutcome) -> &Value {
    assert!(outcome.is_ok(), "expected ok, got {:?}", outcome.error);
    &outcome.result
}

fn error_code(outcome: &ToolOutcome) -> &'static str {
    outcome.error.as_ref().expect("an error").code
}

#[cfg(unix)]
mod unix {
    use super::*;

    #[tokio::test]
    async fn a_run_captures_both_streams_and_the_exit_code() {
        let rig = Rig::new().await;
        let outcome = rig
            .call(json!({ "command": "sh", "args": ["-c", "echo out; echo err >&2; exit 3"] }))
            .await;
        let result = result(&outcome);
        assert_eq!(result["exit_code"], 3);
        assert_eq!(result["stdout"], "out\n");
        assert_eq!(result["stderr"], "err\n");
        assert_eq!(result["timed_out"], false);
        assert!(result["duration_ms"].is_u64());
    }

    #[tokio::test]
    async fn stdin_reaches_the_program_and_zero_is_a_success_code() {
        let rig = Rig::new().await;
        let outcome = rig
            .call(json!({ "command": "cat", "stdin": "hello" }))
            .await;
        assert_eq!(result(&outcome)["stdout"], "hello");
        assert_eq!(result(&outcome)["exit_code"], 0);
    }

    #[tokio::test]
    async fn a_missing_program_is_an_error_not_an_exit_code() {
        let rig = Rig::new().await;
        let outcome = rig.call(json!({ "command": "no-such-program-xyz" })).await;
        assert_eq!(error_code(&outcome), "failed");
        assert!(
            outcome
                .error
                .as_ref()
                .unwrap()
                .message
                .contains("no-such-program-xyz")
        );
    }

    fn alive(pid: i32) -> bool {
        unsafe { libc::kill(pid, 0) == 0 }
    }

    #[tokio::test]
    async fn a_timeout_kills_the_whole_process_tree() {
        let rig = Rig::new().await;
        let started = std::time::Instant::now();
        let outcome = rig
            .call(json!({
                "command": "sleep 60 & echo $! > child.pid; sleep 60",
                "shell": true,
                "timeout_s": 1
            }))
            .await;
        assert_eq!(result(&outcome)["timed_out"], true);
        assert!(started.elapsed() < Duration::from_secs(20));
        let pid: i32 = std::fs::read_to_string(rig.root().join("child.pid"))
            .expect("pid file")
            .trim()
            .parse()
            .expect("pid");
        let mut gone = false;
        for _ in 0..50 {
            if !alive(pid) {
                gone = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(gone, "the background child {pid} survived the timeout");
    }

    #[tokio::test]
    async fn cancelling_stops_the_command_and_reports_cancelled() {
        let rig = Rig::new().await;
        let started = std::time::Instant::now();
        let (outcome, ()) = tokio::join!(rig.sh("sleep 60"), async {
            tokio::time::sleep(Duration::from_millis(400)).await;
            rig.env.cancelled.store(true, Ordering::SeqCst);
        });
        assert_eq!(error_code(&outcome), "cancelled");
        assert!(started.elapsed() < Duration::from_secs(20));
    }

    #[tokio::test]
    async fn the_environment_is_scrubbed_and_secrets_pass_only_by_name() {
        let rig = Rig::new().await;
        rig.env.set_secret("API_TOKEN", "s3cret");
        let outcome = rig
            .call(json!({ "command": "env", "secrets": ["API_TOKEN"] }))
            .await;
        let result = result(&outcome);
        let printed = result["stdout"].as_str().unwrap();
        let names: Vec<&str> = printed
            .lines()
            .filter_map(|line| line.split_once('=').map(|(name, _)| name))
            .collect();
        assert!(
            std::env::vars().any(|(name, _)| name.starts_with("CARGO_")),
            "the test process should carry CARGO_ variables"
        );
        assert!(
            names.iter().all(|name| !name.starts_with("CARGO_")),
            "leaked: {names:?}"
        );
        assert!(printed.contains("API_TOKEN={{secret:API_TOKEN}}"));
        assert!(!printed.contains("s3cret"));
        assert!(
            printed.contains(&format!(
                "HOME={}",
                rig.root().canonicalize().unwrap().display()
            )) || printed.contains(&format!("HOME={}", rig.root().display()))
        );

        let without = rig.call(json!({ "command": "env" })).await;
        assert!(!result_text(&without).contains("s3cret"));
    }

    fn result_text(outcome: &ToolOutcome) -> String {
        result(outcome)["stdout"].as_str().unwrap().to_string()
    }

    #[tokio::test]
    async fn a_secret_reaches_the_program_but_never_its_printed_output() {
        let rig = Rig::new().await;
        rig.env.set_secret("API_TOKEN", "tok/en+value=1");
        let script = r#"test "$API_TOKEN" = 'tok/en+value=1' && echo match; echo "out $API_TOKEN"; echo "err $API_TOKEN" >&2; printf %s "$API_TOKEN" | base64; exit 7"#;
        let outcome = rig
            .call(json!({ "command": "sh", "args": ["-c", script], "secrets": ["API_TOKEN"] }))
            .await;
        let result = result(&outcome);
        let stdout = result["stdout"].as_str().unwrap();
        assert!(
            stdout.starts_with("match\nout {{secret:API_TOKEN}}\n"),
            "{stdout}"
        );
        assert_eq!(result["stderr"], "err {{secret:API_TOKEN}}\n");
        let text = result.to_string();
        assert!(!text.contains("tok/en+value=1"));
        assert!(!text.contains("dG9rL2VuK3ZhbHVlPTE="));
    }

    #[tokio::test]
    async fn an_unknown_secret_is_refused_before_anything_runs() {
        let rig = Rig::new().await;
        let outcome = rig
            .call(json!({ "command": "env", "secrets": ["NOPE"] }))
            .await;
        assert_eq!(error_code(&outcome), "not_found");
    }

    #[tokio::test]
    async fn the_working_directory_stays_inside_the_bot_directory() {
        let rig = Rig::new().await;
        for bad in ["..", "../x", "a/../../x", "/etc", "C:/x", "a\\b"] {
            let outcome = rig.call(json!({ "command": "pwd", "cwd": bad })).await;
            assert!(
                matches!(error_code(&outcome), "scope_refused" | "invalid_args"),
                "{bad} -> {:?}",
                outcome.error
            );
        }
        let outcome = rig
            .call(json!({ "command": "pwd", "cwd": "sub/dir" }))
            .await;
        let printed = result(&outcome)["stdout"]
            .as_str()
            .unwrap()
            .trim()
            .to_string();
        assert!(printed.ends_with("work/sub/dir"), "{printed}");
        assert!(rig.root().join("sub/dir").is_dir());
    }

    #[tokio::test]
    async fn a_symlink_cannot_lead_the_working_directory_out() {
        let rig = Rig::new().await;
        let outside = rig.dir.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::create_dir_all(rig.root()).unwrap();
        std::os::unix::fs::symlink(&outside, rig.root().join("link")).unwrap();
        let outcome = rig.call(json!({ "command": "pwd", "cwd": "link" })).await;
        assert_eq!(error_code(&outcome), "scope_refused");
        let nested = rig
            .call(json!({ "command": "pwd", "cwd": "link/deeper" }))
            .await;
        assert_eq!(error_code(&nested), "scope_refused");
    }

    #[tokio::test]
    async fn output_is_capped_keeping_the_start_and_the_end() {
        let rig = Rig::new().await;
        let outcome = rig
            .sh("i=0; while [ $i -lt 20000 ]; do echo line-$i; i=$((i+1)); done")
            .await;
        let result = result(&outcome);
        let stdout = result["stdout"].as_str().unwrap();
        let cap = rig.env.limits.shell_output_bytes;
        assert!(stdout.len() < cap + 100, "{} bytes", stdout.len());
        assert!(stdout.starts_with("line-0\n"));
        assert!(stdout.trim_end().ends_with("line-19999"));
        assert!(stdout.contains("bytes omitted"));
        assert!(result["stdout_omitted_bytes"].as_u64().unwrap() > 100_000);
        assert!(outcome.truncated);
    }

    #[tokio::test]
    async fn changed_files_are_saved_to_the_workspace_as_handles() {
        let rig = Rig::new().await;
        rig.env
            .put(WORKSPACE, "data.csv", b"a,b\n1,2\n", "text/csv");
        rig.env.put(
            WORKSPACE,
            "bot/notes/secret.md",
            b"internal",
            "text/markdown",
        );
        let before_writes = rig.env.write_count();
        let outcome = rig
            .sh("cat data.csv > copy.csv; echo done > report.txt; mkdir -p out; echo x > out/a.txt; echo y > bot/notes/secret.md 2>/dev/null; true")
            .await;
        let result = result(&outcome);
        let mut paths: Vec<String> = result["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["path"].as_str().unwrap().to_string())
            .collect();
        paths.sort();
        assert_eq!(paths, ["copy.csv", "out/a.txt", "report.txt"]);
        assert_eq!(outcome.saved.len(), 3);
        assert_eq!(outcome.saved[0].app_id, WORKSPACE);
        assert_eq!(rig.env.write_count() - before_writes, 3);
        let files = rig.env.files.lock().unwrap();
        let stored = &files[&(WORKSPACE.to_string(), "copy.csv".to_string())];
        assert_eq!(&stored.0[..], b"a,b\n1,2\n");
        assert_eq!(
            &files[&(WORKSPACE.to_string(), "bot/notes/secret.md".to_string())].0[..],
            b"internal"
        );
    }

    #[tokio::test]
    async fn an_untouched_pulled_file_is_not_written_back_but_an_edited_one_is() {
        let rig = Rig::new().await;
        rig.env.put(WORKSPACE, "keep.txt", b"keep", "text/plain");
        rig.env.put(WORKSPACE, "edit.txt", b"old", "text/plain");
        let outcome = rig.sh("echo new > edit.txt").await;
        let files = result(&outcome)["files"].as_array().unwrap().clone();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0]["path"], "edit.txt");
        assert_eq!(
            &rig.env.files.lock().unwrap()[&(WORKSPACE.to_string(), "edit.txt".to_string())].0[..],
            b"new\n"
        );
    }

    #[tokio::test]
    async fn large_files_and_symlinks_are_not_synced() {
        let mut rig = Rig::new().await;
        rig.env.limits.shell_sync_file_bytes = 1024;
        let secret = rig.dir.path().join("host-secret.txt");
        std::fs::write(&secret, "host secret").unwrap();
        let outcome = rig
            .sh(&format!(
                "head -c 4096 /dev/zero > big.bin; ln -s {} peek.txt; echo ok > small.txt",
                secret.display()
            ))
            .await;
        let result = result(&outcome);
        let saved: Vec<&str> = result["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["path"].as_str().unwrap())
            .collect();
        assert_eq!(saved, ["small.txt"]);
        let skipped = result["files_not_saved"].as_array().unwrap();
        assert!(skipped.iter().any(|s| s["path"] == "big.bin"));
        assert!(
            !rig.env
                .files
                .lock()
                .unwrap()
                .keys()
                .any(|(_, path)| path == "peek.txt")
        );
    }

    #[tokio::test]
    async fn workspace_files_appear_in_the_working_directory() {
        let rig = Rig::new().await;
        rig.env
            .put(WORKSPACE, "notes/todo.txt", b"buy milk", "text/plain");
        let outcome = rig
            .call(json!({ "command": "cat", "args": ["notes/todo.txt"] }))
            .await;
        assert_eq!(result(&outcome)["stdout"], "buy milk");
        assert!(result(&outcome)["files"].as_array().unwrap().is_empty());
    }
}

#[tokio::test]
async fn a_shell_call_cannot_carry_separate_arguments_and_a_program_has_no_spaces() {
    let rig = Rig::new().await;
    let both = rig
        .call(json!({ "command": "ls", "shell": true, "args": ["-l"] }))
        .await;
    assert_eq!(error_code(&both), "invalid_args");
    let spaced = rig.call(json!({ "command": "ls -l" })).await;
    assert_eq!(error_code(&spaced), "invalid_args");
    let empty = rig.call(json!({ "command": "  " })).await;
    assert_eq!(error_code(&empty), "invalid_args");
    let bad_secret = rig
        .call(json!({ "command": "ls", "secrets": ["A-B"] }))
        .await;
    assert_eq!(error_code(&bad_secret), "invalid_args");
}

#[tokio::test]
async fn a_host_without_a_shell_backend_refuses() {
    let env = FakeEnv::new().await;
    let outcome = run(SHELL_RUN, json!({ "command": "ls" }), &env).await;
    assert_eq!(error_code(&outcome), "shell_unavailable");
}

#[test]
fn every_call_is_approved_under_its_program() {
    let scope = |args: Value| grant_scope(&args);
    assert_eq!(
        scope(json!({ "command": "git" })).as_deref(),
        Some("command:git")
    );
    assert_eq!(
        scope(json!({ "command": "/usr/bin/Git" })).as_deref(),
        Some("command:git")
    );
    assert_eq!(
        scope(json!({ "command": "C:\\tools\\node.EXE" })).as_deref(),
        Some("command:node")
    );
    assert_eq!(
        scope(json!({ "command": "rm -rf /", "shell": true })).as_deref(),
        Some("command:sh")
    );
    assert_eq!(scope(json!({ "command": "" })), None);
    assert_ne!(
        scope(json!({ "command": "git" })),
        scope(json!({ "command": "curl" }))
    );
}

#[test]
fn the_tool_asks_and_needs_the_command_line_capability() {
    let spec = crate::spec(SHELL_RUN).expect("spec");
    assert_eq!(spec.approval, crate::Approval::Ask);
    assert_eq!(spec.capability, Some("command_line"));
    assert!(spec.mutating);
}

#[test]
fn the_scrubbed_environment_keeps_only_the_allow_list() {
    let parent = [
        ("PATH", "/usr/bin"),
        ("path", "dup"),
        ("AWS_SECRET_ACCESS_KEY", "k"),
        ("GITHUB_TOKEN", "t"),
        ("HOME", "/Users/me"),
        ("LANG", "C"),
    ]
    .map(|(k, v)| (k.to_string(), v.to_string()));
    let env = scrubbed_env(
        parent,
        Path::new("/work"),
        &[("MY_SECRET".to_string(), "v".to_string())],
    );
    let get = |name: &str| env.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str());
    assert_eq!(get("HOME"), Some("/work"));
    assert_eq!(get("MY_SECRET"), Some("v"));
    assert_eq!(get("LANG"), Some("C"));
    assert!(get("AWS_SECRET_ACCESS_KEY").is_none() && get("GITHUB_TOKEN").is_none());
    assert!(
        env.iter()
            .all(|(n, _)| !n.eq_ignore_ascii_case("home") || n == "HOME")
    );
}

#[test]
fn bubblewrap_binds_only_the_working_directory_writable() {
    let args = bubblewrap_args(Path::new("/data/bot"), Path::new("/data/bot/sub"));
    let joined = args.join(" ");
    assert!(joined.starts_with("--ro-bind / /"));
    assert!(joined.contains("--bind /data/bot /data/bot"));
    assert_eq!(args.iter().filter(|a| *a == "--bind").count(), 1);
    assert!(joined.contains("--chdir /data/bot/sub"));
    assert_eq!(args.last().map(String::as_str), Some("--"));
}
