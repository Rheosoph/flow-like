use std::time::Duration;

const PORT_ENV: &str = "PORT";
const IDLE_TIMEOUT_ENV: &str = "HTTP_IDLE_TIMEOUT_SECS";
const SHUTDOWN_TIMEOUT_ENV: &str = "SHUTDOWN_TIMEOUT_SECS";
const MAX_CONCURRENT_REQUESTS_ENV: &str = "MAX_CONCURRENT_REQUESTS";

const DEFAULT_PORT: u16 = 8080;
/// Above the ALB's default 60 s idle timeout: the target must never close a
/// keep-alive connection the load balancer still considers reusable.
const DEFAULT_IDLE_TIMEOUT_SECS: u64 = 75;
/// With the telemetry flush budget this stays inside ECS's default 30 s
/// `stopTimeout`, after which the container is killed.
const DEFAULT_SHUTDOWN_TIMEOUT_SECS: u64 = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub port: u16,
    pub idle_timeout: Duration,
    pub shutdown_timeout: Duration,
    pub max_concurrent_requests: Option<usize>,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        Self::parse(|name| std::env::var(name).ok())
    }

    fn parse(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        let setting = |name: &str| {
            lookup(name)
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
        };
        Ok(Self {
            port: port_from(setting(PORT_ENV))?,
            idle_timeout: Duration::from_secs(bounded(
                IDLE_TIMEOUT_ENV,
                setting(IDLE_TIMEOUT_ENV),
                DEFAULT_IDLE_TIMEOUT_SECS,
                1..=4_000,
            )?),
            shutdown_timeout: Duration::from_secs(bounded(
                SHUTDOWN_TIMEOUT_ENV,
                setting(SHUTDOWN_TIMEOUT_ENV),
                DEFAULT_SHUTDOWN_TIMEOUT_SECS,
                0..=3_600,
            )?),
            max_concurrent_requests: setting(MAX_CONCURRENT_REQUESTS_ENV)
                .map(|raw| {
                    bounded(MAX_CONCURRENT_REQUESTS_ENV, Some(raw), 0, 1..=1_000_000)
                        .map(|value| value as usize)
                })
                .transpose()?,
        })
    }
}

pub fn port() -> Result<u16, String> {
    port_from(std::env::var(PORT_ENV).ok())
}

fn port_from(raw: Option<String>) -> Result<u16, String> {
    bounded(PORT_ENV, raw, u64::from(DEFAULT_PORT), 1..=65_535).map(|port| port as u16)
}

fn bounded(
    name: &str,
    raw: Option<String>,
    default: u64,
    range: std::ops::RangeInclusive<u64>,
) -> Result<u64, String> {
    let Some(raw) = raw else {
        return Ok(default);
    };
    raw.trim()
        .parse::<u64>()
        .ok()
        .filter(|value| range.contains(value))
        .ok_or_else(|| {
            format!(
                "{name}={raw:?} must be a whole number between {} and {}",
                range.start(),
                range.end()
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn parse(pairs: &[(&str, &str)]) -> Result<Config, String> {
        let values: HashMap<String, String> = pairs
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect();
        Config::parse(|name| values.get(name).cloned())
    }

    #[test]
    fn defaults_fit_behind_an_alb_inside_the_ecs_stop_timeout() {
        assert_eq!(
            parse(&[]).unwrap(),
            Config {
                port: 8080,
                idle_timeout: Duration::from_secs(75),
                shutdown_timeout: Duration::from_secs(20),
                max_concurrent_requests: None,
            }
        );
    }

    #[test]
    fn reads_overrides_and_treats_blank_values_as_unset() {
        let config = parse(&[
            ("PORT", "3000"),
            ("HTTP_IDLE_TIMEOUT_SECS", " 400 "),
            ("SHUTDOWN_TIMEOUT_SECS", "0"),
            ("MAX_CONCURRENT_REQUESTS", "512"),
        ])
        .unwrap();
        assert_eq!(config.port, 3000);
        assert_eq!(config.idle_timeout, Duration::from_secs(400));
        assert_eq!(config.shutdown_timeout, Duration::ZERO);
        assert_eq!(config.max_concurrent_requests, Some(512));
        assert_eq!(
            parse(&[("MAX_CONCURRENT_REQUESTS", "  ")])
                .unwrap()
                .max_concurrent_requests,
            None
        );
    }

    #[test]
    fn rejects_out_of_range_values_naming_the_variable() {
        for (name, value) in [
            ("PORT", "0"),
            ("PORT", "65536"),
            ("PORT", "http"),
            ("HTTP_IDLE_TIMEOUT_SECS", "0"),
            ("SHUTDOWN_TIMEOUT_SECS", "-1"),
            ("MAX_CONCURRENT_REQUESTS", "0"),
        ] {
            let error = parse(&[(name, value)]).unwrap_err();
            assert!(error.starts_with(name), "{error}");
        }
    }
}
