//! The shared result cache of cross-app queries. A fresh entry is served once its Lance tables
//! are confirmed at the versions it read. An expired entry whose tables all have versions is
//! offered back to the query service, which answers `unchanged` without running when none of
//! them moved and the statement does not depend on the time of the run. Entries that read
//! external tables expire by time only. Every entry stays one more day after it expires, to stand
//! in when a refresh fails.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use flow_like_query::QueryResponse;
use flow_like_storage::databases::workbench::SqlQueryResult;
use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::query;

/// How long an expired entry may still answer for a refresh that failed.
pub(super) const STALE_IF_ERROR: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct CachedQuery {
    pub result: SqlQueryResult,
    pub tables: Vec<String>,
    /// The version each Lance `schema.table` was read at.
    pub versions: BTreeMap<String, u64>,
    /// Tables without a version (external sources): the entry can only expire by time.
    pub unversioned_tables: Vec<String>,
    /// What the query service noted while running it, such as a lowered row limit.
    pub warnings: Vec<String>,
    /// Epoch milliseconds.
    pub computed_at: i64,
    /// Epoch milliseconds after which the entry is no longer served without a check.
    pub expires_at: i64,
}

fn millis(duration: Duration) -> i64 {
    duration.as_millis().min(i64::MAX as u128) as i64
}

impl CachedQuery {
    pub(super) fn from_response(
        response: &QueryResponse,
        computed_at: i64,
        ttl: Duration,
    ) -> Result<Self, ApiError> {
        Ok(Self {
            result: query::sql_result(response)?,
            tables: response.trailer.tables.clone(),
            versions: response.trailer.versions.clone(),
            unversioned_tables: response.trailer.unversioned_tables.clone(),
            warnings: response.trailer.warnings.clone(),
            computed_at,
            expires_at: computed_at.saturating_add(millis(ttl)),
        })
    }

    fn fresh(&self, now: i64) -> bool {
        now < self.expires_at
    }

    fn usable(&self, now: i64) -> bool {
        now < self.expires_at.saturating_add(millis(STALE_IF_ERROR))
    }

    /// The versions to offer the service, when every table this entry read has one.
    pub(super) fn reusable_versions(&self) -> Option<BTreeMap<String, u64>> {
        (self.unversioned_tables.is_empty() && !self.versions.is_empty())
            .then(|| self.versions.clone())
    }

    /// The schemas whose Lance tables must still be at the recorded versions before a fresh
    /// entry is served; `None` when it read an external table (time alone decides) or no table.
    pub(super) fn probed_schemas(&self) -> Option<BTreeSet<&str>> {
        self.reusable_versions()?;
        Some(
            self.versions
                .keys()
                .filter_map(|name| name.split_once('.').map(|(schema, _)| schema))
                .collect(),
        )
    }

    /// The tables are still at the versions this entry read: it is as good as a new run.
    fn restamp(&mut self, now: i64, ttl: Duration) {
        self.computed_at = now;
        self.expires_at = now.saturating_add(millis(ttl));
    }
}

/// What a request does with the entry the cache holds for it.
#[derive(Debug)]
pub(super) enum Lookup {
    /// Serve the entry without asking the service.
    Fresh(CachedQuery),
    /// Ask the service, offering `reuse` versions when the held entry may be reused unchanged.
    /// `held` answers if the refresh fails.
    Refresh {
        held: Option<CachedQuery>,
        reuse: Option<BTreeMap<String, u64>>,
    },
}

pub(super) fn lookup(held: Option<CachedQuery>, force_fresh: bool, now: i64) -> Lookup {
    let held = held.filter(|entry| entry.usable(now));
    match held {
        Some(entry) if !force_fresh && entry.fresh(now) => Lookup::Fresh(entry),
        held => {
            let reuse = held
                .as_ref()
                .filter(|_| !force_fresh)
                .and_then(CachedQuery::reusable_versions);
            Lookup::Refresh { held, reuse }
        }
    }
}

/// The entry a request answers with.
#[derive(Debug)]
pub(super) struct Settled {
    pub entry: CachedQuery,
    /// The rows came from an earlier run.
    pub cached: bool,
    /// Why a stale entry answers: the refresh failed.
    pub failure: Option<String>,
    /// The entry is new or restamped and belongs in the cache.
    pub store: bool,
}

