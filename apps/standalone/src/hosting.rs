use crate::{
    config::{HostingConfig, PlacementConfig},
    event_kind::EventKind,
    run_once::RunEnd,
};
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
const PENDING_HANDSHAKES: usize = 256;
const PENDING_HANDSHAKES_PER_SOURCE: usize = 16;
const HANDSHAKE_DEADLINE: Duration = Duration::from_secs(10);

mod actions;
mod assets;
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

const METHODS: [&str; 7] = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];
const MAX_PATH_BYTES: usize = 2048;

/// A method and literal path of a service's listener.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct Route {
    pub(crate) method: String,
    pub(crate) path: String,
}

impl Route {
    fn new(method: &str, path: String) -> Self {
        Self {
            method: method.into(),
            path,
        }
    }
}

impl std::fmt::Display for Route {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{} {}", self.method, self.path)
    }
}

/// The route of an `http` or `api` event, read from its config the way the hub reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EventRoute {
    pub(crate) route: Route,
    /// The config had the only form agents before round two read: a `path` with a leading
    /// `/`, and a `method`.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) strict: bool,
}

/// Why a device cannot serve an event's route. The codes are shared with clients.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RouteProblem {
    Missing,
    Invalid(String),
    Reserved(String),
}

impl RouteProblem {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::Missing => "route_missing",
            Self::Invalid(_) => "route_invalid",
            Self::Reserved(_) => "route_reserved",
        }
    }
}

impl std::fmt::Display for RouteProblem {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing => {
                formatter.write_str("The endpoint names no path; set one such as /orders in Events")
            }
            Self::Invalid(sentence) => formatter.write_str(sentence),
            Self::Reserved(path) => write!(
                formatter,
                "The path {:?} is reserved by the service host for its own pages and channels",
                shortened(path)
            ),
        }
    }
}

impl std::error::Error for RouteProblem {}

fn shortened(text: &str) -> &str {
    let mut end = text.len().min(64);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Paths the service host answers itself. Nothing may be added: a route an older agent
/// serves would stop being valid after an agent update.
fn reserved_path(path: &str) -> bool {
    path == "/services"
        || path == "/ui"
        || path.starts_with("/ui/")
        || path.starts_with("/channels/")
}

/// `written` with a leading `/`, when it is a literal path the listener can match.
fn literal_path(written: &str) -> Result<String, RouteProblem> {
    let path = if written.starts_with('/') {
        written.to_owned()
    } else {
        format!("/{written}")
    };
    let literal = path.len() <= MAX_PATH_BYTES
        && path.bytes().all(|byte| byte.is_ascii_graphic())
        && !path.contains(['?', '#', '\\', '{', '}', '*'])
        && !path.split('/').any(|segment| matches!(segment, "." | ".."));
    if literal {
        return Ok(path);
    }
    Err(RouteProblem::Invalid(format!(
        "The path {:?} is not a literal service path: at most {MAX_PATH_BYTES} printable ASCII characters without spaces, none of ? # \\ {{ }} *, and no . or .. segment",
        shortened(written)
    )))
}

/// The saved method in upper case, `None` when none is saved.
fn saved_method(saved: Option<&Value>) -> Result<Option<String>, RouteProblem> {
    let Some(saved) = saved.filter(|saved| !saved.is_null()) else {
        return Ok(None);
    };
    saved
        .as_str()
        .map(str::to_ascii_uppercase)
        .filter(|method| METHODS.contains(&method.as_str()))
        .map(Some)
        .ok_or_else(|| {
            RouteProblem::Invalid(format!(
                "The method {} is not one a device serves: {}",
                shortened(&saved.to_string()),
                METHODS.join(", ")
            ))
        })
}

/// The route rule of `http` and `api` events: `path`, else `path_suffix`, with a leading `/`
/// added; a missing method is `POST`.
fn config_route(config: &[u8]) -> Result<EventRoute, RouteProblem> {
    let config: Value = serde_json::from_slice(config).unwrap_or(Value::Null);
    let text = |key: &str| config.get(key).and_then(Value::as_str);
    let written = text("path")
        .or_else(|| text("path_suffix"))
        .ok_or(RouteProblem::Missing)?;
    let path = literal_path(written)?;
    let method = saved_method(config.get("method"))?;
    if reserved_path(&path) {
        return Err(RouteProblem::Reserved(path));
    }
    Ok(EventRoute {
        strict: method.is_some() && text("path").is_some_and(|path| path.starts_with('/')),
        route: Route {
            method: method.unwrap_or_else(|| "POST".into()),
            path,
        },
    })
}

/// The route an `http` or `api` event without a default Page takes from its config; `None`
/// for every other event.
pub(crate) fn event_route(event: &Event) -> Result<Option<EventRoute>, RouteProblem> {
    if event.default_page_id.is_some() || !crate::event_kind::serves_requests(&event.event_type) {
        return Ok(None);
    }
    config_route(&event.config).map(Some)
}

/// What a route of the listener starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Door {
    Endpoint,
    Chat,
    Page,
    Run,
}

/// A route an event occupies on its service's listener.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RouteClaim {
    pub(crate) event_id: String,
    pub(crate) route: Route,
    pub(crate) door: Door,
}

fn claim(event_id: &str, method: &str, path: String, door: Door) -> RouteClaim {
    RouteClaim {
        event_id: event_id.into(),
        route: Route::new(method, path),
        door,
    }
}

/// The two endpoints of the Page of event `id`.
fn page_claims(id: &str) -> [RouteClaim; 2] {
    [
        claim(id, "GET", format!("/pages/{id}/bootstrap"), Door::Page),
        claim(id, "POST", format!("/pages/{id}/invoke"), Door::Page),
    ]
}

/// The routes `event` occupies: its Endpoint route, its chat route, its Page's two endpoints,
/// or the run route of a person-started event when the service has a web endpoint.
pub(crate) fn route_claims(event: &Event, has_listener: bool) -> Result<Vec<RouteClaim>> {
    let id = &event.id;
    if event.default_page_id.is_some() {
        return Ok(page_claims(id).into());
    }
    if let Some(EventRoute { route, .. }) =
        event_route(event).with_context(|| format!("route of event {id}"))?
    {
        return Ok(vec![RouteClaim {
            event_id: id.clone(),
            route,
            door: Door::Endpoint,
        }]);
    }
    // Served without a Page and without a route of its own: a chat.
    Ok(match EventKind::of(&event.event_type, false) {
        Some(EventKind::Served) => vec![claim(id, "POST", format!("/chat/{id}"), Door::Chat)],
        Some(EventKind::OnDemand) if has_listener => {
            vec![claim(id, "POST", format!("/run/{id}"), Door::Run)]
        }
        _ => Vec::new(),
    })
}

/// The door and route of each event a listener serves, once their claims passed the check.
/// Run routes come with `PreparedHost::with_on_demand`.
fn served_doors(events: Vec<PreparedInvocation>) -> Result<Vec<(PreparedInvocation, Door, Route)>> {
    let mut claims = Vec::new();
    let mut served = Vec::with_capacity(events.len());
    for prepared in events {
        let owned = route_claims(&prepared.event, true)?;
        let (door, route) = owned
            .first()
            .filter(|claim| claim.door != Door::Run)
            .map(|claim| (claim.door, claim.route.clone()))
            .with_context(|| {
                format!(
                    "Event {} of type {} is not served by the service listener",
                    prepared.event.id, prepared.event.event_type
                )
            })?;
        claims.extend(owned);
        served.push((prepared, door, route));
    }
    check_route_claims(&claims)?;
    Ok(served)
}

/// Refuses a reserved path, and a method and path that two claims share.
pub(crate) fn check_route_claims(claims: &[RouteClaim]) -> Result<()> {
    let mut seen = HashMap::<&Route, &RouteClaim>::with_capacity(claims.len());
    for claim in claims {
        ensure!(
            !reserved_path(&claim.route.path),
            "HTTP event path is reserved by the service host: {} of event {}",
            claim.route,
            claim.event_id
        );
        let Some(first) = seen.insert(&claim.route, claim) else {
            continue;
        };
        ensure!(
            first.door != Door::Page && claim.door != Door::Page,
            "HTTP route conflicts with a Page endpoint: {} of events {} and {}",
            claim.route,
            first.event_id,
            claim.event_id
        );
        anyhow::bail!(
            "Two events claim the same method and service path: {} of events {} and {}",
            claim.route,
            first.event_id,
            claim.event_id
        );
    }
    Ok(())
}

