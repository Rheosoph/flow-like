//! What a statement's `$name` placeholders stand for, so a caller can offer a fitting input:
//! the type the planner inferred, and every table column a placeholder is compared with.

use super::SqlQueryResult;
use datafusion::common::tree_node::{TreeNode, TreeNodeRecursion};
use datafusion::logical_expr::expr::{Between, InList, Like, ScalarFunction};
use datafusion::logical_expr::{BinaryExpr, Expr, LogicalPlan, Operator};
use flow_like_types::json::json;
use flow_like_types::{Result, Value, anyhow};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// A comparison between a placeholder and a source column.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ParameterBinding {
    /// `schema.table` on cross-app sessions, the bare table name on single-app surfaces, or the
    /// CTE/subquery name when the column does not come straight from a table.
    pub table: String,
    pub column: String,
    /// `eq`, `neq`, `lt`, `lte`, `gt`, `gte`, `like`, `ilike` or `in`, read as
    /// `column <operator> $parameter`.
    pub operator: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SqlParameter {
    /// The placeholder name without its leading `$`.
    pub name: String,
    /// The Arrow type the statement fixes for the placeholder, when it fixes one.
    pub data_type: Option<String>,
    pub bindings: Vec<ParameterBinding>,
}

fn unwrapped(expr: &Expr) -> &Expr {
    match expr {
        Expr::Cast(cast) => unwrapped(&cast.expr),
        Expr::TryCast(cast) => unwrapped(&cast.expr),
        Expr::Alias(alias) => unwrapped(&alias.expr),
        other => other,
    }
}

fn placeholder(expr: &Expr) -> Option<String> {
    match unwrapped(expr) {
        Expr::Placeholder(placeholder) => Some(
            placeholder
                .id
                .strip_prefix('$')
                .unwrap_or(&placeholder.id)
                .to_string(),
        ),
        _ => None,
    }
}

fn operator_name(op: Operator, flipped: bool) -> Option<&'static str> {
    Some(match (op, flipped) {
        (Operator::Eq, _) => "eq",
        (Operator::NotEq, _) => "neq",
        (Operator::Lt, false) | (Operator::Gt, true) => "lt",
        (Operator::LtEq, false) | (Operator::GtEq, true) => "lte",
        (Operator::Gt, false) | (Operator::Lt, true) => "gt",
        (Operator::GtEq, false) | (Operator::LtEq, true) => "gte",
        (Operator::LikeMatch, _) => "like",
        (Operator::ILikeMatch, _) => "ilike",
        _ => return None,
    })
}

/// Where the columns of a plan come from: alias or table name → `schema.table`, plus every
/// scanned table with its column names, for unqualified columns.
struct Lineage {
    aliases: HashMap<String, String>,
    scans: Vec<(String, BTreeSet<String>)>,
}

impl Lineage {
    fn of(plan: &LogicalPlan) -> Result<Self> {
        let mut aliases = HashMap::new();
        let mut scans = Vec::new();
        plan.apply_with_subqueries(|node| {
            match node {
                LogicalPlan::TableScan(scan) => {
                    let name = scan.table_name.to_string();
                    aliases.insert(name.clone(), name.clone());
                    aliases.insert(scan.table_name.table().to_string(), name.clone());
                    let columns = scan
                        .source
                        .schema()
                        .fields()
                        .iter()
                        .map(|field| field.name().clone())
                        .collect();
                    scans.push((name, columns));
                }
                LogicalPlan::SubqueryAlias(alias) => {
                    if let LogicalPlan::TableScan(scan) = alias.input.as_ref() {
                        aliases.insert(alias.alias.to_string(), scan.table_name.to_string());
                    }
                }
                _ => {}
            }
            Ok(TreeNodeRecursion::Continue)
        })?;
        Ok(Self { aliases, scans })
    }

    fn table_of(&self, column: &datafusion::common::Column) -> Option<String> {
        if let Some(relation) = &column.relation {
            let relation = relation.to_string();
            return Some(self.aliases.get(&relation).cloned().unwrap_or(relation));
        }
        let mut owners = self
            .scans
            .iter()
            .filter(|(_, columns)| columns.contains(&column.name));
        let owner = owners.next()?;
        owners.next().is_none().then(|| owner.0.clone())
    }
}

