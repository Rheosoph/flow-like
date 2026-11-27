//! `#Team` requests: a person's message that starts with `#Tag`s goes to those teams' lead bots
//! through the person's thread with each team, and the lead's answer to a request that came from
//! the person's own chat with their bot is copied back into that chat.
//!
//! The server reads the tags in [`fan_out`] ([`with_team_tags`]) and writes the resolved team ids
//! into the stored body; [`after_person_send`] routes them once the send committed.

use super::{
    active_members,
    activity::write_activity_in,
    entries::{Content, Outgoing, Sent, fan_out, repeat_of},
    exchange::message_outgoing,
    lock_conversation, now_millis, stored, wake,
};
use crate::{
    bot_host::teams::{org_teams, root_of},
    bot_notify::Reply,
    db::coordination::coordinate,
    entity::{
        bot, bot_team, bot_team_person, bot_team_thread, bot_turn, conversation, inbox_entry, user,
    },
    error::ApiError,
    routes::{
        bot_teams::{
            access::{active, is_active_in, org_of},
            effects::Effects,
            threads::{lead_seat, sync_thread, thread_of},
        },
        bots::lock_bot,
        inbox::{members::Principal, model::InboxEntry, team_room},
        user::identity::public_display_name,
    },
    state::AppState,
};
use chrono::{DateTime, FixedOffset};
use flow_like_types::{
    bot_message::MessageBody,
    bot_team::{TeamRequestRef, leading_team_tags, tag_names_team, team_tag},
    bot_turn::failure_message_id,
    create_id,
    inbox::{
        ActivityBody, ConversationEncryption, ConversationKind, InboxEntryKind, PrincipalKind,
    },
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseTransaction,
    EntityTrait, QueryFilter, QuerySelect,
};
use serde_json::Value;

const REQUEST_PREFIX: &str = "tr-";
const ANSWER_PREFIX: &str = "ta-";

fn digest(prefix: &str, parts: &[&str]) -> String {
    let hash = blake3::hash(parts.join("\0").as_bytes()).to_hex();
    format!("{prefix}{}", &hash[..32])
}

/// The id of the copy of the request in the team's thread: the same request is posted once.
pub(crate) fn request_message_id(
    origin_conversation_id: &str,
    origin_message_id: &str,
    team_id: &str,
) -> String {
    digest(
        REQUEST_PREFIX,
        &[origin_conversation_id, origin_message_id, team_id],
    )
}

/// The id of the activity an answer leaves in its origin chat.
pub(super) fn answer_message_id(reply_message_id: &str, origin_conversation_id: &str) -> String {
    digest(ANSWER_PREFIX, &[reply_message_id, origin_conversation_id])
}

/// The teams the leading `#Tag`s of `text` name, for a person: teams of the orgs they are an
/// active person in whose lead is an active, live bot. Of several teams with the same tag the
/// oldest wins. Each team once, in the order of the tags.
pub(crate) async fn resolve_teams<C: ConnectionTrait>(
    db: &C,
    user_id: &str,
    text: &str,
) -> Result<Vec<bot_team::Model>, ApiError> {
    let tags = leading_team_tags(text);
    if tags.is_empty() {
        return Ok(Vec::new());
    }
    let own = bot_team_person::Entity::find()
        .filter(bot_team_person::Column::UserId.eq(user_id))
        .filter(bot_team_person::Column::Status.eq(active()))
        .all(db)
        .await?;
    let own_teams = bot_team::Entity::find()
        .filter(bot_team::Column::Id.is_in(own.into_iter().map(|row| row.team_id)))
        .all(db)
        .await?;
    let mut candidates: Vec<bot_team::Model> = Vec::new();
    for team in &own_teams {
        let root = root_of(db, team).await?;
        if !candidates.iter().any(|known| known.id == root.id) {
            candidates.extend(org_teams(db, &root).await?);
        }
    }
    candidates.sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));
    candidates.dedup_by(|a, b| a.id == b.id);
    let mut found: Vec<bot_team::Model> = Vec::new();
    for tag in &tags {
        for team in candidates
            .iter()
            .filter(|team| tag_names_team(tag, &team.name))
        {
            if lead_seat(db, team).await?.is_some() {
                if !found.iter().any(|known| known.id == team.id) {
                    found.push(team.clone());
                }
                break;
            }
        }
    }
    Ok(found)
}

fn set_teams(envelope: &mut Value, teams: &[String]) {
    if let Some(body) = envelope.get_mut("body").and_then(Value::as_object_mut) {
        if teams.is_empty() {
            body.remove("teams");
        } else {
            body.insert("teams".to_owned(), Value::from(teams.to_vec()));
        }
    }
}

