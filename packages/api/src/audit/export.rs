//! Per-app audit export. The app's anchored seals and their records travel as NDJSON
//! in the archive line format, pushed to the owner's webhook by the audit worker or
//! pulled through the API. Both go through [`page`], so a receiver verifies either the
//! same way: epoch signatures, seal inclusion proofs, record hashes.

use std::collections::{BTreeSet, HashMap};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use chrono::{DateTime, FixedOffset, SubsecRound, Utc};
use flow_like::flow::execution::{ExecutionEnvironment, egress};
use flow_like_types::anyhow;
use flow_like_types::reqwest::{self, Url};
use hmac::{Hmac, Mac};
use sea_orm::sea_query::{Expr, SimpleExpr};
use sea_orm::{
    ActiveValue::Set, ColumnTrait, Condition, ConnectionTrait, DatabaseConnection, DbErr,
    EntityTrait, Order, QueryFilter, QueryOrder, QuerySelect,
};
use sha2::Sha256;

use crate::entity::{audit_epoch, audit_export_target, audit_record, audit_seal, audit_watermark};
use crate::utils::crypto::decrypt_secret;

use super::record::ACTIVITY_SUFFIX;
use super::verify::sort_records;
use super::wire::{self, EpochLine, Line, RawValues, RecordLine, SealLine, WatermarkLine};
use super::worker::{AuditWorkerContext, TickReport, lease::Lease};

pub const MAX_PAGE_SEALS: u64 = 100;
pub const MAX_PAGE_RECORDS: u64 = 5_000;
pub const NDJSON_CONTENT_TYPE: &str = "application/x-ndjson";
pub const DELIVERY_HEADER: &str = "X-FlowLike-Audit-Delivery";
pub const TIMESTAMP_HEADER: &str = "X-FlowLike-Audit-Timestamp";
pub const SIGNATURE_HEADER: &str = "X-FlowLike-Audit-Signature";
pub const SECRET_PREFIX: &str = "whsec_";

const TARGETS_PER_TICK: u64 = 50;
const CONCURRENT_DELIVERIES: usize = 10;
const DELIVERY_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_FAILURES: i32 = 50;
const MAX_BACKOFF_MINUTES: i64 = 6 * 60;
const MAX_ERROR_CHARS: usize = 200;

/// Last app id of the previous tick's batch, so more than [`TARGETS_PER_TICK`] due
/// targets are served in turn instead of the first ones every tick.
static ROTATION: Mutex<Option<String>> = Mutex::new(None);

#[derive(Clone, Debug, Default)]
pub struct ExportPage {
    pub body: Vec<u8>,
    /// Newest seal sequence the page covers, or the watermark's when the records up to
    /// it were pruned before they were read. `None` when there is nothing new.
    pub last_seq: Option<i64>,
    pub seals: u64,
    pub records: u64,
}

struct ChainSlice {
    watermark: Option<audit_watermark::Model>,
    seals: Vec<audit_seal::Model>,
    last_seq: Option<i64>,
}

impl ChainSlice {
    fn record_count(&self) -> u64 {
        self.seals
            .iter()
            .map(|seal| u64::try_from(seal.record_count).unwrap_or(0))
            .sum()
    }
}

/// One page of a chain after `after_seq`: anchored seals only, in sequence order, at
/// most `max_seals` seals and `max_records` records (a single larger seal still goes
/// alone, so a page always makes progress).
pub async fn page<C: ConnectionTrait>(
    db: &C,
    chain_id: &str,
    after_seq: i64,
    max_seals: u64,
    max_records: u64,
) -> Result<ExportPage, DbErr> {
    let slice = chain_slice(db, chain_id, after_seq, max_seals, max_records).await?;
    let (body, seals, records) = render(db, &[&slice]).await?;
    Ok(ExportPage {
        body,
        last_seq: slice.last_seq,
        seals,
        records,
    })
}

