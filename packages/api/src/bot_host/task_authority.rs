//! What makes a task's instruction the owner's, so that its runs may act on it: the owner wrote
//! it themselves, or the bot scheduled it because the owner asked in their own chat with the bot.
//! The message that asked is derived from the turn the server keeps, never from tool arguments,
//! and read again whenever a run needs authority, with the owner's edits and deletion applied.

use crate::{
    entity::{bot_task, bot_turn, conversation, inbox_entry},
    error::ApiError,
};
use flow_like::flow::copilot::tool_authorization;
use flow_like_types::{
    bot_task::{TaskCreator, UpdateBotTask},
    inbox::ConversationEncryption,
};
use sea_orm::{
    ActiveValue::Set, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, Statement,
};
use serde_json::{Value, json};

const REQUEST_ENTRIES_SQL: &str = r#"SELECT * FROM "InboxEntry" WHERE "inboxId" = $1 AND "conversationId" = $2 AND "senderId" = $3 AND ("messageId" = $4 OR ("kind" IN ('EDIT', 'DELETE') AND ("envelope" -> 'body' ->> 'target') = $4)) ORDER BY "seq", "createdAt", "id""#;

/// The owner's message that asked the bot for a task, in the owner's own chat with the bot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OwnerRequest {
    pub conversation_id: String,
    pub message_id: String,
}

/// Who writes a task's instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TaskAuthor {
    /// The owner with their own session and no bot turn: the task list in the app.
    Owner,
    /// The bot in a turn, with the owner's message when the owner prompted that turn in their own
    /// chat with the bot.
    Bot(Option<OwnerRequest>),
}

impl From<TaskCreator> for TaskAuthor {
    fn from(creator: TaskCreator) -> Self {
        match creator {
            TaskCreator::Owner => Self::Owner,
            TaskCreator::Bot => Self::Bot(None),
        }
    }
}

impl TaskAuthor {
    pub(crate) fn creator(&self) -> TaskCreator {
        match self {
            Self::Owner => TaskCreator::Owner,
            Self::Bot(_) => TaskCreator::Bot,
        }
    }

    /// Whether a change gives the task this author's authority: the owner sending an instruction,
    /// or the bot changing it.
    pub(crate) fn rewrites(&self, sent: Option<&str>, before: &str, after: &str) -> bool {
        match self {
            Self::Owner => sent.is_some(),
            Self::Bot(_) => before != after,
        }
    }

    /// Sets the authority of a task whose instruction this author wrote.
    pub(crate) fn mark(&self, task: &mut bot_task::ActiveModel) {
        let request = match self {
            Self::Owner | Self::Bot(None) => None,
            Self::Bot(Some(request)) => Some(request.clone()),
        };
        task.owner_written = Set(Some(*self == Self::Owner));
        task.request_conversation_id = Set(request.as_ref().map(|r| r.conversation_id.clone()));
        task.request_message_id = Set(request.map(|r| r.message_id));
    }
}

/// A change to a task and who makes it. A bare change counts as the bot's in a turn nobody with
/// authority prompted, so it can take authority away but never add it.
#[derive(Debug, Clone)]
pub(crate) struct TaskEdit {
    pub change: UpdateBotTask,
    pub author: TaskAuthor,
}

impl From<UpdateBotTask> for TaskEdit {
    fn from(change: UpdateBotTask) -> Self {
        Self {
            change,
            author: TaskAuthor::Bot(None),
        }
    }
}

/// Who writes through a call made for `turn_id`: the owner when no turn is named, else the bot,
/// with the owner's message when the owner prompted that turn (see [`owner_request_of_turn`]).
pub(crate) async fn author_of_turn<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
    turn_id: Option<&str>,
) -> Result<TaskAuthor, ApiError> {
    match turn_id {
        None => Ok(TaskAuthor::Owner),
        Some(turn_id) => Ok(TaskAuthor::Bot(
            owner_request_of_turn(db, owner_id, bot_id, turn_id).await?,
        )),
    }
}

