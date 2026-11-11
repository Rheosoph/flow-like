use async_trait::async_trait;
use flow_like::credentials::{SharedCredentials, StoreType};
use flow_like::flow::copilot::{EffectivePermissions, TURN_FILES};
use flow_like_bot_tools_host::AppAccess;
use flow_like_storage::files::store::FlowLikeStore;
use flow_like_storage::lancedb::Connection;
use flow_like_types::Result;
use flow_like_types::bot_restrictions::BotPermission;

use crate::{
    bot_permission::bot_grant_in,
    credentials::{CredentialsAccess, RuntimeCredentialsTrait},
    middleware::jwt::BotUser,
    state::AppState,
};

/// The narrowest credential mode the bot's effective rights allow in `app_id`: write only on the
/// workspace, only with `WriteFiles` (and no read-only file scope) or `WriteDatabase`, and the
/// project database alone when the bot holds no file read. `None` when it holds neither.
pub(crate) fn credentials_mode(
    effective: &EffectivePermissions,
    workspace: &str,
    app_id: &str,
) -> Option<CredentialsAccess> {
    let own = app_id == workspace;
    let write_files =
        own && effective.holds(BotPermission::WriteFiles) && !effective.scopes.files.read_only;
    if effective.holds(BotPermission::ReadFiles) {
        return Some(if write_files {
            CredentialsAccess::EditAppContent
        } else {
            CredentialsAccess::ReadAppContent
        });
    }
    if effective.holds(BotPermission::ReadDatabase) {
        return Some(if own && effective.holds(BotPermission::WriteDatabase) {
            CredentialsAccess::EditAppDb
        } else {
            CredentialsAccess::ReadAppDb
        });
    }
    None
}

/// Credentials minted for the bot principal at the narrowed mode, re-resolved from the database
/// so a revoked grant stops the next handout.
pub(crate) async fn shared_credentials(
    state: &AppState,
    bot: &BotUser,
    workspace: &str,
    app_id: &str,
) -> Result<SharedCredentials> {
    let grant = bot_grant_in(&state.db, bot, app_id)
        .await
        .map_err(|error| flow_like_types::anyhow!("{error}"))?;
    let mode = credentials_mode(&grant.effective, workspace, app_id).ok_or_else(|| {
        flow_like_types::anyhow!("The bot holds no file or database access in app '{app_id}'")
    })?;
    let credentials = state.scoped_credentials(&bot.bot_id, app_id, mode).await?;
    Ok(credentials.into_shared_credentials())
}

/// Credentials of the bot's owner on the bot's workspace; for owner-run maintenance, not turns.
pub(crate) async fn owner_credentials(
    state: &AppState,
    owner: &str,
    workspace: &str,
) -> Result<SharedCredentials> {
    let credentials = state
        .scoped_credentials(owner, workspace, CredentialsAccess::EditAppContent)
        .await?;
    Ok(credentials.into_shared_credentials())
}

pub(crate) struct ApiAccess {
    state: AppState,
    who: Who,
    workspace: String,
    turn_conversation: Option<String>,
}

/// A bot reads the files of its turn's conversation only. `chat` names the turn's own
/// attachments and is served from the turn, not from a conversation.
pub(crate) fn turn_may_read_conversation(
    turn_conversation: Option<&str>,
    conversation_id: &str,
) -> bool {
    conversation_id == TURN_FILES || turn_conversation == Some(conversation_id)
}

enum Who {
    Owner(String),
    Bot(BotUser),
}

impl ApiAccess {
    pub(crate) fn new(state: AppState, owner: &str, workspace: &str) -> Self {
        Self {
            state,
            who: Who::Owner(owner.to_string()),
            workspace: workspace.to_string(),
            turn_conversation: None,
        }
    }

    pub(crate) fn for_bot(
        state: AppState,
        bot: BotUser,
        workspace: &str,
        turn_conversation: Option<&str>,
    ) -> Self {
        Self {
            state,
            who: Who::Bot(bot),
            workspace: workspace.to_string(),
            turn_conversation: turn_conversation.map(str::to_string),
        }
    }

