//! Bots on this desktop asking each other. The crew is the local store's bots (Assistant and
//! Flux rows included), grants live in its `permissions` table, an exchange is a group the
//! person and the two bots belong to, and the callee's turn runs through `global_chat_native`
//! like any other headless bot turn. Account bots join through the hub later.

use std::sync::Arc;

use async_trait::async_trait;
use flow_like::{
    copilot::{CopilotScope, UnifiedChatMessage},
    flow::copilot::{
        BotRunContext, FLUX_BOT_NAME, GENERAL_BOT_DESCRIPTION, GENERAL_BOT_NAME,
        bot_talk::{
            AnsweredRound, BotPresence, BotTalk, BotTalkHost, CrewMember, HeadlessPermission,
            NestedBotTurn,
        },
    },
};
use flow_like_local_bots::{
    Bot, CreateConversationRequest, EntriesQuery, LOCAL_OWNER_ID, LocalBotStore, SendEntryRequest,
};
use flow_like_types::{
    anyhow,
    bot::{BotLocation, BotSystemKind},
    create_id,
    inbox::{ActivityBody, ConversationKind, InboxEntryKind, is_tombstone, plain_envelope},
};
use serde_json::json;
use tauri::{AppHandle, Manager, ipc::Channel};

use crate::{
    bot_host::ApprovalGate,
    functions::{
        ai::native_tool_bridge::{NativeRunOptions, with_execution_profile},
        local_bots::{LocalBots, on_store},
    },
    state::TauriFlowLikeState,
};

const FLUX_DESCRIPTION: &str = "Builds, edits and debugs your apps and flows";
const EXCHANGE_TITLE_SEPARATOR: &str = " ↔ ";
const HISTORY_MESSAGES: usize = 40;
const HISTORY_PAGE_SIZE: u64 = 100;

/// The bot talk of a run on this desktop: the asking bot's own when another bot's ask started
/// the run, else a fresh one that goes on counting from `answered` when a late answer woke the
/// run. `None` for a run the local store can't serve (no store, or the bot lives in the account).
pub(super) async fn talk_for(
    app: &AppHandle,
    bot: &BotRunContext,
    nested: Option<Arc<BotTalk>>,
    answered: Option<&AnsweredRound>,
    gate: Option<Arc<ApprovalGate>>,
    profile_id: &str,
    run_id: &str,
) -> Option<Arc<BotTalk>> {
    if nested.is_some() {
        return nested;
    }
    let host: Arc<dyn BotTalkHost> = if is_local(app, bot).await {
        Arc::new(DesktopTalkHost {
            app: app.clone(),
            profile_id: profile_id.to_string(),
            gate,
        })
    } else {
        Arc::new(crate::bot_host::talk::AccountTalkHost::new(
            app.clone(),
            run_id,
            gate,
        ))
    };
    Some(Arc::new(
        BotTalk::for_run(host, bot, BotLocation::Desktop).with_answered(answered),
    ))
}

/// Whether the bot has a row in this device's store; any other bot is the account's.
async fn is_local(app: &AppHandle, bot: &BotRunContext) -> bool {
    let Some(bots) = app.try_state::<LocalBots>() else {
        return false;
    };
    let member = CrewMember {
        id: bot.bot_id.clone(),
        system: bot.system,
        ..CrewMember::default()
    };
    on_store(&bots, move |store| row_id(store, &member))
        .await
        .ok()
        .flatten()
        .is_some()
}

struct DesktopTalkHost {
    app: AppHandle,
    profile_id: String,
    gate: Option<Arc<ApprovalGate>>,
}

impl DesktopTalkHost {
    async fn store<T, F>(&self, work: F) -> flow_like_types::Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&LocalBotStore) -> flow_like_local_bots::Result<T> + Send + 'static,
    {
        let bots = self
            .app
            .try_state::<LocalBots>()
            .ok_or_else(|| anyhow!("Bots on this device are unavailable"))?;
        on_store(&bots, work)
            .await
            .map_err(|error| anyhow!("{error}"))
    }
}

impl DesktopTalkHost {
    /// The owner's "Always allow" on an ask card becomes the stored permission between the two
    /// bots, and the card's grant is spent.
    async fn settle_card_answer(
        &self,
        from: &CrewMember,
        to: &CrewMember,
    ) -> flow_like_types::Result<bool> {
        match &self.gate {
            Some(gate) => gate.settle_ask_card(to, self.store_grant(from, to)).await,
            None => Ok(false),
        }
    }
}

