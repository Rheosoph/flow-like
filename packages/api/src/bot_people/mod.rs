//! `list_people` and `message_person` on the server: a bot writes to a person its owner already
//! chats with, in its room with that person (`routes::inbox::outreach`), and the person gets one
//! push titled with the bot and its owner. Desktop hosts reach the same code through
//! `routes::bots::people`.

use crate::{
    bot_notify::{NoticeSink, StateSink, display_name},
    entity::{bot, conversation_member, inbox_entry, sea_orm_active_enums::NotificationType},
    push_notifications::DispatchNotificationInput,
    routes::inbox::{
        MEMBER_ACTIVE,
        exchange::{message_outgoing, post_as_bot},
        outreach::{contacts_in, is_contact_in, member_in, open_outreach, person_names},
        person_push::{chat_link, group_key, is_muted},
    },
    state::AppState,
};
use async_trait::async_trait;
use flow_like_bot_tools::{Contact, Delivery, PeopleBackend, PersonMessage, people::MAX_CONTACTS};
use flow_like_types::{Result, anyhow, bot_notify::preview, create_id};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter};
use std::collections::HashSet;

#[cfg(test)]
mod tests;

pub(crate) const PERSON_MESSAGE_PREFIX: &str = "bp-";

/// Where a bot's message to a person is written: the server's rooms, or a test database.
#[async_trait]
pub(crate) trait Rooms: Send + Sync {
    /// The bot's room with the person, started when missing.
    async fn open(
        &self,
        owner_id: &str,
        bot_id: &str,
        person_id: &str,
        title: String,
    ) -> Result<String>;
    async fn post(
        &self,
        conversation_id: &str,
        bot_id: &str,
        message_id: &str,
        text: &str,
    ) -> Result<()>;
}

pub(crate) struct StateRooms<'a>(pub(crate) &'a AppState);

#[async_trait]
impl Rooms for StateRooms<'_> {
    async fn open(
        &self,
        owner_id: &str,
        bot_id: &str,
        person_id: &str,
        title: String,
    ) -> Result<String> {
        open_outreach(self.0, owner_id, bot_id, person_id, title)
            .await
            .map_err(|error| {
                anyhow!("Opening the room of bot '{bot_id}' with '{person_id}' failed: {error}")
            })
    }

    async fn post(
        &self,
        conversation_id: &str,
        bot_id: &str,
        message_id: &str,
        text: &str,
    ) -> Result<()> {
        let outgoing = message_outgoing(message_id, serde_json::json!({ "text": text, "hops": 0 }))
            .map_err(|error| anyhow!("Writing to room '{conversation_id}' failed: {error}"))?;
        post_as_bot(self.0, conversation_id, bot_id, 0, outgoing)
            .await
            .map(|_| ())
            .map_err(|error| anyhow!("Writing to room '{conversation_id}' failed: {error}"))
    }
}

async fn live_bot<C: ConnectionTrait>(db: &C, bot_id: &str) -> Result<bot::Model> {
    bot::Entity::find_by_id(bot_id)
        .filter(bot::Column::DeletedAt.is_null())
        .one(db)
        .await?
        .ok_or_else(|| anyhow!("Bot '{bot_id}' no longer exists"))
}

/// The people `owner_id` chats with directly whose name contains `query` (ignoring case), newest
/// activity first, at most [`MAX_CONTACTS`].
pub(crate) async fn contacts<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    query: Option<&str>,
) -> Result<Vec<Contact>> {
    let wanted = query
        .map(str::trim)
        .filter(|query| !query.is_empty())
        .map(str::to_lowercase);
    let all = contacts_in(db, owner_id)
        .await
        .map_err(|error| anyhow!("Listing the people of '{owner_id}' failed: {error}"))?;
    Ok(all
        .into_iter()
        .filter(|contact| {
            wanted
                .as_deref()
                .is_none_or(|wanted| contact.name.to_lowercase().contains(wanted))
        })
        .take(MAX_CONTACTS)
        .collect())
}

/// Distinct messages the bot wrote to people since `since_ms`.
pub(crate) async fn count_person_messages<C: ConnectionTrait>(
    db: &C,
    bot_id: &str,
    since_ms: i64,
) -> Result<usize> {
    let since = chrono::DateTime::from_timestamp_millis(since_ms)
        .ok_or_else(|| anyhow!("Bad window start {since_ms}"))?
        .fixed_offset();
    let rows = inbox_entry::Entity::find()
        .filter(inbox_entry::Column::SenderId.eq(bot_id))
        .filter(inbox_entry::Column::MessageId.starts_with(PERSON_MESSAGE_PREFIX))
        .filter(inbox_entry::Column::CreatedAt.gte(since))
        .all(db)
        .await?;
    let distinct: HashSet<&str> = rows.iter().map(|row| row.message_id.as_str()).collect();
    Ok(distinct.len())
}

fn push_id(person_id: &str, message_id: &str) -> String {
    let digest = blake3::hash(format!("{person_id}\0{message_id}").as_bytes()).to_hex();
    format!("pb-{}", &digest[..32])
}

