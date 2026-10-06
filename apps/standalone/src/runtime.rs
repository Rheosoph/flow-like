use std::{collections::HashSet, future::Future, path::Path, sync::Arc, time::Duration};

use anyhow::{Context, Result, bail, ensure};
use flow_like_runtime::{
    app::{App, AppVisibility},
    flow::{
        compiled::TemplateCache,
        execution::{
            ExecutionEnvironment, InternalRun, LogLevel, RunPayload, RunStatus,
            service::{ServiceOutcome, ServiceReadyKind, ServiceTlsProvider},
        },
        pin::{ValueType, resolve_schema},
        variable::{Variable, VariableType},
    },
    profile::Profile,
    state::{FlowLikeConfig, FlowLikeState, FlowNodeRegistryInner},
    utils::http::HTTPClient,
};
use flow_like_storage::{
    Path as StorePath,
    databases::vector::lancedb::connect_lance,
    files::store::{FlowLikeStore, local_store::LocalObjectStore},
};
use flow_like_types::authorization::RequestAuthorizer;
use flow_like_types::intercom::BufferedInterComHandler;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::config::{PlacementConfig, ProjectSource};
use crate::event_kind::EventKind;

/// Run the placement in a dedicated workload process until its events finish or stop.
pub async fn run(config: &PlacementConfig, cancellation: CancellationToken) -> Result<()> {
    run_with_ready(config, cancellation, || Ok(())).await
}

/// Check pinned project metadata and overrides before persisting a placement.
pub async fn validate(config: &PlacementConfig) -> Result<()> {
    if config.source == ProjectSource::Online {
        // Network preflight belongs to the supervised workload's launch-bound
        // broker channel. Applying intent does not imply healthy service state.
        return config.validate();
    }
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let state = inspection_state(&config.project_path, None).await?;
    run_with_state_listener(
        config,
        state,
        cancellation,
        Profile::default(),
        None,
        None,
        || async { Ok(()) },
        false,
        None,
        None,
        PersonStartedRuns::default(),
    )
    .await
}

/// Check a staged offline revision and its service readiness contract.
pub async fn validate_rollout(config: &PlacementConfig) -> Result<()> {
    ensure!(
        config.source == ProjectSource::Offline,
        "rollout_online_preflight_unsupported"
    );
    validate_rollout_authorized(config, None, None).await
}

/// Validate pinned metadata and private overrides without starting any service.
pub async fn validate_rollout_authorized(
    config: &PlacementConfig,
    authorizer: Option<Arc<dyn RequestAuthorizer>>,
    workload_identity: Option<crate::config::WorkloadIdentity>,
) -> Result<()> {
    config.validate()?;
    ensure!(!config.events.is_empty(), "placement has no events");
    ensure!(
        config.project_path.is_absolute(),
        "project_path must be absolute"
    );
    match config.source {
        ProjectSource::Offline => {
            let state = inspection_state(&config.project_path, authorizer).await?;
            validate_rollout_with_state(config, state).await
        }
        ProjectSource::Online => {
            let authorizer =
                authorizer.context("Online rollout requires its validation credential broker")?;
            let identity = workload_identity
                .context("Online rollout requires its validation workload identity")?;
            let cache = crate::online::PreflightCache::new(config, &identity)?;
            // Metadata, derived Bit packs, and any local runtime directories stay
            // separate from the live workers and disappear when validation ends.
            let mut runtime = local_config(cache.path())?;
            crate::online::configure_preflight(
                config,
                authorizer.clone(),
                &mut runtime,
                cache.path(),
            )
            .await?;
            let state = initialize_state(runtime, Some(authorizer), None, None).await?;
            validate_rollout_with_state(config, state).await
        }
    }
}

async fn validate_rollout_with_state(
    config: &PlacementConfig,
    state: Arc<FlowLikeState>,
) -> Result<()> {
    let stop = CancellationToken::new();
    stop.cancel();
    run_with_state_listener(
        config,
        state,
        stop,
        Profile::default(),
        None,
        None,
        || async { bail!("Rollout validation cannot acknowledge a running listener") },
        true,
        None,
        None,
        PersonStartedRuns::default(),
    )
    .await
}

/// Notify the supervisor after every selected listener or daemon acknowledges startup.
pub async fn run_with_ready(
    config: &PlacementConfig,
    cancellation: CancellationToken,
    on_ready: impl FnOnce() -> Result<()>,
) -> Result<()> {
    run_authorized_with_ready(config, cancellation, None, None, None, || async {
        on_ready()
    })
    .await
}

pub async fn run_authorized_with_ready<F: Future<Output = Result<()>>>(
    config: &PlacementConfig,
    cancellation: CancellationToken,
    authorizer: Option<Arc<dyn RequestAuthorizer>>,
    api_base_url: Option<&str>,
    workload_identity: Option<crate::config::WorkloadIdentity>,
    on_ready: impl FnOnce() -> F,
) -> Result<()> {
    run_supervised_with_ready(
        config,
        cancellation,
        authorizer,
        api_base_url,
        workload_identity,
        None,
        None,
        None,
        None,
        None,
        None,
        PersonStartedRuns::default(),
        on_ready,
    )
    .await
}

/// The agent parent's queue of person-started runs, which a supervised placement process
/// asks for work.
#[derive(Clone, Default)]
pub struct PersonStartedRuns {
    #[cfg(feature = "on-demand")]
    pub(crate) source: Option<Arc<dyn crate::on_demand::RunSource>>,
}

impl PersonStartedRuns {
    #[cfg(unix)]
    #[cfg_attr(not(feature = "on-demand"), allow(unused_variables))]
    pub fn supervised(broker: Arc<crate::ipc::ChildBroker>) -> Self {
        Self {
            #[cfg(feature = "on-demand")]
            source: Some(broker),
        }
    }
}

/// `schedule` is what a placement with schedules or bots needs to run them, and with
/// `runs` what one with person-started events needs; without them such a placement is only
/// validated.
#[allow(clippy::too_many_arguments)]
pub async fn run_supervised_with_ready<F: Future<Output = Result<()>>>(
    config: &PlacementConfig,
    cancellation: CancellationToken,
    authorizer: Option<Arc<dyn RequestAuthorizer>>,
    api_base_url: Option<&str>,
    workload_identity: Option<crate::config::WorkloadIdentity>,
    outage_authority: Option<Arc<dyn crate::online::outage::OutageAuthority>>,
    inherited_listener: Option<tokio::net::TcpListener>,
    replica: Option<crate::hosting::ReplicaContext>,
    data_root: Option<&Path>,
    service_tls: Option<Arc<dyn ServiceTlsProvider>>,
    mut schedule: Option<crate::schedule::ScheduleContext>,
    runs: PersonStartedRuns,
    on_ready: impl FnOnce() -> F,
) -> Result<()> {
    config.validate()?;
    ensure!(!config.events.is_empty(), "placement has no events");
    ensure!(
        config.project_path.is_absolute(),
        "project_path must be an absolute object-store root"
    );
    if let Some(root) = data_root {
        crate::placement_data::validate_root(root, config)?;
    }
    ensure!(
        config.tls_certificate_id.is_none() || service_tls.is_some(),
        "Managed TLS requires the placement certificate broker"
    );
    if let Some(tls) = &service_tls {
        tls.validate().await?;
    }
    let local_data = data_root.unwrap_or(&config.project_path);
    let mut delegating_user_id = None;
    let mut revoked = CancellationToken::new();
    let state = match config.source {
        ProjectSource::Offline => {
            initialize_state(
                placement_local_config(config, local_data)?,
                authorizer,
                None,
                service_tls.clone(),
            )
            .await?
        }
        ProjectSource::Online => {
            let authorizer =
                authorizer.context("Online placement requires its workload credential broker")?;
            let identity = workload_identity
                .context("Online placement requires its local workload identity")?;
            let mut runtime = placement_local_config(config, local_data)?;
            let online = crate::online::configure_with_local_data(
                config,
                &identity,
                authorizer.clone(),
                &mut runtime,
                local_data,
                outage_authority,
            )
            .await?;
            delegating_user_id = Some(online.delegating_user_id);
            revoked = online.revoked;
            if let Some(schedule) = &mut schedule {
                schedule.claims = Some(online.schedules);
            }
            initialize_state(
                runtime,
                Some(authorizer),
                Some(online.registry),
                service_tls.clone(),
            )
            .await?
        }
    };
    let mut profile = Profile::default();
    if let Some(base) = api_base_url {
        let base = flow_like_device_protocol::canonical_api_base_url(base)?;
        let url = url::Url::parse(&base)?;
        profile.secure = url.scheme() == "https";
        profile.hub = base
            .strip_prefix("https://")
            .or_else(|| base.strip_prefix("http://"))
            .context("Invalid enrolled API URL")?
            .trim_end_matches("/api/v1")
            .to_string();
    }
    until_authorization_revoked(
        revoked,
        cancellation.clone(),
        run_with_state_listener(
            config,
            state,
            cancellation,
            profile,
            inherited_listener,
            replica,
            on_ready,
            false,
            delegating_user_id,
            schedule,
            runs,
        ),
    )
    .await
}

async fn until_authorization_revoked(
    revoked: CancellationToken,
    cancellation: CancellationToken,
    service: impl Future<Output = Result<()>>,
) -> Result<()> {
    tokio::pin!(service);
    tokio::select! {
        biased;
        _ = revoked.cancelled() => {
            // Stop accepting new work even when the current workflow only uses
            // already loaded constants and never touches its fenced stores.
            cancellation.cancel();
            let _ = service.await;
            Err(flow_like_types::authorization::AuthorizationError::Denied.into())
        }
        result = &mut service => {
            if revoked.is_cancelled() { Err(flow_like_types::authorization::AuthorizationError::Denied.into()) } else { result }
        }
    }
}

pub(crate) fn private_runtime_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
        match std::fs::DirBuilder::new().mode(0o700).create(path) {
            Ok(()) => (),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(error) => return Err(error.into()),
        }
        let directory = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY)
            .open(path)?;
        let metadata = directory.metadata()?;
        ensure!(
            metadata.is_dir()
                && !metadata.file_type().is_symlink()
                && metadata.uid() == unsafe { libc::geteuid() },
            "Runtime store directories must be private owned directories without symlinks"
        );
        if metadata.mode() & 0o077 != 0 {
            directory.set_permissions(std::fs::Permissions::from_mode(0o700))?;
        }
        Ok(())
    }
    #[cfg(not(unix))]
    anyhow::bail!("Private runtime directories require Unix permissions")
}

fn local_store(path: &Path) -> Result<FlowLikeStore> {
    private_runtime_directory(path)?;
    existing_local_store(path)
}

fn existing_local_store(path: &Path) -> Result<FlowLikeStore> {
    let metadata = std::fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "Local runtime store must be a directory without symlinks"
    );
    Ok(FlowLikeStore::Local(Arc::new(
        LocalObjectStore::new(path.to_path_buf()).context("open local runtime store")?,
    )))
}

/// Metadata validation must not create caches or mutable stores inside the
/// imported revision. Bit source paths stay local for capability checks; the
/// cancelled validation path verifies them without copying or writing packs.
pub(crate) async fn inspection_state(
    root: &Path,
    authorizer: Option<Arc<dyn RequestAuthorizer>>,
) -> Result<Arc<FlowLikeState>> {
    let mut config = FlowLikeConfig::new();
    let memory = || {
        FlowLikeStore::Memory(Arc::new(
            flow_like_storage::object_store::memory::InMemory::new(),
        ))
    };
    config.register_app_meta_store(existing_local_store(root)?.read_only());
    config.register_app_storage_store(memory());
    config.register_user_store(memory());
    config.register_temporary_store(memory());
    config.register_log_store(memory());
    let bits = root.join("bits");
    config.register_bits_store(match std::fs::symlink_metadata(&bits) {
        Ok(_) => existing_local_store(&bits)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => memory(),
        Err(error) => return Err(error.into()),
    });
    initialize_state(config, authorizer, None, None).await
}

#[cfg(test)]
pub(crate) async fn offline_state(
    root: &Path,
    authorizer: Option<Arc<dyn RequestAuthorizer>>,
) -> Result<Arc<FlowLikeState>> {
    initialize_state(local_config(root)?, authorizer, None, None).await
}

fn local_config(root: &Path) -> Result<FlowLikeConfig> {
    local_config_with_data(root, root)
}

fn placement_local_config(placement: &PlacementConfig, data_root: &Path) -> Result<FlowLikeConfig> {
    let mut config = local_config_with_data(&placement.project_path, data_root)?;
    if !placement.bit_pins.is_empty() {
        // Changed model metadata gets a distinct local path. An old replica
        // can retain its open model while the next revision materializes.
        let bits = crate::dependencies::placement_bit_store(placement, data_root)?;
        config.register_bits_store(local_store(&bits)?);
    }
    Ok(config)
}

fn local_config_with_data(root: &Path, data_root: &Path) -> Result<FlowLikeConfig> {
    let mut config = FlowLikeConfig::new();
    config.register_app_storage_store(local_store(data_root)?);
    let metadata = existing_local_store(root)?;
    config.register_app_meta_store(if root == data_root {
        metadata
    } else {
        metadata.read_only()
    });
    config.register_bits_store(local_store(&data_root.join("bits"))?);
    config.register_user_store(local_store(&data_root.join("user"))?);
    config.register_temporary_store(local_store(&data_root.join("tmp"))?);
    config.register_log_store(local_store(&data_root.join("logs"))?);

    let project_root = data_root.to_path_buf();
    config.register_build_project_database(Arc::new(move |path: StorePath| {
        connect_lance(project_root.join(path.as_ref()).to_string_lossy().as_ref())
    }));
    let user_root = data_root.join("user");
    config.register_build_user_database(Arc::new(move |path: StorePath| {
        connect_lance(user_root.join(path.as_ref()).to_string_lossy().as_ref())
    }));
    let logs_root = data_root.join("logs");
    config.register_build_logs_database(Arc::new(move |path: StorePath| {
        connect_lance(logs_root.join(path.as_ref()).to_string_lossy().as_ref())
    }));
    Ok(config)
}

async fn initialize_state(
    config: FlowLikeConfig,
    authorizer: Option<Arc<dyn RequestAuthorizer>>,
    registry: Option<Arc<flow_like_storage::lance_io::object_store::ObjectStoreRegistry>>,
    service_tls: Option<Arc<dyn ServiceTlsProvider>>,
) -> Result<Arc<FlowLikeState>> {
    let mut state = FlowLikeState::new(config, HTTPClient::new_without_refetch());
    // Store-backed local data works under Server. Ambient host credentials and
    // arbitrary filesystem paths remain unavailable to deployed workflows.
    state.execution_environment = ExecutionEnvironment::Server;
    state.request_authorizer = authorizer;
    state.local_model_router = crate::models::router::placement_router();
    state.service_tls_provider = service_tls;
    if let Some(registry) = registry {
        state.set_lance_store_registry(registry);
    }
    let state = Arc::new(state);
    flow_like_catalog::initialize();
    let catalog = Arc::new(flow_like_catalog::get_catalog());
    let mut registry = state.node_registry.write().await;
    registry.initialize(Arc::downgrade(&state));
    registry.node_registry = Arc::new(FlowNodeRegistryInner::prepare(&catalog));
    drop(registry);
    Ok(state)
}

