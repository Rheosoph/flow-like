use std::sync::{
    Arc,
    atomic::{AtomicU32, Ordering},
};
use std::time::Duration;

use chrono::{DateTime, FixedOffset, Utc};
use flow_like_types::{Value, create_id, tokio};
use sea_orm::{
    ActiveEnum, ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait,
    DatabaseConnection, DatabaseTransaction, DbErr, EntityTrait, IsolationLevel, Order,
    QueryFilter, QueryOrder, QuerySelect, Statement, TransactionTrait, sea_query::NullOrdering,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::db::{AsDbConflict, DbDialect, RetryPolicy, retry_transaction};
use crate::entity::{audit_entry, sea_orm_active_enums::AuditActorType};

use super::chain::{
    EntryHashFields, GENESIS_HASH, HASH_V2_PREFIX, compute_entry_hash, compute_entry_hash_v2,
};
use super::sign::{
    SignatureVerification, current_kid, is_signing_configured, sign_entry,
    verify_entry_signature_for_kid,
};

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct AuditEntryInput {
    pub actor_id: String,
    #[schema(value_type = String)]
    pub actor_type: AuditActorType,
    pub actor_ip: Option<String>,
    pub action: String,
    pub resource_type: String,
    pub resource_id: String,
    /// Chain scope: None = platform root chain, Some(id) = branch chain (app or package)
    pub chain_id: Option<String>,
    pub summary: String,
    #[schema(value_type = Option<Object>)]
    pub details: Option<Value>,
}

impl AuditEntryInput {
    /// An entry for a change no authenticated caller made, such as one applied
    /// from a verified provider webhook. Lands on the root chain unless moved.
    pub fn system(
        actor_id: &str,
        action: &str,
        resource_type: &str,
        resource_id: &str,
        summary: impl Into<String>,
    ) -> Self {
        Self {
            actor_id: actor_id.to_owned(),
            actor_type: AuditActorType::System,
            actor_ip: None,
            action: action.to_owned(),
            resource_type: resource_type.to_owned(),
            resource_id: resource_id.to_owned(),
            chain_id: None,
            summary: summary.into(),
            details: None,
        }
    }

    pub fn on_chain(mut self, chain_id: &str) -> Self {
        self.chain_id = Some(chain_id.to_owned());
        self
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct AuditEntryOutput {
    pub id: String,
    pub sequence: i64,
    pub timestamp: String,
    pub actor_id: String,
    pub actor_type: String,
    pub actor_ip: Option<String>,
    pub action: String,
    pub resource_type: String,
    pub resource_id: String,
    pub chain_id: Option<String>,
    pub summary: String,
    #[schema(value_type = Option<Object>)]
    pub details: Option<Value>,
    pub entry_hash: String,
    pub prev_hash: String,
    pub signature: Option<String>,
    pub kid: Option<String>,
}

impl From<audit_entry::Model> for AuditEntryOutput {
    fn from(m: audit_entry::Model) -> Self {
        Self {
            id: m.id,
            sequence: m.sequence,
            timestamp: m.timestamp.to_rfc3339(),
            actor_id: m.actor_id,
            actor_type: format!("{:?}", m.actor_type),
            actor_ip: m.actor_ip,
            action: m.action,
            resource_type: m.resource_type,
            resource_id: m.resource_id,
            chain_id: m.chain_id,
            summary: m.summary,
            details: m.details,
            entry_hash: m.entry_hash,
            prev_hash: m.prev_hash,
            signature: m.signature,
            kid: m.kid,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct ChainVerification {
    pub valid: bool,
    /// Entries in the requested range plus its verified immediate predecessor or root anchor.
    pub entries_checked: u64,
    pub first_broken_at: Option<i64>,
    /// Every checked entry is signed, verified and uses the complete v2 hash.
    /// An empty chain or one containing unsigned or legacy entries is false.
    pub fully_authenticated: bool,
    pub signatures_verified: u64,
    pub unsigned_entries: u64,
    pub unverifiable_signatures: u64,
    pub legacy_entries: u64,
    /// The chain or requested range holds no entries. A deleted chain looks the same.
    #[serde(default)]
    pub empty: bool,
    /// Root sequence a branch verified from its first entry is anchored to.
    /// None for the root chain, a partial range, or a branch anchored to genesis.
    #[serde(default)]
    pub anchor_sequence: Option<i64>,
}

/// The newest entry of a chain. Retain it outside the database to detect later
/// truncation with the `expected_head_*` verification parameters.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct ChainHead {
    pub chain_id: Option<String>,
    pub sequence: i64,
    pub timestamp: String,
    pub entry_hash: String,
    pub signature: Option<String>,
    pub kid: Option<String>,
}

#[derive(Clone, Debug, Deserialize, ToSchema)]
pub struct AuditFilter {
    pub chain_id: Option<String>,
    pub action: Option<String>,
    pub actor_id: Option<String>,
    pub resource_type: Option<String>,
    pub resource_id: Option<String>,
    /// Keyset cursor: only entries with a lower sequence. Prefer it over `offset`.
    pub before_sequence: Option<i64>,
    pub limit: Option<u64>,
    pub offset: Option<u64>,
}

/// Appends lose commit races on a shared lock row by design, so they get a
/// longer budget than the request default before a mutation is failed.
const APPEND_RETRY: RetryPolicy = RetryPolicy {
    max_attempts: 24,
    base_delay: Duration::from_millis(5),
    max_delay: Duration::from_millis(250),
    max_total: Duration::from_secs(10),
    idempotent: true,
};
const APPEND_DEADLINE: Duration = Duration::from_secs(15);
/// `SET LOCAL`: a stalled writer holding the chain's lock row must not block
/// every other append indefinitely. A timeout surfaces as a retryable `55P03`.
const APPEND_LOCK_TIMEOUT_SQL: &str = "SELECT set_config('lock_timeout', '3s', true)";
const VERIFY_BATCH: u64 = 1_000;
const MAX_QUERY_OFFSET: u64 = 10_000;

type TailRow = (String, Option<String>, i64, DateTime<FixedOffset>);

pub struct AuditService;

impl AuditService {
    /// Record a new audit entry, computing the hash chain and signing it.
    ///
    /// A retained coordination row serializes appends before the tail is read.
    /// The root chain has its own non-null lock id, including when no entry
    /// exists yet. Every retry reads the newly committed tail.
    pub async fn record(
        db: &DatabaseConnection,
        dialect: DbDialect,
        input: AuditEntryInput,
    ) -> flow_like_types::Result<audit_entry::Model> {
        Self::record_internal(db, dialect, input, false).await
    }

    /// Record a lifecycle transition once for a chain, action and resource.
    /// Retried terminal callbacks return the entry from the first successful call.
    pub async fn record_once(
        db: &DatabaseConnection,
        dialect: DbDialect,
        input: AuditEntryInput,
    ) -> flow_like_types::Result<audit_entry::Model> {
        Self::record_internal(db, dialect, input, true).await
    }

    async fn record_internal(
        db: &DatabaseConnection,
        dialect: DbDialect,
        mut input: AuditEntryInput,
        once: bool,
    ) -> flow_like_types::Result<audit_entry::Model> {
        sanitize_input(&mut input);
        let id = create_id();
        let attempts = Arc::new(AtomicU32::new(0));
        let append = retry_transaction::<_, audit_entry::Model, DbErr>(
            db,
            dialect,
            None,
            &APPEND_RETRY,
            move |txn| {
                let input = input.clone();
                let id = id.clone();
                // Only a re-run can find its own id already committed.
                let replay = attempts.fetch_add(1, Ordering::Relaxed) > 0;
                Box::pin(
                    async move { Self::append_entry(txn, dialect, input, id, once, replay).await },
                )
            },
        );
        match tokio::time::timeout(APPEND_DEADLINE, append).await {
            Ok(Ok(entry)) => Ok(entry),
            Ok(Err(error)) => {
                if let Some(conflict) = error.db_conflict() {
                    tracing::error!(
                        conflict = conflict.as_str(),
                        "audit append exhausted its retry budget"
                    );
                }
                Err(error.into())
            }
            Err(_) => Err(flow_like_types::anyhow!(
                "audit append did not finish within {}s",
                APPEND_DEADLINE.as_secs()
            )),
        }
    }

    async fn append_entry(
        txn: &DatabaseTransaction,
        dialect: DbDialect,
        input: AuditEntryInput,
        id: String,
        once: bool,
        replay: bool,
    ) -> Result<audit_entry::Model, DbErr> {
        if dialect.supports_set_config_timeouts() {
            txn.execute_raw(Statement::from_string(
                txn.get_database_backend(),
                APPEND_LOCK_TIMEOUT_SQL,
            ))
            .await?;
        }
        match input.chain_id.as_deref() {
            Some(chain_id) => {
                crate::db::coordination::coordinate(txn, "audit-branch", &[chain_id]).await?;
            }
            None => crate::db::coordination::coordinate(txn, "audit-root", &[]).await?,
        }

        // Reuse the committed result if a previous attempt lost its acknowledgement.
        if replay
            && let Some(existing) = audit_entry::Entity::find_by_id(id.clone()).one(txn).await?
        {
            return Ok(existing);
        }
        if once {
            let existing = audit_entry::Entity::find()
                .filter(chain_filter(input.chain_id.as_deref()))
                .filter(audit_entry::Column::Action.eq(input.action.clone()))
                .filter(audit_entry::Column::ResourceType.eq(input.resource_type.clone()))
                .filter(audit_entry::Column::ResourceId.eq(input.resource_id.clone()))
                .order_by(audit_entry::Column::Sequence, Order::Asc)
                .one(txn)
                .await?;
            if let Some(existing) = existing {
                return Ok(existing);
            }
        }

        // First entry of a branch anchors to the current root tail so branches are
        // cryptographically linked to the global timeline. The root uses genesis.
        let tail = match chain_tail(txn, input.chain_id.as_deref()).await? {
            Some((hash, signature, sequence, timestamp)) => Some((
                hash,
                signature,
                sequence
                    .checked_add(1)
                    .ok_or_else(|| DbErr::Custom("audit sequence overflow".into()))?,
                timestamp,
            )),
            None if input.chain_id.is_some() => chain_tail(txn, None)
                .await?
                .map(|(hash, signature, _, timestamp)| (hash, signature, 1, timestamp)),
            None => None,
        };
        let (prev_hash, prev_signature, next_seq, floor) = match tail {
            Some((hash, signature, sequence, timestamp)) => {
                (hash, signature, sequence, Some(timestamp))
            }
            None => (GENESIS_HASH.to_string(), None, 1, None),
        };
        let now = entry_timestamp(floor);

        let kid = is_signing_configured().then(|| current_kid().to_owned());
        let entry_hash = compute_entry_hash_v2(&EntryHashFields {
            id: &id,
            sequence: next_seq,
            timestamp: &now,
            actor_id: &input.actor_id,
            actor_type: &input.actor_type.to_value(),
            actor_ip: input.actor_ip.as_deref(),
            action: &input.action,
            resource_type: &input.resource_type,
            resource_id: &input.resource_id,
            chain_id: input.chain_id.as_deref(),
            summary: &input.summary,
            details: input.details.as_ref(),
            prev_hash: &prev_hash,
            prev_signature: prev_signature.as_deref(),
            kid: kid.as_deref(),
        });
        let signature = sign_entry(&entry_hash);
        if signature.is_none() && kid.is_some() {
            return Err(DbErr::Custom("audit signing failed".into()));
        }

        let model = audit_entry::ActiveModel {
            id: Set(id),
            sequence: Set(next_seq),
            timestamp: Set(now),
            actor_id: Set(input.actor_id),
            actor_type: Set(input.actor_type),
            actor_ip: Set(input.actor_ip),
            action: Set(input.action),
            resource_type: Set(input.resource_type),
            resource_id: Set(input.resource_id),
            chain_id: Set(input.chain_id),
            summary: Set(input.summary),
            details: Set(input.details),
            entry_hash: Set(entry_hash),
            prev_hash: Set(prev_hash),
            signature: Set(signature),
            kid: Set(kid),
        };

        model.insert(txn).await
    }

    /// Verify hashes, sequence continuity, anchors and every available signature.
    /// One database snapshot keeps the range and its predecessors consistent.
    /// Entries stream through in batches so a long chain never sits in memory.
    pub async fn verify_chain(
        db: &DatabaseConnection,
        dialect: DbDialect,
        chain_id: Option<&str>,
        from_seq: Option<i64>,
        to_seq: Option<i64>,
    ) -> flow_like_types::Result<ChainVerification> {
        let from = from_seq.unwrap_or(1);
        if from < 1 || to_seq.is_some_and(|to| to < from) {
            return Err(flow_like_types::anyhow!(
                "audit sequence range must be positive and ascending"
            ));
        }
        let txn = db
            .begin_with_config(
                dialect.effective_isolation(Some(IsolationLevel::RepeatableRead)),
                None,
            )
            .await?;
        let mut batch = verification_batch(&txn, chain_id, from_seq, to_seq).await?;
        let mut anchor_missing = false;
        let mut anchor_sequence = None;
        let previous = if from > 1 {
            let previous = audit_entry::Entity::find()
                .filter(chain_filter(chain_id))
                .filter(audit_entry::Column::Sequence.eq(from - 1))
                .all(&txn)
                .await?;
            anchor_missing = previous.len() != 1;
            previous.into_iter().next()
        } else if chain_id.is_some() && batch.first().is_some_and(|e| e.prev_hash != GENESIS_HASH) {
            // A branch's initial hash and previous signature come from the same root entry.
            // Never trust an arbitrary prev_hash stored on the branch itself.
            let anchors = audit_entry::Entity::find()
                .filter(audit_entry::Column::ChainId.is_null())
                .filter(audit_entry::Column::EntryHash.eq(batch[0].prev_hash.clone()))
                .all(&txn)
                .await?;
            anchor_missing = anchors.len() != 1;
            anchor_sequence = anchors.first().map(|anchor| anchor.sequence);
            anchors.into_iter().next()
        } else {
            None
        };

        let empty = batch.is_empty();
        let mut verifier = ChainVerifier::new(
            chain_id,
            from,
            previous.as_ref(),
            verify_entry_signature_for_kid,
        );
        // The unique index does not cover the root chain, so a sequence can repeat.
        // Re-reading from the last sequence and skipping the consumed row keeps a
        // duplicate at a batch boundary visible.
        let mut consumed: Option<(i64, String)> = None;
        'chain: loop {
            let full = batch.len() as u64 == VERIFY_BATCH;
            let mut advanced = false;
            for entry in &batch {
                if consumed
                    .as_ref()
                    .is_some_and(|(sequence, id)| entry.sequence == *sequence && entry.id == *id)
                {
                    continue;
                }
                advanced = true;
                if !verifier.push(entry) {
                    break 'chain;
                }
            }
            if !full || !advanced {
                break;
            }
            consumed = batch.pop().map(|entry| (entry.sequence, entry.id));
            let lower = consumed.as_ref().map(|(sequence, _)| *sequence);
            batch = verification_batch(&txn, chain_id, lower, to_seq).await?;
        }
        let mut result = verifier.finish(to_seq);
        result.empty = empty;
        result.anchor_sequence = anchor_sequence;

        if let Some(previous) = previous.as_ref() {
            // Authenticate the boundary too. A current-key branch cannot establish
            // the authenticity of an anchor whose historical key is unavailable.
            let prior = if previous.sequence > 1 {
                audit_entry::Entity::find()
                    .filter(chain_filter(previous.chain_id.as_deref()))
                    .filter(audit_entry::Column::Sequence.eq(previous.sequence - 1))
                    .all(&txn)
                    .await?
            } else if previous.chain_id.is_some() && previous.prev_hash != GENESIS_HASH {
                audit_entry::Entity::find()
                    .filter(audit_entry::Column::ChainId.is_null())
                    .filter(audit_entry::Column::EntryHash.eq(previous.prev_hash.clone()))
                    .all(&txn)
                    .await?
            } else {
                Vec::new()
            };
            let needs_prior = previous.sequence > 1 || previous.prev_hash != GENESIS_HASH;
            let boundary = verify_entries(
                std::slice::from_ref(previous),
                previous.chain_id.as_deref(),
                previous.sequence,
                Some(previous.sequence),
                prior.first(),
                verify_entry_signature_for_kid,
            );
            result.entries_checked += boundary.entries_checked;
            result.signatures_verified += boundary.signatures_verified;
            result.unsigned_entries += boundary.unsigned_entries;
            result.unverifiable_signatures += boundary.unverifiable_signatures;
            result.legacy_entries += boundary.legacy_entries;
            result.valid &= boundary.valid;
            result.fully_authenticated &= boundary.fully_authenticated;
            if boundary.first_broken_at.is_some()
                || previous.sequence < 1
                || (needs_prior && prior.len() != 1)
            {
                // Report the first selected entry that depends on this boundary.
                result.mark_broken(from);
            }
        }
        if anchor_missing {
            result.mark_broken(from);
        }
        // A requested range is evidence only when its boundaries exist.
        if empty && from_seq.is_some() {
            result.mark_broken(from);
        }
        txn.commit().await?;
        Ok(result)
    }

    /// The configured key must reproduce the newest signature made under its id.
    /// A rotated key that kept its id, or a replica with a different key, would
    /// otherwise turn every earlier entry into an apparent forgery.
    pub async fn check_signing_key_continuity(db: &DatabaseConnection) -> Result<(), String> {
        if !is_signing_configured() {
            return Ok(());
        }
        let tail = match newest_first(audit_entry::Entity::find().filter(chain_filter(None)))
            .one(db)
            .await
        {
            Ok(Some(tail)) => tail,
            Ok(None) => return Ok(()),
            // A database that is not migrated or reachable yet is not a key mismatch.
            Err(error) => {
                tracing::warn!(%error, "audit signing key continuity was not checked");
                return Ok(());
            }
        };
        match (tail.kid.as_deref(), tail.signature.as_deref()) {
            (Some(kid), Some(signature))
                if kid == current_kid()
                    && verify_entry_signature_for_kid(&tail.entry_hash, signature, kid)
                        != SignatureVerification::Valid =>
            {
                Err(format!(
                    "audit entry {} was signed under key id {kid} by a different key; \
                     a rotated signing key needs a new key id",
                    tail.sequence
                ))
            }
            _ => Ok(()),
        }
    }

    /// The newest entry of a chain, for retention outside this database.
    pub async fn head(
        db: &DatabaseConnection,
        chain_id: Option<&str>,
    ) -> flow_like_types::Result<Option<ChainHead>> {
        let tail = newest_first(audit_entry::Entity::find().filter(chain_filter(chain_id)))
            .one(db)
            .await?;
        Ok(tail.map(|entry| ChainHead {
            chain_id: entry.chain_id,
            sequence: entry.sequence,
            timestamp: entry.timestamp.to_rfc3339(),
            entry_hash: entry.entry_hash,
            signature: entry.signature,
            kid: entry.kid,
        }))
    }

    /// Whether a previously retained head is still part of the chain. A missing
    /// or different entry at that sequence means the chain was truncated or rewritten.
    pub async fn contains_head(
        db: &DatabaseConnection,
        chain_id: Option<&str>,
        sequence: i64,
        entry_hash: &str,
    ) -> flow_like_types::Result<bool> {
        let hashes: Vec<String> = audit_entry::Entity::find()
            .select_only()
            .column(audit_entry::Column::EntryHash)
            .filter(chain_filter(chain_id))
            .filter(audit_entry::Column::Sequence.eq(sequence))
            .into_tuple()
            .all(db)
            .await?;
        Ok(hashes.len() == 1 && hashes[0] == entry_hash)
    }

    /// Query audit entries with filters.
    pub async fn query(
        db: &DatabaseConnection,
        filter: AuditFilter,
    ) -> flow_like_types::Result<Vec<audit_entry::Model>> {
        let mut query = audit_entry::Entity::find();

        query = query.filter(chain_filter(filter.chain_id.as_deref()));
        if let Some(ref action) = filter.action {
            if action.ends_with(".*") {
                let prefix = &action[..action.len() - 1];
                query = query.filter(audit_entry::Column::Action.starts_with(prefix));
            } else {
                query = query.filter(audit_entry::Column::Action.eq(action.clone()));
            }
        }
        if let Some(ref actor) = filter.actor_id {
            query = query.filter(audit_entry::Column::ActorId.eq(actor.clone()));
        }
        if let Some(ref rt) = filter.resource_type {
            query = query.filter(audit_entry::Column::ResourceType.eq(rt.clone()));
        }
        if let Some(ref rid) = filter.resource_id {
            query = query.filter(audit_entry::Column::ResourceId.eq(rid.clone()));
        }

        if let Some(before) = filter.before_sequence {
            query = query.filter(audit_entry::Column::Sequence.lt(before));
        }

        let limit = filter.limit.unwrap_or(50).min(200);
        let offset = filter.offset.unwrap_or(0).min(MAX_QUERY_OFFSET);

        let entries = newest_first(query)
            .offset(offset)
            .limit(limit)
            .all(db)
            .await?;

        Ok(entries)
    }
}

/// Order by the full `(chainId, sequence)` index key. DSQL only scans that index
/// backward when both columns are ordered; `sequence` alone makes it read and sort
/// the whole root chain (`chainId IS NULL`), which took over a second per append.
pub(crate) fn newest_first<Q: QueryOrder>(query: Q) -> Q {
    query
        .order_by_with_nulls(
            audit_entry::Column::ChainId,
            Order::Desc,
            NullOrdering::First,
        )
        .order_by(audit_entry::Column::Sequence, Order::Desc)
}

pub(crate) fn chain_filter(chain_id: Option<&str>) -> sea_orm::sea_query::SimpleExpr {
    match chain_id {
        Some(cid) => audit_entry::Column::ChainId.eq(cid),
        None => audit_entry::Column::ChainId.is_null(),
    }
}

/// Forward scan of the same `(chainId, sequence)` index key.
fn oldest_first<Q: QueryOrder>(query: Q) -> Q {
    query
        .order_by_with_nulls(audit_entry::Column::ChainId, Order::Asc, NullOrdering::Last)
        .order_by(audit_entry::Column::Sequence, Order::Asc)
}

async fn verification_batch(
    txn: &DatabaseTransaction,
    chain_id: Option<&str>,
    lower: Option<i64>,
    upper: Option<i64>,
) -> Result<Vec<audit_entry::Model>, DbErr> {
    let mut query = audit_entry::Entity::find().filter(chain_filter(chain_id));
    if let Some(lower) = lower {
        query = query.filter(audit_entry::Column::Sequence.gte(lower));
    }
    if let Some(upper) = upper {
        query = query.filter(audit_entry::Column::Sequence.lte(upper));
    }
    oldest_first(query).limit(VERIFY_BATCH).all(txn).await
}

/// Only the fields the next entry links to. `details` stays out of the
/// serialized append transaction.
async fn chain_tail(
    txn: &DatabaseTransaction,
    chain_id: Option<&str>,
) -> Result<Option<TailRow>, DbErr> {
    newest_first(
        audit_entry::Entity::find()
            .select_only()
            .columns([
                audit_entry::Column::EntryHash,
                audit_entry::Column::Signature,
                audit_entry::Column::Sequence,
                audit_entry::Column::Timestamp,
            ])
            .filter(chain_filter(chain_id)),
    )
    .into_tuple::<TailRow>()
    .one(txn)
    .await
}

/// Taken under the chain lock and never earlier than the entry it links to, so
/// sequence order and time order agree within a chain and with its anchor.
/// Milliseconds match `timestamptz(3)`; PostgreSQL would otherwise round the
/// stored value away from the hashed one.
fn entry_timestamp(floor: Option<DateTime<FixedOffset>>) -> DateTime<FixedOffset> {
    let now = DateTime::from_timestamp_millis(Utc::now().timestamp_millis())
        .expect("the current time fits in milliseconds")
        .fixed_offset();
    floor.map_or(now, |floor| now.max(floor))
}

fn strip_nul(text: &mut String) {
    if text.contains('\0') {
        *text = text.replace('\0', "\u{fffd}");
    }
}

fn strip_nul_value(value: &mut Value) {
    match value {
        Value::String(text) => strip_nul(text),
        Value::Array(items) => items.iter_mut().for_each(strip_nul_value),
        Value::Object(map) => {
            for (mut key, mut item) in std::mem::take(map) {
                strip_nul(&mut key);
                strip_nul_value(&mut item);
                map.insert(key, item);
            }
        }
        _ => {}
    }
}

/// PostgreSQL rejects U+0000 in TEXT and JSONB. Replacing it before hashing keeps
/// the stored row equal to the signed one, and a hostile string cannot make the
/// entry for an already committed mutation fail.
fn sanitize_input(input: &mut AuditEntryInput) {
    for text in [
        &mut input.actor_id,
        &mut input.action,
        &mut input.resource_type,
        &mut input.resource_id,
        &mut input.summary,
    ] {
        strip_nul(text);
    }
    for text in [&mut input.actor_ip, &mut input.chain_id]
        .into_iter()
        .flatten()
    {
        strip_nul(text);
    }
    if let Some(details) = input.details.as_mut() {
        strip_nul_value(details);
    }
}

fn model_hash(entry: &audit_entry::Model, prev_signature: Option<&str>) -> String {
    if entry.entry_hash.starts_with(HASH_V2_PREFIX) {
        compute_entry_hash_v2(&EntryHashFields {
            id: &entry.id,
            sequence: entry.sequence,
            timestamp: &entry.timestamp,
            actor_id: &entry.actor_id,
            actor_type: &entry.actor_type.to_value(),
            actor_ip: entry.actor_ip.as_deref(),
            action: &entry.action,
            resource_type: &entry.resource_type,
            resource_id: &entry.resource_id,
            chain_id: entry.chain_id.as_deref(),
            summary: &entry.summary,
            details: entry.details.as_ref(),
            prev_hash: &entry.prev_hash,
            prev_signature,
            kid: entry.kid.as_deref(),
        })
    } else {
        compute_entry_hash(
            entry.sequence,
            &entry.timestamp,
            &entry.actor_id,
            &entry.action,
            &entry.resource_type,
            &entry.resource_id,
            entry.details.as_ref(),
            &entry.prev_hash,
            prev_signature,
        )
    }
}

impl ChainVerification {
    pub(crate) fn mark_broken(&mut self, sequence: i64) {
        self.valid = false;
        self.fully_authenticated = false;
        self.first_broken_at = Some(
            self.first_broken_at
                .map_or(sequence, |old| old.min(sequence)),
        );
    }
}

/// Carries only the link state between entries, so a chain can be verified in
/// batches without holding it in memory.
struct ChainVerifier<'a, F> {
    chain_id: Option<&'a str>,
    verify_signature: F,
    result: ChainVerification,
    expected_hash: String,
    prev_signature: Option<String>,
    expected_sequence: Option<i64>,
    last_sequence: Option<i64>,
    seen_v2: bool,
    seen_signed: bool,
    broken: bool,
}

impl<'a, F: Fn(&str, &str, &str) -> SignatureVerification> ChainVerifier<'a, F> {
    fn new(
        chain_id: Option<&'a str>,
        from: i64,
        previous: Option<&audit_entry::Model>,
        verify_signature: F,
    ) -> Self {
        Self {
            chain_id,
            verify_signature,
            result: ChainVerification {
                valid: true,
                entries_checked: 0,
                first_broken_at: None,
                fully_authenticated: false,
                signatures_verified: 0,
                unsigned_entries: 0,
                unverifiable_signatures: 0,
                legacy_entries: 0,
                empty: false,
                anchor_sequence: None,
            },
            expected_hash: previous.map_or_else(
                || GENESIS_HASH.to_string(),
                |entry| entry.entry_hash.clone(),
            ),
            prev_signature: previous.and_then(|entry| entry.signature.clone()),
            expected_sequence: Some(from),
            last_sequence: None,
            seen_v2: previous.is_some_and(|entry| entry.entry_hash.starts_with(HASH_V2_PREFIX)),
            seen_signed: previous.is_some_and(|entry| entry.signature.is_some()),
            broken: false,
        }
    }

    fn fail(&mut self, sequence: i64) -> bool {
        self.result.mark_broken(sequence);
        self.broken = true;
        false
    }

    /// Returns false once the chain is broken; later entries prove nothing.
    fn push(&mut self, entry: &audit_entry::Model) -> bool {
        self.result.entries_checked += 1;
        let v2 = entry.entry_hash.starts_with(HASH_V2_PREFIX);
        if !v2 {
            self.result.legacy_entries += 1;
        }
        if self.expected_sequence != Some(entry.sequence)
            || entry.chain_id.as_deref() != self.chain_id
            || entry.prev_hash != self.expected_hash
            || (self.seen_v2 && !v2)
            || model_hash(entry, self.prev_signature.as_deref()) != entry.entry_hash
        {
            return self.fail(
                self.expected_sequence
                    .unwrap_or(entry.sequence)
                    .min(entry.sequence),
            );
        }
        match (entry.signature.as_deref(), entry.kid.as_deref()) {
            (Some(signature), Some(kid)) => {
                match (self.verify_signature)(&entry.entry_hash, signature, kid) {
                    SignatureVerification::Valid => self.result.signatures_verified += 1,
                    SignatureVerification::Invalid => return self.fail(entry.sequence),
                    SignatureVerification::Unavailable => {
                        self.result.unverifiable_signatures += 1;
                        self.result.valid = false;
                    }
                }
            }
            // Hashes need no key, so an unsigned entry after signed history is
            // what a rewrite without the signing key looks like.
            (None, None) if self.seen_signed => return self.fail(entry.sequence),
            (None, None) => self.result.unsigned_entries += 1,
            _ => return self.fail(entry.sequence),
        }
        self.expected_hash.clone_from(&entry.entry_hash);
        self.prev_signature.clone_from(&entry.signature);
        self.expected_sequence = entry.sequence.checked_add(1);
        self.last_sequence = Some(entry.sequence);
        self.seen_v2 |= v2;
        self.seen_signed |= entry.signature.is_some();
        true
    }

    fn finish(mut self, to: Option<i64>) -> ChainVerification {
        if !self.broken
            && let Some(to) = to
            && self.last_sequence != Some(to)
        {
            self.result
                .mark_broken(self.expected_sequence.unwrap_or(to));
        }
        self.result.empty = self.result.entries_checked == 0;
        self.result.fully_authenticated = self.result.valid
            && self.result.entries_checked > 0
            && self.result.signatures_verified == self.result.entries_checked
            && self.result.legacy_entries == 0;
        self.result
    }
}

fn verify_entries(
    entries: &[audit_entry::Model],
    chain_id: Option<&str>,
    from: i64,
    to: Option<i64>,
    previous: Option<&audit_entry::Model>,
    verify_signature: impl Fn(&str, &str, &str) -> SignatureVerification,
) -> ChainVerification {
    let mut verifier = ChainVerifier::new(chain_id, from, previous, verify_signature);
    for entry in entries {
        if !verifier.push(entry) {
            break;
        }
    }
    verifier.finish(to)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::base64::{Engine, engine::general_purpose::STANDARD};
    use p256::ecdsa::{
        Signature, SigningKey,
        signature::{Signer, Verifier},
    };

    fn test_key() -> SigningKey {
        SigningKey::from_slice(&[7; 32]).unwrap()
    }

    fn verify_test_signature(hash: &str, signature: &str, kid: &str) -> SignatureVerification {
        if kid != "test-key" {
            return SignatureVerification::Unavailable;
        }
        let valid = STANDARD
            .decode(signature)
            .ok()
            .and_then(|bytes| Signature::from_der(&bytes).ok())
            .is_some_and(|signature| {
                test_key()
                    .verifying_key()
                    .verify(hash.as_bytes(), &signature)
                    .is_ok()
            });
        if valid {
            SignatureVerification::Valid
        } else {
            SignatureVerification::Invalid
        }
    }

    fn seal(entry: &mut audit_entry::Model, previous: Option<&audit_entry::Model>) {
        entry.prev_hash = previous.map_or_else(
            || GENESIS_HASH.to_string(),
            |entry| entry.entry_hash.clone(),
        );
        entry.entry_hash = model_hash(entry, previous.and_then(|entry| entry.signature.as_deref()));
        entry.signature = entry.kid.as_ref().map(|_| {
            let signature: Signature = test_key().sign(entry.entry_hash.as_bytes());
            STANDARD.encode(signature.to_der())
        });
    }

    fn entry(
        sequence: i64,
        chain_id: Option<&str>,
        previous: Option<&audit_entry::Model>,
    ) -> audit_entry::Model {
        let mut entry = audit_entry::Model {
            id: format!("entry-{sequence}"),
            sequence,
            timestamp: chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00.123Z").unwrap(),
            actor_id: "actor".into(),
            actor_type: AuditActorType::User,
            actor_ip: Some("192.0.2.1".into()),
            action: "app.create".into(),
            resource_type: "App".into(),
            resource_id: "app".into(),
            chain_id: chain_id.map(str::to_owned),
            summary: "Created an app".into(),
            details: Some(serde_json::json!({"b": [2, {"z": false, "a": 1}], "a": "x"})),
            entry_hash: HASH_V2_PREFIX.into(),
            prev_hash: GENESIS_HASH.into(),
            signature: None,
            kid: Some("test-key".into()),
        };
        seal(&mut entry, previous);
        entry
    }

    fn verify(
        entries: &[audit_entry::Model],
        chain: Option<&str>,
        from: i64,
        to: Option<i64>,
        previous: Option<&audit_entry::Model>,
    ) -> ChainVerification {
        verify_entries(entries, chain, from, to, previous, verify_test_signature)
    }

    #[test]
    fn signed_chain_and_partial_range_authenticate() {
        let first = entry(1, None, None);
        let second = entry(2, None, Some(&first));
        let result = verify(&[first.clone(), second.clone()], None, 1, None, None);
        assert!(result.valid && result.fully_authenticated);
        assert_eq!(result.signatures_verified, 2);
        assert!(verify(&[second], None, 2, Some(2), Some(&first)).fully_authenticated);
    }

    #[test]
    fn signed_branch_uses_root_anchor_hash_and_signature() {
        let root = entry(1, None, None);
        let branch = entry(1, Some("app"), Some(&root));
        assert!(verify(&[branch.clone()], Some("app"), 1, None, Some(&root)).fully_authenticated);
        assert!(!verify(&[branch.clone()], Some("app"), 1, None, None).valid);
        let mut corrupted_anchor = root;
        corrupted_anchor.signature = Some("tampered".into());
        assert!(!verify(&[branch], Some("app"), 1, None, Some(&corrupted_anchor)).valid);
    }

    #[test]
    fn every_immutable_field_is_authenticated() {
        let original = entry(1, Some("app"), None);
        let mutations: Vec<Box<dyn Fn(&mut audit_entry::Model)>> = vec![
            Box::new(|e| e.id.push('x')),
            Box::new(|e| e.sequence += 1),
            Box::new(|e| e.timestamp += chrono::Duration::milliseconds(1)),
            Box::new(|e| e.actor_id.push('x')),
            Box::new(|e| e.actor_type = AuditActorType::System),
            Box::new(|e| e.actor_ip = None),
            Box::new(|e| e.action.push('x')),
            Box::new(|e| e.resource_type.push('x')),
            Box::new(|e| e.resource_id.push('x')),
            Box::new(|e| e.chain_id = Some("other".into())),
            Box::new(|e| e.summary.push('x')),
            Box::new(|e| e.details = None),
            Box::new(|e| e.prev_hash.push('x')),
            Box::new(|e| e.kid = None),
        ];
        for (index, mutate) in mutations.into_iter().enumerate() {
            let mut tampered = original.clone();
            mutate(&mut tampered);
            assert!(
                !verify(&[tampered], Some("app"), 1, None, None).valid,
                "mutation {index}"
            );
        }
    }

    #[test]
    fn v2_frames_adjacent_fields_and_optional_details() {
        let mut first = entry(1, None, None);
        first.actor_id = "ab".into();
        first.action = "c".into();
        seal(&mut first, None);
        let mut ambiguous = first.clone();
        ambiguous.actor_id = "a".into();
        ambiguous.action = "bc".into();
        assert_ne!(model_hash(&first, None), model_hash(&ambiguous, None));
        first.details = None;
        ambiguous = first.clone();
        ambiguous.details = Some(Value::Null);
        assert_ne!(model_hash(&first, None), model_hash(&ambiguous, None));
    }

    #[test]
    fn sequence_gaps_duplicates_missing_boundaries_and_forged_genesis_fail() {
        let first = entry(1, None, None);
        let skipped = entry(3, None, Some(&first));
        let duplicate = entry(1, None, Some(&first));
        for entries in [vec![first.clone(), skipped], vec![first.clone(), duplicate]] {
            assert!(!verify(&entries, None, 1, None, None).valid);
        }
        assert_eq!(
            verify(std::slice::from_ref(&first), None, 1, Some(2), None).first_broken_at,
            Some(2)
        );
        let missing_first = entry(2, None, None);
        assert_eq!(
            verify(&[missing_first], None, 1, None, None).first_broken_at,
            Some(1)
        );
        let mut forged = first;
        forged.prev_hash = "forged-root-anchor".into();
        forged.entry_hash = model_hash(&forged, None);
        assert!(!verify(&[forged], None, 1, None, None).valid);
    }

    #[test]
    fn last_signature_is_verified_and_cannot_be_removed() {
        let first = entry(1, None, None);
        let last = entry(2, None, Some(&first));
        for signature in [Some("not a signature".into()), None] {
            let mut corrupted = last.clone();
            corrupted.signature = signature;
            let result = verify(&[first.clone(), corrupted], None, 1, None, None);
            assert!(!result.valid);
            assert_eq!(result.first_broken_at, Some(2));
        }
    }

    #[test]
    fn nul_bytes_are_replaced_in_every_stored_string() {
        let mut input = AuditEntryInput {
            actor_id: "actor\0".into(),
            actor_type: AuditActorType::User,
            actor_ip: Some("192.0.2.1\0".into()),
            action: "app.create".into(),
            resource_type: "App".into(),
            resource_id: "app\0".into(),
            chain_id: Some("chain\0".into()),
            summary: "Created \0 an app".into(),
            details: Some(serde_json::json!({"ke\0y": ["va\0lue", {"nested": "\0"}], "n": 1})),
        };
        sanitize_input(&mut input);
        let stored = serde_json::to_string(&input).unwrap();
        assert!(!stored.contains("\\u0000"), "{stored}");
        assert_eq!(input.summary, "Created \u{fffd} an app");
        assert_eq!(input.details.unwrap()["ke\u{fffd}y"][0], "va\u{fffd}lue");
    }

    #[test]
    fn timestamps_never_precede_the_entry_they_link_to() {
        let future = Utc::now().fixed_offset() + chrono::Duration::hours(1);
        assert_eq!(entry_timestamp(Some(future)), future);
        let past = Utc::now().fixed_offset() - chrono::Duration::hours(1);
        let now = entry_timestamp(Some(past));
        assert!(now > past);
        assert_eq!(now.timestamp_subsec_nanos() % 1_000_000, 0);
    }

    #[test]
    fn batched_verification_matches_a_single_pass() {
        let first = entry(1, None, None);
        let second = entry(2, None, Some(&first));
        let third = entry(3, None, Some(&second));
        let mut verifier = ChainVerifier::new(None, 1, None, verify_test_signature);
        assert!(verifier.push(&first));
        assert!(verifier.push(&second));
        assert!(verifier.push(&third));
        let batched = verifier.finish(Some(3));
        assert!(batched.fully_authenticated && !batched.empty);
        assert_eq!(batched.entries_checked, 3);
        let mut broken = ChainVerifier::new(None, 1, None, verify_test_signature);
        assert!(broken.push(&first));
        assert!(!broken.push(&third));
        assert_eq!(broken.finish(None).first_broken_at, Some(2));
        assert!(verify(&[], None, 1, None, None).empty);
    }

    #[test]
    fn unsigned_entries_after_signed_history_break_the_chain() {
        let first = entry(1, None, None);
        let mut stripped = entry(2, None, Some(&first));
        stripped.kid = None;
        seal(&mut stripped, Some(&first));
        let result = verify(&[first.clone(), stripped.clone()], None, 1, None, None);
        assert!(!result.valid);
        assert_eq!(result.first_broken_at, Some(2));
        assert_eq!(
            verify(&[stripped], None, 2, None, Some(&first)).first_broken_at,
            Some(2)
        );
        let mut unsigned = entry(1, None, None);
        unsigned.kid = None;
        seal(&mut unsigned, None);
        let signed = entry(2, None, Some(&unsigned));
        let upgraded = verify(&[unsigned, signed], None, 1, None, None);
        assert!(upgraded.valid && !upgraded.fully_authenticated);
        assert_eq!(upgraded.unsigned_entries, 1);
    }

    #[test]
    fn unknown_key_fails_closed_without_claiming_a_broken_hash() {
        let mut unknown = entry(1, None, None);
        unknown.kid = Some("historical-key".into());
        seal(&mut unknown, None);
        let result = verify(&[unknown], None, 1, None, None);
        assert!(!result.valid && !result.fully_authenticated);
        assert_eq!(result.unverifiable_signatures, 1);
        assert_eq!(result.first_broken_at, None);
    }

    #[test]
    fn legacy_and_unsigned_chains_report_limited_assurance() {
        let mut legacy = entry(1, None, None);
        legacy.entry_hash.clear();
        seal(&mut legacy, None);
        let modern = entry(2, None, Some(&legacy));
        let result = verify(&[legacy, modern.clone()], None, 1, None, None);
        assert!(result.valid && !result.fully_authenticated);
        assert_eq!(result.legacy_entries, 1);
        let mut downgrade = entry(3, None, Some(&modern));
        downgrade.entry_hash.clear();
        seal(&mut downgrade, Some(&modern));
        assert!(!verify(&[downgrade], None, 3, None, Some(&modern)).valid);
        let mut unsigned = entry(1, None, None);
        unsigned.kid = None;
        seal(&mut unsigned, None);
        let result = verify(&[unsigned], None, 1, None, None);
        assert!(result.valid && !result.fully_authenticated);
        assert_eq!(result.unsigned_entries, 1);
        assert!(!verify(&[], None, 1, None, None).fully_authenticated);
        assert!(!verify(&[], None, 1, Some(1), None).valid);
    }
}
