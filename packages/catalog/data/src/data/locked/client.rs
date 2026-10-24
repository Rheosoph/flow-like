use super::types::LockedStorageStatus;
use flow_like::{
    flow::execution::{ExecutionEnvironment, ExecutorApiAuth, context::ExecutionContext},
    hub::{HubRetry, hub_api_url, hub_response_error, send_hub_request},
};
use flow_like_types::{
    Value, anyhow,
    authorization::ResourceAudience,
    json::{DeserializeOwned, Serialize, to_value},
    reqwest,
};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockedAction {
    Status,
    List,
    Get,
    Upload,
    Commit,
    Abort,
    Download,
    Retention,
    LegalHold,
    AutoDelete,
    Purge,
}

impl LockedAction {
    pub fn segment(self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::List => "list",
            Self::Get => "get",
            Self::Upload => "upload",
            Self::Commit => "commit",
            Self::Abort => "abort",
            Self::Download => "download",
            Self::Retention => "retention",
            Self::LegalHold => "legal-hold",
            Self::AutoDelete => "auto-delete",
            Self::Purge => "purge",
        }
    }

    fn operation(self) -> &'static str {
        match self {
            Self::Status => "Reading the locked storage status",
            Self::List => "Listing locked files",
            Self::Get => "Reading the locked file",
            Self::Upload => "Starting the locked upload",
            Self::Commit => "Committing the locked upload",
            Self::Abort => "Aborting the locked upload",
            Self::Download => "Requesting the locked download",
            Self::Retention => "Extending the locked retention",
            Self::LegalHold => "Setting the legal hold",
            Self::AutoDelete => "Setting automatic deletion",
            Self::Purge => "Deleting the expired locked file",
        }
    }

    fn idempotent(self) -> bool {
        matches!(
            self,
            Self::Status | Self::List | Self::Get | Self::Download | Self::Commit
        )
    }

    fn retry(self) -> HubRetry {
        if self.idempotent() {
            TRANSIENT_RETRY
        } else {
            NO_RETRY
        }
    }
}

/// 503 means the feature is disabled, which no retry changes.
fn transient(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::TOO_MANY_REQUESTS
        || (status.is_server_error() && status != reqwest::StatusCode::SERVICE_UNAVAILABLE)
}

fn never(_: reqwest::StatusCode) -> bool {
    false
}

const TRANSIENT_RETRY: HubRetry = HubRetry {
    retryable: transient,
    retries: 3,
    max_wait: Duration::from_secs(30),
};

const NO_RETRY: HubRetry = HubRetry {
    retryable: never,
    retries: 0,
    max_wait: Duration::ZERO,
};

