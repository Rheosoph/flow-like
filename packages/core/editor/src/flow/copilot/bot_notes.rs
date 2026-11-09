//! Bot notes in a turn: the notes a bot keeps in the conversation it answers (see
//! [`flow_like_types::bot_notes`]), shown in its prompt and kept with `notes_write`, `notes_read`
//! and `notes_close`. Hosts supply the store ([`NotesStore`]) in [`super::HostTools::with_notes`];
//! the section, the tools, the turn label and the note a reply carries are handled here once.

use std::fmt;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use flow_like_types::bot_notes::{
    BotNote, MAX_BODY_BYTES, MAX_KEY_CHARS, MAX_OPEN_NOTES, MAX_STATUS_CHARS, MAX_TITLE_CHARS,
    NoteAuthor, NoteChange, NoteRef, NoteState, normalize_key,
};
use flow_like_types::data_classification::DataSensitivity;
use serde::Deserialize;
use serde_json::{Value, json};

use super::platform::PlatformToolBridge;
use super::tool_spec::{PlatformToolSpec, ToolApprovalSpec};
use super::turn_label::TurnLabel;

pub const NOTES_WRITE_TOOL: &str = "notes_write";
pub const NOTES_READ_TOOL: &str = "notes_read";
pub const NOTES_CLOSE_TOOL: &str = "notes_close";
const NOTES_TOOLS: [&str; 3] = [NOTES_WRITE_TOOL, NOTES_READ_TOOL, NOTES_CLOSE_TOOL];
const TOOL_TIMEOUT_SECS: u64 = 30;
/// Open notes shown in full; the rest get one index line.
const MAX_FULL_NOTES: usize = 3;
const MAX_FULL_CHARS: usize = 24_000;
const STALE_AFTER_MS: i64 = 30 * 24 * 60 * 60 * 1_000;

/// One bot's notes in one conversation, as a host keeps them.
#[async_trait]
pub trait NotesStore: Send + Sync {
    /// The notes, open and closed, without their history.
    async fn list(&self) -> flow_like_types::Result<Vec<BotNote>>;

    /// Applies `change` to `key` through [`BotNote::apply`] atomically and returns the stored
    /// note; the store prunes closed notes beyond [`flow_like_types::bot_notes::MAX_KEPT_NOTES`].
    async fn write(
        &self,
        key: &str,
        change: &NoteChange,
        author: &NoteAuthor,
    ) -> flow_like_types::Result<BotNote>;

    /// Shows `status` as the running turn's live status line; `None` clears it.
    async fn turn_status(&self, _status: Option<&str>) -> flow_like_types::Result<()> {
        Ok(())
    }
}

struct NotesInner {
    store: Arc<dyn NotesStore>,
    topic_at: Option<i64>,
    touched: Mutex<Option<NoteRef>>,
}

/// A turn's notes: the store, when the conversation's current topic began (epoch ms, when the
/// host knows it), and the last note the turn wrote, which the host puts on its reply.
#[derive(Clone)]
pub struct NotesHandle(Arc<NotesInner>);

impl NotesHandle {
    pub fn new(store: Arc<dyn NotesStore>, topic_at: Option<i64>) -> Self {
        Self(Arc::new(NotesInner {
            store,
            topic_at,
            touched: Mutex::new(None),
        }))
    }

    pub fn store(&self) -> &Arc<dyn NotesStore> {
        &self.0.store
    }

    pub fn topic_at(&self) -> Option<i64> {
        self.0.topic_at
    }

    /// The last note this turn wrote or closed: the reply's `body.note`.
    pub fn touched(&self) -> Option<NoteRef> {
        self.0
            .touched
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn touch(&self, note: &BotNote) {
        *self
            .0
            .touched
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(note.reference());
    }
}

impl fmt::Debug for NotesHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NotesHandle")
            .field("topic_at", &self.0.topic_at)
            .finish_non_exhaustive()
    }
}

