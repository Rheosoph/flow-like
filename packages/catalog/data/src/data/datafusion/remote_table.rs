//! Table providers over warehouses that answer one SQL statement at a time (BigQuery, Athena).
//! A scan sends its projection, the filters the remote evaluates exactly as DataFusion would,
//! and its limit as one statement, then streams the result pages. DataFusion applies whatever
//! was not pushed down. Remotes return every value as text; [`ValueKind`] decodes it.
#![cfg_attr(not(all(feature = "bigquery", feature = "athena")), allow(dead_code))]

use base64::Engine;
use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Timelike};
use flow_like_storage::arrow::array::{
    ArrayRef, BinaryArray, BooleanArray, Date32Array, Decimal128Array, Decimal256Array,
    Float32Array, Float64Array, Int8Array, Int16Array, Int32Array, Int64Array, StringArray,
    Time64MicrosecondArray, TimestampMicrosecondArray,
};
use flow_like_storage::arrow::compute::kernels::cast_utils::parse_decimal;
use flow_like_storage::arrow::datatypes::{
    DataType, Decimal128Type, Decimal256Type, Field, Schema, SchemaRef, TimeUnit,
};
use flow_like_storage::arrow::record_batch::{RecordBatch, RecordBatchOptions};
use flow_like_storage::datafusion::catalog::{Session, TableProvider};
use flow_like_storage::datafusion::common::{DataFusionError, Result as DFResult, ScalarValue};
use flow_like_storage::datafusion::execution::{SendableRecordBatchStream, TaskContext};
use flow_like_storage::datafusion::logical_expr::expr::{Between, BinaryExpr, InList, Like};
use flow_like_storage::datafusion::logical_expr::{
    Expr, Operator, TableProviderFilterPushDown, TableType,
};
use flow_like_storage::datafusion::physical_expr::EquivalenceProperties;
use flow_like_storage::datafusion::physical_plan::execution_plan::{Boundedness, EmissionType};
use flow_like_storage::datafusion::physical_plan::stream::RecordBatchStreamAdapter;
use flow_like_storage::datafusion::physical_plan::{
    DisplayAs, DisplayFormatType, ExecutionPlan, Partitioning, PlanProperties,
};
use futures::stream::BoxStream;
use futures::{StreamExt, TryStreamExt};
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

/// How long a statement may run when the caller set no deadline.
pub(crate) const DEFAULT_STATEMENT_TIMEOUT: Duration = Duration::from_secs(300);
/// Longer lists stay in DataFusion rather than growing the statement.
const MAX_IN_LIST: usize = 1_000;
/// Bounds the batches a count-only scan emits, however many rows it counted.
const MAX_COUNT_BATCHES: usize = 65_536;
const UNIX_EPOCH_DAYS_FROM_CE: i32 = 719_163;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Dialect {
    /// GoogleSQL.
    BigQuery,
    /// Athena's Trino engine.
    Trino,
}

/// How a column's values travel: the Arrow type, the text the remote renders a value as, and
/// the SQL literal of the remote type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ValueKind {
    Text,
    Boolean,
    Int8,
    Int16,
    Int32,
    Int64,
    Float32,
    Float64,
    /// Decimal128 up to precision 38, Decimal256 above.
    Decimal {
        precision: u8,
        scale: i8,
    },
    /// `YYYY-MM-DD`.
    Date,
    /// `HH:MM:SS[.ffffff]`, microseconds.
    Time,
    /// `YYYY-MM-DD[ T]HH:MM:SS[.f]`, with a zone suffix when `utc`; microseconds.
    Timestamp {
        utc: bool,
    },
    /// Integer microseconds since the epoch, a UTC instant.
    EpochMicros,
    /// Base64 text.
    Binary,
}

impl ValueKind {
    pub(crate) fn data_type(self) -> DataType {
        match self {
            Self::Text => DataType::Utf8,
            Self::Boolean => DataType::Boolean,
            Self::Int8 => DataType::Int8,
            Self::Int16 => DataType::Int16,
            Self::Int32 => DataType::Int32,
            Self::Int64 => DataType::Int64,
            Self::Float32 => DataType::Float32,
            Self::Float64 => DataType::Float64,
            Self::Decimal { precision, scale } if precision <= 38 => {
                DataType::Decimal128(precision, scale)
            }
            Self::Decimal { precision, scale } => DataType::Decimal256(precision, scale),
            Self::Date => DataType::Date32,
            Self::Time => DataType::Time64(TimeUnit::Microsecond),
            Self::Timestamp { utc } => {
                DataType::Timestamp(TimeUnit::Microsecond, utc.then(|| Arc::from("UTC")))
            }
            Self::EpochMicros => DataType::Timestamp(TimeUnit::Microsecond, Some(Arc::from("UTC"))),
            Self::Binary => DataType::Binary,
        }
    }

    fn zoned(self) -> bool {
        matches!(self, Self::Timestamp { utc: true } | Self::EpochMicros)
    }
}

/// Which predicates on a column the remote evaluates with DataFusion's semantics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Filtering {
    /// Comparisons, IN, BETWEEN, IS [NOT] NULL, and LIKE on text.
    Exact,
    /// Arrow orders floats totally (NaN above everything, -0 below +0) where SQL engines
    /// follow IEEE, so comparisons go out widened to never drop a row DataFusion keeps, and
    /// DataFusion filters again.
    Float,
    /// Only IS [NOT] NULL: collated or padded text, binary, geography, JSON and the like.
    NullOnly,
    /// Nothing: the value is reshaped on the way out, such as a record sent as JSON.
    Never,
}

/// How a column appears in the select list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Select {
    Column,
    /// Records, arrays and maps as JSON text.
    Json,
    /// Types without an Arrow mapping, as the remote's text cast.
    Text,
    /// Binary as base64 text.
    Base64,
    /// UTC instants as integer microseconds since the epoch, exact on every result page
    /// (BigQuery's default text for TIMESTAMP is float seconds).
    EpochMicros,
}

#[derive(Clone, Debug)]
pub(crate) struct RemoteColumn {
    pub name: String,
    pub kind: ValueKind,
    pub filtering: Filtering,
    pub select: Select,
}

impl RemoteColumn {
    pub(crate) fn new(name: impl Into<String>, kind: ValueKind, filtering: Filtering) -> Self {
        Self {
            name: name.into(),
            kind,
            filtering,
            select: Select::Column,
        }
    }

    pub(crate) fn selected_as(mut self, select: Select) -> Self {
        self.select = select;
        self
    }
}

