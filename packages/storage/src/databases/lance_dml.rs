//! SQL `UPDATE`/`DELETE` support for Lance tables registered into DataFusion.
//!
//! DataFusion hands DML to the table provider as logical [`Expr`]s
//! (`delete_from(filters)`, `update(assignments, filters)`), while LanceDB's
//! mutation API takes SQL *strings* parsed by lance's own filter parser. The
//! translator here unparses the conservative subset both sides agree on and
//! hard-errors on everything else — on a DELETE, a mistranslated predicate
//! destroys data instead of returning wrong rows.
//!
//! Lance-parser contract the rendering targets (lance-datafusion 4.0):
//! - Identifiers are backtick-delimited; `"double quotes"` are string LITERALS.
//! - Strings escape `'` by doubling; backslash is not an escape.
//! - Temporal literals are typed: `timestamp(p) 'YYYY-MM-DD HH:MM:SS[.f]'`
//!   (p = 0/3/6/9), `date 'YYYY-MM-DD'`, `decimal(p,s) '1.23'`. Bare ints or
//!   strings do NOT coerce to temporal columns.
//! - Timezones are rejected by lance's type parser, so tz-aware timestamp
//!   literals are rendered as their UTC wall clock: arrow timestamps are
//!   epoch-relative instants and casts between tz variants preserve the value,
//!   so the comparison stays exact.
//!
//! Two DataFusion planning behaviors shape the API:
//! - The optimizer folds constant predicates before the provider is called:
//!   no WHERE, `WHERE true` and `WHERE false` ALL arrive as an empty filter
//!   list. An empty list is therefore refused rather than treated as
//!   "all rows" — otherwise `WHERE false` (e.g. a folded `$parameter`) would
//!   silently rewrite the whole table.
//! - EXPLAIN invokes `delete_from`/`update` at plan time, so the mutation runs
//!   inside [`LanceDmlExec::execute`], never in the plan constructors.

use std::any::Any;
use std::sync::Arc;

use arrow_array::{RecordBatch, UInt64Array};
use arrow_schema::{DataType, Field, Schema as ArrowSchema, SchemaRef, TimeUnit};
use datafusion::common::{DataFusionError, Result as DataFusionResult, ScalarValue};
use datafusion::execution::{SendableRecordBatchStream, TaskContext};
use datafusion::logical_expr::{Expr, Operator};
use datafusion::physical_expr::EquivalenceProperties;
use datafusion::physical_plan::execution_plan::{Boundedness, EmissionType};
use datafusion::physical_plan::stream::RecordBatchStreamAdapter;
use datafusion::physical_plan::{
    DisplayAs, DisplayFormatType, ExecutionPlan, Partitioning, PlanProperties,
};
use lancedb::Table;

fn plan_err<T>(message: String) -> DataFusionResult<T> {
    Err(DataFusionError::Plan(message))
}

/// Renders the WHERE clause of an UPDATE/DELETE as a lance predicate string.
/// The filters are the AND-split conjuncts DataFusion extracted; an empty list
/// is refused (see module docs — it is indistinguishable from `WHERE false`).
pub fn filters_to_lance_predicate(
    filters: &[Expr],
    schema: &SchemaRef,
) -> DataFusionResult<String> {
    if filters.is_empty() {
        return plan_err(
            "UPDATE/DELETE on a Lance table requires a WHERE clause referencing at least one \
             column. Constant conditions (no WHERE, WHERE true, WHERE false or a folded \
             $parameter) are optimized away before they reach the table and cannot be told \
             apart, so they are refused instead of writing the whole table. To affect every \
             row, use the dedicated database update/delete nodes with an explicit `true` filter."
                .to_string(),
        );
    }
    let rendered = filters
        .iter()
        .map(|filter| render_expr(filter, schema))
        .collect::<DataFusionResult<Vec<_>>>()?;
    Ok(rendered.join(" AND "))
}

