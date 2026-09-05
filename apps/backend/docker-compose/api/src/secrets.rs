use std::{env, error::Error, fs};

/// Resolve mounted Compose secrets before starting any runtime threads.
pub fn load() -> Result<(), Box<dyn Error>> {
    for name in [
        "DATABASE_URL",
        "REDIS_URL",
        "AWS_ACCESS_KEY_ID",
        "AWS_SECRET_ACCESS_KEY",
        "AWS_SESSION_TOKEN",
        "STS_ISSUER_ACCESS_KEY",
        "STS_ISSUER_SECRET_KEY",
        "BACKEND_KEY",
        "BACKEND_PUB",
        "SINK_TOKEN_ENCRYPTION_KEY",
        "SINK_SECRET",
        "MAINTENANCE_TOKEN",
        "EXECUTION_MANAGER_TOKEN",
        "CRON_SINK_TOKEN",
    ] {
        let Some(path) = env::var_os(format!("{name}_FILE")).filter(|value| !value.is_empty())
        else {
            continue;
        };
        if env::var_os(name).is_some_and(|value| !value.is_empty()) {
            return Err(format!("set either {name} or {name}_FILE").into());
        }
        let metadata = fs::metadata(&path)?;
        if !metadata.is_file() || metadata.len() > 65536 {
            return Err(format!("{name}_FILE must name a regular file of at most 64 KiB").into());
        }
        let value = fs::read_to_string(path)?;
        let value = value.trim_end_matches(['\r', '\n']);
        if value.is_empty() || value.contains('\0') {
            return Err(format!("{name}_FILE contains an invalid secret").into());
        }
        // SAFETY: main calls this before creating Tokio or any application threads.
        unsafe {
            env::set_var(name, value);
            env::remove_var(format!("{name}_FILE"));
        }
    }
    Ok(())
}