/// The owner's message that started `turn_id`, when the owner prompted that turn of their bot in
/// their own chat with it: a message or call line the owner sent there. A trigger (a task run,
/// mail, a webhook) carries the owner's sender id but is not their message, so it never counts;
/// neither does a turn in a group, one a member or another bot asked for, or an unprompted one.
pub(crate) async fn owner_request_of_turn<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
    turn_id: &str,
) -> Result<Option<OwnerRequest>, ApiError> {
    let Some(turn) = bot_turn::Entity::find_by_id(turn_id)
        .filter(bot_turn::Column::BotId.eq(bot_id))
        .filter(bot_turn::Column::OwnerId.eq(owner_id))
        .one(db)
        .await?
    else {
        return Ok(None);
    };
    if turn.unprompted == Some(true)
        || turn
            .requester_id
            .as_deref()
            .is_some_and(|requester| requester != owner_id)
        || !crate::routes::inbox::approvals::is_owner_chat(
            db,
            owner_id,
            bot_id,
            &turn.conversation_id,
        )
        .await?
    {
        return Ok(None);
    }
    let asked = inbox_entry::Entity::find()
        .filter(inbox_entry::Column::InboxId.eq(bot_id))
        .filter(inbox_entry::Column::ConversationId.eq(&turn.conversation_id))
        .filter(inbox_entry::Column::MessageId.eq(&turn.trigger_message_id))
        .filter(inbox_entry::Column::SenderId.eq(owner_id))
        .filter(inbox_entry::Column::Kind.is_in(["MESSAGE", "TRANSCRIPT"]))
        .one(db)
        .await?
        .is_some();
    Ok(asked.then_some(OwnerRequest {
        conversation_id: turn.conversation_id,
        message_id: turn.trigger_message_id,
    }))
}

/// A trigger's `configured_trigger_instruction`, and the owner's chat when it holds the message
/// that asked for a task end-to-end encrypted: hosts with the bot's keys read it there.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Configured {
    pub instruction: Option<Value>,
    pub unreadable: Option<String>,
}

/// What a run of task `task_id` may act on. An instruction the owner wrote is theirs. A task the
/// bot scheduled carries the owner's message that asked for it, read now; the bot's own wording
/// is evidence only. Rows from before this was recorded, tasks a member or another bot asked for
/// and a deleted message authorize nothing.
pub(crate) async fn configured_for_task<C: ConnectionTrait>(
    db: &C,
    turn: &bot_turn::Model,
    task_id: &str,
) -> Result<Configured, ApiError> {
    let Some(task) = bot_task::Entity::find_by_id(task_id)
        .filter(bot_task::Column::BotId.eq(&turn.bot_id))
        .filter(bot_task::Column::OwnerId.eq(&turn.owner_id))
        .one(db)
        .await?
    else {
        return Ok(Configured::default());
    };
    if task.owner_written == Some(true) {
        return Ok(Configured {
            instruction: Some(tool_authorization::owner_task_instruction(
                &task.id,
                &task.instruction,
            )),
            unreadable: None,
        });
    }
    let (Some(conversation_id), Some(message_id)) =
        (task.request_conversation_id, task.request_message_id)
    else {
        return Ok(Configured::default());
    };
    let Some(conversation) = conversation::Entity::find_by_id(&conversation_id)
        .one(db)
        .await?
    else {
        return Ok(Configured::default());
    };
    if ConversationEncryption::parse(&conversation.encryption) == Some(ConversationEncryption::E2e)
    {
        return Ok(Configured {
            instruction: Some(json!({
                "id": task.id,
                "source": "schedule",
                "scheduled_instruction": task.instruction,
                "owner_message_ref": {"conversation_id": conversation_id, "message_id": message_id},
            })),
            unreadable: Some(conversation_id),
        });
    }
    let entries = request_entries(db, turn, &conversation_id, &message_id).await?;
    let message = tool_authorization::owner_message(
        super::authorization::inbox_events(&entries),
        &turn.owner_id,
        &message_id,
    );
    Ok(Configured {
        instruction: message.map(|message| {
            tool_authorization::requested_task_instruction(&task.id, &task.instruction, message)
        }),
        unreadable: None,
    })
}

/// The bot's copy of the owner's message and of the owner's edits and deletions of it, in
/// conversation order.
async fn request_entries<C: ConnectionTrait>(
    db: &C,
    turn: &bot_turn::Model,
    conversation_id: &str,
    message_id: &str,
) -> Result<Vec<inbox_entry::Model>, ApiError> {
    Ok(inbox_entry::Entity::find()
        .from_raw_sql(Statement::from_sql_and_values(
            db.get_database_backend(),
            REQUEST_ENTRIES_SQL,
            [
                turn.bot_id.as_str().into(),
                conversation_id.into(),
                turn.owner_id.as_str().into(),
                message_id.into(),
            ],
        ))
        .all(db)
        .await?)
}
