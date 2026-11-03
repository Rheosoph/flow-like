//! A bot's workspace: a hidden app of its owner that holds the bot's files and databases. It is
//! made on first use, one per bot, and the owner is its only member. It never shows where apps
//! are listed, can't be shared, published, handed over or attached to a bot, and goes with its
//! bot. Its storage counts toward the owner's plan; it takes no project slot.

use super::{clone::source_name, lock_bot, model::BotSettings, owned_bot};
use crate::{
    audit::{AuditRecordInput, record_entry},
    audit_branch, capacity,
    credentials::{CredentialsAccess, RuntimeCredentials},
    deletion::{
        DeletionRoot, PassBudget, PassOutcome,
        job::{self, not_pending_deletion},
        run_pass,
    },
    ensure_permission,
    entity::{app, bot, deletion_job, sea_orm_active_enums::AuditActorType},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::{
        fork::begin_online::BundleSummary,
        internal::upsert_app::{NewApp, insert_app_rows, materialize_new_app},
    },
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, FixedOffset, Utc};
use flow_like::{
    bit::Metadata,
    flow::copilot::{BotRunContext, GENERAL_BOT_ID},
};
use flow_like_types::{bot::BotSystemKind, create_id};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseTransaction, DbErr, EntityTrait, QueryFilter,
    QuerySelect, sea_query::Query,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use utoipa::ToSchema;

pub(crate) const BOT_WORKSPACE_REFUSED: &str = "BOT_WORKSPACE_REFUSED";
pub(crate) const BOT_WORKSPACE_LOCKED: &str = "BOT_WORKSPACE_LOCKED";

/// The app that holds a bot's files and databases.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct BotWorkspace {
    pub app_id: String,
}

/// What a move is about to upload into the workspace, counted on the device.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct WorkspaceUploadRequest {
    pub summary: BundleSummary,
}

/// Where an upload into the workspace goes and the credentials it uses.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct WorkspaceUpload {
    /// The workspace's root in object storage, `apps/<app_id>`: files go below `upload/`,
    /// databases below `storage/db/`.
    pub upload_path: String,
    /// Credentials that read and write only the workspace's content, never its boards, events
    /// or pages.
    #[schema(value_type = Object)]
    pub shared_credentials: serde_json::Value,
    /// When the credentials stop working; finish the upload before then.
    pub expiration: Option<DateTime<Utc>>,
}

/// Refuses an action that would share, publish, hand over or attach a bot's workspace.
pub(crate) fn locked(app_id: &str) -> ApiError {
    ApiError::coded(
        StatusCode::CONFLICT,
        BOT_WORKSPACE_LOCKED,
        format!(
            "App '{app_id}' is a bot's workspace: it stays private to the bot and its owner, so it can't be shared, published, handed over or attached to a bot"
        ),
    )
}

/// The apps among `app_ids` that are bots' workspaces, in one query.
pub(crate) async fn workspaces_among<C: ConnectionTrait>(
    db: &C,
    app_ids: impl IntoIterator<Item = impl Into<String>>,
) -> Result<BTreeSet<String>, DbErr> {
    let app_ids: Vec<String> = app_ids.into_iter().map(Into::into).collect();
    if app_ids.is_empty() {
        return Ok(BTreeSet::new());
    }
    Ok(app::Entity::find()
        .select_only()
        .column(app::Column::Id)
        .filter(app::Column::Id.is_in(app_ids))
        .filter(app::Column::BotId.is_not_null())
        .into_tuple::<String>()
        .all(db)
        .await?
        .into_iter()
        .collect())
}

/// Refuses with `BOT_WORKSPACE_LOCKED` when any of `app_ids` is a bot's workspace.
pub(crate) async fn refuse_workspaces<C: ConnectionTrait>(
    db: &C,
    app_ids: impl IntoIterator<Item = impl Into<String>>,
) -> Result<(), ApiError> {
    match workspaces_among(db, app_ids).await?.into_iter().next() {
        Some(app_id) => Err(locked(&app_id)),
        None => Ok(()),
    }
}

fn refuse_flux(bot: &bot::Model) -> Result<(), ApiError> {
    if bot.system_key.as_deref() == Some(BotSystemKind::Flux.as_str()) {
        return Err(ApiError::coded(
            StatusCode::CONFLICT,
            BOT_WORKSPACE_REFUSED,
            "Flux can't have a workspace yet; its duplicates can",
        ));
    }
    Ok(())
}

