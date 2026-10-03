//! Widgets for a run, fetched from the hub instead of the meta store.
//!
//! The `Instantiate Widget` node used to read `apps/{app}/manifest.app` and
//! every `{widget}.widget` straight from meta storage, which was the one
//! run-time read that kept a storage credential in the executor. The widgets
//! of the packages an app pins were never readable that way: the hub keeps the
//! pins in its database, and the manifest it stores is not kept in step. An
//! executor has neither a meta store nor a database, so both lists come from
//! the hub. It already authenticates this executor by its JWT for progress
//! reporting, so widgets travel the same way: one authenticated GET per list
//! and app per run, cached for the life of the run so N instantiations of the
//! same widget cost one call.

use crate::resolve::{fetch_bounded_with, max_remote_payload_bytes};
use flow_like::a2ui::micro_widget::{AppWidgetSource, PackageWidgetRef, PackageWidgetSource};
use flow_like::a2ui::widget::Widget;
use flow_like::state::FlowLikeState;
use flow_like_types::{anyhow, async_trait};
use serde::de::DeserializeOwned;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

/// How a run reaches its hub: the callback URL from the executor JWT claims
/// and the JWT itself, which the hub accepts as this executor's identity.
pub(crate) struct HubAccess {
    pub callback_url: String,
    pub jwt: String,
    pub hosted_frontend: bool,
}

impl HubAccess {
    /// Point `state` at the hub for what the executor has no store or database
    /// for: one source answers for the app's own widgets and for the widgets of
    /// the packages it pins. For a hosted frontend the JWT is also the run's
    /// credential for hosted model calls.
    pub(crate) async fn register_on(self, state: &mut FlowLikeState) {
        if self.hosted_frontend {
            state.hosted_model_token = Some(self.jwt.clone());
        }
        let widgets = Arc::new(HubWidgetSource::new(&self.callback_url, self.jwt));
        state.register_app_widget_source(widgets.clone()).await;
        state.register_package_widget_source(widgets).await;
    }
}

/// One hub list per app, kept for the life of the run.
type PerApp<T> = Mutex<HashMap<String, Arc<Vec<T>>>>;

pub struct HubWidgetSource {
    base_url: String,
    jwt: String,
    cache: PerApp<Widget>,
    package_cache: PerApp<PackageWidgetRef>,
}

impl HubWidgetSource {
    pub fn new(callback_url: &str, jwt: String) -> Self {
        Self {
            base_url: callback_url.trim_end_matches('/').to_string(),
            jwt,
            cache: Mutex::new(HashMap::new()),
            package_cache: Mutex::new(HashMap::new()),
        }
    }

    /// The hub route serving an app's declarative widgets to its executors.
    pub fn widgets_url(&self, app_id: &str) -> String {
        format!("{}/api/v1/execution/apps/{app_id}/widgets", self.base_url)
    }

    /// The hub route serving the widgets of an app's pinned packages to its
    /// executors.
    pub fn package_widgets_url(&self, app_id: &str) -> String {
        format!(
            "{}/api/v1/execution/apps/{app_id}/package-widgets",
            self.base_url
        )
    }

    /// One of an app's lists, from `cache` when this run already asked for it.
    /// Only an answer is cached, an empty one like any other: after a failure
    /// the next call asks the hub again.
    async fn load<T: DeserializeOwned>(
        &self,
        cache: &PerApp<T>,
        url: String,
        app_id: &str,
        what: &str,
    ) -> flow_like_types::Result<Arc<Vec<T>>> {
        if let Some(entries) = cache.lock().await.get(app_id) {
            return Ok(entries.clone());
        }
        // The JWT is a bearer credential: it goes in the header and never in an
        // error, which is why the failures below name the app, not the request.
        let body = fetch_bounded_with(&url, Some(&self.jwt), max_remote_payload_bytes())
            .await
            .map_err(|e| anyhow!("failed to load {what} of app {app_id} from the hub: {e}"))?;
        let entries: Vec<T> = serde_json::from_slice(&body)
            .map_err(|e| anyhow!("hub returned unreadable {what} for app {app_id}: {e}"))?;
        let entries = Arc::new(entries);
        cache
            .lock()
            .await
            .insert(app_id.to_string(), entries.clone());
        Ok(entries)
    }

    #[cfg(test)]
    async fn prime(&self, app_id: &str, widgets: Vec<Widget>) {
        self.cache
            .lock()
            .await
            .insert(app_id.to_string(), Arc::new(widgets));
    }

    #[cfg(test)]
    async fn prime_package_widgets(&self, app_id: &str, widgets: Vec<PackageWidgetRef>) {
        self.package_cache
            .lock()
            .await
            .insert(app_id.to_string(), Arc::new(widgets));
    }
}

#[async_trait]
impl AppWidgetSource for HubWidgetSource {
    async fn list_app_widgets(&self, app_id: &str) -> flow_like_types::Result<Arc<Vec<Widget>>> {
        self.load(&self.cache, self.widgets_url(app_id), app_id, "widgets")
            .await
    }
}

