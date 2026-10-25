use std::{collections::HashMap, sync::Arc};

use datafusion::{
    common::{Column, Spans},
    logical_expr::{
        expr::{
            AggregateFunction, AggregateFunctionParams, Alias, Exists, InList, InSubquery,
            PlannedReplaceSelectItem, ScalarFunction, SetComparison, Sort, Unnest, WildcardOptions,
            WindowFunction, WindowFunctionParams,
        },
        Between, BinaryExpr, Case, Cast, Expr, GroupingSet, Like, Limit, LogicalPlan, Subquery,
        TryCast,
    },
    sql::TableReference,
};

use crate::get_table_source;

use super::SQLTableSource;

type Result<T> = std::result::Result<T, datafusion::error::DataFusionError>;

/// Rewrite LogicalPlan's table scans and expressions to use the federated table name.
#[derive(Debug)]
pub struct RewriteTableScanAnalyzer;

impl RewriteTableScanAnalyzer {
    pub fn rewrite(plan: LogicalPlan) -> Result<LogicalPlan> {
        let known_rewrites = &mut HashMap::new();
        rewrite_table_scans(&plan, known_rewrites)
    }
}

/// Rewrite table scans to use the original federated table name.
fn rewrite_table_scans(
    plan: &LogicalPlan,
    known_rewrites: &mut HashMap<TableReference, TableReference>,
) -> Result<LogicalPlan> {
    if plan.inputs().is_empty() {
        if let LogicalPlan::TableScan(table_scan) = plan {
            let original_table_name = table_scan.table_name.clone();
            let mut new_table_scan = table_scan.clone();

            let Some(federated_source) = get_table_source(&table_scan.source)? else {
                // Not a federated source
                return Ok(plan.clone());
            };

            match federated_source.as_any().downcast_ref::<SQLTableSource>() {
                Some(sql_table_source) => {
                    let remote_table_name = sql_table_source.table_reference();
                    known_rewrites.insert(original_table_name, remote_table_name.clone());

                    // Rewrite the schema of this node to have the remote table as the qualifier.
                    let new_schema = (*new_table_scan.projected_schema)
                        .clone()
                        .replace_qualifier(remote_table_name.clone());
                    new_table_scan.projected_schema = Arc::new(new_schema);
                    new_table_scan.table_name = remote_table_name;
                }
                None => {
                    // Not a SQLTableSource (is this possible?)
                    return Ok(plan.clone());
                }
            }

            return Ok(LogicalPlan::TableScan(new_table_scan));
        } else {
            return Ok(plan.clone());
        }
    }

    if let LogicalPlan::Limit(limit) = plan {
        let rewritten_skip = limit
            .skip
            .as_ref()
            .map(|skip| rewrite_table_scans_in_expr(*skip.clone(), known_rewrites).map(Box::new))
            .transpose()?;

        let rewritten_fetch = limit
            .fetch
            .as_ref()
            .map(|fetch| rewrite_table_scans_in_expr(*fetch.clone(), known_rewrites).map(Box::new))
            .transpose()?;

        // explicitly set fetch and skip
        let new_plan = LogicalPlan::Limit(Limit {
            skip: rewritten_skip,
            fetch: rewritten_fetch,
            input: Arc::new(rewrite_table_scans(&limit.input, known_rewrites)?),
        });

        return Ok(new_plan);
    }

    let rewritten_inputs = plan
        .inputs()
        .into_iter()
        .map(|plan| rewrite_table_scans(plan, known_rewrites))
        .collect::<Result<Vec<_>>>()?;

    let mut new_expressions = vec![];
    for expression in plan.expressions() {
        let new_expr = rewrite_table_scans_in_expr(expression.clone(), known_rewrites)?;
        new_expressions.push(new_expr);
    }

    let new_plan = plan.with_new_exprs(new_expressions, rewritten_inputs)?;

    Ok(new_plan)
}