impl Dialect {
    pub(crate) fn engine(self) -> &'static str {
        match self {
            Self::BigQuery => "BigQuery",
            Self::Trino => "Athena",
        }
    }

    pub(crate) fn identifier(self, name: &str) -> DFResult<String> {
        if name.is_empty() || name.chars().any(char::is_control) {
            return Err(DataFusionError::Plan(format!(
                "{}: identifier {name:?} is empty or holds a control character",
                self.engine()
            )));
        }
        Ok(match self {
            Self::BigQuery => format!("`{}`", name.replace('\\', "\\\\").replace('`', "\\`")),
            Self::Trino => format!("\"{}\"", name.replace('"', "\"\"")),
        })
    }

    /// `catalog.schema.table`-style reference from unquoted parts.
    pub(crate) fn relation(self, parts: &[&str]) -> DFResult<String> {
        Ok(parts
            .iter()
            .map(|part| self.identifier(part))
            .collect::<DFResult<Vec<_>>>()?
            .join("."))
    }

    /// A string literal, or `None` when the dialect cannot carry the value safely.
    fn string(self, value: &str) -> Option<String> {
        match self {
            Self::BigQuery => {
                let mut literal = String::with_capacity(value.len() + 2);
                literal.push('\'');
                for c in value.chars() {
                    match c {
                        '\\' => literal.push_str("\\\\"),
                        '\'' => literal.push_str("\\'"),
                        c if c.is_control() => literal.push_str(&format!("\\u{:04x}", c as u32)),
                        c => literal.push(c),
                    }
                }
                literal.push('\'');
                Some(literal)
            }
            Self::Trino => (!value.chars().any(char::is_control))
                .then(|| format!("'{}'", value.replace('\'', "''"))),
        }
    }

    fn select(self, column: &RemoteColumn) -> DFResult<String> {
        let name = self.identifier(&column.name)?;
        Ok(match (column.select, self) {
            (Select::Column, _) => name,
            (Select::Json, Self::BigQuery) => {
                format!("IF({name} IS NULL, NULL, TO_JSON_STRING({name})) AS {name}")
            }
            (Select::Json, Self::Trino) => format!("json_format(CAST({name} AS JSON)) AS {name}"),
            (Select::Text, Self::BigQuery) => format!("CAST({name} AS STRING) AS {name}"),
            (Select::Text, Self::Trino) => format!("CAST({name} AS VARCHAR) AS {name}"),
            (Select::Base64, Self::BigQuery) => format!("TO_BASE64({name}) AS {name}"),
            (Select::Base64, Self::Trino) => format!("to_base64({name}) AS {name}"),
            (Select::EpochMicros, Self::BigQuery) => format!("UNIX_MICROS({name}) AS {name}"),
            (Select::EpochMicros, Self::Trino) => {
                return Err(DataFusionError::NotImplemented(format!(
                    "Athena column {name} cannot be read as epoch microseconds"
                )));
            }
        })
    }

    fn is_nan(self, name: &str) -> String {
        match self {
            Self::BigQuery => format!("IS_NAN({name})"),
            Self::Trino => format!("is_nan({name})"),
        }
    }

    /// `value` as a literal of the column's remote type, or `None` when it has another type,
    /// is NULL, or falls outside what the remote type holds.
    fn literal(self, kind: ValueKind, value: &ScalarValue) -> Option<String> {
        match kind {
            ValueKind::Text => self.string(text_value(value)?),
            ValueKind::Boolean => match value {
                ScalarValue::Boolean(Some(true)) => Some("TRUE".to_string()),
                ScalarValue::Boolean(Some(false)) => Some("FALSE".to_string()),
                _ => None,
            },
            ValueKind::Int8 | ValueKind::Int16 | ValueKind::Int32 | ValueKind::Int64 => {
                integer_value(value)
                    .filter(|value| (i64::MIN as i128 + 1..=i64::MAX as i128).contains(value))
                    .map(|value| value.to_string())
            }
            ValueKind::Float32 | ValueKind::Float64 => float_value(value).map(float_literal),
            ValueKind::Decimal { precision, scale } => {
                let integer = match value {
                    ScalarValue::Decimal128(Some(value), _, value_scale)
                        if *value_scale == scale =>
                    {
                        value.to_string()
                    }
                    ScalarValue::Decimal256(Some(value), _, value_scale)
                        if *value_scale == scale =>
                    {
                        value.to_string()
                    }
                    _ => return None,
                };
                let text = decimal_text(&integer, scale);
                Some(match self {
                    Self::BigQuery if precision <= 38 => format!("NUMERIC '{text}'"),
                    Self::BigQuery => format!("BIGNUMERIC '{text}'"),
                    Self::Trino => format!("DECIMAL '{text}'"),
                })
            }
            ValueKind::Date => match value {
                ScalarValue::Date32(Some(days)) => {
                    date_text(*days).map(|date| format!("DATE '{date}'"))
                }
                _ => None,
            },
            ValueKind::Time => match value {
                ScalarValue::Time64Microsecond(Some(micros)) => {
                    time_text(*micros).map(|time| format!("TIME '{time}'"))
                }
                ScalarValue::Time64Nanosecond(Some(nanos)) if nanos % 1_000 == 0 => {
                    time_text(nanos / 1_000).map(|time| format!("TIME '{time}'"))
                }
                _ => None,
            },
            ValueKind::Timestamp { .. } | ValueKind::EpochMicros => {
                let (micros, zoned) = timestamp_value(value)?;
                if zoned != kind.zoned() {
                    return None;
                }
                let text = timestamp_text(micros)?;
                Some(match (self, zoned) {
                    (Self::BigQuery, true) => format!("TIMESTAMP '{text}+00:00'"),
                    (Self::BigQuery, false) => format!("DATETIME '{text}'"),
                    (Self::Trino, true) => format!("TIMESTAMP '{text} UTC'"),
                    (Self::Trino, false) => format!("TIMESTAMP '{text}'"),
                })
            }
            ValueKind::Binary => None,
        }
    }
}

fn text_value(value: &ScalarValue) -> Option<&str> {
    match value {
        ScalarValue::Utf8(Some(text))
        | ScalarValue::LargeUtf8(Some(text))
        | ScalarValue::Utf8View(Some(text)) => Some(text),
        _ => None,
    }
}

fn integer_value(value: &ScalarValue) -> Option<i128> {
    Some(match value {
        ScalarValue::Int8(Some(value)) => *value as i128,
        ScalarValue::Int16(Some(value)) => *value as i128,
        ScalarValue::Int32(Some(value)) => *value as i128,
        ScalarValue::Int64(Some(value)) => *value as i128,
        ScalarValue::UInt8(Some(value)) => *value as i128,
        ScalarValue::UInt16(Some(value)) => *value as i128,
        ScalarValue::UInt32(Some(value)) => *value as i128,
        ScalarValue::UInt64(Some(value)) => *value as i128,
        _ => return None,
    })
}

/// Finite floats only; a single-precision value widens exactly.
fn float_value(value: &ScalarValue) -> Option<f64> {
    match value {
        ScalarValue::Float32(Some(value)) => Some(*value as f64),
        ScalarValue::Float64(Some(value)) => Some(*value),
        _ => None,
    }
    .filter(|value| value.is_finite())
}

/// Scientific notation is a double literal in both dialects (Trino reads `1.5` as DECIMAL),
/// and Rust prints the shortest text that reads back as the same double.
fn float_literal(value: f64) -> String {
    format!("{value:e}").replace('e', "E")
}

fn decimal_text(integer: &str, scale: i8) -> String {
    let (sign, digits) = match integer.strip_prefix('-') {
        Some(digits) => ("-", digits),
        None => ("", integer),
    };
    if scale <= 0 {
        let zeros = "0".repeat(scale.unsigned_abs() as usize);
        return format!("{sign}{digits}{zeros}");
    }
    let scale = scale as usize;
    let padded = format!("{digits:0>width$}", width = scale + 1);
    let (whole, fraction) = padded.split_at(padded.len() - scale);
    format!("{sign}{whole}.{fraction}")
}

fn in_sql_range(date: NaiveDate) -> bool {
    (1..=9999).contains(&date.year())
}

fn date_text(days: i32) -> Option<String> {
    let date = NaiveDate::from_num_days_from_ce_opt(days.checked_add(UNIX_EPOCH_DAYS_FROM_CE)?)?;
    in_sql_range(date).then(|| date.format("%Y-%m-%d").to_string())
}

fn time_text(micros: i64) -> Option<String> {
    if !(0..86_400_000_000).contains(&micros) {
        return None;
    }
    let time = NaiveTime::from_num_seconds_from_midnight_opt(
        (micros / 1_000_000) as u32,
        (micros % 1_000_000) as u32 * 1_000,
    )?;
    Some(time.format("%H:%M:%S%.6f").to_string())
}

