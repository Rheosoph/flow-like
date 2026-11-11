//! A flow run triggered by a bot runs as the bot. The owner stays the payer (the dispatch
//! `user_id`, quota), the credentials are minted for the bot id at the narrowed mode, and the
//! executor token carries the bot so every callback resolves through the bot's live grant.

use crate::{
    bot_permission::{BotGrant, bot_grant_in},
    credentials::CredentialsAccess,
    error::ApiError,
    middleware::jwt::{AppUser, BotUser},
    permission::role_permission::RolePermissions,
    state::AppState,
};
use flow_like::flow::copilot::EffectivePermissions;
use flow_like_types::bot_restrictions::{BotPermission, BotScopes};
use sea_orm::ConnectionTrait;

const BOT_ID_PREFIX: &str = "bot_";

pub(crate) struct BotFlowRun {
    bot: BotUser,
    grant: BotGrant,
}

/// The bot an authenticated caller stands for: the token's own bot, or the bot a connection
/// token was minted for by a bot-run flow (its `technical_user_id`).
fn bot_behind(user: &AppUser) -> Option<BotUser> {
    match user {
        AppUser::Bot(bot) => Some(bot.clone()),
        AppUser::ConnectedApp(connected) => connected.bot.clone().or_else(|| {
            connected
                .technical_user_id
                .as_deref()
                .and_then(bot_from_technical_id)
                .map(|bot| BotUser {
                    turn_id: connected.run_id.clone().unwrap_or_default(),
                    ..bot
                })
        }),
        _ => None,
    }
}

pub(crate) fn bot_from_technical_id(technical_user_id: &str) -> Option<BotUser> {
    technical_user_id
        .starts_with(BOT_ID_PREFIX)
        .then(|| BotUser {
            bot_id: technical_user_id.to_owned(),
            turn_id: String::new(),
            conv_id: None,
        })
}

pub(crate) fn event_allowed(scopes: &BotScopes, event_id: &str) -> bool {
    scopes
        .events
        .as_ref()
        .is_none_or(|allowed| allowed.iter().any(|id| id == event_id))
}

pub(crate) fn board_allowed(scopes: &BotScopes, board_id: &str) -> bool {
    scopes
        .boards
        .as_ref()
        .is_none_or(|allowed| allowed.iter().any(|id| id == board_id))
}

/// Write access to the app's content only when the bot may write files and tables everywhere;
/// a prefix or table scope cannot be enforced inside a bucket policy, so it closes writes.
/// Everything else runs on replay credentials (reads kept, content writes dropped).
pub(crate) fn credentials_access_for(effective: &EffectivePermissions) -> CredentialsAccess {
    let scopes = &effective.scopes;
    let writes_everywhere = effective.holds(BotPermission::WriteFiles)
        && effective.holds(BotPermission::WriteDatabase)
        && !scopes.files.read_only
        && scopes.files.prefixes.is_empty()
        && scopes.tables.is_none();
    if writes_everywhere {
        CredentialsAccess::ServerExecute
    } else {
        CredentialsAccess::ShadowExecute
    }
}

impl BotFlowRun {
    pub(crate) fn new(bot: BotUser, grant: BotGrant) -> Self {
        Self { bot, grant }
    }

    pub(crate) fn bot_id(&self) -> &str {
        &self.bot.bot_id
    }

    pub(crate) fn user(&self) -> AppUser {
        AppUser::Bot(self.bot.clone())
    }

    pub(crate) fn owner_id(&self) -> &str {
        &self.grant.owner_id
    }

    pub(crate) fn credentials_access(&self) -> CredentialsAccess {
        credentials_access_for(&self.grant.effective)
    }

    pub(crate) fn ensure_event(&self, event_id: &str) -> Result<(), ApiError> {
        if event_allowed(&self.grant.effective.scopes, event_id) {
            return Ok(());
        }
        Err(ApiError::forbidden(
            "This bot is not allowed to run this event",
        ))
    }