// The function replaces occurrences of table_ref_str in col_name with the new name defined by rewrite.
// The name to rewrite should NOT be a substring of another name.
// Supports multiple occurrences of table_ref_str in col_name.
pub fn rewrite_column_name_in_expr(
    col_name: &str,
    table_ref_str: &str,
    rewrite: &str,
    start_pos: usize,
) -> Option<String> {
    if start_pos >= col_name.len() {
        return None;
    }

    // Find the first occurrence of table_ref_str starting from start_pos
    let idx = col_name[start_pos..].find(table_ref_str)?;

    // Calculate the absolute index of the occurrence in string as the index above is relative to start_pos
    let idx = start_pos + idx;

    if idx > 0 {
        // Check if the previous character is alphabetic, numeric, underscore or period, in which case we
        // should not rewrite as it is a part of another name.
        if let Some(prev_char) = col_name.chars().nth(idx - 1) {
            if prev_char.is_alphabetic()
                || prev_char.is_numeric()
                || prev_char == '_'
                || prev_char == '.'
            {
                return rewrite_column_name_in_expr(
                    col_name,
                    table_ref_str,
                    rewrite,
                    idx + table_ref_str.len(),
                );
            }
        }
    }

    // Check if the next character is alphabetic, numeric or underscore, in which case we
    // should not rewrite as it is a part of another name.
    if let Some(next_char) = col_name.chars().nth(idx + table_ref_str.len()) {
        if next_char.is_alphabetic() || next_char.is_numeric() || next_char == '_' {
            return rewrite_column_name_in_expr(
                col_name,
                table_ref_str,
                rewrite,
                idx + table_ref_str.len(),
            );
        }
    }

    // Found full match, replace table_ref_str occurrence with rewrite
    let rewritten_name = format!(
        "{}{}{}",
        &col_name[..idx],
        rewrite,
        &col_name[idx + table_ref_str.len()..]
    );
    // Check if the rewritten name contains more occurrence of table_ref_str, and rewrite them as well
    // This is done by providing the updated start_pos for search
    match rewrite_column_name_in_expr(&rewritten_name, table_ref_str, rewrite, idx + rewrite.len())
    {
        Some(new_name) => Some(new_name), // more occurrences found
        None => Some(rewritten_name),     // no more occurrences/changes
    }
}

