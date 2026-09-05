//! Connection budgets shared by PostgreSQL deployment targets.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoolConfig {
    pub max_connections: u32,
    pub min_connections: u32,
}

impl PoolConfig {
    pub fn from_env() -> Result<Self, String> {
        Self::parse(|name| std::env::var(name).ok())
    }

    fn parse(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        let setting = |name, default| match lookup(name) {
            Some(value) => value
                .parse::<u32>()
                .map_err(|_| format!("{name} must be an unsigned integer")),
            None => Ok(default),
        };
        let max_connections = setting("DATABASE_POOL_MAX_CONNECTIONS", 10)?;
        let min_connections = setting("DATABASE_POOL_MIN_CONNECTIONS", 1)?;
        if max_connections == 0 {
            return Err("DATABASE_POOL_MAX_CONNECTIONS must be greater than zero".into());
        }
        if min_connections > max_connections {
            return Err(
                "DATABASE_POOL_MIN_CONNECTIONS cannot exceed DATABASE_POOL_MAX_CONNECTIONS".into(),
            );
        }
        Ok(Self {
            max_connections,
            min_connections,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_budget_defaults_and_overrides() {
        assert_eq!(
            PoolConfig::parse(|_| None).unwrap(),
            PoolConfig {
                max_connections: 10,
                min_connections: 1
            }
        );
        assert_eq!(
            PoolConfig::parse(|name| Some(
                if name.ends_with("MAX_CONNECTIONS") {
                    "3"
                } else {
                    "0"
                }
                .into()
            ))
            .unwrap(),
            PoolConfig {
                max_connections: 3,
                min_connections: 0
            }
        );
    }

    #[test]
    fn invalid_budgets_fail_before_opening_connections() {
        assert!(PoolConfig::parse(|_| Some("0".into())).is_err());
        assert!(PoolConfig::parse(|_| Some("invalid".into())).is_err());
        assert!(
            PoolConfig::parse(|name| Some(
                if name.ends_with("MAX_CONNECTIONS") {
                    "1"
                } else {
                    "2"
                }
                .into()
            ))
            .is_err()
        );
    }
}
