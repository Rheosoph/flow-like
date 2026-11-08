use flow_like_types::user_availability::{BusyInterval, FreeBusy};
use serde_json::Value;

fn instant(value: &Value) -> Option<i64> {
    let text = value.as_str()?;
    chrono::DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|time| time.timestamp_millis())
        .or_else(|| {
            chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f")
                .ok()
                .map(|time| time.and_utc().timestamp_millis())
        })
}

pub(super) fn google(value: &Value, start: i64, end: i64) -> Result<FreeBusy, &'static str> {
    let calendar = &value["calendars"]["primary"];
    if calendar["errors"]
        .as_array()
        .is_some_and(|errors| !errors.is_empty())
    {
        return Err("Calendar access failed. Reconnect your calendar");
    }
    let entries = calendar["busy"]
        .as_array()
        .ok_or("Calendar returned an invalid availability response")?;
    let mut intervals = Vec::new();
    for entry in entries {
        intervals.push(BusyInterval {
            start: instant(&entry["start"]).ok_or("Invalid calendar start time")?,
            end: instant(&entry["end"]).ok_or("Invalid calendar end time")?,
        });
    }
    finish(start, end, intervals)
}

pub(super) fn microsoft(value: &Value, start: i64, end: i64) -> Result<FreeBusy, &'static str> {
    if value.get("@odata.nextLink").is_some() {
        return Err("This calendar has too many entries in the requested day");
    }
    let entries = value["value"]
        .as_array()
        .ok_or("Calendar returned an invalid availability response")?;
    let mut intervals = Vec::new();
    for entry in entries {
        if entry["isCancelled"].as_bool() == Some(true)
            || matches!(entry["showAs"].as_str(), Some("free" | "workingElsewhere"))
        {
            continue;
        }
        for side in ["start", "end"] {
            if entry[side]["timeZone"]
                .as_str()
                .is_some_and(|zone| !matches!(zone, "UTC" | "Etc/UTC"))
            {
                return Err("Calendar did not return UTC times");
            }
        }
        intervals.push(BusyInterval {
            start: instant(&entry["start"]["dateTime"]).ok_or("Invalid calendar start time")?,
            end: instant(&entry["end"]["dateTime"]).ok_or("Invalid calendar end time")?,
        });
    }
    finish(start, end, intervals)
}

fn finish(start: i64, end: i64, intervals: Vec<BusyInterval>) -> Result<FreeBusy, &'static str> {
    if intervals
        .iter()
        .any(|interval| interval.end <= interval.start)
    {
        return Err("Calendar returned an invalid time range");
    }
    let result = FreeBusy::normalized(start, end, intervals);
    if result.busy.len() > 128 {
        return Err("This calendar has too many busy periods in the requested day");
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn google_projects_ranges_and_rejects_provider_errors() {
        let data = json!({"calendars":{"primary":{"busy":[{"start":"2026-11-08T10:00:00Z","end":"2026-11-08T11:00:00Z","secret":"ignored"}]}}});
        let start = instant(&json!("2026-11-08T10:30:00Z")).unwrap();
        let end = start + 3_600_000;
        let result = google(&data, start, end).unwrap();
        assert_eq!(
            result.busy,
            vec![BusyInterval {
                start,
                end: start + 1_800_000
            }]
        );
        assert!(
            google(
                &json!({"calendars":{"primary":{"errors":[{"reason":"forbidden"}],"busy":[]}}}),
                start,
                end
            )
            .is_err()
        );
        assert!(!serde_json::to_string(&result).unwrap().contains("secret"));
    }
    #[test]
    fn microsoft_discards_free_cancelled_and_details() {
        let time = json!({"dateTime":"2026-11-08T10:00:00.0000000","timeZone":"UTC"});
        let end = json!({"dateTime":"2026-11-08T11:00:00.0000000","timeZone":"UTC"});
        let data = json!({"value":[{"start":time,"end":end,"showAs":"busy","subject":"secret"},{"start":time,"end":end,"showAs":"free"},{"start":time,"end":end,"isCancelled":true}]});
        let start = instant(&time["dateTime"]).unwrap();
        let result = microsoft(&data, start, start + 86_400_000).unwrap();
        assert_eq!(result.busy.len(), 1);
        assert!(!serde_json::to_string(&result).unwrap().contains("secret"));
        assert!(
            microsoft(
                &json!({"value":[],"@odata.nextLink":"https://example.com"}),
                start,
                start + 1
            )
            .is_err()
        );
    }
}