/// The workspace of the caller's live bot, unless its deletion is running. Owner and bot are
/// checked in SQL, so another person's bot never answers.
pub(super) async fn live_workspace<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
) -> Result<Option<String>, DbErr> {
    app::Entity::find()
        .select_only()
        .column(app::Column::Id)
        .filter(app::Column::BotId.eq(bot_id))
        .filter(
            app::Column::BotId.in_subquery(
                Query::select()
                    .column(bot::Column::Id)
                    .from(bot::Entity)
                    .and_where(bot::Column::Id.eq(bot_id))
                    .and_where(bot::Column::OwnerId.eq(owner_id))
                    .and_where(bot::Column::DeletedAt.is_null())
                    .to_owned(),
            ),
        )
        .filter(not_pending_deletion(
            DeletionRoot::App,
            (app::Entity, app::Column::Id),
        ))
        .into_tuple()
        .one(db)
        .await
}

/// The bot's workspace app, whatever state it is in, and whether its deletion is running.
async fn workspace_row<C: ConnectionTrait>(
    db: &C,
    bot_id: &str,
) -> Result<Option<(String, bool)>, DbErr> {
    app::Entity::find()
        .select_only()
        .column(app::Column::Id)
        .expr(not_pending_deletion(
            DeletionRoot::App,
            (app::Entity, app::Column::Id),
        ))
        .filter(app::Column::BotId.eq(bot_id))
        .into_tuple::<(String, bool)>()
        .one(db)
        .await
        .map(|row| row.map(|(app_id, settled)| (app_id, !settled)))
}

/// The app a new workspace of `bot` becomes, named after the bot like on the desktop.
fn workspace_app(bot: &bot::Model) -> NewApp {
    let settings = BotSettings::from_row(bot);
    let name = source_name(bot, &settings);
    NewApp {
        id: create_id(),
        owner_id: bot.owner_id.clone(),
        metadata: Metadata {
            name: format!("{name}'s workspace"),
            description: format!("The files, databases and memory of bot {name}"),
            ..Metadata::default()
        },
        language: "en".to_string(),
        bits: Vec::new(),
        bot_id: Some(bot.id.clone()),
    }
}

/// What a bot's workspace claim found.
#[derive(Debug, Clone)]
pub(super) enum Claim {
    /// The bot's workspace.
    Existing(String),
    /// The bot has none; this is the app to make.
    New(NewApp),
}

/// The caller's bot's workspace or the app to make for it. Another person's or a deleted bot
/// reads as missing; Flux and a workspace whose deletion still runs are refused. Call under the
/// bot's lock: the answer holds until the transaction ends.
pub(super) async fn claim_in<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
) -> Result<Claim, ApiError> {
    let bot = owned_bot(db, owner_id, bot_id).await?;
    refuse_flux(&bot)?;
    match workspace_row(db, bot_id).await? {
        Some((_, true)) => Err(ApiError::conflict(format!(
            "The workspace of bot '{bot_id}' is being deleted; try again in a moment"
        ))),
        Some((app_id, false)) => Ok(Claim::Existing(app_id)),
        None => Ok(Claim::New(workspace_app(&bot))),
    }
}

/// [`claim_in`] under the bot's lock, writing a new workspace's rows with the owner's account
/// admitting it, so concurrent first uses end with one workspace.
pub(super) async fn create_in(
    txn: &DatabaseTransaction,
    owner_id: &str,
    bot_id: &str,
    now: DateTime<FixedOffset>,
) -> Result<Claim, ApiError> {
    lock_bot(txn, bot_id).await?;
    let claim = claim_in(txn, owner_id, bot_id).await?;
    if let Claim::New(new_app) = &claim {
        capacity::admit_bot_workspace(txn, owner_id, &new_app.id).await?;
        insert_app_rows(txn, new_app, now).await?;
    }
    Ok(claim)
}

/// A bot's workspace and whether this call made it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Ensured {
    pub app_id: String,
    pub created: bool,
}

/// The caller's bot's workspace, made on first use: refused like a new project when the
/// owner's storage is full, its manifest saved once its rows commit.
pub(crate) async fn ensure(
    state: &AppState,
    owner_id: &str,
    bot_id: &str,
) -> Result<Ensured, ApiError> {
    if let Some(app_id) = live_workspace(&state.db, owner_id, bot_id).await? {
        return Ok(Ensured {
            app_id,
            created: false,
        });
    }
    refuse_flux(&owned_bot(&state.db, owner_id, bot_id).await?)?;
    capacity::check_storage_write(state, "", owner_id, 0).await?;
    let now = Utc::now().fixed_offset();
    let claim = state
        .transaction(|txn| {
            let (owner_id, bot_id) = (owner_id.to_string(), bot_id.to_string());
            Box::pin(async move { create_in(txn, &owner_id, &bot_id, now).await })
        })
        .await?;
    match claim {
        Claim::Existing(app_id) => Ok(Ensured {
            app_id,
            created: false,
        }),
        Claim::New(new_app) => {
            materialize_new_app(state, &new_app).await?;
            Ok(Ensured {
                app_id: new_app.id,
                created: true,
            })
        }
    }
}

