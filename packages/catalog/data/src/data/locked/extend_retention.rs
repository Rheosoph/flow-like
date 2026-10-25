use super::{
    add_exec_out, add_item_id_pin, add_text_pin, add_version_output,
    client::{LockedAccess, LockedAction, LockedApi},
    locked_node, optional_text, read_required, resolve_version_id,
    types::{LockedItemVersion, LockedRetentionRequest},
};
use chrono::{DateTime, NaiveDate, SecondsFormat, TimeDelta, Utc};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{anyhow, async_trait, json::json};

/// RFC 3339 retention date from either an explicit date or a number of days from `now`.
pub(crate) fn retain_until(
    date: &str,
    days_from_now: i64,
    now: DateTime<Utc>,
) -> flow_like_types::Result<String> {
    let date = date.trim();
    let until = match (date.is_empty(), days_from_now) {
        (_, days) if days < 0 => {
            return Err(anyhow!(
                "Retain Days From Now must not be negative, got {days}"
            ));
        }
        (true, 0) => return Err(anyhow!("Set Retain Until or Retain Days From Now")),
        (false, days) if days > 0 => {
            return Err(anyhow!(
                "Set either Retain Until or Retain Days From Now, not both"
            ));
        }
        (true, days) => TimeDelta::try_days(days)
            .and_then(|delta| now.checked_add_signed(delta))
            .ok_or_else(|| anyhow!("Retain Days From Now is too large: {days}"))?,
        (false, _) => parse_date(date)?,
    };
    Ok(until.to_rfc3339_opts(SecondsFormat::Secs, true))
}

fn parse_date(date: &str) -> flow_like_types::Result<DateTime<Utc>> {
    if let Ok(until) = DateTime::parse_from_rfc3339(date) {
        return Ok(until.with_timezone(&Utc));
    }
    NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .ok()
        .and_then(|day| day.and_hms_opt(0, 0, 0))
        .map(|midnight| midnight.and_utc())
        .ok_or_else(|| {
            anyhow!(
                "Retain Until '{date}' is not an RFC 3339 date-time such as 2036-12-31T00:00:00Z or a date such as 2036-12-31"
            )
        })
}

#[crate::register_node]
#[derive(Default)]
pub struct LockedExtendRetentionNode;

impl LockedExtendRetentionNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for LockedExtendRetentionNode {
    fn get_node(&self) -> Node {
        let mut node = locked_node(
            "locked_extend_retention",
            "Extend Locked File Retention",
            "Moves the retention date of a locked version further into the future. Retention can only be extended, never shortened; needs app admin rights and per-object locks",
            "extendRetention",
            5,
        );
        add_item_id_pin(&mut node, "ID of the locked file");
        add_text_pin(
            &mut node,
            "version_id",
            "Version ID",
            "ID of the version to extend; empty extends the newest locked version",
        );
        add_text_pin(
            &mut node,
            "retain_until",
            "Retain Until",
            "New retention date as RFC 3339 date-time (2036-12-31T00:00:00Z) or date (2036-12-31); leave empty when using Retain Days From Now",
        );
        node.add_input_pin(
            "extend_days",
            "Retain Days From Now",
            "Retain until this many days from now instead of a fixed date; 0 uses Retain Until",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        add_exec_out(&mut node, "The retention was extended");
        add_version_output(&mut node, "The version with its new retention date");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let item_id = read_required(context, "item_id", "Locked File ID").await?;
        let version_id = optional_text(context.evaluate_pin::<String>("version_id").await?);
        let until = retain_until(
            &context.evaluate_pin::<String>("retain_until").await?,
            context.evaluate_pin::<i64>("extend_days").await?,
            Utc::now(),
        )?;
        let api = LockedApi::bind(context, LockedAccess::Write)?;
        let version_id = resolve_version_id(context, &api, &item_id, version_id).await?;
        let version: LockedItemVersion = api
            .post(
                context,
                LockedAction::Retention,
                &LockedRetentionRequest {
                    item_id: &item_id,
                    version_id: &version_id,
                    retain_until: &until,
                },
            )
            .await?;
        context.set_pin_value("version", json!(version)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-21T12:30:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn days_count_from_now() {
        assert_eq!(retain_until("", 30, now()).unwrap(), "2026-11-20T12:30:00Z");
        assert_eq!(
            retain_until("  ", 3650, now()).unwrap(),
            "2036-10-18T12:30:00Z"
        );
    }

    #[test]
    fn dates_are_normalized_to_utc() {
        assert_eq!(
            retain_until("2036-12-31T01:00:00+02:00", 0, now()).unwrap(),
            "2036-12-30T23:00:00Z"
        );
        assert_eq!(
            retain_until("2036-12-31", 0, now()).unwrap(),
            "2036-12-31T00:00:00Z"
        );
    }

    #[test]
    fn exactly_one_source_is_required() {
        assert!(retain_until("", 0, now()).is_err());
        assert!(retain_until("2036-12-31", 5, now()).is_err());
        assert!(retain_until("", -1, now()).is_err());
        assert!(retain_until("next year", 0, now()).is_err());
        assert!(retain_until("", i64::MAX, now()).is_err());
    }
}
