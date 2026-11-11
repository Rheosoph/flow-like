//! The bot a global chat turn runs as: the caller's own live bot and the apps it may use, mapped
//! to the core bot runtime. Another person's bot, a deleted bot and an unknown id all read as
//! "not yours"; the `bot_` prefix is never trusted.

use super::{apps::attached_apps, model::BotSettings, provision};
use crate::{
    entity::{bot, profile},
    error::ApiError,
    routes::ai::global_chat::ensure_metered_or_customer_provider,
    state::AppState,
};
use flow_like::{
    flow::copilot::{
        BotRunContext, BotToolHost, BotToolset, FLUX_BOT_ID, FLUX_BOT_NAME,
        GENERAL_BOT_DESCRIPTION, GENERAL_BOT_ID, GENERAL_BOT_NAME, memory::MemoryScope,
        resolve_bot_model, runnable_completion_model,
    },
    flow_like_model_provider::provider::is_hosted_provider_name,
    profile::{Profile, ProfileApp},
    state::FlowLikeState,
};
use flow_like_types::{
    Value,
    bot::{BotCapability, BotModelPolicy, BotSystemKind},
};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QuerySelect};
use std::{collections::BTreeSet, sync::Arc};

/// The bot a chat turn answers as, and the model policy the host resolves. Bots that hold
/// `flowpilot` (Flux and its duplicates) run FlowPilot's toolset with their row's edits. Tools
/// run in the browser that holds the chat
/// (`BotToolHost::Chat`); a server worker without one has no native app, data or file tools yet
/// and sets `BotToolHost::CoreOnly`.
#[derive(Debug, Clone)]
pub(crate) struct ChatBot {
    pub(crate) context: BotRunContext,
    pub(crate) model: BotModelPolicy,
}

/// Bot ids are `bot_` plus a cuid; anything longer than this can't name one.
const MAX_BOT_ID_CHARS: usize = 256;

/// The trimmed `bot_id` of a request, `None` when absent or blank.
fn requested_bot_id(bot_id: Option<&str>) -> Result<Option<&str>, ApiError> {
    let bot_id = bot_id.map(str::trim).filter(|bot_id| !bot_id.is_empty());
    if bot_id.is_some_and(|bot_id| bot_id.chars().count() > MAX_BOT_ID_CHARS) {
        return Err(ApiError::bad_request(format!(
            "Bot ids are at most {MAX_BOT_ID_CHARS} characters."
        )));
    }
    Ok(bot_id)
}

/// The caller's bot that `bot_id` names: `None` without an id (today's FlowPilot run), Assistant
/// for `general` and Flux for `flux` (set up first when the caller has no rows yet, like
/// `GET /bots`), otherwise the caller's live bot, in its workspace (made on first use).
/// `request_profile` backs `profile_apps` for a bot that names no profile of its own.
pub(crate) async fn chat_bot(
    state: &AppState,
    owner_id: &str,
    bot_id: Option<&str>,
    request_profile: &Profile,
) -> Result<Option<ChatBot>, ApiError> {
    let Some(bot_id) = requested_bot_id(bot_id)? else {
        return Ok(None);
    };
    if bot_id == GENERAL_BOT_ID || bot_id == FLUX_BOT_ID {
        provision::bots_with_system(state, owner_id).await?;
    }
    let mut bot = load_chat_bot(&state.db, owner_id, bot_id, request_profile).await?;
    if let Some(bot) = bot.as_mut() {
        bot.context.workspace_app_id =
            super::workspace::chat_workspace(state, owner_id, &bot.context).await;
        bot.context.host_tools = crate::bot_tools::host_tools(state);
        if bot.context.has(BotCapability::Skills) {
            bot.context.skills = super::skills::active_index(&state.db, &bot.context.bot_id)
                .await
                .unwrap_or_else(|error| {
                    tracing::warn!(%error, "Bot skills: the active skills were not loaded");
                    Vec::new()
                });
        }
    }
    Ok(bot)
}