fn secret_path(config: &PlacementConfig, hosting: &HostingConfig) -> Result<Option<PathBuf>> {
    let Some(secret) = hosting.auth_secret.as_deref() else {
        ensure!(
            hosting.authentication == crate::config::ServiceAuthentication::None,
            "Token authentication requires a service authentication secret"
        );
        return Ok(None);
    };
    let path = crate::config::private_secret_path(config, secret)?;
    read_token(&path)?;
    Ok(Some(path))
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
fn service_fingerprint(secret: Option<&Path>) -> Result<blake3::Hash> {
    match secret {
        Some(path) => Ok(blake3::hash(&read_token(path)?)),
        // Public services still scope Page actions and reply channels to their own
        // capability registries. Their service fingerprint must agree across replicas.
        None => Ok(blake3::hash(b"flow-like-service-authentication-none-v1")),
    }
}

fn authenticated(headers: &HeaderMap, path: Option<&Path>) -> Result<Option<blake3::Hash>> {
    let Some(path) = path else {
        return service_fingerprint(None).map(Some);
    };
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

#[derive(Clone)]
struct Hosted {
    invocation: PreparedInvocation,
    door: Door,
}

/// The service page's way into the service's quick actions and forms.
#[cfg(feature = "on-demand")]
struct RunDoor {
    events: HashMap<String, crate::on_demand::OnDemandEvent>,
    counters: crate::on_demand::Counters,
}

/// The run route of a person-started event.
#[cfg(feature = "on-demand")]
fn run_route(event: &crate::on_demand::OnDemandEvent) -> Result<RouteClaim> {
    match route_claims(&event.invocation().event, true)?.as_slice() {
        [claim] if claim.door == Door::Run => Ok(claim.clone()),
        _ => anyhow::bail!("Event {} is not started by a person", event.id()),
    }
}

struct HostState {
    events: HashMap<Route, Hosted>,
    #[cfg(feature = "on-demand")]
    run_door: Option<RunDoor>,
    pages: HashMap<String, pages::PreparedPage>,
    state: Arc<FlowLikeState>,
    profile: Profile,
    project_id: String,
    visibility: AppVisibility,
    execution_sub: Option<String>,
    secret: Option<PathBuf>,
    timeout: Duration,
    capacity: Arc<Semaphore>,
    cancel: CancellationToken,
    channels: channels::Channels,
    actions: Arc<actions::Registry>,
    assets: Arc<assets::Registry>,
    #[cfg(unix)]
    reply_route: Option<Arc<forward::Route>>,
    inventory: Value,
    #[cfg(feature = "frontend")]
    ui_policy: axum::http::HeaderValue,
}

#[cfg(feature = "on-demand")]
impl HostState {
    /// The routes this listener serves, as the claims of their events.
    fn claims(&self) -> Vec<RouteClaim> {
        self.events
            .iter()
            .map(|(route, hosted)| RouteClaim {
                event_id: hosted.invocation.event.id.clone(),
                route: route.clone(),
                door: hosted.door,
            })
            .chain(self.pages.keys().flat_map(|id| page_claims(id)))
            .collect()
    }
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
    handshakes: crate::acme::PendingConnections<
        Result<(
            flow_like_runtime::flow::execution::service::BoxedServiceIo,
            std::net::SocketAddr,
        )>,
    >,
}

impl ReadyListener {
    fn new(
        listener: TcpListener,
        ready: Option<oneshot::Sender<()>>,
        tls: Option<Arc<dyn flow_like_runtime::flow::execution::service::ServiceTlsProvider>>,
    ) -> Self {
        Self {
            listener,
            ready,
            tls,
            handshakes: crate::acme::PendingConnections::new(
                PENDING_HANDSHAKES,
                PENDING_HANDSHAKES_PER_SOURCE,
            ),
        }
    }
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
        // Pending handshakes are unauthenticated; they never pause accepting.
        loop {
            tokio::select! {
                Some(result) = self.handshakes.join_next(), if !self.handshakes.is_empty() => {
                    if let Ok(connection) = result { return connection; }
                }
                (stream, addr) = axum::serve::Listener::accept(&mut self.listener) => {
                    let Some(tls) = self.tls.clone() else { return (Box::new(stream), addr); };
                    self.handshakes.spawn(addr.ip(), async move {
                        let stream = tokio::time::timeout(HANDSHAKE_DEADLINE, tls.accept(stream))
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
        for (invocation, door, route) in served_doors(events)? {
            if door == Door::Page {
                let id = invocation.event.id.clone();
                ensure!(
                    pages
                        .insert(
                            id,
                            pages::PreparedPage::load(
                                &config.id,
                                &config.project_id,
                                invocation,
                                &state
                            )
                            .await?
                        )
                        .is_none(),
                    "Duplicate Page Event"
                );
            } else {
                routes.insert(route, Hosted { invocation, door });
            }
        }
        // On-demand routes are attached by with_on_demand before serving.
        ensure!(
            !routes.is_empty() || !pages.is_empty() || cfg!(feature = "on-demand"),
            "Hosting requires an HTTP, chat or Page event"
        );
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
            .iter()
            .map(|(route, entry)| {
                let mut event = public_event(&entry.invocation.event);
                if entry.door == Door::Endpoint {
                    event["route"] =
                        serde_json::json!({"method": route.method, "path": route.path});
                }
                event
            })
            .chain(
                pages
                    .values()
                    .map(|entry| public_event(&entry.invocation.event)),
            )
            .collect::<Vec<_>>();
        inventory.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
        let assets = Arc::new(assets::Registry::new(
            &config.project_id,
            state.config.read().await.stores.clone(),
            incarnation.clone(),
        ));
        Ok(Self {
            listener,
            #[cfg(unix)]
            reply_listener,
            state: Arc::new(HostState {
                events: routes,
                #[cfg(feature = "on-demand")]
                run_door: None,
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
                assets,
                #[cfg(unix)]
                reply_route,
                inventory: serde_json::json!({"project_id":config.project_id,"events":inventory}),
                #[cfg(feature = "frontend")]
                ui_policy: frontend::content_security_policy(&hosting.ui_origins)?,
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

    /// Offers the service's quick actions and forms at `POST /run/{event id}`, behind the
    /// service token, and lists them in the inventory. Runs through this door are counted in
    /// `counters` as runs of the service page. Call before serving.
    #[cfg(feature = "on-demand")]
    pub(crate) fn with_on_demand(
        &mut self,
        events: &[crate::on_demand::OnDemandEvent],
        counters: crate::on_demand::Counters,
    ) -> Result<()> {
        let host = Arc::get_mut(&mut self.state)
            .context("Person-started events must be offered before serving")?;
        ensure!(
            host.run_door.is_none(),
            "Person-started events are offered once"
        );
        let runs = events.iter().map(run_route).collect::<Result<Vec<_>>>()?;
        let mut claims = host.claims();
        claims.extend(runs.iter().cloned());
        check_route_claims(&claims)?;
        let inventory = host
            .inventory
            .get_mut("events")
            .and_then(Value::as_array_mut)
            .context("Service inventory has no event list")?;
        for (event, run) in events.iter().zip(runs) {
            inventory.push(crate::on_demand::inventory_entry(event));
            let invocation = event.invocation().clone();
            host.events.insert(
                run.route,
                Hosted {
                    invocation,
                    door: Door::Run,
                },
            );
        }
        inventory.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
        host.run_door = Some(RunDoor {
            events: events
                .iter()
                .map(|event| (event.id().to_owned(), event.clone()))
                .collect(),
            counters,
        });
        Ok(())
    }

    #[cfg(test)]
    pub(crate) async fn serve(self) -> Result<()> {
        self.serve_with_ready(None).await
    }

    pub(crate) async fn serve_with_ready(self, ready: Option<oneshot::Sender<()>>) -> Result<()> {
        ensure!(
            !self.state.events.is_empty() || !self.state.pages.is_empty(),
            "Hosting requires at least one deployed event"
        );
        let cancel = self.state.cancel.clone();
        #[cfg(unix)]
        let mut replies = tokio::task::JoinSet::new();
        #[cfg(unix)]
        if let Some(listener) = self.reply_listener {
            replies.spawn(listener.serve(self.state.clone()));
        }
        let result = axum::serve(
            ReadyListener::new(
                self.listener,
                ready,
                self.state.state.service_tls_provider.clone(),
            ),
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
    if !assets::route(request.uri().path())
        && let Some(response) = frontend::asset(&request, &host.ui_policy)
    {
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
    let service_fingerprint = match authenticated(request.headers(), host.secret.as_deref()) {
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
    if assets::route(&path) {
        return assets::read(&host, request, service_fingerprint).await;
    }
    if path == "/services" && request.method() == axum::http::Method::GET {
        let mut inventory = host.inventory.clone();
        host.assets.rewrite(&mut inventory, service_fingerprint);
        return axum::Json(inventory).into_response();
    }
    let page_route = path
        .strip_prefix("/pages/")
        .and_then(|rest| rest.split_once('/'));
    let page = page_route.and_then(|(id, _)| host.pages.get(id));
    if request.method() == axum::http::Method::GET
        && page_route.is_some_and(|(_, suffix)| suffix == "bootstrap")
    {
        if let Some(page) = page {
            let mut bootstrap = page.bootstrap.clone();
            host.assets.rewrite(&mut bootstrap, service_fingerprint);
            return axum::Json(bootstrap).into_response();
        }
    }
    let page = page.filter(|_| {
        request.method() == axum::http::Method::POST
            && page_route.is_some_and(|(_, suffix)| suffix == "invoke")
    });
    let hosted = host
        .events
        .get(&Route::new(request.method().as_str(), path.clone()))
        .cloned();
    if hosted.is_none() && page.is_none() {
        return StatusCode::NOT_FOUND.into_response();
    }
    let body_kind = body_kind(request.headers());
    if hosted.is_some() && body_kind == BodyKind::Multipart {
        return (
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "Multipart bodies are not supported by a device service",
        )
            .into_response();
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
    let body = tokio::select! {_=host.cancel.cancelled()=>return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    result=tokio::time::timeout(host.timeout,to_bytes(request.into_body(),BODY_LIMIT))=>match result {Ok(Ok(body))=>body,Ok(Err(_))=>return StatusCode::PAYLOAD_TOO_LARGE.into_response(),Err(_)=>return StatusCode::REQUEST_TIMEOUT.into_response()}};
    let request_bytes = body.len() as u64 + query.as_ref().map_or(0, |value| value.len() as u64);
    let (prepared, payload, door) = if let Some(page) = page {
        if body_kind != BodyKind::Json || query.is_some() {
            return (StatusCode::BAD_REQUEST, "Page actions require a JSON body").into_response();
        }
        match page.select(&host, &body, service_fingerprint).await {
            Ok((prepared, payload)) => (prepared, payload, Door::Page),
            Err(_) => {
                return (StatusCode::BAD_REQUEST, "Invalid or stale Page action").into_response();
            }
        }
    } else {
        let Hosted { invocation, door } = hosted.expect("checked HTTP route");
        let payload = match door {
            #[cfg(feature = "on-demand")]
            Door::Run => {
                match run_payload(
                    &host,
                    &invocation.event.id,
                    query.is_some(),
                    &body,
                    body_kind,
                ) {
                    Ok(payload) => Some(payload),
                    Err(refusal) => return refusal.into_response(),
                }
            }
            _ => match request_payload(query.as_deref(), &body, body_kind, door == Door::Chat) {
                Ok(payload) => payload,
                Err(_) => {
                    return (StatusCode::BAD_REQUEST, "Invalid event payload").into_response();
                }
            },
        };
        (invocation, payload, door)
    };
    if door != Door::Endpoint {
        let run = Streamed {
            prepared,
            payload,
            door,
            service_fingerprint,
            request_bytes,
        };
        return streamed_run(host, run, permit);
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
    let end = invoke(
        &host,
        &prepared,
        payload,
        callback,
        host.cancel.child_token(),
        service_fingerprint,
        request_bytes,
    )
    .await;
    if end != RunEnd::Succeeded {
        return (
            StatusCode::BAD_GATEWAY,
            "Workflow failed or exceeded its request deadline",
        )
            .into_response();
    }
    axum::Json(
        response
            .lock()
            .await
            .clone()
            .unwrap_or(serde_json::json!({"completed":true})),
    )
    .into_response()
}

/// A run whose events the answer streams: a chat, a Page action, or a run of the service page.
struct Streamed {
    prepared: PreparedInvocation,
    payload: Option<Value>,
    door: Door,
    service_fingerprint: blake3::Hash,
    request_bytes: u64,
}

/// Starts `run` and answers with its events, ending with `done` or `error`.
fn streamed_run(
    host: Arc<HostState>,
    run: Streamed,
    permit: tokio::sync::OwnedSemaphorePermit,
) -> Response {
    let Streamed {
        prepared,
        payload,
        door,
        service_fingerprint,
        request_bytes,
    } = run;
    let (tx, rx) = mpsc::channel(32);
    let (finished, completion) = tokio::sync::oneshot::channel();
    let cancel = host.cancel.child_token();
    let run_cancel = cancel.clone();
    #[cfg(feature = "on-demand")]
    let counted = host
        .run_door
        .as_ref()
        .filter(|_| door == Door::Run)
        .map(|run| {
            run.counters
                .begin(&prepared.event.id, crate::on_demand::Origin::ServicePage)
        });
    tokio::spawn(async move {
        let _permit = permit;
        let callback = stream_callback(door, tx, run_cancel.clone());
        let end = invoke(
            &host,
            &prepared,
            payload,
            callback,
            run_cancel.clone(),
            service_fingerprint,
            request_bytes,
        )
        .await;
        #[cfg(feature = "on-demand")]
        if let Some(counted) = counted {
            counted.end(end);
        }
        let _ = finished.send(end == RunEnd::Succeeded);
    });
    event_answer(rx, completion, cancel.drop_guard())
}

/// Sends the run events the answer of `door` carries to `output`; a reader that went away
/// cancels the run.
fn stream_callback(
    door: Door,
    output: mpsc::Sender<InterComEvent>,
    cancel: CancellationToken,
) -> InterComCallback {
    Some(Arc::new(move |event: InterComEvent| {
        let output = output.clone();
        let cancel = cancel.clone();
        Box::pin(async move {
            if streams(door, &event.event_type) {
                let payload_bytes = serde_json::to_vec(&event.payload)?.len();
                ensure!(
                    payload_bytes <= BODY_LIMIT,
                    "Chat event exceeds the service response limit"
                );
                tokio::select! {_=cancel.cancelled()=>{},result=output.send(event)=>{if result.is_err() {cancel.cancel();} else {crate::usage::response_payload(payload_bytes);}}}
            }
            Ok(())
        })
    }))
}

/// The answer to a streamed run: its events, then `done` or `error` once `completion` says
/// how it ended. Dropping the answer cancels the run through `guard`.
fn event_answer(
    rx: mpsc::Receiver<InterComEvent>,
    completion: oneshot::Receiver<bool>,
    guard: tokio_util::sync::DropGuard,
) -> Response {
    // Completion has its own channel. Retained runtime callbacks may still
    // own a sender, so closing their event channel cannot end the HTTP stream.
    let stream = futures_util::stream::unfold(
        (rx, completion, guard, false),
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
    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

/// Run events a streamed answer carries: a chat its messages; a Page also its renders and its
/// result; the run route of a quick action or form also its text output.
fn streams(door: Door, event_type: &str) -> bool {
    event_type == "run_initiated"
        || event_type.starts_with("chat_")
        || (door != Door::Chat && matches!(event_type, "a2ui" | "generic_result"))
        || (door == Door::Run && matches!(event_type, "text_output" | "stream_text"))
}

/// What a run through the service page sends: a JSON object, or no body for no fields.
#[cfg(feature = "on-demand")]
fn run_object(query: bool, body: &[u8], kind: BodyKind) -> Option<Value> {
    if query {
        return None;
    }
    if body.is_empty() {
        return Some(Value::Object(Default::default()));
    }
    (kind == BodyKind::Json)
        .then(|| serde_json::from_slice::<Value>(body).ok())
        .flatten()
        .filter(Value::is_object)
}

/// Why the run route does not start a run.
#[cfg(feature = "on-demand")]
enum RunRefusal {
    NoEvent,
    Body,
    Fields(Vec<String>),
}

#[cfg(feature = "on-demand")]
impl IntoResponse for RunRefusal {
    fn into_response(self) -> Response {
        match self {
            Self::NoEvent => StatusCode::NOT_FOUND.into_response(),
            Self::Body => (StatusCode::BAD_REQUEST, "Runs take a JSON object body").into_response(),
            Self::Fields(fields) => {
                let refusal = serde_json::json!({"code": "invalid_fields", "fields": fields});
                (StatusCode::BAD_REQUEST, axum::Json(refusal)).into_response()
            }
        }
    }
}

/// The fields of a run through the service page: a JSON object that passes the form's field
/// check, or why it is refused, with the names of the refused fields.
#[cfg(feature = "on-demand")]
fn run_payload(
    host: &HostState,
    event_id: &str,
    query: bool,
    body: &[u8],
    kind: BodyKind,
) -> std::result::Result<Value, RunRefusal> {
    let event = host
        .run_door
        .as_ref()
        .and_then(|door| door.events.get(event_id))
        .ok_or(RunRefusal::NoEvent)?;
    let payload = run_object(query, body, kind).ok_or(RunRefusal::Body)?;
    crate::on_demand::check_fields(event, &payload).map_err(RunRefusal::Fields)?;
    Ok(payload)
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

/// How a request body is read, by its media type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BodyKind {
    Json,
    Form,
    Multipart,
    Text,
}

fn body_kind(headers: &HeaderMap) -> BodyKind {
    let media = headers
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(|value| value.trim().to_ascii_lowercase())
        .unwrap_or_default();
    match media.as_str() {
        "application/json" => BodyKind::Json,
        "application/x-www-form-urlencoded" => BodyKind::Form,
        media if media.starts_with("multipart/") => BodyKind::Multipart,
        _ => BodyKind::Text,
    }
}

/// Query or form-encoded fields as the hub reads them: a repeated key becomes a list, `key[]`
/// always makes one, and a key that is blank is `value`.
fn form_fields(input: &str) -> serde_json::Map<String, Value> {
    let mut fields = serde_json::Map::new();
    for (key, value) in url::form_urlencoded::parse(input.as_bytes()) {
        let key = key.trim();
        let (key, list) = match key.strip_suffix("[]") {
            Some(stripped) => (stripped.trim(), true),
            None => (key, false),
        };
        let key = if key.is_empty() { "value" } else { key };
        let value = Value::String(value.into_owned());
        match fields.get_mut(key) {
            Some(Value::Array(values)) => values.push(value),
            Some(first) => *first = Value::Array(vec![first.take(), value]),
            None => {
                fields.insert(
                    key.to_owned(),
                    if list {
                        Value::Array(vec![value])
                    } else {
                        value
                    },
                );
            }
        }
    }
    fields
}

/// A request body as a run reads it: JSON, form fields, or one text value.
fn body_value(bytes: &[u8], kind: BodyKind) -> Result<Option<Value>> {
    if bytes.is_empty() {
        return Ok(None);
    }
    Ok(Some(match kind {
        BodyKind::Json => serde_json::from_slice::<Value>(bytes)?,
        BodyKind::Form => Value::Object(form_fields(std::str::from_utf8(bytes)?)),
        BodyKind::Text => Value::String(std::str::from_utf8(bytes)?.into()),
        BodyKind::Multipart => anyhow::bail!("Multipart bodies are not supported"),
    }))
}

fn request_payload(
    query: Option<&str>,
    bytes: &[u8],
    kind: BodyKind,
    chat: bool,
) -> Result<Option<Value>> {
    let mut values = query.map(form_fields).unwrap_or_default();
    let payload = match body_value(bytes, kind)? {
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
) -> RunEnd {
    let invocation = crate::usage::begin(request_bytes);
    let _cancel_on_drop = cancel.clone().drop_guard();
    let ended = tokio::select! {_=cancel.cancelled()=>None,
    result=tokio::time::timeout(host.timeout,invoke_inner(host,prepared,payload,callback,cancel.clone(),service_fingerprint))=>Some(result)};
    let end = match ended {
        _ if cancel.is_cancelled() => RunEnd::Cancelled,
        None => RunEnd::Cancelled,
        Some(Err(_)) => RunEnd::TimedOut,
        Some(Ok(Ok(()))) => RunEnd::Succeeded,
        Some(Ok(Err(_))) => RunEnd::Failed,
    };
    invocation.finish(match end {
        RunEnd::Succeeded => crate::usage::Outcome::Succeeded,
        RunEnd::Cancelled => crate::usage::Outcome::Cancelled,
        RunEnd::Failed | RunEnd::TimedOut => crate::usage::Outcome::Failed,
    });
    end
}

/// The process-wide in-process channel registry only forgets a run when it is closed, and a
/// request can end on any await point, including its deadline.
pub(crate) struct CloseChannelOnDrop(pub(crate) Arc<flow_like_types::channel::InProcessChannel>);
impl Drop for CloseChannelOnDrop {
    fn drop(&mut self) {
        use flow_like_types::channel::Channel;
        let channel = self.0.clone();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move { channel.close().await });
        }
    }
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
        host.secret.as_deref(),
        host.timeout,
        cancel.clone(),
        service_fingerprint,
    )?;
    let channel = InProcessChannel::register(run_id.clone(), host.timeout).await;
    let _close_channel = CloseChannelOnDrop(channel.clone());
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
                host.timeout,
                cancel.clone(),
            );
            (Some(producer), Some(Arc::new(sealer)))
        })
        .unwrap_or_default();
    let output = channels::wrap(callback, registration.grant.clone());
    let assets = host.assets.clone();
    let callback: InterComCallback = Some(Arc::new(move |mut event: InterComEvent| {
        assets.rewrite(&mut event.payload, service_fingerprint);
        let unavailable = sealer.as_ref().and_then(|sealer| {
            sealer
                .seal_payload(&event.event_type, &mut event.payload)
                .unavailable
        });
        let output = output.clone();
        crate::usage::runtime_message();
        Box::pin(async move {
            if let Some(output) = output {
                output(event).await?;
            }
            // The render still ships, but the run log names the actions left without authority.
            if let Some(reason) = unavailable {
                anyhow::bail!("Page actions were sent without a working capability: {reason}");
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

    /// Run channels a Responder keeps past its run, so only an explicit close can
    /// unregister them.
    static RETAINED_CHANNELS: std::sync::Mutex<Vec<Arc<dyn flow_like_types::channel::Channel>>> =
        std::sync::Mutex::new(Vec::new());

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
            if payload.get("retain_channel") == Some(&Value::Bool(true)) {
                RETAINED_CHANNELS.lock().unwrap().push(
                    context
                        .channel
                        .clone()
                        .expect("hosted runs carry a channel"),
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

    /// An event of the fixture's flow; a `null` config is an empty one.
    fn hosted_event(id: &str, event_type: &str, config: Value) -> Event {
        let now = SystemTime::now();
        Event {
            id: id.into(),
            name: id.to_ascii_uppercase(),
            description: String::new(),
            board_id: "board".into(),
            board_version: Some((1, 0, 0)),
            node_id: "entry".into(),
            variables: HashMap::new(),
            config: if config.is_null() {
                Vec::new()
            } else {
                serde_json::to_vec(&config).unwrap()
            },
            active: true,
            canary: None,
            variants: vec![],
            priority: 0,
            event_type: event_type.into(),
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
        }
    }

    fn routed(config: Value) -> Result<String, RouteProblem> {
        config_route(&serde_json::to_vec(&config).unwrap()).map(|route| route.route.to_string())
    }

    fn claims_of(events: &[&Event], has_listener: bool) -> Vec<RouteClaim> {
        events
            .iter()
            .flat_map(|event| route_claims(event, has_listener).unwrap())
            .collect()
    }

    /// The literal table of design §1.2, which the client's route rule carries too.
    #[test]
    fn the_route_rule_reads_a_route_as_the_hub_does() {
        use serde_json::json;
        assert_eq!(
            routed(json!({"path":"/orders","method":"get"})).unwrap(),
            "GET /orders"
        );
        assert_eq!(routed(json!({"path":"orders"})).unwrap(), "POST /orders");
        assert_eq!(
            routed(json!({"path_suffix":"/a/b","method":"PUT"})).unwrap(),
            "PUT /a/b"
        );
        assert_eq!(
            routed(json!({"sink_type":"http","method":"GET","path":"/cm1abc","public_endpoint":true,"auth_token":"x"}))
                .unwrap(),
            "GET /cm1abc"
        );
        for path in ["/servicesx", "/runner", "/run/evt_a"] {
            assert_eq!(
                routed(json!({"path":path})).unwrap(),
                format!("POST {path}")
            );
        }
        let longest = format!("/{}", "a".repeat(MAX_PATH_BYTES - 1));
        assert!(routed(json!({"path":longest})).is_ok());
        let strict = |config: Value| {
            config_route(&serde_json::to_vec(&config).unwrap())
                .unwrap()
                .strict
        };
        assert!(strict(json!({"path":"/orders","method":"get"})));
        for config in [
            json!({"path":"orders"}),
            json!({"path":"/orders"}),
            json!({"path_suffix":"/a/b","method":"PUT"}),
        ] {
            assert!(!strict(config.clone()), "{config}");
        }
    }

    #[test]
    fn the_route_rule_refuses_what_a_device_cannot_serve() {
        use serde_json::json;
        let refused = [
            (json!({}), "route_missing"),
            (json!({"path":7}), "route_missing"),
            (json!([]), "route_missing"),
            (json!({"path":"/a b"}), "route_invalid"),
            (json!({"path":"/a?x=1"}), "route_invalid"),
            (json!({"path":"/a/../b"}), "route_invalid"),
            (json!({"path":"/ä"}), "route_invalid"),
            (json!({"path":"/x","method":"TRACE"}), "route_invalid"),
            (
                json!({"path": format!("/{}", "a".repeat(MAX_PATH_BYTES))}),
                "route_invalid",
            ),
            (json!({"path":"/services"}), "route_reserved"),
            (json!({"path":"/ui/x"}), "route_reserved"),
            (json!({"path":"/channels/1"}), "route_reserved"),
        ];
        for (config, code) in refused {
            let problem = routed(config.clone()).unwrap_err();
            assert_eq!(problem.code(), code, "{config}");
            let sentence = problem.to_string();
            assert!((20..=480).contains(&sentence.len()), "{sentence}");
        }
        assert_eq!(config_route(b"").unwrap_err().code(), "route_missing");
    }

    fn endpoint(config: Value) -> Event {
        hosted_event("orders", "api", config)
    }

    #[test]
    fn a_run_route_conflicts_only_with_its_own_method_and_path_in_its_own_service() {
        use serde_json::json;
        let form = hosted_event("evt_form", "generic_form", Value::Null);
        let at_run = endpoint(json!({"path":"/run/evt_form","method":"POST"}));
        assert_eq!(
            route_claims(&form, true).unwrap(),
            vec![RouteClaim {
                event_id: "evt_form".into(),
                route: Route::new("POST", "/run/evt_form".into()),
                door: Door::Run,
            }]
        );
        assert!(route_claims(&form, false).unwrap().is_empty());
        let conflict = check_route_claims(&claims_of(&[&at_run, &form], true)).unwrap_err();
        assert!(
            conflict
                .to_string()
                .starts_with("Two events claim the same method and service path"),
            "{conflict}"
        );
        assert!(conflict.to_string().contains("POST /run/evt_form"));
        for (events, has_listener) in [(vec![&at_run], true), (vec![&at_run, &form], false)] {
            check_route_claims(&claims_of(&events, has_listener)).unwrap();
        }
        let read = endpoint(json!({"path":"/run/evt_form","method":"GET"}));
        check_route_claims(&claims_of(&[&read, &form], true)).unwrap();
    }

    #[test]
    fn chats_pages_and_endpoints_of_one_service_never_share_a_route() {
        use serde_json::json;
        let chat = hosted_event("chat", "simple_chat", Value::Null);
        let at_chat = endpoint(json!({"path":"/chat/chat","method":"POST"}));
        assert!(
            check_route_claims(&claims_of(&[&chat, &at_chat], true))
                .unwrap_err()
                .to_string()
                .starts_with("Two events claim the same method and service path")
        );
        let mut page = hosted_event("page_event", "api", json!({"path":"/orders"}));
        page.default_page_id = Some("page".into());
        assert_eq!(
            claims_of(&[&page], true)
                .iter()
                .map(|claim| (claim.route.to_string(), claim.door))
                .collect::<Vec<_>>(),
            [
                ("GET /pages/page_event/bootstrap".to_owned(), Door::Page),
                ("POST /pages/page_event/invoke".to_owned(), Door::Page),
            ]
        );
        let at_page = endpoint(json!({"path":"/pages/page_event/bootstrap","method":"GET"}));
        assert!(
            check_route_claims(&claims_of(&[&page, &at_page], true))
                .unwrap_err()
                .to_string()
                .starts_with("HTTP route conflicts with a Page endpoint")
        );
        let twin = hosted_event("twin", "http", json!({"path":"/orders","method":"POST"}));
        assert!(
            check_route_claims(&claims_of(
                &[&endpoint(json!({"path":"orders"})), &twin],
                true
            ))
            .is_err()
        );
        let reserved = route_claims(&endpoint(json!({"path":"/services"})), true).unwrap_err();
        assert!(format!("{reserved:#}").contains("reserved"), "{reserved:#}");
        for kind in ["daemon", "cron", "rest"] {
            assert!(
                route_claims(&hosted_event("other", kind, Value::Null), true)
                    .unwrap()
                    .is_empty()
            );
        }
    }

    /// The service access token the fixture stores.
    const SERVICE_TOKEN: &str = "tttttttttttttttttttttttttttttttt";
    /// The token of an Endpoint's hub copy; a device never honours it.
    const HUB_TOKEN: &str = "event-token-of-the-hub-copy-0000";

    /// A listener of the fixture's project that serves until `stop`.
    struct Served {
        address: std::net::SocketAddr,
        calls: Arc<AtomicUsize>,
        cancel: CancellationToken,
        server: tokio::task::JoinHandle<Result<()>>,
        client: reqwest::Client,
    }

    impl Served {
        fn start(host: PreparedHost, calls: Arc<AtomicUsize>, cancel: CancellationToken) -> Self {
            Self {
                address: host.listener.local_addr().unwrap(),
                calls,
                cancel,
                server: tokio::spawn(host.serve()),
                client: reqwest::Client::new(),
            }
        }

        fn url(&self, path: &str) -> String {
            format!("http://{}{path}", self.address)
        }

        /// What the service's inventory lists, by event id.
        async fn inventory(&self) -> Vec<(String, Value)> {
            let inventory: Value = self
                .client
                .get(self.url("/services"))
                .bearer_auth(SERVICE_TOKEN)
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            assert!(!inventory.to_string().contains(HUB_TOKEN));
            inventory["events"]
                .as_array()
                .unwrap()
                .iter()
                .map(|event| (event["id"].as_str().unwrap().to_owned(), event.clone()))
                .collect()
        }

        async fn stop(self) {
            self.cancel.cancel();
            tokio::time::timeout(Duration::from_secs(5), self.server)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
        }
    }

    /// The fixture's service with an Endpoint of the editor's default config at
    /// `GET /cm1abc`, and an `http` event at `POST /run/x` whose hub copy has a token.
    async fn endpoints(root: &Path) -> Served {
        let (config, state, mut events, calls, _) = fixture(root).await;
        let template = events[0].template.clone();
        events[0].event = hosted_event(
            "endpoint",
            "api",
            serde_json::json!({"sink_type":"http","method":"GET","path":"/cm1abc","public_endpoint":false}),
        );
        events.push(PreparedInvocation {
            event: hosted_event(
                "run-path",
                "http",
                serde_json::json!({"path":"/run/x","method":"POST","auth_token":HUB_TOKEN}),
            ),
            template,
            action_admission: None,
        });
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
        Served::start(host, calls, cancel)
    }

    #[tokio::test]
    async fn an_endpoint_with_the_editor_default_config_is_served_only_with_the_service_token() {
        let directory = tempfile::tempdir().unwrap();
        let served = endpoints(directory.path()).await;
        let (client, endpoint, run_path) =
            (&served.client, served.url("/cm1abc"), served.url("/run/x"));
        for bearer in [None, Some(HUB_TOKEN)] {
            for request in [client.get(&endpoint), client.post(&run_path)] {
                let request = match bearer {
                    Some(token) => request.bearer_auth(token),
                    None => request,
                };
                let status = request.send().await.unwrap().status();
                assert_eq!(status, StatusCode::UNAUTHORIZED);
            }
        }
        let answer = client.get(&endpoint).bearer_auth(SERVICE_TOKEN).send();
        let answer: Value = answer.await.unwrap().json().await.unwrap();
        assert_eq!(answer["count"], 1);
        let run = client.post(&run_path).bearer_auth(SERVICE_TOKEN);
        let run = run.json(&serde_json::json!({"from":"run"})).send();
        assert_eq!(run.await.unwrap().status(), StatusCode::OK);
        let wrong_method = client.post(&endpoint).bearer_auth(SERVICE_TOKEN).send();
        assert_eq!(wrong_method.await.unwrap().status(), StatusCode::NOT_FOUND);
        assert_eq!(served.calls.load(Ordering::SeqCst), 2);
        served.stop().await;
    }

    #[tokio::test]
    async fn an_endpoint_reads_form_bodies_and_refuses_multipart_bodies() {
        let directory = tempfile::tempdir().unwrap();
        let served = endpoints(directory.path()).await;
        let post = |bearer: &str, content_type: &str, body: &str| {
            served
                .client
                .post(served.url("/run/x"))
                .bearer_auth(bearer)
                .header("content-type", content_type)
                .body(body.to_owned())
                .send()
        };
        let form = post(
            SERVICE_TOKEN,
            "application/x-www-form-urlencoded",
            "name=Ada&tags=a&tags=b",
        );
        let form: Value = form.await.unwrap().json().await.unwrap();
        assert_eq!(
            form["payload"],
            serde_json::json!({"name":"Ada","tags":["a","b"]})
        );
        let multipart = "multipart/form-data; boundary=x";
        let unauthorized = post("wrong-token", multipart, "--x--").await.unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
        let refused = post(SERVICE_TOKEN, multipart, "--x--").await.unwrap();
        assert_eq!(refused.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
        assert_eq!(
            refused.text().await.unwrap(),
            "Multipart bodies are not supported by a device service"
        );
        assert_eq!(served.calls.load(Ordering::SeqCst), 1);
        served.stop().await;
    }

    #[tokio::test]
    async fn the_inventory_names_the_route_of_each_endpoint() {
        let directory = tempfile::tempdir().unwrap();
        let served = endpoints(directory.path()).await;
        let routes = served
            .inventory()
            .await
            .into_iter()
            .map(|(id, event)| (id, event["route"].clone()))
            .collect::<Vec<_>>();
        let route = |method: &str, path: &str| serde_json::json!({"method":method,"path":path});
        assert_eq!(
            routes,
            [
                ("chat".to_owned(), Value::Null),
                ("endpoint".to_owned(), route("GET", "/cm1abc")),
                ("page_event".to_owned(), Value::Null),
                ("run-path".to_owned(), route("POST", "/run/x")),
            ]
        );
        served.stop().await;
    }

    #[tokio::test]
    async fn a_listener_refuses_a_doubled_route_before_it_binds() {
        let directory = tempfile::tempdir().unwrap();
        let (config, state, mut events, _, _) = fixture(directory.path()).await;
        let mut twin = events[0].clone();
        twin.event.id = "twin".into();
        events.push(twin);
        let error = PreparedHost::bind(
            &config,
            events,
            state,
            Profile::default(),
            AppVisibility::Offline,
            CancellationToken::new(),
        )
        .await
        .err()
        .expect("a doubled route is refused");
        assert!(
            error
                .to_string()
                .starts_with("Two events claim the same method and service path"),
            "{error}"
        );
        let port = config.hosting.as_ref().unwrap().port;
        assert!(std::net::TcpListener::bind(("127.0.0.1", port)).is_ok());
    }

    /// A quick action of the fixture's flow, and counters that write to `root`.
    #[cfg(feature = "on-demand")]
    fn quick_action(
        root: &Path,
        state: &Arc<FlowLikeState>,
        template: Arc<CompiledRunTemplate>,
    ) -> (
        crate::on_demand::OnDemandEvent,
        crate::on_demand::OnDemandContext,
    ) {
        let action = crate::on_demand::prepare(PreparedInvocation {
            event: hosted_event("evt_action", "quick_action", Value::Null),
            template,
            action_admission: None,
        })
        .unwrap();
        let run = Arc::new(crate::run_once::RunContext {
            project_id: "project".into(),
            state: state.clone(),
            profile: Profile::default(),
            visibility: AppVisibility::Offline,
            execution_sub: None,
        });
        let context = crate::on_demand::OnDemandContext::new(
            run,
            None,
            crate::on_demand::state_directory(root, "placement"),
            "placement".into(),
            0,
            1,
            1,
            Duration::from_secs(5),
        );
        crate::on_demand::prepare_state(std::slice::from_ref(&action), &context).unwrap();
        (action, context)
    }

    /// The fixture's service with the quick action `evt_action` on its service page.
    #[cfg(feature = "on-demand")]
    async fn service_page(root: &Path) -> Served {
        let (config, state, events, calls, _) = fixture(root).await;
        let (action, context) = quick_action(root, &state, events[0].template.clone());
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
        host.with_on_demand(std::slice::from_ref(&action), context.counters())
            .unwrap();
        Served::start(host, calls, cancel)
    }

    #[cfg(feature = "on-demand")]
    #[tokio::test]
    async fn an_explicit_listener_can_serve_only_a_quick_action() {
        let directory = tempfile::tempdir().unwrap();
        let (config, state, events, calls, _) = fixture(directory.path()).await;
        let (action, context) = quick_action(directory.path(), &state, events[0].template.clone());
        let cancel = CancellationToken::new();
        let mut host = PreparedHost::bind(
            &config,
            vec![],
            state,
            Profile::default(),
            AppVisibility::Offline,
            cancel.clone(),
        )
        .await
        .unwrap();
        host.with_on_demand(std::slice::from_ref(&action), context.counters())
            .unwrap();
        let served = Served::start(host, calls, cancel);
        let inventory = served.inventory().await;
        assert_eq!(inventory.len(), 1);
        assert_eq!(inventory[0].0, "evt_action");
        assert_eq!(inventory[0].1["event_type"], "quick_action");
        let response = served
            .client
            .post(served.url("/run/evt_action"))
            .bearer_auth(SERVICE_TOKEN)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let response = response.text().await.unwrap();
        assert!(response.contains("event: done"), "{response}");
        assert_eq!(served.calls.load(Ordering::SeqCst), 1);
        served.stop().await;
    }

    #[tokio::test]
    async fn assets_require_service_auth_and_stream_only_project_uploads() {
        let directory = tempfile::tempdir().unwrap();
        let uploads = directory.path().join("apps/project/upload/media");
        std::fs::create_dir_all(&uploads).unwrap();
        std::fs::write(uploads.join("image.png"), b"0123456789").unwrap();
        let served = endpoints(directory.path()).await;
        let path = "/ui/assets?store=upload&path=media%2Fimage.png";
        let missing = served.client.get(served.url(path)).send().await.unwrap();
        assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);
        let response = served
            .client
            .get(served.url(path))
            .bearer_auth(SERVICE_TOKEN)
            .header("range", "bytes=2-5")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(response.headers()["content-type"], "image/png");
        assert_eq!(response.headers()["content-range"], "bytes 2-5/10");
        assert_eq!(response.bytes().await.unwrap(), &b"2345"[..]);
        let response = served
            .client
            .head(served.url(path))
            .bearer_auth(SERVICE_TOKEN)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["content-length"], "10");
        assert!(response.bytes().await.unwrap().is_empty());
        let denied = served
            .client
            .get(served.url("/ui/assets?store=storage&path=private.db"))
            .bearer_auth(SERVICE_TOKEN)
            .send()
            .await
            .unwrap();
        assert_eq!(denied.status(), StatusCode::NOT_FOUND);
        served.stop().await;
    }

    #[cfg(feature = "on-demand")]
    #[tokio::test]
    async fn the_run_route_refuses_callers_without_the_token_and_fields_the_form_lacks() {
        let directory = tempfile::tempdir().unwrap();
        let served = service_page(directory.path()).await;
        let run = |bearer: &str, path: &str, body: &str| {
            served
                .client
                .post(served.url(path))
                .bearer_auth(bearer)
                .header("content-type", "application/json")
                .body(body.to_owned())
                .send()
        };
        let unauthorized = run("wrong-token", "/run/evt_action", "{}").await.unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
        let refused = run(
            SERVICE_TOKEN,
            "/run/evt_action",
            r#"{"title":"no such field"}"#,
        );
        let refused = refused.await.unwrap();
        assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            refused.json::<Value>().await.unwrap(),
            serde_json::json!({"code":"invalid_fields","fields":["title"]})
        );
        for (path, body) in [
            ("/run/evt_action", "[1]"),
            ("/run/evt_action", "not json"),
            ("/run/evt_action?title=x", "{}"),
        ] {
            let status = run(SERVICE_TOKEN, path, body).await.unwrap().status();
            assert_eq!(status, StatusCode::BAD_REQUEST, "{path} {body}");
        }
        assert_eq!(served.calls.load(Ordering::SeqCst), 0);
        served.stop().await;
    }

    #[cfg(feature = "on-demand")]
    #[tokio::test]
    async fn the_service_page_runs_a_quick_action_streams_it_and_counts_it() {
        let directory = tempfile::tempdir().unwrap();
        let served = service_page(directory.path()).await;
        let streamed = served
            .client
            .post(served.url("/run/evt_action"))
            .bearer_auth(SERVICE_TOKEN)
            .send()
            .await
            .unwrap();
        assert_eq!(streamed.status(), StatusCode::OK);
        let body = tokio::time::timeout(Duration::from_secs(5), streamed.text());
        let body = body.await.unwrap().unwrap();
        let frames: Vec<_> = body
            .split("\n\n")
            .filter_map(|frame| frame.lines().find_map(|line| line.strip_prefix("event: ")))
            .collect();
        assert_eq!(frames.first(), Some(&"run_initiated"));
        assert!(frames.contains(&"generic_result"), "{frames:?}");
        assert_eq!(frames.last(), Some(&"done"));
        assert_eq!(served.calls.load(Ordering::SeqCst), 1);
        let state = crate::on_demand::state_directory(directory.path(), "placement");
        let counted: Value =
            serde_json::from_slice(&std::fs::read(state.join("state.0.json")).unwrap()).unwrap();
        let entry = &counted["events"]["evt_action"];
        assert_eq!((&entry["runs"], &entry["running"]), (&1.into(), &0.into()));
        assert_eq!(entry["last"]["origin"], "service_page");
        assert_eq!(entry["last"]["outcome"], "succeeded");
        let listed = served.inventory().await;
        let ids = listed.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>();
        assert_eq!(ids, ["chat", "evt_action", "http", "page_event"]);
        served.stop().await;
    }

    #[cfg(feature = "on-demand")]
    #[tokio::test]
    async fn an_endpoint_at_the_run_route_of_a_quick_action_keeps_it_off_the_service_page() {
        let directory = tempfile::tempdir().unwrap();
        let (config, state, mut events, _, _) = fixture(directory.path()).await;
        let (action, context) = quick_action(directory.path(), &state, events[0].template.clone());
        events[0].event = hosted_event(
            "orders",
            "api",
            serde_json::json!({"path":"/run/evt_action","method":"POST"}),
        );
        let mut host = PreparedHost::bind(
            &config,
            events,
            state,
            Profile::default(),
            AppVisibility::Offline,
            CancellationToken::new(),
        )
        .await
        .unwrap();
        let error = host
            .with_on_demand(std::slice::from_ref(&action), context.counters())
            .unwrap_err();
        assert!(
            error
                .to_string()
                .starts_with("Two events claim the same method and service path"),
            "{error}"
        );
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
        let event = hosted_event(
            "http",
            "http",
            serde_json::json!({"path":"/echo","method":"POST"}),
        );
        let chat = hosted_event("chat", "simple_chat", Value::Null);
        let mut page = hosted_event("page_event", "page", Value::Null);
        page.default_page_id = Some("page".into());
        page.node_id.clear();
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
            tunnel_services: vec![],
            tls_certificate_id: None,
            hosting: Some(HostingConfig {
                host: "127.0.0.1".parse().unwrap(),
                port,
                max_in_flight: 1,
                request_timeout_secs: 1,
                authentication: Default::default(),
                auth_secret: Some("listener".into()),
                ui_origins: Vec::new(),
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
    async fn public_services_serve_without_a_secret_and_keep_reply_capabilities() {
        let directory = tempfile::tempdir().unwrap();
        let (mut config, state, events, calls, _) = fixture(directory.path()).await;
        let hosting = config.hosting.as_mut().unwrap();
        hosting.authentication = crate::config::ServiceAuthentication::None;
        hosting.auth_secret = None;
        std::fs::remove_dir_all(directory.path().join(".secrets")).unwrap();
        validate_access_token(&config).unwrap();
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
        assert!(host.state.secret.is_none());
        let fingerprint = service_fingerprint(None).unwrap();
        let registration = host
            .state
            .channels
            .register(
                "public-run".into(),
                None,
                Duration::from_secs(60),
                CancellationToken::new(),
                fingerprint,
            )
            .unwrap();
        let mut handle =
            serde_json::json!({"channel_id":"public-run","transport":{"type":"in_process"}});
        registration.grant.rewrite(&mut handle);
        let reply_token = handle["transport"]["token"].as_str().unwrap();
        assert!(channels::authorize(&host.state, "public-run", "").is_err());
        assert!(channels::authorize(&host.state, "public-run", reply_token).is_ok());
        let address = host.listener.local_addr().unwrap();
        drop(registration);
        let server = tokio::spawn(host.serve());
        let client = reqwest::Client::new();
        let base = format!("http://{address}");
        let inventory: Value = client
            .get(format!("{base}/services"))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(inventory["project_id"], "project");
        let response = client
            .post(format!("{base}/echo"))
            .json(&serde_json::json!({"message":"public"}))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        assert_eq!(
            response.json::<Value>().await.unwrap()["payload"]["message"],
            "public"
        );
        let bootstrap: Value = client
            .get(format!("{base}/pages/page_event/bootstrap"))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        let rendered = client.post(format!("{base}/pages/page_event/invoke"))
            .json(&serde_json::json!({"manifest_revision":bootstrap["execution_revision"],"trigger":{"kind":"special","special_event":"load"}}))
            .send().await.unwrap().error_for_status().unwrap().text().await.unwrap();
        assert!(rendered.contains("event: done"));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert!(!directory.path().join(".secrets").exists());
        stop.cancel();
        tokio::time::timeout(Duration::from_secs(5), server)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
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
            .json(&serde_json::json!({"messages":[], "retain_channel":true}))
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
        let run_id = body
            .split("\n\n")
            .find(|frame| frame.contains("event: run_initiated"))
            .and_then(|frame| frame.lines().find_map(|line| line.strip_prefix("data: ")))
            .and_then(|data| serde_json::from_str::<Value>(data).ok())
            .and_then(|data| data["run_id"].as_str().map(str::to_owned))
            .expect("chat stream announces its run");
        let retained = RETAINED_CHANNELS
            .lock()
            .unwrap()
            .iter()
            .find(|channel| channel.channel_id() == run_id)
            .cloned()
            .expect("the chat run kept a strong reference to its channel");
        let mut unregistered = false;
        for _ in 0..50 {
            if flow_like_types::channel::InProcessChannel::lookup(&run_id)
                .await
                .is_none()
            {
                unregistered = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(
            unregistered,
            "a finished request closes its in-process channel while references remain"
        );
        drop(retained);
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

    struct StalledTls;
    impl flow_like_runtime::flow::execution::service::ServiceTlsProvider for StalledTls {
        fn validate(
            &self,
        ) -> flow_like_runtime::flow::execution::service::ServiceTlsFuture<'_, ()> {
            Box::pin(async { Ok(()) })
        }
        fn accept(
            &self,
            mut stream: tokio::net::TcpStream,
        ) -> flow_like_runtime::flow::execution::service::ServiceTlsFuture<
            '_,
            flow_like_runtime::flow::execution::service::BoxedServiceIo,
        > {
            Box::pin(async move {
                let mut hello = [0_u8; 1];
                tokio::io::AsyncReadExt::read_exact(&mut stream, &mut hello).await?;
                Ok(Box::new(stream) as flow_like_runtime::flow::execution::service::BoxedServiceIo)
            })
        }
    }

    #[tokio::test]
    async fn idle_handshakes_never_pause_the_tls_accept_loop() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let mut ready = ReadyListener::new(listener, None, Some(Arc::new(StalledTls)));
        let mut idle = Vec::new();
        for _ in 0..PENDING_HANDSHAKES_PER_SOURCE * 5 {
            idle.push(tokio::net::TcpStream::connect(address).await.unwrap());
        }
        let mut client = tokio::net::TcpStream::connect(address).await.unwrap();
        tokio::io::AsyncWriteExt::write_all(&mut client, b"h")
            .await
            .unwrap();
        let (_, peer) = tokio::time::timeout(
            Duration::from_secs(2),
            axum::serve::Listener::accept(&mut ready),
        )
        .await
        .expect("idle handshakes kept a completed client in the backlog");
        assert_eq!(peer, client.local_addr().unwrap());
        drop(idle);
    }

    #[tokio::test]
    async fn ending_a_request_closes_its_channel_even_while_references_remain() {
        use flow_like_types::channel::InProcessChannel;
        let channel =
            InProcessChannel::register("hosted-close-on-drop", Duration::from_secs(60)).await;
        drop(CloseChannelOnDrop(channel.clone()));
        for _ in 0..50 {
            if InProcessChannel::lookup("hosted-close-on-drop")
                .await
                .is_none()
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("the in-process channel registry kept a finished request");
    }

    #[test]
    fn payload_validation_rejects_ambiguous_or_invalid_requests() {
        use BodyKind::{Json, Multipart, Text};
        assert!(request_payload(None, b"{bad", Json, false).is_err());
        assert!(request_payload(None, b"{}", Json, true).is_err());
        assert!(request_payload(Some("a=1"), b"text", Text, false).is_err());
        assert!(request_payload(None, b"--x", Multipart, false).is_err());
        assert!(request_payload(None, &[0xff, 0xfe], Text, false).is_err());
        assert_eq!(
            request_payload(Some("query=yes"), br#"{"body":1}"#, Json, false).unwrap(),
            Some(serde_json::json!({"query":"yes","body":1}))
        );
        assert_eq!(
            request_payload(None, b"plain", Text, false).unwrap(),
            Some(serde_json::json!("plain"))
        );
        assert_eq!(request_payload(None, b"", Text, false).unwrap(), None);
    }

    /// The hub's reading of query fields and form-encoded bodies.
    #[test]
    fn query_fields_and_form_bodies_are_read_as_the_hub_reads_them() {
        use serde_json::json;
        let query = |input: &str| {
            request_payload(Some(input), b"", BodyKind::Text, false)
                .unwrap()
                .unwrap()
        };
        assert_eq!(query("a=1&a=2"), json!({"a":["1","2"]}));
        assert_eq!(query("a[]=1"), json!({"a":["1"]}));
        assert_eq!(query("a=1&a[]=2&a=3"), json!({"a":["1","2","3"]}));
        assert_eq!(query("a[]=1&a=2"), json!({"a":["1","2"]}));
        assert_eq!(query("=x&[]=y"), json!({"value":["x","y"]}));
        assert_eq!(
            query("q=a+b%21&+key+=1&flag"),
            json!({"q":"a b!","key":"1","flag":""})
        );
        assert_eq!(query("caf%C3%A9=%E2%9C%93&&"), json!({"café":"✓"}));
        assert_eq!(
            request_payload(None, b"name=Ada&tags=a&tags=b", BodyKind::Form, false).unwrap(),
            Some(json!({"name":"Ada","tags":["a","b"]}))
        );
        assert_eq!(
            request_payload(
                Some("name=query&page=2"),
                b"name=body",
                BodyKind::Form,
                false
            )
            .unwrap(),
            Some(json!({"name":"body","page":"2"}))
        );
        let kind = |value: &str| {
            let mut headers = HeaderMap::new();
            headers.insert("content-type", value.parse().unwrap());
            body_kind(&headers)
        };
        assert_eq!(kind("application/json; charset=utf-8"), BodyKind::Json);
        assert_eq!(kind("Application/X-WWW-Form-Urlencoded"), BodyKind::Form);
        assert_eq!(kind("multipart/form-data; boundary=x"), BodyKind::Multipart);
        assert_eq!(kind("multipart/mixed"), BodyKind::Multipart);
        assert_eq!(kind("text/plain"), BodyKind::Text);
        assert_eq!(body_kind(&HeaderMap::new()), BodyKind::Text);
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
                host.state.secret.as_deref(),
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
        std::fs::write(host.state.secret.as_ref().unwrap(), [b'r'; 32]).unwrap();
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
        let upload = directory.path().join("apps/project/upload/logo.png");
        std::fs::create_dir_all(upload.parent().unwrap()).unwrap();
        std::fs::write(&upload, b"replica asset").unwrap();
        let mut media =
            serde_json::json!({"src":url::Url::from_file_path(&upload).unwrap().to_string()});
        owner_state
            .assets
            .rewrite(&mut media, blake3::hash(&[b't'; 32]));
        let asset_url = media["src"].as_str().unwrap();
        assert_eq!(asset_url.split('/').count(), 5);
        let asset_request = || {
            Request::builder()
                .uri(asset_url)
                .header("authorization", format!("Bearer {SERVICE_TOKEN}"))
                .body(axum::body::Body::empty())
                .unwrap()
        };
        let workflow_permit = other.state.capacity.clone().try_acquire_owned().unwrap();
        let response = dispatch(State(other.state.clone()), asset_request()).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            to_bytes(response.into_body(), 32).await.unwrap(),
            &b"replica asset"[..]
        );
        drop(workflow_permit);
        assert_eq!(
            dispatch(State(previous_revision.state.clone()), asset_request())
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
        let run = uuid::Uuid::new_v4().to_string();
        let registration = owner_state
            .channels
            .register(
                run.clone(),
                owner_state.secret.as_deref(),
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
            owner_state.timeout,
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
            owner_state.timeout,
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
            owner_state.timeout,
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
        std::fs::write(owner_state.secret.as_ref().unwrap(), [b'r'; 32]).unwrap();
        let rotated_asset = Request::builder()
            .uri(asset_url)
            .header("authorization", format!("Bearer {}", "r".repeat(32)))
            .body(axum::body::Body::empty())
            .unwrap();
        assert_eq!(
            dispatch(State(other.state.clone()), rotated_asset)
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
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
        let secret = host.state.secret.clone().unwrap();
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