async fn chain_slice<C: ConnectionTrait>(
    db: &C,
    chain_id: &str,
    after_seq: i64,
    max_seals: u64,
    max_records: u64,
) -> Result<ChainSlice, DbErr> {
    let mut slice = ChainSlice {
        watermark: None,
        seals: Vec::new(),
        last_seq: None,
    };
    if max_seals == 0 || max_records == 0 {
        return Ok(slice);
    }
    let candidates = audit_seal::Entity::find()
        .filter(audit_seal::Column::ChainId.eq(chain_id))
        .filter(audit_seal::Column::Seq.gt(after_seq))
        .order_by(audit_seal::Column::Seq, Order::Asc)
        .limit(max_seals)
        .all(db)
        .await?;
    let mut records = 0u64;
    for seal in candidates {
        // Epochs anchor a chain's seals in sequence order: stop at the first one that
        // no epoch covers yet, so the cursor never skips a seal.
        if seal.epoch_seq.is_none() {
            break;
        }
        let count = u64::try_from(seal.record_count).unwrap_or(0);
        if !slice.seals.is_empty() && records + count > max_records {
            break;
        }
        records += count;
        slice.seals.push(seal);
    }
    // A watermark at the cursor only matters next to seals; alone it would be resent
    // on every page without moving the cursor.
    let min_watermark = if slice.seals.is_empty() {
        after_seq.saturating_add(1)
    } else {
        after_seq
    };
    slice.watermark = audit_watermark::Entity::find_by_id(chain_id)
        .one(db)
        .await?
        .filter(|watermark| watermark.seq >= min_watermark);
    slice.last_seq = std::cmp::max(
        slice.seals.last().map(|seal| seal.seq),
        slice.watermark.as_ref().map(|watermark| watermark.seq),
    );
    Ok(slice)
}

/// Watermarks first, then every referenced epoch once, then each seal followed by its
/// records in hash order. Raw values travel: the app owner controls its users' data.
async fn render<C: ConnectionTrait>(
    db: &C,
    slices: &[&ChainSlice],
) -> Result<(Vec<u8>, u64, u64), DbErr> {
    let seals: Vec<&audit_seal::Model> =
        slices.iter().flat_map(|slice| slice.seals.iter()).collect();
    let epoch_seqs: BTreeSet<i64> = seals.iter().filter_map(|seal| seal.epoch_seq).collect();
    let epochs = if epoch_seqs.is_empty() {
        Vec::new()
    } else {
        audit_epoch::Entity::find()
            .filter(audit_epoch::Column::Seq.is_in(epoch_seqs))
            .order_by(audit_epoch::Column::Seq, Order::Asc)
            .all(db)
            .await?
    };
    let mut by_seal: HashMap<String, Vec<audit_record::Model>> = HashMap::new();
    if !seals.is_empty() {
        let seal_ids = seals.iter().map(|seal| seal.id.clone());
        let records = audit_record::Entity::find()
            .filter(audit_record::Column::SealId.is_in(seal_ids))
            .all(db)
            .await?;
        for record in records {
            if let Some(seal_id) = record.seal_id.clone() {
                by_seal.entry(seal_id).or_default().push(record);
            }
        }
    }

    let mut body = Vec::new();
    for watermark in slices.iter().filter_map(|slice| slice.watermark.as_ref()) {
        body.extend(wire::encode(&Line::Watermark(WatermarkLine::from(
            watermark,
        ))));
    }
    for epoch in &epochs {
        body.extend(wire::encode(&Line::Epoch(EpochLine::from(epoch))));
    }
    let mut record_count = 0u64;
    for seal in &seals {
        body.extend(wire::encode(&Line::Seal(SealLine::from(*seal))));
        let mut members = by_seal.remove(&seal.id).unwrap_or_default();
        sort_records(&mut members);
        for record in &members {
            body.extend(wire::encode(&Line::Record(RecordLine::from_model(
                record,
                RawValues::ALL,
            ))));
        }
        record_count += members.len() as u64;
    }
    Ok((body, seals.len() as u64, record_count))
}

/// A new webhook signing secret: 32 random bytes, hex, with the `whsec_` prefix.
pub fn new_secret() -> flow_like_types::Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes)
        .map_err(|error| anyhow!("audit webhook secret could not be generated: {error}"))?;
    Ok(format!("{SECRET_PREFIX}{}", hex::encode(bytes)))
}

/// The `v1` signature: hex HMAC-SHA256 keyed with the secret string (prefix included)
/// over `"<timestamp>.<body>"`.
pub fn signature(secret: &str, timestamp: i64, body: &[u8]) -> String {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(secret.as_bytes())
        .expect("HMAC accepts keys of any length");
    mac.update(timestamp.to_string().as_bytes());
    mac.update(b".");
    mac.update(body);
    hex::encode(mac.finalize().into_bytes())
}

