//! Per-turn Live Activity subscriptions. Tokens stay encrypted on the owner's push target.

use super::owned_bot;
use crate::{
    bot_host::ledger,
    entity::{
        bot, bot_turn, push_notification_target,
        sea_orm_active_enums::{PushNotificationTargetPlatform, PushNotificationTargetProvider},
    },
    error::ApiError,
    middleware::jwt::AppUser,
    push_notifications::{metadata_unchanged, send_live_activity},
    routes::app::events::db::decrypt_token,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use sea_orm::{
    ColumnTrait, Condition, ConnectionTrait, EntityTrait, Order, QueryFilter, QueryOrder,
    QuerySelect, QueryTrait, sea_query::Expr,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub(crate) const METADATA_KEY: &str = "_bot_live_activities";
const MAX_ACTIVITIES: usize = 8;
const LIFETIME_SECS: i64 = 8 * 60 * 60;

#[derive(Clone, Serialize, Deserialize)]
struct Subscription {
    activity_id: String,
    bot_id: String,
    turn_id: String,
    token_encrypted: String,
    expires_at: i64,
    last_status: Option<String>,
    #[serde(default)]
    last_sent_at: i64,
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct RegisterLiveActivity {
    pub activity_id: String,
    pub turn_id: String,
    pub push_target_id: String,
    pub push_token: String,
    pub native_scope: String,
}

#[derive(Deserialize)]
pub struct RemoveLiveActivity {
    pub push_target_id: String,
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b':'))
}

fn checked(request: &RegisterLiveActivity) -> Result<(), ApiError> {
    if !valid_id(&request.activity_id)
        || !valid_id(&request.turn_id)
        || !valid_id(&request.push_target_id)
    {
        return Err(ApiError::bad_request(
            "Activity, turn and push target ids must contain at most 128 letters, digits, hyphens, underscores or colons",
        ));
    }
    if request.native_scope.is_empty() || request.native_scope.len() > 1024 {
        return Err(ApiError::bad_request(
            "native_scope must match this device's active profile",
        ));
    }
    if !(32..=512).contains(&request.push_token.len())
        || request.push_token.len() % 2 != 0
        || !request.push_token.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(ApiError::bad_request(
            "push_token must be the hexadecimal ActivityKit update token",
        ));
    }
    Ok(())
}

fn subscriptions(metadata: Option<&Value>) -> Vec<Subscription> {
    metadata
        .and_then(|m| m.get(METADATA_KEY))
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default()
}

fn metadata_with(previous: Option<&Value>, subscriptions: &[Subscription]) -> Value {
    let mut metadata = previous
        .cloned()
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    if subscriptions.is_empty() {
        metadata.as_object_mut().unwrap().remove(METADATA_KEY);
    } else {
        metadata[METADATA_KEY] = json!(subscriptions);
    }
    metadata
}

async fn target<C: ConnectionTrait>(
    db: &C,
    owner: &str,
    id: &str,
) -> Result<push_notification_target::Model, ApiError> {
    push_notification_target::Entity::find_by_id(id)
        .filter(push_notification_target::Column::UserId.eq(owner))
        .filter(push_notification_target::Column::Platform.eq(PushNotificationTargetPlatform::Ios))
        .filter(push_notification_target::Column::Provider.eq(PushNotificationTargetProvider::Fcm))
        .filter(push_notification_target::Column::PushEnabled.eq(true))
        .filter(push_notification_target::Column::InvalidatedAt.is_null())
        .one(db)
        .await?
        .ok_or_else(|| ApiError::not_found("An enabled iOS FCM push target is required"))
}

async fn save(
    state: &AppState,
    target: &push_notification_target::Model,
    rows: &[Subscription],
) -> Result<bool, ApiError> {
    Ok(push_notification_target::Entity::update_many()
        .col_expr(
            push_notification_target::Column::Metadata,
            Expr::value(Some(metadata_with(target.metadata.as_ref(), rows))),
        )
        .filter(push_notification_target::Column::Id.eq(&target.id))
        .filter(push_notification_target::Column::UserId.eq(&target.user_id))
        .filter(push_notification_target::Column::TokenEncrypted.eq(&target.token_encrypted))
        .filter(push_notification_target::Column::PushEnabled.eq(true))
        .filter(metadata_unchanged(&target.metadata))
        .exec(&state.db)
        .await?
        .rows_affected
        == 1)
}

