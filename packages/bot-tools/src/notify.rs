use std::sync::Arc;

use async_trait::async_trait;
use flow_like_types::Result;
use flow_like_types::bot_notify::{BODY_MAX_CHARS, TITLE_MAX_CHARS, TOOL_MAX_PER_WINDOW};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

use crate::env::ToolEnv;
use crate::error::{ToolError, ToolResult, code};
use crate::outcome::ToolOutput;
use crate::spec::{Approval, ToolSpec, schema_of};

pub const NOTIFY_OWNER: &str = "notify_owner";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerNotice {
    pub title: String,
    pub body: String,
    /// An in-app path; `None` opens the bot's chat.
    pub link: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoticeReceipt {
    pub id: String,
    /// The owner muted this bot, so nothing was sent.
    pub muted: bool,
}

/// How a host reaches the owner of the bot that runs the tool. The notice is grouped under the bot
/// and opens its chat; counting what the bot sent lets the tool enforce its rate limit across turns.
#[async_trait]
pub trait OwnerNotifier: Send + Sync {
    async fn sent_since(&self, since_ms: i64) -> Result<usize>;
    async fn deliver(&self, notice: OwnerNotice) -> Result<NoticeReceipt>;
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct NotifyOwnerArgs {
    /// A short headline. At most 120 characters; the bot's name is shown above it.
    pub title: String,
    /// What the owner should know. At most 500 characters.
    pub body: String,
    /// An in-app path to open instead of this bot's chat (starts with `/`).
    #[serde(default)]
    pub link: Option<String>,
}

pub fn specs() -> Vec<ToolSpec> {
    vec![ToolSpec {
        name: NOTIFY_OWNER,
        description: "Send the owner a notification on their phone and computer, shown under this bot's name; it opens this bot's chat. Use it sparingly for what needs their attention; at most 10 per hour.",
        capability: Some("push_notifications"),
        module: "notify",
        approval: Approval::None,
        approval_args: &[],
        mutating: true,
        input_schema: schema_of::<NotifyOwnerArgs>(),
    }]
}

fn checked(value: &str, field: &str, max: usize) -> ToolResult<String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(ToolError::invalid(format!("'{field}' is empty")));
    }
    let chars = value.chars().count();
    if chars > max {
        return Err(ToolError::invalid(format!(
            "'{field}' has {chars} characters; at most {max} are allowed"
        )));
    }
    Ok(value.to_string())
}

fn checked_link(link: Option<String>) -> ToolResult<Option<String>> {
    let Some(link) = link.map(|link| link.trim().to_string()) else {
        return Ok(None);
    };
    if link.is_empty() {
        return Ok(None);
    }
    if !link.starts_with('/') || link.starts_with("//") || link.contains(['\\', '\n', '\r']) {
        return Err(ToolError::invalid(
            "'link' must be an in-app path that starts with a single '/'",
        ));
    }
    Ok(Some(link))
}

pub fn notice_from(title: &str, body: &str, link: Option<String>) -> ToolResult<OwnerNotice> {
    Ok(OwnerNotice {
        title: checked(title, "title", TITLE_MAX_CHARS)?,
        body: checked(body, "body", BODY_MAX_CHARS)?,
        link: checked_link(link)?,
    })
}

pub(crate) fn window_ms() -> i64 {
    flow_like_types::bot_notify::TOOL_WINDOW_SECS * 1000
}