/// A webhook URL that passed the egress checks, with the address deliveries connect to.
pub struct WebhookTarget {
    url: Url,
    /// Domain and the checked address it resolved to; `None` for an IP literal.
    pinned: Option<(String, SocketAddr)>,
}

impl WebhookTarget {
    /// A client that connects only to the checked address: no proxy, no redirects.
    fn client(&self) -> flow_like_types::Result<reqwest::Client> {
        let mut builder = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .timeout(DELIVERY_TIMEOUT);
        if let Some((domain, addr)) = &self.pinned {
            builder = builder.resolve(domain, *addr);
        }
        builder
            .build()
            .map_err(|error| anyhow!("audit webhook HTTP client could not be built: {error}"))
    }
}

/// Check a webhook URL against the platform's egress policy and resolve it. Server-side
/// it must be `https` and every address it resolves to must be public; plain `http` and
/// private addresses are allowed only when the process runs as a local environment.
pub async fn check_url(raw: &str) -> flow_like_types::Result<WebhookTarget> {
    let url = Url::parse(raw)
        .map_err(|error| anyhow!("audit webhook URL is not a valid URL: {error}"))?;
    let environment = ExecutionEnvironment::server_default();
    let guarded = environment == ExecutionEnvironment::Server;
    match url.scheme() {
        "https" => {}
        "http" if !guarded => {}
        scheme => {
            return Err(anyhow!("audit webhook URL must use https, not '{scheme}'"));
        }
    }
    let host = url
        .host_str()
        .filter(|host| !host.is_empty())
        .ok_or_else(|| anyhow!("audit webhook URL has no host"))?
        .to_owned();
    let port = url
        .port_or_known_default()
        .ok_or_else(|| anyhow!("audit webhook URL has no port"))?;
    egress::ensure_url_allowed(environment, &url)
        .map_err(|_| anyhow!("audit webhook host '{host}' is not a public address"))?;
    let literal = host
        .strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(&host)
        .parse::<IpAddr>()
        .ok();
    let addrs = match literal {
        Some(ip) => vec![SocketAddr::new(ip, port)],
        // The resolution error is not passed on: it would tell the caller how the
        // platform's network resolves names it should not learn about.
        None => egress::resolve_socket_addrs(environment, &host, port)
            .await
            .map_err(|_| {
                anyhow!("audit webhook host '{host}' does not resolve to a public address")
            })?,
    };
    if guarded && addrs.iter().any(|addr| !is_public(addr.ip())) {
        return Err(anyhow!(
            "audit webhook host '{host}' does not resolve to a public address"
        ));
    }
    let pinned = match (literal, addrs.first()) {
        (None, Some(addr)) => Some((host, *addr)),
        _ => None,
    };
    Ok(WebhookTarget { url, pinned })
}

/// Globally routable unicast. The egress guard blocks the host plane (loopback,
/// link-local, metadata endpoints); a webhook additionally may not reach private,
/// shared, documentation or reserved space.
fn is_public(ip: IpAddr) -> bool {
    if egress::is_blocked_ip(ip) || ip.is_unspecified() {
        return false;
    }
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, c, _] = v4.octets();
            !(v4.is_private()
                || v4.is_documentation()
                || (a == 100 && (64..128).contains(&b))
                || (a == 192 && b == 0 && c == 0)
                || (a == 198 && (b == 18 || b == 19))
                || a >= 240)
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4() {
                return is_public(IpAddr::V4(v4));
            }
            let segments = v6.segments();
            if segments[0] == 0x0064 && segments[1] == 0xff9b {
                let embedded = (u32::from(segments[6]) << 16) | u32::from(segments[7]);
                return is_public(IpAddr::V4(Ipv4Addr::from(embedded)));
            }
            !((segments[0] & 0xfe00) == 0xfc00
                || (segments[0] & 0xffc0) == 0xfec0
                || (segments[0] == 0x2001 && segments[1] == 0x0db8))
        }
    }
}

struct Delivery {
    body: Vec<u8>,
    evidence_last: Option<i64>,
    activity_last: Option<i64>,
}

