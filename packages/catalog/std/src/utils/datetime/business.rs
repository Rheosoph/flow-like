use super::util::read_date;
use crate::utils::pure_scores;
use chrono::{DateTime, Datelike, Duration, Utc, Weekday};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

fn is_business_day(date: DateTime<Utc>) -> bool {
    !matches!(date.weekday(), Weekday::Sat | Weekday::Sun)
}

#[crate::register_node]
#[derive(Default)]
pub struct DateBusinessDaysBetweenNode {}

impl DateBusinessDaysBetweenNode {
    pub fn new() -> Self {
        DateBusinessDaysBetweenNode {}
    }
}

#[async_trait]
impl NodeLogic for DateBusinessDaysBetweenNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "utils_datetime_business_days_between",
            "Business Days Between",
            "Counts the working days between two dates, skipping weekends",
            "Utils/DateTime",
        );
        node.add_icon("/flow/icons/calendar.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("start", "Start", "Start of the range", VariableType::Date);
        node.add_input_pin("end", "End", "End of the range", VariableType::Date);
        node.add_input_pin(
            "include_end",
            "Include End",
            "Count the end day itself when it is a working day",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin(
            "days",
            "Days",
            "Working days in the range, negative when the end lies before the start",
            VariableType::Integer,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let start = read_date(context, "start").await?;
        let end = read_date(context, "end").await?;
        let include_end: bool = context.evaluate_pin("include_end").await?;

        let backwards = end < start;
        let (from, to) = if backwards {
            (end, start)
        } else {
            (start, end)
        };

        let mut cursor = from.date_naive();
        let last = to.date_naive();
        let mut days: i64 = 0;

        while cursor < last || (include_end && cursor == last) {
            let day = cursor
                .and_hms_opt(0, 0, 0)
                .map(|naive| naive.and_utc())
                .unwrap_or(from);
            if is_business_day(day) {
                days += 1;
            }
            cursor = match cursor.succ_opt() {
                Some(next) => next,
                None => break,
            };
        }

        context
            .set_pin_value("days", json!(if backwards { -days } else { days }))
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct DateAddBusinessDaysNode {}

impl DateAddBusinessDaysNode {
    pub fn new() -> Self {
        DateAddBusinessDaysNode {}
    }
}

#[async_trait]
impl NodeLogic for DateAddBusinessDaysNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "utils_datetime_add_business_days",
            "Add Business Days",
            "Moves a date forward or back by working days, skipping weekends",
            "Utils/DateTime",
        );
        node.add_icon("/flow/icons/calendar.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("date", "Date", "Input Date", VariableType::Date);
        node.add_input_pin(
            "days",
            "Days",
            "Working days to add, negative to go back",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(1)));

        node.add_output_pin(
            "result",
            "Result",
            "The shifted date, always landing on a working day",
            VariableType::Date,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let date = read_date(context, "date").await?;
        let days: i64 = context.evaluate_pin("days").await?;

        let step = if days < 0 {
            Duration::days(-1)
        } else {
            Duration::days(1)
        };

        let mut cursor = date;
        let mut remaining = days.abs().min(100_000);

        while remaining > 0 {
            cursor += step;
            if is_business_day(cursor) {
                remaining -= 1;
            }
        }

        context.set_pin_value("result", json!(cursor)).await?;
        Ok(())
    }
}
