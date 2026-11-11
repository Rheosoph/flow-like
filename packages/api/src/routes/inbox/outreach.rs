//! The room a bot shares with one person its owner chats with (`message_person`): a group of the
//! owner, the bot and the person, one per bot and person, found by its key. The person joins at
//! once: their accepted direct chat with the owner is their consent. The bot joins without any of
//! the owner's apps ([`OUTREACH_LIMITS`]); the owner can widen that in the room's limits.

use super::{
    active_member, allocate_seqs,
    conversations::{NewConversation, conversation_row, member_row, normalize_title},
    insert_copies,
    members::{Principal, join_copies},
    now_millis, stored,
};
use crate::{
    db::coordination::coordinate,
    entity::{conversation, conversation_member, user},
    error::ApiError,
    routes::user::identity::public_display_name,
    state::AppState,
};
use chrono::{DateTime, FixedOffset};
use flow_like_bot_tools::Contact;
use flow_like_types::{
    create_id,
    inbox::{ConversationEncryption, ConversationKind, PrincipalKind, direct_pair_key},
};
use sea_orm::{
    ActiveValue::Set, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect, sea_query::OnConflict,
};
use std::collections::HashMap;

/// Room limits of the bot in its room with a person: none of the owner's apps.
pub(crate) const OUTREACH_LIMITS: &str = r#"{"apps":[]}"#;

const UNNAMED: &str = "Unnamed person";

/// The key under which a bot has its room with a person.
pub(crate) fn outreach_key(bot_id: &str, person_id: &str) -> String {
    format!("outreach:{bot_id}:{person_id}")
}

fn outreach_limits() -> flow_like_types::Value {
    serde_json::from_str(OUTREACH_LIMITS).expect("valid room limits")
}

/// Display names of `ids`, read in one query; an id without a name maps to [`UNNAMED`].
pub(crate) async fn person_names<C: ConnectionTrait>(
    db: &C,
    ids: &[String],
) -> Result<HashMap<String, String>, ApiError> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<(String, Option<String>, Option<String>)> = user::Entity::find()
        .select_only()
        .column(user::Column::Id)
        .column(user::Column::Name)
        .column(user::Column::PreferredUsername)
        .filter(user::Column::Id.is_in(ids.iter().map(String::as_str)))
        .into_tuple()
        .all(db)
        .await?;
    let mut names: HashMap<String, String> = rows
        .into_iter()
        .map(|(id, name, preferred)| {
            let name = public_display_name(name.as_deref(), preferred.as_deref(), None)
                .unwrap_or_else(|| UNNAMED.to_string());
            (id, name)
        })
        .collect();
    for id in ids {
        names
            .entry(id.clone())
            .or_insert_with(|| UNNAMED.to_string());
    }
    Ok(names)
}

fn person_kind() -> String {
    stored(PrincipalKind::User.as_str())
}

/// The direct conversations `owner_id` is an active member of, newest activity first.
async fn directs_of<C: ConnectionTrait>(db: &C, owner_id: &str) -> Result<Vec<String>, ApiError> {
    let mine: Vec<String> = conversation_member::Entity::find()
        .select_only()
        .column(conversation_member::Column::ConversationId)
        .filter(conversation_member::Column::PrincipalId.eq(owner_id))
        .filter(active_member())
        .into_tuple()
        .all(db)
        .await?;
    if mine.is_empty() {
        return Ok(Vec::new());
    }
    Ok(conversation::Entity::find()
        .select_only()
        .column(conversation::Column::Id)
        .filter(conversation::Column::Id.is_in(mine))
        .filter(conversation::Column::Kind.eq(stored(ConversationKind::Direct.as_str())))
        .order_by_desc(conversation::Column::LastActivityAt)
        .into_tuple()
        .all(db)
        .await?)
}

/// The active person other than `owner_id` in each of `directs`, in their order.
async fn people_in<C: ConnectionTrait>(
    db: &C,
    directs: &[String],
    owner_id: &str,
) -> Result<Vec<String>, ApiError> {
    if directs.is_empty() {
        return Ok(Vec::new());
    }
    let others: Vec<(String, String)> = conversation_member::Entity::find()
        .select_only()
        .column(conversation_member::Column::ConversationId)
        .column(conversation_member::Column::PrincipalId)
        .filter(
            conversation_member::Column::ConversationId.is_in(directs.iter().map(String::as_str)),
        )
        .filter(conversation_member::Column::PrincipalId.ne(owner_id))
        .filter(conversation_member::Column::PrincipalKind.eq(person_kind()))
        .filter(active_member())
        .into_tuple()
        .all(db)
        .await?;
    let person_of: HashMap<&str, &str> = others
        .iter()
        .map(|(conversation_id, person)| (conversation_id.as_str(), person.as_str()))
        .collect();
    Ok(directs
        .iter()
        .filter_map(|id| person_of.get(id.as_str()).map(|person| person.to_string()))
        .collect())
}

/// The people `owner_id` has an accepted direct chat with (both members active), newest activity
/// first.
pub(crate) async fn contacts_in<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
) -> Result<Vec<Contact>, ApiError> {
    let people = people_in(db, &directs_of(db, owner_id).await?, owner_id).await?;
    let blocked = super::blocks::blocked_between(db, owner_id, &people, false).await?;
    let mut names = person_names(db, &people).await?;
    Ok(people
        .into_iter()
        .filter(|id| !blocked.contains(id))
        .map(|id| {
            let name = names.remove(&id).unwrap_or_else(|| UNNAMED.to_string());
            Contact { id, name }
        })
        .collect())
}