/// Renders UPDATE assignments as `(column, lance SQL expression)` pairs for
/// `UpdateBuilder::column`. Targets must be real columns of this table —
/// qualifiers were already stripped, so a foreign column smuggled in by
/// `UPDATE … FROM` or a rewritten subquery only shows up as an unknown name.
/// Column names are passed through raw: lance resolves them as schema keys,
/// not as SQL (backticks there would fail to match).
pub fn assignments_to_lance_updates(
    assignments: &[(String, Expr)],
    schema: &SchemaRef,
) -> DataFusionResult<Vec<(String, String)>> {
    let mut updates = Vec::with_capacity(assignments.len());
    for (column, value) in assignments {
        ensure_column(schema, column)?;
        // DataFusion already drops `SET a = a`; drop any that slip through so
        // lance does not rewrite an untouched column.
        if let Expr::Column(source) = value
            && source.name == *column
        {
            continue;
        }
        updates.push((column.clone(), render_expr(value, schema)?));
    }
    if updates.is_empty() {
        return plan_err(
            "UPDATE contains no effective assignments (every SET clause assigns a column to \
             itself)"
                .to_string(),
        );
    }
    Ok(updates)
}

fn ensure_column(schema: &SchemaRef, name: &str) -> DataFusionResult<()> {
    if schema.field_with_name(name).is_err() {
        return plan_err(format!(
            "Column '{name}' does not exist on this Lance table. UPDATE/DELETE can only \
             reference the table's own columns; UPDATE … FROM other tables and subqueries \
             are not supported."
        ));
    }
    Ok(())
}

/// Backtick-quotes an identifier for lance's filter parser, where `"col"` is a
/// string literal (see `filter_identifier` in the graph module for the same rule).
fn quote_column(name: &str) -> String {
    format!("`{}`", name.replace('`', "``"))
}

fn sql_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn operator_token(op: &Operator) -> DataFusionResult<&'static str> {
    Ok(match op {
        Operator::Eq => "=",
        Operator::NotEq => "!=",
        Operator::Lt => "<",
        Operator::LtEq => "<=",
        Operator::Gt => ">",
        Operator::GtEq => ">=",
        Operator::And => "AND",
        Operator::Or => "OR",
        Operator::Plus => "+",
        Operator::Minus => "-",
        Operator::Multiply => "*",
        Operator::Divide => "/",
        Operator::Modulo => "%",
        Operator::StringConcat => "||",
        other => {
            return plan_err(format!(
                "Operator '{other}' is not supported in Lance UPDATE/DELETE statements"
            ));
        }
    })
}

fn render_expr(expr: &Expr, schema: &SchemaRef) -> DataFusionResult<String> {
    match expr {
        Expr::Alias(alias) => render_expr(&alias.expr, schema),
        Expr::Column(column) => {
            ensure_column(schema, &column.name)?;
            Ok(quote_column(&column.name))
        }
        Expr::Literal(value, _) => render_scalar(value),
        Expr::BinaryExpr(binary) => {
            let left = render_expr(&binary.left, schema)?;
            let right = render_expr(&binary.right, schema)?;
            Ok(format!("({left} {} {right})", operator_token(&binary.op)?))
        }
        Expr::Not(inner) => Ok(format!("(NOT {})", render_expr(inner, schema)?)),
        Expr::Negative(inner) => Ok(format!("(- {})", render_expr(inner, schema)?)),
        Expr::IsNull(inner) => Ok(format!("({} IS NULL)", render_expr(inner, schema)?)),
        Expr::IsNotNull(inner) => Ok(format!("({} IS NOT NULL)", render_expr(inner, schema)?)),
        // UNKNOWN is the boolean NULL, and lance's parser has no IS UNKNOWN.
        Expr::IsUnknown(inner) => Ok(format!("({} IS NULL)", render_expr(inner, schema)?)),
        Expr::IsNotUnknown(inner) => Ok(format!("({} IS NOT NULL)", render_expr(inner, schema)?)),
        Expr::IsTrue(inner) => Ok(format!("({} IS TRUE)", render_expr(inner, schema)?)),
        Expr::IsFalse(inner) => Ok(format!("({} IS FALSE)", render_expr(inner, schema)?)),
        Expr::IsNotTrue(inner) => Ok(format!("({} IS NOT TRUE)", render_expr(inner, schema)?)),
        Expr::IsNotFalse(inner) => Ok(format!("({} IS NOT FALSE)", render_expr(inner, schema)?)),
        Expr::InList(in_list) => {
            let target = render_expr(&in_list.expr, schema)?;
            let values = in_list
                .list
                .iter()
                .map(|item| render_expr(item, schema))
                .collect::<DataFusionResult<Vec<_>>>()?;
            let negated = if in_list.negated { " NOT" } else { "" };
            Ok(format!("({target}{negated} IN ({}))", values.join(", ")))
        }
        Expr::Between(between) => {
            let target = render_expr(&between.expr, schema)?;
            let low = render_expr(&between.low, schema)?;
            let high = render_expr(&between.high, schema)?;
            let negated = if between.negated { " NOT" } else { "" };
            Ok(format!("({target}{negated} BETWEEN {low} AND {high})"))
        }
        Expr::Like(like) => {
            let target = render_expr(&like.expr, schema)?;
            let pattern = render_expr(&like.pattern, schema)?;
            let negated = if like.negated { " NOT" } else { "" };
            let operator = if like.case_insensitive {
                "ILIKE"
            } else {
                "LIKE"
            };
            let escape = match like.escape_char {
                Some(escape_char) => format!(" ESCAPE {}", sql_string(&escape_char.to_string())),
                None => String::new(),
            };
            Ok(format!("({target}{negated} {operator} {pattern}{escape})"))
        }
        Expr::Cast(cast) => {
            let inner = render_expr(&cast.expr, schema)?;
            Ok(format!(
                "CAST({inner} AS {})",
                cast_type_name(&cast.data_type)?
            ))
        }
        other => plan_err(format!(
            "Expression '{other}' is not supported in Lance UPDATE/DELETE statements. \
             Supported: column references, literals, comparisons, AND/OR/NOT, arithmetic, \
             IS [NOT] NULL/TRUE/FALSE, IN, BETWEEN, LIKE and CAST."
        )),
    }
}

