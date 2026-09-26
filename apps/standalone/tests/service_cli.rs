use anyhow::Result;
use std::{
    path::Path,
    process::{Command, Output},
};

fn command(directory: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_flow-like-standalone"));
    command
        .current_dir(directory)
        .env_remove("FLOW_LIKE_STANDALONE_STATE_DIR");
    command
}

fn unit(output: Output, state: &Path) -> Result<String> {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout)?;
    #[cfg(not(target_os = "macos"))]
    assert!(
        text.contains(&format!("WorkingDirectory={}\n", state.display())),
        "{text}"
    );
    #[cfg(target_os = "macos")]
    assert!(
        text.contains(&format!(
            "<key>WorkingDirectory</key><string>{}</string>",
            state.display()
        )),
        "{text}"
    );
    assert!(!state.exists());
    Ok(text)
}

#[cfg(unix)]
#[test]
fn printing_a_service_unit_does_not_initialize_a_device() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let state = directory.path().join("not-created");
    let output = command(directory.path())
        .args(["--state-dir", state.to_str().unwrap(), "service-unit"])
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout)?;
    #[cfg(target_os = "linux")]
    assert!(text.contains("WantedBy=default.target"));
    #[cfg(target_os = "macos")]
    assert!(text.contains("<key>LimitLoadToSessionType</key><string>Aqua</string>"));
    assert!(!state.exists());
    Ok(())
}

#[test]
fn installation_requires_existing_state_before_any_service_operation() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let state = directory.path().join("not-created");
    let output = command(directory.path())
        .args(["--state-dir", state.to_str().unwrap(), "install-service"])
        .output()?;
    assert!(!output.status.success());
    assert!(!state.exists());
    Ok(())
}

#[test]
fn service_help_describes_login_boundary_and_read_only_inspection() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let output = command(directory.path()).arg("--help").output()?;
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout)?;
    assert!(text.contains("macOS after the owner logs in"));
    assert!(text.contains("service-status"));
    assert!(std::fs::read_dir(directory.path())?.next().is_none());
    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
#[test]
fn unsupported_service_commands_do_not_initialize_a_device() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let state = directory.path().join("not-created");
    for command in ["install-service", "uninstall-service"] {
        let output = self::command(directory.path())
            .args(["--state-dir", state.to_str().unwrap(), command])
            .output()?;
        assert!(!output.status.success());
        assert!(!state.exists());
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn service_unit_resolves_only_the_selected_standalone_configuration() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let cwd = directory.path().canonicalize()?;
    unit(
        command(&cwd).arg("service-unit").output()?,
        &cwd.join(".flow-like-standalone"),
    )?;

    std::fs::write(
        cwd.join(".env"),
        "FLOW_LIKE_STANDALONE_STATE_DIR='./from-file'\nUNRELATED_SECRET=keep-private\nRUST_LOG=trace\n",
    )?;
    let text = unit(
        command(&cwd).arg("service-unit").output()?,
        &cwd.join("from-file"),
    )?;
    assert!(!text.contains("keep-private"));
    assert!(!text.contains("RUST_LOG"));
    assert!(!text.contains("Environment="));
    unit(
        command(&cwd)
            .env("FLOW_LIKE_STANDALONE_STATE_DIR", "from-process")
            .arg("service-unit")
            .output()?,
        &cwd.join("from-process"),
    )?;
    unit(
        command(&cwd)
            .env("FLOW_LIKE_STANDALONE_STATE_DIR", "from-process")
            .args(["--state-dir", "from-argument", "service-unit"])
            .output()?,
        &cwd.join("from-argument"),
    )?;

    let config = cwd.join("config");
    std::fs::create_dir(&config)?;
    std::fs::write(
        config.join("device.env"),
        "FLOW_LIKE_STANDALONE_STATE_DIR=relative-state\n",
    )?;
    unit(
        command(&cwd)
            .args(["--env-file", "config/device.env", "service-unit"])
            .output()?,
        &config.join("relative-state"),
    )?;
    unit(
        command(&cwd)
            .env("FLOW_LIKE_STANDALONE_STATE_DIR", "process-wins")
            .args(["--env-file", "config/device.env", "service-unit"])
            .output()?,
        &cwd.join("process-wins"),
    )?;
    unit(
        command(&cwd)
            .args([
                "--env-file",
                "config/device.env",
                "--state-dir",
                "argument-wins",
                "service-unit",
            ])
            .output()?,
        &cwd.join("argument-wins"),
    )?;
    assert_eq!(std::fs::read_dir(&cwd)?.count(), 2);
    assert_eq!(std::fs::read_dir(config)?.count(), 1);
    Ok(())
}

