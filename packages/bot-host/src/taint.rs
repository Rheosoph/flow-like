//! What a local or account turn reads before its first tool call (`todo/bots-talk.md`, Step 2),
//! computed from the entries the turn's history comes from. The server host does the same in
//! `flow_like_api::bot_host::taint`.

use crate::rule::is_plain;
use flow_like::flow::copilot::TurnReads;
use flow_like_local_bots::{InboxEntry, LOCAL_OWNER_ID};
use flow_like_types::{
    data_classification::DataSensitivity,
    inbox::{InboxEntryKind, TriggerBody, TriggerSource, is_tombstone},
};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

pub const TAINT_FIELD: &str = "taint";

/// The taint a message body records; empty when it records none.
pub fn recorded(body: &Value) -> DataSensitivity {
    body.get(TAINT_FIELD)
        .and_then(|taint| serde_json::from_value(taint.clone()).ok())
        .unwrap_or_default()
}

fn read(entries: &[InboxEntry]) -> impl Iterator<Item = &InboxEntry> {
    let since = entries
        .iter()
        .rposition(|entry| entry.kind == InboxEntryKind::Topic)
        .map_or(0, |marker| marker + 1);
    entries[since..]
        .iter()
        .filter(|entry| is_plain(entry) && !is_tombstone(&entry.envelope))
}

/// The bots other than `bot_id` whose messages `entries` hold, for a host to look up their labels.
pub fn bot_senders(entries: &[InboxEntry], bot_id: &str) -> Vec<String> {
    let found: HashSet<&str> = read(entries)
        .map(|entry| entry.sender_id.as_str())
        .filter(|sender| *sender != bot_id && *sender != LOCAL_OWNER_ID)
        .collect();
    found.into_iter().map(str::to_owned).collect()
}

/// What reading `entries` (oldest first) puts into the turn of `bot_id`. `labels` holds the
/// classification of each bot sender that has one.
pub fn reads_of(
    entries: &[InboxEntry],
    bot_id: &str,
    labels: &HashMap<String, DataSensitivity>,
) -> TurnReads {
    let mut reads = TurnReads::default();
    let mut counted: HashSet<&str> = HashSet::new();
    for entry in read(entries) {
        let body = &entry.envelope["body"];
        reads.recorded = reads.recorded.combine(recorded(body));
        if entry.kind == InboxEntryKind::Trigger {
            reads.external_trigger |= TriggerBody::parse(body).is_ok_and(|trigger| {
                matches!(trigger.source, TriggerSource::Email | TriggerSource::Webhook)
            });
        }
        let sender = entry.sender_id.as_str();
        if sender != bot_id
            && counted.insert(sender)
            && let Some(label) = labels.get(sender)
        {
            reads.bot_senders.push(*label);
        }
    }
    reads
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use flow_like_types::data_classification::DataClassification;
    use serde_json::json;

    fn entry(sender: &str, kind: InboxEntryKind, body: Value) -> InboxEntry {
        InboxEntry {
            id: "e".into(),
            message_id: "m".into(),
            conversation_id: "c".into(),
            sender_id: sender.into(),
            kind,
            envelope: json!({ "v": 1, "enc": "none", "body": body }),
            created_at: Utc::now().fixed_offset(),
            seq: Some(1),
            cursor: String::new(),
        }
    }

    fn level(classification: DataClassification) -> DataSensitivity {
        DataSensitivity {
            classification: Some(classification),
            pii: false,
        }
    }

    #[test]
    fn a_bot_sender_and_a_recorded_taint_seed_the_turn() {
        let labels = HashMap::from([("peer".to_owned(), level(DataClassification::Internal))]);
        let entries = [
            entry(
                "peer",
                InboxEntryKind::Message,
                json!({ "text": "hi", TAINT_FIELD: { "classification": "confidential" } }),
            ),
            entry(LOCAL_OWNER_ID, InboxEntryKind::Message, json!({ "text": "and?" })),
        ];
        assert_eq!(bot_senders(&entries, "me"), vec!["peer".to_owned()]);
        let reads = reads_of(&entries, "me", &labels);
        assert_eq!(reads.seed(), level(DataClassification::Confidential));
        assert_eq!(reads.bot_senders, vec![level(DataClassification::Internal)]);
        assert!(reads_of(&entries[1..], "me", &labels).seed().is_empty());
    }

    #[test]
    fn mail_and_webhook_triggers_are_external_and_a_schedule_is_not() {
        let trigger = |source: &str| {
            entry(
                "system",
                InboxEntryKind::Trigger,
                json!({ "source": source, "ref_id": "r", "title": "t", "text": "x" }),
            )
        };
        let seeded = |source| reads_of(&[trigger(source)], "me", &HashMap::new()).external_trigger;
        assert!(seeded("email"));
        assert!(seeded("webhook"));
        assert!(!seeded("schedule"));
    }

    #[test]
    fn entries_before_a_new_topic_are_not_read() {
        let entries = [
            entry(
                "peer",
                InboxEntryKind::Message,
                json!({ "text": "old", TAINT_FIELD: { "classification": "restricted" } }),
            ),
            entry(LOCAL_OWNER_ID, InboxEntryKind::Topic, json!({})),
            entry(LOCAL_OWNER_ID, InboxEntryKind::Message, json!({ "text": "new" })),
        ];
        assert!(reads_of(&entries, "me", &HashMap::new()).seed().is_empty());
    }
}
