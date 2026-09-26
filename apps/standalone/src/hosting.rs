use crate::config::{HostingConfig, PlacementConfig};
use anyhow::{Context, Result, ensure};
use axum::{
    Router,
    body::to_bytes,
    extract::{Request, State},
    http::{HeaderMap, StatusCode},
    response::{
        IntoResponse, Response, Sse,
        sse::{Event as SseEvent, KeepAlive},
    },
};
use flow_like_runtime::{
    app::AppVisibility,
    flow::{
        compiled::CompiledRunTemplate,
        event::Event,
        execution::{InternalRun, LogLevel, RunPayload, RunStatus},
    },
    profile::Profile,
    state::FlowLikeState,
};
use flow_like_types::{
    intercom::{InterComCallback, InterComEvent},
    utils::constant_time_eq,
};
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::HashMap,
    convert::Infallible,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{
    net::TcpListener,
    sync::{Mutex, Semaphore, mpsc, oneshot},
};
use tokio_util::sync::CancellationToken;

const BODY_LIMIT: usize = 10 * 1024 * 1024;

mod actions;
mod channels;
#[cfg(unix)]
mod forward;
#[cfg(feature = "frontend")]
mod frontend;
mod pages;

#[derive(Clone, Copy)]
pub struct ReplicaContext {
    pub supervisor_pid: u32,
    pub config_revision: u64,
    pub slot: u8,
}

#[derive(Clone)]
pub(crate) struct PreparedInvocation {
    pub event: Event,
    pub template: Arc<CompiledRunTemplate>,
    pub(crate) action_admission: Option<actions::Admission>,
}

#[derive(Deserialize)]
struct HttpEventConfig {
    path: String,
    method: String,
}

fn event_route(event: &Event) -> Result<(String, String)> {
    if event.event_type == "simple_chat" {
        return Ok(("POST".into(), format!("/chat/{}", event.id)));
    }
    ensure!(event.event_type == "http", "Unsupported hosted event type");
    let config: HttpEventConfig = serde_json::from_slice(&event.config)
        .context("Invalid HTTP event routing configuration")?;
    let method = config.method.to_ascii_uppercase();
    ensure!(
        matches!(
            method.as_str(),
            "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS"
        ),
        "Unsupported HTTP event method"
    );
    ensure!(
        config.path.starts_with('/')
            && config.path.len() <= 2048
            && config.path.bytes().all(|b| b.is_ascii_graphic())
            && !config.path.contains(['?', '#', '\\', '{', '}', '*'])
            && !config.path.split('/').any(|p| p == "." || p == ".."),
        "HTTP events require a literal absolute path"
    );
    Ok((method, config.path))
}

fn secret_path(config: &PlacementConfig, hosting: &HostingConfig) -> Result<PathBuf> {
    let path = crate::config::private_secret_path(config, &hosting.auth_secret)?;
    read_token(&path)?;
    Ok(path)
}
pub(crate) fn validate_access_token(config: &PlacementConfig) -> Result<()> {
    let hosting = config
        .hosting
        .as_ref()
        .context("Hosted rollout requires hosting configuration")?;
    hosting.validate()?;
    secret_path(config, hosting)?;
    Ok(())
}
fn read_token(path: &Path) -> Result<zeroize::Zeroizing<Vec<u8>>> {
    let token = crate::vault::read_private(path)?;
    ensure!(
        (32..=4096).contains(&token.len()) && token.iter().all(|b| b.is_ascii_graphic()),
        "Service access token must contain 32 to 4096 printable bytes without whitespace"
    );
    Ok(token)
}
fn authenticated(headers: &HeaderMap, path: &Path) -> Result<Option<blake3::Hash>> {
    if headers.get_all("authorization").iter().count() != 1 {
        return Ok(None);
    }
    let Some(value) = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
    else {
        return Ok(None);
    };
    let token = read_token(path)?;
    Ok(constant_time_eq(value.as_bytes(), &token).then(|| blake3::hash(&token)))
}

struct HostState {
    events: HashMap<(String, String), PreparedInvocation>,
    pages: HashMap<String, pages::PreparedPage>,
    state: Arc<FlowLikeState>,
    profile: Profile,
    project_id: String,
    visibility: AppVisibility,
    execution_sub: Option<String>,
    secret: PathBuf,
    timeout: Duration,
    capacity: Arc<Semaphore>,
    cancel: CancellationToken,
    channels: channels::Channels,
    actions: Arc<actions::Registry>,
    #[cfg(unix)]
    reply_route: Option<Arc<forward::Route>>,
    inventory: Value,
}
pub(crate) struct PreparedHost {
    listener: TcpListener,
    state: Arc<HostState>,
    #[cfg(unix)]
    reply_listener: Option<forward::Listener>,
}

struct ReadyListener {
    listener: TcpListener,
    ready: Option<oneshot::Sender<()>>,
    tls: Option<Arc<dyn flow_like_runtime::flow::execution::service::ServiceTlsProvider>>,
    handshakes: tokio::task::JoinSet<
        Result<(
            flow_like_runtime::flow::execution::service::BoxedServiceIo,
            std::net::SocketAddr,
        )>,
    >,
}

impl axum::serve::Listener for ReadyListener {
    type Io = flow_like_runtime::flow::execution::service::BoxedServiceIo;
    type Addr = std::net::SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        // Each replica must enter its own accept loop. A request through the
        // inherited socket could otherwise confirm a different replica.
        if let Some(ready) = self.ready.take() {
            let _ = ready.send(());
        }
        loop {
            tokio::select! {
                result = self.handshakes.join_next(), if !self.handshakes.is_empty() => {
                    if let Some(Ok(Ok(connection))) = result { return connection; }
                }
                (stream, addr) = axum::serve::Listener::accept(&mut self.listener), if self.handshakes.len() < 64 => {
                    let Some(tls) = self.tls.clone() else { return (Box::new(stream), addr); };
                    self.handshakes.spawn(async move {
                        let stream = tokio::time::timeout(std::time::Duration::from_secs(10), tls.accept(stream))
                            .await.context("TLS handshake timed out")??;
                        Ok((stream, addr))
                    });
                }
            }
        }
    }

    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        self.listener.local_addr()
    }
}

