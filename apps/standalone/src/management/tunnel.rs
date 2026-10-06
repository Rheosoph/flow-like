use super::*;
#[cfg(feature = "runtime")]
use crate::models::{acquire::AcquisitionManager, host::ModelHost};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

/// A tunnel retains the controller identity while short Noise certificates rotate.
#[derive(Clone)]
pub(crate) struct TunnelAuthority {
    service: Arc<ManagementService>,
    certificate: ControllerCertificate,
    signer: Ed25519PublicKey,
}

pub(crate) type LiveTunnelAuthority = Arc<std::sync::RwLock<TunnelAuthority>>;

#[derive(Clone)]
pub(crate) enum InternalTarget {
    Read(Arc<ReadTarget>),
    Artifact(ArtifactTarget),
    #[cfg(feature = "runtime")]
    ModelAsset(ModelAssetTarget),
}

/// Where a tunnel open leads: a deployed service, or the device's model gateway.
pub(crate) enum OpenTarget {
    Service(ServiceTarget),
    #[cfg(feature = "runtime")]
    ModelGateway(GatewayTarget),
}

/// Requests of a model gateway stream reach the agent's loopback gateway as the principal
/// of the tunnel's grant; the client never names one.
#[cfg(feature = "runtime")]
#[derive(Clone)]
pub(crate) struct GatewayTarget {
    pub address: SocketAddr,
    pub secret: zeroize::Zeroizing<String>,
    pub consumer: ModelConsumer,
}

/// One push into a model asset job; the device verifies the pinned digest at the end.
#[cfg(feature = "runtime")]
#[derive(Clone)]
pub(crate) struct ModelAssetTarget {
    pub acquisition: AcquisitionManager,
    pub job_id: String,
    pub digest: ModelAssetDigest,
    pub offset: u64,
    /// Whether the push may stop a download the device runs itself: only the owner and Manage
    /// models may, a deployer pushes only what the device could not fetch.
    pub take_over: bool,
    principal: String,
}

/// Reads that answer over a data stream, up to 1 MiB.
fn bulk_read(command: &ManagementCommand) -> bool {
    matches!(
        command,
        ManagementCommand::Inspect
            | ManagementCommand::InspectPage { .. }
            | ManagementCommand::Logs { .. }
            | ManagementCommand::Messages { .. }
            | ManagementCommand::MetricsHistory { .. }
            | ManagementCommand::TelemetryRead { .. }
            | ManagementCommand::TelemetryRosterRead { .. }
            | ManagementCommand::ArchiveRead { .. }
            | ManagementCommand::ArchiveRosterRead { .. }
            | ManagementCommand::Models {
                request: ModelsRequest::Stats { .. }
            }
    )
}

/// The consumer the gateway records the requests of `authority` as.
pub(super) fn consumer_of(authority: &Authority) -> ModelConsumer {
    authority
        .grant
        .as_ref()
        .map_or(ModelConsumer::Owner, |grant| ModelConsumer::Grant {
            grant_id: grant.grant_id.clone(),
        })
}

/// Manage models pushes to any job, Deploy only to jobs a deploy asked for.
#[cfg(feature = "runtime")]
fn may_push(authority: &Authority) -> bool {
    authority.grant.as_ref().is_none_or(|grant| {
        grant.capabilities.iter().any(|capability| {
            matches!(
                capability,
                ManagementCapability::ModelManage | ManagementCapability::Deploy
            )
        })
    })
}

/// A deployer completes only downloads the deploy of its own project asked for; the
/// device verifies the pinned digest whoever sends the bytes.
#[cfg(feature = "runtime")]
fn require_push(store: &StateStore, authority: &Authority, operation_ids: &[String]) -> Result<()> {
    if authority.permits(ManagementCapability::ModelManage, None, None) {
        return Ok(());
    }
    refuse_unless(
        super::models::deploy_asked(store, authority, operation_ids)?,
        RejectionCode::Unauthorized,
        "Pushing this model file needs Manage models, or Deploy on a project whose deploy asked for it",
    )
}

#[cfg(feature = "runtime")]
fn running_host() -> Result<Arc<ModelHost>> {
    ModelHost::current().ok_or_else(|| {
        refusal(
            RejectionCode::Unsupported,
            "The model host of this device is not running",
        )
    })
}

#[cfg(feature = "runtime")]
fn model_asset_target(
    store: &StateStore,
    authority: &Authority,
    job_id: &str,
    offset: u64,
) -> Result<InternalTarget> {
    refuse_unless(
        may_push(authority),
        RejectionCode::Unauthorized,
        "Pushing model files needs Manage models or Deploy",
    )?;
    let host = running_host()?;
    asset_target(store, authority, host.acquisition(), job_id, offset)
        .map(InternalTarget::ModelAsset)
}

#[cfg(not(feature = "runtime"))]
fn model_asset_target(_: &StateStore, _: &Authority, _: &str, _: u64) -> Result<InternalTarget> {
    Err(refusal(
        RejectionCode::Unsupported,
        "This device agent was built without the model host",
    ))
}

