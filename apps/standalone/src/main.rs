use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};
use flow_like_standalone::{
    config::PlacementConfig,
    enrollment, release,
    state::{DesiredState, StateStore},
    supervisor, vault,
};
use std::{
    io::Read,
    path::{Path, PathBuf},
};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

const STATE_DIRECTORY_VARIABLE: &str = "FLOW_LIKE_STANDALONE_STATE_DIR";
const MAX_ENV_FILE_BYTES: u64 = 65_536;

#[derive(Parser)]
#[command(
    version,
    about = "Run persistent Flow-Like project services on this device"
)]
struct Cli {
    /// Local state directory, overriding .env. Defaults to .flow-like-standalone.
    #[arg(long, global = true, env = "FLOW_LIKE_STANDALONE_STATE_DIR")]
    state_dir: Option<PathBuf>,
    /// Validate and read standalone settings from this file; relative state paths use its directory.
    /// Without this flag, ./.env is read only when no state override is supplied.
    #[arg(long, global = true)]
    env_file: Option<PathBuf>,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Create local device state.
    Init,
    /// Print release compatibility without opening device state.
    ReleaseInfo,
    #[command(hide = true)]
    UpdateGuard {
        #[arg(long)]
        operation_id: String,
    },
    /// Create an enrollment package using a user access token file with mode 0600.
    /// The controller and invitation vaults remain locally in the state directory.
    Setup {
        #[arg(long)]
        api_url: String,
        #[arg(long)]
        name: String,
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        access_token_file: PathBuf,
        /// Pinned release URL and public keys from the configured hub.
        #[arg(
            long,
            required_unless_present = "development_binary",
            conflicts_with = "development_binary"
        )]
        release_trust: Option<PathBuf>,
        /// Target Rust triple from the signed release manifest.
        #[arg(long, requires = "release_trust")]
        target: Option<String>,
        #[arg(long, value_enum, default_value = "binary")]
        mode: release::PackageMode,
        /// Explicitly package a local development binary without release trust or automatic updates.
        #[arg(long)]
        development_binary: Option<PathBuf>,
    },
    /// Register this device from an exported package, or recover its previous enrollment.
    Enroll { package: PathBuf },
    /// Recover a registration receipt or recheck denied access using the permanent device key.
    RecoverEnrollment,
    /// Install autostart: Linux at boot, macOS after the owner logs in.
    InstallService,
    /// Stop and remove this agent's user service, preserving device and project data.
    UninstallService,
    /// Print a systemd unit or macOS LaunchAgent plist without installing it.
    ServiceUnit,
    /// Inspect this exact user service without creating device state.
    ServiceStatus,
    /// Persist a pinned offline placement manifest. Reapplying preserves its desired state.
    #[command(after_long_help = r#"Example placement.json:
{
  "id": "local-api",
  "project_id": "my-project",
  "deployment_id": "api",
  "revision": "release-1",
  "source": "offline",
  "project_path": "./project-store",
  "events": [{
    "event_id": "daemon-event",
    "event_version": [1, 0, 0],
    "board_version": [1, 0, 0]
  }],
  "variables": {}
}

