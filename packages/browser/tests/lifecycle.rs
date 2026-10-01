fn main() {
    #[cfg(unix)]
    harness::main();
    #[cfg(not(unix))]
    println!(
        "lifecycle: skipped; the fake browser relies on Unix process groups, Windows job teardown is tested in windows_process"
    );
}

#[cfg(unix)]
mod harness {
    use std::future::Future;
    use std::path::{Path, PathBuf};
    use std::pin::Pin;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    use flow_like_browser::launch::{
        BrowserKind, Executable, ExecutableSource, Flavor, LaunchOptions,
    };
    use flow_like_browser::test_hooks::{self, LaunchedProcess, Profile};

    const FAKE_ENV: &str = "FLOW_LIKE_FAKE_CHROME";
    const MODE_SWITCH: &str = "--user-agent=fake-mode:";
    const CHROME_PID_FILE: &str = "fake-chrome.pid";
    const GRANDCHILD_PID_FILE: &str = "fake-grandchild.pid";
    const FLUSHED_FILE: &str = "fake-flushed";
    const ACTIVE_PORT_FILE: &str = "DevToolsActivePort";
    const FAKE_ENDPOINT: &str = "ws://127.0.0.1:12345/devtools/browser/fake";
    const FAKE_LIFETIME: Duration = Duration::from_secs(120);
    const FLUSH_DELAY: Duration = Duration::from_millis(300);
    const ARMED_GRACE: Duration = Duration::from_secs(2);
    const GONE_WITHIN: Duration = Duration::from_secs(10);
    const EXIT_WITHIN: Duration = Duration::from_secs(5);
    const DELIBERATE_PANIC: &str = "deliberate panic between spawn and hand-off";

    type CaseFuture = Pin<Box<dyn Future<Output = ()> + Send>>;
    type Case = (String, fn() -> CaseFuture);

    pub fn main() {
        match std::env::var(FAKE_ENV).as_deref() {
            Ok("1") => fake_chrome(),
            Ok("grandchild") => std::thread::sleep(FAKE_LIFETIME),
            _ => run_cases(),
        }
    }

    fn fake_chrome() {
        let args: Vec<String> = std::env::args().skip(1).collect();
        if args.iter().any(|arg| arg == "--version") {
            println!("Fake Chrome 154.0.0.0");
            return;
        }
        let Some(dir) = switch_value(&args, "--user-data-dir=").map(PathBuf::from) else {
            eprintln!("fake chrome: --user-data-dir is missing");
            std::process::exit(2);
        };
        write_file(&dir.join(CHROME_PID_FILE), &std::process::id().to_string());
        write_file(
            &dir.join(GRANDCHILD_PID_FILE),
            &start_grandchild().to_string(),
        );
        let mode = switch_value(&args, MODE_SWITCH).unwrap_or_else(|| "ok".to_owned());
        act_out(&mode, &dir);
        if !args.iter().any(|arg| arg == "--remote-debugging-pipe") {
            std::thread::sleep(FAKE_LIFETIME);
        } else if wait_for_lifeline_eof() && mode == "flush" {
            std::thread::sleep(FLUSH_DELAY);
            write_file(&dir.join(FLUSHED_FILE), "flushed");
        }
    }