fn unknown_bot(member: &CrewMember) -> flow_like_types::Error {
    anyhow!("'{}' is not a bot on this computer", member.name)
}

#[async_trait]
impl BotTalkHost for DesktopTalkHost {
    async fn roster(&self, caller: &CrewMember) -> flow_like_types::Result<Vec<CrewMember>> {
        let caller = caller.clone();
        self.store(move |store| roster_of(store, &caller))
            .await?
            .ok_or_else(|| anyhow!("The asking bot isn't a bot on this computer"))
    }

    async fn has_grant(&self, from: &CrewMember, to: &CrewMember) -> flow_like_types::Result<bool> {
        let (from_member, to_member) = (from.clone(), to.clone());
        let stored = self
            .store(move |store| match rows(store, &from_member, &to_member)? {
                Some((from, to)) => store.has_permission(&from, &to),
                None => Ok(false),
            })
            .await?;
        if stored {
            return Ok(true);
        }
        self.settle_card_answer(from, to).await
    }

    async fn headless_permission(
        &self,
        caller: &CrewMember,
        callee: &CrewMember,
        question: &str,
    ) -> HeadlessPermission {
        match &self.gate {
            Some(gate) => gate.ask_bot(caller, callee, question).await,
            None => HeadlessPermission::Unavailable,
        }
    }

    async fn store_grant(&self, from: &CrewMember, to: &CrewMember) -> flow_like_types::Result<()> {
        let (from_member, to_member) = (from.clone(), to.clone());
        self.store(move |store| match rows(store, &from_member, &to_member)? {
            Some((from, to)) => store.grant_permission(&from, &to).map(|_| true),
            None => Ok(false),
        })
        .await?
        .then_some(())
        .ok_or_else(|| unknown_bot(to))
    }

    async fn exchange_conversation(
        &self,
        caller: &CrewMember,
        callee: &CrewMember,
    ) -> flow_like_types::Result<String> {
        let (caller_member, callee_member) = (caller.clone(), callee.clone());
        self.store(move |store| exchange_of(store, &caller_member, &callee_member))
            .await?
            .ok_or_else(|| unknown_bot(callee))
    }

    async fn record(
        &self,
        conversation_id: &str,
        sender: &CrewMember,
        text: &str,
    ) -> flow_like_types::Result<String> {
        let (conversation_id, member, text) = (
            conversation_id.to_string(),
            sender.clone(),
            text.to_string(),
        );
        self.store(move |store| record_in(store, &conversation_id, &member, &text))
            .await?
            .ok_or_else(|| unknown_bot(sender))
    }

    async fn record_question(
        &self,
        conversation_id: &str,
        caller: &CrewMember,
        question: &str,
        _round: u32,
    ) -> flow_like_types::Result<String> {
        let (conversation_id, member, question) = (
            conversation_id.to_string(),
            caller.clone(),
            question.to_string(),
        );
        self.store(move |store| record_question_in(store, &conversation_id, &member, &question))
            .await?
            .ok_or_else(|| unknown_bot(caller))
    }

    async fn run_turn(&self, turn: NestedBotTurn) -> flow_like_types::Result<String> {
        let (conversation_id, question_id, callee) = (
            turn.conversation_id.clone(),
            turn.question_message_id.clone(),
            turn.callee.id.clone(),
        );
        let history = self
            .store(move |store| history_before(store, &conversation_id, &question_id, &callee))
            .await?;
        let callee_name = turn.callee.name.clone();
        let task = tokio::spawn(answer_turn(
            self.app.clone(),
            self.profile_id.clone(),
            turn,
            history,
        ));
        let _abort = AbortOnDrop(task.abort_handle());
        task.await
            .map_err(|error| anyhow!("{callee_name}'s turn ended without an answer: {error}"))?
    }
}

