//! Deleting a conversation clears one person's visible history without rewriting other inboxes.
//! The sequence boundary remains when new messages or an explicit start reopen a direct chat.

use super::{active_member, conversation_not_found, lock_conversation, members, now_millis, wake};
use crate::{
    entity::{conversation, conversation_member, inbox_entry, pinned_conversation},
    error::ApiError,
    middleware::jwt::AppUser,
    state::AppState,
};
use axum::{
    Extension,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, FixedOffset};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DbErr, EntityTrait, QueryFilter, QuerySelect,
    sea_query::{Expr, Func, SimpleExpr},
};

/// A correlated filter for an InboxEntry query. Keeping the rows preserves message-id claims.
pub(crate) fn visible_entries() -> SimpleExpr {
    Expr::cust(visible_entries_sql("\"InboxEntry\""))
}

fn above_boundary(entry: &str) -> String {
    format!(
        r#"{entry}."cleared" IS NOT true AND NOT EXISTS (SELECT 1 FROM "ConversationMember" cleared
        WHERE cleared."conversationId" = {entry}."conversationId"
          AND cleared."principalId" = {entry}."inboxId"
          AND (({entry}."seq" IS NOT NULL AND {entry}."seq" <= cleared."clearedSeq")
            OR ({entry}."seq" IS NULL AND {entry}."createdAt" <= cleared."clearedLegacyAt")))"#
    )
}

pub(super) fn visible_entries_sql(entry: &str) -> String {
    let visible = above_boundary(entry);
    let parent_visible = above_boundary("cleared_target");
    format!(
        r#"({visible}) AND NOT ({entry}."kind" IN ('EDIT', 'REACTION', 'DELETE')
        AND EXISTS (SELECT 1 FROM "InboxEntry" cleared_target
          WHERE cleared_target."inboxId" = {entry}."inboxId"
            AND cleared_target."conversationId" = {entry}."conversationId"
            AND cleared_target."messageId" = ({entry}."envelope" -> 'body' ->> 'target')
            AND NOT ({parent_visible})))"#
    )
}

pub(super) fn next_clear_marker(
    previous: Option<DateTime<FixedOffset>>,
    now: DateTime<FixedOffset>,
) -> DateTime<FixedOffset> {
    std::cmp::max(
        previous
            .and_then(|at| at.checked_add_signed(chrono::Duration::milliseconds(1)))
            .unwrap_or(now),
        now,
    )
}

pub(super) async fn reopen<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    principal: &str,
) -> Result<(), DbErr> {
    conversation_member::Entity::update_many()
        .col_expr(
            conversation_member::Column::HiddenAt,
            Expr::value(None::<DateTime<FixedOffset>>),
        )
        .filter(conversation_member::Column::ConversationId.eq(conversation_id))
        .filter(conversation_member::Column::PrincipalId.eq(principal))
        .filter(conversation_member::Column::HiddenAt.is_not_null())
        .filter(active_member())
        .exec(db)
        .await?;
    Ok(())
}

pub(super) async fn delete_in<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    principal: &str,
    now: DateTime<FixedOffset>,
) -> Result<Vec<String>, ApiError> {
    let member = conversation_member::Entity::find_by_id((
        conversation_id.to_string(),
        principal.to_string(),
    ))
    .one(db)
    .await?
    .ok_or_else(|| conversation_not_found(conversation_id))?;
    let conversation = conversation::Entity::find_by_id(conversation_id)
        .one(db)
        .await?
        .ok_or_else(|| conversation_not_found(conversation_id))?;
    // A retry after leaving a group succeeds without advancing the clear boundary.
    if member.hidden_at.is_some()
        && (member.left_at.is_some() || conversation.kind.eq_ignore_ascii_case("DIRECT"))
    {
        return Ok(vec![principal.to_string()]);
    }
    if member.left_at.is_some() || member.status != super::MEMBER_ACTIVE {
        return Err(conversation_not_found(conversation_id));
    }
    let people = if conversation.kind.eq_ignore_ascii_case("GROUP") {
        members::remove_in(db, conversation_id, principal, principal, now).await?
    } else {
        vec![principal.to_string()]
    };
    let last_seq = conversation::Entity::find_by_id(conversation_id)
        .one(db)
        .await?
        .and_then(|conversation| conversation.last_seq)
        .unwrap_or(0);
    let legacy: Option<DateTime<FixedOffset>> = inbox_entry::Entity::find()
        .select_only()
        .column_as(
            Expr::expr(Func::max(Expr::col(inbox_entry::Column::CreatedAt))),
            "newest",
        )
        .filter(inbox_entry::Column::InboxId.eq(principal))
        .filter(inbox_entry::Column::ConversationId.eq(conversation_id))
        .filter(inbox_entry::Column::Seq.is_null())
        .into_tuple::<Option<DateTime<FixedOffset>>>()
        .one(db)
        .await?
        .flatten();
    let cleared_at = next_clear_marker(member.cleared_at, now);
    let legacy_boundary = std::cmp::max(legacy.unwrap_or(now), now);
    conversation_member::Entity::update_many()
        .col_expr(conversation_member::Column::HiddenAt, Expr::value(now))
        .col_expr(
            conversation_member::Column::ClearedAt,
            Expr::value(cleared_at),
        )
        .col_expr(
            conversation_member::Column::ClearedLegacyAt,
            Expr::value(legacy_boundary),
        )
        .col_expr(
            conversation_member::Column::ClearedSeq,
            Expr::value(last_seq),
        )
        .filter(conversation_member::Column::ConversationId.eq(conversation_id))
        .filter(conversation_member::Column::PrincipalId.eq(principal))
        .exec(db)
        .await?;
    pinned_conversation::Entity::delete_by_id((principal.to_string(), conversation_id.to_string()))
        .exec(db)
        .await?;
    if conversation.kind.eq_ignore_ascii_case("DIRECT") {
        crate::bot_host::notes::clear_owner_notes_in(db, conversation_id, principal).await?;
    }
    Ok(people)
}

#[utoipa::path(delete, path = "/inbox/conversations/{conversation_id}", tag = "Inbox",
    params(("conversation_id" = String, Path, description = "The conversation to delete from your inbox")),
    responses((status = 204, description = "Your history is cleared. Groups are left with your bots; direct chats can reopen on a new message."),
        (status = 404, description = "You are not a member of this conversation")), security(("bearer_auth" = [])))]
pub async fn delete_conversation(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(conversation_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let sub = user.sub()?;
    let people = state
        .transaction(|txn| {
            let (id, sub) = (conversation_id.clone(), sub.clone());
            Box::pin(async move {
                lock_conversation(txn, &id).await?;
                delete_in(txn, &id, &sub, now_millis()).await
            })
        })
        .await?;
    wake(&state, people).await;
    Ok(StatusCode::NO_CONTENT)
}