#[utoipa::path(post, path = "/bots/{bot_id}/live-activities", tag = "Bots",
    params(("bot_id" = String, Path)), request_body = RegisterLiveActivity,
    responses((status = 200, description = "Registered"), (status = 400, description = "Invalid token or provider"), (status = 404, description = "Bot, turn or push target not owned by caller")), security(("bearer_auth" = [])))]
#[tracing::instrument(skip_all)]
pub async fn register(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
    Json(body): Json<RegisterLiveActivity>,
) -> Result<Json<Value>, ApiError> {
    checked(&body)?;
    let owner = user.sub()?;
    let config = &state.platform_config.push_notifications;
    if !config.enabled || !config.allow_mobile || config.fcm.is_none() {
        return Err(ApiError::bad_request(
            "Live Activities require mobile push with FCM configured",
        ));
    }
    owned_bot(&state.db, &owner, &bot_id).await?;
    let turn = ledger::find(&state.db, &body.turn_id, Some(&owner))
        .await?
        .filter(|turn| turn.bot_id == bot_id)
        .ok_or_else(|| ApiError::not_found("Turn not found"))?;
    let now = chrono::Utc::now().timestamp();
    for _ in 0..5 {
        let target = target(&state.db, &owner, &body.push_target_id).await?;
        if target
            .metadata
            .as_ref()
            .and_then(|value| value.get("native_scope"))
            .and_then(Value::as_str)
            != Some(body.native_scope.as_str())
        {
            return Err(ApiError::conflict(
                "This activity belongs to a different native profile",
            ));
        }
        let mut rows = subscriptions(target.metadata.as_ref());
        rows.retain(|row| row.expires_at > now);
        if rows.iter().any(|row| {
            row.activity_id == body.activity_id && (row.bot_id != bot_id || row.turn_id != turn.id)
        }) {
            return Err(ApiError::bad_request(
                "The activity already follows another turn",
            ));
        }
        let expires_at = rows
            .iter()
            .find(|row| row.activity_id == body.activity_id)
            .map_or(now + LIFETIME_SECS, |row| row.expires_at);
        rows.retain(|row| row.activity_id != body.activity_id);
        if rows.len() >= MAX_ACTIVITIES {
            return Err(ApiError::bad_request(
                "This device already follows eight bot turns",
            ));
        }
        rows.push(Subscription {
            activity_id: body.activity_id.clone(),
            bot_id: bot_id.clone(),
            turn_id: turn.id.clone(),
            token_encrypted: crate::utils::crypto::encrypt_secret(
                &body.push_token,
                &state.encryption_key,
            ),
            expires_at,
            last_status: None,
            last_sent_at: 0,
        });
        if save(&state, &target, &rows).await? {
            refresh_owner(&state, &owner).await;
            return Ok(Json(
                json!({"registered": true, "activity_id": body.activity_id}),
            ));
        }
    }
    Err(ApiError::conflict(
        "Push target changed; retry registering the activity",
    ))
}

#[utoipa::path(delete, path = "/bots/{bot_id}/live-activities/{activity_id}", tag = "Bots",
    params(("bot_id" = String, Path), ("activity_id" = String, Path), ("push_target_id" = String, Query)),
    responses((status = 200, description = "Removed"), (status = 404, description = "Push target not owned by caller")), security(("bearer_auth" = [])))]
#[tracing::instrument(skip_all)]
pub async fn unregister(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((bot_id, activity_id)): Path<(String, String)>,
    Query(query): Query<RemoveLiveActivity>,
) -> Result<Json<Value>, ApiError> {
    let owner = user.sub()?;
    for _ in 0..5 {
        let target = target(&state.db, &owner, &query.push_target_id).await?;
        let mut rows = subscriptions(target.metadata.as_ref());
        rows.retain(|row| row.activity_id != activity_id || row.bot_id != bot_id);
        if save(&state, &target, &rows).await? {
            return Ok(Json(json!({"registered": false})));
        }
    }
    Err(ApiError::conflict(
        "Push target changed; retry removing the activity",
    ))
}

fn status(turn: Option<&bot_turn::Model>, expired: bool) -> (&'static str, bool) {
    if expired {
        return ("expired", true);
    }
    match turn.map(|turn| turn.status.as_str()) {
        Some("QUEUED") => ("queued", false),
        Some("RUNNING") => ("running", false),
        Some("WAITING") => ("waiting", false),
        Some("DONE") => ("done", true),
        Some("FAILED") => ("failed", true),
        _ => ("cancelled", true),
    }
}

