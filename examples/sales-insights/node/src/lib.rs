//! Sales Insights — WASM nodes for the micro-frontend example package.
//!
//! The three nodes pair with the two React widgets shipped in the same
//! package and form a complete round trip:
//!
//! 1. `sales_demo_data` produces rows → **instantiate** the Sales Chart with them.
//! 2. `apply_sales_filter` narrows rows using the Filter Panel's state → feed
//!    `Update Widget Inputs` to **update** the live chart.
//! 3. `sales_summary` aggregates whatever the chart reports back through
//!    `Query Widget` → **read** live widget state.

use flow_like_wasm_sdk::*;

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize, JsonSchema)]
pub struct SalesRow {
    /// Bucket label, e.g. a month
    pub label: String,
    /// Revenue in the package's reporting currency
    pub value: f64,
    /// Product category the revenue belongs to
    pub category: String,
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize, JsonSchema)]
pub struct SalesFilter {
    /// Lower revenue bound (inclusive)
    pub min: f64,
    /// Upper revenue bound (inclusive); `0` disables the bound
    pub max: f64,
    /// Categories to keep; empty keeps every category
    pub categories: Vec<String>,
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize, JsonSchema)]
pub struct SalesSummary {
    pub total: f64,
    pub average: f64,
    pub best_label: String,
    pub best_value: f64,
    pub worst_label: String,
    pub worst_value: f64,
    pub count: u64,
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const CATEGORIES: [&str; 3] = ["Hardware", "Software", "Services"];

/// Deterministic pseudo-random revenue so demo boards replay identically.
fn demo_value(month: usize, seed: i64) -> f64 {
    let mixed = (month as i64 + 1)
        .wrapping_mul(seed.abs().max(1))
        .wrapping_mul(2_654_435_761);
    let magnitude = (mixed % 900).unsigned_abs() as f64;
    ((1_000.0 + magnitude * 12.5) * 100.0).round() / 100.0
}

pub fn build_demo_rows(months: usize, seed: i64) -> Vec<SalesRow> {
    (0..months.clamp(1, 12))
        .map(|month| SalesRow {
            label: MONTHS[month].to_string(),
            value: demo_value(month, seed),
            category: CATEGORIES[month % CATEGORIES.len()].to_string(),
        })
        .collect()
}

pub fn filter_rows(rows: &[SalesRow], filter: &SalesFilter) -> Vec<SalesRow> {
    rows.iter()
        .filter(|row| row.value >= filter.min)
        .filter(|row| filter.max <= 0.0 || row.value <= filter.max)
        .filter(|row| filter.categories.is_empty() || filter.categories.contains(&row.category))
        .cloned()
        .collect()
}

pub fn summarize(rows: &[SalesRow]) -> SalesSummary {
    if rows.is_empty() {
        return SalesSummary::default();
    }

    let total: f64 = rows.iter().map(|row| row.value).sum();
    let best = rows
        .iter()
        .max_by(|a, b| a.value.total_cmp(&b.value))
        .expect("non-empty");
    let worst = rows
        .iter()
        .min_by(|a, b| a.value.total_cmp(&b.value))
        .expect("non-empty");

    SalesSummary {
        total: (total * 100.0).round() / 100.0,
        average: ((total / rows.len() as f64) * 100.0).round() / 100.0,
        best_label: best.label.clone(),
        best_value: best.value,
        worst_label: worst.label.clone(),
        worst_value: worst.value,
        count: rows.len() as u64,
    }
}

// ── Node 1: Demo data for instantiating the chart ──────────────────────

#[register_node]
#[derive(Default)]
pub struct SalesDemoDataNode;

impl WasmNode for SalesDemoDataNode {
    fn get_node(&self) -> NodeDefinition {
        let mut node = NodeDefinition::new(
            "sales_demo_data",
            "Sales Demo Data",
            "Generates deterministic demo revenue rows to instantiate the Sales Chart widget with",
            "Sales Insights",
        );
        node.add_input_pin("exec", "Exec", "Trigger pin", VariableType::Execution);
        node.add_input_pin(
            "months",
            "Months",
            "How many months to generate (1-12)",
            VariableType::Integer,
        )
        .set_default_value(json!(6));
        node.add_input_pin(
            "seed",
            "Seed",
            "Seed making the generated revenue reproducible",
            VariableType::Integer,
        )
        .set_default_value(json!(7));
        node.add_output_pin(
            "exec_out",
            "Done",
            "Execution continues",
            VariableType::Execution,
        );
        node.add_output_pin(
            "rows",
            "Rows",
            "Revenue rows — connect to the chart widget's Rows pin",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<SalesRow>();
        node.add_output_pin(
            "categories",
            "Categories",
            "Distinct categories — connect to the filter panel's Categories pin",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);
        node
    }

    fn run(&self, mut ctx: Context) -> ExecutionResult {
        let months = ctx.get_i64("months").unwrap_or(6).clamp(1, 12) as usize;
        let seed = ctx.get_i64("seed").unwrap_or(7);
        let rows = build_demo_rows(months, seed);

        let mut categories: Vec<String> = rows.iter().map(|row| row.category.clone()).collect();
        categories.sort();
        categories.dedup();

        ctx.set_output_json("rows", &rows);
        ctx.set_output_json("categories", &categories);
        ctx.activate_exec("exec_out");
        ctx.success()
    }
}

// ── Node 2: Filter rows for updating the live chart ────────────────────

#[register_node]
#[derive(Default)]
pub struct ApplySalesFilterNode;

impl WasmNode for ApplySalesFilterNode {
    fn get_node(&self) -> NodeDefinition {
        let mut node = NodeDefinition::new(
            "apply_sales_filter",
            "Apply Sales Filter",
            "Filters revenue rows with the Filter Panel widget's state — feed the result into Update Widget Inputs",
            "Sales Insights",
        );
        node.add_input_pin("exec", "Exec", "Trigger pin", VariableType::Execution);
        node.add_input_pin("input_rows", "Rows", "Rows to filter", VariableType::Struct)
            .set_value_type(ValueType::Array)
            .set_schema::<SalesRow>();
        node.add_input_pin(
            "filter",
            "Filter",
            "Filter state — connect the Filter Panel's getValue query result",
            VariableType::Struct,
        )
        .set_schema::<SalesFilter>()
        .set_default_value(json!({ "min": 0.0, "max": 0.0, "categories": [] }));
        node.add_output_pin(
            "exec_out",
            "Done",
            "Execution continues",
            VariableType::Execution,
        );
        node.add_output_pin(
            "output_rows",
            "Filtered Rows",
            "Rows passing the filter",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<SalesRow>();
        node.add_output_pin(
            "removed",
            "Removed",
            "How many rows the filter dropped",
            VariableType::Integer,
        );
        node
    }

    fn run(&self, mut ctx: Context) -> ExecutionResult {
        let rows: Vec<SalesRow> = ctx.get_input_as("input_rows").unwrap_or_default();
        let filter: SalesFilter = ctx.get_input_as("filter").unwrap_or_default();

        let filtered = filter_rows(&rows, &filter);
        let removed = rows.len().saturating_sub(filtered.len()) as i64;

        ctx.set_output_json("output_rows", &filtered);
        ctx.set_output("removed", removed);
        ctx.activate_exec("exec_out");
        ctx.success()
    }
}

// ── Node 3: Summarize what the widget reported back ────────────────────

#[register_node]
#[derive(Default)]
pub struct SalesSummaryNode;

impl WasmNode for SalesSummaryNode {
    fn get_node(&self) -> NodeDefinition {
        let mut node = NodeDefinition::new(
            "sales_summary",
            "Sales Summary",
            "Aggregates revenue rows — pair it with the chart widget's getSeries query to summarize live widget state",
            "Sales Insights",
        );
        node.add_input_pin("exec", "Exec", "Trigger pin", VariableType::Execution);
        node.add_input_pin(
            "input_rows",
            "Rows",
            "Rows to aggregate",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<SalesRow>();
        node.add_output_pin(
            "exec_out",
            "Done",
            "Execution continues",
            VariableType::Execution,
        );
        node.add_output_pin(
            "summary",
            "Summary",
            "Totals and extremes across the rows",
            VariableType::Struct,
        )
        .set_schema::<SalesSummary>();
        node.add_output_pin(
            "headline",
            "Headline",
            "Formatted summary — feed it back into the chart widget's Title input",
            VariableType::String,
        );
        node
    }

    fn run(&self, mut ctx: Context) -> ExecutionResult {
        let rows: Vec<SalesRow> = ctx.get_input_as("input_rows").unwrap_or_default();
        let summary = summarize(&rows);
        let headline = if summary.count == 0 {
            "No revenue in range".to_string()
        } else {
            format!(
                "{} months · {:.0} total · peak {} ({:.0})",
                summary.count, summary.total, summary.best_label, summary.best_value
            )
        };

        ctx.set_output_json("summary", &summary);
        ctx.set_output("headline", headline);
        ctx.activate_exec("exec_out");
        ctx.success()
    }
}

wasm_main!();

#[cfg(test)]
mod tests {
    use super::*;