    /// The content store for a conversation's files, for a principal that is an active member of
    /// the conversation. Room limits that narrow the bot's apps do not apply: the person shared
    /// the file in this room.
    async fn conversation_store(&self, conversation_id: &str) -> Result<FlowLikeStore> {
        if matches!(self.who, Who::Bot(_))
            && !turn_may_read_conversation(self.turn_conversation.as_deref(), conversation_id)
        {
            return Err(flow_like_types::anyhow!(
                "Conversation '{conversation_id}' is not this turn's conversation; a bot reads the files of the conversation it answers in"
            ));
        }
        let principal = match &self.who {
            Who::Owner(owner) => owner.as_str(),
            Who::Bot(bot) => bot.bot_id.as_str(),
        };
        crate::routes::inbox::files::ensure_member(&self.state.db, conversation_id, principal)
            .await
            .map_err(|error| {
                flow_like_types::anyhow!(
                    "'{principal}' is not an active member of conversation '{conversation_id}': {error}"
                )
            })?;
        let master = self.state.master_credentials().await?;
        master.to_store(false).await
    }

    async fn credentials(&self, app_id: &str) -> Result<SharedCredentials> {
        match &self.who {
            Who::Owner(owner) => owner_credentials(&self.state, owner, app_id).await,
            Who::Bot(bot) => shared_credentials(&self.state, bot, &self.workspace, app_id).await,
        }
    }
}

#[async_trait]
impl AppAccess for ApiAccess {
    async fn store(&self, app_id: &str) -> Result<FlowLikeStore> {
        if let Some(conversation_id) = crate::routes::inbox::files::conversation_of_app(app_id) {
            return self.conversation_store(conversation_id).await;
        }
        self.credentials(app_id)
            .await?
            .to_store_type(StoreType::Content)
            .await
    }

    async fn connection(&self, app_id: &str) -> Result<Connection> {
        let builder = self.credentials(app_id).await?.to_db(app_id).await?;
        Ok(builder.execute().await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permission::role_permission::RolePermissions;
    use flow_like_types::bot_restrictions::BotScopes;

    const WS: &str = "ws";
    const DOCS: &str = "docs";

    fn holding(permissions: RolePermissions) -> EffectivePermissions {
        EffectivePermissions {
            role_mask: Some(permissions.bits()),
            ..EffectivePermissions::default()
        }
    }

    fn files_rw() -> RolePermissions {
        RolePermissions::ReadFiles | RolePermissions::WriteFiles
    }

    fn is_write(mode: &CredentialsAccess) -> bool {
        matches!(
            mode,
            CredentialsAccess::EditAppContent | CredentialsAccess::EditAppDb
        )
    }

    #[test]
    fn a_turn_reads_its_own_conversation_and_the_turn_files_only() {
        assert!(turn_may_read_conversation(Some("room-1"), "room-1"));
        assert!(turn_may_read_conversation(Some("room-1"), TURN_FILES));
        assert!(turn_may_read_conversation(None, TURN_FILES));
        assert!(!turn_may_read_conversation(Some("room-1"), "room-2"));
        assert!(!turn_may_read_conversation(None, "room-1"));
    }

    #[test]
    fn a_mask_without_write_files_mints_read_only_credentials() {
        let mode = credentials_mode(&holding(RolePermissions::ReadFiles), WS, WS).unwrap();
        assert!(matches!(mode, CredentialsAccess::ReadAppContent));
        let mode = credentials_mode(&holding(files_rw()), WS, WS).unwrap();
        assert!(is_write(&mode));
    }

    #[test]
    fn a_read_only_file_scope_blocks_writes_even_with_write_files() {
        let mut effective = holding(files_rw());
        effective.scopes = BotScopes::default();
        effective.scopes.files.read_only = true;
        let mode = credentials_mode(&effective, WS, WS).unwrap();
        assert!(!is_write(&mode));
    }

    #[test]
    fn other_apps_are_never_writable() {
        let mode = credentials_mode(&holding(files_rw()), WS, DOCS).unwrap();
        assert!(!is_write(&mode));
    }

    #[test]
    fn database_only_bots_get_the_project_database_prefix() {
        let mode = credentials_mode(&holding(RolePermissions::ReadDatabase), WS, WS).unwrap();
        assert!(matches!(mode, CredentialsAccess::ReadAppDb));
        let both = RolePermissions::ReadDatabase | RolePermissions::WriteDatabase;
        let mode = credentials_mode(&holding(both), WS, WS).unwrap();
        assert!(matches!(mode, CredentialsAccess::EditAppDb));
    }

    #[test]
    fn a_bot_with_no_file_or_database_right_gets_no_credentials() {
        assert!(credentials_mode(&holding(RolePermissions::ReadBoards), WS, WS).is_none());
    }
}
