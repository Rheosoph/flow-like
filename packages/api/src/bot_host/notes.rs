//! A bot's notes in one conversation, stored in the hub database (`BotNote`). The server's turns
//! read and write them through [`ServerNotes`]; the owner's routes (`routes/bots/notes.rs`)
//! share the row conversions and [`write_in`], so both apply changes through [`BotNote::apply`]
//! and compare-and-set on `version`.

use crate::{
    entity::{bot, bot_note},
    routes::inbox::now_millis,
};
use async_trait::async_trait;
use chrono::{DateTime, FixedOffset};
use flow_like::flow::copilot::bot_notes::NotesStore;
use flow_like_types::{
    anyhow,
    bot_notes::{
        BotNote, MAX_OPEN_NOTES, NoteAuthor, NoteChange, NoteError, NoteState, NoteVersion,
        prunable,
    },
    data_classification::{DataClassification, DataSensitivity},
};
use sea_orm::{
    ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseConnection, DbErr, EntityTrait,
    QueryFilter, QueryOrder, QuerySelect, sea_query::Expr,
};
use std::sync::Arc;

pub(crate) fn time_of(millis: i64) -> DateTime<FixedOffset> {
    DateTime::from_timestamp_millis(millis)
        .unwrap_or_default()
        .fixed_offset()
}

fn sensitivity_of(row: &bot_note::Model) -> DataSensitivity {
    DataSensitivity {
        classification: row
            .classification
            .as_deref()
            .and_then(DataClassification::parse),
        pii: row.pii,
    }
}

/// The stored row as a note; `history` is kept only when asked for.
pub(crate) fn note_of(row: bot_note::Model, with_history: bool) -> BotNote {
    let history: Vec<NoteVersion> = if with_history {
        serde_json::from_value(row.history.clone()).unwrap_or_default()
    } else {
        Vec::new()
    };
    BotNote {
        key: row.key.clone(),
        title: row.title.clone(),
        status: row.status.clone(),
        body: row.body.clone(),
        state: NoteState::from_stored(&row.state),
        version: u32::try_from(row.version).unwrap_or(0),
        sensitivity: sensitivity_of(&row),
        updated_by: row.updated_by.clone(),
        created_at: row.created_at.timestamp_millis(),
        updated_at: row.updated_at.timestamp_millis(),
        history,
    }
}

fn row_of(bot_id: &str, conversation_id: &str, note: &BotNote) -> bot_note::ActiveModel {
    bot_note::ActiveModel {
        bot_id: Set(bot_id.to_owned()),
        conversation_id: Set(conversation_id.to_owned()),
        key: Set(note.key.clone()),
        title: Set(note.title.clone()),
        status: Set(note.status.clone()),
        body: Set(note.body.clone()),
        state: Set(note.state.stored().to_owned()),
        version: Set(i32::try_from(note.version).unwrap_or(i32::MAX)),
        classification: Set(note
            .sensitivity
            .classification
            .map(|classification| classification.as_str().to_owned())),
        pii: Set(note.sensitivity.pii),
        updated_by: Set(note.updated_by.clone()),
        history: Set(serde_json::to_value(&note.history).unwrap_or_default()),
        created_at: Set(time_of(note.created_at)),
        updated_at: Set(time_of(note.updated_at)),
    }
}

fn of_bot_in(bot_id: &str, conversation_id: &str) -> sea_orm::Select<bot_note::Entity> {
    bot_note::Entity::find()
        .filter(bot_note::Column::BotId.eq(bot_id))
        .filter(bot_note::Column::ConversationId.eq(conversation_id))
}

/// The bot's notes in the conversation, newest first, without history.
pub(crate) async fn list_in<C: ConnectionTrait>(
    db: &C,
    bot_id: &str,
    conversation_id: &str,
) -> Result<Vec<BotNote>, DbErr> {
    Ok(of_bot_in(bot_id, conversation_id)
        .order_by_desc(bot_note::Column::UpdatedAt)
        .order_by_asc(bot_note::Column::Key)
        .all(db)
        .await?
        .into_iter()
        .map(|row| note_of(row, false))
        .collect())
}