fn column_of(expr: &Expr) -> Option<&datafusion::common::Column> {
    match unwrapped(expr) {
        Expr::Column(column) => Some(column),
        _ => None,
    }
}

fn bindings_of(
    expr: &Expr,
    lineage: &Lineage,
    out: &mut BTreeMap<String, BTreeSet<ParameterBinding>>,
) {
    let mut bind = |name: String, column: &datafusion::common::Column, operator: &str| {
        if let Some(table) = lineage.table_of(column) {
            out.entry(name).or_default().insert(ParameterBinding {
                table,
                column: column.name.clone(),
                operator: operator.to_string(),
            });
        }
    };
    match expr {
        Expr::BinaryExpr(BinaryExpr { left, op, right }) => {
            if let (Some(column), Some(name)) = (column_of(left), placeholder(right))
                && let Some(operator) = operator_name(*op, false)
            {
                bind(name, column, operator);
            } else if let (Some(name), Some(column)) = (placeholder(left), column_of(right))
                && let Some(operator) = operator_name(*op, true)
            {
                bind(name, column, operator);
            }
        }
        Expr::Like(Like {
            expr,
            pattern,
            case_insensitive,
            ..
        }) => {
            if let (Some(column), Some(name)) = (column_of(expr), placeholder(pattern)) {
                bind(
                    name,
                    column,
                    if *case_insensitive { "ilike" } else { "like" },
                );
            }
        }
        Expr::InList(InList { expr, list, .. }) => {
            if let Some(column) = column_of(expr) {
                for item in list {
                    if let Some(name) = placeholder(item) {
                        bind(name, column, "eq");
                    }
                }
            }
        }
        Expr::Between(Between {
            expr, low, high, ..
        }) => {
            if let Some(column) = column_of(expr) {
                if let Some(name) = placeholder(low) {
                    bind(name, column, "gte");
                }
                if let Some(name) = placeholder(high) {
                    bind(name, column, "lte");
                }
            }
        }
        Expr::ScalarFunction(ScalarFunction { func, args })
            if matches!(func.name(), "array_has" | "list_has" | "array_contains")
                && args.len() == 2 =>
        {
            if let (Some(name), Some(column)) = (placeholder(&args[0]), column_of(&args[1])) {
                bind(name, column, "in");
            }
        }
        _ => {}
    }
}

/// Every placeholder of `plan` with its inferred type and the columns it is compared with.
/// Placeholders appear once, in name order; one used only in a projection has no bindings.
pub fn describe_parameters(plan: &LogicalPlan) -> Result<Vec<SqlParameter>> {
    let fields = plan.get_parameter_fields()?;
    let lineage = Lineage::of(plan)?;
    let mut bindings: BTreeMap<String, BTreeSet<ParameterBinding>> = BTreeMap::new();
    plan.apply_with_subqueries(|node| {
        node.apply_expressions(|expr| {
            expr.apply(|inner| {
                bindings_of(inner, &lineage, &mut bindings);
                Ok(TreeNodeRecursion::Continue)
            })
        })
    })?;
    let mut parameters: BTreeMap<String, SqlParameter> = fields
        .into_iter()
        .map(|(id, field)| {
            let name = id.strip_prefix('$').unwrap_or(&id).to_string();
            let parameter = SqlParameter {
                name: name.clone(),
                data_type: field.map(|field| field.data_type().to_string()),
                bindings: Vec::new(),
            };
            (name, parameter)
        })
        .collect();
    for (name, found) in bindings {
        let parameter = parameters
            .entry(name.clone())
            .or_insert_with(|| SqlParameter {
                name,
                data_type: None,
                bindings: Vec::new(),
            });
        parameter.bindings = found.into_iter().collect();
    }
    Ok(parameters.into_values().collect())
}

/// Distinct values a parameter options statement lists at most.
pub const PARAMETER_OPTION_VALUES: usize = 200;
/// Longest identifier or search text an options statement takes.
const MAX_OPTION_NAME_CHARS: usize = 256;

/// The statement listing what a parameter compared with `table.column` can take.
#[derive(Debug, Clone, PartialEq)]
pub struct ParameterOptionsQuery {
    pub sql: String,
    pub params: Value,
    pub rows: usize,
}