pub fn locked_url(
    hub: &str,
    secure: bool,
    app_id: &str,
    action: LockedAction,
    execution: bool,
) -> flow_like_types::Result<reqwest::Url> {
    let segment = action.segment();
    if execution {
        hub_api_url(
            hub,
            secure,
            &["execution", "apps", app_id, "locked", segment],
        )
    } else {
        hub_api_url(hub, secure, &["apps", app_id, "locked", segment])
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockedAccess {
    Read,
    Write,
}

enum LockedRoute {
    Execution(ExecutorApiAuth),
    User { hub: String, bearer: Option<String> },
}

/// Locked storage API of the current app: the executor route with the executor's credential on
/// server runs, the user route on the profile hub otherwise.
pub struct LockedApi {
    client: reqwest::Client,
    app_id: String,
    route: LockedRoute,
    secure: bool,
}

impl LockedApi {
    pub fn bind(context: &ExecutionContext, access: LockedAccess) -> flow_like_types::Result<Self> {
        let cache = context
            .execution_cache
            .as_ref()
            .ok_or_else(|| anyhow!("Locked storage nodes require an app execution"))?;
        if access == LockedAccess::Write && cache.shadow {
            return Err(anyhow!("Shadow runs cannot change locked storage"));
        }
        let route = match context.executor_api_auth.clone() {
            Some(auth) => LockedRoute::Execution(auth),
            None if context.execution_environment() == ExecutionEnvironment::Server => {
                return Err(anyhow!(
                    "Locked storage on a server run requires an authenticated executor session"
                ));
            }
            None => {
                let authorizer = context.request_authorizer().is_some();
                let token = context
                    .token
                    .as_deref()
                    .map(str::trim)
                    .filter(|token| !token.is_empty());
                if token.is_none() && !authorizer {
                    return Err(anyhow!(
                        "Locked storage requires signing in to the app's hub '{}'",
                        context.profile.hub
                    ));
                }
                LockedRoute::User {
                    hub: context.profile.hub.clone(),
                    bearer: token.filter(|_| !authorizer).map(str::to_owned),
                }
            }
        };
        // The executor's API base comes from its verified callback claim and may address a
        // private service in Docker or Kubernetes.
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(330))
            .build()?;
        Ok(Self {
            client,
            app_id: cache.app_id.clone(),
            route,
            secure: context.profile.secure,
        })
    }

    fn url(&self, action: LockedAction) -> flow_like_types::Result<reqwest::Url> {
        match &self.route {
            LockedRoute::Execution(auth) => {
                locked_url(auth.api_url(), self.secure, &self.app_id, action, true)
            }
            LockedRoute::User { hub, .. } => {
                locked_url(hub, self.secure, &self.app_id, action, false)
            }
        }
    }

    fn bearer(&self) -> Option<&str> {
        match &self.route {
            LockedRoute::Execution(auth) => Some(auth.token()),
            LockedRoute::User { bearer, .. } => bearer.as_deref(),
        }
    }

    async fn send(
        &self,
        context: &ExecutionContext,
        action: LockedAction,
        body: Option<&Value>,
    ) -> flow_like_types::Result<reqwest::Response> {
        let url = self.url(action)?;
        let executor = matches!(self.route, LockedRoute::Execution(_));
        let (client, url, bearer) = (&self.client, &url, self.bearer());
        send_hub_request(client, action.retry(), move || async move {
            let mut request = match body {
                Some(body) => client.post(url.clone()).json(body),
                None => client.get(url.clone()),
            };
            if let Some(token) = bearer {
                request = request.bearer_auth(token);
            }
            let request = request.build()?;
            if executor {
                return Ok(request);
            }
            context
                .authorize_request(request, ResourceAudience::ProjectApi)
                .await
        })
        .await
    }

    async fn decode<T: DeserializeOwned>(
        action: LockedAction,
        response: reqwest::Response,
    ) -> flow_like_types::Result<T> {
        if !response.status().is_success() {
            return Err(hub_response_error(action.operation(), response).await);
        }
        response.json().await.map_err(|error| {
            anyhow!(
                "{} returned an unexpected body: {error}",
                action.operation()
            )
        })
    }

    pub async fn post<T: DeserializeOwned>(
        &self,
        context: &ExecutionContext,
        action: LockedAction,
        body: &impl Serialize,
    ) -> flow_like_types::Result<T> {
        let body = to_value(body)?;
        let response = self.send(context, action, Some(&body)).await?;
        Self::decode(action, response).await
    }

    /// Like [`Self::post`], with `None` for an unknown item (404).
    pub async fn find<T: DeserializeOwned>(
        &self,
        context: &ExecutionContext,
        action: LockedAction,
        body: &impl Serialize,
    ) -> flow_like_types::Result<Option<T>> {
        let body = to_value(body)?;
        let response = self.send(context, action, Some(&body)).await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        Self::decode(action, response).await.map(Some)
    }

    pub async fn status(
        &self,
        context: &ExecutionContext,
    ) -> flow_like_types::Result<LockedStorageStatus> {
        let response = self.send(context, LockedAction::Status, None).await?;
        Self::decode(LockedAction::Status, response).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn executor_runs_use_the_execution_route_and_users_the_app_route() {
        for hub in ["example.com", "https://example.com/api/v1"] {
            assert_eq!(
                locked_url(hub, true, "app-1", LockedAction::List, false)
                    .unwrap()
                    .as_str(),
                "https://example.com/api/v1/apps/app-1/locked/list"
            );
            assert_eq!(
                locked_url(hub, true, "app-1", LockedAction::LegalHold, true)
                    .unwrap()
                    .as_str(),
                "https://example.com/api/v1/execution/apps/app-1/locked/legal-hold"
            );
        }
        assert_eq!(
            locked_url(
                "http://api.internal:8080",
                true,
                "app/other",
                LockedAction::Purge,
                true
            )
            .unwrap()
            .as_str(),
            "http://api.internal:8080/api/v1/execution/apps/app%2Fother/locked/purge"
        );
    }

    #[test]
    fn only_idempotent_actions_retry_transient_failures() {
        for action in [
            LockedAction::Status,
            LockedAction::List,
            LockedAction::Get,
            LockedAction::Download,
            LockedAction::Commit,
        ] {
            let retry = action.retry();
            assert_eq!(retry.retries, 3, "{action:?}");
            assert!((retry.retryable)(reqwest::StatusCode::TOO_MANY_REQUESTS));
            assert!((retry.retryable)(reqwest::StatusCode::BAD_GATEWAY));
            assert!(!(retry.retryable)(reqwest::StatusCode::SERVICE_UNAVAILABLE));
            assert!(!(retry.retryable)(reqwest::StatusCode::CONFLICT));
        }
        for action in [
            LockedAction::Upload,
            LockedAction::Abort,
            LockedAction::Retention,
            LockedAction::LegalHold,
            LockedAction::AutoDelete,
            LockedAction::Purge,
        ] {
            let retry = action.retry();
            assert_eq!(retry.retries, 0, "{action:?}");
            assert!(!(retry.retryable)(reqwest::StatusCode::TOO_MANY_REQUESTS));
        }
    }
}
