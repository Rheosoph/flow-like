//! The HTTP transport's row store: one `Channel` row per pending request, inbound message or
//! cancel tombstone, scoped by `(channelId, sub)`. The API's own waiters (global chat) use
//! [`DbChannelStore`] directly; executors reach the same functions through `/channels` routes.

use chrono::Utc;
use flow_like_types::Value;
use flow_like_types::async_trait;
use flow_like_types::channel::{ChannelPoll, ChannelStore, MAX_TTL, now_unix};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, DbErr, EntityTrait,
    PaginatorTrait, QueryFilter, QueryOrder,
};

use crate::entity::{
    channel,
    prelude::Channel,
    sea_orm_active_enums::{ChannelMessageKind, ChannelMessageStatus},
};

/// Open request rows one subject may hold across all channels; the oldest is evicted beyond it.
pub const MAX_OPEN_REQUESTS_PER_SUB: u64 = 32;
/// Unconsumed inbound messages per channel; a client hammering the composer during a slow
/// round must not grow the next prompt without bound.
pub const MAX_PENDING_INBOUND_PER_CHANNEL: u64 = 8;

pub fn cancel_row_id(channel_id: &str) -> String {
    format!("cancel:{channel_id}")
}

/// A registration may not outlive [`MAX_TTL`] and must lie in the future.
pub fn clamp_expires_at(requested: i64, now: i64) -> i64 {
    requested.clamp(now + 1, now + MAX_TTL.as_secs() as i64)
}

pub fn parse_value(raw: Option<&str>) -> Value {
    raw.and_then(|text| serde_json::from_str(text).ok())
        .unwrap_or(Value::Null)
}

/// Whether a subject with `open` requests must first make room for one more.
pub fn needs_eviction(open: u64) -> bool {
    open >= MAX_OPEN_REQUESTS_PER_SUB
}

pub fn inbound_is_full(pending: u64) -> bool {
    pending >= MAX_PENDING_INBOUND_PER_CHANNEL
}

/// A request starts pending and is flipped by the reply; inbound messages and cancel tombstones
/// are complete the moment they are written.
fn initial_status(kind: &ChannelMessageKind) -> ChannelMessageStatus {
    match kind {
        ChannelMessageKind::Request => ChannelMessageStatus::Pending,
        ChannelMessageKind::Inbound | ChannelMessageKind::Cancel => ChannelMessageStatus::Responded,
    }
}

fn new_row(
    id: String,
    channel_id: &str,
    sub: &str,
    app_id: Option<&str>,
    kind: ChannelMessageKind,
    expires_at: i64,
    value: Option<String>,
) -> channel::ActiveModel {
    channel::ActiveModel {
        id: Set(id),
        channel_id: Set(channel_id.to_string()),
        sub: Set(sub.to_string()),
        app_id: Set(app_id.map(str::to_string)),
        status: Set(initial_status(&kind)),
        kind: Set(kind),
        expires_at: Set(expires_at),
        value: Set(value),
        created_at: Set(Utc::now().naive_utc()),
    }
}

async fn count_open_requests(db: &DatabaseConnection, sub: &str, now: i64) -> Result<u64, DbErr> {
    Channel::find()
        .filter(channel::Column::Sub.eq(sub))
        .filter(channel::Column::Kind.eq(ChannelMessageKind::Request))
        .filter(channel::Column::ExpiresAt.gt(now))
        .count(db)
        .await
}

/// Register a pending request row. Clamps the expiry, keeps the subject under
/// [`MAX_OPEN_REQUESTS_PER_SUB`] (expired rows first, then the oldest live one) and returns the
/// expiry actually stored.
pub async fn register_request(
    db: &DatabaseConnection,
    channel_id: &str,
    request_id: &str,
    sub: &str,
    app_id: Option<&str>,
    expires_at: i64,
) -> Result<i64, DbErr> {
    let now = now_unix();
    let expires_at = clamp_expires_at(expires_at, now);

    if needs_eviction(count_open_requests(db, sub, now).await?) {
        Channel::delete_many()
            .filter(channel::Column::Sub.eq(sub))
            .filter(channel::Column::ExpiresAt.lte(now))
            .exec(db)
            .await?;
        if needs_eviction(count_open_requests(db, sub, now).await?)
            && let Some(oldest) = Channel::find()
                .filter(channel::Column::Sub.eq(sub))
                .filter(channel::Column::Kind.eq(ChannelMessageKind::Request))
                .order_by_asc(channel::Column::CreatedAt)
                .one(db)
                .await?
        {
            Channel::delete_by_id(oldest.id).exec(db).await?;
        }
    }

    new_row(
        request_id.to_string(),
        channel_id,
        sub,
        app_id,
        ChannelMessageKind::Request,
        expires_at,
        None,
    )
    .insert(db)
    .await?;
    Ok(expires_at)
}

