use crate::entity::wasm_package_version;
use crate::execution::wasm_resolve::UPGRADE_MARKER;
use chrono::{DateTime, Duration, Utc};

use super::dispatch::URL_TTL_SECS;

/// The version row has no dispatch column. A claimed artifact upgrade records
/// its start in `compilationError`; every other dispatch happens at publish,
/// so `publishedAt` is the latest dispatch the row can prove.
pub fn dispatched_at(version: &wasm_package_version::Model) -> DateTime<Utc> {
    let marker_start = version
        .compilation_error
        .as_deref()
        .and_then(|value| value.strip_prefix(UPGRADE_MARKER))
        .and_then(|value| value.split(':').next())
        .and_then(|value| value.parse::<i64>().ok())
        .and_then(|seconds| DateTime::from_timestamp(seconds, 0));
    let published = version.published_at.to_utc();
    marker_start.map_or(published, |start| start.max(published))
}

/// `None` when a PENDING version may be dispatched again, because the signed
/// URL of its last job has expired and that job can no longer download the
/// WASM. Otherwise the moment from which a retry is accepted.
pub fn retry_pending_after(
    version: &wasm_package_version::Model,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    let retry_at = dispatched_at(version) + Duration::seconds(URL_TTL_SECS as i64);
    (retry_at > now).then_some(retry_at)
}

pub fn still_pending_message(version: &wasm_package_version::Model, retry_at: DateTime<Utc>) -> String {
    format!(
        "Version {} of package {} is PENDING: its compile job was dispatched at {} and may still be running. Retry after {}.",
        version.version,
        version.package_id,
        dispatched_at(version).to_rfc3339(),
        retry_at.to_rfc3339(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending(published_at: &str, error: Option<&str>) -> wasm_package_version::Model {
        serde_json::from_value(serde_json::json!({
            "id": "version", "package_id": "com.example.typst", "version": "0.4.0",
            "wasm_path": "node.wasm", "wasm_hash": "hash", "wasm_size": 8,
            "yanked": false, "published_at": published_at, "duplicate_flagged": false,
            "nodes": [], "widgets": [], "status": "PendingReview",
            "compilation_status": "Pending", "compilation_error": error,
        }))
        .unwrap()
    }

    fn at(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value).unwrap().to_utc()
    }

    #[test]
    fn a_pending_version_older_than_the_signed_url_is_dispatched_again() {
        let version = pending("2026-11-10T18:00:00Z", None);
        assert_eq!(retry_pending_after(&version, at("2026-11-10T21:00:00Z")), None);
        assert_eq!(retry_pending_after(&version, at("2026-11-10T19:00:00Z")), None);
    }

    #[test]
    fn a_recently_dispatched_version_is_refused_with_the_retry_time() {
        let version = pending("2026-11-10T18:00:00Z", None);
        let retry_at = retry_pending_after(&version, at("2026-11-10T18:20:00Z")).unwrap();
        assert_eq!(retry_at, at("2026-11-10T19:00:00Z"));
        let message = still_pending_message(&version, retry_at);
        assert!(message.contains("0.4.0") && message.contains("com.example.typst"));
        assert!(message.contains("PENDING") && message.contains("2026-11-10T19:00:00"));
    }

    #[test]
    fn an_upgrade_claim_moves_the_dispatch_time_forward() {
        let claimed_at = at("2026-11-10T20:00:00Z").timestamp();
        let version = pending(
            "2026-11-01T00:00:00Z",
            Some(&format!("{UPGRADE_MARKER}{claimed_at}:claim")),
        );
        assert!(retry_pending_after(&version, at("2026-11-10T20:30:00Z")).is_some());
        assert!(retry_pending_after(&version, at("2026-11-10T21:30:00Z")).is_none());
    }
}