/// One note with its history.
pub(crate) async fn get_in<C: ConnectionTrait>(
    db: &C,
    bot_id: &str,
    conversation_id: &str,
    key: &str,
) -> Result<Option<BotNote>, DbErr> {
    Ok(of_bot_in(bot_id, conversation_id)
        .filter(bot_note::Column::Key.eq(key))
        .one(db)
        .await?
        .map(|row| note_of(row, true)))
}

#[derive(Debug)]
pub(crate) enum WriteError {
    Invalid(NoteError),
    /// The note moved past `expected_version`, or another writer won the compare-and-set.
    Conflict {
        current: Option<u32>,
    },
    TooManyOpen,
    Db(DbErr),
}

impl From<DbErr> for WriteError {
    fn from(error: DbErr) -> Self {
        Self::Db(error)
    }
}

impl std::fmt::Display for WriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(error) => error.fmt(f),
            Self::Conflict {
                current: Some(version),
            } => write!(
                f,
                "the note changed meanwhile; it is at version {version} now"
            ),
            Self::Conflict { current: None } => {
                f.write_str("the note changed meanwhile; read it again")
            }
            Self::TooManyOpen => write!(
                f,
                "a bot keeps at most {MAX_OPEN_NOTES} open notes in one conversation; close one first"
            ),
            Self::Db(error) => write!(f, "the note could not be stored: {error}"),
        }
    }
}

enum Attempt {
    Stored(BotNote),
    Lost(Option<u32>),
}

async fn attempt_in<C: ConnectionTrait>(
    db: &C,
    bot_id: &str,
    conversation_id: &str,
    key: &str,
    change: &NoteChange,
    author: &NoteAuthor,
    expected_version: Option<u32>,
) -> Result<Attempt, WriteError> {
    let all = of_bot_in(bot_id, conversation_id).all(db).await?;
    let existing = all.iter().find(|row| row.key == key).cloned();
    let current = existing
        .as_ref()
        .map(|row| u32::try_from(row.version).unwrap_or(0));
    if let Some(expected) = expected_version
        && current != Some(expected)
    {
        return Ok(Attempt::Lost(current));
    }
    let existing_note = existing.clone().map(|row| note_of(row, true));
    let note =
        BotNote::apply(existing_note.as_ref(), key, change, author).map_err(WriteError::Invalid)?;
    if existing_note.as_ref() == Some(&note) {
        return Ok(Attempt::Stored(note));
    }
    let reopens = note.is_open() && existing_note.as_ref().is_none_or(|old| !old.is_open());
    if reopens {
        let open = all
            .iter()
            .filter(|row| row.key != key && NoteState::from_stored(&row.state) == NoteState::Open)
            .count();
        if open >= MAX_OPEN_NOTES {
            return Err(WriteError::TooManyOpen);
        }
    }
    let stored = match &existing {
        None => {
            let inserted = bot_note::Entity::insert(row_of(bot_id, conversation_id, &note))
                .on_conflict_do_nothing()
                .exec(db)
                .await?;
            matches!(inserted, sea_orm::TryInsertResult::Inserted(_))
        }
        Some(old) => {
            let update = row_of(bot_id, conversation_id, &note);
            bot_note::Entity::update_many()
                .col_expr(bot_note::Column::Title, Expr::value(note.title.clone()))
                .col_expr(bot_note::Column::Status, Expr::value(note.status.clone()))
                .col_expr(bot_note::Column::Body, Expr::value(note.body.clone()))
                .col_expr(bot_note::Column::State, Expr::value(note.state.stored()))
                .col_expr(
                    bot_note::Column::Version,
                    Expr::value(update.version.unwrap()),
                )
                .col_expr(
                    bot_note::Column::Classification,
                    Expr::value(update.classification.unwrap()),
                )
                .col_expr(bot_note::Column::Pii, Expr::value(note.sensitivity.pii))
                .col_expr(
                    bot_note::Column::UpdatedBy,
                    Expr::value(note.updated_by.clone()),
                )
                .col_expr(
                    bot_note::Column::History,
                    Expr::value(update.history.unwrap()),
                )
                .col_expr(
                    bot_note::Column::UpdatedAt,
                    Expr::value(time_of(note.updated_at)),
                )
                .filter(bot_note::Column::BotId.eq(bot_id))
                .filter(bot_note::Column::ConversationId.eq(conversation_id))
                .filter(bot_note::Column::Key.eq(key))
                .filter(bot_note::Column::Version.eq(old.version))
                .exec(db)
                .await?
                .rows_affected
                > 0
        }
    };
    if !stored {
        return Ok(Attempt::Lost(current));
    }
    if existing.is_none() {
        prune_in(db, bot_id, conversation_id).await?;
    }
    Ok(Attempt::Stored(note))
}

