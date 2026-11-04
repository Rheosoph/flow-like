//! `message_owner` on this desktop. A local bot writes into its direct chat with the person and
//! alerts them through the window; an account bot asks the hub, which writes the message, groups,
//! limits and pushes it, and the window shows it too.

use super::{
    hub::ProfileHub,
    notify::{Alert, NOTIFICATION_EVENT, alert_for, batch},
};
use async_trait::async_trait;
use flow_like_bot_tools::{NoticeReceipt, OwnerNotice, OwnerNotifier, TurnSite};
use flow_like_local_bots::Bot;
use flow_like_types::{
    Result, anyhow,
    bot_notify::{TOOL_SOURCE, notifications_muted},
    create_id,
};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, LazyLock, Mutex, PoisonError},
};
use tauri::{AppHandle, Manager};

#[async_trait]
pub(crate) trait OwnerDirectory: Send + Sync {
    async fn local_bot(&self, bot_id: &str) -> Result<Option<Bot>>;
    /// Writes the message into the local bot's direct chat; `false` when `from_conversation` is
    /// that chat and nothing was written.
    async fn post_local(
        &self,
        bot_id: &str,
        from_conversation: Option<&str>,
        message_id: &str,
        text: &str,
    ) -> Result<bool>;
    async fn account_notify(
        &self,
        bot_id: &str,
        conversation_id: Option<&str>,
        notice: &OwnerNotice,
    ) -> Result<NoticeReceipt>;
}

pub(crate) trait AlertSink: Send + Sync {
    fn show(&self, alert: &Alert);
}

/// When each local bot last notified, newest last.
#[derive(Default)]
pub(crate) struct SentLog(Mutex<HashMap<String, VecDeque<i64>>>);

impl SentLog {
    pub(crate) fn since(&self, bot_id: &str, since_ms: i64) -> usize {
        let mut log = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(sent) = log.get_mut(bot_id) else {
            return 0;
        };
        while sent.front().is_some_and(|at| *at < since_ms) {
            sent.pop_front();
        }
        sent.len()
    }

    pub(crate) fn record(&self, bot_id: &str, at_ms: i64) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(bot_id.to_owned())
            .or_default()
            .push_back(at_ms);
    }
}

pub(crate) struct DesktopOwnerNotifier {
    bot_id: String,
    name: String,
    site: Option<TurnSite>,
    directory: Arc<dyn OwnerDirectory>,
    sink: Arc<dyn AlertSink>,
    log: Arc<SentLog>,
}

impl DesktopOwnerNotifier {
    pub(crate) fn new(
        bot_id: &str,
        name: &str,
        site: Option<TurnSite>,
        directory: Arc<dyn OwnerDirectory>,
        sink: Arc<dyn AlertSink>,
        log: Arc<SentLog>,
    ) -> Self {
        Self {
            bot_id: bot_id.to_owned(),
            name: name.to_owned(),
            site,
            directory,
            sink,
            log,
        }
    }
}

#[async_trait]
impl OwnerNotifier for DesktopOwnerNotifier {
    async fn sent_since(&self, since_ms: i64) -> Result<usize> {
        Ok(self.log.since(&self.bot_id, since_ms))
    }

    async fn deliver(&self, notice: OwnerNotice) -> Result<NoticeReceipt> {
        if self.site == Some(TurnSite::OwnerLive) {
            return Ok(NoticeReceipt {
                in_owner_chat: true,
                ..Default::default()
            });
        }
        let conversation_id = match &self.site {
            Some(TurnSite::Conversation(id)) if !id.is_empty() => Some(id.as_str()),
            _ => None,
        };
        let text = notice.push_body();
        if let Some(bot) = self.directory.local_bot(&self.bot_id).await? {
            let message_id = format!("mo-{}", create_id());
            let written = self
                .directory
                .post_local(
                    &self.bot_id,
                    conversation_id,
                    &message_id,
                    &notice.message(),
                )
                .await?;
            if !written {
                return Ok(NoticeReceipt {
                    in_owner_chat: true,
                    ..Default::default()
                });
            }
            self.log
                .record(&self.bot_id, chrono::Utc::now().timestamp_millis());
            if notifications_muted(&bot.appearance) {
                return Ok(NoticeReceipt {
                    id: message_id,
                    muted: true,
                    posted: true,
                    ..Default::default()
                });
            }
            let key = format!("{TOOL_SOURCE}:{message_id}");
            self.sink.show(&alert_for(&bot, text, None, &key));
            return Ok(NoticeReceipt {
                id: message_id,
                posted: true,
                ..Default::default()
            });
        }
        let receipt = self
            .directory
            .account_notify(&self.bot_id, conversation_id, &notice)
            .await?;
        if !receipt.muted && !receipt.in_owner_chat {
            self.sink.show(&Alert {
                bot_id: self.bot_id.clone(),
                title: self.name.clone(),
                body: text,
                link: flow_like_types::bot_notify::chat_link(&self.bot_id),
                key: format!("{TOOL_SOURCE}:{}", receipt.id),
            });
        }
        Ok(receipt)
    }
}