/// The push session of an open job the authority may complete.
#[cfg(feature = "runtime")]
fn asset_target(
    store: &StateStore,
    authority: &Authority,
    acquisition: &AcquisitionManager,
    job_id: &str,
    offset: u64,
) -> Result<ModelAssetTarget> {
    let job = acquisition
        .jobs()
        .into_iter()
        .find(|job| job.job_id == job_id)
        .ok_or_else(|| {
            refusal(
                RejectionCode::Invalid,
                format!("Model job {job_id} is not open on this device"),
            )
        })?;
    require_push(store, authority, &job.operation_ids)?;
    Ok(ModelAssetTarget {
        acquisition: acquisition.clone(),
        job_id: job.job_id,
        digest: job.asset.digest,
        offset,
        take_over: authority.permits(ManagementCapability::ModelManage, None, None),
        principal: authority.principal.clone(),
    })
}

/// A finished job takes no more bytes, so only an open one needs the push authority.
#[cfg(feature = "runtime")]
fn check_model_asset(
    store: &StateStore,
    authority: &Authority,
    target: &ModelAssetTarget,
) -> Result<()> {
    ensure!(
        authority.principal == target.principal,
        "Model asset stream principal changed"
    );
    let open = target
        .acquisition
        .jobs()
        .into_iter()
        .find(|job| job.job_id == target.job_id);
    match open {
        Some(job) => require_push(store, authority, &job.operation_ids),
        None => Ok(()),
    }
}

/// The gateway, the address of its loopback listener and the principal of `authority`.
#[cfg(feature = "runtime")]
fn gateway_of(host: &ModelHost, authority: &Authority) -> GatewayTarget {
    GatewayTarget {
        address: host.gateway().address(),
        secret: zeroize::Zeroizing::new(host.gateway().agent_secret().to_owned()),
        consumer: consumer_of(authority),
    }
}

#[cfg(feature = "runtime")]
fn gateway(authority: &Authority) -> Result<OpenTarget> {
    let host = running_host()?;
    Ok(OpenTarget::ModelGateway(gateway_of(&host, authority)))
}

#[cfg(not(feature = "runtime"))]
fn gateway(_: &Authority) -> Result<OpenTarget> {
    Err(refusal(
        RejectionCode::Unsupported,
        "This device agent was built without the model host",
    ))
}

pub(crate) struct ReadTarget {
    request: ManagementRequest,
    authority: Authority,
}

#[derive(Clone)]
pub(crate) struct ArtifactTarget {
    pub project_id: String,
    pub transfer_id: String,
    pub file_index: Option<u32>,
    pub offset: u64,
    principal: String,
}

pub(crate) fn live_authority(access: &LiveTunnelAuthority) -> Result<TunnelAuthority> {
    Ok(access
        .read()
        .map_err(|_| anyhow::anyhow!("Tunnel authority unavailable"))?
        .clone())
}