impl PreparedHost {
    #[cfg(test)]
    pub(crate) async fn bind(
        config: &PlacementConfig,
        events: Vec<PreparedInvocation>,
        state: Arc<FlowLikeState>,
        profile: Profile,
        visibility: AppVisibility,
        cancel: CancellationToken,
    ) -> Result<Self> {
        Self::bind_with_listener(
            config, events, state, profile, visibility, cancel, None, None,
        )
        .await
    }
    pub(crate) async fn bind_with_listener(
        config: &PlacementConfig,
        events: Vec<PreparedInvocation>,
        state: Arc<FlowLikeState>,
        profile: Profile,
        visibility: AppVisibility,
        cancel: CancellationToken,
        inherited_listener: Option<TcpListener>,
        replica: Option<ReplicaContext>,
    ) -> Result<Self> {
        ensure!(
            config.max_replicas == 1 || replica.is_some(),
            "Replicated hosting requires its supervisor routing context"
        );
        #[cfg(not(unix))]
        ensure!(
            replica.is_none(),
            "Replica channel forwarding requires Unix sockets"
        );
        #[cfg(unix)]
        let reply_listener = replica
            .map(|replica| forward::Listener::bind(config, replica))
            .transpose()?;
        #[cfg(unix)]
        let reply_route = reply_listener
            .as_ref()
            .map(|listener| listener.route.clone());
        #[cfg(unix)]
        let incarnation = reply_route.as_ref().map(|route| route.incarnation.clone());
        #[cfg(not(unix))]
        let incarnation = None;
        let hosting = config
            .hosting
            .as_ref()
            .context("HTTP and chat events require hosting configuration")?;
        hosting.validate()?;
        ensure!(
            config.tls_certificate_id.is_none() || state.service_tls_provider.is_some(),
            "Managed TLS identity is unavailable"
        );
        if let Some(tls) = &state.service_tls_provider {
            tls.validate().await?;
        }
        let secret = secret_path(config, hosting)?;
        let mut routes = HashMap::new();
        let mut pages = HashMap::new();
        for prepared in events {
            if prepared.event.default_page_id.is_some() {
                let id = prepared.event.id.clone();
                ensure!(
                    pages
                        .insert(
                            id,
                            pages::PreparedPage::load(
                                &config.id,
                                &config.project_id,
                                prepared,
                                &state
                            )
                            .await?
                        )
                        .is_none(),
                    "Duplicate Page Event"
                );
                continue;
            }
            let route = event_route(&prepared.event)?;
            ensure!(
                route.1 != "/services"
                    && route.1 != "/ui"
                    && !route.1.starts_with("/ui/")
                    && !route.1.starts_with("/channels/"),
                "HTTP event path is reserved by the service host"
            );
            ensure!(
                routes.insert(route, prepared).is_none(),
                "Two events claim the same method and service path"
            );
        }
        ensure!(
            !routes.is_empty() || !pages.is_empty(),
            "Hosting requires an HTTP, chat or Page event"
        );
        for id in pages.keys() {
            ensure!(
                !routes.contains_key(&("GET".into(), format!("/pages/{id}/bootstrap")))
                    && !routes.contains_key(&("POST".into(), format!("/pages/{id}/invoke"))),
                "HTTP route conflicts with a Page endpoint"
            );
        }
        let listener = match inherited_listener {
            Some(listener) => {
                ensure!(
                    listener.local_addr()? == std::net::SocketAddr::new(hosting.host, hosting.port),
                    "Inherited placement listener address differs"
                );
                listener
            }
            None => TcpListener::bind((hosting.host, hosting.port))
                .await
                .context("Bind placement HTTP service")?,
        };
        let mut inventory = routes
            .values()
            .map(|entry| public_event(&entry.event))
            .chain(
                pages
                    .values()
                    .map(|entry| public_event(&entry.invocation.event)),
            )
            .collect::<Vec<_>>();
        inventory.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
        Ok(Self {
            listener,
            #[cfg(unix)]
            reply_listener,
            state: Arc::new(HostState {
                events: routes,
                pages,
                state,
                profile,
                project_id: config.project_id.clone(),
                visibility,
                execution_sub: None,
                secret,
                timeout: Duration::from_secs(u64::from(hosting.request_timeout_secs)),
                capacity: Arc::new(Semaphore::new(usize::from(hosting.max_in_flight))),
                cancel,
                channels: channels::Channels::new(incarnation.clone()),
                actions: Arc::new(actions::Registry::new(incarnation)),
                #[cfg(unix)]
                reply_route,
                inventory: serde_json::json!({"project_id":config.project_id,"events":inventory}),
            }),
        })
    }
    pub(crate) fn set_execution_sub(&mut self, sub: String) -> Result<()> {
        ensure!(!sub.is_empty(), "Online host requires a delegating user");
        Arc::get_mut(&mut self.state)
            .context("Host identity must be configured before serving")?
            .execution_sub = Some(sub);
        Ok(())
    }

    #[cfg(test)]
    pub(crate) async fn serve(self) -> Result<()> {
        self.serve_with_ready(None).await
    }

    pub(crate) async fn serve_with_ready(self, ready: Option<oneshot::Sender<()>>) -> Result<()> {
        let cancel = self.state.cancel.clone();
        #[cfg(unix)]
        let mut replies = tokio::task::JoinSet::new();
        #[cfg(unix)]
        if let Some(listener) = self.reply_listener {
            replies.spawn(listener.serve(self.state.clone()));
        }
        let result = axum::serve(
            ReadyListener {
                listener: self.listener,
                ready,
                tls: self.state.state.service_tls_provider.clone(),
                handshakes: tokio::task::JoinSet::new(),
            },
            Router::new()
                .fallback(dispatch)
                .layer(axum::middleware::from_fn(no_store))
                .with_state(self.state),
        )
        .with_graceful_shutdown(cancel.cancelled_owned())
        .await
        .context("Placement HTTP listener failed");
        #[cfg(unix)]
        replies.abort_all();
        result
    }
}

async fn no_store(request: Request, next: axum::middleware::Next) -> Response {
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        "cache-control",
        axum::http::HeaderValue::from_static("no-store"),
    );
    response.headers_mut().insert(
        "x-content-type-options",
        axum::http::HeaderValue::from_static("nosniff"),
    );
    response
}

