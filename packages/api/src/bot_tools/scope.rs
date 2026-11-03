use std::collections::BTreeSet;

use flow_like::flow::copilot::{BotRunContext, EffectivePermissions};
use flow_like_bot_tools_host::{JobApp, JobScope};
use flow_like_storage::contracts::column_metadata::ColumnReader;
use flow_like_types::bot::BotCapability;
use flow_like_types::bot_restrictions::BotPermission;
use flow_like_types::data_classification::DataClassification;

use crate::{
    bot_permission::bot_permission_with_grant,
    error::ApiError,
    middleware::jwt::{AppPermissionResponse, BotUser},
    routes::app::db::{ScopeParams, column_reader},
    state::AppState,
};

/// What the bot holds in one app: the owner's role narrowed by the bot's restrictions and the
/// room's limits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BotRights {
    pub effective: EffectivePermissions,
    pub reader: ColumnReader,
}

impl BotRights {
    pub(crate) fn of(permission: &AppPermissionResponse, effective: EffectivePermissions) -> Self {
        Self {
            effective,
            reader: column_reader(permission, &ScopeParams::default()),
        }
    }

    fn files(&self) -> bool {
        self.effective.holds(BotPermission::ReadFiles)
    }

    fn tables(&self) -> bool {
        let named = self
            .effective
            .scopes
            .tables
            .as_ref()
            .is_none_or(|tables| !tables.is_empty());
        named && (self.files() || self.effective.holds(BotPermission::ReadDatabase))
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

/// The scope of one bot turn from what the bot holds in each candidate app. An app the bot holds
/// nothing in is left out.
pub(crate) fn scope_from(
    bot: &BotRunContext,
    rights: impl IntoIterator<Item = (String, Option<BotRights>)>,
) -> JobScope {
    let apps = rights
        .into_iter()
        .filter_map(|(app_id, rights)| {
            let rights = rights?;
            let (files, tables) = (rights.files(), rights.tables());
            if !files && !tables {
                return None;
            }
            Some(JobApp {
                app_id,
                alias: None,
                files,
                tables,
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
    principal: &BotUser,
    bot: &BotRunContext,
) -> Result<JobScope, ApiError> {
    let mut rights = Vec::new();
    for app_id in candidate_apps(bot) {
        let held = match bot_permission_with_grant(principal, &app_id, state).await {
            Ok((permission, grant)) => Some(BotRights::of(&permission, grant.effective)),
            Err(_) => None,
        };
        rights.push((app_id, held));
    }
    Ok(scope_from(bot, rights))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permission::role_permission::RolePermissions;

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

    fn rights(bits: i64, reader: ColumnReader) -> Option<BotRights> {
        Some(BotRights {
            effective: EffectivePermissions {
                role_mask: Some(bits),
                ..EffectivePermissions::default()
            },
            reader,
        })
    }

    const FILES: i64 = RolePermissions::ReadFiles.bits();
    const DATABASE: i64 = RolePermissions::ReadDatabase.bits();

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
                ("a".to_string(), rights(FILES, ColumnReader::Unrestricted)),
                (
                    "b".to_string(),
                    rights(DATABASE, ColumnReader::with_attributes(["hr"])),
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
            [("a".to_string(), rights(0, ColumnReader::Unrestricted))],
        );
        assert!(scope.apps.is_empty());
    }

    #[test]
    fn a_restricted_bot_loses_the_file_access_its_mask_does_not_grant() {
        let scope = scope_from(
            &bot(),
            [
                (
                    "a".to_string(),
                    rights(DATABASE, ColumnReader::Unrestricted),
                ),
                ("b".to_string(), rights(FILES, ColumnReader::Unrestricted)),
            ],
        );
        assert!(!scope.apps[0].files && scope.apps[0].tables);
        assert!(scope.apps[1].files && scope.apps[1].tables);
    }

    #[test]
    fn an_empty_table_allowlist_closes_the_tables_even_with_file_access() {
        let mut held = rights(FILES | DATABASE, ColumnReader::Unrestricted).unwrap();
        held.effective.scopes.tables = Some(Vec::new());
        let scope = scope_from(&bot(), [("a".to_string(), Some(held))]);
        assert!(scope.apps[0].files && !scope.apps[0].tables);
    }
}