pub async fn poll_request(
    db: &DatabaseConnection,
    channel_id: &str,
    request_id: &str,
    sub: &str,
) -> Result<ChannelPoll, DbErr> {
    let Some(row) = Channel::find_by_id(request_id).one(db).await? else {
        return Ok(ChannelPoll::Missing);
    };
    if row.channel_id != channel_id || row.sub != sub || row.kind != ChannelMessageKind::Request {
        return Ok(ChannelPoll::Missing);
    }
    Ok(match row.status {
        ChannelMessageStatus::Responded => {
            ChannelPoll::Responded(parse_value(row.value.as_deref()))
        }
        ChannelMessageStatus::Pending => ChannelPoll::Pending,
    })
}

pub async fn remove_request(
    db: &DatabaseConnection,
    channel_id: &str,
    request_id: &str,
    sub: &str,
) -> Result<(), DbErr> {
    Channel::delete_many()
        .filter(channel::Column::Id.eq(request_id))
        .filter(channel::Column::ChannelId.eq(channel_id))
        .filter(channel::Column::Sub.eq(sub))
        .exec(db)
        .await?;
    Ok(())
}

/// Take the pending inbound messages, oldest first. Only the rows read are deleted, so a push
/// racing the drain keeps its row for the next round instead of being lost.
pub async fn drain_inbound(
    db: &DatabaseConnection,
    channel_id: &str,
    sub: &str,
) -> Result<Vec<Value>, DbErr> {
    let rows = Channel::find()
        .filter(channel::Column::ChannelId.eq(channel_id))
        .filter(channel::Column::Sub.eq(sub))
        .filter(channel::Column::Kind.eq(ChannelMessageKind::Inbound))
        .order_by_asc(channel::Column::CreatedAt)
        .all(db)
        .await?;
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<String> = rows.iter().map(|row| row.id.clone()).collect();
    let values = rows
        .into_iter()
        .map(|row| parse_value(row.value.as_deref()))
        .collect();
    Channel::delete_many()
        .filter(channel::Column::Id.is_in(ids))
        .exec(db)
        .await?;
    Ok(values)
}

pub async fn count_pending_inbound(
    db: &DatabaseConnection,
    channel_id: &str,
    sub: &str,
) -> Result<u64, DbErr> {
    Channel::find()
        .filter(channel::Column::ChannelId.eq(channel_id))
        .filter(channel::Column::Sub.eq(sub))
        .filter(channel::Column::Kind.eq(ChannelMessageKind::Inbound))
        .count(db)
        .await
}

pub async fn insert_inbound(
    db: &DatabaseConnection,
    channel_id: &str,
    sub: &str,
    app_id: Option<&str>,
    expires_at: i64,
    value: &Value,
) -> Result<(), DbErr> {
    new_row(
        flow_like_types::create_id(),
        channel_id,
        sub,
        app_id,
        ChannelMessageKind::Inbound,
        expires_at,
        Some(value.to_string()),
    )
    .insert(db)
    .await?;
    Ok(())
}

pub async fn is_cancelled(
    db: &DatabaseConnection,
    channel_id: &str,
    sub: &str,
) -> Result<bool, DbErr> {
    Ok(Channel::find_by_id(cancel_row_id(channel_id))
        .one(db)
        .await?
        .is_some_and(|row| row.sub == sub && row.kind == ChannelMessageKind::Cancel))
}

/// Write the cancel tombstone. Idempotent: an existing tombstone is the desired state.
pub async fn insert_cancel(
    db: &DatabaseConnection,
    channel_id: &str,
    sub: &str,
    app_id: Option<&str>,
    expires_at: i64,
) -> Result<(), DbErr> {
    if is_cancelled(db, channel_id, sub).await? {
        return Ok(());
    }
    let inserted = new_row(
        cancel_row_id(channel_id),
        channel_id,
        sub,
        app_id,
        ChannelMessageKind::Cancel,
        expires_at,
        None,
    )
    .insert(db)
    .await;
    match inserted {
        Ok(_) => Ok(()),
        Err(error) if is_cancelled(db, channel_id, sub).await? => {
            tracing::debug!(%error, channel_id, "cancel tombstone raced another cancel");
            Ok(())
        }
        Err(error) => Err(error),
    }
}