pub(super) async fn load_chat_bot<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
    request_profile: &Profile,
) -> Result<Option<ChatBot>, ApiError> {
    let row = match bot_id {
        GENERAL_BOT_ID => system_row(db, owner_id, BotSystemKind::General).await?,
        FLUX_BOT_ID => system_row(db, owner_id, BotSystemKind::Flux).await?,
        _ => Some(live_owned_bot(db, owner_id, bot_id).await?),
    };
    let Some(row) = row else {
        return Ok((bot_id == GENERAL_BOT_ID).then(|| ChatBot {
            context: BotRunContext::general(),
            model: BotModelPolicy::default(),
        }));
    };
    let system = row.system_key.as_deref().and_then(BotSystemKind::parse);
    let settings = BotSettings::from_row(&row);
    let selected_app_ids = if settings.has(BotCapability::SelectedApps) {
        attached_apps(db, owner_id, &row.id)
            .await?
            .into_iter()
            .filter(|app| app.owner_role.is_some())
            .map(|app| app.app_id)
            .collect()
    } else {
        BTreeSet::new()
    };
    let profile_app_ids = if settings.has(BotCapability::ProfileApps) {
        profile_app_ids(
            db,
            owner_id,
            settings.profile_id.as_deref(),
            request_profile,
        )
        .await?
    } else {
        BTreeSet::new()
    };
    let connectors = if settings.has(BotCapability::Connectors) {
        crate::bot_tools::connectors::rules_of(db, &row.id).await?
    } else {
        Vec::new()
    };
    let general = system == Some(BotSystemKind::General);
    let text = |value: Option<String>| {
        value
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    };
    let name = text(settings.name).unwrap_or_else(|| match system {
        Some(BotSystemKind::General) => GENERAL_BOT_NAME.to_string(),
        Some(BotSystemKind::Flux) => FLUX_BOT_NAME.to_string(),
        None => row.id.clone(),
    });
    let description =
        text(settings.description).or_else(|| general.then(|| GENERAL_BOT_DESCRIPTION.to_string()));
    Ok(Some(ChatBot {
        context: BotRunContext {
            bot_id: row.id,
            system,
            toolset: BotToolset::of(&settings.capabilities),
            name,
            description,
            instructions: text(settings.instructions),
            capabilities: settings.capabilities,
            classification: settings.classification,
            selected_app_ids,
            profile_app_ids,
            workspace_app_id: None,
            embedding_model: settings.embedding_model,
            decision_model: settings.model.decision.clone(),
            decision_hosted_only: settings.location != flow_like_types::bot::BotLocation::Desktop,
            acknowledgement: false,
            live_call: false,
            spoken_drafts: false,
            tool_gate: None,
            ephemeral_user_context: false,
            suppress_owner_notifications: false,
            stt_model: settings.stt_model,
            tts_model: settings.tts_model,
            image_model: settings.image_model,
            vision_model: settings.vision_model,
            subtask_model: settings.subtask_model,
            tool_host: BotToolHost::Chat,
            host_tools: Default::default(),
            skills: Vec::new(),
            skills_ranked: false,
            requester: None,
            permissions: Default::default(),
            connectors,
            compute: Default::default(),
            recovery: None,
        },
        model: settings.model,
    }))
}

/// Whose memory a memory request means: FlowPilot's in `profile_id` without a bot (or for Flux),
/// otherwise the store of one of the caller's bots.
pub(crate) async fn memory_scope<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    profile_id: &str,
    bot_id: Option<&str>,
) -> Result<MemoryScope, ApiError> {
    let flowpilot = || MemoryScope::Profile(profile_id.to_string());
    let Some(bot_id) = requested_bot_id(bot_id)? else {
        return Ok(flowpilot());
    };
    let row = match bot_id {
        FLUX_BOT_ID => return Ok(flowpilot()),
        GENERAL_BOT_ID => system_row(db, owner_id, BotSystemKind::General).await?,
        _ => Some(live_owned_bot(db, owner_id, bot_id).await?),
    };
    Ok(match row {
        None => MemoryScope::Bot(GENERAL_BOT_ID.to_string()),
        Some(row) if row.system_key.as_deref() == Some(BotSystemKind::Flux.as_str()) => flowpilot(),
        Some(row) => MemoryScope::Bot(row.id),
    })
}

fn not_yours(bot_id: &str) -> ApiError {
    ApiError::not_found(format!("Bot '{bot_id}' isn't one of yours"))
}

async fn live_owned_bot<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
) -> Result<bot::Model, ApiError> {
    bot::Entity::find_by_id(bot_id)
        .filter(bot::Column::OwnerId.eq(owner_id))
        .filter(bot::Column::DeletedAt.is_null())
        .one(db)
        .await?
        .ok_or_else(|| not_yours(bot_id))
}

