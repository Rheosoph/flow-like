//! `notify_owner` on this desktop. A local bot alerts the person through the window; an account
//! bot asks the hub, which groups, limits and pushes the notification, and the window shows it too.

use super::{
    hub::ProfileHub,
    notify::{Alert, NOTIFICATION_EVENT, alert_for, batch},
};
use async_trait::async_trait;
use flow_like_bot_tools::{NoticeReceipt, OwnerNotice, OwnerNotifier};
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
    async fn account_notify(&self, bot_id: &str, notice: &OwnerNotice) -> Result<NoticeReceipt>;
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
    directory: Arc<dyn OwnerDirectory>,
    sink: Arc<dyn AlertSink>,
    log: Arc<SentLog>,
}

impl DesktopOwnerNotifier {
    pub(crate) fn new(
        bot_id: &str,
        name: &str,
        directory: Arc<dyn OwnerDirectory>,
        sink: Arc<dyn AlertSink>,
        log: Arc<SentLog>,
    ) -> Self {
        Self {
            bot_id: bot_id.to_owned(),
            name: name.to_owned(),
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
        let text = format!("{}: {}", notice.title, notice.body);
        if let Some(bot) = self.directory.local_bot(&self.bot_id).await? {
            if notifications_muted(&bot.appearance) {
                return Ok(NoticeReceipt {
                    id: String::new(),
                    muted: true,
                });
            }
            let id = create_id();
            let key = format!("{TOOL_SOURCE}:{id}");
            self.sink.show(&alert_for(&bot, text, notice.link, &key));
            self.log
                .record(&self.bot_id, chrono::Utc::now().timestamp_millis());
            return Ok(NoticeReceipt { id, muted: false });
        }
        let receipt = self.directory.account_notify(&self.bot_id, &notice).await?;
        if !receipt.muted {
            self.sink.show(&Alert {
                bot_id: self.bot_id.clone(),
                title: self.name.clone(),
                body: text,
                link: notice
                    .link
                    .unwrap_or_else(|| flow_like_types::bot_notify::chat_link(&self.bot_id)),
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

    async fn account_notify(&self, bot_id: &str, notice: &OwnerNotice) -> Result<NoticeReceipt> {
        ProfileHub::new(self.0.clone())
            .notify_owner(bot_id, notice)
            .await
            .map_err(|failure| anyhow!("{failure}"))
    }
}

impl AlertSink for Live {
    fn show(&self, alert: &Alert) {
        crate::utils::emit_to_ui(&self.0, NOTIFICATION_EVENT, batch(alert));
    }
}

static LOG: LazyLock<Arc<SentLog>> = LazyLock::new(Arc::default);

pub(crate) fn notifier_for(app: &AppHandle, bot_id: &str, name: &str) -> Arc<dyn OwnerNotifier> {
    let live = Arc::new(Live(app.clone()));
    Arc::new(DesktopOwnerNotifier::new(
        bot_id,
        name,
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
        account_muted: bool,
        account_calls: Mutex<Vec<String>>,
        shown: Mutex<Vec<Alert>>,
    }

    #[async_trait]
    impl OwnerDirectory for Fakes {
        async fn local_bot(&self, _bot_id: &str) -> Result<Option<Bot>> {
            Ok(self.local.lock().unwrap().clone())
        }

        async fn account_notify(
            &self,
            bot_id: &str,
            _notice: &OwnerNotice,
        ) -> Result<NoticeReceipt> {
            self.account_calls.lock().unwrap().push(bot_id.to_owned());
            Ok(NoticeReceipt {
                id: "n-9".into(),
                muted: self.account_muted,
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
            title: "Heads up".into(),
            body: "Look".into(),
            link: None,
        }
    }

    fn notifier(fakes: &Arc<Fakes>) -> DesktopOwnerNotifier {
        DesktopOwnerNotifier::new(
            "bot_1",
            "Ada",
            fakes.clone(),
            fakes.clone(),
            Arc::default(),
        )
    }

    #[tokio::test]
    async fn a_local_bot_alerts_under_its_name_and_counts_toward_the_limit() {
        let fakes = Arc::new(Fakes::default());
        *fakes.local.lock().unwrap() = Some(local_bot(json!({ "kind": "pip" })));
        let notifier = notifier(&fakes);
        let receipt = notifier.deliver(notice()).await.unwrap();
        assert!(!receipt.muted);
        let shown = fakes.shown.lock().unwrap().clone();
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
    async fn a_muted_local_bot_shows_nothing_and_is_not_counted() {
        let fakes = Arc::new(Fakes::default());
        *fakes.local.lock().unwrap() =
            Some(local_bot(json!({ "kind": "pip", "mute_notifications": true })));
        let notifier = notifier(&fakes);
        assert!(notifier.deliver(notice()).await.unwrap().muted);
        assert!(fakes.shown.lock().unwrap().is_empty());
        assert_eq!(notifier.sent_since(0).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn an_account_bot_goes_through_the_hub_and_still_shows_here() {
        let fakes = Arc::new(Fakes::default());
        let notifier = notifier(&fakes);
        let receipt = notifier.deliver(notice()).await.unwrap();
        assert_eq!(receipt.id, "n-9");
        assert_eq!(fakes.account_calls.lock().unwrap().as_slice(), ["bot_1"]);
        let shown = fakes.shown.lock().unwrap().clone();
        assert_eq!((shown[0].title.as_str(), shown[0].key.as_str()), ("Ada", "bot-notify-tool:n-9"));
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