fn quote_option_name(identifier: &str) -> Result<String> {
    if identifier.is_empty()
        || identifier.contains('\0')
        || identifier.chars().count() > MAX_OPTION_NAME_CHARS
    {
        return Err(anyhow!("'{identifier}' is not a valid identifier"));
    }
    Ok(format!("\"{}\"", identifier.replace('"', "\"\"")))
}

/// `schema.table` or `table`, quoted part by part.
fn quote_option_table(table: &str) -> Result<String> {
    match table.split_once('.') {
        Some((schema, name)) => Ok(format!(
            "{}.{}",
            quote_option_name(schema)?,
            quote_option_name(name)?
        )),
        None => quote_option_name(table),
    }
}

/// Up to [`PARAMETER_OPTION_VALUES`] distinct non-null values of `table.column`, only those
/// containing `search` when it is given, or the column's minimum and maximum with `range`. Every
/// identifier part is quoted, so no name can change the statement.
pub fn parameter_options_query(
    table: &str,
    column: &str,
    search: Option<&str>,
    range: bool,
) -> Result<ParameterOptionsQuery> {
    let source = quote_option_table(table)?;
    let field = quote_option_name(column)?;
    if range {
        return Ok(ParameterOptionsQuery {
            sql: format!("SELECT MIN({field}) AS \"min\", MAX({field}) AS \"max\" FROM {source}"),
            params: Value::Null,
            rows: 1,
        });
    }
    let search = search.filter(|text| !text.is_empty());
    if search.is_some_and(|text| text.chars().count() > MAX_OPTION_NAME_CHARS) {
        return Err(anyhow!(
            "The search text is longer than {MAX_OPTION_NAME_CHARS} characters"
        ));
    }
    let filter = if search.is_some() {
        format!(" AND strpos(lower(CAST({field} AS VARCHAR)), lower($__search)) > 0")
    } else {
        String::new()
    };
    Ok(ParameterOptionsQuery {
        sql: format!(
            "SELECT DISTINCT {field} AS \"value\" FROM {source} \
             WHERE {field} IS NOT NULL{filter} ORDER BY 1"
        ),
        params: search.map_or(Value::Null, |text| json!({ "__search": text })),
        rows: PARAMETER_OPTION_VALUES,
    })
}

