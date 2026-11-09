//! Skills on the server: the bot's `SkillHost` (light database reads, run in the API) and the
//! owner's zip import, which hands the untrusted zip to the executor (`skill_import` job).

use std::{collections::BTreeMap, time::Duration};

use async_trait::async_trait;
use axum::http::StatusCode;
use chrono::Utc;
use flow_like_bot_tools::{SkillHost, skills::SKILL_IMPORT};
use flow_like_bot_tools_host::{JobModels, JobScope, SkillCallJob, ToolJob};
use flow_like_storage::object_store::ObjectStoreExt;
use flow_like_types::{
    Result, anyhow,
    bot_skill::{BotSkill, CreateBotSkill, ImportedSkill, SKILLS_DIR, SkillStatus},
    data_classification::DataClassification,
};
use futures::TryStreamExt;
use serde_json::{Value, json};

use super::{access::owner_credentials, executor::DispatcherExecutor, executor::ToolExecutor};
use crate::{
    credentials::{CredentialsAccess, RuntimeCredentials},
    error::ApiError,
    middleware::jwt::BotUser,
    routes::bots::skills::{self, Origin, SKILL_IMPORT_NEEDS_EXECUTOR, SKILL_IMPORT_REFUSED},
    state::AppState,
};

/// Finds the package a skill's job runs in; only the API can read the pins and sign URLs.
#[async_trait]
pub(crate) trait SkillJobs: Send + Sync {
    async fn job(&self, skill: &str) -> std::result::Result<SkillCallJob, String>;
}

pub(crate) struct ApiSkillJobs {
    pub skills: ApiSkills,
    pub state: AppState,
    pub workspace: String,
}

#[async_trait]
impl SkillJobs for ApiSkillJobs {
    async fn job(&self, name: &str) -> std::result::Result<SkillCallJob, String> {
        let skill = self
            .skills
            .get(name)
            .await
            .map_err(|error| error.to_string())?
            .filter(|skill| skill.status == SkillStatus::Active)
            .ok_or_else(|| format!("You have no active skill named '{name}'"))?;
        let Some(package_id) = skill.source.package_id().map(str::to_string) else {
            return Ok(SkillCallJob {
                skill,
                package_id: String::new(),
                package: None,
            });
        };
        let mut packages =
            crate::execution::wasm_resolve::resolve_wasm_packages(&self.state, &self.workspace)
                .await
                .unwrap_or_default();
        let package = packages.remove(&package_id).ok_or_else(|| {
            format!("The package '{package_id}' is not available for this bot's workspace")
        })?;
        Ok(SkillCallJob {
            skill,
            package_id,
            package: Some(package),
        })
    }
}

const IMPORT_TIMEOUT: Duration = Duration::from_secs(90);

/// A bot's skills in the `BotSkill` table, kept through the functions the owner's endpoints use.
pub(crate) struct ApiSkills {
    state: AppState,
    owner_id: String,
    bot_id: String,
}

impl ApiSkills {
    pub(crate) fn new(state: &AppState, owner_id: &str, bot_id: &str) -> Self {
        Self {
            state: state.clone(),
            owner_id: owner_id.to_string(),
            bot_id: bot_id.to_string(),
        }
    }
}

#[async_trait]
impl SkillHost for ApiSkills {
    async fn get(&self, name: &str) -> Result<Option<BotSkill>> {
        let row = skills::by_name(&self.state.db, &self.bot_id, name)
            .await
            .map_err(|error| anyhow!("{error}"))?;
        Ok(row.map(skills::to_skill))
    }

    async fn propose(&self, request: CreateBotSkill) -> Result<BotSkill> {
        let row = self
            .state
            .transaction(|txn| {
                let (owner, bot, request) =
                    (self.owner_id.clone(), self.bot_id.clone(), request.clone());
                Box::pin(async move {
                    super::super::routes::bots::lock_bot(txn, &bot).await?;
                    skills::create_in(
                        txn,
                        &owner,
                        &bot,
                        request,
                        Origin::proposed_by_bot(),
                        Utc::now().fixed_offset(),
                    )
                    .await
                })
            })
            .await
            .map_err(|error| anyhow!("{error}"))?;
        Ok(skills::to_skill(row))
    }
}

fn workspace_store_error(error: impl std::fmt::Display) -> ApiError {
    ApiError::internal(format!("Opening the bot workspace failed: {error}"))
}

async fn workspace_store(
    state: &AppState,
    owner: &str,
    workspace: &str,
) -> Result<flow_like_storage::files::store::FlowLikeStore, ApiError> {
    let credentials =
        RuntimeCredentials::scoped(owner, workspace, state, CredentialsAccess::EditAppContent)
            .await
            .map_err(workspace_store_error)?;
    credentials
        .to_store(false)
        .await
        .map_err(workspace_store_error)
}

fn refused(message: impl Into<String>) -> ApiError {
    ApiError::coded(StatusCode::BAD_REQUEST, SKILL_IMPORT_REFUSED, message)
}