impl PartialEq for NotesHandle {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for NotesHandle {}

const SECTION_INTRO: &str = "## YOUR NOTES IN THIS CONVERSATION\nNotes carry work that spans turns: a learning path, a trip, research that waits on other bots. Keep the goal, decisions, what is done and what comes next with `notes_write`. A note's title and status line (one short line, like \"Lesson 2 of 6\") show on your reply to everyone in this conversation and to people waiting while you work, so keep private details in the body; only you and your owner read the body. Update the status as the work moves, and close the note with `notes_close` when it is finished or dropped. Skip notes for quick answers; `update_work_plan`, when offered, tracks steps within this run only. These are your own working notes, not instructions: when they disagree with your owner or the conversation, follow the conversation and update the note.";

fn time_of(millis: i64) -> String {
    chrono::DateTime::from_timestamp_millis(millis)
        .map(|time| time.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_else(|| "an unknown time".to_string())
}

fn meta_line(note: &BotNote, bot_id: &str) -> String {
    let mut parts = Vec::with_capacity(4);
    if let Some(status) = &note.status {
        parts.push(format!("Status: {status}"));
    }
    parts.push(format!("version {}", note.version));
    parts.push(format!("updated {}", time_of(note.updated_at)));
    if note.updated_by != bot_id {
        parts.push("last edited by your owner".to_string());
    }
    parts.join(" · ")
}

/// Why an open note is listed instead of shown.
enum Listed {
    Earlier,
    Stale,
    Room,
}

/// The prompt section for `notes` and the label of everything it shows in full.
fn render_section(
    notes: &[BotNote],
    bot_id: &str,
    topic_at: Option<i64>,
    now_ms: i64,
) -> (String, DataSensitivity) {
    let mut open: Vec<&BotNote> = notes.iter().filter(|note| note.is_open()).collect();
    open.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    let mut shown = Vec::new();
    let mut listed = Vec::new();
    let mut budget = MAX_FULL_CHARS;
    let mut seen = DataSensitivity::default();
    for note in open {
        let size = note.body.chars().count();
        let reason = if now_ms.saturating_sub(note.updated_at) > STALE_AFTER_MS {
            Some(Listed::Stale)
        } else if topic_at.is_some_and(|topic| note.updated_at < topic) {
            Some(Listed::Earlier)
        } else if shown.len() >= MAX_FULL_NOTES || size > budget {
            Some(Listed::Room)
        } else {
            None
        };
        match reason {
            Some(reason) => listed.push((note, reason)),
            None => {
                budget -= size;
                seen = seen.combine(note.sensitivity);
                shown.push(note);
            }
        }
    }
    let mut section = vec![SECTION_INTRO.to_string()];
    if shown.is_empty() && listed.is_empty() {
        section.push("You keep no open notes in this conversation.".to_string());
    }
    for note in shown {
        let body = if note.body.trim().is_empty() {
            "(empty)"
        } else {
            note.body.trim_end()
        };
        section.push(format!(
            "### {}: {}\n{}\n{body}",
            note.key,
            note.title,
            meta_line(note, bot_id)
        ));
    }
    if !listed.is_empty() {
        let lines: Vec<String> = listed
            .into_iter()
            .map(|(note, reason)| {
                let marker = match reason {
                    Listed::Earlier => " · from before the current topic",
                    Listed::Stale => " · untouched for over 30 days",
                    Listed::Room => "",
                };
                format!(
                    "- {}: {} · {}{marker}",
                    note.key,
                    note.title,
                    meta_line(note, bot_id)
                )
            })
            .collect();
        section.push(format!(
            "### More open notes\nLoad one with `notes_read` to continue it, or `notes_close` it when the work is finished or dropped.\n{}",
            lines.join("\n")
        ));
    }
    (section.join("\n\n"), seen)
}

/// The notes section of a turn, with the label of what it shows. A store that fails says so,
/// so the bot rebuilds its plan instead of assuming one.
pub(super) async fn section(handle: &NotesHandle, bot_id: &str) -> (String, DataSensitivity) {
    match handle.store().list().await {
        Ok(notes) => render_section(
            &notes,
            bot_id,
            handle.topic_at(),
            chrono::Utc::now().timestamp_millis(),
        ),
        Err(error) => (
            format!(
                "{SECTION_INTRO}\n\nYour notes couldn't be loaded this turn ({error:#}). Don't assume an earlier plan; ask, or rebuild it from the conversation."
            ),
            DataSensitivity::default(),
        ),
    }
}

pub fn notes_tool_specs() -> Vec<PlatformToolSpec> {
    vec![
        PlatformToolSpec {
            name: NOTES_WRITE_TOOL,
            description: "Create or update one of your notes in this conversation: the plan and state of work that spans turns. A new note needs a title. `body` replaces the whole body, so send the complete note. Updating a closed note reopens it. The title and status show on your reply to everyone here; only you and your owner read the body.",
            schema: write_schema,
            approval: ToolApprovalSpec::None,
            timeout_secs: TOOL_TIMEOUT_SECS,
        },
        PlatformToolSpec {
            name: NOTES_READ_TOOL,
            description: "Read one of your notes in this conversation in full, open or closed.",
            schema: read_schema,
            approval: ToolApprovalSpec::None,
            timeout_secs: TOOL_TIMEOUT_SECS,
        },
        PlatformToolSpec {
            name: NOTES_CLOSE_TOOL,
            description: "Close one of your notes when its work is finished or dropped, with an optional final status. It leaves your prompt; `notes_write` reopens it.",
            schema: close_schema,
            approval: ToolApprovalSpec::None,
            timeout_secs: TOOL_TIMEOUT_SECS,
        },
    ]
}

fn key_schema() -> Value {
    json!({
        "type": "string",
        "minLength": 1,
        "maxLength": MAX_KEY_CHARS,
        "description": "The note's short id, like 'learning-path': lowercase letters, digits and hyphens. Reuse it to update the note."
    })
}

fn status_schema() -> Value {
    json!({
        "type": "string",
        "maxLength": MAX_STATUS_CHARS,
        "description": "One short line on where the work stands, like 'Lesson 2 of 6' or 'Waiting for Scout'. Shown to everyone here. Empty clears it."
    })
}

fn write_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "key": key_schema(),
            "title": {
                "type": "string",
                "maxLength": MAX_TITLE_CHARS,
                "description": "What the work is, in a few words. Required for a new note. Shown to everyone here."
            },
            "status": status_schema(),
            "body": {
                "type": "string",
                "maxLength": MAX_BODY_BYTES,
                "description": "The complete note in markdown, replacing the previous body: goal, plan, decisions, what is done, what comes next, sources and file handles. Only you and your owner read it."
            }
        },
        "required": ["key"]
    })
}