async fn prune_in<C: ConnectionTrait>(
    db: &C,
    bot_id: &str,
    conversation_id: &str,
) -> Result<(), DbErr> {
    let kept = list_in(db, bot_id, conversation_id).await?;
    let stale = prunable(&kept);
    if stale.is_empty() {
        return Ok(());
    }
    bot_note::Entity::delete_many()
        .filter(bot_note::Column::BotId.eq(bot_id))
        .filter(bot_note::Column::ConversationId.eq(conversation_id))
        .filter(bot_note::Column::Key.is_in(stale))
        .filter(bot_note::Column::State.eq(NoteState::Closed.stored()))
        .exec(db)
        .await?;
    Ok(())
}

/// Creates or revises `key` through [`BotNote::apply`], compare-and-set on its version. Without
/// `expected_version` a lost race is retried once on the fresh row; with it, any move past the
/// expected version is a [`WriteError::Conflict`]. A new note prunes the oldest closed ones beyond
/// the kept limit.
pub(crate) async fn write_in<C: ConnectionTrait>(
    db: &C,
    bot_id: &str,
    conversation_id: &str,
    key: &str,
    change: &NoteChange,
    author: &NoteAuthor,
    expected_version: Option<u32>,
) -> Result<BotNote, WriteError> {
    let attempts = if expected_version.is_some() { 1 } else { 2 };
    let mut current = None;
    for _ in 0..attempts {
        match attempt_in(
            db,
            bot_id,
            conversation_id,
            key,
            change,
            author,
            expected_version,
        )
        .await?
        {
            Attempt::Stored(note) => return Ok(note),
            Attempt::Lost(version) => current = version,
        }
    }
    Err(WriteError::Conflict { current })
}

/// Deletes the notes of every bot `owner_id` owns in a direct conversation the owner cleared.
pub(crate) async fn clear_owner_notes_in<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    owner_id: &str,
) -> Result<u64, DbErr> {
    let owned = bot::Entity::find()
        .select_only()
        .column(bot::Column::Id)
        .filter(bot::Column::OwnerId.eq(owner_id))
        .into_tuple::<String>()
        .all(db)
        .await?;
    if owned.is_empty() {
        return Ok(0);
    }
    Ok(bot_note::Entity::delete_many()
        .filter(bot_note::Column::ConversationId.eq(conversation_id))
        .filter(bot_note::Column::BotId.is_in(owned))
        .exec(db)
        .await?
        .rows_affected)
}

/// One bot's notes in one conversation for a server turn. `turn` is the held turn (id, host id)
/// whose live status line `turn_status` sets.
pub(crate) struct ServerNotes {
    pub(crate) db: DatabaseConnection,
    pub(crate) bot_id: String,
    pub(crate) conversation_id: String,
    pub(crate) turn: Option<(String, String)>,
}

impl ServerNotes {
    pub(crate) fn shared(self) -> Arc<dyn NotesStore> {
        Arc::new(self)
    }
}

#[async_trait]
impl NotesStore for ServerNotes {
    async fn list(&self) -> flow_like_types::Result<Vec<BotNote>> {
        Ok(list_in(&self.db, &self.bot_id, &self.conversation_id).await?)
    }