/// Microseconds since the epoch and whether the value carries a zone; sub-microsecond values
/// would be rounded by the remote, so they are refused.
fn timestamp_value(value: &ScalarValue) -> Option<(i64, bool)> {
    match value {
        ScalarValue::TimestampSecond(Some(value), zone) => {
            Some((value.checked_mul(1_000_000)?, zone.is_some()))
        }
        ScalarValue::TimestampMillisecond(Some(value), zone) => {
            Some((value.checked_mul(1_000)?, zone.is_some()))
        }
        ScalarValue::TimestampMicrosecond(Some(value), zone) => Some((*value, zone.is_some())),
        ScalarValue::TimestampNanosecond(Some(value), zone) if value % 1_000 == 0 => {
            Some((value / 1_000, zone.is_some()))
        }
        _ => None,
    }
}

fn timestamp_text(micros: i64) -> Option<String> {
    let instant = DateTime::from_timestamp_micros(micros)?.naive_utc();
    in_sql_range(instant.date()).then(|| instant.format("%Y-%m-%d %H:%M:%S%.6f").to_string())
}

/// A predicate in the remote dialect. Not `exact` when the remote may keep rows the filter
/// drops; never the other way round.
#[derive(Debug, PartialEq, Eq)]
struct Pushed {
    sql: String,
    exact: bool,
}

struct Translator<'a> {
    dialect: Dialect,
    columns: &'a [RemoteColumn],
    schema: &'a Schema,
}

impl Translator<'_> {
    fn column(&self, expr: &Expr) -> Option<&RemoteColumn> {
        match expr {
            Expr::Column(column) => self
                .schema
                .index_of(&column.name)
                .ok()
                .and_then(|index| self.columns.get(index)),
            _ => None,
        }
    }

    fn name(&self, column: &RemoteColumn) -> Option<String> {
        self.dialect.identifier(&column.name).ok()
    }

    fn translate(&self, expr: &Expr) -> Option<Pushed> {
        match expr {
            Expr::BinaryExpr(BinaryExpr {
                left,
                op: Operator::And,
                right,
            }) => match (self.translate(left), self.translate(right)) {
                (Some(left), Some(right)) => Some(Pushed {
                    sql: format!("({}) AND ({})", left.sql, right.sql),
                    exact: left.exact && right.exact,
                }),
                (Some(one), None) | (None, Some(one)) => Some(Pushed {
                    exact: false,
                    ..one
                }),
                (None, None) => None,
            },
            Expr::BinaryExpr(BinaryExpr {
                left,
                op: Operator::Or,
                right,
            }) => {
                let (left, right) = (self.translate(left)?, self.translate(right)?);
                Some(Pushed {
                    sql: format!("({}) OR ({})", left.sql, right.sql),
                    exact: left.exact && right.exact,
                })
            }
            Expr::BinaryExpr(BinaryExpr { left, op, right }) => self.comparison(left, *op, right),
            Expr::Not(inner) => {
                let inner = self.translate(inner)?;
                inner.exact.then(|| Pushed {
                    sql: format!("NOT ({})", inner.sql),
                    exact: true,
                })
            }
            Expr::IsNull(inner) => self.null_check(inner, "IS NULL"),
            Expr::IsNotNull(inner) => self.null_check(inner, "IS NOT NULL"),
            Expr::Column(_) => {
                let column = self.column(expr)?;
                (column.kind == ValueKind::Boolean && column.filtering == Filtering::Exact)
                    .then(|| self.name(column))
                    .flatten()
                    .map(|sql| Pushed { sql, exact: true })
            }
            Expr::Like(like) => self.like(like),
            Expr::InList(list) => self.in_list(list),
            Expr::Between(between) => self.between(between),
            _ => None,
        }
    }

    fn comparison(&self, left: &Expr, op: Operator, right: &Expr) -> Option<Pushed> {
        let (column, op, value) = match (self.column(left), right) {
            (Some(column), Expr::Literal(value, _)) => (column, op, value),
            _ => match left {
                Expr::Literal(value, _) => (self.column(right)?, op.swap()?, value),
                _ => return None,
            },
        };
        let symbol = match op {
            Operator::Eq => "=",
            Operator::NotEq => "<>",
            Operator::Lt => "<",
            Operator::LtEq => "<=",
            Operator::Gt => ">",
            Operator::GtEq => ">=",
            _ => return None,
        };
        let name = self.name(column)?;
        let literal = self.dialect.literal(column.kind, value)?;
        match column.filtering {
            Filtering::Exact => Some(Pushed {
                sql: format!("{name} {symbol} {literal}"),
                exact: true,
            }),
            Filtering::Float => self.float_comparison(&name, op, &literal, value),
            Filtering::NullOnly | Filtering::Never => None,
        }
    }

    /// The IEEE comparison widened to a superset of Arrow's total order: NaN rows are kept for
    /// `>`, `>=` and `<>`, strict bounds include equality (signed zeros), and `<> ±0` is not
    /// sent at all, since IEEE treats -0 and +0 as equal.
    fn float_comparison(
        &self,
        name: &str,
        op: Operator,
        literal: &str,
        value: &ScalarValue,
    ) -> Option<Pushed> {
        let zero = float_value(value)? == 0.0;
        let nan = self.dialect.is_nan(name);
        let sql = match op {
            Operator::Eq => format!("{name} = {literal}"),
            Operator::NotEq if !zero => format!("({name} <> {literal} OR {nan})"),
            Operator::Gt | Operator::GtEq => format!("({name} >= {literal} OR {nan})"),
            Operator::Lt | Operator::LtEq => format!("{name} <= {literal}"),
            _ => return None,
        };
        Some(Pushed { sql, exact: false })
    }

    fn null_check(&self, inner: &Expr, check: &str) -> Option<Pushed> {
        let column = self.column(inner)?;
        if column.filtering == Filtering::Never {
            return None;
        }
        Some(Pushed {
            sql: format!("{} {check}", self.name(column)?),
            exact: true,
        })
    }

    /// Patterns with a backslash are left to DataFusion: escapes differ between the engines.
    fn like(&self, like: &Like) -> Option<Pushed> {
        if like.case_insensitive || like.escape_char.is_some() {
            return None;
        }
        let column = self.column(&like.expr)?;
        if column.kind != ValueKind::Text || column.filtering != Filtering::Exact {
            return None;
        }
        let pattern = match like.pattern.as_ref() {
            Expr::Literal(value, _) => text_value(value)?,
            _ => return None,
        };
        if pattern.contains('\\') {
            return None;
        }
        let not = if like.negated { "NOT " } else { "" };
        Some(Pushed {
            sql: format!(
                "{} {not}LIKE {}",
                self.name(column)?,
                self.dialect.string(pattern)?
            ),
            exact: true,
        })
    }

    fn in_list(&self, list: &InList) -> Option<Pushed> {
        let column = self.column(&list.expr)?;
        if list.list.is_empty() || list.list.len() > MAX_IN_LIST {
            return None;
        }
        let exact = match column.filtering {
            Filtering::Exact => true,
            Filtering::Float if !list.negated => false,
            _ => return None,
        };
        let values = list
            .list
            .iter()
            .map(|item| match item {
                Expr::Literal(value, _) => self.dialect.literal(column.kind, value),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()?;
        let not = if list.negated { "NOT " } else { "" };
        Some(Pushed {
            sql: format!("{} {not}IN ({})", self.name(column)?, values.join(", ")),
            exact,
        })
    }

    fn between(&self, between: &Between) -> Option<Pushed> {
        let column = self.column(&between.expr)?;
        let exact = match column.filtering {
            Filtering::Exact => true,
            Filtering::Float if !between.negated => false,
            _ => return None,
        };
        let bound = |expr: &Expr| match expr {
            Expr::Literal(value, _) => self.dialect.literal(column.kind, value),
            _ => None,
        };
        let (low, high) = (bound(between.low.as_ref())?, bound(between.high.as_ref())?);
        let not = if between.negated { "NOT " } else { "" };
        Some(Pushed {
            sql: format!("{} {not}BETWEEN {low} AND {high}", self.name(column)?),
            exact,
        })
    }
}

fn classify(pushed: Option<Pushed>) -> TableProviderFilterPushDown {
    match pushed {
        Some(Pushed { exact: true, .. }) => TableProviderFilterPushDown::Exact,
        Some(_) => TableProviderFilterPushDown::Inexact,
        None => TableProviderFilterPushDown::Unsupported,
    }
}

/// One statement and how its result decodes.
#[derive(Debug)]
pub(crate) struct Statement {
    pub sql: String,
    /// Kinds of the result columns, in order.
    pub kinds: Vec<ValueKind>,
    /// The schema result rows decode into.
    pub schema: SchemaRef,
    /// Set when the scan needs no columns: the statement counts the rows and the scan emits
    /// column-less batches of about this many rows.
    pub count_batch_rows: Option<usize>,
}

/// A remote table as the SQL side sees it.
#[derive(Debug)]
struct Shape {
    dialect: Dialect,
    /// The quoted table reference.
    relation: String,
    columns: Vec<RemoteColumn>,
    schema: SchemaRef,
}

impl Shape {
    fn new(dialect: Dialect, relation: String, columns: Vec<RemoteColumn>) -> Self {
        let schema = Arc::new(Schema::new(
            columns
                .iter()
                .map(|column| Field::new(&column.name, column.kind.data_type(), true))
                .collect::<Vec<_>>(),
        ));
        Self {
            dialect,
            relation,
            columns,
            schema,
        }
    }

    fn translator(&self) -> Translator<'_> {
        Translator {
            dialect: self.dialect,
            columns: &self.columns,
            schema: &self.schema,
        }
    }

    /// The statement of one scan and the schema the scan produces. A limit is sent only when
    /// every filter is exact: otherwise DataFusion still drops rows after the remote.
    fn statement(
        &self,
        projection: Option<&Vec<usize>>,
        filters: &[Expr],
        limit: Option<usize>,
        batch_rows: usize,
    ) -> DFResult<(Statement, SchemaRef)> {
        let engine = self.dialect.engine();
        let translator = self.translator();
        let mut predicates = Vec::with_capacity(filters.len());
        let mut exact = true;
        for filter in filters {
            let pushed = translator.translate(filter).ok_or_else(|| {
                DataFusionError::Internal(format!(
                    "{engine} scan received a filter it cannot send: {filter}"
                ))
            })?;
            exact &= pushed.exact;
            predicates.push(format!("({})", pushed.sql));
        }
        let relation = &self.relation;
        let condition = if predicates.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", predicates.join(" AND "))
        };
        let limit = limit.filter(|_| exact);
        let limit_clause = limit
            .map(|rows| format!(" LIMIT {rows}"))
            .unwrap_or_default();

        let indices: Vec<usize> = match projection {
            Some(projection) => projection.clone(),
            None => (0..self.columns.len()).collect(),
        };
        let output = Arc::new(self.schema.project(&indices)?);
        if indices.is_empty() {
            let sql = match limit {
                None => format!("SELECT COUNT(*) FROM {relation}{condition}"),
                Some(_) => format!(
                    "SELECT COUNT(*) FROM (SELECT 1 FROM {relation}{condition}{limit_clause}) AS limited"
                ),
            };
            let count = Arc::new(Schema::new(vec![Field::new(
                "count",
                DataType::Int64,
                true,
            )]));
            return Ok((
                Statement {
                    sql,
                    kinds: vec![ValueKind::Int64],
                    schema: count,
                    count_batch_rows: Some(batch_rows.max(1)),
                },
                output,
            ));
        }
        let mut selected = Vec::with_capacity(indices.len());
        let mut kinds = Vec::with_capacity(indices.len());
        for index in &indices {
            let column = self.columns.get(*index).ok_or_else(|| {
                DataFusionError::Internal(format!(
                    "{engine} scan projects column {index} of {}",
                    self.columns.len()
                ))
            })?;
            selected.push(self.dialect.select(column)?);
            kinds.push(column.kind);
        }
        Ok((
            Statement {
                sql: format!(
                    "SELECT {} FROM {relation}{condition}{limit_clause}",
                    selected.join(", ")
                ),
                kinds,
                schema: output.clone(),
                count_batch_rows: None,
            },
            output,
        ))
    }
}