async fn refresh_target(
    state: &AppState,
    target: push_notification_target::Model,
) -> Result<(), ApiError> {
    let now = chrono::Utc::now().timestamp();
    let rows = subscriptions(target.metadata.as_ref());
    if rows.is_empty() {
        return Ok(());
    }
    let mut next = Vec::with_capacity(rows.len());
    for mut row in rows {
        let bot = bot::Entity::find_by_id(&row.bot_id)
            .filter(bot::Column::OwnerId.eq(&target.user_id))
            .filter(bot::Column::DeletedAt.is_null())
            .one(&state.db)
            .await?;
        let turn = if bot.is_some() {
            ledger::find(&state.db, &row.turn_id, Some(&target.user_id))
                .await?
                .filter(|turn| turn.bot_id == row.bot_id)
        } else {
            None
        };
        let (status, end) = status(turn.as_ref(), row.expires_at <= now);
        if row.last_status.as_deref() == Some(status) && row.last_sent_at + 300 > now {
            next.push(row);
            continue;
        }
        let Some(token) = decrypt_token(&row.token_encrypted, &state.encryption_key) else {
            continue;
        };
        let title = bot
            .as_ref()
            .map(crate::bot_notify::display_name)
            .unwrap_or_else(|| "Bot task".into());
        let content = json!({"title": title, "status": status, "updatedAt": now as f64});
        match send_live_activity(state, &target, &token, content, end, now).await {
            Ok(()) => {
                if !end {
                    row.last_status = Some(status.into());
                    row.last_sent_at = now;
                    next.push(row);
                }
            }
            Err(_) => {
                // Provider errors may contain tokens; log only the target id.
                tracing::warn!(target_id = %target.id, "Bot Live Activity update was not accepted");
                if row.expires_at + 3600 > now {
                    next.push(row);
                }
            }
        }
    }
    save(state, &target, &next).await?;
    Ok(())
}

async fn refresh(state: &AppState, owner: Option<&str>) -> Result<(), ApiError> {
    if !state.platform_config.push_notifications.enabled
        || !state.platform_config.push_notifications.allow_mobile
    {
        return Ok(());
    }
    let mut query = push_notification_target::Entity::find()
        .filter(push_notification_target::Column::Platform.eq(PushNotificationTargetPlatform::Ios))
        .filter(push_notification_target::Column::Provider.eq(PushNotificationTargetProvider::Fcm))
        .filter(push_notification_target::Column::PushEnabled.eq(true))
        .filter(push_notification_target::Column::InvalidatedAt.is_null())
        .filter(push_notification_target::Column::Metadata.is_not_null());
    if let Some(owner) = owner {
        query = query.filter(push_notification_target::Column::UserId.eq(owner));
    }
    let mut after: Option<String> = None;
    loop {
        let mut page = query
            .clone()
            .order_by_asc(push_notification_target::Column::Id)
            .limit(100);
        if let Some(id) = &after {
            page = page.filter(push_notification_target::Column::Id.gt(id));
        }
        let targets = page.all(&state.db).await?;
        if targets.is_empty() {
            break;
        }
        after = targets.last().map(|target| target.id.clone());
        for target in targets {
            if let Err(error) = refresh_target(state, target).await {
                tracing::warn!(%error, "Bot Live Activity target refresh failed");
            }
        }
    }
    Ok(())
}

pub(crate) async fn refresh_owner(state: &AppState, owner: &str) {
    if let Err(error) = refresh(state, Some(owner)).await {
        tracing::warn!(%error, "Bot Live Activity refresh failed");
    }
}

pub(crate) async fn sweep(state: &AppState) {
    if let Err(error) = refresh(state, None).await {
        tracing::warn!(%error, "Bot Live Activity sweep failed");
    }
}

#[utoipa::path(get, path = "/bots/mobile-status", tag = "Bots",
    responses((status = 200, description = "Up to fifty active or recently completed turns owned by you", body = super::turns::BotTurnList)), security(("bearer_auth" = [])))]