/// Delete every row of one channel owned by `sub`. Returns the number of rows removed.
pub async fn close_channel(
    db: &DatabaseConnection,
    channel_id: &str,
    sub: &str,
) -> Result<u64, DbErr> {
    let result = Channel::delete_many()
        .filter(channel::Column::ChannelId.eq(channel_id))
        .filter(channel::Column::Sub.eq(sub))
        .exec(db)
        .await?;
    Ok(result.rows_affected)
}

/// [`ChannelStore`] over the `Channel` table for waiters that live in the API process.
pub struct DbChannelStore {
    db: DatabaseConnection,
    sub: String,
    app_id: Option<String>,
}

impl DbChannelStore {
    pub fn new(db: DatabaseConnection, sub: impl Into<String>, app_id: Option<String>) -> Self {
        Self {
            db,
            sub: sub.into(),
            app_id,
        }
    }
}

fn db_error(operation: &str, error: DbErr) -> flow_like_types::Error {
    flow_like_types::anyhow!("channel store {operation} failed: {error}")
}

#[async_trait]
impl ChannelStore for DbChannelStore {
    async fn register(
        &self,
        channel_id: &str,
        request_id: &str,
        expires_at: i64,
    ) -> flow_like_types::Result<()> {
        register_request(
            &self.db,
            channel_id,
            request_id,
            &self.sub,
            self.app_id.as_deref(),
            expires_at,
        )
        .await
        .map(drop)
        .map_err(|e| db_error("register", e))
    }

    async fn poll(
        &self,
        channel_id: &str,
        request_id: &str,
    ) -> flow_like_types::Result<ChannelPoll> {
        poll_request(&self.db, channel_id, request_id, &self.sub)
            .await
            .map_err(|e| db_error("poll", e))
    }

    async fn remove(&self, channel_id: &str, request_id: &str) -> flow_like_types::Result<()> {
        remove_request(&self.db, channel_id, request_id, &self.sub)
            .await
            .map_err(|e| db_error("remove", e))
    }

    async fn drain_inbound(&self, channel_id: &str) -> flow_like_types::Result<Vec<Value>> {
        drain_inbound(&self.db, channel_id, &self.sub)
            .await
            .map_err(|e| db_error("drain_inbound", e))
    }

    async fn is_cancelled(&self, channel_id: &str) -> flow_like_types::Result<bool> {
        is_cancelled(&self.db, channel_id, &self.sub)
            .await
            .map_err(|e| db_error("is_cancelled", e))
    }

    async fn close(&self, channel_id: &str) -> flow_like_types::Result<()> {
        close_channel(&self.db, channel_id, &self.sub)
            .await
            .map(drop)
            .map_err(|e| db_error("close", e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expiry_is_clamped_to_the_channel_ttl_window() {
        let now = 1_000;
        let max = MAX_TTL.as_secs() as i64;
        assert_eq!(clamp_expires_at(now + 60, now), now + 60);
        assert_eq!(clamp_expires_at(now - 5, now), now + 1);
        assert_eq!(clamp_expires_at(now, now), now + 1);
        assert_eq!(clamp_expires_at(now + max + 1, now), now + max);
        assert_eq!(max, 9 * 60 * 60);
    }

    #[test]
    fn caps() {
        assert!(!needs_eviction(MAX_OPEN_REQUESTS_PER_SUB - 1));
        assert!(needs_eviction(MAX_OPEN_REQUESTS_PER_SUB));
        assert!(!inbound_is_full(MAX_PENDING_INBOUND_PER_CHANNEL - 1));
        assert!(inbound_is_full(MAX_PENDING_INBOUND_PER_CHANNEL));
        assert_eq!(cancel_row_id("run-1"), "cancel:run-1");
    }

    #[test]
    fn rows_start_in_the_status_their_kind_implies() {
        assert_eq!(
            initial_status(&ChannelMessageKind::Request),
            ChannelMessageStatus::Pending
        );
        assert_eq!(
            initial_status(&ChannelMessageKind::Inbound),
            ChannelMessageStatus::Responded
        );
        assert_eq!(
            initial_status(&ChannelMessageKind::Cancel),
            ChannelMessageStatus::Responded
        );
    }

    #[test]
    fn values_parse_leniently() {
        assert_eq!(parse_value(Some(r#"{"a":1}"#)), serde_json::json!({"a": 1}));
        assert_eq!(parse_value(Some("not json")), Value::Null);
        assert_eq!(parse_value(None), Value::Null);
    }
}
