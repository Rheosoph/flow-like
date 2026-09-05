// Stub implementations for remaining sinks
// These provide basic structure but need full implementation later

use anyhow::Result;
use tauri::AppHandle;

use super::manager::DbConnection;
use super::{EventRegistration, EventSink};

// ========== PLACEHOLDER IMPLEMENTATIONS ==========
// These sinks have minimal implementations to allow compilation
// TODO: Implement full functionality with proper tables and infrastructure

// File Sink
#[async_trait::async_trait]
impl EventSink for crate::event_sink::file::FileSink {
    async fn start(&self, _app_handle: &AppHandle, _db: DbConnection) -> Result<()> {
        tracing::warn!("FileSink not yet fully implemented");
        Ok(())
    }

    async fn stop(&self, _app_handle: &AppHandle, _db: DbConnection) -> Result<()> {
        Ok(())
    }

    async fn on_register(
        &self,
        _app_handle: &AppHandle,
        registration: &EventRegistration,
        _db: DbConnection,
    ) -> Result<()> {
        tracing::info!(
            "Registered file watcher: {} -> event {} (NOT IMPLEMENTED)",
            self.path,
            registration.event_id
        );
        Ok(())
    }

    async fn on_unregister(
        &self,
        _app_handle: &AppHandle,
        registration: &EventRegistration,
        _db: DbConnection,
    ) -> Result<()> {
        tracing::info!("Unregistered file watcher: {}", registration.event_id);
        Ok(())
    }
}

// Webhook Sink
#[async_trait::async_trait]
impl EventSink for crate::event_sink::webhook::WebhookSink {
    async fn start(&self, _app_handle: &AppHandle, _db: DbConnection) -> Result<()> {
        tracing::warn!("WebhookSink not yet fully implemented");
        Ok(())
    }

    async fn stop(&self, _app_handle: &AppHandle, _db: DbConnection) -> Result<()> {
        Ok(())
    }

    async fn on_register(
        &self,
        _app_handle: &AppHandle,
        registration: &EventRegistration,
        _db: DbConnection,
    ) -> Result<()> {
        tracing::info!(
            "Registered webhook: {} -> event {} (NOT IMPLEMENTED)",
            self.path,
            registration.event_id
        );
        Ok(())
    }

    async fn on_unregister(
        &self,
        _app_handle: &AppHandle,
        registration: &EventRegistration,
        _db: DbConnection,
    ) -> Result<()> {
        tracing::info!("Unregistered webhook: {}", registration.event_id);
        Ok(())
    }
}

// Slack Sink
#[async_trait::async_trait]
impl EventSink for crate::event_sink::slack::SlackSink {
    async fn start(&self, _app_handle: &AppHandle, _db: DbConnection) -> Result<()> {
        tracing::warn!("SlackSink not yet fully implemented");
        Ok(())
    }

    async fn stop(&self, _app_handle: &AppHandle, _db: DbConnection) -> Result<()> {
        Ok(())
    }

    async fn on_register(
        &self,
        _app_handle: &AppHandle,
        registration: &EventRegistration,
        _db: DbConnection,
    ) -> Result<()> {
        tracing::info!(
            "Registered Slack: channel {} -> event {} (NOT IMPLEMENTED)",
            self.channel_id.as_deref().unwrap_or("any"),
            registration.event_id
        );
        Ok(())
    }

    async fn on_unregister(
        &self,
        _app_handle: &AppHandle,
        registration: &EventRegistration,
        _db: DbConnection,
    ) -> Result<()> {
        tracing::info!("Unregistered Slack: {}", registration.event_id);
        Ok(())
    }
}

// Web Watcher Sink
#[async_trait::async_trait]
impl EventSink for crate::event_sink::web_watcher::WebWatcherSink {
    async fn start(&self, _app_handle: &AppHandle, _db: DbConnection) -> Result<()> {
        tracing::warn!("WebWatcherSink not yet fully implemented");
        Ok(())
    }

    async fn stop(&self, _app_handle: &AppHandle, _db: DbConnection) -> Result<()> {
        Ok(())
    }

    async fn on_register(
        &self,
        _app_handle: &AppHandle,
        registration: &EventRegistration,
        _db: DbConnection,
    ) -> Result<()> {
        tracing::info!(
            "Registered web watcher for event {} (NOT IMPLEMENTED)",
            registration.event_id
        );
        Ok(())
    }