#[cfg(test)]
async fn run_with_state(
    config: &PlacementConfig,
    state: Arc<FlowLikeState>,
    cancellation: CancellationToken,
) -> Result<()> {
    run_with_state_ready(config, state, cancellation, Profile::default(), || async {
        Ok(())
    })
    .await
}

#[cfg(test)]
async fn run_with_state_ready<F: Future<Output = Result<()>>>(
    config: &PlacementConfig,
    state: Arc<FlowLikeState>,
    cancellation: CancellationToken,
    profile: Profile,
    on_ready: impl FnOnce() -> F,
) -> Result<()> {
    run_with_state_listener(
        config,
        state,
        cancellation,
        profile,
        None,
        None,
        on_ready,
        false,
        None,
        None,
        PersonStartedRuns::default(),
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn run_with_state_listener<F: Future<Output = Result<()>>>(
    config: &PlacementConfig,
    state: Arc<FlowLikeState>,
    cancellation: CancellationToken,
    mut profile: Profile,
    inherited_listener: Option<tokio::net::TcpListener>,
    replica: Option<crate::hosting::ReplicaContext>,
    on_ready: impl FnOnce() -> F,
    validate_secrets: bool,
    delegating_user_id: Option<String>,
    mut schedule: Option<crate::schedule::ScheduleContext>,
    runs: PersonStartedRuns,
) -> Result<()> {
    #[cfg(not(feature = "on-demand"))]
    let _ = runs;
    let app = App::load(config.project_id.clone(), state.clone())
        .await
        .context("load project manifest")?;
    ensure!(
        app.id == config.project_id
            && matches!(app.visibility, AppVisibility::Offline)
                == (config.source == ProjectSource::Offline),
        "placement must reference the matching project source"
    );
    crate::dependencies::register_packages(config, &app, &state).await?;
    crate::dependencies::hydrate_bits(
        config,
        &app,
        &state,
        &mut profile,
        !cancellation.is_cancelled(),
    )
    .await?;

    let template_cache = TemplateCache::default();
    for pin in &config.artifact_pins {
        let version = pinned_version(pin.version)?;
        match pin.kind {
            crate::config::ArtifactKind::Widget => {
                let widget = app.open_widget(pin.id.clone(), Some(version)).await?;
                ensure!(
                    widget.id == pin.id && widget.version == Some(version),
                    "Widget does not match the placement pin"
                );
            }
            crate::config::ArtifactKind::Template => {
                let template = flow_like_runtime::flow::board::Board::load_template(
                    StorePath::from("apps").join(app.id.as_str()),
                    &pin.id,
                    state.clone(),
                    Some(version),
                )
                .await?;
                ensure!(
                    template.id == pin.id && template.version == version,
                    "Template does not match the placement pin"
                );
            }
        }
    }
    let mut prepared = Vec::with_capacity(config.events.len());
    let mut hosted = Vec::new();
    let mut scheduled = Vec::new();
    let mut native_host_required = false;
    let mut selected_events = HashSet::new();
    let mut applied_variables = HashSet::new();
    let mut self_firing = 0;
    let mut bot_ids = Vec::new();
    #[cfg(feature = "bots")]
    let mut bots = Vec::new();
    #[cfg(feature = "on-demand")]
    let mut on_demand = Vec::new();
    let mut route_claims = Vec::new();
    let stop = cancellation.child_token();
    let _cancel_on_drop = stop.clone().drop_guard();

    // Resolve every pin and override before any event can open a listener.
    for binding in &config.events {
        ensure!(
            selected_events.insert(binding.event_id.clone()),
            "duplicate event binding {}",
            binding.event_id
        );
        let event_version = pinned_version(binding.event_version)?;
        let board_version = pinned_version(binding.board_version)?;
        let mut event = flow_like_runtime::flow::event::Event::load_pinned(
            &binding.event_id,
            &app,
            event_version,
        )
        .await
        .with_context(|| format!("load pinned event {}", binding.event_id))?;
        let kind = EventKind::runnable(&event.event_type, event.default_page_id.is_some());
        ensure!(
            config.max_replicas == 1 || kind.is_some_and(|kind| !kind.limits_instances()),
            "Multiple replicas require HTTP, chat, or Page events; {} cannot be replicated",
            if kind == Some(EventKind::Bot) {
                "bots"
            } else {
                "scheduled and daemon triggers"
            }
        );
        ensure!(
            event.id == binding.event_id && event.event_version == event_version,
            "event archive does not match the requested identity and version"
        );
        ensure!(event.active, "event {} is inactive", event.id);
        let Some(kind) = kind else {
            bail!(
                "event {} needs an unsupported {} sink",
                event.id,
                event.event_type
            );
        };
        self_firing += usize::from(kind.self_firing());
        if kind == EventKind::Bot {
            bot_ids.push(event.id.clone());
        }
        ensure!(
            event.variant_set().is_empty(),
            "Traffic variants require separate pinned placements"
        );
        ensure!(
            event.board_version == Some(board_version),
            "event {} does not pin the deployment's board version",
            event.id
        );
        let source_board = app
            .open_board(event.board_id.clone(), Some(false), Some(board_version))
            .await?;
        crate::dependencies::validate_board_packages(config, &source_board.snapshot())?;
        let template = template_cache
            .resolve(
                &state,
                &app.id,
                &event.board_id,
                Some(board_version),
                None,
                "",
            )
            .await
            .with_context(|| format!("resolve board for event {}", event.id))?;
        ensure!(
            template.board.id == event.board_id && template.board.version == board_version,
            "board archive does not match the event's pinned board"
        );
        ensure!(
            event.default_page_id.is_some()
                || template
                    .nodes
                    .iter()
                    .any(|node| node.id.as_ref() == event.node_id && node.can_seed_run),
            "event {} does not reference an executable entry node",
            event.id
        );

        for variable in &template.variables {
            // The agent would hand a bot's token to such a variable as its value.
            ensure!(
                crate::event_kind::bot_token_event(&variable.variable.id).is_none(),
                "variable id {} is reserved for a bot token",
                variable.variable.id
            );
            let secret = config.secret_overrides.get(&variable.variable.id);
            let plain = config.variables.get(&variable.variable.id);
            if secret.is_none() && plain.is_none() {
                continue;
            }
            let mut overridden = variable.variable.clone();
            ensure!(
                overridden.secret == secret.is_some(),
                "variable {} must use the matching secret or plaintext override type",
                overridden.id
            );
            ensure!(
                overridden.exposed || overridden.runtime_configured,
                "variable {} cannot be overridden by the placement",
                overridden.id
            );
            applied_variables.insert(overridden.id.clone());
            // Applying intent checks the pinned contract. Private values are
            // required when the workload starts, after secret provisioning.
            if secret.is_some() && stop.is_cancelled() && !validate_secrets {
                continue;
            }
            let private_value;
            let value = if let Some(name) = secret {
                let bytes =
                    crate::vault::read_private(&crate::config::private_secret_path(config, name)?)?;
                private_value = serde_json::from_slice::<serde_json::Value>(&bytes)
                    .map_err(|_| anyhow::anyhow!("Placement secret must contain a JSON value"))?;
                &private_value
            } else {
                plain.expect("override exists")
            };
            // Validation errors can contain the supplied value. Keep secrets out
            // of the supervisor's diagnostic stream.
            ensure!(
                validate_override(&overridden, &template.board.refs, value).is_ok(),
                "placement variable {} does not satisfy its pinned schema",
                overridden.id
            );
            overridden.default_value = Some(serde_json::to_vec(value)?);
            event.variables.insert(overridden.id.clone(), overridden);
        }
        // Checked at validation: a route the listener cannot take must never reach a start.
        route_claims.extend(crate::hosting::route_claims(
            &event,
            config.hosting.is_some(),
        )?);

        // A schedule needs no listener and reports no readiness of its own.
        if kind == EventKind::Scheduled {
            let spec = schedule_of(&event)?;
            if !stop.is_cancelled() {
                let invocation = crate::hosting::PreparedInvocation {
                    event,
                    template,
                    action_admission: None,
                };
                scheduled.push((spec, invocation));
            }
            continue;
        }

        // A bot connects only after Ready; its settings, and with secrets its token, are
        // checked here.
        #[cfg(feature = "bots")]
        if kind == EventKind::Bot {
            let invocation = crate::hosting::PreparedInvocation {
                event,
                template,
                action_admission: None,
            };
            let bot =
                crate::bots::prepare(config, invocation, validate_secrets || !stop.is_cancelled())?;
            if !stop.is_cancelled() {
                bots.push(bot);
            }
            continue;
        }

        // A person starts it. An optional configured listener also exposes its streamed run route.
        #[cfg(feature = "on-demand")]
        if kind == EventKind::OnDemand {
            native_host_required |= config.hosting.is_some();
            let event = crate::on_demand::prepare(crate::hosting::PreparedInvocation {
                event,
                template,
                action_admission: None,
            })?;
            if !stop.is_cancelled() {
                on_demand.push(event);
            }
            continue;
        }

        let readiness = service_readiness_source(&event, &template.board)?;
        native_host_required |= readiness.is_none();
        if stop.is_cancelled() {
            continue;
        }

        if kind.hosted() {
            hosted.push(crate::hosting::PreparedInvocation {
                event,
                template,
                action_admission: None,
            });
            continue;
        }

        let event_id = event.id.clone();
        let payload = RunPayload {
            id: event.node_id.clone(),
            payload: None,
            runtime_variables: None,
            filter_secrets: Some(false),
        };
        let callback_event_id = event_id.clone();
        let observer = crate::usage::service_observer();
        let callback_observer = observer.clone();
        let buffered = BufferedInterComHandler::new(
            Arc::new(move |events| {
                let event_id = callback_event_id.clone();
                let observer = callback_observer.clone();
                Box::pin(async move {
                    for _ in &events {
                        observer.message();
                    }
                    tracing::trace!(event_id, messages = events.len(), "Runtime events buffered");
                    Ok(())
                })
            }),
            Some(100),
            Some(400),
            Some(true),
        );
        let mut run = InternalRun::from_template(
            &app.id,
            template,
            Some(event),
            &state,
            &profile,
            &payload,
            false,
            buffered.into_callback(),
            None,
            None,
            Default::default(),
            None,
            None,
        )
        .await
        .with_context(|| format!("prepare event {event_id}"))?;
        run.set_usage_attribution_from_visibility(&app.visibility)
            .await;
        if let Some(sub) = &delegating_user_id {
            run.set_execution_sub(sub.clone()).await;
            run.set_unresolved_user_context().await;
        } else {
            run.set_offline_user_context();
        }
        let (node_id, kind) = readiness.context("Service requires a readiness source")?;
        let receiver = run.set_service_readiness(node_id, kind).await;
        let drain = CancellationToken::new();
        run.set_service_drain(drain.clone()).await;
        run.set_service_observer(observer.clone()).await;
        let run_stop = CancellationToken::new();
        run.set_cancellation_token(run_stop.clone());
        run.set_cancellation_log("Standalone placement stopped", LogLevel::Info);
        run.set_log_flush_policy(Duration::from_secs(5), 500)
            .await?;
        prepared.push((
            event_id, run, buffered, receiver, kind, drain, run_stop, observer,
        ));
    }
    ensure!(
        self_firing <= MAX_SELF_FIRING,
        "A service runs at most {MAX_SELF_FIRING} schedules and bots, got {self_firing}"
    );
    crate::hosting::check_route_claims(&route_claims)?;
    check_overrides_applied(config, &applied_variables, &bot_ids)?;
    if validate_secrets && native_host_required {
        crate::hosting::validate_access_token(config)?;
    }
    ensure!(
        native_host_required || config.hosting.is_none(),
        "Hosting configuration requires an HTTP, chat, Page, quick action, or form event"
    );
    if stop.is_cancelled() {
        return Ok(());
    }
    #[cfg(any(feature = "on-demand", feature = "bots"))]
    let run_context = Arc::new(crate::run_once::RunContext {
        project_id: app.id.clone(),
        state: state.clone(),
        profile: profile.clone(),
        visibility: app.visibility.clone(),
        execution_sub: delegating_user_id.clone(),
    });
    #[cfg(feature = "on-demand")]
    let on_demand_context = (!on_demand.is_empty())
        .then(|| {
            person_started_context(
                config,
                schedule.as_ref(),
                replica,
                run_context.clone(),
                &runs,
            )
        })
        .transpose()?;
    let gates = crate::schedule::ClaimGates::new(bot_ids);
    // Without a schedule context the scheduler below refuses the bots with round one's sentence.
    #[cfg(feature = "bots")]
    let bot_context = match &schedule {
        Some(context) if !bots.is_empty() => Some(crate::bots::BotContext::beside(
            context,
            run_context.clone(),
            gates.clone(),
        )?),
        _ => None,
    };
    let scheduler = prepare_schedules(&mut schedule, &scheduled, &gates)?;
    #[cfg(feature = "bots")]
    if let Some(context) = &bot_context {
        crate::bots::prepare_state(&bots, context)?;
    }
    #[cfg(feature = "on-demand")]
    if let Some(context) = &on_demand_context {
        crate::on_demand::prepare_state(&on_demand, context)?;
    }
    let hosting = if native_host_required {
        Some(
            crate::hosting::PreparedHost::bind_with_listener(
                config,
                hosted,
                state.clone(),
                profile.clone(),
                app.visibility.clone(),
                stop.clone(),
                inherited_listener,
                replica,
            )
            .await?,
        )
    } else {
        ensure!(
            config.hosting.is_none(),
            "Hosting configuration requires an HTTP or chat event"
        );
        None
    };
    let mut tasks = JoinSet::new();
    let mut readiness = JoinSet::new();
    if let Some(mut hosting) = hosting {
        if let Some(sub) = delegating_user_id.clone() {
            hosting.set_execution_sub(sub)?;
        }
        // An explicitly configured listener offers its forms on the service page.
        #[cfg(feature = "on-demand")]
        if let Some(context) = &on_demand_context {
            hosting.with_on_demand(&on_demand, context.counters())?;
        }
        let (ready, receiver) = tokio::sync::oneshot::channel();
        tasks.spawn(async move { hosting.serve_with_ready(Some(ready)).await });
        readiness.spawn(async move {
            receiver
                .await
                .context("Native HTTP host exited before accepting requests")
        });
    }
    for (event_id, mut run, buffered, receiver, kind, drain, run_stop, observer) in prepared {
        let ready_event = event_id.clone();
        readiness.spawn(async move {
            receiver
                .await
                .with_context(|| format!("Service {ready_event} exited before reporting readiness"))
        });
        let state = state.clone();
        let stop = stop.clone();
        tasks.spawn(async move {
            let _cancel_on_drop = run_stop.clone().drop_guard();
            let invocation = observer.begin(0);
            tracing::info!(event_id, "Starting standalone event");
            {
                let execution = run.execute(state);
                tokio::pin!(execution);
                tokio::select! {
                    _ = &mut execution => (),
                    _ = stop.cancelled() => {
                        drain.cancel();
                        if kind == ServiceReadyKind::Daemon { run_stop.cancel(); }
                        if tokio::time::timeout(Duration::from_secs(12), &mut execution).await.is_err() {
                            run_stop.cancel();
                            execution.await;
                        }
                    }
                }
            }
            buffered.flush().await?;
            let status = run.get_status().await;
            invocation.finish(if stop.is_cancelled() { ServiceOutcome::Cancelled } else { ServiceOutcome::Failed });
            if stop.is_cancelled() { return Ok(()); }
            match status {
                RunStatus::Success | RunStatus::Stopped => bail!("Persistent service {event_id} exited; services must keep running after readiness"),
                RunStatus::Failed => bail!("standalone event {event_id} failed"),
                RunStatus::Running => bail!("standalone event {event_id} exited without final status"),
            }
        });
    }
    // A oneshot has one receiver: every task that waits for Ready gets a channel of its own.
    let mut ready_senders = Vec::new();
    if let Some(scheduler) = scheduler {
        let environment = crate::schedule::RunEnvironment {
            project_id: app.id.clone(),
            state: state.clone(),
            profile: profile.clone(),
            visibility: app.visibility.clone(),
            execution_sub: delegating_user_id,
        };
        let runner = crate::schedule::flow_runner(environment, scheduled);
        let ready = ready_channel(&mut ready_senders);
        tasks.spawn(scheduler.run(runner, ready, stop.clone()));
    } else if let Some(context) = schedule {
        // Detached: a task that ends in `tasks` counts as a service that exited.
        let ready = ready_channel(&mut ready_senders);
        tokio::spawn(crate::schedule::hand_back_all(context, ready, stop.clone()));
    }
    #[cfg(feature = "bots")]
    if let Some(context) = bot_context {
        let ready = ready_channel(&mut ready_senders);
        tasks.spawn(crate::bots::run(bots, context, ready, stop.clone()));
    }
    #[cfg(feature = "on-demand")]
    if let Some(context) = on_demand_context {
        let ready = ready_channel(&mut ready_senders);
        tasks.spawn(crate::on_demand::run(
            on_demand,
            context,
            ready,
            stop.clone(),
        ));
    }
    let deadline = tokio::time::sleep(SERVICE_STARTUP_TIMEOUT);
    tokio::pin!(deadline);
    let startup = async {
        while !readiness.is_empty() {
            tokio::select! {
                biased;
                _ = stop.cancelled() => return Ok(false),
                result = tasks.join_next() => {
                    result.context("No service remained during startup")?.context("Service task failed during startup")??;
                    bail!("Service exited before placement readiness");
                }
                _ = &mut deadline => bail!("Service readiness timed out after 60 seconds; check listener binding and the Service Ready node"),
                result = readiness.join_next() => { result.context("Missing service readiness result")?.context("Service readiness task failed")??; }
            }
        }
        if stop.is_cancelled() { return Ok(false); }
        on_ready().await.context("report running placement to the supervisor")?;
        Ok(true)
    }.await;
    if matches!(startup, Ok(true)) {
        // Schedules, bots and person-started runs begin only once the service was accepted
        // as ready. A sender dropped without a send means "never ready".
        for sender in ready_senders {
            let _ = sender.send(());
        }
    }
    let mut first_error = startup.err();
    if first_error.is_some() || stop.is_cancelled() {
        stop.cancel();
    }
    while let Some(result) = tasks.join_next().await {
        match result
            .context("standalone event task failed")
            .and_then(|result| result)
        {
            Err(error) => {
                stop.cancel();
                first_error.get_or_insert(error);
            }
            Ok(()) if !stop.is_cancelled() => {
                stop.cancel();
                first_error.get_or_insert_with(|| {
                    anyhow::anyhow!("Persistent service exited unexpectedly")
                });
            }
            _ => (),
        }
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

const SERVICE_STARTUP_TIMEOUT: Duration = crate::supervisor::SERVICE_STARTUP_TIMEOUT;
/// Schedules and bots of one service together: the size of one claim call.
const MAX_SELF_FIRING: usize = 64;

/// What the person-started events of a supervised process share. The schedule context is the
/// one only a supervised process has: it names the placement's data root and revisions.
#[cfg(feature = "on-demand")]
fn person_started_context(
    config: &PlacementConfig,
    supervised: Option<&crate::schedule::ScheduleContext>,
    replica: Option<crate::hosting::ReplicaContext>,
    run: Arc<crate::run_once::RunContext>,
    runs: &PersonStartedRuns,
) -> Result<crate::on_demand::OnDemandContext> {
    let supervised =
        supervised.context("person-started events need the supervised placement runtime")?;
    let data_root = supervised
        .state_dir
        .parent()
        .and_then(Path::parent)
        .context("The schedule state directory lies outside a placement data root")?;
    Ok(crate::on_demand::OnDemandContext::new(
        run,
        runs.source.clone(),
        crate::on_demand::state_directory(data_root, &supervised.placement_id),
        supervised.placement_id.clone(),
        replica.map_or(0, |replica| replica.slot),
        supervised.config_revision,
        supervised.intent_revision,
        crate::on_demand::time_limit(config),
    ))
}

/// Every override names a variable of the pinned flows, or, for a secret, the token of a bot
/// event of this placement; anything else would be applied nowhere.
fn check_overrides_applied(
    config: &PlacementConfig,
    applied_variables: &HashSet<String>,
    bot_ids: &[String],
) -> Result<()> {
    let applied = |id: &str| {
        ensure!(
            applied_variables.contains(id),
            "placement variable {id} is absent from the selected pinned boards"
        );
        Ok(())
    };
    config.variables.keys().try_for_each(|id| applied(id))?;
    for id in config.secret_overrides.keys() {
        match crate::event_kind::bot_token_event(id) {
            Some(event_id) => ensure!(
                bot_ids.iter().any(|bot| bot == event_id),
                "secret {id} names the token of a bot that this placement does not run"
            ),
            None => applied(id)?,
        }
    }
    Ok(())
}

/// The receiver of one task that begins only once the service was accepted as ready.
fn ready_channel(
    senders: &mut Vec<tokio::sync::oneshot::Sender<()>>,
) -> tokio::sync::oneshot::Receiver<()> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    senders.push(sender);
    receiver
}

/// The schedule of an event of the kind `Scheduled`. An event with a Page is served, and
/// its schedule is not read.
fn schedule_of(
    event: &flow_like_runtime::flow::event::Event,
) -> Result<crate::schedule::ScheduleSpec> {
    crate::schedule::ScheduleSpec::from_event(event)
        .with_context(|| format!("schedule of event {}", event.id))
}

/// The scheduler of a placement that has schedules or bots: it makes the one claim call
/// for both. Its state directory must be usable before any listener opens: without a
/// durable watermark a restart could repeat a scheduled time.
fn prepare_schedules(
    context: &mut Option<crate::schedule::ScheduleContext>,
    scheduled: &[(
        crate::schedule::ScheduleSpec,
        crate::hosting::PreparedInvocation,
    )],
    gates: &crate::schedule::ClaimGates,
) -> Result<Option<crate::schedule::Scheduler>> {
    if scheduled.is_empty() && gates.is_empty() {
        return Ok(None);
    }
    let context = context
        .take()
        .context("schedules need the supervised placement runtime")?;
    let specs = scheduled
        .iter()
        .map(|(spec, invocation)| (invocation.event.id.clone(), spec.clone()))
        .collect();
    crate::schedule::prepare_with_gates(context, specs, gates.clone())
        .map(Some)
        .context("prepare the placement's schedules")
}

/// Only a top-level node can announce this event's startup. Request function
/// layers inherit the run cache but never receive readiness authority.
pub(crate) fn service_readiness_source(
    event: &flow_like_runtime::flow::event::Event,
    board: &flow_like_runtime::flow::board::Board,
) -> Result<Option<(String, ServiceReadyKind)>> {
    if EventKind::of(&event.event_type, event.default_page_id.is_some())
        .is_some_and(EventKind::hosted)
    {
        return Ok(None);
    }
    let source = crate::event_kind::OwnReadiness::of(&event.event_type)
        .with_context(|| format!("Event {} does not support supervised readiness", event.id))?;
    let name = source.node_name();
    let kind = match source {
        crate::event_kind::OwnReadiness::RestServer => ServiceReadyKind::RestListener,
        crate::event_kind::OwnReadiness::McpServer => ServiceReadyKind::McpListener,
        crate::event_kind::OwnReadiness::ServiceReady => ServiceReadyKind::Daemon,
    };
    let mut candidates = board.nodes.values().filter(|node| node.name == name);
    let node = candidates.next().with_context(|| {
        format!(
            "Event {} requires one top-level {name} node to report service readiness",
            event.id
        )
    })?;
    ensure!(
        candidates.next().is_none(),
        "Event {} has multiple {name} nodes; split independently managed services into separate pinned boards",
        event.id
    );
    Ok(Some((node.id.clone(), kind)))
}

fn pinned_version(version: [u32; 3]) -> Result<(u32, u32, u32)> {
    ensure!(
        !version.contains(&u32::MAX),
        "standalone deployments require explicit versions, not a Latest sentinel"
    );
    Ok((version[0], version[1], version[2]))
}

struct RejectSchemaRetrieval;

impl jsonschema::Retrieve for RejectSchemaRetrieval {
    fn retrieve(
        &self,
        _uri: &jsonschema::Uri<String>,
    ) -> std::result::Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "placement schemas must contain all referenced definitions",
        )
        .into())
    }
}

