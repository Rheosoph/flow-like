use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseConnection, IsolationLevel, Statement};

/// Rows one Aurora DSQL transaction may insert, update or delete in total.
pub const DSQL_MAX_ROWS_PER_TRANSACTION: usize = 3_000;

/// Environment override for [`DbDialect::detect`], for tests and unusual proxies.
pub const DIALECT_ENV: &str = "FLOW_LIKE_DB_DIALECT";

/// The engine behind the PostgreSQL wire protocol the API is talking to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DbDialect {
    #[default]
    Postgres,
    CockroachDb,
    Dsql,
}

impl DbDialect {
    /// The dialect for a pool: what the process that built the pool knows,
    /// else `FLOW_LIKE_DB_DIALECT`, else a probe of the server.
    pub async fn resolve(explicit: Option<Self>, db: &DatabaseConnection) -> Self {
        match explicit {
            Some(dialect) => dialect,
            None => Self::detect(db).await,
        }
    }

    /// Identify the engine: an explicit `FLOW_LIKE_DB_DIALECT`, else `version()`
    /// for CockroachDB, else the DSQL-only `sys` schema, else PostgreSQL.
    pub async fn detect(db: &DatabaseConnection) -> Self {
        if let Some(forced) = std::env::var(DIALECT_ENV)
            .ok()
            .and_then(|v| Self::parse(&v))
        {
            return forced;
        }
        if db.get_database_backend() != DatabaseBackend::Postgres {
            return Self::Postgres;
        }
        match Self::probe(db).await {
            Ok(dialect) => dialect,
            Err(error) => {
                tracing::warn!(%error, "database dialect probe failed; assuming PostgreSQL");
                Self::Postgres
            }
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "postgres" | "postgresql" => Some(Self::Postgres),
            "cockroach" | "cockroachdb" | "crdb" => Some(Self::CockroachDb),
            "dsql" | "aurora-dsql" | "aurora_dsql" => Some(Self::Dsql),
            _ => None,
        }
    }

    async fn probe(db: &DatabaseConnection) -> Result<Self, sea_orm::DbErr> {
        let version = db
            .query_one_raw(Statement::from_string(
                DatabaseBackend::Postgres,
                "SELECT version() AS version",
            ))
            .await?
            .and_then(|row| row.try_get::<String>("", "version").ok())
            .unwrap_or_default();
        if version.contains("CockroachDB") {
            return Ok(Self::CockroachDb);
        }
        if version.contains("DSQL") {
            return Ok(Self::Dsql);
        }
        let has_sys_jobs = db
            .query_one_raw(Statement::from_string(
                DatabaseBackend::Postgres,
                "SELECT 1 FROM sys.jobs LIMIT 1",
            ))
            .await
            .is_ok();
        Ok(if has_sys_jobs {
            Self::Dsql
        } else {
            Self::Postgres
        })
    }

    pub fn is_dsql(self) -> bool {
        matches!(self, Self::Dsql)
    }

    /// DSQL runs every transaction at snapshot isolation and offers no other
    /// level, so a requested level is dropped there and honoured elsewhere.
    pub fn effective_isolation(self, requested: Option<IsolationLevel>) -> Option<IsolationLevel> {
        match self {
            Self::Dsql => None,
            Self::Postgres | Self::CockroachDb => requested,
        }
    }

    /// Whether one transaction is capped in rows and bytes.
    pub fn bounded_transactions(self) -> bool {
        self.is_dsql()
    }

    /// Whether commit-time conflicts are the normal way writers are serialized,
    /// as opposed to blocking on locks.
    pub fn optimistic_concurrency(self) -> bool {
        matches!(self, Self::Dsql | Self::CockroachDb)
    }

    /// Whether `pg_stat_*` and `pg_class` style catalog views are readable.
    pub fn has_pg_stat_catalog(self) -> bool {
        matches!(self, Self::Postgres | Self::CockroachDb)
    }

    /// Whether `set_config('statement_timeout', …, true)` style session
    /// timeouts are accepted.
    pub fn supports_set_config_timeouts(self) -> bool {
        matches!(self, Self::Postgres | Self::CockroachDb)
    }

    /// Whether `percentile_cont(…) WITHIN GROUP (ORDER BY …)` is available.
    ///
    /// Aurora DSQL does support it — verified against a live cluster by
    /// `dsql_live::percentile_cont_reports_support`, which fails if this ever
    /// disagrees with the engine. Without it, percentile metrics fall back to
    /// folding every row in the API process and the admin query endpoint
    /// refuses them outright.
    pub fn supports_ordered_set_aggregates(self) -> bool {
        true
    }
}

impl std::fmt::Display for DbDialect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Postgres => "postgresql",
            Self::CockroachDb => "cockroachdb",
            Self::Dsql => "dsql",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_spelling() {
        assert_eq!(DbDialect::parse("PostgreSQL"), Some(DbDialect::Postgres));
        assert_eq!(DbDialect::parse(" crdb "), Some(DbDialect::CockroachDb));
        assert_eq!(DbDialect::parse("aurora-dsql"), Some(DbDialect::Dsql));
        assert_eq!(DbDialect::parse("mysql"), None);
    }

    #[test]
    fn dsql_has_a_single_isolation_level() {
        let requested = Some(IsolationLevel::ReadCommitted);
        assert_eq!(DbDialect::Dsql.effective_isolation(requested), None);
        assert_eq!(
            DbDialect::Postgres.effective_isolation(requested),
            requested
        );
        assert_eq!(
            DbDialect::CockroachDb.effective_isolation(requested),
            requested
        );
    }

    #[test]
    fn catalog_predicates_split_dsql_from_the_rest() {
        for dialect in [DbDialect::Postgres, DbDialect::CockroachDb] {
            assert!(dialect.has_pg_stat_catalog());
            assert!(dialect.supports_set_config_timeouts());
            assert!(dialect.supports_ordered_set_aggregates());
            assert!(!dialect.bounded_transactions());
        }
        assert!(!DbDialect::Dsql.has_pg_stat_catalog());
        assert!(!DbDialect::Dsql.supports_set_config_timeouts());
        assert!(DbDialect::Dsql.supports_ordered_set_aggregates());
        assert!(DbDialect::Dsql.bounded_transactions());
        assert!(DbDialect::Dsql.optimistic_concurrency());
        assert!(DbDialect::CockroachDb.optimistic_concurrency());
        assert!(!DbDialect::Postgres.optimistic_concurrency());
    }
}
