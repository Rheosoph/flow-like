use crate::utils::pure_scores;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

fn format_node(id: &str, label: &str, description: &str) -> Node {
    let mut node = Node::new(id, label, description, "Utils/Format");
    node.add_icon("/flow/icons/type.svg");
    node.set_scores(pure_scores());
    node
}

/// Groups the integer part from the right, which is what every thousands
/// separator does regardless of the locale's choice of character.
fn group_digits(digits: &str, separator: &str) -> String {
    if separator.is_empty() || digits.len() <= 3 {
        return digits.to_string();
    }

    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    let leading = digits.len() % 3;

    if leading > 0 {
        grouped.push_str(&digits[..leading]);
    }
    for (index, chunk) in digits.as_bytes()[leading..].chunks(3).enumerate() {
        if index > 0 || leading > 0 {
            grouped.push_str(separator);
        }
        grouped.push_str(&String::from_utf8_lossy(chunk));
    }

    grouped
}

#[crate::register_node]
#[derive(Default)]
pub struct FormatNumberNode {}

impl FormatNumberNode {
    pub fn new() -> Self {
        FormatNumberNode {}
    }
}

#[async_trait]
impl NodeLogic for FormatNumberNode {
    fn get_node(&self) -> Node {
        let mut node = format_node(
            "format_number",
            "Format Number",
            "Renders a number for display with fixed decimals and separators",
        );
        node.set_flowscript_name("fmt", "number");

        node.add_input_pin("value", "Value", "Number to format", VariableType::Float);
        node.add_input_pin(
            "decimals",
            "Decimals",
            "Decimal places to keep",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(2)));
        node.add_input_pin(
            "thousands",
            "Thousands Separator",
            "Inserted every three digits, empty for none",
            VariableType::String,
        )
        .set_default_value(Some(json!(",")));
        node.add_input_pin(
            "decimal_point",
            "Decimal Separator",
            "Character between the whole and fractional part",
            VariableType::String,
        )
        .set_default_value(Some(json!(".")));
        node.add_input_pin(
            "prefix",
            "Prefix",
            "Put in front, for example a currency symbol",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "suffix",
            "Suffix",
            "Appended, for example a unit",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "as_percent",
            "As Percent",
            "Multiply by 100 and append a percent sign",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin("text", "Text", "The formatted number", VariableType::String);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let value: f64 = context.evaluate_pin("value").await?;
        let decimals: i64 = context.evaluate_pin("decimals").await?;
        let thousands: String = context.evaluate_pin("thousands").await?;
        let decimal_point: String = context.evaluate_pin("decimal_point").await?;
        let prefix: String = context.evaluate_pin("prefix").await?;
        let suffix: String = context.evaluate_pin("suffix").await?;
        let as_percent: bool = context.evaluate_pin("as_percent").await?;

        let value = if as_percent { value * 100.0 } else { value };
        let rendered = format!("{:.*}", decimals.clamp(0, 15) as usize, value.abs());

        let (whole, fraction) = match rendered.split_once('.') {
            Some((whole, fraction)) => (whole, Some(fraction)),
            None => (rendered.as_str(), None),
        };

        let mut text = String::new();
        if value.is_sign_negative() && value != 0.0 {
            text.push('-');
        }
        text.push_str(&prefix);
        text.push_str(&group_digits(whole, &thousands));
        if let Some(fraction) = fraction {
            text.push_str(&decimal_point);
            text.push_str(fraction);
        }
        text.push_str(&suffix);
        if as_percent {
            text.push('%');
        }

        context.set_pin_value("text", json!(text)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct FormatBytesNode {}

impl FormatBytesNode {
    pub fn new() -> Self {
        FormatBytesNode {}
    }
}

#[async_trait]
impl NodeLogic for FormatBytesNode {
    fn get_node(&self) -> Node {
        let mut node = format_node(
            "format_bytes",
            "Format File Size",
            "Turns a byte count into a readable size such as 1.4 MB",
        );
        node.set_flowscript_name("fmt", "bytes");

        node.add_input_pin("bytes", "Bytes", "Number of bytes", VariableType::Integer)
            .set_default_value(Some(json!(0)));
        node.add_input_pin(
            "standard",
            "Standard",
            "Decimal counts in 1000s (MB), Binary in 1024s (MiB)",
            VariableType::String,
        )
        .set_default_value(Some(json!("Decimal")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["Decimal".to_string(), "Binary".to_string()])
                .build(),
        );
        node.add_input_pin(
            "decimals",
            "Decimals",
            "Decimal places to keep",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(1)));

        node.add_output_pin("text", "Text", "The readable size", VariableType::String);
        node.add_output_pin(
            "unit",
            "Unit",
            "The unit that was chosen",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let bytes: i64 = context.evaluate_pin("bytes").await?;
        let standard: String = context.evaluate_pin("standard").await?;
        let decimals: i64 = context.evaluate_pin("decimals").await?;

        let binary = standard == "Binary";
        let step = if binary { 1024.0 } else { 1000.0 };
        let units: [&str; 7] = if binary {
            ["B", "KiB", "MiB", "GiB", "TiB", "PiB", "EiB"]
        } else {
            ["B", "KB", "MB", "GB", "TB", "PB", "EB"]
        };

        let mut size = bytes.abs() as f64;
        let mut unit = 0;
        while size >= step && unit < units.len() - 1 {
            size /= step;
            unit += 1;
        }

        let decimals = if unit == 0 {
            0
        } else {
            decimals.clamp(0, 6) as usize
        };
        let sign = if bytes < 0 { "-" } else { "" };
        let text = format!("{sign}{size:.decimals$} {}", units[unit]);

        context.set_pin_value("text", json!(text)).await?;
        context.set_pin_value("unit", json!(units[unit])).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct FormatOrdinalNode {}

impl FormatOrdinalNode {
    pub fn new() -> Self {
        FormatOrdinalNode {}
    }
}

#[async_trait]
impl NodeLogic for FormatOrdinalNode {
    fn get_node(&self) -> Node {
        let mut node = format_node(
            "format_ordinal",
            "Ordinal",
            "Writes a number as 1st, 2nd, 3rd and so on",
        );
        node.set_flowscript_name("fmt", "ordinal");
        node.set_receiver("value");

        node.add_input_pin("value", "Value", "Number to write", VariableType::Integer)
            .set_default_value(Some(json!(1)));
        node.add_output_pin("text", "Text", "The ordinal", VariableType::String);
        node.add_output_pin(
            "suffix",
            "Suffix",
            "Just the two letter suffix",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let value: i64 = context.evaluate_pin("value").await?;

        let magnitude = value.abs();
        let suffix = match (magnitude % 100, magnitude % 10) {
            (11..=13, _) => "th",
            (_, 1) => "st",
            (_, 2) => "nd",
            (_, 3) => "rd",
            _ => "th",
        };

        context
            .set_pin_value("text", json!(format!("{value}{suffix}")))
            .await?;
        context.set_pin_value("suffix", json!(suffix)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct MapRangeNode {}

impl MapRangeNode {
    pub fn new() -> Self {
        MapRangeNode {}
    }
}

#[async_trait]
impl NodeLogic for MapRangeNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "float_map_range",
            "Map Range",
            "Rescales a value from one range into another",
            "Math/Float",
        );
        node.set_flowscript_name("float", "mapRange");
        node.set_receiver("value");
        node.add_icon("/flow/icons/sigma.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("value", "Value", "Value to rescale", VariableType::Float);
        for (name, label, default) in [
            ("in_min", "In Min", 0.0),
            ("in_max", "In Max", 1.0),
            ("out_min", "Out Min", 0.0),
            ("out_max", "Out Max", 100.0),
        ] {
            node.add_input_pin(name, label, label, VariableType::Float)
                .set_default_value(Some(json!(default)));
        }
        node.add_input_pin(
            "clamp",
            "Clamp",
            "Keep the result inside the output range",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_output_pin(
            "result",
            "Result",
            "The rescaled value",
            VariableType::Float,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let value: f64 = context.evaluate_pin("value").await?;
        let in_min: f64 = context.evaluate_pin("in_min").await?;
        let in_max: f64 = context.evaluate_pin("in_max").await?;
        let out_min: f64 = context.evaluate_pin("out_min").await?;
        let out_max: f64 = context.evaluate_pin("out_max").await?;
        let clamp: bool = context.evaluate_pin("clamp").await?;

        if in_min == in_max {
            return Err(flow_like_types::anyhow!(
                "Input range is empty: In Min and In Max are both {in_min}"
            ));
        }

        let ratio = (value - in_min) / (in_max - in_min);
        let mut result = out_min + ratio * (out_max - out_min);

        if clamp {
            let (low, high) = if out_min <= out_max {
                (out_min, out_max)
            } else {
                (out_max, out_min)
            };
            result = result.clamp(low, high);
        }

        context.set_pin_value("result", json!(result)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct PercentChangeNode {}

impl PercentChangeNode {
    pub fn new() -> Self {
        PercentChangeNode {}
    }
}

#[async_trait]
impl NodeLogic for PercentChangeNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "float_percent_change",
            "Percent Change",
            "How much a value moved relative to where it started",
            "Math/Float",
        );
        node.set_flowscript_name("float", "percentChange");
        node.set_receiver("from");
        node.add_icon("/flow/icons/sigma.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("from", "From", "Earlier value", VariableType::Float)
            .set_default_value(Some(json!(0.0)));
        node.add_input_pin("to", "To", "Later value", VariableType::Float)
            .set_default_value(Some(json!(0.0)));

        node.add_output_pin(
            "percent",
            "Percent",
            "Change in percent, negative when the value fell",
            VariableType::Float,
        );
        node.add_output_pin("delta", "Delta", "Absolute change", VariableType::Float);
        node.add_output_pin(
            "defined",
            "Defined",
            "False when the earlier value was zero, which has no percentage",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let from: f64 = context.evaluate_pin("from").await?;
        let to: f64 = context.evaluate_pin("to").await?;

        let defined = from != 0.0;
        let percent = if defined {
            (to - from) / from.abs() * 100.0
        } else {
            0.0
        };

        context.set_pin_value("percent", json!(percent)).await?;
        context.set_pin_value("delta", json!(to - from)).await?;
        context.set_pin_value("defined", json!(defined)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct FormatDurationNode {}

impl FormatDurationNode {
    pub fn new() -> Self {
        FormatDurationNode {}
    }
}

#[async_trait]
impl NodeLogic for FormatDurationNode {
    fn get_node(&self) -> Node {
        let mut node = format_node(
            "format_duration",
            "Format Duration",
            "Writes a number of seconds as a readable duration such as 2h 15m",
        );
        node.set_flowscript_name("fmt", "duration");

        node.add_input_pin(
            "seconds",
            "Seconds",
            "Length of the duration in seconds",
            VariableType::Float,
        )
        .set_default_value(Some(json!(0.0)));
        node.add_input_pin(
            "style",
            "Style",
            "Short writes 2h 15m, Long writes 2 hours 15 minutes, Clock writes 02:15:00",
            VariableType::String,
        )
        .set_default_value(Some(json!("Short")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "Short".to_string(),
                    "Long".to_string(),
                    "Clock".to_string(),
                ])
                .build(),
        );
        node.add_input_pin(
            "max_parts",
            "Max Parts",
            "How many units to show before stopping, for example 2 gives 2h 15m instead of 2h 15m 3s",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(2)));

        node.add_output_pin(
            "text",
            "Text",
            "The readable duration",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let seconds: f64 = context.evaluate_pin("seconds").await?;
        let style: String = context.evaluate_pin("style").await?;
        let max_parts: i64 = context.evaluate_pin("max_parts").await?;

        let total = seconds.abs().round() as i64;
        let sign = if seconds < 0.0 { "-" } else { "" };

        if style == "Clock" {
            let text = format!(
                "{sign}{:02}:{:02}:{:02}",
                total / 3_600,
                (total % 3_600) / 60,
                total % 60
            );
            context.set_pin_value("text", json!(text)).await?;
            return Ok(());
        }

        const UNITS: [(i64, &str, &str); 5] = [
            (86_400, "d", "day"),
            (3_600, "h", "hour"),
            (60, "m", "minute"),
            (1, "s", "second"),
            (0, "s", "second"),
        ];

        let long = style == "Long";
        let mut remaining = total;
        let mut parts: Vec<String> = Vec::new();

        for (size, short_name, long_name) in UNITS.into_iter().take(4) {
            if parts.len() as i64 >= max_parts.max(1) {
                break;
            }
            let amount = remaining / size;
            if amount == 0 && !parts.is_empty() {
                continue;
            }
            if amount == 0 {
                continue;
            }
            remaining -= amount * size;
            parts.push(if long {
                let plural = if amount == 1 { "" } else { "s" };
                format!("{amount} {long_name}{plural}")
            } else {
                format!("{amount}{short_name}")
            });
        }

        if parts.is_empty() {
            parts.push(if long {
                "0 seconds".to_string()
            } else {
                "0s".to_string()
            });
        }

        context
            .set_pin_value("text", json!(format!("{sign}{}", parts.join(" "))))
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct RoundToMultipleNode {}

impl RoundToMultipleNode {
    pub fn new() -> Self {
        RoundToMultipleNode {}
    }
}

#[async_trait]
impl NodeLogic for RoundToMultipleNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "float_round_to_multiple",
            "Round To Multiple",
            "Snaps a value to the nearest multiple, for example the nearest 0.05 or 25",
            "Math/Float",
        );
        node.set_flowscript_name("float", "roundToMultiple");
        node.set_receiver("value");
        node.add_icon("/flow/icons/sigma.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("value", "Value", "Value to snap", VariableType::Float);
        node.add_input_pin(
            "multiple",
            "Multiple",
            "Step size to snap to",
            VariableType::Float,
        )
        .set_default_value(Some(json!(1.0)));
        node.add_input_pin(
            "mode",
            "Mode",
            "Which direction to snap in",
            VariableType::String,
        )
        .set_default_value(Some(json!("Nearest")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "Nearest".to_string(),
                    "Up".to_string(),
                    "Down".to_string(),
                ])
                .build(),
        );

        node.add_output_pin("result", "Result", "The snapped value", VariableType::Float);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let value: f64 = context.evaluate_pin("value").await?;
        let multiple: f64 = context.evaluate_pin("multiple").await?;
        let mode: String = context.evaluate_pin("mode").await?;

        if multiple == 0.0 {
            return Err(flow_like_types::anyhow!("Multiple must not be zero"));
        }

        let steps = value / multiple;
        let snapped = match mode.as_str() {
            "Up" => steps.ceil(),
            "Down" => steps.floor(),
            _ => steps.round(),
        } * multiple;

        context.set_pin_value("result", json!(snapped)).await?;
        Ok(())
    }
}
