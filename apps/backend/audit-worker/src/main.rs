use std::{
    env, fs,
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use flow_like::hub::AuditConfig;
use flow_like_api::{
    audit::{
        keys,
        worker::{self, AuditWorkerContext},
    },
    db::DbDialect,
};
use flow_like_types::{Context, Result, anyhow};
use sea_orm::{ConnectOptions, Database};
use serde::Deserialize;

#[derive(Deserialize)]
struct Config {
    audit: AuditConfig,
}

fn value(name: &str) -> Option<String> {
    env::var(name).ok().filter(|v| !v.trim().is_empty())
}

fn required(name: &str) -> Result<String> {
    value(name).ok_or_else(|| anyhow!("{name} is required on the dedicated audit worker"))
}

fn health_path() -> PathBuf {
    value("AUDIT_HEALTH_FILE")
        .unwrap_or_else(|| "/tmp/audit-worker-health".into())
        .into()
}

fn unix_seconds() -> Result<u64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs())
}

fn check_health() -> Result<()> {
    let timestamp: u64 = fs::read_to_string(health_path())?.trim().parse()?;
    let age = unix_seconds()?
        .checked_sub(timestamp)
        .ok_or_else(|| anyhow!("worker health timestamp is in the future"))?;
    if age > 900 {
        return Err(anyhow!("no successful audit tick in the last 15 minutes"));
    }
    Ok(())
}

fn write_health() -> Result<()> {
    let path = health_path();
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, unix_seconds()?.to_string())?;
    fs::rename(temporary, path)?;
    Ok(())
}

/// Resolve file secrets before creating any runtime threads. Vault's token sink
/// remains a file and is reread by the signer on each request.
fn load_secrets() -> Result<()> {
    for name in [
        "DATABASE_URL",
        "AUDIT_ENTRY_KEY",
        "AUDIT_ENTRY_KEY_PREVIOUS",
        "AUDIT_SIGNING_KEY",
        "AUDIT_VERIFYING_KEYS",
        "AUDIT_BUCKET_ACCESS_KEY_ID",
        "AUDIT_BUCKET_SECRET_ACCESS_KEY",
        "AUDIT_KMS_AWS_ACCESS_KEY_ID",
        "AUDIT_KMS_AWS_SECRET_ACCESS_KEY",
        "SINK_TOKEN_ENCRYPTION_KEY",
        "FLOW_LIKE_CONFIG_JSON",
    ] {
        let Some(path) = value(&format!("{name}_FILE")) else {
            continue;
        };
        if value(name).is_some() {
            return Err(anyhow!("set either {name} or {name}_FILE"));
        }
        let content = fs::read_to_string(&path).with_context(|| format!("reading {name}_FILE"))?;
        let content = content.trim_end_matches(['\r', '\n']);
        if content.is_empty() || content.contains('\0') {
            return Err(anyhow!("{name}_FILE is empty or invalid"));
        }
        // SAFETY: main resolves secrets before constructing the Tokio runtime.
        unsafe {
            env::set_var(name, content);
            env::remove_var(format!("{name}_FILE"));
        }
    }
    Ok(())
}

fn config() -> Result<AuditConfig> {
    let json = match (
        value("FLOW_LIKE_CONFIG_JSON"),
        value("FLOW_LIKE_CONFIG_PATH"),
    ) {
        (Some(_), Some(_)) => {
            return Err(anyhow!(
                "set only one of FLOW_LIKE_CONFIG_JSON and FLOW_LIKE_CONFIG_PATH"
            ));
        }
        (Some(json), None) => json,
        (None, Some(path)) => fs::read_to_string(path).context("reading audit worker config")?,
        (None, None) => {
            return Err(anyhow!(
                "an explicit audit policy is required via FLOW_LIKE_CONFIG_JSON or FLOW_LIKE_CONFIG_PATH"
            ));
        }
    };
    let mut config: Config = serde_json::from_str(&json).context("invalid audit worker config")?;
    if !config.audit.enabled {
        return Err(anyhow!("audit is disabled in the worker configuration"));
    }
    config.audit.require_signing = true;
    Ok(config.audit)
}

async fn shutdown_signal() -> Result<()> {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {
            _ = terminate.recv() => {},
            signal = tokio::signal::ctrl_c() => signal?,
        }
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c().await?;
    Ok(())
}