/// Type names accepted by lance's `parse_type` for CAST targets.
fn cast_type_name(data_type: &DataType) -> DataFusionResult<String> {
    Ok(match data_type {
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => "string".to_string(),
        DataType::Boolean => "boolean".to_string(),
        DataType::Int8 => "tinyint".to_string(),
        DataType::Int16 => "smallint".to_string(),
        DataType::Int32 => "int".to_string(),
        DataType::Int64 => "bigint".to_string(),
        DataType::UInt8 => "tinyint unsigned".to_string(),
        DataType::UInt16 => "smallint unsigned".to_string(),
        DataType::UInt32 => "int unsigned".to_string(),
        DataType::UInt64 => "bigint unsigned".to_string(),
        DataType::Float32 => "float".to_string(),
        DataType::Float64 => "double".to_string(),
        DataType::Date32 => "date".to_string(),
        DataType::Binary | DataType::LargeBinary => "binary".to_string(),
        DataType::Timestamp(unit, None) => format!("timestamp({})", timestamp_precision(unit)),
        DataType::Decimal128(precision, scale) if *scale >= 0 => {
            format!("decimal({precision},{scale})")
        }
        other => {
            return plan_err(format!(
                "CAST to '{other}' is not supported in Lance UPDATE/DELETE statements"
            ));
        }
    })
}

fn timestamp_precision(unit: &TimeUnit) -> u8 {
    match unit {
        TimeUnit::Second => 0,
        TimeUnit::Millisecond => 3,
        TimeUnit::Microsecond => 6,
        TimeUnit::Nanosecond => 9,
    }
}

fn render_scalar(value: &ScalarValue) -> DataFusionResult<String> {
    if value.is_null() {
        return Ok("NULL".to_string());
    }
    Ok(match value {
        ScalarValue::Boolean(Some(v)) => v.to_string(),
        ScalarValue::Int8(Some(v)) => v.to_string(),
        ScalarValue::Int16(Some(v)) => v.to_string(),
        ScalarValue::Int32(Some(v)) => v.to_string(),
        ScalarValue::Int64(Some(v)) => v.to_string(),
        ScalarValue::UInt8(Some(v)) => v.to_string(),
        ScalarValue::UInt16(Some(v)) => v.to_string(),
        ScalarValue::UInt32(Some(v)) => v.to_string(),
        // Lance parses numbers as i64 before falling back to f64; a value past
        // i64::MAX would round through the float path and match wrong rows.
        ScalarValue::UInt64(Some(v)) if *v <= i64::MAX as u64 => v.to_string(),
        ScalarValue::Float32(Some(v)) if v.is_finite() => format!("{v:?}"),
        ScalarValue::Float64(Some(v)) if v.is_finite() => format!("{v:?}"),
        ScalarValue::Utf8(Some(v))
        | ScalarValue::LargeUtf8(Some(v))
        | ScalarValue::Utf8View(Some(v)) => sql_string(v),
        ScalarValue::TimestampSecond(Some(v), _) => render_timestamp(*v, TimeUnit::Second)?,
        ScalarValue::TimestampMillisecond(Some(v), _) => {
            render_timestamp(*v, TimeUnit::Millisecond)?
        }
        ScalarValue::TimestampMicrosecond(Some(v), _) => {
            render_timestamp(*v, TimeUnit::Microsecond)?
        }
        ScalarValue::TimestampNanosecond(Some(v), _) => render_timestamp(*v, TimeUnit::Nanosecond)?,
        ScalarValue::Date32(Some(days)) => render_date32(*days)?,
        ScalarValue::Decimal128(Some(v), precision, scale) if *scale >= 0 => {
            render_decimal(*v, *precision, *scale)
        }
        other => {
            return plan_err(format!(
                "Literal '{other}' (type {}) is not supported in Lance UPDATE/DELETE statements",
                other.data_type()
            ));
        }
    })
}

