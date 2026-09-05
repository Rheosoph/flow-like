use crate::{Error, Result};
use std::{env, fs};
use url::Url;

/// Configuration held only in the trusted supervisor. Never log this value.
#[derive(Clone)]
pub struct CommonConfig {
    pub token: String,
    pub callback_url: String,
    pub object_store_url: String,
    pub allowed_https_hosts: Vec<String>,
    pub buckets: Vec<String>,
    pub object_store_tls_gateway: bool,
    pub backend_pub: String,
    pub capacity: usize,
    /// Durations in seconds, shared with deployment environment variables.
    pub timeout: u64,
    pub startup_grace: u64,
    pub terminal_grace: u64,
    pub cleanup_timeout: u64,
    pub warm_pool_size: usize,
    pub warm_create_concurrency: usize,
    pub warm_idle_seconds: u64,
    pub installation: String,
}

pub fn secret(name: &str) -> Result<String> {
    match env::var(format!("{name}_FILE")) {
        Ok(path) => Ok(fs::read_to_string(path)
            .map_err(|_| Error::invalid(format!("Cannot read {name}_FILE")))?
            .trim_end_matches(['\r', '\n'])
            .to_owned()),
        Err(env::VarError::NotPresent) => Ok(env::var(name).unwrap_or_default()),
        Err(_) => Err(Error::invalid(format!("Invalid {name}_FILE"))),
    }
}

pub fn positive(name: &str, default: u64, maximum: u64) -> Result<u64> {
    env::var(name)
        .unwrap_or_else(|_| default.to_string())
        .parse::<u64>()
        .ok()
        .filter(|value| (1..=maximum).contains(value))
        .ok_or_else(|| Error::invalid(format!("{name} must be between 1 and {maximum}")))
}

pub fn safe_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
}

pub fn origin(value: &str) -> Result<Url> {
    let url = Url::parse(value).map_err(|_| Error::invalid("Expected an HTTP(S) origin"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !matches!(url.path(), "" | "/")
        || value.contains('*')
    {
        return Err(Error::invalid(
            "Expected an HTTP(S) origin without credentials, path or wildcards",
        ));
    }
    Ok(url)
}

impl CommonConfig {
    pub fn budget(&self) -> u64 {
        self.timeout + self.startup_grace + self.terminal_grace + self.cleanup_timeout
    }

    pub fn from_env(kubernetes: bool) -> Result<Self> {
        let value =
            |key: &str| env::var(key).map_err(|_| Error::invalid(format!("{key} is required")));
        let result = Self {
            token: secret("EXECUTION_MANAGER_TOKEN")?,
            callback_url: value("EXECUTION_CALLBACK_URL")?
                .trim_end_matches('/')
                .to_owned(),
            object_store_url: value("EXECUTION_OBJECT_STORE_URL")?
                .trim_end_matches('/')
                .to_owned(),
            allowed_https_hosts: env::var("EXECUTION_ALLOWED_HTTPS_HOSTS")
                .unwrap_or_default()
                .split(',')
                .filter(|v| !v.is_empty())
                .map(str::to_owned)
                .collect(),
            buckets: ["META_BUCKET", "CONTENT_BUCKET", "LOG_BUCKET"]
                .into_iter()
                .zip(["flow-like-meta", "flow-like-content", "flow-like-logs"])
                .map(|(name, default)| env::var(name).unwrap_or_else(|_| default.into()))
                .collect(),
            object_store_tls_gateway: env::var("EXECUTION_OBJECT_STORE_TLS_GATEWAY").as_deref()
                == Ok("true"),
            backend_pub: secret("BACKEND_PUB")?,
            capacity: positive(
                "MAX_CONCURRENT_EXECUTIONS",
                if kubernetes { 10 } else { 4 },
                1024,
            )? as usize,
            timeout: positive("EXECUTION_TIMEOUT_SECONDS", 3600, 86400)?,
            startup_grace: positive(
                "EXECUTION_STARTUP_GRACE_SECONDS",
                if kubernetes {
                    30
                } else {
                    positive("SANDBOX_STARTUP_TIMEOUT_SECONDS", 120, 600)?
                },
                600,
            )?,
            terminal_grace: positive("EXECUTION_TERMINAL_GRACE_SECONDS", 60, 300)?,
            cleanup_timeout: positive("EXECUTION_CLEANUP_TIMEOUT_SECONDS", 30, 300)?,
            warm_pool_size: positive(
                if kubernetes {
                    "WARM_POOL_SIZE"
                } else {
                    "SANDBOX_WARM_POOL_SIZE"
                },
                2,
                1024,
            )? as usize,
            warm_create_concurrency: positive(
                if kubernetes {
                    "WARM_POOL_CREATION_CONCURRENCY"
                } else {
                    "SANDBOX_CREATE_CONCURRENCY"
                },
                2,
                32,
            )? as usize,
            warm_idle_seconds: positive(
                if kubernetes {
                    "WARM_POOL_MAX_AGE_SECONDS"
                } else {
                    "SANDBOX_IDLE_TIMEOUT_SECONDS"
                },
                if kubernetes { 600 } else { 300 },
                3600,
            )?,
            installation: env::var("EXECUTION_INSTALLATION_ID")
                .unwrap_or_else(|_| "flowlike".into()),
        };
        if result.token.len() < 32 || result.backend_pub.is_empty() {
            return Err(Error::invalid(
                "EXECUTION_MANAGER_TOKEN (at least 32 characters) and BACKEND_PUB are required",
            ));
        }
        if origin(&result.callback_url)?.scheme() != "http" {
            return Err(Error::invalid(
                "Execution callbacks require the private HTTP gateway",
            ));
        }
        if origin(&result.object_store_url)?.scheme() == "https" && !result.object_store_tls_gateway
        {
            return Err(Error::invalid(
                "HTTPS storage requires EXECUTION_OBJECT_STORE_TLS_GATEWAY=true and a bucket-only TLS gateway",
            ));
        }
        if !safe_id(&result.installation) {
            return Err(Error::invalid("Invalid execution installation ID"));
        }
        for bucket in &result.buckets {
            if !(3..=63).contains(&bucket.len())
                || !bucket
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                || !bucket.as_bytes()[0].is_ascii_alphanumeric()
                || !bucket.as_bytes()[bucket.len() - 1].is_ascii_alphanumeric()
            {
                return Err(Error::invalid(
                    "Object buckets must be lowercase DNS names without periods",
                ));
            }
        }
        if result
            .buckets
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len()
            != 3
        {
            return Err(Error::invalid(
                "Metadata, content and logs require distinct buckets",
            ));
        }
        for host in &result.allowed_https_hosts {
            if host.len() > 253
                || host.is_empty()
                || !host.as_bytes()[0].is_ascii_alphanumeric()
                || !host
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".-".contains(&b))
            {
                return Err(Error::invalid("HTTPS grants require exact DNS names"));
            }
        }
        Ok(result)
    }
}