async fn dispatch(State(host): State<Arc<HostState>>, request: Request) -> Response {
    #[cfg(feature = "frontend")]
    if let Some(response) = frontend::asset(&request) {
        return response;
    }
    if let Some(id) = request
        .uri()
        .path()
        .strip_prefix("/channels/")
        .map(str::to_owned)
    {
        return channels::push(&host, request, &id).await;
    }
    let service_fingerprint = match authenticated(request.headers(), &host.secret) {
        Ok(Some(fingerprint)) => fingerprint,
        Ok(None) => {
            return (StatusCode::UNAUTHORIZED, "Service authorization required").into_response();
        }
        Err(_) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "Service authorization is unavailable",
            )
                .into_response();
        }
    };
    let path = request.uri().path().to_owned();
    if path == "/services" && request.method() == axum::http::Method::GET {
        return axum::Json(host.inventory.clone()).into_response();
    }
    let page_route = path
        .strip_prefix("/pages/")
        .and_then(|rest| rest.split_once('/'));
    let page = page_route.and_then(|(id, _)| host.pages.get(id));
    if request.method() == axum::http::Method::GET
        && page_route.is_some_and(|(_, suffix)| suffix == "bootstrap")
    {
        if let Some(page) = page {
            return axum::Json(page.bootstrap.clone()).into_response();
        }
    }
    let page = page.filter(|_| {
        request.method() == axum::http::Method::POST
            && page_route.is_some_and(|(_, suffix)| suffix == "invoke")
    });
    let prepared = host
        .events
        .get(&(request.method().as_str().into(), path.clone()))
        .cloned();
    if prepared.is_none() && page.is_none() {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Ok(permit) = host.capacity.clone().try_acquire_owned() else {
        crate::usage::concurrency_rejected();
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [("retry-after", "1")],
            "Service concurrency limit reached",
        )
            .into_response();
    };
    let query = request.uri().query().map(str::to_owned);
    let is_json = request
        .headers()
        .get("content-type")
        .and_then(|h| h.to_str().ok())
        .is_some_and(|s| {
            s.split(';')
                .next()
                .is_some_and(|s| s.trim().eq_ignore_ascii_case("application/json"))
        });
    let body = tokio::select! {_=host.cancel.cancelled()=>return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    result=tokio::time::timeout(host.timeout,to_bytes(request.into_body(),BODY_LIMIT))=>match result {Ok(Ok(body))=>body,Ok(Err(_))=>return StatusCode::PAYLOAD_TOO_LARGE.into_response(),Err(_)=>return StatusCode::REQUEST_TIMEOUT.into_response()}};
    let is_page = page.is_some();
    let request_bytes = body.len() as u64 + query.as_ref().map_or(0, |value| value.len() as u64);
    let (prepared, payload) = if let Some(page) = page {
        if !is_json || query.is_some() {
            return (StatusCode::BAD_REQUEST, "Page actions require a JSON body").into_response();
        }
        match page.select(&host, &body, service_fingerprint).await {
            Ok(selected) => selected,
            Err(_) => {
                return (StatusCode::BAD_REQUEST, "Invalid or stale Page action").into_response();
            }
        }
    } else {
        let prepared = prepared.expect("checked HTTP route");
        let payload = match request_payload(
            query.as_deref(),
            &body,
            is_json,
            prepared.event.event_type == "simple_chat",
        ) {
            Ok(payload) => payload,
            Err(_) => return (StatusCode::BAD_REQUEST, "Invalid event payload").into_response(),
        };
        (prepared, payload)
    };
    if is_page || prepared.event.event_type == "simple_chat" {
        let (tx, rx) = mpsc::channel(32);
        let (finished, completion) = tokio::sync::oneshot::channel();
        let cancel = host.cancel.child_token();
        let run_cancel = cancel.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let callback_cancel = run_cancel.clone();
            let output = tx;
            let callback: InterComCallback = Some(Arc::new(move |event: InterComEvent| {
                let output = output.clone();
                let cancel = callback_cancel.clone();
                Box::pin(async move {
                    if event.event_type == "run_initiated"
                        || event.event_type.starts_with("chat_")
                        || (is_page
                            && matches!(event.event_type.as_str(), "a2ui" | "generic_result"))
                    {
                        let payload_bytes = serde_json::to_vec(&event.payload)?.len();
                        ensure!(
                            payload_bytes <= BODY_LIMIT,
                            "Chat event exceeds the service response limit"
                        );
                        tokio::select! {_=cancel.cancelled()=>{},result=output.send(event)=>{if result.is_err() {cancel.cancel();} else {crate::usage::response_payload(payload_bytes);}}}
                    }
                    Ok(())
                })
            }));
            let result = invoke(
                &host,
                &prepared,
                payload,
                callback,
                run_cancel.clone(),
                service_fingerprint,
                request_bytes,
            )
            .await;
            let _ = finished.send(result.is_ok());
        });
        // Completion has its own channel. Retained runtime callbacks may still
        // own a sender, so closing their event channel cannot end the HTTP stream.
        let stream = futures_util::stream::unfold(
            (rx, completion, cancel.drop_guard(), false),
            |(mut rx, mut completion, guard, ended)| async move {
                if ended {
                    return None;
                }
                let (event, ended) = tokio::select! {biased;
                    event=rx.recv()=>match event {
                        Some(event)=>(event,false),
                        None=>{let ok=(&mut completion).await.unwrap_or(false);(InterComEvent::with_type(if ok {"done"} else {"error"},serde_json::json!({"completed":ok})),true)},
                    },
                    result=&mut completion=>{let ok=result.unwrap_or(false);(InterComEvent::with_type(if ok {"done"} else {"error"},serde_json::json!({"completed":ok})),true)}
                };
                let event = SseEvent::default()
                    .event(&event.event_type)
                    .json_data(&event.payload)
                    .unwrap_or_else(|_| SseEvent::default().event("error").data("{}"));
                Some((Ok::<_, Infallible>(event), (rx, completion, guard, ended)))
            },
        );
        return Sse::new(stream)
            .keep_alive(KeepAlive::default())
            .into_response();
    }
    let _permit = permit;
    let response = Arc::new(Mutex::new(None));
    let output = response.clone();
    let callback: InterComCallback = Some(Arc::new(move |event| {
        let output = output.clone();
        Box::pin(async move {
            if event.event_type == "generic_result" {
                let payload_bytes = serde_json::to_vec(&event.payload)?.len();
                ensure!(
                    payload_bytes <= BODY_LIMIT,
                    "Event exceeds the service response limit"
                );
                *output.lock().await = Some(event.payload);
                crate::usage::response_payload(payload_bytes);
            }
            Ok(())
        })
    }));
    match invoke(
        &host,
        &prepared,
        payload,
        callback,
        host.cancel.child_token(),
        service_fingerprint,
        request_bytes,
    )
    .await
    {
        Ok(()) => axum::Json(
            response
                .lock()
                .await
                .clone()
                .unwrap_or(serde_json::json!({"completed":true})),
        )
        .into_response(),
        Err(_) => (
            StatusCode::BAD_GATEWAY,
            "Workflow failed or exceeded its request deadline",
        )
            .into_response(),
    }
}

fn public_event(event: &Event) -> Value {
    let mut event = event.clone();
    event.variables.clear();
    event.config.clear();
    event.node_id.clear();
    event.inputs.clear();
    event.notes = None;
    event.canary = None;
    event.variants.clear();
    event.correlation_mappings = None;
    serde_json::to_value(event).expect("Event metadata serializes")
}

