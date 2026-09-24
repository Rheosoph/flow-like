use std::{collections::HashSet, future::Future, path::Path, sync::Arc, time::Duration};

use anyhow::{Context, Result, bail, ensure};
use flow_like_runtime::{
    app::{App, AppVisibility},
    flow::{
        compiled::TemplateCache,
        execution::{
            ExecutionEnvironment, InternalRun, LogLevel, RunPayload, RunStatus,
            service::{ServiceOutcome, ServiceReadyKind},
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
    files::store::{FlowLikeStore, local_store::LocalObjectStore},
    lancedb,
};
use flow_like_types::authorization::RequestAuthorizer;
use flow_like_types::intercom::BufferedInterComHandler;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::config::{PlacementConfig, ProjectSource};

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
            let state = initialize_state(runtime, Some(authorizer), None).await?;
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
        on_ready,
    )
    .await
}

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
    let local_data = data_root.unwrap_or(&config.project_path);
    let mut delegating_user_id = None;
    let mut revoked = CancellationToken::new();
    let state = match config.source {
        ProjectSource::Offline => {
            initialize_state(
                placement_local_config(config, local_data)?,
                authorizer,
                None,
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
            initialize_state(runtime, Some(authorizer), Some(online.registry)).await?
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
    initialize_state(config, authorizer, None).await
}

pub(crate) async fn offline_state(
    root: &Path,
    authorizer: Option<Arc<dyn RequestAuthorizer>>,
) -> Result<Arc<FlowLikeState>> {
    initialize_state(local_config(root)?, authorizer, None).await
}

fn local_config(root: &Path) -> Result<FlowLikeConfig> {
    local_config_with_data(root, root)
}

fn placement_local_config(placement: &PlacementConfig, data_root: &Path) -> Result<FlowLikeConfig> {
    let mut config = local_config_with_data(&placement.project_path, data_root)?;
    if !placement.bit_pins.is_empty() {
        // Changed model metadata gets a distinct local path. An old replica
        // can retain its open model while the next revision materializes.
        let digest =
            flow_like_device_protocol::artifact_sha256(&serde_json::to_vec(&placement.bit_pins)?);
        config.register_bits_store(local_store(&data_root.join("bits").join(digest))?);
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
        lancedb::connect(project_root.join(path.as_ref()).to_string_lossy().as_ref())
    }));
    let user_root = data_root.join("user");
    config.register_build_user_database(Arc::new(move |path: StorePath| {
        lancedb::connect(user_root.join(path.as_ref()).to_string_lossy().as_ref())
    }));
    let logs_root = data_root.join("logs");
    config.register_build_logs_database(Arc::new(move |path: StorePath| {
        lancedb::connect(logs_root.join(path.as_ref()).to_string_lossy().as_ref())
    }));
    Ok(config)
}

async fn initialize_state(
    config: FlowLikeConfig,
    authorizer: Option<Arc<dyn RequestAuthorizer>>,
    registry: Option<Arc<flow_like_storage::lance_io::object_store::ObjectStoreRegistry>>,
) -> Result<Arc<FlowLikeState>> {
    let mut state = FlowLikeState::new(config, HTTPClient::new_without_refetch());
    // Store-backed local data works under Server. Ambient host credentials and
    // arbitrary filesystem paths remain unavailable to deployed workflows.
    state.execution_environment = ExecutionEnvironment::Server;
    state.request_authorizer = authorizer;
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
    )
    .await
}

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
) -> Result<()> {
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
    let mut native_host_required = false;
    let mut selected_events = HashSet::new();
    let mut applied_variables = HashSet::new();
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
        ensure!(
            config.max_replicas == 1
                || matches!(event.event_type.as_str(), "http" | "simple_chat")
                || event.default_page_id.is_some(),
            "Multiple replicas require HTTP, chat, or Page events; scheduled and daemon triggers cannot be replicated"
        );
        ensure!(
            event.id == binding.event_id && event.event_version == event_version,
            "event archive does not match the requested identity and version"
        );
        ensure!(event.active, "event {} is inactive", event.id);
        ensure!(
            event.default_page_id.is_some()
                || matches!(
                    event.event_type.as_str(),
                    "daemon" | "rest" | "mcp" | "http" | "simple_chat"
                ),
            "event {} needs an unsupported {} sink",
            event.id,
            event.event_type
        );
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
        crate::dependencies::validate_board_packages(config, &*source_board.lock().await)?;
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

        let readiness = service_readiness_source(&event, &template.board)?;
        native_host_required |= readiness.is_none();
        if stop.is_cancelled() {
            continue;
        }

        if event.default_page_id.is_some()
            || matches!(event.event_type.as_str(), "http" | "simple_chat")
        {
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
    for id in config
        .variables
        .keys()
        .chain(config.secret_overrides.keys())
    {
        ensure!(
            applied_variables.contains(id),
            "placement variable {id} is absent from the selected pinned boards"
        );
    }
    if validate_secrets && native_host_required {
        crate::hosting::validate_access_token(config)?;
    }
    ensure!(
        native_host_required || config.hosting.is_none(),
        "Hosting configuration requires an HTTP, chat, or Page event"
    );
    if stop.is_cancelled() {
        return Ok(());
    }
    let hosting = if !hosted.is_empty() {
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
        if let Some(sub) = delegating_user_id {
            hosting.set_execution_sub(sub)?;
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

/// Only a top-level node can announce this event's startup. Request function
/// layers inherit the run cache but never receive readiness authority.
pub(crate) fn service_readiness_source(
    event: &flow_like_runtime::flow::event::Event,
    board: &flow_like_runtime::flow::board::Board,
) -> Result<Option<(String, ServiceReadyKind)>> {
    if event.default_page_id.is_some()
        || matches!(event.event_type.as_str(), "http" | "simple_chat")
    {
        return Ok(None);
    }
    let (name, kind) = match event.event_type.as_str() {
        "rest" => ("rest_server", ServiceReadyKind::RestListener),
        "mcp" => ("mcp_server", ServiceReadyKind::McpListener),
        "daemon" => ("service_ready", ServiceReadyKind::Daemon),
        _ => bail!("Event {} does not support supervised readiness", event.id),
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
mod tests {
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

    pub(super) async fn fixture(
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
            auth_secret: "listener".into(),
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
        let directory = tempfile::tempdir().unwrap();
        let (config, state, started, starts) = fixture(directory.path()).await;
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
            .lock()
            .await
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
}