    #[allow(
        clippy::zombie_processes,
        reason = "the grandchild outlives the fake browser so the launcher group kill stays observable"
    )]
    fn start_grandchild() -> u32 {
        Command::new(std::env::current_exe().expect("fake chrome path"))
            .env(FAKE_ENV, "grandchild")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("fake chrome starts its grandchild")
            .id()
    }

    fn act_out(mode: &str, dir: &Path) {
        let exit_with = |code: i32, stderr: &str| -> ! {
            eprintln!("{stderr}");
            std::process::exit(code)
        };
        match mode {
            "exit21" => exit_with(
                21,
                "[1:1:ERROR:process_singleton_posix.cc(345)] Failed to create a ProcessSingleton for your profile directory.",
            ),
            "sandbox" => exit_with(
                1,
                "[1:1:FATAL:zygote_host_impl_linux.cc(128)] No usable sandbox! Update your kernel",
            ),
            "crash" => exit_with(3, "fake chrome crashed on purpose"),
            "stderr" => eprintln!(
                "\nDevTools listening on ws://127.0.0.1:23456/devtools/browser/from-stderr"
            ),
            "garbage" => write_file(
                &dir.join(ACTIVE_PORT_FILE),
                "not-a-port\n/devtools/browser/x",
            ),
            "silent" => {}
            _ => write_file(&dir.join(ACTIVE_PORT_FILE), "12345\n/devtools/browser/fake"),
        }
    }

    fn wait_for_lifeline_eof() -> bool {
        use std::io::Read;
        use std::os::fd::FromRawFd;

        let (closed, lifeline_closed) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            // SAFETY: the launcher installed the lifeline read end at fd 3 and nothing else here owns it.
            let mut lifeline = unsafe { std::fs::File::from_raw_fd(3) };
            let mut buffer = [0u8; 256];
            while matches!(lifeline.read(&mut buffer), Ok(read) if read > 0) {}
            let _ = closed.send(());
        });
        lifeline_closed.recv_timeout(FAKE_LIFETIME).is_ok()
    }

    fn switch_value(args: &[String], prefix: &str) -> Option<String> {
        args.iter()
            .find_map(|arg| arg.strip_prefix(prefix))
            .map(str::to_owned)
    }

    fn write_file(path: &Path, contents: &str) {
        std::fs::write(path, contents).unwrap_or_else(|error| {
            panic!("fake chrome could not write {}: {error}", path.display())
        });
    }

    macro_rules! cases {
        ($($case:ident),* $(,)?) => {
            [$((stringify!($case).to_owned(), (|| Box::pin($case()) as CaseFuture) as fn() -> CaseFuture)),*]
        };
    }

    fn run_cases() {
        // SAFETY: this runs before the runtime or any other thread exists; the fake browsers inherit it.
        unsafe { std::env::set_var(FAKE_ENV, "1") };
        install_quiet_panic_hook();
        let filter = std::env::args().skip(1).find(|arg| !arg.starts_with('-'));
        let cases: [Case; 12] = cases![
            temporary_profile_reports_the_endpoint_and_is_removed,
            closing_the_lifeline_ends_the_browser,
            the_watchdog_kills_after_the_grace,
            a_dropped_process_keeps_its_armed_grace,
            a_panic_between_spawn_and_hand_off_leaves_no_process,
            cancelling_startup_leaves_no_process,
            startup_failures_are_reported_and_leave_no_process,
            the_stderr_announcement_is_the_fallback_endpoint,
            a_persistent_profile_is_prepared_and_kept,
            chrome_for_testing_builds_are_marked_while_in_use,
            proxy_credentials_fail_before_anything_starts,
            adopted_processes_are_killed_with_their_group,
        ];
        let selected: Vec<&Case> = cases
            .iter()
            .filter(|(name, _)| filter.as_deref().is_none_or(|filter| name.contains(filter)))
            .collect();
        let failed = run_selected(&selected);
        report(selected.len(), &failed);
    }

    fn run_selected(selected: &[&Case]) -> Vec<String> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        println!("\nrunning {} tests", selected.len());
        let mut failed = Vec::new();
        for (name, case) in selected {
            let started = Instant::now();
            if runtime.block_on(runtime.spawn(case())).is_ok() {
                let seconds = started.elapsed().as_secs_f64();
                println!("test {name} ... ok ({seconds:.2} s)");
            } else {
                println!("test {name} ... FAILED");
                failed.push(name.clone());
            }
        }
        failed
    }

    fn report(selected: usize, failed: &[String]) {
        let passed = selected - failed.len();
        if failed.is_empty() {
            println!("\ntest result: ok. {passed} passed; 0 failed\n");
            return;
        }
        println!("\nfailures:\n    {}", failed.join("\n    "));
        println!(
            "\ntest result: FAILED. {passed} passed; {} failed\n",
            failed.len()
        );
        std::process::exit(1);
    }

    fn install_quiet_panic_hook() {
        let default_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if info.payload().downcast_ref::<&str>() != Some(&DELIBERATE_PANIC) {
                default_hook(info);
            }
        }));
    }

    fn fake_executable() -> Executable {
        Executable {
            path: std::env::current_exe().expect("test binary path"),
            flavor: Flavor::Chrome,
            version: None,
            source: ExecutableSource::Explicit,
        }
    }

    fn options(user_data_dir: Option<&Path>, mode: &str) -> LaunchOptions {
        LaunchOptions {
            kind: BrowserKind::Chrome,
            executable: None,
            headless: true,
            window_size: (800, 600),
            user_agent: Some(format!("fake-mode:{mode}")),
            user_data_dir: user_data_dir.map(Path::to_path_buf),
            proxy: None,
            locale: None,
            ignore_https_errors: false,
            cache_dir: std::env::temp_dir().join("flow-like-lifecycle-cache"),
            page_load_timeout: Duration::from_secs(30),
            launch_timeout: Duration::from_secs(10),
        }
    }

    async fn launch(mode: &str) -> LaunchedProcess {
        test_hooks::spawn(&fake_executable(), &options(None, mode))
            .await
            .unwrap_or_else(|error| {
                panic!("the fake browser in mode {mode} did not start: {error}")
            })
    }

    fn read_pid(dir: &Path, file: &str) -> u32 {
        let path = dir.join(file);
        std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("reading {}: {error}", path.display()))
            .trim()
            .parse()
            .unwrap_or_else(|error| panic!("{} holds no pid: {error}", path.display()))
    }

    fn alive(pid: u32) -> bool {
        let pid = libc::pid_t::try_from(pid).expect("pid fits pid_t");
        // SAFETY: signal 0 only checks that the process exists.
        let result = unsafe { libc::kill(pid, 0) };
        result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }

    async fn wait_until(what: &str, condition: impl Fn() -> bool) {
        let deadline = Instant::now() + GONE_WITHIN;
        while !condition() {
            assert!(
                Instant::now() < deadline,
                "{what} did not happen within {GONE_WITHIN:?}"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    async fn assert_gone(pid: u32, what: &str) {
        wait_until(&format!("exit of {what} (pid {pid})"), || !alive(pid)).await;
    }

    async fn wait_for_file(path: &Path) {
        wait_until(&format!("creation of {}", path.display()), || {
            path.is_file()
        })
        .await;
    }

    async fn temporary_profile_reports_the_endpoint_and_is_removed() {
        let launched = launch("ok").await;
        assert_eq!(launched.ws_url, FAKE_ENDPOINT);
        assert_eq!(launched.port, 12345);
        let process = &launched.process;
        let profile = process.profile().clone();
        assert!(profile.temporary);
        let name = profile
            .dir
            .file_name()
            .and_then(|name| name.to_str())
            .expect("profile name");
        assert!(
            name.starts_with(&format!("flow-like-browser-{}-", std::process::id())),
            "{name}"
        );
        assert!(profile.dir.join("Default").join("Preferences").is_file());
        let staging = process.staging_dir().expect("staging folder").to_path_buf();
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&staging)
                .expect("staging")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o700);
        }
        assert_eq!(read_pid(&profile.dir, CHROME_PID_FILE), process.pid());
        let grandchild = read_pid(&profile.dir, GRANDCHILD_PID_FILE);
        assert!(alive(grandchild));

        process.kill();
        assert!(
            process.wait_exit(EXIT_WITHIN).await,
            "the fake browser survived kill"
        );
        assert_gone(grandchild, "the fake browser's grandchild").await;
        process.cleanup();
        assert!(!profile.dir.exists(), "the temporary profile is removed");
        assert!(!staging.exists(), "the staging folder is removed");
    }

    async fn closing_the_lifeline_ends_the_browser() {
        let launched = launch("ok").await;
        let process = &launched.process;
        let grandchild = read_pid(&process.profile().dir, GRANDCHILD_PID_FILE);
        assert!(
            !process.wait_exit(Duration::from_millis(200)).await,
            "the fake browser waits while the lifeline is open"
        );
        process.close_lifeline();
        assert!(
            process.wait_exit(EXIT_WITHIN).await,
            "the fake browser exits on lifeline EOF"
        );
        assert!(alive(grandchild), "the grandchild outlives its parent");
        process.kill();
        assert_gone(grandchild, "the orphaned grandchild").await;
        process.cleanup();
    }

    async fn the_watchdog_kills_after_the_grace() {
        let launched = launch("ok").await;
        let process = &launched.process;
        let grandchild = read_pid(&process.profile().dir, GRANDCHILD_PID_FILE);
        let armed = Instant::now();
        process.arm_watchdog(Duration::from_millis(400));
        assert!(
            !process.wait_exit(Duration::from_millis(150)).await,
            "the watchdog waits for its grace"
        );
        assert!(
            process.wait_exit(EXIT_WITHIN).await,
            "the watchdog killed the browser"
        );
        assert!(armed.elapsed() >= Duration::from_millis(400));
        assert_gone(grandchild, "the grandchild after the watchdog kill").await;
        process.cleanup();
    }

    async fn a_dropped_process_keeps_its_armed_grace() {
        let profile = tempfile::tempdir().expect("tempdir");
        let launched =
            test_hooks::spawn(&fake_executable(), &options(Some(profile.path()), "flush"))
                .await
                .expect("fake browser starts");
        let chrome = read_pid(profile.path(), CHROME_PID_FILE);
        let grandchild = read_pid(profile.path(), GRANDCHILD_PID_FILE);
        let staging = launched
            .process
            .staging_dir()
            .expect("staging folder")
            .to_path_buf();
        let armed = Instant::now();
        launched.process.close_lifeline();
        launched.process.arm_watchdog(ARMED_GRACE);
        drop(launched);

        wait_for_file(&profile.path().join(FLUSHED_FILE)).await;
        assert!(
            armed.elapsed() < ARMED_GRACE,
            "the fake browser finished its lifeline shutdown instead of being killed on drop"
        );
        assert!(
            alive(grandchild),
            "the group is only killed when the grace ends"
        );
        assert!(staging.is_dir(), "the workspace lives until the grace ends");

        assert_gone(grandchild, "the grandchild at the end of the grace").await;
        assert!(armed.elapsed() >= ARMED_GRACE);
        assert_gone(chrome, "the gracefully exited fake browser").await;
        wait_until("removal of the staging folder", || !staging.exists()).await;
        assert!(
            profile.path().join("Default").join("Preferences").is_file(),
            "persistent profiles are never removed"
        );
    }

    async fn a_panic_between_spawn_and_hand_off_leaves_no_process() {
        let (report, reported) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let launched = launch("ok").await;
            let dir = launched.process.profile().dir.clone();
            let staging = launched.process.staging_dir().map(Path::to_path_buf);
            let pids = (
                read_pid(&dir, CHROME_PID_FILE),
                read_pid(&dir, GRANDCHILD_PID_FILE),
            );
            report.send((dir, staging, pids)).expect("report");
            std::panic::panic_any(DELIBERATE_PANIC);
        });
        let error = task.await.expect_err("the task panicked");
        assert!(error.is_panic());
        let (dir, staging, (chrome, grandchild)) = reported.await.expect("reported");
        assert_gone(chrome, "the fake browser dropped by the panic").await;
        assert_gone(grandchild, "its grandchild").await;
        assert!(!dir.exists(), "the temporary profile is removed on drop");
        assert!(staging.is_some_and(|staging| !staging.exists()));
    }

    async fn cancelling_startup_leaves_no_process() {
        let profile = tempfile::tempdir().expect("tempdir");
        let executable = fake_executable();
        let options = options(Some(profile.path()), "silent");
        let grandchild_file = profile.path().join(GRANDCHILD_PID_FILE);
        tokio::select! {
            result = test_hooks::spawn(&executable, &options) => {
                panic!("a silent browser never reports an endpoint, got {:?}", result.map(|launched| launched.ws_url));
            }
            () = wait_for_file(&grandchild_file) => {}
        }
        assert_gone(
            read_pid(profile.path(), CHROME_PID_FILE),
            "the cancelled browser",
        )
        .await;
        assert_gone(
            read_pid(profile.path(), GRANDCHILD_PID_FILE),
            "its grandchild",
        )
        .await;
        assert!(profile.path().join("Default").join("Preferences").is_file());
    }

    async fn startup_failures_are_reported_and_leave_no_process() {
        let expectations = [
            ("exit21", "is in use by"),
            ("sandbox", "restricts the browser sandbox (AppArmor)"),
            (
                "crash",
                "Google Chrome exited during startup (code 3): fake chrome crashed on purpose",
            ),
            ("garbage", "is not a valid DevToolsActivePort file"),
            ("silent", "DevToolsActivePort was not written"),
        ];
        for (mode, expected) in expectations {
            let profile = tempfile::tempdir().expect("tempdir");
            let mut options = options(Some(profile.path()), mode);
            options.launch_timeout = Duration::from_secs(1);
            let error = test_hooks::spawn(&fake_executable(), &options)
                .await
                .err()
                .unwrap_or_else(|| panic!("mode {mode} must fail"))
                .to_string();
            assert!(error.contains(expected), "mode {mode}: {error}");
            assert_gone(read_pid(profile.path(), CHROME_PID_FILE), mode).await;
            assert_gone(
                read_pid(profile.path(), GRANDCHILD_PID_FILE),
                &format!("{mode} grandchild"),
            )
            .await;
        }
    }

    async fn the_stderr_announcement_is_the_fallback_endpoint() {
        let launched = launch("stderr").await;
        assert_eq!(
            launched.ws_url,
            "ws://127.0.0.1:23456/devtools/browser/from-stderr"
        );
        assert!(
            launched
                .process
                .stderr_tail()
                .contains("DevTools listening on")
        );
        launched.process.kill();
        assert!(launched.process.wait_exit(EXIT_WITHIN).await);
        launched.process.cleanup();
    }

    async fn a_persistent_profile_is_prepared_and_kept() {
        let profile = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            profile.path().join(ACTIVE_PORT_FILE),
            "1\n/devtools/browser/stale",
        )
        .expect("stale port file");
        let mut options = options(Some(profile.path()), "ok");
        options.locale = Some("de-DE".to_owned());
        let launched = test_hooks::spawn(&fake_executable(), &options)
            .await
            .expect("fake browser starts");
        assert_eq!(
            launched.ws_url, FAKE_ENDPOINT,
            "the stale file was deleted first"
        );
        assert!(!launched.process.profile().temporary);
        let preferences: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(profile.path().join("Default").join("Preferences"))
                .expect("preferences"),
        )
        .expect("preferences json");
        assert_eq!(preferences["intl"]["accept_languages"], "de-DE");
        let grandchild = read_pid(profile.path(), GRANDCHILD_PID_FILE);
        launched.process.kill();
        assert!(launched.process.wait_exit(EXIT_WITHIN).await);
        assert_gone(grandchild, "the persistent-profile grandchild").await;
        launched.process.cleanup();
        assert!(profile.path().join("Default").join("Preferences").is_file());
        assert!(profile.path().join("First Run").is_file());
    }

    async fn chrome_for_testing_builds_are_marked_while_in_use() {
        let cache = tempfile::tempdir().expect("tempdir");
        let install = cache
            .path()
            .join("chrome-for-testing")
            .join("linux64-154.0.8037.92");
        let binary_dir = install.join("chrome-linux64");
        std::fs::create_dir_all(&binary_dir).expect("install layout");
        let binary = binary_dir.join("chrome");
        std::os::unix::fs::symlink(std::env::current_exe().expect("test binary"), &binary)
            .expect("symlink the fake browser");
        let executable = Executable {
            path: binary,
            flavor: Flavor::ChromeForTesting,
            version: Some("154.0.8037.92".to_owned()),
            source: ExecutableSource::CachedCft,
        };
        let marker = install.join(format!(".in-use-{}", std::process::id()));
        let started = test_hooks::spawn(&executable, &options(None, "ok")).await;
        if apparmor_refuses_downloaded_builds() {
            let error = started
                .err()
                .expect("AppArmor refuses Chrome for Testing")
                .to_string();
            assert!(error.contains("AppArmor"), "{error}");
            assert!(!marker.exists());
            return;
        }
        let launched = started.expect("fake Chrome for Testing starts");
        assert!(
            marker.is_file(),
            "{} marks the build as in use",
            marker.display()
        );
        launched.process.kill();
        assert!(launched.process.wait_exit(EXIT_WITHIN).await);
        launched.process.cleanup();
        assert!(!marker.exists(), "cleanup removes the in-use marker");
    }

    fn apparmor_refuses_downloaded_builds() -> bool {
        cfg!(target_os = "linux")
            && std::fs::read_to_string("/proc/sys/kernel/apparmor_restrict_unprivileged_userns")
                .is_ok_and(|value| value.trim() == "1")
            && !Path::new("/opt/google/chrome/chrome-sandbox").exists()
    }

    async fn proxy_credentials_fail_before_anything_starts() {
        let profile = tempfile::tempdir().expect("tempdir");
        let mut options = options(Some(profile.path()), "ok");
        options.proxy = Some(flow_like_browser::launch::ProxyConfig {
            server: "http://user:secret@proxy.local:3128".to_owned(),
            bypass: Vec::new(),
        });
        let error = test_hooks::spawn(&fake_executable(), &options)
            .await
            .err()
            .expect("credentials are refused")
            .to_string();
        assert!(
            error.starts_with("Proxy credentials are not supported"),
            "{error}"
        );
        assert!(!error.contains("secret"));
        assert!(
            !profile.path().join(CHROME_PID_FILE).exists(),
            "no browser was started"
        );
        assert!(
            !profile.path().join("Default").exists(),
            "the profile is untouched"
        );
    }

    async fn adopted_processes_are_killed_with_their_group() {
        let profile = tempfile::tempdir().expect("tempdir");
        let child = tokio::process::Command::new("sleep")
            .arg("30")
            .process_group(0)
            .kill_on_drop(true)
            .spawn()
            .expect("sleep starts");
        let pid = child.id().expect("pid");
        let process = test_hooks::adopt_process(
            child,
            Profile {
                dir: profile.path().to_path_buf(),
                temporary: false,
            },
            None,
        );
        assert_eq!(process.pid(), pid);
        assert!(process.staging_dir().is_none());
        process.kill();
        assert!(process.wait_exit(EXIT_WITHIN).await);
        process.cleanup();
        assert!(
            profile.path().is_dir(),
            "persistent profiles are never removed"
        );
    }
}