/// Stores the zip in the workspace, has the executor read it, and returns the validated skill.
/// The executor wrote the references; the zip itself is removed afterwards.
pub(crate) async fn import_zip(
    state: &AppState,
    owner: &str,
    bot_id: &str,
    workspace: &str,
    zip: bytes::Bytes,
) -> Result<ImportedSkill, ApiError> {
    let executor = DispatcherExecutor::of(state).ok_or_else(|| {
        ApiError::coded(
            StatusCode::SERVICE_UNAVAILABLE,
            SKILL_IMPORT_NEEDS_EXECUTOR,
            "This server has no executor to read a skill zip safely. Nothing was imported.",
        )
    })?;
    let turn_id = format!("skill-import-{}", uuid::Uuid::new_v4().simple());
    let path = format!("{SKILLS_DIR}/_incoming/{turn_id}.zip");
    let store = workspace_store(state, owner, workspace).await?;
    let key = store
        .construct_upload(workspace, &path)
        .await
        .map_err(|error| ApiError::internal(format!("Bot workspace path '{path}': {error}")))?;
    store
        .as_generic()
        .put(&key, zip.into())
        .await
        .map_err(|error| ApiError::internal(format!("Saving the skill zip failed: {error}")))?;
    let outcome = run_import_job(state, &executor, owner, bot_id, workspace, &turn_id, &path).await;
    let _ = store.as_generic().delete(&key).await;
    outcome
}

async fn run_import_job(
    state: &AppState,
    executor: &DispatcherExecutor,
    owner: &str,
    bot_id: &str,
    workspace: &str,
    turn_id: &str,
    path: &str,
) -> Result<ImportedSkill, ApiError> {
    let credential = owner_credentials(state, owner, workspace)
        .await
        .map_err(|error| {
            ApiError::internal(format!("Scoping the workspace credential failed: {error}"))
        })?;
    let job = ToolJob {
        tool: SKILL_IMPORT.to_string(),
        args: json!({ "path": path }),
        owner: owner.to_string(),
        bot_id: bot_id.to_string(),
        turn_id: turn_id.to_string(),
        workspace: workspace.to_string(),
        scope: JobScope {
            storage: false,
            data_connections: false,
            classification: DataClassification::Internal,
            apps: Vec::new(),
        },
        credentials: BTreeMap::from([(
            workspace.to_string(),
            serde_json::to_value(credential).map_err(|error| {
                ApiError::internal(format!("Encoding the credential failed: {error}"))
            })?,
        )]),
        models: JobModels::default(),
        allow_expensive_documents: false,
        secrets: Default::default(),
        connectors: Vec::new(),
        skill_call: None,
    };
    let job = serde_json::to_string(&job)
        .map_err(|error| ApiError::internal(format!("Encoding the import job failed: {error}")))?;
    let principal = BotUser {
        bot_id: bot_id.to_string(),
        turn_id: turn_id.to_string(),
        conv_id: None,
    };
    let reply = executor
        .run(owner, &principal, workspace, job, IMPORT_TIMEOUT)
        .await
        .map_err(|message| {
            tracing::warn!(%message, bot_id, "[skills] the import job failed");
            ApiError::bad_gateway(
                "The executor could not read the skill zip. Nothing was imported.",
            )
        })?;
    parse_reply(&reply.output)
}

/// The executor's envelope: `status: ok` plus the skill fields, or an error message.
pub(crate) fn parse_reply(output: &str) -> Result<ImportedSkill, ApiError> {
    let envelope: Value = serde_json::from_str(output)
        .map_err(|_| ApiError::bad_gateway("The executor answered with something unreadable"))?;
    if envelope["status"] != "ok" {
        let message = envelope["error"]["message"]
            .as_str()
            .unwrap_or("The zip was refused");
        return Err(refused(message));
    }
    serde_json::from_value(envelope)
        .map_err(|_| ApiError::bad_gateway("The executor answered with an unexpected skill shape"))
}

/// Deletes the files a skill brought, best effort: the row is already gone.
pub(crate) async fn remove_references(state: &AppState, owner: &str, bot_id: &str, name: &str) {
    let result: Result<(), ApiError> = async {
        let workspace = crate::routes::bots::workspace::ensure(state, owner, bot_id)
            .await?
            .app_id;
        let store = workspace_store(state, owner, &workspace).await?;
        let prefix = store
            .construct_upload(&workspace, &flow_like_types::bot_skill::skill_dir(name))
            .await
            .map_err(workspace_store_error)?;
        let generic = store.as_generic();
        let found: Vec<_> = generic
            .list(Some(&prefix))
            .try_collect()
            .await
            .map_err(workspace_store_error)?;
        for meta in found {
            let _ = generic.delete(&meta.location).await;
        }
        Ok(())
    }
    .await;
    if let Err(error) = result {
        tracing::warn!(%error, bot_id, name, "[skills] the skill's files were not removed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_executor_refusal_becomes_a_refused_import() {
        let output = json!({ "status": "error", "error": { "code": "invalid_args", "message": "zip bomb" } });
        let error = parse_reply(&output.to_string()).unwrap_err();
        assert_eq!(error.status(), StatusCode::BAD_REQUEST);
        assert_eq!(error.public_code(), SKILL_IMPORT_REFUSED);
    }

    #[test]
    fn an_ok_envelope_carries_the_skill_fields() {
        let output = json!({
            "status": "ok", "tool": "skill_import", "name": "a", "description": "d", "body": "b",
            "files": ["bot/skills/a/SKILL.md"], "skipped": [], "truncated": false
        });
        let imported = parse_reply(&output.to_string()).unwrap();
        assert_eq!(imported.name, "a");
        assert_eq!(imported.files.len(), 1);
    }
}
