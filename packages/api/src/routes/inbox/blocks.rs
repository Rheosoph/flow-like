//! Personal blocks stop direct messages and invitations in both directions. A user's bots
//! inherit their owner's blocks. Shared rooms keep their membership and message history.

use super::{MEMBER_BLOCKED, MEMBER_DECLINED, now_millis, stored, wake};
use crate::{
    db::coordination::transaction_lock_id,
    entity::{bot, conversation, conversation_member, inbox_block, mutation_lock, user},
    error::ApiError,
    middleware::jwt::AppUser,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, FixedOffset};
use flow_like_types::inbox::{ConversationKind, PrincipalKind};
use sea_orm::{
    ActiveValue::Set,
    ColumnTrait, Condition, ConnectionTrait, DbErr, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect, QueryTrait,
    sea_query::{Expr, OnConflict},
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use utoipa::ToSchema;

pub(crate) const INBOX_BLOCKED: &str = "INBOX_BLOCKED";

#[cfg(test)]
pub(super) const TEST_TABLES: [&str; 1] = [r#"CREATE TABLE public."InboxBlock" (
        "ownerId" TEXT NOT NULL, "principalId" TEXT NOT NULL,
        "principalKind" TEXT NOT NULL, "createdAt" TEXT NOT NULL,
        PRIMARY KEY ("ownerId", "principalId"))"#];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct InboxBlock {
    pub principal_id: String,
    /// `user` or `bot`.
    pub kind: String,
    pub created_at: DateTime<FixedOffset>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct InboxBlockList {
    pub blocks: Vec<InboxBlock>,
}

fn blocked() -> ApiError {
    ApiError::coded(
        StatusCode::FORBIDDEN,
        INBOX_BLOCKED,
        "Direct messages and invitations are unavailable between these accounts",
    )
}

async fn bot_owners<C: ConnectionTrait>(
    db: &C,
    ids: &[String],
) -> Result<HashMap<String, String>, DbErr> {
    Ok(bot::Entity::find()
        .select_only()
        .column(bot::Column::Id)
        .column(bot::Column::OwnerId)
        .filter(bot::Column::Id.is_in(ids.iter().cloned()))
        .into_tuple::<(String, String)>()
        .all(db)
        .await?
        .into_iter()
        .collect())
}

/// Account locks follow conversation locks and are always taken in account-id order. Block
/// mutations take no later conversation locks, so a send finishes first or sees the block.
async fn lock_accounts<C: ConnectionTrait>(db: &C, owners: &BTreeSet<String>) -> Result<(), DbErr> {
    for owner in owners {
        mutation_lock::Entity::insert(mutation_lock::ActiveModel {
            id: Set(transaction_lock_id("inbox-blocks", &[owner])),
            updated_at: Set(now_millis()),
            owner: Set(None),
            expires_at: Set(None),
        })
        .on_conflict(
            OnConflict::column(mutation_lock::Column::Id)
                .update_column(mutation_lock::Column::UpdatedAt)
                .to_owned(),
        )
        .exec_without_returning(db)
        .await?;
    }
    Ok(())
}

/// Older installations stored a block on the recipient's message-request membership. Read
/// those rows until the person unblocks, without requiring a large data migration.
async fn legacy_blocks<C: ConnectionTrait>(
    db: &C,
    owners: &BTreeSet<String>,
) -> Result<Vec<inbox_block::Model>, DbErr> {
    let owned_bots: HashMap<String, String> = bot::Entity::find()
        .select_only()
        .column(bot::Column::Id)
        .column(bot::Column::OwnerId)
        .filter(bot::Column::OwnerId.is_in(owners.iter().cloned()))
        .into_tuple::<(String, String)>()
        .all(db)
        .await?
        .into_iter()
        .collect();
    let principals: Vec<String> = owners.iter().chain(owned_bots.keys()).cloned().collect();
    Ok(conversation_member::Entity::find()
        .filter(conversation_member::Column::Status.eq(MEMBER_BLOCKED))
        .filter(conversation_member::Column::PrincipalId.is_in(principals))
        .all(db)
        .await?
        .into_iter()
        .filter_map(|row| {
            Some(inbox_block::Model {
                owner_id: owned_bots
                    .get(&row.principal_id)
                    .cloned()
                    .unwrap_or(row.principal_id),
                principal_id: row.invited_by?,
                principal_kind: stored(PrincipalKind::User.as_str()),
                created_at: row.joined_at,
            })
        })
        .collect())
}

pub(crate) async fn blocks_for<C: ConnectionTrait>(
    db: &C,
    owner: &str,
) -> Result<Vec<InboxBlock>, DbErr> {
    let rows = inbox_block::Entity::find()
        .filter(inbox_block::Column::OwnerId.eq(owner))
        .order_by_desc(inbox_block::Column::CreatedAt)
        .all(db)
        .await?;
    let combined: BTreeMap<String, InboxBlock> =
        legacy_blocks(db, &BTreeSet::from([owner.to_string()]))
            .await?
            .into_iter()
            .chain(rows)
            .map(|row| {
                (
                    row.principal_id.clone(),
                    InboxBlock {
                        principal_id: row.principal_id,
                        kind: row.principal_kind.to_ascii_lowercase(),
                        created_at: row.created_at,
                    },
                )
            })
            .collect();
    let mut blocks: Vec<InboxBlock> = combined.into_values().collect();
    blocks.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then(a.principal_id.cmp(&b.principal_id))
    });
    Ok(blocks)
}