    async fn write(
        &self,
        key: &str,
        change: &NoteChange,
        author: &NoteAuthor,
    ) -> flow_like_types::Result<BotNote> {
        write_in(
            &self.db,
            &self.bot_id,
            &self.conversation_id,
            key,
            change,
            author,
            None,
        )
        .await
        .map_err(|error| anyhow!("{error}"))
    }

    async fn turn_status(&self, status: Option<&str>) -> flow_like_types::Result<()> {
        let Some((turn_id, host_id)) = &self.turn else {
            return Ok(());
        };
        super::ledger::set_status_text_in(&self.db, turn_id, host_id, status, now_millis()).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bot_host::fixture::{database, insert_bot};
    use crate::routes::inbox::fixture::running_turn;
    use flow_like::flow::copilot::bot_notes::NotesStore;
    use flow_like_types::bot_notes::{MAX_KEPT_NOTES, MAX_KEPT_VERSIONS};

    fn author(by: &str, at: i64) -> NoteAuthor {
        NoteAuthor {
            by: by.into(),
            at,
            sensitivity: DataSensitivity::default(),
        }
    }

    fn create(title: &str, state: Option<NoteState>) -> NoteChange {
        NoteChange {
            title: Some(title.into()),
            state,
            ..Default::default()
        }
    }

    fn status(status: &str) -> NoteChange {
        NoteChange {
            status: Some(status.into()),
            ..Default::default()
        }
    }

    async fn put(
        db: &DatabaseConnection,
        key: &str,
        change: &NoteChange,
        at: i64,
        expected: Option<u32>,
    ) -> Result<BotNote, WriteError> {
        write_in(
            db,
            "bot_a",
            "c1",
            key,
            change,
            &author("bot_a", at),
            expected,
        )
        .await
    }

    #[tokio::test]
    async fn a_revision_keeps_history_and_lists_leave_it_out() {
        let db = database().await;
        put(&db, "plan", &create("Plan", None), 1, None)
            .await
            .unwrap();
        for step in 2..=8 {
            put(&db, "plan", &status(&format!("Step {step}")), step, None)
                .await
                .unwrap();
        }
        let note = get_in(&db, "bot_a", "c1", "plan").await.unwrap().unwrap();
        assert_eq!(note.version, 8);
        assert_eq!(note.history.len(), MAX_KEPT_VERSIONS);
        assert_eq!(note.history[0].version, 7);
        let listed = list_in(&db, "bot_a", "c1").await.unwrap();
        assert!(listed[0].history.is_empty());
        assert!(list_in(&db, "bot_b", "c1").await.unwrap().is_empty());
        assert!(list_in(&db, "bot_a", "c2").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_stale_expected_version_conflicts_and_changes_nothing() {
        let db = database().await;
        put(&db, "plan", &create("Plan", None), 1, None)
            .await
            .unwrap();
        put(&db, "plan", &status("Step 2"), 2, Some(1))
            .await
            .unwrap();
        let stale = put(&db, "plan", &status("Mine"), 3, Some(1)).await;
        assert!(matches!(
            stale,
            Err(WriteError::Conflict { current: Some(2) })
        ));
        let missing = put(&db, "other", &create("Other", None), 3, Some(1)).await;
        assert!(matches!(
            missing,
            Err(WriteError::Conflict { current: None })
        ));
        let note = get_in(&db, "bot_a", "c1", "plan").await.unwrap().unwrap();
        assert_eq!((note.version, note.status.as_deref()), (2, Some("Step 2")));
    }

    #[tokio::test]
    async fn a_lost_compare_and_set_is_not_stored() {
        let db = database().await;
        put(&db, "plan", &create("Plan", None), 1, None)
            .await
            .unwrap();
        let moved = bot_note::Entity::update_many()
            .col_expr(bot_note::Column::Version, Expr::value(5))
            .exec(&db)
            .await
            .unwrap();
        assert_eq!(moved.rows_affected, 1);
        let outcome = put(&db, "plan", &status("Late"), 2, None).await.unwrap();
        assert_eq!(outcome.version, 6);
        assert_eq!(outcome.history[0].version, 5);
    }

    #[tokio::test]
    async fn creating_beyond_the_kept_limit_prunes_the_oldest_closed_notes() {
        let db = database().await;
        for index in 0..=MAX_KEPT_NOTES {
            put(
                &db,
                &format!("n{index}"),
                &create("Note", Some(NoteState::Closed)),
                index as i64,
                None,
            )
            .await
            .unwrap();
        }
        let kept = list_in(&db, "bot_a", "c1").await.unwrap();
        assert_eq!(kept.len(), MAX_KEPT_NOTES);
        assert!(kept.iter().all(|note| note.key != "n0"));
    }

    #[tokio::test]
    async fn open_notes_are_capped_on_create_and_on_reopen() {
        let db = database().await;
        for index in 0..MAX_OPEN_NOTES {
            put(
                &db,
                &format!("n{index}"),
                &create("Note", None),
                index as i64,
                None,
            )
            .await
            .unwrap();
        }
        let ninth = put(&db, "extra", &create("Note", None), 20, None).await;
        assert!(matches!(ninth, Err(WriteError::TooManyOpen)));
        let closed = put(
            &db,
            "done",
            &create("Done", Some(NoteState::Closed)),
            21,
            None,
        )
        .await
        .unwrap();
        assert_eq!(closed.state, NoteState::Closed);
        let reopen = NoteChange {
            state: Some(NoteState::Open),
            ..Default::default()
        };
        assert!(matches!(
            put(&db, "done", &reopen, 22, None).await,
            Err(WriteError::TooManyOpen)
        ));
        put(
            &db,
            "n0",
            &NoteChange {
                state: Some(NoteState::Closed),
                ..Default::default()
            },
            23,
            None,
        )
        .await
        .unwrap();
        assert!(put(&db, "done", &reopen, 24, None).await.is_ok());
    }

    #[tokio::test]
    async fn clearing_removes_only_the_owners_bots_notes_in_that_conversation() {
        let db = database().await;
        insert_bot(&db, "bot_a", "user-a", "SERVER").await;
        insert_bot(&db, "bot_b", "user-b", "SERVER").await;
        for (bot, conversation) in [("bot_a", "c1"), ("bot_a", "c2"), ("bot_b", "c1")] {
            write_in(
                &db,
                bot,
                conversation,
                "plan",
                &create("Plan", None),
                &author(bot, 1),
                None,
            )
            .await
            .unwrap();
        }
        assert_eq!(clear_owner_notes_in(&db, "c1", "user-a").await.unwrap(), 1);
        assert!(list_in(&db, "bot_a", "c1").await.unwrap().is_empty());
        assert_eq!(list_in(&db, "bot_a", "c2").await.unwrap().len(), 1);
        assert_eq!(list_in(&db, "bot_b", "c1").await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn the_server_store_sets_the_status_of_the_held_turn_only() {
        let db = database().await;
        insert_bot(&db, "bot_a", "user-a", "SERVER").await;
        let now = crate::routes::inbox::now_millis();
        running_turn(
            &db,
            "t1",
            "bot_a",
            "c1",
            "host-1",
            now + chrono::Duration::seconds(60),
        )
        .await;
        let store = |host: &str| ServerNotes {
            db: db.clone(),
            bot_id: "bot_a".into(),
            conversation_id: "c1".into(),
            turn: Some(("t1".into(), host.into())),
        };
        store("host-1")
            .turn_status(Some("Lesson 2 of 6"))
            .await
            .unwrap();
        let text = |db: &DatabaseConnection| {
            let db = db.clone();
            async move {
                crate::routes::inbox::fixture::turn(&db, "t1")
                    .await
                    .status_text
            }
        };
        assert_eq!(text(&db).await.as_deref(), Some("Lesson 2 of 6"));
        store("host-2").turn_status(Some("Hijacked")).await.unwrap();
        assert_eq!(text(&db).await.as_deref(), Some("Lesson 2 of 6"));
        store("host-1").turn_status(None).await.unwrap();
        assert_eq!(text(&db).await, None);
        let written = store("host-1")
            .write("plan", &create("Plan", None), &author("bot_a", 1))
            .await
            .unwrap();
        assert_eq!(written.version, 1);
    }
}