/// Runs statements for [`RemoteTable`] scans.
pub(crate) trait RemoteSource: fmt::Debug + Send + Sync {
    /// Starts the statement once the stream is first polled and streams its rows, decoded
    /// per [`Statement::kinds`] into [`Statement::schema`]. Dropping the stream abandons it.
    fn execute(
        self: Arc<Self>,
        statement: Arc<Statement>,
    ) -> BoxStream<'static, DFResult<RecordBatch>>;
}

/// One remote table or view.
#[derive(Debug)]
pub(crate) struct RemoteTable {
    shape: Shape,
    source: Arc<dyn RemoteSource>,
}

impl RemoteTable {
    pub(crate) fn new(
        dialect: Dialect,
        relation: String,
        columns: Vec<RemoteColumn>,
        source: Arc<dyn RemoteSource>,
    ) -> Self {
        Self {
            shape: Shape::new(dialect, relation, columns),
            source,
        }
    }
}

#[async_trait::async_trait]
impl TableProvider for RemoteTable {
    fn schema(&self) -> SchemaRef {
        self.shape.schema.clone()
    }

    fn table_type(&self) -> TableType {
        TableType::Base
    }

    fn supports_filters_pushdown(
        &self,
        filters: &[&Expr],
    ) -> DFResult<Vec<TableProviderFilterPushDown>> {
        let translator = self.shape.translator();
        Ok(filters
            .iter()
            .map(|filter| classify(translator.translate(filter)))
            .collect())
    }

    async fn scan(
        &self,
        state: &dyn Session,
        projection: Option<&Vec<usize>>,
        filters: &[Expr],
        limit: Option<usize>,
    ) -> DFResult<Arc<dyn ExecutionPlan>> {
        let (statement, output) =
            self.shape
                .statement(projection, filters, limit, state.config().batch_size())?;
        Ok(Arc::new(RemoteScanExec::new(
            self.shape.dialect,
            self.source.clone(),
            Arc::new(statement),
            output,
        )))
    }
}

/// Streams the result of one statement; the remote query starts on the first poll.
#[derive(Debug)]
pub(crate) struct RemoteScanExec {
    dialect: Dialect,
    source: Arc<dyn RemoteSource>,
    statement: Arc<Statement>,
    schema: SchemaRef,
    properties: Arc<PlanProperties>,
}

impl RemoteScanExec {
    fn new(
        dialect: Dialect,
        source: Arc<dyn RemoteSource>,
        statement: Arc<Statement>,
        schema: SchemaRef,
    ) -> Self {
        let properties = Arc::new(PlanProperties::new(
            EquivalenceProperties::new(schema.clone()),
            Partitioning::UnknownPartitioning(1),
            EmissionType::Incremental,
            Boundedness::Bounded,
        ));
        Self {
            dialect,
            source,
            statement,
            schema,
            properties,
        }
    }
}

impl DisplayAs for RemoteScanExec {
    fn fmt_as(&self, _: DisplayFormatType, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "RemoteScanExec: engine={}, sql={}",
            self.dialect.engine(),
            self.statement.sql
        )
    }
}

impl ExecutionPlan for RemoteScanExec {
    fn name(&self) -> &str {
        "RemoteScanExec"
    }

    fn properties(&self) -> &Arc<PlanProperties> {
        &self.properties
    }

