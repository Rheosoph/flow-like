use std::collections::BTreeSet;

use flow_like::flow::copilot::BotRunContext;
use flow_like_bot_tools_host::{JobApp, JobScope};
use flow_like_storage::contracts::column_metadata::ColumnReader;
use flow_like_types::bot::BotCapability;
use flow_like_types::data_classification::DataClassification;

use crate::{
    error::ApiError,
    middleware::jwt::{AppPermissionResponse, user_app_permission_uncached},
    permission::role_permission::RolePermissions,
    routes::app::db::{ScopeParams, column_reader},
    state::AppState,
};

/// What the bot's owner holds in one app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OwnerRights {
    pub files: bool,
    pub tables: bool,
    pub reader: ColumnReader,
}

impl OwnerRights {
    pub(crate) fn of(permission: &AppPermissionResponse) -> Self {
        let files = permission.has_permission(RolePermissions::ReadFiles);
        Self {
            files,
            tables: files || permission.has_permission(RolePermissions::ReadDatabase),
            reader: column_reader(permission, &ScopeParams::default()),
        }
    }
}

fn column_attributes(reader: &ColumnReader) -> Option<Vec<String>> {
    match reader {
        ColumnReader::Unrestricted => None,
        ColumnReader::Attributes(held) => Some(held.iter().cloned().collect()),
    }
}

/// The apps a bot may name: attached apps under `selected_apps`, profile apps under
/// `profile_apps`.
pub(crate) fn candidate_apps(bot: &BotRunContext) -> BTreeSet<String> {
    bot.selected_app_ids
        .iter()
        .chain(&bot.profile_app_ids)
        .filter(|app_id| bot.allows_app(app_id))
        .filter(|app_id| bot.workspace_app_id.as_deref() != Some(app_id.as_str()))
        .cloned()
        .collect()
}

/// The scope of one bot turn from what its owner holds in each candidate app. An app the owner
/// holds nothing in is left out.
pub(crate) fn scope_from(
    bot: &BotRunContext,
    rights: impl IntoIterator<Item = (String, Option<OwnerRights>)>,
) -> JobScope {
    let apps = rights
        .into_iter()
        .filter_map(|(app_id, rights)| {
            let rights = rights.filter(|rights| rights.files || rights.tables)?;
            Some(JobApp {
                app_id,
                alias: None,
                files: rights.files,
                tables: rights.tables,
                column_attributes: column_attributes(&rights.reader),
            })
        })
        .collect();
    JobScope {
        storage: bot.has(BotCapability::Storage),
        data_connections: bot.has(BotCapability::DataConnections),
        classification: bot
            .classification
            .classification
            .unwrap_or(DataClassification::Internal),
        apps,
    }
}

pub(crate) async fn job_scope(
    state: &AppState,
    owner: &str,
    bot: &BotRunContext,
) -> Result<JobScope, ApiError> {
    let mut rights = Vec::new();
    for app_id in candidate_apps(bot) {
        let held = match user_app_permission_uncached(owner, &app_id, state).await {
            Ok(permission) => Some(OwnerRights::of(&permission)),
            Err(_) => None,
        };
        rights.push((app_id, held));
    }
    Ok(scope_from(bot, rights))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bot() -> BotRunContext {
        BotRunContext {
            capabilities: vec![
                BotCapability::SelectedApps,
                BotCapability::ProfileApps,
                BotCapability::Storage,
            ],
            selected_app_ids: ["a".to_string(), "ws".to_string()].into(),
            profile_app_ids: ["b".to_string(), "c".to_string()].into(),
            workspace_app_id: Some("ws".to_string()),
            ..BotRunContext::general()
        }
    }

    fn rights(files: bool, tables: bool, reader: ColumnReader) -> Option<OwnerRights> {
        Some(OwnerRights {
            files,
            tables,
            reader,
        })
    }

    #[test]
    fn candidates_are_the_selected_and_profile_apps_without_the_workspace() {
        let candidates: Vec<_> = candidate_apps(&bot()).into_iter().collect();
        assert_eq!(candidates, ["a", "b", "c"]);
        let mut without_profile = bot();
        without_profile.capabilities = vec![BotCapability::SelectedApps];
        assert_eq!(candidate_apps(&without_profile).len(), 1);
    }

    #[test]
    fn the_scope_keeps_only_apps_the_owner_holds_and_their_column_attributes() {
        let scope = scope_from(
            &bot(),
            [
                (
                    "a".to_string(),
                    rights(true, true, ColumnReader::Unrestricted),
                ),
                (
                    "b".to_string(),
                    rights(false, true, ColumnReader::with_attributes(["hr"])),
                ),
                ("c".to_string(), None),
            ],
        );
        assert!(scope.storage && !scope.data_connections);
        assert_eq!(scope.apps.len(), 2);
        assert_eq!(scope.apps[0].column_attributes, None);
        assert!(!scope.apps[1].files && scope.apps[1].tables);
        assert_eq!(
            scope.apps[1].column_attributes,
            Some(vec!["hr".to_string()])
        );
    }

    #[test]
    fn an_owner_with_no_rights_in_an_app_drops_it() {
        let scope = scope_from(
            &bot(),
            [(
                "a".to_string(),
                rights(false, false, ColumnReader::Unrestricted),
            )],
        );
        assert!(scope.apps.is_empty());
    }
}
