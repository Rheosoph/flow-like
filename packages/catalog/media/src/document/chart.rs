use std::fmt;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChartType {
    Bar,
    Line,
    Pie,
    Scatter,
    Area,
    Radar,
    Funnel,
}

impl fmt::Display for ChartType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bar => write!(f, "bar"),
            Self::Line => write!(f, "line"),
            Self::Pie => write!(f, "pie"),
            Self::Scatter => write!(f, "scatter"),
            Self::Area => write!(f, "area"),
            Self::Radar => write!(f, "radar"),
            Self::Funnel => write!(f, "funnel"),
        }
    }
}

impl ChartType {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "bar" => Some(Self::Bar),
            "line" => Some(Self::Line),
            "pie" => Some(Self::Pie),
            "scatter" => Some(Self::Scatter),
            "area" => Some(Self::Area),
            "radar" => Some(Self::Radar),
            "funnel" => Some(Self::Funnel),
            _ => None,
        }
    }

    pub fn supported_in_office(&self) -> bool {
        matches!(self, Self::Bar | Self::Line | Self::Pie)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChartLayout {
    #[default]
    Vertical,
    Horizontal,
}

#[derive(Debug, Clone)]
pub enum CellValue {
    Text(String),
    Number(f64),
}

impl CellValue {
    pub fn as_f64(&self) -> f64 {
        match self {
            Self::Number(n) => *n,
            Self::Text(_) => 0.0,
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Text(s) => s,
            Self::Number(_) => "",
        }
    }
}

impl fmt::Display for CellValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Text(s) => write!(f, "{}", s),
            Self::Number(n) => write!(f, "{}", n),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ChartConfig {
    pub chart_type: Option<ChartType>,
    pub title: Option<String>,
    pub x_label: Option<String>,
    pub y_label: Option<String>,
    pub colors: Option<Vec<String>>,
    pub stacked: bool,
    pub layout: ChartLayout,
}

#[derive(Debug, Clone)]
pub struct CsvData {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<CellValue>>,
}

#[derive(Debug, Clone)]
pub enum ChartData {
    Csv(CsvData),
    Json(String),
}

#[derive(Debug, Clone)]
pub struct ChartInput {
    pub config: ChartConfig,
    pub data: ChartData,
}

/// Flattened chart data ready for Office XML / PDF rendering.
#[derive(Debug, Clone)]
pub struct OfficeChartData {
    pub chart_type: ChartType,
    pub title: Option<String>,
    pub categories: Vec<String>,
    pub series: Vec<ChartSeries>,
    pub colors: Vec<String>,
    pub stacked: bool,
    pub layout: ChartLayout,
}

#[derive(Debug, Clone)]
pub struct ChartSeries {
    pub name: String,
    pub values: Vec<f64>,
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

fn parse_csv(content: &str) -> CsvData {
    let lines: Vec<&str> = content
        .trim()
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();

    if lines.is_empty() {
        return CsvData {
            headers: Vec::new(),
            rows: Vec::new(),
        };
    }

    let headers: Vec<String> = lines[0].split(',').map(|h| h.trim().to_string()).collect();
    let mut rows = Vec::new();

    for line in &lines[1..] {
        let cells: Vec<CellValue> = line
            .split(',')
            .map(|cell| {
                let trimmed = cell.trim();
                match trimmed.parse::<f64>() {
                    Ok(n) => CellValue::Number(n),
                    Err(_) => CellValue::Text(trimmed.to_string()),
                }
            })
            .collect();
        rows.push(cells);
    }

    CsvData { headers, rows }
}

fn parse_config(block: &str) -> ChartConfig {
    let mut config = ChartConfig::default();

    for line in block.lines() {
        let Some(colon) = line.find(':') else {
            continue;
        };
        let key = line[..colon].trim();
        let value = line[colon + 1..].trim();

        match key {
            "type" => config.chart_type = ChartType::from_str(value),
            "title" => config.title = Some(value.to_string()),
            "xLabel" => config.x_label = Some(value.to_string()),
            "yLabel" => config.y_label = Some(value.to_string()),
            "stacked" => config.stacked = value == "true",
            "layout" => {
                if value == "horizontal" {
                    config.layout = ChartLayout::Horizontal;
                }
            }
            "colors" => {
                if value.starts_with('[') && value.ends_with(']') {
                    let inner = &value[1..value.len() - 1];
                    config.colors = Some(
                        inner
                            .split(',')
                            .map(|v| v.trim().trim_matches('"').trim_matches('\'').to_string())
                            .collect(),
                    );
                } else {
                    config.colors = Some(vec![value.to_string()]);
                }
            }
            _ => {}
        }
    }
    config
}

fn auto_detect_chart_type(data: &CsvData) -> ChartType {
    if data.headers.is_empty() || data.rows.is_empty() {
        return ChartType::Bar;
    }

    let num_cols = data.headers.len();
    let num_rows = data.rows.len();

    if num_cols == 2 {
        let second_col_numeric = data.rows.iter().all(|row| {
            row.get(1)
                .is_some_and(|v| matches!(v, CellValue::Number(_)))
        });
        if second_col_numeric && num_rows <= 6 {
            return ChartType::Pie;
        }
    }

    if num_cols >= 3 {
        let has_time_like = data.rows.iter().any(|row| {
            let val = row
                .first()
                .map(|v| v.to_string().to_lowercase())
                .unwrap_or_default();
            val.contains("jan")
                || val.contains("feb")
                || val.contains("mar")
                || val.contains("q1")
                || val.contains("q2")
                || val.starts_with("20")
        });
        if has_time_like {
            return ChartType::Line;
        }
    }

    ChartType::Bar
}

/// Parse a chart code block (content inside ` ```nivo` or ` ```plotly`).
pub fn parse_chart_block(content: &str) -> Option<ChartInput> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return None;
    }

    // JSON mode: starts with { or [
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        return Some(ChartInput {
            config: ChartConfig::default(),
            data: ChartData::Json(trimmed.to_string()),
        });
    }

    // CSV mode: optional frontmatter config separated by \n---\n
    let (config_block, csv_content) = if let Some(pos) = trimmed.find("\n---\n") {
        (&trimmed[..pos], &trimmed[pos + 5..])
    } else {
        ("", trimmed)
    };

    let mut config = if config_block.is_empty() {
        ChartConfig::default()
    } else {
        parse_config(config_block)
    };

    let csv_data = parse_csv(csv_content);

    if config.chart_type.is_none() {
        config.chart_type = Some(auto_detect_chart_type(&csv_data));
    }

    Some(ChartInput {
        config,
        data: ChartData::Csv(csv_data),
    })
}

