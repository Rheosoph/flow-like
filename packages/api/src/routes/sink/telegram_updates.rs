//! What the Telegram webhook decides about an update before it starts a run: whether the chat
//! may start one, and whether this `update_id` already did.

use crate::cache::{CacheStoreError, PlatformCache, Reservation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

const NAMESPACE: &str = "telegram-update";
/// Telegram redelivers an unacknowledged update for a day at most.
const HANDLED_TTL: Duration = Duration::from_secs(24 * 60 * 60);
/// A replica that dies mid-dispatch frees its update for the redelivery after this long.
const IN_FLIGHT_TTL: Duration = Duration::from_secs(120);

const CHAT_SOURCES: [&str; 8] = [
    "message",
    "edited_message",
    "channel_post",
    "edited_channel_post",
    "my_chat_member",
    "chat_member",
    "chat_join_request",
    "business_message",
];

pub(super) fn update_id(update: &Value) -> Option<i64> {
    update.get("update_id")?.as_i64()
}

fn chat_id(update: &Value) -> Option<i64> {
    CHAT_SOURCES
        .iter()
        .find_map(|source| update.get(*source))
        .or_else(|| update.get("callback_query")?.get("message"))
        .and_then(|message| message.get("chat")?.get("id")?.as_i64())
}

fn id_list(config: &Value, key: &str) -> Vec<String> {
    config
        .get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| match entry {
            Value::String(id) => Some(id.trim().to_owned()),
            Value::Number(id) => Some(id.to_string()),
            _ => None,
        })
        .filter(|id| !id.is_empty())
        .collect()
}

