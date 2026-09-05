use chrono::Utc;
use flow_like_types::{Value, create_id};
use sea_orm::{
    ActiveEnum, ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection,
    DatabaseTransaction, DbErr, EntityTrait, IsolationLevel, Order, QueryFilter, QueryOrder,
    QuerySelect, TransactionTrait, sea_query::Expr,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::db::{DbDialect, RetryPolicy, retry_transaction};
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
}

#[derive(Clone, Debug, Deserialize, ToSchema)]
pub struct AuditFilter {
    pub chain_id: Option<String>,
    pub action: Option<String>,
    pub actor_id: Option<String>,
    pub resource_type: Option<String>,
    pub resource_id: Option<String>,
    pub limit: Option<u64>,
    pub offset: Option<u64>,
}

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
        input: AuditEntryInput,
        once: bool,
    ) -> flow_like_types::Result<audit_entry::Model> {
        let id = create_id();
        // Match timestamptz(3) before hashing: PostgreSQL rounds finer precision
        // on insert, which would otherwise change the signed timestamp.
        let now = chrono::DateTime::from_timestamp_millis(Utc::now().timestamp_millis())
            .expect("the current time fits in milliseconds")
            .fixed_offset();
        let entry = retry_transaction::<_, audit_entry::Model, DbErr>(
            db,
            dialect,
            None,
            &RetryPolicy::idempotent(),
            move |txn| {
                let input = input.clone();
                let id = id.clone();
                Box::pin(async move { Self::append_entry(txn, input, now, id, once).await })
            },
        )
        .await?;
        Ok(entry)
    }

    async fn append_entry(
        txn: &DatabaseTransaction,
        input: AuditEntryInput,
        now: chrono::DateTime<chrono::FixedOffset>,
        id: String,
        once: bool,
    ) -> Result<audit_entry::Model, DbErr> {
        use sea_orm::sea_query::ExprTrait;

        match input.chain_id.as_deref() {
            Some(chain_id) => {
                crate::db::coordination::coordinate(txn, "audit-branch", &[chain_id]).await?;
            }
            None => crate::db::coordination::coordinate(txn, "audit-root", &[]).await?,
        }

        // Reuse the committed result if a previous attempt lost its acknowledgement.
        if let Some(existing) = audit_entry::Entity::find_by_id(id.clone()).one(txn).await? {
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

        let last_entry = audit_entry::Entity::find()
            .filter(if let Some(ref cid) = input.chain_id {
                Expr::col(audit_entry::Column::ChainId).eq(Expr::value(cid.clone()))
            } else {
                Expr::col(audit_entry::Column::ChainId).is_null()
            })
            .order_by(audit_entry::Column::Sequence, Order::Desc)
            .one(txn)
            .await?;

        let (prev_hash, prev_signature, next_seq) = match last_entry {
            Some(ref entry) => (
                entry.entry_hash.clone(),
                entry.signature.clone(),
                entry
                    .sequence
                    .checked_add(1)
                    .ok_or_else(|| DbErr::Custom("audit sequence overflow".into()))?,
            ),
            None => {
                // First entry in this chain. For branch chains, anchor to the
                // current tail of the root chain so branches are cryptographically
                // linked to the global timeline. Root chain uses genesis.
                if input.chain_id.is_some() {
                    let root_tail = audit_entry::Entity::find()
                        .filter(Expr::col(audit_entry::Column::ChainId).is_null())
                        .order_by(audit_entry::Column::Sequence, Order::Desc)
                        .one(txn)
                        .await?;
                    match root_tail {
                        Some(entry) => (entry.entry_hash.clone(), entry.signature.clone(), 1),
                        None => (GENESIS_HASH.to_string(), None, 1),
                    }
                } else {
                    (GENESIS_HASH.to_string(), None, 1)
                }
            }
        };

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
        let mut query = audit_entry::Entity::find().filter(chain_filter(chain_id));
        if from_seq.is_some() {
            query = query.filter(audit_entry::Column::Sequence.gte(from));
        }
        if let Some(to) = to_seq {
            query = query.filter(audit_entry::Column::Sequence.lte(to));
        }
        let entries = query
            .order_by(audit_entry::Column::Sequence, Order::Asc)
            .all(&txn)
            .await?;
        let mut anchor_missing = false;
        let previous = if from > 1 {
            let previous = audit_entry::Entity::find()
                .filter(chain_filter(chain_id))
                .filter(audit_entry::Column::Sequence.eq(from - 1))
                .all(&txn)
                .await?;
            anchor_missing = previous.len() != 1;
            previous.into_iter().next()
        } else if chain_id.is_some() && entries.first().is_some_and(|e| e.prev_hash != GENESIS_HASH)
        {
            // A branch's initial hash and previous signature come from the same root entry.
            // Never trust an arbitrary prev_hash stored on the branch itself.
            let anchors = audit_entry::Entity::find()
                .filter(audit_entry::Column::ChainId.is_null())
                .filter(audit_entry::Column::EntryHash.eq(entries[0].prev_hash.clone()))
                .all(&txn)
                .await?;
            anchor_missing = anchors.len() != 1;
            anchors.into_iter().next()
        } else {
            None
        };
        let mut result = verify_entries(
            &entries,
            chain_id,
            from,
            to_seq,
            previous.as_ref(),
            verify_entry_signature_for_kid,
        );
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
        if entries.is_empty() && from_seq.is_some() {
            result.mark_broken(from);
        }
        txn.commit().await?;
        Ok(result)
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

        let limit = filter.limit.unwrap_or(50).min(200);
        let offset = filter.offset.unwrap_or(0);

        let entries = query
            .order_by(audit_entry::Column::Sequence, Order::Desc)
            .offset(offset)
            .limit(limit)
            .all(db)
            .await?;

        Ok(entries)
    }
}