    async fn on_unregister(
        &self,
        _app_handle: &AppHandle,
        registration: &EventRegistration,
        _db: DbConnection,
    ) -> Result<()> {
        tracing::info!("Unregistered web watcher: {}", registration.event_id);
        Ok(())
    }
}

// GeoLocation Sink
#[async_trait::async_trait]
impl EventSink for crate::event_sink::geolocation::GeoLocationSink {
    async fn start(&self, _app_handle: &AppHandle, _db: DbConnection) -> Result<()> {
        tracing::warn!("GeoLocationSink not yet fully implemented");
        Ok(())
    }

    async fn stop(&self, _app_handle: &AppHandle, _db: DbConnection) -> Result<()> {
        Ok(())
    }

    async fn on_register(
        &self,
        _app_handle: &AppHandle,
        registration: &EventRegistration,
        _db: DbConnection,
    ) -> Result<()> {
        tracing::info!(
            "Registered geofence: ({}, {}) -> event {} (NOT IMPLEMENTED)",
            self.latitude,
            self.longitude,
            registration.event_id
        );
        Ok(())
    }

    async fn on_unregister(
        &self,
        _app_handle: &AppHandle,
        registration: &EventRegistration,
        _db: DbConnection,
    ) -> Result<()> {
        tracing::info!("Unregistered geofence: {}", registration.event_id);
        Ok(())
    }
}

// GitHub Sink
#[async_trait::async_trait]
impl EventSink for crate::event_sink::github::GitHubSink {
    async fn start(&self, _app_handle: &AppHandle, _db: DbConnection) -> Result<()> {
        tracing::warn!("GitHubSink not yet fully implemented");
        Ok(())
    }

    async fn stop(&self, _app_handle: &AppHandle, _db: DbConnection) -> Result<()> {
        Ok(())
    }

    async fn on_register(
        &self,
        _app_handle: &AppHandle,
        registration: &EventRegistration,
        _db: DbConnection,
    ) -> Result<()> {
        tracing::info!(
            "Registered GitHub watcher: {} -> event {} (NOT IMPLEMENTED)",
            self.repository,
            registration.event_id
        );
        Ok(())
    }

    async fn on_unregister(
        &self,
        _app_handle: &AppHandle,
        registration: &EventRegistration,
        _db: DbConnection,
    ) -> Result<()> {
        tracing::info!("Unregistered GitHub watcher: {}", registration.event_id);
        Ok(())
    }
}

// MQTT Sink
#[async_trait::async_trait]
impl EventSink for crate::event_sink::mqtt::MQTTSink {
    async fn start(&self, _app_handle: &AppHandle, _db: DbConnection) -> Result<()> {
        tracing::warn!("MQTTSink not yet fully implemented");
        Ok(())
    }

    async fn stop(&self, _app_handle: &AppHandle, _db: DbConnection) -> Result<()> {
        Ok(())
    }

    async fn on_register(
        &self,
        _app_handle: &AppHandle,
        registration: &EventRegistration,
        _db: DbConnection,
    ) -> Result<()> {
        tracing::info!(
            "Registered MQTT subscription for event {} (NOT IMPLEMENTED)",
            registration.event_id
        );
        Ok(())
    }

    async fn on_unregister(
        &self,
        _app_handle: &AppHandle,
        registration: &EventRegistration,
        _db: DbConnection,
    ) -> Result<()> {
        tracing::info!("Unregistered MQTT subscription: {}", registration.event_id);
        Ok(())
    }
}

// Notion Sink
#[async_trait::async_trait]
impl EventSink for crate::event_sink::notion::NotionSink {
    async fn start(&self, _app_handle: &AppHandle, _db: DbConnection) -> Result<()> {
        tracing::warn!("NotionSink not yet fully implemented");
        Ok(())
    }

    async fn stop(&self, _app_handle: &AppHandle, _db: DbConnection) -> Result<()> {
        Ok(())
    }

    async fn on_register(
        &self,
        _app_handle: &AppHandle,
        registration: &EventRegistration,
        _db: DbConnection,
    ) -> Result<()> {
        tracing::info!(
            "Registered Notion watcher: db {:?} -> event {} (NOT IMPLEMENTED)",
            self.database_id,
            registration.event_id
        );
        Ok(())
    }

    async fn on_unregister(
        &self,
        _app_handle: &AppHandle,
        registration: &EventRegistration,
        _db: DbConnection,
    ) -> Result<()> {
        tracing::info!("Unregistered Notion watcher: {}", registration.event_id);
        Ok(())
    }
}
