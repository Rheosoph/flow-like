//! A local chat's history for the webview and any other caller: the chat read by position,
//! "New topic" markers and the summaries (segments) a bot's host writes. The rules are the
//! store's, which mirror the account inbox's; every command rejects with `{ code, message }`.

use super::{CommandError, LocalBots, on_store};
use flow_like_local_bots::{
    ConversationSegment, ConversationSegmentList, InboxEntriesPage, InboxEntry, LOCAL_OWNER_ID,
    PositionQuery, SegmentsQuery, SendEntryRequest, WriteSegmentRequest,
};
use flow_like_types::inbox::{InboxEntryKind, plain_envelope};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::State;

/// One message of a bot's earlier chat, moved into its conversation.
#[derive(Debug, Clone, Deserialize)]
pub struct ImportedMessage {
    pub message_id: String,
    /// `owner` or `bot`.
    pub sender: String,
    pub envelope: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportedHistory {
    pub imported: u64,
    pub skipped: u64,
}

async fn import_history(
    bots: &LocalBots,
    conversation_id: String,
    bot_id: String,
    messages: Vec<ImportedMessage>,
) -> Result<ImportedHistory, CommandError> {
    on_store(bots, move |store| {
        let mut done = ImportedHistory {
            imported: 0,
            skipped: 0,
        };
        for message in messages {
            let sender = if message.sender == "bot" {
                bot_id.as_str()
            } else {
                LOCAL_OWNER_ID
            };
            let existed = store.has_message(&message.message_id)?;
            store.send(
                &conversation_id,
                sender,
                SendEntryRequest {
                    message_id: message.message_id,
                    kind: InboxEntryKind::Message,
                    envelope: Some(message.envelope),
                    envelopes: None,
                },
            )?;
            if existed {
                done.skipped += 1;
            } else {
                done.imported += 1;
            }
        }
        Ok(done)
    })
    .await
}

async fn read_by_position(
    bots: &LocalBots,
    conversation_id: String,
    query: PositionQuery,
) -> Result<InboxEntriesPage, CommandError> {
    on_store(bots, move |store| {
        store.entries_by_position(&conversation_id, &query)
    })
    .await
}

async fn send_topic(
    bots: &LocalBots,
    conversation_id: String,
    message_id: String,
    title: Option<String>,
) -> Result<InboxEntry, CommandError> {
    let body = match title {
        Some(title) => json!({ "title": title }),
        None => json!({}),
    };
    let request = SendEntryRequest {
        message_id,
        kind: InboxEntryKind::Topic,
        envelope: Some(plain_envelope(body)),
        envelopes: None,
    };
    on_store(bots, move |store| {
        store.send(&conversation_id, LOCAL_OWNER_ID, request)
    })
    .await
}

async fn list_segments(
    bots: &LocalBots,
    conversation_id: String,
    query: SegmentsQuery,
) -> Result<ConversationSegmentList, CommandError> {
    on_store(bots, move |store| store.segments(&conversation_id, &query)).await
}

async fn write_segment(
    bots: &LocalBots,
    conversation_id: String,
    author_id: String,
    request: WriteSegmentRequest,
) -> Result<ConversationSegment, CommandError> {
    on_store(bots, move |store| {
        store.write_segment(&conversation_id, &author_id, request)
    })
    .await
}

/// The chat's events by position, oldest first: `{ from_seq, to_seq, around_seq,
/// before_count, after_count, limit }` as in the account inbox's entries query. `next_from_seq`
/// and `next_to_seq` name where to continue.
#[tauri::command]
pub async fn local_inbox_entries_by_position(
    state: State<'_, LocalBots>,
    conversation_id: String,
    query: PositionQuery,
) -> Result<InboxEntriesPage, CommandError> {
    read_by_position(&state, conversation_id, query).await
}

/// Starts the chat's next segment: a "New topic" marker, optionally titled. Sending the same
/// `messageId` again returns the first marker.
#[tauri::command]
pub async fn local_inbox_topic(
    state: State<'_, LocalBots>,
    conversation_id: String,
    message_id: String,
    title: Option<String>,
) -> Result<InboxEntry, CommandError> {
    send_topic(&state, conversation_id, message_id, title).await
}

/// The chat's summaries, the newest version of each range, oldest first. `{ author, level,
/// covering, after, limit }`; a direct chat lists its bot's unless `author` names another.
#[tauri::command]
pub async fn local_inbox_segments(
    state: State<'_, LocalBots>,
    conversation_id: String,
    query: SegmentsQuery,
) -> Result<ConversationSegmentList, CommandError> {
    list_segments(&state, conversation_id, query).await
}

/// Stores the summary bot `authorId`'s host made, written once: the same id and content
/// returns the first write, a changed summary goes under a new id as the range's next version.
#[tauri::command]
pub async fn local_inbox_write_segment(
    state: State<'_, LocalBots>,
    conversation_id: String,
    author_id: String,
    request: WriteSegmentRequest,
) -> Result<ConversationSegment, CommandError> {
    write_segment(&state, conversation_id, author_id, request).await
}

/// Moves a bot's earlier chat into its conversation, oldest first: `owner` messages as the
/// person, `bot` ones as `botId`. A message id that is already stored is skipped.
#[tauri::command]
pub async fn local_inbox_import_history(
    state: State<'_, LocalBots>,
    conversation_id: String,
    bot_id: String,
    entries: Vec<ImportedMessage>,
) -> Result<ImportedHistory, CommandError> {
    import_history(&state, conversation_id, bot_id, entries).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_local_bots::{
        BotMover, ChangeNotifier, KeyError, LocalBotStore, SecretKeyProvider, codes,
    };
    use serde::Serialize;
    use serde_json::Value;
    use std::{
        path::PathBuf,
        sync::{
            Arc,
            atomic::{AtomicU32, Ordering},
        },
    };
    use zeroize::Zeroizing;

    struct NoKey;

    impl SecretKeyProvider for NoKey {
        fn key(&self, _create: bool) -> Result<Zeroizing<[u8; 32]>, KeyError> {
            Err(KeyError::Missing("no key in these tests".into()))
        }
    }

    struct Quiet;

    impl ChangeNotifier for Quiet {
        fn bots_changed(&self, _bot_id: Option<&str>) {}
        fn inbox_changed(&self, _conversation_id: Option<&str>) {}
    }

    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            static NEXT: AtomicU32 = AtomicU32::new(0);
            let dir = std::env::temp_dir().join(format!(
                "flow-like-local-history-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn open(scratch: &Scratch) -> (LocalBots, Arc<LocalBotStore>) {
        let store = Arc::new(
            LocalBotStore::open(
                scratch.0.join("local_bots.db"),
                Arc::new(NoKey),
                Arc::new(Quiet),
            )
            .unwrap(),
        );
        let bots = LocalBots {
            store: Ok(store.clone()),
            mover: BotMover::default(),
        };
        (bots, store)
    }

    fn unavailable() -> LocalBots {
        LocalBots {
            store: Err(CommandError::new(
                codes::STORAGE,
                "Bots on this device are unavailable: the file is locked",
            )),
            mover: BotMover::default(),
        }
    }

    fn shape<T: Serialize>(result: Result<T, CommandError>) -> Value {
        serde_json::to_value(result.err().expect("the command rejects")).unwrap()
    }

    fn rejected_with<T: Serialize>(result: Result<T, CommandError>, code: &str) -> String {
        let error = shape(result);
        let keys: Vec<&str> = error
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys.len(),
            2,
            "a rejection is exactly code and message: {error}"
        );
        assert_eq!(error["code"], code, "{error}");
        error["message"].as_str().unwrap().to_string()
    }

    fn summary(id: &str, from: i64, to: i64) -> WriteSegmentRequest {
        serde_json::from_value(json!({
            "id": id, "level": 0, "seq_from": from, "seq_to": to, "summary": "Planning"
        }))
        .unwrap()
    }

    fn chat(store: &LocalBotStore) -> (String, String) {
        let bot = store
            .create_bot(
                serde_json::from_value(json!({ "name": "Ada", "appearance": { "kind": "pip" } }))
                    .unwrap(),
            )
            .unwrap();
        let conversation = store
            .create_conversation(
                serde_json::from_value(json!({ "kind": "direct", "members": [bot.id] })).unwrap(),
            )
            .unwrap();
        (conversation.id, bot.id)
    }

    #[tokio::test]
    async fn an_unavailable_store_rejects_every_command_with_the_reason() {
        let bots = unavailable();
        let reason = "Bots on this device are unavailable: the file is locked";
        let query = || PositionQuery {
            from_seq: Some(1),
            ..PositionQuery::default()
        };
        let read = read_by_position(&bots, "c".into(), query()).await;
        assert_eq!(rejected_with(read, codes::STORAGE), reason);
        let topic = send_topic(&bots, "c".into(), "t_1".into(), None).await;
        assert_eq!(rejected_with(topic, codes::STORAGE), reason);
        let list = list_segments(&bots, "c".into(), SegmentsQuery::default()).await;
        assert_eq!(rejected_with(list, codes::STORAGE), reason);
        let write = write_segment(&bots, "c".into(), "bot".into(), summary("s", 1, 1)).await;
        assert_eq!(rejected_with(write, codes::STORAGE), reason);
    }

    #[tokio::test]
    async fn reads_by_position_answer_like_the_accounts_and_reject_with_code_and_message() {
        let scratch = Scratch::new();
        let (bots, store) = open(&scratch);
        let (conversation, _) = chat(&store);
        for n in 1..=4 {
            send_topic(
                &bots,
                conversation.clone(),
                format!("t_{n}"),
                Some(format!("T{n}")),
            )
            .await
            .unwrap();
        }
        let page = read_by_position(
            &bots,
            conversation.clone(),
            serde_json::from_value(json!({ "around_seq": 2, "before_count": 1, "after_count": 1 }))
                .unwrap(),
        )
        .await
        .unwrap();
        let wire = serde_json::to_value(&page).unwrap();
        assert_eq!(
            wire["entries"]
                .as_array()
                .unwrap()
                .iter()
                .map(|entry| entry["seq"].as_i64().unwrap())
                .collect::<Vec<_>>(),
            [1, 2, 3]
        );
        assert_eq!(wire["next_from_seq"], 4);
        assert_eq!(wire["next_to_seq"], Value::Null);
        assert_eq!(wire["next_before"], Value::Null);

        let backwards = PositionQuery {
            from_seq: Some(4),
            to_seq: Some(2),
            ..PositionQuery::default()
        };
        let message = rejected_with(
            read_by_position(&bots, conversation.clone(), backwards).await,
            codes::INVALID,
        );
        assert!(
            message.contains("from_seq 4 comes after to_seq 2"),
            "{message}"
        );
        let nothing = read_by_position(&bots, conversation.clone(), PositionQuery::default());
        rejected_with(nothing.await, codes::INVALID);
        let missing = PositionQuery {
            from_seq: Some(1),
            ..PositionQuery::default()
        };
        let message = rejected_with(
            read_by_position(&bots, "nope".into(), missing).await,
            codes::NOT_FOUND,
        );
        assert!(message.contains("'nope'"), "{message}");
    }

    fn imported(id: &str, sender: &str, text: &str) -> ImportedMessage {
        ImportedMessage {
            message_id: id.into(),
            sender: sender.into(),
            envelope: json!({ "v": 1, "enc": "none", "body": { "text": text } }),
        }
    }

    #[tokio::test]
    async fn an_earlier_chat_imports_once_as_the_person_and_the_bot() {
        let scratch = Scratch::new();
        let (bots, store) = open(&scratch);
        let (conversation, bot) = chat(&store);
        let messages = || {
            vec![
                imported("old_1", "owner", "Hello"),
                imported("old_2", "bot", "Hi, how can I help?"),
            ]
        };

        let first = import_history(&bots, conversation.clone(), bot.clone(), messages())
            .await
            .unwrap();
        assert_eq!((first.imported, first.skipped), (2, 0));
        let again = import_history(&bots, conversation.clone(), bot.clone(), messages())
            .await
            .unwrap();
        assert_eq!((again.imported, again.skipped), (0, 2));

        let page = read_by_position(
            &bots,
            conversation,
            serde_json::from_value(json!({ "from_seq": 1, "limit": 10 })).unwrap(),
        )
        .await
        .unwrap();
        let senders: Vec<(&str, &str)> = page
            .entries
            .iter()
            .map(|entry| (entry.message_id.as_str(), entry.sender_id.as_str()))
            .collect();
        assert_eq!(
            senders,
            [("old_1", LOCAL_OWNER_ID), ("old_2", bot.as_str())]
        );
    }

    #[tokio::test]
    async fn a_topic_marker_takes_the_next_position_and_a_retry_returns_the_first() {
        let scratch = Scratch::new();
        let (bots, store) = open(&scratch);
        let (conversation, _) = chat(&store);
        let first = send_topic(
            &bots,
            conversation.clone(),
            "t_1".into(),
            Some("Launch".into()),
        )
        .await
        .unwrap();
        assert_eq!((first.kind.as_str(), first.seq), ("topic", Some(1)));
        assert_eq!(first.sender_id, LOCAL_OWNER_ID);
        assert_eq!(
            first.envelope,
            json!({ "v": 1, "enc": "none", "body": { "title": "Launch" } })
        );
        let untitled = send_topic(&bots, conversation.clone(), "t_2".into(), None)
            .await
            .unwrap();
        assert_eq!(untitled.envelope["body"], json!({}));
        let again = send_topic(
            &bots,
            conversation.clone(),
            "t_1".into(),
            Some("Launch".into()),
        )
        .await
        .unwrap();
        assert_eq!(again, first);

        let long = send_topic(
            &bots,
            conversation.clone(),
            "t_3".into(),
            Some("x".repeat(121)),
        );
        let message = rejected_with(long.await, codes::INVALID);
        assert!(message.contains("121"), "{message}");
        let bad_id = send_topic(&bots, conversation.clone(), "not an id".into(), None);
        rejected_with(bad_id.await, codes::INVALID);
        let missing = send_topic(&bots, "nope".into(), "t_4".into(), None);
        rejected_with(missing.await, codes::NOT_FOUND);
    }

    #[tokio::test]
    async fn segments_are_written_once_listed_by_version_and_rejected_with_code_and_message() {
        let scratch = Scratch::new();
        let (bots, store) = open(&scratch);
        let (conversation, bot) = chat(&store);
        for n in 1..=3 {
            send_topic(&bots, conversation.clone(), format!("t_{n}"), None)
                .await
                .unwrap();
        }
        let first = write_segment(
            &bots,
            conversation.clone(),
            bot.clone(),
            summary("s1", 1, 3),
        )
        .await
        .unwrap();
        assert_eq!((first.version, first.author_id.as_str()), (1, bot.as_str()));
        let again = write_segment(
            &bots,
            conversation.clone(),
            bot.clone(),
            summary("s1", 1, 3),
        )
        .await
        .unwrap();
        assert_eq!(again, first);
        let mut second = summary("s2", 1, 3);
        second.summary = "Revised".into();
        let second = write_segment(&bots, conversation.clone(), bot.clone(), second)
            .await
            .unwrap();
        assert_eq!(second.version, 2);
        let list = list_segments(&bots, conversation.clone(), SegmentsQuery::default())
            .await
            .unwrap();
        assert_eq!(
            list.segments
                .iter()
                .map(|s| s.id.as_str())
                .collect::<Vec<_>>(),
            ["s2"]
        );
        assert_eq!(list.next_after, None);

        let mut taken = summary("s1", 1, 3);
        taken.summary = "Other".into();
        rejected_with(
            write_segment(&bots, conversation.clone(), bot.clone(), taken).await,
            codes::INBOX_SEGMENT_ID_TAKEN,
        );
        rejected_with(
            write_segment(
                &bots,
                conversation.clone(),
                bot.clone(),
                summary("s3", 1, 9),
            )
            .await,
            codes::INBOX_SEGMENT_RANGE,
        );
        let mut orphan = summary("s4", 1, 2);
        orphan.parent_id = Some("missing".into());
        rejected_with(
            write_segment(&bots, conversation.clone(), bot.clone(), orphan).await,
            codes::INBOX_SEGMENT_PARENT,
        );
        rejected_with(
            write_segment(
                &bots,
                conversation.clone(),
                bot.clone(),
                summary("a b", 1, 2),
            )
            .await,
            codes::INVALID,
        );
        rejected_with(
            write_segment(
                &bots,
                conversation.clone(),
                "nobody".into(),
                summary("s5", 1, 2),
            )
            .await,
            codes::NOT_FOUND,
        );
        rejected_with(
            write_segment(&bots, "nope".into(), bot.clone(), summary("s5", 1, 2)).await,
            codes::NOT_FOUND,
        );
        let bad_level = SegmentsQuery {
            level: Some(10),
            ..SegmentsQuery::default()
        };
        rejected_with(
            list_segments(&bots, conversation.clone(), bad_level).await,
            codes::INVALID,
        );
        rejected_with(
            list_segments(&bots, "nope".into(), SegmentsQuery::default()).await,
            codes::NOT_FOUND,
        );
    }

    #[test]
    fn the_webview_sends_queries_as_plain_snake_case_objects() {
        let position: PositionQuery = serde_json::from_value(json!({
            "from_seq": 4, "limit": 3
        }))
        .unwrap();
        assert_eq!(
            (position.from_seq, position.limit, position.around_seq),
            (Some(4), Some(3), None)
        );
        let segments: SegmentsQuery = serde_json::from_value(json!({
            "author": "bot_1", "covering": 7, "after": "3:0:5:bot_1"
        }))
        .unwrap();
        assert_eq!(
            (
                segments.author.as_deref(),
                segments.covering,
                segments.level
            ),
            (Some("bot_1"), Some(7), None)
        );
        assert_eq!(
            serde_json::from_value::<SegmentsQuery>(json!({})).unwrap(),
            SegmentsQuery::default()
        );
    }
}