struct AbortOnDrop(tokio::task::AbortHandle);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// The callee's turn, spawned so it starts on a fresh stack: polled inline it stacks a whole
/// second chat turn on the asker's, which overflows the worker's 2 MiB (a debug build already on
/// the first ask, any build once enough bots ask down the chain).
async fn answer_turn(
    app: AppHandle,
    profile_id: String,
    turn: NestedBotTurn,
    history: Vec<UnifiedChatMessage>,
) -> flow_like_types::Result<String> {
    let state = app
        .try_state::<TauriFlowLikeState>()
        .ok_or_else(|| anyhow!("The desktop is still starting"))?;
    let run_id = format!("bot-talk-{}", create_id());
    let options = NativeRunOptions {
        profile_id: Some(profile_id.clone()),
        conversation_id: Some(run_id.clone()),
        bot_talk: Some(turn.talk),
        ..Default::default()
    };
    let response = with_execution_profile(
        profile_id,
        Box::pin(crate::functions::ai::copilot::global_chat_native(
            app.clone(),
            None,
            state,
            CopilotScope::Both,
            turn.question,
            None,
            Some(history),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(run_id),
            Channel::<String>::new(|_| Ok(())),
            options,
            Some(turn.callee.id),
        )),
    )
    .await
    .map_err(|error| anyhow!("{error}"))?;
    Ok(response.message)
}

fn default_name(system: Option<BotSystemKind>, id: &str) -> String {
    match system {
        Some(BotSystemKind::General) => GENERAL_BOT_NAME.to_string(),
        Some(BotSystemKind::Flux) => FLUX_BOT_NAME.to_string(),
        None => id.to_string(),
    }
}

fn default_description(system: Option<BotSystemKind>) -> Option<String> {
    match system {
        Some(BotSystemKind::General) => Some(GENERAL_BOT_DESCRIPTION.to_string()),
        Some(BotSystemKind::Flux) => Some(FLUX_DESCRIPTION.to_string()),
        None => None,
    }
}

fn crew_member(bot: Bot) -> CrewMember {
    CrewMember {
        name: bot
            .name
            .clone()
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| default_name(bot.system, &bot.id)),
        description: bot
            .description
            .clone()
            .filter(|text| !text.trim().is_empty())
            .or_else(|| default_description(bot.system)),
        id: bot.id,
        system: bot.system,
        location: BotLocation::Desktop,
        presence: BotPresence::Online,
        classification: bot.classification,
    }
}

/// The store row `member` stands for: its own id, else the built-in row of its kind (a run of
/// Assistant without a row carries the id `general`).
fn row_id(
    store: &LocalBotStore,
    member: &CrewMember,
) -> flow_like_local_bots::Result<Option<String>> {
    if store.get_bot(&member.id)?.is_some() {
        return Ok(Some(member.id.clone()));
    }
    let Some(kind) = member.system else {
        return Ok(None);
    };
    store.ensure_built_ins()?;
    Ok(store.built_in_bot(kind)?.map(|bot| bot.id))
}

fn rows(
    store: &LocalBotStore,
    from: &CrewMember,
    to: &CrewMember,
) -> flow_like_local_bots::Result<Option<(String, String)>> {
    Ok(row_id(store, from)?.zip(row_id(store, to)?))
}

/// Every other bot on this device, built-ins first; `None` when `caller` isn't one of them.
fn roster_of(
    store: &LocalBotStore,
    caller: &CrewMember,
) -> flow_like_local_bots::Result<Option<Vec<CrewMember>>> {
    store.ensure_built_ins()?;
    let Some(caller_row) = row_id(store, caller)? else {
        return Ok(None);
    };
    Ok(Some(
        store
            .list_bots()?
            .into_iter()
            .filter(|bot| bot.id != caller_row)
            .map(crew_member)
            .collect(),
    ))
}