/// Deliver pending pages to every due webhook: at most [`TARGETS_PER_TICK`] per tick,
/// [`CONCURRENT_DELIVERIES`] at a time, with the lease renewed between batches.
pub async fn deliver(
    context: &AuditWorkerContext,
    lease: &Lease,
    now: DateTime<Utc>,
    report: &mut TickReport,
) -> flow_like_types::Result<()> {
    let Some(key) = context.encryption_key else {
        return Ok(());
    };
    let targets = due_targets(&context.db, now).await?;
    for batch in targets.chunks(CONCURRENT_DELIVERIES) {
        if !lease.keepalive().await {
            tracing::warn!("audit export stopped: the worker lost its lease");
            return Ok(());
        }
        let outcomes = futures::future::join_all(
            batch
                .iter()
                .map(|target| deliver_one(&context.db, &key, target, now)),
        )
        .await;
        for (target, outcome) in batch.iter().zip(outcomes) {
            match outcome {
                Ok(true) => report.exports_delivered += 1,
                Ok(false) => {}
                Err(error) => tracing::error!(
                    target: "audit",
                    app_id = %target.app_id,
                    %error,
                    "audit export delivery could not be recorded"
                ),
            }
        }
    }
    Ok(())
}

async fn due_targets(
    db: &DatabaseConnection,
    now: DateTime<Utc>,
) -> Result<Vec<audit_export_target::Model>, DbErr> {
    let now = now.fixed_offset();
    let due = || {
        audit_export_target::Entity::find()
            .filter(audit_export_target::Column::Active.eq(true))
            .filter(
                Condition::any()
                    .add(audit_export_target::Column::NextAttemptAt.is_null())
                    .add(audit_export_target::Column::NextAttemptAt.lte(now)),
            )
            .order_by(audit_export_target::Column::AppId, Order::Asc)
            .limit(TARGETS_PER_TICK)
    };
    let after = ROTATION
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let mut targets = match after {
        Some(after) => {
            due()
                .filter(audit_export_target::Column::AppId.gt(after))
                .all(db)
                .await?
        }
        None => Vec::new(),
    };
    if targets.is_empty() {
        targets = due().all(db).await?;
    }
    *ROTATION.lock().unwrap_or_else(PoisonError::into_inner) =
        if targets.len() as u64 == TARGETS_PER_TICK {
            targets.last().map(|target| target.app_id.clone())
        } else {
            None
        };
    Ok(targets)
}

/// Returns whether a page was delivered. Delivery failures are stored on the target;
/// only database errors are returned.
async fn deliver_one(
    db: &DatabaseConnection,
    key: &[u8; 32],
    target: &audit_export_target::Model,
    now: DateTime<Utc>,
) -> Result<bool, DbErr> {
    let Some(delivery) = next_delivery(db, target).await? else {
        return Ok(false);
    };
    let outcome = match decrypt_secret(&target.secret, key) {
        Some(secret) => post(&target.url, &secret, delivery.body).await,
        None => Err("signing secret cannot be decrypted with the platform key".to_owned()),
    };
    match outcome {
        Ok(()) => {
            record_success(
                db,
                target,
                delivery.evidence_last,
                delivery.activity_last,
                now,
            )
            .await
        }
        Err(error) => {
            record_failure(db, target, &error, now).await?;
            Ok(false)
        }
    }
}

/// The app's evidence chain first, its activity chain with the remaining budget.
async fn next_delivery(
    db: &DatabaseConnection,
    target: &audit_export_target::Model,
) -> Result<Option<Delivery>, DbErr> {
    let evidence = chain_slice(
        db,
        &target.app_id,
        target.evidence_cursor,
        MAX_PAGE_SEALS,
        MAX_PAGE_RECORDS,
    )
    .await?;
    let activity_chain = format!("{}{ACTIVITY_SUFFIX}", target.app_id);
    let activity = chain_slice(
        db,
        &activity_chain,
        target.activity_cursor,
        MAX_PAGE_SEALS.saturating_sub(evidence.seals.len() as u64),
        MAX_PAGE_RECORDS.saturating_sub(evidence.record_count()),
    )
    .await?;
    if evidence.last_seq.is_none() && activity.last_seq.is_none() {
        return Ok(None);
    }
    let (body, _, _) = render(db, &[&evidence, &activity]).await?;
    Ok(Some(Delivery {
        body,
        evidence_last: evidence.last_seq,
        activity_last: activity.last_seq,
    }))
}