/// `outgoing` with `body.teams` set to the teams a person's leading `#Tag`s name, in `envelopes`
/// too, and the same ids in `links.teams`. A `teams` a client wrote is dropped. `None` when the
/// message names no team and carried none.
pub(super) async fn with_team_tags<C: ConnectionTrait>(
    db: &C,
    conversation: &conversation::Model,
    sender: &str,
    bots: &[bot::Model],
    outgoing: &Outgoing,
    envelopes: &mut [(String, Value)],
) -> Result<Option<Outgoing>, ApiError> {
    if !matches!(
        outgoing.kind,
        InboxEntryKind::Message | InboxEntryKind::Edit
    ) {
        return Ok(None);
    }
    let Content::Plain(envelope) = &outgoing.content else {
        return Ok(None);
    };
    let body = &envelope["body"];
    let carried = body.get("teams").is_some();
    let routable = outgoing.kind == InboxEntryKind::Message
        && bots.iter().all(|bot| bot.id != sender)
        && ConversationEncryption::parse(&conversation.encryption)
            != Some(ConversationEncryption::E2e)
        && body.get("team_request").is_none_or(Value::is_null);
    let teams: Vec<String> = match body["text"].as_str() {
        Some(text) if routable => resolve_teams(db, sender, text)
            .await?
            .into_iter()
            .map(|team| team.id)
            .collect(),
        _ => Vec::new(),
    };
    if teams.is_empty() && !carried {
        return Ok(None);
    }
    let mut tagged = outgoing.clone();
    tagged.links.teams = teams.clone();
    let stored = match &mut tagged.content {
        Content::Plain(envelope) => Some(envelope),
        Content::Sealed(_) => None,
    };
    for envelope in envelopes
        .iter_mut()
        .map(|(_, envelope)| envelope)
        .chain(stored)
    {
        set_teams(envelope, &teams);
    }
    Ok(Some(tagged))
}

/// Whether the lead's answer to a request written in `conversation_id` is copied back there: it
/// is the person's plain direct chat with a bot they own.
pub(crate) async fn answers_in_origin<C: ConnectionTrait>(
    db: &C,
    person_id: &str,
    conversation_id: &str,
) -> Result<bool, ApiError> {
    let Some(row) = conversation::Entity::find_by_id(conversation_id)
        .one(db)
        .await?
    else {
        return Ok(false);
    };
    if ConversationKind::parse(&row.kind) != Some(ConversationKind::Direct)
        || ConversationEncryption::parse(&row.encryption) == Some(ConversationEncryption::E2e)
    {
        return Ok(false);
    }
    let members: Vec<Principal> = active_members(db, &[conversation_id.to_string()])
        .await?
        .iter()
        .map(Principal::from)
        .collect();
    let [first, second] = members.as_slice() else {
        return Ok(false);
    };
    let bot_id = match (first, second) {
        (user, bot) | (bot, user)
            if user.id == person_id
                && user.kind == PrincipalKind::User
                && bot.kind == PrincipalKind::Bot =>
        {
            &bot.id
        }
        _ => return Ok(false),
    };
    Ok(bot::Entity::find_by_id(bot_id)
        .filter(bot::Column::OwnerId.eq(person_id))
        .filter(bot::Column::DeletedAt.is_null())
        .one(db)
        .await?
        .is_some())
}

async fn person_label<C: ConnectionTrait>(db: &C, user_id: &str) -> String {
    let row = user::Entity::find_by_id(user_id)
        .select_only()
        .column(user::Column::Name)
        .column(user::Column::PreferredUsername)
        .into_tuple::<(Option<String>, Option<String>)>()
        .one(db)
        .await
        .ok()
        .flatten();
    row.and_then(|(name, preferred)| {
        public_display_name(name.as_deref(), preferred.as_deref(), None)
    })
    .unwrap_or_else(|| "Someone".to_string())
}

/// The message a person sent that named teams.
#[derive(Debug, Clone)]
pub(crate) struct Origin {
    pub conversation_id: String,
    pub message_id: String,
    pub text: String,
}

async fn create_thread<C: ConnectionTrait>(
    db: &C,
    team: &bot_team::Model,
    person_id: &str,
    now: DateTime<FixedOffset>,
    effects: &mut Effects,
) -> Result<bot_team_thread::Model, ApiError> {
    let conversation_id = create_id();
    let title = format!(
        "{} · {}",
        team_tag(&team.name),
        person_label(db, person_id).await
    );
    effects.wake(team_room::create_room(db, &conversation_id, person_id, &title, now).await?);
    Ok(bot_team_thread::ActiveModel {
        team_id: Set(team.id.clone()),
        user_id: Set(person_id.to_string()),
        conversation_id: Set(conversation_id),
        created_at: Set(now),
    }
    .insert(db)
    .await?)
}