fn schema_validator(schema: &serde_json::Value) -> Result<jsonschema::Validator> {
    jsonschema::options()
        .with_retriever(RejectSchemaRetrieval)
        .with_pattern_options(jsonschema::PatternOptions::regex())
        .should_validate_formats(true)
        .should_ignore_unknown_formats(false)
        .build(schema)
        .map_err(|_| anyhow::anyhow!("unsupported or invalid placement variable schema"))
}

pub(crate) fn validate_override(
    variable: &Variable,
    refs: &std::collections::HashMap<String, String>,
    value: &serde_json::Value,
) -> Result<()> {
    let schema = variable
        .schema
        .as_deref()
        .map(|schema| resolve_schema(schema, refs))
        .transpose()?;
    if variable.data_type == VariableType::Geometry {
        let mut resolved = variable.clone();
        resolved.schema = schema.map(str::to_owned);
        return resolved.validate_value(value);
    }
    ensure!(
        variable.data_type != VariableType::Execution,
        "execution variables cannot be configured"
    );
    let validator = schema
        .map(|schema| {
            ensure!(
                schema.len() <= 64 * 1024,
                "placement variable schema exceeds 64 KiB"
            );
            let schema: serde_json::Value = serde_json::from_str(schema)?;
            schema_validator(&schema)
        })
        .transpose()?;
    let date_validator = (variable.data_type == VariableType::Date)
        .then(|| schema_validator(&serde_json::json!({"type": "string", "format": "date-time"})))
        .transpose()?;
    let validate_item = |value: &serde_json::Value| -> Result<()> {
        let type_matches = match variable.data_type {
            VariableType::String | VariableType::PathBuf | VariableType::Date => value.is_string(),
            VariableType::Integer => value.as_i64().is_some(),
            VariableType::Float => value.as_f64().is_some(),
            VariableType::Boolean => value.is_boolean(),
            VariableType::Struct => value.is_object(),
            VariableType::Byte => value
                .as_u64()
                .is_some_and(|number| number <= u64::from(u8::MAX)),
            VariableType::Generic => true,
            VariableType::Execution | VariableType::Geometry => false,
        };
        ensure!(type_matches, "placement variable has the wrong JSON type");
        ensure!(
            date_validator
                .as_ref()
                .is_none_or(|schema| schema.is_valid(value)),
            "placement date must be an RFC 3339 date-time"
        );
        ensure!(
            validator
                .as_ref()
                .is_none_or(|schema| schema.is_valid(value)),
            "placement variable violates its schema"
        );
        Ok(())
    };
    match variable.value_type {
        ValueType::Normal => validate_item(value),
        ValueType::Array | ValueType::HashSet => {
            let values = value
                .as_array()
                .context("placement variable requires an array")?;
            ensure!(
                values.len() <= 16_384,
                "placement collection exceeds 16384 entries"
            );
            if variable.value_type == ValueType::HashSet {
                ensure!(
                    schema_validator(&serde_json::json!({"uniqueItems": true}))?.is_valid(value),
                    "placement set contains duplicate values"
                );
            }
            for value in values {
                validate_item(value)?;
            }
            Ok(())
        }
        ValueType::HashMap => {
            let values = value
                .as_object()
                .context("placement variable requires an object")?;
            ensure!(
                values.len() <= 16_384,
                "placement collection exceeds 16384 entries"
            );
            for value in values.values() {
                validate_item(value)?;
            }
            Ok(())
        }
    }
}