project_path is relative to the manifest and contains apps/<project_id>/manifest.app,
the pinned event/board archives, and required local project files and databases.
Variables use board variable IDs. secret_overrides maps variable IDs to private JSON
secret files installed with set-secret. Use export-env to generate an environment
template from the pinned offline project, then apply --variables-env to import it.
The native HTTP, chat and page host uses hosting.host and hosting.port. REST/MCP
Server nodes acknowledge readiness after binding their listener. Other daemon
workflows must call Service Ready after initialization and continue running.
REST, MCP and daemon placements currently support one replica."#)]
    Apply {
        manifest: PathBuf,
        #[arg(long)]
        stopped: bool,
        #[arg(long)]
        variables_env: Option<PathBuf>,
    },
    /// Export exposed variables from a pinned offline placement as a private environment template.
    ExportEnv {
        manifest: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    /// Import one offline project from an existing object-store root.
    ImportProject {
        project_id: String,
        source: PathBuf,
        /// Private JSON selecting exact offline Bit metadata and WASM package hashes.
        #[arg(long)]
        assets_file: Option<PathBuf>,
    },
    /// Start the supervisor and restore persisted running placements.
    Run,
    /// Request that a placement run; also clears its crash-loop retry limit.
    Start { placement: String },
    /// Request graceful shutdown of a placement without deleting its data.
    Stop { placement: String },
    /// Remove a stopped placement, permanently retire its ID, and preserve its data.
    Remove { placement: String },
    /// Print local device and process state as JSON, excluding variables and credentials.
    Status,
    /// Install a placement secret from a private file, without putting it in argv or JSON output.
    SetSecret {
        placement: String,
        name: String,
        #[arg(long)]
        value_file: PathBuf,
    },
    #[command(hide = true)]
    RunPlacement {
        placement: String,
        #[arg(long, default_value_t = 0)]
        replica_slot: u8,
        #[arg(long)]
        config_revision: u64,
        #[arg(long)]
        intent_revision: u64,
        #[arg(long)]
        parent_pid: u32,
        #[arg(long)]
        broker_fd: i32,
        #[arg(long)]
        placement_lock_fd: i32,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_writer(std::io::stderr)
        .init();
    let cli = Cli::parse();
    if let Commands::RunPlacement {
        placement,
        replica_slot,
        config_revision,
        intent_revision,
        parent_pid,
        broker_fd,
        placement_lock_fd,
    } = &cli.command
    {
        return run_child(
            placement,
            *replica_slot,
            *config_revision,
            *intent_revision,
            *parent_pid,
            *broker_fd,
            *placement_lock_fd,
        )
        .await;
    }
    if matches!(cli.command, Commands::ReleaseInfo) {
        println!(
            "{}",
            serde_json::json!({"version":env!("CARGO_PKG_VERSION"),"target":flow_like_device_protocol::ReleaseTarget::current()?.triple(),"state_schema_version":flow_like_standalone::state::SCHEMA_VERSION,"runtime":cfg!(feature="runtime")})
        );
        return Ok(());
    }
    let selected_state_dir = resolve_state_dir(cli.state_dir.as_deref(), cli.env_file.as_deref())?;
    // Service inspection and removal must not create a new device identity.
    if matches!(
        &cli.command,
        Commands::InstallService
            | Commands::UninstallService
            | Commands::ServiceUnit
            | Commands::ServiceStatus
    ) {
        let state_dir = std::path::absolute(&selected_state_dir)?;
        let executable = std::env::current_exe()?;
        match cli.command {
            Commands::InstallService => {
                ensure!(
                    cfg!(feature = "runtime"),
                    "Service installation requires a binary built with the runtime feature"
                );
                let unit =
                    flow_like_standalone::service::install_user_service(&executable, &state_dir)
                        .await?;
                println!(
                    "{}",
                    serde_json::json!({"service":"installed","unit":unit,"state_dir":state_dir,"startup":flow_like_standalone::service::service_startup()})
                );
            }
            Commands::UninstallService => {
                let removed =
                    flow_like_standalone::service::uninstall_user_service(&executable, &state_dir)
                        .await?;
                println!(
                    "{}",
                    serde_json::json!({"service":if removed {"removed"} else {"not_installed"},"state_dir":state_dir})
                );
            }
            Commands::ServiceUnit => print!(
                "{}",
                flow_like_standalone::service::user_service_definition(&executable, &state_dir)?
            ),
            Commands::ServiceStatus => println!(
                "{}",
                serde_json::to_string(
                    &flow_like_standalone::service::user_service_status(&executable, &state_dir)
                        .await?
                )?
            ),
            _ => unreachable!(),
        }
        return Ok(());
    }
    let state_dir = supervisor::prepare_state_dir(&selected_state_dir)?;
    if let Commands::UpdateGuard { operation_id } = &cli.command {
        return release::update::guard(&state_dir, operation_id).await;
    }
    if matches!(cli.command, Commands::Run)
        && release::update::recover_after_boot(&state_dir, &flow_like_standalone::host::boot_id()?)?
    {
        anyhow::bail!("Restored the previous verified release after reboot; restart the agent");
    }
    let mut store = StateStore::open(&state_dir.join("management.sqlite"))?;
    match cli.command {
        Commands::Init => println!(
            "{}",
            serde_json::json!({"device_id":store.device_id(),"enrollment":store.registration()?.map(|record|record.connection_status).unwrap_or_else(||"not_configured".into()),"state_dir":state_dir})
        ),
        Commands::ImportProject {
            project_id,
            source,
            assets_file,
        } => {
            let assets =
                assets_file
                    .map(
                        |path| -> Result<
                            flow_like_standalone::project_artifacts::ProjectArtifactAssets,
                        > {
                            Ok(serde_json::from_slice(&vault::read_private(&path)?)?)
                        },
                    )
                    .transpose()?
                    .unwrap_or_default();
            let receipt = flow_like_standalone::project_artifacts::import_local_selected(
                &store,
                &state_dir,
                &project_id,
                &source,
                &assets,
            )?;
            println!("{}", serde_json::to_string_pretty(&receipt)?);
        }
        Commands::Setup {
            api_url,
            name,
            output,
            access_token_file,
            release_trust,
            target,
            mode,
            development_binary,
        } => {
            ensure!(
                cfg!(feature = "runtime"),
                "Deployment packages require a binary built with the runtime feature"
            );
            let token = vault::read_private(&access_token_file)?;
            let token = std::str::from_utf8(&token)?.trim();
            let prepared = if let Some(trust_path) = release_trust {
                let trust = release::ReleaseTrust::load(&trust_path)?;
                let target = target
                    .map(|value| serde_json::from_value(serde_json::Value::String(value)))
                    .transpose()?
                    .unwrap_or(flow_like_device_protocol::ReleaseTarget::current()?);
                Some(
                    release::PreparedPackage::download(
                        &trust,
                        target,
                        mode,
                        &state_dir.join("downloads"),
                    )
                    .await?,
                )
            } else {
                None
            };
            let password = vault::read_password(true)?;
            let manifest = if let Some(prepared) = &prepared {
                enrollment::create_release_package(
                    &state_dir.join("controllers"),
                    &api_url,
                    &name,
                    &output,
                    token,
                    &password,
                    prepared,
                )
                .await?
            } else {
                ensure!(
                    matches!(mode, release::PackageMode::Binary),
                    "Development packages support binary mode only"
                );
                enrollment::create_package(
                    &state_dir.join("controllers"),
                    &api_url,
                    &name,
                    &output,
                    token,
                    &password,
                    &development_binary
                        .context("Select a signed release or an explicit development binary")?,
                )
                .await?
            };
            println!(
                "{}",
                serde_json::json!({"device_id":manifest.device_id,"expires_at":manifest.expires_at,
                "package":output,"platform":prepared.as_ref().map(|value|value.target().triple()),
                "controller_vault":state_dir.join("controllers").join(format!("{}.vault",manifest.device_id)),
                "invitation_vault":state_dir.join("controllers").join(format!("{}.invitation.vault",manifest.device_id)),
                "next":"On the target: ./flow-like-standalone --state-dir ./state enroll .; then ./flow-like-standalone --state-dir ./state run"})
            );
        }
        Commands::Enroll { package } => {
            drop(store);
            release::install_package_trust(&state_dir, &package)?;
            let receipt = enrollment::enroll(&state_dir, &package).await?;
            println!(
                "{}",
                serde_json::json!({"device_id":receipt.device_id,"enrollment":"enrolled","registered_at":receipt.registered_at})
            );
        }
        Commands::RecoverEnrollment => {
            drop(store);
            let receipt = enrollment::recover_registration(&state_dir).await?;
            println!(
                "{}",
                serde_json::json!({"device_id":receipt.device_id,"enrollment":"enrolled","registered_at":receipt.registered_at})
            );
        }
        Commands::InstallService
        | Commands::UninstallService
        | Commands::ServiceUnit
        | Commands::ServiceStatus => {
            unreachable!()
        }
        Commands::ExportEnv { manifest, output } => {
            #[cfg(feature = "runtime")]
            flow_like_standalone::environment::export(&PlacementConfig::load(&manifest)?, &output)
                .await?;
            #[cfg(not(feature = "runtime"))]
            {
                let _ = (manifest, output);
                anyhow::bail!("Environment export requires the runtime feature");
            }
        }
        Commands::Apply {
            manifest,
            stopped,
            variables_env,
        } => {
            let mut config = PlacementConfig::load(&manifest)?;
            store.check_placement_identity(&config.id, &serde_json::to_value(&config)?)?;
            if let Some(path) = variables_env {
                #[cfg(feature = "runtime")]
                flow_like_standalone::environment::apply(&mut config, &path).await?;
                #[cfg(not(feature = "runtime"))]
                {
                    let _ = path;
                    anyhow::bail!("Variable environment import requires the runtime feature");
                }
            }
            if stopped {
                config.validate()?;
            } else {
                validate_for_install(&config).await?;
            }
            let record = store.upsert_placement(
                &config.id,
                &serde_json::to_value(&config)?,
                if stopped {
                    DesiredState::Stopped
                } else {
                    DesiredState::Running
                },
            )?;
            println!(
                "{}",
                serde_json::json!({"placement_id":record.id,"config_revision":record.config_revision,"desired_state":record.desired_state})
            );
        }
        Commands::Start { placement } => {
            store.set_desired_state(&placement, DesiredState::Running)?
        }
        Commands::Stop { placement } => {
            store.set_desired_state(&placement, DesiredState::Stopped)?
        }
        Commands::Remove { placement } => store.remove_placement(&placement)?,
        Commands::SetSecret {
            placement,
            name,
            value_file,
        } => {
            let value = vault::read_private(&value_file)?;
            flow_like_standalone::secrets::install_current(&store, &placement, &name, &value)?;
            println!(
                "{}",
                serde_json::json!({"placement_id":placement,"name":name,"secret":"installed"})
            );
        }
        Commands::Status => {
            let placements: Vec<_> = store
                .list_placements()?
                .into_iter()
                .map(|placement| {
                    serde_json::json!({
                        "id": placement.id,
                        "desired_state": placement.desired_state,
                        "observed_state": placement.observed_state,
                        "config_revision": placement.config_revision,
                        "applied_revision": placement.applied_revision,
                        "process_id": placement.process_id,
                        "last_error": placement.last_error,
                        "desired_replicas": placement.desired_replicas,
                        "running_replicas": placement.running_replicas,
                        "ready_replicas": placement.ready_replicas,
                        "replicas": placement.replicas.iter().map(|r|serde_json::json!({"slot":r.slot,"observed_state":r.observed_state,"config_revision":r.config_revision,"applied_revision":r.applied_revision,"process_id":r.process_id})).collect::<Vec<_>>(),
                    })
                })
                .collect();
            let agent_running = supervisor::agent_is_running(&state_dir)?;
            let registration = store.registration()?;
            let enrollment = registration
                .as_ref()
                .map(|record| {
                    serde_json::json!({
                        "status":record.connection_status,"owner_id":record.manifest.owner_id,
                        "api_base_url":record.manifest.api_base_url,"name":record.manifest.name,
                        "last_contact_at":record.last_contact_at,
                    })
                })
                .unwrap_or(serde_json::Value::String("not_configured".into()));
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({"device_id":store.device_id(),"enrollment":enrollment,"agent_running":agent_running,"observations_current":agent_running,"placements":placements})
                )?
            );
        }
        Commands::Run => {
            let _runtime_lock = supervisor::lock_file(&state_dir.join("runtime.lock"))?;
            supervisor::require_supported_supervision()?;
            ensure!(
                cfg!(feature = "runtime"),
                "This binary was built without the runtime feature"
            );
            drop(store);
            flow_like_standalone::certificates::collect_unused(&state_dir)?;
            let session = enrollment::DeviceSession::load(&state_dir)?.map(std::sync::Arc::new);
            let boot_id = flow_like_standalone::host::boot_id()?;
            let run_id = uuid::Uuid::new_v4().to_string();
            flow_like_standalone::host::reconcile_boot(&state_dir, &boot_id)?;
            let cancel = CancellationToken::new();
            let signal = tokio::spawn(supervisor::shutdown_signal(cancel.clone()));
            let presence_cancel = cancel.child_token();
            let presence = session.clone().map(|session| {
                tokio::spawn(enrollment::maintain_presence_with_session(
                    state_dir.clone(),
                    session,
                    presence_cancel.clone(),
                ))
            });
            let management = session.clone().map(|session| {
                flow_like_standalone::management::ManagementService::new(
                    state_dir.clone(),
                    session,
                    boot_id.clone(),
                )
            });
            let transport = management.map(|management| {
                let device = session
                    .as_ref()
                    .expect("management has an enrolled device")
                    .clone();
                tokio::spawn(flow_like_standalone::transport::run(
                    device,
                    management,
                    cancel.child_token(),
                ))
            });
            let fleet = session.clone().map(|device| {
                tokio::spawn(flow_like_standalone::fleet::publish(
                    state_dir.clone(),
                    device,
                    boot_id.clone(),
                    cancel.child_token(),
                ))
            });
            let certificate_inventory = session.clone().map(|device| {
                tokio::spawn(flow_like_standalone::certificate_inventory::publish(
                    state_dir.clone(),
                    device,
                    cancel.child_token(),
                ))
            });
            let certificate_renewal = tokio::spawn(flow_like_standalone::certificate_issuers::run(
                state_dir.clone(),
                cancel.child_token(),
            ));
            let acme = tokio::spawn(flow_like_standalone::acme::run(
                state_dir.clone(),
                cancel.child_token(),
            ));
            let metrics = tokio::spawn(flow_like_standalone::telemetry::sample(
                state_dir.clone(),
                cancel.child_token(),
            ));
            let secrets = tokio::spawn(flow_like_standalone::secrets::publish(
                state_dir.clone(),
                cancel.child_token(),
            ));
            let shared_telemetry = session.clone().map(|device| {
                tokio::spawn(flow_like_standalone::telemetry_groups::publish_live(
                    state_dir.clone(),
                    device,
                    cancel.child_token(),
                ))
            });
            let archives = session.clone().map(|device| {
                tokio::spawn(flow_like_standalone::archives::publish(
                    state_dir.clone(),
                    device,
                    cancel.child_token(),
                ))
            });
            let reboot = {
                let root = state_dir.clone();
                let boot = boot_id.clone();
                let stop = cancel.clone();
                tokio::spawn(async move {
                    flow_like_standalone::host::watch_reboot(&root, &boot, stop).await
                })
            };
            let update = session.as_ref().map(|device| {
                let root = state_dir.clone();
                let boot = boot_id.clone();
                let run = run_id.clone();
                let id = device.manifest().device_id.clone();
                let stop = cancel.clone();
                tokio::spawn(async move {
                    flow_like_standalone::host::watch_update(&root, &id, &boot, &run, stop).await
                })
            });
            let result = supervisor::run_with_session_and_ready(
                &state_dir,
                &std::env::current_exe()?,
                cancel.clone(),
                session.clone(),
                || {
                    release::update::confirm_ready(
                        &state_dir,
                        session
                            .as_ref()
                            .map(|d| d.manifest().device_id.as_str())
                            .unwrap_or("unenrolled"),
                        &boot_id,
                        &run_id,
                    )
                },
            )
            .await;
            cancel.cancel();
            presence_cancel.cancel();
            if let Some(transport) = transport {
                let _ = transport.await;
            }
            let _ = certificate_renewal.await;
            let _ = acme.await;
            let _ = metrics.await;
            if let Some(fleet) = fleet {
                let _ = fleet.await;
            }
            if let Some(inventory) = certificate_inventory {
                let _ = inventory.await;
            }
            let _ = secrets.await;
            if let Some(archives) = archives {
                let _ = archives.await;
            }
            if let Some(shared) = shared_telemetry {
                let _ = shared.await;
            }
            let _ = reboot.await;
            if let Some(update) = update {
                let _ = update.await;
            }
            if let Some(presence) = presence {
                match presence.await {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => tracing::error!("Device presence stopped: {error}"),
                    Err(error) => tracing::error!("Device presence task failed: {error}"),
                }
            }
            signal.abort();
            result?;
            flow_like_standalone::host::dispatch_reboot(&state_dir, &boot_id).await?;
            flow_like_standalone::host::dispatch_update(&state_dir, &boot_id).await?;
        }
        Commands::RunPlacement { .. } | Commands::ReleaseInfo | Commands::UpdateGuard { .. } => {
            unreachable!()
        }
    }
    Ok(())
}

