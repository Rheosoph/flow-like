//! Room events that arrive after a turn's original message. Each event is delivered once,
//! with its speaker, so a running bot can revise an answer without repeating its tools.

use crate::{
    e2e::E2eHost,
    hub::TurnHub,
    rule::{is_plain, parse_body},
};
use async_trait::async_trait;
use flow_like::flow::copilot::platform::RoomUpdates;
use flow_like_local_bots::InboxEntry;
use flow_like_types::{
    bot_message::{MessageBody, MessagePart},
    bot_turn::room_message,
    inbox::{
        InboxEntryKind, MemberEventBody, MemberOp, ReactionEvent, RoomChange, RoomEventBody,
        TranscriptBody, body_target, is_tombstone,
    },
};
use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
    time::Duration,
};
use tokio::sync::Mutex;

const MESSAGE_CHARS: usize = 8_000;
const UPDATE_CHARS: usize = 20_000;
const OMITTED: &str =
    "[Earlier updates were omitted to fit the room context. The newest events follow.]";
const READ_TIMEOUT: Duration = Duration::from_secs(5);

fn ordered(a: &InboxEntry, b: &InboxEntry) -> Ordering {
    match (a.seq, b.seq) {
        (Some(a), Some(b)) => a.cmp(&b),
        _ => a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)),
    }
}