#[cfg(unix)]
#[test]
fn explicit_env_files_are_required_and_validated_without_disclosing_values() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let cwd = directory.path().canonicalize()?;
    let state = cwd.join("not-created");
    for extra in [vec![], vec!["--state-dir", "not-created"]] {
        let output = command(&cwd)
            .args(&extra)
            .args(["--env-file", "missing.env", "service-unit"])
            .output()?;
        assert!(!output.status.success());
        assert!(!state.exists());
    }

    std::fs::write(
        cwd.join("broken.env"),
        "UNRELATED_SECRET='never-print-this-value\n",
    )?;
    for extra in [vec![], vec!["--state-dir", "not-created"]] {
        let output = command(&cwd)
            .args(&extra)
            .args(["--env-file", "broken.env", "service-unit"])
            .output()?;
        assert!(!output.status.success());
        let error = String::from_utf8(output.stderr)?;
        assert!(error.contains("Invalid standalone env file syntax"));
        assert!(!error.contains("never-print-this-value"));
        assert!(!error.contains("UNRELATED_SECRET"));
        assert!(!state.exists());
    }
    std::fs::write(
        cwd.join(".env"),
        "UNRELATED_SECRET='never-print-this-value\n",
    )?;
    let output = command(&cwd).arg("service-unit").output()?;
    assert!(!output.status.success());
    assert!(!String::from_utf8(output.stderr)?.contains("never-print-this-value"));
    unit(
        command(&cwd)
            .args(["--state-dir", "not-created", "service-unit"])
            .output()?,
        &state,
    )?;
    unit(
        command(&cwd)
            .env("FLOW_LIKE_STANDALONE_STATE_DIR", "not-created")
            .arg("service-unit")
            .output()?,
        &state,
    )?;
    assert_eq!(std::fs::read_dir(cwd)?.count(), 2);
    Ok(())
}

#[cfg(unix)]
#[test]
fn empty_state_settings_never_change_the_working_directory() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir()?;
    let cwd = directory.path().canonicalize()?;
    std::fs::set_permissions(&cwd, std::fs::Permissions::from_mode(0o755))?;
    std::fs::write(
        cwd.join(".env"),
        "FLOW_LIKE_STANDALONE_STATE_DIR=valid-file-state\n",
    )?;
    for output in [
        command(&cwd)
            .args(["--state-dir", "", "service-unit"])
            .output()?,
        command(&cwd)
            .env("FLOW_LIKE_STANDALONE_STATE_DIR", "")
            .arg("service-unit")
            .output()?,
    ] {
        assert!(!output.status.success());
        assert!(!cwd.join("valid-file-state").exists());
    }
    std::fs::write(cwd.join(".env"), "FLOW_LIKE_STANDALONE_STATE_DIR=''\n")?;
    let output = command(&cwd).arg("service-unit").output()?;
    assert!(!output.status.success());
    assert!(String::from_utf8(output.stderr)?.contains("must not be empty"));
    assert_eq!(std::fs::metadata(&cwd)?.permissions().mode() & 0o777, 0o755);
    assert_eq!(std::fs::read_dir(&cwd)?.count(), 1);
    Ok(())
}

#[cfg(unix)]
#[test]
fn env_files_are_bounded_and_unknown_settings_do_not_become_service_environment() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let cwd = directory.path().canonicalize()?;
    std::fs::write(
        cwd.join(".env"),
        "UNRELATED_SECRET=never-export-this\nRUST_LOG=trace\n",
    )?;
    let text = unit(
        command(&cwd).arg("service-unit").output()?,
        &cwd.join(".flow-like-standalone"),
    )?;
    assert!(!text.contains("never-export-this"));
    assert!(!text.contains("RUST_LOG"));
    assert!(!text.contains("Environment="));
    std::fs::write(cwd.join(".env"), vec![b'#'; 65_537])?;
    let output = command(&cwd).arg("service-unit").output()?;
    assert!(!output.status.success());
    assert!(String::from_utf8(output.stderr)?.contains("size limit"));
    assert_eq!(std::fs::read_dir(&cwd)?.count(), 1);
    Ok(())
}