#[cfg(unix)]
async fn run_child(
    placement: &str,
    replica_slot: u8,
    config_revision: u64,
    intent_revision: u64,
    parent_pid: u32,
    broker_fd: i32,
    placement_lock_fd: i32,
) -> Result<()> {
    supervisor::require_supported_supervision()?;
    let _placement_lock = flow_like_standalone::ipc::inherited_placement_lock(placement_lock_fd)?;
    let (bootstrap, broker) = flow_like_standalone::ipc::ChildBroker::inherited(broker_fd).await?;
    ensure!(
        bootstrap.config.id == placement
            && bootstrap.replica_slot == replica_slot
            && bootstrap.config_revision == config_revision
            && bootstrap.intent_revision == intent_revision
            && bootstrap.parent_pid == parent_pid,
        "Workload launch does not match its supervisor channel"
    );
    let sandboxed = flow_like_standalone::isolation::sandboxed(&bootstrap.config);
    flow_like_standalone::isolation::verify_child(&bootstrap.config)?;
    if !sandboxed {
        // SAFETY: getppid has no preconditions.
        ensure!(
            unsafe { libc::getppid() } as u32 == parent_pid,
            "Supervisor is no longer the workload parent"
        );
    }
    let cancel = CancellationToken::new();
    let signal = tokio::spawn(supervisor::shutdown_signal(cancel.clone()));
    let parent =
        (!sandboxed).then(|| tokio::spawn(supervisor::watch_parent(parent_pid, cancel.clone())));
    #[cfg(feature = "runtime")]
    let result = {
        let _ = flow_like_standalone::usage::snapshot();
        let mut usage_reporter = None;
        let usage_stop = tokio_util::sync::CancellationToken::new();
        let authorizer = Some(broker.clone()
            as std::sync::Arc<dyn flow_like_types_contracts::authorization::RequestAuthorizer>);
        let listener = if bootstrap.inherited_listener {
            Some(flow_like_standalone::ipc::inherited_listener(
                bootstrap
                    .config
                    .hosting
                    .as_ref()
                    .context("Inherited listener has no hosting configuration")?,
            )?)
        } else {
            None
        };
        let result = flow_like_standalone::runtime::run_supervised_with_ready(
            &bootstrap.config,
            cancel.clone(),
            authorizer,
            bootstrap.api_base_url.as_deref(),
            broker.identity(),
            Some(broker.clone() as std::sync::Arc<dyn flow_like_standalone::online::outage::OutageAuthority>),
            listener,
            (bootstrap.config.max_replicas > 1).then_some(flow_like_standalone::hosting::ReplicaContext {
                supervisor_pid: bootstrap.parent_pid,
                config_revision: bootstrap.config_revision,
                slot: bootstrap.replica_slot,
            }),
            Some(bootstrap.data_root.as_deref().context("Supervisor did not provide the placement data root")?),
            bootstrap.config.tls_certificate_id.as_ref().map(|_| flow_like_standalone::service_tls::ManagedTls::new(broker.clone()) as std::sync::Arc<dyn flow_like_runtime::flow::execution::service::ServiceTlsProvider>),
            || async {
                broker.ready().await?;
                {
                    let broker = broker.clone();
                    let stop = usage_stop.clone();
                    usage_reporter = Some(tokio::spawn(async move {
                        let mut tick = tokio::time::interval(std::time::Duration::from_secs(5));
                        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                        loop {
                            tokio::select! { _=stop.cancelled()=>break, _=tick.tick()=>() }
                            let reported = broker.report_usage(flow_like_standalone::usage::snapshot()).await;
                            if reported.is_err() {
                                tracing::warn!("Workload usage reporting stopped after its broker channel closed");
                                break;
                            }
                        }
                    }));
                }
                Ok(())
            },
        )
        .await;
        cancel.cancel();
        usage_stop.cancel();
        if let Some(mut reporter) = usage_reporter {
            if tokio::time::timeout(std::time::Duration::from_secs(2), &mut reporter)
                .await
                .is_err()
            {
                reporter.abort();
                let _ = reporter.await;
            } else {
                let final_snapshot = flow_like_standalone::usage::snapshot();
                if final_snapshot.in_flight == 0 {
                    let _ = tokio::time::timeout(
                        std::time::Duration::from_secs(2),
                        broker.report_final_usage(final_snapshot),
                    )
                    .await;
                }
            }
        }
        result
    };
    #[cfg(not(feature = "runtime"))]
    let result: Result<()> = {
        let _ = (bootstrap, broker, cancel);
        Err(anyhow::anyhow!(
            "This binary was built without the runtime feature"
        ))
    };
    signal.abort();
    if let Some(parent) = parent {
        parent.abort();
    }
    result
}