/// The group of the person and the two bots, made on their first exchange.
fn exchange_of(
    store: &LocalBotStore,
    caller: &CrewMember,
    callee: &CrewMember,
) -> flow_like_local_bots::Result<Option<String>> {
    let Some((first, second)) = rows(store, caller, callee)? else {
        return Ok(None);
    };
    let mut pair = [(first, caller.name.clone()), (second, callee.name.clone())];
    pair.sort();
    let wanted = [LOCAL_OWNER_ID, pair[0].0.as_str(), pair[1].0.as_str()];
    let existing = store.conversations()?.into_iter().find(|conversation| {
        conversation.kind == ConversationKind::Group
            && conversation
                .title
                .as_deref()
                .is_some_and(|title| title.contains(EXCHANGE_TITLE_SEPARATOR))
            && conversation.members.len() == wanted.len()
            && wanted.iter().all(|id| {
                conversation
                    .members
                    .iter()
                    .any(|member| member.principal_id == *id)
            })
    });
    if let Some(conversation) = existing {
        return Ok(Some(conversation.id));
    }
    let conversation = store.create_conversation(CreateConversationRequest {
        kind: ConversationKind::Group,
        title: Some(format!(
            "{}{EXCHANGE_TITLE_SEPARATOR}{}",
            pair[0].1, pair[1].1
        )),
        members: vec![pair[0].0.clone(), pair[1].0.clone()],
        encryption: None,
        bot_limits: Default::default(),
    })?;
    Ok(Some(conversation.id))
}

/// Saves `text` in the exchange as `sender`'s message; returns its message id.
fn record_in(
    store: &LocalBotStore,
    conversation_id: &str,
    sender: &CrewMember,
    text: &str,
) -> flow_like_local_bots::Result<Option<String>> {
    let Some(sender_row) = row_id(store, sender)? else {
        return Ok(None);
    };
    let entry = store.send(
        conversation_id,
        &sender_row,
        SendEntryRequest {
            message_id: format!("bot-talk-{}", create_id()),
            kind: InboxEntryKind::Message,
            envelope: Some(plain_envelope(json!({ "text": text }))),
            envelopes: None,
        },
    )?;
    Ok(Some(entry.message_id))
}

/// The recipient belongs to this exchange. A nested ask must not change the recipient used
/// when the parent resumes, and recording an answer must not create another ask activity.
fn record_question_in(
    store: &LocalBotStore,
    conversation_id: &str,
    caller: &CrewMember,
    question: &str,
) -> flow_like_local_bots::Result<Option<String>> {
    let Some(caller_row) = row_id(store, caller)? else {
        return Ok(None);
    };
    let callee_id = store
        .conversation(conversation_id)?
        .and_then(|conversation| {
            conversation.members.into_iter().find_map(|member| {
                (member.principal_id != caller_row && member.principal_id != LOCAL_OWNER_ID)
                    .then_some(member.principal_id)
            })
        });
    let callee = match callee_id {
        Some(id) => store.get_bot(&id)?.map(crew_member),
        None => None,
    };
    let message_id = record_in(store, conversation_id, caller, question)?;
    if let (Some(message_id), Some(callee)) = (&message_id, &callee) {
        record_ask_activity(store, conversation_id, message_id, caller, callee, question);
    }
    Ok(message_id)
}

/// Leaves the question as an activity in the asker's chat and in the callee's, when that exists.
/// A failure is logged: the ask goes on without it.
fn record_ask_activity(
    store: &LocalBotStore,
    exchange_id: &str,
    ask_id: &str,
    asker: &CrewMember,
    callee: &CrewMember,
    question: &str,
) {
    let (Ok(Some(asker_row)), Ok(Some(callee_row))) = (row_id(store, asker), row_id(store, callee))
    else {
        return;
    };
    let body = ActivityBody::bot_ask(ask_id, &asker_row, &callee_row, exchange_id, question);
    for (bot_id, start) in [(&asker_row, true), (&callee_row, false)] {
        let digest = blake3::hash(format!("{ask_id}\0{bot_id}").as_bytes()).to_hex();
        let message_id = format!("act-{}", &digest[..32]);
        if let Err(error) = store.post_activity(bot_id, &message_id, &body, start) {
            tracing::warn!(%error, %bot_id, "Bot talk: the ask was not recorded as activity");
        }
    }
}

