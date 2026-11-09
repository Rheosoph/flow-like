//! Notes: keyed, versioned working state a bot keeps in one conversation, so a plan survives the
//! turns and bot-to-bot hops a request takes. The body is read only by the bot (in that
//! conversation) and its owner; the title and status line are shown to everyone in the
//! conversation on the replies that touched the note. Every store applies changes through
//! [`BotNote::apply`], so versioning, history and sensitivity follow one rule on every host.

use crate::data_classification::DataSensitivity;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt;

pub const MAX_KEY_CHARS: usize = 48;
pub const MAX_TITLE_CHARS: usize = 120;
pub const MAX_STATUS_CHARS: usize = 120;
pub const MAX_BODY_BYTES: usize = 16 * 1024;
/// Open notes one bot keeps in one conversation.
pub const MAX_OPEN_NOTES: usize = 8;
/// Notes one bot keeps in one conversation, open and closed; stores prune the oldest closed.
pub const MAX_KEPT_NOTES: usize = 32;
/// Previous versions kept on a note for its owner.
pub const MAX_KEPT_VERSIONS: usize = 5;

pub const STORED_OPEN: &str = "OPEN";
pub const STORED_CLOSED: &str = "CLOSED";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NoteState {
    #[default]
    Open,
    Closed,
}

impl NoteState {
    pub fn stored(self) -> &'static str {
        match self {
            Self::Open => STORED_OPEN,
            Self::Closed => STORED_CLOSED,
        }
    }

    pub fn from_stored(value: &str) -> Self {
        if value.eq_ignore_ascii_case(STORED_CLOSED) {
            Self::Closed
        } else {
            Self::Open
        }
    }
}

/// A previous version of a note, kept for its owner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NoteVersion {
    pub version: u32,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    pub body: String,
    pub state: NoteState,
    /// The bot id, or the owner's sub when the owner edited it.
    pub updated_by: String,
    /// Epoch milliseconds.
    pub updated_at: i64,
}

/// One note of one bot in one conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BotNote {
    /// Lowercase letters, digits and hyphens: `learning-path`.
    pub key: String,
    pub title: String,
    /// One line shown with the title, such as "Lesson 2 of 6".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Markdown only the bot and its owner read.
    #[serde(default)]
    pub body: String,
    pub state: NoteState,
    pub version: u32,
    /// The most sensitive data the note was written under.
    #[serde(default)]
    pub sensitivity: DataSensitivity,
    /// The bot id, or the owner's sub when the owner edited it last.
    pub updated_by: String,
    /// Epoch milliseconds.
    pub created_at: i64,
    pub updated_at: i64,
    /// Previous versions, newest first, at most [`MAX_KEPT_VERSIONS`]. Lists leave them out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<NoteVersion>,
}

/// What a write changes; `None` keeps the current value. An empty `status` clears it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NoteChange {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<NoteState>,
}

impl NoteChange {
    pub fn is_empty(&self) -> bool {
        self.title.is_none() && self.status.is_none() && self.body.is_none() && self.state.is_none()
    }
}

/// Who writes a change, when, and the most sensitive data the writer had seen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteAuthor {
    pub by: String,
    pub at: i64,
    pub sensitivity: DataSensitivity,
}

/// The note a message was written with: shown as a chip, opened by the owner's info button.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NoteRef {
    pub key: String,
    pub version: u32,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub closed: bool,
}

/// `PATCH /bots/{bot_id}/notes/{conversation_id}/{key}`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NoteWriteRequest {
    #[serde(flatten)]
    pub change: NoteChange,
    /// Refuses the write when the note moved past this version (the owner's editor sends it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_version: Option<u32>,
    /// A host writing for the bot's turn, not the owner editing.
    #[serde(default, skip_serializing_if = "is_false")]
    pub as_bot: bool,
    /// The turn label of a host writing as the bot.
    #[serde(default)]
    pub sensitivity: DataSensitivity,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NoteList {
    pub notes: Vec<BotNote>,
}

/// Why a note or change was refused; the text names the field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteError(String);

impl fmt::Display for NoteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for NoteError {}