struct Live(AppHandle);

#[async_trait]
impl OwnerDirectory for Live {
    async fn local_bot(&self, bot_id: &str) -> Result<Option<Bot>> {
        let Some(bots) = self.0.try_state::<crate::functions::local_bots::LocalBots>() else {
            return Ok(None);
        };
        let id = bot_id.to_owned();
        crate::functions::local_bots::on_store(&bots, move |store| store.get_bot(&id))
            .await
            .map_err(|error| anyhow!(error.to_string()))
    }

    async fn post_local(
        &self,
        bot_id: &str,
        from_conversation: Option<&str>,
        message_id: &str,
        text: &str,
    ) -> Result<bool> {
        let bots = self
            .0
            .try_state::<crate::functions::local_bots::LocalBots>()
            .ok_or_else(|| anyhow!("The local bots store is not open"))?;
        let (bot_id, from, request) = (
            bot_id.to_owned(),
            from_conversation.map(str::to_owned),
            owner_message_request(message_id, text),
        );
        crate::functions::local_bots::on_store(&bots, move |store| {
            store.send_to_owner(&bot_id, from.as_deref(), request)
        })
        .await
        .map(|entry| entry.is_some())
        .map_err(|error| anyhow!(error.to_string()))
    }

    async fn account_notify(
        &self,
        bot_id: &str,
        conversation_id: Option<&str>,
        notice: &OwnerNotice,
    ) -> Result<NoticeReceipt> {
        ProfileHub::new(self.0.clone())
            .message_owner(bot_id, conversation_id, notice)
            .await
            .map_err(|failure| anyhow!("{failure}"))
    }
}

fn owner_message_request(
    message_id: &str,
    text: &str,
) -> flow_like_local_bots::SendEntryRequest {
    flow_like_local_bots::SendEntryRequest {
        message_id: message_id.to_owned(),
        kind: flow_like_types::inbox::InboxEntryKind::Message,
        envelope: Some(flow_like_types::inbox::plain_envelope(
            serde_json::json!({ "text": text, "hops": 0 }),
        )),
        envelopes: None,
    }
}

impl AlertSink for Live {
    fn show(&self, alert: &Alert) {
        crate::utils::emit_to_ui(&self.0, NOTIFICATION_EVENT, batch(alert));
    }
}

static LOG: LazyLock<Arc<SentLog>> = LazyLock::new(Arc::default);

