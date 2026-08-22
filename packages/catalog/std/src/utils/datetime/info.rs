use super::truncate::days_in_month;
use super::util::read_date;
use crate::utils::pure_scores;
use chrono::{Datelike, NaiveDate, TimeZone, Utc, Weekday};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct DateFromPartsNode {}

impl DateFromPartsNode {
    pub fn new() -> Self {
        DateFromPartsNode {}
    }
}

#[async_trait]
impl NodeLogic for DateFromPartsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "utils_datetime_from_parts",
            "From Parts",
            "Builds a date from year, month, day and time components",
            "Utils/DateTime",
        );
        node.add_icon("/flow/icons/calendar.svg");
        node.set_scores(pure_scores());

        for (name, label, default) in [
            ("year", "Year", 1970),
            ("month", "Month", 1),
            ("day", "Day", 1),
            ("hour", "Hour", 0),
            ("minute", "Minute", 0),
            ("second", "Second", 0),
        ] {
            node.add_input_pin(name, label, label, VariableType::Integer)
                .set_default_value(Some(json!(default)));
        }

        node.add_output_pin("date", "Date", "The assembled date", VariableType::Date);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let year: i64 = context.evaluate_pin("year").await?;
        let month: i64 = context.evaluate_pin("month").await?;
        let day: i64 = context.evaluate_pin("day").await?;
        let hour: i64 = context.evaluate_pin("hour").await?;
        let minute: i64 = context.evaluate_pin("minute").await?;
        let second: i64 = context.evaluate_pin("second").await?;

        let date = NaiveDate::from_ymd_opt(year as i32, month as u32, day as u32)
            .and_then(|date| date.and_hms_opt(hour as u32, minute as u32, second as u32))
            .map(|naive| Utc.from_utc_datetime(&naive))
            .ok_or_else(|| {
                flow_like_types::anyhow!(
                    "{year}-{month}-{day} {hour}:{minute}:{second} is not a valid date"
                )
            })?;

        context.set_pin_value("date", json!(date)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct DateCalendarInfoNode {}

impl DateCalendarInfoNode {
    pub fn new() -> Self {
        DateCalendarInfoNode {}
    }
}

#[async_trait]
impl NodeLogic for DateCalendarInfoNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "utils_datetime_calendar_info",
            "Calendar Info",
            "Week number, weekend and leap year facts about a date",
            "Utils/DateTime",
        );
        node.add_icon("/flow/icons/calendar.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("date", "Date", "Input Date", VariableType::Date);

        node.add_output_pin(
            "is_weekend",
            "Is Weekend",
            "True on Saturday and Sunday",
            VariableType::Boolean,
        );
        node.add_output_pin(
            "is_leap_year",
            "Is Leap Year",
            "True when February has 29 days that year",
            VariableType::Boolean,
        );
        node.add_output_pin(
            "week",
            "ISO Week",
            "ISO 8601 week number",
            VariableType::Integer,
        );
        node.add_output_pin(
            "iso_year",
            "ISO Year",
            "Year the ISO week belongs to",
            VariableType::Integer,
        );
        node.add_output_pin(
            "quarter",
            "Quarter",
            "Quarter of the year, 1 to 4",
            VariableType::Integer,
        );
        node.add_output_pin(
            "days_in_month",
            "Days In Month",
            "Length of the month the date falls in",
            VariableType::Integer,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let date = read_date(context, "date").await?;
        let iso = date.iso_week();
        let weekend = matches!(date.weekday(), Weekday::Sat | Weekday::Sun);
        let year = date.year();
        let leap = NaiveDate::from_ymd_opt(year, 2, 29).is_some();

        context.set_pin_value("is_weekend", json!(weekend)).await?;
        context.set_pin_value("is_leap_year", json!(leap)).await?;
        context
            .set_pin_value("week", json!(iso.week() as i64))
            .await?;
        context
            .set_pin_value("iso_year", json!(iso.year() as i64))
            .await?;
        context
            .set_pin_value("quarter", json!((date.month0() / 3 + 1) as i64))
            .await?;
        context
            .set_pin_value(
                "days_in_month",
                json!(days_in_month(year, date.month()) as i64),
            )
            .await?;
        Ok(())
    }
}