/// POST one page. The error is what the owner sees: a status or an error class, never
/// the response body.
async fn post(url: &str, secret: &str, body: Vec<u8>) -> Result<(), String> {
    let target = check_url(url).await.map_err(|error| error.to_string())?;
    let client = target.client().map_err(|error| error.to_string())?;
    let timestamp = Utc::now().timestamp();
    let signature = signature(secret, timestamp, &body);
    let response = client
        .post(target.url)
        .header(reqwest::header::CONTENT_TYPE, NDJSON_CONTENT_TYPE)
        .header(DELIVERY_HEADER, uuid::Uuid::new_v4().to_string())
        .header(TIMESTAMP_HEADER, timestamp.to_string())
        .header(SIGNATURE_HEADER, format!("v1={signature}"))
        .body(body)
        .send()
        .await
        .map_err(|error| error_class(&error).to_owned())?;
    let status = response.status();
    if status.is_success() {
        Ok(())
    } else {
        Err(format!("HTTP {}", status.as_u16()))
    }
}

fn error_class(error: &reqwest::Error) -> &'static str {
    if error.is_timeout() {
        "timeout"
    } else if error.is_connect() {
        "connection failed"
    } else if error.is_redirect() {
        "redirect refused"
    } else if error.is_request() || error.is_body() {
        "request failed"
    } else {
        "delivery failed"
    }
}

/// Only owner changes advance this revision. Advancing at least one stored millisecond
/// also distinguishes two saves in the same millisecond or after a clock adjustment.
pub(crate) fn configuration_revision(now: DateTime<FixedOffset>) -> SimpleExpr {
    Expr::cust_with_values(
        r#"GREATEST("updatedAt" + interval '1 millisecond', $1)"#,
        [now.trunc_subsecs(3)],
    )
}

/// A response may finish after an owner replaces the destination or after another
/// delivery advances it. A fresh encrypted secret also distinguishes delete/recreate.
fn delivery_snapshot(target: &audit_export_target::Model) -> Condition {
    Condition::all()
        .add(audit_export_target::Column::AppId.eq(&target.app_id))
        .add(audit_export_target::Column::Secret.eq(&target.secret))
        .add(audit_export_target::Column::Url.eq(&target.url))
        .add(audit_export_target::Column::Active.eq(target.active))
        .add(audit_export_target::Column::UpdatedAt.eq(target.updated_at))
        .add(audit_export_target::Column::EvidenceCursor.eq(target.evidence_cursor))
        .add(audit_export_target::Column::ActivityCursor.eq(target.activity_cursor))
        .add(audit_export_target::Column::Failures.eq(target.failures))
}

async fn record_success(
    db: &DatabaseConnection,
    target: &audit_export_target::Model,
    evidence_last: Option<i64>,
    activity_last: Option<i64>,
    now: DateTime<Utc>,
) -> Result<bool, DbErr> {
    let now = now.fixed_offset();
    let update = audit_export_target::ActiveModel {
        evidence_cursor: Set(evidence_last.unwrap_or(target.evidence_cursor)),
        activity_cursor: Set(activity_last.unwrap_or(target.activity_cursor)),
        failures: Set(0),
        next_attempt_at: Set(None),
        last_delivered_at: Set(Some(now)),
        last_error: Set(None),
        ..Default::default()
    };
    let updated = audit_export_target::Entity::update_many()
        .set(update)
        .filter(delivery_snapshot(target))
        .exec(db)
        .await?;
    Ok(updated.rows_affected == 1)
}