/// What routing one request wrote.
pub(crate) struct Routed {
    pub sent: Sent,
    pub effects: Effects,
}

/// Posts the person's request to the team's lead in their thread with the team, creating and
/// seating the thread first. `None` when the team can't take it any more: gone, no live lead, or
/// the person left its org. A repeat of the same request posts nothing new. The caller holds the
/// locks ([`route_request`] takes them).
pub(crate) async fn route_request_in<C: ConnectionTrait>(
    db: &C,
    person_id: &str,
    team_id: &str,
    origin: &Origin,
    now: DateTime<FixedOffset>,
) -> Result<Option<Routed>, ApiError> {
    let Some(team) = bot_team::Entity::find_by_id(team_id).one(db).await? else {
        return Ok(None);
    };
    let org = org_of(db, &team).await?;
    if !is_active_in(db, &org.ids(), person_id).await? {
        return Ok(None);
    }
    let Some(lead) = lead_seat(db, &team).await? else {
        return Ok(None);
    };
    let mut effects = Effects::default();
    let thread = match thread_of(db, team_id, person_id).await? {
        Some(thread) => thread,
        None => create_thread(db, &team, person_id, now, &mut effects).await?,
    };
    sync_thread(db, &team, &thread, person_id, now, &mut effects).await?;
    let request = TeamRequestRef {
        team_id: team.id.clone(),
        origin_conversation_id: origin.conversation_id.clone(),
        origin_message_id: origin.message_id.clone(),
        answer_here: answers_in_origin(db, person_id, &origin.conversation_id).await?,
    };
    let message_id = request_message_id(&origin.conversation_id, &origin.message_id, &team.id);
    let body = MessageBody {
        text: origin.text.clone(),
        mentions: vec![lead.bot.id.clone()],
        hops: Some(0),
        team_request: Some(request),
        ..MessageBody::default()
    };
    let body = serde_json::to_value(&body)
        .map_err(|error| ApiError::internal(format!("A team request was not encoded: {error}")))?;
    let outgoing = message_outgoing(&message_id, body)?;
    let sent = match repeat_of(db, &thread.conversation_id, person_id, &message_id).await? {
        Some(sent) => sent,
        None => fan_out(db, &thread.conversation_id, person_id, 0, &outgoing, now).await?,
    };
    Ok(Some(Routed { sent, effects }))
}

async fn lock_request(
    txn: &DatabaseTransaction,
    person_id: &str,
    team_id: &str,
) -> Result<(), ApiError> {
    let thread = thread_of(txn, team_id, person_id).await?;
    if thread.is_none() {
        coordinate(txn, "bot-team", &[team_id]).await?;
    }
    coordinate(txn, "bot-team-thread", &[team_id, person_id]).await?;
    let team = bot_team::Entity::find_by_id(team_id).one(txn).await?;
    if let Some(lead) = team.and_then(|team| team.lead_bot_id) {
        lock_bot(txn, &lead).await?;
    }
    if let Some(thread) = thread_of(txn, team_id, person_id).await? {
        team_room::lock_room(txn, &thread.conversation_id).await?;
    }
    Ok(())
}

async fn route_request(
    state: &AppState,
    person_id: &str,
    team_id: &str,
    origin: &Origin,
) -> Result<Option<Routed>, ApiError> {
    state
        .transaction(|txn| {
            let (person_id, team_id, origin) =
                (person_id.to_string(), team_id.to_string(), origin.clone());
            Box::pin(async move {
                lock_request(txn, &person_id, &team_id).await?;
                route_request_in(txn, &person_id, &team_id, &origin, now_millis()).await
            })
        })
        .await
}

