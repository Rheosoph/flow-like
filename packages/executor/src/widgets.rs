//! Declarative widgets for a run, fetched from the hub instead of the meta store.
//!
//! The `Instantiate Widget` node used to read `apps/{app}/manifest.app` and
//! every `{widget}.widget` straight from meta storage, which was the one
//! run-time read that kept a storage credential in the executor. The hub
//! already authenticates this executor by its JWT for progress reporting, so
//! widgets travel the same way: one authenticated GET per app per run, cached
//! for the life of the run so N instantiations of the same widget cost one call.

use crate::resolve::{fetch_bounded_with, max_remote_payload_bytes};
use flow_like::a2ui::micro_widget::AppWidgetSource;
use flow_like::a2ui::widget::Widget;
use flow_like_types::{anyhow, async_trait};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

/// How a run reaches its hub: the callback URL from the executor JWT claims
/// and the JWT itself, which the hub accepts as this executor's identity.
pub(crate) struct HubAccess {
    pub callback_url: String,
    pub jwt: String,
}

pub struct HubWidgetSource {
    base_url: String,
    jwt: String,
    cache: Mutex<HashMap<String, Arc<Vec<Widget>>>>,
}

impl HubWidgetSource {
    pub fn new(callback_url: &str, jwt: String) -> Self {
        Self {
            base_url: callback_url.trim_end_matches('/').to_string(),
            jwt,
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// The hub route serving an app's declarative widgets to its executors.
    pub fn widgets_url(&self, app_id: &str) -> String {
        format!("{}/api/v1/execution/apps/{app_id}/widgets", self.base_url)
    }

    async fn fetch(&self, app_id: &str) -> flow_like_types::Result<Vec<Widget>> {
        let url = self.widgets_url(app_id);
        // The JWT is a bearer credential: it goes in the header and never in an
        // error, which is why the failure below names the app, not the request.
        let body = fetch_bounded_with(&url, Some(&self.jwt), max_remote_payload_bytes())
            .await
            .map_err(|e| anyhow!("failed to load widgets of app {app_id} from the hub: {e}"))?;
        serde_json::from_slice(&body)
            .map_err(|e| anyhow!("hub returned unreadable widgets for app {app_id}: {e}"))
    }

    #[cfg(test)]
    async fn prime(&self, app_id: &str, widgets: Vec<Widget>) {
        self.cache
            .lock()
            .await
            .insert(app_id.to_string(), Arc::new(widgets));
    }
}

#[async_trait]
impl AppWidgetSource for HubWidgetSource {
    async fn list_app_widgets(&self, app_id: &str) -> flow_like_types::Result<Arc<Vec<Widget>>> {
        if let Some(widgets) = self.cache.lock().await.get(app_id) {
            return Ok(widgets.clone());
        }
        let widgets = Arc::new(self.fetch(app_id).await?);
        self.cache
            .lock()
            .await
            .insert(app_id.to_string(), widgets.clone());
        Ok(widgets)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widgets_url_is_the_hub_execution_route_without_a_double_slash() {
        let source = HubWidgetSource::new("https://api.example/", "jwt".into());
        assert_eq!(
            source.widgets_url("app-1"),
            "https://api.example/api/v1/execution/apps/app-1/widgets"
        );
    }

    #[tokio::test]
    async fn a_cached_app_is_served_without_touching_the_network() {
        // Port 9 (discard) refuses connections, so any fetch would fail loudly.
        let source = HubWidgetSource::new("http://127.0.0.1:9", "jwt".into());
        source.prime("app-1", Vec::new()).await;
        let first = source
            .list_app_widgets("app-1")
            .await
            .expect("served from cache");
        let second = source
            .list_app_widgets("app-1")
            .await
            .expect("served from cache");
        assert!(
            Arc::ptr_eq(&first, &second),
            "one entry, shared across calls"
        );
    }

    #[tokio::test]
    async fn an_unknown_app_reaches_the_hub_and_reports_the_app_not_the_token() {
        let source = HubWidgetSource::new("http://127.0.0.1:9", "secret-jwt".into());
        let error = source
            .list_app_widgets("app-2")
            .await
            .err()
            .expect("connection refused");
        let message = error.to_string();
        assert!(message.contains("app-2"), "{message}");
        assert!(!message.contains("secret-jwt"), "{message}");
    }
}