/// The `chat_whitelist` / `chat_blacklist` of the event config, as the device bots read them:
/// a listed chat never starts a run, and a non-empty allow list admits only the chats on it.
/// An update that names no chat passes only while there is no allow list.
pub(super) fn chat_admitted(config: &[u8], update: &Value) -> bool {
    let Ok(config) = serde_json::from_slice::<Value>(config) else {
        return true;
    };
    let allow = id_list(&config, "chat_whitelist");
    let deny = id_list(&config, "chat_blacklist");
    let chat = chat_id(update).map(|chat| chat.to_string());
    match chat {
        Some(chat) => !deny.contains(&chat) && (allow.is_empty() || allow.contains(&chat)),
        None => allow.is_empty(),
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum Record {
    InFlight,
    Done,
}

pub(super) enum Claim {
    /// This request owns the update and must settle it.
    Fresh(UpdateClaim),
    /// A request already handled this update.
    Duplicate,
    /// A request is handling it right now; a redelivery may find it settled.
    InFlight,
}

/// Holds an update for the request that handles it. Dropped without [`UpdateClaim::settle`] (an
/// error, or a cancelled request), the claim is released, so the redelivery Telegram makes for
/// a failed answer runs.
pub(super) struct UpdateClaim {
    cache: PlatformCache,
    key: String,
    settled: bool,
}

fn key(event_id: &str, update_id: i64) -> String {
    format!("{event_id}:{update_id}")
}

pub(super) async fn claim(
    cache: PlatformCache,
    event_id: &str,
    update_id: i64,
) -> Result<Claim, CacheStoreError> {
    let key = key(event_id, update_id);
    Ok(
        match cache
            .try_insert(NAMESPACE, &key, &Record::InFlight, IN_FLIGHT_TTL)
            .await?
        {
            Reservation::Acquired => Claim::Fresh(UpdateClaim {
                cache,
                key,
                settled: false,
            }),
            Reservation::Held(Record::Done) => Claim::Duplicate,
            Reservation::Held(Record::InFlight) => Claim::InFlight,
        },
    )
}

impl UpdateClaim {
    /// A handled update stays claimed for a day, so its redelivery is a duplicate.
    pub(super) async fn settle(mut self) {
        self.settled = true;
        if let Err(error) = self
            .cache
            .set(NAMESPACE, &self.key, &Record::Done, HANDLED_TTL)
            .await
        {
            tracing::warn!(%error, update = %self.key, "Could not record a handled Telegram update");
        }
    }
}

impl Drop for UpdateClaim {
    fn drop(&mut self) {
        if self.settled {
            return;
        }
        let Ok(runtime) = flow_like_types::tokio::runtime::Handle::try_current() else {
            return;
        };
        let (cache, key) = (self.cache.clone(), std::mem::take(&mut self.key));
        runtime.spawn(async move {
            if let Err(error) = cache.delete(NAMESPACE, &key).await {
                tracing::warn!(%error, update = %key, "Could not release a Telegram update claim");
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    async fn fresh_after_release(cache: &PlatformCache, event_id: &str, update_id: i64) -> bool {
        for _ in 0..50 {
            if matches!(
                claim(cache.clone(), event_id, update_id).await.unwrap(),
                Claim::Fresh(_)
            ) {
                return true;
            }
            flow_like_types::tokio::task::yield_now().await;
        }
        false
    }

    fn config(settings: Value) -> Vec<u8> {
        serde_json::to_vec(&settings).unwrap()
    }

    fn message_in(chat: i64) -> Value {
        json!({"update_id": 1, "message": {"message_id": 5, "chat": {"id": chat, "type": "private"}}})
    }

    #[test]
    fn without_lists_every_chat_may_start_a_run() {
        assert!(chat_admitted(&config(json!({})), &message_in(42)));
        assert!(chat_admitted(
            &config(json!({"chat_whitelist": []})),
            &message_in(42)
        ));
        assert!(chat_admitted(b"", &message_in(42)));
        assert!(chat_admitted(&config(json!({})), &json!({"update_id": 2})));
    }

    #[test]
    fn an_allow_list_admits_only_its_chats() {
        let allowed = config(json!({"chat_whitelist": ["42", -100200]}));
        assert!(chat_admitted(&allowed, &message_in(42)));
        assert!(chat_admitted(&allowed, &message_in(-100200)));
        assert!(!chat_admitted(&allowed, &message_in(43)));
    }

    #[test]
    fn the_deny_list_wins_over_the_allow_list() {
        let settings = config(json!({"chat_whitelist": ["42"], "chat_blacklist": ["42"]}));
        assert!(!chat_admitted(&settings, &message_in(42)));
        assert!(!chat_admitted(
            &config(json!({"chat_blacklist": [" 7 "]})),
            &message_in(7)
        ));
    }

    #[test]
    fn every_update_kind_that_names_a_chat_is_checked_against_the_allow_list() {
        let allowed = config(json!({"chat_whitelist": ["42"]}));
        for source in CHAT_SOURCES {
            let chat = |id: i64| json!({"update_id": 1, source: {"chat": {"id": id}}});
            assert!(chat_admitted(&allowed, &chat(42)), "{source}");
            assert!(!chat_admitted(&allowed, &chat(9)), "{source}");
        }
        let button =
            |id: i64| json!({"update_id": 1, "callback_query": {"message": {"chat": {"id": id}}}});
        assert!(chat_admitted(&allowed, &button(42)));
        assert!(!chat_admitted(&allowed, &button(9)));
    }

    #[test]
    fn an_update_without_a_chat_is_refused_while_an_allow_list_exists() {
        let inline = json!({"update_id": 3, "inline_query": {"from": {"id": 42}, "query": "x"}});
        assert!(!chat_admitted(
            &config(json!({"chat_whitelist": ["42"]})),
            &inline
        ));
        assert!(chat_admitted(
            &config(json!({"chat_blacklist": ["42"]})),
            &inline
        ));
        assert!(!chat_admitted(
            &config(json!({"chat_whitelist": ["42"]})),
            &Value::String("not json".into())
        ));
    }

    #[test]
    fn the_update_id_is_read_from_the_body() {
        assert_eq!(update_id(&message_in(1)), Some(1));
        assert_eq!(update_id(&json!({"update_id": "7"})), None);
        assert_eq!(update_id(&json!({"message": {}})), None);
    }

    #[flow_like_types::tokio::test]
    async fn a_handled_update_is_a_duplicate_and_a_failed_one_is_released() {
        let (_, cache) = crate::cache::memory_platform_cache();

        let Claim::Fresh(first) = claim(cache.clone(), "evt", 10).await.unwrap() else {
            panic!("the first delivery owns the update");
        };
        assert!(matches!(
            claim(cache.clone(), "evt", 10).await.unwrap(),
            Claim::InFlight
        ));
        first.settle().await;
        for _ in 0..2 {
            assert!(matches!(
                claim(cache.clone(), "evt", 10).await.unwrap(),
                Claim::Duplicate
            ));
        }

        let Claim::Fresh(failed) = claim(cache.clone(), "evt", 11).await.unwrap() else {
            panic!("a new update is fresh");
        };
        drop(failed);
        assert!(fresh_after_release(&cache, "evt", 11).await);
    }

    #[flow_like_types::tokio::test]
    async fn updates_are_claimed_per_event() {
        let (_, cache) = crate::cache::memory_platform_cache();
        assert!(matches!(
            claim(cache.clone(), "evt_a", 5).await.unwrap(),
            Claim::Fresh(_)
        ));
        assert!(matches!(
            claim(cache.clone(), "evt_b", 5).await.unwrap(),
            Claim::Fresh(_)
        ));
    }
}