async fn record_failure(
    db: &DatabaseConnection,
    target: &audit_export_target::Model,
    error: &str,
    now: DateTime<Utc>,
) -> Result<(), DbErr> {
    let now = now.fixed_offset();
    let failures = target.failures.saturating_add(1);
    let backoff = 2i64
        .saturating_pow(failures.max(0).unsigned_abs())
        .min(MAX_BACKOFF_MINUTES);
    let mut update = audit_export_target::ActiveModel {
        failures: Set(failures),
        next_attempt_at: Set(Some(now + chrono::Duration::minutes(backoff))),
        last_error: Set(Some(error.chars().take(MAX_ERROR_CHARS).collect())),
        ..Default::default()
    };
    // Only ever switched off here: a concurrent owner change must not be re-enabled.
    if failures >= MAX_FAILURES {
        update.active = Set(false);
    }
    let updated = audit_export_target::Entity::update_many()
        .set(update)
        .filter(delivery_snapshot(target))
        .exec(db)
        .await?;
    if updated.rows_affected == 1 && failures >= MAX_FAILURES {
        tracing::warn!(
            target: "audit",
            app_id = %target.app_id,
            failures,
            "audit webhook disabled after consecutive failures"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ActiveModelTrait, ConnectOptions, Database};

    #[tokio::test]
    #[ignore = "requires AUDIT_TEST_DATABASE_URL pointing to a disposable PostgreSQL database"]
    async fn delayed_delivery_cannot_update_changed_webhook() {
        let url = std::env::var("AUDIT_TEST_DATABASE_URL")
            .expect("set AUDIT_TEST_DATABASE_URL to a disposable PostgreSQL database");
        let mut options = ConnectOptions::new(url);
        options.sqlx_logging(false);
        let db = Database::connect(options).await.unwrap();
        db.execute_unprepared(
            r#"CREATE TABLE IF NOT EXISTS "AuditExportTarget" (
                "appId" TEXT PRIMARY KEY, url TEXT NOT NULL, secret TEXT NOT NULL,
                active BOOLEAN NOT NULL, "evidenceCursor" BIGINT NOT NULL,
                "activityCursor" BIGINT NOT NULL, failures INTEGER NOT NULL,
                "nextAttemptAt" TIMESTAMPTZ(3), "lastDeliveredAt" TIMESTAMPTZ(3),
                "lastError" TEXT, "createdAt" TIMESTAMPTZ(3) NOT NULL,
                "updatedAt" TIMESTAMPTZ(3) NOT NULL
            )"#,
        )
        .await
        .unwrap();
        let now = Utc::now().trunc_subsecs(3);
        let at = now.fixed_offset();
        let original = audit_export_target::Model {
            app_id: format!("audit-webhook-race-{}", uuid::Uuid::new_v4()),
            url: "https://receiver.example/audit".into(),
            secret: "original-encrypted-secret".into(),
            active: true,
            evidence_cursor: 7,
            activity_cursor: 4,
            failures: 0,
            next_attempt_at: None,
            last_delivered_at: None,
            last_error: None,
            created_at: at,
            updated_at: at,
        };
        let insert: audit_export_target::ActiveModel = original.clone().into();
        insert.insert(&db).await.unwrap();

        // Two owner edits within the same stored millisecond must each invalidate the
        // captured target, even when the second restores the original configuration.
        for (url, active) in [
            ("https://replacement.example/audit", false),
            (original.url.as_str(), true),
        ] {
            audit_export_target::Entity::update_many()
                .col_expr(audit_export_target::Column::Url, Expr::value(url))
                .col_expr(audit_export_target::Column::Active, Expr::value(active))
                .col_expr(
                    audit_export_target::Column::UpdatedAt,
                    configuration_revision(at),
                )
                .filter(audit_export_target::Column::AppId.eq(&original.app_id))
                .exec(&db)
                .await
                .unwrap();
        }
        let edited = webhook(&db, &original.app_id).await;
        assert_eq!(edited.updated_at, at + chrono::Duration::milliseconds(2));
        assert!(
            !record_success(&db, &original, Some(100), Some(80), now)
                .await
                .unwrap()
        );
        record_failure(&db, &original, "old response", now)
            .await
            .unwrap();
        assert_eq!(webhook(&db, &original.app_id).await, edited);

        // A recreated target can have exactly the same stored time and settings. Its
        // newly encrypted secret distinguishes it from the deleted target.
        audit_export_target::Entity::delete_by_id(&original.app_id)
            .exec(&db)
            .await
            .unwrap();
        let replacement = audit_export_target::Model {
            secret: "replacement-encrypted-secret".into(),
            ..original.clone()
        };
        let insert: audit_export_target::ActiveModel = replacement.clone().into();
        insert.insert(&db).await.unwrap();
        assert!(
            !record_success(&db, &original, Some(100), Some(80), now)
                .await
                .unwrap()
        );
        record_failure(&db, &original, "deleted target response", now)
            .await
            .unwrap();
        assert_eq!(webhook(&db, &original.app_id).await, replacement);

        // A current completion advances cursors, and the stale copy cannot overwrite
        // that delivery's progress or turn a success into a failure.
        assert!(
            record_success(&db, &replacement, Some(100), Some(80), now)
                .await
                .unwrap()
        );
        let delivered = webhook(&db, &original.app_id).await;
        assert_eq!(
            (delivered.evidence_cursor, delivered.activity_cursor),
            (100, 80)
        );
        assert_eq!(delivered.updated_at, replacement.updated_at);
        record_failure(&db, &replacement, "stale delivery response", now)
            .await
            .unwrap();
        assert_eq!(webhook(&db, &original.app_id).await, delivered);

        audit_export_target::Entity::update_many()
            .col_expr(audit_export_target::Column::Failures, Expr::value(49))
            .filter(audit_export_target::Column::AppId.eq(&original.app_id))
            .exec(&db)
            .await
            .unwrap();
        let failing = webhook(&db, &original.app_id).await;
        audit_export_target::Entity::update_many()
            .col_expr(audit_export_target::Column::Failures, Expr::value(0))
            .col_expr(
                audit_export_target::Column::UpdatedAt,
                configuration_revision(at),
            )
            .filter(audit_export_target::Column::AppId.eq(&original.app_id))
            .exec(&db)
            .await
            .unwrap();
        let resumed = webhook(&db, &original.app_id).await;
        record_failure(&db, &failing, "old fiftieth failure", now)
            .await
            .unwrap();
        assert!(resumed.active);
        assert_eq!(webhook(&db, &original.app_id).await, resumed);

        audit_export_target::Entity::update_many()
            .col_expr(audit_export_target::Column::Failures, Expr::value(49))
            .filter(audit_export_target::Column::AppId.eq(&original.app_id))
            .exec(&db)
            .await
            .unwrap();
        let current = webhook(&db, &original.app_id).await;
        record_failure(&db, &current, "current fiftieth failure", now)
            .await
            .unwrap();
        let stopped = webhook(&db, &original.app_id).await;
        assert!(!stopped.active);
        assert_eq!(stopped.failures, 50);
        assert_eq!(stopped.updated_at, current.updated_at);

        audit_export_target::Entity::delete_by_id(&original.app_id)
            .exec(&db)
            .await
            .unwrap();
    }

    async fn webhook(db: &DatabaseConnection, app_id: &str) -> audit_export_target::Model {
        audit_export_target::Entity::find_by_id(app_id)
            .one(db)
            .await
            .unwrap()
            .unwrap()
    }

    #[test]
    fn signatures_cover_timestamp_and_body() {
        let signed = signature("whsec_test", 1_700_000_000, b"{\"type\":\"seal\"}\n");
        assert_eq!(signed.len(), 64);
        assert_eq!(
            signed,
            signature("whsec_test", 1_700_000_000, b"{\"type\":\"seal\"}\n")
        );
        assert_ne!(
            signed,
            signature("whsec_test", 1_700_000_001, b"{\"type\":\"seal\"}\n")
        );
        assert_ne!(
            signed,
            signature("whsec_other", 1_700_000_000, b"{\"type\":\"seal\"}\n")
        );
        assert_ne!(signed, signature("whsec_test", 1_700_000_000, b"{}\n"));
    }

    #[test]
    fn secrets_are_prefixed_and_random() {
        let first = new_secret().unwrap();
        let second = new_secret().unwrap();
        assert!(first.starts_with(SECRET_PREFIX));
        assert_eq!(first.len(), SECRET_PREFIX.len() + 64);
        assert_ne!(first, second);
    }

    #[test]
    fn only_public_addresses_are_reachable() {
        for private in [
            "10.0.0.1",
            "172.16.4.2",
            "192.168.1.1",
            "100.64.0.1",
            "127.0.0.1",
            "169.254.169.254",
            "0.0.0.0",
            "192.0.2.10",
            "198.18.0.1",
            "240.0.0.1",
            "::1",
            "fd00::1",
            "fe80::1",
            "::ffff:10.0.0.1",
            "64:ff9b::a00:1",
            "2001:db8::1",
        ] {
            assert!(!is_public(private.parse().unwrap()), "{private}");
        }
        for public in [
            "93.184.216.34",
            "1.1.1.1",
            "2606:4700:4700::1111",
            "::ffff:1.1.1.1",
        ] {
            assert!(is_public(public.parse().unwrap()), "{public}");
        }
    }
}