fn rewrite_table_scans_in_expr(
    expr: Expr,
    known_rewrites: &mut HashMap<TableReference, TableReference>,
) -> Result<Expr> {
    match expr {
        Expr::ScalarSubquery(subquery) => {
            let new_subquery = rewrite_table_scans(&subquery.subquery, known_rewrites)?;
            let outer_ref_columns = subquery
                .outer_ref_columns
                .into_iter()
                .map(|e| rewrite_table_scans_in_expr(e, known_rewrites))
                .collect::<Result<Vec<Expr>>>()?;
            Ok(Expr::ScalarSubquery(Subquery {
                subquery: Arc::new(new_subquery),
                outer_ref_columns,
                spans: Spans::new(),
            }))
        }
        Expr::BinaryExpr(binary_expr) => {
            let left = rewrite_table_scans_in_expr(*binary_expr.left, known_rewrites)?;
            let right = rewrite_table_scans_in_expr(*binary_expr.right, known_rewrites)?;
            Ok(Expr::BinaryExpr(BinaryExpr::new(
                Box::new(left),
                binary_expr.op,
                Box::new(right),
            )))
        }
        Expr::Column(mut col) => {
            if let Some(rewrite) = col.relation.as_ref().and_then(|r| known_rewrites.get(r)) {
                Ok(Expr::Column(Column::new(Some(rewrite.clone()), &col.name)))
            } else {
                // This prevent over-eager rewrite and only pass the column into below rewritten
                // rule like MAX(...)
                if col.relation.is_some() {
                    return Ok(Expr::Column(col));
                }

                // Check if any of the rewrites match any substring in col.name, and replace that part of the string if so.
                // This will handles cases like "MAX(foo.df_table.a)" -> "MAX(remote_table.a)"
                let (new_name, was_rewritten) = known_rewrites.iter().fold(
                    (col.name.to_string(), false),
                    |(col_name, was_rewritten), (table_ref, rewrite)| {
                        match rewrite_column_name_in_expr(
                            &col_name,
                            &table_ref.to_string(),
                            &rewrite.to_string(),
                            0,
                        ) {
                            Some(new_name) => (new_name, true),
                            None => (col_name, was_rewritten),
                        }
                    },
                );
                if was_rewritten {
                    Ok(Expr::Column(Column::new(col.relation.take(), new_name)))
                } else {
                    Ok(Expr::Column(col))
                }
            }
        }
        Expr::Alias(alias) => {
            let expr = rewrite_table_scans_in_expr(*alias.expr, known_rewrites)?;
            if let Some(relation) = &alias.relation {
                if let Some(rewrite) = known_rewrites.get(relation) {
                    return Ok(Expr::Alias(Alias::new(
                        expr,
                        Some(rewrite.clone()),
                        alias.name,
                    )));
                }
            }
            Ok(Expr::Alias(Alias::new(expr, alias.relation, alias.name)))
        }
        Expr::Like(like) => {
            let expr = rewrite_table_scans_in_expr(*like.expr, known_rewrites)?;
            let pattern = rewrite_table_scans_in_expr(*like.pattern, known_rewrites)?;
            Ok(Expr::Like(Like::new(
                like.negated,
                Box::new(expr),
                Box::new(pattern),
                like.escape_char,
                like.case_insensitive,
            )))
        }
        Expr::SimilarTo(similar_to) => {
            let expr = rewrite_table_scans_in_expr(*similar_to.expr, known_rewrites)?;
            let pattern = rewrite_table_scans_in_expr(*similar_to.pattern, known_rewrites)?;
            Ok(Expr::SimilarTo(Like::new(
                similar_to.negated,
                Box::new(expr),
                Box::new(pattern),
                similar_to.escape_char,
                similar_to.case_insensitive,
            )))
        }
        Expr::Not(e) => {
            let expr = rewrite_table_scans_in_expr(*e, known_rewrites)?;
            Ok(Expr::Not(Box::new(expr)))
        }
        Expr::IsNotNull(e) => {
            let expr = rewrite_table_scans_in_expr(*e, known_rewrites)?;
            Ok(Expr::IsNotNull(Box::new(expr)))
        }
        Expr::IsNull(e) => {
            let expr = rewrite_table_scans_in_expr(*e, known_rewrites)?;
            Ok(Expr::IsNull(Box::new(expr)))
        }
        Expr::IsTrue(e) => {
            let expr = rewrite_table_scans_in_expr(*e, known_rewrites)?;
            Ok(Expr::IsTrue(Box::new(expr)))
        }
        Expr::IsFalse(e) => {
            let expr = rewrite_table_scans_in_expr(*e, known_rewrites)?;
            Ok(Expr::IsFalse(Box::new(expr)))
        }
        Expr::IsUnknown(e) => {
            let expr = rewrite_table_scans_in_expr(*e, known_rewrites)?;
            Ok(Expr::IsUnknown(Box::new(expr)))
        }
        Expr::IsNotTrue(e) => {
            let expr = rewrite_table_scans_in_expr(*e, known_rewrites)?;
            Ok(Expr::IsNotTrue(Box::new(expr)))
        }
        Expr::IsNotFalse(e) => {
            let expr = rewrite_table_scans_in_expr(*e, known_rewrites)?;
            Ok(Expr::IsNotFalse(Box::new(expr)))
        }
        Expr::IsNotUnknown(e) => {
            let expr = rewrite_table_scans_in_expr(*e, known_rewrites)?;
            Ok(Expr::IsNotUnknown(Box::new(expr)))
        }
        Expr::Negative(e) => {
            let expr = rewrite_table_scans_in_expr(*e, known_rewrites)?;
            Ok(Expr::Negative(Box::new(expr)))
        }
        Expr::Between(between) => {
            let expr = rewrite_table_scans_in_expr(*between.expr, known_rewrites)?;
            let low = rewrite_table_scans_in_expr(*between.low, known_rewrites)?;
            let high = rewrite_table_scans_in_expr(*between.high, known_rewrites)?;
            Ok(Expr::Between(Between::new(
                Box::new(expr),
                between.negated,
                Box::new(low),
                Box::new(high),
            )))
        }
        Expr::Case(case) => {
            let expr = case
                .expr
                .map(|e| rewrite_table_scans_in_expr(*e, known_rewrites))
                .transpose()?
                .map(Box::new);
            let else_expr = case
                .else_expr
                .map(|e| rewrite_table_scans_in_expr(*e, known_rewrites))
                .transpose()?
                .map(Box::new);
            let when_expr = case
                .when_then_expr
                .into_iter()
                .map(|(when, then)| {
                    let when = rewrite_table_scans_in_expr(*when, known_rewrites);
                    let then = rewrite_table_scans_in_expr(*then, known_rewrites);

                    match (when, then) {
                        (Ok(when), Ok(then)) => Ok((Box::new(when), Box::new(then))),
                        (Err(e), _) | (_, Err(e)) => Err(e),
                    }
                })
                .collect::<Result<Vec<(Box<Expr>, Box<Expr>)>>>()?;
            Ok(Expr::Case(Case::new(expr, when_expr, else_expr)))
        }
        Expr::Cast(cast) => {
            let expr = rewrite_table_scans_in_expr(*cast.expr, known_rewrites)?;
            Ok(Expr::Cast(Cast::new(Box::new(expr), cast.data_type)))
        }
        Expr::TryCast(try_cast) => {
            let expr = rewrite_table_scans_in_expr(*try_cast.expr, known_rewrites)?;
            Ok(Expr::TryCast(TryCast::new(
                Box::new(expr),
                try_cast.data_type,
            )))
        }
        Expr::ScalarFunction(sf) => {
            let args = sf
                .args
                .into_iter()
                .map(|e| rewrite_table_scans_in_expr(e, known_rewrites))
                .collect::<Result<Vec<Expr>>>()?;
            Ok(Expr::ScalarFunction(ScalarFunction {
                func: sf.func,
                args,
            }))
        }
        Expr::AggregateFunction(af) => {
            let args = af
                .params
                .args
                .into_iter()
                .map(|e| rewrite_table_scans_in_expr(e, known_rewrites))
                .collect::<Result<Vec<Expr>>>()?;
            let filter = af
                .params
                .filter
                .map(|e| rewrite_table_scans_in_expr(*e, known_rewrites))
                .transpose()?
                .map(Box::new);
            let order_by = af
                .params
                .order_by
                .into_iter()
                .map(|sort| {
                    Ok(Sort {
                        expr: rewrite_table_scans_in_expr(sort.expr, known_rewrites)?,
                        ..sort
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            let params = AggregateFunctionParams {
                args,
                distinct: af.params.distinct,
                filter,
                order_by,
                null_treatment: af.params.null_treatment,
            };
            Ok(Expr::AggregateFunction(AggregateFunction {
                func: af.func,
                params,
            }))
        }
        Expr::WindowFunction(wf) => {
            let args = wf
                .params
                .args
                .into_iter()
                .map(|e| rewrite_table_scans_in_expr(e, known_rewrites))
                .collect::<Result<Vec<Expr>>>()?;
            let partition_by = wf
                .params
                .partition_by
                .into_iter()
                .map(|e| rewrite_table_scans_in_expr(e, known_rewrites))
                .collect::<Result<Vec<Expr>>>()?;
            let order_by = wf
                .params
                .order_by
                .into_iter()
                .map(|sort| {
                    Ok(Sort {
                        expr: rewrite_table_scans_in_expr(sort.expr, known_rewrites)?,
                        ..sort
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            let params = WindowFunctionParams {
                args,
                partition_by,
                order_by,
                window_frame: wf.params.window_frame,
                null_treatment: wf.params.null_treatment,
                distinct: wf.params.distinct,
                filter: wf.params.filter,
            };
            Ok(Expr::WindowFunction(Box::new(WindowFunction {
                fun: wf.fun,
                params,
            })))
        }
        Expr::InList(il) => {
            let expr = rewrite_table_scans_in_expr(*il.expr, known_rewrites)?;
            let list = il
                .list
                .into_iter()
                .map(|e| rewrite_table_scans_in_expr(e, known_rewrites))
                .collect::<Result<Vec<Expr>>>()?;
            Ok(Expr::InList(InList::new(Box::new(expr), list, il.negated)))
        }
        Expr::Exists(exists) => {
            let subquery_plan = rewrite_table_scans(&exists.subquery.subquery, known_rewrites)?;
            let outer_ref_columns = exists
                .subquery
                .outer_ref_columns
                .into_iter()
                .map(|e| rewrite_table_scans_in_expr(e, known_rewrites))
                .collect::<Result<Vec<Expr>>>()?;
            let subquery = Subquery {
                subquery: Arc::new(subquery_plan),
                outer_ref_columns,
                spans: Spans::new(),
            };
            Ok(Expr::Exists(Exists::new(subquery, exists.negated)))
        }
        Expr::InSubquery(is) => {
            let expr = rewrite_table_scans_in_expr(*is.expr, known_rewrites)?;
            let subquery_plan = rewrite_table_scans(&is.subquery.subquery, known_rewrites)?;
            let outer_ref_columns = is
                .subquery
                .outer_ref_columns
                .into_iter()
                .map(|e| rewrite_table_scans_in_expr(e, known_rewrites))
                .collect::<Result<Vec<Expr>>>()?;
            let subquery = Subquery {
                subquery: Arc::new(subquery_plan),
                outer_ref_columns,
                spans: Spans::new(),
            };
            Ok(Expr::InSubquery(InSubquery::new(
                Box::new(expr),
                subquery,
                is.negated,
            )))
        }
        // TODO: remove the next line after `Expr::Wildcard` is removed in datafusion
        #[expect(deprecated)]
        Expr::Wildcard { qualifier, options } => {
            let options = WildcardOptions {
                replace: options
                    .replace
                    .map(|replace| -> Result<PlannedReplaceSelectItem> {
                        Ok(PlannedReplaceSelectItem {
                            planned_expressions: replace
                                .planned_expressions
                                .into_iter()
                                .map(|expr| rewrite_table_scans_in_expr(expr, known_rewrites))
                                .collect::<Result<Vec<_>>>()?,
                            ..replace
                        })
                    })
                    .transpose()?,
                ..*options
            };
            if let Some(rewrite) = qualifier.as_ref().and_then(|q| known_rewrites.get(q)) {
                Ok(Expr::Wildcard {
                    qualifier: Some(rewrite.clone()),
                    options: Box::new(options),
                })
            } else {
                Ok(Expr::Wildcard {
                    qualifier,
                    options: Box::new(options),
                })
            }
        }
        Expr::GroupingSet(gs) => match gs {
            GroupingSet::Rollup(exprs) => {
                let exprs = exprs
                    .into_iter()
                    .map(|e| rewrite_table_scans_in_expr(e, known_rewrites))
                    .collect::<Result<Vec<Expr>>>()?;
                Ok(Expr::GroupingSet(GroupingSet::Rollup(exprs)))
            }
            GroupingSet::Cube(exprs) => {
                let exprs = exprs
                    .into_iter()
                    .map(|e| rewrite_table_scans_in_expr(e, known_rewrites))
                    .collect::<Result<Vec<Expr>>>()?;
                Ok(Expr::GroupingSet(GroupingSet::Cube(exprs)))
            }
            GroupingSet::GroupingSets(vec_exprs) => {
                let vec_exprs = vec_exprs
                    .into_iter()
                    .map(|exprs| {
                        exprs
                            .into_iter()
                            .map(|e| rewrite_table_scans_in_expr(e, known_rewrites))
                            .collect::<Result<Vec<Expr>>>()
                    })
                    .collect::<Result<Vec<Vec<Expr>>>>()?;
                Ok(Expr::GroupingSet(GroupingSet::GroupingSets(vec_exprs)))
            }
        },
        Expr::OuterReferenceColumn(dt, col) => {
            if let Some(rewrite) = col.relation.as_ref().and_then(|r| known_rewrites.get(r)) {
                Ok(Expr::OuterReferenceColumn(
                    dt,
                    Column::new(Some(rewrite.clone()), &col.name),
                ))
            } else {
                Ok(Expr::OuterReferenceColumn(dt, col))
            }
        }
        Expr::Unnest(unnest) => {
            let expr = rewrite_table_scans_in_expr(*unnest.expr, known_rewrites)?;
            Ok(Expr::Unnest(Unnest::new(expr)))
        }
        Expr::ScalarVariable(_, _) | Expr::Literal(_, _) | Expr::Placeholder(_) => Ok(expr),
        Expr::SetComparison(sc) => {
            let expr = rewrite_table_scans_in_expr(*sc.expr, known_rewrites)?;
            let subquery_plan = rewrite_table_scans(&sc.subquery.subquery, known_rewrites)?;
            let outer_ref_columns = sc
                .subquery
                .outer_ref_columns
                .into_iter()
                .map(|e| rewrite_table_scans_in_expr(e, known_rewrites))
                .collect::<Result<Vec<Expr>>>()?;
            let subquery = Subquery {
                subquery: Arc::new(subquery_plan),
                outer_ref_columns,
                spans: Spans::new(),
            };
            Ok(Expr::SetComparison(SetComparison::new(
                Box::new(expr),
                subquery,
                sc.op,
                sc.quantifier,
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::sql::table::SQLTable;
    use crate::sql::{RemoteTableRef, SQLExecutor, SQLFederationProvider, SQLTableSource};
    use crate::FederatedTableProviderAdaptor;
    use async_trait::async_trait;
    use datafusion::arrow::datatypes::{Schema, SchemaRef};
    use datafusion::execution::SendableRecordBatchStream;
    use datafusion::physical_plan::PhysicalExpr;
    use datafusion::sql::unparser::dialect::Dialect;
    use datafusion::sql::unparser::plan_to_sql;
    use datafusion::{
        arrow::datatypes::{DataType, Field},
        catalog::{MemorySchemaProvider, SchemaProvider},
        common::Column,
        datasource::{DefaultTableSource, TableProvider},
        execution::context::SessionContext,
        logical_expr::LogicalPlanBuilder,
        prelude::Expr,
    };

    use super::*;

    struct TestExecutor;

    #[async_trait]
    impl SQLExecutor for TestExecutor {
        fn name(&self) -> &str {
            "TestExecutor"
        }

        fn compute_context(&self) -> Option<String> {
            None
        }

        fn dialect(&self) -> Arc<dyn Dialect> {
            unimplemented!()
        }

        fn execute(
            &self,
            _query: &str,
            _schema: SchemaRef,
            _filters: &[Arc<dyn PhysicalExpr>],
        ) -> Result<SendableRecordBatchStream> {
            unimplemented!()
        }

        async fn table_names(&self) -> Result<Vec<String>> {
            unimplemented!()
        }

        async fn get_table_schema(&self, _table_name: &str) -> Result<SchemaRef> {
            unimplemented!()
        }
    }

    #[derive(Debug)]
    struct TestTable {
        name: RemoteTableRef,
        schema: SchemaRef,
    }

    impl TestTable {
        fn new(name: String, schema: SchemaRef) -> Self {
            TestTable {
                name: name.try_into().unwrap(),
                schema,
            }
        }
    }

    impl SQLTable for TestTable {
        fn table_reference(&self) -> TableReference {
            TableReference::from(&self.name)
        }

        fn schema(&self) -> datafusion::arrow::datatypes::SchemaRef {
            self.schema.clone()
        }

        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }

    fn get_test_table_provider() -> Arc<dyn TableProvider> {
        let schema = Arc::new(Schema::new(vec![
            Field::new("a", DataType::Int64, false),
            Field::new("b", DataType::Utf8, false),
            Field::new("c", DataType::Date32, false),
        ]));
        let table = Arc::new(TestTable::new("remote_table".to_string(), schema));
        let provider = Arc::new(SQLFederationProvider::new(Arc::new(TestExecutor)));
        let table_source = Arc::new(SQLTableSource { provider, table });
        Arc::new(FederatedTableProviderAdaptor::new(table_source))
    }

    fn get_test_table_source() -> Arc<DefaultTableSource> {
        Arc::new(DefaultTableSource::new(get_test_table_provider()))
    }

    fn get_test_df_context() -> SessionContext {
        let ctx = SessionContext::new();
        let catalog = ctx
            .catalog("datafusion")
            .expect("default catalog is datafusion");
        let foo_schema = Arc::new(MemorySchemaProvider::new()) as Arc<dyn SchemaProvider>;
        catalog
            .register_schema("foo", Arc::clone(&foo_schema))
            .expect("to register schema");
        foo_schema
            .register_table("df_table".to_string(), get_test_table_provider())
            .expect("to register table");

        let public_schema = catalog
            .schema("public")
            .expect("public schema should exist");
        public_schema
            .register_table("app_table".to_string(), get_test_table_provider())
            .expect("to register table");

        ctx
    }

    #[test]
    fn test_rewrite_table_scans_basic() -> Result<()> {
        let plan = LogicalPlanBuilder::scan("foo.df_table", get_test_table_source(), None)?
            .project(vec![
                Expr::Column(Column::from_qualified_name("foo.df_table.a")),
                Expr::Column(Column::from_qualified_name("foo.df_table.b")),
                Expr::Column(Column::from_qualified_name("foo.df_table.c")),
            ])?
            .build()?;

        let rewritten_plan = RewriteTableScanAnalyzer::rewrite(plan)?;

        println!("rewritten_plan: \n{:#?}", rewritten_plan);
        let unparsed_sql = plan_to_sql(&rewritten_plan)?;

        println!("unparsed_sql: \n{unparsed_sql}");

        assert_eq!(
            format!("{unparsed_sql}"),
            r#"SELECT remote_table.a, remote_table.b, remote_table.c FROM remote_table"#
        );

        Ok(())
    }

    fn init_tracing() {
        let subscriber = tracing_subscriber::FmtSubscriber::builder()
            .with_env_filter("debug")
            .with_ansi(true)
            .finish();
        let _ = tracing::subscriber::set_global_default(subscriber);
    }

    #[tokio::test]
    async fn test_rewrite_table_scans_agg() -> Result<()> {
        init_tracing();
        let ctx = get_test_df_context();

        let agg_tests = vec![
            (
                "SELECT MAX(a) FROM foo.df_table",
                r#"SELECT max(remote_table.a) FROM remote_table"#,
            ),
            (
                "SELECT foo.df_table.a FROM foo.df_table",
                r#"SELECT remote_table.a FROM remote_table"#,
            ),
            (
                "SELECT MIN(a) FROM foo.df_table",
                r#"SELECT min(remote_table.a) FROM remote_table"#,
            ),
            (
                "SELECT AVG(a) FROM foo.df_table",
                r#"SELECT avg(remote_table.a) FROM remote_table"#,
            ),
            (
                "SELECT SUM(a) FROM foo.df_table",
                r#"SELECT sum(remote_table.a) FROM remote_table"#,
            ),
            (
                "SELECT COUNT(a) FROM foo.df_table",
                r#"SELECT count(remote_table.a) FROM remote_table"#,
            ),
            (
                "SELECT COUNT(a) as cnt FROM foo.df_table",
                r#"SELECT count(remote_table.a) AS cnt FROM remote_table"#,
            ),
            (
                "SELECT COUNT(a) as cnt FROM foo.df_table",
                r#"SELECT count(remote_table.a) AS cnt FROM remote_table"#,
            ),
            (
                "SELECT app_table from (SELECT a as app_table FROM app_table) b",
                r#"SELECT b.app_table FROM (SELECT remote_table.a AS app_table FROM remote_table) AS b"#,
            ),
            (
                "SELECT MAX(app_table) from (SELECT a as app_table FROM app_table) b",
                r#"SELECT max(b.app_table) FROM (SELECT remote_table.a AS app_table FROM remote_table) AS b"#,
            ),
            // multiple occurrences of the same table in single aggregation expression
            (
                "SELECT COUNT(CASE WHEN a > 0 THEN a ELSE 0 END) FROM app_table",
                r#"SELECT count(CASE WHEN (remote_table.a > 0) THEN remote_table.a ELSE 0 END) FROM remote_table"#,
            ),
            // different tables in single aggregation expression
            (
                "SELECT COUNT(CASE WHEN appt.a > 0 THEN appt.a ELSE dft.a END) FROM app_table as appt, foo.df_table as dft",
                "SELECT count(CASE WHEN (appt.a > 0) THEN appt.a ELSE dft.a END) FROM remote_table AS appt CROSS JOIN remote_table AS dft"
            ),
        ];

        for test in agg_tests {
            test_sql(&ctx, test.0, test.1).await?;
        }

        Ok(())
    }

    #[tokio::test]
    async fn test_rewrite_table_scans_alias() -> Result<()> {
        init_tracing();
        let ctx = get_test_df_context();

        let tests = vec![
            (
                "SELECT COUNT(app_table_a) FROM (SELECT a as app_table_a FROM app_table)",
                r#"SELECT count(app_table_a) FROM (SELECT remote_table.a AS app_table_a FROM remote_table)"#,
            ),
            (
                "SELECT app_table_a FROM (SELECT a as app_table_a FROM app_table)",
                r#"SELECT app_table_a FROM (SELECT remote_table.a AS app_table_a FROM remote_table)"#,
            ),
            (
                "SELECT aapp_table FROM (SELECT a as aapp_table FROM app_table)",
                r#"SELECT aapp_table FROM (SELECT remote_table.a AS aapp_table FROM remote_table)"#,
            ),
        ];

        for test in tests {
            test_sql(&ctx, test.0, test.1).await?;
        }

        Ok(())
    }

    #[tokio::test]
    async fn test_rewrite_table_scans_preserve_existing_alias() -> Result<()> {
        init_tracing();
        let ctx = get_test_df_context();

        let tests = vec![
            (
                "SELECT b.a AS app_table_a FROM app_table AS b",
                r#"SELECT b.a AS app_table_a FROM remote_table AS b"#,
            ),
            (
                "SELECT app_table_a FROM (SELECT a as app_table_a FROM app_table AS b)",
                r#"SELECT app_table_a FROM (SELECT b.a AS app_table_a FROM remote_table AS b)"#,
            ),
            (
                "SELECT COUNT(b.a) FROM app_table AS b",
                r#"SELECT count(b.a) FROM remote_table AS b"#,
            ),
        ];

        for test in tests {
            test_sql(&ctx, test.0, test.1).await?;
        }

        Ok(())
    }

    async fn test_sql(ctx: &SessionContext, sql_query: &str, expected_sql: &str) -> Result<()> {
        let data_frame = ctx.sql(sql_query).await?;

        println!("before optimization: \n{:#?}", data_frame.logical_plan());

        let rewritten_plan = RewriteTableScanAnalyzer::rewrite(data_frame.logical_plan().clone())?;

        println!("rewritten_plan: \n{:#?}", rewritten_plan);

        let unparsed_sql = plan_to_sql(&rewritten_plan)?;

        println!("unparsed_sql: \n{unparsed_sql}");

        assert_eq!(
            format!("{unparsed_sql}"),
            expected_sql,
            "SQL under test: {}",
            sql_query
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_rewrite_table_scans_limit_offset() -> Result<()> {
        init_tracing();
        let ctx = get_test_df_context();

        let tests = vec![
            // Basic LIMIT
            (
                "SELECT a FROM foo.df_table LIMIT 5",
                r#"SELECT remote_table.a FROM remote_table LIMIT 5"#,
            ),
            // Basic OFFSET
            (
                "SELECT a FROM foo.df_table OFFSET 5",
                r#"SELECT remote_table.a FROM remote_table OFFSET 5"#,
            ),
            // OFFSET after LIMIT
            (
                "SELECT a FROM foo.df_table LIMIT 10 OFFSET 5",
                r#"SELECT remote_table.a FROM remote_table LIMIT 10 OFFSET 5"#,
            ),
            // LIMIT after OFFSET
            (
                "SELECT a FROM foo.df_table OFFSET 5 LIMIT 10",
                r#"SELECT remote_table.a FROM remote_table LIMIT 10 OFFSET 5"#,
            ),
            // Zero OFFSET
            (
                "SELECT a FROM foo.df_table OFFSET 0",
                r#"SELECT remote_table.a FROM remote_table OFFSET 0"#,
            ),
            // Zero LIMIT
            (
                "SELECT a FROM foo.df_table LIMIT 0",
                r#"SELECT remote_table.a FROM remote_table LIMIT 0"#,
            ),
            // Zero LIMIT and OFFSET
            (
                "SELECT a FROM foo.df_table LIMIT 0 OFFSET 0",
                r#"SELECT remote_table.a FROM remote_table LIMIT 0 OFFSET 0"#,
            ),
        ];

        for test in tests {
            test_sql(&ctx, test.0, test.1).await?;
        }

        Ok(())
    }

    fn get_multipart_test_table_provider() -> Arc<dyn TableProvider> {
        let schema = Arc::new(Schema::new(vec![
            Field::new("a", DataType::Int64, false),
            Field::new("b", DataType::Utf8, false),
            Field::new("c", DataType::Date32, false),
        ]));
        let table = Arc::new(TestTable::new("default.remote_table".to_string(), schema));
        let provider = Arc::new(SQLFederationProvider::new(Arc::new(TestExecutor)));
        let table_source = Arc::new(SQLTableSource { provider, table });
        Arc::new(FederatedTableProviderAdaptor::new(table_source))
    }

    fn get_multipart_test_df_context() -> SessionContext {
        let ctx = SessionContext::new();
        let catalog = ctx
            .catalog("datafusion")
            .expect("default catalog is datafusion");
        let foo_schema = Arc::new(MemorySchemaProvider::new()) as Arc<dyn SchemaProvider>;
        catalog
            .register_schema("foo", Arc::clone(&foo_schema))
            .expect("to register schema");
        foo_schema
            .register_table("df_table".to_string(), get_multipart_test_table_provider())
            .expect("to register table");

        let public_schema = catalog
            .schema("public")
            .expect("public schema should exist");
        public_schema
            .register_table("app_table".to_string(), get_multipart_test_table_provider())
            .expect("to register table");

        ctx
    }

    #[tokio::test]
    async fn test_rewrite_multipart_table() -> Result<()> {
        init_tracing();
        let ctx = get_multipart_test_df_context();

        let tests = vec![
            (
                "SELECT MAX(a) FROM foo.df_table",
                r#"SELECT max(remote_table.a) FROM "default".remote_table"#,
            ),
            (
                "SELECT foo.df_table.a FROM foo.df_table",
                r#"SELECT remote_table.a FROM "default".remote_table"#,
            ),
            (
                "SELECT MIN(a) FROM foo.df_table",
                r#"SELECT min(remote_table.a) FROM "default".remote_table"#,
            ),
            (
                "SELECT AVG(a) FROM foo.df_table",
                r#"SELECT avg(remote_table.a) FROM "default".remote_table"#,
            ),
            (
                "SELECT COUNT(a) as cnt FROM foo.df_table",
                r#"SELECT count(remote_table.a) AS cnt FROM "default".remote_table"#,
            ),
            (
                "SELECT app_table from (SELECT a as app_table FROM app_table) b",
                r#"SELECT b.app_table FROM (SELECT remote_table.a AS app_table FROM "default".remote_table) AS b"#,
            ),
            (
                "SELECT MAX(app_table) from (SELECT a as app_table FROM app_table) b",
                r#"SELECT max(b.app_table) FROM (SELECT remote_table.a AS app_table FROM "default".remote_table) AS b"#,
            ),
            (
                "SELECT COUNT(app_table_a) FROM (SELECT a as app_table_a FROM app_table)",
                r#"SELECT count(app_table_a) FROM (SELECT remote_table.a AS app_table_a FROM "default".remote_table)"#,
            ),
            (
                "SELECT app_table_a FROM (SELECT a as app_table_a FROM app_table)",
                r#"SELECT app_table_a FROM (SELECT remote_table.a AS app_table_a FROM "default".remote_table)"#,
            ),
            (
                "SELECT aapp_table FROM (SELECT a as aapp_table FROM app_table)",
                r#"SELECT aapp_table FROM (SELECT remote_table.a AS aapp_table FROM "default".remote_table)"#,
            ),
        ];

        for test in tests {
            test_sql(&ctx, test.0, test.1).await?;
        }

        Ok(())
    }
}
