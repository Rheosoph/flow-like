#![cfg(all(unix, feature = "runtime"))]

use anyhow::{Context, Result, ensure};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use flow_like_device_protocol::*;
use flow_like_runtime::{
    app::App,
    bit::Metadata,
    flow::{
        board::Board,
        event::{Event, EventExecutionMode, EventExposure},
        pin::ValueType,
        variable::{Variable, VariableType},
    },
    state::{FlowLikeConfig, FlowLikeState, FlowNodeRegistryInner},
    utils::http::HTTPClient,
};
use flow_like_standalone::{
    config::{EventBinding, HostingConfig, PlacementConfig, ProjectSource, RestartPolicy},
    crypto::noise,
    enrollment::{DeviceSession, unix_time},
    management::{ManagementConnection, ManagementService},
    project_artifacts,
    state::{ObservedState, RegistrationRecord, StateStore},
    supervisor, vault,
};
use flow_like_storage::{
    Path as StorePath,
    files::store::{FlowLikeStore, local_store::LocalObjectStore},
};
use rand_core::{OsRng, RngCore};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap},
    fs::File,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Arc,
    time::{Duration, SystemTime},
};
use tempfile::TempDir;
use uuid::Uuid;

const SERVICE_TOKEN: &str = "rollout-service-access-token-32-characters";
const VARIABLE_SECRET: &str = "private-device-variable-survives-rollout";
const ROLLOUT_DEADLINE_SECONDS: u64 = 90;

// A parallel fork briefly inherits another test's artifact lock until exec.
// Keep fixture imports outside that window without serializing service tests.
static PROCESS_START_GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn artifact_io<T>(operation: impl FnOnce() -> Result<T>) -> Result<T> {
    let _guard = PROCESS_START_GATE.lock().unwrap();
    operation()
}

struct Agent {
    child: Child,
    process_group: i32,
    root: PathBuf,
}

impl Agent {
    fn start(root: &Path) -> Result<Self> {
        let _guard = PROCESS_START_GATE.lock().unwrap();
        let log = File::options()
            .create(true)
            .append(true)
            .open(root.join("agent-test.log"))?;
        let child = Command::new(env!("CARGO_BIN_EXE_flow-like-standalone"))
            .args([
                "--state-dir",
                root.to_str().context("UTF-8 test path")?,
                "run",
            ])
            .env("RUST_LOG", "warn")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(log)
            .process_group(0)
            .spawn()?;
        let process_group = child.id() as i32;
        Ok(Self {
            child,
            process_group,
            root: root.into(),
        })
    }

    async fn stop(&mut self) -> Result<()> {
        ensure!(
            self.child.try_wait()?.is_none(),
            "agent exited before shutdown"
        );
        ensure!(
            unsafe { libc::kill(self.child.id() as i32, libc::SIGTERM) } == 0,
            "signal test agent"
        );
        wait_for("agent graceful shutdown", || {
            Ok(self.child.try_wait()?.is_some())
        })
        .await?;
        ensure!(
            self.child
                .try_wait()?
                .context("agent exit status")?
                .success()
        );
        self.process_group = 0;
        Ok(())
    }

    async fn crash(&mut self) -> Result<()> {
        let workers = self.worker_groups()?;
        ensure!(
            unsafe { libc::kill(-self.process_group, libc::SIGKILL) } == 0,
            "terminate isolated test process group"
        );
        for group in workers {
            unsafe { libc::kill(-group, libc::SIGKILL) };
        }
        wait_for("agent crash exit", || Ok(self.child.try_wait()?.is_some())).await?;
        self.process_group = 0;
        Ok(())
    }

    fn worker_groups(&self) -> Result<Vec<i32>> {
        Ok(StateStore::open(&self.root.join("management.sqlite"))?
            .list_placements()?
            .into_iter()
            .flat_map(|placement| placement.replicas)
            .filter_map(|replica| replica.process_id.map(|pid| pid as i32))
            .collect())
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        // Workloads have separate process groups from their supervisor. Clean up all groups
        // owned by this fixture if an assertion interrupts graceful shutdown.
        if self.process_group != 0 {
            let workers = self.worker_groups().unwrap_or_default();
            unsafe { libc::kill(-self.process_group, libc::SIGKILL) };
            for group in workers {
                unsafe { libc::kill(-group, libc::SIGKILL) };
            }
        }
        let _ = self.child.wait();
    }
}

struct Controller {
    root: PathBuf,
    device_id: String,
    service: Arc<ManagementService>,
    connection: ManagementConnection,
    session: noise::Session,
}

impl Controller {
    async fn connect(root: &Path, signer: &SigningKey, peer: [u8; 32]) -> Result<Self> {
        let device = Arc::new(DeviceSession::load(root)?.context("enrolled fixture")?);
        let device_id = device.manifest().device_id.clone();
        let service = ManagementService::new(root.into(), device, "rollout-test-boot".into());
        let now = unix_time()?;
        service.refresh_authority(now + 300)?;
        let session_id = Uuid::new_v4().to_string();
        let private = [37; 32];
        let certificate = sign_controller_certificate(
            &ControllerCertificate {
                version: 1,
                device_id: device_id.clone(),
                grant_id: "owner".into(),
                session_id: session_id.clone(),
                management_key: x25519_dalek::x25519(private, x25519_dalek::X25519_BASEPOINT_BYTES),
                issued_at: now,
                expires_at: now + 300,
            },
            signer,
        )?;
        let mut connection = service.connect(&certificate, "owner", &session_id)?;
        let mut handshake = noise::Handshake::initiator(&private, peer, &device_id, &session_id)?;
        let first = handshake.write()?;
        handshake.read(&connection.receive(&first).await?)?;
        let ready = connection.receive(&handshake.write()?).await?;
        let mut session = handshake.finish()?;
        let ready: Value = serde_json::from_slice(&session.decrypt(&ready)?)?;
        ensure!(ready["ready"] == true && ready["device_id"] == device_id);
        Ok(Self {
            root: root.into(),
            device_id,
            service,
            connection,
            session,
        })
    }

    fn request(&self, command: ManagementCommand) -> Result<ManagementRequest> {
        let now = unix_time()?;
        Ok(ManagementRequest {
            operation_id: Uuid::new_v4().to_string(),
            device_id: self.device_id.clone(),
            issued_at: now,
            expires_at: now + 60,
            command,
        })
    }

    async fn transmit(&mut self, request: &ManagementRequest) -> Result<ManagementResponse> {
        self.service.refresh_authority(unix_time()? + 300)?;
        let encrypted = self.session.encrypt(&serde_json::to_vec(request)?)?;
        let response = self.connection.receive(&encrypted).await?;
        let response: ManagementResponse =
            serde_json::from_slice(&self.session.decrypt(&response)?)?;
        let expected = match &request.command {
            ManagementCommand::Operation { operation_id } => operation_id,
            _ => &request.operation_id,
        };
        ensure!(response.operation_id == *expected);
        Ok(response)
    }

    async fn command(&mut self, command: ManagementCommand) -> Result<ManagementResponse> {
        let request = self.request(command)?;
        let response = self.transmit(&request).await?;
        ensure!(
            !matches!(response.state.as_str(), "failed" | "rejected"),
            "command failed: {}",
            response.result
        );
        Ok(response)
    }

    async fn operation(&mut self, id: &str) -> Result<ManagementResponse> {
        self.command(ManagementCommand::Operation {
            operation_id: id.into(),
        })
        .await
    }

    async fn wire(&mut self, command: Value) -> Result<ManagementResponse> {
        self.command(serde_json::from_value(command)?).await
    }

    async fn stage(
        &mut self,
        config: &PlacementConfig,
        expected_revision: u64,
        stabilization_seconds: u64,
    ) -> Result<String> {
        let request = self.request(serde_json::from_value(json!({
            "type":"stage_rollout", "config":config,
            "expected_revision":expected_revision,
            "stabilization_seconds":stabilization_seconds, "deadline_seconds":ROLLOUT_DEADLINE_SECONDS
        }))?)?;
        let response = self.transmit(&request).await?;
        ensure!(
            response.result["state"] == "staged",
            "staging failed: {}",
            serde_json::to_string(&response)?
        );
        let duplicate = self.transmit(&request).await?;
        assert_eq!(
            serde_json::to_value(&response)?,
            serde_json::to_value(duplicate)?
        );
        assert_eq!(
            self.operation(&request.operation_id).await?.result,
            response.result
        );
        Ok(response.result["rollout_id"]
            .as_str()
            .context("rollout ID")?
            .to_owned())
    }

    async fn activate(&mut self, rollout: &str) -> Result<()> {
        let response = self
            .wire(json!({"type":"activate_rollout","rollout_id":rollout}))
            .await?;
        assert_eq!(response.result["state"], "validating");
        Ok(())
    }