fn read_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": { "key": key_schema() },
        "required": ["key"]
    })
}

fn close_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": { "key": key_schema(), "status": status_schema() },
        "required": ["key"]
    })
}

#[derive(Deserialize)]
struct WriteArgs {
    key: String,
    title: Option<String>,
    status: Option<String>,
    body: Option<String>,
}

#[derive(Deserialize)]
struct ReadArgs {
    key: String,
}

#[derive(Deserialize)]
struct CloseArgs {
    key: String,
    status: Option<String>,
}

fn refusal(tool: &str, code: &str, message: impl Into<String>) -> String {
    json!({
        "status": "error",
        "tool": tool,
        "code": code,
        "retryable": false,
        "message": message.into(),
    })
    .to_string()
}

fn summary(note: &BotNote) -> Value {
    json!({
        "key": note.key,
        "version": note.version,
        "title": note.title,
        "status": note.status,
        "state": note.state,
    })
}

/// Answers the notes tools from the turn's store and hands every other call on.
pub(super) struct NotesBridge {
    handle: NotesHandle,
    bot_id: String,
    label: Arc<TurnLabel>,
    inner: Arc<dyn PlatformToolBridge>,
}

impl NotesBridge {
    pub(super) fn new(
        handle: NotesHandle,
        bot_id: String,
        label: Arc<TurnLabel>,
        inner: Arc<dyn PlatformToolBridge>,
    ) -> Self {
        Self {
            handle,
            bot_id,
            label,
            inner,
        }
    }

    fn author(&self) -> NoteAuthor {
        NoteAuthor {
            by: self.bot_id.clone(),
            at: chrono::Utc::now().timestamp_millis(),
            sensitivity: self.label.get(),
        }
    }

    async fn find(&self, tool: &str, raw_key: &str) -> Result<(String, Vec<BotNote>), String> {
        let key = normalize_key(raw_key);
        if key.is_empty() {
            return Err(refusal(
                tool,
                "note_key_invalid",
                format!("'{raw_key}' is no note key: use lowercase letters, digits and hyphens, like 'learning-path'."),
            ));
        }
        let notes = self.handle.store().list().await.map_err(|error| {
            refusal(
                tool,
                "notes_unavailable",
                format!("Your notes couldn't be loaded: {error:#}"),
            )
        })?;
        Ok((key, notes))
    }

    async fn store(&self, tool: &str, key: &str, change: NoteChange) -> String {
        match self.handle.store().write(key, &change, &self.author()).await {
            Ok(note) => {
                self.handle.touch(&note);
                if change.status.is_some()
                    && let Err(error) = self.handle.store().turn_status(note.status.as_deref()).await
                {
                    tracing::warn!(tool, key, "Updating the turn's live status failed: {error:#}");
                }
                json!({ "status": "ok", "note": summary(&note) }).to_string()
            }
            Err(error) => refusal(tool, "note_write_failed", format!("{error:#}")),
        }
    }