/// Returns targets that cannot exchange direct messages or invitations with `principal`.
/// Mutation callers hold account locks until their transaction commits; read-only lists do not.
pub(crate) async fn blocked_between<C: ConnectionTrait>(
    db: &C,
    principal: &str,
    targets: &[String],
    lock: bool,
) -> Result<HashSet<String>, DbErr> {
    let pairs: Vec<_> = targets
        .iter()
        .map(|target| (principal.to_string(), target.clone()))
        .collect();
    Ok(blocked_pairs(db, &pairs, lock)
        .await?
        .into_iter()
        .map(|(_, target)| target)
        .collect())
}

pub(crate) async fn blocked_pairs<C: ConnectionTrait>(
    db: &C,
    candidates: &[(String, String)],
    lock: bool,
) -> Result<HashSet<(String, String)>, DbErr> {
    if candidates.is_empty() {
        return Ok(HashSet::new());
    }
    let ids: Vec<String> = candidates
        .iter()
        .flat_map(|(a, b)| [a.clone(), b.clone()])
        .collect();
    let bots = bot_owners(db, &ids).await?;
    let owner = |id: &str| bots.get(id).map_or_else(|| id.to_string(), Clone::clone);
    let candidates: Vec<_> = candidates
        .iter()
        .filter(|(principal, target)| owner(principal) != owner(target))
        .collect();
    if candidates.is_empty() {
        return Ok(HashSet::new());
    }
    let owners: BTreeSet<String> = candidates
        .iter()
        .flat_map(|(principal, target)| [owner(principal), owner(target)])
        .collect();
    if lock {
        lock_accounts(db, &owners).await?;
    }
    let rows = inbox_block::Entity::find()
        .filter(inbox_block::Column::OwnerId.is_in(owners.iter().cloned()))
        .all(db)
        .await?;
    let pairs: HashSet<(String, String)> = rows
        .into_iter()
        .chain(legacy_blocks(db, &owners).await?)
        .map(|row| (row.owner_id, row.principal_id))
        .collect();
    Ok(candidates
        .into_iter()
        .filter(|(principal, target)| {
            let principal_owner = owner(principal);
            let target_owner = owner(target);
            principal_owner != target_owner
                && (pairs.contains(&(principal_owner.clone(), target.clone()))
                    || pairs.contains(&(principal_owner.clone(), target_owner.clone()))
                    || pairs.contains(&(target_owner.clone(), principal.to_string()))
                    || pairs.contains(&(target_owner, principal_owner.clone())))
        })
        .cloned()
        .collect())
}

pub(crate) async fn ensure_allowed<C: ConnectionTrait>(
    db: &C,
    principal: &str,
    other: &str,
) -> Result<(), ApiError> {
    if blocked_between(db, principal, &[other.to_string()], true)
        .await?
        .is_empty()
    {
        Ok(())
    } else {
        Err(blocked())
    }
}

pub(crate) async fn ensure_direct_allowed<C: ConnectionTrait>(
    db: &C,
    conversation: &conversation::Model,
    sender: &str,
) -> Result<(), ApiError> {
    if ConversationKind::parse(&conversation.kind) != Some(ConversationKind::Direct) {
        // `message_person` opens a personal bot room before sending in a separate transaction.
        // Recheck at delivery so a block between those transactions still stops the bot.
        if let Some(person) = conversation
            .direct_key
            .as_deref()
            .and_then(|key| key.strip_prefix(&format!("outreach:{sender}:")))
        {
            return ensure_allowed(db, sender, person).await;
        }
        return Ok(());
    }
    let others = conversation_member::Entity::find()
        .select_only()
        .column(conversation_member::Column::PrincipalId)
        .filter(conversation_member::Column::ConversationId.eq(&conversation.id))
        .filter(conversation_member::Column::PrincipalId.ne(sender))
        .filter(conversation_member::Column::LeftAt.is_null())
        .into_tuple::<String>()
        .all(db)
        .await?;
    if blocked_between(db, sender, &others, true).await?.is_empty() {
        Ok(())
    } else {
        Err(blocked())
    }
}