fn refuse<T>(message: impl Into<String>) -> Result<T, NoteError> {
    Err(NoteError(message.into()))
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn chars(value: &str) -> usize {
    value.chars().count()
}

/// 1 to [`MAX_KEY_CHARS`] of `a-z`, `0-9` and `-`, starting with a letter or digit.
pub fn is_note_key(key: &str) -> bool {
    let mut bytes = key.bytes();
    key.len() <= MAX_KEY_CHARS
        && bytes
            .next()
            .is_some_and(|first| first.is_ascii_lowercase() || first.is_ascii_digit())
        && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

/// A key from what a model or person typed: lowercased, spaces and underscores as hyphens.
pub fn normalize_key(raw: &str) -> String {
    let mut key = String::new();
    for char in raw.trim().chars().flat_map(char::to_lowercase) {
        let mapped = match char {
            'a'..='z' | '0'..='9' => char,
            ' ' | '_' | '-' | '.' | '/' => '-',
            _ => continue,
        };
        if mapped == '-' && (key.is_empty() || key.ends_with('-')) {
            continue;
        }
        key.push(mapped);
    }
    key.trim_end_matches('-').chars().take(MAX_KEY_CHARS).collect()
}

fn check_key(key: &str) -> Result<(), NoteError> {
    if is_note_key(key) {
        return Ok(());
    }
    refuse(format!(
        "note key '{}' must be 1 to {MAX_KEY_CHARS} of a-z, 0-9 and '-', starting with a letter or digit",
        key.chars().take(80).collect::<String>()
    ))
}

fn check_line(field: &str, value: &str, min: usize, max: usize) -> Result<(), NoteError> {
    let count = chars(value);
    if (min..=max).contains(&count) && !value.chars().any(char::is_control) {
        return Ok(());
    }
    refuse(format!(
        "note {field} is one line of {min} to {max} characters; this one has {count} characters or a line break"
    ))
}

fn check_body(body: &str) -> Result<(), NoteError> {
    if body.len() <= MAX_BODY_BYTES {
        return Ok(());
    }
    refuse(format!(
        "note body has {} bytes; at most {MAX_BODY_BYTES} are allowed, so keep the essentials and drop the rest",
        body.len()
    ))
}

fn tidy_status(status: &str) -> Option<String> {
    let status = status.trim();
    (!status.is_empty()).then(|| status.to_string())
}

impl BotNote {
    /// Creates `key` from `change`, or revises `existing` with it. A change that alters nothing
    /// returns `existing` unchanged, at its version.
    pub fn apply(
        existing: Option<&Self>,
        key: &str,
        change: &NoteChange,
        author: &NoteAuthor,
    ) -> Result<Self, NoteError> {
        let note = match existing {
            Some(existing) => existing.revise(change, author),
            None => Self::create(key, change, author),
        }?;
        note.check()?;
        Ok(note)
    }

    fn create(key: &str, change: &NoteChange, author: &NoteAuthor) -> Result<Self, NoteError> {
        check_key(key)?;
        let Some(title) = change.title.as_deref().map(str::trim).filter(|t| !t.is_empty()) else {
            return refuse(format!("note '{key}' is new, so it needs a title"));
        };
        Ok(Self {
            key: key.to_string(),
            title: title.to_string(),
            status: change.status.as_deref().and_then(tidy_status),
            body: change.body.clone().unwrap_or_default(),
            state: change.state.unwrap_or_default(),
            version: 1,
            sensitivity: author.sensitivity,
            updated_by: author.by.clone(),
            created_at: author.at,
            updated_at: author.at,
            history: Vec::new(),
        })
    }

    fn revise(&self, change: &NoteChange, author: &NoteAuthor) -> Result<Self, NoteError> {
        let title = change
            .title
            .as_deref()
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .map_or_else(|| self.title.clone(), str::to_string);
        let status = match &change.status {
            Some(status) => tidy_status(status),
            None => self.status.clone(),
        };
        let body = change.body.clone().unwrap_or_else(|| self.body.clone());
        let state = change.state.unwrap_or(self.state);
        if title == self.title && status == self.status && body == self.body && state == self.state
        {
            return Ok(self.clone());
        }
        let mut history = Vec::with_capacity(MAX_KEPT_VERSIONS);
        history.push(self.snapshot());
        history.extend(self.history.iter().take(MAX_KEPT_VERSIONS - 1).cloned());
        Ok(Self {
            key: self.key.clone(),
            title,
            status,
            body,
            state,
            version: self.version.saturating_add(1),
            sensitivity: self.sensitivity.combine(author.sensitivity),
            updated_by: author.by.clone(),
            created_at: self.created_at,
            updated_at: author.at,
            history,
        })
    }

    pub fn check(&self) -> Result<(), NoteError> {
        check_key(&self.key)?;
        check_line("title", &self.title, 1, MAX_TITLE_CHARS)?;
        if let Some(status) = &self.status {
            check_line("status", status, 1, MAX_STATUS_CHARS)?;
        }
        check_body(&self.body)?;
        if self.version == 0 {
            return refuse(format!("note '{}' has version 0; versions start at 1", self.key));
        }
        if self.updated_by.trim().is_empty() {
            return refuse(format!("note '{}' names nobody in updated_by", self.key));
        }
        Ok(())
    }

    pub fn is_open(&self) -> bool {
        self.state == NoteState::Open
    }

    /// The current version without the history.
    pub fn snapshot(&self) -> NoteVersion {
        NoteVersion {
            version: self.version,
            title: self.title.clone(),
            status: self.status.clone(),
            body: self.body.clone(),
            state: self.state,
            updated_by: self.updated_by.clone(),
            updated_at: self.updated_at,
        }
    }

    /// `version` when it is current or kept.
    pub fn at_version(&self, version: u32) -> Option<NoteVersion> {
        if version == self.version {
            return Some(self.snapshot());
        }
        self.history
            .iter()
            .find(|kept| kept.version == version)
            .cloned()
    }

    /// The note without its history, as lists return it.
    pub fn without_history(mut self) -> Self {
        self.history.clear();
        self
    }

    pub fn reference(&self) -> NoteRef {
        NoteRef {
            key: self.key.clone(),
            version: self.version,
            title: self.title.clone(),
            status: self.status.clone(),
            closed: self.state == NoteState::Closed,
        }
    }
}

impl NoteRef {
    pub fn check(&self) -> Result<(), NoteError> {
        check_key(&self.key)?;
        check_line("title", &self.title, 1, MAX_TITLE_CHARS)?;
        if let Some(status) = &self.status {
            check_line("status", status, 1, MAX_STATUS_CHARS)?;
        }
        if self.version == 0 {
            return refuse(format!("note '{}' has version 0; versions start at 1", self.key));
        }
        Ok(())
    }

    /// What a model reads beside the message that carries it.
    pub fn line(&self) -> String {
        let status = self
            .status
            .as_deref()
            .map(|status| format!(" · {status}"))
            .unwrap_or_default();
        let closed = if self.closed { " · closed" } else { "" };
        format!("[note {} v{}{status}{closed}]", self.key, self.version)
    }
}

/// Keeps the newest [`MAX_KEPT_NOTES`] of `notes`, open ones first, and returns the keys of the
/// closed notes a store should delete.
pub fn prunable(notes: &[BotNote]) -> Vec<String> {
    if notes.len() <= MAX_KEPT_NOTES {
        return Vec::new();
    }
    let mut closed: Vec<&BotNote> = notes.iter().filter(|note| !note.is_open()).collect();
    closed.sort_by_key(|note| note.updated_at);
    closed
        .into_iter()
        .take(notes.len() - MAX_KEPT_NOTES)
        .map(|note| note.key.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_classification::DataClassification;

    fn author(by: &str, at: i64) -> NoteAuthor {
        NoteAuthor {
            by: by.into(),
            at,
            sensitivity: DataSensitivity::default(),
        }
    }

    fn confidential() -> DataSensitivity {
        DataSensitivity {
            classification: Some(DataClassification::Confidential),
            pii: false,
        }
    }

    fn created() -> BotNote {
        BotNote::apply(
            None,
            "learning-path",
            &NoteChange {
                title: Some("Rust learning path".into()),
                status: Some("Lesson 1 of 6".into()),
                body: Some("Level: beginner".into()),
                state: None,
            },
            &author("bot_a", 10),
        )
        .unwrap()
    }

    #[test]
    fn a_new_note_needs_a_valid_key_and_a_title() {
        let untitled = BotNote::apply(None, "plan", &NoteChange::default(), &author("bot_a", 1));
        assert!(untitled.unwrap_err().to_string().contains("needs a title"));
        let bad_key = BotNote::apply(
            None,
            "Learning Path",
            &NoteChange {
                title: Some("x".into()),
                ..Default::default()
            },
            &author("bot_a", 1),
        );
        assert!(bad_key.unwrap_err().to_string().contains("note key"));
        let note = created();
        assert_eq!((note.version, note.state), (1, NoteState::Open));
        assert_eq!(note.created_at, 10);
    }

    #[test]
    fn revising_bumps_the_version_keeps_history_and_never_lowers_sensitivity() {
        let note = created();
        let tainted = BotNote::apply(
            Some(&note),
            "learning-path",
            &NoteChange {
                status: Some("Lesson 2 of 6".into()),
                ..Default::default()
            },
            &NoteAuthor {
                sensitivity: confidential(),
                ..author("bot_a", 20)
            },
        )
        .unwrap();
        assert_eq!(tainted.version, 2);
        assert_eq!(tainted.history[0].status.as_deref(), Some("Lesson 1 of 6"));
        assert_eq!(tainted.body, "Level: beginner");
        let owner_edit = BotNote::apply(
            Some(&tainted),
            "learning-path",
            &NoteChange {
                body: Some("Level: intermediate".into()),
                ..Default::default()
            },
            &author("user_1", 30),
        )
        .unwrap();
        assert_eq!(owner_edit.sensitivity, confidential());
        assert_eq!(owner_edit.updated_by, "user_1");
        assert_eq!(owner_edit.at_version(1).unwrap().status.as_deref(), Some("Lesson 1 of 6"));
    }

    #[test]
    fn a_change_that_alters_nothing_keeps_the_version() {
        let note = created();
        let same = BotNote::apply(
            Some(&note),
            "learning-path",
            &NoteChange {
                status: Some(" Lesson 1 of 6 ".into()),
                ..Default::default()
            },
            &author("bot_a", 50),
        )
        .unwrap();
        assert_eq!(same, note);
    }

    #[test]
    fn history_keeps_only_the_newest_versions() {
        let mut note = created();
        for lesson in 2..=10 {
            note = BotNote::apply(
                Some(&note),
                "learning-path",
                &NoteChange {
                    status: Some(format!("Lesson {lesson} of 10")),
                    ..Default::default()
                },
                &author("bot_a", lesson),
            )
            .unwrap();
        }
        assert_eq!(note.version, 10);
        assert_eq!(note.history.len(), MAX_KEPT_VERSIONS);
        assert_eq!(note.history[0].version, 9);
        assert!(note.at_version(4).is_none());
    }

    #[test]
    fn an_empty_status_clears_it_and_closing_shows_on_the_reference() {
        let closed = BotNote::apply(
            Some(&created()),
            "learning-path",
            &NoteChange {
                status: Some(String::new()),
                state: Some(NoteState::Closed),
                ..Default::default()
            },
            &author("bot_a", 60),
        )
        .unwrap();
        assert_eq!(closed.status, None);
        let reference = closed.reference();
        assert!(reference.closed);
        assert_eq!(reference.line(), "[note learning-path v2 · closed]");
    }

    #[test]
    fn oversized_bodies_and_multi_line_status_are_refused() {
        let note = created();
        let big = BotNote::apply(
            Some(&note),
            "learning-path",
            &NoteChange {
                body: Some("x".repeat(MAX_BODY_BYTES + 1)),
                ..Default::default()
            },
            &author("bot_a", 70),
        );
        assert!(big.unwrap_err().to_string().contains("bytes"));
        let multi = BotNote::apply(
            Some(&note),
            "learning-path",
            &NoteChange {
                status: Some("a\nb".into()),
                ..Default::default()
            },
            &author("bot_a", 70),
        );
        assert!(multi.unwrap_err().to_string().contains("status"));
    }

    #[test]
    fn keys_normalize_from_what_people_type() {
        assert_eq!(normalize_key(" Learning Path_2026 "), "learning-path-2026");
        assert_eq!(normalize_key("--Plan//A--"), "plan-a");
        assert!(is_note_key(&normalize_key("Reise nach Lissabon")));
        assert_eq!(normalize_key("ÄÖÜ"), "");
    }

    #[test]
    fn pruning_drops_the_oldest_closed_notes_only() {
        let mut notes: Vec<BotNote> = (0..MAX_KEPT_NOTES + 3)
            .map(|index| BotNote {
                key: format!("n{index}"),
                state: if index % 2 == 0 {
                    NoteState::Closed
                } else {
                    NoteState::Open
                },
                updated_at: index as i64,
                ..created()
            })
            .collect();
        notes.reverse();
        assert_eq!(prunable(&notes), vec!["n0", "n2", "n4"]);
        assert!(prunable(&notes[..MAX_KEPT_NOTES]).is_empty());
    }
}