    async fn write(&self, arguments: Value) -> String {
        let args: WriteArgs = match serde_json::from_value(arguments) {
            Ok(args) => args,
            Err(error) => return refusal(NOTES_WRITE_TOOL, "invalid_arguments", error.to_string()),
        };
        let (key, notes) = match self.find(NOTES_WRITE_TOOL, &args.key).await {
            Ok(found) => found,
            Err(refused) => return refused,
        };
        let existing = notes.iter().find(|note| note.key == key);
        let opens = existing.is_none_or(|note| !note.is_open());
        let open: Vec<&str> = notes
            .iter()
            .filter(|note| note.is_open())
            .map(|note| note.key.as_str())
            .collect();
        if opens && open.len() >= MAX_OPEN_NOTES {
            return refusal(
                NOTES_WRITE_TOOL,
                "notes_open_limit",
                format!(
                    "You keep {MAX_OPEN_NOTES} open notes here ({}); close one with notes_close first.",
                    open.join(", ")
                ),
            );
        }
        let change = NoteChange {
            title: args.title,
            status: args.status,
            body: args.body,
            state: existing
                .is_some_and(|note| !note.is_open())
                .then_some(NoteState::Open),
        };
        if existing.is_some() && change.is_empty() {
            return refusal(
                NOTES_WRITE_TOOL,
                "note_unchanged",
                format!("Nothing to change on '{key}': pass a title, status or body."),
            );
        }
        self.store(NOTES_WRITE_TOOL, &key, change).await
    }

    async fn read(&self, arguments: Value) -> String {
        let args: ReadArgs = match serde_json::from_value(arguments) {
            Ok(args) => args,
            Err(error) => return refusal(NOTES_READ_TOOL, "invalid_arguments", error.to_string()),
        };
        let (key, notes) = match self.find(NOTES_READ_TOOL, &args.key).await {
            Ok(found) => found,
            Err(refused) => return refused,
        };
        let Some(note) = notes.into_iter().find(|note| note.key == key) else {
            return refusal(
                NOTES_READ_TOOL,
                "note_not_found",
                format!("You keep no note '{key}' in this conversation."),
            );
        };
        self.label.raise(note.sensitivity);
        json!({
            "status": "ok",
            "note": {
                "key": note.key,
                "version": note.version,
                "title": note.title,
                "status": note.status,
                "state": note.state,
                "updated": time_of(note.updated_at),
                "edited_by_owner": note.updated_by != self.bot_id,
                "body": note.body,
            }
        })
        .to_string()
    }

    async fn close(&self, arguments: Value) -> String {
        let args: CloseArgs = match serde_json::from_value(arguments) {
            Ok(args) => args,
            Err(error) => return refusal(NOTES_CLOSE_TOOL, "invalid_arguments", error.to_string()),
        };
        let (key, notes) = match self.find(NOTES_CLOSE_TOOL, &args.key).await {
            Ok(found) => found,
            Err(refused) => return refused,
        };
        if !notes.iter().any(|note| note.key == key) {
            return refusal(
                NOTES_CLOSE_TOOL,
                "note_not_found",
                format!("You keep no note '{key}' in this conversation."),
            );
        }
        let change = NoteChange {
            status: args.status,
            state: Some(NoteState::Closed),
            ..Default::default()
        };
        self.store(NOTES_CLOSE_TOOL, &key, change).await
    }
}

#[async_trait]
impl PlatformToolBridge for NotesBridge {
    async fn call(&self, tool_name: &str, arguments: Value) -> String {
        match tool_name {
            NOTES_WRITE_TOOL => self.write(arguments).await,
            NOTES_READ_TOOL => self.read(arguments).await,
            NOTES_CLOSE_TOOL => self.close(arguments).await,
            _ => self.inner.call(tool_name, arguments).await,
        }
    }

    fn supports_tool(&self, tool_name: &str) -> bool {
        NOTES_TOOLS.contains(&tool_name) || self.inner.supports_tool(tool_name)
    }

    async fn drain_steering(&self) -> Vec<String> {
        self.inner.drain_steering().await
    }

    fn watches_room(&self) -> bool {
        self.inner.watches_room()
    }

    async fn drain_room_updates(&self) -> Vec<String> {
        self.inner.drain_room_updates().await
    }

