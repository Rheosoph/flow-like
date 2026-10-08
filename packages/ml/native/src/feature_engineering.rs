use flow_like_ml_core::{Error, Result, content_digest, require};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    time::Instant,
};

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct FeaturePipeline {
    pub steps: Vec<FeatureStep>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FeatureStep {
    DeriveColumns {
        columns: Vec<DerivedColumn>,
    },
    WindowFeatures {
        group_by: Vec<String>,
        timestamp_column: String,
        features: Vec<WindowFeature>,
    },
    JoinSources {
        alias: String,
        keys: Vec<JoinKey>,
        join: JoinKind,
        columns: Vec<JoinColumn>,
        #[serde(default)]
        missing: MissingJoin,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct DerivedColumn {
    pub name: String,
    pub expression: Expression,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Expression {
    Number {
        value: f64,
    },
    Column {
        name: String,
    },
    Unary {
        op: UnaryOperator,
        value: Box<Expression>,
    },
    Binary {
        op: BinaryOperator,
        left: Box<Expression>,
        right: Box<Expression>,
    },
    Clip {
        value: Box<Expression>,
        minimum: f64,
        maximum: f64,
    },
    Coalesce {
        values: Vec<Expression>,
    },
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum UnaryOperator {
    Abs,
    Negate,
    Log,
    Log1p,
    Sqrt,
    Exp,
    Square,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum BinaryOperator {
    Add,
    Subtract,
    Multiply,
    SafeDivide,
    Power,
    Minimum,
    Maximum,
    Greater,
    GreaterEqual,
    Less,
    LessEqual,
    Equal,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct WindowFeature {
    pub name: String,
    pub column: String,
    pub operation: WindowOperation,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WindowOperation {
    Lag {
        steps: usize,
    },
    Delta {
        lag: usize,
    },
    Rate {
        lag: usize,
    },
    Rolling {
        statistic: RollingStatistic,
        window: WindowSize,
        min_periods: usize,
        include_current: bool,
    },
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RollingStatistic {
    Mean,
    Std,
    Rms,
    Minimum,
    Maximum,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WindowSize {
    Rows { count: usize },
    DurationMs { milliseconds: u64 },
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct JoinKey {
    pub left: String,
    pub right: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct JoinColumn {
    pub source: String,
    pub name: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum JoinKind {
    Exact,
    BackwardAsOf {
        left_timestamp: String,
        right_timestamp: String,
        #[serde(default)]
        right_available_at: Option<String>,
        #[serde(default)]
        tolerance_ms: Option<u64>,
    },
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MissingJoin {
    #[default]
    Null,
    Error,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(default)]
pub struct FeaturePolicy {
    pub allowed_input_columns: Vec<String>,
    pub forbidden_columns: Vec<String>,
    /// Keys and timestamps may drive grouping or alignment without becoming feature values.
    pub metadata_columns: Vec<String>,
    pub allowed_sources: BTreeMap<String, Vec<String>>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(default)]
pub struct FeatureLimits {
    pub maximum_rows: usize,
    pub maximum_bytes: usize,
    pub maximum_columns: usize,
    pub maximum_expression_nodes: usize,
    pub maximum_history_rows: usize,
    pub maximum_groups: usize,
    pub maximum_join_rows: usize,
    pub maximum_duration_ms: u64,
}
impl Default for FeatureLimits {
    fn default() -> Self {
        Self {
            maximum_rows: 100_000,
            maximum_bytes: 64 * 1024 * 1024,
            maximum_columns: 256,
            maximum_expression_nodes: 256,
            maximum_history_rows: 10_000,
            maximum_groups: 1_000,
            maximum_join_rows: 10_000,
            maximum_duration_ms: 30_000,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct FittedJoinSource {
    pub step_index: usize,
    pub rows: Vec<Value>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct FittedFeaturePipeline {
    pub format_version: u32,
    pub pipeline: FeaturePipeline,
    pub policy: FeaturePolicy,
    pub limits: FeatureLimits,
    pub joins: Vec<FittedJoinSource>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct FeatureStreamState {
    pub pipeline_digest: String,
    pub windows: Vec<WindowState>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct WindowState {
    pub step_index: usize,
    pub groups: BTreeMap<String, WindowGroup>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct WindowGroup {
    pub last_timestamp_ms: i64,
    pub history: VecDeque<HistoryRow>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct HistoryRow {
    pub timestamp_ms: i64,
    pub values: BTreeMap<String, Option<f64>>,
}

pub struct FeatureStream {
    fitted: FittedFeaturePipeline,
    state: FeatureStreamState,
    lookups: BTreeMap<usize, JoinLookup>,
    usage: (usize, usize, usize),
    join_bytes: usize,
}
enum JoinLookup {
    Exact(BTreeMap<String, usize>),
    AsOf(BTreeMap<String, Vec<(i64, Option<i64>, usize)>>),
}
struct Deadline {
    started: Instant,
    milliseconds: u64,
}
impl Deadline {
    fn new(limits: &FeatureLimits) -> Self {
        Self {
            started: Instant::now(),
            milliseconds: limits.maximum_duration_ms,
        }
    }
    fn check(&self) -> Result<()> {
        require(
            self.started.elapsed().as_millis() <= self.milliseconds as u128,
            "Feature transformation duration budget exhausted",
        )
    }
}

fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}
pub fn serialized_bytes(value: &impl Serialize) -> Result<usize> {
    struct Counter(usize);
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self
                .0
                .checked_add(bytes.len())
                .ok_or_else(|| std::io::Error::other("Feature serialization size overflow"))?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter(0);
    serde_json::to_writer(&mut counter, value)?;
    Ok(counter.0)
}
fn name(value: &str) -> Result<()> {
    require(
        !value.trim().is_empty() && value.len() <= 256,
        "Feature names must be nonempty and at most 256 bytes",
    )
}
fn cell<'a>(row: &'a Value, column: &str) -> Result<&'a Value> {
    row.as_object()
        .and_then(|row| row.get(column))
        .ok_or_else(|| invalid(format!("Missing feature column '{column}'")))
}
fn number(value: &Value) -> Result<Option<f64>> {
    if value.is_null() {
        return Ok(None);
    }
    let value = value
        .as_f64()
        .ok_or_else(|| invalid("Numeric features require numbers or null"))?;
    require(value.is_finite(), "Feature values must be finite")?;
    Ok(Some(value))
}
fn finite(value: f64) -> Result<Option<f64>> {
    require(
        value.is_finite(),
        "Feature arithmetic overflowed; clip or rescale its inputs",
    )?;
    Ok(Some(value))
}
fn timestamp(row: &Value, column: &str) -> Result<i64> {
    cell(row, column)?
        .as_i64()
        .ok_or_else(|| invalid(format!("Timestamp '{column}' must be integer milliseconds")))
}
fn key(row: &Value, columns: &[&str], allow_null: bool) -> Result<Option<String>> {
    let mut values = Vec::new();
    for column in columns {
        let value = cell(row, column)?;
        if value.is_null() && allow_null {
            return Ok(None);
        }
        require(
            value.is_string() || value.is_number() || value.is_boolean(),
            "Join/group keys must be non-null scalar values",
        )?;
        values.push(value);
    }
    let key = serde_json::to_string(&values)?;
    require(key.len() <= 4096, "Join/group key exceeds 4096 bytes")?;
    Ok(Some(key))
}
fn insert(row: &mut Value, column: &str, value: Value) -> Result<()> {
    let object = row
        .as_object_mut()
        .ok_or_else(|| invalid("Feature rows must be objects"))?;
    require(
        !object.contains_key(column),
        format!("Feature output '{column}' already exists"),
    )?;
    object.insert(column.into(), value);
    Ok(())
}

impl FeatureLimits {
    fn validate(&self) -> Result<()> {
        require(
            (1..=1_000_000).contains(&self.maximum_rows)
                && (1024..=1024 * 1024 * 1024).contains(&self.maximum_bytes)
                && (1..=4096).contains(&self.maximum_columns)
                && (1..=4096).contains(&self.maximum_expression_nodes)
                && (1..=1_000_000).contains(&self.maximum_history_rows)
                && (1..=100_000).contains(&self.maximum_groups)
                && self.maximum_join_rows <= 1_000_000
                && (1..=300_000).contains(&self.maximum_duration_ms),
            "Invalid feature pipeline resource limits",
        )
    }
}
fn available(column: &str, columns: &BTreeSet<String>) -> Result<()> {
    name(column)?;
    require(
        columns.contains(column),
        format!("Column '{column}' is not an allowed feature input"),
    )
}
fn output(
    column: &str,
    columns: &mut BTreeSet<String>,
    reserved: &mut BTreeSet<String>,
    limits: &FeatureLimits,
) -> Result<()> {
    name(column)?;
    require(
        reserved.insert(column.into()),
        format!("Output '{column}' is duplicate, protected, or already exists"),
    )?;
    columns.insert(column.into());
    require(
        columns.len() <= limits.maximum_columns,
        "Feature column budget exceeded",
    )
}
fn validate_expression(
    expression: &Expression,
    columns: &BTreeSet<String>,
    count: &mut usize,
    depth: usize,
    limits: &FeatureLimits,
) -> Result<()> {
    *count += 1;
    require(
        *count <= limits.maximum_expression_nodes && depth <= 64,
        "Expression node or depth budget exceeded",
    )?;
    match expression {
        Expression::Number { value } => {
            require(value.is_finite(), "Expression constants must be finite")
        }
        Expression::Column { name } => available(name, columns),
        Expression::Unary { value, .. } => {
            validate_expression(value, columns, count, depth + 1, limits)
        }
        Expression::Binary { left, right, .. } => {
            validate_expression(left, columns, count, depth + 1, limits)?;
            validate_expression(right, columns, count, depth + 1, limits)
        }
        Expression::Clip {
            value,
            minimum,
            maximum,
        } => {
            require(
                minimum.is_finite() && maximum.is_finite() && minimum <= maximum,
                "Clip bounds must be finite and ordered",
            )?;
            validate_expression(value, columns, count, depth + 1, limits)
        }
        Expression::Coalesce { values } => {
            require(!values.is_empty(), "Coalesce requires at least one value")?;
            for value in values {
                validate_expression(value, columns, count, depth + 1, limits)?;
            }
            Ok(())
        }
    }
}
impl FeaturePipeline {
    pub fn generated_columns(&self) -> Vec<String> {
        self.steps
            .iter()
            .flat_map(|step| match step {
                FeatureStep::DeriveColumns { columns } => {
                    columns.iter().map(|c| c.name.clone()).collect::<Vec<_>>()
                }
                FeatureStep::WindowFeatures { features, .. } => {
                    features.iter().map(|c| c.name.clone()).collect()
                }
                FeatureStep::JoinSources { columns, .. } => {
                    columns.iter().map(|c| c.name.clone()).collect()
                }
            })
            .collect()
    }
    pub fn validate(&self, policy: &FeaturePolicy, limits: &FeatureLimits) -> Result<()> {
        limits.validate()?;
        require(
            self.steps.len() <= 32,
            "Feature pipeline supports at most 32 ordered steps",
        )?;
        let mut columns = BTreeSet::new();
        let mut reserved = BTreeSet::new();
        let metadata: BTreeSet<_> = policy.metadata_columns.iter().cloned().collect();
        for column in policy
            .allowed_input_columns
            .iter()
            .chain(&policy.metadata_columns)
            .chain(&policy.forbidden_columns)
        {
            name(column)?;
            reserved.insert(column.clone());
        }
        for column in &policy.allowed_input_columns {
            require(
                !policy.forbidden_columns.contains(column) && columns.insert(column.clone()),
                "Allowed feature inputs must be unique and exclude protected columns",
            )?;
        }
        require(
            columns.len() <= limits.maximum_columns,
            "Allowed inputs exceed column budget",
        )?;
        let mut nodes = 0;
        for step in &self.steps {
            match step {
                FeatureStep::DeriveColumns { columns: derived } => {
                    require(!derived.is_empty(), "Derivation step is empty")?;
                    for column in derived {
                        validate_expression(&column.expression, &columns, &mut nodes, 0, limits)?;
                        output(&column.name, &mut columns, &mut reserved, limits)?;
                    }
                }
                FeatureStep::WindowFeatures {
                    group_by,
                    timestamp_column,
                    features,
                } => {
                    require(
                        !features.is_empty() && group_by.len() <= 16,
                        "Window step requires features and at most 16 group keys",
                    )?;
                    for column in group_by.iter().chain(std::iter::once(timestamp_column)) {
                        name(column)?;
                        require(
                            columns.contains(column) || metadata.contains(column),
                            format!("Window key '{column}' is not allowed metadata"),
                        )?;
                    }
                    for feature in features {
                        available(&feature.column, &columns)?;
                        match &feature.operation {
                            WindowOperation::Lag { steps }
                            | WindowOperation::Delta { lag: steps }
                            | WindowOperation::Rate { lag: steps } => require(
                                *steps > 0 && *steps <= limits.maximum_history_rows,
                                "Window lag exceeds history budget",
                            )?,
                            WindowOperation::Rolling {
                                window,
                                min_periods,
                                ..
                            } => {
                                require(
                                    *min_periods > 0 && *min_periods <= limits.maximum_history_rows,
                                    "Rolling minimum periods exceed history budget",
                                )?;
                                match window {
                                    WindowSize::Rows { count } => require(
                                        *count > 0
                                            && *count <= limits.maximum_history_rows
                                            && min_periods <= count,
                                        "Rolling row count is invalid",
                                    )?,
                                    WindowSize::DurationMs { milliseconds } => require(
                                        *milliseconds > 0 && *milliseconds <= i64::MAX as u64,
                                        "Rolling duration is invalid",
                                    )?,
                                }
                            }
                        }
                    }
                    for feature in features {
                        output(&feature.name, &mut columns, &mut reserved, limits)?;
                    }
                }
                FeatureStep::JoinSources {
                    alias,
                    keys,
                    join,
                    columns: selected,
                    ..
                } => {
                    name(alias)?;
                    let source = policy.allowed_sources.get(alias).ok_or_else(|| {
                        invalid(format!("Join source '{alias}' is not authorized"))
                    })?;
                    require(
                        !selected.is_empty() && keys.len() <= 16,
                        "Join requires selected outputs and at most 16 keys",
                    )?;
                    require(
                        !keys.is_empty() || matches!(join, JoinKind::BackwardAsOf { .. }),
                        "Exact joins need at least one key",
                    )?;
                    let mut right_keys = BTreeSet::new();
                    let mut left_keys = BTreeSet::new();
                    for item in keys {
                        require(
                            (columns.contains(&item.left) || metadata.contains(&item.left))
                                && source.contains(&item.right),
                            "Join key is not authorized",
                        )?;
                        require(
                            left_keys.insert(&item.left) && right_keys.insert(&item.right),
                            "Join keys must be unique",
                        )?;
                    }
                    if let JoinKind::BackwardAsOf {
                        left_timestamp,
                        right_timestamp,
                        right_available_at,
                        tolerance_ms,
                    } = join
                    {
                        require(
                            columns.contains(left_timestamp) || metadata.contains(left_timestamp),
                            "As-of left timestamp is not authorized",
                        )?;
                        require(
                            source.contains(right_timestamp)
                                && right_available_at
                                    .as_ref()
                                    .is_none_or(|column| source.contains(column)),
                            "As-of source timestamp is not authorized",
                        )?;
                        require(
                            tolerance_ms.is_none_or(|value| value <= i64::MAX as u64),
                            "As-of tolerance exceeds supported clock range",
                        )?;
                    }
                    for column in selected {
                        require(
                            source.contains(&column.source)
                                && !policy.forbidden_columns.contains(&column.source),
                            "Join output column is not authorized",
                        )?;
                        output(&column.name, &mut columns, &mut reserved, limits)?;
                    }
                }
            }
        }
        Ok(())
    }
    pub fn fit(
        &self,
        sources: &BTreeMap<String, Vec<Value>>,
        policy: &FeaturePolicy,
        limits: &FeatureLimits,
    ) -> Result<FittedFeaturePipeline> {
        self.validate(policy, limits)?;
        let deadline = Deadline::new(limits);
        let mut joins = Vec::new();
        let mut total = 0usize;
        let mut bytes = 0usize;
        for (step_index, step) in self.steps.iter().enumerate() {
            let FeatureStep::JoinSources {
                alias,
                keys,
                join,
                columns,
                ..
            } = step
            else {
                continue;
            };
            let rows = sources
                .get(alias)
                .ok_or_else(|| invalid(format!("Missing declared join source '{alias}'")))?;
            total = total
                .checked_add(rows.len())
                .ok_or_else(|| invalid("Join row count overflows"))?;
            require(
                total <= limits.maximum_join_rows,
                "Join lookup row budget exceeded",
            )?;
            let mut selected: BTreeSet<_> = keys
                .iter()
                .map(|key| key.right.as_str())
                .chain(columns.iter().map(|column| column.source.as_str()))
                .collect();
            if let JoinKind::BackwardAsOf {
                right_timestamp,
                right_available_at,
                ..
            } = join
            {
                selected.insert(right_timestamp);
                if let Some(column) = right_available_at {
                    selected.insert(column);
                }
            }
            let projected = rows
                .iter()
                .map(|row| -> Result<Value> {
                    deadline.check()?;
                    let mut projected_bytes = 2usize;
                    for (index, column) in selected.iter().enumerate() {
                        let value = cell(row, column)?;
                        require(
                            value.is_null()
                                || value.is_number()
                                || value.is_string()
                                || value.is_boolean(),
                            "Joined values must be scalars or null",
                        )?;
                        projected_bytes = projected_bytes
                            .checked_add(serialized_bytes(column)?)
                            .and_then(|size| size.checked_add(1 + usize::from(index > 0)))
                            .and_then(|size| size.checked_add(serialized_bytes(value).ok()?))
                            .ok_or_else(|| invalid("Join byte count overflows"))?;
                    }
                    bytes = bytes
                        .checked_add(projected_bytes)
                        .ok_or_else(|| invalid("Join byte count overflows"))?;
                    require(
                        bytes <= limits.maximum_bytes,
                        "Fitted join lookup exceeds byte budget",
                    )?;
                    let projected = Value::Object(
                        selected
                            .iter()
                            .map(|column| Ok((column.to_string(), cell(row, column)?.clone())))
                            .collect::<Result<_>>()?,
                    );
                    Ok(projected)
                })
                .collect::<Result<Vec<_>>>()?;
            build_lookup(step, &projected)?;
            joins.push(FittedJoinSource {
                step_index,
                rows: projected,
            });
        }
        let fitted = FittedFeaturePipeline {
            format_version: 1,
            pipeline: self.clone(),
            policy: policy.clone(),
            limits: limits.clone(),
            joins,
        };
        fitted.validate()?;
        Ok(fitted)
    }
}
fn evaluate(expression: &Expression, row: &Value) -> Result<Option<f64>> {
    match expression {
        Expression::Number { value } => finite(*value),
        Expression::Column { name } => number(cell(row, name)?),
        Expression::Coalesce { values } => {
            for value in values {
                if let Some(value) = evaluate(value, row)? {
                    return Ok(Some(value));
                }
            }
            Ok(None)
        }
        Expression::Clip {
            value,
            minimum,
            maximum,
        } => Ok(evaluate(value, row)?.map(|value| value.clamp(*minimum, *maximum))),
        Expression::Unary { op, value } => {
            let Some(value) = evaluate(value, row)? else {
                return Ok(None);
            };
            finite(match op {
                UnaryOperator::Abs => value.abs(),
                UnaryOperator::Negate => -value,
                UnaryOperator::Log => {
                    if value <= 0. {
                        return Ok(None);
                    }
                    value.ln()
                }
                UnaryOperator::Log1p => {
                    if value <= -1. {
                        return Ok(None);
                    }
                    value.ln_1p()
                }
                UnaryOperator::Sqrt => {
                    if value < 0. {
                        return Ok(None);
                    }
                    value.sqrt()
                }
                UnaryOperator::Exp => value.exp(),
                UnaryOperator::Square => value * value,
            })
        }
        Expression::Binary { op, left, right } => {
            let (Some(left), Some(right)) = (evaluate(left, row)?, evaluate(right, row)?) else {
                return Ok(None);
            };
            finite(match op {
                BinaryOperator::Add => left + right,
                BinaryOperator::Subtract => left - right,
                BinaryOperator::Multiply => left * right,
                BinaryOperator::SafeDivide => {
                    if right == 0. {
                        return Ok(None);
                    }
                    left / right
                }
                BinaryOperator::Power => left.powf(right),
                BinaryOperator::Minimum => left.min(right),
                BinaryOperator::Maximum => left.max(right),
                BinaryOperator::Greater => f64::from(u8::from(left > right)),
                BinaryOperator::GreaterEqual => f64::from(u8::from(left >= right)),
                BinaryOperator::Less => f64::from(u8::from(left < right)),
                BinaryOperator::LessEqual => f64::from(u8::from(left <= right)),
                BinaryOperator::Equal => f64::from(u8::from(left == right)),
            })
        }
    }
}
fn build_lookup(step: &FeatureStep, rows: &[Value]) -> Result<JoinLookup> {
    let FeatureStep::JoinSources { keys, join, .. } = step else {
        return Err(invalid("Lookup is attached to a non-join step"));
    };
    let columns: Vec<_> = keys.iter().map(|key| key.right.as_str()).collect();
    match join {
        JoinKind::Exact => {
            let mut lookup = BTreeMap::new();
            for (index, row) in rows.iter().enumerate() {
                let key = key(row, &columns, false)?.unwrap();
                require(
                    lookup.insert(key, index).is_none(),
                    "Exact join source has duplicate keys; row multiplication is forbidden",
                )?;
            }
            Ok(JoinLookup::Exact(lookup))
        }
        JoinKind::BackwardAsOf {
            right_timestamp,
            right_available_at,
            ..
        } => {
            let mut lookup: BTreeMap<String, Vec<_>> = BTreeMap::new();
            for (index, row) in rows.iter().enumerate() {
                let key = key(row, &columns, false)?.unwrap();
                let time = timestamp(row, right_timestamp)?;
                let available = right_available_at
                    .as_ref()
                    .map(|column| timestamp(row, column))
                    .transpose()?;
                require(
                    available.is_none_or(|available| available >= time),
                    "Join source availability precedes its event timestamp",
                )?;
                lookup
                    .entry(key)
                    .or_default()
                    .push((time, available, index));
            }
            for values in lookup.values_mut() {
                values.sort_by_key(|value| value.0);
                require(
                    values.windows(2).all(|pair| pair[0].0 < pair[1].0),
                    "As-of source contains ambiguous duplicate key/timestamp rows",
                )?;
            }
            Ok(JoinLookup::AsOf(lookup))
        }
    }
}

fn statistic(values: &[f64], kind: RollingStatistic) -> Result<Option<f64>> {
    if values.is_empty() {
        return Ok(None);
    }
    let n = values.len() as f64;
    let mean = values.iter().map(|value| value / n).sum::<f64>();
    finite(match kind {
        RollingStatistic::Mean => mean,
        RollingStatistic::Minimum => values.iter().copied().fold(f64::INFINITY, f64::min),
        RollingStatistic::Maximum => values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        RollingStatistic::Std => {
            let deviations = values.iter().map(|value| value - mean).collect::<Vec<_>>();
            let scale = deviations
                .iter()
                .map(|value| value.abs())
                .fold(0., f64::max);
            if scale == 0. {
                0.
            } else {
                scale
                    * (deviations
                        .iter()
                        .map(|value| (value / scale).powi(2) / n)
                        .sum::<f64>())
                    .sqrt()
            }
        }
        RollingStatistic::Rms => {
            let scale = values.iter().map(|value| value.abs()).fold(0., f64::max);
            if scale == 0. {
                0.
            } else {
                scale
                    * (values
                        .iter()
                        .map(|value| (value / scale).powi(2) / n)
                        .sum::<f64>())
                    .sqrt()
            }
        }
    })
}
fn window_row(
    row: &mut Value,
    group_by: &[String],
    timestamp_column: &str,
    features: &[WindowFeature],
    previous: Option<&WindowGroup>,
) -> Result<(String, WindowGroup)> {
    let group_key = key(
        row,
        &group_by.iter().map(String::as_str).collect::<Vec<_>>(),
        false,
    )?
    .unwrap();
    let now = timestamp(row, timestamp_column)?;
    if let Some(previous) = previous {
        require(
            now > previous.last_timestamp_ms,
            "Window timestamps must strictly increase within each group",
        )?;
    }
    let mut group = previous.cloned().unwrap_or(WindowGroup {
        last_timestamp_ms: now,
        history: VecDeque::new(),
    });
    let columns: BTreeSet<_> = features
        .iter()
        .map(|feature| feature.column.as_str())
        .collect();
    let current = HistoryRow {
        timestamp_ms: now,
        values: columns
            .iter()
            .map(|column| Ok((column.to_string(), number(cell(row, column)?)?)))
            .collect::<Result<_>>()?,
    };
    let mut retained_rows = 0usize;
    let mut retained_duration = 0u64;
    for feature in features {
        let present = current.values[&feature.column];
        let value = match &feature.operation {
            WindowOperation::Lag { steps }
            | WindowOperation::Delta { lag: steps }
            | WindowOperation::Rate { lag: steps } => {
                retained_rows = retained_rows.max(*steps);
                let before = group
                    .history
                    .len()
                    .checked_sub(*steps)
                    .and_then(|index| group.history.get(index));
                match (
                    before,
                    before.and_then(|row| row.values[&feature.column]),
                    present,
                ) {
                    (_, value, _) if matches!(feature.operation, WindowOperation::Lag { .. }) => {
                        value
                    }
                    (Some(before), Some(value), Some(present)) => {
                        let delta = present - value;
                        finite(
                            if matches!(feature.operation, WindowOperation::Rate { .. }) {
                                delta / ((now as i128 - before.timestamp_ms as i128) as f64 / 1000.)
                            } else {
                                delta
                            },
                        )?
                    }
                    _ => None,
                }
            }
            WindowOperation::Rolling {
                statistic: kind,
                window,
                min_periods,
                include_current,
            } => {
                let mut values = match window {
                    WindowSize::Rows { count } => {
                        let preceding = count.saturating_sub(usize::from(*include_current));
                        retained_rows = retained_rows.max(preceding);
                        group
                            .history
                            .iter()
                            .rev()
                            .take(preceding)
                            .filter_map(|row| row.values[&feature.column])
                            .collect::<Vec<_>>()
                    }
                    WindowSize::DurationMs { milliseconds } => {
                        retained_duration = retained_duration.max(*milliseconds);
                        let start = now as i128 - *milliseconds as i128;
                        group
                            .history
                            .iter()
                            .filter(|row| row.timestamp_ms as i128 >= start)
                            .filter_map(|row| row.values[&feature.column])
                            .collect()
                    }
                };
                if *include_current {
                    values.extend(present);
                }
                if values.len() < *min_periods {
                    None
                } else {
                    statistic(&values, *kind)?
                }
            }
        };
        insert(row, &feature.name, serde_json::to_value(value)?)?;
    }
    group.last_timestamp_ms = now;
    group.history.push_back(current);
    while group.history.len() > retained_rows {
        let keep_for_duration = retained_duration > 0
            && group.history.front().is_some_and(|row| {
                row.timestamp_ms as i128 >= now as i128 - retained_duration as i128
            });
        if keep_for_duration {
            break;
        }
        group.history.pop_front();
    }
    Ok((group_key, group))
}
fn group_usage(key: &str, group: &WindowGroup) -> (usize, usize, usize) {
    (
        1,
        group.history.len(),
        key.len()
            + 64
            + group
                .history
                .iter()
                .map(|row| {
                    64 + row
                        .values
                        .keys()
                        .map(|column| column.len() + 64)
                        .sum::<usize>()
                })
                .sum::<usize>(),
    )
}
fn add_usage(total: &mut (usize, usize, usize), amount: (usize, usize, usize)) -> Result<()> {
    total.0 = total
        .0
        .checked_add(amount.0)
        .ok_or_else(|| invalid("Window group count overflow"))?;
    total.1 = total
        .1
        .checked_add(amount.1)
        .ok_or_else(|| invalid("Window row count overflow"))?;
    total.2 = total
        .2
        .checked_add(amount.2)
        .ok_or_else(|| invalid("Window state size overflow"))?;
    Ok(())
}
fn check_usage(usage: (usize, usize, usize), limits: &FeatureLimits) -> Result<()> {
    require(
        usage.0 <= limits.maximum_groups
            && usage.1 <= limits.maximum_history_rows
            && usage.2 <= limits.maximum_bytes,
        "Window state exceeds group, history or byte budget; shorten the window or increase the declared budget",
    )
}
fn check_capacity(joins: usize, state: usize, output: usize, limits: &FeatureLimits) -> Result<()> {
    let total = joins
        .checked_add(state)
        .and_then(|sum| sum.checked_add(output));
    require(
        total.is_some_and(|total| total <= limits.maximum_bytes),
        "Fitted joins, window state and transformed rows exceed the shared byte budget",
    )
}
fn replace_group_usage(
    mut usage: (usize, usize, usize),
    key: &str,
    previous: Option<&WindowGroup>,
    next: (usize, usize, usize),
) -> Result<(usize, usize, usize)> {
    if let Some(previous) = previous {
        let old = group_usage(key, previous);
        usage.0 -= old.0;
        usage.1 -= old.1;
        usage.2 -= old.2;
    }
    add_usage(&mut usage, next)?;
    Ok(usage)
}
fn next_window_usage(
    key: &str,
    now: i64,
    features: &[WindowFeature],
    previous: Option<&WindowGroup>,
) -> Result<(usize, usize, usize)> {
    if let Some(previous) = previous {
        require(
            now > previous.last_timestamp_ms,
            "Window timestamps must strictly increase within each group",
        )?;
    }
    let mut rows = 0;
    let mut duration = 0;
    for feature in features {
        match &feature.operation {
            WindowOperation::Lag { steps }
            | WindowOperation::Delta { lag: steps }
            | WindowOperation::Rate { lag: steps } => rows = rows.max(*steps),
            WindowOperation::Rolling {
                window: WindowSize::Rows { count },
                include_current,
                ..
            } => rows = rows.max(count.saturating_sub(usize::from(*include_current))),
            WindowOperation::Rolling {
                window: WindowSize::DurationMs { milliseconds },
                ..
            } => duration = duration.max(*milliseconds),
        }
    }
    let prior = previous.map_or(0, |group| group.history.len());
    let temporal = if duration == 0 {
        0
    } else {
        1 + previous.map_or(0, |group| {
            group
                .history
                .iter()
                .rev()
                .take_while(|row| row.timestamp_ms as i128 >= now as i128 - duration as i128)
                .count()
        })
    };
    let retained = rows.min(prior + 1).max(temporal);
    let columns: BTreeSet<_> = features.iter().map(|feature| &feature.column).collect();
    let row_bytes = 64
        + columns
            .iter()
            .map(|column| column.len() + 64)
            .sum::<usize>();
    Ok((1, retained, key.len() + 64 + retained * row_bytes))
}
fn row_bounds(row: &Value, limits: &FeatureLimits) -> Result<usize> {
    require(
        row.as_object()
            .is_some_and(|row| row.len() <= limits.maximum_columns),
        "Feature rows must be objects within the column budget",
    )?;
    let bytes = serialized_bytes(row)?;
    require(
        bytes <= limits.maximum_bytes,
        "Feature row exceeds byte budget",
    )?;
    Ok(bytes)
}

impl FittedFeaturePipeline {
    fn join_storage_bytes(&self) -> Result<usize> {
        let mut bytes = 0usize;
        for source in &self.joins {
            let FeatureStep::JoinSources { keys, .. } = &self.pipeline.steps[source.step_index]
            else {
                return Err(invalid("Lookup is attached to a non-join step"));
            };
            let keys = keys
                .iter()
                .map(|key| key.right.as_str())
                .collect::<Vec<_>>();
            for row in &source.rows {
                let row_size = serialized_bytes(row)?;
                let key_size = key(row, &keys, false)?.unwrap().len();
                bytes = bytes
                    .checked_add(row_size)
                    .and_then(|bytes| bytes.checked_add(key_size + 64))
                    .ok_or_else(|| invalid("Fitted join byte overflow"))?;
                check_capacity(bytes, 0, 0, &self.limits)?;
            }
        }
        Ok(bytes)
    }
    fn input_row_bounds(&self, row: &Value) -> Result<usize> {
        let bytes = row_bounds(row, &self.limits)?;
        let object = row.as_object().unwrap();
        let outputs = self.pipeline.generated_columns();
        require(
            object
                .len()
                .checked_add(outputs.len())
                .is_some_and(|columns| columns <= self.limits.maximum_columns),
            "Input and generated columns exceed the column budget",
        )?;
        require(
            outputs.iter().all(|name| !object.contains_key(name)),
            "Generated feature already exists in an input row",
        )?;
        Ok(bytes)
    }
    pub fn validate(&self) -> Result<()> {
        require(
            self.format_version == 1,
            "Unknown fitted feature pipeline version",
        )?;
        self.pipeline.validate(&self.policy, &self.limits)?;
        let mut found = BTreeSet::new();
        let mut rows = 0usize;
        let mut bytes = 0usize;
        for joined in &self.joins {
            require(
                found.insert(joined.step_index),
                "Fitted join step is duplicated",
            )?;
            let step = self
                .pipeline
                .steps
                .get(joined.step_index)
                .ok_or_else(|| invalid("Fitted join step is out of bounds"))?;
            require(
                matches!(step, FeatureStep::JoinSources { .. }),
                "Fitted lookup is attached to a non-join step",
            )?;
            let FeatureStep::JoinSources {
                keys,
                columns,
                join,
                ..
            } = step
            else {
                unreachable!()
            };
            let mut expected: BTreeSet<&str> = keys
                .iter()
                .map(|key| key.right.as_str())
                .chain(columns.iter().map(|column| column.source.as_str()))
                .collect();
            if let JoinKind::BackwardAsOf {
                right_timestamp,
                right_available_at,
                ..
            } = join
            {
                expected.insert(right_timestamp);
                if let Some(column) = right_available_at {
                    expected.insert(column);
                }
            }
            rows = rows
                .checked_add(joined.rows.len())
                .ok_or_else(|| invalid("Fitted join row overflow"))?;
            for row in &joined.rows {
                require(
                    row.as_object().is_some_and(|object| {
                        object.keys().map(String::as_str).collect::<BTreeSet<_>>() == expected
                            && object.values().all(|value| {
                                value.is_null()
                                    || value.is_number()
                                    || value.is_boolean()
                                    || value.is_string()
                            })
                    }),
                    "Fitted join row contains undeclared columns or nonscalar values",
                )?;
                bytes = bytes
                    .checked_add(serialized_bytes(row)?)
                    .ok_or_else(|| invalid("Fitted join byte overflow"))?;
            }
            require(
                rows <= self.limits.maximum_join_rows && bytes <= self.limits.maximum_bytes,
                "Fitted join exceeds row or byte limits",
            )?;
            build_lookup(step, &joined.rows)?;
        }
        require(
            found.len()
                == self
                    .pipeline
                    .steps
                    .iter()
                    .filter(|step| matches!(step, FeatureStep::JoinSources { .. }))
                    .count(),
            "Fitted join lookup is missing",
        )?;
        self.join_storage_bytes()?;
        Ok(())
    }
    pub fn digest(&self) -> Result<String> {
        Ok(content_digest(&serde_json::to_vec(self)?))
    }
    pub fn stream(&self) -> Result<FeatureStream> {
        let state = FeatureStreamState {
            pipeline_digest: self.digest()?,
            windows: self
                .pipeline
                .steps
                .iter()
                .enumerate()
                .filter_map(|(step_index, step)| {
                    matches!(step, FeatureStep::WindowFeatures { .. }).then_some(WindowState {
                        step_index,
                        groups: BTreeMap::new(),
                    })
                })
                .collect(),
        };
        self.stream_with_state(state)
    }
    pub fn stream_with_state(&self, state: FeatureStreamState) -> Result<FeatureStream> {
        self.validate()?;
        let join_bytes = self.join_storage_bytes()?;
        require(
            state.pipeline_digest == self.digest()?,
            "Window state belongs to a different fitted pipeline",
        )?;
        let mut seen = BTreeSet::new();
        let mut usage = (0, 0, 0);
        for window in &state.windows {
            require(
                seen.insert(window.step_index),
                "Window state step is duplicated",
            )?;
            let Some(FeatureStep::WindowFeatures {
                features, group_by, ..
            }) = self.pipeline.steps.get(window.step_index)
            else {
                return Err(invalid("State is attached to a non-window step"));
            };
            let columns: BTreeSet<_> = features
                .iter()
                .map(|feature| feature.column.clone())
                .collect();
            for (group_key, group) in &window.groups {
                require(
                    group_key.len() <= 4096,
                    "State group key exceeds size limit",
                )?;
                let keys: Vec<Value> = serde_json::from_str(group_key)?;
                require(
                    keys.len() == group_by.len()
                        && serde_json::to_string(&keys)? == *group_key
                        && keys
                            .iter()
                            .all(|key| key.is_string() || key.is_number() || key.is_boolean()),
                    "Window state group key is malformed",
                )?;
                let mut previous = None;
                for row in &group.history {
                    require(
                        row.timestamp_ms <= group.last_timestamp_ms
                            && previous.is_none_or(|time| time < row.timestamp_ms),
                        "Window state timestamps are inconsistent",
                    )?;
                    require(
                        row.values.keys().cloned().collect::<BTreeSet<_>>() == columns
                            && row.values.values().flatten().all(|value| value.is_finite()),
                        "Window state feature values differ from the pipeline",
                    )?;
                    previous = Some(row.timestamp_ms);
                }
                add_usage(&mut usage, group_usage(group_key, group))?;
                check_usage(usage, &self.limits)?;
                check_capacity(join_bytes, usage.2, 0, &self.limits)?;
            }
        }
        require(
            seen.len()
                == self
                    .pipeline
                    .steps
                    .iter()
                    .filter(|step| matches!(step, FeatureStep::WindowFeatures { .. }))
                    .count(),
            "Window state is missing a pipeline step",
        )?;
        let lookups = self
            .joins
            .iter()
            .map(|joined| {
                Ok((
                    joined.step_index,
                    build_lookup(&self.pipeline.steps[joined.step_index], &joined.rows)?,
                ))
            })
            .collect::<Result<_>>()?;
        Ok(FeatureStream {
            fitted: self.clone(),
            state,
            lookups,
            usage,
            join_bytes,
        })
    }
    pub fn transform_rows(&self, rows: &[Value]) -> Result<Vec<Value>> {
        let deadline = Deadline::new(&self.limits);
        require(
            rows.len() <= self.limits.maximum_rows,
            "Feature row count exceeds its budget",
        )?;
        let mut bytes = 0usize;
        for row in rows {
            bytes = bytes
                .checked_add(self.input_row_bounds(row)?)
                .ok_or_else(|| invalid("Feature byte count overflows"))?;
            require(
                bytes <= self.limits.maximum_bytes,
                "Input table exceeds feature byte budget",
            )?;
        }
        let mut stream = self.stream()?;
        check_capacity(stream.join_bytes, stream.usage.2, bytes, &self.limits)?;
        let mut result = rows.to_vec();
        for index in 0..self.pipeline.steps.len() {
            let mut order = (0..result.len()).collect::<Vec<_>>();
            if let FeatureStep::WindowFeatures {
                timestamp_column, ..
            } = &self.pipeline.steps[index]
            {
                let times = result
                    .iter()
                    .map(|row| timestamp(row, timestamp_column))
                    .collect::<Result<Vec<_>>>()?;
                order.sort_by_key(|position| (times[*position], *position));
            }
            for position in order {
                deadline.check()?;
                let previous_bytes = serialized_bytes(&result[position])?;
                let update = stream.apply_step(
                    index,
                    &mut result[position],
                    &deadline,
                    stream.usage,
                    bytes,
                )?;
                bytes = bytes
                    .checked_sub(previous_bytes)
                    .and_then(|value| value.checked_add(serialized_bytes(&result[position]).ok()?))
                    .ok_or_else(|| invalid("Transformed table byte count overflow"))?;
                if let Some(update) = update {
                    stream.commit_updates(vec![update], bytes)?;
                }
                check_capacity(stream.join_bytes, stream.usage.2, bytes, &self.limits)?;
                row_bounds(&result[position], &self.limits)?;
            }
        }
        Ok(result)
    }
}

impl FeatureStream {
    pub fn state(&self) -> FeatureStreamState {
        self.state.clone()
    }
    pub fn transform_row(&mut self, row: &Value) -> Result<Value> {
        let deadline = Deadline::new(&self.fitted.limits);
        let mut bytes = self.fitted.input_row_bounds(row)?;
        check_capacity(self.join_bytes, self.usage.2, bytes, &self.fitted.limits)?;
        let mut result = row.clone();
        let mut pending = Vec::new();
        let mut usage = self.usage;
        for index in 0..self.fitted.pipeline.steps.len() {
            deadline.check()?;
            if let Some(update) = self.apply_step(index, &mut result, &deadline, usage, bytes)? {
                let previous = self
                    .state
                    .windows
                    .iter()
                    .find(|window| window.step_index == index)
                    .and_then(|window| window.groups.get(&update.1));
                usage = replace_group_usage(
                    usage,
                    &update.1,
                    previous,
                    group_usage(&update.1, &update.2),
                )?;
                pending.push(update);
            }
            bytes = row_bounds(&result, &self.fitted.limits)?;
            check_capacity(self.join_bytes, usage.2, bytes, &self.fitted.limits)?;
        }
        deadline.check()?;
        self.commit_updates(pending, bytes)?;
        Ok(result)
    }
    fn commit_updates(
        &mut self,
        pending: Vec<(usize, String, WindowGroup)>,
        output_bytes: usize,
    ) -> Result<()> {
        let mut usage = self.usage;
        for (index, key, group) in &pending {
            let window = self
                .state
                .windows
                .iter()
                .find(|window| window.step_index == *index)
                .ok_or_else(|| invalid("Missing window state"))?;
            if let Some(previous) = window.groups.get(key) {
                let old = group_usage(key, previous);
                usage.0 -= old.0;
                usage.1 -= old.1;
                usage.2 -= old.2;
            }
            add_usage(&mut usage, group_usage(key, group))?;
            check_usage(usage, &self.fitted.limits)?;
        }
        check_capacity(self.join_bytes, usage.2, output_bytes, &self.fitted.limits)?;
        for (index, key, group) in pending {
            self.state
                .windows
                .iter_mut()
                .find(|window| window.step_index == index)
                .unwrap()
                .groups
                .insert(key, group);
        }
        self.usage = usage;
        Ok(())
    }
    fn apply_step(
        &self,
        index: usize,
        row: &mut Value,
        deadline: &Deadline,
        usage: (usize, usize, usize),
        output_bytes: usize,
    ) -> Result<Option<(usize, String, WindowGroup)>> {
        match &self.fitted.pipeline.steps[index] {
            FeatureStep::DeriveColumns { columns } => {
                for column in columns {
                    let value = serde_json::to_value(evaluate(&column.expression, row)?)?;
                    insert(row, &column.name, value)?;
                }
            }
            FeatureStep::WindowFeatures {
                group_by,
                timestamp_column,
                features,
            } => {
                let key = key(
                    row,
                    &group_by.iter().map(String::as_str).collect::<Vec<_>>(),
                    false,
                )?
                .unwrap();
                let previous = self
                    .state
                    .windows
                    .iter()
                    .find(|window| window.step_index == index)
                    .and_then(|window| window.groups.get(&key));
                let next =
                    next_window_usage(&key, timestamp(row, timestamp_column)?, features, previous)?;
                let projected = replace_group_usage(usage, &key, previous, next)?;
                check_usage(projected, &self.fitted.limits)?;
                check_capacity(
                    self.join_bytes,
                    projected.2,
                    output_bytes,
                    &self.fitted.limits,
                )?;
                let (key, group) = window_row(row, group_by, timestamp_column, features, previous)?;
                return Ok(Some((index, key, group)));
            }
            FeatureStep::JoinSources {
                keys,
                join,
                columns,
                missing,
                ..
            } => {
                let key = key(
                    row,
                    &keys.iter().map(|key| key.left.as_str()).collect::<Vec<_>>(),
                    true,
                )?;
                let selected = match (&self.lookups[&index], join, key) {
                    (JoinLookup::Exact(lookup), JoinKind::Exact, Some(key)) => {
                        lookup.get(&key).copied()
                    }
                    (
                        JoinLookup::AsOf(lookup),
                        JoinKind::BackwardAsOf {
                            left_timestamp,
                            tolerance_ms,
                            ..
                        },
                        Some(key),
                    ) => {
                        let time = timestamp(row, left_timestamp)?;
                        if let Some(rows) = lookup.get(&key) {
                            let end = rows.partition_point(|row| row.0 <= time);
                            let mut selected = None;
                            for (event, available, index) in rows[..end].iter().rev() {
                                deadline.check()?;
                                if tolerance_ms.is_some_and(|tolerance| {
                                    time as i128 - *event as i128 > tolerance as i128
                                }) {
                                    break;
                                }
                                if available.is_none_or(|available| available <= time) {
                                    selected = Some(*index);
                                    break;
                                }
                            }
                            selected
                        } else {
                            None
                        }
                    }
                    (_, _, None) => None,
                    _ => return Err(invalid("Fitted join lookup type differs from its step")),
                };
                if selected.is_none() && *missing == MissingJoin::Error {
                    return Err(invalid("Join has no permitted matching source row"));
                }
                let source = self
                    .fitted
                    .joins
                    .iter()
                    .find(|joined| joined.step_index == index)
                    .unwrap();
                for column in columns {
                    let value = selected
                        .map(|index| cell(&source.rows[index], &column.source).cloned())
                        .transpose()?
                        .unwrap_or(Value::Null);
                    require(
                        value.is_null()
                            || value.is_number()
                            || value.is_string()
                            || value.is_boolean(),
                        "Joined feature must be a scalar or null",
                    )?;
                    insert(row, &column.name, value)?;
                }
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn policy() -> FeaturePolicy {
        FeaturePolicy {
            allowed_input_columns: vec!["x".into(), "y".into()],
            forbidden_columns: vec!["id".into(), "time".into(), "target".into()],
            metadata_columns: vec!["id".into(), "time".into(), "device".into()],
            allowed_sources: BTreeMap::new(),
        }
    }
    fn column(name: &str) -> Expression {
        Expression::Column { name: name.into() }
    }
    fn rolling(name: &str, kind: RollingStatistic, min_periods: usize) -> WindowFeature {
        WindowFeature {
            name: name.into(),
            column: "x".into(),
            operation: WindowOperation::Rolling {
                statistic: kind,
                window: WindowSize::Rows { count: 2 },
                min_periods,
                include_current: true,
            },
        }
    }
    fn temporal() -> FeaturePipeline {
        FeaturePipeline {
            steps: vec![
                FeatureStep::DeriveColumns {
                    columns: vec![DerivedColumn {
                        name: "ratio".into(),
                        expression: Expression::Coalesce {
                            values: vec![
                                Expression::Binary {
                                    op: BinaryOperator::SafeDivide,
                                    left: Box::new(column("x")),
                                    right: Box::new(column("y")),
                                },
                                Expression::Number { value: 0. },
                            ],
                        },
                    }],
                },
                FeatureStep::WindowFeatures {
                    group_by: vec!["device".into()],
                    timestamp_column: "time".into(),
                    features: vec![
                        WindowFeature {
                            name: "lag".into(),
                            column: "x".into(),
                            operation: WindowOperation::Lag { steps: 1 },
                        },
                        rolling("mean", RollingStatistic::Mean, 1),
                        rolling("std", RollingStatistic::Std, 2),
                        rolling("rms", RollingStatistic::Rms, 2),
                        WindowFeature {
                            name: "delta".into(),
                            column: "x".into(),
                            operation: WindowOperation::Delta { lag: 1 },
                        },
                        WindowFeature {
                            name: "rate".into(),
                            column: "x".into(),
                            operation: WindowOperation::Rate { lag: 1 },
                        },
                    ],
                },
            ],
        }
    }
    #[test]
    fn causal_batch_and_resumed_stream_match_without_future_or_group_leakage() {
        let fitted = temporal()
            .fit(&BTreeMap::new(), &policy(), &FeatureLimits::default())
            .unwrap();
        let rows = vec![
            json!({"id":"a3","device":"a","time":3000,"x":6.,"y":2.}),
            json!({"id":"a1","device":"a","time":1000,"x":2.,"y":0.}),
            json!({"id":"b1","device":"b","time":1000,"x":10.,"y":2.}),
            json!({"id":"a2","device":"a","time":2000,"x":4.,"y":2.}),
        ];
        let batch = fitted.transform_rows(&rows).unwrap();
        assert_eq!(batch[0]["id"], "a3");
        assert_eq!(batch[0]["lag"], 4.);
        assert_eq!(batch[0]["mean"], 5.);
        assert_eq!(batch[0]["std"], 1.);
        assert_eq!(batch[0]["delta"], 2.);
        assert_eq!(batch[0]["rate"], 2.);
        assert!((batch[0]["rms"].as_f64().unwrap() - 26f64.sqrt()).abs() < 1e-12);
        assert!(batch[1]["lag"].is_null());
        assert_eq!(batch[1]["ratio"], 0.);
        assert_eq!(batch[1]["mean"], 2.);
        assert!(batch[2]["lag"].is_null());
        let restored: FittedFeaturePipeline =
            serde_json::from_slice(&serde_json::to_vec(&fitted).unwrap()).unwrap();
        let mut stream = restored.stream().unwrap();
        for i in [1, 2] {
            assert_eq!(stream.transform_row(&rows[i]).unwrap(), batch[i]);
        }
        let state: FeatureStreamState =
            serde_json::from_slice(&serde_json::to_vec(&stream.state()).unwrap()).unwrap();
        let mut resumed = restored.stream_with_state(state).unwrap();
        for i in [3, 0] {
            assert_eq!(resumed.transform_row(&rows[i]).unwrap(), batch[i]);
        }
        let state = resumed.state();
        assert!(resumed.transform_row(&rows[1]).is_err());
        assert_eq!(resumed.state(), state);
        assert!(
            restored
                .transform_rows(&[rows[1].clone(), rows[1].clone()])
                .is_err()
        );
        assert!(restored.transform_rows(&rows[0..1]).unwrap()[0]["lag"].is_null());
    }
    #[test]
    fn expressions_and_policy_reject_leakage_collisions_and_unbounded_work() {
        let mut pipeline = FeaturePipeline {
            steps: vec![FeatureStep::DeriveColumns {
                columns: vec![DerivedColumn {
                    name: "new".into(),
                    expression: column("target"),
                }],
            }],
        };
        assert!(
            pipeline
                .fit(&BTreeMap::new(), &policy(), &FeatureLimits::default())
                .is_err()
        );
        let FeatureStep::DeriveColumns { columns } = &mut pipeline.steps[0] else {
            unreachable!()
        };
        columns[0].expression = Expression::Clip {
            value: Box::new(Expression::Unary {
                op: UnaryOperator::Log,
                value: Box::new(column("x")),
            }),
            minimum: -1.,
            maximum: 1.,
        };
        let fitted = pipeline
            .fit(&BTreeMap::new(), &policy(), &FeatureLimits::default())
            .unwrap();
        let values = fitted
            .transform_rows(&[json!({"x":-1.,"y":0.}), json!({"x":100.,"y":1.})])
            .unwrap();
        assert!(values[0]["new"].is_null());
        assert_eq!(values[1]["new"], 1.);
        assert!(fitted.transform_rows(&[json!({"x":2.,"new":0.})]).is_err());
        let limits = FeatureLimits {
            maximum_expression_nodes: 2,
            ..FeatureLimits::default()
        };
        assert!(pipeline.validate(&policy(), &limits).is_err());
        let FeatureStep::DeriveColumns { columns } = &mut pipeline.steps[0] else {
            unreachable!()
        };
        columns[0].name = "id".into();
        assert!(
            pipeline
                .validate(&policy(), &FeatureLimits::default())
                .is_err()
        );
    }
    #[test]
    fn window_state_changes_are_atomic_and_bounded() {
        let mut pipeline = FeaturePipeline {
            steps: vec![FeatureStep::WindowFeatures {
                group_by: vec!["device".into()],
                timestamp_column: "time".into(),
                features: vec![WindowFeature {
                    name: "history_mean".into(),
                    column: "x".into(),
                    operation: WindowOperation::Rolling {
                        statistic: RollingStatistic::Mean,
                        window: WindowSize::DurationMs {
                            milliseconds: 10_000,
                        },
                        min_periods: 1,
                        include_current: true,
                    },
                }],
            }],
        };
        pipeline.steps.push(FeatureStep::DeriveColumns {
            columns: vec![DerivedColumn {
                name: "exponential".into(),
                expression: Expression::Unary {
                    op: UnaryOperator::Exp,
                    value: Box::new(column("x")),
                },
            }],
        });
        let limits = FeatureLimits {
            maximum_history_rows: 2,
            maximum_groups: 1,
            ..FeatureLimits::default()
        };
        let fitted = pipeline.fit(&BTreeMap::new(), &policy(), &limits).unwrap();
        let mut stream = fitted.stream().unwrap();
        stream
            .transform_row(&json!({"device":"a","time":1,"x":0.}))
            .unwrap();
        let initial = stream.state();
        assert!(
            stream
                .transform_row(&json!({"device":"a","time":2,"x":1000.}))
                .is_err()
        );
        assert_eq!(stream.state(), initial);
        assert!(
            stream
                .transform_row(&json!({"device":"b","time":2,"x":0.}))
                .is_err()
        );
        assert_eq!(stream.state(), initial);
        stream
            .transform_row(&json!({"device":"a","time":2,"x":1.}))
            .unwrap();
        let state = stream.state();
        assert!(
            stream
                .transform_row(&json!({"device":"a","time":3,"x":2.}))
                .is_err()
        );
        assert_eq!(stream.state(), state);
        let mut corrupt = state;
        corrupt.pipeline_digest = "other".into();
        assert!(fitted.stream_with_state(corrupt).is_err());
        let mut corrupt = initial;
        let mut noncanonical = corrupt.clone();
        let group = noncanonical.windows[0].groups.remove("[\"a\"]").unwrap();
        noncanonical.windows[0]
            .groups
            .insert("[ \"a\" ]".into(), group);
        assert!(fitted.stream_with_state(noncanonical).is_err());
        corrupt.windows[0]
            .groups
            .get_mut("[\"a\"]")
            .unwrap()
            .history[0]
            .values
            .insert("target".into(), Some(1.));
        assert!(fitted.stream_with_state(corrupt).is_err());
    }
    #[test]
    fn joins_are_single_match_causal_and_preserve_fitted_lookup_data() {
        let mut policy = policy();
        policy.allowed_sources.insert(
            "sensor".into(),
            vec![
                "device".into(),
                "time".into(),
                "available".into(),
                "offset".into(),
            ],
        );
        let join = FeatureStep::JoinSources {
            alias: "sensor".into(),
            keys: vec![JoinKey {
                left: "device".into(),
                right: "device".into(),
            }],
            join: JoinKind::BackwardAsOf {
                left_timestamp: "time".into(),
                right_timestamp: "time".into(),
                right_available_at: Some("available".into()),
                tolerance_ms: Some(5),
            },
            columns: vec![JoinColumn {
                source: "offset".into(),
                name: "sensor_offset".into(),
            }],
            missing: MissingJoin::Null,
        };
        let sources = BTreeMap::from([(
            "sensor".into(),
            vec![
                json!({"device":"a","time":1,"available":1,"offset":10.,"private_target":"never persist"}),
                json!({"device":"a","time":3,"available":8,"offset":30.}),
                json!({"device":"a","time":9,"available":9,"offset":90.}),
            ],
        )]);
        let pipeline = FeaturePipeline { steps: vec![join] };
        let fitted = pipeline
            .fit(&sources, &policy, &FeatureLimits::default())
            .unwrap();
        assert!(
            !serde_json::to_string(&fitted)
                .unwrap()
                .contains("private_target")
        );
        let mut corrupt = fitted.clone();
        corrupt.joins[0].rows[0]["undeclared"] = json!("cannot persist");
        assert!(corrupt.validate().is_err());
        let output = fitted
            .transform_rows(&[
                json!({"device":"a","time":0}),
                json!({"device":"a","time":4}),
                json!({"device":"a","time":7}),
                json!({"device":"a","time":8}),
                json!({"device":"missing","time":8}),
            ])
            .unwrap();
        assert!(output[0]["sensor_offset"].is_null());
        assert_eq!(output[1]["sensor_offset"], 10.);
        assert!(output[2]["sensor_offset"].is_null());
        assert_eq!(output[3]["sensor_offset"], 30.);
        assert!(output[4]["sensor_offset"].is_null());
        let mut duplicate = sources.clone();
        duplicate
            .get_mut("sensor")
            .unwrap()
            .push(sources["sensor"][0].clone());
        assert!(
            pipeline
                .fit(&duplicate, &policy, &FeatureLimits::default())
                .is_err()
        );
        let mut exact = pipeline.clone();
        let FeatureStep::JoinSources { join, .. } = &mut exact.steps[0] else {
            unreachable!()
        };
        *join = JoinKind::Exact;
        assert!(
            exact
                .fit(&sources, &policy, &FeatureLimits::default())
                .is_err()
        );
        let one = BTreeMap::from([("sensor".into(), vec![sources["sensor"][0].clone()])]);
        let fitted = exact.fit(&one, &policy, &FeatureLimits::default()).unwrap();
        assert_eq!(
            fitted
                .transform_rows(&[json!({"device":"a","time":4})])
                .unwrap()[0]["sensor_offset"],
            10.
        );
        let mut denied = policy.clone();
        denied
            .allowed_sources
            .get_mut("sensor")
            .unwrap()
            .retain(|column| column != "offset");
        assert!(exact.fit(&one, &denied, &FeatureLimits::default()).is_err());
        let mut protected = policy;
        protected.forbidden_columns.push("offset".into());
        assert!(
            exact
                .fit(&one, &protected, &FeatureLimits::default())
                .is_err()
        );
    }
    #[test]
    fn resource_budgets_cover_all_steps_join_storage_and_outputs() {
        let pipeline = FeaturePipeline {
            steps: ["first", "second"]
                .into_iter()
                .map(|name| FeatureStep::WindowFeatures {
                    group_by: vec!["device".into()],
                    timestamp_column: "time".into(),
                    features: vec![WindowFeature {
                        name: name.into(),
                        column: "x".into(),
                        operation: WindowOperation::Rolling {
                            statistic: RollingStatistic::Mean,
                            window: WindowSize::DurationMs { milliseconds: 100 },
                            min_periods: 1,
                            include_current: true,
                        },
                    }],
                })
                .collect(),
        };
        let limits = FeatureLimits {
            maximum_history_rows: 3,
            ..FeatureLimits::default()
        };
        let fitted = pipeline.fit(&BTreeMap::new(), &policy(), &limits).unwrap();
        let mut stream = fitted.stream().unwrap();
        stream
            .transform_row(&json!({"device":"a","time":1,"x":1}))
            .unwrap();
        let original = stream.state();
        assert!(
            stream
                .transform_row(&json!({"device":"a","time":2,"x":2}))
                .is_err()
        );
        assert_eq!(stream.state(), original);
        let mut excessive = original;
        for window in &mut excessive.windows {
            let group = window.groups.values_mut().next().unwrap();
            let mut next = group.history[0].clone();
            next.timestamp_ms = 2;
            group.history.push_back(next);
            group.last_timestamp_ms = 2;
        }
        assert!(fitted.stream_with_state(excessive).is_err());
        let limits = FeatureLimits {
            maximum_columns: 4,
            ..FeatureLimits::default()
        };
        let fitted = pipeline.fit(&BTreeMap::new(), &policy(), &limits).unwrap();
        assert!(
            fitted
                .transform_rows(&[json!({"device":"a","time":1,"x":1})])
                .is_err()
        );

        let join = FeaturePipeline {
            steps: vec![FeatureStep::JoinSources {
                alias: "side".into(),
                keys: vec![JoinKey {
                    left: "device".into(),
                    right: "device".into(),
                }],
                join: JoinKind::Exact,
                columns: vec![JoinColumn {
                    source: "text".into(),
                    name: "joined".into(),
                }],
                missing: MissingJoin::Error,
            }],
        };
        let mut policy = policy();
        policy
            .allowed_sources
            .insert("side".into(), vec!["device".into(), "text".into()]);
        let sources = BTreeMap::from([(
            "side".into(),
            vec![json!({"device":"a","text":"z".repeat(550)})],
        )]);
        let limits = FeatureLimits {
            maximum_bytes: 1024,
            ..FeatureLimits::default()
        };
        let fitted = join.fit(&sources, &policy, &limits).unwrap();
        assert!(
            fitted
                .stream()
                .unwrap()
                .transform_row(&json!({"device":"a"}))
                .is_err()
        );
        assert!(fitted.transform_rows(&[json!({"device":"a"})]).is_err());
        let mut repeated = join.clone();
        let mut next = repeated.steps[0].clone();
        if let FeatureStep::JoinSources { columns, .. } = &mut next {
            columns[0].name = "joined_again".into();
        }
        repeated.steps.push(next);
        assert!(repeated.fit(&sources, &policy, &limits).is_err());
    }
}