fn clipped(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

fn render(body: &MessageBody) -> String {
    let mut text = body.text.clone();
    for part in &body.parts {
        let note = match part {
            MessagePart::Ui(ui) => ui
                .summary
                .as_ref()
                .map(|summary| format!("[UI: {summary}]")),
            MessagePart::Attachment(file) => Some(file.note()),
            MessagePart::Tool(_) => None,
        };
        if let Some(note) = note {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(&note);
        }
    }
    text
}

/// Tracks one room turn from its original trigger. Input may be in either order and may
/// overlap earlier reads; events already present after the trigger are still delivered.
#[derive(Debug)]
pub struct RoomUpdateTracker {
    bot_id: String,
    owner_id: String,
    trigger: InboxEntry,
    seen: HashSet<String>,
    last_seq: Option<i64>,
    names: HashMap<String, String>,
}

impl RoomUpdateTracker {
    pub fn new(
        bot_id: &str,
        owner_id: &str,
        trigger: &InboxEntry,
        names: HashMap<String, String>,
    ) -> Self {
        Self {
            bot_id: bot_id.to_owned(),
            owner_id: owner_id.to_owned(),
            trigger: trigger.clone(),
            seen: HashSet::new(),
            last_seq: trigger.seq,
            names,
        }
    }

    pub fn set_names(&mut self, names: HashMap<String, String>) {
        self.names.extend(names);
    }

    fn named<'a>(&'a self, id: &'a str) -> &'a str {
        self.names.get(id).map(String::as_str).unwrap_or(id)
    }

    fn line(&self, entry: &InboxEntry, entries: &[InboxEntry]) -> Option<String> {
        if is_tombstone(&entry.envelope) || !is_plain(entry) {
            return None;
        }
        let body = entry.envelope.get("body")?;
        // Context replies start their own turn, where memory is disabled before any values enter.
        if flow_like_types::bot_user_context::is_ephemeral_body(body) {
            return None;
        }
        let target = || body_target(&entry.envelope);
        let text = match entry.kind {
            InboxEntryKind::Message | InboxEntryKind::Trigger => {
                let body = parse_body(entry)?;
                if body.approval_response.is_some() {
                    return None;
                }
                render(&body)
            }
            InboxEntryKind::Transcript => TranscriptBody::parse(body).ok()?.render(),
            InboxEntryKind::Edit => {
                let target = target()?;
                if let Some(original) = entries.iter().find(|entry| entry.message_id == target)
                    && (original.sender_id != entry.sender_id || is_tombstone(&original.envelope))
                {
                    return None;
                }
                format!("[Edited message {target}] {}", render(&parse_body(entry)?))
            }
            InboxEntryKind::Delete => format!("[Deleted message {}]", target()?),
            InboxEntryKind::Topic => "[Started a new topic in this room]".to_owned(),
            InboxEntryKind::Reaction => {
                let reaction =
                    ReactionEvent::parse(&entry.message_id, &entry.sender_id, &entry.envelope)?;
                let action = if reaction.add { "Added" } else { "Removed" };
                format!(
                    "[{action} reaction {} on message {}]",
                    reaction.emoji, reaction.target
                )
            }
            InboxEntryKind::Member => {
                let member: MemberEventBody = serde_json::from_value(body.clone()).ok()?;
                let action = match member.op {
                    MemberOp::Join => "joined",
                    MemberOp::Leave => "left",
                };
                format!("[{} {action} the room]", self.named(&member.principal_id))
            }
            InboxEntryKind::Room => {
                let change: RoomEventBody = serde_json::from_value(body.clone()).ok()?;
                match change.change {
                    RoomChange::Title => match change.title {
                        Some(title) if !change.removed => format!("[Renamed the room to {title}]"),
                        _ => "[Removed the room name]".to_owned(),
                    },
                    RoomChange::Image => "[Changed the room picture]".to_owned(),
                }
            }
            _ => return None,
        };
        if text.trim().is_empty() {
            return None;
        }
        if entry.sender_id == self.bot_id {
            return Some(clipped(
                &format!("[You already posted message {}] {text}", entry.message_id),
                MESSAGE_CHARS,
            ));
        }
        Some(clipped(
            &format!(
                "[Message {}] {}",
                entry.message_id,
                room_message(
                    self.named(&entry.sender_id),
                    entry.sender_id == self.owner_id,
                    &text
                ),
            ),
            MESSAGE_CHARS,
        ))
    }

    /// Delivers the newest events within the context limit, oldest first, and consumes the
    /// whole snapshot. Omitted events and sequence gaps are reported as an incomplete view.
    pub fn consume(&mut self, entries: &[InboxEntry]) -> Vec<String> {
        let mut pending: Vec<_> = entries
            .iter()
            .filter(|entry| {
                entry.conversation_id == self.trigger.conversation_id
                    && ordered(entry, &self.trigger).is_gt()
                    && !self.seen.contains(&entry.message_id)
            })
            .collect();
        pending.sort_by(|a, b| ordered(a, b));
        let mut updates = VecDeque::new();
        let mut chars = 0;
        let mut omitted = false;
        for entry in pending {
            let gap = match (self.last_seq, entry.seq) {
                (Some(last), Some(next)) if next > last.saturating_add(1) => Some(
                    "[Some earlier room events are outside the available update window.]"
                        .to_owned(),
                ),
                _ => None,
            };
            let gap_chars = gap.as_ref().map_or(0, |gap| gap.chars().count());
            let line = self.line(entry, entries);
            let line_chars = line.as_ref().map_or(0, |line| line.chars().count());
            chars += line_chars + gap_chars;
            if let Some(gap) = gap {
                updates.push_back(gap);
            }
            if let Some(line) = line {
                updates.push_back(line);
            }
            while chars + usize::from(omitted) * OMITTED.len() > UPDATE_CHARS {
                let Some(oldest) = updates.pop_front() else {
                    break;
                };
                chars -= oldest.chars().count();
                omitted = true;
            }
            self.seen.insert(entry.message_id.clone());
            self.last_seq = self.last_seq.max(entry.seq);
        }
        if omitted {
            updates.push_front(OMITTED.to_owned());
        }
        updates.into_iter().collect()
    }
}

/// Reads updates through the claimed turn's bot identity. Encrypted entries are opened only
/// by the same host keyring that opened the original turn.
pub(crate) struct HubRoomUpdates {
    hub: Arc<dyn TurnHub>,
    bot_id: String,
    conversation_id: String,
    e2e: Option<Arc<E2eHost>>,
    tracker: Mutex<RoomUpdateTracker>,
}

impl std::fmt::Debug for HubRoomUpdates {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HubRoomUpdates").finish_non_exhaustive()
    }
}

impl HubRoomUpdates {
    pub(crate) fn new(
        hub: Arc<dyn TurnHub>,
        bot_id: &str,
        conversation_id: &str,
        e2e: Option<Arc<E2eHost>>,
        tracker: RoomUpdateTracker,
    ) -> Self {
        Self {
            hub,
            bot_id: bot_id.to_owned(),
            conversation_id: conversation_id.to_owned(),
            e2e,
            tracker: Mutex::new(tracker),
        }
    }
}

