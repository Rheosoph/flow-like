use super::*;
use std::{
    ffi::CStr,
    fs::{DirBuilder, File},
    io::ErrorKind,
    os::unix::{
        ffi::OsStrExt,
        fs::{DirBuilderExt, MetadataExt},
    },
    process::Stdio,
    time::Duration,
};
use tokio::{io::AsyncReadExt, process::Command};

struct User {
    uid: u32,
    home: PathBuf,
}

impl User {
    fn current() -> Result<Self> {
        // SAFETY: geteuid has no arguments or preconditions.
        let uid = unsafe { libc::geteuid() };
        ensure!(
            uid != 0,
            "Install a LaunchAgent as the device owner, not root"
        );
        let mut entry = std::mem::MaybeUninit::<libc::passwd>::uninit();
        let mut found = std::ptr::null_mut();
        let mut buffer = vec![0u8; 65_536];
        // SAFETY: all pointers refer to writable storage that remains alive until pw_dir is copied.
        let code = unsafe {
            libc::getpwuid_r(
                uid,
                entry.as_mut_ptr(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &mut found,
            )
        };
        ensure!(
            code == 0 && !found.is_null(),
            "Cannot resolve the current user's account home"
        );
        // SAFETY: successful getpwuid_r initialized the entry and its NUL-terminated pw_dir.
        let directory = unsafe { CStr::from_ptr((*found).pw_dir) };
        let home = PathBuf::from(std::ffi::OsStr::from_bytes(directory.to_bytes()));
        unit_path(&home, "Account home")?;
        Ok(Self { uid, home })
    }

    fn domain(&self) -> String {
        format!("gui/{}", self.uid)
    }
    fn target(&self) -> String {
        format!("{}/{LAUNCHD_LABEL}", self.domain())
    }

    fn directory(&self, path: &Path, private: bool) -> Result<()> {
        let metadata = std::fs::symlink_metadata(path)
            .with_context(|| format!("Inspect service directory {}", path.display()))?;
        ensure!(
            metadata.is_dir() && metadata.uid() == self.uid,
            "Service directories must be owned by the current user and must not be symlinks"
        );
        ensure!(
            metadata.mode() & (if private { 0o077 } else { 0o022 }) == 0,
            "Service directory permissions are too broad; state requires owner-only access"
        );
        Ok(())
    }

    fn plist(&self, create: bool) -> Result<PathBuf> {
        self.directory(&self.home, false)?;
        let library = self.home.join("Library");
        let agents = library.join("LaunchAgents");
        for path in [&library, &agents] {
            match std::fs::symlink_metadata(path) {
                Ok(_) => self.directory(path, false)?,
                Err(error) if error.kind() == ErrorKind::NotFound && create => {
                    DirBuilder::new().mode(0o700).create(path)?;
                    self.directory(path, false)?;
                }
                Err(error) if error.kind() == ErrorKind::NotFound => break,
                Err(error) => return Err(error).context("Inspect LaunchAgents directory"),
            }
        }
        Ok(agents.join(format!("{LAUNCHD_LABEL}.plist")))
    }

    fn executable(&self, path: &Path) -> Result<()> {
        let metadata = std::fs::symlink_metadata(path).context("Inspect service executable")?;
        ensure!(
            metadata.is_file() && metadata.mode() & 0o111 != 0,
            "Service executable must be an executable regular file, not a symlink"
        );
        ensure!(
            metadata.uid() == 0 || metadata.uid() == self.uid,
            "Service executable must belong to the current user or root"
        );
        ensure!(
            metadata.mode() & 0o022 == 0,
            "Service executable must not be writable by other users"
        );
        Ok(())
    }
}

struct Reply {
    code: Option<i32>,
    out: String,
    err: String,
}

impl Reply {
    fn success(self, action: &str) -> Result<String> {
        ensure!(
            self.code == Some(0),
            "{action} failed: {}",
            self.err.chars().take(2048).collect::<String>().trim()
        );
        Ok(self.out)
    }
}

#[async_trait::async_trait]
trait Runner: Sync {
    async fn run(&self, args: &[&str]) -> Result<Reply>;
}

struct Launchctl;
#[async_trait::async_trait]
impl Runner for Launchctl {
    async fn run(&self, args: &[&str]) -> Result<Reply> {
        let domain_only = args.first() == Some(&"print")
            && args
                .get(1)
                .is_some_and(|target| target.matches('/').count() == 1);
        tokio::time::timeout(Duration::from_secs(45), async {
            let mut child = Command::new("/bin/launchctl")
                .args(args)
                .env("LC_ALL", "C")
                .stdin(Stdio::null())
                .stdout(if domain_only {
                    Stdio::null()
                } else {
                    Stdio::piped()
                })
                .stderr(Stdio::piped())
                .kill_on_drop(true)
                .spawn()
                .context("Run launchctl; macOS with a logged-in GUI user is required")?;
            let stdout = child.stdout.take();
            let stderr = child
                .stderr
                .take()
                .context("Missing launchctl stderr pipe")?;
            let (out, err, status) = tokio::try_join!(
                async {
                    let mut bytes = Vec::new();
                    if let Some(pipe) = stdout {
                        pipe.take(131_073).read_to_end(&mut bytes).await?;
                    }
                    ensure!(
                        bytes.len() <= 131_072,
                        "launchctl output exceeds the inspection limit"
                    );
                    Ok::<_, anyhow::Error>(bytes)
                },
                async {
                    let mut bytes = Vec::new();
                    stderr.take(16_385).read_to_end(&mut bytes).await?;
                    ensure!(
                        bytes.len() <= 16_384,
                        "launchctl error output exceeds the inspection limit"
                    );
                    Ok::<_, anyhow::Error>(bytes)
                },
                async { Ok::<_, anyhow::Error>(child.wait().await?) }
            )?;
            Ok(Reply {
                code: status.code(),
                out: String::from_utf8(out)?,
                err: String::from_utf8_lossy(&err).into_owned(),
            })
        })
        .await
        .context("launchctl timed out; inspect the service before retrying")?
    }
}

async fn domain_available(runner: &impl Runner, user: &User) -> Result<bool> {
    let reply = runner.run(&["print", &user.domain()]).await?;
    if reply.code == Some(0) {
        return Ok(true);
    }
    // Do not turn a permissions or launchctl failure into permission to replace a job.
    if reply.code == Some(113) && reply.err.contains("Could not find domain") {
        return Ok(false);
    }
    reply.success("Inspect the user's GUI login session")?;
    unreachable!()
}

struct Loaded {
    process_id: Option<u32>,
}

fn loaded_reply(reply: Reply, user: &User, path: &Path) -> Result<Option<Loaded>> {
    if reply.code == Some(113)
        && reply
            .err
            .contains(&format!("Could not find service \"{LAUNCHD_LABEL}\""))
    {
        return Ok(None);
    }
    let text = reply.success("Inspect the registered LaunchAgent")?;
    // `print` is diagnostic output, not a stable API. Unknown or ambiguous formats fail closed.
    let mut lines = text.lines();
    ensure!(
        lines.next() == Some(format!("{} = {{", user.target()).as_str()),
        "Unsupported launchctl inspection format; refusing to change the service"
    );
    let mut depth = 1u32;
    let mut origin = None;
    let mut process_id = None;
    for line in lines {
        let line = line.trim();
        if depth == 1 {
            if let Some(value) = line.strip_prefix("path = ") {
                ensure!(
                    origin.replace(value).is_none(),
                    "Ambiguous launchctl service origin"
                );
            }
            if let Some(value) = line.strip_prefix("pid = ") {
                ensure!(process_id.is_none(), "Ambiguous launchctl process ID");
                let pid = value
                    .parse::<u32>()
                    .context("Unsupported launchctl process ID")?;
                ensure!(pid > 0, "Invalid launchctl process ID");
                process_id = Some(pid);
            }
        }
        if line == "}" {
            depth = depth.checked_sub(1).context("Invalid launchctl nesting")?;
        } else if line.ends_with(" = {") {
            depth = depth.checked_add(1).context("Invalid launchctl nesting")?;
        }
        if depth == 0 {
            break;
        }
    }
    ensure!(
        depth == 0 && origin.is_some_and(|value| Path::new(value) == path),
        "launchd resolves this label to another or unknown plist; refusing to manage it"
    );
    Ok(Some(Loaded { process_id }))
}

async fn loaded(runner: &impl Runner, user: &User, path: &Path) -> Result<Option<Loaded>> {
    loaded_reply(runner.run(&["print", &user.target()]).await?, user, path)
}

async fn install_with(
    runner: &impl Runner,
    user: &User,
    executable: &Path,
    state: &Path,
) -> Result<PathBuf> {
    let expected = launchd_user_plist(executable, state)?;
    user.executable(executable)?;
    user.directory(state, true)?;
    let path = user.plist(false)?;
    let existed = private_file::matches(&path, &expected)?;
    ensure!(
        domain_available(runner, user).await?,
        "Log in to this macOS account's GUI session before installation. This LaunchAgent starts after user login, including after reboot; it cannot run before login"
    );
    let already_loaded = loaded(runner, user, &path).await?.is_some();
    ensure!(
        existed || !already_loaded,
        "A service with this label is already loaded without the managed plist; refusing to adopt it"
    );
    user.plist(true)?;
    private_file::create_or_verify(&path, &expected)?;
    runner
        .run(&["enable", &user.target()])
        .await?
        .success("Enable LaunchAgent at user login")?;
    if !already_loaded {
        runner
            .run(&[
                "bootstrap",
                &user.domain(),
                unit_path(&path, "LaunchAgent plist")?,
            ])
            .await?
            .success("Load LaunchAgent; the plist remains installed if startup failed")?;
    }
    // Without '-k', an already-running agent and its workloads are preserved.
    runner
        .run(&["kickstart", "-p", &user.target()])
        .await?
        .success("Start LaunchAgent")?;
    ensure!(
        loaded(runner, user, &path)
            .await?
            .is_some_and(|service| service.process_id.is_some()),
        "LaunchAgent did not remain running; inspect launchctl status before retrying"
    );
    Ok(path)
}

async fn uninstall_with(
    runner: &impl Runner,
    user: &User,
    executable: &Path,
    state: &Path,
) -> Result<bool> {
    let expected = launchd_user_plist(executable, state)?;
    let path = user.plist(false)?;
    if !private_file::matches(&path, &expected)? {
        return Ok(false);
    }
    ensure!(
        domain_available(runner, user).await?,
        "Log in to this macOS account's GUI session before removing its LaunchAgent"
    );
    let active = loaded(runner, user, &path).await?.is_some();
    runner
        .run(&["disable", &user.target()])
        .await?
        .success("Disable LaunchAgent at user login")?;
    if active {
        runner
            .run(&["bootout", &user.target()])
            .await?
            .success("Stop LaunchAgent; its plist remains installed if stopping failed")?;
    }
    ensure!(
        private_file::matches(&path, &expected)?,
        "LaunchAgent plist changed during removal"
    );
    std::fs::remove_file(&path)?;
    File::open(path.parent().context("Missing LaunchAgents directory")?)?.sync_all()?;
    Ok(true)
}

async fn status_with(
    runner: &impl Runner,
    user: &User,
    executable: &Path,
    state: &Path,
) -> Result<ServiceStatus> {
    let expected = launchd_user_plist(executable, state)?;
    let path = user.plist(false)?;
    let installed = private_file::matches(&path, &expected)?;
    let mut result = ServiceStatus {
        manager: "launchd",
        startup: "user_login",
        definition: path.clone(),
        installed,
        manager_available: None,
        loaded: false,
        process_id: None,
    };
    if !installed {
        return Ok(result);
    }
    let available = domain_available(runner, user).await?;
    result.manager_available = Some(available);
    if available && let Some(service) = loaded(runner, user, &path).await? {
        result.loaded = true;
        result.process_id = service.process_id;
    }
    Ok(result)
}

pub(super) async fn install(executable: &Path, state: &Path) -> Result<PathBuf> {
    ensure!(
        cfg!(feature = "runtime"),
        "Service installation requires a binary built with the runtime feature"
    );
    install_with(&Launchctl, &User::current()?, executable, state).await
}
pub(super) async fn uninstall(executable: &Path, state: &Path) -> Result<bool> {
    uninstall_with(&Launchctl, &User::current()?, executable, state).await
}
pub(super) async fn status(executable: &Path, state: &Path) -> Result<ServiceStatus> {
    status_with(&Launchctl, &User::current()?, executable, state).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::VecDeque,
        os::unix::fs::{PermissionsExt, symlink},
        sync::Mutex,
    };