pub(crate) async fn block_in<C: ConnectionTrait>(
    db: &C,
    owner: &str,
    principal: &str,
    now: DateTime<FixedOffset>,
) -> Result<(), ApiError> {
    if owner == principal {
        return Err(ApiError::bad_request("You can't block yourself"));
    }
    let target_bot = bot::Entity::find_by_id(principal).one(db).await?;
    let kind = if let Some(bot) = target_bot {
        if bot.owner_id == owner {
            return Err(ApiError::bad_request("You can't block your own bot"));
        }
        if bot.deleted_at.is_some() {
            return Err(ApiError::not_found("This bot is no longer available"));
        }
        PrincipalKind::Bot
    } else {
        if user::Entity::find_by_id(principal)
            .select_only()
            .column(user::Column::Id)
            .into_tuple::<String>()
            .one(db)
            .await?
            .is_none()
        {
            return Err(ApiError::not_found("This person or bot was not found"));
        }
        PrincipalKind::User
    };
    lock_accounts(db, &BTreeSet::from([owner.to_string()])).await?;
    inbox_block::Entity::insert(inbox_block::ActiveModel {
        owner_id: Set(owner.to_string()),
        principal_id: Set(principal.to_string()),
        principal_kind: Set(stored(kind.as_str())),
        created_at: Set(now),
    })
    .on_conflict(
        OnConflict::columns([
            inbox_block::Column::OwnerId,
            inbox_block::Column::PrincipalId,
        ])
        .do_nothing()
        .to_owned(),
    )
    .exec_without_returning(db)
    .await?;
    Ok(())
}

pub(crate) async fn unblock_in<C: ConnectionTrait>(
    db: &C,
    owner: &str,
    principal: &str,
) -> Result<(), ApiError> {
    lock_accounts(db, &BTreeSet::from([owner.to_string()])).await?;
    inbox_block::Entity::delete_by_id((owner.to_string(), principal.to_string()))
        .exec(db)
        .await?;
    let bots = bot::Entity::find()
        .select_only()
        .column(bot::Column::Id)
        .filter(bot::Column::OwnerId.eq(owner))
        .into_query();
    conversation_member::Entity::update_many()
        .col_expr(
            conversation_member::Column::Status,
            Expr::value(MEMBER_DECLINED),
        )
        .filter(conversation_member::Column::Status.eq(MEMBER_BLOCKED))
        .filter(conversation_member::Column::InvitedBy.eq(principal))
        .filter(
            Condition::any()
                .add(conversation_member::Column::PrincipalId.eq(owner))
                .add(conversation_member::Column::PrincipalId.in_subquery(bots)),
        )
        .exec(db)
        .await?;
    Ok(())
}

#[utoipa::path(get, path = "/inbox/blocks", tag = "Inbox",
    responses((status = 200, body = InboxBlockList, description = "People and bots you blocked")),
    security(("bearer_auth" = [])))]
pub async fn list_blocks(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<InboxBlockList>, ApiError> {
    let owner = user.sub()?;
    Ok(Json(InboxBlockList {
        blocks: blocks_for(&state.db, &owner).await?,
    }))
}

#[utoipa::path(put, path = "/inbox/blocks/{principal_id}", tag = "Inbox",
    params(("principal_id" = String, Path, description = "A person or another person's bot")),
    responses((status = 204, description = "Blocked. Existing history and shared rooms stay available; direct messages and invitations stop in both directions."),
        (status = 400, description = "You cannot block yourself or your own bot"),
        (status = 404, description = "The person or bot is unavailable")), security(("bearer_auth" = [])))]
pub async fn block_principal(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(principal): Path<String>,
) -> Result<StatusCode, ApiError> {
    let owner = user.sub()?;
    state
        .transaction(|txn| {
            let (owner, principal) = (owner.clone(), principal.clone());
            Box::pin(async move { block_in(txn, &owner, &principal, now_millis()).await })
        })
        .await?;
    wake(&state, vec![owner]).await;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(delete, path = "/inbox/blocks/{principal_id}", tag = "Inbox",
    params(("principal_id" = String, Path, description = "The blocked person or bot")),
    responses((status = 204, description = "Unblocked. Declined message requests still require acceptance.")), security(("bearer_auth" = [])))]
pub async fn unblock_principal(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(principal): Path<String>,
) -> Result<StatusCode, ApiError> {
    let owner = user.sub()?;
    state
        .transaction(|txn| {
            let (owner, principal) = (owner.clone(), principal.clone());
            Box::pin(async move { unblock_in(txn, &owner, &principal).await })
        })
        .await?;
    wake(&state, vec![owner]).await;
    Ok(StatusCode::NO_CONTENT)
}