    fn children(&self) -> Vec<&Arc<dyn ExecutionPlan>> {
        vec![]
    }

    fn with_new_children(
        self: Arc<Self>,
        children: Vec<Arc<dyn ExecutionPlan>>,
    ) -> DFResult<Arc<dyn ExecutionPlan>> {
        if children.is_empty() {
            Ok(self)
        } else {
            Err(DataFusionError::Internal(
                "RemoteScanExec has no children".to_string(),
            ))
        }
    }

    fn execute(
        &self,
        partition: usize,
        _context: Arc<TaskContext>,
    ) -> DFResult<SendableRecordBatchStream> {
        if partition != 0 {
            return Err(DataFusionError::Internal(format!(
                "RemoteScanExec has one partition, partition {partition} was requested"
            )));
        }
        let rows = self.source.clone().execute(self.statement.clone());
        let stream = match self.statement.count_batch_rows {
            None => rows,
            Some(batch_rows) => {
                let schema = self.schema.clone();
                let engine = self.dialect.engine();
                futures::stream::once(async move {
                    let batches: Vec<RecordBatch> = rows.try_collect().await?;
                    let counted = counted_rows(&batches, engine)?;
                    Ok::<_, DataFusionError>(futures::stream::iter(column_less_batches(
                        schema, counted, batch_rows,
                    )))
                })
                .try_flatten()
                .boxed()
            }
        };
        Ok(Box::pin(RecordBatchStreamAdapter::new(
            self.schema.clone(),
            stream,
        )))
    }
}

fn counted_rows(batches: &[RecordBatch], engine: &str) -> DFResult<usize> {
    use flow_like_storage::arrow::array::Array;
    let unexpected = || {
        DataFusionError::Execution(format!(
            "{engine} answered a row count with an unexpected result"
        ))
    };
    let batch = batches
        .iter()
        .find(|batch| batch.num_rows() > 0)
        .ok_or_else(unexpected)?;
    let counts = batch
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .ok_or_else(unexpected)?;
    if counts.is_null(0) {
        return Err(unexpected());
    }
    usize::try_from(counts.value(0)).map_err(|_| unexpected())
}

/// Batches without columns that add up to `rows` rows.
fn column_less_batches(
    schema: SchemaRef,
    rows: usize,
    batch_rows: usize,
) -> impl Iterator<Item = DFResult<RecordBatch>> + Send {
    let chunk = batch_rows.max(rows.div_ceil(MAX_COUNT_BATCHES)).max(1);
    (0..rows.div_ceil(chunk)).map(move |index| {
        let size = chunk.min(rows - index * chunk);
        RecordBatch::try_new_with_options(
            schema.clone(),
            vec![],
            &RecordBatchOptions::new().with_row_count(Some(size)),
        )
        .map_err(DataFusionError::from)
    })
}

/// Decodes one page of text rows (`None` is SQL NULL) into a batch of `schema`.
pub(crate) fn text_batch(
    engine: &str,
    schema: &SchemaRef,
    kinds: &[ValueKind],
    rows: &[Vec<Option<String>>],
) -> DFResult<RecordBatch> {
    if let Some(row) = rows.iter().position(|row| row.len() != kinds.len()) {
        return Err(DataFusionError::Execution(format!(
            "{engine} returned {} values in row {row} for {} columns",
            rows[row].len(),
            kinds.len()
        )));
    }
    let mut columns = Vec::with_capacity(kinds.len());
    for (index, kind) in kinds.iter().enumerate() {
        let values = rows.iter().map(|row| row[index].as_deref());
        let column = decode_column(*kind, values).map_err(|row| {
            DataFusionError::Execution(format!(
                "{engine} returned a value in column '{}' (row {row}) that does not read as {:?}",
                schema.field(index).name(),
                kind.data_type()
            ))
        })?;
        columns.push(column);
    }
    RecordBatch::try_new_with_options(
        schema.clone(),
        columns,
        &RecordBatchOptions::new().with_row_count(Some(rows.len())),
    )
    .map_err(DataFusionError::from)
}

/// The column, or the index of the first row whose text does not parse.
fn decode_column<'a>(
    kind: ValueKind,
    values: impl Iterator<Item = Option<&'a str>>,
) -> Result<ArrayRef, usize> {
    Ok(match kind {
        ValueKind::Text => Arc::new(values.collect::<StringArray>()),
        ValueKind::Boolean => Arc::new(parsed::<BooleanArray, _>(values, parse_bool)?),
        ValueKind::Int8 => Arc::new(parsed::<Int8Array, _>(values, |text| text.parse().ok())?),
        ValueKind::Int16 => Arc::new(parsed::<Int16Array, _>(values, |text| text.parse().ok())?),
        ValueKind::Int32 => Arc::new(parsed::<Int32Array, _>(values, |text| text.parse().ok())?),
        ValueKind::Int64 => Arc::new(parsed::<Int64Array, _>(values, |text| text.parse().ok())?),
        ValueKind::Float32 => Arc::new(parsed::<Float32Array, _>(values, |text| {
            text.parse::<f32>()
                .ok()
                .map(|value| if value.is_nan() { f32::NAN } else { value })
        })?),
        ValueKind::Float64 => Arc::new(parsed::<Float64Array, _>(values, |text| {
            text.parse::<f64>()
                .ok()
                .map(|value| if value.is_nan() { f64::NAN } else { value })
        })?),
        ValueKind::Decimal { precision, scale } if precision <= 38 => Arc::new(
            parsed::<Decimal128Array, _>(values, |text| {
                parse_decimal::<Decimal128Type>(text, precision, scale).ok()
            })?
            .with_precision_and_scale(precision, scale)
            .map_err(|_| 0usize)?,
        ),
        ValueKind::Decimal { precision, scale } => Arc::new(
            parsed::<Decimal256Array, _>(values, |text| {
                parse_decimal::<Decimal256Type>(text, precision, scale).ok()
            })?
            .with_precision_and_scale(precision, scale)
            .map_err(|_| 0usize)?,
        ),
        ValueKind::Date => Arc::new(parsed::<Date32Array, _>(values, parse_date)?),
        ValueKind::Time => Arc::new(parsed::<Time64MicrosecondArray, _>(values, parse_time)?),
        ValueKind::Timestamp { utc } => Arc::new(
            parsed::<TimestampMicrosecondArray, _>(values, |text| parse_timestamp(text, utc))?
                .with_timezone_opt(utc.then_some("UTC")),
        ),
        ValueKind::EpochMicros => Arc::new(
            parsed::<TimestampMicrosecondArray, _>(values, |text| text.parse().ok())?
                .with_timezone("UTC"),
        ),
        ValueKind::Binary => Arc::new(parsed::<BinaryArray, _>(values, |text| {
            base64::engine::general_purpose::STANDARD.decode(text).ok()
        })?),
    })
}

fn parsed<'a, A, T>(
    values: impl Iterator<Item = Option<&'a str>>,
    parse: impl Fn(&str) -> Option<T>,
) -> Result<A, usize>
where
    A: FromIterator<Option<T>>,
{
    values
        .enumerate()
        .map(|(row, value)| match value {
            None => Ok(None),
            Some(text) => parse(text).map(Some).ok_or(row),
        })
        .collect()
}

fn parse_bool(text: &str) -> Option<bool> {
    if text.eq_ignore_ascii_case("true") {
        Some(true)
    } else if text.eq_ignore_ascii_case("false") {
        Some(false)
    } else {
        None
    }
}

fn parse_date(text: &str) -> Option<i32> {
    let date = NaiveDate::parse_from_str(text.trim(), "%Y-%m-%d").ok()?;
    Some(date.num_days_from_ce() - UNIX_EPOCH_DAYS_FROM_CE)
}