/// The push of one bot message to the person, grouped under the room and opening it.
fn person_push(
    person_id: &str,
    conversation_id: &str,
    message_id: &str,
    title: String,
    text: &str,
) -> DispatchNotificationInput {
    DispatchNotificationInput {
        notification_priority: Default::default(),
        bot_context: None,
        user_id: person_id.to_string(),
        app_id: None,
        title,
        description: Some(preview(text)),
        icon: None,
        image: None,
        link: Some(chat_link(conversation_id)),
        notification_type: NotificationType::System,
        source_run_id: Some(group_key(conversation_id)),
        source_node_id: Some(message_id.to_string()),
    }
}

/// Sends the push unless the person muted the room; a failed push leaves the message standing.
async fn pushed(
    sink: &dyn NoticeSink,
    member: &conversation_member::Model,
    input: DispatchNotificationInput,
) -> bool {
    if is_muted(member.limits.as_ref()) {
        return false;
    }
    let id = push_id(
        &input.user_id,
        input.source_node_id.as_deref().unwrap_or_default(),
    );
    match sink.send(&id, input).await {
        Ok(()) => true,
        Err(error) => {
            tracing::warn!(%error, "Pushing a bot message to a person failed");
            false
        }
    }
}

/// The person's name and the owner's, as the room title and the push show them.
async fn names_of<C: ConnectionTrait>(
    db: &C,
    person_id: &str,
    owner_id: &str,
) -> Result<(String, String)> {
    let mut names = person_names(db, &[person_id.to_string(), owner_id.to_string()])
        .await
        .map_err(|error| {
            anyhow!("Reading the names of '{person_id}' and '{owner_id}' failed: {error}")
        })?;
    Ok((
        names.remove(person_id).unwrap_or_default(),
        names.remove(owner_id).unwrap_or_default(),
    ))
}

/// The person's row in the room while they are in it.
async fn present_member<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    person_id: &str,
) -> Result<Option<conversation_member::Model>> {
    let member = member_in(db, conversation_id, person_id)
        .await
        .map_err(|error| {
            anyhow!("Reading the members of room '{conversation_id}' failed: {error}")
        })?;
    Ok(member.filter(|member| member.left_at.is_none() && member.status == MEMBER_ACTIVE))
}

/// Writes the bot's message into its room with the person, after checking that the owner still
/// chats with them and that they haven't left the room, then pushes it to them unless they muted
/// the room.
pub(crate) async fn deliver_person_message<C: ConnectionTrait>(
    db: &C,
    rooms: &dyn Rooms,
    sink: &dyn NoticeSink,
    bot_id: &str,
    person_id: &str,
    text: &str,
) -> Result<Delivery> {
    let bot = live_bot(db, bot_id).await?;
    let contact = is_contact_in(db, &bot.owner_id, person_id)
        .await
        .map_err(|error| anyhow!("Checking '{person_id}' as a contact failed: {error}"))?;
    if !contact {
        return Ok(Delivery::NotAContact);
    }
    let (name, owner) = names_of(db, person_id, &bot.owner_id).await?;
    let bot_name = display_name(&bot);
    let title = format!("{bot_name} ↔ {name}");
    let conversation_id = rooms.open(&bot.owner_id, bot_id, person_id, title).await?;
    let Some(member) = present_member(db, &conversation_id, person_id).await? else {
        return Ok(Delivery::Left { name });
    };
    let message_id = format!("{PERSON_MESSAGE_PREFIX}{}", create_id());
    rooms
        .post(&conversation_id, bot_id, &message_id, text)
        .await?;
    let notified = pushed(
        sink,
        &member,
        person_push(
            person_id,
            &conversation_id,
            &message_id,
            format!("{bot_name} ({owner}'s bot)"),
            text,
        ),
    )
    .await;
    Ok(Delivery::Sent(PersonMessage {
        conversation_id,
        message_id,
        name,
        notified,
    }))
}

/// `list_people` and `message_person` for one bot on the server.
pub(crate) struct ServerPeople {
    state: AppState,
    bot_id: String,
    owner_id: String,
}

impl ServerPeople {
    pub(crate) fn new(state: &AppState, bot_id: &str, owner_id: &str) -> Self {
        Self {
            state: state.clone(),
            bot_id: bot_id.to_string(),
            owner_id: owner_id.to_string(),
        }
    }
}

#[async_trait]
impl PeopleBackend for ServerPeople {
    async fn contacts(&self, query: Option<&str>) -> Result<Vec<Contact>> {
        contacts(&self.state.db, &self.owner_id, query).await
    }

    async fn sent_since(&self, since_ms: i64) -> Result<usize> {
        count_person_messages(&self.state.db, &self.bot_id, since_ms).await
    }

    async fn message(&self, person_id: &str, text: &str) -> Result<Delivery> {
        deliver_person_message(
            &self.state.db,
            &StateRooms(&self.state),
            &StateSink(&self.state),
            &self.bot_id,
            person_id,
            text,
        )
        .await
    }
}
