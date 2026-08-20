//! Read-only SQL validation shared by every surface that executes caller-supplied
//! SQL against registered Lance tables (graph SQL, query workbench, the table
//! query endpoints, cached flow queries). Lives outside the `graph` feature so
//! ungated callers can reject writes without pulling in the graph engine.

use flow_like_types::{Result, anyhow};

/// Validates that a string is a single read-only SQL statement.
pub fn validate_readonly_sql(query: &str) -> Result<()> {
    use datafusion::sql::parser::{DFParser, Statement as DFStatement};

    let statements =
        DFParser::parse_sql(query).map_err(|e| anyhow!("Failed to parse SQL query: {}", e))?;
    if statements.len() != 1 {
        return Err(anyhow!("Exactly one SQL statement is allowed per query"));
    }
    match statements.front() {
        Some(DFStatement::Statement(inner))
            if matches!(
                inner.as_ref(),
                datafusion::sql::sqlparser::ast::Statement::Query(_)
            ) =>
        {
            Ok(())
        }
        _ => Err(anyhow!(
            "Only a single read-only SELECT statement is allowed on this surface"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_single_select_statements() {
        assert!(validate_readonly_sql("SELECT * FROM people LIMIT 5").is_ok());
        assert!(validate_readonly_sql("WITH x AS (SELECT 1) SELECT * FROM x").is_ok());
        assert!(validate_readonly_sql("DROP TABLE people").is_err());
        assert!(validate_readonly_sql("COPY people TO '/tmp/out.csv'").is_err());
        assert!(
            validate_readonly_sql("CREATE EXTERNAL TABLE t STORED AS CSV LOCATION '/etc/passwd'")
                .is_err()
        );
        assert!(validate_readonly_sql("SELECT 1; SELECT 2").is_err());
        assert!(validate_readonly_sql("INSERT INTO people VALUES (1)").is_err());
        assert!(validate_readonly_sql("UPDATE people SET a = 1").is_err());
        assert!(validate_readonly_sql("DELETE FROM people WHERE a = 1").is_err());
    }
}