fn parse_time(text: &str) -> Option<i64> {
    let time = NaiveTime::parse_from_str(text.trim(), "%H:%M:%S%.f").ok()?;
    Some(time.num_seconds_from_midnight() as i64 * 1_000_000 + time.nanosecond() as i64 / 1_000)
}

fn parse_naive(text: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S%.f")
        .or_else(|_| NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f"))
        .ok()
}

/// Microseconds since the epoch. Zoned text may end in `UTC`, `Z`, an offset or an IANA zone
/// name, as Trino renders `timestamp with time zone`.
fn parse_timestamp(text: &str, utc: bool) -> Option<i64> {
    let text = text.trim();
    if !utc {
        return Some(parse_naive(text)?.and_utc().timestamp_micros());
    }
    if let Ok(instant) = DateTime::parse_from_rfc3339(text) {
        return Some(instant.timestamp_micros());
    }
    for format in [
        "%Y-%m-%d %H:%M:%S%.f %:z",
        "%Y-%m-%d %H:%M:%S%.f%:z",
        "%Y-%m-%d %H:%M:%S%.f%#z",
    ] {
        if let Ok(instant) = DateTime::parse_from_str(text, format) {
            return Some(instant.timestamp_micros());
        }
    }
    if let Some(local) = text.strip_suffix('Z') {
        return Some(parse_naive(local.trim_end())?.and_utc().timestamp_micros());
    }
    let (local, zone) = text.rsplit_once(' ')?;
    let zone: chrono_tz::Tz = zone.parse().ok()?;
    let local = parse_naive(local.trim_end())?;
    Some(
        zone.from_local_datetime(&local)
            .single()?
            .timestamp_micros(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_storage::arrow::array::{Array, AsArray};
    use flow_like_storage::arrow::datatypes::{Date32Type, Int64Type, TimestampMicrosecondType};
    use flow_like_storage::datafusion::logical_expr::{col, lit};
    use std::ops::Not;

    fn columns() -> Vec<RemoteColumn> {
        vec![
            RemoteColumn::new("id", ValueKind::Int64, Filtering::Exact),
            RemoteColumn::new("name", ValueKind::Text, Filtering::Exact),
            RemoteColumn::new("price", ValueKind::Float64, Filtering::Float),
            RemoteColumn::new("label", ValueKind::Text, Filtering::NullOnly),
            RemoteColumn::new("payload", ValueKind::Text, Filtering::Never)
                .selected_as(Select::Json),
            RemoteColumn::new("active", ValueKind::Boolean, Filtering::Exact),
            RemoteColumn::new("day", ValueKind::Date, Filtering::Exact),
        ]
    }

    fn shape(dialect: Dialect) -> Shape {
        let relation = dialect.relation(&["proj", "sales", "orders"]).unwrap();
        Shape::new(dialect, relation, columns())
    }

    fn translate(dialect: Dialect, expr: Expr) -> Option<Pushed> {
        shape(dialect).translator().translate(&expr)
    }

    fn exact(sql: &str) -> Option<Pushed> {
        Some(Pushed {
            sql: sql.to_string(),
            exact: true,
        })
    }

    fn inexact(sql: &str) -> Option<Pushed> {
        Some(Pushed {
            sql: sql.to_string(),
            exact: false,
        })
    }

    fn build(
        dialect: Dialect,
        projection: Option<Vec<usize>>,
        filters: &[Expr],
        limit: Option<usize>,
    ) -> (Statement, SchemaRef) {
        shape(dialect)
            .statement(projection.as_ref(), filters, limit, 8192)
            .unwrap()
    }

    #[test]
    fn identifiers_are_quoted_and_escaped_per_dialect() {
        assert_eq!(
            Dialect::BigQuery.identifier("a`b\\c").unwrap(),
            "`a\\`b\\\\c`"
        );
        assert_eq!(Dialect::Trino.identifier("a\"b").unwrap(), "\"a\"\"b\"");
        assert_eq!(
            Dialect::BigQuery.relation(&["my-proj", "ds", "t"]).unwrap(),
            "`my-proj`.`ds`.`t`"
        );
        assert_eq!(
            Dialect::Trino
                .relation(&["AwsDataCatalog", "db", "t"])
                .unwrap(),
            "\"AwsDataCatalog\".\"db\".\"t\""
        );
        for dialect in [Dialect::BigQuery, Dialect::Trino] {
            assert!(dialect.identifier("").is_err());
            assert!(dialect.identifier("a\nb").is_err());
        }
    }

    #[test]
    fn bigquery_timestamps_are_selected_as_epoch_micros() {
        let column = RemoteColumn::new("at", ValueKind::EpochMicros, Filtering::Exact)
            .selected_as(Select::EpochMicros);
        assert_eq!(
            Dialect::BigQuery.select(&column).unwrap(),
            "UNIX_MICROS(`at`) AS `at`"
        );
        assert!(Dialect::Trino.select(&column).is_err());
    }

    #[test]
    fn string_literals_cannot_break_out() {
        assert_eq!(
            Dialect::BigQuery.string("it's \\ a\nb").unwrap(),
            "'it\\'s \\\\ a\\u000ab'"
        );
        assert_eq!(Dialect::Trino.string("it's").unwrap(), "'it''s'");
        assert_eq!(Dialect::Trino.string("a\\b").unwrap(), "'a\\b'");
        assert_eq!(Dialect::Trino.string("a\nb"), None);
        assert_eq!(Dialect::BigQuery.string("ünï").unwrap(), "'ünï'");
    }

    #[test]
    fn literals_follow_the_remote_type() {
        let bq = Dialect::BigQuery;
        let trino = Dialect::Trino;
        assert_eq!(
            bq.literal(ValueKind::Int64, &ScalarValue::Int32(Some(-7)))
                .unwrap(),
            "-7"
        );
        assert_eq!(
            bq.literal(ValueKind::Int64, &ScalarValue::Int64(Some(i64::MIN))),
            None
        );
        assert_eq!(
            bq.literal(ValueKind::Int64, &ScalarValue::Int64(None)),
            None
        );
        assert_eq!(bq.literal(ValueKind::Int64, &ScalarValue::from("5")), None);
        assert_eq!(
            bq.literal(ValueKind::Boolean, &ScalarValue::Boolean(Some(true)))
                .unwrap(),
            "TRUE"
        );
        assert_eq!(
            bq.literal(ValueKind::Float64, &ScalarValue::Float64(Some(1.5)))
                .unwrap(),
            "1.5E0"
        );
        assert_eq!(
            trino
                .literal(ValueKind::Float32, &ScalarValue::Float32(Some(0.1)))
                .unwrap(),
            float_literal(0.1f32 as f64)
        );
        assert_eq!(
            bq.literal(ValueKind::Float64, &ScalarValue::Float64(Some(f64::NAN))),
            None
        );
        assert_eq!(
            bq.literal(
                ValueKind::Decimal {
                    precision: 38,
                    scale: 9
                },
                &ScalarValue::Decimal128(Some(-12_345), 38, 9)
            )
            .unwrap(),
            "NUMERIC '-0.000012345'"
        );
        assert_eq!(
            trino
                .literal(
                    ValueKind::Decimal {
                        precision: 10,
                        scale: 2
                    },
                    &ScalarValue::Decimal128(Some(12_345), 10, 2)
                )
                .unwrap(),
            "DECIMAL '123.45'"
        );
        assert_eq!(
            bq.literal(
                ValueKind::Decimal {
                    precision: 38,
                    scale: 9
                },
                &ScalarValue::Decimal128(Some(12_345), 10, 2)
            ),
            None
        );
        assert_eq!(
            bq.literal(ValueKind::Date, &ScalarValue::Date32(Some(19_723)))
                .unwrap(),
            "DATE '2024-01-01'"
        );
        assert_eq!(
            bq.literal(ValueKind::Date, &ScalarValue::Date32(Some(-800_000))),
            None
        );
        assert_eq!(
            bq.literal(
                ValueKind::Time,
                &ScalarValue::Time64Microsecond(Some(3_723_000_001))
            )
            .unwrap(),
            "TIME '01:02:03.000001'"
        );
    }

    #[test]
    fn timestamp_literals_keep_the_zone_semantics_of_the_column() {
        let utc =
            ScalarValue::TimestampMicrosecond(Some(1_704_067_200_000_001), Some("UTC".into()));
        let naive = ScalarValue::TimestampMillisecond(Some(1_704_067_200_123), None);
        assert_eq!(
            Dialect::BigQuery
                .literal(ValueKind::EpochMicros, &utc)
                .unwrap(),
            "TIMESTAMP '2024-01-01 00:00:00.000001+00:00'"
        );
        assert_eq!(
            Dialect::BigQuery
                .literal(ValueKind::Timestamp { utc: false }, &naive)
                .unwrap(),
            "DATETIME '2024-01-01 00:00:00.123000'"
        );
        assert_eq!(
            Dialect::Trino
                .literal(ValueKind::Timestamp { utc: true }, &utc)
                .unwrap(),
            "TIMESTAMP '2024-01-01 00:00:00.000001 UTC'"
        );
        assert_eq!(
            Dialect::Trino
                .literal(ValueKind::Timestamp { utc: false }, &naive)
                .unwrap(),
            "TIMESTAMP '2024-01-01 00:00:00.123000'"
        );
        assert_eq!(
            Dialect::Trino.literal(ValueKind::Timestamp { utc: false }, &utc),
            None
        );
        assert_eq!(
            Dialect::BigQuery.literal(ValueKind::EpochMicros, &naive),
            None
        );
        let sub_micro = ScalarValue::TimestampNanosecond(Some(1_704_067_200_000_000_001), None);
        assert_eq!(
            Dialect::Trino.literal(ValueKind::Timestamp { utc: false }, &sub_micro),
            None
        );
    }

    #[test]
    fn simple_predicates_are_exact() {
        let bq = Dialect::BigQuery;
        assert_eq!(translate(bq, col("id").gt(lit(5i64))), exact("`id` > 5"));
        assert_eq!(translate(bq, lit(5i64).lt(col("id"))), exact("`id` > 5"));
        assert_eq!(
            translate(bq, col("name").not_eq(lit("x"))),
            exact("`name` <> 'x'")
        );
        assert_eq!(
            translate(bq, col("label").is_null()),
            exact("`label` IS NULL")
        );
        assert_eq!(
            translate(bq, col("id").is_not_null()),
            exact("`id` IS NOT NULL")
        );
        assert_eq!(translate(bq, col("active")), exact("`active`"));
        assert_eq!(
            translate(bq, col("name").like(lit("a%_b"))),
            exact("`name` LIKE 'a%_b'")
        );
        assert_eq!(
            translate(Dialect::Trino, col("name").not_like(lit("a%"))),
            exact("\"name\" NOT LIKE 'a%'")
        );
        assert_eq!(
            translate(bq, col("id").in_list(vec![lit(1i64), lit(2i64)], false)),
            exact("`id` IN (1, 2)")
        );
        assert_eq!(
            translate(bq, col("name").in_list(vec![lit("a")], true)),
            exact("`name` NOT IN ('a')")
        );
        assert_eq!(
            translate(bq, col("id").between(lit(1i64), lit(9i64))),
            exact("`id` BETWEEN 1 AND 9")
        );
        assert_eq!(
            translate(bq, col("day").gt_eq(lit(ScalarValue::Date32(Some(19_723))))),
            exact("`day` >= DATE '2024-01-01'")
        );
        assert_eq!(
            translate(bq, col("id").gt(lit(1i64)).or(col("name").eq(lit("a")))),
            exact("(`id` > 1) OR (`name` = 'a')")
        );
        assert_eq!(
            translate(bq, col("id").gt(lit(1i64)).not()),
            exact("NOT (`id` > 1)")
        );
    }

    #[test]
    fn float_comparisons_are_widened_and_inexact() {
        let bq = Dialect::BigQuery;
        assert_eq!(
            translate(bq, col("price").gt(lit(2.5f64))),
            inexact("(`price` >= 2.5E0 OR IS_NAN(`price`))")
        );
        assert_eq!(
            translate(bq, col("price").lt(lit(0.0f64))),
            inexact("`price` <= 0E0")
        );
        assert_eq!(
            translate(bq, col("price").eq(lit(1.0f64))),
            inexact("`price` = 1E0")
        );
        assert_eq!(
            translate(Dialect::Trino, col("price").not_eq(lit(3.0f64))),
            inexact("(\"price\" <> 3E0 OR is_nan(\"price\"))")
        );
        assert_eq!(translate(bq, col("price").not_eq(lit(0.0f64))), None);
        assert_eq!(translate(bq, col("price").not_eq(lit(-0.0f64))), None);
        assert_eq!(
            translate(bq, col("price").in_list(vec![lit(1.0f64)], true)),
            None
        );
        assert_eq!(
            translate(bq, col("price").in_list(vec![lit(1.0f64)], false)),
            inexact("`price` IN (1E0)")
        );
        assert_eq!(translate(bq, col("price").gt(lit(1.0f64)).not()), None);
        assert_eq!(
            translate(bq, col("price").is_null()),
            exact("`price` IS NULL")
        );
    }

    #[test]
    fn untranslatable_predicates_stay_in_datafusion() {
        let bq = Dialect::BigQuery;
        assert_eq!(translate(bq, col("label").eq(lit("x"))), None);
        assert_eq!(translate(bq, col("payload").is_null()), None);
        assert_eq!(translate(bq, col("payload").eq(lit("{}"))), None);
        assert_eq!(translate(bq, col("name").like(lit("a\\%"))), None);
        assert_eq!(translate(bq, col("name").ilike(lit("a%"))), None);
        assert_eq!(translate(bq, col("id").eq(col("id"))), None);
        assert_eq!(translate(bq, col("missing").eq(lit(1i64))), None);
        assert_eq!(translate(bq, col("id").eq(lit("1"))), None);
        assert_eq!(translate(bq, (col("id") + lit(1i64)).gt(lit(2i64))), None);
        assert_eq!(
            translate(
                bq,
                col("id").in_list(vec![lit(1i64), lit(ScalarValue::Int64(None))], false)
            ),
            None
        );
        assert_eq!(
            translate(Dialect::Trino, col("name").eq(lit("line\nbreak"))),
            None
        );
        assert_eq!(
            translate(bq, col("id").gt(lit(1i64)).or(col("label").eq(lit("x")))),
            None
        );
    }

    #[test]
    fn a_conjunction_keeps_its_translatable_half_as_inexact() {
        let pushed = translate(
            Dialect::BigQuery,
            col("id").gt(lit(1i64)).and(col("label").eq(lit("x"))),
        );
        assert_eq!(pushed, inexact("`id` > 1"));
        assert_eq!(classify(pushed), TableProviderFilterPushDown::Inexact);
        assert_eq!(
            classify(translate(Dialect::BigQuery, col("id").gt(lit(1i64)))),
            TableProviderFilterPushDown::Exact
        );
        assert_eq!(
            classify(translate(Dialect::BigQuery, col("label").eq(lit("x")))),
            TableProviderFilterPushDown::Unsupported
        );
    }

    #[test]
    fn statements_carry_projection_filters_and_limit() {
        let (bq, output) = build(
            Dialect::BigQuery,
            Some(vec![1, 0, 4]),
            &[col("id").gt(lit(5i64)), col("name").like(lit("a%"))],
            Some(10),
        );
        assert_eq!(
            bq.sql,
            "SELECT `name`, `id`, IF(`payload` IS NULL, NULL, TO_JSON_STRING(`payload`)) AS `payload` \
             FROM `proj`.`sales`.`orders` WHERE (`id` > 5) AND (`name` LIKE 'a%') LIMIT 10"
        );
        assert_eq!(
            bq.kinds,
            vec![ValueKind::Text, ValueKind::Int64, ValueKind::Text]
        );
        let names: Vec<_> = output
            .fields()
            .iter()
            .map(|field| field.name().clone())
            .collect();
        assert_eq!(names, ["name", "id", "payload"]);
        assert_eq!(bq.count_batch_rows, None);

        let (trino, _) = build(Dialect::Trino, Some(vec![4, 6]), &[], None);
        assert_eq!(
            trino.sql,
            "SELECT json_format(CAST(\"payload\" AS JSON)) AS \"payload\", \"day\" FROM \"proj\".\"sales\".\"orders\""
        );
    }

    #[test]
    fn inexact_filters_drop_the_limit() {
        let (statement, _) = build(
            Dialect::BigQuery,
            Some(vec![0]),
            &[col("price").gt(lit(1.0f64))],
            Some(10),
        );
        assert_eq!(
            statement.sql,
            "SELECT `id` FROM `proj`.`sales`.`orders` WHERE ((`price` >= 1E0 OR IS_NAN(`price`)))"
        );
    }

    #[test]
    fn scans_without_columns_count_rows() {
        let (counted, output) = build(
            Dialect::BigQuery,
            Some(vec![]),
            &[col("id").gt(lit(1i64))],
            None,
        );
        assert_eq!(
            counted.sql,
            "SELECT COUNT(*) FROM `proj`.`sales`.`orders` WHERE (`id` > 1)"
        );
        assert_eq!(counted.kinds, vec![ValueKind::Int64]);
        assert_eq!(counted.count_batch_rows, Some(8192));
        assert!(output.fields().is_empty());

        let (limited, _) = build(Dialect::Trino, Some(vec![]), &[], Some(5));
        assert_eq!(
            limited.sql,
            "SELECT COUNT(*) FROM (SELECT 1 FROM \"proj\".\"sales\".\"orders\" LIMIT 5) AS limited"
        );
    }

    #[test]
    fn untranslatable_filters_in_a_scan_are_an_error() {
        let result =
            shape(Dialect::BigQuery).statement(None, &[col("label").eq(lit("x"))], None, 8192);
        assert!(result.is_err());
    }

    #[test]
    fn column_less_batches_add_up() {
        let schema = Arc::new(Schema::empty());
        let sizes: Vec<usize> = column_less_batches(schema.clone(), 20_000, 8192)
            .map(|batch| batch.unwrap().num_rows())
            .collect();
        assert_eq!(sizes, [8192, 8192, 3616]);
        assert_eq!(column_less_batches(schema.clone(), 0, 8192).count(), 0);
        assert!(column_less_batches(schema, usize::MAX / 2, 8192).count() <= MAX_COUNT_BATCHES);
    }

    #[test]
    fn text_rows_decode_into_arrow_types() {
        let kinds = [
            ValueKind::Int64,
            ValueKind::Float64,
            ValueKind::Boolean,
            ValueKind::Decimal {
                precision: 38,
                scale: 9,
            },
            ValueKind::Date,
            ValueKind::Time,
            ValueKind::Timestamp { utc: false },
            ValueKind::Timestamp { utc: true },
            ValueKind::EpochMicros,
            ValueKind::Binary,
            ValueKind::Text,
        ];
        let schema = Arc::new(Schema::new(
            kinds
                .iter()
                .enumerate()
                .map(|(index, kind)| Field::new(format!("c{index}"), kind.data_type(), true))
                .collect::<Vec<_>>(),
        ));
        let row = |values: [Option<&str>; 11]| -> Vec<Option<String>> {
            values
                .iter()
                .map(|value| value.map(str::to_string))
                .collect()
        };
        let rows = vec![
            row([
                Some("42"),
                Some("NaN"),
                Some("true"),
                Some("1.5"),
                Some("2024-01-01"),
                Some("01:02:03.5"),
                Some("2024-01-01T00:00:01.25"),
                Some("2024-01-01 00:00:00.000 UTC"),
                Some("1704067200000001"),
                Some("aGk="),
                Some(""),
            ]),
            row([None; 11]),
        ];
        let batch = text_batch("Test", &schema, &kinds, &rows).unwrap();
        assert_eq!(batch.num_rows(), 2);
        assert_eq!(batch.column(0).as_primitive::<Int64Type>().value(0), 42);
        assert!(
            batch
                .column(1)
                .as_primitive::<flow_like_storage::arrow::datatypes::Float64Type>()
                .value(0)
                .is_nan()
        );
        assert!(batch.column(2).as_boolean().value(0));
        assert_eq!(
            batch.column(3).as_primitive::<Decimal128Type>().value(0),
            1_500_000_000
        );
        assert_eq!(
            batch.column(4).as_primitive::<Date32Type>().value(0),
            19_723
        );
        assert_eq!(
            batch
                .column(6)
                .as_primitive::<TimestampMicrosecondType>()
                .value(0),
            1_704_067_201_250_000
        );
        assert_eq!(
            batch
                .column(7)
                .as_primitive::<TimestampMicrosecondType>()
                .value(0),
            1_704_067_200_000_000
        );
        assert_eq!(
            batch
                .column(8)
                .as_primitive::<TimestampMicrosecondType>()
                .value(0),
            1_704_067_200_000_001
        );
        assert_eq!(batch.column(9).as_binary::<i32>().value(0), b"hi");
        assert_eq!(batch.column(10).as_string::<i32>().value(0), "");
        for column in batch.columns() {
            assert!(column.is_null(1));
        }
        assert_eq!(
            batch.schema().field(8).data_type(),
            &DataType::Timestamp(TimeUnit::Microsecond, Some(Arc::from("UTC")))
        );
    }

    #[test]
    fn decode_errors_name_the_column_but_not_the_value() {
        let schema = Arc::new(Schema::new(vec![Field::new(
            "amount",
            DataType::Int64,
            true,
        )]));
        let rows = vec![
            vec![Some("1".to_string())],
            vec![Some("secret-ish".to_string())],
        ];
        let error = text_batch("BigQuery", &schema, &[ValueKind::Int64], &rows)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("'amount'") && error.contains("row 1"),
            "{error}"
        );
        assert!(!error.contains("secret-ish"), "{error}");
        let short = vec![vec![]];
        assert!(text_batch("BigQuery", &schema, &[ValueKind::Int64], &short).is_err());
    }

    #[test]
    fn zoned_timestamps_accept_trino_zone_names_and_offsets() {
        let expected = 1_704_067_200_000_000;
        for text in [
            "2024-01-01 00:00:00.000 UTC",
            "2024-01-01 01:00:00.000 Europe/Berlin",
            "2024-01-01 01:00:00 +01:00",
            "2024-01-01T00:00:00Z",
            "2024-01-01 00:00:00Z",
        ] {
            assert_eq!(parse_timestamp(text, true), Some(expected), "{text}");
        }
        assert_eq!(
            parse_timestamp("2024-01-01 00:00:00 Nowhere/City", true),
            None
        );
    }

    #[test]
    fn decimal_text_places_the_point() {
        assert_eq!(decimal_text("12345", 2), "123.45");
        assert_eq!(decimal_text("-5", 2), "-0.05");
        assert_eq!(decimal_text("7", 0), "7");
        assert_eq!(decimal_text("7", -2), "700");
    }
}