#[cfg(not(unix))]
async fn run_child(
    _placement: &str,
    _replica_slot: u8,
    _config_revision: u64,
    _intent_revision: u64,
    _parent_pid: u32,
    _broker_fd: i32,
    _placement_lock_fd: i32,
) -> Result<()> {
    anyhow::bail!("Standalone workload credential channels currently require Unix")
}

fn resolve_state_dir(state_override: Option<&Path>, env_file: Option<&Path>) -> Result<PathBuf> {
    // Clap has already selected the explicit argument over the process variable.
    // Explicit files are still validated; automatic files cannot override either.
    let from_file = match env_file {
        Some(path) => read_env_state_dir(path, false)?,
        None if state_override.is_none() => read_env_state_dir(Path::new(".env"), true)?,
        None => None,
    };
    let selected = state_override
        .map(Path::to_path_buf)
        .or(from_file)
        .unwrap_or_else(|| PathBuf::from(".flow-like-standalone"));
    ensure!(
        !selected.as_os_str().is_empty(),
        "Standalone state directory must not be empty"
    );
    Ok(selected)
}

fn read_env_state_dir(path: &Path, optional: bool) -> Result<Option<PathBuf>> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = match options.open(path) {
        Err(error) if optional && error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        result => result.context("Open standalone env file")?,
    };
    let metadata = file.metadata().context("Inspect standalone env file")?;
    ensure!(
        metadata.is_file(),
        "Standalone env file must be a regular file"
    );
    ensure!(
        metadata.len() <= MAX_ENV_FILE_BYTES,
        "Standalone env file exceeds the size limit"
    );
    let mut bytes = Zeroizing::new(Vec::new());
    file.take(MAX_ENV_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .context("Read standalone env file")?;
    ensure!(
        bytes.len() as u64 <= MAX_ENV_FILE_BYTES,
        "Standalone env file exceeds the size limit"
    );
    let mut state = None;
    for entry in dotenvy::from_read_iter(bytes.as_slice()) {
        // Parser errors may contain an unrelated secret from the offending line.
        let (key, value) =
            entry.map_err(|_| anyhow::anyhow!("Invalid standalone env file syntax"))?;
        if key == STATE_DIRECTORY_VARIABLE {
            ensure!(
                state.is_none(),
                "Standalone state directory is repeated in the env file"
            );
            ensure!(
                !value.is_empty(),
                "Standalone state directory must not be empty"
            );
            let value = PathBuf::from(value);
            state = Some(if value.is_absolute() {
                value
            } else {
                let file_path = std::path::absolute(path)?;
                file_path
                    .parent()
                    .context("Standalone env file needs a parent directory")?
                    .join(value)
            });
        }
    }
    Ok(state)
}

async fn validate_for_install(config: &PlacementConfig) -> Result<()> {
    #[cfg(feature = "runtime")]
    {
        flow_like_standalone::runtime::validate(config).await
    }
    #[cfg(not(feature = "runtime"))]
    {
        let _ = config;
        anyhow::bail!("Placement validation requires a binary built with the runtime feature")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_reader_never_mutates_process_environment() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("device.env");
        let marker = format!("FLOW_LIKE_UNRELATED_TEST_{}", uuid::Uuid::new_v4().simple());
        let previous_state = std::env::var_os(STATE_DIRECTORY_VARIABLE);
        assert!(std::env::var_os(&marker).is_none());
        std::fs::write(
            &path,
            format!("{marker}=unrelated-value\n{STATE_DIRECTORY_VARIABLE}=file-state\n"),
        )?;
        assert_eq!(
            read_env_state_dir(&path, false)?,
            Some(directory.path().join("file-state"))
        );
        assert!(std::env::var_os(marker).is_none());
        assert_eq!(std::env::var_os(STATE_DIRECTORY_VARIABLE), previous_state);
        assert!(!directory.path().join("file-state").exists());
        Ok(())
    }
}