fn request_payload(
    query: Option<&str>,
    bytes: &[u8],
    is_json: bool,
    chat: bool,
) -> Result<Option<Value>> {
    let mut values = serde_json::Map::new();
    if let Some(query) = query {
        for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
            ensure!(!values.contains_key(key.as_ref()), "Duplicate query field");
            values.insert(key.into_owned(), Value::String(value.into_owned()));
        }
    }
    let body = if bytes.is_empty() {
        None
    } else if is_json {
        Some(serde_json::from_slice::<Value>(bytes)?)
    } else {
        Some(Value::String(std::str::from_utf8(bytes)?.into()))
    };
    let payload = match body {
        Some(Value::Object(body)) => {
            values.extend(body);
            Some(Value::Object(values))
        }
        Some(body) => {
            ensure!(values.is_empty(), "Query fields require an object payload");
            Some(body)
        }
        None if values.is_empty() => None,
        None => Some(Value::Object(values)),
    };
    if chat {
        ensure!(
            payload
                .as_ref()
                .and_then(|p| p.get("messages"))
                .is_some_and(Value::is_array),
            "Chat requires a messages array"
        );
    }
    Ok(payload)
}

async fn invoke(
    host: &HostState,
    prepared: &PreparedInvocation,
    payload: Option<Value>,
    callback: InterComCallback,
    cancel: CancellationToken,
    service_fingerprint: blake3::Hash,
    request_bytes: u64,
) -> Result<()> {
    let invocation = crate::usage::begin(request_bytes);
    let _cancel_on_drop = cancel.clone().drop_guard();
    let result = tokio::select! {_=cancel.cancelled()=>Err(anyhow::anyhow!("Request cancelled")),
    result=tokio::time::timeout(host.timeout,invoke_inner(host,prepared,payload,callback,cancel.clone(),service_fingerprint))=>result.context("Request deadline exceeded").and_then(|result|result)};
    invocation.finish(if cancel.is_cancelled() {
        crate::usage::Outcome::Cancelled
    } else if result.is_ok() {
        crate::usage::Outcome::Succeeded
    } else {
        crate::usage::Outcome::Failed
    });
    result
}