async fn run(once: bool) -> Result<()> {
    match fs::remove_file(health_path()) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let entry = required("AUDIT_ENTRY_KEY")?;
    keys::init_entry_key(Some(&entry), None)?;
    keys::init_previous_entry_key(value("AUDIT_ENTRY_KEY_PREVIOUS").as_deref())?;
    let mut options = ConnectOptions::new(required("DATABASE_URL")?);
    options
        .max_connections(5)
        .min_connections(1)
        .connect_timeout(Duration::from_secs(15))
        .sqlx_logging(false);
    let db = Database::connect(options)
        .await
        .context("connecting audit worker database")?;
    let dialect = DbDialect::detect(&db).await;
    let encryption_key = value("SINK_TOKEN_ENCRYPTION_KEY")
        .map(|secret| *blake3::hash(secret.as_bytes()).as_bytes());
    let context = AuditWorkerContext::from_env(db, dialect, config()?, encryption_key, !once)?;
    if context.bucket.is_none() {
        return Err(anyhow!("an immutable audit bucket/container is required"));
    }
    tracing::info!(?dialect, once, "dedicated audit worker started");
    let mut ticker = tokio::time::interval(worker::TICK_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let shutdown = shutdown_signal();
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            signal = &mut shutdown => {
                signal?;
                context.release_lease().await;
                return Ok(());
            },
            _ = ticker.tick() => {}
        }
        let result = worker::tick(&context, chrono::Utc::now()).await.and_then(|report| {
            tracing::info!(report = %serde_json::to_string(&report).unwrap_or_default(), "audit tick completed");
            if !report.failed_steps.is_empty() {
                return Err(anyhow!("audit tick failed in steps: {}", report.failed_steps.join(", ")));
            }
            write_health()
        });
        if once {
            return result;
        }
        if let Err(error) = result {
            tracing::error!(%error, "audit tick failed");
        }
    }
}

fn checkpoint_is_fresh(written_at_ms: i64, now_ms: i64, max_age_seconds: i64) -> Result<()> {
    let age = now_ms.checked_sub(written_at_ms);
    let maximum = max_age_seconds.checked_mul(1000);
    if max_age_seconds <= 0
        || !matches!((age, maximum), (Some(age), Some(maximum)) if age >= 0 && age <= maximum)
    {
        return Err(anyhow!(
            "retained audit checkpoint is stale or has a future timestamp"
        ));
    }
    Ok(())
}

async fn verify_checkpoint(path: &str) -> Result<()> {
    // This mode requires no entry key, private key, or storage write credentials.
    flow_like_api::audit::signer::register_verifying_keys_json(&required("AUDIT_VERIFYING_KEYS")?)?;
    let checkpoint: worker::checkpoint::Checkpoint = serde_json::from_slice(&fs::read(path)?)?;
    worker::checkpoint::authenticate(&checkpoint)?;
    let max_age: i64 = value("AUDIT_CHECKPOINT_MAX_AGE_SECONDS")
        .unwrap_or_else(|| "172800".into())
        .parse()?;
    checkpoint_is_fresh(
        checkpoint.written_at_ms,
        chrono::Utc::now().timestamp_millis(),
        max_age,
    )?;
    let db = Database::connect(required("DATABASE_URL")?).await?;
    worker::checkpoint::verify_retained(&db, &checkpoint).await?;
    let epochs = flow_like_api::audit::verify::verify_epochs(&db, true).await?;
    if !epochs.valid {
        return Err(anyhow!("audit epoch verification failed"));
    }
    println!("Checkpoint signature, freshness, retained chain tips and epoch timeline verified");
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<_> = env::args().skip(1).collect();
    if args == ["--health-check"] {
        return check_health();
    }
    let verify = args.len() == 2 && args[0] == "--verify-checkpoint";
    if !args.is_empty() && args != ["--once"] && !verify {
        return Err(anyhow!(
            "usage: flow-like-audit-worker [--once | --health-check | --verify-checkpoint FILE]"
        ));
    }
    load_secrets()?;
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    if verify {
        runtime.block_on(verify_checkpoint(&args[1]))
    } else {
        runtime.block_on(run(args == ["--once"]))
    }
}

#[cfg(test)]
mod tests {
    use super::{Config, checkpoint_is_fresh};

    #[test]
    fn worker_configuration_requires_an_explicit_audit_policy() {
        assert!(serde_json::from_str::<Config>("{}").is_err());
        assert!(serde_json::from_str::<Config>(r#"{"audit": {}}"#).is_ok());
    }

    #[test]
    fn retained_checkpoint_freshness_rejects_future_stale_and_overflowing_values() {
        assert!(checkpoint_is_fresh(1000, 2000, 1).is_ok());
        assert!(checkpoint_is_fresh(1000, 2001, 1).is_err());
        assert!(checkpoint_is_fresh(2001, 2000, 1).is_err());
        assert!(checkpoint_is_fresh(i64::MIN, 2000, 1).is_err());
        assert!(checkpoint_is_fresh(1000, 2000, i64::MAX).is_err());
        assert!(checkpoint_is_fresh(1000, 2000, 0).is_err());
    }
}