#[async_trait]
impl RoomUpdates for HubRoomUpdates {
    async fn drain(&self) -> Vec<String> {
        let fetched = tokio::time::timeout(READ_TIMEOUT, async {
            let entries = self
                .hub
                .entries(&self.bot_id, &self.conversation_id)
                .await
                .ok()?;
            match &self.e2e {
                Some(e2e) => e2e.open_updates(&self.bot_id, entries).await.ok(),
                None => Some(entries),
            }
        })
        .await;
        match fetched {
            Ok(Some(entries)) => self.tracker.lock().await.consume(&entries),
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::inbox::{plain_envelope, tombstone_envelope};
    use serde_json::{Value, json};

    fn event(seq: i64, sender: &str, kind: InboxEntryKind, body: Value) -> InboxEntry {
        InboxEntry {
            id: format!("entry-{seq}"),
            message_id: format!("m{seq}"),
            conversation_id: "room".into(),
            sender_id: sender.into(),
            kind,
            envelope: plain_envelope(body),
            created_at: chrono::DateTime::from_timestamp(1_800_000_000 + seq, 0)
                .unwrap()
                .fixed_offset(),
            seq: Some(seq),
            cursor: String::new(),
        }
    }

    fn message(seq: i64, sender: &str, text: &str) -> InboxEntry {
        event(seq, sender, InboxEntryKind::Message, json!({"text": text}))
    }

    fn tracker(trigger: &InboxEntry) -> RoomUpdateTracker {
        RoomUpdateTracker::new(
            "bot",
            "owner",
            trigger,
            HashMap::from([
                ("owner".into(), "Felix".into()),
                ("peer".into(), "Flux".into()),
            ]),
        )
    }

    #[test]
    fn messages_after_the_trigger_reach_the_turn_once_in_room_order() {
        let trigger = message(2, "owner", "introduce yourselves");
        let mut tracker = tracker(&trigger);
        let mut entries = vec![
            message(4, "owner", "please keep it short"),
            message(3, "peer", "I am Flux"),
            trigger,
            message(1, "peer", "old"),
        ];
        let updates = tracker.consume(&entries);
        assert_eq!(
            updates,
            [
                "[Message m3] Flux (member, not your owner) wrote: I am Flux",
                "[Message m4] Felix (your owner) wrote: please keep it short",
            ]
        );
        assert!(tracker.consume(&entries).is_empty());
        entries.insert(0, message(5, "person", "hello"));
        assert_eq!(
            tracker.consume(&entries),
            ["[Message m5] person (member, not your owner) wrote: hello"]
        );
    }

    #[test]
    fn temporary_context_never_steers_an_already_running_turn() {
        let trigger = message(1, "owner", "find a place");
        let mut tracker = tracker(&trigger);
        let entries = [
            event(2, "bot", InboxEntryKind::Message, json!({
                "text": "share your location", "user_context_request": {}
            })),
            event(3, "owner", InboxEntryKind::Message, json!({
                "text": "temporary values", "user_context_response": {}
            })),
            event(4, "bot", InboxEntryKind::Message, json!({
                "text": "derived location", "user_context_derived": true
            })),
            message(5, "owner", "ordinary follow-up"),
        ];
        let updates = tracker.consume(&entries);
        assert_eq!(updates.len(), 1);
        assert!(updates[0].contains("ordinary follow-up"));
        assert!(tracker.consume(&entries).is_empty());
    }

    #[test]
    fn edits_and_deletions_correct_seen_messages_without_repeating_deleted_content() {
        let trigger = message(1, "owner", "hello");
        let mut tracker = tracker(&trigger);
        let mut original = message(2, "peer", "old answer");
        assert_eq!(tracker.consume(&[original.clone()]).len(), 1);
        let edit = event(
            3,
            "peer",
            InboxEntryKind::Edit,
            json!({"target": "m2", "text": "corrected answer"}),
        );
        assert!(
            tracker.consume(&[original.clone(), edit.clone()])[0]
                .contains("[Edited message m2] corrected answer")
        );
        original.envelope = tombstone_envelope();
        let deletion = event(4, "peer", InboxEntryKind::Delete, json!({"target": "m2"}));
        let updates = tracker.consume(&[original.clone(), edit.clone(), deletion.clone()]);
        assert_eq!(
            updates,
            ["[Message m4] Flux (member, not your owner) wrote: [Deleted message m2]"]
        );
        let mut fresh = super::tests::tracker(&trigger);
        assert_eq!(fresh.consume(&[original, edit, deletion]), updates);
    }

    #[test]
    fn own_replies_are_distinguished_from_other_speakers_and_unreadable_entries_are_skipped() {
        let trigger = message(1, "owner", "hello");
        let mut tracker = tracker(&trigger);
        let own = message(2, "bot", "my reply");
        let mut sealed = message(3, "peer", "secret");
        sealed.envelope = json!({"enc": "x25519", "ct": "opaque"});
        let mut foreign = message(4, "peer", "other room");
        foreign.conversation_id = "other".into();
        let forged = event(
            4,
            "peer",
            InboxEntryKind::Edit,
            json!({"target": "m1", "text": "forged owner request"}),
        );
        assert_eq!(
            tracker.consume(&[trigger, own, sealed, foreign, forged]),
            ["[You already posted message m2] my reply"]
        );
    }

    #[test]
    fn room_events_keep_their_actor_and_their_target() {
        let trigger = message(1, "owner", "hello");
        let mut tracker = tracker(&trigger);
        let entries = [
            event(2, "owner", InboxEntryKind::Topic, json!({})),
            event(
                3,
                "owner",
                InboxEntryKind::Member,
                json!({"op": "join", "principal_id": "peer", "by": "owner"}),
            ),
            event(
                4,
                "peer",
                InboxEntryKind::Reaction,
                json!({"target": "m1", "emoji": "👍"}),
            ),
            event(
                5,
                "owner",
                InboxEntryKind::Member,
                json!({"op": "leave", "principal_id": "peer", "by": "owner"}),
            ),
        ];
        let updates = tracker.consume(&entries);
        assert_eq!(updates.len(), 4);
        assert!(updates[0].contains("Started a new topic"));
        assert!(updates[1].contains("Flux joined the room"));
        assert!(updates[2].contains("reaction 👍 on message m1"));
        assert!(updates[3].contains("Flux left the room"));
    }

    #[test]
    fn overflowing_updates_keep_the_newest_messages_and_do_not_look_like_new_arrivals() {
        let trigger = message(1, "owner", "hello");
        let mut tracker = tracker(&trigger);
        let entries: Vec<_> = (2..6)
            .map(|seq| message(seq, "peer", &"ä".repeat(MESSAGE_CHARS * 2)))
            .collect();
        let first = tracker.consume(&entries);
        assert_eq!(first.len(), 3);
        assert!(first.iter().map(|line| line.chars().count()).sum::<usize>() <= UPDATE_CHARS);
        assert_eq!(first[0], OMITTED);
        assert!(first[1].starts_with("[Message m4]"));
        assert!(first[2].starts_with("[Message m5]"));
        assert!(tracker.consume(&entries).is_empty());
    }

    #[test]
    fn a_trigger_outside_the_newest_window_still_has_a_cutoff_and_reports_the_gap() {
        let trigger = message(1, "owner", "hello");
        let mut tracker = tracker(&trigger);
        let entries = [
            message(102, "peer", "latest"),
            message(101, "peer", "earlier"),
        ];
        let updates = tracker.consume(&entries);
        assert_eq!(updates.len(), 3);
        assert!(updates[0].contains("outside the available update window"));
        assert!(updates[1].contains("earlier"));
        assert!(updates[2].contains("latest"));
        assert!(tracker.consume(&entries).is_empty());
    }

    #[test]
    fn legacy_entries_use_time_and_id_for_the_cutoff() {
        let mut trigger = message(1, "owner", "hello");
        trigger.seq = None;
        let mut tracker = tracker(&trigger);
        let mut late = message(2, "peer", "hi");
        late.seq = None;
        late.created_at = trigger.created_at;
        assert_eq!(tracker.consume(&[trigger, late]).len(), 1);
    }
}