/// Renders an arrow timestamp as a lance typed literal. Arrow timestamps are
/// epoch-relative instants whether or not a timezone is attached, and the tz
/// only changes display — so the UTC wall clock is exact for both flavors,
/// and lance's tz-less parser accepts it.
fn render_timestamp(value: i64, unit: TimeUnit) -> DataFusionResult<String> {
    let (seconds, nanoseconds, format) = match unit {
        TimeUnit::Second => (value, 0, "%Y-%m-%d %H:%M:%S"),
        TimeUnit::Millisecond => (
            value.div_euclid(1_000),
            (value.rem_euclid(1_000) * 1_000_000) as u32,
            "%Y-%m-%d %H:%M:%S%.3f",
        ),
        TimeUnit::Microsecond => (
            value.div_euclid(1_000_000),
            (value.rem_euclid(1_000_000) * 1_000) as u32,
            "%Y-%m-%d %H:%M:%S%.6f",
        ),
        TimeUnit::Nanosecond => (
            value.div_euclid(1_000_000_000),
            value.rem_euclid(1_000_000_000) as u32,
            "%Y-%m-%d %H:%M:%S%.9f",
        ),
    };
    chrono::DateTime::from_timestamp(seconds, nanoseconds)
        .map(|dt| {
            format!(
                "timestamp({}) '{}'",
                timestamp_precision(&unit),
                dt.format(format)
            )
        })
        .ok_or_else(|| {
            DataFusionError::Plan(format!(
                "Timestamp value {value} ({unit:?}) is out of the renderable range"
            ))
        })
}

fn render_date32(days: i32) -> DataFusionResult<String> {
    let epoch = chrono::DateTime::UNIX_EPOCH.date_naive();
    chrono::TimeDelta::try_days(days as i64)
        .and_then(|offset| epoch.checked_add_signed(offset))
        .map(|date| format!("date '{}'", date.format("%Y-%m-%d")))
        .ok_or_else(|| {
            DataFusionError::Plan(format!(
                "Date value {days} (days since epoch) is out of the renderable range"
            ))
        })
}

fn render_decimal(value: i128, precision: u8, scale: i8) -> String {
    let sign = if value < 0 { "-" } else { "" };
    let magnitude = value.unsigned_abs();
    let base = 10u128.pow(scale as u32);
    let digits = if scale == 0 {
        magnitude.to_string()
    } else {
        format!(
            "{}.{:0width$}",
            magnitude / base,
            magnitude % base,
            width = scale as usize
        )
    };
    format!("decimal({precision},{scale}) '{sign}{digits}'")
}

/// What [`LanceDmlExec`] runs when the plan executes.
#[derive(Debug, Clone)]
pub enum LanceDmlOp {
    Delete {
        predicate: String,
    },
    Update {
        predicate: String,
        assignments: Vec<(String, String)>,
    },
}

/// Executes one Lance mutation and emits DataFusion's DML result shape: a
/// single row with a non-nullable UInt64 `count` of affected rows. The
/// mutation happens inside `execute()` — plan construction must stay
/// side-effect-free because EXPLAIN builds (but never runs) this plan.
#[derive(Debug)]
pub struct LanceDmlExec {
    table: Table,
    op: LanceDmlOp,
    schema: SchemaRef,
    properties: PlanProperties,
}

