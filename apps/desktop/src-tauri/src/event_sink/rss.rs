use anyhow::Result;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use tauri::{AppHandle, Manager};

use super::manager::{DbConnection, EventSinkManager};
use super::trigger_token::TriggerAccount;
use super::{EventRegistration, EventSink};
use crate::state::TauriEventBusState;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RSSSink {
    pub feed_url: String,
    pub poll_interval: u64,
    pub headers: Option<Vec<(String, String)>>,
    pub filter_keywords: Option<Vec<String>>,
}

impl RSSSink {
    fn init_tables(db: &DbConnection) -> Result<()> {
        let conn = db.lock().unwrap();

        conn.execute(
            "CREATE TABLE IF NOT EXISTS rss_feeds (
                event_id TEXT PRIMARY KEY,
                feed_url TEXT NOT NULL,
                poll_interval INTEGER NOT NULL,
                headers TEXT,
                filter_keywords TEXT,
                last_item_guid TEXT,
                last_pub_date TEXT,
                last_checked INTEGER,
                created_at INTEGER NOT NULL
            )",
            [],
        )?;

        Ok(())
    }

    fn add_feed(
        db: &DbConnection,
        registration: &EventRegistration,
        config: &RSSSink,
    ) -> Result<()> {
        let conn = db.lock().unwrap();

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs() as i64;

        let headers_json = config
            .headers
            .as_ref()
            .and_then(|h| serde_json::to_string(h).ok());

        let keywords_json = config
            .filter_keywords
            .as_ref()
            .and_then(|k| serde_json::to_string(k).ok());

        conn.execute(
            "INSERT OR REPLACE INTO rss_feeds
             (event_id, feed_url, poll_interval, headers, filter_keywords, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                registration.event_id,
                config.feed_url,
                config.poll_interval,
                headers_json,
                keywords_json,
                now,
            ],
        )?;

        Ok(())
    }

    fn remove_feed(db: &DbConnection, event_id: &str) -> Result<()> {
        let conn = db.lock().unwrap();
        conn.execute(
            "DELETE FROM rss_feeds WHERE event_id = ?1",
            params![event_id],
        )?;
        Ok(())
    }

    async fn process_feeds(db: &DbConnection, app_handle: &AppHandle) -> Result<()> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs() as i64;

        let due = Self::due_feeds(db, now)?;
        if due.is_empty() {
            return Ok(());
        }
        for event_id in due {
            tracing::info!("Checking RSS feed for event {}", event_id);

            // TODO: Fetch and parse RSS feed
            // TODO: Compare with last_item_guid
            // For now, we'll trigger an event on each check
            if let Some(registration) =
                Self::firing_saved_registration(db, app_handle, &event_id).await
            {
                Self::push(app_handle, registration);
            }
            Self::mark_checked(db, &event_id, now)?;
        }

        Ok(())
    }

    async fn firing_saved_registration(
        db: &DbConnection,
        app_handle: &AppHandle,
        event_id: &str,
    ) -> Option<EventRegistration> {
        let registration = EventSinkManager::read_registration(db.clone(), event_id);
        let account = if let Ok(Some(row)) = &registration {
            if let Some(binding) = row.profile_binding.as_ref() {
                if let Err(error) = binding.resolve(app_handle).await {
                    tracing::warn!(event_id, error = %error, "RSS trigger's saved profile is unavailable");
                    return None;
                }
                binding.account()
            } else {
                None
            }
        } else {
            None
        };
        Self::firing_registration(event_id, registration, account.as_ref())
    }

    /// The event ids of the feeds due for a check at `now`.
    fn due_feeds(db: &DbConnection, now: i64) -> Result<Vec<String>> {
        let conn = db.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT event_id FROM rss_feeds
             WHERE last_checked IS NULL OR last_checked + poll_interval <= ?1",
        )?;
        let event_ids = stmt
            .query_map(params![now], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(event_ids)
    }

    /// The registration a due feed of `event_id` fires with: none when it is
    /// missing or unreadable, or cannot start a run while `account` is signed
    /// in, like an online trigger whose personal access token was verified
    /// for another account or hub, or never.
    fn firing_registration(
        event_id: &str,
        registration: Result<Option<EventRegistration>>,
        account: Option<&TriggerAccount>,
    ) -> Option<EventRegistration> {
        match registration {
            Ok(Some(registration)) if registration.can_fire(account) => Some(registration),
            Ok(Some(_)) => {
                tracing::warn!(
                    event_id = %event_id,
                    "Skipping an RSS feed without a valid saved profile and trigger token; enable its trigger again"
                );
                None
            }
            Ok(None) => {
                tracing::error!("Could not find registration info for RSS feed {}", event_id);
                None
            }
            Err(error) => {
                tracing::error!(
                    event_id = %event_id,
                    error = %error,
                    "Could not read the registration of an RSS feed"
                );
                None
            }
        }
    }

    fn push(app_handle: &AppHandle, registration: EventRegistration) {
        let event_id = registration.event_id.clone();
        let Some(event_bus_state) = app_handle.try_state::<TauriEventBusState>() else {
            tracing::error!("EventBus state not available for RSS feed {}", event_id);
            return;
        };
        let offline = registration.offline;
        match event_bus_state
            .0
            .push_registration(registration, None, None, None)
        {
            Ok(()) => tracing::info!(
                "RSS event {} triggered successfully (offline: {})",
                event_id,
                offline
            ),
            Err(e) => tracing::error!("Failed to push RSS event to EventBus: {}", e),
        }
    }

    fn mark_checked(db: &DbConnection, event_id: &str, now: i64) -> Result<()> {
        let conn = db.lock().unwrap();
        conn.execute(
            "UPDATE rss_feeds SET last_checked = ?1 WHERE event_id = ?2",
            params![now, event_id],
        )?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl EventSink for RSSSink {
    async fn start(&self, app_handle: &AppHandle, db: DbConnection) -> Result<()> {
        Self::init_tables(&db)?;

        let app_handle = app_handle.clone();
        let running = Arc::new(AtomicBool::new(true));
        let worker_running = running.clone();
        let worker_db = db.clone();

        flow_like_types::tokio::spawn(async move {
            tracing::info!("RSS worker thread started");

            while worker_running.load(Ordering::Relaxed) {
                if let Err(e) = Self::process_feeds(&worker_db, &app_handle).await {
                    tracing::error!("Error processing RSS feeds: {}", e);
                }

                flow_like_types::tokio::time::sleep(Duration::from_secs(10)).await;
            }

            tracing::info!("RSS worker thread stopped");
        });

        tracing::info!("RSS sink started - worker ready");
        Ok(())
    }

    async fn stop(&self, _app_handle: &AppHandle, _db: DbConnection) -> Result<()> {
        tracing::info!("RSS sink stopped");
        Ok(())
    }

    async fn on_register(
        &self,
        _app_handle: &AppHandle,
        registration: &EventRegistration,
        db: DbConnection,
    ) -> Result<()> {
        Self::add_feed(&db, registration, self)?;
        tracing::info!("Registered RSS feed for event {}", registration.event_id);
        Ok(())
    }

    async fn on_unregister(
        &self,
        _app_handle: &AppHandle,
        registration: &EventRegistration,
        db: DbConnection,
    ) -> Result<()> {
        Self::remove_feed(&db, &registration.event_id)?;
        tracing::info!("Unregistered RSS feed: {}", registration.event_id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_sink::EventConfig;
    use std::collections::HashMap;

    /// A feed registration whose PAT, when `owner` is set, was verified for
    /// that subject.
    fn feed(offline: bool, pat: Option<&str>, owner: Option<&str>) -> EventRegistration {
        let now = std::time::SystemTime::now();
        EventRegistration {
            event_id: "event".to_string(),
            name: "Feed".to_string(),
            r#type: "rss".to_string(),
            updated_at: now,
            created_at: now,
            config: EventConfig::Rss(RSSSink {
                feed_url: "https://example.com/feed.xml".to_string(),
                poll_interval: 300,
                headers: None,
                filter_keywords: None,
            }),
            offline,
            app_id: "app".to_string(),
            default_payload: None,
            personal_access_token: pat.map(str::to_string),
            pat_owner_sub: owner.map(str::to_string),
            hub_origin: owner.map(|_| "https://hub.example".to_string()),
            profile_binding: Some(crate::event_sink::trigger_token::TriggerProfile {
                profile_id: "profile-1".into(),
                hub_origin: "https://hub.example".into(),
                issuer: owner.map(|_| "https://issuer.example".into()),
                subject: owner.map(str::to_owned),
            }),
            remote: false,
            oauth_tokens: HashMap::new(),
        }
    }

    fn signed_in(sub: &str, hub_origin: &str) -> TriggerAccount {
        TriggerAccount {
            profile_id: "profile-1".into(),
            issuer: "https://issuer.example".into(),
            sub: sub.to_string(),
            hub_origin: hub_origin.to_string(),
        }
    }

    fn fires(
        registration: Result<Option<EventRegistration>>,
        account: Option<&TriggerAccount>,
    ) -> bool {
        RSSSink::firing_registration("event", registration, account).is_some()
    }

    #[test]
    fn a_due_feed_skips_a_registration_that_cannot_fire() {
        let owner = signed_in("user-1", "https://hub.example");
        let owner = Some(&owner);
        assert!(fires(Ok(Some(feed(true, None, None))), owner));
        assert!(fires(
            Ok(Some(feed(false, Some("pat_1"), Some("user-1")))),
            owner
        ));
        assert!(!fires(
            Ok(Some(feed(false, Some("pat_legacy"), None))),
            owner
        ));
        assert!(!fires(
            Ok(Some(feed(false, Some("  "), Some("user-1")))),
            owner
        ));
        assert!(!fires(Ok(Some(feed(false, None, None))), owner));
        assert!(!fires(Ok(None), owner));
        assert!(!fires(
            Err(anyhow::anyhow!("unreadable registration")),
            owner
        ));
    }

    #[test]
    fn an_online_feed_fires_only_while_its_token_owner_is_signed_in_on_its_hub() {
        let online = || Ok(Some(feed(false, Some("pat_1"), Some("user-1"))));
        let other_account = signed_in("user-2", "https://hub.example");
        let other_hub = signed_in("user-1", "https://other-hub.example");
        assert!(!fires(online(), Some(&other_account)));
        assert!(!fires(online(), Some(&other_hub)));
        assert!(!fires(online(), None));

        let offline = || Ok(Some(feed(true, None, None)));
        assert!(fires(offline(), None));
        assert!(fires(offline(), Some(&other_account)));
    }
}