/// Routes the teams a committed message of a person names. Failures are logged: the person's own
/// send stands.
pub(crate) async fn after_person_send(state: &AppState, entry: &InboxEntry) {
    if entry.kind != InboxEntryKind::Message.as_str() {
        return;
    }
    let body = &entry.envelope["body"];
    let teams: Vec<String> = body["teams"]
        .as_array()
        .map(|ids| {
            ids.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    if teams.is_empty() {
        return;
    }
    let origin = Origin {
        conversation_id: entry.conversation_id.clone(),
        message_id: entry.message_id.clone(),
        text: body["text"].as_str().unwrap_or_default().to_string(),
    };
    for team_id in &teams {
        match route_request(state, &entry.sender_id, team_id, &origin).await {
            Ok(Some(routed)) => {
                let mut people = routed.sent.wake;
                people.extend(routed.effects.wake);
                people.sort();
                people.dedup();
                wake(state, people).await;
                crate::bot_host::dispatch(state, &routed.sent.bots).await;
            }
            Ok(None) => {
                tracing::info!(%team_id, "A team request was not routed: the team has no live lead for this person any more");
            }
            Err(error) => {
                tracing::warn!(%error, %team_id, message_id = %entry.message_id, "A team request was not routed");
            }
        }
    }
}

/// A lead's answer to copy into the chat the request came from.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AnswerPlan {
    pub origin_conversation_id: String,
    pub person_id: String,
    pub message_id: String,
    pub body: ActivityBody,
}

async fn person_message<C: ConnectionTrait>(
    db: &C,
    person_id: &str,
    conversation_id: Option<&str>,
    message_id: &str,
) -> Result<Option<MessageBody>, ApiError> {
    let mut find = inbox_entry::Entity::find()
        .filter(inbox_entry::Column::InboxId.eq(person_id))
        .filter(inbox_entry::Column::SenderId.eq(person_id))
        .filter(inbox_entry::Column::MessageId.eq(message_id))
        .filter(inbox_entry::Column::Kind.eq(stored(InboxEntryKind::Message.as_str())));
    if let Some(conversation_id) = conversation_id {
        find = find.filter(inbox_entry::Column::ConversationId.eq(conversation_id));
    }
    Ok(find
        .one(db)
        .await?
        .and_then(|entry| MessageBody::parse(entry.envelope.get("body")?).ok()))
}

/// Where and what to write when `reply` answers a team request that asked for its answer in the
/// chat it came from. Everything is read from what the server wrote, never from the request's
/// own claims.
pub(crate) async fn plan_answer<C: ConnectionTrait>(
    db: &C,
    reply: &Reply<'_>,
) -> Result<Option<AnswerPlan>, ApiError> {
    if reply.text.trim().is_empty() {
        return Ok(None);
    }
    let Some(turn) = bot_turn::Entity::find_by_id(reply.turn_id).one(db).await? else {
        return Ok(None);
    };
    if turn.bot_id != reply.bot_id
        || reply.message_id == failure_message_id(&turn.bot_id, &turn.trigger_message_id)
    {
        return Ok(None);
    }
    let Some(thread) = bot_team_thread::Entity::find()
        .filter(bot_team_thread::Column::ConversationId.eq(&turn.conversation_id))
        .one(db)
        .await?
    else {
        return Ok(None);
    };
    let Some(request) = person_message(
        db,
        &thread.user_id,
        Some(&thread.conversation_id),
        &turn.trigger_message_id,
    )
    .await?
    .and_then(|body| body.team_request)
    .filter(|request| request.answer_here && request.team_id == thread.team_id) else {
        return Ok(None);
    };
    let origin = person_message(
        db,
        &thread.user_id,
        Some(&request.origin_conversation_id),
        &request.origin_message_id,
    )
    .await?;
    if !origin.is_some_and(|body| body.teams.contains(&thread.team_id))
        || !answers_in_origin(db, &thread.user_id, &request.origin_conversation_id).await?
    {
        return Ok(None);
    }
    Ok(Some(AnswerPlan {
        message_id: answer_message_id(reply.message_id, &request.origin_conversation_id),
        body: ActivityBody::team_answer(
            &thread.team_id,
            &turn.bot_id,
            &thread.conversation_id,
            &request.origin_message_id,
            reply.message_id,
            reply.text,
        ),
        origin_conversation_id: request.origin_conversation_id,
        person_id: thread.user_id,
    }))
}

/// Writes the answer into the origin chat as the thread's person; the caller holds the origin's
/// lock. A repeat writes nothing new.
pub(crate) async fn write_answer_in<C: ConnectionTrait>(
    db: &C,
    plan: &AnswerPlan,
    now: DateTime<FixedOffset>,
) -> Result<Sent, ApiError> {
    write_activity_in(
        db,
        &plan.origin_conversation_id,
        &plan.person_id,
        &plan.message_id,
        &plan.body,
        now,
    )
    .await
}

async fn write_answer(state: &AppState, plan: &AnswerPlan) -> Result<Sent, ApiError> {
    state
        .transaction(|txn| {
            let plan = plan.clone();
            Box::pin(async move {
                lock_conversation(txn, &plan.origin_conversation_id).await?;
                write_answer_in(txn, &plan, now_millis()).await
            })
        })
        .await
}

/// Copies a lead's answer into the chat its request came from, after the reply committed.
/// Failures are logged.
pub(crate) async fn after_reply(state: &AppState, reply: &Reply<'_>) {
    let result = async {
        match plan_answer(&state.db, reply).await? {
            Some(plan) => {
                let sent = write_answer(state, &plan).await?;
                wake(state, sent.wake).await;
                Ok::<(), ApiError>(())
            }
            None => Ok(()),
        }
    }
    .await;
    if let Err(error) = result {
        tracing::warn!(%error, bot_id = reply.bot_id, "A team answer was not copied into its chat");
    }
}