/// Every bot with a row works in a workspace, except Flux; Assistant without a row has none.
pub(super) fn keeps_workspace(bot: &BotRunContext) -> bool {
    bot.system != Some(BotSystemKind::Flux) && bot.bot_id != GENERAL_BOT_ID
}

/// The workspace a chat turn of `bot` works in, made on first use. A workspace that can't be had
/// right now (storage full, usage still being counted, deletion running) leaves the turn without
/// one instead of failing it.
pub(crate) async fn chat_workspace(
    state: &AppState,
    owner_id: &str,
    bot: &BotRunContext,
) -> Option<String> {
    if !keeps_workspace(bot) {
        return None;
    }
    match ensure(state, owner_id, &bot.bot_id).await {
        Ok(workspace) => {
            if workspace.created {
                record_entry(
                    state,
                    AuditRecordInput {
                        actor_id: format!("bot:{owner_id}:{}", bot.bot_id),
                        actor_type: AuditActorType::User,
                        actor_ip: crate::audit::request::actor_ip(),
                        action: "bot.workspace.create".to_string(),
                        resource_type: "Bot".to_string(),
                        resource_id: bot.bot_id.clone(),
                        scope: Some(workspace.app_id.clone()),
                        details: Some(serde_json::json!({ "via": "chat" })),
                    },
                )
                .await;
            }
            Some(workspace.app_id)
        }
        Err(error) => {
            tracing::warn!(bot_id = %bot.bot_id, %error, "Bot runs without its workspace");
            None
        }
    }
}

/// Queues the deletion of the bot's workspace in the transaction that deletes the bot, so both
/// commit or neither does. The caller holds the bot's lock, so no workspace appears meanwhile.
pub(super) async fn queue_deletion(
    txn: &DatabaseTransaction,
    owner_id: &str,
    bot_id: &str,
) -> Result<Option<deletion_job::Model>, ApiError> {
    let app_id: Option<String> = app::Entity::find()
        .select_only()
        .column(app::Column::Id)
        .filter(app::Column::BotId.eq(bot_id))
        .into_tuple()
        .one(txn)
        .await?;
    match app_id {
        Some(app_id) => Ok(Some(
            job::enqueue_in(txn, DeletionRoot::App, &app_id, Some(owner_id)).await?,
        )),
        None => Ok(None),
    }
}

/// A request-sized first pass of a queued workspace deletion, like other deletes run inline; a
/// large workspace or a failed pass is finished by the deletion worker.
pub(super) async fn delete_queued(state: &AppState, queued: &deletion_job::Model) {
    if queued.status == job::STATUS_DONE {
        return;
    }
    let claimed = match job::claim(state, &queued.id).await {
        Ok(Some(claimed)) => claimed,
        Ok(None) => return,
        Err(error) => {
            tracing::warn!(job_id = %queued.id, %error, "Claiming a bot workspace deletion failed; the deletion worker takes it");
            return;
        }
    };
    match run_pass(state, &claimed, PassBudget::inline()).await {
        Ok(PassOutcome::Completed) => {}
        Ok(outcome) => {
            tracing::info!(job_id = %claimed.id, ?outcome, "The deletion worker finishes the bot workspace deletion");
        }
        Err(error) => {
            tracing::warn!(job_id = %claimed.id, %error, "Deleting a bot workspace failed; the deletion worker retries it");
        }
    }
}

/// The bytes a workspace upload declares, unless it is larger or has more files than this hub
/// copies in one fork.
pub(super) fn upload_bytes(
    summary: &BundleSummary,
    max_size_bytes: u64,
    max_file_count: u64,
) -> Result<i64, ApiError> {
    if summary.total_size_bytes > max_size_bytes {
        return Err(ApiError::bad_request(format!(
            "The upload is {} bytes; this hub takes at most {max_size_bytes} bytes at once",
            summary.total_size_bytes
        )));
    }
    if summary.total_object_count > max_file_count {
        return Err(ApiError::bad_request(format!(
            "The upload has {} files; this hub takes at most {max_file_count} files at once",
            summary.total_object_count
        )));
    }
    i64::try_from(summary.total_size_bytes)
        .map_err(|_| ApiError::bad_request("The upload is too large"))
}

pub(super) fn upload_path(app_id: &str) -> String {
    format!("apps/{app_id}")
}