/// Combines the service's answer with the held entry: an `unchanged` answer restamps it, new
/// rows replace it, and a failure falls back to it when there is one.
pub(super) fn settle(
    held: Option<CachedQuery>,
    outcome: Result<QueryResponse, ApiError>,
    now: i64,
    ttl: Duration,
) -> Result<Settled, ApiError> {
    match outcome {
        Ok(response) if response.trailer.unchanged => {
            let mut entry = held.ok_or_else(|| {
                ApiError::bad_gateway(
                    "The query service reused a result that was not offered to it",
                )
            })?;
            entry.restamp(now, ttl);
            Ok(Settled {
                entry,
                cached: true,
                failure: None,
                store: true,
            })
        }
        Ok(response) => Ok(Settled {
            entry: CachedQuery::from_response(&response, now, ttl)?,
            cached: false,
            failure: None,
            store: true,
        }),
        Err(error) => match held {
            Some(entry) => {
                tracing::info!(
                    status = error.status().as_u16(),
                    "Cross-app query refresh failed; serving the last result"
                );
                Ok(Settled {
                    failure: Some(failure_warning(&error)),
                    entry,
                    cached: true,
                    store: false,
                })
            }
            None => Err(error),
        },
    }
}

fn failure_warning(error: &ApiError) -> String {
    let reason = error
        .public_message()
        .map(str::to_string)
        .unwrap_or_else(|| {
            error
                .status()
                .canonical_reason()
                .unwrap_or("unknown error")
                .to_string()
        });
    format!("The refresh failed ({reason}); showing the last result")
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_query::Trailer;
    use flow_like_storage::arrow_array::{Int64Array, RecordBatch};
    use flow_like_storage::arrow_schema::{DataType, Field, Schema};
    use serde_json::json;
    use std::sync::Arc;

    const TTL: Duration = Duration::from_secs(60);

    fn rows(values: Vec<i64>, trailer: Trailer) -> QueryResponse {
        let schema = Arc::new(Schema::new(vec![Field::new("n", DataType::Int64, false)]));
        let batch =
            RecordBatch::try_new(schema.clone(), vec![Arc::new(Int64Array::from(values))]).unwrap();
        QueryResponse {
            schema: Some(schema),
            batches: vec![batch],
            json: None,
            trailer,
        }
    }

    fn versioned() -> Trailer {
        Trailer {
            row_count: 1,
            tables: vec!["shop.orders".to_string()],
            versions: BTreeMap::from([("shop.orders".to_string(), 7)]),
            ..Trailer::default()
        }
    }

    fn external() -> Trailer {
        Trailer {
            row_count: 1,
            tables: vec!["warehouse.orders".to_string()],
            unversioned_tables: vec!["warehouse.orders".to_string()],
            ..Trailer::default()
        }
    }

    fn entry(trailer: Trailer, computed_at: i64) -> CachedQuery {
        CachedQuery::from_response(&rows(vec![1], trailer), computed_at, TTL).unwrap()
    }

    #[test]
    fn fresh_entries_answer_without_the_service() {
        let held = entry(versioned(), 1_000);
        assert_eq!(held.expires_at, 61_000);
        assert!(matches!(
            lookup(Some(held.clone()), false, 60_999),
            Lookup::Fresh(_)
        ));
        assert!(
            matches!(
                lookup(Some(held), true, 2_000),
                Lookup::Refresh {
                    held: Some(_),
                    reuse: None
                }
            ),
            "fresh: true skips the cached rows and asks for a full run"
        );
    }

    #[test]
    fn expired_entries_offer_their_versions_unless_a_table_is_external() {
        let Lookup::Refresh { held, reuse } =
            lookup(Some(entry(versioned(), 1_000)), false, 61_000)
        else {
            panic!("an expired entry is refreshed");
        };
        assert!(held.is_some());
        assert_eq!(
            reuse,
            Some(BTreeMap::from([("shop.orders".to_string(), 7)]))
        );

        let Lookup::Refresh { held, reuse } = lookup(Some(entry(external(), 1_000)), false, 61_000)
        else {
            panic!("an expired entry is refreshed");
        };
        assert!(held.is_some(), "it still answers if the refresh fails");
        assert_eq!(reuse, None, "external tables have no version to compare");

        let no_tables = entry(Trailer::default(), 1_000);
        let Lookup::Refresh { reuse, .. } = lookup(Some(no_tables), false, 61_000) else {
            panic!("an expired entry is refreshed");
        };
        assert_eq!(reuse, None, "a statement over no table is simply run again");
    }

    #[test]
    fn fresh_entries_name_the_schemas_to_probe() {
        let mut trailer = versioned();
        trailer.tables.push("crm.customers".to_string());
        trailer.versions.insert("crm.customers".to_string(), 3);
        assert_eq!(
            entry(trailer, 1_000).probed_schemas(),
            Some(BTreeSet::from(["crm", "shop"]))
        );
        assert_eq!(
            entry(external(), 1_000).probed_schemas(),
            None,
            "rows of an external table expire by time"
        );
        assert_eq!(entry(Trailer::default(), 1_000).probed_schemas(), None);
    }

    #[test]
    fn entries_stand_in_for_one_day_after_they_expire() {
        let held = entry(versioned(), 1_000);
        let last = 61_000 + millis(STALE_IF_ERROR) - 1;
        assert!(matches!(
            lookup(Some(held.clone()), false, last),
            Lookup::Refresh { held: Some(_), .. }
        ));
        assert!(matches!(
            lookup(Some(held), false, last + 1),
            Lookup::Refresh {
                held: None,
                reuse: None
            }
        ));
    }

    #[test]
    fn an_unchanged_answer_restamps_the_held_rows() {
        let held = entry(versioned(), 1_000);
        let unchanged = QueryResponse {
            trailer: Trailer {
                unchanged: true,
                ..Trailer::default()
            },
            ..QueryResponse::default()
        };
        let settled = settle(Some(held), Ok(unchanged), 90_000, TTL).unwrap();
        assert!(settled.cached && settled.store);
        assert_eq!(settled.failure, None);
        assert_eq!(settled.entry.result.rows, vec![json!({ "n": 1 })]);
        assert_eq!(settled.entry.computed_at, 90_000);
        assert_eq!(settled.entry.expires_at, 150_000);

        let orphan = QueryResponse {
            trailer: Trailer {
                unchanged: true,
                ..Trailer::default()
            },
            ..QueryResponse::default()
        };
        assert!(settle(None, Ok(orphan), 90_000, TTL).is_err());
    }

    #[test]
    fn new_rows_replace_the_held_entry() {
        let held = entry(versioned(), 1_000);
        let mut trailer = versioned();
        trailer.versions.insert("shop.orders".to_string(), 8);
        trailer.warnings = vec!["The row limit was lowered".to_string()];
        let settled = settle(Some(held), Ok(rows(vec![5, 6], trailer)), 90_000, TTL).unwrap();
        assert!(!settled.cached && settled.store);
        assert_eq!(settled.entry.result.row_count, 2);
        assert_eq!(settled.entry.versions["shop.orders"], 8);
        assert_eq!(settled.entry.warnings, vec!["The row limit was lowered"]);
        assert_eq!(settled.entry.computed_at, 90_000);
    }

    #[test]
    fn a_failed_refresh_serves_the_held_entry_and_says_why() {
        let held = entry(versioned(), 1_000);
        let settled = settle(
            Some(held),
            Err(ApiError::gateway_timeout(
                "SQL query timed out after 50000ms",
            )),
            90_000,
            TTL,
        )
        .unwrap();
        assert!(settled.cached && !settled.store);
        assert_eq!(
            settled.entry.computed_at, 1_000,
            "stale rows keep their age"
        );
        let warning = settled.failure.unwrap();
        assert!(warning.contains("SQL query timed out"), "{warning}");

        let error = settle(None, Err(ApiError::bad_request("no table")), 90_000, TTL).unwrap_err();
        assert_eq!(error.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[test]
    fn hidden_failures_are_named_by_their_status() {
        let warning = failure_warning(&ApiError::internal("secret detail"));
        assert!(!warning.contains("secret detail"), "{warning}");
        assert!(warning.contains("Internal Server Error"), "{warning}");
    }
}
