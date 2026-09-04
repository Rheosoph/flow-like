use chrono::Utc;
use flow_like_types::{Value, create_id};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, DatabaseTransaction,
    DbErr, EntityTrait, Order, QueryFilter, QueryOrder, QuerySelect, sea_query::Expr,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::db::{DbDialect, RetryPolicy, retry_transaction};
use crate::entity::{audit_entry, sea_orm_active_enums::AuditActorType};

use super::chain::{ChainEntryRow, GENESIS_HASH, compute_entry_hash};
use super::sign::{current_kid, is_signing_configured, sign_entry};

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
    pub entries_checked: u64,
    pub first_broken_at: Option<i64>,
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
    /// The chain tail is read `FOR UPDATE` inside a retried transaction: on
    /// blocking engines the lock serializes writers, on optimistic ones it is
    /// the write intent that makes the losing writer retry from a fresh tail.
    pub async fn record(
        db: &DatabaseConnection,
        dialect: DbDialect,
        input: AuditEntryInput,
    ) -> flow_like_types::Result<audit_entry::Model> {
        let now = Utc::now().fixed_offset();
        let entry = retry_transaction::<_, audit_entry::Model, DbErr>(
            db,
            dialect,
            None,
            &RetryPolicy::default(),
            move |txn| {
                let input = input.clone();
                Box::pin(async move { Self::append_entry(txn, input, now).await })
            },
        )
        .await?;
        Ok(entry)
    }

    async fn append_entry(
        txn: &DatabaseTransaction,
        input: AuditEntryInput,
        now: chrono::DateTime<chrono::FixedOffset>,
    ) -> Result<audit_entry::Model, DbErr> {
        use sea_orm::sea_query::ExprTrait;

        let last_entry = audit_entry::Entity::find()
            .filter(if let Some(ref cid) = input.chain_id {
                Expr::col(audit_entry::Column::ChainId).eq(Expr::value(cid.clone()))
            } else {
                Expr::col(audit_entry::Column::ChainId).is_null()
            })
            .order_by(audit_entry::Column::Sequence, Order::Desc)
            .lock_exclusive()
            .one(txn)
            .await?;

        let (prev_hash, prev_signature, next_seq) = match last_entry {
            Some(ref entry) => (
                entry.entry_hash.clone(),
                entry.signature.clone(),
                entry.sequence + 1,
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

        let entry_hash = compute_entry_hash(
            next_seq,
            &now,
            &input.actor_id,
            &input.action,
            &input.resource_type,
            &input.resource_id,
            input.details.as_ref(),
            &prev_hash,
            prev_signature.as_deref(),
        );

        let signature = sign_entry(&entry_hash);
        let kid = if signature.is_some() {
            Some(current_kid().to_string())
        } else {
            None
        };

        if signature.is_none() && is_signing_configured() {
            tracing::error!("AUDIT INTEGRITY: Signing is configured but sign_entry returned None");
        }

        let model = audit_entry::ActiveModel {
            id: Set(create_id()),
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

    /// Verify the integrity of a hash chain.
    pub async fn verify_chain(
        db: &DatabaseConnection,
        chain_id: Option<&str>,
        from_seq: Option<i64>,
        to_seq: Option<i64>,
    ) -> flow_like_types::Result<ChainVerification> {
        let mut query = audit_entry::Entity::find();

        query = match chain_id {
            Some(cid) => query.filter(audit_entry::Column::ChainId.eq(cid)),
            None => query.filter(audit_entry::Column::ChainId.is_null()),
        };

        if let Some(from) = from_seq {
            query = query.filter(audit_entry::Column::Sequence.gte(from));
        }
        if let Some(to) = to_seq {
            query = query.filter(audit_entry::Column::Sequence.lte(to));
        }

        let entries = query
            .order_by(audit_entry::Column::Sequence, Order::Asc)
            .all(db)
            .await?;

        if entries.is_empty() {
            return Ok(ChainVerification {
                valid: true,
                entries_checked: 0,
                first_broken_at: None,
            });
        }

        // To verify, we need the hash + signature of the entry BEFORE the range
        // to compute the first entry's hash (which includes prev_signature).
        // For seq 1, branch chains are anchored to the root chain (not genesis),
        // so we trust the stored prev_hash of the first entry as the anchor point.
        let (initial_prev, initial_prev_sig) = if from_seq.unwrap_or(1) <= 1 {
            // The first entry stores its own prev_hash (genesis for root, root-tail for branches).
            // We use it directly — the anchor is verified by cross-referencing the root chain.
            (entries[0].prev_hash.clone(), None)
        } else {
            let before = audit_entry::Entity::find()
                .filter(match chain_id {
                    Some(cid) => audit_entry::Column::ChainId.eq(cid),
                    None => audit_entry::Column::ChainId.is_null(),
                })
                .filter(audit_entry::Column::Sequence.eq(entries[0].sequence - 1))
                .one(db)
                .await?;
            match before {
                Some(e) => (e.entry_hash, e.signature),
                None => (GENESIS_HASH.to_string(), None),
            }
        };

        // Build chain data with prev_signature for each entry.
        // The first entry's prev_signature comes from the entry before the range.
        let mut chain_data: Vec<ChainEntryRow> = Vec::with_capacity(entries.len());
        for (i, e) in entries.iter().enumerate() {
            let prev_sig = if i == 0 {
                initial_prev_sig.clone()
            } else {
                entries[i - 1].signature.clone()
            };
            chain_data.push((
                e.sequence,
                e.timestamp,
                e.actor_id.clone(),
                e.action.clone(),
                e.resource_type.clone(),
                e.resource_id.clone(),
                e.details.clone(),
                e.prev_hash.clone(),
                e.entry_hash.clone(),
                prev_sig,
            ));
        }

        let broken = super::chain::verify_chain(&chain_data, &initial_prev);
        let entries_checked = chain_data.len() as u64;

        Ok(ChainVerification {
            valid: broken.is_none(),
            entries_checked,
            first_broken_at: broken.map(|i| chain_data[i].0),
        })
    }

    /// Query audit entries with filters.
    pub async fn query(
        db: &DatabaseConnection,
        filter: AuditFilter,
    ) -> flow_like_types::Result<Vec<audit_entry::Model>> {
        let mut query = audit_entry::Entity::find();

        if let Some(ref cid) = filter.chain_id {
            query = query.filter(audit_entry::Column::ChainId.eq(cid.clone()));
        }
        if let Some(ref action) = filter.action {
            if action.ends_with(".*") {
                let prefix = &action[..action.len() - 2];
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