// ---------------------------------------------------------------------------
// Conversion to Office-ready data
// ---------------------------------------------------------------------------

/// The Flow-Like categorical ramp (`--fl-chat-chart-1..8`), brand ember first.
///
/// The `--chart-1..5` tokens span only ~40° of hue, so a multi-series chart drawn from them
/// reads as a single colour; this ramp is spaced across the wheel for exactly that reason.
const DEFAULT_CHART_COLORS: &[&str] = &[
    "FB562D", "8B61E3", "09AEAE", "EBA42C", "2B7AD6", "42AA60", "DA529C", "62778D",
];

/// Convert parsed chart input into flat data suitable for DrawingML / PDF rendering.
/// Returns `None` for JSON mode (pass-through only) or empty data.
pub fn chart_input_to_office_data(input: &ChartInput) -> Option<OfficeChartData> {
    let csv = match &input.data {
        ChartData::Csv(csv) => csv,
        ChartData::Json(_) => return None,
    };

    if csv.headers.is_empty() || csv.rows.is_empty() {
        return None;
    }

    let chart_type = input.config.chart_type.unwrap_or(ChartType::Bar);

    // For pie: categories from first column, single series from second column
    if chart_type == ChartType::Pie {
        let categories: Vec<String> = csv
            .rows
            .iter()
            .map(|row| row.first().map(|v| v.to_string()).unwrap_or_default())
            .collect();
        let values: Vec<f64> = csv
            .rows
            .iter()
            .map(|row| row.get(1).map(|v| v.as_f64()).unwrap_or(0.0))
            .collect();

        let colors = resolve_colors(&input.config.colors, categories.len());

        return Some(OfficeChartData {
            chart_type,
            title: input.config.title.clone(),
            categories,
            series: vec![ChartSeries {
                name: csv
                    .headers
                    .get(1)
                    .cloned()
                    .unwrap_or_else(|| "Value".into()),
                values,
            }],
            colors,
            stacked: false,
            layout: input.config.layout,
        });
    }

    // For bar/line/area/scatter/radar: first column = categories, remaining = series
    let categories: Vec<String> = csv
        .rows
        .iter()
        .map(|row| row.first().map(|v| v.to_string()).unwrap_or_default())
        .collect();

    let series: Vec<ChartSeries> = csv
        .headers
        .iter()
        .skip(1)
        .enumerate()
        .map(|(si, name)| {
            let values: Vec<f64> = csv
                .rows
                .iter()
                .map(|row| row.get(si + 1).map(|v| v.as_f64()).unwrap_or(0.0))
                .collect();
            ChartSeries {
                name: name.clone(),
                values,
            }
        })
        .collect();

    let num_colors = series.len().max(1);
    let colors = resolve_colors(&input.config.colors, num_colors);

    Some(OfficeChartData {
        chart_type,
        title: input.config.title.clone(),
        categories,
        series,
        colors,
        stacked: input.config.stacked,
        layout: input.config.layout,
    })
}