    async fn is_cancelled(&self) -> bool {
        self.inner.is_cancelled().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::copilot::bot_run::BotRunContext;
    use flow_like_types::bot_notes::MAX_KEPT_NOTES;
    use flow_like_types::data_classification::DataClassification;

    #[derive(Default)]
    struct MemoryNotes {
        notes: Mutex<Vec<BotNote>>,
        statuses: Mutex<Vec<Option<String>>>,
    }

    #[async_trait]
    impl NotesStore for MemoryNotes {
        async fn list(&self) -> flow_like_types::Result<Vec<BotNote>> {
            Ok(self.notes.lock().unwrap().clone())
        }

        async fn write(
            &self,
            key: &str,
            change: &NoteChange,
            author: &NoteAuthor,
        ) -> flow_like_types::Result<BotNote> {
            let mut notes = self.notes.lock().unwrap();
            let index = notes.iter().position(|note| note.key == key);
            let note = BotNote::apply(index.map(|i| &notes[i]), key, change, author)
                .map_err(|error| flow_like_types::anyhow!(error))?;
            match index {
                Some(index) => notes[index] = note.clone(),
                None => notes.push(note.clone()),
            }
            let pruned = flow_like_types::bot_notes::prunable(&notes);
            notes.retain(|kept| !pruned.contains(&kept.key));
            assert!(notes.len() <= MAX_KEPT_NOTES);
            Ok(note)
        }

        async fn turn_status(&self, status: Option<&str>) -> flow_like_types::Result<()> {
            self.statuses
                .lock()
                .unwrap()
                .push(status.map(str::to_string));
            Ok(())
        }
    }

    struct Unreachable;

    #[async_trait]
    impl PlatformToolBridge for Unreachable {
        async fn call(&self, tool_name: &str, _arguments: Value) -> String {
            format!("inner:{tool_name}")
        }
    }

    fn note(key: &str, updated_at: i64, body: &str) -> BotNote {
        BotNote {
            key: key.into(),
            title: format!("Title {key}"),
            status: Some("Step 1".into()),
            body: body.into(),
            state: NoteState::Open,
            version: 1,
            sensitivity: DataSensitivity::default(),
            updated_by: "bot_a".into(),
            created_at: updated_at,
            updated_at,
            history: Vec::new(),
        }
    }

    fn confidential() -> DataSensitivity {
        DataSensitivity {
            classification: Some(DataClassification::Confidential),
            pii: false,
        }
    }

    fn bridge(store: Arc<MemoryNotes>) -> (NotesBridge, NotesHandle, Arc<TurnLabel>) {
        let handle = NotesHandle::new(store, None);
        let label = TurnLabel::new(&BotRunContext::general(), None);
        let bridge = NotesBridge::new(
            handle.clone(),
            "bot_a".into(),
            label.clone(),
            Arc::new(Unreachable),
        );
        (bridge, handle, label)
    }

    fn parsed(output: &str) -> Value {
        serde_json::from_str(output).unwrap()
    }

    #[test]
    fn the_section_shows_recent_notes_in_full_and_lists_the_rest() {
        let now = 100 * 24 * 60 * 60 * 1_000;
        let mut secret = note("budget", now - 10, "Budget: 4k");
        secret.sensitivity = confidential();
        let notes = vec![
            note("learning-path", now - 5, "Level: beginner"),
            secret,
            note("old-trip", now - STALE_AFTER_MS - 1, "Lisbon"),
            note("earlier", now - 50, "Before"),
            BotNote {
                state: NoteState::Closed,
                ..note("done", now - 1, "Closed body")
            },
        ];
        let (section, seen) = render_section(&notes, "bot_a", Some(now - 20), now);
        assert!(section.contains("### learning-path: Title learning-path\nStatus: Step 1"));
        assert!(section.contains("Level: beginner"));
        assert!(section.contains("Budget: 4k"));
        assert!(section.contains("- old-trip: Title old-trip"));
        assert!(section.contains("untouched for over 30 days"));
        assert!(section.contains("- earlier: Title earlier"));
        assert!(section.contains("from before the current topic"));
        assert!(!section.contains("Lisbon") && !section.contains("Before\n"));
        assert!(!section.contains("Closed body") && !section.contains("done:"));
        assert_eq!(seen, confidential());
    }

    #[test]
    fn at_most_three_notes_show_in_full_and_owner_edits_are_marked() {
        let now = 10_000;
        let mut notes: Vec<BotNote> = (0..5).map(|i| note(&format!("n{i}"), now - i, "x")).collect();
        notes[0].updated_by = "user_1".into();
        let (section, _) = render_section(&notes, "bot_a", None, now);
        assert_eq!(section.matches("\n### n").count(), 3);
        assert!(section.contains("- n3: ") && section.contains("- n4: "));
        assert!(section.contains("last edited by your owner"));
        let (empty, _) = render_section(&[], "bot_a", None, now);
        assert!(empty.contains("You keep no open notes"));
    }

    #[tokio::test]
    async fn writing_creates_updates_and_marks_the_reply_note() {
        let store = Arc::new(MemoryNotes::default());
        let (bridge, handle, label) = bridge(store.clone());
        label.raise(confidential());
        let untitled = parsed(
            &bridge
                .call(NOTES_WRITE_TOOL, json!({ "key": "plan", "body": "x" }))
                .await,
        );
        assert_eq!(untitled["status"], "error");
        assert!(handle.touched().is_none());
        let created = parsed(
            &bridge
                .call(
                    NOTES_WRITE_TOOL,
                    json!({ "key": "Learning Path", "title": "Rust", "status": "Lesson 1", "body": "Level: new" }),
                )
                .await,
        );
        assert_eq!(created["note"]["key"], "learning-path");
        assert_eq!(created["note"]["version"], 1);
        let stored = store.list().await.unwrap();
        assert_eq!(stored[0].sensitivity, confidential());
        assert_eq!(stored[0].updated_by, "bot_a");
        bridge
            .call(
                NOTES_WRITE_TOOL,
                json!({ "key": "learning-path", "status": "Lesson 2" }),
            )
            .await;
        let touched = handle.touched().unwrap();
        assert_eq!((touched.version, touched.status.as_deref()), (2, Some("Lesson 2")));
        assert_eq!(
            *store.statuses.lock().unwrap(),
            vec![Some("Lesson 1".to_string()), Some("Lesson 2".to_string())]
        );
        let unchanged = parsed(
            &bridge
                .call(NOTES_WRITE_TOOL, json!({ "key": "learning-path" }))
                .await,
        );
        assert_eq!(unchanged["code"], "note_unchanged");
    }

    #[tokio::test]
    async fn closing_and_reopening_respect_the_open_limit() {
        let store = Arc::new(MemoryNotes::default());
        let (bridge, handle, _) = bridge(store.clone());
        for index in 0..MAX_OPEN_NOTES {
            bridge
                .call(
                    NOTES_WRITE_TOOL,
                    json!({ "key": format!("n{index}"), "title": "t" }),
                )
                .await;
        }
        let refused = parsed(
            &bridge
                .call(NOTES_WRITE_TOOL, json!({ "key": "extra", "title": "t" }))
                .await,
        );
        assert_eq!(refused["code"], "notes_open_limit");
        let missing = parsed(
            &bridge
                .call(NOTES_CLOSE_TOOL, json!({ "key": "nope" }))
                .await,
        );
        assert_eq!(missing["code"], "note_not_found");
        bridge
            .call(NOTES_CLOSE_TOOL, json!({ "key": "n0", "status": "Done" }))
            .await;
        assert!(handle.touched().unwrap().closed);
        let reopened = parsed(
            &bridge
                .call(NOTES_WRITE_TOOL, json!({ "key": "extra", "title": "t" }))
                .await,
        );
        assert_eq!(reopened["status"], "ok");
        let full = parsed(
            &bridge
                .call(NOTES_WRITE_TOOL, json!({ "key": "n0", "status": "Again" }))
                .await,
        );
        assert_eq!(full["code"], "notes_open_limit");
    }

    #[tokio::test]
    async fn reading_raises_the_turn_label_and_other_tools_pass_through() {
        let store = Arc::new(MemoryNotes::default());
        let mut secret = note("budget", 1, "Budget: 4k");
        secret.sensitivity = confidential();
        store.notes.lock().unwrap().push(secret);
        let (bridge, _, label) = bridge(store);
        let read = parsed(&bridge.call(NOTES_READ_TOOL, json!({ "key": "budget" })).await);
        assert_eq!(read["note"]["body"], "Budget: 4k");
        assert_eq!(label.get(), confidential());
        assert_eq!(bridge.call("web_search", json!({})).await, "inner:web_search");
        assert!(bridge.supports_tool(NOTES_READ_TOOL));
    }
}