impl LanceDmlExec {
    pub fn new(table: Table, op: LanceDmlOp) -> Self {
        let schema: SchemaRef = Arc::new(ArrowSchema::new(vec![Field::new(
            "count",
            DataType::UInt64,
            false,
        )]));
        let properties = PlanProperties::new(
            EquivalenceProperties::new(schema.clone()),
            Partitioning::UnknownPartitioning(1),
            EmissionType::Final,
            Boundedness::Bounded,
        );
        Self {
            table,
            op,
            schema,
            properties,
        }
    }
}

impl DisplayAs for LanceDmlExec {
    fn fmt_as(&self, _: DisplayFormatType, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.op {
            LanceDmlOp::Delete { predicate } => {
                write!(f, "LanceDmlExec: DELETE WHERE {predicate}")
            }
            LanceDmlOp::Update {
                predicate,
                assignments,
            } => {
                let columns = assignments
                    .iter()
                    .map(|(column, _)| column.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(f, "LanceDmlExec: UPDATE SET [{columns}] WHERE {predicate}")
            }
        }
    }
}

impl ExecutionPlan for LanceDmlExec {
    fn name(&self) -> &str {
        "LanceDmlExec"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn properties(&self) -> &PlanProperties {
        &self.properties
    }

    fn children(&self) -> Vec<&Arc<dyn ExecutionPlan>> {
        vec![]
    }

    fn with_new_children(
        self: Arc<Self>,
        children: Vec<Arc<dyn ExecutionPlan>>,
    ) -> DataFusionResult<Arc<dyn ExecutionPlan>> {
        if children.is_empty() {
            Ok(self)
        } else {
            Err(DataFusionError::Internal(
                "LanceDmlExec takes no children".to_string(),
            ))
        }
    }

    fn execute(
        &self,
        partition: usize,
        _context: Arc<TaskContext>,
    ) -> DataFusionResult<SendableRecordBatchStream> {
        if partition != 0 {
            return Err(DataFusionError::Internal(format!(
                "LanceDmlExec has a single partition, got request for partition {partition}"
            )));
        }
        let table = self.table.clone();
        let op = self.op.clone();
        let schema = self.schema.clone();
        let stream = futures::stream::once(async move {
            let count = match op {
                LanceDmlOp::Delete { predicate } => {
                    table
                        .delete(&predicate)
                        .await
                        .map_err(|e| {
                            DataFusionError::External(
                                format!("Lance DELETE (predicate: {predicate}) failed: {e}").into(),
                            )
                        })?
                        .num_deleted_rows
                }
                LanceDmlOp::Update {
                    predicate,
                    assignments,
                } => {
                    let mut update = table.update().only_if(predicate.clone());
                    for (column, value) in assignments {
                        update = update.column(column, value);
                    }
                    update
                        .execute()
                        .await
                        .map_err(|e| {
                            DataFusionError::External(
                                format!("Lance UPDATE (predicate: {predicate}) failed: {e}").into(),
                            )
                        })?
                        .rows_updated
                }
            };
            RecordBatch::try_new(schema, vec![Arc::new(UInt64Array::from(vec![count]))])
                .map_err(DataFusionError::from)
        });
        Ok(Box::pin(RecordBatchStreamAdapter::new(
            self.schema.clone(),
            stream,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use datafusion::prelude::{col, lit};

    fn schema() -> SchemaRef {
        Arc::new(ArrowSchema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("name", DataType::Utf8, true),
            Field::new("flag", DataType::Boolean, true),
            Field::new("score", DataType::Float64, true),
            Field::new(
                "created_at",
                DataType::Timestamp(TimeUnit::Microsecond, None),
                true,
            ),
            Field::new("weird `col`", DataType::Utf8, true),
        ]))
    }

    #[test]
    fn refuses_empty_filters() {
        let error = filters_to_lance_predicate(&[], &schema()).unwrap_err();
        assert!(error.to_string().contains("requires a WHERE clause"));
    }

    #[test]
    fn renders_basic_comparisons_and_strings() {
        let filters = vec![
            col("id").gt(lit(10i64)),
            col("name").eq(lit("O'Brien")),
            col("flag"),
        ];
        assert_eq!(
            filters_to_lance_predicate(&filters, &schema()).unwrap(),
            "(`id` > 10) AND (`name` = 'O''Brien') AND `flag`"
        );
    }

    #[test]
    fn renders_backticked_identifiers() {
        let weird = Expr::Column(datafusion::common::Column::new_unqualified("weird `col`"));
        let filters = vec![weird.is_not_null()];
        assert_eq!(
            filters_to_lance_predicate(&filters, &schema()).unwrap(),
            "(`weird ``col``` IS NOT NULL)"
        );
    }

    #[test]
    fn renders_in_list_not_and_arithmetic() {
        let filters = vec![
            col("id").in_list(vec![lit(1i64), lit(2i64), lit(3i64)], true),
            (col("score") + lit(1.5f64)).lt_eq(lit(10.0f64)),
            Expr::Not(Box::new(col("flag"))),
        ];
        assert_eq!(
            filters_to_lance_predicate(&filters, &schema()).unwrap(),
            "(`id` NOT IN (1, 2, 3)) AND ((`score` + 1.5) <= 10.0) AND (NOT `flag`)"
        );
    }

    #[test]
    fn renders_timestamps_as_typed_literals() {
        // 2021-01-01T00:00:00.123456 UTC in microseconds.
        let ts = ScalarValue::TimestampMicrosecond(Some(1_609_459_200_123_456), None);
        let filters = vec![col("created_at").lt(Expr::Literal(ts, None))];
        assert_eq!(
            filters_to_lance_predicate(&filters, &schema()).unwrap(),
            "(`created_at` < timestamp(6) '2021-01-01 00:00:00.123456')"
        );

        let aware = ScalarValue::TimestampSecond(Some(1_609_459_200), Some("UTC".into()));
        assert_eq!(
            render_scalar(&aware).unwrap(),
            "timestamp(0) '2021-01-01 00:00:00'"
        );
    }

    #[test]
    fn renders_dates_and_decimals() {
        assert_eq!(
            render_scalar(&ScalarValue::Date32(Some(20_675))).unwrap(),
            "date '2026-08-10'"
        );
        assert_eq!(
            render_scalar(&ScalarValue::Decimal128(Some(-1_238), 9, 3)).unwrap(),
            "decimal(9,3) '-1.238'"
        );
        assert_eq!(
            render_scalar(&ScalarValue::Decimal128(Some(42), 5, 0)).unwrap(),
            "decimal(5,0) '42'"
        );
    }

    #[test]
    fn renders_typed_nulls_as_null() {
        assert_eq!(render_scalar(&ScalarValue::Utf8(None)).unwrap(), "NULL");
        assert_eq!(render_scalar(&ScalarValue::Null).unwrap(), "NULL");
    }

    #[test]
    fn refuses_unsafe_literals() {
        assert!(render_scalar(&ScalarValue::UInt64(Some(u64::MAX))).is_err());
        assert!(render_scalar(&ScalarValue::Float64(Some(f64::NAN))).is_err());
    }

    #[test]
    fn refuses_unknown_columns_and_exotic_expressions() {
        let unknown = filters_to_lance_predicate(&[col("other_y").eq(lit(1i64))], &schema());
        assert!(unknown.unwrap_err().to_string().contains("does not exist"));

        let case = Expr::Case(datafusion::logical_expr::Case {
            expr: None,
            when_then_expr: vec![(Box::new(lit(true)), Box::new(lit(1i64)))],
            else_expr: None,
        });
        assert!(filters_to_lance_predicate(&[case], &schema()).is_err());
    }

    #[test]
    fn assignments_render_and_drop_identities() {
        let assignments = vec![
            ("name".to_string(), lit("renamed")),
            ("score".to_string(), col("score") * lit(2.0f64)),
            ("flag".to_string(), col("flag")),
        ];
        let updates = assignments_to_lance_updates(&assignments, &schema()).unwrap();
        assert_eq!(
            updates,
            vec![
                ("name".to_string(), "'renamed'".to_string()),
                ("score".to_string(), "(`score` * 2.0)".to_string()),
            ]
        );

        let identity_only = vec![("flag".to_string(), col("flag"))];
        assert!(assignments_to_lance_updates(&identity_only, &schema()).is_err());

        let foreign = vec![("other_y".to_string(), lit(1i64))];
        assert!(assignments_to_lance_updates(&foreign, &schema()).is_err());
    }
}
