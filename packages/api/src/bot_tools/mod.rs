//! Bot tools on the server: files, knowledge, documents, data, Excel and code for web chat and
//! headless turns. Light calls (Lance tables, object reads) run in the API; everything that
//! parses, writes files or runs code is a tool job on the executor. Contract:
//! `todo/bots-capabilities/lanes/HOST-SERVER-TOOLS.md`.

mod access;
pub(crate) mod apps;
mod configuration;
pub(crate) mod connectors;
mod crew;
mod directory;
mod email;
mod executor;
mod host;
mod images;
mod routing;
mod schedule;
mod scope;
mod secrets;
pub(crate) mod skills;
mod turn_files;

#[cfg(test)]
mod tests;

pub(crate) use access::{ApiAccess, owner_credentials};
pub(crate) use executor::{DispatcherExecutor, ToolExecutor};
pub(crate) use host::{AppDirectory, BotToolsHost, TurnDeps, TurnModels};
pub(crate) use turn_files::AttachedFile;

use flow_like::flow::copilot::HostTools;

use crate::{middleware::jwt::BotUser, state::AppState};

/// What this deployment can run for a bot: the in-process set, plus the executor's tools when
/// an executor takes tool jobs.
pub(crate) fn host_tools(state: &AppState) -> HostTools {
    routing::host_tools(
        executor::available(state),
        &routing::code_languages_from_env(),
    )
}

pub(crate) fn executor_for(state: &AppState) -> Option<std::sync::Arc<dyn ToolExecutor>> {
    DispatcherExecutor::of(state).map(|executor| std::sync::Arc::new(executor) as _)
}