fn chain_filter(chain_id: Option<&str>) -> sea_orm::sea_query::SimpleExpr {
    match chain_id {
        Some(cid) => audit_entry::Column::ChainId.eq(cid),
        None => audit_entry::Column::ChainId.is_null(),
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
    fn mark_broken(&mut self, sequence: i64) {
        self.valid = false;
        self.fully_authenticated = false;
        self.first_broken_at = Some(
            self.first_broken_at
                .map_or(sequence, |old| old.min(sequence)),
        );
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
    let mut result = ChainVerification {
        valid: true,
        entries_checked: 0,
        first_broken_at: None,
        fully_authenticated: false,
        signatures_verified: 0,
        unsigned_entries: 0,
        unverifiable_signatures: 0,
        legacy_entries: 0,
    };
    let mut expected_hash = previous.map_or(GENESIS_HASH, |entry| entry.entry_hash.as_str());
    let mut prev_signature = previous.and_then(|entry| entry.signature.as_deref());
    let mut expected_sequence = Some(from);
    let mut seen_v2 = previous.is_some_and(|entry| entry.entry_hash.starts_with(HASH_V2_PREFIX));
    for entry in entries {
        result.entries_checked += 1;
        let v2 = entry.entry_hash.starts_with(HASH_V2_PREFIX);
        if !v2 {
            result.legacy_entries += 1;
        }
        if expected_sequence != Some(entry.sequence)
            || entry.chain_id.as_deref() != chain_id
            || entry.prev_hash != expected_hash
            || (seen_v2 && !v2)
            || model_hash(entry, prev_signature) != entry.entry_hash
        {
            result.mark_broken(
                expected_sequence
                    .unwrap_or(entry.sequence)
                    .min(entry.sequence),
            );
            return result;
        }
        match (entry.signature.as_deref(), entry.kid.as_deref()) {
            (Some(signature), Some(kid)) => {
                match verify_signature(&entry.entry_hash, signature, kid) {
                    SignatureVerification::Valid => result.signatures_verified += 1,
                    SignatureVerification::Invalid => {
                        result.mark_broken(entry.sequence);
                        return result;
                    }
                    SignatureVerification::Unavailable => {
                        result.unverifiable_signatures += 1;
                        result.valid = false;
                    }
                }
            }
            (None, None) => result.unsigned_entries += 1,
            _ => {
                result.mark_broken(entry.sequence);
                return result;
            }
        }
        expected_hash = &entry.entry_hash;
        prev_signature = entry.signature.as_deref();
        expected_sequence = entry.sequence.checked_add(1);
        seen_v2 |= v2;
    }
    if let Some(to) = to {
        if entries.last().map(|entry| entry.sequence) != Some(to) {
            result.mark_broken(expected_sequence.unwrap_or(to));
        }
    }
    result.fully_authenticated = result.valid
        && !entries.is_empty()
        && result.signatures_verified == result.entries_checked
        && result.legacy_entries == 0;
    result
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