    struct Step {
        args: Vec<String>,
        reply: Reply,
    }
    struct Fake(Mutex<VecDeque<Step>>);
    impl Fake {
        fn new(steps: Vec<Step>) -> Self {
            Self(Mutex::new(steps.into()))
        }
        fn complete(&self) {
            assert!(
                self.0.lock().unwrap().is_empty(),
                "expected launchctl calls were not made"
            );
        }
    }
    #[async_trait::async_trait]
    impl Runner for Fake {
        async fn run(&self, args: &[&str]) -> Result<Reply> {
            let step = self
                .0
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected launchctl call");
            assert_eq!(step.args, args);
            Ok(step.reply)
        }
    }
    fn step(args: &[&str], code: i32, out: String, err: &str) -> Step {
        Step {
            args: args.iter().map(|s| (*s).to_owned()).collect(),
            reply: Reply {
                code: Some(code),
                out,
                err: err.into(),
            },
        }
    }
    fn ok(args: &[&str]) -> Step {
        step(args, 0, String::new(), "")
    }
    fn absent(user: &User) -> Step {
        step(
            &["print", &user.target()],
            113,
            String::new(),
            &format!(
                "Could not find service \"{LAUNCHD_LABEL}\" in domain for user gui:{}",
                user.uid
            ),
        )
    }
    fn running(user: &User, path: &Path, pid: Option<u32>) -> Step {
        let pid = pid
            .map(|pid| format!("\tpid = {pid}\n"))
            .unwrap_or_default();
        step(
            &["print", &user.target()],
            0,
            format!(
                "{} = {{\n\tpath = {}\n\targuments = {{\n\t\tpath = ignored nested argument\n\t}}\n{pid}}}\n",
                user.target(),
                path.display()
            ),
            "",
        )
    }
    fn fixture() -> Result<(tempfile::TempDir, User, PathBuf, PathBuf)> {
        let temp = tempfile::tempdir()?;
        let home = temp.path().join("account-home");
        DirBuilder::new().mode(0o700).create(&home)?;
        let state = home.join("device-state");
        DirBuilder::new().mode(0o700).create(&state)?;
        let executable = home.join("agent & executable");
        std::fs::write(&executable, "fixture only")?;
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700))?;
        // SAFETY: geteuid has no arguments or preconditions.
        let user = User {
            uid: unsafe { libc::geteuid() },
            home,
        };
        Ok((temp, user, executable, state))
    }

    #[tokio::test]
    async fn install_and_idempotent_retry_use_only_the_exact_user_label() -> Result<()> {
        let (_temp, user, exe, state) = fixture()?;
        let path = user.plist(false)?;
        let fake = Fake::new(vec![
            ok(&["print", &user.domain()]),
            absent(&user),
            ok(&["enable", &user.target()]),
            ok(&["bootstrap", &user.domain(), path.to_str().unwrap()]),
            ok(&["kickstart", "-p", &user.target()]),
            running(&user, &path, Some(42)),
        ]);
        assert_eq!(install_with(&fake, &user, &exe, &state).await?, path);
        fake.complete();
        assert!(private_file::matches(
            &path,
            &launchd_user_plist(&exe, &state)?
        )?);
        let fake = Fake::new(vec![
            ok(&["print", &user.domain()]),
            running(&user, &path, Some(42)),
            ok(&["enable", &user.target()]),
            ok(&["kickstart", "-p", &user.target()]),
            running(&user, &path, Some(42)),
        ]);
        install_with(&fake, &user, &exe, &state).await?;
        fake.complete();
        assert!(!state.join("management.sqlite").exists());
        Ok(())
    }

    #[tokio::test]
    async fn conflicting_plist_and_loaded_origin_are_never_modified() -> Result<()> {
        let (_temp, user, exe, state) = fixture()?;
        let path = user.plist(true)?;
        private_file::create_or_verify(&path, &launchd_user_plist(&exe, &state)?)?;
        let fake = Fake::new(vec![]);
        assert!(install_with(&fake, &user, &exe, &user.home).await.is_err());
        assert!(
            uninstall_with(&fake, &user, &exe, &user.home)
                .await
                .is_err()
        );
        fake.complete();
        let fake = Fake::new(vec![
            ok(&["print", &user.domain()]),
            running(&user, Path::new("/other/agent.plist"), Some(99)),
        ]);
        assert!(uninstall_with(&fake, &user, &exe, &state).await.is_err());
        fake.complete();
        assert!(path.exists());
        Ok(())
    }

    #[tokio::test]
    async fn unavailable_gui_and_invalid_paths_do_not_publish_a_plist() -> Result<()> {
        let (_temp, user, exe, state) = fixture()?;
        let fake = Fake::new(vec![step(
            &["print", &user.domain()],
            113,
            String::new(),
            "Could not find domain for",
        )]);
        let error = install_with(&fake, &user, &exe, &state)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("cannot run before login"));
        fake.complete();
        assert!(!user.home.join("Library").exists());
        let fake = Fake::new(vec![]);
        let linked = user.home.join("linked-executable");
        symlink(&exe, &linked)?;
        assert!(install_with(&fake, &user, &linked, &state).await.is_err());
        std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o755))?;
        assert!(install_with(&fake, &user, &exe, &state).await.is_err());
        fake.complete();
        Ok(())
    }

    #[tokio::test]
    async fn startup_failure_preserves_exact_definition_for_recovery() -> Result<()> {
        let (_temp, user, exe, state) = fixture()?;
        let path = user.plist(false)?;
        let fake = Fake::new(vec![
            ok(&["print", &user.domain()]),
            absent(&user),
            ok(&["enable", &user.target()]),
            step(
                &["bootstrap", &user.domain(), path.to_str().unwrap()],
                5,
                String::new(),
                "fixture startup failure",
            ),
        ]);
        assert!(install_with(&fake, &user, &exe, &state).await.is_err());
        fake.complete();
        assert!(private_file::matches(
            &path,
            &launchd_user_plist(&exe, &state)?
        )?);
        Ok(())
    }

    #[tokio::test]
    async fn removal_preserves_state_and_stopped_failure_preserves_definition() -> Result<()> {
        let (_temp, user, exe, state) = fixture()?;
        let path = user.plist(true)?;
        private_file::create_or_verify(&path, &launchd_user_plist(&exe, &state)?)?;
        let fake = Fake::new(vec![
            ok(&["print", &user.domain()]),
            running(&user, &path, Some(42)),
            ok(&["disable", &user.target()]),
            step(
                &["bootout", &user.target()],
                5,
                String::new(),
                "fixture stop failure",
            ),
        ]);
        assert!(uninstall_with(&fake, &user, &exe, &state).await.is_err());
        fake.complete();
        assert!(path.exists());
        let fake = Fake::new(vec![
            ok(&["print", &user.domain()]),
            running(&user, &path, Some(42)),
            ok(&["disable", &user.target()]),
            ok(&["bootout", &user.target()]),
        ]);
        assert!(uninstall_with(&fake, &user, &exe, &state).await?);
        fake.complete();
        assert!(!path.exists());
        assert!(state.is_dir());
        let fake = Fake::new(vec![]);
        assert!(!uninstall_with(&fake, &user, &exe, &state).await?);
        fake.complete();
        Ok(())
    }

    #[tokio::test]
    async fn inspection_does_not_create_state_or_confuse_loaded_with_running() -> Result<()> {
        let (_temp, user, exe, state) = fixture()?;
        let path = user.plist(false)?;
        let fake = Fake::new(vec![]);
        let absent = status_with(&fake, &user, &exe, &state).await?;
        assert!(!absent.installed);
        assert_eq!(absent.manager_available, None);
        assert!(!user.home.join("Library").exists());
        fake.complete();
        user.plist(true)?;
        private_file::create_or_verify(&path, &launchd_user_plist(&exe, &state)?)?;
        let fake = Fake::new(vec![
            ok(&["print", &user.domain()]),
            running(&user, &path, None),
        ]);
        let status = status_with(&fake, &user, &exe, &state).await?;
        fake.complete();
        assert!(status.installed && status.loaded);
        assert_eq!(status.startup, "user_login");
        assert!(status.process_id.is_none());
        let fake = Fake::new(vec![step(
            &["print", &user.domain()],
            113,
            String::new(),
            "Could not find domain for",
        )]);
        let status = status_with(&fake, &user, &exe, &state).await?;
        fake.complete();
        assert!(status.installed && !status.loaded);
        assert_eq!(status.manager_available, Some(false));
        Ok(())
    }

    #[test]
    fn diagnostic_parser_rejects_unknown_or_ambiguous_service_origins() -> Result<()> {
        let (_temp, user, _, _) = fixture()?;
        let path = user.plist(false)?;
        for out in [
            "new unsupported format".to_string(),
            format!(
                "{} = {{\npath = {}\npath = {}\n}}",
                user.target(),
                path.display(),
                path.display()
            ),
            format!(
                "{} = {{\narguments = {{\npath = {}\n}}\n}}",
                user.target(),
                path.display()
            ),
        ] {
            assert!(
                loaded_reply(
                    Reply {
                        code: Some(0),
                        out,
                        err: String::new()
                    },
                    &user,
                    &path
                )
                .is_err()
            );
        }
        assert!(
            loaded_reply(
                Reply {
                    code: Some(1),
                    out: String::new(),
                    err: "permission denied".into()
                },
                &user,
                &path
            )
            .is_err()
        );
        Ok(())
    }
}