/// The rows of a [`parameter_options_query`] as answered: `{values, truncated}`, or the
/// `{min, max}` row for a range.
pub fn parameter_options_answer(result: SqlQueryResult, range: bool) -> Value {
    if range {
        return result.rows.into_iter().next().unwrap_or(Value::Null);
    }
    json!({
        "values": result
            .rows
            .into_iter()
            .map(|row| row.get("value").cloned().unwrap_or(Value::Null))
            .collect::<Vec<_>>(),
        "truncated": result.truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::datatypes::{DataType, Field, Schema, TimeUnit};
    use datafusion::datasource::MemTable;
    use datafusion::prelude::SessionContext;
    use std::sync::Arc;

    async fn plan(sql: &str) -> LogicalPlan {
        let ctx = SessionContext::new();
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("region", DataType::Utf8, true),
            Field::new("total", DataType::Float64, true),
            Field::new(
                "placed",
                DataType::Timestamp(TimeUnit::Microsecond, None),
                true,
            ),
        ]));
        ctx.register_table(
            "orders",
            Arc::new(MemTable::try_new(schema, vec![vec![]]).unwrap()),
        )
        .unwrap();
        ctx.sql(sql).await.unwrap().logical_plan().clone()
    }

    fn binding(table: &str, column: &str, operator: &str) -> ParameterBinding {
        ParameterBinding {
            table: table.to_string(),
            column: column.to_string(),
            operator: operator.to_string(),
        }
    }

    fn find<'a>(parameters: &'a [SqlParameter], name: &str) -> &'a SqlParameter {
        parameters
            .iter()
            .find(|parameter| parameter.name == name)
            .unwrap_or_else(|| panic!("no parameter {name} in {parameters:?}"))
    }

    #[tokio::test]
    async fn placeholders_resolve_through_aliases_to_typed_columns() {
        let parameters = describe_parameters(
            &plan(
                "SELECT o.region, SUM(o.total) AS total FROM orders o \
                 WHERE o.region = $region AND o.total >= $min AND $max >= o.total \
                 AND o.placed BETWEEN $from AND $to GROUP BY o.region",
            )
            .await,
        )
        .unwrap();

        let region = find(&parameters, "region");
        assert_eq!(region.data_type.as_deref(), Some("Utf8"));
        assert_eq!(region.bindings, vec![binding("orders", "region", "eq")]);
        let min = find(&parameters, "min");
        assert_eq!(min.data_type.as_deref(), Some("Float64"));
        assert_eq!(min.bindings, vec![binding("orders", "total", "gte")]);
        assert_eq!(
            find(&parameters, "max").bindings,
            vec![binding("orders", "total", "lte")],
            "a placeholder on the left reads as the column compared with it"
        );
        assert_eq!(
            find(&parameters, "from").bindings,
            vec![binding("orders", "placed", "gte")]
        );
        assert_eq!(
            find(&parameters, "to").bindings,
            vec![binding("orders", "placed", "lte")]
        );
        assert_eq!(
            parameters
                .iter()
                .map(|parameter| parameter.name.as_str())
                .collect::<Vec<_>>(),
            vec!["from", "max", "min", "region", "to"]
        );
    }

    #[tokio::test]
    async fn lists_patterns_and_unbound_placeholders() {
        let parameters = describe_parameters(
            &plan(
                "SELECT $label AS label, id FROM orders \
                 WHERE id IN ($first, $second) AND region LIKE $pattern",
            )
            .await,
        )
        .unwrap();
        assert_eq!(
            find(&parameters, "first").bindings,
            vec![binding("orders", "id", "eq")]
        );
        assert_eq!(
            find(&parameters, "second").bindings,
            vec![binding("orders", "id", "eq")]
        );
        assert_eq!(
            find(&parameters, "pattern").bindings,
            vec![binding("orders", "region", "like")]
        );
        assert!(find(&parameters, "label").bindings.is_empty());
    }

    #[test]
    fn option_statements_quote_every_identifier_part() {
        let distinct =
            parameter_options_query("crm.customers", "re\"gion", Some("we"), false).unwrap();
        assert_eq!(
            distinct.sql,
            "SELECT DISTINCT \"re\"\"gion\" AS \"value\" FROM \"crm\".\"customers\" \
             WHERE \"re\"\"gion\" IS NOT NULL AND strpos(lower(CAST(\"re\"\"gion\" AS VARCHAR)), \
             lower($__search)) > 0 ORDER BY 1"
        );
        assert_eq!(distinct.params, json!({ "__search": "we" }));
        assert_eq!(distinct.rows, PARAMETER_OPTION_VALUES);

        let unfiltered = parameter_options_query("orders", "region", Some(""), false).unwrap();
        assert!(!unfiltered.sql.contains("strpos"));
        assert_eq!(unfiltered.params, Value::Null);

        let range = parameter_options_query("orders", "total", None, true).unwrap();
        assert_eq!(
            range.sql,
            "SELECT MIN(\"total\") AS \"min\", MAX(\"total\") AS \"max\" FROM \"orders\""
        );
        assert_eq!(range.rows, 1);

        assert!(parameter_options_query("crm.", "region", None, false).is_err());
        assert!(parameter_options_query("orders", "", None, false).is_err());
        assert!(parameter_options_query("orders", "a\0b", None, false).is_err());
        let long = "s".repeat(MAX_OPTION_NAME_CHARS + 1);
        assert!(parameter_options_query("orders", "region", Some(&long), false).is_err());
        assert!(parameter_options_query("orders", &long, None, false).is_err());
    }

    #[test]
    fn option_answers_list_values_or_the_range_row() {
        let result = |rows: Vec<Value>, truncated| SqlQueryResult {
            columns: Vec::new(),
            row_count: rows.len(),
            rows,
            truncated,
        };
        assert_eq!(
            parameter_options_answer(
                result(
                    vec![json!({ "value": "East" }), json!({ "value": "West" })],
                    true
                ),
                false
            ),
            json!({ "values": ["East", "West"], "truncated": true })
        );
        assert_eq!(
            parameter_options_answer(result(vec![json!({ "min": 1, "max": 9 })], false), true),
            json!({ "min": 1, "max": 9 })
        );
    }
}