#[cfg(test)]
#[path = "runtime_mcp_tests.rs"]
mod mcp_tests;

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::config::{EventBinding, RestartPolicy};
    use flow_like_runtime::{
        bit::Metadata,
        flow::{
            board::{Board, commands::pins::connect_pins::connect_pins},
            event::{Event, EventExecutionMode, EventExposure},
            execution::context::ExecutionContext,
            node::{Node, NodeLogic},
            pin::ValueType,
            variable::{Variable, VariableType},
        },
    };
    use std::{
        collections::HashMap,
        sync::atomic::{AtomicUsize, Ordering},
        time::SystemTime,
    };
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        sync::{Notify, oneshot},
    };

    struct PersistentNode {
        started: Arc<Notify>,
        starts: Arc<AtomicUsize>,
    }

    #[flow_like_types::async_trait]
    impl NodeLogic for PersistentNode {
        fn get_node(&self) -> Node {
            let mut node = Node::new("standalone_test_daemon", "Daemon", "", "Tests");
            node.set_start(true);
            node.set_long_running(true);
            node.add_input_pin("exec_in", "Execute", "", VariableType::Execution);
            node
        }

        async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
            self.starts.fetch_add(1, Ordering::SeqCst);
            self.started.notify_one();
            std::future::pending().await
        }
    }

    pub(super) struct ListeningProbe(pub(super) std::sync::Mutex<Option<oneshot::Sender<String>>>);

    struct SecretProbe(std::sync::Mutex<Option<oneshot::Sender<(serde_json::Value, bool)>>>);

    #[flow_like_types::async_trait]
    impl NodeLogic for SecretProbe {
        fn get_node(&self) -> Node {
            let mut node = Node::new("standalone_test_daemon", "Daemon", "", "Tests");
            node.set_start(true);
            node.set_long_running(true);
            node.add_input_pin("exec_in", "Execute", "", VariableType::Execution);
            node
        }

        async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
            let (value, sensitive) = context.get_variable_value_ref("credential").await?;
            let value = value.lock().await.clone();
            if let Some(sender) = self.0.lock().unwrap().take() {
                let _ = sender.send((value, sensitive));
            }
            std::future::pending().await
        }
    }

    #[flow_like_types::async_trait]
    impl NodeLogic for ListeningProbe {
        fn get_node(&self) -> Node {
            let mut node = Node::new("standalone_test_listening", "Listening", "", "Tests");
            node.add_input_pin("exec", "Execute", "", VariableType::Execution);
            node.add_input_pin("address", "Address", "", VariableType::String);
            node
        }

        async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
            let address: String = context.evaluate_pin("address").await?;
            if let Some(sender) = self.0.lock().unwrap().take() {
                let _ = sender.send(address);
            }
            Ok(())
        }
    }

    pub(super) fn connect(board: &mut Board, from: &str, output: &str, to: &str, input: &str) {
        let output = board.nodes[from]
            .get_pin_by_name(output)
            .unwrap()
            .id
            .clone();
        let input = board.nodes[to].get_pin_by_name(input).unwrap().id.clone();
        connect_pins(board, from, &output, to, &input).unwrap();
    }

    #[tokio::test]
    async fn imported_deployment_discovery_pins_events_and_omits_private_defaults() {
        let source = tempfile::tempdir().unwrap();
        let (config, state, _, _) = fixture(source.path()).await;
        let mut app = App::load(config.project_id.clone(), state).await.unwrap();
        app.events = vec!["event".into()];
        app.save().await.unwrap();
        let mut event = app.get_event("event", Some((1, 0, 0))).await.unwrap();
        let mut secret = Variable::new("Credential", VariableType::String, ValueType::Normal);
        secret.id = "credential".into();
        secret.secret = true;
        secret.default_value = Some(br#""do-not-disclose""#.to_vec());
        event.variables.insert(secret.id.clone(), secret);
        event.event_version = (1, 0, 1);
        // Normal event publication archives the previous version, not this current one.
        event.save(&app, None).await.unwrap();
        let target = tempfile::tempdir().unwrap();
        crate::supervisor::prepare_state_dir(target.path()).unwrap();
        let store =
            crate::state::StateStore::open(&target.path().join("management.sqlite")).unwrap();
        let imported =
            crate::project_artifacts::import_local(&store, target.path(), "project", source.path())
                .unwrap();
        let revision = imported.descriptor.manifest_sha256;
        let events = crate::deployment::describe(target.path(), "project", &revision, None, None)
            .await
            .unwrap();
        assert_eq!(events["items"][0]["id"], "event");
        assert_eq!(
            events["items"][0]["event_version"],
            serde_json::json!([1, 0, 1])
        );
        assert_eq!(events["items"][0]["eligible"], true);
        let variables =
            crate::deployment::describe(target.path(), "project", &revision, Some("event"), None)
                .await
                .unwrap();
        assert_eq!(variables["items"][0]["id"], "credential");
        assert_eq!(variables["items"][0]["secret"], true);
        assert!(!variables.to_string().contains("do-not-disclose"));
        assert!(!variables.to_string().contains("default_value"));
        assert!(
            crate::deployment::describe(target.path(), "project", &revision, Some("other"), None)
                .await
                .is_err()
        );
        assert!(
            crate::deployment::describe(target.path(), "other", &revision, None, None)
                .await
                .is_err()
        );
        event.save(&app, Some((1, 0, 1))).await.unwrap();
        event.updated_at = SystemTime::now();
        event.save(&app, None).await.unwrap();
        let timestamp_only =
            crate::project_artifacts::import_local(&store, target.path(), "project", source.path())
                .unwrap();
        let events = crate::deployment::describe(
            target.path(),
            "project",
            &timestamp_only.descriptor.manifest_sha256,
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(events["items"][0]["eligible"], true);
        event.route = Some("/changed-after-archive".into());
        event.save(&app, None).await.unwrap();
        let changed_route =
            crate::project_artifacts::import_local(&store, target.path(), "project", source.path())
                .unwrap();
        let events = crate::deployment::describe(
            target.path(),
            "project",
            &changed_route.descriptor.manifest_sha256,
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(events["items"][0]["eligible"], false);
    }

    pub(crate) async fn fixture(
        root: &Path,
    ) -> (
        PlacementConfig,
        Arc<FlowLikeState>,
        Arc<Notify>,
        Arc<AtomicUsize>,
    ) {
        let state = offline_state(root, None).await.unwrap();
        let started = Arc::new(Notify::new());
        let starts = Arc::new(AtomicUsize::new(0));
        let logic = Arc::new(PersistentNode {
            started: started.clone(),
            starts: starts.clone(),
        });
        state.node_registry.write().await.push_node(logic.clone());
        let app = App::new(
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
            StorePath::from("apps/project"),
            state.clone(),
        );
        let mut node = logic.get_node();
        node.id = "entry".into();
        board.nodes.insert(node.id.clone(), node);
        let registry = state.node_registry.read().await;
        let definition = registry.get_node("service_ready").unwrap();
        let mut ready = registry.instantiate(&definition).unwrap().get_node();
        drop(registry);
        ready.id = "ready".into();
        board.nodes.insert(ready.id.clone(), ready);
        connect(&mut board, "ready", "exec_out", "entry", "exec_in");
        let mut secret = Variable::new("Credential", VariableType::String, ValueType::Normal);
        secret.id = "credential".into();
        secret.secret = true;
        secret.runtime_configured = true;
        board.variables.insert(secret.id.clone(), secret);
        board.snapshot_at_version((1, 0, 0), None).await.unwrap();
        let now = SystemTime::now();
        let event = Event {
            id: "event".into(),
            name: "Persistent service".into(),
            description: String::new(),
            board_id: board.id.clone(),
            board_version: Some((1, 0, 0)),
            node_id: "ready".into(),
            variables: HashMap::new(),
            config: vec![],
            active: true,
            canary: None,
            variants: vec![],
            priority: 0,
            event_type: "daemon".into(),
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
        event.save(&app, Some((1, 0, 0))).await.unwrap();
        let config = PlacementConfig {
            id: "placement".into(),
            project_id: app.id,
            deployment_id: "deployment".into(),
            revision: "r1".into(),
            source: ProjectSource::Offline,
            online_metadata_sha256: None,
            project_path: root.into(),
            events: vec![EventBinding {
                event_id: "event".into(),
                event_version: [1, 0, 0],
                board_version: [1, 0, 0],
            }],
            artifact_pins: Vec::new(),
            package_pins: Vec::new(),
            bit_pins: Vec::new(),
            max_replicas: 1,
            tunnel_services: vec![],
            tls_certificate_id: None,
            hosting: None,
            variables: Default::default(),
            secret_overrides: Default::default(),
            resource_grant: None,
            offline_writes: None,
            resources: None,
            restart: RestartPolicy::default(),
        };
        (config, state, started, starts)
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn inspection_keeps_source_read_only_and_mutable_roots_become_private() -> Result<()> {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let revision = tempfile::tempdir()?;
        let data = tempfile::tempdir()?;
        std::fs::set_permissions(revision.path(), std::fs::Permissions::from_mode(0o555))?;
        std::fs::set_permissions(data.path(), std::fs::Permissions::from_mode(0o755))?;
        let inspection = inspection_state(revision.path(), None).await?;
        assert_eq!(std::fs::metadata(revision.path())?.mode() & 0o777, 0o555);
        assert_eq!(std::fs::read_dir(revision.path())?.count(), 0);
        drop(inspection);
        let _runtime = local_config_with_data(revision.path(), data.path())?;
        assert_eq!(std::fs::metadata(revision.path())?.mode() & 0o777, 0o555);
        assert_eq!(std::fs::metadata(data.path())?.mode() & 0o777, 0o700);
        for child in ["bits", "user", "tmp", "logs"] {
            assert_eq!(
                std::fs::metadata(data.path().join(child))?.mode() & 0o777,
                0o700
            );
        }
        let link = data.path().join("link");
        std::os::unix::fs::symlink(revision.path(), &link)?;
        assert!(private_runtime_directory(&link).is_err());
        assert_eq!(std::fs::metadata(revision.path())?.mode() & 0o777, 0o555);
        std::fs::set_permissions(revision.path(), std::fs::Permissions::from_mode(0o700))?;
        Ok(())
    }

    #[tokio::test]
    async fn filesystem_embeddings_prefer_local_even_with_cloud_authorization() -> Result<()> {
        use flow_like_runtime::{
            bit::{Bit, BitTypes},
            flow_like_model_provider::provider::{
                EmbeddingModelProvider, ModelProvider, Pooling, Prefix, RemoteEmbeddingProvider,
                RemoteExecutionConfig,
            },
            models::embedding_factory::{EmbeddingFactory, prefers_local_execution},
        };
        use flow_like_types::authorization::{AuthorizationFuture, AuthorizationRequest};

        struct CloudAuthorizer;
        impl RequestAuthorizer for CloudAuthorizer {
            fn authorize<'a>(&'a self, _: AuthorizationRequest<'a>) -> AuthorizationFuture<'a> {
                panic!("Choosing local embeddings must not request cloud authorization");
            }
        }

        let bit = Bit {
            bit_type: BitTypes::Embedding,
            parameters: serde_json::to_value(EmbeddingModelProvider {
                languages: vec!["en".into()],
                vector_length: 384,
                input_length: 512,
                prefix: Prefix {
                    query: String::new(),
                    paragraph: String::new(),
                },
                pooling: Pooling::Mean,
                provider: ModelProvider {
                    provider_name: "Local".into(),
                    model_id: Some("embedding-model".into()),
                    api_surface: None,
                    version: None,
                    params: None,
                },
                remote: Some(RemoteExecutionConfig {
                    implementation: Some(RemoteEmbeddingProvider::Internal),
                    model_id: Some("embedding-model".into()),
                    ..Default::default()
                }),
            })?,
            ..Bit::default()
        };
        assert!(bit.try_to_embedding().unwrap().supports_remote());
        let root = tempfile::tempdir()?;
        let state = offline_state(root.path(), Some(Arc::new(CloudAuthorizer))).await?;
        assert!(state.request_authorizer.is_some());
        assert!(FlowLikeState::can_execute_local_bit_models(&state).await);
        assert!(prefers_local_execution(&bit, &state).await);
        // No file or download URL: the local loader stops before inference. A
        // remote route would instead construct a proxy despite the missing file.
        let error = match EmbeddingFactory::new()
            .build_text_routed(&bit, state, None, None)
            .await
        {
            Ok(_) => panic!("The filesystem host must load this embedding locally"),
            Err(error) => error,
        };
        assert_eq!(error.to_string(), "No model path");
        Ok(())
    }

    #[tokio::test]
    async fn supervised_stores_keep_pinned_metadata_separate_from_persistent_data() -> Result<()> {
        use flow_like_storage::object_store::ObjectStoreExt;
        let revision = tempfile::tempdir()?;
        let data = tempfile::tempdir()?;
        std::fs::create_dir_all(revision.path().join("apps/project"))?;
        std::fs::write(revision.path().join("apps/project/manifest.app"), b"pinned")?;
        let runtime = local_config_with_data(revision.path(), data.path())?;
        let storage = runtime
            .stores
            .app_storage_store
            .as_ref()
            .unwrap()
            .as_generic();
        storage
            .put(
                &StorePath::from("apps/project/files/live"),
                bytes::Bytes::from_static(b"live").into(),
            )
            .await?;
        assert_eq!(
            std::fs::read(data.path().join("apps/project/files/live"))?,
            b"live"
        );
        assert!(!revision.path().join("apps/project/files/live").exists());
        let metadata = runtime.stores.app_meta_store.as_ref().unwrap().as_generic();
        assert_eq!(
            metadata
                .get(&StorePath::from("apps/project/manifest.app"))
                .await?
                .bytes()
                .await?
                .as_ref(),
            b"pinned"
        );
        assert!(
            metadata
                .put(
                    &StorePath::from("apps/project/manifest.app"),
                    bytes::Bytes::from_static(b"changed").into()
                )
                .await
                .is_err()
        );
        let database = (runtime.callbacks.build_project_database.as_ref().unwrap())(
            StorePath::from("apps/project/storage/db"),
        )
        .execute()
        .await?;
        assert!(database.table_names().execute().await?.is_empty());
        assert!(data.path().join("apps/project/storage/db").is_dir());
        assert!(!revision.path().join("apps/project/storage/db").exists());
        assert!(data.path().join("bits").is_dir());
        for name in ["bits", "user", "tmp", "logs"] {
            assert!(
                !revision.path().join(name).exists(),
                "runtime changed revision/{name}"
            );
        }
        for (store, prefix) in [
            (runtime.stores.user_store.as_ref().unwrap(), "user"),
            (runtime.stores.temporary_store.as_ref().unwrap(), "tmp"),
            (runtime.stores.log_store.as_ref().unwrap(), "logs"),
        ] {
            store
                .as_generic()
                .put(
                    &StorePath::from("entry"),
                    bytes::Bytes::from_static(b"local").into(),
                )
                .await?;
            assert!(data.path().join(prefix).join("entry").exists());
            assert!(!revision.path().join(prefix).join("entry").exists());
        }
        Ok(())
    }

    #[tokio::test]
    async fn native_host_serves_before_ready_and_closes_on_rejected_readiness() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let (mut config, state, _started, starts) = fixture(directory.path()).await;
        validate_rollout_with_state(&config, state.clone()).await?;
        let mut online = config.clone();
        online.source = ProjectSource::Online;
        assert!(
            validate_rollout(&online)
                .await
                .unwrap_err()
                .to_string()
                .contains("rollout_online_preflight_unsupported")
        );
        let app = App::load(config.project_id.clone(), state.clone()).await?;
        let mut event = app.get_event("event", Some((1, 0, 0))).await?;
        event.event_type = "http".into();
        event.config = serde_json::to_vec(&serde_json::json!({
            "method": "POST", "path": "/invoke"
        }))?;
        event.event_version = (2, 0, 0);
        event.save(&app, Some((2, 0, 0))).await?;
        config.events[0].event_version = [2, 0, 0];
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        config.hosting = Some(crate::config::HostingConfig {
            host: address.ip(),
            port: address.port(),
            max_in_flight: 1,
            request_timeout_secs: 5,
            authentication: Default::default(),
            auth_secret: Some("listener".into()),
            ui_origins: Vec::new(),
        });
        let token = "t".repeat(32);
        config
            .secret_overrides
            .insert("credential".into(), "rollout-variable".into());
        assert!(
            validate_rollout_with_state(&config, state.clone())
                .await
                .is_err()
        );
        for invalid in [b"not-json-private".as_slice(), b"123"] {
            crate::secrets::install(&config, "rollout-variable", invalid)?;
            let error = validate_rollout_with_state(&config, state.clone())
                .await
                .unwrap_err();
            assert!(!format!("{error:#}").contains("not-json-private"));
        }
        crate::secrets::install(&config, "rollout-variable", br#""private-value""#)?;
        assert!(
            validate_rollout_with_state(&config, state.clone())
                .await
                .is_err(),
            "Missing service token must fail preflight"
        );
        crate::secrets::install(&config, "listener", b"too-short")?;
        assert!(
            validate_rollout_with_state(&config, state.clone())
                .await
                .is_err(),
            "Invalid service token must fail preflight"
        );
        crate::secrets::install(&config, "listener", token.as_bytes())?;
        validate_rollout_with_state(&config, state.clone()).await?;
        let mut acknowledged = false;
        let error = run_with_state_listener(
            &config,
            state,
            CancellationToken::new(),
            Profile::default(),
            Some(listener),
            None,
            || async {
                let client = reqwest::Client::builder()
                    .no_proxy()
                    .timeout(Duration::from_secs(2))
                    .build()?;
                let response = client
                    .get(format!("http://{address}/services"))
                    .bearer_auth(&token)
                    .send()
                    .await?
                    .error_for_status()?
                    .json::<serde_json::Value>()
                    .await?;
                ensure!(response["project_id"] == "project");
                ensure!(response["events"][0]["id"] == "event");
                acknowledged = true;
                bail!("Supervisor rejected stale placement readiness")
            },
            false,
            None,
            None,
            PersonStartedRuns::default(),
        )
        .await
        .unwrap_err();
        assert!(acknowledged, "Ready must run after the host starts serving");
        assert!(format!("{error:#}").contains("Supervisor rejected"));
        assert_eq!(starts.load(Ordering::SeqCst), 0);
        // Dropping the rejected runtime aborts its host task; yield until the
        // listener descriptor is released, rather than depending on scheduling.
        tokio::time::timeout(Duration::from_secs(2), async {
            while tokio::net::TcpStream::connect(address).await.is_ok() {
                tokio::task::yield_now().await;
            }
        })
        .await?;
        Ok(())
    }

    #[tokio::test]
    async fn pinned_daemon_remains_running_until_cancelled() {
        assert_pinned_daemon_runs(false).await;
    }

    #[tokio::test]
    async fn device_source_definition_runs_on_its_deployment_destination() {
        assert_pinned_daemon_runs(true).await;
    }

    async fn assert_pinned_daemon_runs(device_source: bool) {
        let directory = tempfile::tempdir().unwrap();
        let (mut config, state, started, starts) = fixture(directory.path()).await;
        if device_source {
            let app = App::load(config.project_id.clone(), state.clone())
                .await
                .unwrap();
            let mut event = app.get_event("event", Some((1, 0, 0))).await.unwrap();
            event.set_device_source().unwrap();
            event.event_version = (1, 0, 1);
            event.save(&app, Some((1, 0, 1))).await.unwrap();
            config.events[0].event_version = [1, 0, 1];
        }
        let stop = CancellationToken::new();
        let runtime_stop = stop.clone();
        let (ready_sender, ready_receiver) = oneshot::channel();
        let mut task = tokio::spawn(async move {
            run_with_state_ready(&config, state, runtime_stop, Profile::default(), || async {
                let _ = ready_sender.send(());
                Ok(())
            })
            .await
        });
        tokio::time::timeout(Duration::from_secs(10), ready_receiver)
            .await
            .unwrap()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(10), started.notified())
            .await
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut task)
                .await
                .is_err()
        );
        assert_eq!(starts.load(Ordering::SeqCst), 1);
        stop.cancel();
        tokio::time::timeout(Duration::from_secs(10), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(starts.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn authorization_revocation_stops_a_ready_service_without_further_resource_reads()
    -> Result<()> {
        let directory = tempfile::tempdir()?;
        let (config, state, _, _) = fixture(directory.path()).await;
        let stop = CancellationToken::new();
        let revoked = CancellationToken::new();
        let revoke = revoked.clone();
        let (ready, receiver) = oneshot::channel();
        let task = tokio::spawn(async move {
            until_authorization_revoked(
                revoked,
                stop.clone(),
                run_with_state_ready(&config, state, stop, Profile::default(), || async {
                    let _ = ready.send(());
                    Ok(())
                }),
            )
            .await
        });
        tokio::time::timeout(Duration::from_secs(10), receiver).await??;
        revoke.cancel();
        let error = tokio::time::timeout(Duration::from_secs(10), task)
            .await??
            .unwrap_err();
        assert!(matches!(
            error.downcast_ref::<flow_like_types::authorization::AuthorizationError>(),
            Some(flow_like_types::authorization::AuthorizationError::Denied)
        ));
        Ok(())
    }

    #[tokio::test]
    async fn daemon_readiness_is_explicit_and_cancellation_does_not_acknowledge_startup()
    -> Result<()> {
        let directory = tempfile::tempdir()?;
        let (mut config, state, started, _) = fixture(directory.path()).await;
        let app = App::load(config.project_id.clone(), state.clone()).await?;
        let mut event = app.get_event("event", Some((1, 0, 0))).await?;
        let mut board = app
            .open_board(event.board_id.clone(), Some(false), event.board_version)
            .await?
            .snapshot()
            .as_ref()
            .clone();
        assert_eq!(
            service_readiness_source(&event, &board)?.unwrap().1,
            ServiceReadyKind::Daemon
        );
        let ready = board.nodes.remove("ready").unwrap();
        assert!(
            service_readiness_source(&event, &board)
                .unwrap_err()
                .to_string()
                .contains("service_ready")
        );
        board.nodes.insert("ready".into(), ready.clone());
        let mut duplicate = ready;
        duplicate.id = "duplicate".into();
        board.nodes.insert("duplicate".into(), duplicate);
        assert!(
            service_readiness_source(&event, &board)
                .unwrap_err()
                .to_string()
                .contains("multiple")
        );
        // Starting downstream of Service Ready must not report preflight as healthy.
        event.node_id = "entry".into();
        event.event_version = (2, 0, 0);
        event.save(&app, Some((2, 0, 0))).await?;
        config.events[0].event_version = [2, 0, 0];
        let stop = CancellationToken::new();
        let child_stop = stop.clone();
        let (ready, mut receiver) = oneshot::channel();
        let task = tokio::spawn(async move {
            run_with_state_ready(&config, state, child_stop, Profile::default(), || async {
                let _ = ready.send(());
                Ok(())
            })
            .await
        });
        tokio::time::timeout(Duration::from_secs(10), started.notified()).await?;
        assert!(matches!(
            receiver.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        stop.cancel();
        tokio::time::timeout(Duration::from_secs(10), task).await???;
        assert!(receiver.await.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn rest_and_mcp_bind_failures_never_acknowledge_readiness() -> Result<()> {
        for kind in ["rest", "mcp"] {
            let directory = tempfile::tempdir()?;
            let (mut config, state, _, _) = fixture(directory.path()).await;
            let occupied = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
            let app = App::load(config.project_id.clone(), state.clone()).await?;
            let mut board = Board::new(
                Some("listener-board".into()),
                StorePath::from("apps/project"),
                state.clone(),
            );
            {
                let registry = state.node_registry.read().await;
                for (name, id) in [
                    (format!("{kind}_server_config"), "config"),
                    (format!("{kind}_server"), "server"),
                ] {
                    let definition = registry.get_node(&name).unwrap();
                    let mut node = registry.instantiate(&definition)?.get_node();
                    node.id = id.into();
                    if id == "config" {
                        node.get_pin_mut_by_name("host")
                            .unwrap()
                            .set_default_value(Some(serde_json::json!("127.0.0.1")));
                        node.get_pin_mut_by_name("port")
                            .unwrap()
                            .set_default_value(Some(serde_json::json!(
                                occupied.local_addr()?.port()
                            )));
                    }
                    board.nodes.insert(id.into(), node);
                }
            }
            connect(&mut board, "config", "config", "server", "config");
            board.snapshot_at_version((1, 0, 0), None).await?;
            let mut event = app.get_event("event", Some((1, 0, 0))).await?;
            event.board_id = board.id;
            event.node_id = "server".into();
            event.event_type = kind.into();
            event.event_version = (2, 0, 0);
            event.save(&app, Some((2, 0, 0))).await?;
            config.events[0].event_version = [2, 0, 0];
            validate_rollout_with_state(&config, state.clone()).await?;
            let result = tokio::time::timeout(
                Duration::from_secs(10),
                run_with_state_ready(
                    &config,
                    state,
                    CancellationToken::new(),
                    Profile::default(),
                    || async { panic!("failed bind cannot report ready") },
                ),
            )
            .await?;
            assert!(
                result.is_err(),
                "{kind} exited successfully after a failed bind"
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn current_event_pin_runs_without_an_archive_and_rejects_a_newer_live_version() {
        let directory = tempfile::tempdir().unwrap();
        let (mut config, state, started, starts) = fixture(directory.path()).await;
        let app = App::load(config.project_id.clone(), state.clone())
            .await
            .unwrap();
        let mut event = app.get_event("event", Some((1, 0, 0))).await.unwrap();
        event.event_version = (1, 0, 1);
        event.save(&app, None).await.unwrap();
        config.events[0].event_version = [1, 0, 1];
        let stop = CancellationToken::new();
        let _stop_on_drop = stop.clone().drop_guard();
        let runtime_stop = stop.clone();
        let runtime_config = config.clone();
        let runtime_state = state.clone();
        let task = tokio::spawn(async move {
            run_with_state(&runtime_config, runtime_state, runtime_stop).await
        });
        tokio::time::timeout(Duration::from_secs(10), started.notified())
            .await
            .unwrap();
        assert_eq!(starts.load(Ordering::SeqCst), 1);
        stop.cancel();
        tokio::time::timeout(Duration::from_secs(10), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();

        // An incomplete source with neither the selected current nor archived version
        // must not silently run the replacement event.
        event.event_version = (1, 0, 2);
        event.save(&app, None).await.unwrap();
        assert!(
            run_with_state(&config, state, CancellationToken::new())
                .await
                .is_err()
        );
        assert_eq!(starts.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn mismatched_pin_prevents_any_service_start() {
        let directory = tempfile::tempdir().unwrap();
        let (mut config, state, _started, starts) = fixture(directory.path()).await;
        config.events[0].board_version = [2, 0, 0];
        let mut ready = false;
        let error = run_with_state_ready(
            &config,
            state,
            CancellationToken::new(),
            Profile::default(),
            || async {
                ready = true;
                Ok(())
            },
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("does not pin"));
        assert!(!ready);
        assert_eq!(starts.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn unknown_placement_variable_prevents_any_service_start() {
        let directory = tempfile::tempdir().unwrap();
        let (mut config, state, _started, starts) = fixture(directory.path()).await;
        config
            .variables
            .insert("missing-variable".into(), serde_json::json!("secret"));
        let error = run_with_state(&config, state, CancellationToken::new())
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("absent from the selected pinned boards")
        );
        assert_eq!(starts.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn plaintext_secret_override_is_rejected_without_echoing_value() {
        let directory = tempfile::tempdir().unwrap();
        let (mut config, state, _started, starts) = fixture(directory.path()).await;
        config
            .variables
            .insert("credential".into(), serde_json::json!("private-test-value"));
        let error = run_with_state(&config, state, CancellationToken::new())
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("matching secret or plaintext override type")
        );
        assert!(!error.to_string().contains("private-test-value"));
        assert_eq!(starts.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn private_secret_is_required_on_start_and_keeps_runtime_sensitivity() {
        let directory = tempfile::tempdir().unwrap();
        let (mut config, state, _started, starts) = fixture(directory.path()).await;
        config
            .secret_overrides
            .insert("credential".into(), "service-credential".into());

        // Staged intent can precede provisioning. Starting cannot.
        let staged = CancellationToken::new();
        staged.cancel();
        run_with_state(&config, state.clone(), staged)
            .await
            .unwrap();
        assert!(
            run_with_state(&config, state.clone(), CancellationToken::new())
                .await
                .is_err()
        );
        for invalid in [b"private-test-value".as_slice(), b"123"] {
            crate::secrets::install(&config, "service-credential", invalid).unwrap();
            let error = run_with_state(&config, state.clone(), CancellationToken::new())
                .await
                .unwrap_err();
            assert!(!format!("{error:#}").contains("private-test-value"));
            assert_eq!(starts.load(Ordering::SeqCst), 0);
        }

        crate::secrets::install(&config, "service-credential", br#""private-test-value""#).unwrap();
        let (sender, receiver) = oneshot::channel();
        state
            .node_registry
            .write()
            .await
            .push_node(Arc::new(SecretProbe(std::sync::Mutex::new(Some(sender)))));
        let stop = CancellationToken::new();
        let _stop_on_drop = stop.clone().drop_guard();
        let runtime_stop = stop.clone();
        let task = tokio::spawn(async move { run_with_state(&config, state, runtime_stop).await });
        let (value, sensitive) = tokio::time::timeout(Duration::from_secs(10), receiver)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(value, serde_json::json!("private-test-value"));
        assert!(sensitive, "secret injection must retain log filtering");
        stop.cancel();
        tokio::time::timeout(Duration::from_secs(10), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn environment_template_uses_pinned_variable_contract_and_private_secret_files() {
        let directory = tempfile::tempdir().unwrap();
        let (mut config, _state, _started, _starts) = fixture(directory.path()).await;
        let template = directory.path().join("variables.env.example");
        crate::environment::export(&config, &template)
            .await
            .unwrap();
        let exported = std::fs::read_to_string(&template).unwrap();
        let key = exported
            .lines()
            .find_map(|line| {
                line.strip_prefix("# FLOW_LIKE_VAR_")
                    .and_then(|line| line.split_once('='))
            })
            .map(|(suffix, _)| format!("FLOW_LIKE_VAR_{suffix}"))
            .unwrap();
        assert!(exported.contains("credential"));
        assert!(!exported.contains("private-test-value"));
        let input = directory.path().join("variables.env");
        crate::vault::write_new_private(
            &input,
            format!("{key}='\"private-test-value-${{HOME}}\"'\n").as_bytes(),
        )
        .unwrap();
        crate::environment::apply(&mut config, &input)
            .await
            .unwrap();
        assert!(config.variables.is_empty());
        let secret = config.secret_overrides.get("credential").unwrap();
        let bytes = crate::vault::read_private(
            &crate::config::private_secret_path(&config, secret).unwrap(),
        )
        .unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(),
            "private-test-value-${HOME}"
        );
        assert!(
            !serde_json::to_string(&config)
                .unwrap()
                .contains("private-test-value")
        );
        let invalid = directory.path().join("foreign.env");
        crate::vault::write_new_private(&invalid, b"FLOW_LIKE_VAR_00_00='\"private-test-value\"'")
            .unwrap();
        let before = config.clone();
        let error = crate::environment::apply(&mut config, &invalid)
            .await
            .unwrap_err();
        assert!(!format!("{error:#}").contains("private-test-value"));
        assert_eq!(config, before);
    }

    #[tokio::test]
    async fn rest_catalog_binds_with_placement_variables_and_stops_on_cancel() {
        let directory = tempfile::tempdir().unwrap();
        let (mut config, state, _started, _starts) = fixture(directory.path()).await;
        let app = App::load(config.project_id.clone(), state.clone())
            .await
            .unwrap();
        let mut board = Board::new(
            Some("rest-board".into()),
            StorePath::from("apps/project"),
            state.clone(),
        );
        for (id, data_type, default) in [
            (
                "rest-host",
                VariableType::String,
                serde_json::json!("invalid-bind-host.invalid"),
            ),
            ("rest-port", VariableType::Integer, serde_json::json!(65535)),
        ] {
            let mut variable = Variable::new(id, data_type.clone(), ValueType::Normal);
            variable.id = id.into();
            variable.exposed = true;
            variable.set_default_value(default);
            board.variables.insert(id.into(), variable);
            let registry = state.node_registry.read().await;
            let definition = registry.get_node("variable_get").unwrap();
            let mut node = registry.instantiate(&definition).unwrap().get_node();
            node.id = format!("get-{id}");
            node.get_pin_mut_by_name("var_ref")
                .unwrap()
                .set_default_value(Some(serde_json::json!(id)));
            node.get_pin_mut_by_name("value_ref").unwrap().data_type = data_type;
            board.nodes.insert(node.id.clone(), node);
        }
        for (catalog_name, id) in [("rest_server_config", "config"), ("rest_server", "server")] {
            let registry = state.node_registry.read().await;
            let definition = registry.get_node(catalog_name).unwrap();
            let mut node = registry.instantiate(&definition).unwrap().get_node();
            node.id = id.into();
            board.nodes.insert(node.id.clone(), node);
        }
        let (sender, receiver) = oneshot::channel();
        let probe = Arc::new(ListeningProbe(std::sync::Mutex::new(Some(sender))));
        let mut node = probe.get_node();
        node.id = "listening".into();
        board.nodes.insert(node.id.clone(), node);
        state.node_registry.write().await.push_node(probe);
        connect(&mut board, "get-rest-host", "value_ref", "config", "host");
        connect(&mut board, "get-rest-port", "value_ref", "config", "port");
        connect(&mut board, "config", "config", "server", "config");
        connect(&mut board, "server", "local_addr", "listening", "address");
        connect(&mut board, "server", "on_listening", "listening", "exec");
        board.snapshot_at_version((1, 0, 0), None).await.unwrap();
        let mut event = app.get_event("event", Some((1, 0, 0))).await.unwrap();
        event.board_id = board.id;
        event.node_id = "server".into();
        event.event_type = "rest".into();
        event.event_version = (2, 0, 0);
        event.save(&app, Some((2, 0, 0))).await.unwrap();
        config.events[0].event_version = [2, 0, 0];
        config
            .variables
            .insert("rest-host".into(), serde_json::json!("127.0.0.1"));
        config
            .variables
            .insert("rest-port".into(), serde_json::json!(0));

        let mut invalid = config.clone();
        invalid
            .variables
            .insert("rest-port".into(), serde_json::json!("not-a-port"));
        let error = run_with_state_ready(
            &invalid,
            state.clone(),
            CancellationToken::new(),
            Profile::default(),
            || async { panic!("invalid port must fail before readiness") },
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("rest-port"));
        assert!(
            error
                .to_string()
                .contains("does not satisfy its pinned schema")
        );

        let stop = CancellationToken::new();
        let _stop_on_drop = stop.clone().drop_guard();
        let runtime_stop = stop.clone();
        validate_rollout_with_state(&config, state.clone())
            .await
            .unwrap();
        let (ready_sender, ready_receiver) = oneshot::channel();
        let task = tokio::spawn(async move {
            run_with_state_ready(&config, state, runtime_stop, Profile::default(), || async {
                ready_sender.send(()).unwrap();
                Ok(())
            })
            .await
        });
        tokio::time::timeout(Duration::from_secs(10), ready_receiver)
            .await
            .unwrap()
            .unwrap();
        let address: std::net::SocketAddr = tokio::time::timeout(Duration::from_secs(10), receiver)
            .await
            .unwrap()
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(address.ip(), std::net::Ipv4Addr::LOCALHOST);
        assert_ne!(address.port(), 0);
        let response = tokio::time::timeout(Duration::from_secs(5), async {
            let mut client = tokio::net::TcpStream::connect(address).await.unwrap();
            client
                .write_all(b"GET /missing HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                .await
                .unwrap();
            let mut response = String::new();
            client.read_to_string(&mut response).await.unwrap();
            response
        })
        .await
        .unwrap();
        assert!(response.starts_with("HTTP/1.1 404"), "{response}");
        assert!(response.contains("Not Found"));
        assert!(
            !task.is_finished(),
            "REST listener exited after serving one request"
        );
        stop.cancel();
        tokio::time::timeout(Duration::from_secs(10), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(tokio::net::TcpStream::connect(address).await.is_err());
    }

    #[test]
    fn override_schema_resolves_board_refs_and_enforces_nested_local_refs() {
        let mut variable = Variable::new("Settings", VariableType::Struct, ValueType::Normal);
        variable.schema = Some("settings-schema".into());
        let refs = HashMap::from([(
            "settings-schema".into(),
            serde_json::json!({
                "type": "object",
                "$defs": {"limit": {"type": "integer", "minimum": 1, "maximum": 3}},
                "properties": {"limit": {"$ref": "#/$defs/limit"}},
                "required": ["limit"],
                "additionalProperties": false
            })
            .to_string(),
        )]);
        assert!(validate_override(&variable, &refs, &serde_json::json!({"limit": 2})).is_ok());
        assert!(validate_override(&variable, &refs, &serde_json::json!({"limit": 0})).is_err());
        assert!(validate_override(&variable, &refs, &serde_json::json!({"limit": "2"})).is_err());
        assert!(
            validate_override(
                &variable,
                &refs,
                &serde_json::json!({"limit": 2, "extra": true})
            )
            .is_err()
        );
        variable.value_type = ValueType::Array;
        assert!(validate_override(&variable, &refs, &serde_json::json!([{"limit": 2}])).is_ok());
        assert!(validate_override(&variable, &refs, &serde_json::json!([{"limit": 0}])).is_err());
    }

    #[test]
    fn override_schemas_cannot_retrieve_external_definitions() {
        let mut variable = Variable::new("Settings", VariableType::Struct, ValueType::Normal);
        for reference in ["https://example.invalid/schema", "file:///etc/passwd"] {
            variable.schema = Some(serde_json::json!({"$ref": reference}).to_string());
            assert!(validate_override(&variable, &HashMap::new(), &serde_json::json!({})).is_err());
        }
    }

    #[test]
    fn override_collections_preserve_declared_types_and_set_uniqueness() {
        let mut variable = Variable::new("Ports", VariableType::Integer, ValueType::Array);
        let refs = HashMap::new();
        assert!(validate_override(&variable, &refs, &serde_json::json!([80, 443])).is_ok());
        assert!(validate_override(&variable, &refs, &serde_json::json!([80, "443"])).is_err());
        assert!(validate_override(&variable, &refs, &serde_json::json!(80)).is_err());
        variable.value_type = ValueType::HashSet;
        assert!(validate_override(&variable, &refs, &serde_json::json!([80, 80])).is_err());
        variable.value_type = ValueType::HashMap;
        assert!(validate_override(&variable, &refs, &serde_json::json!({"http": 80})).is_ok());
        assert!(validate_override(&variable, &refs, &serde_json::json!({"http": "80"})).is_err());
        variable.value_type = ValueType::Normal;
        variable.data_type = VariableType::Execution;
        assert!(validate_override(&variable, &refs, &serde_json::Value::Null).is_err());
    }

    #[derive(Default)]
    struct ScheduledProbe {
        runs: AtomicUsize,
        ran: Notify,
        payloads: std::sync::Mutex<Vec<Option<serde_json::Value>>>,
    }

    #[flow_like_types::async_trait]
    impl NodeLogic for ScheduledProbe {
        fn get_node(&self) -> Node {
            let mut node = Node::new("standalone_test_schedule", "Schedule", "", "Tests");
            node.set_start(true);
            node.add_input_pin("exec_in", "Execute", "", VariableType::Execution);
            node
        }

        async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
            let payload = context.get_payload().await?.payload.clone();
            self.payloads.lock().unwrap().push(payload);
            self.runs.fetch_add(1, Ordering::SeqCst);
            self.ran.notify_one();
            Ok(())
        }
    }

    pub(crate) struct TestClock(std::sync::atomic::AtomicI64);

    impl TestClock {
        pub(crate) fn at(time: &str) -> Arc<Self> {
            let clock = Arc::new(Self(Default::default()));
            clock.set(time);
            clock
        }

        pub(crate) fn set(&self, time: &str) {
            let seconds = time
                .parse::<chrono::DateTime<chrono::Utc>>()
                .unwrap()
                .timestamp();
            self.0.store(seconds * 1000, Ordering::SeqCst);
        }
    }

    impl crate::schedule::Clock for TestClock {
        fn now_ms(&self) -> i64 {
            self.0.load(Ordering::SeqCst)
        }
    }

    /// Records the claim calls of a placement; every schedule asked for is claimed.
    #[derive(Default)]
    pub(crate) struct RecordingHub(pub(crate) std::sync::Mutex<Vec<Vec<String>>>);

    #[flow_like_types::async_trait]
    impl crate::schedule::ScheduleClaims for RecordingHub {
        async fn claim(
            &self,
            ids: &[String],
        ) -> std::result::Result<crate::schedule::ClaimOutcome, crate::schedule::ClaimError>
        {
            self.0.lock().unwrap().push(ids.to_vec());
            Ok(crate::schedule::ClaimOutcome {
                claimed: ids.iter().map(|id| (id.clone(), 0)).collect(),
                held: Vec::new(),
            })
        }
    }

    pub(crate) fn schedule_context(
        root: &Path,
        clock: Arc<TestClock>,
        claims: Option<Arc<RecordingHub>>,
    ) -> crate::schedule::ScheduleContext {
        crate::schedule::ScheduleContext {
            state_dir: root.join(".standalone-schedule").join("placement"),
            placement_id: "placement".into(),
            project_id: "project".into(),
            config_revision: 1,
            intent_revision: 1,
            grant_id: claims.as_ref().map(|_| "grant".into()),
            arbiter: None,
            claims: claims.map(|claims| claims as Arc<dyn crate::schedule::ScheduleClaims>),
            clock,
        }
    }

    fn schedule_state(root: &Path) -> serde_json::Value {
        std::fs::read(root.join(".standalone-schedule/placement/state.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub(crate) async fn until(what: &str, condition: impl Fn() -> bool) {
        tokio::time::timeout(Duration::from_secs(15), async {
            while !condition() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("Timed out waiting until {what}"));
    }

    /// Starts the placement as a supervised process does, with `context` for its
    /// schedules and bots. `on_ready` stands for the supervisor accepting the service.
    pub(crate) fn start_supervised<F>(
        config: PlacementConfig,
        state: Arc<FlowLikeState>,
        context: crate::schedule::ScheduleContext,
        on_ready: impl FnOnce() -> F + Send + 'static,
    ) -> (CancellationToken, tokio::task::JoinHandle<Result<()>>)
    where
        F: Future<Output = Result<()>> + Send + 'static,
    {
        start_supervised_with(
            config,
            state,
            context,
            PersonStartedRuns::default(),
            on_ready,
        )
    }

    /// `start_supervised` with the agent parent's queue of person-started runs.
    pub(crate) fn start_supervised_with<F>(
        config: PlacementConfig,
        state: Arc<FlowLikeState>,
        context: crate::schedule::ScheduleContext,
        runs: PersonStartedRuns,
        on_ready: impl FnOnce() -> F + Send + 'static,
    ) -> (CancellationToken, tokio::task::JoinHandle<Result<()>>)
    where
        F: Future<Output = Result<()>> + Send + 'static,
    {
        let stop = CancellationToken::new();
        let runtime_stop = stop.clone();
        let task = tokio::spawn(async move {
            run_with_state_listener(
                &config,
                state,
                runtime_stop,
                Profile::default(),
                None,
                None,
                on_ready,
                false,
                None,
                Some(context),
                runs,
            )
            .await
        });
        (stop, task)
    }

    /// The fixture with a second event `report`: a `cron` event on a flow of its own whose
    /// start node returns at once. The placement lists the daemon and the schedule.
    async fn scheduled_fixture(
        root: &Path,
        schedule: serde_json::Value,
    ) -> (PlacementConfig, Arc<FlowLikeState>, Arc<ScheduledProbe>) {
        let (mut config, state, _, _) = fixture(root).await;
        let probe = Arc::new(ScheduledProbe::default());
        state.node_registry.write().await.push_node(probe.clone());
        let app = App::load(config.project_id.clone(), state.clone())
            .await
            .unwrap();
        let mut board = Board::new(
            Some("schedule-board".into()),
            StorePath::from("apps/project"),
            state.clone(),
        );
        let mut node = probe.get_node();
        node.id = "tick".into();
        board.nodes.insert(node.id.clone(), node);
        board.snapshot_at_version((1, 0, 0), None).await.unwrap();
        let mut event = app.get_event("event", Some((1, 0, 0))).await.unwrap();
        event.id = "report".into();
        event.board_id = board.id;
        event.node_id = "tick".into();
        event.event_type = "cron".into();
        event.config = serde_json::to_vec(&schedule).unwrap();
        event.save(&app, Some((1, 0, 0))).await.unwrap();
        config.events.push(EventBinding {
            event_id: "report".into(),
            event_version: [1, 0, 0],
            board_version: [1, 0, 0],
        });
        (config, state, probe)
    }

    #[tokio::test]
    async fn validation_refuses_a_schedule_a_device_cannot_run() -> Result<()> {
        for (schedule, problem) in [
            (
                serde_json::json!({"expression": "0 0 9 ? * *"}),
                "day-of-month",
            ),
            (
                serde_json::json!({"expression": "0 9 * * *", "timezone": "Mars/Olympus"}),
                "Mars/Olympus",
            ),
            (
                serde_json::json!({"expression": "*/30 * * * * *"}),
                "more often than once a minute",
            ),
            (serde_json::json!({}), "no cron expression"),
        ] {
            let directory = tempfile::tempdir()?;
            let (config, state, probe) = scheduled_fixture(directory.path(), schedule).await;
            let staged = CancellationToken::new();
            staged.cancel();
            for error in [
                run_with_state(&config, state.clone(), staged).await,
                validate_rollout_with_state(&config, state.clone()).await,
                run_with_state(&config, state, CancellationToken::new()).await,
            ] {
                let error = format!("{:#}", error.unwrap_err());
                assert!(
                    error.contains("schedule of event report") && error.contains(problem),
                    "{error}"
                );
            }
            assert_eq!(probe.runs.load(Ordering::SeqCst), 0);
            assert!(!directory.path().join(".standalone-schedule").exists());
        }
        Ok(())
    }

    #[tokio::test]
    async fn a_schedule_validates_without_state_and_starts_only_supervised() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let schedule =
            serde_json::json!({"expression": "0 0 2 * * *", "timezone": "Europe/Berlin"});
        let (mut config, state, _) = scheduled_fixture(directory.path(), schedule).await;
        validate_rollout_with_state(&config, state.clone()).await?;
        // Validation parses only; it keeps no state and needs no supervised runtime.
        assert!(!directory.path().join(".standalone-schedule").exists());
        // A real start of a schedule does: the plain `run` has no data root for a watermark.
        let error = run_with_state(&config, state.clone(), CancellationToken::new())
            .await
            .unwrap_err();
        assert!(
            format!("{error:#}").contains("schedules need the supervised placement runtime"),
            "{error:#}"
        );
        // Each replica is a process of its own and would run every time once.
        config.max_replicas = 2;
        let error = validate_rollout_with_state(&config, state)
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("scheduled and daemon triggers cannot be replicated")
        );
        Ok(())
    }

    #[tokio::test]
    async fn a_schedule_only_placement_is_ready_without_a_listener_and_runs_its_times() -> Result<()>
    {
        let directory = tempfile::tempdir()?;
        let schedule = serde_json::json!({"expression": "* * * * *", "payload": {"region": "eu"}});
        let (mut config, state, probe) = scheduled_fixture(directory.path(), schedule).await;
        config.events.remove(0);
        let clock = TestClock::at("2026-09-01T10:00:59Z");
        let context = schedule_context(directory.path(), clock.clone(), None);
        let (ready, is_ready) = oneshot::channel();
        let (accept, accepted) = oneshot::channel::<()>();
        let (stop, task) = start_supervised(config, state, context, || async move {
            let _ = ready.send(());
            let _ = accepted.await;
            Ok(())
        });
        tokio::time::timeout(Duration::from_secs(10), is_ready).await??;
        // The supervisor has not accepted the service yet: nothing is decided or armed.
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(schedule_state(directory.path())["decided"], false);
        accept.send(()).unwrap();
        until("the schedule is armed", || {
            schedule_state(directory.path())["decided"] == true
        })
        .await;
        let file = schedule_state(directory.path());
        assert_eq!(file["events"]["report"]["hold"], serde_json::Value::Null);
        assert_eq!(file["events"]["report"]["expression"], "* * * * *");
        assert_eq!(
            probe.runs.load(Ordering::SeqCst),
            0,
            "nothing runs at start"
        );
        assert!(!task.is_finished());

        clock.set("2026-09-01T10:01:00Z");
        tokio::time::timeout(Duration::from_secs(15), probe.ran.notified()).await?;
        assert_eq!(
            *probe.payloads.lock().unwrap(),
            [Some(serde_json::json!({"region": "eu"}))]
        );
        until("the run is recorded", || {
            schedule_state(directory.path())["events"]["report"]["last"]["outcome"] == "succeeded"
        })
        .await;
        assert!(
            !task.is_finished(),
            "a run that ended is not a service that exited"
        );
        stop.cancel();
        tokio::time::timeout(Duration::from_secs(10), task).await???;
        let entry = &schedule_state(directory.path())["events"]["report"];
        assert_eq!((&entry["runs"], &entry["failed"]), (&1.into(), &0.into()));
        assert_eq!(probe.runs.load(Ordering::SeqCst), 1);
        Ok(())
    }

    #[tokio::test]
    async fn a_schedule_next_to_a_service_arms_after_the_service_is_ready() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let schedule = serde_json::json!({"expression": "0 0 1 1 *"});
        let (config, state, probe) = scheduled_fixture(directory.path(), schedule).await;
        let hub = Arc::new(RecordingHub::default());
        let context = schedule_context(
            directory.path(),
            TestClock::at("2026-09-01T10:00:00Z"),
            Some(hub.clone()),
        );
        let (ready, is_ready) = oneshot::channel();
        let asked_before_ready = hub.clone();
        let (stop, task) = start_supervised(config, state, context, || async move {
            // The daemon reported Service Ready; the hub was not asked before.
            let asked = asked_before_ready.0.lock().unwrap().len();
            let _ = ready.send(asked);
            Ok(())
        });
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(10), is_ready).await??,
            0
        );
        until("the schedule is claimed", || {
            schedule_state(directory.path())["decided"] == true
        })
        .await;
        assert_eq!(*hub.0.lock().unwrap(), [["report"]]);
        let entry = &schedule_state(directory.path())["events"]["report"];
        assert_eq!(entry["confirmed"], true);
        assert_eq!(entry["timezone"], "UTC");
        assert_eq!(probe.runs.load(Ordering::SeqCst), 0);
        stop.cancel();
        tokio::time::timeout(Duration::from_secs(20), task).await???;
        Ok(())
    }

    #[tokio::test]
    async fn a_placement_without_schedules_tells_the_hub_once_and_forgets_old_state() -> Result<()>
    {
        let directory = tempfile::tempdir()?;
        let (config, state, started, _) = fixture(directory.path()).await;
        let stale = directory.path().join(".standalone-schedule/placement");
        std::fs::create_dir_all(&stale)?;
        std::fs::write(stale.join("state.json"), b"{}")?;
        let hub = Arc::new(RecordingHub::default());
        let context = schedule_context(
            directory.path(),
            TestClock::at("2026-09-01T10:00:00Z"),
            Some(hub.clone()),
        );
        let (stop, task) = start_supervised(config, state, context, || async { Ok(()) });
        tokio::time::timeout(Duration::from_secs(10), started.notified()).await?;
        until("the hub was told", || !hub.0.lock().unwrap().is_empty()).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(*hub.0.lock().unwrap(), [Vec::<String>::new()]);
        assert!(!stale.join("state.json").exists());
        assert!(
            !task.is_finished(),
            "the hand-back is not a service that exited"
        );
        stop.cancel();
        tokio::time::timeout(Duration::from_secs(20), task).await???;
        Ok(())
    }

    #[tokio::test]
    async fn a_cron_event_with_a_page_stays_a_served_page() -> Result<()> {
        let directory = tempfile::tempdir()?;
        // Not a readable schedule: a served Page never has its schedule parsed.
        let schedule = serde_json::json!({"expression": "every now and then"});
        let (mut config, state, probe) = scheduled_fixture(directory.path(), schedule).await;
        config.events.remove(0);
        let app = App::load(config.project_id.clone(), state.clone()).await?;
        let mut event = app.get_event("report", Some((1, 0, 0))).await?;
        event.default_page_id = Some("page".into());
        event.event_version = (2, 0, 0);
        event.save(&app, Some((2, 0, 0))).await?;
        config.events[0].event_version = [2, 0, 0];
        let staged = CancellationToken::new();
        staged.cancel();
        run_with_state(&config, state.clone(), staged).await?;
        // It needs the native host like every Page, also with more than one replica.
        let error = validate_rollout_with_state(&config, state.clone())
            .await
            .unwrap_err();
        assert!(
            format!("{error:#}").contains("hosting configuration"),
            "{error:#}"
        );
        config.max_replicas = 2;
        let staged = CancellationToken::new();
        staged.cancel();
        run_with_state(&config, state.clone(), staged).await?;
        config.max_replicas = 1;
        // A start with a schedule context starts no scheduler for it: it fails on the
        // missing listener, and the context keeps no state.
        let context = schedule_context(
            directory.path(),
            TestClock::at("2026-09-01T10:00:00Z"),
            None,
        );
        let error = run_with_state_listener(
            &config,
            state,
            CancellationToken::new(),
            Profile::default(),
            None,
            None,
            || async { panic!("a Page without a listener cannot be ready") },
            false,
            None,
            Some(context),
            PersonStartedRuns::default(),
        )
        .await
        .unwrap_err();
        assert!(!format!("{error:#}").contains("schedule"), "{error:#}");
        assert!(!directory.path().join(".standalone-schedule").exists());
        assert_eq!(probe.runs.load(Ordering::SeqCst), 0);
        Ok(())
    }

    /// The fixture's daemon, pinned as event version 2.0.0 to a board that also declares
    /// `variable`.
    async fn declaring(
        root: &Path,
        variable: Variable,
    ) -> (PlacementConfig, Arc<FlowLikeState>, Arc<AtomicUsize>) {
        let (mut config, state, _, starts) = fixture(root).await;
        let app = App::load(config.project_id.clone(), state.clone())
            .await
            .unwrap();
        let source = app
            .open_board("board".into(), Some(false), Some((1, 0, 0)))
            .await
            .unwrap()
            .snapshot();
        let mut board = Board::new(
            Some("declaring-board".into()),
            StorePath::from("apps/project"),
            state.clone(),
        );
        board.nodes = source.nodes.clone();
        board.variables = source.variables.clone();
        board.variables.insert(variable.id.clone(), variable);
        board.snapshot_at_version((1, 0, 0), None).await.unwrap();
        let mut event = app.get_event("event", Some((1, 0, 0))).await.unwrap();
        event.board_id = board.id;
        event.event_version = (2, 0, 0);
        event.save(&app, Some((2, 0, 0))).await.unwrap();
        config.events[0].event_version = [2, 0, 0];
        (config, state, starts)
    }

    #[tokio::test]
    async fn a_flow_variable_never_takes_a_bot_token_key() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let mut reserved = Variable::new("Token", VariableType::String, ValueType::Normal);
        reserved.id = crate::event_kind::bot_token_key("event");
        reserved.secret = true;
        reserved.exposed = true;
        let (mut config, state, starts) = declaring(directory.path(), reserved.clone()).await;
        config
            .secret_overrides
            .insert(reserved.id.clone(), "bot-token".into());
        crate::secrets::install(&config, "bot-token", br#""123456:private-bot-token""#)?;
        let staged = CancellationToken::new();
        staged.cancel();
        for result in [
            run_with_state(&config, state.clone(), staged.clone()).await,
            validate_rollout_with_state(&config, state.clone()).await,
            run_with_state(&config, state.clone(), CancellationToken::new()).await,
        ] {
            let error = format!("{:#}", result.unwrap_err());
            assert!(
                error.contains("variable id event.event.bot_token is reserved"),
                "{error}"
            );
            assert!(!error.contains("private-bot-token"), "{error}");
        }
        // Without the key the flow is refused all the same: a later deploy could add it.
        config.secret_overrides.clear();
        let error = run_with_state(&config, state, staged).await.unwrap_err();
        assert!(error.to_string().contains("is reserved"), "{error:#}");
        assert_eq!(starts.load(Ordering::SeqCst), 0);
        Ok(())
    }

    #[tokio::test]
    async fn a_bot_token_key_of_an_event_that_is_no_bot_is_refused() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let (mut config, state, _, starts) = fixture(directory.path()).await;
        config.secret_overrides.insert(
            crate::event_kind::bot_token_key("event"),
            "bot-token".into(),
        );
        let staged = CancellationToken::new();
        staged.cancel();
        let error = format!(
            "{:#}",
            run_with_state(&config, state, staged).await.unwrap_err()
        );
        assert!(error.contains("event.event.bot_token"), "{error}");
        assert_eq!(starts.load(Ordering::SeqCst), 0);
        Ok(())
    }

    #[tokio::test]
    async fn a_service_holds_at_most_64_schedules_and_bots() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let schedule = serde_json::json!({"expression": "0 0 1 1 *"});
        let (mut config, state, _) = scheduled_fixture(directory.path(), schedule).await;
        let app = App::load(config.project_id.clone(), state.clone()).await?;
        let report = app.get_event("report", Some((1, 0, 0))).await?;
        for index in 1..=64 {
            let mut event = report.clone();
            event.id = format!("report-{index}");
            event.save(&app, Some((1, 0, 0))).await?;
        }
        let bind = |config: &mut PlacementConfig, index: usize| {
            config.events.push(EventBinding {
                event_id: format!("report-{index}"),
                event_version: [1, 0, 0],
                board_version: [1, 0, 0],
            })
        };
        (1..64).for_each(|index| bind(&mut config, index));
        let staged = CancellationToken::new();
        staged.cancel();
        run_with_state(&config, state.clone(), staged.clone()).await?;
        bind(&mut config, 64);
        for result in [
            run_with_state(&config, state.clone(), staged).await,
            validate_rollout_with_state(&config, state).await,
        ] {
            let error = format!("{:#}", result.unwrap_err());
            assert!(
                error.contains("at most 64 schedules and bots, got 65"),
                "{error}"
            );
        }
        Ok(())
    }

    /// Adds the event `id` of `event_type` with `event_config` on the fixture's flow.
    async fn add_event(
        config: &mut PlacementConfig,
        state: &Arc<FlowLikeState>,
        id: &str,
        event_type: &str,
        event_config: &serde_json::Value,
    ) {
        let app = App::load(config.project_id.clone(), state.clone())
            .await
            .unwrap();
        let mut event = app.get_event("event", Some((1, 0, 0))).await.unwrap();
        event.id = id.into();
        event.event_type = event_type.into();
        event.config = serde_json::to_vec(event_config).unwrap();
        event.save(&app, Some((1, 0, 0))).await.unwrap();
        config.events.push(EventBinding {
            event_id: id.into(),
            event_version: [1, 0, 0],
            board_version: [1, 0, 0],
        });
    }

    /// The fixture's flow behind events of `event_type` with these configs, in one service
    /// with a web endpoint and its access token.
    async fn service_of(
        root: &Path,
        event_type: &str,
        events: &[(&str, serde_json::Value)],
    ) -> (PlacementConfig, Arc<FlowLikeState>) {
        let (mut config, state, _, _) = fixture(root).await;
        config.events.clear();
        for (id, event_config) in events {
            add_event(&mut config, &state, id, event_type, event_config).await;
        }
        config.hosting = Some(crate::config::HostingConfig {
            host: std::net::Ipv4Addr::LOCALHOST.into(),
            port: 1,
            max_in_flight: 1,
            request_timeout_secs: 5,
            authentication: Default::default(),
            auth_secret: Some("listener".into()),
            ui_origins: Vec::new(),
        });
        crate::secrets::install(&config, "listener", "t".repeat(32).as_bytes()).unwrap();
        (config, state)
    }

    #[tokio::test]
    async fn a_route_the_listener_cannot_take_is_refused_at_validation() -> Result<()> {
        use serde_json::json;
        for (routes, problem) in [
            (
                vec![
                    ("orders", json!({"path": "/orders", "method": "GET"})),
                    ("orders-copy", json!({"path": "orders", "method": "get"})),
                ],
                "Two events claim the same method and service path: GET /orders",
            ),
            (
                vec![("services", json!({"path": "/services", "method": "GET"}))],
                "route of event services: The path \"/services\" is reserved",
            ),
            (
                vec![("ui", json!({"path": "/ui/x"}))],
                "route of event ui: The path \"/ui/x\" is reserved",
            ),
            (
                vec![("spaced", json!({"path": "/a b"}))],
                "route of event spaced: The path",
            ),
            (
                vec![("nowhere", json!({}))],
                "route of event nowhere: The endpoint names no path",
            ),
        ] {
            let directory = tempfile::tempdir()?;
            let (config, state) = service_of(directory.path(), "http", &routes).await;
            let staged = CancellationToken::new();
            staged.cancel();
            for result in [
                run_with_state(&config, state.clone(), staged).await,
                validate_rollout_with_state(&config, state).await,
            ] {
                let error = format!("{:#}", result.unwrap_err());
                assert!(error.contains(problem), "{error}");
            }
        }
        // `/run/…` is no reserved prefix: only the run route of a form of the service conflicts.
        let directory = tempfile::tempdir()?;
        let (config, state) = service_of(
            directory.path(),
            "http",
            &[
                ("report", json!({"path": "/run/other", "method": "POST"})),
                ("orders", json!({"path_suffix": "orders"})),
            ],
        )
        .await;
        let staged = CancellationToken::new();
        staged.cancel();
        run_with_state(&config, state.clone(), staged).await?;
        validate_rollout_with_state(&config, state).await?;
        Ok(())
    }

    /// A service of one `api` event on the flow of `scheduled_fixture`, whose start node
    /// records its payload and returns.
    #[cfg(feature = "api-events")]
    async fn endpoint_service(
        root: &Path,
        endpoint: serde_json::Value,
        port: u16,
    ) -> (PlacementConfig, Arc<FlowLikeState>, Arc<ScheduledProbe>) {
        let unused = serde_json::json!({"expression": "0 0 1 1 *"});
        let (mut config, state, probe) = scheduled_fixture(root, unused).await;
        let app = App::load(config.project_id.clone(), state.clone())
            .await
            .unwrap();
        let mut event = app.get_event("report", Some((1, 0, 0))).await.unwrap();
        event.id = "orders".into();
        event.event_type = "api".into();
        event.config = serde_json::to_vec(&endpoint).unwrap();
        event.save(&app, Some((1, 0, 0))).await.unwrap();
        config.events = vec![EventBinding {
            event_id: "orders".into(),
            event_version: [1, 0, 0],
            board_version: [1, 0, 0],
        }];
        config.hosting = Some(crate::config::HostingConfig {
            host: std::net::Ipv4Addr::LOCALHOST.into(),
            port,
            max_in_flight: 2,
            request_timeout_secs: 5,
            authentication: Default::default(),
            auth_secret: Some("listener".into()),
            ui_origins: Vec::new(),
        });
        crate::secrets::install(&config, "listener", "t".repeat(32).as_bytes()).unwrap();
        (config, state, probe)
    }

    #[cfg(feature = "api-events")]
    #[tokio::test]
    async fn an_endpoint_alone_is_served_behind_the_service_token() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let missing_path = serde_json::json!({"method": "GET"});
        let (config, state, _) = endpoint_service(directory.path(), missing_path, 1).await;
        let staged = CancellationToken::new();
        staged.cancel();
        for result in [
            run_with_state(&config, state.clone(), staged).await,
            validate_rollout_with_state(&config, state).await,
        ] {
            let error = format!("{:#}", result.unwrap_err());
            assert!(error.contains("route of event orders"), "{error}");
        }

        let directory = tempfile::tempdir()?;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        // The editor's default config; the event's own token never reaches a device.
        let editor = serde_json::json!({"sink_type": "http", "method": "GET", "path": "/orders", "public_endpoint": true});
        let (config, state, probe) =
            endpoint_service(directory.path(), editor, address.port()).await;
        validate_rollout_with_state(&config, state.clone()).await?;
        let stop = CancellationToken::new();
        let _stop_on_drop = stop.clone().drop_guard();
        let (ready, is_ready) = oneshot::channel();
        let runtime_stop = stop.clone();
        let task = tokio::spawn(async move {
            run_with_state_listener(
                &config,
                state,
                runtime_stop,
                Profile::default(),
                Some(listener),
                None,
                || async move {
                    let _ = ready.send(());
                    Ok(())
                },
                false,
                None,
                None,
                PersonStartedRuns::default(),
            )
            .await
        });
        tokio::time::timeout(Duration::from_secs(10), is_ready).await??;
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()?;
        let orders = format!("http://{address}/orders?region=eu");
        assert_eq!(client.get(&orders).send().await?.status(), 401);
        let answer = client
            .get(&orders)
            .bearer_auth("t".repeat(32))
            .send()
            .await?;
        assert_eq!(answer.status(), 200);
        assert_eq!(
            *probe.payloads.lock().unwrap(),
            [Some(serde_json::json!({"region": "eu"}))]
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(!task.is_finished(), "an Endpoint service keeps running");
        stop.cancel();
        tokio::time::timeout(Duration::from_secs(20), task).await???;
        Ok(())
    }

    #[cfg(feature = "on-demand")]
    #[tokio::test]
    async fn a_form_keeps_the_instances_of_its_service_and_claims_its_run_route() -> Result<()> {
        use serde_json::json;
        let directory = tempfile::tempdir()?;
        let (mut config, state) =
            service_of(directory.path(), "simple_chat", &[("chat", json!({}))]).await;
        add_event(&mut config, &state, "form", "generic_form", &json!({})).await;
        add_event(&mut config, &state, "action", "quick_action", &json!({})).await;
        config.max_replicas = 2;
        let staged = CancellationToken::new();
        staged.cancel();
        run_with_state(&config, state.clone(), staged.clone()).await?;
        validate_rollout_with_state(&config, state.clone()).await?;

        let mut taken = config.clone();
        let endpoint = json!({"path": "/run/form", "method": "POST"});
        add_event(&mut taken, &state, "endpoint", "http", &endpoint).await;
        for result in [
            run_with_state(&taken, state.clone(), staged.clone()).await,
            validate_rollout_with_state(&taken, state.clone()).await,
        ] {
            let error = format!("{:#}", result.unwrap_err());
            assert!(
                error.contains("Two events claim the same method and service path: POST /run/form"),
                "{error}"
            );
        }
        let mut free = config;
        let endpoint = json!({"path": "/run/form", "method": "GET"});
        add_event(&mut free, &state, "endpoint-get", "http", &endpoint).await;
        run_with_state(&free, state, staged).await?;
        Ok(())
    }

    #[cfg(feature = "on-demand")]
    #[tokio::test]
    async fn an_on_demand_only_service_can_explicitly_configure_hosting() -> Result<()> {
        for kind in ["quick_action", "generic_form"] {
            let directory = tempfile::tempdir()?;
            let (config, state) =
                service_of(directory.path(), kind, &[("run", serde_json::json!({}))]).await;
            let staged = CancellationToken::new();
            staged.cancel();
            run_with_state(&config, state.clone(), staged).await?;
            validate_rollout_with_state(&config, state.clone()).await?;
            std::fs::remove_file(crate::config::private_secret_path(&config, "listener")?)?;
            assert!(validate_rollout_with_state(&config, state).await.is_err());
        }
        Ok(())
    }

    #[cfg(feature = "on-demand")]
    #[tokio::test]
    async fn a_form_alone_is_ready_without_a_listener_and_keeps_running() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let (mut config, state) = service_of(
            directory.path(),
            "generic_form",
            &[("form", serde_json::json!({}))],
        )
        .await;
        config.hosting = None;
        let error = run_with_state(&config, state.clone(), CancellationToken::new())
            .await
            .unwrap_err();
        assert!(
            format!("{error:#}")
                .contains("person-started events need the supervised placement runtime"),
            "{error:#}"
        );
        let context = schedule_context(
            directory.path(),
            TestClock::at("2026-09-01T10:00:00Z"),
            None,
        );
        let (ready, is_ready) = oneshot::channel();
        let (stop, task) = start_supervised(config, state, context, || async move {
            let _ = ready.send(());
            Ok(())
        });
        tokio::time::timeout(Duration::from_secs(10), is_ready).await??;
        let contract = directory
            .path()
            .join(".standalone-run/placement/contract.json");
        let contract: serde_json::Value = serde_json::from_slice(&std::fs::read(contract)?)?;
        assert!(contract["events"]["form"].is_object(), "{contract}");
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(!task.is_finished(), "a form-only service keeps running");
        assert!(!directory.path().join(".standalone-schedule").exists());
        stop.cancel();
        tokio::time::timeout(Duration::from_secs(20), task).await???;
        Ok(())
    }

    #[cfg(feature = "scheduled-once")]
    #[tokio::test]
    async fn a_one_time_schedule_alone_is_ready_and_runs_once_at_its_time() -> Result<()> {
        use serde_json::json;
        let directory = tempfile::tempdir()?;
        let impossible = json!({"scheduled_for": {"date": "2026-02-30", "time": "09:30"}});
        let (config, state, _) = scheduled_fixture(directory.path(), impossible).await;
        let staged = CancellationToken::new();
        staged.cancel();
        for result in [
            run_with_state(&config, state.clone(), staged).await,
            validate_rollout_with_state(&config, state).await,
        ] {
            let error = format!("{:#}", result.unwrap_err());
            assert!(error.contains("schedule of event report"), "{error}");
        }

        let directory = tempfile::tempdir()?;
        let once = json!({"scheduled_for": {"date": "2026-09-01", "time": "12:00"}});
        let (mut config, state, probe) = scheduled_fixture(directory.path(), once).await;
        config.events.remove(0);
        let clock = TestClock::at("2026-09-01T10:00:00Z");
        let context = schedule_context(directory.path(), clock.clone(), None);
        let (stop, task) = start_supervised(config, state, context, || async { Ok(()) });
        until("the one-time schedule is armed", || {
            schedule_state(directory.path())["decided"] == true
        })
        .await;
        let entry = &schedule_state(directory.path())["events"]["report"];
        assert_eq!(entry["once_state"], "pending", "{entry}");
        assert_eq!(probe.runs.load(Ordering::SeqCst), 0);
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(!task.is_finished(), "a schedule-only service keeps running");

        clock.set("2026-09-01T12:00:00Z");
        tokio::time::timeout(Duration::from_secs(15), probe.ran.notified()).await?;
        until("the run is recorded", || {
            schedule_state(directory.path())["events"]["report"]["once_state"] == "ran"
        })
        .await;
        clock.set("2026-09-01T12:00:30Z");
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(probe.runs.load(Ordering::SeqCst), 1);
        assert!(!task.is_finished(), "nothing more to run is no exit");
        stop.cancel();
        tokio::time::timeout(Duration::from_secs(20), task).await???;
        Ok(())
    }

    #[cfg(all(feature = "on-demand", feature = "api-events"))]
    #[tokio::test]
    async fn a_form_of_a_service_with_a_web_endpoint_runs_from_its_service_page() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let endpoint = serde_json::json!({"method": "GET", "path": "/orders"});
        let (mut config, state, probe) =
            endpoint_service(directory.path(), endpoint, address.port()).await;
        let app = App::load(config.project_id.clone(), state.clone()).await?;
        let mut form = app.get_event("report", Some((1, 0, 0))).await?;
        form.id = "form".into();
        form.event_type = "generic_form".into();
        form.config = b"{}".to_vec();
        form.save(&app, Some((1, 0, 0))).await?;
        config.events.push(EventBinding {
            event_id: "form".into(),
            event_version: [1, 0, 0],
            board_version: [1, 0, 0],
        });
        let context = schedule_context(
            directory.path(),
            TestClock::at("2026-09-01T10:00:00Z"),
            None,
        );
        let stop = CancellationToken::new();
        let _stop_on_drop = stop.clone().drop_guard();
        let (ready, is_ready) = oneshot::channel();
        let runtime_stop = stop.clone();
        let task = tokio::spawn(async move {
            run_with_state_listener(
                &config,
                state,
                runtime_stop,
                Profile::default(),
                Some(listener),
                None,
                || async move {
                    let _ = ready.send(());
                    Ok(())
                },
                false,
                None,
                Some(context),
                PersonStartedRuns::default(),
            )
            .await
        });
        tokio::time::timeout(Duration::from_secs(10), is_ready).await??;
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()?;
        let token = "t".repeat(32);
        let services = client
            .get(format!("http://{address}/services"))
            .bearer_auth(&token)
            .send()
            .await?
            .error_for_status()?
            .json::<serde_json::Value>()
            .await?;
        assert!(
            services["events"]
                .as_array()
                .is_some_and(|events| events.iter().any(|event| event["id"] == "form")),
            "{services}"
        );
        let run = format!("http://{address}/run/form");
        let refused = client
            .post(&run)
            .json(&serde_json::json!({}))
            .send()
            .await?;
        assert_eq!(refused.status(), 401);
        let answer = client
            .post(&run)
            .bearer_auth(&token)
            .json(&serde_json::json!({}))
            .send()
            .await?;
        assert_eq!(answer.status(), 200);
        let stream = answer.text().await?;
        assert!(stream.contains("done"), "{stream}");
        assert_eq!(probe.runs.load(Ordering::SeqCst), 1);
        assert!(!task.is_finished());
        stop.cancel();
        tokio::time::timeout(Duration::from_secs(20), task).await???;
        Ok(())
    }

    #[cfg(feature = "bots-telegram")]
    #[tokio::test]
    async fn a_bot_token_key_counts_only_for_a_bot_of_the_service() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let (mut config, state) = service_of(
            directory.path(),
            "telegram",
            &[("helper", serde_json::json!({}))],
        )
        .await;
        config.hosting = None;
        let token_key = crate::event_kind::bot_token_key("helper");
        config
            .secret_overrides
            .insert(token_key.clone(), "helper-token".into());
        crate::secrets::install(
            &config,
            "helper-token",
            br#""123456:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA""#,
        )?;
        let staged = CancellationToken::new();
        staged.cancel();
        run_with_state(&config, state.clone(), staged.clone()).await?;
        validate_rollout_with_state(&config, state.clone()).await?;

        let mut stray = config.clone();
        stray.secret_overrides.insert(
            crate::event_kind::bot_token_key("other"),
            "helper-token".into(),
        );
        let error = run_with_state(&stray, state.clone(), staged.clone())
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains(
                "secret event.other.bot_token names the token of a bot that this placement does not run"
            ),
            "{error:#}"
        );
        // A plain value under a token key is no token: nothing applies it.
        let mut plain = config.clone();
        plain.variables.insert(
            token_key,
            serde_json::json!("123456:AAAAAAAAAAAAAAAAAAAAAAAA"),
        );
        let error = run_with_state(&plain, state.clone(), staged.clone())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("is absent"), "{error:#}");

        config.max_replicas = 2;
        let error = run_with_state(&config, state, staged).await.unwrap_err();
        assert!(
            error.to_string().contains("bots cannot be replicated"),
            "{error:#}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn a_type_whose_part_this_build_lacks_is_refused_as_before() -> Result<()> {
        for event_type in ["api", "quick_action", "generic_form", "telegram", "discord"]
            .into_iter()
            .filter(|event_type| !crate::event_kind::built_in(event_type))
        {
            let directory = tempfile::tempdir()?;
            let (mut config, state, _, starts) = fixture(directory.path()).await;
            let app = App::load(config.project_id.clone(), state.clone()).await?;
            let mut event = app.get_event("event", Some((1, 0, 0))).await?;
            event.event_type = event_type.into();
            event.event_version = (2, 0, 0);
            event.save(&app, Some((2, 0, 0))).await?;
            config.events[0].event_version = [2, 0, 0];
            let staged = CancellationToken::new();
            staged.cancel();
            let error = run_with_state(&config, state.clone(), staged)
                .await
                .unwrap_err();
            assert_eq!(
                error.to_string(),
                format!("event event needs an unsupported {event_type} sink")
            );
            config.max_replicas = 2;
            let error = validate_rollout_with_state(&config, state)
                .await
                .unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("scheduled and daemon triggers cannot be replicated"),
                "{error:#}"
            );
            assert_eq!(starts.load(Ordering::SeqCst), 0);
        }
        Ok(())
    }
}