fn resolve_colors(user_colors: &Option<Vec<String>>, count: usize) -> Vec<String> {
    if let Some(colors) = user_colors {
        let mut out = Vec::with_capacity(count);
        for i in 0..count {
            out.push(
                colors
                    .get(i)
                    .unwrap_or(&DEFAULT_CHART_COLORS[i % DEFAULT_CHART_COLORS.len()].to_string())
                    .trim_start_matches('#')
                    .to_uppercase(),
            );
        }
        out
    } else {
        (0..count)
            .map(|i| DEFAULT_CHART_COLORS[i % DEFAULT_CHART_COLORS.len()].to_string())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_csv_with_frontmatter() {
        let content = r#"type: bar
title: Revenue by Quarter
colors: [#FF4343, #4B5563, #9CA3AF]
stacked: true
---
Quarter,Product A,Product B
Q1,120,80
Q2,150,95
Q3,180,110
Q4,200,130"#;

        let input = parse_chart_block(content).unwrap();
        assert_eq!(input.config.chart_type, Some(ChartType::Bar));
        assert_eq!(input.config.title.as_deref(), Some("Revenue by Quarter"));
        assert!(input.config.stacked);
        assert!(input.config.colors.is_some());

        let office = chart_input_to_office_data(&input).unwrap();
        assert_eq!(office.chart_type, ChartType::Bar);
        assert_eq!(office.categories, vec!["Q1", "Q2", "Q3", "Q4"]);
        assert_eq!(office.series.len(), 2);
        assert_eq!(office.series[0].name, "Product A");
        assert_eq!(office.series[0].values, vec![120.0, 150.0, 180.0, 200.0]);
        assert_eq!(office.series[1].name, "Product B");
        assert_eq!(office.series[1].values, vec![80.0, 95.0, 110.0, 130.0]);
    }

    #[test]
    fn parse_csv_pie_auto_detect() {
        let content = "Category,Value\nDesktop,65\nMobile,25\nTablet,10";
        let input = parse_chart_block(content).unwrap();
        assert_eq!(input.config.chart_type, Some(ChartType::Pie));

        let office = chart_input_to_office_data(&input).unwrap();
        assert_eq!(office.chart_type, ChartType::Pie);
        assert_eq!(office.categories.len(), 3);
        assert_eq!(office.series[0].values, vec![65.0, 25.0, 10.0]);
    }

    #[test]
    fn parse_json_mode() {
        let content = r#"{"chartType": "bar", "data": [{"x": "A", "y": 10}]}"#;
        let input = parse_chart_block(content).unwrap();
        assert!(matches!(input.data, ChartData::Json(_)));
        assert!(chart_input_to_office_data(&input).is_none());
    }

    #[test]
    fn parse_csv_no_frontmatter() {
        let content = "Month,Sales\nJan,100\nFeb,120\nMar,140\nApr,160\nMay,180\nJun,200\nJul,220";
        let input = parse_chart_block(content).unwrap();
        assert_eq!(input.config.chart_type, Some(ChartType::Bar));

        let office = chart_input_to_office_data(&input).unwrap();
        assert_eq!(office.series.len(), 1);
        assert_eq!(office.series[0].name, "Sales");
    }

    #[test]
    fn empty_content_returns_none() {
        assert!(parse_chart_block("").is_none());
        assert!(parse_chart_block("   ").is_none());
    }
}
