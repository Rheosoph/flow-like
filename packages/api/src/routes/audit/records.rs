use axum::{
    Extension, Json,
    extract::{Query, State},
};
use std::collections::HashSet;

use chrono::{DateTime, TimeDelta, Utc};
use flow_like_types::Value;
use sea_orm::{
    ActiveEnum, ColumnTrait, Condition, EntityTrait, Order, QueryFilter, QueryOrder, QuerySelect,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::{
    audit::verify::INVALID_SEAL_ID,
    entity::{audit_record, audit_seal},
    error::ApiError,
    middleware::jwt::AppUser,
    state::AppState,
};

use super::{chain_or_platform, ensure_chain_access};

const DEFAULT_LIMIT: u64 = 50;
const MAX_LIMIT: u64 = 500;
/// Span of the chain a filtered page examines.
const FILTER_WINDOW: TimeDelta = TimeDelta::days(30);

#[derive(Debug, Deserialize, IntoParams)]
pub struct AuditRecordParams {
    /// Chain id: an app or package id, `<id>#activity` for its verbose-only records, or
    /// `platform` (the default).
    pub chain_id: Option<String>,
    /// Exact action, or a prefix ending in `*` such as `board.*`.
    pub action: Option<String>,
    pub actor_id: Option<String>,
    pub resource_type: Option<String>,
    pub resource_id: Option<String>,
    /// Cursor from the previous page's `next`: only records older than this timestamp.
    pub before_ms: Option<i64>,
    /// Cursor from the previous page's `next`: breaks ties within `before_ms`.
    pub before_id: Option<String>,
    /// Page size, default 50, at most 500.
    pub limit: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AuditRecordStatus {
    /// Written, not yet sealed; protected by its MAC until the worker seals it.
    Pending,
    Sealed,
    /// Failed its MAC check before sealing and was quarantined.
    Invalid,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct AuditRecordView {
    pub id: String,
    pub chain_id: String,
    pub timestamp_ms: i64,
    pub actor_id: String,
    pub actor_type: String,
    pub action: String,
    pub resource_type: String,
    pub resource_id: String,
    #[schema(value_type = Option<Object>)]
    pub details: Option<Value>,
    pub actor_ip: Option<String>,
    pub status: AuditRecordStatus,
    pub seal_id: Option<String>,
    /// An IP was recorded and has expired; its commitment still verifies.
    pub ip_redacted: bool,
    /// Details were recorded and have expired; their commitment still verifies.
    pub details_redacted: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct AuditRecordCursor {
    pub before_ms: i64,
    pub before_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct AuditRecordPage {
    /// Newest first.
    pub records: Vec<AuditRecordView>,
    /// Pass as `before_ms` and `before_id` for the next page; absent on the last page.
    pub next: Option<AuditRecordCursor>,
}

impl From<audit_record::Model> for AuditRecordView {
    fn from(record: audit_record::Model) -> Self {
        let status = match record.seal_id.as_deref() {
            None => AuditRecordStatus::Pending,
            Some(INVALID_SEAL_ID) => AuditRecordStatus::Invalid,
            Some(_) => AuditRecordStatus::Sealed,
        };
        Self {
            ip_redacted: record.ip_commitment.is_some() && record.actor_ip.is_none(),
            details_redacted: record.details_commitment.is_some() && record.details.is_none(),
            actor_type: record.actor_type.to_value(),
            timestamp_ms: record.timestamp.timestamp_millis(),
            seal_id: record
                .seal_id
                .filter(|_| status == AuditRecordStatus::Sealed),
            status,
            id: record.id,
            chain_id: record.chain_id,
            actor_id: record.actor_id,
            action: record.action,
            resource_type: record.resource_type,
            resource_id: record.resource_id,
            details: record.details,
            actor_ip: record.actor_ip,
        }
    }
}

#[utoipa::path(
    get,
    path = "/audit/records",
    tag = "audit",
    description = "List the records of an audit chain, newest first, including records that are not sealed yet. Page with the returned cursor. App chains require Owner on that app; the platform chains and package chains require the Admin global permission.",
    params(AuditRecordParams),
    responses(
        (status = 200, description = "One page of audit records", body = AuditRecordPage),
        (status = 400, description = "Invalid cursor"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "GET /audit/records", skip_all)]
pub async fn list_records(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(params): Query<AuditRecordParams>,
) -> Result<Json<AuditRecordPage>, ApiError> {
    let chain_id = chain_or_platform(params.chain_id);
    ensure_chain_access(&user, &state, &chain_id).await?;
    let limit = params.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);

    let before = params
        .before_ms
        .map(|before_ms| {
            DateTime::from_timestamp_millis(before_ms).ok_or_else(|| {
                ApiError::bad_request(format!("before_ms {before_ms} is out of range"))
            })
        })
        .transpose()?;
    let filtered = params
        .action
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
        || [
            params.actor_id.as_deref(),
            params.resource_type.as_deref(),
            params.resource_id.as_deref(),
        ]
        .into_iter()
        .flatten()
        .any(|value| !value.is_empty());
    // The filters are not indexed, so a filtered page looks at one window of the chain;
    // an empty page with a cursor continues further back.
    let window_start = filtered.then(|| before.unwrap_or_else(Utc::now) - FILTER_WINDOW);

    let mut query =
        audit_record::Entity::find().filter(audit_record::Column::ChainId.eq(&chain_id));
    if let Some(window_start) = window_start {
        query = query.filter(audit_record::Column::Timestamp.gt(window_start.fixed_offset()));
    }
    if let Some(action) = params
        .action
        .as_deref()
        .map(str::trim)
        .filter(|action| !action.is_empty())
    {
        query = match action.strip_suffix('*') {
            Some(prefix) => query.filter(audit_record::Column::Action.starts_with(prefix)),
            None => query.filter(audit_record::Column::Action.eq(action)),
        };
    }
    for (column, value) in [
        (audit_record::Column::ActorId, params.actor_id.as_deref()),
        (
            audit_record::Column::ResourceType,
            params.resource_type.as_deref(),
        ),
        (
            audit_record::Column::ResourceId,
            params.resource_id.as_deref(),
        ),
    ] {
        if let Some(value) = value.filter(|value| !value.is_empty()) {
            query = query.filter(column.eq(value));
        }
    }
    match (before, params.before_id.as_deref()) {
        (Some(before), before_id) => {
            let before = before.fixed_offset();
            query = match before_id.filter(|before_id| !before_id.is_empty()) {
                Some(before_id) => query
                    .filter(audit_record::Column::Timestamp.lte(before))
                    .filter(
                        Condition::any()
                            .add(audit_record::Column::Timestamp.lt(before))
                            .add(audit_record::Column::Id.lt(before_id)),
                    ),
                None => query.filter(audit_record::Column::Timestamp.lt(before)),
            };
        }
        (None, Some(_)) => {
            return Err(ApiError::bad_request("before_id requires before_ms"));
        }
        (None, None) => {}
    }

    let mut records = query
        .order_by(audit_record::Column::Timestamp, Order::Desc)
        .order_by(audit_record::Column::Id, Order::Desc)
        .limit(limit + 1)
        .all(&state.db)
        .await?;
    let next = if records.len() as u64 > limit {
        records.truncate(limit as usize);
        records.last().map(|record| AuditRecordCursor {
            before_ms: record.timestamp.timestamp_millis(),
            before_id: record.id.clone(),
        })
    } else {
        match window_start {
            Some(window_start) => audit_record::Entity::find()
                .filter(audit_record::Column::ChainId.eq(&chain_id))
                .filter(audit_record::Column::Timestamp.lte(window_start.fixed_offset()))
                .one(&state.db)
                .await?
                .map(|_| AuditRecordCursor {
                    before_ms: window_start.timestamp_millis() + 1,
                    before_id: String::new(),
                }),
            None => None,
        }
    };
    let seals = existing_seals(&state, &chain_id, &records).await?;
    Ok(Json(AuditRecordPage {
        records: records
            .into_iter()
            .map(|record| {
                let mut view = AuditRecordView::from(record);
                if view.status == AuditRecordStatus::Sealed
                    && view
                        .seal_id
                        .as_deref()
                        .is_none_or(|seal_id| !seals.contains(seal_id))
                {
                    view.status = AuditRecordStatus::Invalid;
                    view.seal_id = None;
                }
                view
            })
            .collect(),
        next,
    }))
}

/// Seal ids of this chain that exist among the page's records. A record naming a seal
/// that is not in its chain was never sealed there and is shown as invalid.
async fn existing_seals(
    state: &AppState,
    chain_id: &str,
    records: &[audit_record::Model],
) -> Result<HashSet<String>, ApiError> {
    let ids: HashSet<&str> = records
        .iter()
        .filter_map(|record| record.seal_id.as_deref())
        .filter(|seal_id| *seal_id != INVALID_SEAL_ID)
        .collect();
    if ids.is_empty() {
        return Ok(HashSet::new());
    }
    Ok(audit_seal::Entity::find()
        .select_only()
        .column(audit_seal::Column::Id)
        .filter(audit_seal::Column::ChainId.eq(chain_id))
        .filter(audit_seal::Column::Id.is_in(ids))
        .into_tuple::<String>()
        .all(&state.db)
        .await?
        .into_iter()
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::sea_orm_active_enums::AuditActorType;

    fn record(seal_id: Option<&str>) -> audit_record::Model {
        audit_record::Model {
            id: "record-1".into(),
            chain_id: "app".into(),
            timestamp: DateTime::from_timestamp_millis(1_700_000_000_123)
                .unwrap()
                .fixed_offset(),
            actor_id: "user-1".into(),
            actor_type: AuditActorType::User,
            action: "board.update".into(),
            resource_type: "Board".into(),
            resource_id: "board-1".into(),
            ip_commitment: Some(vec![1; 32]),
            details_commitment: Some(vec![2; 32]),
            actor_ip: None,
            ip_salt: None,
            details: Some(serde_json::json!({ "count": 1 })),
            details_salt: Some(vec![3; 32]),
            mac: None,
            seal_id: seal_id.map(str::to_owned),
            entry_kid: None,
        }
    }

    #[test]
    fn status_and_redaction_follow_the_row() {
        let pending = AuditRecordView::from(record(None));
        assert_eq!(pending.status, AuditRecordStatus::Pending);
        assert!(pending.seal_id.is_none());
        assert!(pending.ip_redacted);
        assert!(!pending.details_redacted);
        assert_eq!(pending.timestamp_ms, 1_700_000_000_123);

        let sealed = AuditRecordView::from(record(Some("seal-1")));
        assert_eq!(sealed.status, AuditRecordStatus::Sealed);
        assert_eq!(sealed.seal_id.as_deref(), Some("seal-1"));

        let invalid = AuditRecordView::from(record(Some(INVALID_SEAL_ID)));
        assert_eq!(invalid.status, AuditRecordStatus::Invalid);
        assert!(invalid.seal_id.is_none());
    }
}