pub async fn notify_owner(env: &dyn ToolEnv, args: NotifyOwnerArgs) -> ToolResult<ToolOutput> {
    let notifier: Arc<dyn OwnerNotifier> = env.notifier().ok_or_else(|| {
        ToolError::new(
            code::FAILED,
            "This host cannot send the owner notifications",
        )
    })?;
    let notice = notice_from(&args.title, &args.body, args.link)?;
    let since = chrono::Utc::now().timestamp_millis() - window_ms();
    let sent = notifier.sent_since(since).await?;
    if sent >= TOOL_MAX_PER_WINDOW {
        return Err(ToolError::new(
            code::RATE_LIMITED,
            format!(
                "This bot already sent {sent} notifications in the last hour; at most {TOOL_MAX_PER_WINDOW} are allowed. Put the rest in your reply instead."
            ),
        ));
    }
    let receipt = notifier.deliver(notice).await?;
    Ok(ToolOutput::new(json!({
        "notified": !receipt.muted,
        "muted": receipt.muted,
        "notification_id": receipt.id,
        "remaining_this_hour": TOOL_MAX_PER_WINDOW.saturating_sub(sent + 1),
    })))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::testing::FakeEnv;
    use serde_json::{Value, json};

    #[derive(Default)]
    struct FakeNotifier {
        prior: AtomicUsize,
        muted: bool,
        delivered: Mutex<Vec<OwnerNotice>>,
    }

    #[async_trait]
    impl OwnerNotifier for FakeNotifier {
        async fn sent_since(&self, _since_ms: i64) -> Result<usize> {
            Ok(self.prior.load(Ordering::SeqCst) + self.delivered.lock().unwrap().len())
        }

        async fn deliver(&self, notice: OwnerNotice) -> Result<NoticeReceipt> {
            if !self.muted {
                self.delivered.lock().unwrap().push(notice);
            }
            Ok(NoticeReceipt {
                id: "n1".into(),
                muted: self.muted,
            })
        }
    }

    async fn env_with(notifier: Arc<FakeNotifier>) -> FakeEnv {
        let env = FakeEnv::new().await;
        *env.notifier.lock().unwrap() = Some(notifier);
        env
    }

    async fn call(env: &FakeEnv, args: Value) -> crate::ToolOutcome {
        crate::run(NOTIFY_OWNER, args, env).await
    }

    #[test]
    fn the_spec_needs_the_push_capability() {
        let spec = crate::spec(NOTIFY_OWNER).expect("registered");
        assert_eq!(spec.capability, Some("push_notifications"));
        assert!(spec.mutating);
    }

    #[tokio::test]
    async fn a_notice_reaches_the_notifier_trimmed() {
        let notifier = Arc::new(FakeNotifier::default());
        let env = env_with(notifier.clone()).await;
        let outcome = call(
            &env,
            json!({ "title": " Report ready ", "body": "Q3 is done", "link": "/store" }),
        )
        .await;
        assert_eq!(outcome.status, crate::Status::Ok, "{:?}", outcome.error);
        assert_eq!(outcome.result["notified"], true);
        let delivered = notifier.delivered.lock().unwrap();
        assert_eq!(delivered[0].title, "Report ready");
        assert_eq!(delivered[0].link.as_deref(), Some("/store"));
    }

    #[tokio::test]
    async fn the_tenth_notice_of_an_hour_passes_and_the_eleventh_is_refused() {
        let notifier = Arc::new(FakeNotifier::default());
        notifier
            .prior
            .store(TOOL_MAX_PER_WINDOW - 1, Ordering::SeqCst);
        let env = env_with(notifier.clone()).await;
        let tenth = call(&env, json!({ "title": "t", "body": "b" })).await;
        assert_eq!(tenth.status, crate::Status::Ok);
        assert_eq!(tenth.result["remaining_this_hour"], 0);
        let eleventh = call(&env, json!({ "title": "t", "body": "b" })).await;
        assert_eq!(eleventh.status, crate::Status::Error);
        assert_eq!(eleventh.error.unwrap().code, code::RATE_LIMITED);
        assert_eq!(notifier.delivered.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn bad_arguments_never_reach_the_notifier() {
        let notifier = Arc::new(FakeNotifier::default());
        let env = env_with(notifier.clone()).await;
        let long = "x".repeat(BODY_MAX_CHARS + 1);
        for args in [
            json!({ "title": " ", "body": "b" }),
            json!({ "title": "t", "body": long }),
            json!({ "title": "t", "body": "b", "link": "https://evil.example" }),
            json!({ "title": "t", "body": "b", "link": "//evil.example" }),
        ] {
            let outcome = call(&env, args).await;
            assert_eq!(outcome.error.expect("refused").code, code::INVALID_ARGS);
        }
        assert!(notifier.delivered.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_muted_bot_reports_that_nothing_was_sent() {
        let notifier = Arc::new(FakeNotifier {
            muted: true,
            ..Default::default()
        });
        let env = env_with(notifier).await;
        let outcome = call(&env, json!({ "title": "t", "body": "b" })).await;
        assert_eq!(outcome.result["notified"], false);
        assert_eq!(outcome.result["muted"], true);
    }

    #[tokio::test]
    async fn a_host_without_a_notifier_fails_plainly() {
        let env = FakeEnv::new().await;
        let outcome = call(&env, json!({ "title": "t", "body": "b" })).await;
        assert_eq!(outcome.error.expect("failed").code, code::FAILED);
    }
}