async fn system_row<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    kind: BotSystemKind,
) -> Result<Option<bot::Model>, ApiError> {
    Ok(bot::Entity::find()
        .filter(bot::Column::OwnerId.eq(owner_id))
        .filter(bot::Column::SystemKey.eq(kind.as_str()))
        .filter(bot::Column::DeletedAt.is_null())
        .one(db)
        .await?)
}

fn app_ids(apps: &[ProfileApp]) -> BTreeSet<String> {
    apps.iter().map(|app| app.app_id.clone()).collect()
}

/// Apps of the bot's own profile (the caller's, not deleted), or of the request's profile when
/// the bot names none.
async fn profile_app_ids<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_profile_id: Option<&str>,
    request_profile: &Profile,
) -> Result<BTreeSet<String>, ApiError> {
    let request_apps = || app_ids(request_profile.apps.as_deref().unwrap_or_default());
    let Some(profile_id) = bot_profile_id.filter(|id| *id != request_profile.id) else {
        return Ok(request_apps());
    };
    owner_profile_app_ids(db, owner_id, profile_id).await
}

/// The apps `profile_id` lists when it is one of `owner_id`'s live profiles; empty otherwise.
pub(crate) async fn owner_profile_app_ids<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    profile_id: &str,
) -> Result<BTreeSet<String>, ApiError> {
    let apps: Option<Option<Value>> = profile::Entity::find()
        .select_only()
        .column(profile::Column::Apps)
        .filter(profile::Column::Id.eq(profile_id))
        .filter(profile::Column::UserId.eq(owner_id))
        .filter(profile::Column::DeletedAt.is_null())
        .into_tuple()
        .one(db)
        .await?;
    Ok(apps
        .flatten()
        .and_then(|apps| serde_json::from_value::<Vec<ProfileApp>>(apps).ok())
        .map(|apps| app_ids(&apps))
        .unwrap_or_default())
}

/// The bot's model on the server: its primary, then its fallbacks in order, the first the caller
/// can run here (a hosted model their plan covers, or one on their own provider). `None` leaves
/// the chat's own model selection.
pub(crate) async fn server_bot_model(
    state: &AppState,
    owner_id: &str,
    flow_like_state: &Arc<FlowLikeState>,
    profile: &Profile,
    policy: &BotModelPolicy,
) -> Option<String> {
    bot_model_where(state, owner_id, flow_like_state, profile, policy, false).await
}

/// [`server_bot_model`] for a model that leaves the server: only hosted models qualify, since a
/// customer provider's Bit carries the owner's credentials.
pub(crate) async fn hosted_bot_model(
    state: &AppState,
    owner_id: &str,
    flow_like_state: &Arc<FlowLikeState>,
    profile: &Profile,
    policy: &BotModelPolicy,
) -> Option<String> {
    bot_model_where(state, owner_id, flow_like_state, profile, policy, true).await
}