    async fn wait_rollout(&mut self, rollout: &str, expected: &str) -> Result<Value> {
        let mut last_status = None;
        // Rollback receives a fresh deadline after the candidate fails. The test
        // must allow both bounded phases, including worker drain and preparation.
        let phases = if expected == "rolled_back" { 2 } else { 1 };
        let budget = Duration::from_secs(ROLLOUT_DEADLINE_SECONDS * phases + 15);
        let result = tokio::time::timeout(budget, async {
            loop {
                let response = self
                    .wire(json!({"type":"rollout","rollout_id":rollout}))
                    .await?;
                let state = response.result["state"].as_str().context("rollout state")?;
                if state == expected {
                    return Ok(response.result);
                }
                ensure!(
                    !matches!(state, "failed" | "cancelled" | "healthy" | "rolled_back"),
                    "unexpected rollout terminal state: {}",
                    response.result
                );
                last_status = Some(response.result);
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await;
        result.with_context(|| {
            let placements = StateStore::open(&self.root.join("management.sqlite"))
                .and_then(|store| store.list_placements())
                .map(|placements| placements.into_iter().map(|placement| json!({
                    "id": placement.id,
                    "config_revision": placement.config_revision,
                    "intent_revision": placement.intent_revision,
                    "desired_state": placement.desired_state,
                    "observed_state": placement.observed_state,
                    "last_error": placement.last_error,
                    "replicas": placement.replicas,
                })).collect::<Vec<_>>());
            let logs = std::fs::read_to_string(self.root.join("agent-test.log")).unwrap_or_default();
            let tail = logs.lines().rev().take(20).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n");
            format!("rollout did not reach {expected}; last status: {last_status:?}; placements: {placements:?}; recent agent log:\n{tail}")
        })?
    }
}

struct Fixture {
    directory: TempDir,
    root: PathBuf,
    owner: SigningKey,
    peer: [u8; 32],
    port: u16,
}

impl Fixture {
    fn new() -> Result<Self> {
        Self::with_api_base("http://127.0.0.1:9/api/v1")
    }

    fn with_api_base(api_base_url: &str) -> Result<Self> {
        let directory = tempfile::tempdir()?;
        let root = supervisor::prepare_state_dir(&directory.path().join("device"))?;
        let mut store = StateStore::open(&root.join("management.sqlite"))?;
        let owner = SigningKey::generate();
        let bootstrap = SigningKey::generate();
        let auth = SigningKey::generate();
        let telemetry = SigningKey::generate();
        let mut private = [0; 32];
        OsRng.fill_bytes(&mut private);
        let peer = x25519_dalek::x25519(private, x25519_dalek::X25519_BASEPOINT_BYTES);
        let mut keys = Vec::with_capacity(96);
        keys.extend_from_slice(&auth.to_bytes());
        keys.extend_from_slice(&telemetry.to_bytes());
        keys.extend_from_slice(&private);
        vault::write_new_private(&root.join("device.keys"), &keys)?;
        let now = unix_time()?;
        let manifest = OnboardingManifest {
            version: 1,
            enrollment_id: Uuid::new_v4().to_string(),
            device_id: Uuid::new_v4().to_string(),
            owner_id: "rollout-owner".into(),
            name: "Rollout integration fixture".into(),
            api_base_url: api_base_url.into(),
            bootstrap_key: bootstrap.public_key(),
            controller_key: owner.public_key(),
            owner_invitation_key: SigningKey::generate().public_key(),
            issued_at: now,
            expires_at: now + 3600,
        };
        let manifest_jws = sign_manifest(&manifest, &owner)?;
        let identity = DeviceIdentity {
            auth_key: auth.public_key(),
            telemetry_key: telemetry.public_key(),
            management_key: peer,
        };
        let binding_jws = sign_binding(
            &EnrollmentBinding {
                version: 1,
                enrollment_id: manifest.enrollment_id.clone(),
                device_id: manifest.device_id.clone(),
                identity: identity.clone(),
                manifest_digest: compact_digest(&manifest_jws),
                challenge_id: Uuid::new_v4().to_string(),
                challenge_nonce: Uuid::new_v4().to_string(),
                issued_at: now,
                expires_at: now + 60,
            },
            &bootstrap,
        )?;
        let receipt = DeviceReceipt {
            enrollment_id: manifest.enrollment_id.clone(),
            device_id: manifest.device_id.clone(),
            owner_id: manifest.owner_id.clone(),
            name: manifest.name.clone(),
            identity,
            manifest_jws: manifest_jws.clone(),
            binding_jws: binding_jws.clone(),
            registered_at: now,
            auth_epoch: 1,
        };
        store.begin_registration(&RegistrationRecord {
            manifest,
            manifest_jws,
            binding_jws: Some(binding_jws),
            attempted_bindings: vec![],
            receipt: Some(receipt),
            last_contact_at: None,
            connection_status: "active".into(),
        })?;
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        Ok(Self {
            directory,
            root,
            owner,
            peer,
            port,
        })
    }

    fn store(&self) -> Result<StateStore> {
        StateStore::open(&self.root.join("management.sqlite"))
    }

    async fn controller(&self) -> Result<Controller> {
        Controller::connect(&self.root, &self.owner, self.peer).await
    }

    async fn project(&self, version: u32, route: &str) -> Result<PlacementConfig> {
        self.project_with_response_variable(version, route, None)
            .await
    }

    async fn project_with_response_variable(
        &self,
        version: u32,
        route: &str,
        variable: Option<&str>,
    ) -> Result<PlacementConfig> {
        let source = self.directory.path().join(format!("source-{version}"));
        let source = supervisor::prepare_state_dir(&source)?;
        let local = || -> Result<FlowLikeStore> {
            Ok(FlowLikeStore::Local(Arc::new(LocalObjectStore::new(
                source.clone(),
            )?)))
        };
        let mut config = FlowLikeConfig::new();
        config.register_app_meta_store(local()?);
        config.register_app_storage_store(local()?);
        config.register_user_store(local()?);
        config.register_temporary_store(local()?);
        config.register_log_store(local()?);
        config.register_bits_store(local()?);
        let state = Arc::new(FlowLikeState::new(
            config,
            HTTPClient::new_without_refetch(),
        ));
        flow_like_catalog::initialize();
        let catalog = Arc::new(flow_like_catalog::get_catalog());
        {
            let mut registry = state.node_registry.write().await;
            registry.initialize(Arc::downgrade(&state));
            registry.node_registry = Arc::new(FlowNodeRegistryInner::prepare(&catalog));
        }
        let app = App::new(
            Some("project".into()),
            Metadata::default(),
            vec![],
            state.clone(),
        )
        .await?;
        let mut board = Board::new(Some("board".into()), StorePath::from("apps/project"), state);
        let catalog_node = |name: &str| {
            catalog
                .iter()
                .map(|logic| logic.get_node())
                .find(|node| node.name == name)
                .with_context(|| format!("catalog node {name}"))
        };
        let mut entry = catalog_node("events_generic")?;
        entry.id = "entry".into();
        let mut result = catalog_node("events_generic_return_result")?;
        result.id = "result".into();
        let from = entry
            .pins
            .values()
            .find(|p| p.name == "exec_out")
            .unwrap()
            .id
            .clone();
        let to = result
            .pins
            .values()
            .find(|p| p.name == "exec_in")
            .unwrap()
            .id
            .clone();
        entry
            .pins
            .get_mut(&from)
            .unwrap()
            .connected_to
            .insert(to.clone());
        result.pins.get_mut(&to).unwrap().depends_on.insert(from);
        let response = result
            .pins
            .values_mut()
            .find(|p| p.name == "response")
            .unwrap();
        response.default_value = Some(serde_json::to_vec(&json!({"revision":version}))?);
        if let Some(variable) = variable {
            let mut getter = catalog_node("variable_get")?;
            getter.id = "get-device-setting".into();
            getter
                .get_pin_mut_by_name("var_ref")
                .context("variable reference")?
                .set_default_value(Some(json!(variable)));
            let output = getter
                .get_pin_mut_by_name("value_ref")
                .context("variable output")?;
            output.data_type = VariableType::String;
            output.connected_to.insert(response.id.clone());
            response.depends_on.insert(output.id.clone());
            response.data_type = VariableType::String;
            response.default_value = None;
            board.nodes.insert(getter.id.clone(), getter);
        }
        board.nodes.insert(entry.id.clone(), entry);
        board.nodes.insert(result.id.clone(), result);
        let mut secret = Variable::new("Private setting", VariableType::String, ValueType::Normal);
        secret.id = "credential".into();
        secret.secret = true;
        secret.runtime_configured = true;
        board.variables.insert(secret.id.clone(), secret);
        let mut public = Variable::new("Device setting", VariableType::String, ValueType::Normal);
        public.id = "device-setting".into();
        public.exposed = true;
        board.variables.insert(public.id.clone(), public);
        board.snapshot_at_version((version, 0, 0), None).await?;
        let now = SystemTime::now();
        let event = Event {
            id: "http".into(),
            name: format!("Revision {version}"),
            description: String::new(),
            board_id: board.id.clone(),
            board_version: Some((version, 0, 0)),
            node_id: "entry".into(),
            variables: HashMap::new(),
            config: serde_json::to_vec(&json!({"path":route,"method":"POST"}))?,
            active: true,
            canary: None,
            variants: vec![],
            priority: 0,
            event_type: "http".into(),
            notes: None,
            event_version: (version, 0, 0),
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
        event.save(&app, Some((version, 0, 0))).await?;
        app.save().await?;
        let receipt = artifact_io(|| {
            project_artifacts::import_local(&self.store()?, &self.root, "project", &source)
        })?;
        let revision = receipt.descriptor.manifest_sha256;
        Ok(PlacementConfig {
            id: "api".into(),
            project_id: "project".into(),
            deployment_id: "deployment".into(),
            project_path: project_artifacts::managed_revision(&self.root, "project", &revision)?,
            revision,
            source: ProjectSource::Offline,
            online_metadata_sha256: None,
            events: vec![EventBinding {
                event_id: "http".into(),
                event_version: [version, 0, 0],
                board_version: [version, 0, 0],
            }],
            artifact_pins: vec![],
            package_pins: vec![],
            bit_pins: vec![],
            tls_certificate_id: None,
            hosting: Some(HostingConfig {
                host: "127.0.0.1".parse()?,
                port: self.port,
                max_in_flight: 4,
                request_timeout_secs: 5,
                auth_secret: "service-token".into(),
            }),
            max_replicas: 2,
            variables: BTreeMap::from([("device-setting".into(), json!("device-specific"))]),
            secret_overrides: BTreeMap::from([("credential".into(), "credential-secret".into())]),
            resource_grant: None,
            offline_writes: None,
            resources: None,
            restart: RestartPolicy {
                initial_backoff_secs: 1,
                max_backoff_secs: 1,
                max_restarts: 0,
            },
        })
    }

    async fn catalog_service(
        &self,
        version: u32,
        kind: &str,
        port: u16,
        exits: bool,
    ) -> Result<PlacementConfig> {
        let mut placement = self.project(version, "/unused").await?;
        let source = self.directory.path().join(format!("source-{version}"));
        let mut configuration = FlowLikeConfig::new();
        configuration.register_app_meta_store(FlowLikeStore::Local(Arc::new(
            LocalObjectStore::new(source.clone())?,
        )));
        let state = Arc::new(FlowLikeState::new(
            configuration,
            HTTPClient::new_without_refetch(),
        ));
        let app = App::load("project".into(), state.clone()).await?;
        let mut board = Board::new(
            Some("service-board".into()),
            StorePath::from("apps/project"),
            state,
        );
        let catalog = flow_like_catalog::get_catalog();
        let mut insert = |name: &str, id: &str| -> Result<()> {
            let mut node = catalog
                .iter()
                .map(|logic| logic.get_node())
                .find(|node| node.name == name)
                .with_context(|| format!("catalog node {name}"))?;
            node.id = id.into();
            board.nodes.insert(id.into(), node);
            Ok(())
        };
        let entry;
        if kind == "daemon" {
            insert("service_ready", "ready")?;
            if !exits {
                insert("delay", "work")?;
            }
            entry = "ready";
            if !exits {
                board
                    .nodes
                    .get_mut("work")
                    .unwrap()
                    .get_pin_mut_by_name("time")
                    .unwrap()
                    .set_default_value(Some(json!(600000.0)));
                connect_catalog(&mut board, "ready", "exec_out", "work", "exec_in")?;
            }
        } else {
            insert(&format!("{kind}_server_config"), "config")?;
            insert(&format!("{kind}_server"), "server")?;
            entry = "server";
            let config = board.nodes.get_mut("config").unwrap();
            config
                .get_pin_mut_by_name("host")
                .unwrap()
                .set_default_value(Some(json!("127.0.0.1")));
            config
                .get_pin_mut_by_name("port")
                .unwrap()
                .set_default_value(Some(json!(port)));
            connect_catalog(&mut board, "config", "config", "server", "config")?;
        }
        board.snapshot_at_version((version, 0, 0), None).await?;
        let mut event = app.get_event("http", Some((version, 0, 0))).await?;
        // Keep the native HTTP fixture's archived version immutable.
        event.id = "service".into();
        event.node_id = entry.into();
        event.event_type = kind.into();
        event.board_id = board.id;
        event.config.clear();
        event.save(&app, Some((version, 0, 0))).await?;
        let receipt = artifact_io(|| {
            project_artifacts::import_local(&self.store()?, &self.root, "project", &source)
        })?;
        placement.revision = receipt.descriptor.manifest_sha256;
        placement.project_path =
            project_artifacts::managed_revision(&self.root, "project", &placement.revision)?;
        placement.events[0].event_id = event.id;
        placement.hosting = None;
        placement.variables.clear();
        placement.secret_overrides.clear();
        placement.max_replicas = 1;
        Ok(placement)
    }

    async fn wait_ready(&self, revision: u64) -> Result<()> {
        let result = wait_for("placement readiness", || {
            Ok(self.store()?.get_placement("api")?.is_some_and(|record| {
                record.observed_state == ObservedState::Running
                    && record.applied_revision == Some(revision)
                    && record.ready_replicas == record.desired_replicas
                    && record.ready_replicas > 0
            }))
        })
        .await;
        result.with_context(|| {
            let logs =
                std::fs::read_to_string(self.root.join("agent-test.log")).unwrap_or_default();
            let tail = logs
                .lines()
                .rev()
                .take(20)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join("\n");
            format!("agent did not make revision {revision} ready; recent fixture log:\n{tail}")
        })
    }

    async fn request(&self, route: &str, token: Option<&str>) -> Result<reqwest::Response> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()?;
        let mut request = client
            .post(format!("http://127.0.0.1:{}{route}", self.port))
            .json(&json!({}));
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        Ok(request.send().await?)
    }

    async fn assert_serving(&self, route: &str, version: u32) -> Result<()> {
        self.assert_serving_with_token(route, version, SERVICE_TOKEN)
            .await
    }

    async fn assert_serving_with_token(
        &self,
        route: &str,
        version: u32,
        token: &str,
    ) -> Result<()> {
        for token in [None, Some("incorrect-service-token")] {
            assert_eq!(
                self.request(route, token).await?.status(),
                reqwest::StatusCode::UNAUTHORIZED
            );
        }
        let response = self.request(route, Some(token)).await?;
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        assert_eq!(response.json::<Value>().await?, json!({"revision":version}));
        Ok(())
    }

    fn assert_preserved(&self, expected: &PlacementConfig, marker: &Path) -> Result<()> {
        let record = self.store()?.get_placement("api")?.context("placement")?;
        let actual: PlacementConfig = serde_json::from_value(record.config)?;
        assert_eq!(&actual, expected);
        assert_eq!(
            std::fs::read(marker)?,
            b"mutable data written after initial deployment"
        );
        let secrets = actual.project_path.join(".secrets/api");
        assert_eq!(
            vault::read_private(&secrets.join("service-token.secret"))?.as_slice(),
            SERVICE_TOKEN.as_bytes()
        );
        assert_eq!(
            vault::read_private(&secrets.join("credential-secret.secret"))?.as_slice(),
            serde_json::to_vec(VARIABLE_SECRET)?
        );
        Ok(())
    }
}

fn connect_catalog(
    board: &mut Board,
    from: &str,
    output: &str,
    to: &str,
    input: &str,
) -> Result<()> {
    let output = board.nodes[from]
        .get_pin_by_name(output)
        .context("output pin")?
        .id
        .clone();
    let input = board.nodes[to]
        .get_pin_by_name(input)
        .context("input pin")?
        .id
        .clone();
    flow_like_runtime::flow::board::commands::pins::connect_pins::connect_pins(
        board, from, &output, to, &input,
    )?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires loopback listeners and spawning the standalone runtime binary"]
async fn catalog_rest_mcp_and_daemon_rollouts_wait_for_readiness_and_rollback() -> Result<()> {
    for kind in ["rest", "mcp", "daemon"] {
        let fixture = Fixture::new()?;
        let initial = fixture
            .catalog_service(1, kind, fixture.port, false)
            .await?;
        let mut controller = fixture.controller().await?;
        controller
            .command(ManagementCommand::Apply {
                config: serde_json::to_value(&initial)?,
                expected_revision: 0,
                start: true,
            })
            .await?;
        let mut agent = Agent::start(&fixture.root)?;
        fixture
            .wait_ready(1)
            .await
            .with_context(|| format!("initial {kind}"))?;
        let original_pid = fixture.store()?.get_placement("api")?.unwrap().replicas[0].process_id;
        if kind != "daemon" {
            assert_catalog_listener(fixture.port).await?;
        }
        let candidate = fixture
            .catalog_service(2, kind, fixture.port, false)
            .await?;
        let rollout = controller.stage(&candidate, 1, 2).await?;
        controller.activate(&rollout).await?;
        controller
            .wait_rollout(&rollout, "healthy")
            .await
            .with_context(|| format!("healthy {kind} rollout"))?;
        fixture.wait_ready(2).await?;
        assert_ne!(
            fixture.store()?.get_placement("api")?.unwrap().replicas[0].process_id,
            original_pid
        );
        if kind != "daemon" {
            assert_catalog_listener(fixture.port).await?;
        }

        // Metadata validation succeeds, then the listener cannot bind or the daemon
        // returns immediately. Neither can survive the stabilization interval.
        let occupied = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let failing = fixture
            .catalog_service(3, kind, occupied.local_addr()?.port(), true)
            .await?;
        let rollout = controller.stage(&failing, 2, 3).await?;
        controller.activate(&rollout).await?;
        let result = controller
            .wait_rollout(&rollout, "rolled_back")
            .await
            .with_context(|| format!("failed {kind} rollback"))?;
        assert_eq!(
            result["failure_code"], "candidate_failed",
            "{kind} must fail promptly rather than exhaust its activation deadline"
        );
        fixture.wait_ready(4).await?;
        let restored: PlacementConfig =
            serde_json::from_value(fixture.store()?.get_placement("api")?.unwrap().config)?;
        assert_eq!(restored, candidate);
        if kind != "daemon" {
            assert_catalog_listener(fixture.port).await?;
        }

        let next = fixture
            .catalog_service(4, kind, fixture.port, false)
            .await?;
        let rollout = controller.stage(&next, 4, 20).await?;
        controller.activate(&rollout).await?;
        fixture.wait_ready(5).await?;
        controller
            .command(ManagementCommand::Stop {
                placement_id: "api".into(),
                expected_revision: 5,
            })
            .await?;
        controller.wait_rollout(&rollout, "cancelled").await?;
        wait_for("stopped catalog worker", || {
            Ok(fixture
                .store()?
                .get_placement("api")?
                .is_some_and(|record| {
                    record.observed_state == ObservedState::Stopped
                        && record
                            .replicas
                            .iter()
                            .all(|replica| replica.process_id.is_none())
                }))
        })
        .await?;
        if kind != "daemon" {
            assert!(
                tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, fixture.port))
                    .await
                    .is_err()
            );
        }
        agent.stop().await?;
    }
    Ok(())
}

async fn assert_catalog_listener(port: u16) -> Result<()> {
    let response = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()?
        .get(format!("http://127.0.0.1:{port}/nonexistent-route"))
        .send()
        .await?;
    ensure!(
        response.status() == reqwest::StatusCode::NOT_FOUND,
        "catalog listener did not serve its route response"
    );
    Ok(())
}

async fn open_mcp_session(port: u16) -> Result<String> {
    let response = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()?
        .post(format!("http://127.0.0.1:{port}/mcp"))
        .header("accept", "application/json")
        .json(&json!({
            "jsonrpc":"2.0", "id":1, "method":"initialize",
            "params":{"protocolVersion":"2025-06-18", "capabilities":{},
                "clientInfo":{"name":"standalone-lifecycle-test","version":"1"}},
        }))
        .send()
        .await?
        .error_for_status()?;
    let session = response
        .headers()
        .get("mcp-session-id")
        .context("MCP session header")?
        .to_str()?
        .to_owned();
    let initialized: Value = response.json().await?;
    assert_eq!(initialized["result"]["protocolVersion"], "2025-06-18");
    Ok(session)
}

async fn assert_mcp_session(port: u16, session: &str) -> Result<()> {
    let response: Value = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()?
        .post(format!("http://127.0.0.1:{port}/mcp"))
        .header("accept", "application/json")
        .header("mcp-session-id", session)
        .header("mcp-protocol-version", "2025-06-18")
        .json(&json!({"jsonrpc":"2.0", "id":2, "method":"tools/list"}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(response["id"], 2);
    assert_eq!(response["result"]["tools"], json!([]));
    Ok(())
}

async fn wait_for(label: &str, mut predicate: impl FnMut() -> Result<bool>) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if predicate()? {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .with_context(|| format!("timed out waiting for {label}"))?
}

// This harness exercises the shipped executable, SQLite, child IPC and catalog runtime.
// The management frames use real certificates and Noise over an in-process ordered transport;
// signaling, WebRTC connectivity and host service installation have their own acceptance tests.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires loopback listeners and spawning two standalone runtime binaries"]
async fn two_devices_keep_project_variables_data_and_management_independent() -> Result<()> {
    async fn publish(controller: &mut Controller, name: &str, value: String) -> Result<()> {
        let response = controller
            .command(ManagementCommand::SetSecret {
                placement_id: "api".into(),
                expected_revision: 1,
                name: name.into(),
                value: SecretValue(value),
            })
            .await?;
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let operation = controller.operation(&response.operation_id).await?;
                if operation.state == "completed" {
                    return Ok::<_, anyhow::Error>(());
                }
                ensure!(
                    !matches!(operation.state.as_str(), "failed" | "rejected"),
                    "secret publication failed"
                );
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .context("device secret publication timeout")??;
        Ok(())
    }
    async fn serving(fixture: &Fixture, token: &str, expected: &str) -> Result<()> {
        let response = fixture.request("/device", Some(token)).await?;
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        assert_eq!(response.json::<Value>().await?, json!(expected));
        Ok(())
    }

    let first = Fixture::new()?;
    let mut second = Fixture::new()?;
    if second.port == first.port {
        let reservation = std::net::TcpListener::bind(("127.0.0.1", first.port))?;
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        second.port = listener.local_addr()?.port();
        drop(reservation);
    }
    let mut a = first
        .project_with_response_variable(1, "/device", Some("device-setting"))
        .await?;
    // Import the exact same source snapshot on the second device. Only placement
    // settings and device-local mutable data may differ between these instances.
    let imported = artifact_io(|| {
        project_artifacts::import_local(
            &second.store()?,
            &second.root,
            "project",
            &first.directory.path().join("source-1"),
        )
    })?;
    assert_eq!(imported.descriptor.manifest_sha256, a.revision);
    let mut b = a.clone();
    b.project_path = project_artifacts::managed_revision(&second.root, "project", &b.revision)?;
    b.hosting.as_mut().unwrap().port = second.port;
    a.variables
        .insert("device-setting".into(), json!("plant-a"));
    b.variables
        .insert("device-setting".into(), json!("plant-b"));
    a.max_replicas = 1;
    b.max_replicas = 1;
    assert_ne!(first.owner.public_key(), second.owner.public_key());
    assert_ne!(first.peer, second.peer);
    assert!(
        Controller::connect(&second.root, &first.owner, second.peer)
            .await
            .is_err()
    );
    assert!(
        Controller::connect(&first.root, &second.owner, first.peer)
            .await
            .is_err()
    );
    let mut first_controller = first.controller().await?;
    let mut second_controller = second.controller().await?;
    assert_ne!(first_controller.device_id, second_controller.device_id);
    for (controller, config) in [(&mut first_controller, &a), (&mut second_controller, &b)] {
        let response = controller
            .command(ManagementCommand::Apply {
                config: serde_json::to_value(config)?,
                expected_revision: 0,
                start: false,
            })
            .await?;
        assert_eq!(response.result["config_revision"], 1);
    }
    let mut first_agent = Agent::start(&first.root)?;
    let mut second_agent = Agent::start(&second.root)?;
    let first_token = "device-a-service-token-distinct-32-characters";
    let second_token = "device-b-service-token-distinct-32-characters";
    for (controller, token, secret) in [
        (&mut first_controller, first_token, "private-a"),
        (&mut second_controller, second_token, "private-b"),
    ] {
        publish(controller, "service-token", token.into()).await?;
        publish(
            controller,
            "credential-secret",
            serde_json::to_string(secret)?,
        )
        .await?;
        controller
            .command(ManagementCommand::Start {
                placement_id: "api".into(),
                expected_revision: 1,
            })
            .await?;
    }
    tokio::try_join!(first.wait_ready(1), second.wait_ready(1))?;
    serving(&first, first_token, "plant-a").await?;
    serving(&second, second_token, "plant-b").await?;
    assert_eq!(
        first.request("/device", Some(second_token)).await?.status(),
        reqwest::StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        second.request("/device", Some(first_token)).await?.status(),
        reqwest::StatusCode::UNAUTHORIZED
    );
    let mut markers = Vec::new();
    for (fixture, contents) in [
        (&first, b"local-a".as_slice()),
        (&second, b"local-b".as_slice()),
    ] {
        let data = supervisor::prepare_state_dir(
            &fixture
                .root
                .join("placement-data/api/current/store/apps/project/files"),
        )?;
        let marker = data.join("same-filename.txt");
        vault::write_new_private(&marker, contents)?;
        markers.push(marker);
    }
    let second_processes = second_agent.worker_groups()?;
    assert!(!second_processes.is_empty());
    let stopped = tokio_util::sync::CancellationToken::new();
    tokio::try_join!(
        async {
            first_agent.stop().await?;
            stopped.cancel();
            Ok::<_, anyhow::Error>(())
        },
        async {
            loop {
                serving(&second, second_token, "plant-b").await?;
                if stopped.is_cancelled() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Ok::<_, anyhow::Error>(())
        }
    )?;
    drop(first_agent);
    drop(first_controller);
    assert_eq!(second_agent.worker_groups()?, second_processes);
    assert!(first.request("/device", Some(first_token)).await.is_err());
    let mut first_agent = Agent::start(&first.root)?;
    let mut first_controller = first.controller().await?;
    first.wait_ready(1).await?;
    serving(&first, first_token, "plant-a").await?;
    serving(&second, second_token, "plant-b").await?;
    assert_eq!(second_agent.worker_groups()?, second_processes);
    assert_eq!(std::fs::read(&markers[0])?, b"local-a");
    assert_eq!(std::fs::read(&markers[1])?, b"local-b");
    for (fixture, expected, controller) in [
        (&first, &a, &mut first_controller),
        (&second, &b, &mut second_controller),
    ] {
        let configuration = controller
            .command(ManagementCommand::PlacementConfiguration {
                placement_id: "api".into(),
            })
            .await?;
        let persisted: PlacementConfig =
            serde_json::from_value(configuration.result["config"].clone())?;
        assert_eq!(&persisted, expected);
        assert_eq!(
            fixture
                .store()?
                .get_placement("api")?
                .context("placement after restart")?
                .config_revision,
            1
        );
    }
    tokio::try_join!(first_agent.stop(), second_agent.stop())?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires loopback listeners and spawning the standalone runtime binary"]
async fn workflow_rollout_preserves_secrets_and_mutable_data_across_recovery() -> Result<()> {
    let fixture = Fixture::new()?;
    let initial = fixture.project(1, "/first").await?;
    assert!(
        Controller::connect(&fixture.root, &SigningKey::generate(), fixture.peer)
            .await
            .is_err()
    );
    let mut controller = fixture.controller().await?;
    let response = controller
        .command(ManagementCommand::Apply {
            config: serde_json::to_value(&initial)?,
            expected_revision: 0,
            start: false,
        })
        .await?;
    assert_eq!(response.result["config_revision"], 1);
    let mut agent = Agent::start(&fixture.root)?;
    for (name, value) in [
        ("service-token", SERVICE_TOKEN.to_owned()),
        ("credential-secret", serde_json::to_string(VARIABLE_SECRET)?),
    ] {
        let response = controller
            .command(ManagementCommand::SetSecret {
                placement_id: "api".into(),
                expected_revision: 1,
                name: name.into(),
                value: SecretValue(value),
            })
            .await?;
        let operation = response.operation_id;
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if controller.operation(&operation).await?.state == "completed" {
                    return Ok::<_, anyhow::Error>(());
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .context("secret publication timeout")??;
    }
    controller
        .command(ManagementCommand::Start {
            placement_id: "api".into(),
            expected_revision: 1,
        })
        .await?;
    fixture.wait_ready(1).await?;
    fixture.assert_serving("/first", 1).await?;
    controller
        .command(ManagementCommand::Scale {
            placement_id: "api".into(),
            expected_revision: 1,
            replicas: 2,
        })
        .await?;
    fixture.wait_ready(1).await?;
    assert_eq!(
        fixture
            .store()?
            .get_placement("api")?
            .unwrap()
            .ready_replicas,
        2
    );

    let data = fixture
        .root
        .join("placement-data/api/current/store/apps/project/files");
    let data = supervisor::prepare_state_dir(&data)?;
    let marker = data.join("customer-data.txt");
    vault::write_new_private(&marker, b"mutable data written after initial deployment")?;

    let healthy = fixture.project(2, "/second").await?;
    let stale = controller.request(serde_json::from_value(json!({
        "type":"stage_rollout", "config":healthy, "expected_revision":0,
        "stabilization_seconds":2, "deadline_seconds":90
    }))?)?;
    assert_eq!(controller.transmit(&stale).await?.state, "rejected");
    let rollout = controller.stage(&healthy, 1, 2).await?;
    // Staging must leave the current service available until activation passes preflight.
    fixture.assert_serving("/first", 1).await?;
    controller.activate(&rollout).await?;
    controller.wait_rollout(&rollout, "healthy").await?;
    fixture.wait_ready(2).await?;
    fixture.assert_serving("/second", 2).await?;
    assert_eq!(
        fixture
            .request("/first", Some(SERVICE_TOKEN))
            .await?
            .status(),
        reqwest::StatusCode::NOT_FOUND
    );
    fixture.assert_preserved(&healthy, &marker)?;

    let mut invalid = healthy.clone();
    invalid.events[0].board_version = [99, 0, 0];
    let invalid_rollout = controller.stage(&invalid, 2, 2).await?;
    let previous_pids: Vec<_> = fixture
        .store()?
        .get_placement("api")?
        .unwrap()
        .replicas
        .into_iter()
        .map(|r| r.process_id)
        .collect();
    controller.activate(&invalid_rollout).await?;
    controller.wait_rollout(&invalid_rollout, "failed").await?;
    fixture.wait_ready(2).await?;
    assert_eq!(
        fixture
            .store()?
            .get_placement("api")?
            .unwrap()
            .replicas
            .into_iter()
            .map(|r| r.process_id)
            .collect::<Vec<_>>(),
        previous_pids
    );
    fixture.assert_serving("/second", 2).await?;
    fixture.assert_preserved(&healthy, &marker)?;

    let mut missing_secret = healthy.clone();
    missing_secret
        .secret_overrides
        .insert("credential".into(), "missing-candidate-secret".into());
    let missing_secret_rollout = controller.stage(&missing_secret, 2, 2).await?;
    controller.activate(&missing_secret_rollout).await?;
    controller
        .wait_rollout(&missing_secret_rollout, "failed")
        .await?;
    fixture.wait_ready(2).await?;
    fixture.assert_serving("/second", 2).await?;
    fixture.assert_preserved(&healthy, &marker)?;

    let mut candidate = fixture.project(3, "/candidate").await?;
    let candidate_token = "candidate-service-token-for-rollback-test";
    candidate.hosting.as_mut().unwrap().auth_secret = "candidate-service".into();
    candidate
        .secret_overrides
        .insert("credential".into(), "candidate-credential".into());
    let failed_rollout = controller.stage(&candidate, 2, 20).await?;
    let overwrite_live_secret = controller.request(serde_json::from_value(json!({
        "type":"rollout_secret", "rollout_id":failed_rollout,
        "name":"service-token", "value":"replacement-must-not-touch-live-service-token"
    }))?)?;
    assert_eq!(
        controller.transmit(&overwrite_live_secret).await?.state,
        "rejected"
    );
    fixture.assert_serving("/second", 2).await?;
    for (name, value) in [
        ("candidate-service", candidate_token.to_owned()),
        (
            "candidate-credential",
            serde_json::to_string("candidate-private-setting")?,
        ),
    ] {
        controller
            .wire(json!({
                "type":"rollout_secret", "rollout_id":failed_rollout, "name":name, "value":value
            }))
            .await?;
    }
    controller.activate(&failed_rollout).await?;
    fixture.wait_ready(3).await?;
    fixture
        .assert_serving_with_token("/candidate", 3, candidate_token)
        .await?;
    assert_eq!(
        fixture
            .request("/candidate", Some(SERVICE_TOKEN))
            .await?
            .status(),
        reqwest::StatusCode::UNAUTHORIZED
    );
    let candidate_pid = fixture
        .store()?
        .get_placement("api")?
        .context("candidate placement")?
        .process_id
        .context("candidate child PID")?;
    // Kill only the ready workload from this fixture. The agent must observe the exit and
    // execute its real rollback state machine, including a new configuration revision.
    ensure!(unsafe { libc::kill(candidate_pid as i32, libc::SIGKILL) } == 0);
    controller
        .wait_rollout(&failed_rollout, "rolled_back")
        .await?;
    fixture.wait_ready(4).await?;
    fixture.assert_serving("/second", 2).await?;
    assert_eq!(
        fixture
            .request("/second", Some(candidate_token))
            .await?
            .status(),
        reqwest::StatusCode::UNAUTHORIZED
    );
    fixture.assert_preserved(&healthy, &marker)?;

    let recovered = fixture.project(4, "/recovered").await?;
    let recovered_rollout = controller.stage(&recovered, 4, 3).await?;
    controller.activate(&recovered_rollout).await?;
    fixture.wait_ready(5).await?;
    agent.crash().await?;
    drop(agent);
    drop(controller);
    let mut agent = Agent::start(&fixture.root)?;
    let mut controller = fixture.controller().await?;
    controller
        .wait_rollout(&recovered_rollout, "healthy")
        .await?;
    fixture.wait_ready(5).await?;
    fixture.assert_serving("/recovered", 4).await?;
    fixture.assert_preserved(&recovered, &marker)?;
    assert_eq!(
        controller
            .wire(json!({"type":"rollout","rollout_id":failed_rollout}))
            .await?
            .result["state"],
        "rolled_back"
    );

    let cancelled = fixture.project(5, "/cancelled").await?;
    let cancelled_rollout = controller.stage(&cancelled, 5, 20).await?;
    controller
        .command(ManagementCommand::Stop {
            placement_id: "api".into(),
            expected_revision: 5,
        })
        .await?;
    controller
        .wait_rollout(&cancelled_rollout, "cancelled")
        .await?;
    wait_for("operator stop after staged update", || {
        Ok(fixture
            .store()?
            .get_placement("api")?
            .is_some_and(|record| {
                record.observed_state == ObservedState::Stopped && record.running_replicas == 0
            }))
    })
    .await?;
    let activation = controller.request(serde_json::from_value(json!({
        "type":"activate_rollout", "rollout_id":cancelled_rollout
    }))?)?;
    assert_eq!(controller.transmit(&activation).await?.state, "rejected");
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let stopped = fixture
        .store()?
        .get_placement("api")?
        .context("stopped placement")?;
    assert_eq!(
        stopped.desired_state,
        flow_like_standalone::state::DesiredState::Stopped
    );
    assert_eq!(stopped.running_replicas, 0);
    fixture.assert_preserved(&recovered, &marker)?;
    agent.stop().await?;
    Ok(())
}

#[derive(Clone)]
struct CloudInstance {
    registration: InstanceRegistration,
    receipt: InstanceReceipt,
    retired: bool,
    resources: Vec<(String, String)>,
    project_tokens: usize,
    issued_tokens: HashMap<String, i64>,
    active_workloads_at_registration: usize,
    retired_workloads_at_registration: usize,
}

struct MockCloudServer(tokio::task::JoinHandle<std::io::Result<()>>);

impl Drop for MockCloudServer {
    fn drop(&mut self) {
        self.0.abort();
    }
}

struct MockCloud {
    status: std::sync::atomic::AtomicU16,
    grant_expires_at: i64,
    storage_lease_seconds: i64,
    project_token_seconds: i64,
    base: String,
    device_id: String,
    device_key: Ed25519PublicKey,
    documents: HashMap<String, Value>,
    instances: std::sync::Mutex<HashMap<String, CloudInstance>>,
    blocked_path: std::sync::Mutex<Option<String>>,
    blocked_entered: tokio::sync::Notify,
    release_blocked: tokio::sync::Notify,
}

impl MockCloud {
    fn snapshots(&self) -> Vec<CloudInstance> {
        self.instances.lock().unwrap().values().cloned().collect()
    }

    fn authorize_resource(
        &self,
        method: &str,
        path: &str,
        headers: &axum::http::HeaderMap,
    ) -> Result<InstanceRegistration> {
        let authorization = headers
            .get("authorization")
            .context("resource authorization")?
            .to_str()?;
        let token = authorization.strip_prefix("DPoP ").context("DPoP token")?;
        let (id, _) = token
            .strip_prefix("project-")
            .and_then(|token| token.split_once('.'))
            .context("project token generation")?;
        let mut instances = self.instances.lock().unwrap();
        let instance = instances
            .get_mut(id)
            .context("registered resource identity")?;
        ensure!(
            !instance.retired,
            "retired identity used a project resource"
        );
        let now = unix_time()?;
        ensure!(
            instance
                .issued_tokens
                .get(token)
                .is_some_and(|expiry| *expiry > now),
            "unknown or expired resource token"
        );
        let url = format!("{}{}", self.base.trim_end_matches("/api/v1"), path);
        verify_dpop(
            headers
                .get("dpop")
                .context("resource possession proof")?
                .to_str()?,
            &instance.registration.workload_key,
            &DpopContext {
                method,
                url: &url,
                access_token: Some(token),
                nonce: Some("online-rollout-test-nonce"),
                key_thumbprint: &instance.registration.workload_key.thumbprint()?,
                now: unix_time()?,
            },
        )?;
        instance.resources.push((method.into(), path.into()));
        ensure!(
            instance.registration.purpose != InstancePurpose::RolloutValidation
                || (method == "GET" && !path.contains("/storage")),
            "validation identity requested more than project metadata"
        );
        Ok(instance.registration.clone())
    }

    async fn handle(
        &self,
        method: &str,
        path: &str,
        headers: &axum::http::HeaderMap,
        body: &[u8],
    ) -> Result<Value> {
        let instances_path = format!("/api/v1/devices/{}/instances", self.device_id);
        if method == "POST" && path == instances_path {
            let request: InstanceRegistrationRequest = serde_json::from_slice(body)?;
            let endpoint = format!("{}/devices/{}/instances", self.base, self.device_id);
            let now = unix_time()?;
            let registration = verify_instance_registration(
                &request.registration_jws,
                &self.device_key,
                &self.device_id,
                &endpoint,
                now,
            )?;
            verify_instance_possession(
                &request.possession_jws,
                &registration.workload_key,
                &registration.instance_id,
                &endpoint,
                &request.registration_jws,
                now,
            )?;
            ensure!(
                registration.project_id == "project"
                    && registration.placement_id == "api"
                    && registration.grant_id == "online-project-grant"
                    && registration.authz_version == 1
            );
            let mut instances = self.instances.lock().unwrap();
            let active_workloads = instances
                .values()
                .filter(|instance| {
                    !instance.retired && instance.registration.purpose == InstancePurpose::Workload
                })
                .count();
            let retired_workloads = instances
                .values()
                .filter(|instance| {
                    instance.retired && instance.registration.purpose == InstancePurpose::Workload
                })
                .count();
            if registration.purpose == InstancePurpose::RolloutValidation {
                ensure!(registration.billing_grant_id.is_none());
                ensure!(registration.billing_authz_version.is_none());
                ensure!(
                    instances
                        .values()
                        .filter(|instance| {
                            !instance.retired
                                && instance.registration.purpose
                                    == InstancePurpose::RolloutValidation
                        })
                        .count()
                        < 2,
                    "validation capacity was not released"
                );
            } else {
                ensure!(
                    active_workloads == 0,
                    "serving grant max_instances=1 requires retirement before replacement admission"
                );
            }
            let receipt = InstanceReceipt {
                instance_id: registration.instance_id.clone(),
                purpose: registration.purpose,
                device_id: registration.device_id.clone(),
                grant_id: registration.grant_id.clone(),
                billing_grant_id: registration.billing_grant_id.clone(),
                workload_key: registration.workload_key.clone(),
                key_epoch: 1,
                registered_at: now,
                lease_expires_at: now + 600,
                registration_jws: request.registration_jws,
            };
            instances.insert(
                registration.instance_id.clone(),
                CloudInstance {
                    registration,
                    receipt: receipt.clone(),
                    retired: false,
                    resources: vec![],
                    project_tokens: 0,
                    issued_tokens: HashMap::new(),
                    active_workloads_at_registration: active_workloads,
                    retired_workloads_at_registration: retired_workloads,
                },
            );
            return Ok(serde_json::to_value(receipt)?);
        }
        if method == "DELETE" && path.starts_with(&format!("{instances_path}/")) {
            let id = path.rsplit('/').next().context("retirement identity")?;
            let request: ReceiptRequest = serde_json::from_slice(body)?;
            let endpoint = format!("{}/devices/{}/instances/{id}", self.base, self.device_id);
            verify_client_assertion(
                &request.client_assertion,
                &self.device_key,
                &self.device_id,
                &endpoint,
                unix_time()?,
            )?;
            if let Some(instance) = self.instances.lock().unwrap().get_mut(id) {
                instance.retired = true;
            }
            return Ok(json!({}));
        }
        let suffix = path
            .strip_prefix("/api/v1/instances/")
            .context("instance endpoint")?;
        if let Some((id, action)) = suffix.split_once('/') {
            if id != "project" && method == "POST" {
                let request: InstanceTokenRequest = serde_json::from_slice(body)?;
                let mut instances = self.instances.lock().unwrap();
                let instance = instances.get_mut(id).context("token instance")?;
                ensure!(!instance.retired, "retired instance requested a token");
                let endpoint = format!("{}/instances/{id}/{action}", self.base);
                verify_workload_assertion(
                    &request.client_assertion,
                    &instance.registration.workload_key,
                    id,
                    &endpoint,
                    unix_time()?,
                )?;
                if action == "receipt" {
                    return Ok(serde_json::to_value(&instance.receipt)?);
                }
                instance.resources.push((method.into(), path.into()));
                ensure!(action == "project-token", "unexpected model token request");
                instance.project_tokens += 1;
                let now = unix_time()?;
                let token = format!("project-{id}.{}", instance.project_tokens);
                instance
                    .issued_tokens
                    .insert(token.clone(), now + self.project_token_seconds);
                return Ok(serde_json::to_value(InstanceTokenResponse {
                    access_token: token,
                    token_type: "DPoP".into(),
                    expires_in: self.project_token_seconds as u64,
                    expires_at: now + self.project_token_seconds,
                    dpop_nonce: "online-rollout-test-nonce".into(),
                    lease_expires_at: now + 600,
                })?);
            }
        }
        let registration = self.authorize_resource(method, path, headers)?;
        if suffix == "project/storage" {
            ensure!(method == "POST");
            let sub = "owner";
            let locations = [
                (StoragePurpose::Files, "apps/project/upload/".to_owned()),
                (StoragePurpose::Storage, "apps/project/storage/".to_owned()),
                (StoragePurpose::User, format!("users/{sub}/apps/project/")),
                (
                    StoragePurpose::Temporary,
                    format!(
                        "{}/",
                        flow_like_types::storage_paths::temporary_prefixes(sub, "project").0
                    ),
                ),
            ]
            .into_iter()
            .map(|(purpose, prefix)| {
                (
                    purpose,
                    InstanceStorageLocation {
                        uri: format!("s3://fixture/{prefix}"),
                        prefix,
                        credential_id: "shared".into(),
                        options: BTreeMap::from([
                            ("aws_region".into(), "eu-central-1".into()),
                            ("aws_endpoint".into(), "https://127.0.0.1:9".into()),
                        ]),
                    },
                )
            })
            .collect();
            return Ok(serde_json::to_value(InstanceStorageLease {
                instance_id: registration.instance_id,
                device_id: registration.device_id,
                device_auth_epoch: registration.device_auth_epoch,
                key_epoch: 1,
                grant_id: registration.grant_id,
                authz_version: registration.authz_version,
                project_id: registration.project_id,
                placement_id: registration.placement_id,
                deployment_id: registration.deployment_id,
                delegating_user_id: sub.into(),
                access: OnlineProjectAccess::ReadOnly,
                expires_at: unix_time()? + self.storage_lease_seconds,
                grant_expires_at: Some(self.grant_expires_at),
                locations,
                credentials: BTreeMap::from([(
                    "shared".into(),
                    InstanceStorageCredential::AwsSession {
                        access_key_id: "fixture-key".into(),
                        secret_access_key: "fixture-secret".into(),
                        session_token: "fixture-session".into(),
                    },
                )]),
            })?);
        }
        ensure!(method == "GET");
        let block = registration.purpose == InstancePurpose::RolloutValidation
            && self.blocked_path.lock().unwrap().as_deref() == Some(path);
        if block {
            self.blocked_entered.notify_one();
            self.release_blocked.notified().await;
        }
        self.documents
            .get(path)
            .cloned()
            .context("missing pinned project document")
    }
}

async fn mock_cloud_request(
    axum::extract::State(cloud): axum::extract::State<Arc<MockCloud>>,
    method: axum::http::Method,
    uri: axum::http::Uri,
    headers: axum::http::HeaderMap,
    body: bytes::Bytes,
) -> std::result::Result<axum::Json<Value>, axum::http::StatusCode> {
    let status = cloud.status.load(std::sync::atomic::Ordering::SeqCst);
    if status != 200 {
        return Err(axum::http::StatusCode::from_u16(status).unwrap());
    }
    // Presence/signaling are tested separately. This server owns only workload resources.
    if !uri.path().starts_with("/api/v1/instances/")
        && !uri
            .path()
            .starts_with(&format!("/api/v1/devices/{}/instances", cloud.device_id))
    {
        return Err(axum::http::StatusCode::NOT_FOUND);
    }
    cloud
        .handle(method.as_str(), uri.path(), &headers, &body)
        .await
        .map(axum::Json)
        .map_err(|error| {
            eprintln!("Mock instance API rejected {method} {uri}: {error:#}");
            axum::http::StatusCode::BAD_REQUEST
        })
}

async fn online_project(
    fixture: &Fixture,
    version: u32,
    route: &str,
    documents: &mut HashMap<String, Value>,
) -> Result<PlacementConfig> {
    let placement = fixture.project(version, route).await?;
    approve_online_project(fixture, placement, documents).await
}

async fn approve_online_project(
    fixture: &Fixture,
    mut placement: PlacementConfig,
    documents: &mut HashMap<String, Value>,
) -> Result<PlacementConfig> {
    let state = Arc::new(FlowLikeState::new(
        FlowLikeConfig::with_default_store(FlowLikeStore::Local(Arc::new(LocalObjectStore::new(
            placement.project_path.clone(),
        )?))),
        HTTPClient::new_without_refetch(),
    ));
    let mut app = App::load("project".into(), state.clone()).await?;
    app.visibility = flow_like_runtime::app::AppVisibility::Private;
    app.boards.clear();
    app.events.clear();
    let mut approved = BTreeMap::new();
    for binding in &placement.events {
        let [major, minor, patch] = binding.event_version;
        let event = Event::load_pinned(&binding.event_id, &app, (major, minor, patch)).await?;
        let board = Board::load(
            StorePath::from("apps/project"),
            &event.board_id,
            state.clone(),
            event.board_version,
        )
        .await?;
        app.events.push(event.id.clone());
        app.boards.push(board.id.clone());
        approved.insert(
            format!("events/{}/versions/{major}/{minor}/{patch}", event.id),
            serde_json::to_value(event)?,
        );
        let [major, minor, patch] = binding.board_version;
        approved.insert(
            format!("boards/{}/versions/{major}/{minor}/{patch}", board.id),
            serde_json::to_value(board)?,
        );
    }
    approved.insert("app".to_owned(), serde_json::to_value(app)?);
    for (suffix, document) in &approved {
        documents.insert(
            format!("/api/v1/instances/project/{suffix}"),
            document.clone(),
        );
    }
    let bundle =
        serde_json::to_vec(&json!({"version":1,"project_id":"project","documents":approved}))?;
    placement.online_metadata_sha256 = Some(artifact_sha256(&bundle));
    let marker =
        serde_json::to_vec(&json!({"version":1,"project_id":"project","source":"online"}))?;
    let files = [
        ("apps/project/online-metadata.json", bundle),
        ("apps/project/online-source.json", marker),
    ];
    let manifest = ProjectArtifactManifest {
        version: 1,
        project_id: "project".into(),
        source: ProjectArtifactSource::Online,
        files: files
            .iter()
            .map(|(path, data)| ProjectArtifactFile {
                path: (*path).into(),
                size: data.len() as u64,
                sha256: artifact_sha256(data),
            })
            .collect(),
        bit_pins: vec![],
        package_pins: vec![],
    };
    let store = fixture.store()?;
    let transfer = Uuid::new_v4().to_string();
    let receipt = artifact_io(|| {
        project_artifacts::begin(
            &store,
            &fixture.root,
            &transfer,
            "controller",
            &manifest.descriptor()?,
        )?;
        for (index, bytes) in std::iter::once((None, manifest.canonical_bytes()?)).chain(
            files
                .into_iter()
                .enumerate()
                .map(|(i, (_, bytes))| (Some(i as u32), bytes)),
        ) {
            for (chunk, bytes) in bytes.chunks(PROJECT_ARTIFACT_CHUNK_BYTES).enumerate() {
                project_artifacts::chunk(
                    &store,
                    &fixture.root,
                    "project",
                    &transfer,
                    "controller",
                    index,
                    (chunk * PROJECT_ARTIFACT_CHUNK_BYTES) as u64,
                    &URL_SAFE_NO_PAD.encode(bytes),
                )?;
            }
        }
        project_artifacts::commit(&store, &fixture.root, "project", &transfer, "controller")
    })?;
    placement.source = ProjectSource::Online;
    placement.project_path = receipt
        .project_path
        .context("approved artifact path")?
        .into();
    placement.revision = receipt.descriptor.manifest_sha256;
    placement.resource_grant = Some(flow_like_standalone::config::ResourceGrantRef {
        grant_id: "online-project-grant".into(),
        authz_version: 1,
        billing_grant_id: Some("owner-model-billing".into()),
        billing_authz_version: Some(1),
    });
    Ok(placement)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires loopback listeners and spawning the standalone runtime binary"]
async fn online_rollout_uses_metadata_only_credentials_and_stop_fences_preflight() -> Result<()> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}/api/v1", listener.local_addr()?);
    let fixture = Fixture::with_api_base(&base)?;
    let mut documents = HashMap::new();
    let initial = online_project(&fixture, 1, "/online-first", &mut documents).await?;
    let healthy = online_project(&fixture, 2, "/online-second", &mut documents).await?;
    let cancelled = online_project(&fixture, 3, "/online-cancelled", &mut documents).await?;
    // Identical event IDs and versions do not authorize new executable bytes.
    // The API may change its response after the controller approved the bundle.
    for (path, document) in &mut documents {
        if path.contains("/events/") {
            document["config"] = serde_json::to_value(serde_json::to_vec(
                &json!({"path":"/unapproved-server-route","method":"POST"}),
            )?)?;
        }
    }
    let registration = fixture
        .store()?
        .registration()?
        .context("registered device")?;
    let cloud = Arc::new(MockCloud {
        status: std::sync::atomic::AtomicU16::new(200),
        grant_expires_at: unix_time()? + 86400,
        storage_lease_seconds: 3600,
        project_token_seconds: 300,
        base,
        device_id: registration.manifest.device_id,
        device_key: registration
            .receipt
            .context("device receipt")?
            .identity
            .auth_key,
        documents,
        instances: Default::default(),
        blocked_path: Default::default(),
        blocked_entered: Default::default(),
        release_blocked: Default::default(),
    });
    let router = axum::Router::new()
        .fallback(mock_cloud_request)
        .with_state(cloud.clone());
    let _server = MockCloudServer(tokio::spawn(
        async move { axum::serve(listener, router).await },
    ));
    let mut controller = fixture.controller().await?;
    controller
        .command(ManagementCommand::Apply {
            config: serde_json::to_value(&initial)?,
            expected_revision: 0,
            start: false,
        })
        .await?;
    let mut agent = Agent::start(&fixture.root)?;
    for (name, value) in [
        ("service-token", SERVICE_TOKEN.to_owned()),
        ("credential-secret", serde_json::to_string(VARIABLE_SECRET)?),
    ] {
        let response = controller
            .command(ManagementCommand::SetSecret {
                placement_id: "api".into(),
                expected_revision: 1,
                name: name.into(),
                value: SecretValue(value),
            })
            .await?;
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if controller.operation(&response.operation_id).await?.state == "completed" {
                    return Ok::<_, anyhow::Error>(());
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .context("online secret publication")??;
    }
    controller
        .command(ManagementCommand::Start {
            placement_id: "api".into(),
            expected_revision: 1,
        })
        .await?;
    fixture.wait_ready(1).await?;
    fixture.assert_serving("/online-first", 1).await?;

    let rollout = controller.stage(&healthy, 1, 2).await?;
    fixture.assert_serving("/online-first", 1).await?;
    controller.activate(&rollout).await?;
    controller.wait_rollout(&rollout, "healthy").await?;
    fixture.wait_ready(2).await?;
    fixture.assert_serving("/online-second", 2).await?;
    let first_validations: Vec<_> = cloud
        .snapshots()
        .into_iter()
        .filter(|instance| instance.registration.purpose == InstancePurpose::RolloutValidation)
        .collect();
    assert_eq!(first_validations.len(), 2);
    assert!(first_validations.iter().all(|instance| instance.retired));

    let previous_pid = fixture.store()?.get_placement("api")?.unwrap().process_id;
    let mut invalid = healthy.clone();
    invalid.events[0].event_version = [99, 0, 0];
    let request = controller.request(serde_json::from_value(json!({
        "type":"stage_rollout", "config":invalid, "expected_revision":2,
        "stabilization_seconds":2, "deadline_seconds":ROLLOUT_DEADLINE_SECONDS,
    }))?)?;
    assert_eq!(controller.transmit(&request).await?.state, "rejected");
    fixture.assert_serving("/online-second", 2).await?;
    let unchanged = fixture.store()?.get_placement("api")?.unwrap();
    assert_eq!(unchanged.config_revision, 2);
    assert_eq!(unchanged.process_id, previous_pid);

    *cloud.blocked_path.lock().unwrap() = Some("/api/v1/instances/project/app".into());
    let stopped = controller.stage(&cancelled, 2, 2).await?;
    controller.activate(&stopped).await?;
    tokio::time::timeout(Duration::from_secs(15), cloud.blocked_entered.notified())
        .await
        .context("candidate grant preflight did not block")?;
    fixture.assert_serving("/online-second", 2).await?;
    controller
        .command(ManagementCommand::Stop {
            placement_id: "api".into(),
            expected_revision: 2,
        })
        .await?;
    controller.wait_rollout(&stopped, "cancelled").await?;
    cloud.release_blocked.notify_one();
    wait_for("online worker stopped", || {
        Ok(fixture
            .store()?
            .get_placement("api")?
            .is_some_and(|placement| {
                placement.config_revision == 2
                    && placement.running_replicas == 0
                    && placement.observed_state == ObservedState::Stopped
            }))
    })
    .await?;
    wait_for("validation credentials retired after cancellation", || {
        Ok(cloud
            .snapshots()
            .iter()
            .filter(|instance| instance.registration.purpose == InstancePurpose::RolloutValidation)
            .all(|instance| instance.retired))
    })
    .await?;
    let snapshots = cloud.snapshots();
    assert_eq!(
        snapshots
            .iter()
            .filter(|instance| instance.registration.purpose == InstancePurpose::Workload)
            .count(),
        2
    );
    let mut retired_at_admission: Vec<_> = snapshots
        .iter()
        .filter(|instance| instance.registration.purpose == InstancePurpose::Workload)
        .map(|instance| {
            assert_eq!(instance.active_workloads_at_registration, 0);
            instance.retired_workloads_at_registration
        })
        .collect();
    retired_at_admission.sort_unstable();
    assert_eq!(retired_at_admission, [0, 1]);
    let validation_count = snapshots
        .iter()
        .filter(|instance| instance.registration.purpose == InstancePurpose::RolloutValidation)
        .count();
    assert_eq!(validation_count, 3);
    for instance in snapshots {
        assert!(instance.project_tokens > 0);
        if instance.registration.purpose == InstancePurpose::RolloutValidation {
            assert!(instance.retired);
            assert!(instance.resources.iter().all(|(method, path)| {
                (method == "GET" && path == "/api/v1/instances/project/app")
                    || (method == "POST" && path.ends_with("/project-token"))
            }));
        } else {
            assert_eq!(
                instance.registration.billing_grant_id.as_deref(),
                Some("owner-model-billing")
            );
            assert_eq!(
                instance
                    .resources
                    .iter()
                    .filter(|(_, path)| path.ends_with("/project/storage"))
                    .count(),
                2
            );
        }
    }
    for candidate in [&healthy, &cancelled] {
        let cache_root = candidate.project_path.join(".standalone-cache/api");
        match std::fs::read_dir(cache_root) {
            Ok(entries) => {
                for entry in entries {
                    assert!(
                        !entry?
                            .file_name()
                            .to_string_lossy()
                            .starts_with("preflight-")
                    );
                }
            }
            // Cancellation can stop the previous revision's preflight before
            // validation ever creates a cache for the candidate.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    let activation = controller.request(serde_json::from_value(json!({
        "type":"activate_rollout", "rollout_id":stopped,
    }))?)?;
    assert_eq!(controller.transmit(&activation).await?.state, "rejected");
    assert_eq!(
        fixture
            .store()?
            .get_placement("api")?
            .unwrap()
            .config_revision,
        2
    );
    agent.stop().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires loopback listeners and spawning the standalone runtime binary"]
async fn online_service_restarts_through_api_outage_and_denial_fences_reboot() -> Result<()> {
    use std::sync::atomic::Ordering;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}/api/v1", listener.local_addr()?);
    let fixture = Fixture::with_api_base(&base)?;
    let mut documents = HashMap::new();
    let mut placement = online_project(&fixture, 1, "/outage", &mut documents).await?;
    placement.max_replicas = 1;
    let registration = fixture.store()?.registration()?.context("device")?;
    let cloud = Arc::new(MockCloud {
        status: std::sync::atomic::AtomicU16::new(200),
        grant_expires_at: unix_time()? + 86400,
        storage_lease_seconds: 4,
        project_token_seconds: 300,
        base,
        device_id: registration.manifest.device_id,
        device_key: registration.receipt.context("receipt")?.identity.auth_key,
        documents,
        instances: Default::default(),
        blocked_path: Default::default(),
        blocked_entered: Default::default(),
        release_blocked: Default::default(),
    });
    let router = axum::Router::new()
        .fallback(mock_cloud_request)
        .with_state(cloud.clone());
    let _server = MockCloudServer(tokio::spawn(
        async move { axum::serve(listener, router).await },
    ));
    let mut controller = fixture.controller().await?;
    controller
        .command(ManagementCommand::Apply {
            config: serde_json::to_value(&placement)?,
            expected_revision: 0,
            start: false,
        })
        .await?;
    let mut agent = Agent::start(&fixture.root)?;
    for (name, value) in [
        ("service-token", SERVICE_TOKEN.to_owned()),
        ("credential-secret", serde_json::to_string(VARIABLE_SECRET)?),
    ] {
        let response = controller
            .command(ManagementCommand::SetSecret {
                placement_id: "api".into(),
                expected_revision: 1,
                name: name.into(),
                value: SecretValue(value),
            })
            .await?;
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if controller.operation(&response.operation_id).await?.state == "completed" {
                    return Ok::<_, anyhow::Error>(());
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await??;
    }
    controller
        .command(ManagementCommand::Start {
            placement_id: "api".into(),
            expected_revision: 1,
        })
        .await?;
    fixture.wait_ready(1).await?;
    fixture
        .assert_serving("/outage", 1)
        .await
        .context("Initial online service readiness")?;
    let initial_instances = cloud.snapshots().len();
    let snapshot_path = fixture
        .root
        .join("placement-data/api/current/store/.standalone-cache/api/outage/snapshot.json");
    let saved_snapshot = std::fs::read(&snapshot_path)?;

    cloud.status.store(503, Ordering::SeqCst);
    let old_pid = fixture.store()?.get_placement("api")?.unwrap().replicas[0].process_id;
    controller
        .command(ManagementCommand::Restart {
            placement_id: "api".into(),
            expected_revision: 1,
        })
        .await?;
    wait_for("cached worker replacement", || {
        Ok(fixture
            .store()?
            .get_placement("api")?
            .is_some_and(|placement| {
                placement.replicas.iter().any(|replica| {
                    replica.process_id.is_some()
                        && replica.process_id != old_pid
                        && replica.observed_state == ObservedState::Running
                })
            }))
    })
    .await?;
    fixture.wait_ready(1).await?;
    fixture
        .assert_serving("/outage", 1)
        .await
        .context("Cached service after worker restart")?;
    assert_eq!(
        cloud.snapshots().len(),
        initial_instances,
        "Cached service cannot admit an instance during outage"
    );

    let previous = fixture
        .store()?
        .get_placement("api")?
        .context("placement")?;
    let previous_pid = previous.replicas[0]
        .process_id
        .context("cached worker process before agent reboot")?;
    agent.crash().await?;
    agent = Agent::start(&fixture.root)?;
    // The killed agent leaves its last readiness row in SQLite until the new
    // supervisor reconciles it. Require the replacement worker's own readiness.
    wait_for("cached worker readiness after agent reboot", || {
        Ok(fixture
            .store()?
            .get_placement("api")?
            .is_some_and(|record| {
                record.config_revision == previous.config_revision
                    && record.intent_revision == previous.intent_revision
                    && record.replicas.iter().any(|replica| {
                        replica.process_id.is_some_and(|pid| pid != previous_pid)
                            && replica.config_revision == previous.config_revision
                            && replica.intent_revision == previous.intent_revision
                            && replica.applied_revision == Some(previous.config_revision)
                            && replica.observed_state == ObservedState::Running
                    })
            }))
    })
    .await?;
    fixture.wait_ready(1).await?;
    fixture
        .assert_serving("/outage", 1)
        .await
        .context("Cached service after agent reboot")?;
    assert_eq!(
        cloud.snapshots().len(),
        initial_instances,
        "Agent reboot restores metadata without a cloud identity"
    );
    cloud.status.store(200, Ordering::SeqCst);
    tokio::time::timeout(Duration::from_secs(90), async {
        loop {
            if cloud.snapshots().iter().any(|instance| {
                instance.registration.purpose == InstancePurpose::Workload
                    && !instance.retired
                    && instance
                        .resources
                        .iter()
                        .any(|(_, path)| path.ends_with("/project/storage"))
            }) && cloud.snapshots().len() > initial_instances
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .context("Fresh instance admission after backend recovery")?;
    fixture
        .assert_serving("/outage", 1)
        .await
        .context("Service after fresh backend admission")?;

    // The short fixture credential renews in the background; confirmed denial
    // must also stop workflows whose current request needs no cloud object.
    cloud.status.store(403, Ordering::SeqCst);
    wait_for("revoked outage snapshot wipe", || {
        Ok(!snapshot_path.exists())
    })
    .await?;
    wait_for("denied service drain", || {
        Ok(fixture
            .store()?
            .get_placement("api")?
            .unwrap()
            .ready_replicas
            == 0)
    })
    .await?;
    assert!(
        fixture
            .request("/outage", Some(SERVICE_TOKEN))
            .await
            .map_or(true, |response| !response.status().is_success())
    );
    agent.crash().await?;
    // A rollback of child-writable files must not roll back the parent denial.
    vault::write_new_private(&snapshot_path, &saved_snapshot)?;
    cloud.status.store(503, Ordering::SeqCst);
    agent = Agent::start(&fixture.root)?;
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(
        fixture
            .store()?
            .get_placement("api")?
            .unwrap()
            .ready_replicas,
        0
    );
    assert!(
        fixture
            .request("/outage", Some(SERVICE_TOKEN))
            .await
            .map_or(true, |response| !response.status().is_success())
    );
    agent.stop().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires loopback listeners and spawning the standalone runtime binary"]
async fn online_catalog_services_keep_workers_through_renewal_and_outage_then_drain_on_revocation()
-> Result<()> {
    use std::sync::atomic::Ordering;
    for kind in ["rest", "mcp", "daemon"] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let base = format!("http://{}/api/v1", listener.local_addr()?);
        let fixture = Fixture::with_api_base(&base)?;
        let mut documents = HashMap::new();
        let local = fixture
            .catalog_service(1, kind, fixture.port, false)
            .await?;
        let placement = approve_online_project(&fixture, local, &mut documents).await?;
        let registration = fixture.store()?.registration()?.context("device")?;
        let cloud = Arc::new(MockCloud {
            status: std::sync::atomic::AtomicU16::new(200),
            grant_expires_at: unix_time()? + 86400,
            storage_lease_seconds: 4,
            // Resource tokens must last longer than the broker's 60-second safety
            // margin. This brings proactive rotation into the process test window.
            project_token_seconds: 65,
            base,
            device_id: registration.manifest.device_id,
            device_key: registration.receipt.context("receipt")?.identity.auth_key,
            documents,
            instances: Default::default(),
            blocked_path: Default::default(),
            blocked_entered: Default::default(),
            release_blocked: Default::default(),
        });
        let router = axum::Router::new()
            .fallback(mock_cloud_request)
            .with_state(cloud.clone());
        let _server = MockCloudServer(tokio::spawn(
            async move { axum::serve(listener, router).await },
        ));
        let mut controller = fixture.controller().await?;
        controller
            .command(ManagementCommand::Apply {
                config: serde_json::to_value(&placement)?,
                expected_revision: 0,
                start: true,
            })
            .await?;
        let mut agent = Agent::start(&fixture.root)?;
        fixture
            .wait_ready(1)
            .await
            .with_context(|| format!("online {kind} readiness"))?;
        let initial = fixture
            .store()?
            .get_placement("api")?
            .context("running service")?;
        let process_id = initial.replicas[0].process_id.context("running worker")?;
        let mcp_session = if kind == "mcp" {
            Some(open_mcp_session(fixture.port).await?)
        } else {
            None
        };
        let live = || {
            cloud.snapshots().into_iter().find(|instance| {
                instance.registration.purpose == InstancePurpose::Workload && !instance.retired
            })
        };
        wait_for("project token and storage credential rotation", || {
            Ok(live().is_some_and(|instance| {
                instance.project_tokens >= 3
                    && instance
                        .resources
                        .iter()
                        .filter(|(_, path)| path.ends_with("/project/storage"))
                        .count()
                        >= 3
            }))
        })
        .await
        .with_context(|| format!("{kind} refresh without restart"))?;
        let before_outage = live().context("resource identity")?;
        assert_eq!(
            cloud
                .snapshots()
                .iter()
                .filter(|i| i.registration.purpose == InstancePurpose::Workload)
                .count(),
            1
        );
        cloud.status.store(503, Ordering::SeqCst);
        // Exceed the real provider credential TTL while local service stays live.
        tokio::time::sleep(Duration::from_secs(7)).await;
        let during = fixture
            .store()?
            .get_placement("api")?
            .context("outage service")?;
        assert_eq!(
            during.replicas[0].process_id,
            Some(process_id),
            "{kind} restarted on transient outage"
        );
        assert_eq!(
            during.ready_replicas, 1,
            "{kind} lost local service readiness"
        );
        if kind != "daemon" {
            assert_catalog_listener(fixture.port).await?;
        }

        if let Some(session) = &mcp_session {
            assert_mcp_session(fixture.port, session).await?;
        }
        cloud.status.store(200, Ordering::SeqCst);
        wait_for("resource access recovered on the original worker", || {
            Ok(live().is_some_and(|instance| {
                instance.project_tokens > before_outage.project_tokens
                    && instance
                        .resources
                        .iter()
                        .filter(|(_, path)| path.ends_with("/project/storage"))
                        .count()
                        > before_outage
                            .resources
                            .iter()
                            .filter(|(_, path)| path.ends_with("/project/storage"))
                            .count()
            }))
        })
        .await
        .with_context(|| format!("{kind} recovery"))?;
        assert_eq!(
            fixture.store()?.get_placement("api")?.unwrap().replicas[0].process_id,
            Some(process_id)
        );
        assert_eq!(
            live().unwrap().registration.instance_id,
            before_outage.registration.instance_id
        );
        if let Some(session) = &mcp_session {
            assert_mcp_session(fixture.port, session).await?;
        }

        cloud.status.store(403, Ordering::SeqCst);
        wait_for(
            "confirmed revocation drains long-running catalog service",
            || {
                Ok(fixture
                    .store()?
                    .get_placement("api")?
                    .is_some_and(|record| record.ready_replicas == 0))
            },
        )
        .await
        .with_context(|| format!("{kind} revocation"))?;
        if kind != "daemon" {
            wait_for("revoked listener closed", || {
                Ok(std::net::TcpStream::connect_timeout(
                    &std::net::SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, fixture.port)),
                    Duration::from_millis(100),
                )
                .is_err())
            })
            .await?;
        }
        agent.stop().await?;
    }
    Ok(())
}

fn certificate_client(certificate: &rcgen::Certificate) -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .tls_built_in_root_certs(false)
        .add_root_certificate(reqwest::Certificate::from_der(certificate.der())?)
        .tls_info(true)
        .pool_max_idle_per_host(0)
        .build()?)
}

async fn certificate_service_request(
    client: &reqwest::Client,
    kind: &str,
    port: u16,
    session: Option<&str>,
) -> Result<(Vec<u8>, Option<String>)> {
    let origin = format!("https://127.0.0.1:{port}");
    let request = match kind {
        "http" => client
            .post(format!("{origin}/tls"))
            .bearer_auth(SERVICE_TOKEN)
            .json(&json!({})),
        "mcp" => {
            let request = client
                .post(format!("{origin}/mcp"))
                .header("accept", "application/json");
            if let Some(session) = session {
                request
                    .header("mcp-session-id", session)
                    .header("mcp-protocol-version", "2025-06-18")
                    .json(&json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}))
            } else {
                request.json(&json!({"jsonrpc":"2.0","id":1,"method":"initialize",
                    "params":{"protocolVersion":"2025-06-18","capabilities":{},
                        "clientInfo":{"name":"certificate-lifecycle-test","version":"1"}}}))
            }
        }
        _ => client.get(format!("{origin}/nonexistent-route")),
    };
    let response = request.send().await?;
    let peer = response
        .extensions()
        .get::<reqwest::tls::TlsInfo>()
        .and_then(reqwest::tls::TlsInfo::peer_certificate)
        .context("TLS peer identity")?
        .to_vec();
    let returned_session = response
        .headers()
        .get("mcp-session-id")
        .map(|value| value.to_str().map(str::to_owned))
        .transpose()?;
    if kind == "rest" {
        ensure!(
            response.status() == reqwest::StatusCode::NOT_FOUND,
            "REST TLS route response"
        );
    } else {
        let response: Value = response.error_for_status()?.json().await?;
        if kind == "http" {
            ensure!(
                response == json!({"revision":1}),
                "Native HTTPS workflow response"
            );
        } else if session.is_some() {
            ensure!(
                response["result"]["tools"] == json!([]),
                "Retained MCP session after TLS rotation"
            );
        } else {
            ensure!(
                response["result"]["protocolVersion"] == "2025-06-18",
                "MCP HTTPS initialization"
            );
            ensure!(returned_session.is_some(), "MCP session header");
        }
    }
    Ok((peer, returned_session))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires loopback listeners and spawning the standalone runtime binary"]
async fn device_certificates_rotate_live_native_rest_and_mcp_services() -> Result<()> {
    for kind in ["http", "rest", "mcp"] {
        let fixture = Fixture::new()?;
        let mut placement = if kind == "http" {
            fixture.project(1, "/tls").await?
        } else {
            fixture
                .catalog_service(1, kind, fixture.port, false)
                .await?
        };
        let certificate_id = Uuid::new_v4().to_string();
        placement.tls_certificate_id = Some(certificate_id.clone());
        let first =
            rcgen::generate_simple_self_signed(vec!["localhost".into(), "127.0.0.1".into()])?;
        let second =
            rcgen::generate_simple_self_signed(vec!["localhost".into(), "127.0.0.1".into()])?;
        let mut controller = fixture.controller().await?;
        let imported = controller
            .command(ManagementCommand::PutCertificate {
                certificate_id: certificate_id.clone(),
                label: format!("{kind} service"),
                expected_revision: 0,
                certificate_chain_pem: SecretValue(first.cert.pem()),
                private_key_pem: SecretValue(first.signing_key.serialize_pem()),
            })
            .await?;
        assert_eq!(imported.state, "completed");
        assert_eq!(imported.result["certificate"]["revision"], 1);
        assert!(!serde_json::to_string(&imported)?.contains("PRIVATE KEY"));
        controller
            .command(ManagementCommand::Apply {
                config: serde_json::to_value(&placement)?,
                expected_revision: 0,
                start: false,
            })
            .await?;
        let mut agent = Agent::start(&fixture.root)?;
        if kind == "http" {
            for (name, value) in [
                ("service-token", SERVICE_TOKEN.to_owned()),
                ("credential-secret", serde_json::to_string(VARIABLE_SECRET)?),
            ] {
                let operation = controller
                    .command(ManagementCommand::SetSecret {
                        placement_id: "api".into(),
                        expected_revision: 1,
                        name: name.into(),
                        value: SecretValue(value),
                    })
                    .await?
                    .operation_id;
                tokio::time::timeout(Duration::from_secs(10), async {
                    loop {
                        if controller.operation(&operation).await?.state == "completed" {
                            return Ok::<_, anyhow::Error>(());
                        }
                        tokio::time::sleep(Duration::from_millis(25)).await;
                    }
                })
                .await
                .context("Service credential publication")??;
            }
        }
        controller
            .command(ManagementCommand::Start {
                placement_id: "api".into(),
                expected_revision: 1,
            })
            .await?;
        fixture
            .wait_ready(1)
            .await
            .with_context(|| format!("{kind} TLS readiness"))?;
        let before = fixture
            .store()?
            .get_placement("api")?
            .context("Running TLS placement")?;
        let pids: Vec<_> = before
            .replicas
            .iter()
            .map(|replica| replica.process_id)
            .collect();
        assert!(
            reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(5))
                .build()?
                .get(format!(
                    "http://127.0.0.1:{}/nonexistent-route",
                    fixture.port
                ))
                .send()
                .await
                .is_err(),
            "{kind} listener must reject plaintext"
        );
        let (peer, session) = certificate_service_request(
            &certificate_client(&first.cert)?,
            kind,
            fixture.port,
            None,
        )
        .await?;
        assert_eq!(peer.as_slice(), first.cert.der().as_ref());
        let listed = controller
            .command(ManagementCommand::Certificates {
                placement_id: None,
                after: None,
                limit: 4,
            })
            .await?;
        assert_eq!(
            listed.result["certificates"][0]["certificate_id"],
            certificate_id
        );
        assert_eq!(listed.result["certificates"][0]["binding_count"], 1);
        assert_eq!(
            listed.result["certificates"][0]["bindings"][0]["placement_id"],
            "api"
        );
        assert!(
            listed.result["certificates"][0]["not_after"]
                .as_i64()
                .unwrap()
                > unix_time()?
        );
        let delete = controller.request(ManagementCommand::DeleteCertificate {
            certificate_id: certificate_id.clone(),
            expected_revision: 1,
        })?;
        assert_eq!(controller.transmit(&delete).await?.state, "rejected");
        let wrong_key = controller.request(ManagementCommand::PutCertificate {
            certificate_id: certificate_id.clone(),
            label: "Rejected mismatch".into(),
            expected_revision: 1,
            certificate_chain_pem: SecretValue(first.cert.pem()),
            private_key_pem: SecretValue(second.signing_key.serialize_pem()),
        })?;
        assert_eq!(controller.transmit(&wrong_key).await?.state, "rejected");
        let rotated = controller
            .command(ManagementCommand::PutCertificate {
                certificate_id: certificate_id.clone(),
                label: format!("{kind} renewed"),
                expected_revision: 1,
                certificate_chain_pem: SecretValue(second.cert.pem()),
                private_key_pem: SecretValue(second.signing_key.serialize_pem()),
            })
            .await?;
        assert_eq!(rotated.result["certificate"]["revision"], 2);
        assert_ne!(
            rotated.result["certificate"]["sha256_fingerprint"],
            imported.result["certificate"]["sha256_fingerprint"]
        );
        let client = certificate_client(&second.cert)?;
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                if let Ok((peer, _)) =
                    certificate_service_request(&client, kind, fixture.port, session.as_deref())
                        .await
                {
                    ensure!(
                        peer.as_slice() == second.cert.der().as_ref(),
                        "New TLS handshake must present the renewed certificate"
                    );
                    return Ok::<_, anyhow::Error>(());
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .with_context(|| format!("{kind} live TLS rotation"))??;
        let after = fixture
            .store()?
            .get_placement("api")?
            .context("Placement after certificate rotation")?;
        assert_eq!(
            after
                .replicas
                .iter()
                .map(|replica| replica.process_id)
                .collect::<Vec<_>>(),
            pids,
            "Certificate renewal must preserve the workflow process"
        );
        assert_eq!(
            (after.config_revision, after.intent_revision),
            (before.config_revision, before.intent_revision)
        );
        controller
            .command(ManagementCommand::Stop {
                placement_id: "api".into(),
                expected_revision: 1,
            })
            .await?;
        wait_for("TLS placement stopped", || {
            Ok(fixture
                .store()?
                .get_placement("api")?
                .is_some_and(|record| {
                    record.observed_state == ObservedState::Stopped
                        && record
                            .replicas
                            .iter()
                            .all(|replica| replica.process_id.is_none())
                }))
        })
        .await?;
        controller
            .command(ManagementCommand::Remove {
                placement_id: "api".into(),
                expected_revision: 1,
            })
            .await?;
        let deleted = controller
            .command(ManagementCommand::DeleteCertificate {
                certificate_id,
                expected_revision: 2,
            })
            .await?;
        assert_eq!(deleted.result["deleted"], true);
        agent.stop().await?;
    }
    Ok(())
}