    fn rows() -> Vec<SalesRow> {
        vec![
            SalesRow {
                label: "Jan".into(),
                value: 1_000.0,
                category: "Hardware".into(),
            },
            SalesRow {
                label: "Feb".into(),
                value: 3_000.0,
                category: "Software".into(),
            },
            SalesRow {
                label: "Mar".into(),
                value: 2_000.0,
                category: "Hardware".into(),
            },
        ]
    }

    #[test]
    fn demo_rows_are_deterministic_and_clamped() {
        assert_eq!(build_demo_rows(6, 7), build_demo_rows(6, 7));
        assert_eq!(build_demo_rows(99, 7).len(), 12);
        assert_eq!(build_demo_rows(0, 7).len(), 1);
        assert_ne!(
            build_demo_rows(6, 7)[0].value,
            build_demo_rows(6, 8)[0].value
        );
    }

    #[test]
    fn filter_applies_bounds_and_categories() {
        let filter = SalesFilter {
            min: 1_500.0,
            max: 0.0,
            categories: vec!["Hardware".into()],
        };
        let filtered = filter_rows(&rows(), &filter);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].label, "Mar");

        let unbounded = SalesFilter::default();
        assert_eq!(filter_rows(&rows(), &unbounded).len(), 3);

        let capped = SalesFilter {
            min: 0.0,
            max: 2_000.0,
            categories: vec![],
        };
        assert_eq!(filter_rows(&rows(), &capped).len(), 2);
    }

    #[test]
    fn summary_reports_totals_and_extremes() {
        let summary = summarize(&rows());
        assert_eq!(summary.count, 3);
        assert_eq!(summary.total, 6_000.0);
        assert_eq!(summary.average, 2_000.0);
        assert_eq!(summary.best_label, "Feb");
        assert_eq!(summary.worst_label, "Jan");

        assert_eq!(summarize(&[]).count, 0);
    }

    #[test]
    fn node_definitions_expose_array_pins() {
        let node = SalesDemoDataNode.get_node();
        let rows_pin = node.pins.iter().find(|pin| pin.name == "rows").unwrap();
        assert_eq!(rows_pin.value_type, Some(ValueType::Array));
        assert!(rows_pin.schema.is_some());

        let filter_node = ApplySalesFilterNode.get_node();
        assert!(filter_node.pins.iter().any(|pin| pin.name == "filter"));
        assert!(filter_node.pins.iter().any(|pin| pin.name == "output_rows"));
    }
}
