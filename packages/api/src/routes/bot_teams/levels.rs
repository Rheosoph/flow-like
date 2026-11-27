//! Where people stand in an org: their classification, and which org a team conversation (a
//! team room, a person's thread with a team, or a person's chat with a bot) belongs to.

use crate::{
    bot_host::teams::{active_team_of, root_of},
    entity::{bot_org_classification, bot_team, bot_team_chat, bot_team_thread},
};
use flow_like_types::{bot_team::UNCLASSIFIED_PERSON, data_classification::DataClassification};
use sea_orm::{ColumnTrait, ConnectionTrait, DbErr, EntityTrait, QueryFilter};
use std::collections::HashMap;

/// The classification of each of `user_ids` in the org under `org_id`; a person without one
/// counts as [`UNCLASSIFIED_PERSON`].
pub(crate) async fn person_levels<C: ConnectionTrait>(
    db: &C,
    org_id: &str,
    user_ids: &[String],
) -> Result<HashMap<String, DataClassification>, DbErr> {
    let mut levels: HashMap<String, DataClassification> = user_ids
        .iter()
        .map(|id| (id.clone(), UNCLASSIFIED_PERSON))
        .collect();
    if user_ids.is_empty() {
        return Ok(levels);
    }
    let rows = bot_org_classification::Entity::find()
        .filter(bot_org_classification::Column::OrgId.eq(org_id))
        .filter(bot_org_classification::Column::UserId.is_in(user_ids.iter().cloned()))
        .all(db)
        .await?;
    for row in rows {
        if let Some(level) = DataClassification::parse(&row.classification) {
            levels.insert(row.user_id, level);
        }
    }
    Ok(levels)
}

pub(crate) async fn person_level<C: ConnectionTrait>(
    db: &C,
    org_id: &str,
    user_id: &str,
) -> Result<DataClassification, DbErr> {
    Ok(person_levels(db, org_id, &[user_id.to_string()])
        .await?
        .remove(user_id)
        .unwrap_or(UNCLASSIFIED_PERSON))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TeamConversationKind {
    /// A team's room: its active people and bots.
    Room,
    /// A person's thread with a team's lead.
    Thread,
    /// A person's chat with another owner's bot.
    Chat,
}

/// A conversation that belongs to an org's teams.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TeamConversation {
    pub kind: TeamConversationKind,
    /// The org's top-level team.
    pub org_id: String,
    /// The team of a room or a thread; `None` for a chat.
    pub team_id: Option<String>,
}

/// The team conversation `conversation_id` is and its org; `None` for any other conversation,
/// and for a chat whose bot is active in no team.
pub(crate) async fn team_conversation<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
) -> Result<Option<TeamConversation>, DbErr> {
    let room = bot_team::Entity::find()
        .filter(bot_team::Column::ConversationId.eq(conversation_id))
        .one(db)
        .await?;
    if let Some(team) = room {
        return Ok(Some(TeamConversation {
            kind: TeamConversationKind::Room,
            org_id: root_of(db, &team).await?.id,
            team_id: Some(team.id),
        }));
    }
    let thread = bot_team_thread::Entity::find()
        .filter(bot_team_thread::Column::ConversationId.eq(conversation_id))
        .one(db)
        .await?;
    if let Some(thread) = thread {
        let Some(team) = bot_team::Entity::find_by_id(thread.team_id).one(db).await? else {
            return Ok(None);
        };
        return Ok(Some(TeamConversation {
            kind: TeamConversationKind::Thread,
            org_id: root_of(db, &team).await?.id,
            team_id: Some(team.id),
        }));
    }
    let chat = bot_team_chat::Entity::find()
        .filter(bot_team_chat::Column::ConversationId.eq(conversation_id))
        .one(db)
        .await?;
    let Some(chat) = chat else {
        return Ok(None);
    };
    let Some(team) = active_team_of(db, &chat.bot_id).await? else {
        return Ok(None);
    };
    Ok(Some(TeamConversation {
        kind: TeamConversationKind::Chat,
        org_id: root_of(db, &team).await?.id,
        team_id: None,
    }))
}