/// The bot tools of one turn, including configuration tools before a workspace exists.
/// `attached` holds the files of the message being answered, by attachment name.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn turn_host(
    state: &AppState,
    owner: &str,
    turn_id: &str,
    bot: &flow_like::flow::copilot::BotRunContext,
    profile: &flow_like::profile::Profile,
    flow_like_state: &std::sync::Arc<flow_like::state::FlowLikeState>,
    chat: &flow_like::bit::Bit,
    token: Option<String>,
    grants: std::sync::Arc<dyn flow_like::flow::copilot::tool_grants::GrantStore>,
    conv_id: Option<&str>,
    context_conversation: Option<&str>,
    site: Option<flow_like_bot_tools::TurnSite>,
    attached: std::collections::BTreeMap<String, AttachedFile>,
) -> Option<std::sync::Arc<BotToolsHost>> {
    let workspace = bot.workspace_app_id.clone().unwrap_or_default();
    let principal = BotUser {
        bot_id: bot.bot_id.clone(),
        turn_id: turn_id.to_string(),
        conv_id: conv_id.map(str::to_string),
    };
    if bot.host_tools.is_empty() {
        return None;
    }
    let embedding = match bot
        .embedding_model
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    {
        Some(id) => profile
            .find_bit(id, flow_like_state.http_client.clone())
            .await
            .map_err(|error| tracing::warn!(%error, id, "Bot tools: embedding model not found"))
            .ok(),
        None => None,
    };
    let vision = match bot
        .vision_model
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    {
        Some(id) => profile
            .find_bit(id, flow_like_state.http_client.clone())
            .await
            .map_err(|error| tracing::warn!(%error, id, "Bot tools: vision model not found"))
            .ok(),
        None => None,
    };
    let models = flow_like_bot_tools_host::StateModels {
        state: flow_like_state.clone(),
        chat: Some(chat.clone()),
        embedding: embedding.clone(),
        token,
        usage: flow_like::models::llm::ModelUsageContext {
            app_id: Some(workspace.clone()),
            run_id: Some(turn_id.to_string()),
            api_base_url: None,
        },
    };
    let vision_source = vision.clone().map(|bit| {
        std::sync::Arc::new(models.answering_with(bit))
            as std::sync::Arc<dyn flow_like_bot_tools_host::ModelSource>
    });
    let source = std::sync::Arc::new(flow_like_bot_tools_host::VisionRouted::new(
        std::sync::Arc::new(models),
        vision_source,
    ));
    let notifier = std::sync::Arc::new(
        crate::bot_notify::ServerNotifier::new(state, &bot.bot_id, site).with_turn(turn_id),
    );
    let scheduler = std::sync::Arc::new(schedule::ApiScheduler::new(state, owner, &bot.bot_id));
    let crew = (bot.toolset == flow_like::flow::copilot::BotToolset::FlowPilot).then(|| {
        std::sync::Arc::new(crew::ApiCrew::new(
            state,
            owner,
            &bot.bot_id,
            std::sync::Arc::new(profile.clone()),
            flow_like_state.clone(),
        )) as std::sync::Arc<dyn flow_like_bot_tools::crew::CrewBackend>
    });
    let host = BotToolsHost::new(TurnDeps {
        owner: owner.to_string(),
        bot: principal.clone(),
        turn_id: turn_id.to_string(),
        workspace: workspace.clone(),
        host_tools: bot.host_tools.clone().with_user_context(
            bot.has(flow_like_types::bot::BotCapability::UserContext)
                && context_conversation.is_some(),
        ),
        directory: std::sync::Arc::new(directory::ApiDirectory {
            state: state.clone(),
            principal: principal.clone(),
            bot: bot.clone(),
        }),
        access: std::sync::Arc::new(access::ApiAccess::for_bot(
            state.clone(),
            principal,
            &workspace,
            context_conversation,
        )),
        models: TurnModels {
            source,
            chat: Some(chat.clone()),
            embedding,
            vision,
        },
        executor: executor_for(state),
        grants,
        secrets: std::sync::Arc::new(secrets::ServerSecrets::new(state, &bot.bot_id)),
        connectors: Some(std::sync::Arc::new(connectors::ApiConnectors::new(
            state,
            owner,
            &bot.bot_id,
        ))),
    })
    .with_configuration(std::sync::Arc::new(configuration::ApiConfiguration::new(
        state,
        owner,
        &bot.bot_id,
        std::sync::Arc::new(profile.clone()),
        flow_like_state.clone(),
    )))
    .with_notifier(notifier)
    .with_people(std::sync::Arc::new(crate::bot_people::ServerPeople::new(
        state,
        &bot.bot_id,
        owner,
    )))
    .with_user_context(std::sync::Arc::new(
        crate::routes::bots::user_context::ServerUserContext::new(
            state,
            &bot.bot_id,
            context_conversation,
        ),
    ))
    .with_scheduler(scheduler)
    .with_mail(std::sync::Arc::new(email::ApiMail::new(state, &bot.bot_id)))
    .with_skills(std::sync::Arc::new(skills::ApiSkills::new(
        state,
        owner,
        &bot.bot_id,
    )))
    .with_skill_jobs(std::sync::Arc::new(skills::ApiSkillJobs {
        skills: skills::ApiSkills::new(state, owner, &bot.bot_id),
        state: state.clone(),
        workspace: workspace.clone(),
    }))
    .with_images(std::sync::Arc::new(images::ImageBits::new(
        std::sync::Arc::new(profile.clone()),
        flow_like_state.clone(),
        bot.image_model.clone(),
    )))
    .with_files(std::sync::Arc::new(turn_files::ApiTurnFiles::new(
        state,
        &bot.bot_id,
        context_conversation,
        attached,
    )));
    let host = if bot.tool_host == flow_like::flow::copilot::BotToolHost::Hosted {
        host.with_apps(std::sync::Arc::new(apps::AppTools::new(
            state,
            BotUser {
                bot_id: bot.bot_id.clone(),
                turn_id: turn_id.to_string(),
                conv_id: conv_id.map(str::to_string),
            },
            bot,
        )))
    } else {
        host
    };
    Some(std::sync::Arc::new(match crew {
        Some(crew) => host.with_crew(crew),
        None => host,
    }))
}
