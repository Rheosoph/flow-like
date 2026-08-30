use super::util::read_date;
use crate::utils::pure_scores;
use chrono::{DateTime, Datelike, Duration, NaiveDate, TimeZone, Timelike, Utc};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

pub const UNITS: [&str; 8] = [
    "Second", "Minute", "Hour", "Day", "Week", "Month", "Quarter", "Year",
];

fn at_midnight(date: NaiveDate) -> DateTime<Utc> {
    Utc.from_utc_datetime(&date.and_hms_opt(0, 0, 0).unwrap_or_default())
}

/// Snaps a date to the start of the given unit, or to the last instant inside it.
/// Unknown units fall back to Day, which is what an unset dropdown means.
pub fn truncate_to(date: DateTime<Utc>, unit: &str, end: bool) -> DateTime<Utc> {
    let start = match unit {
        "Second" => date.with_nanosecond(0).unwrap_or(date),
        "Minute" => date
            .with_second(0)
            .and_then(|date| date.with_nanosecond(0))
            .unwrap_or(date),
        "Hour" => date
            .with_minute(0)
            .and_then(|date| date.with_second(0))
            .and_then(|date| date.with_nanosecond(0))
            .unwrap_or(date),
        "Week" => {
            let weekday = date.weekday().num_days_from_monday() as i64;
            at_midnight(date.date_naive()) - Duration::days(weekday)
        }
        "Month" => date
            .date_naive()
            .with_day(1)
            .map(at_midnight)
            .unwrap_or(date),
        "Quarter" => {
            let quarter_start = (date.month0() / 3) * 3 + 1;
            date.date_naive()
                .with_day(1)
                .and_then(|day| day.with_month(quarter_start))
                .map(at_midnight)
                .unwrap_or(date)
        }
        "Year" => date
            .date_naive()
            .with_ordinal(1)
            .map(at_midnight)
            .unwrap_or(date),
        _ => at_midnight(date.date_naive()),
    };

    if !end {
        return start;
    }

    let next = match unit {
        "Second" => start + Duration::seconds(1),
        "Minute" => start + Duration::minutes(1),
        "Hour" => start + Duration::hours(1),
        "Week" => start + Duration::days(7),
        "Month" => add_months(start, 1),
        "Quarter" => add_months(start, 3),
        "Year" => add_months(start, 12),
        _ => start + Duration::days(1),
    };

    next - Duration::milliseconds(1)
}

/// Calendar-aware month arithmetic: the 31st of January plus one month is the
/// 28th (or 29th) of February, never the 3rd of March.
pub fn add_months(date: DateTime<Utc>, months: i64) -> DateTime<Utc> {
    let total = date.year() as i64 * 12 + date.month0() as i64 + months;
    let year = total.div_euclid(12) as i32;
    let month = total.rem_euclid(12) as u32 + 1;
    let day = date.day().min(days_in_month(year, month));

    NaiveDate::from_ymd_opt(year, month, day)
        .and_then(|day| {
            day.and_hms_nano_opt(date.hour(), date.minute(), date.second(), date.nanosecond())
        })
        .map(|naive| Utc.from_utc_datetime(&naive))
        .unwrap_or(date)
}

pub fn days_in_month(year: i32, month: u32) -> u32 {
    let (next_year, next_month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };

    NaiveDate::from_ymd_opt(next_year, next_month, 1)
        .and_then(|first| first.pred_opt())
        .map(|last| last.day())
        .unwrap_or(28)
}

fn boundary_node(id: &str, label: &str, description: &str) -> Node {
    let mut node = Node::new(id, label, description, "Utils/DateTime");
    node.add_icon("/flow/icons/calendar.svg");
    node.set_scores(pure_scores());

    node.add_input_pin("date", "Date", "Input Date", VariableType::Date);
    node.add_input_pin("unit", "Unit", "Unit to snap to", VariableType::String)
        .set_default_value(Some(json!("Day")))
        .set_options(
            PinOptions::new()
                .set_valid_values(UNITS.iter().map(|unit| unit.to_string()).collect())
                .build(),
        );
    node.add_output_pin("result", "Result", description, VariableType::Date);

    node
}

#[crate::register_node]
#[derive(Default)]
pub struct DateStartOfNode {}

impl DateStartOfNode {
    pub fn new() -> Self {
        DateStartOfNode {}
    }
}

#[async_trait]
impl NodeLogic for DateStartOfNode {
    fn get_node(&self) -> Node {
        let mut node = boundary_node(
            "utils_datetime_start_of",
            "Start Of",
            "The first instant of the day, week, month, quarter or year",
        );
        node.set_flowscript_name("datetime", "startOf");
        node.set_receiver("date");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let date = read_date(context, "date").await?;
        let unit: String = context.evaluate_pin("unit").await?;
        context
            .set_pin_value("result", json!(truncate_to(date, &unit, false)))
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct DateEndOfNode {}

impl DateEndOfNode {
    pub fn new() -> Self {
        DateEndOfNode {}
    }
}

#[async_trait]
impl NodeLogic for DateEndOfNode {
    fn get_node(&self) -> Node {
        let mut node = boundary_node(
            "utils_datetime_end_of",
            "End Of",
            "The last instant of the day, week, month, quarter or year",
        );
        node.set_flowscript_name("datetime", "endOf");
        node.set_receiver("date");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let date = read_date(context, "date").await?;
        let unit: String = context.evaluate_pin("unit").await?;
        context
            .set_pin_value("result", json!(truncate_to(date, &unit, true)))
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct DateShiftNode {}

impl DateShiftNode {
    pub fn new() -> Self {
        DateShiftNode {}
    }
}

#[async_trait]
impl NodeLogic for DateShiftNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "utils_datetime_shift_calendar",
            "Add Months / Years",
            "Calendar-aware shift that keeps the day of month where it exists",
            "Utils/DateTime",
        );
        node.set_flowscript_name("datetime", "shiftCalendar");
        node.set_receiver("date");
        node.add_icon("/flow/icons/calendar.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("date", "Date", "Input Date", VariableType::Date);
        node.add_input_pin(
            "months",
            "Months",
            "Months to add, negative to go back",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));
        node.add_input_pin(
            "years",
            "Years",
            "Years to add, negative to go back",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_output_pin("result", "Result", "The shifted date", VariableType::Date);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let date = read_date(context, "date").await?;
        let months: i64 = context.evaluate_pin("months").await?;
        let years: i64 = context.evaluate_pin("years").await?;

        let shifted = add_months(date, months.saturating_add(years.saturating_mul(12)));
        context.set_pin_value("result", json!(shifted)).await?;
        Ok(())
    }
}