pub(crate) fn bounded_response(response: &impl serde::Serialize) -> Result<Vec<u8>> {
    struct Buffer(Vec<u8>);
    impl std::io::Write for Buffer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.0.len() + bytes.len() > 1024 * 1024 {
                return Err(std::io::Error::other(
                    "Internal tunnel response exceeds 1 MiB",
                ));
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut buffer = Buffer(Vec::new());
    serde_json::to_writer(&mut buffer, response)?;
    Ok(buffer.0)
}

pub(super) fn service_listeners(
    store: &StateStore,
    authority: &Authority,
    request: &ManagementRequest,
    placement_id: &str,
) -> Result<ManagementResponse> {
    validate_management_id(placement_id)?;
    let (record, project_id) = placement_scope(store, placement_id)?;
    authority.require(
        ManagementCapability::ServiceConnect,
        Some(&project_id),
        Some(placement_id),
    )?;
    let config: PlacementConfig = serde_json::from_value(record.config)?;
    let mut services = Vec::new();
    if let Some(hosting) = &config.hosting {
        let mut entry = json!({"id":"hosting","host":hosting.host,"port":hosting.port,"protocol":if config.tls_certificate_id.is_some() {"https"} else {"http"}});
        if let Some(certificate) = config.tls_certificate_id.as_deref()
            && let Ok(metadata) = crate::certificates::metadata(store, certificate)
            && let Some(name) = certificate_server_name(&metadata)
        {
            entry["tls_server_name"] = name.into();
        }
        services.push(entry);
    }
    for configured in &config.tunnel_services {
        let mut entry = json!({"id":configured.id,"host":configured.host,"port":configured.port,"protocol":configured.protocol});
        if let Some(name) = &configured.tls_server_name {
            entry["tls_server_name"] = name.clone().into();
        }
        services.push(entry);
    }
    let response = ManagementResponse {
        operation_id: request.operation_id.clone(),
        state: "completed".into(),
        result: json!({"placement_id":placement_id,"project_id":project_id,"config_revision":record.config_revision,"services":services}),
    };
    refuse_unless(
        serde_json::to_vec(&response)?.len() <= noise::MAX_PLAINTEXT,
        RejectionCode::Limit,
        "Service listener discovery exceeds the remote read limit",
    )?;
    Ok(response)
}

fn certificate_server_name(
    metadata: &flow_like_device_protocol::CertificateMetadata,
) -> Option<String> {
    metadata
        .dns_names
        .first()
        .map(|name| {
            name.strip_prefix("*.")
                .map_or_else(|| name.clone(), |suffix| format!("tunnel.{suffix}"))
        })
        .or_else(|| metadata.ip_addresses.first().cloned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{DesiredState, ObservedState};

    #[test]
    fn service_tunnel_grants_bind_capability_placement_and_live_configuration() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let owner = SigningKey::generate();
        let controller = SigningKey::generate();
        let device = DeviceSession::test_management_session(
            "http://127.0.0.1:1/api/v1".into(),
            "device".into(),
            SigningKey::generate(),
            owner.public_key(),
        )
        .test_with_invitation_key(owner.public_key());
        let service =
            ManagementService::new(directory.path().into(), Arc::new(device), "boot".into());
        let now = unix_time()?;
        service.refresh_authority(now + 300)?;
        let mut store = StateStore::open(&directory.path().join("management.sqlite"))?;
        for (id, project) in [
            ("allowed", "project"),
            ("other", "project"),
            ("foreign", "foreign"),
        ] {
            let config = json!({"id":id,"project_id":project,"deployment_id":format!("deployment-{id}"),"revision":"v1","source":"offline","project_path":directory.path(),"events":[],"tunnel_services":[{"id":"database","host":"127.0.0.1","port":5432,"protocol":"tcp"}],"hosting":{"host":"0.0.0.0","port":8080,"max_in_flight":4,"request_timeout_secs":30,"auth_secret":"service"}});
            store.upsert_placement(id, &config, DesiredState::Running)?;
            store.record_observed(id, ObservedState::Running, None, None, Some(1))?;
            store.connection.execute("INSERT INTO placement_replicas(placement_id,slot,config_revision,intent_revision,observed_state,applied_revision) VALUES(?1,0,1,1,'running',1)", [id])?;
        }
        let allowed = ManagementGrant {
            grant_id: "allowed-grant".into(),
            user_id: "reader".into(),
            controller_key: controller.public_key(),
            scope: ManagementScope::Placement {
                project_id: "project".into(),
                placement_id: "allowed".into(),
            },
            capabilities: vec![ManagementCapability::ServiceConnect],
            expires_at: now + 1000,
            group_id: None,
            group_version: None,
        };
        let status_only = ManagementGrant {
            grant_id: "status-only".into(),
            capabilities: vec![ManagementCapability::Status],
            ..allowed.clone()
        };
        let project = ManagementGrant {
            grant_id: "project-grant".into(),
            scope: ManagementScope::Project {
                project_id: "project".into(),
            },
            ..allowed.clone()
        };
        let logs = ManagementGrant {
            grant_id: "logs-only".into(),
            capabilities: vec![ManagementCapability::Logs],
            ..allowed.clone()
        };
        let mut policy = ManagementPolicy {
            version: 1,
            device_id: "device".into(),
            policy_version: 1,
            previous_policy_digest: None,
            grants: vec![allowed, status_only, project, logs],
            issued_at: now,
            expires_at: now + 1000,
        };
        let signed = sign_management_policy(&policy, &owner)?;
        store.accept_management_policy(&signed, &owner.public_key(), "device", now)?;
        let connect = |grant: &str| -> Result<TunnelAuthority> {
            let cert = sign_controller_certificate(
                &ControllerCertificate {
                    version: 1,
                    device_id: "device".into(),
                    grant_id: grant.into(),
                    session_id: "session".into(),
                    management_key: x25519_dalek::x25519(
                        [7; 32],
                        x25519_dalek::X25519_BASEPOINT_BYTES,
                    ),
                    issued_at: now,
                    expires_at: now + 300,
                },
                &controller,
            )?;
            Ok(service.connect_tunnel(&cert, grant, "session")?.0)
        };
        let authority = connect("allowed-grant")?;
        let read_listeners =
            |authority: &TunnelAuthority, placement: &str| -> Result<ManagementResponse> {
                let request = ManagementRequest {
                    operation_id: "service-discovery".into(),
                    device_id: "device".into(),
                    issued_at: now,
                    expires_at: now + 60,
                    command: ManagementCommand::ServiceListeners {
                        placement_id: placement.into(),
                    },
                };
                let mut store = StateStore::open(&directory.path().join("management.sqlite"))?;
                execute(
                    &mut store,
                    &authority.authority()?,
                    &request,
                    service.device.manifest(),
                    "boot",
                    directory.path(),
                    now,
                )
            };
        let listeners = read_listeners(&authority, "allowed")?;
        assert_eq!(listeners.state, "completed");
        assert_eq!(
            listeners.result["services"][0],
            json!({"id":"hosting","host":"0.0.0.0","port":8080,"protocol":"http"})
        );
        assert_eq!(
            listeners.result["services"][1],
            json!({"id":"database","host":"127.0.0.1","port":5432,"protocol":"tcp"})
        );
        let serialized = serde_json::to_string(&listeners)?;
        for private in [
            "project_path",
            "auth_secret",
            "tls_sha256_fingerprint",
            "secret_overrides",
            "variables",
        ] {
            assert!(!serialized.contains(private));
        }
        assert!(read_listeners(&authority, "other").is_err());
        assert!(read_listeners(&connect("status-only")?, "allowed").is_err());
        let target = authority.target("allowed", "hosting")?;
        assert_eq!(target.address, "127.0.0.1:8080".parse::<SocketAddr>()?);
        assert_eq!(
            authority.target("allowed", "database")?.address.port(),
            5432
        );
        assert!(authority.target("other", "database").is_err());
        assert!(
            authority
                .target_mode(
                    "allowed",
                    "database",
                    flow_like_device_protocol::TunnelMode::Http
                )
                .is_err()
        );
        assert!(
            authority
                .target_mode(
                    "allowed",
                    "hosting",
                    flow_like_device_protocol::TunnelMode::Http
                )?
                .tls
                .is_none()
        );
        assert!(authority.target("other", "hosting").is_err());
        assert!(authority.target("foreign", "hosting").is_err());
        assert!(authority.target("allowed", "127.0.0.1:22").is_err());
        let open = flow_like_device_protocol::TunnelOpen {
            placement_id: "allowed".into(),
            service_id: "hosting".into(),
            mode: flow_like_device_protocol::TunnelMode::Tcp,
            target: flow_like_device_protocol::TunnelTarget::Service,
        };
        match authority.open_target(&open)? {
            OpenTarget::Service(opened) => assert_eq!(opened, target),
            #[cfg(feature = "runtime")]
            OpenTarget::ModelGateway(_) => panic!("a service open leads to its service"),
        }
        assert!(
            connect("status-only")?
                .target("allowed", "hosting")
                .is_err()
        );
        let project = connect("project-grant")?;
        assert!(project.target("other", "hosting").is_ok());
        assert!(project.target("foreign", "hosting").is_err());
        let logs = connect("logs-only")?;
        assert!(logs.target("allowed", "hosting").is_err());
        let read = |placement: &str| TunnelDataOpen::Request {
            request: ManagementRequest {
                operation_id: "logs-read".into(),
                device_id: "device".into(),
                issued_at: now,
                expires_at: now + 60,
                command: ManagementCommand::Logs {
                    placement_id: Some(placement.into()),
                    after: 0,
                    limit: 32,
                },
            },
        };
        let InternalTarget::Read(allowed_read) = logs.internal_target(&read("allowed"))? else {
            unreachable!()
        };
        let response: Value = serde_json::from_slice(&logs.read_internal(&allowed_read)?)?;
        assert_eq!(response["state"], "completed");
        let InternalTarget::Read(denied_read) = logs.internal_target(&read("other"))? else {
            unreachable!()
        };
        let response: Value = serde_json::from_slice(&logs.read_internal(&denied_read)?)?;
        assert_eq!(response["state"], "rejected");
        assert_eq!(response["result"]["code"], "unauthorized");
        let TunnelDataOpen::Request { mut request } = read("allowed") else {
            unreachable!()
        };
        request.command = ManagementCommand::Reboot {
            expected_boot_id: "boot".into(),
        };
        assert!(
            logs.internal_target(&TunnelDataOpen::Request { request })
                .is_err()
        );
        let mut changed = store
            .get_placement("allowed")?
            .context("Missing placement")?
            .config;
        changed["hosting"]["port"] = json!(8081);
        store.upsert_placement("allowed", &changed, DesiredState::Running)?;
        assert!(authority.check_target(&target).is_err());
        store.record_observed("allowed", ObservedState::Running, None, None, Some(2))?;
        store.connection.execute("UPDATE placement_replicas SET config_revision=2,applied_revision=2 WHERE placement_id='allowed'", [])?;
        assert_eq!(authority.target("allowed", "hosting")?.address.port(), 8081);
        assert!(authority.check_target(&target).is_err());
        policy.policy_version = 2;
        policy.previous_policy_digest = Some(compact_digest(&signed));
        policy.grants.clear();
        store.accept_management_policy(
            &sign_management_policy(&policy, &owner)?,
            &owner.public_key(),
            "device",
            now,
        )?;
        assert!(authority.check().is_err());
        assert!(authority.target("allowed", "hosting").is_err());
        Ok(())
    }

    #[cfg(feature = "runtime")]
    struct ModelFixture {
        directory: tempfile::TempDir,
        service: Arc<ManagementService>,
        host: Arc<ModelHost>,
        signer: SigningKey,
        controller: SigningKey,
        now: i64,
    }

    #[cfg(feature = "runtime")]
    async fn model_fixture() -> ModelFixture {
        use crate::models::{
            engines::fake::{FakeBehaviour, FakeLauncher},
            fetch::{AddressPolicy, Fetcher},
            host::{HostConfig, HostParts},
        };
        let directory = tempfile::tempdir().expect("a state directory");
        let (signer, controller) = (SigningKey::generate(), SigningKey::generate());
        let device = DeviceSession::test_management_session(
            "http://127.0.0.1:1/api/v1".into(),
            "device".into(),
            SigningKey::generate(),
            controller.public_key(),
        )
        .test_with_invitation_key(signer.public_key());
        let service =
            ManagementService::new(directory.path().into(), Arc::new(device), "boot".into());
        let now = unix_time().expect("a clock");
        service
            .refresh_authority(now + 300)
            .expect("an admitted device");
        let parts = HostParts {
            fetcher: Fetcher::new(AddressPolicy::global_only()).expect("a fetcher"),
            runtime_source: None,
            launcher: Arc::new(FakeLauncher {
                behaviour: FakeBehaviour::default(),
            }),
        };
        let host = ModelHost::start(directory.path(), HostConfig::default(), parts)
            .await
            .expect("a model host");
        ModelFixture {
            directory,
            service,
            host,
            signer,
            controller,
            now,
        }
    }

    #[cfg(feature = "runtime")]
    impl ModelFixture {
        fn grant(
            &self,
            id: &str,
            key: &SigningKey,
            scope: ManagementScope,
            capabilities: Vec<ManagementCapability>,
        ) -> ManagementGrant {
            ManagementGrant {
                grant_id: id.into(),
                user_id: id.into(),
                controller_key: key.public_key(),
                scope,
                capabilities,
                expires_at: self.now + 1_000,
                group_id: None,
                group_version: None,
            }
        }

        fn device_grant(
            &self,
            id: &str,
            key: &SigningKey,
            capability: ManagementCapability,
        ) -> ManagementGrant {
            self.grant(id, key, ManagementScope::Device, vec![capability])
        }

        fn store(&self) -> StateStore {
            StateStore::open(&self.directory.path().join("management.sqlite"))
                .expect("the management store")
        }

        /// Accepts policy `version`, which follows the policy `previous` when there is one.
        fn accept(
            &self,
            version: u64,
            previous: Option<&str>,
            grants: Vec<ManagementGrant>,
        ) -> String {
            let policy = ManagementPolicy {
                version: 1,
                device_id: "device".into(),
                policy_version: version,
                previous_policy_digest: previous.map(compact_digest),
                grants,
                issued_at: self.now,
                expires_at: self.now + 1_000,
            };
            let signed = sign_management_policy(&policy, &self.signer).expect("a signed policy");
            self.store()
                .accept_management_policy(&signed, &self.signer.public_key(), "device", self.now)
                .expect("an accepted policy");
            signed
        }

        fn connect(&self, grant: &str, key: &SigningKey) -> TunnelAuthority {
            let certificate = ControllerCertificate {
                version: 1,
                device_id: "device".into(),
                grant_id: grant.into(),
                session_id: format!("session-{grant}"),
                management_key: x25519_dalek::x25519([9; 32], x25519_dalek::X25519_BASEPOINT_BYTES),
                issued_at: self.now,
                expires_at: self.now + 300,
            };
            let signed = sign_controller_certificate(&certificate, key).expect("a certificate");
            self.service
                .connect_tunnel(&signed, grant, &certificate.session_id)
                .expect("a tunnel")
                .0
        }

        fn gateway(&self, tunnel: &TunnelAuthority) -> GatewayTarget {
            gateway_of(&self.host, &tunnel.authority().expect("a current grant"))
        }

        fn push(&self, grant: &str, key: &SigningKey, job_id: &str) -> Result<ModelAssetTarget> {
            let authority = self.connect(grant, key).authority()?;
            asset_target(
                &self.store(),
                &authority,
                self.host.acquisition(),
                job_id,
                0,
            )
        }
    }

    #[cfg(feature = "runtime")]
    fn gateway_open() -> flow_like_device_protocol::TunnelOpen {
        flow_like_device_protocol::TunnelOpen {
            placement_id: String::new(),
            service_id: String::new(),
            mode: flow_like_device_protocol::TunnelMode::Http,
            target: flow_like_device_protocol::TunnelTarget::ModelGateway,
        }
    }

    /// One open job that a journaled Ensure of `project` asked for, with the keys of a model
    /// manager, a deployer of `project`, a deployer of `other` and a status reader.
    #[cfg(feature = "runtime")]
    async fn push_fixture() -> (ModelFixture, [SigningKey; 4], String) {
        let fixture = model_fixture().await;
        let keys = [(); 4].map(|()| SigningKey::generate());
        let project = |project_id: &str| ManagementScope::Project {
            project_id: project_id.into(),
        };
        let deploy = || vec![ManagementCapability::Deploy];
        let grants = vec![
            fixture.device_grant("manager", &keys[0], ManagementCapability::ModelManage),
            fixture.grant("deployer", &keys[1], project("project"), deploy()),
            fixture.grant("stranger", &keys[2], project("other"), deploy()),
            fixture.device_grant("viewer", &keys[3], ManagementCapability::Status),
        ];
        fixture.accept(1, None, grants);
        let asset = ModelAssetDescriptor {
            digest: ModelAssetDigest {
                algorithm: DigestAlgorithm::Sha256,
                hex: "c".repeat(64),
            },
            size: 4_096,
            file_name: "weights.gguf".into(),
            sources: vec![],
        };
        let ensured = fixture.host.acquisition().ensure(&asset, Some("ensure-op"));
        let job = ensured
            .expect("an ensured asset")
            .job_id
            .expect("an open job");
        fixture.store().connection.execute("INSERT INTO management_operations(operation_id,request_digest,principal,project_id,accepted_at,result_json) VALUES('ensure-op','digest','deployer','project',?1,'{}')", [fixture.now]).expect("a journaled ensure");
        (fixture, keys, job)
    }

    #[cfg(feature = "runtime")]
    #[tokio::test]
    async fn model_gateway_streams_send_as_their_grant() {
        let fixture = model_fixture().await;
        let key = SigningKey::generate();
        let user = fixture.device_grant("user", &key, ManagementCapability::ModelUse);
        fixture.accept(1, None, vec![user]);
        let tunnel = fixture.connect("user", &key);
        let target = fixture.gateway(&tunnel);
        let grant = ModelConsumer::Grant {
            grant_id: "user".into(),
        };
        assert_eq!(target.consumer, grant);
        assert_eq!(target.address, fixture.host.gateway().address());
        assert_eq!(*target.secret, fixture.host.gateway().agent_secret());
        tunnel
            .check_gateway(&target)
            .expect("the grant keeps its stream");
        let owner = fixture.connect("owner", &fixture.controller);
        assert_eq!(fixture.gateway(&owner).consumer, ModelConsumer::Owner);
        let other = owner.check_gateway(&target);
        assert!(other.is_err(), "a stream keeps its principal");
        let named = flow_like_device_protocol::TunnelOpen {
            placement_id: "allowed".into(),
            service_id: "hosting".into(),
            ..gateway_open()
        };
        let named = tunnel.open_target(&named);
        assert!(
            named.is_err(),
            "a model gateway open named a deployed service"
        );
    }

    #[cfg(feature = "runtime")]
    #[tokio::test]
    async fn model_gateway_access_ends_without_use_models() {
        let fixture = model_fixture().await;
        let (user_key, viewer_key) = (SigningKey::generate(), SigningKey::generate());
        let user = fixture.device_grant("user", &user_key, ManagementCapability::ModelUse);
        let viewer = fixture.device_grant("viewer", &viewer_key, ManagementCapability::Status);
        let first = fixture.accept(1, None, vec![user.clone(), viewer]);
        let viewer = fixture.connect("viewer", &viewer_key);
        let refused = viewer.open_target(&gateway_open()).err();
        let refused = refused.expect("a grant without Use models opened the model gateway");
        assert_eq!(rejection_code(&refused), RejectionCode::Unauthorized);
        let tunnel = fixture.connect("user", &user_key);
        let target = fixture.gateway(&tunnel);
        let demoted = ManagementGrant {
            capabilities: vec![ManagementCapability::Status],
            ..user
        };
        fixture.accept(2, Some(&first), vec![demoted]);
        assert!(tunnel.check_gateway(&target).is_err());
    }

    #[cfg(feature = "runtime")]
    #[tokio::test]
    async fn model_asset_pushes_need_manage_models_or_the_deploy_that_asked() {
        let (fixture, keys, job) = push_fixture().await;
        let managed = fixture.push("manager", &keys[0], &job);
        let managed = managed.expect("a model manager pushes");
        assert_eq!(managed.job_id, job);
        let deployed = fixture.push("deployer", &keys[1], &job);
        let deployed = deployed.expect("the deployer whose deploy asked pushes");
        assert_eq!(deployed.digest, managed.digest);
        let owner = fixture.push("owner", &fixture.controller, &job);
        let owner = owner.expect("the owner pushes");
        let take_over = [&owner, &managed, &deployed].map(|target| target.take_over);
        assert_eq!(
            take_over,
            [true, true, false],
            "only the owner and Manage models stop the device's own download"
        );
        let stranger = fixture.push("stranger", &keys[2], &job).err();
        let stranger = stranger.expect("a deployer of another project pushed to this job");
        assert_eq!(rejection_code(&stranger), RejectionCode::Unauthorized);
        let open = TunnelDataOpen::ModelAsset {
            job_id: job.clone(),
            offset: 0,
        };
        let viewer = fixture
            .connect("viewer", &keys[3])
            .internal_target(&open)
            .err();
        let viewer = viewer.expect("a grant without Manage models or Deploy pushed a model file");
        assert_eq!(rejection_code(&viewer), RejectionCode::Unauthorized);
    }

    #[cfg(feature = "runtime")]
    #[tokio::test]
    async fn model_asset_pushes_need_an_open_job_and_keep_their_principal() {
        let (fixture, keys, job) = push_fixture().await;
        let closed = "12345678-1234-1234-1234-123456789abc";
        let unknown = fixture.push("manager", &keys[0], closed).err();
        let unknown = unknown.expect("a push reached a job that is not open");
        assert_eq!(rejection_code(&unknown), RejectionCode::Invalid);
        let managed = fixture.push("manager", &keys[0], &job);
        let managed = InternalTarget::ModelAsset(managed.expect("a model manager pushes"));
        let deployer = fixture.connect("deployer", &keys[1]);
        let other = deployer.check_internal(&managed);
        assert!(other.is_err(), "a push stream keeps its principal");
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ServiceTarget {
    pub address: SocketAddr,
    pub placement_id: String,
    pub service_id: String,
    pub mode: flow_like_device_protocol::TunnelMode,
    pub tls: Option<ServiceTlsTarget>,
    config_revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ServiceTlsTarget {
    pub server_name: String,
    pub sha256_fingerprint: String,
}

impl ManagementService {
    pub(crate) fn connect_tunnel(
        self: &Arc<Self>,
        certificate_jws: &str,
        grant_id: &str,
        session_id: &str,
    ) -> Result<(TunnelAuthority, noise::Handshake)> {
        let connection = self.connect(certificate_jws, grant_id, session_id)?;
        let handshake = self.device.tunnel_noise_responder(
            &connection.certificate.management_key,
            &connection.certificate.session_id,
        )?;
        Ok((
            TunnelAuthority {
                service: self.clone(),
                certificate: connection.certificate,
                signer: connection.signer,
            },
            handshake,
        ))
    }
}

impl TunnelAuthority {
    pub(crate) fn internal_target(&self, open: &TunnelDataOpen) -> Result<InternalTarget> {
        open.validate()?;
        let authority = self.authority()?;
        let store = StateStore::open(&self.service.state_dir.join("management.sqlite"))?;
        match open {
            TunnelDataOpen::Request { request } => {
                ensure!(bulk_read(&request.command), "Command is not a bulk read");
                validate_request(
                    request,
                    &self.service.device.manifest().device_id,
                    unix_time()?,
                )?;
                Ok(InternalTarget::Read(Arc::new(ReadTarget {
                    request: request.clone(),
                    authority,
                })))
            }
            TunnelDataOpen::Artifact {
                project_id,
                transfer_id,
                file_index,
                offset,
            } => {
                authority.require(ManagementCapability::Deploy, Some(project_id), None)?;
                let status = crate::project_artifacts::status(
                    &store,
                    &self.service.state_dir,
                    project_id,
                    transfer_id,
                    &authority.principal,
                    *file_index,
                )?;
                ensure!(
                    status.state == ArtifactTransferState::Receiving && *offset <= status.offset,
                    "Artifact stream is not resumable at the requested offset"
                );
                Ok(InternalTarget::Artifact(ArtifactTarget {
                    project_id: project_id.clone(),
                    transfer_id: transfer_id.clone(),
                    file_index: *file_index,
                    offset: *offset,
                    principal: authority.principal,
                }))
            }
            TunnelDataOpen::ModelAsset { job_id, offset } => {
                model_asset_target(&store, &authority, job_id, *offset)
            }
        }
    }

    /// The listener an open names. The model gateway needs Use models and a running host.
    pub(crate) fn open_target(
        &self,
        open: &flow_like_device_protocol::TunnelOpen,
    ) -> Result<OpenTarget> {
        open.validate()?;
        match open.target {
            flow_like_device_protocol::TunnelTarget::Service => self
                .target_mode(&open.placement_id, &open.service_id, open.mode)
                .map(OpenTarget::Service),
            flow_like_device_protocol::TunnelTarget::ModelGateway => {
                let authority = self.authority()?;
                authority.require(ManagementCapability::ModelUse, None, None)?;
                gateway(&authority)
            }
        }
    }

    /// The grant still holds Use models and is the principal the stream sends as.
    #[cfg(feature = "runtime")]
    pub(crate) fn check_gateway(&self, target: &GatewayTarget) -> Result<()> {
        let authority = self.authority()?;
        authority.require(ManagementCapability::ModelUse, None, None)?;
        ensure!(
            consumer_of(&authority) == target.consumer,
            "Model gateway principal changed"
        );
        Ok(())
    }

    pub(crate) fn check_internal(&self, target: &InternalTarget) -> Result<()> {
        let authority = self.authority()?;
        let store = StateStore::open(&self.service.state_dir.join("management.sqlite"))?;
        match target {
            InternalTarget::Read(target) => {
                validate_request(
                    &target.request,
                    &self.service.device.manifest().device_id,
                    unix_time()?,
                )?;
                target.authority.require_current(
                    &store,
                    self.service.device.manifest(),
                    unix_time()?,
                )
            }
            InternalTarget::Artifact(target) => self.check_artifact(&store, &authority, target),
            #[cfg(feature = "runtime")]
            InternalTarget::ModelAsset(target) => check_model_asset(&store, &authority, target),
        }
    }

    fn check_artifact(
        &self,
        store: &StateStore,
        authority: &Authority,
        target: &ArtifactTarget,
    ) -> Result<()> {
        authority.require(ManagementCapability::Deploy, Some(&target.project_id), None)?;
        ensure!(
            authority.principal == target.principal,
            "Artifact stream principal changed"
        );
        let receiving: bool = store.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM project_artifact_transfers WHERE transfer_id=?1 AND project_id=?2 AND principal=?3 AND state='receiving' AND expires_at>?4)",
            params![target.transfer_id, target.project_id, target.principal, unix_time()?], |row| row.get(0))?;
        ensure!(receiving, "Artifact transfer is no longer receiving");
        Ok(())
    }

    pub(crate) fn read_internal(&self, target: &ReadTarget) -> Result<Vec<u8>> {
        self.check_internal(&InternalTarget::Read(Arc::new(ReadTarget {
            request: target.request.clone(),
            authority: target.authority.clone(),
        })))?;
        let authority = self.authority()?;
        let mut store = StateStore::open(&self.service.state_dir.join("management.sqlite"))?;
        let now = unix_time()?;
        let manifest = self.service.device.manifest();
        let response = match target.request.command {
            ManagementCommand::TelemetryRead { .. }
            | ManagementCommand::TelemetryRosterRead { .. } => {
                execute_telemetry_group(&store, &authority, &target.request, &self.service, now)
            }
            ManagementCommand::Models { .. } => super::models::read(
                &store,
                &authority,
                &target.request,
                manifest,
                now,
                MODELS_BULK_REPLY_MAX_BYTES,
            ),
            _ => execute(
                &mut store,
                &authority,
                &target.request,
                manifest,
                &self.service.boot_id,
                &self.service.state_dir,
                now,
            ),
        }
        .unwrap_or_else(|error| {
            rejected(
                &authority,
                &target.request.operation_id,
                rejection_code(&error),
                format!("{error:#}"),
            )
        });
        self.check_lease()?;
        bounded_response(&response)
    }

    pub(crate) fn write_artifact(
        &self,
        target: &ArtifactTarget,
        offset: u64,
        bytes: &[u8],
    ) -> Result<ArtifactTransferStatus> {
        let authority = self.authority()?;
        let store = StateStore::open(&self.service.state_dir.join("management.sqlite"))?;
        self.check_artifact(&store, &authority, target)?;
        crate::project_artifacts::chunk_bytes(
            &store,
            &self.service.state_dir,
            &target.project_id,
            &target.transfer_id,
            &target.principal,
            target.file_index,
            offset,
            bytes,
        )
    }

    pub(crate) fn artifact_status(
        &self,
        target: &ArtifactTarget,
    ) -> Result<ArtifactTransferStatus> {
        let authority = self.authority()?;
        let store = StateStore::open(&self.service.state_dir.join("management.sqlite"))?;
        self.check_artifact(&store, &authority, target)?;
        crate::project_artifacts::status(
            &store,
            &self.service.state_dir,
            &target.project_id,
            &target.transfer_id,
            &target.principal,
            target.file_index,
        )
    }

    pub(crate) fn expires_at(&self) -> i64 {
        self.certificate.expires_at
    }
    pub(crate) fn check(&self) -> Result<()> {
        self.authority().map(|_| ())
    }

    pub(crate) fn check_lease(&self) -> Result<()> {
        let now = unix_time()?;
        ensure!(
            now < self.certificate.expires_at
                && now < self.service.authority_until.load(Ordering::Acquire),
            "Tunnel authorization expired"
        );
        Ok(())
    }

    fn authority(&self) -> Result<Authority> {
        self.check_lease()?;
        let now = unix_time()?;
        let store = StateStore::open(&self.service.state_dir.join("management.sqlite"))?;
        let authority = self
            .service
            .current_authority(&store, &self.certificate.grant_id, now)?;
        ensure!(
            authority.key == self.signer,
            "Tunnel controller was revoked"
        );
        Ok(authority)
    }

    pub(crate) fn renew(&self, certificate_jws: &str) -> Result<(Self, noise::Handshake)> {
        self.check()?;
        let certificate =
            verify_controller_certificate(certificate_jws, &self.signer, unix_time()?)?;
        ensure!(
            certificate.session_id != self.certificate.session_id,
            "Tunnel renewal must use a fresh session identity"
        );
        let (renewed, handshake) = self.service.connect_tunnel(
            certificate_jws,
            &self.certificate.grant_id,
            &certificate.session_id,
        )?;
        ensure!(
            renewed.signer == self.signer,
            "Tunnel renewal changed controller"
        );
        ensure!(
            renewed.certificate.management_key != self.certificate.management_key,
            "Tunnel renewal must rotate its key"
        );
        ensure!(
            renewed.certificate.expires_at > self.certificate.expires_at,
            "Tunnel renewal must extend authorization"
        );
        Ok((renewed, handshake))
    }

    /// Callers name a deployed service. They never choose a network destination.
    #[cfg(test)]
    pub(crate) fn target(&self, placement_id: &str, service_id: &str) -> Result<ServiceTarget> {
        self.target_mode(
            placement_id,
            service_id,
            flow_like_device_protocol::TunnelMode::Tcp,
        )
    }

    pub(crate) fn target_mode(
        &self,
        placement_id: &str,
        service_id: &str,
        mode: flow_like_device_protocol::TunnelMode,
    ) -> Result<ServiceTarget> {
        use crate::config::TunnelServiceProtocol;
        use flow_like_device_protocol::TunnelMode;
        validate_management_id(placement_id)?;
        validate_management_id(service_id)?;
        let authority = self.authority()?;
        let store = StateStore::open(&self.service.state_dir.join("management.sqlite"))?;
        let record = store
            .get_placement(placement_id)?
            .context("Unknown placement")?;
        require_serving(&record)?;
        let config: PlacementConfig = serde_json::from_value(record.config)?;
        authority.require(
            ManagementCapability::ServiceConnect,
            Some(&config.project_id),
            Some(placement_id),
        )?;
        let (host, port, tls) = if service_id == "hosting" {
            let hosting = config.hosting.context("Placement has no hosted service")?;
            hosting.validate()?;
            let tls = if mode == TunnelMode::Http {
                config
                    .tls_certificate_id
                    .as_deref()
                    .map(|id| {
                        let metadata = crate::certificates::metadata(&store, id)?;
                        let now = unix_time()?;
                        ensure!(
                            metadata.not_before <= now && now < metadata.not_after,
                            "Hosting certificate is outside its validity period"
                        );
                        let server_name = certificate_server_name(&metadata)
                            .context("Hosting certificate has no server name")?;
                        Ok::<_, anyhow::Error>(ServiceTlsTarget {
                            server_name,
                            sha256_fingerprint: metadata.sha256_fingerprint,
                        })
                    })
                    .transpose()?
            } else {
                None
            };
            (hosting.host, hosting.port, tls)
        } else {
            let configured = config
                .tunnel_services
                .iter()
                .find(|service| service.id == service_id)
                .context("Unknown deployed service")?;
            configured.validate()?;
            ensure!(
                mode == TunnelMode::Tcp || configured.protocol != TunnelServiceProtocol::Tcp,
                "HTTP mode requires an HTTP service"
            );
            let tls = if mode == TunnelMode::Http
                && configured.protocol == TunnelServiceProtocol::Https
            {
                Some(ServiceTlsTarget {
                    server_name: configured
                        .tls_server_name
                        .clone()
                        .context("Missing TLS server name")?,
                    sha256_fingerprint: configured
                        .tls_sha256_fingerprint
                        .clone()
                        .context("Missing TLS fingerprint")?,
                })
            } else {
                None
            };
            (configured.host, configured.port, tls)
        };
        let host = match host {
            IpAddr::V4(host) if host.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
            IpAddr::V6(host) if host.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
            host => host,
        };
        Ok(ServiceTarget {
            address: SocketAddr::new(host, port),
            placement_id: placement_id.into(),
            service_id: service_id.into(),
            mode,
            tls,
            config_revision: record.config_revision,
        })
    }

    pub(crate) fn check_target(&self, target: &ServiceTarget) -> Result<()> {
        ensure!(
            self.target_mode(&target.placement_id, &target.service_id, target.mode)? == *target,
            "Deployed service changed"
        );
        Ok(())
    }
}