/// Whether `owner_id` and `person_id` share an accepted direct chat.
pub(crate) async fn is_contact_in<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    person_id: &str,
) -> Result<bool, ApiError> {
    if owner_id == person_id {
        return Ok(false);
    }
    if !super::blocks::blocked_between(db, owner_id, &[person_id.to_string()], false)
        .await?
        .is_empty()
    {
        return Ok(false);
    }
    let Some(direct) = conversation::Entity::find()
        .filter(conversation::Column::DirectKey.eq(direct_pair_key(owner_id, person_id)))
        .filter(conversation::Column::Kind.eq(stored(ConversationKind::Direct.as_str())))
        .one(db)
        .await?
    else {
        return Ok(false);
    };
    let active = conversation_member::Entity::find()
        .filter(conversation_member::Column::ConversationId.eq(direct.id.as_str()))
        .filter(conversation_member::Column::PrincipalId.is_in([owner_id, person_id]))
        .filter(conversation_member::Column::PrincipalKind.eq(person_kind()))
        .filter(active_member())
        .all(db)
        .await?;
    Ok(active.len() == 2)
}

/// Adds the owner, the bot (without the owner's apps) and the person to a new room and writes
/// the bot's and the person's join entries into every member's inbox.
async fn seat_members<C: ConnectionTrait>(
    db: &C,
    room_id: &str,
    owner_id: &str,
    bot_id: &str,
    person_id: &str,
    now: DateTime<FixedOffset>,
) -> Result<(), ApiError> {
    let principal = |id: &str, kind| Principal {
        id: id.to_string(),
        kind,
    };
    let owner = principal(owner_id, PrincipalKind::User);
    let joined = [
        principal(bot_id, PrincipalKind::Bot),
        principal(person_id, PrincipalKind::User),
    ];
    let mut bot_row = member_row(room_id, &joined[0], Some(owner_id), now);
    bot_row.limits = Set(Some(outreach_limits()));
    conversation_member::Entity::insert_many([
        member_row(room_id, &owner, None, now),
        bot_row,
        member_row(room_id, &joined[1], Some(owner_id), now),
    ])
    .exec_without_returning(db)
    .await?;
    let mut everyone = vec![owner];
    everyone.extend(joined.iter().cloned());
    let first_seq = allocate_seqs(db, room_id, joined.len()).await?;
    insert_copies(
        db,
        join_copies(room_id, &joined, owner_id, &everyone, now, first_seq),
    )
    .await
}

/// The bot's room with the person, started when missing; concurrent starts keep one. Returns the
/// room's id either way.
pub(crate) async fn start_outreach_in<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
    person_id: &str,
    title: String,
    now: DateTime<FixedOffset>,
) -> Result<String, ApiError> {
    super::blocks::ensure_allowed(db, bot_id, person_id).await?;
    let key = outreach_key(bot_id, person_id);
    let plan = NewConversation {
        kind: ConversationKind::Group,
        title: normalize_title(Some(title))?,
        members: vec![bot_id.to_string(), person_id.to_string()],
        encryption: ConversationEncryption::Plain,
    };
    let id = create_id();
    let row = conversation_row(&id, owner_id, &plan, Some(key.clone()), now);
    let inserted = conversation::Entity::insert(row)
        .on_conflict(
            OnConflict::column(conversation::Column::DirectKey)
                .do_nothing()
                .to_owned(),
        )
        .exec_without_returning(db)
        .await?;
    if inserted == 1 {
        seat_members(db, &id, owner_id, bot_id, person_id, now).await?;
        return Ok(id);
    }
    conversation::Entity::find()
        .filter(conversation::Column::DirectKey.eq(key))
        .one(db)
        .await?
        .map(|row| row.id)
        .ok_or_else(|| {
            ApiError::internal(
                "A bot's room with a person vanished right after a conflicting insert",
            )
        })
}

/// [`start_outreach_in`] in its own transaction, serialized per bot and person.
pub(crate) async fn open_outreach(
    state: &AppState,
    owner_id: &str,
    bot_id: &str,
    person_id: &str,
    title: String,
) -> Result<String, ApiError> {
    state
        .transaction(|txn| {
            let (owner, bot, person, title) = (
                owner_id.to_string(),
                bot_id.to_string(),
                person_id.to_string(),
                title.clone(),
            );
            Box::pin(async move {
                coordinate(txn, "bot-outreach", &[bot.as_str(), person.as_str()]).await?;
                start_outreach_in(txn, &owner, &bot, &person, title, now_millis()).await
            })
        })
        .await
}

/// The person's row in a room; `None` when they were never in it.
pub(crate) async fn member_in<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    principal_id: &str,
) -> Result<Option<conversation_member::Model>, ApiError> {
    Ok(conversation_member::Entity::find()
        .filter(conversation_member::Column::ConversationId.eq(conversation_id))
        .filter(conversation_member::Column::PrincipalId.eq(principal_id))
        .one(db)
        .await?)
}