    pub(crate) fn ensure_board(&self, board_id: &str) -> Result<(), ApiError> {
        if board_allowed(&self.grant.effective.scopes, board_id) {
            return Ok(());
        }
        Err(ApiError::forbidden(
            "This bot is not allowed to run this board",
        ))
    }

    fn ensure_holds(&self, required: RolePermissions) -> Result<(), ApiError> {
        if self.grant.permissions.contains(required) {
            return Ok(());
        }
        Err(ApiError::forbidden(
            "The bot does not hold this permission in the app",
        ))
    }

    /// `None` when the caller is not a bot. A bot's grant is re-read here so the allow-lists
    /// and the credentials mode come from the same live row the funnel just admitted.
    pub(crate) async fn resolve_in<C: ConnectionTrait>(
        db: &C,
        user: &AppUser,
        app_id: &str,
        required: RolePermissions,
    ) -> Result<Option<Self>, ApiError> {
        let Some(bot) = bot_behind(user) else {
            return Ok(None);
        };
        let grant = bot_grant_in(db, &bot, app_id).await?;
        let run = Self::new(bot, grant);
        run.ensure_holds(required)?;
        Ok(Some(run))
    }

    pub(crate) async fn admit_event_in<C: ConnectionTrait>(
        db: &C,
        user: &AppUser,
        app_id: &str,
        event_id: &str,
        page_trigger: bool,
    ) -> Result<Option<Self>, ApiError> {
        if page_trigger && bot_behind(user).is_some() {
            return Err(ApiError::forbidden("A bot cannot trigger a Page action"));
        }
        let run = Self::resolve_in(db, user, app_id, RolePermissions::ExecuteEvents).await?;
        if let Some(run) = &run {
            run.ensure_event(event_id)?;
        }
        Ok(run)
    }

    pub(crate) async fn admit_event(
        user: &AppUser,
        app_id: &str,
        state: &AppState,
        event_id: &str,
        page_trigger: bool,
    ) -> Result<Option<Self>, ApiError> {
        Self::admit_event_in(&state.db, user, app_id, event_id, page_trigger).await
    }

    pub(crate) async fn admit_board_in<C: ConnectionTrait>(
        db: &C,
        user: &AppUser,
        app_id: &str,
        board_id: &str,
    ) -> Result<Option<Self>, ApiError> {
        let run = Self::resolve_in(db, user, app_id, RolePermissions::ExecuteBoards).await?;
        if let Some(run) = &run {
            run.ensure_board(board_id)?;
        }
        Ok(run)
    }

    pub(crate) async fn admit_board(
        user: &AppUser,
        app_id: &str,
        state: &AppState,
        board_id: &str,
    ) -> Result<Option<Self>, ApiError> {
        Self::admit_board_in(&state.db, user, app_id, board_id).await
    }
}

/// Who the run's storage credentials are minted for and at which mode: the bot id at the
/// narrowed mode for a bot run, otherwise the caller's `sub` at `default`.
pub(crate) fn credential_target(
    bot: Option<&BotFlowRun>,
    sub: &str,
    default: CredentialsAccess,
) -> (String, CredentialsAccess) {
    match bot {
        Some(bot) => (bot.bot_id().to_owned(), bot.credentials_access()),
        None => (sub.to_owned(), default),
    }
}

/// Retains bot and group attribution through every hop. Chained executors keep the connection
/// permission path, which intersects the connection role with the bot's current grant.
pub(crate) fn bind_executor(bot: Option<&BotFlowRun>, token: String) -> Result<String, ApiError> {
    let Some(bot) = bot else {
        return Ok(token);
    };
    crate::execution::bind_bot_identity(&token, &bot.bot.bot_id, bot.bot.conv_id.as_deref())
        .map_err(|error| ApiError::internal_error(flow_like_types::anyhow!(error)))
}

#[cfg(test)]
#[path = "bot_flow_tests.rs"]
mod tests;