pub(crate) fn notifier_for(
    app: &AppHandle,
    bot_id: &str,
    name: &str,
    site: Option<TurnSite>,
) -> Arc<dyn OwnerNotifier> {
    let live = Arc::new(Live(app.clone()));
    Arc::new(DesktopOwnerNotifier::new(
        bot_id,
        name,
        site,
        live.clone(),
        live,
        LOG.clone(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[derive(Default)]
    struct Fakes {
        local: Mutex<Option<Bot>>,
        in_owner_chat: bool,
        account_muted: bool,
        account_calls: Mutex<Vec<(String, Option<String>)>>,
        posted: Mutex<Vec<(String, String)>>,
        shown: Mutex<Vec<Alert>>,
    }

    #[async_trait]
    impl OwnerDirectory for Fakes {
        async fn local_bot(&self, _bot_id: &str) -> Result<Option<Bot>> {
            Ok(self.local.lock().unwrap().clone())
        }

        async fn post_local(
            &self,
            _bot_id: &str,
            _from_conversation: Option<&str>,
            message_id: &str,
            text: &str,
        ) -> Result<bool> {
            if self.in_owner_chat {
                return Ok(false);
            }
            self.posted
                .lock()
                .unwrap()
                .push((message_id.to_owned(), text.to_owned()));
            Ok(true)
        }

        async fn account_notify(
            &self,
            bot_id: &str,
            conversation_id: Option<&str>,
            _notice: &OwnerNotice,
        ) -> Result<NoticeReceipt> {
            self.account_calls
                .lock()
                .unwrap()
                .push((bot_id.to_owned(), conversation_id.map(str::to_owned)));
            Ok(NoticeReceipt {
                id: "mo-9".into(),
                muted: self.account_muted,
                in_owner_chat: self.in_owner_chat,
                posted: !self.in_owner_chat,
            })
        }
    }

    impl AlertSink for Fakes {
        fn show(&self, alert: &Alert) {
            self.shown.lock().unwrap().push(alert.clone());
        }
    }

    fn local_bot(appearance: serde_json::Value) -> Bot {
        super::super::notify::test_bot(appearance)
    }

    fn notice() -> OwnerNotice {
        OwnerNotice {
            title: Some("Heads up".into()),
            text: "Look".into(),
        }
    }

    fn notifier(fakes: &Arc<Fakes>) -> DesktopOwnerNotifier {
        DesktopOwnerNotifier::new(
            "bot_1",
            "Ada",
            Some(TurnSite::Conversation("mail-thread".into())),
            fakes.clone(),
            fakes.clone(),
            Arc::default(),
        )
    }

    #[tokio::test]
    async fn a_local_bot_writes_the_message_alerts_under_its_name_and_counts() {
        let fakes = Arc::new(Fakes::default());
        *fakes.local.lock().unwrap() = Some(local_bot(json!({ "kind": "pip" })));
        let notifier = notifier(&fakes);
        let receipt = notifier.deliver(notice()).await.unwrap();
        assert!(!receipt.muted && receipt.posted && !receipt.in_owner_chat);
        let posted = fakes.posted.lock().unwrap().clone();
        assert_eq!(posted.len(), 1);
        assert_eq!(posted[0].0, receipt.id);
        assert_eq!(posted[0].1, "Heads up\n\nLook");
        let shown = fakes.shown.lock().unwrap().clone();
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].title, "Ada");
        assert_eq!(shown[0].body, "Heads up: Look");
        assert_eq!(shown[0].link, "/bots?bot=bot_1");
        assert!(shown[0].key.starts_with(TOOL_SOURCE));
        assert_eq!(notifier.sent_since(0).await.unwrap(), 1);
        assert_eq!(
            notifier
                .sent_since(chrono::Utc::now().timestamp_millis() + 1000)
                .await
                .unwrap(),
            0
        );
        assert!(fakes.account_calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_local_bot_in_the_owner_chat_writes_and_shows_nothing() {
        let fakes = Arc::new(Fakes {
            in_owner_chat: true,
            ..Default::default()
        });
        *fakes.local.lock().unwrap() = Some(local_bot(json!({ "kind": "pip" })));
        let notifier = notifier(&fakes);
        assert!(notifier.deliver(notice()).await.unwrap().in_owner_chat);
        assert!(fakes.shown.lock().unwrap().is_empty());
        assert_eq!(notifier.sent_since(0).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn a_chat_the_owner_reads_live_writes_and_shows_nothing_for_any_bot() {
        for local in [true, false] {
            let fakes = Arc::new(Fakes::default());
            if local {
                *fakes.local.lock().unwrap() = Some(local_bot(json!({ "kind": "pip" })));
            }
            let live = DesktopOwnerNotifier::new(
                "bot_1",
                "Ada",
                Some(TurnSite::OwnerLive),
                fakes.clone(),
                fakes.clone(),
                Arc::default(),
            );
            assert!(live.deliver(notice()).await.unwrap().in_owner_chat);
            assert!(fakes.posted.lock().unwrap().is_empty());
            assert!(fakes.account_calls.lock().unwrap().is_empty());
            assert!(fakes.shown.lock().unwrap().is_empty());
        }
    }

    #[tokio::test]
    async fn a_muted_local_bot_still_writes_but_shows_nothing_and_is_counted() {
        let fakes = Arc::new(Fakes::default());
        *fakes.local.lock().unwrap() =
            Some(local_bot(json!({ "kind": "pip", "mute_notifications": true })));
        let notifier = notifier(&fakes);
        let receipt = notifier.deliver(notice()).await.unwrap();
        assert!(receipt.muted && receipt.posted);
        assert_eq!(fakes.posted.lock().unwrap().len(), 1);
        assert!(fakes.shown.lock().unwrap().is_empty());
        assert_eq!(notifier.sent_since(0).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn an_account_bot_goes_through_the_hub_and_still_shows_here() {
        let fakes = Arc::new(Fakes::default());
        let notifier = notifier(&fakes);
        let receipt = notifier.deliver(notice()).await.unwrap();
        assert_eq!(receipt.id, "mo-9");
        assert_eq!(
            fakes.account_calls.lock().unwrap().as_slice(),
            [("bot_1".to_owned(), Some("mail-thread".to_owned()))]
        );
        let shown = fakes.shown.lock().unwrap().clone();
        assert_eq!((shown[0].title.as_str(), shown[0].key.as_str()), ("Ada", "bot-notify-tool:mo-9"));
    }

    #[tokio::test]
    async fn an_account_bot_in_the_owner_chat_shows_nothing() {
        let fakes = Arc::new(Fakes {
            in_owner_chat: true,
            ..Default::default()
        });
        let receipt = notifier(&fakes).deliver(notice()).await.unwrap();
        assert!(receipt.in_owner_chat);
        assert!(fakes.shown.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_muted_account_bot_shows_nothing() {
        let fakes = Arc::new(Fakes {
            account_muted: true,
            ..Default::default()
        });
        notifier(&fakes).deliver(notice()).await.unwrap();
        assert!(fakes.shown.lock().unwrap().is_empty());
    }
}