async fn invoke_inner(
    host: &HostState,
    prepared: &PreparedInvocation,
    payload: Option<Value>,
    callback: InterComCallback,
    cancel: CancellationToken,
    service_fingerprint: blake3::Hash,
) -> Result<()> {
    use flow_like_types::channel::{Channel, InProcessChannel};
    let run_id = uuid::Uuid::new_v4().to_string();
    let registration = host.channels.register(
        run_id.clone(),
        &host.secret,
        host.timeout,
        cancel.clone(),
        service_fingerprint,
    )?;
    let channel = InProcessChannel::register(run_id.clone(), host.timeout).await;
    let (producer, sealer) = host
        .pages
        .get(&prepared.event.id)
        .map(|page| {
            let (producer, sealer) = host.actions.begin(
                run_id.clone(),
                page.scope(&host.project_id),
                prepared
                    .template
                    .nodes
                    .iter()
                    .filter(|node| node.can_seed_run)
                    .map(|node| node.id.to_string())
                    .collect(),
                service_fingerprint,
                cancel.clone(),
            );
            (Some(producer), Some(Arc::new(sealer)))
        })
        .unwrap_or_default();
    let output = channels::wrap(callback, registration.grant.clone());
    let callback: InterComCallback = Some(Arc::new(move |mut event: InterComEvent| {
        if let Some(sealer) = &sealer {
            sealer.seal_payload(&event.event_type, &mut event.payload);
        }
        let output = output.clone();
        crate::usage::runtime_message();
        Box::pin(async move {
            if let Some(output) = output {
                output(event).await?;
            }
            Ok(())
        })
    }));
    if let Some(callback) = &callback {
        callback(InterComEvent::with_type(
            "run_initiated",
            serde_json::json!({"run_id":run_id, "channel":channel.handle()}),
        ))
        .await?;
    }
    let mut run = InternalRun::from_template(
        &host.project_id,
        prepared.template.clone(),
        Some(prepared.event.clone()),
        &host.state,
        &host.profile,
        &RunPayload {
            id: prepared.event.node_id.clone(),
            payload,
            runtime_variables: None,
            filter_secrets: Some(false),
        },
        false,
        callback,
        None,
        None,
        Default::default(),
        Some(run_id),
        Some(channel),
    )
    .await?;
    run.set_usage_attribution_from_visibility(&host.visibility)
        .await;
    if let Some(sub) = &host.execution_sub {
        run.set_execution_sub(sub.clone()).await;
        run.set_unresolved_user_context().await;
    } else {
        run.set_offline_user_context();
    }
    run.set_cancellation_token(cancel.clone());
    run.set_cancellation_log("Standalone request stopped", LogLevel::Info);
    run.set_log_flush_policy(Duration::from_secs(5), 500)
        .await?;
    if let Some(admission) = &prepared.action_admission {
        admission
            .validate(host, &prepared.event.node_id, service_fingerprint)
            .await?;
    }
    run.execute(host.state.clone()).await;
    ensure!(
        matches!(run.get_status().await, RunStatus::Success),
        "Workflow request failed"
    );
    if let Some(producer) = producer {
        producer.complete();
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::config::{EventBinding, ProjectSource};
    use flow_like_runtime::{
        bit::Metadata,
        flow::{
            board::Board,
            compiled::TemplateCache,
            event::{EventExecutionMode, EventExposure},
            execution::context::ExecutionContext,
            node::{Node, NodeLogic},
        },
    };
    use std::{
        os::unix::fs::DirBuilderExt,
        sync::atomic::{AtomicUsize, Ordering},
        time::SystemTime,
    };

    struct Responder {
        calls: Arc<AtomicUsize>,
        entered: Arc<tokio::sync::Notify>,
    }
    #[flow_like_types::async_trait]
    impl NodeLogic for Responder {
        fn get_node(&self) -> Node {
            let mut node = Node::new("standalone_test_hosted_responder", "Respond", "", "Tests");
            node.set_start(true);
            node
        }
        async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
            let count = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            let payload = context
                .get_payload()
                .await?
                .payload
                .clone()
                .unwrap_or(Value::Null);
            if let Some(expected) = payload.get("expected_sub").and_then(Value::as_str) {
                let run = context.run.upgrade().expect("live hosted run");
                assert_eq!(run.lock().await.sub, expected);
                let cached = context.execution_cache.as_ref().expect("execution paths");
                assert_eq!(cached.sub, expected);
                assert_eq!(
                    cached.get_user_dir(false).unwrap().as_ref(),
                    flow_like_storage::Path::from(format!("users/{expected}/apps/project"))
                        .to_string()
                );
                let user = context.user_context().expect("delegating user context");
                assert_eq!(user.sub, expected);
                assert!(
                    user.role.is_none(),
                    "a device grant does not grant the delegator's role"
                );
            }
            if payload.get("render") == Some(&Value::Bool(true)) {
                context.stream_response("a2ui", serde_json::json!({"type":"createElement", "component":{"id":"new-button", "component":{"type":"button", "eventHandlers":{"click":[{"name":"workflow_event", "context":{"nodeId":"entry"}}]}}}})).await?;
            }
            if payload.get("wait") == Some(&Value::Bool(true)) {
                self.entered.notify_one();
                std::future::pending::<()>().await;
            }
            context
                .stream_response("chat_stream_partial", payload.clone())
                .await?;
            context
                .stream_response(
                    "generic_result",
                    serde_json::json!({"count":count,"payload":payload}),
                )
                .await?;
            Ok(())
        }
    }

    async fn fixture(
        root: &Path,
    ) -> (
        PlacementConfig,
        Arc<FlowLikeState>,
        Vec<PreparedInvocation>,
        Arc<AtomicUsize>,
        Arc<tokio::sync::Notify>,
    ) {
        let state = crate::runtime::offline_state(root, None).await.unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let entered = Arc::new(tokio::sync::Notify::new());
        let logic = Arc::new(Responder {
            calls: calls.clone(),
            entered: entered.clone(),
        });
        state.node_registry.write().await.push_node(logic.clone());
        let app = flow_like_runtime::app::App::new(
            Some("project".into()),
            Metadata::default(),
            vec![],
            state.clone(),
        )
        .await
        .unwrap();
        app.save().await.unwrap();
        let mut board = Board::new(
            Some("board".into()),
            flow_like_storage::Path::from("apps/project"),
            state.clone(),
        );
        let mut node = logic.get_node();
        node.id = "entry".into();
        board.nodes.insert(node.id.clone(), node);
        let mut page =
            flow_like_runtime::a2ui::widget::Page::new("page", "Page", "/").with_board_id("board");
        page.on_load_event_id = Some("entry".into());
        page.components.push(
            serde_json::from_value(serde_json::json!({
                "id":"button", "component":{"type":"button", "eventHandlers":{"click":[{
                    "name":"workflow_event","context":{"nodeId":"entry"}
                }]}}
            }))
            .unwrap(),
        );
        board.save_page(&page, None).await.unwrap();
        board.snapshot_at_version((1, 0, 0), None).await.unwrap();
        let template = TemplateCache::default()
            .resolve(&state, "project", "board", Some((1, 0, 0)), None, "")
            .await
            .unwrap();
        let now = SystemTime::now();
        let event = Event {
            id: "http".into(),
            name: "HTTP".into(),
            description: String::new(),
            board_id: "board".into(),
            board_version: Some((1, 0, 0)),
            node_id: "entry".into(),
            variables: HashMap::new(),
            config: serde_json::to_vec(&serde_json::json!({"path":"/echo","method":"POST"}))
                .unwrap(),
            active: true,
            canary: None,
            variants: vec![],
            priority: 0,
            event_type: "http".into(),
            notes: None,
            event_version: (1, 0, 0),
            created_at: now,
            updated_at: now,
            default_page_id: None,
            inputs: vec![],
            route: None,
            is_default: false,
            execution_mode: EventExecutionMode::Local,
            exposure: EventExposure::Public,
            correlation_mappings: None,
        };
        let mut chat = event.clone();
        chat.id = "chat".into();
        chat.event_type = "simple_chat".into();
        chat.config = Vec::new();
        let mut page = event.clone();
        page.id = "page_event".into();
        page.event_type = "page".into();
        page.default_page_id = Some("page".into());
        page.node_id.clear();
        page.config.clear();
        let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = socket.local_addr().unwrap().port();
        drop(socket);
        let config = PlacementConfig {
            id: "placement".into(),
            project_id: "project".into(),
            deployment_id: "deployment".into(),
            revision: "one".into(),
            source: ProjectSource::Offline,
            online_metadata_sha256: None,
            project_path: root.into(),
            events: vec![EventBinding {
                event_id: "http".into(),
                event_version: [1, 0, 0],
                board_version: [1, 0, 0],
            }],
            artifact_pins: Vec::new(),
            package_pins: Vec::new(),
            bit_pins: Vec::new(),
            max_replicas: 1,
            tls_certificate_id: None,
            hosting: Some(HostingConfig {
                host: "127.0.0.1".parse().unwrap(),
                port,
                max_in_flight: 1,
                request_timeout_secs: 1,
                auth_secret: "listener".into(),
            }),
            variables: Default::default(),
            secret_overrides: Default::default(),
            resource_grant: None,
            offline_writes: None,
            resources: None,
            restart: Default::default(),
        };
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(root.join(".secrets/placement"))
            .unwrap();
        crate::vault::write_new_private(
            &root.join(".secrets/placement/listener.secret"),
            &[b't'; 32],
        )
        .unwrap();
        (
            config,
            state,
            vec![
                PreparedInvocation {
                    event,
                    template: template.clone(),
                    action_admission: None,
                },
                PreparedInvocation {
                    event: chat,
                    template: template.clone(),
                    action_admission: None,
                },
                PreparedInvocation {
                    event: page,
                    template,
                    action_admission: None,
                },
            ],
            calls,
            entered,
        )
    }

    #[tokio::test]
    async fn persistent_listener_checks_auth_refreshes_secret_and_bounds_requests() {
        let directory = tempfile::tempdir().unwrap();
        let (config, state, events, calls, entered) = fixture(directory.path()).await;
        let cancel = CancellationToken::new();
        let mut host = PreparedHost::bind(
            &config,
            events,
            state,
            Profile::default(),
            AppVisibility::Offline,
            cancel.clone(),
        )
        .await
        .unwrap();
        host.set_execution_sub("auth0|delegating-user".into())
            .unwrap();
        let address = host.listener.local_addr().unwrap();
        let server = tokio::spawn(host.serve());
        let client = reqwest::Client::new();
        let url = format!("http://{address}/echo");
        let token = "t".repeat(32);
        assert_eq!(
            client
                .post(&url)
                .json(&serde_json::json!({}))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        for count in 1..=2 {
            let response = client
                .post(&url)
                .bearer_auth(&token)
                .json(&serde_json::json!({"message":count,"expected_sub":"auth0|delegating-user"}))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()["cache-control"], "no-store");
            let value: Value = response.json().await.unwrap();
            assert_eq!(value["count"], count);
            assert_eq!(value["payload"]["message"], count);
        }
        let waiting = tokio::spawn({
            let client = client.clone();
            let url = url.clone();
            let token = token.clone();
            async move {
                client
                    .post(url)
                    .bearer_auth(token)
                    .json(&serde_json::json!({"wait":true}))
                    .send()
                    .await
                    .unwrap()
            }
        });
        tokio::time::timeout(Duration::from_secs(5), entered.notified())
            .await
            .unwrap();
        assert_eq!(
            client
                .post(&url)
                .bearer_auth(&token)
                .json(&serde_json::json!({}))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::TOO_MANY_REQUESTS
        );
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(5), waiting)
                .await
                .unwrap()
                .unwrap()
                .status(),
            StatusCode::BAD_GATEWAY
        );
        let chat = client
            .post(format!("http://{address}/chat/chat"))
            .bearer_auth(&token)
            .json(&serde_json::json!({"messages":[]}))
            .send()
            .await
            .unwrap();
        assert_eq!(chat.status(), StatusCode::OK);
        assert!(
            chat.headers()["content-type"]
                .to_str()
                .unwrap()
                .starts_with("text/event-stream")
        );
        let body = tokio::time::timeout(Duration::from_secs(5), chat.text())
            .await
            .unwrap()
            .unwrap();
        assert!(body.contains("event: chat_stream_partial"));
        assert!(body.contains("event: done"));
        let rotated = "r".repeat(32);
        std::fs::write(
            directory.path().join(".secrets/placement/listener.secret"),
            &rotated,
        )
        .unwrap();
        assert_eq!(
            client
                .post(&url)
                .bearer_auth(&token)
                .json(&serde_json::json!({}))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            client
                .post(&url)
                .bearer_auth(rotated)
                .json(&serde_json::json!({}))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
        assert_eq!(calls.load(Ordering::SeqCst), 5);
        cancel.cancel();
        tokio::time::timeout(Duration::from_secs(5), server)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }

    #[test]
    fn payload_validation_rejects_ambiguous_or_invalid_requests() {
        assert!(request_payload(Some("a=1&a=2"), b"", false, false).is_err());
        assert!(request_payload(None, b"{bad", true, false).is_err());
        assert!(request_payload(None, b"{}", true, true).is_err());
        assert_eq!(
            request_payload(Some("query=yes"), br#"{"body":1}"#, true, false).unwrap(),
            Some(serde_json::json!({"query":"yes","body":1}))
        );
    }

    #[tokio::test]
    async fn pages_dispatch_only_pinned_actions_and_lifecycle_targets() {
        let directory = tempfile::tempdir().unwrap();
        let (config, state, events, calls, _) = fixture(directory.path()).await;
        let page_invocation = events
            .iter()
            .find(|event| event.event.default_page_id.is_some())
            .unwrap()
            .clone();
        let other_scope =
            pages::PreparedPage::load("other-placement", "project", page_invocation, &state)
                .await
                .unwrap();
        let cancel = CancellationToken::new();
        let host = PreparedHost::bind(
            &config,
            events,
            state,
            Profile::default(),
            AppVisibility::Offline,
            cancel.clone(),
        )
        .await
        .unwrap();
        let address = host.listener.local_addr().unwrap();
        let server = tokio::spawn(host.serve());
        let client = reqwest::Client::new();
        let base = format!("http://{address}/pages/page_event");
        let token = "t".repeat(32);
        let bootstrap: Value = client
            .get(format!("{base}/bootstrap"))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        let revision = bootstrap["execution_revision"].as_str().unwrap();
        let action = &bootstrap["page"]["components"][0]["component"]["eventHandlers"]["click"][0];
        assert!(action["context"].get("nodeId").is_none());
        assert_eq!(action["pageAction"]["manifestRevision"], revision);
        assert_ne!(other_scope.bootstrap["execution_revision"], revision);
        let action_id = action["pageAction"]["actionId"].as_str().unwrap();
        for request in [
            serde_json::json!({"manifest_revision":"stale","trigger":{"kind":"action","action_id":action_id}}),
            serde_json::json!({"manifest_revision":revision,"trigger":{"kind":"action","action_id":"entry"}}),
            serde_json::json!({"manifest_revision":revision,"trigger":{"kind":"action","action_id":action_id,"node_id":"entry"}}),
            serde_json::json!({"manifest_revision":revision,"trigger":{"kind":"special","special_event":"unload"}}),
            serde_json::json!({"manifest_revision":other_scope.bootstrap["execution_revision"],"trigger":{"kind":"action","action_id":action_id}}),
        ] {
            assert_eq!(
                client
                    .post(format!("{base}/invoke"))
                    .bearer_auth(&token)
                    .json(&request)
                    .send()
                    .await
                    .unwrap()
                    .status(),
                StatusCode::BAD_REQUEST
            );
        }
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        for trigger in [
            serde_json::json!({"kind":"action","action_id":action_id}),
            serde_json::json!({"kind":"special","special_event":"load"}),
        ] {
            let response = client.post(format!("{base}/invoke")).bearer_auth(&token).json(&serde_json::json!({"manifest_revision":revision,"trigger":trigger,"payload":{"message":"page"}})).send().await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let body = tokio::time::timeout(Duration::from_secs(5), response.text())
                .await
                .unwrap()
                .unwrap();
            assert!(body.contains("event: generic_result"));
            assert!(body.contains("event: done"));
            assert!(body.contains("page"));
        }
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        cancel.cancel();
        tokio::time::timeout(Duration::from_secs(5), server)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn run_reply_grants_bind_channels_and_end_with_the_run_or_service_secret() {
        use flow_like_types::channel::{Channel, ChannelOutcome, InProcessChannel};
        let directory = tempfile::tempdir().unwrap();
        let (config, state, events, _, _) = fixture(directory.path()).await;
        let host = PreparedHost::bind(
            &config,
            events,
            state,
            Profile::default(),
            AppVisibility::Offline,
            CancellationToken::new(),
        )
        .await
        .unwrap();
        let registration = host
            .state
            .channels
            .register(
                "run".into(),
                &host.state.secret,
                Duration::from_secs(60),
                CancellationToken::new(),
                blake3::hash(&[b't'; 32]),
            )
            .unwrap();
        let channel = InProcessChannel::register("run", Duration::from_secs(60)).await;
        let ticket = channel.open(Duration::from_secs(60)).await.unwrap();
        let mut handle = serde_json::to_value(&ticket.handle).unwrap();
        registration.grant.rewrite(&mut handle);
        assert_eq!(handle["transport"]["type"], "http");
        assert_eq!(handle["transport"]["push_url"], "/channels/run");
        let token = handle["transport"]["token"].as_str().unwrap();
        let request = |token: &str, channel_id: &str| {
            Request::builder().method("POST").uri("/channels/run").header("authorization", format!("Bearer {token}")).body(axum::body::Body::from(serde_json::to_vec(&serde_json::json!({"channel_id":channel_id,"request_id":ticket.request_id,"kind":"reply","value":{"answer":42}})).unwrap())).unwrap()
        };
        assert_eq!(
            channels::push(&host.state, request(&"t".repeat(32), "run"), "run")
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            channels::push(&host.state, request(token, "different-run"), "run")
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            channels::push(&host.state, request(token, "run"), "run")
                .await
                .status(),
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            channel.wait(&ticket, None).await.unwrap(),
            ChannelOutcome::Responded(serde_json::json!({"answer":42}))
        );
        std::fs::write(&host.state.secret, [b'r'; 32]).unwrap();
        assert_eq!(
            channels::push(&host.state, request(token, "run"), "run")
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
        drop(registration);
        assert_eq!(
            channels::push(&host.state, request(token, "run"), "run")
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn replica_reply_forwarding_preserves_owner_authority_and_revision_scope() {
        use flow_like_types::channel::{Channel, ChannelOutcome, InProcessChannel};
        let directory = tempfile::tempdir().unwrap();
        let (mut config, state, events, _, _) = fixture(directory.path()).await;
        config.max_replicas = 2;
        let hosting = config.hosting.as_ref().unwrap();
        let shared = std::net::TcpListener::bind((hosting.host, hosting.port)).unwrap();
        shared.set_nonblocking(true).unwrap();
        let stop = CancellationToken::new();
        let context = ReplicaContext {
            supervisor_pid: std::process::id(),
            config_revision: 1,
            slot: 0,
        };
        let mut owner = PreparedHost::bind_with_listener(
            &config,
            events.clone(),
            state.clone(),
            Profile::default(),
            AppVisibility::Offline,
            stop.clone(),
            Some(TcpListener::from_std(shared.try_clone().unwrap()).unwrap()),
            Some(context),
        )
        .await
        .unwrap();
        let other = PreparedHost::bind_with_listener(
            &config,
            events.clone(),
            state.clone(),
            Profile::default(),
            AppVisibility::Offline,
            stop.clone(),
            Some(TcpListener::from_std(shared.try_clone().unwrap()).unwrap()),
            Some(ReplicaContext { slot: 1, ..context }),
        )
        .await
        .unwrap();
        let previous_revision = PreparedHost::bind_with_listener(
            &config,
            events,
            state,
            Profile::default(),
            AppVisibility::Offline,
            stop.clone(),
            Some(TcpListener::from_std(shared).unwrap()),
            Some(ReplicaContext {
                config_revision: 2,
                ..context
            }),
        )
        .await
        .unwrap();
        let owner_state = owner.state.clone();
        let listener = owner.reply_listener.take().unwrap();
        let socket_path = listener.route.path_for_test();
        let server = tokio::spawn(listener.serve(owner_state.clone()));
        let run = uuid::Uuid::new_v4().to_string();
        let registration = owner_state
            .channels
            .register(
                run.clone(),
                &owner_state.secret,
                Duration::from_secs(60),
                CancellationToken::new(),
                blake3::hash(&[b't'; 32]),
            )
            .unwrap();
        let channel = InProcessChannel::register(run.clone(), Duration::from_secs(60)).await;
        let ticket = channel.open(Duration::from_secs(60)).await.unwrap();
        let mut handle = serde_json::to_value(&ticket.handle).unwrap();
        registration.grant.rewrite(&mut handle);
        let url = handle["transport"]["push_url"].as_str().unwrap();
        assert_eq!(url.split('/').count(), 4);
        let token = handle["transport"]["token"].as_str().unwrap();
        let request = |path: &str, token: &str, channel_id: &str| {
            Request::builder().method("POST").uri(path).header("authorization", format!("Bearer {token}"))
                .body(axum::body::Body::from(serde_json::to_vec(&serde_json::json!({
                    "channel_id":channel_id,"request_id":ticket.request_id,"kind":"reply","value":{"answer":42}
                })).unwrap())).unwrap()
        };
        // The accepting worker has no grant. It must reach the original owner.
        assert!(channels::authorize(&other.state, &run, token).is_err());
        assert_eq!(
            dispatch(
                State(other.state.clone()),
                request(url, &"x".repeat(43), &run)
            )
            .await
            .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            dispatch(
                State(other.state.clone()),
                request(url, token, "another-run")
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            dispatch(
                State(previous_revision.state.clone()),
                request(url, token, &run)
            )
            .await
            .status(),
            StatusCode::GONE
        );
        assert_eq!(
            dispatch(
                State(other.state.clone()),
                request(&format!("/channels/../{run}"), token, &run)
            )
            .await
            .status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            dispatch(State(other.state.clone()), request(url, token, &run))
                .await
                .status(),
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            channel.wait(&ticket, None).await.unwrap(),
            ChannelOutcome::Responded(serde_json::json!({"answer":42}))
        );
        let page = owner_state.pages.get("page_event").unwrap();
        let scope = page.scope("project");
        let source_cancel = CancellationToken::new();
        let (producer, sealer) = owner_state.actions.begin(
            "producer-run".into(),
            scope.clone(),
            ["entry".into()].into_iter().collect(),
            blake3::hash(&[b't'; 32]),
            source_cancel.clone(),
        );
        let dynamic_payload = || serde_json::json!({"type":"createElement", "component":{"id":"dynamic-button", "component":{"type":"button", "eventHandlers":{"click":[{"name":"workflow_event", "context":{"nodeId":"entry", "appId":"project", "boardId":"board"}}]}}}});
        let mut dynamic = dynamic_payload();
        assert_eq!(sealer.seal_payload("a2ui", &mut dynamic).sealed, 1);
        let action = &dynamic["component"]["component"]["eventHandlers"]["click"][0];
        assert_eq!(action["context"], serde_json::json!({}));
        let action_id = action["pageAction"]["actionId"].as_str().unwrap();
        let capability = action["pageAction"]["capabilityJwt"].as_str().unwrap();
        let fingerprint = blake3::hash(&[b't'; 32]);
        assert_eq!(
            actions::resolve(&other.state, &scope, action_id, capability, fingerprint)
                .await
                .unwrap(),
            "entry"
        );
        let invocation = serde_json::to_vec(&serde_json::json!({"manifest_revision":scope.manifest_revision, "trigger":{"kind":"action", "action_id":action_id, "capability_jwt":capability}})).unwrap();
        assert_eq!(
            other.state.pages["page_event"]
                .select(&other.state, &invocation, fingerprint)
                .await
                .unwrap()
                .0
                .event
                .node_id,
            "entry"
        );
        let mut wrong_scope = scope.clone();
        wrong_scope.event_id = "other-event".into();
        assert!(
            actions::resolve(
                &other.state,
                &wrong_scope,
                action_id,
                capability,
                fingerprint
            )
            .await
            .is_err()
        );
        assert!(
            actions::resolve(
                &previous_revision.state,
                &scope,
                action_id,
                capability,
                fingerprint
            )
            .await
            .is_err()
        );
        assert!(
            actions::resolve(&other.state, &scope, "da1_forged", capability, fingerprint)
                .await
                .is_err()
        );
        producer.complete();
        drop(producer);
        source_cancel.cancel();
        assert_eq!(
            actions::resolve(&other.state, &scope, action_id, capability, fingerprint)
                .await
                .unwrap(),
            "entry"
        );
        let failed_cancel = CancellationToken::new();
        let (failed, failed_sealer) = owner_state.actions.begin(
            "failed-run".into(),
            scope.clone(),
            ["entry".into()].into_iter().collect(),
            fingerprint,
            failed_cancel.clone(),
        );
        let mut failed_payload = dynamic_payload();
        failed_sealer.seal_payload("a2ui", &mut failed_payload);
        let failed_action =
            &failed_payload["component"]["component"]["eventHandlers"]["click"][0]["pageAction"];
        let failed_request = serde_json::to_vec(&serde_json::json!({"manifest_revision":scope.manifest_revision,"trigger":{"kind":"action","action_id":failed_action["actionId"],"capability_jwt":failed_action["capabilityJwt"]}})).unwrap();
        let queued = other.state.pages["page_event"]
            .select(&other.state, &failed_request, fingerprint)
            .await
            .unwrap()
            .0;
        failed_cancel.cancel();
        assert!(
            queued
                .action_admission
                .as_ref()
                .unwrap()
                .validate(&other.state, &queued.event.node_id, fingerprint)
                .await
                .is_err()
        );
        assert!(
            actions::resolve(
                &other.state,
                &scope,
                failed_action["actionId"].as_str().unwrap(),
                failed_action["capabilityJwt"].as_str().unwrap(),
                fingerprint
            )
            .await
            .is_err()
        );
        drop(failed);
        let (abandoned, abandoned_sealer) = owner_state.actions.begin(
            "abandoned-run".into(),
            scope.clone(),
            ["entry".into()].into_iter().collect(),
            fingerprint,
            CancellationToken::new(),
        );
        let mut abandoned_payload = dynamic_payload();
        abandoned_sealer.seal_payload("a2ui", &mut abandoned_payload);
        let abandoned_action =
            &abandoned_payload["component"]["component"]["eventHandlers"]["click"][0]["pageAction"];
        drop(abandoned);
        assert!(
            actions::resolve(
                &other.state,
                &scope,
                abandoned_action["actionId"].as_str().unwrap(),
                abandoned_action["capabilityJwt"].as_str().unwrap(),
                fingerprint
            )
            .await
            .is_err()
        );
        std::fs::write(&owner_state.secret, [b'r'; 32]).unwrap();
        assert!(
            actions::resolve(&other.state, &scope, action_id, capability, fingerprint)
                .await
                .is_err()
        );
        assert_eq!(
            dispatch(State(other.state.clone()), request(url, token, &run))
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
        drop(registration);
        assert_eq!(
            dispatch(State(other.state.clone()), request(url, token, &run))
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
        stop.cancel();
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
        assert!(!socket_path.exists());
        assert_eq!(
            dispatch(State(other.state.clone()), request(url, token, &run))
                .await
                .status(),
            StatusCode::GONE
        );
    }

    #[tokio::test]
    async fn rendered_page_actions_require_the_issued_capability_after_the_stream_completes() {
        let directory = tempfile::tempdir().unwrap();
        let (config, state, events, calls, _) = fixture(directory.path()).await;
        let stop = CancellationToken::new();
        let host = PreparedHost::bind(
            &config,
            events,
            state,
            Profile::default(),
            AppVisibility::Offline,
            stop.clone(),
        )
        .await
        .unwrap();
        let secret = host.state.secret.clone();
        let address = host.listener.local_addr().unwrap();
        let server = tokio::spawn(host.serve());
        let client = reqwest::Client::new();
        let base = format!("http://{address}/pages/page_event");
        let bearer = "t".repeat(32);
        let bootstrap: Value = client
            .get(format!("{base}/bootstrap"))
            .bearer_auth(&bearer)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let revision = bootstrap["execution_revision"].as_str().unwrap();
        let compiled_action = bootstrap["page"]["components"][0]["component"]["eventHandlers"]["click"][0]["pageAction"]["actionId"].as_str().unwrap();
        let rendered = client.post(format!("{base}/invoke")).bearer_auth(&bearer).json(&serde_json::json!({"manifest_revision":revision,"trigger":{"kind":"action","action_id":compiled_action},"payload":{"render":true}})).send().await.unwrap().error_for_status().unwrap().text().await.unwrap();
        let frame = rendered
            .split("\n\n")
            .find(|frame| frame.starts_with("event: a2ui"))
            .unwrap();
        let payload: Value = serde_json::from_str(
            frame
                .lines()
                .find_map(|line| line.strip_prefix("data: "))
                .unwrap(),
        )
        .unwrap();
        let action = &payload["component"]["component"]["eventHandlers"]["click"][0];
        assert!(action["context"].get("nodeId").is_none());
        let id = action["pageAction"]["actionId"].as_str().unwrap();
        let capability = action["pageAction"]["capabilityJwt"].as_str().unwrap();
        let request = serde_json::json!({"manifest_revision":revision,"trigger":{"kind":"action","action_id":id,"capability_jwt":capability},"payload":{"message":"clicked"}});
        for invalid in [
            serde_json::json!({"manifest_revision":revision,"trigger":{"kind":"action","action_id":id}}),
            serde_json::json!({"manifest_revision":revision,"trigger":{"kind":"action","action_id":id,"capability_jwt":"forged"}}),
            serde_json::json!({"manifest_revision":revision,"trigger":{"kind":"action","action_id":compiled_action,"capability_jwt":capability}}),
            serde_json::json!({"manifest_revision":revision,"trigger":{"kind":"action","action_id":id,"capability_jwt":capability,"node_id":"entry"}}),
        ] {
            assert_eq!(
                client
                    .post(format!("{base}/invoke"))
                    .bearer_auth(&bearer)
                    .json(&invalid)
                    .send()
                    .await
                    .unwrap()
                    .status(),
                StatusCode::BAD_REQUEST
            );
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let response = client
            .post(format!("{base}/invoke"))
            .bearer_auth(&bearer)
            .json(&request)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.text().await.unwrap().contains("clicked"));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        std::fs::write(secret, [b'r'; 32]).unwrap();
        assert_eq!(
            client
                .post(format!("{base}/invoke"))
                .bearer_auth("r".repeat(32))
                .json(&request)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
        stop.cancel();
        tokio::time::timeout(Duration::from_secs(5), server)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }
}
