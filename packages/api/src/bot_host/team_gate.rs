//! The classification gate of team conversations (a team room, a person's thread with a team, a
//! person's chat with a bot): a bot takes no turn while any active person other than its owner
//! is classified below it, and a chat takes none once it is over. A refused bot says so once per
//! conversation.

use crate::{
    bot_notify::display_name,
    entity::{bot, bot_team, bot_team_chat},
    error::ApiError,
    routes::{
        bot_teams::{
            access::{is_active_in, org_of},
            levels::{TeamConversationKind, person_levels, team_conversation},
        },
        inbox::{active_members, approvals::post_notice_in, delivery::BotRecipient},
    },
};
use chrono::{DateTime, FixedOffset};
use flow_like_types::{
    bot_team::{BotChatStatus, person_may_talk},
    bot_turn::classification_refused_message_id,
    data_classification::DataClassification,
    inbox::{ConversationKind, PrincipalKind},
};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter};
use std::collections::HashMap;

/// The level a bot of `owner_id` is refused at: it is classified above a person other than its
/// owner. `None` when it may answer everyone here.
pub(crate) fn refused_level(
    bot_level: Option<DataClassification>,
    owner_id: &str,
    people: &[(String, DataClassification)],
) -> Option<DataClassification> {
    people
        .iter()
        .any(|(person, level)| person != owner_id && !person_may_talk(Some(*level), bot_level))
        .then(|| bot_level.unwrap_or(flow_like_types::bot_team::UNCLASSIFIED_PERSON))
}

/// The message id of a bot's gate notice: one per conversation and bot.
pub(crate) fn gate_notice_message_id(bot_id: &str, conversation_id: &str) -> String {
    classification_refused_message_id(bot_id, &format!("team-gate:{conversation_id}"))
}

async fn chat_is_over<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    org_id: &str,
) -> Result<bool, ApiError> {
    let Some(chat) = bot_team_chat::Entity::find()
        .filter(bot_team_chat::Column::ConversationId.eq(conversation_id))
        .one(db)
        .await?
    else {
        return Ok(true);
    };
    if BotChatStatus::parse(&chat.status) != Some(BotChatStatus::Active) {
        return Ok(true);
    }
    let Some(root) = bot_team::Entity::find_by_id(org_id).one(db).await? else {
        return Ok(true);
    };
    let org = org_of(db, &root).await?;
    Ok(!is_active_in(db, &org.ids(), &chat.user_id).await?)
}

/// The recipients that may take a turn in their conversation. A group that is no team
/// conversation passes whole, and so does a direct one without a query.
pub(crate) async fn admit<'a, C: ConnectionTrait>(
    db: &C,
    conversation_kind: ConversationKind,
    waking: Vec<&'a BotRecipient>,
    now: DateTime<FixedOffset>,
) -> Result<Vec<&'a BotRecipient>, ApiError> {
    let Some(conversation_id) = waking.first().map(|item| item.conversation_id.clone()) else {
        return Ok(waking);
    };
    if conversation_kind != ConversationKind::Group {
        return Ok(waking);
    }
    let Some(team_conversation) = team_conversation(db, &conversation_id).await? else {
        return Ok(waking);
    };
    let people_ids: Vec<String> = active_members(db, std::slice::from_ref(&conversation_id))
        .await?
        .into_iter()
        .filter(|member| PrincipalKind::parse(&member.principal_kind) == Some(PrincipalKind::User))
        .map(|member| member.principal_id)
        .collect();
    let levels = person_levels(db, &team_conversation.org_id, &people_ids).await?;
    let people: Vec<(String, DataClassification)> = levels.into_iter().collect();
    let over = team_conversation.kind == TeamConversationKind::Chat
        && chat_is_over(db, &conversation_id, &team_conversation.org_id).await?;
    let bots: HashMap<String, bot::Model> = bot::Entity::find()
        .filter(bot::Column::Id.is_in(waking.iter().map(|item| item.bot_id.clone())))
        .all(db)
        .await?
        .into_iter()
        .map(|row| (row.id.clone(), row))
        .collect();
    let mut admitted = Vec::with_capacity(waking.len());
    for item in waking {
        if over {
            continue;
        }
        let Some(row) = bots.get(&item.bot_id) else {
            admitted.push(item);
            continue;
        };
        let level = row
            .classification
            .as_deref()
            .and_then(DataClassification::parse);
        let Some(refused) = refused_level(level, &item.owner_id, &people) else {
            admitted.push(item);
            continue;
        };
        tracing::info!(
            bot_id = item.bot_id,
            conversation_id,
            "A bot took no turn: a person in this team conversation is classified below it"
        );
        let text = format!(
            "{} doesn't answer here: someone in this chat isn't classified for {}.",
            display_name(row),
            refused.as_str()
        );
        Box::pin(post_notice_in(
            db,
            &conversation_id,
            &item.bot_id,
            &gate_notice_message_id(&item.bot_id, &conversation_id),
            &text,
            now,
        ))
        .await?;
    }
    Ok(admitted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use DataClassification::*;

    fn people(levels: &[(&str, DataClassification)]) -> Vec<(String, DataClassification)> {
        levels
            .iter()
            .map(|(id, level)| (id.to_string(), *level))
            .collect()
    }

    #[test]
    fn a_bot_is_refused_while_someone_other_than_its_owner_ranks_below_it() {
        let room = people(&[("owner", Public), ("anna", Internal)]);
        assert_eq!(refused_level(Some(Confidential), "owner", &room), Some(Confidential));
        assert_eq!(refused_level(Some(Internal), "owner", &room), None);
        assert_eq!(refused_level(None, "owner", &room), None);
    }

    #[test]
    fn the_owner_alone_never_gates_their_bot() {
        let room = people(&[("owner", Public)]);
        assert_eq!(refused_level(Some(Restricted), "owner", &room), None);
    }

    #[test]
    fn an_unclassified_bot_counts_as_internal_and_a_public_person_gates_it() {
        let room = people(&[("owner", Internal), ("guest", Public)]);
        assert_eq!(refused_level(None, "owner", &room), Some(Internal));
    }
}
