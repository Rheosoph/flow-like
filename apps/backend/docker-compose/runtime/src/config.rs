use std::env;

#[derive(Clone, Debug)]
pub struct Config {
    pub port: u16,
    pub max_concurrent_executions: usize,
    pub queue_worker_enabled: bool,
    pub queue_worker_concurrency: usize,
    pub redis_url: Option<String>,
    pub redis_queue_name: String,
    pub poll_timeout_secs: u64,
    pub isolation_mode: String,
    pub manager_url: Option<String>,
    pub manager_token: Option<String>,
    pub api_url: Option<String>,
}

fn positive(name: &str, fallback: u64, max: u64) -> Result<u64, ConfigError> {
    let value = env::var(name)
        .unwrap_or_else(|_| fallback.to_string())
        .parse::<u64>()
        .map_err(|_| ConfigError::InvalidValue(name.into()))?;
    if value == 0 || value > max {
        return Err(ConfigError::InvalidValue(name.into()));
    }
    Ok(value)
}

fn secret(name: &str) -> Result<Option<String>, ConfigError> {
    let value = env::var(name).ok().filter(|value| !value.is_empty());
    let file_name = format!("{name}_FILE");
    let path = env::var(&file_name).ok().filter(|path| !path.is_empty());
    match (value, path) {
        (Some(_), Some(_)) => Err(ConfigError::InvalidValue(format!(
            "set only one of {name} or {file_name}"
        ))),
        (value, None) => Ok(value),
        (None, Some(path)) => {
            let metadata = std::fs::metadata(&path)
                .map_err(|_| ConfigError::InvalidValue(file_name.clone()))?;
            if !metadata.is_file() || metadata.len() > 65536 {
                return Err(ConfigError::InvalidValue(file_name));
            }
            std::fs::read_to_string(path)
                .map(|value| Some(value.trim_end_matches(['\r', '\n']).to_owned()))
                .map_err(|_| ConfigError::InvalidValue(file_name))
        }
    }
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        positive(
            if env::var("EXECUTION_TIMEOUT_SECONDS").is_ok() {
                "EXECUTION_TIMEOUT_SECONDS"
            } else {
                "EXECUTOR_TIMEOUT_SECS"
            },
            3600,
            86400,
        )?;
        let isolation_mode =
            env::var("EXECUTION_ISOLATION_MODE").unwrap_or_else(|_| "per_run".into());
        if !matches!(isolation_mode.as_str(), "per_run" | "trusted_shared") {
            return Err(ConfigError::InvalidValue("EXECUTION_ISOLATION_MODE".into()));
        }
        let queue_worker_enabled = match env::var("QUEUE_WORKER_ENABLED").as_deref() {
            Ok("true" | "1") => true,
            Ok("false" | "0") | Err(_) => false,
            _ => return Err(ConfigError::InvalidValue("QUEUE_WORKER_ENABLED".into())),
        };
        let manager_token = secret("EXECUTION_MANAGER_TOKEN")?;
        let manager_url = env::var("EXECUTION_MANAGER_URL").ok();
        let api_url = env::var("API_URL").ok();
        if isolation_mode == "per_run"
            && queue_worker_enabled
            && (manager_url.as_ref().is_none_or(|v| v.is_empty())
                || api_url.as_ref().is_none_or(|v| v.is_empty())
                || manager_token.as_ref().is_none_or(|v| v.len() < 32))
        {
            return Err(ConfigError::InvalidValue(
                "EXECUTION_MANAGER_URL / EXECUTION_MANAGER_TOKEN / API_URL".into(),
            ));
        }
        let redis_url = secret("REDIS_URL")?;
        if queue_worker_enabled && redis_url.as_ref().is_none_or(|v| v.is_empty()) {
            return Err(ConfigError::InvalidValue("REDIS_URL".into()));
        }
        Ok(Self {
            port: positive("PORT", 9000, u16::MAX as u64)? as u16,
            max_concurrent_executions: positive("MAX_CONCURRENT_EXECUTIONS", 10, 1024)? as usize,
            queue_worker_enabled,
            queue_worker_concurrency: positive("QUEUE_WORKER_CONCURRENCY", 10, 65536)? as usize,
            redis_url,
            redis_queue_name: env::var("REDIS_EXECUTION_QUEUE")
                .unwrap_or_else(|_| "exec:jobs:v3".into()),
            poll_timeout_secs: positive("QUEUE_POLL_TIMEOUT_SECS", 5, 60)?,
            isolation_mode,
            manager_url,
            manager_token,
            api_url,
        })
    }
}

#[derive(Debug)]
pub enum ConfigError {
    InvalidValue(String),
}
impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidValue(var) => write!(f, "Invalid value for: {var}"),
        }
    }
}
impl std::error::Error for ConfigError {}