/// What `callee` and the bot asking it said in the exchange before the question, oldest first:
/// the callee's messages are its own answers, the other bot's are the user turns.
fn history_before(
    store: &LocalBotStore,
    conversation_id: &str,
    question_message_id: &str,
    callee: &str,
) -> flow_like_local_bots::Result<Vec<UnifiedChatMessage>> {
    let mut before = None;
    let mut found_question = false;
    let mut earlier = Vec::new();
    loop {
        let page = store.entries_as(
            conversation_id,
            callee,
            &EntriesQuery {
                before,
                limit: Some(HISTORY_PAGE_SIZE),
                ..Default::default()
            },
        )?;
        let mut topic_boundary = false;
        for entry in page.entries {
            if !found_question {
                found_question = entry.message_id == question_message_id;
                continue;
            }
            topic_boundary = entry.kind == InboxEntryKind::Topic
                && flow_like_bot_host::rule::is_plain(&entry)
                && !is_tombstone(&entry.envelope);
            earlier.push(entry);
            if topic_boundary {
                break;
            }
        }
        earlier.reverse();
        let history = flow_like_bot_host::transcript::render_history(
            callee,
            ConversationKind::Direct,
            &earlier,
            &Default::default(),
            LOCAL_OWNER_ID,
            HISTORY_MESSAGES,
        );
        if history.len() == HISTORY_MESSAGES || topic_boundary || page.next_before.is_none() {
            return Ok(history);
        }
        earlier.reverse();
        before = page.next_before;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::flow::copilot::ChatRole;
    use flow_like_local_bots::{ChangeNotifier, CreateBotRequest, KeyError, SecretKeyProvider};
    use zeroize::Zeroizing;

    struct Key;

    impl SecretKeyProvider for Key {
        fn key(&self, _create: bool) -> Result<Zeroizing<[u8; 32]>, KeyError> {
            Ok(Zeroizing::new([7; 32]))
        }
    }

    struct Quiet;

    impl ChangeNotifier for Quiet {
        fn bots_changed(&self, _bot_id: Option<&str>) {}
        fn inbox_changed(&self, _conversation_id: Option<&str>) {}
    }

    struct TempDir(std::path::PathBuf);

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn store() -> (LocalBotStore, TempDir) {
        let dir = TempDir(std::env::temp_dir().join(format!("bot-talk-{}", create_id())));
        let store =
            LocalBotStore::open(&dir.0.join("local_bots.db"), Arc::new(Key), Arc::new(Quiet))
                .unwrap();
        (store, dir)
    }

    fn bot(store: &LocalBotStore, name: &str, description: Option<&str>) -> CrewMember {
        let request: CreateBotRequest = serde_json::from_value(json!({
            "name": name,
            "description": description,
            "appearance": { "kind": "pip" }
        }))
        .unwrap();
        crew_member(store.create_bot(request).unwrap())
    }

    fn names(roster: &[CrewMember]) -> Vec<&str> {
        roster.iter().map(|member| member.name.as_str()).collect()
    }

    #[test]
    fn the_crew_is_every_other_bot_on_this_desktop_built_ins_included() {
        let (store, _dir) = store();
        let scout = bot(&store, "Scout", Some("Finds things"));
        bot(
            &store,
            "Research digest",
            Some("Weekly papers\nSecond line"),
        );

        let roster = roster_of(&store, &scout).unwrap().unwrap();

        let all = names(&roster);
        assert!(all.contains(&"Research digest"));
        assert!(all.contains(&"Assistant") && all.contains(&"Flux"));
        assert!(!all.contains(&"Scout"));
        let digest = roster.iter().find(|m| m.name == "Research digest").unwrap();
        assert_eq!(
            digest.description.as_deref(),
            Some("Weekly papers\nSecond line")
        );
        assert_eq!(digest.presence, BotPresence::Online);
        assert_eq!(digest.location, BotLocation::Desktop);
        let flux = roster.iter().find(|m| m.name == "Flux").unwrap();
        assert_eq!(flux.system, Some(BotSystemKind::Flux));
        assert_eq!(flux.description.as_deref(), Some(FLUX_DESCRIPTION));

        let stranger = CrewMember {
            id: "bot_account".into(),
            name: "Account".into(),
            ..Default::default()
        };
        assert_eq!(roster_of(&store, &stranger).unwrap(), None);
    }

    #[test]
    fn assistant_without_a_row_is_the_built_in_row_and_never_its_own_crew() {
        let (store, _dir) = store();
        let scout = bot(&store, "Scout", None);
        let general = CrewMember {
            id: "general".into(),
            name: "Assistant".into(),
            system: Some(BotSystemKind::General),
            ..Default::default()
        };

        let roster = roster_of(&store, &general).unwrap().unwrap();

        assert!(!names(&roster).contains(&"Assistant"));
        assert!(names(&roster).contains(&"Scout"));
        let (from, to) = rows(&store, &general, &scout).unwrap().unwrap();
        assert_ne!(from, "general");
        assert_eq!(to, scout.id);
        store.grant_permission(&from, &to).unwrap();
        assert!(store.has_permission(&from, &to).unwrap());
    }

    #[test]
    fn an_exchange_is_one_group_per_pair_that_both_bots_write_in() {
        let (store, _dir) = store();
        let scout = bot(&store, "Scout", None);
        let ledger = bot(&store, "Ledger", None);
        let digest = bot(&store, "Digest", None);

        let first = exchange_of(&store, &scout, &ledger).unwrap().unwrap();
        assert_eq!(
            exchange_of(&store, &ledger, &scout).unwrap().unwrap(),
            first
        );
        assert_ne!(
            exchange_of(&store, &scout, &digest).unwrap().unwrap(),
            first
        );

        let conversation = store.conversation(&first).unwrap().unwrap();
        assert_eq!(conversation.kind, ConversationKind::Group);
        assert_eq!(conversation.members.len(), 3);
        assert!(
            conversation
                .title
                .unwrap()
                .contains(EXCHANGE_TITLE_SEPARATOR)
        );

        let question = record_in(&store, &first, &scout, "Total?")
            .unwrap()
            .unwrap();
        let answer = record_in(&store, &first, &ledger, "1.2M").unwrap().unwrap();
        assert_ne!(question, answer);
        let entries = store
            .entries(&first, &EntriesQuery::default())
            .unwrap()
            .entries;
        let senders: Vec<&str> = entries
            .iter()
            .filter(|entry| entry.kind == InboxEntryKind::Message)
            .map(|entry| entry.sender_id.as_str())
            .collect();
        assert_eq!(senders, [ledger.id.as_str(), scout.id.as_str()]);
    }

    #[test]
    fn nested_answers_do_not_record_another_ask_or_change_its_recipient() {
        let (store, _dir) = store();
        let scout = bot(&store, "Scout", None);
        let ledger = bot(&store, "Ledger", None);
        let digest = bot(&store, "Digest", None);
        let chats: Vec<_> = [&scout, &ledger, &digest]
            .iter()
            .map(|member| {
                store
                    .create_conversation(CreateConversationRequest {
                        kind: ConversationKind::Direct,
                        title: None,
                        members: vec![member.id.clone()],
                        encryption: None,
                        bot_limits: Default::default(),
                    })
                    .unwrap()
                    .id
            })
            .collect();
        let first = exchange_of(&store, &scout, &ledger).unwrap().unwrap();
        let second = exchange_of(&store, &ledger, &digest).unwrap().unwrap();

        let ask = record_question_in(&store, &first, &scout, "Q3 total?")
            .unwrap()
            .unwrap();
        let nested = record_question_in(&store, &second, &ledger, "Q3 source?")
            .unwrap()
            .unwrap();
        record_in(&store, &second, &digest, "The report says 1.2M.").unwrap();
        record_in(&store, &first, &ledger, "1.2M").unwrap();
        let follow_up = record_question_in(&store, &first, &scout, "And Q4?")
            .unwrap()
            .unwrap();

        let activities: Vec<Vec<ActivityBody>> = chats
            .iter()
            .map(|chat| {
                let mut entries = store
                    .entries(chat, &EntriesQuery::default())
                    .unwrap()
                    .entries;
                entries.reverse();
                entries
                    .iter()
                    .filter(|entry| entry.kind == InboxEntryKind::Activity)
                    .map(|entry| ActivityBody::parse(&entry.envelope["body"]).unwrap())
                    .collect()
            })
            .collect();
        let first_ask = ActivityBody::bot_ask(&ask, &scout.id, &ledger.id, &first, "Q3 total?");
        let nested_ask =
            ActivityBody::bot_ask(&nested, &ledger.id, &digest.id, &second, "Q3 source?");
        let follow_up_ask =
            ActivityBody::bot_ask(&follow_up, &scout.id, &ledger.id, &first, "And Q4?");
        assert_eq!(
            activities,
            [
                vec![first_ask.clone(), follow_up_ask.clone()],
                vec![first_ask, nested_ask.clone(), follow_up_ask],
                vec![nested_ask],
            ]
        );
    }

    #[test]
    fn the_callee_s_history_is_what_came_before_the_question() {
        let (store, _dir) = store();
        let scout = bot(&store, "Scout", None);
        let ledger = bot(&store, "Ledger", None);
        let conversation = exchange_of(&store, &scout, &ledger).unwrap().unwrap();
        record_in(&store, &conversation, &scout, "Q3?").unwrap();
        record_in(&store, &conversation, &ledger, "1.2M").unwrap();
        let question = record_in(&store, &conversation, &scout, "And Q4?")
            .unwrap()
            .unwrap();

        let history = history_before(&store, &conversation, &question, &ledger.id).unwrap();

        let turns: Vec<(bool, &str)> = history
            .iter()
            .map(|message| {
                (
                    matches!(message.role, ChatRole::Assistant),
                    message.content.as_str(),
                )
            })
            .collect();
        assert_eq!(turns, [(false, "Q3?"), (true, "1.2M")]);
        let first = record_in(&store, &conversation, &scout, "Hi")
            .unwrap()
            .unwrap();
        assert!(
            history_before(&store, &conversation, &first, &ledger.id)
                .unwrap()
                .len()
                >= 3
        );
    }

    #[test]
    fn a_question_keeps_its_history_when_newer_messages_fill_multiple_pages() {
        let (store, _dir) = store();
        let scout = bot(&store, "Scout", None);
        let ledger = bot(&store, "Ledger", None);
        let conversation = exchange_of(&store, &scout, &ledger).unwrap().unwrap();
        for index in 0..45 {
            let sender = if index % 2 == 0 { &scout } else { &ledger };
            record_in(&store, &conversation, sender, &format!("Earlier {index}")).unwrap();
        }
        let question = record_in(&store, &conversation, &scout, "Current question")
            .unwrap()
            .unwrap();
        for index in 0..105 {
            record_in(&store, &conversation, &scout, &format!("Later {index}")).unwrap();
        }

        let history = history_before(&store, &conversation, &question, &ledger.id).unwrap();

        assert_eq!(history.len(), HISTORY_MESSAGES);
        for (message, index) in history.iter().zip(5..45) {
            assert_eq!(message.content, format!("Earlier {index}"));
            assert_eq!(matches!(message.role, ChatRole::Assistant), index % 2 == 1);
        }
    }

    #[test]
    fn an_exchange_respects_topic_boundaries_edits_and_deleted_messages() {
        let (store, _dir) = store();
        let scout = bot(&store, "Scout", None);
        let ledger = bot(&store, "Ledger", None);
        let conversation = exchange_of(&store, &scout, &ledger).unwrap().unwrap();
        let event = |sender: &str, kind, body| {
            store
                .send(
                    &conversation,
                    sender,
                    SendEntryRequest {
                        message_id: format!("event-{}", create_id()),
                        kind,
                        envelope: Some(plain_envelope(body)),
                        envelopes: None,
                    },
                )
                .unwrap()
        };
        record_in(&store, &conversation, &scout, "Old topic").unwrap();
        event(LOCAL_OWNER_ID, InboxEntryKind::Topic, json!({}));
        let original = record_in(&store, &conversation, &scout, "Check Q3")
            .unwrap()
            .unwrap();
        event(
            LOCAL_OWNER_ID,
            InboxEntryKind::Topic,
            json!({ "deleted": true }),
        );
        record_in(&store, &conversation, &ledger, "I am checking.").unwrap();
        event(
            &scout.id,
            InboxEntryKind::Edit,
            json!({ "target": original, "text": "Check Q4" }),
        );
        let deleted = record_in(&store, &conversation, &scout, "Discard this")
            .unwrap()
            .unwrap();
        event(
            &scout.id,
            InboxEntryKind::Delete,
            json!({ "target": deleted }),
        );
        let question = record_in(&store, &conversation, &scout, "Any update?")
            .unwrap()
            .unwrap();

        let history = history_before(&store, &conversation, &question, &ledger.id).unwrap();

        assert_eq!(history.len(), 2);
        assert_eq!(history[0].content, "Check Q4");
        assert!(matches!(history[0].role, ChatRole::User));
        assert_eq!(history[1].content, "I am checking.");
        assert!(matches!(history[1].role, ChatRole::Assistant));
    }
}