async fn bot_model_where(
    state: &AppState,
    owner_id: &str,
    flow_like_state: &Arc<FlowLikeState>,
    profile: &Profile,
    policy: &BotModelPolicy,
    hosted_only: bool,
) -> Option<String> {
    if policy.primary.is_none() && policy.fallbacks.is_empty() {
        return None;
    }
    let tier = crate::middleware::jwt::tier_for_sub(state, owner_id)
        .await
        .ok();
    let tier = tier.as_ref();
    resolve_bot_model(policy, move |model_id| async move {
        let Some(bit) = runnable_completion_model(flow_like_state, profile, &model_id).await else {
            return false;
        };
        let Some(provider) = bit.try_to_provider() else {
            return false;
        };
        ensure_metered_or_customer_provider(&provider.provider_name).is_ok()
            && (!hosted_only || is_hosted_provider_name(&provider.provider_name))
            && (!is_hosted_provider_name(&provider.provider_name)
                || tier.is_none_or(|tier| crate::model_tier::llm_bit_allowed(&bit, tier)))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::{super::validation::system_defaults, *};
    use crate::{
        db::DbDialect,
        entity::{bot_app, membership, role},
    };
    use axum::http::StatusCode;
    use chrono::{DateTime, FixedOffset};
    use flow_like_types::data_classification::{DataClassification, DataSensitivity};
    use sea_orm::{
        ActiveModelTrait, ActiveValue::Set, ConnectOptions, Database, DatabaseConnection, Schema,
        sea_query::Table,
    };

    async fn database() -> DatabaseConnection {
        let mut options = ConnectOptions::new("sqlite::memory:");
        options.max_connections(1);
        let db = Database::connect(options).await.unwrap();
        db.execute_unprepared("ATTACH DATABASE ':memory:' AS public")
            .await
            .unwrap();
        let schema = Schema::new(db.get_database_backend());
        for source in [
            schema.create_table_from_entity(bot::Entity),
            schema.create_table_from_entity(bot_app::Entity),
            schema.create_table_from_entity(membership::Entity),
            schema.create_table_from_entity(role::Entity),
            schema.create_table_from_entity(profile::Entity),
        ] {
            let mut table = Table::create();
            table.table(source.get_table_name().unwrap().clone());
            for column in source.get_columns() {
                table.col(column.clone());
            }
            db.execute(&table).await.unwrap();
        }
        db.execute_unprepared(
            r#"CREATE UNIQUE INDEX public."Bot_ownerId_systemKey_key" ON "Bot" ("ownerId", "systemKey")"#,
        )
        .await
        .unwrap();
        db
    }

    fn at(seconds: i64) -> DateTime<FixedOffset> {
        DateTime::from_timestamp(1_800_000_000 + seconds, 0)
            .unwrap()
            .fixed_offset()
    }

    async fn insert_bot(
        db: &DatabaseConnection,
        owner_id: &str,
        system: Option<BotSystemKind>,
        configure: impl FnOnce(&mut BotSettings),
    ) -> bot::Model {
        let mut settings = system_defaults(system.unwrap_or(BotSystemKind::Flux));
        if system.is_none() {
            settings.name = Some("Scout".into());
            settings.capabilities.clear();
        }
        configure(&mut settings);
        provision::new_bot_row(owner_id, system, None, &settings, at(0))
            .unwrap()
            .insert(db)
            .await
            .unwrap()
    }

    async fn member(db: &DatabaseConnection, user_id: &str, app_id: &str) {
        let role_id = format!("role-{app_id}");
        role::ActiveModel {
            id: Set(role_id.clone()),
            name: Set("Editor".into()),
            permissions: Set(1),
            app_id: Set(Some(app_id.into())),
            created_at: Set(at(0)),
            updated_at: Set(at(0)),
            ..Default::default()
        }
        .insert(db)
        .await
        .unwrap();
        membership::ActiveModel {
            id: Set(format!("member-{user_id}-{app_id}")),
            user_id: Set(user_id.into()),
            app_id: Set(app_id.into()),
            role_id: Set(role_id),
            created_at: Set(at(0)),
            updated_at: Set(at(0)),
            joined_via: Set(None),
        }
        .insert(db)
        .await
        .unwrap();
    }

    async fn attach(db: &DatabaseConnection, bot_id: &str, app_id: &str) {
        crate::routes::bots::apps::unrestricted_row(bot_id, app_id, at(0))
            .insert(db)
            .await
            .unwrap();
    }

    async fn insert_profile(db: &DatabaseConnection, id: &str, user_id: &str, apps: &[&str]) {
        let apps: Vec<ProfileApp> = apps
            .iter()
            .map(|app_id| ProfileApp::new(app_id.to_string()))
            .collect();
        profile::ActiveModel {
            id: Set(id.into()),
            name: Set(id.into()),
            apps: Set(Some(serde_json::to_value(apps).unwrap())),
            hub: Set("api.flow-like.com".into()),
            user_id: Set(user_id.into()),
            created_at: Set(at(0)),
            updated_at: Set(at(0)),
            ..Default::default()
        }
        .insert(db)
        .await
        .unwrap();
    }

    fn request_profile(apps: &[&str]) -> Profile {
        Profile {
            id: "profile-request".into(),
            apps: Some(
                apps.iter()
                    .map(|app_id| ProfileApp::new(app_id.to_string()))
                    .collect(),
            ),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn another_persons_deleted_or_unknown_bot_is_not_yours() {
        let db = database().await;
        let theirs = insert_bot(&db, "someone-else", None, |_| {}).await;
        let gone = insert_bot(&db, "owner", None, |_| {}).await;
        let mut deleted: bot::ActiveModel = gone.clone().into();
        deleted.deleted_at = Set(Some(at(10)));
        deleted.update(&db).await.unwrap();

        for bot_id in [theirs.id.as_str(), gone.id.as_str(), "bot_unknown"] {
            let error = load_chat_bot(&db, "owner", bot_id, &request_profile(&[]))
                .await
                .unwrap_err();
            assert_eq!(error.status(), StatusCode::NOT_FOUND);
            assert_eq!(
                error.public_message(),
                Some(format!("Bot '{bot_id}' isn't one of yours").as_str())
            );
        }
    }

    #[tokio::test]
    async fn flux_runs_flowpilot_with_its_rows_edits() {
        let db = database().await;
        assert!(
            load_chat_bot(&db, "owner", FLUX_BOT_ID, &request_profile(&[]))
                .await
                .unwrap()
                .is_none(),
            "without a row Flux is today's FlowPilot run"
        );

        let flux = insert_bot(&db, "owner", Some(BotSystemKind::Flux), |settings| {
            settings.name = Some("Fluxy".into());
            settings.instructions = Some("Always add tests.".into());
            settings.model = BotModelPolicy {
                decision: None,
                primary: Some("model-a".into()),
                fallbacks: Vec::new(),
            };
        })
        .await;
        insert_bot(&db, "someone-else", Some(BotSystemKind::Flux), |_| {}).await;
        for bot_id in [flux.id.as_str(), FLUX_BOT_ID] {
            let bot = load_chat_bot(&db, "owner", bot_id, &request_profile(&[]))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(bot.context.bot_id, flux.id);
            assert_eq!(bot.context.system, Some(BotSystemKind::Flux));
            assert_eq!(bot.context.toolset, BotToolset::FlowPilot);
            assert_eq!(bot.context.name, "Fluxy");
            assert_eq!(
                bot.context.instructions.as_deref(),
                Some("Always add tests.")
            );
            assert_eq!(bot.model.primary.as_deref(), Some("model-a"));
            assert_eq!(
                bot.context.memory_scope("profile-1"),
                MemoryScope::Profile("profile-1".into())
            );
        }

        let untouched = insert_bot(&db, "fresh", Some(BotSystemKind::Flux), |_| {}).await;
        let bot = load_chat_bot(&db, "fresh", &untouched.id, &request_profile(&[]))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(bot.context.name, FLUX_BOT_NAME);
        assert_eq!(
            flow_like::flow::copilot::bot_persona_section(&bot.context),
            None
        );
    }

    #[tokio::test]
    async fn bots_holding_flowpilot_run_its_toolset_with_their_own_memory() {
        let db = database().await;
        let duplicate = insert_bot(&db, "owner", None, |settings| {
            settings.name = Some("Flux copy".into());
            settings.capabilities = vec![BotCapability::Flowpilot];
        })
        .await;
        let bot = load_chat_bot(&db, "owner", &duplicate.id, &request_profile(&[]))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(bot.context.toolset, BotToolset::FlowPilot);
        assert_eq!(bot.context.system, None);
        assert_eq!(bot.context.name, "Flux copy");
        assert_eq!(
            bot.context.memory_scope("profile-1"),
            MemoryScope::Bot(duplicate.id.clone())
        );
        assert_eq!(
            memory_scope(&db, "owner", "profile-1", Some(&duplicate.id))
                .await
                .unwrap(),
            MemoryScope::Bot(duplicate.id.clone())
        );

        let stored_empty = insert_bot(&db, "early", Some(BotSystemKind::Flux), |settings| {
            settings.capabilities.clear();
        })
        .await;
        let flux = load_chat_bot(&db, "early", &stored_empty.id, &request_profile(&[]))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(flux.context.toolset, BotToolset::FlowPilot);
        assert_eq!(flux.context.capabilities, [BotCapability::Flowpilot]);
    }

    #[tokio::test]
    async fn a_bot_brings_its_memory_settings_to_the_run() {
        let db = database().await;
        let keeper = insert_bot(&db, "owner", None, |settings| {
            settings.capabilities = vec![BotCapability::Memory];
            settings.embedding_model = Some("embed-a".into());
        })
        .await;
        let quiet = insert_bot(&db, "owner", None, |_| {}).await;
        let load = |bot_id: String| {
            let db = &db;
            async move {
                load_chat_bot(db, "owner", &bot_id, &request_profile(&[]))
                    .await
                    .unwrap()
                    .unwrap()
                    .context
            }
        };
        let keeper = load(keeper.id).await;
        assert_eq!(keeper.embedding_model.as_deref(), Some("embed-a"));
        assert_eq!(keeper.memory_model(Some("embed-chat")), Some("embed-a"));
        assert_eq!(load(quiet.id).await.memory_model(Some("embed-chat")), None);
        assert_eq!(
            load(GENERAL_BOT_ID.into())
                .await
                .memory_model(Some("embed-chat")),
            Some("embed-chat"),
            "Assistant remembers by default"
        );
    }

    #[tokio::test]
    async fn assistant_is_the_callers_row_or_its_defaults() {
        let db = database().await;
        let defaults = load_chat_bot(&db, "owner", GENERAL_BOT_ID, &request_profile(&[]))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(defaults.context, BotRunContext::general());

        let row = insert_bot(&db, "owner", Some(BotSystemKind::General), |settings| {
            settings.instructions = Some("Say hello first.".into());
        })
        .await;
        insert_bot(
            &db,
            "someone-else",
            Some(BotSystemKind::General),
            |settings| {
                settings.name = Some("Not mine".into());
            },
        )
        .await;
        let assistant = load_chat_bot(&db, "owner", GENERAL_BOT_ID, &request_profile(&[]))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(assistant.context.bot_id, row.id);
        assert_eq!(assistant.context.name, GENERAL_BOT_NAME);
        assert_eq!(
            assistant.context.description.as_deref(),
            Some(GENERAL_BOT_DESCRIPTION)
        );
        assert_eq!(
            assistant.context.instructions.as_deref(),
            Some("Say hello first.")
        );
        assert_eq!(
            assistant.context.capabilities,
            [BotCapability::Internet, BotCapability::Memory]
        );
        assert_eq!(
            load_chat_bot(&db, "owner", &row.id, &request_profile(&[]))
                .await
                .unwrap()
                .unwrap()
                .context,
            assistant.context
        );
    }

    #[tokio::test]
    async fn a_custom_bot_runs_with_its_settings_and_the_apps_it_may_use() {
        let db = database().await;
        member(&db, "owner", "app-crm").await;
        member(&db, "someone-else", "app-left").await;
        insert_profile(&db, "profile-bot", "owner", &["app-notes", "app-wiki"]).await;
        insert_profile(&db, "profile-theirs", "someone-else", &["app-secret"]).await;
        let scout = insert_bot(&db, "owner", None, |settings| {
            settings.description = Some("Finds things".into());
            settings.instructions = Some("  ".into());
            settings.capabilities = vec![
                BotCapability::Internet,
                BotCapability::ProfileApps,
                BotCapability::SelectedApps,
                BotCapability::DataStudio,
            ];
            settings.profile_id = Some("profile-bot".into());
            settings.classification = DataSensitivity {
                classification: Some(DataClassification::Restricted),
                pii: true,
            };
            settings.model = BotModelPolicy {
                decision: None,
                primary: Some("model-a".into()),
                fallbacks: vec!["model-b".into()],
            };
        })
        .await;
        attach(&db, &scout.id, "app-crm").await;
        attach(&db, &scout.id, "app-left").await;

        let bot = load_chat_bot(&db, "owner", &scout.id, &request_profile(&["app-request"]))
            .await
            .unwrap()
            .unwrap();
        let context = &bot.context;
        assert_eq!(context.bot_id, scout.id);
        assert_eq!(context.name, "Scout");
        assert_eq!(context.description.as_deref(), Some("Finds things"));
        assert_eq!(context.instructions, None);
        assert_eq!(
            context.classification.classification,
            Some(DataClassification::Restricted)
        );
        assert_eq!(context.selected_app_ids, ["app-crm".to_string()].into());
        assert_eq!(
            context.profile_app_ids,
            ["app-notes".to_string(), "app-wiki".to_string()].into()
        );
        for app_id in ["app-crm", "app-notes", "app-wiki"] {
            assert!(context.allows_app(app_id), "{app_id}");
        }
        for app_id in ["app-left", "app-request", "app-secret"] {
            assert!(!context.allows_app(app_id), "{app_id}");
        }
        assert_eq!(context.workspace_app_id, None);
        assert_eq!(context.toolset, BotToolset::Capabilities);
        assert_eq!(
            context.memory_scope("profile-1"),
            MemoryScope::Bot(scout.id.clone())
        );
        assert_eq!(bot.model.primary.as_deref(), Some("model-a"));
        assert_eq!(bot.model.fallbacks, ["model-b"]);

        let theirs = insert_bot(&db, "owner", None, |settings| {
            settings.capabilities = vec![BotCapability::ProfileApps];
            settings.profile_id = Some("profile-theirs".into());
        })
        .await;
        assert!(
            load_chat_bot(&db, "owner", &theirs.id, &request_profile(&[]))
                .await
                .unwrap()
                .unwrap()
                .context
                .profile_app_ids
                .is_empty()
        );

        let unscoped = insert_bot(&db, "owner", None, |settings| {
            settings.capabilities = vec![BotCapability::ProfileApps];
        })
        .await;
        assert_eq!(
            load_chat_bot(
                &db,
                "owner",
                &unscoped.id,
                &request_profile(&["app-request"])
            )
            .await
            .unwrap()
            .unwrap()
            .context
            .profile_app_ids,
            ["app-request".to_string()].into()
        );

        let switched_off = insert_bot(&db, "owner", None, |settings| {
            settings.capabilities = vec![BotCapability::Internet];
            settings.profile_id = Some("profile-bot".into());
        })
        .await;
        attach(&db, &switched_off.id, "app-crm").await;
        let switched_off = load_chat_bot(
            &db,
            "owner",
            &switched_off.id,
            &request_profile(&["app-request"]),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(switched_off.context.selected_app_ids.is_empty());
        assert!(switched_off.context.profile_app_ids.is_empty());
        assert!(!switched_off.context.allows_app("app-crm"));
    }

    #[tokio::test]
    async fn memory_requests_reach_only_the_callers_own_stores() {
        let db = database().await;
        let flux = insert_bot(&db, "owner", Some(BotSystemKind::Flux), |_| {}).await;
        let scout = insert_bot(&db, "owner", None, |_| {}).await;
        let theirs = insert_bot(&db, "someone-else", None, |_| {}).await;
        let flowpilot = MemoryScope::Profile("profile-1".into());

        let scope = |bot_id: Option<String>| {
            let db = &db;
            async move { memory_scope(db, "owner", "profile-1", bot_id.as_deref()).await }
        };
        assert_eq!(scope(None).await.unwrap(), flowpilot);
        assert_eq!(scope(Some(FLUX_BOT_ID.into())).await.unwrap(), flowpilot);
        assert_eq!(scope(Some(flux.id.clone())).await.unwrap(), flowpilot);
        assert_eq!(
            scope(Some(GENERAL_BOT_ID.into())).await.unwrap(),
            MemoryScope::Bot(GENERAL_BOT_ID.into())
        );
        assert_eq!(
            scope(Some(scout.id.clone())).await.unwrap(),
            MemoryScope::Bot(scout.id.clone())
        );
        let general = insert_bot(&db, "owner", Some(BotSystemKind::General), |_| {}).await;
        assert_eq!(
            scope(Some(GENERAL_BOT_ID.into())).await.unwrap(),
            MemoryScope::Bot(general.id.clone())
        );
        assert_eq!(
            scope(Some(theirs.id.clone())).await.unwrap_err().status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            scope(Some("b".repeat(MAX_BOT_ID_CHARS + 1)))
                .await
                .unwrap_err()
                .status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(scope(Some("  ".into())).await.unwrap(), flowpilot);
    }

    #[tokio::test]
    async fn general_provisioning_is_idempotent_per_owner() {
        let db = database().await;
        provision::insert_system_bots(&db, DbDialect::Postgres, "owner", &BotSystemKind::ALL)
            .await
            .unwrap();
        let first = load_chat_bot(&db, "owner", GENERAL_BOT_ID, &request_profile(&[]))
            .await
            .unwrap()
            .unwrap();
        provision::insert_system_bots(&db, DbDialect::Postgres, "owner", &BotSystemKind::ALL)
            .await
            .unwrap();
        let second = load_chat_bot(&db, "owner", GENERAL_BOT_ID, &request_profile(&[]))
            .await
            .unwrap()
            .unwrap();
        assert!(first.context.bot_id.starts_with("bot_"));
        assert_eq!(first.context.bot_id, second.context.bot_id);
    }
}