#[async_trait]
impl PackageWidgetSource for HubWidgetSource {
    async fn list_widgets(
        &self,
        app_id: &str,
        _state: Arc<FlowLikeState>,
    ) -> flow_like_types::Result<Vec<PackageWidgetRef>> {
        let url = self.package_widgets_url(app_id);
        let widgets = self
            .load(&self.package_cache, url, app_id, "package widgets")
            .await?;
        Ok(widgets.as_ref().clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::Path;
    use axum::http::header::AUTHORIZATION;
    use axum::http::{HeaderMap, StatusCode};
    use axum::routing::get;
    use axum::{Json, Router};
    use flow_like::a2ui::micro_widget::load_package_widgets;
    use flow_like::state::FlowLikeConfig;
    use flow_like::utils::http::HTTPClient;

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

    /// App id and `Authorization` header of each request a test hub received.
    type Requests = Arc<Mutex<Vec<(String, String)>>>;

    /// A state without any store: the hub source must answer from the hub alone.
    fn state() -> FlowLikeState {
        FlowLikeState::new(FlowLikeConfig::new(), HTTPClient::new_without_refetch())
    }

    fn entry(widget_id: &str) -> PackageWidgetRef {
        PackageWidgetRef {
            package_id: "com.example.sales".into(),
            package_version: "1.2.0".into(),
            widget_id: widget_id.into(),
            name: "Sales Chart".into(),
            description: "Interactive chart".into(),
            bundle_hash: Some("abc123".into()),
            contract: serde_json::json!({ "contractVersion": 1, "id": widget_id }),
        }
    }

    /// `PackageWidgetRef` has no `PartialEq`; its wire form compares every field.
    fn wire(widgets: &[PackageWidgetRef]) -> serde_json::Value {
        serde_json::to_value(widgets).expect("package widgets serialise")
    }

    /// A hub on a local port that serves `apps` one of their lists at `route`
    /// (`widgets` or `package-widgets`) and refuses every other app, the way
    /// the real one refuses a run that is not bound to the app it asks about.
    /// It knows no other path, so a source that asks for the wrong list fails.
    async fn hub<T>(
        route: &str,
        apps: HashMap<&'static str, Vec<T>>,
    ) -> (String, Requests, tokio::task::JoinHandle<()>)
    where
        T: serde::Serialize + Clone + Send + Sync + 'static,
    {
        let apps = Arc::new(apps);
        let requests = Requests::default();
        let received = requests.clone();
        let router = Router::new().route(
            &format!("/api/v1/execution/apps/{{app_id}}/{route}"),
            get(move |Path(app_id): Path<String>, headers: HeaderMap| {
                let (apps, received) = (apps.clone(), received.clone());
                async move {
                    let authorization = headers
                        .get(AUTHORIZATION)
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or_default()
                        .to_string();
                    let answer = apps.get(app_id.as_str()).cloned();
                    received.lock().await.push((app_id, authorization));
                    answer.map(Json).ok_or(StatusCode::FORBIDDEN)
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        (base, requests, server)
    }

    #[test]
    fn package_widgets_url_is_the_hub_execution_route_without_a_double_slash() {
        let source = HubWidgetSource::new("https://api.example/", "jwt".into());
        assert_eq!(
            source.package_widgets_url("app-1"),
            "https://api.example/api/v1/execution/apps/app-1/package-widgets"
        );
    }

    #[tokio::test]
    async fn cached_package_widgets_are_served_without_touching_the_network() {
        // Port 9 (discard) refuses connections, so any fetch would fail loudly.
        let source = HubWidgetSource::new("http://127.0.0.1:9", "jwt".into());
        source
            .prime_package_widgets("app-1", vec![entry("sales-chart")])
            .await;
        let widgets = source
            .list_widgets("app-1", Arc::new(state()))
            .await
            .expect("served from cache");
        assert_eq!(wire(&widgets), wire(&[entry("sales-chart")]));
    }

    #[tokio::test]
    async fn package_widgets_from_an_unreachable_hub_report_the_app_not_the_token() {
        let source = HubWidgetSource::new("http://127.0.0.1:9", "secret-jwt".into());
        let error = source
            .list_widgets("app-2", Arc::new(state()))
            .await
            .err()
            .expect("connection refused");
        let message = error.to_string();
        assert!(message.contains("app-2"), "{message}");
        assert!(!message.contains("secret-jwt"), "{message}");
    }

    #[tokio::test]
    async fn package_widgets_come_from_the_hub_once_per_app_under_the_executor_jwt() {
        let apps = HashMap::from([
            ("app-1", vec![entry("sales-chart")]),
            ("app-2", vec![entry("stock-table")]),
        ]);
        let (base, requests, server) = hub("package-widgets", apps).await;
        let source = HubWidgetSource::new(&base, "executor-jwt".into());
        let state = Arc::new(state());

        let first = source
            .list_widgets("app-1", state.clone())
            .await
            .expect("served by the hub");
        let repeat = source
            .list_widgets("app-1", state.clone())
            .await
            .expect("served from cache");
        let other = source
            .list_widgets("app-2", state)
            .await
            .expect("served by the hub");
        server.abort();

        assert_eq!(wire(&first), wire(&[entry("sales-chart")]));
        assert_eq!(wire(&repeat), wire(&first));
        assert_eq!(wire(&other), wire(&[entry("stock-table")]));
        assert_eq!(
            *requests.lock().await,
            [
                ("app-1".to_string(), "Bearer executor-jwt".to_string()),
                ("app-2".to_string(), "Bearer executor-jwt".to_string()),
            ],
            "one authenticated request per app; the repeat is answered from the cache"
        );
    }

    #[tokio::test]
    async fn an_app_without_package_widgets_is_cached_like_any_other() {
        let nothing_pinned = HashMap::from([("app-1", Vec::<PackageWidgetRef>::new())]);
        let (base, requests, server) = hub("package-widgets", nothing_pinned).await;
        let source = HubWidgetSource::new(&base, "executor-jwt".into());
        let state = Arc::new(state());

        for _ in 0..2 {
            let widgets = source
                .list_widgets("app-1", state.clone())
                .await
                .expect("an empty list is an answer");
            assert!(widgets.is_empty());
        }
        server.abort();

        let asked = requests.lock().await.len();
        assert_eq!(asked, 1, "the empty answer is cached like any other");
    }

    #[tokio::test]
    async fn a_refused_app_reports_the_app_not_the_token_and_is_asked_again() {
        let no_app = HashMap::<&str, Vec<PackageWidgetRef>>::new();
        let (base, requests, server) = hub("package-widgets", no_app).await;
        let source = HubWidgetSource::new(&base, "secret-jwt".into());
        let state = Arc::new(state());

        for _ in 0..2 {
            let error = source
                .list_widgets("app-9", state.clone())
                .await
                .err()
                .expect("the hub answers 403");
            let message = error.to_string();
            assert!(message.contains("app-9"), "{message}");
            assert!(!message.contains("secret-jwt"), "{message}");
        }
        server.abort();

        let asked = requests.lock().await.len();
        assert_eq!(asked, 2, "a refusal is not cached");
    }

    #[tokio::test]
    async fn a_hub_registers_the_app_and_the_package_widget_source() {
        let mut state = state();
        let access = HubAccess {
            callback_url: "http://127.0.0.1:9".into(),
            jwt: "jwt".into(),
            hosted_frontend: false,
        };

        access.register_on(&mut state).await;

        assert!(state.app_widget_source().await.is_some());
        assert!(
            state.package_widget_source().await.is_some(),
            "without it Instantiate Widget finds no package widget on an executor"
        );
    }

    #[tokio::test]
    async fn the_registered_source_asks_the_runs_hub_under_the_runs_token() {
        let apps = HashMap::from([("app-1", vec![entry("sales-chart")])]);
        let (base, requests, server) = hub("package-widgets", apps).await;
        let mut state = state();
        let access = HubAccess {
            callback_url: base,
            jwt: "executor-jwt".into(),
            hosted_frontend: false,
        };
        access.register_on(&mut state).await;

        // The way the Instantiate Widget node reaches the source.
        let widgets = load_package_widgets("app-1", Arc::new(state))
            .await
            .expect("served by the hub");
        server.abort();

        assert_eq!(wire(&widgets), wire(&[entry("sales-chart")]));
        assert_eq!(
            *requests.lock().await,
            [("app-1".to_string(), "Bearer executor-jwt".to_string())]
        );
    }

    #[tokio::test]
    async fn only_a_hosted_frontend_run_uses_its_token_for_hosted_models() {
        for (hosted_frontend, token) in [(true, Some("jwt")), (false, None)] {
            let mut state = state();
            let access = HubAccess {
                callback_url: "http://127.0.0.1:9".into(),
                jwt: "jwt".into(),
                hosted_frontend,
            };

            access.register_on(&mut state).await;

            assert_eq!(
                state.hosted_model_token.as_deref(),
                token,
                "hosted_frontend = {hosted_frontend}"
            );
        }
    }

    #[tokio::test]
    async fn an_apps_own_widgets_come_from_the_hub_once_under_the_executor_jwt() {
        let widget = Widget::new("widget-1", "My Widget", "root");
        let apps = HashMap::from([("app-1", vec![widget.clone()])]);
        let (base, requests, server) = hub("widgets", apps).await;
        let source = HubWidgetSource::new(&base, "executor-jwt".into());

        let first = source
            .list_app_widgets("app-1")
            .await
            .expect("served by the hub");
        let repeat = source
            .list_app_widgets("app-1")
            .await
            .expect("served from cache");
        server.abort();

        assert!(
            Arc::ptr_eq(&first, &repeat),
            "one entry, shared across calls"
        );
        assert_eq!(
            serde_json::to_value(first.as_ref()).expect("widgets serialise"),
            serde_json::to_value(vec![widget]).expect("widgets serialise")
        );
        assert_eq!(
            *requests.lock().await,
            [("app-1".to_string(), "Bearer executor-jwt".to_string())],
            "one authenticated request to the widgets route; the repeat is answered from the cache"
        );
    }
}
