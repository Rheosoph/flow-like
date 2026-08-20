//! SQL statement-shape validation shared by every surface that executes
//! caller-supplied SQL against registered Lance tables (graph SQL, query
//! workbench, the table query endpoints, cached flow queries, and the
//! DML-permitting SQL Query node). Lives outside the `graph` feature so
//! ungated callers can reject unsafe statements without pulling in the
//! graph engine.

use std::ops::ControlFlow;

use datafusion::sql::sqlparser::ast::{Query, SetExpr, Statement, Visit, Visitor};
use flow_like_types::{Result, anyhow};

/// Breaks on any query whose body is a DML statement — catches `WITH t AS
/// (SELECT …) DELETE FROM …` and friends, which parse as `Statement::Query`.
struct DmlBodyFinder;

impl Visitor for DmlBodyFinder {
    type Break = ();

    fn pre_visit_query(&mut self, query: &Query) -> ControlFlow<()> {
        if matches!(
            query.body.as_ref(),
            SetExpr::Insert(_) | SetExpr::Update(_) | SetExpr::Delete(_) | SetExpr::Merge(_)
        ) {
            return ControlFlow::Break(());
        }
        ControlFlow::Continue(())
    }
}

fn contains_dml_body(statement: &Statement) -> bool {
    statement.visit(&mut DmlBodyFinder).is_break()
}

/// Breaks on any embedded `Query` node. Inside an UPDATE/DELETE statement
/// every `Query` is a subquery (WHERE `IN (SELECT …)`/`EXISTS`, a scalar
/// subquery in SET, a derived table, or a CTE).
struct AnyQueryFinder;

impl Visitor for AnyQueryFinder {
    type Break = ();

    fn pre_visit_query(&mut self, _query: &Query) -> ControlFlow<()> {
        ControlFlow::Break(())
    }
}

fn contains_any_query(statement: &Statement) -> bool {
    statement.visit(&mut AnyQueryFinder).is_break()
}

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
            if matches!(inner.as_ref(), Statement::Query(_)) && !contains_dml_body(inner) =>
        {
            Ok(())
        }
        _ => Err(anyhow!(
            "Only a single read-only SELECT statement is allowed on this surface"
        )),
    }
}

/// Validates the statement shapes Lance DML translation can execute
/// faithfully, for surfaces that deliberately allow writes.
///
/// DataFusion hands the table provider only the plain WHERE conjuncts that
/// survive optimization: `IN (SELECT …)`/`EXISTS` conditions are decorrelated
/// into joins the provider never sees, while filters *inside* the subquery
/// survive and would run against the target table — silently mutating the
/// wrong rows. The same applies to `UPDATE … FROM` and `DELETE … USING`
/// (qualifiers are stripped) and to scalar subqueries in SET (silently
/// dropped). Those shapes are therefore refused up front; SELECT statements
/// (including ones with subqueries) and INSERT pass through untouched.
pub fn validate_lance_dml_sql(query: &str) -> Result<()> {
    use datafusion::sql::parser::{DFParser, Statement as DFStatement};

    let statements =
        DFParser::parse_sql(query).map_err(|e| anyhow!("Failed to parse SQL query: {}", e))?;
    if statements.len() != 1 {
        return Err(anyhow!("Exactly one SQL statement is allowed per query"));
    }
    let Some(DFStatement::Statement(inner)) = statements.front() else {
        return Ok(());
    };
    match inner.as_ref() {
        Statement::Update { from, .. } => {
            if from.is_some() {
                return Err(anyhow!(
                    "UPDATE … FROM another table is not supported on Lance tables; the joined \
                     condition would not reach the table and the wrong rows could be updated"
                ));
            }
            if contains_any_query(inner) {
                return Err(anyhow!(
                    "UPDATE with subqueries is not supported on Lance tables — only plain WHERE \
                     conditions reach the table, so subquery conditions would be silently \
                     dropped. Materialize the values first and use literals instead."
                ));
            }
            Ok(())
        }
        Statement::Delete(delete) => {
            if delete.using.is_some() {
                return Err(anyhow!(
                    "DELETE … USING another table is not supported on Lance tables; the joined \
                     condition would not reach the table and the wrong rows could be deleted"
                ));
            }
            if contains_any_query(inner) {
                return Err(anyhow!(
                    "DELETE with subqueries is not supported on Lance tables — only plain WHERE \
                     conditions reach the table, so subquery conditions would be silently \
                     dropped. Materialize the id set first and use IN (…) with literal values."
                ));
            }
            Ok(())
        }
        Statement::Query(_) if contains_dml_body(inner) => Err(anyhow!(
            "DML wrapped in WITH/subqueries is not supported; use a plain \
             INSERT/UPDATE/DELETE statement"
        )),
        _ => Ok(()),
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

    #[test]
    fn readonly_rejects_with_wrapped_dml() {
        assert!(validate_readonly_sql("WITH x AS (SELECT 1) DELETE FROM people").is_err());
        assert!(validate_readonly_sql("WITH x AS (SELECT 1) UPDATE people SET a = 1").is_err());
        assert!(
            validate_readonly_sql("WITH x AS (SELECT 1) INSERT INTO people VALUES (1)").is_err()
        );
        assert!(validate_readonly_sql("WITH x AS (DELETE FROM t) SELECT * FROM x").is_err());
    }

    #[test]
    fn dml_shape_allows_plain_statements() {
        assert!(
            validate_lance_dml_sql("SELECT * FROM people WHERE id IN (SELECT id FROM b)").is_ok()
        );
        assert!(validate_lance_dml_sql("INSERT INTO people SELECT * FROM other").is_ok());
        assert!(validate_lance_dml_sql("UPDATE people SET name = 'x' WHERE id = 1").is_ok());
        assert!(validate_lance_dml_sql("DELETE FROM people WHERE id IN (1, 2, 3)").is_ok());
    }

    #[test]
    fn dml_shape_rejects_subqueries_and_multi_table_forms() {
        assert!(
            validate_lance_dml_sql("DELETE FROM people WHERE id IN (SELECT id FROM banned)")
                .is_err()
        );
        assert!(
            validate_lance_dml_sql(
                "DELETE FROM people WHERE name = 'c' AND id IN (SELECT id FROM banned WHERE flag)"
            )
            .is_err()
        );
        assert!(
            validate_lance_dml_sql(
                "DELETE FROM people WHERE EXISTS (SELECT 1 FROM banned WHERE banned.id = people.id)"
            )
            .is_err()
        );
        assert!(
            validate_lance_dml_sql(
                "UPDATE people SET name = (SELECT name FROM other WHERE id = 1)"
            )
            .is_err()
        );
        assert!(
            validate_lance_dml_sql(
                "UPDATE people SET name = o.name FROM other o WHERE o.id = people.id"
            )
            .is_err()
        );
        assert!(
            validate_lance_dml_sql("DELETE FROM people USING banned WHERE people.id = banned.id")
                .is_err()
        );
        assert!(validate_lance_dml_sql("WITH x AS (SELECT 1) DELETE FROM people").is_err());
        assert!(validate_lance_dml_sql("UPDATE people SET a = 1; SELECT 1").is_err());
    }
}
