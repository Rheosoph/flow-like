//! Calendar day to instant conversions for the `timestamptz` columns.
//!
//! Every date/time column stores an instant with an offset, so a `NaiveDate`
//! coming off a query string or a rollup key has to be anchored to a zone
//! before it can bound a query. All of them are anchored to UTC.

use chrono::{DateTime, FixedOffset, NaiveDate, NaiveTime};

/// Midnight UTC opening `day`.
pub fn utc_midnight(day: NaiveDate) -> DateTime<FixedOffset> {
    day.and_time(NaiveTime::MIN).and_utc().fixed_offset()
}

/// 23:59:59 UTC on `day` — the inclusive upper bound the analytics and sales
/// endpoints have always used, kept to the second so their day windows keep
/// covering exactly the rows they covered before.
pub fn utc_day_end(day: NaiveDate) -> DateTime<FixedOffset> {
    day.and_hms_opt(23, 59, 59)
        .map(|stamp| stamp.and_utc().fixed_offset())
        .unwrap_or_else(|| utc_midnight(day))
}