pub async fn mobile_status(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<super::turns::BotTurnList>, ApiError> {
    let owner = user.sub()?;
    let turns = mobile_turns(&state.db, &owner, chrono::Utc::now().fixed_offset()).await?;
    Ok(Json(super::turns::BotTurnList {
        turns: turns
            .iter()
            .map(|turn| ledger::turn_json(turn, Vec::new()).into())
            .collect(),
    }))
}

async fn mobile_turns<C: ConnectionTrait>(
    db: &C,
    owner: &str,
    now: chrono::DateTime<chrono::FixedOffset>,
) -> Result<Vec<bot_turn::Model>, ApiError> {
    let bots = bot::Entity::find()
        .select_only()
        .column(bot::Column::Id)
        .filter(bot::Column::OwnerId.eq(owner))
        .filter(bot::Column::DeletedAt.is_null())
        .into_query();
    bot_turn::Entity::find()
        .filter(bot_turn::Column::OwnerId.eq(owner))
        .filter(bot_turn::Column::BotId.in_subquery(bots))
        .filter(
            Condition::any()
                .add(bot_turn::Column::Status.is_in(["QUEUED", "RUNNING", "WAITING"]))
                .add(bot_turn::Column::UpdatedAt.gte(now - chrono::Duration::hours(24))),
        )
        // A completed turn must not displace active work from the bounded mobile snapshot.
        .order_by::<Expr>(
            Expr::case(
                bot_turn::Column::Status.is_in(["QUEUED", "RUNNING", "WAITING"]),
                0,
            )
            .finally(1)
            .into(),
            Order::Asc,
        )
        .order_by_desc(bot_turn::Column::UpdatedAt)
        .order_by_desc(bot_turn::Column::Id)
        .limit(50)
        .all(db)
        .await
        .map_err(ApiError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ActiveModelTrait, ActiveValue::Set};

    async fn insert_turn(
        db: &sea_orm::DatabaseConnection,
        id: &str,
        bot_id: &str,
        owner: &str,
        status: &str,
        time: chrono::DateTime<chrono::FixedOffset>,
    ) {
        bot_turn::ActiveModel {
            id: Set(id.into()),
            bot_id: Set(bot_id.into()),
            owner_id: Set(owner.into()),
            conversation_id: Set("room".into()),
            trigger_message_id: Set(id.into()),
            host_kind: Set("SERVER".into()),
            status: Set(status.into()),
            attempts: Set(1),
            host_id: Set(None),
            lease_until: Set(None),
            operation_id: Set(None),
            reply_message_id: Set(None),
            error_code: Set(None),
            hops: Set(0),
            requester_id: Set(None),
            audience: Set("OWNER".into()),
            status_text: Set(None),
            unprompted: Set(None),
            created_at: Set(time),
            updated_at: Set(time),
        }
        .insert(db)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn mobile_status_excludes_foreign_deleted_and_old_completed_turns() {
        let db = crate::routes::inbox::fixture::database().await;
        for (id, owner) in [
            ("mine", "owner"),
            ("foreign", "other"),
            ("deleted", "owner"),
        ] {
            crate::routes::inbox::fixture::insert_bot(&db, id, owner, "SERVER").await;
        }
        crate::routes::inbox::fixture::delete_bot(&db, "deleted").await;
        let now = chrono::Utc::now().fixed_offset();
        for (id, bot, owner, state, old) in [
            ("current", "mine", "owner", "RUNNING", false),
            ("old-active", "mine", "owner", "WAITING", true),
            ("old-done", "mine", "owner", "DONE", true),
            ("other-owner", "foreign", "other", "RUNNING", false),
            ("foreign-bot", "foreign", "owner", "RUNNING", false),
            ("deleted-bot", "deleted", "owner", "RUNNING", false),
        ] {
            let time = if old {
                now - chrono::Duration::days(2)
            } else {
                now
            };
            insert_turn(&db, id, bot, owner, state, time).await;
        }
        let rows = mobile_turns(&db, "owner", now).await.unwrap();
        assert_eq!(
            rows.into_iter().map(|row| row.id).collect::<Vec<_>>(),
            ["current", "old-active"]
        );
    }

    #[tokio::test]
    async fn mobile_status_keeps_active_turns_ahead_of_newer_completions() {
        let db = crate::routes::inbox::fixture::database().await;
        crate::routes::inbox::fixture::insert_bot(&db, "mine", "owner", "SERVER").await;
        let now = chrono::Utc::now().fixed_offset();
        let active = ["running", "waiting", "queued"];
        for (offset, id) in active.iter().enumerate() {
            insert_turn(
                &db,
                id,
                "mine",
                "owner",
                &id.to_ascii_uppercase(),
                now - chrono::Duration::days(2) - chrono::Duration::seconds(offset as i64),
            )
            .await;
        }
        for offset in 0..55 {
            insert_turn(
                &db,
                &format!("done-{offset}"),
                "mine",
                "owner",
                "DONE",
                now - chrono::Duration::seconds(offset),
            )
            .await;
        }

        let rows = mobile_turns(&db, "owner", now).await.unwrap();
        assert_eq!(rows.len(), 50);
        assert_eq!(
            rows[..3]
                .iter()
                .map(|row| row.id.as_str())
                .collect::<Vec<_>>(),
            active,
        );
        assert_eq!(rows[3].id, "done-0");
        assert_eq!(rows.last().unwrap().id, "done-46");
    }

    #[tokio::test]
    async fn target_access_is_owned_enabled_and_ios_only() {
        let db = sea_orm::Database::connect("sqlite::memory:").await.unwrap();
        db.execute_unprepared("ATTACH DATABASE ':memory:' AS public")
            .await
            .unwrap();
        db.execute_unprepared(r#"CREATE TABLE public."PushNotificationTarget" (
            id TEXT PRIMARY KEY, "userId" TEXT NOT NULL, "deviceId" TEXT NOT NULL,
            platform TEXT NOT NULL, provider TEXT NOT NULL, "tokenEncrypted" TEXT NOT NULL,
            "endpointArn" TEXT, "installationId" TEXT, "channelId" TEXT, "deviceName" TEXT,
            metadata TEXT, "pushEnabled" BOOLEAN NOT NULL, "failureCount" INTEGER NOT NULL,
            "lastRegisteredAt" TEXT NOT NULL, "lastSeenAt" TEXT NOT NULL,
            "invalidatedAt" TEXT, "invalidationReason" TEXT, "createdAt" TEXT NOT NULL, "updatedAt" TEXT NOT NULL
        )"#).await.unwrap();
        db.execute_unprepared(r#"INSERT INTO public."PushNotificationTarget" (
            id, "userId", "deviceId", platform, provider, "tokenEncrypted", "pushEnabled", "failureCount",
            "lastRegisteredAt", "lastSeenAt", "createdAt", "updatedAt") VALUES
            ('target', 'owner', 'phone', 'IOS', 'FCM', 'encrypted', true, 0,
            '2026-11-04T12:00:00+00:00', '2026-11-04T12:00:00+00:00', '2026-11-04T12:00:00+00:00', '2026-11-04T12:00:00+00:00')"#).await.unwrap();
        assert!(target(&db, "owner", "target").await.is_ok());
        assert_eq!(
            target(&db, "other", "target").await.unwrap_err().status(),
            axum::http::StatusCode::NOT_FOUND
        );
        db.execute_unprepared(r#"UPDATE public."PushNotificationTarget" SET platform = 'ANDROID'"#)
            .await
            .unwrap();
        assert!(target(&db, "owner", "target").await.is_err());
        db.execute_unprepared(
            r#"UPDATE public."PushNotificationTarget" SET platform = 'IOS', "pushEnabled" = false"#,
        )
        .await
        .unwrap();
        assert!(target(&db, "owner", "target").await.is_err());
    }

    #[test]
    fn rejects_unbounded_and_nonhex_tokens() {
        let mut body = RegisterLiveActivity {
            activity_id: "activity-1".into(),
            turn_id: "bt_1".into(),
            push_target_id: "target".into(),
            push_token: "ab".repeat(32),
            native_scope: "profile-a".into(),
        };
        assert!(checked(&body).is_ok());
        body.push_token = "secret/path".into();
        assert!(checked(&body).is_err());
        body.push_token = "ab".repeat(300);
        assert!(checked(&body).is_err());
    }
    #[test]
    fn expired_activities_end_even_when_the_turn_is_missing() {
        assert_eq!(status(None, true), ("expired", true));
        assert_eq!(status(None, false), ("cancelled", true));
    }
    #[test]
    fn metadata_keeps_other_provider_state_and_removes_finished_tokens() {
        let initial = json!({"source": "ios", METADATA_KEY: []});
        let saved = metadata_with(Some(&initial), &[]);
        assert_eq!(saved["source"], "ios");
        assert!(saved.get(METADATA_KEY).is_none());
    }
}