#[utoipa::path(
    post,
    path = "/bots/{bot_id}/workspace",
    tag = "Bots",
    params(("bot_id" = String, Path, description = "Your bot's id")),
    responses(
        (status = 201, description = "Your bot's new workspace: an app of yours that holds the bot's files and databases. It never shows in your apps, can't be shared, published or handed over, and is deleted with the bot. Its storage counts toward your plan; it takes no project.", body = BotWorkspace),
        (status = 200, description = "The workspace the bot already has", body = BotWorkspace),
        (status = 401, description = "Not signed in"),
        (status = 402, description = "PLAN_LIMIT_EXCEEDED: your storage is full, so no workspace can be set up"),
        (status = 403, description = "Only people manage bots, not API keys or running flows"),
        (status = 404, description = "You have no bot with this id"),
        (status = 409, description = "BOT_WORKSPACE_REFUSED: Flux has no workspace yet (its duplicates do); CONFLICT: the bot's workspace is still being deleted, try again shortly"),
        (status = 503, description = "USAGE_REFRESH_PENDING: your storage totals are being counted; try again shortly")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "POST /bots/{bot_id}/workspace", skip(state, user))]
pub async fn ensure_bot_workspace(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
) -> Result<(StatusCode, Json<BotWorkspace>), ApiError> {
    let sub = user.sub()?;
    let workspace = ensure(&state, &sub, &bot_id).await?;
    let status = if workspace.created {
        audit_branch!(
            state,
            user,
            workspace.app_id,
            "bot.workspace.create",
            "Bot",
            bot_id
        );
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((
        status,
        Json(BotWorkspace {
            app_id: workspace.app_id,
        }),
    ))
}

#[utoipa::path(
    post,
    path = "/bots/{bot_id}/workspace/upload",
    tag = "Bots",
    params(("bot_id" = String, Path, description = "Your bot's id")),
    request_body = WorkspaceUploadRequest,
    responses(
        (status = 200, description = "Where to upload the bot's files and databases, with credentials that reach only its workspace's content. A bot without a workspace gets one first.", body = WorkspaceUpload),
        (status = 400, description = "The upload is larger or has more files than this hub takes at once"),
        (status = 401, description = "Not signed in"),
        (status = 402, description = "PLAN_LIMIT_EXCEEDED: the upload doesn't fit in your storage"),
        (status = 403, description = "Only people manage bots, not API keys or running flows"),
        (status = 404, description = "You have no bot with this id"),
        (status = 409, description = "BOT_WORKSPACE_REFUSED: Flux has no workspace yet; CONFLICT: the bot's workspace is still being deleted, try again shortly"),
        (status = 503, description = "USAGE_REFRESH_PENDING: your storage totals are being counted; try again shortly")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "POST /bots/{bot_id}/workspace/upload", skip(state, user, body))]
pub async fn upload_to_bot_workspace(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
    Json(body): Json<WorkspaceUploadRequest>,
) -> Result<Json<WorkspaceUpload>, ApiError> {
    let sub = user.sub()?;
    let forking = &state.platform_config.forking;
    let bytes = upload_bytes(
        &body.summary,
        forking.max_size_bytes,
        forking.max_file_count,
    )?;
    let workspace = ensure(&state, &sub, &bot_id).await?;
    let app_id = workspace.app_id;
    if workspace.created {
        audit_branch!(state, user, app_id, "bot.workspace.create", "Bot", bot_id);
    }
    ensure_permission!(user, &app_id, &state, RolePermissions::WriteFiles);
    capacity::check_storage_write(&state, &app_id, &sub, bytes).await?;
    let credentials =
        RuntimeCredentials::scoped(&sub, &app_id, &state, CredentialsAccess::EditAppContent)
            .await
            .map_err(|error| {
                tracing::error!(%app_id, %error, "Scoping workspace upload credentials failed");
                ApiError::internal("Failed to create the workspace upload credentials")
            })?;
    let shared_credentials =
        serde_json::to_value(credentials.into_shared_credentials()).map_err(|error| {
            tracing::error!(%app_id, %error, "Serializing workspace upload credentials failed");
            ApiError::internal("Failed to create the workspace upload credentials")
        })?;
    audit_branch!(
        state,
        user,
        app_id,
        "bot.workspace.upload",
        "Bot",
        bot_id,
        serde_json::json!({
            "total_size_bytes": body.summary.total_size_bytes,
            "total_object_count": body.summary.total_object_count,
        })
    );
    Ok(Json(WorkspaceUpload {
        upload_path: upload_path(&app_id),
        shared_credentials,
        expiration: credentials.expiration(),
    }))
}
