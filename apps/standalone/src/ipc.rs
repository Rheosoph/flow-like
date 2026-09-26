use crate::{broker::WorkloadBroker, config::PlacementConfig, state::StateStore};
use anyhow::{Context, Result, ensure};
use flow_like_types_contracts::authorization::{
    AuthorizationAttribution, AuthorizationError, AuthorizationFuture, AuthorizationRequest,
    RequestAuthorization, RequestAuthorizer, ResourceAudience,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    path::PathBuf,
    sync::Arc,
    time::{Duration, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixStream,
    sync::Mutex,
};
use tokio_util::sync::CancellationToken;
use zeroize::{Zeroize, Zeroizing};

const MAX_FRAME_BYTES: usize = 2 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(75);
pub const CHILD_BROKER_FD: i32 = 3;
pub const CHILD_PLACEMENT_LOCK_FD: i32 = 4;
pub const CHILD_LISTENER_FD: i32 = 5;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChildBootstrap {
    pub config: PlacementConfig,
    #[serde(default)]
    pub data_root: Option<PathBuf>,
    #[serde(default)]
    pub replica_slot: u8,
    #[serde(default)]
    pub inherited_listener: bool,
    pub config_revision: u64,
    pub intent_revision: u64,
    pub parent_pid: u32,
    pub api_base_url: Option<String>,
    #[serde(default)]
    pub workload_identity: Option<crate::config::WorkloadIdentity>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
enum ChildRequest {
    TlsIdentity {
        known_revision: Option<u64>,
    },
    #[cfg(feature = "runtime")]
    OutageSeal {
        claim: crate::online::outage::SnapshotClaim,
    },
    #[cfg(feature = "runtime")]
    OutageVerify {
        claim: crate::online::outage::SnapshotClaim,
        seal: String,
    },
    #[cfg(feature = "runtime")]
    OutageDeny {
        binding: String,
    },
    Ready,
    Usage {
        snapshot: crate::usage::RuntimeUsageSnapshot,
        #[serde(default)]
        finalized: bool,
    },
    Authorize {
        method: String,
        url: String,
    },
    AuthorizeProject {
        method: String,
        url: String,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case", deny_unknown_fields)]
enum ParentResponse {
    TlsIdentity {
        identity: Option<crate::certificates::CertificateIdentity>,
    },
    #[cfg(feature = "runtime")]
    OutageSealed {
        seal: String,
    },
    #[cfg(feature = "runtime")]
    OutageAccepted,
    Ready,
    Usage {
        sequence: u64,
    },
    Authorization {
        authorization: String,
        dpop: String,
        expires_at: u64,
    },
    Error {
        code: String,
    },
}

impl Drop for ParentResponse {
    fn drop(&mut self) {
        if let Self::Authorization {
            authorization,
            dpop,
            ..
        } = self
        {
            authorization.zeroize();
            dpop.zeroize();
        }
    }
}

async fn write_frame<T: Serialize>(stream: &mut UnixStream, message: &T) -> Result<()> {
    let bytes = Zeroizing::new(serde_json::to_vec(message)?);
    ensure!(bytes.len() <= MAX_FRAME_BYTES, "Broker frame is too large");
    stream.write_u32(bytes.len().try_into()?).await?;
    stream.write_all(&bytes).await?;
    stream.flush().await?;
    Ok(())
}

async fn read_frame<T: DeserializeOwned>(stream: &mut UnixStream) -> Result<T> {
    let length = stream.read_u32().await? as usize;
    ensure!(
        length > 0 && length <= MAX_FRAME_BYTES,
        "Invalid broker frame length"
    );
    let mut bytes = Zeroizing::new(vec![0; length]);
    stream.read_exact(&mut bytes).await?;
    serde_json::from_slice(&bytes).context("Invalid broker frame")
}

/// The parent endpoint never enters the child. CLOEXEC remains set except for
/// the selected socket duplicated to descriptor 3 in that child's pre-exec hook.
pub fn socket_pair() -> Result<(UnixStream, OwnedFd)> {
    let (parent, child) = std::os::unix::net::UnixStream::pair()?;
    parent.set_nonblocking(true)?;
    Ok((UnixStream::from_std(parent)?, child.into()))
}

pub fn attach_child_descriptors(
    command: &mut tokio::process::Command,
    socket: &OwnedFd,
    placement_lock: &std::fs::File,
) -> Result<[OwnedFd; 2]> {
    let socket = duplicate_launch_descriptor(socket.as_raw_fd())?;
    let placement_lock = duplicate_launch_descriptor(placement_lock.as_raw_fd())?;
    let descriptors = [socket, placement_lock];
    let sources = [descriptors[0].as_raw_fd(), descriptors[1].as_raw_fd()];
    // SAFETY: the pre-exec closure uses only async-signal-safe libc calls. The
    // The returned descriptors stay alive through spawn. Duplicating above both
    // destination slots avoids clobbering the other source during dup2.
    unsafe {
        command.pre_exec(move || {
            for (source, target) in sources
                .into_iter()
                .zip([CHILD_BROKER_FD, CHILD_PLACEMENT_LOCK_FD])
            {
                if libc::dup2(source, target) < 0 || libc::fcntl(target, libc::F_SETFD, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
    Ok(descriptors)
}

fn duplicate_launch_descriptor(descriptor: i32) -> Result<OwnedFd> {
    // SAFETY: fcntl creates a new descriptor owned by this call on success.
    let duplicate = unsafe { libc::fcntl(descriptor, libc::F_DUPFD_CLOEXEC, 6) };
    ensure!(
        duplicate >= 6,
        "Cannot prepare inherited workload descriptor"
    );
    Ok(unsafe { OwnedFd::from_raw_fd(duplicate) })
}

pub fn attach_listener(
    command: &mut tokio::process::Command,
    listener: &std::net::TcpListener,
) -> Result<OwnedFd> {
    let source = duplicate_launch_descriptor(listener.as_raw_fd())?;
    let descriptor = source.as_raw_fd();
    unsafe {
        command.pre_exec(move || {
            if libc::dup2(descriptor, CHILD_LISTENER_FD) < 0
                || libc::fcntl(CHILD_LISTENER_FD, libc::F_SETFD, 0) < 0
            {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    Ok(source)
}
fn validate_listener_socket(fd: i32) -> Result<()> {
    let mut kind = 0i32;
    let mut length = std::mem::size_of::<i32>() as libc::socklen_t;
    unsafe {
        ensure!(
            libc::fcntl(fd, libc::F_GETFD) >= 0,
            "Inherited service listener is missing"
        );
        let result = libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_TYPE,
            (&mut kind as *mut i32).cast(),
            &mut length,
        );
        ensure!(
            result == 0,
            "Cannot inspect inherited service socket: {}",
            std::io::Error::last_os_error()
        );
        ensure!(
            length as usize == std::mem::size_of::<i32>() && kind == libc::SOCK_STREAM,
            "Inherited service socket is not a stream"
        );
    }
    #[cfg(target_os = "macos")]
    {
        // Darwin exposes SO_ACCEPTCONN as a kernel flag but rejects it through
        // getsockopt. TCP_CONNECTION_INFO reports TCPS_LISTEN from tcp_fsm.h.
        let mut info: libc::tcp_connection_info = unsafe { std::mem::zeroed() };
        let mut length = std::mem::size_of_val(&info) as libc::socklen_t;
        let result = unsafe {
            libc::getsockopt(
                fd,
                libc::IPPROTO_TCP,
                libc::TCP_CONNECTION_INFO,
                (&mut info as *mut libc::tcp_connection_info).cast(),
                &mut length,
            )
        };
        ensure!(
            result == 0,
            "Cannot inspect inherited TCP state: {}",
            std::io::Error::last_os_error()
        );
        ensure!(
            length > 0 && (length as usize) <= std::mem::size_of_val(&info) && info.tcpi_state == 1,
            "Inherited TCP socket is not listening"
        );
    }
    #[cfg(not(target_os = "macos"))]
    {
        let mut accepts = 0i32;
        let mut length = std::mem::size_of::<i32>() as libc::socklen_t;
        let result = unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_ACCEPTCONN,
                (&mut accepts as *mut i32).cast(),
                &mut length,
            )
        };
        ensure!(
            result == 0,
            "Cannot inspect inherited listener state: {}",
            std::io::Error::last_os_error()
        );
        ensure!(
            length as usize == std::mem::size_of::<i32>() && accepts != 0,
            "Inherited socket is not listening"
        );
    }
    Ok(())
}
pub fn inherited_listener(
    hosting: &crate::config::HostingConfig,
) -> Result<tokio::net::TcpListener> {
    let fd = CHILD_LISTENER_FD;
    validate_listener_socket(fd)?;
    let duplicate = duplicate_launch_descriptor(fd)?;
    let probe = std::net::TcpListener::from(duplicate);
    ensure!(
        probe.local_addr()? == std::net::SocketAddr::new(hosting.host, hosting.port),
        "Inherited service listener address differs"
    );
    drop(probe);
    unsafe {
        ensure!(
            libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) == 0,
            "Protect inherited listener"
        );
    }
    let listener = unsafe { std::net::TcpListener::from_raw_fd(fd) };
    listener.set_nonblocking(true)?;
    Ok(tokio::net::TcpListener::from_std(listener)?)
}

pub fn inherited_placement_lock(descriptor: i32) -> Result<std::fs::File> {
    ensure!(
        descriptor == CHILD_PLACEMENT_LOCK_FD,
        "Unsupported placement lock descriptor"
    );
    let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
    // SAFETY: fstat initializes the live stat buffer on success. Validation takes
    // no ownership, so a missing descriptor produces an ordinary error.
    unsafe {
        ensure!(
            libc::fcntl(descriptor, libc::F_GETFD) >= 0
                && libc::fstat(descriptor, metadata.as_mut_ptr()) == 0,
            "Workload placement lock descriptor is missing"
        );
        let metadata = metadata.assume_init();
        ensure!(
            metadata.st_mode & libc::S_IFMT == libc::S_IFREG
                && metadata.st_uid == libc::geteuid()
                && metadata.st_mode & 0o077 == 0,
            "Workload placement lock must be a private file owned by this user"
        );
        ensure!(
            libc::fcntl(descriptor, libc::F_SETFD, libc::FD_CLOEXEC) == 0,
            "Cannot protect the placement lock descriptor"
        );
        Ok(std::fs::File::from_raw_fd(descriptor))
    }
}

/// One channel is shared by retained clients. Serial requests prevent response
/// confusion and cap the broker to one in-flight request per workload.
pub struct ChildBroker {
    stream: Mutex<Option<UnixStream>>,
    resource_base: Option<String>,
    identity: Option<crate::config::WorkloadIdentity>,
}

pub(crate) enum TlsIdentityUpdate {
    Busy,
    Unchanged,
    Changed(crate::certificates::CertificateIdentity),
}

impl ChildBroker {
    pub async fn inherited(descriptor: i32) -> Result<(ChildBootstrap, Arc<Self>)> {
        ensure!(
            descriptor == CHILD_BROKER_FD,
            "Unsupported broker descriptor"
        );
        validate_inherited_socket(descriptor)?;
        // SAFETY: this hidden child command takes ownership of the descriptor
        // inherited from its supervisor exactly once.
        let socket = unsafe { std::os::unix::net::UnixStream::from_raw_fd(descriptor) };
        // Do not leak broker access to commands launched by workflow nodes.
        ensure!(
            unsafe { libc::fcntl(descriptor, libc::F_SETFD, libc::FD_CLOEXEC) } == 0,
            "Cannot protect the broker descriptor"
        );
        socket.set_nonblocking(true)?;
        Self::connect(UnixStream::from_std(socket)?).await
    }

    pub(crate) async fn connect(mut stream: UnixStream) -> Result<(ChildBootstrap, Arc<Self>)> {
        let bootstrap: ChildBootstrap =
            tokio::time::timeout(Duration::from_secs(15), read_frame(&mut stream)).await??;
        bootstrap.config.validate()?;
        if let Some(root) = &bootstrap.data_root {
            crate::placement_data::validate_root(root, &bootstrap.config)?;
        }
        if let Some(identity) = &bootstrap.workload_identity {
            identity.validate()?;
        }
        ensure!(
            bootstrap.config.resource_grant.is_none() || bootstrap.workload_identity.is_some(),
            "Missing workload identity binding"
        );
        ensure!(bootstrap.parent_pid > 0, "Invalid supervisor process ID");
        let resource_base = bootstrap
            .api_base_url
            .as_ref()
            .map(|base| {
                flow_like_device_protocol::canonical_api_base_url(base)
                    .map(|base| format!("{base}/instances"))
            })
            .transpose()?;
        ensure!(
            bootstrap.config.resource_grant.is_none() || resource_base.is_some(),
            "Resource placement is missing its enrolled API"
        );
        let identity = bootstrap.workload_identity.clone();
        Ok((
            bootstrap,
            Arc::new(Self {
                stream: Mutex::new(Some(stream)),
                resource_base,
                identity,
            }),
        ))
    }

    pub fn identity(&self) -> Option<crate::config::WorkloadIdentity> {
        self.identity.clone()
    }

    async fn request(&self, request: &ChildRequest) -> Result<ParentResponse> {
        let mut guard = self.stream.lock().await;
        Self::exchange(&mut guard, request).await
    }

    async fn exchange(
        guard: &mut Option<UnixStream>,
        request: &ChildRequest,
    ) -> Result<ParentResponse> {
        let mut stream = guard
            .take()
            .context("Credential broker channel is closed")?;
        let response = tokio::time::timeout(REQUEST_TIMEOUT, async {
            write_frame(&mut stream, request).await?;
            read_frame::<ParentResponse>(&mut stream).await
        })
        .await
        .context("Credential broker timed out")??;
        *guard = Some(stream);
        Ok(response)
    }

    /// A busy resource exchange must not interrupt a still-valid TLS identity.
    /// Busy is distinct from an explicit supervisor denial or a closed channel.
    pub(crate) async fn try_tls_identity(&self, known_revision: u64) -> Result<TlsIdentityUpdate> {
        let Ok(mut guard) = self.stream.try_lock() else {
            return Ok(TlsIdentityUpdate::Busy);
        };
        match &mut Self::exchange(
            &mut guard,
            &ChildRequest::TlsIdentity {
                known_revision: Some(known_revision),
            },
        )
        .await?
        {
            ParentResponse::TlsIdentity { identity } => Ok(match identity.take() {
                Some(identity) => TlsIdentityUpdate::Changed(identity),
                None => TlsIdentityUpdate::Unchanged,
            }),
            _ => anyhow::bail!("Placement TLS identity unavailable"),
        }
    }

    #[cfg(test)]
    pub(crate) async fn lock_channel_for_test(
        &self,
    ) -> tokio::sync::MutexGuard<'_, Option<UnixStream>> {
        self.stream.lock().await
    }

    pub async fn tls_identity(
        &self,
        known_revision: Option<u64>,
    ) -> Result<Option<crate::certificates::CertificateIdentity>> {
        match &mut self
            .request(&ChildRequest::TlsIdentity { known_revision })
            .await?
        {
            ParentResponse::TlsIdentity { identity } => Ok(identity.take()),
            _ => anyhow::bail!("Placement TLS identity unavailable"),
        }
    }

    pub async fn ready(&self) -> Result<()> {
        ensure!(
            matches!(
                self.request(&ChildRequest::Ready).await?,
                ParentResponse::Ready
            ),
            "Supervisor did not accept workload readiness"
        );
        Ok(())
    }

    pub async fn report_usage(&self, snapshot: crate::usage::RuntimeUsageSnapshot) -> Result<()> {
        self.report_usage_checkpoint(snapshot, false).await
    }

    pub async fn report_final_usage(
        &self,
        snapshot: crate::usage::RuntimeUsageSnapshot,
    ) -> Result<()> {
        self.report_usage_checkpoint(snapshot, true).await
    }

    async fn report_usage_checkpoint(
        &self,
        snapshot: crate::usage::RuntimeUsageSnapshot,
        finalized: bool,
    ) -> Result<()> {
        let sequence = snapshot.sequence;
        ensure!(
            matches!(self.request(&ChildRequest::Usage { snapshot, finalized }).await?,
            ParentResponse::Usage { sequence: accepted } if accepted == sequence),
            "Supervisor did not accept workload usage"
        );
        Ok(())
    }
}

impl RequestAuthorizer for ChildBroker {
    fn attribution(&self) -> AuthorizationAttribution {
        AuthorizationAttribution::InstanceGrant
    }

    fn resource_base_url(&self, audience: ResourceAudience) -> Option<String> {
        self.resource_base.as_ref().map(|base| match audience {
            ResourceAudience::HostedModels => base.clone(),
            ResourceAudience::ProjectApi => format!("{base}/project"),
        })
    }
    fn authorize<'a>(&'a self, request: AuthorizationRequest<'a>) -> AuthorizationFuture<'a> {
        Box::pin(async move {
            if request.method.len() > 16 || request.url.len() > 4096 {
                return Err(AuthorizationError::InvalidRequest);
            }
            let response = self
                .request(&match request.audience {
                    ResourceAudience::HostedModels => ChildRequest::Authorize {
                        method: request.method.into(),
                        url: request.url.into(),
                    },
                    ResourceAudience::ProjectApi => ChildRequest::AuthorizeProject {
                        method: request.method.into(),
                        url: request.url.into(),
                    },
                })
                .await
                .map_err(|_| AuthorizationError::Unavailable)?;
            match &response {
                ParentResponse::Authorization {
                    authorization,
                    dpop,
                    expires_at,
                } => RequestAuthorization::new(
                    authorization.clone(),
                    Some(dpop.clone()),
                    UNIX_EPOCH + Duration::from_secs(*expires_at),
                ),
                ParentResponse::Error { code } if code == "denied" => {
                    Err(AuthorizationError::Denied)
                }
                ParentResponse::Error { code } if code == "expired" => {
                    Err(AuthorizationError::Expired)
                }
                ParentResponse::Error { code } if code == "invalid_request" => {
                    Err(AuthorizationError::InvalidRequest)
                }
                ParentResponse::Error { code } if code == "invalid_response" => {
                    Err(AuthorizationError::InvalidResponse)
                }
                _ => Err(AuthorizationError::Unavailable),
            }
        })
    }
}

#[cfg(feature = "runtime")]
#[async_trait::async_trait]
impl crate::online::outage::OutageAuthority for ChildBroker {
    async fn seal(&self, claim: &crate::online::outage::SnapshotClaim) -> Result<String> {
        match self
            .request(&ChildRequest::OutageSeal {
                claim: claim.clone(),
            })
            .await?
        {
            ParentResponse::OutageSealed { ref seal } => Ok(seal.clone()),
            ParentResponse::Error { ref code } if code == "denied" => {
                Err(AuthorizationError::Denied.into())
            }
            ParentResponse::Error { ref code } if code == "unavailable" => {
                Err(AuthorizationError::Unavailable.into())
            }
            _ => anyhow::bail!("Supervisor did not authorize the outage snapshot"),
        }
    }
    async fn verify(&self, claim: &crate::online::outage::SnapshotClaim, seal: &str) -> Result<()> {
        match self
            .request(&ChildRequest::OutageVerify {
                claim: claim.clone(),
                seal: seal.into(),
            })
            .await?
        {
            ParentResponse::OutageAccepted => Ok(()),
            ParentResponse::Error { ref code } if code == "denied" => {
                Err(AuthorizationError::Denied.into())
            }
            ParentResponse::Error { ref code } if code == "unavailable" => {
                Err(AuthorizationError::Unavailable.into())
            }
            _ => anyhow::bail!("Supervisor rejected the saved outage authorization"),
        }
    }
    async fn deny(&self, binding: &str) -> Result<()> {
        ensure!(
            matches!(
                self.request(&ChildRequest::OutageDeny {
                    binding: binding.into()
                })
                .await?,
                ParentResponse::OutageAccepted
            ),
            "Cannot persist outage authorization revocation"
        );
        Ok(())
    }
}

pub async fn serve(
    stream: UnixStream,
    bootstrap: ChildBootstrap,
    state_dir: PathBuf,
    process_id: u32,
    broker: Option<Arc<WorkloadBroker>>,
    cancel: CancellationToken,
) -> Result<()> {
    serve_with_drain(
        stream,
        bootstrap,
        state_dir,
        process_id,
        broker,
        cancel,
        CancellationToken::new(),
    )
    .await
}

pub(crate) async fn serve_with_drain(
    mut stream: UnixStream,
    bootstrap: ChildBootstrap,
    state_dir: PathBuf,
    process_id: u32,
    broker: Option<Arc<WorkloadBroker>>,
    cancel: CancellationToken,
    drain: CancellationToken,
) -> Result<()> {
    let mut prepared = false;
    let mut last_usage: Option<crate::usage::RuntimeUsageSnapshot> = None;
    let mut last_usage_at: Option<std::time::Instant> = None;
    let process_run_id = uuid::Uuid::new_v4().to_string();
    let usage_binding = crate::operational::ProcessBinding {
        run_id: process_run_id.clone(),
        placement_id: bootstrap.config.id.clone(),
        slot: bootstrap.replica_slot,
        process_id,
        config_revision: bootstrap.config_revision,
        intent_revision: bootstrap.intent_revision,
    };
    let mut usage_guard = crate::operational::UsageProcessGuard::new(&state_dir, &process_run_id);
    let mut usage_registered = false;
    tokio::select! { _ = cancel.cancelled() => return Ok(()),
    result = tokio::time::timeout(Duration::from_secs(15),write_frame(&mut stream,&bootstrap)) => result?? }
    loop {
        let request: ChildRequest = tokio::select! {
            _ = cancel.cancelled() => return Ok(()),
            request = read_frame(&mut stream) => request?,
        };
        let store = StateStore::open(&state_dir.join("management.sqlite"))?;
        let is_usage = matches!(&request, ChildRequest::Usage { .. });
        let current = store.replica_is_current(
            &bootstrap.config.id,
            bootstrap.replica_slot,
            bootstrap.config_revision,
            bootstrap.intent_revision,
            process_id,
        )?;
        ensure!(
            usage_binding.is_physically_bound(&store)?,
            "Workload no longer has its original physical slot"
        );
        let mut response = if !is_usage && (drain.is_cancelled() || !current) {
            ParentResponse::Error {
                code: "unavailable".into(),
            }
        } else {
            match request {
                ChildRequest::TlsIdentity { known_revision } => {
                    drop(store);
                    match bootstrap
                        .config
                        .tls_certificate_id
                        .as_deref()
                        .ok_or_else(|| anyhow::anyhow!("No certificate assigned"))
                        .and_then(|id| crate::certificates::load_identity(&state_dir, id))
                    {
                        Ok(identity) => ParentResponse::TlsIdentity {
                            identity: (known_revision != Some(identity.revision))
                                .then_some(identity),
                        },
                        Err(_) => ParentResponse::Error {
                            code: "certificate_unavailable".into(),
                        },
                    }
                }
                #[cfg(feature = "runtime")]
                ChildRequest::OutageSeal { ref claim }
                | ChildRequest::OutageVerify { ref claim, .. } => {
                    use crate::online::outage::OutageAuthority;
                    drop(store);
                    if let Some(broker) = &broker {
                        let result = match &request {
                            ChildRequest::OutageSeal { .. } if !prepared => broker
                                .seal(claim)
                                .await
                                .map(|seal| ParentResponse::OutageSealed { seal }),
                            ChildRequest::OutageVerify { seal, .. } if !prepared => broker
                                .verify(claim, seal)
                                .await
                                .map(|()| ParentResponse::OutageAccepted),
                            _ => Err(AuthorizationError::InvalidRequest.into()),
                        };
                        result.unwrap_or_else(|error| ParentResponse::Error {
                            code: match crate::online::authorization_error(&error) {
                                AuthorizationError::Denied => "denied",
                                AuthorizationError::Unavailable => "unavailable",
                                _ => "invalid_response",
                            }
                            .into(),
                        })
                    } else {
                        ParentResponse::Error {
                            code: "denied".into(),
                        }
                    }
                }
                #[cfg(feature = "runtime")]
                ChildRequest::OutageDeny { binding } => {
                    use crate::online::outage::OutageAuthority;
                    drop(store);
                    if let Some(broker) = &broker {
                        broker.deny(&binding).await?;
                        ParentResponse::OutageAccepted
                    } else {
                        ParentResponse::Error {
                            code: "denied".into(),
                        }
                    }
                }
                ChildRequest::Usage {
                    snapshot,
                    finalized,
                } => {
                    let retry = last_usage.as_ref() == Some(&snapshot);
                    ensure!(
                        prepared && (retry || snapshot.validate_after(last_usage.as_ref())),
                        "Invalid workload usage sequence or counters"
                    );
                    ensure!(
                        retry
                            || finalized
                            || last_usage_at
                                .is_none_or(|at| at.elapsed() >= Duration::from_secs(1)),
                        "Workload usage exceeds its reporting rate"
                    );
                    drop(store);
                    let telemetry = crate::telemetry::TelemetryStore::open(&state_dir)?;
                    if !usage_registered {
                        telemetry.begin_usage(&usage_binding)?;
                        usage_guard.arm();
                        usage_registered = true;
                    }
                    telemetry.record_usage(&usage_binding, &snapshot, finalized)?;
                    let sequence = snapshot.sequence;
                    last_usage = Some(snapshot);
                    last_usage_at = Some(std::time::Instant::now());
                    ParentResponse::Usage { sequence }
                }
                ChildRequest::Ready => {
                    drop(store);
                    // Offline hosted-resource grants remain lazy: local service readiness
                    // must not depend on cloud availability or resource capacity.
                    if bootstrap.config.source == crate::config::ProjectSource::Online {
                        if let Some(broker) = &broker {
                            tokio::select! { _ = cancel.cancelled() => return Ok(()), result = broker.prepare() => result? }
                        }
                    }
                    let store = StateStore::open(&state_dir.join("management.sqlite"))?;
                    ensure!(
                        !prepared
                            && store.record_replica_prepared(
                                &bootstrap.config.id,
                                bootstrap.replica_slot,
                                bootstrap.config_revision,
                                bootstrap.intent_revision,
                                process_id
                            )?,
                        "Workload readiness is stale or duplicated"
                    );
                    store.connection.execute(
                        "DELETE FROM telemetry_records WHERE placement_id=?1 AND kind=?2",
                        rusqlite::params![
                            bootstrap.config.id,
                            format!("usage-{}", bootstrap.replica_slot)
                        ],
                    )?;
                    drop(store);
                    crate::telemetry::TelemetryStore::open(&state_dir)?
                        .begin_usage(&usage_binding)?;
                    usage_guard.arm();
                    usage_registered = true;
                    prepared = true;
                    ParentResponse::Ready
                }
                ChildRequest::Authorize {
                    ref method,
                    ref url,
                }
                | ChildRequest::AuthorizeProject {
                    ref method,
                    ref url,
                } => {
                    drop(store);
                    let audience = if matches!(&request, ChildRequest::AuthorizeProject { .. }) {
                        ResourceAudience::ProjectApi
                    } else {
                        ResourceAudience::HostedModels
                    };
                    if !prepared && audience == ResourceAudience::HostedModels {
                        ParentResponse::Error {
                            code: "denied".into(),
                        }
                    } else if let Some(broker) = &broker {
                        match tokio::select! { _ = cancel.cancelled() => return Ok(()), _ = drain.cancelled() => Err(AuthorizationError::Unavailable), result = broker.authorize(AuthorizationRequest {
                            audience,method,url,
                        }) => result }
                        {
                            Ok(authorization) => ParentResponse::Authorization {
                                authorization: authorization.authorization().into(),
                                dpop: authorization
                                    .dpop()
                                    .context("Broker omitted request proof")?
                                    .into(),
                                expires_at: authorization
                                    .expires_at()
                                    .duration_since(UNIX_EPOCH)?
                                    .as_secs(),
                            },
                            Err(error) => ParentResponse::Error {
                                code: match error {
                                    AuthorizationError::Denied => "denied",
                                    AuthorizationError::Expired => "expired",
                                    AuthorizationError::InvalidRequest => "invalid_request",
                                    AuthorizationError::InvalidResponse => "invalid_response",
                                    _ => "unavailable",
                                }
                                .into(),
                            },
                        }
                    } else {
                        ParentResponse::Error {
                            code: "denied".into(),
                        }
                    }
                }
            }
        };
        let current = StateStore::open(&state_dir.join("management.sqlite"))?;
        ensure!(
            usage_binding.is_physically_bound(&current)?,
            "Workload physical slot changed while processing broker request"
        );
        if !is_usage
            && (drain.is_cancelled()
                || !current.replica_is_current(
                    &bootstrap.config.id,
                    bootstrap.replica_slot,
                    bootstrap.config_revision,
                    bootstrap.intent_revision,
                    process_id,
                )?)
        {
            response = ParentResponse::Error {
                code: "unavailable".into(),
            };
        }
        tokio::select! { _ = cancel.cancelled() => return Ok(()), result = write_frame(&mut stream,&response) => result? }
    }
}

fn validate_inherited_socket(descriptor: i32) -> Result<()> {
    let mut kind: libc::c_int = 0;
    let mut length = std::mem::size_of_val(&kind) as libc::socklen_t;
    let mut address = std::mem::MaybeUninit::<libc::sockaddr_storage>::zeroed();
    let mut address_length = std::mem::size_of::<libc::sockaddr_storage>() as libc::socklen_t;
    // SAFETY: every pointer names a live buffer of the length supplied. These
    // calls only inspect the descriptor and do not take ownership of it.
    unsafe {
        ensure!(
            libc::fcntl(descriptor, libc::F_GETFD) >= 0,
            "Workload broker descriptor is missing"
        );
        ensure!(
            libc::getsockopt(
                descriptor,
                libc::SOL_SOCKET,
                libc::SO_TYPE,
                (&mut kind as *mut libc::c_int).cast(),
                &mut length
            ) == 0
                && kind == libc::SOCK_STREAM,
            "Workload broker descriptor is not a stream socket"
        );
        ensure!(
            libc::getsockname(descriptor, address.as_mut_ptr().cast(), &mut address_length) == 0
                && address.assume_init().ss_family as libc::c_int == libc::AF_UNIX,
            "Workload broker descriptor is not a Unix socket"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn tls_ipc_never_accepts_a_child_selected_certificate() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let root = crate::supervisor::prepare_state_dir(directory.path())?;
        let mut store = StateStore::open(&root.join("management.sqlite"))?;
        let config: PlacementConfig = serde_json::from_value(serde_json::json!({
            "id":"placement", "project_id":"project", "deployment_id":"deployment",
            "revision":"v1", "source":"offline", "project_path":root,
            "events":[{"event_id":"event","event_version":[1,0,0],"board_version":[1,0,0]}]
        }))?;
        store.upsert_placement(
            "placement",
            &serde_json::to_value(&config)?,
            crate::state::DesiredState::Running,
        )?;
        store.claim_replica("placement", 0, 1, 1)?;
        store.record_replica(
            "placement",
            0,
            1,
            1,
            crate::state::ObservedState::Starting,
            Some(42),
            None,
        )?;
        let identity = rcgen::generate_simple_self_signed(vec!["other.example.test".into()])?;
        let certificate_id = uuid::Uuid::new_v4().to_string();
        crate::certificates::put(
            &store,
            &root,
            &certificate_id,
            "Another service",
            0,
            &identity.cert.pem(),
            &identity.signing_key.serialize_pem(),
            crate::enrollment::unix_time()?,
        )?;
        let bootstrap = ChildBootstrap {
            config,
            data_root: None,
            replica_slot: 0,
            inherited_listener: false,
            config_revision: 1,
            intent_revision: 1,
            parent_pid: 1,
            api_base_url: None,
            workload_identity: None,
        };
        let (parent, mut child) = UnixStream::pair()?;
        let server = tokio::spawn(serve(
            parent,
            bootstrap,
            root,
            42,
            None,
            CancellationToken::new(),
        ));
        let _: ChildBootstrap = read_frame(&mut child).await?;
        write_frame(
            &mut child,
            &ChildRequest::TlsIdentity {
                known_revision: None,
            },
        )
        .await?;
        assert!(
            matches!(read_frame::<ParentResponse>(&mut child).await?, ParentResponse::Error { ref code } if code == "certificate_unavailable")
        );
        write_frame(
            &mut child,
            &serde_json::json!({
                "operation":"tls_identity", "known_revision":null, "certificate_id":certificate_id
            }),
        )
        .await?;
        assert!(
            tokio::time::timeout(
                Duration::from_secs(2),
                read_frame::<ParentResponse>(&mut child)
            )
            .await?
            .is_err()
        );
        assert!(server.await?.is_err());
        Ok(())
    }

    #[test]
    fn listener_validation_rejects_connected_and_unopened_tcp_sockets() -> Result<()> {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        validate_listener_socket(listener.as_raw_fd())?;
        let client = std::net::TcpStream::connect(listener.local_addr()?)?;
        let (accepted, _) = listener.accept()?;
        assert!(validate_listener_socket(client.as_raw_fd()).is_err());
        assert!(validate_listener_socket(accepted.as_raw_fd()).is_err());
        let raw = unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0) };
        ensure!(raw >= 0, "Create unconnected TCP socket");
        let unconnected = unsafe { OwnedFd::from_raw_fd(raw) };
        assert!(validate_listener_socket(unconnected.as_raw_fd()).is_err());
        assert!(validate_listener_socket(-1).is_err());
        Ok(())
    }

    #[tokio::test]
    async fn inherited_listener_child_fixture() -> Result<()> {
        let Ok(port) = std::env::var("FLOW_LIKE_TEST_LISTENER_PORT") else {
            return Ok(());
        };
        let hosting = crate::config::HostingConfig {
            host: std::net::Ipv4Addr::LOCALHOST.into(),
            port: port.parse()?,
            max_in_flight: 1,
            request_timeout_secs: 5,
            auth_secret: "test".into(),
        };
        let listener = inherited_listener(&hosting)?;
        let (mut stream, _) =
            tokio::time::timeout(Duration::from_secs(10), listener.accept()).await??;
        stream
            .write_all(std::process::id().to_string().as_bytes())
            .await?;
        Ok(())
    }
    #[tokio::test]
    async fn inherited_listener_accepts_on_two_independent_children() -> Result<()> {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?;
        let mut children = Vec::new();
        for _ in 0..2 {
            let mut command = tokio::process::Command::new(std::env::current_exe()?);
            command
                .args([
                    "--exact",
                    "ipc::tests::inherited_listener_child_fixture",
                    "--nocapture",
                ])
                .env("FLOW_LIKE_TEST_LISTENER_PORT", address.port().to_string())
                .kill_on_drop(true)
                .stdout(std::process::Stdio::inherit())
                .stderr(std::process::Stdio::inherit());
            let descriptor = attach_listener(&mut command, &listener)?;
            children.push(command.spawn()?);
            drop(descriptor);
        }
        let mut pids = std::collections::HashSet::new();
        for _ in 0..2 {
            let mut stream = tokio::net::TcpStream::connect(address).await?;
            let mut response = String::new();
            tokio::time::timeout(
                Duration::from_secs(10),
                stream.read_to_string(&mut response),
            )
            .await
            .context("Receive PID from inherited-listener child")??;
            pids.insert(response.parse::<u32>()?);
        }
        assert_eq!(pids.len(), 2);
        for mut child in children {
            assert!(
                tokio::time::timeout(Duration::from_secs(10), child.wait())
                    .await??
                    .success()
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn inherited_placement_lock_lives_until_the_child_exits() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let lock_path = directory.path().join("placement.lock");
        let lock = crate::supervisor::lock_file(&lock_path)?;
        let (mut channel, child_socket) = socket_pair()?;
        let mut command = tokio::process::Command::new("/bin/sh");
        command.args(["-c", "printf ready >&3; read message <&3"]);
        command.kill_on_drop(true);
        let inherited = attach_child_descriptors(&mut command, &child_socket, &lock)?;
        assert!(inherited.iter().all(|fd| fd.as_raw_fd() >= 5));
        let mut child = command.spawn()?;
        drop(inherited);
        drop(child_socket);
        drop(lock);
        let mut ready = [0; 5];
        tokio::time::timeout(Duration::from_secs(5), channel.read_exact(&mut ready)).await??;
        assert_eq!(&ready, b"ready");
        assert!(crate::supervisor::lock_file(&lock_path).is_err());
        channel.write_all(b"quit\n").await?;
        assert!(
            tokio::time::timeout(Duration::from_secs(5), child.wait())
                .await??
                .success()
        );
        // Parallel subprocess tests may briefly inherit the CLOEXEC source
        // between their fork and exec, even after this child has been reaped.
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if crate::supervisor::lock_file(&lock_path).is_ok() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await?;
        Ok(())
    }

    #[tokio::test]
    async fn readiness_and_authorization_follow_the_current_placement_intent() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let config: PlacementConfig = serde_json::from_value(serde_json::json!({"id":"placement",
            "project_id":"project","deployment_id":"deployment","revision":"one","source":"offline",
            "project_path":directory.path(),"events":[{"event_id":"daemon","event_version":[1,0,0],"board_version":[1,0,0]}]}))?;
        let mut store = StateStore::open(&directory.path().join("management.sqlite"))?;
        store.upsert_placement(
            "placement",
            &serde_json::to_value(&config)?,
            crate::state::DesiredState::Running,
        )?;
        store.claim_replica("placement", 0, 1, 1)?;
        store.record_replica(
            "placement",
            0,
            1,
            1,
            crate::state::ObservedState::Starting,
            Some(42),
            None,
        )?;
        let bootstrap = ChildBootstrap {
            config,
            data_root: None,
            replica_slot: 0,
            inherited_listener: false,
            config_revision: 1,
            intent_revision: 1,
            parent_pid: 1,
            api_base_url: None,
            workload_identity: None,
        };
        let (parent, child) = UnixStream::pair()?;
        let state_dir = directory.path().to_path_buf();
        let cancel = CancellationToken::new();
        let server = tokio::spawn(serve(
            parent,
            bootstrap,
            state_dir,
            42,
            None,
            cancel.clone(),
        ));
        let (_, child) = ChildBroker::connect(child).await?;
        child.ready().await?;
        assert_eq!(
            store.get_placement("placement")?.unwrap().observed_state,
            crate::state::ObservedState::Running
        );
        let usage = crate::usage::RuntimeUsageSnapshot {
            version: 1,
            sequence: 1,
            invocations_started: 2,
            invocations_succeeded: 1,
            in_flight: 1,
            runtime_messages: 7,
            ..Default::default()
        };
        child.report_usage(usage).await?;
        let telemetry = crate::telemetry::TelemetryStore::open(directory.path())?;
        let stored = telemetry.read(Some("placement"), "usage-0", 0, 1)?;
        assert_eq!(stored["records"][0]["data"]["process_id"], 42);
        assert_eq!(stored["records"][0]["data"]["project_id"], "project");
        assert_eq!(
            stored["records"][0]["data"]["counters"]["runtime_messages"],
            7
        );
        let ciphertext: Vec<u8> = store.connection.query_row(
            "SELECT ciphertext FROM telemetry_records WHERE kind='usage-0'",
            [],
            |row| row.get(0),
        )?;
        assert!(
            !ciphertext
                .windows(b"runtime_messages".len())
                .any(|bytes| bytes == b"runtime_messages")
        );
        let request = AuthorizationRequest {
            audience: ResourceAudience::HostedModels,
            method: "POST",
            url: "https://api.example/api/v1/instances/responses",
        };
        assert_eq!(
            child.authorize(request).await.unwrap_err(),
            AuthorizationError::Denied
        );
        store.set_desired_state("placement", crate::state::DesiredState::Stopped)?;
        let request = AuthorizationRequest {
            audience: ResourceAudience::HostedModels,
            method: "POST",
            url: "https://api.example/api/v1/instances/responses",
        };
        assert_eq!(
            child.authorize(request).await.unwrap_err(),
            AuthorizationError::Unavailable
        );
        assert!(
            !server.is_finished(),
            "Final usage can still use the original physical binding"
        );
        cancel.cancel();
        tokio::time::timeout(Duration::from_secs(2), server).await???;
        Ok(())
    }
    #[tokio::test]
    async fn offline_ready_with_optional_hosted_grant_never_connects_to_cloud() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let cloud = std::net::TcpListener::bind("127.0.0.1:0")?;
        cloud.set_nonblocking(true)?;
        let base = format!("http://{}/api/v1", cloud.local_addr()?);
        let config: PlacementConfig = serde_json::from_value(
            serde_json::json!({"id":"placement","project_id":"offline","deployment_id":"deployment","revision":"one","source":"offline","project_path":directory.path(),"events":[{"event_id":"daemon","event_version":[1,0,0],"board_version":[1,0,0]}],"resource_grant":{"grant_id":"grant","authz_version":1,"billing_grant_id":"billing","billing_authz_version":1}}),
        )?;
        let mut store = StateStore::open(&directory.path().join("management.sqlite"))?;
        store.upsert_placement(
            "placement",
            &serde_json::to_value(&config)?,
            crate::state::DesiredState::Running,
        )?;
        store.claim_replica("placement", 0, 1, 1)?;
        store.record_replica(
            "placement",
            0,
            1,
            1,
            crate::state::ObservedState::Starting,
            Some(42),
            None,
        )?;
        let device = Arc::new(crate::enrollment::DeviceSession::test_session(
            base.clone(),
            uuid::Uuid::new_v4().to_string(),
            flow_like_device_protocol::SigningKey::generate(),
        ));
        let broker = Arc::new(WorkloadBroker::new(
            device,
            config.clone(),
            directory.path().into(),
            1,
            1,
        )?);
        let bootstrap = ChildBootstrap {
            config,
            data_root: None,
            replica_slot: 0,
            inherited_listener: false,
            config_revision: 1,
            intent_revision: 1,
            parent_pid: 1,
            api_base_url: Some(base),
            workload_identity: Some(broker.identity()),
        };
        let (parent, child) = UnixStream::pair()?;
        let cancel = CancellationToken::new();
        let server = tokio::spawn(serve(
            parent,
            bootstrap,
            directory.path().into(),
            42,
            Some(broker),
            cancel.clone(),
        ));
        let (_, child) = ChildBroker::connect(child).await?;
        tokio::time::timeout(Duration::from_secs(2), child.ready()).await??;
        assert_eq!(store.get_placement("placement")?.unwrap().ready_replicas, 1);
        assert_eq!(
            cloud.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        cancel.cancel();
        server.await??;
        Ok(())
    }

    #[tokio::test]
    async fn replica_responses_use_slot_identity_and_fence_scale_down_during_cloud_wait()
    -> Result<()> {
        let directory = tempfile::tempdir()?;
        let cloud = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let base = format!("http://{}/api/v1", cloud.local_addr()?);
        let device_id = uuid::Uuid::new_v4().to_string();
        let requested = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let requested_handler = requested.clone();
        let release_handler = release.clone();
        let app = axum::Router::new().route(
            &format!("/api/v1/devices/{device_id}/instances"),
            axum::routing::post(move || {
                let requested = requested_handler.clone();
                let release = release_handler.clone();
                async move {
                    requested.notify_one();
                    release.notified().await;
                    axum::http::StatusCode::FORBIDDEN
                }
            }),
        );
        let cloud_cancel = CancellationToken::new();
        let shutdown = cloud_cancel.clone();
        let cloud_task = tokio::spawn(async move {
            axum::serve(cloud, app)
                .with_graceful_shutdown(shutdown.cancelled_owned())
                .await
        });
        let config: PlacementConfig = serde_json::from_value(serde_json::json!({
            "id":"placement","project_id":"offline","deployment_id":"deployment",
            "revision":"one","source":"offline","project_path":directory.path(),
            "events":[{"event_id":"http","event_version":[1,0,0],"board_version":[1,0,0]}],
            "max_replicas":2,
            "hosting":{"host":"127.0.0.1","port":8080,"max_in_flight":4,"request_timeout_secs":30,"auth_secret":"service"},
            "resource_grant":{"grant_id":"grant","authz_version":1,"billing_grant_id":"billing","billing_authz_version":1}
        }))?;
        let mut store = StateStore::open(&directory.path().join("management.sqlite"))?;
        store.upsert_placement(
            &config.id,
            &serde_json::to_value(&config)?,
            crate::state::DesiredState::Running,
        )?;
        store.set_replica_count(&config.id, 1, 2)?;
        let device = Arc::new(crate::enrollment::DeviceSession::test_session(
            base.clone(),
            device_id,
            flow_like_device_protocol::SigningKey::generate(),
        ));
        let cancel = CancellationToken::new();
        let mut channels = Vec::new();
        let mut servers = Vec::new();
        for slot in 0..2 {
            let pid = 42 + u32::from(slot);
            assert!(store.claim_replica(&config.id, slot, 1, 1)?);
            store.record_replica(
                &config.id,
                slot,
                1,
                1,
                crate::state::ObservedState::Starting,
                Some(pid),
                None,
            )?;
            let broker = Arc::new(WorkloadBroker::new_replica(
                device.clone(),
                config.clone(),
                directory.path().into(),
                1,
                1,
                slot,
            )?);
            let bootstrap = ChildBootstrap {
                config: config.clone(),
                data_root: None,
                replica_slot: slot,
                inherited_listener: false,
                config_revision: 1,
                intent_revision: 1,
                parent_pid: 1,
                api_base_url: Some(base.clone()),
                workload_identity: Some(broker.identity()),
            };
            let (parent, child) = UnixStream::pair()?;
            servers.push(tokio::spawn(serve(
                parent,
                bootstrap,
                directory.path().into(),
                pid,
                Some(broker),
                cancel.clone(),
            )));
            let (_, child) = ChildBroker::connect(child).await?;
            tokio::time::timeout(Duration::from_secs(2), child.ready()).await??;
            channels.push(child);
        }
        assert_eq!(store.get_placement(&config.id)?.unwrap().ready_replicas, 2);
        let second = channels[1].clone();
        let url = format!("{base}/instances/responses");
        let authorization = tokio::spawn(async move {
            second
                .authorize(AuthorizationRequest {
                    audience: ResourceAudience::HostedModels,
                    method: "POST",
                    url: &url,
                })
                .await
        });
        tokio::time::timeout(Duration::from_secs(5), requested.notified()).await?;
        store.set_replica_count(&config.id, 1, 1)?;
        release.notify_one();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(5), authorization)
                .await??
                .unwrap_err(),
            AuthorizationError::Unavailable,
        );
        let removed = servers.pop().context("Missing second replica task")?;
        assert!(
            !removed.is_finished(),
            "A fenced replica may still report final usage"
        );
        channels[0]
            .report_usage(crate::usage::RuntimeUsageSnapshot {
                version: 1,
                sequence: 1,
                ..Default::default()
            })
            .await?;
        assert_eq!(store.get_placement(&config.id)?.unwrap().ready_replicas, 1);
        cancel.cancel();
        removed.await??;
        servers
            .pop()
            .context("Missing first replica task")?
            .await??;
        cloud_cancel.cancel();
        cloud_task.await??;
        Ok(())
    }

    #[tokio::test]
    async fn graceful_drain_denies_resources_but_acknowledges_bound_final_usage() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let config: PlacementConfig = serde_json::from_value(
            serde_json::json!({"id":"placement","project_id":"project","deployment_id":"deployment","revision":"one","source":"offline","project_path":directory.path(),"hosting":{"host":"127.0.0.1","port":8080,"max_in_flight":4,"request_timeout_secs":30,"auth_secret":"service"},"events":[{"event_id":"http","event_version":[1,0,0],"board_version":[1,0,0]}]}),
        )?;
        let mut store = StateStore::open(&directory.path().join("management.sqlite"))?;
        store.upsert_placement(
            "placement",
            &serde_json::to_value(&config)?,
            crate::state::DesiredState::Running,
        )?;
        store.claim_replica("placement", 0, 1, 1)?;
        store.record_replica(
            "placement",
            0,
            1,
            1,
            crate::state::ObservedState::Starting,
            Some(42),
            None,
        )?;
        let bootstrap = ChildBootstrap {
            config,
            data_root: None,
            replica_slot: 0,
            inherited_listener: false,
            config_revision: 1,
            intent_revision: 1,
            parent_pid: 1,
            api_base_url: None,
            workload_identity: None,
        };
        let (parent, child) = UnixStream::pair()?;
        let cancel = CancellationToken::new();
        let drain = CancellationToken::new();
        let server = tokio::spawn(serve_with_drain(
            parent,
            bootstrap,
            directory.path().into(),
            42,
            None,
            cancel.clone(),
            drain.clone(),
        ));
        let (_, child) = ChildBroker::connect(child).await?;
        child.ready().await?;
        child
            .report_usage(crate::usage::RuntimeUsageSnapshot {
                version: 1,
                sequence: 1,
                invocations_started: 1,
                in_flight: 1,
                ..Default::default()
            })
            .await?;
        drain.cancel();
        store.set_desired_state("placement", crate::state::DesiredState::Stopped)?;
        store.record_replica(
            "placement",
            0,
            1,
            1,
            crate::state::ObservedState::Stopping,
            Some(42),
            None,
        )?;
        let response = child
            .request(&ChildRequest::Authorize {
                method: "POST".into(),
                url: "https://example.test/api/v1/instances/responses".into(),
            })
            .await?;
        assert!(matches!(&response,ParentResponse::Error{code} if code=="unavailable"));
        assert!(child.ready().await.is_err());
        let final_usage = crate::usage::RuntimeUsageSnapshot {
            version: 1,
            sequence: 2,
            invocations_started: 1,
            invocations_succeeded: 1,
            runtime_messages: 3,
            ..Default::default()
        };
        child.report_final_usage(final_usage.clone()).await?;
        child.report_final_usage(final_usage).await?;
        let telemetry = crate::telemetry::TelemetryStore::open(directory.path())?;
        let totals = telemetry.retained_usage(Some("placement"), None)?;
        assert_eq!(totals["counters"]["invocations_succeeded"], 1);
        assert_eq!(totals["coverage"]["finalized_runs"], 1);
        assert_eq!(totals["coverage"]["incomplete_runs"], 0);
        store.record_replica(
            "placement",
            0,
            1,
            1,
            crate::state::ObservedState::Starting,
            Some(43),
            None,
        )?;
        assert!(
            child
                .report_final_usage(crate::usage::RuntimeUsageSnapshot {
                    version: 1,
                    sequence: 3,
                    invocations_started: 2,
                    invocations_succeeded: 2,
                    ..Default::default()
                })
                .await
                .is_err()
        );
        assert!(server.await?.is_err());
        assert_eq!(
            telemetry.retained_usage(Some("placement"), None)?["counters"]["invocations_started"],
            1
        );
        cancel.cancel();
        Ok(())
    }

    #[test]
    fn missing_or_non_socket_descriptor_is_rejected_without_taking_ownership() {
        assert!(validate_inherited_socket(-1).is_err());
        let file = tempfile::tempfile().unwrap();
        assert!(validate_inherited_socket(file.as_raw_fd()).is_err());
        assert!(file.metadata().is_ok());
    }
    #[tokio::test]
    async fn oversized_frame_is_rejected_before_allocation() {
        let (mut reader, mut writer) = UnixStream::pair().unwrap();
        writer
            .write_u32((MAX_FRAME_BYTES + 1) as u32)
            .await
            .unwrap();
        assert!(read_frame::<ChildRequest>(&mut reader).await.is_err());
    }

    #[tokio::test]
    async fn ipc_authorization_does_not_accept_ready_as_a_credential() {
        let (client, mut server) = UnixStream::pair().unwrap();
        let client = ChildBroker {
            stream: Mutex::new(Some(client)),
            resource_base: Some("https://api.example/api/v1/instances".into()),
            identity: None,
        };
        let peer = tokio::spawn(async move {
            let _: ChildRequest = read_frame(&mut server).await.unwrap();
            write_frame(&mut server, &ParentResponse::Ready)
                .await
                .unwrap();
        });
        assert_eq!(
            client
                .authorize(AuthorizationRequest {
                    audience: ResourceAudience::HostedModels,
                    method: "POST",
                    url: "https://api.example/api/v1/instances/responses"
                })
                .await
                .unwrap_err(),
            AuthorizationError::Unavailable
        );
        peer.await.unwrap();
    }

    #[tokio::test]
    async fn ipc_transports_per_request_proofs_and_denials() {
        let (client, mut server) = UnixStream::pair().unwrap();
        let client = ChildBroker {
            stream: Mutex::new(Some(client)),
            resource_base: None,
            identity: None,
        };
        assert_eq!(
            client.attribution(),
            AuthorizationAttribution::InstanceGrant
        );
        let peer = tokio::spawn(async move {
            for i in 0..4 {
                let request: ChildRequest = read_frame(&mut server).await.unwrap();
                assert!(matches!(request, ChildRequest::Authorize { .. }));
                let response = if i >= 2 {
                    ParentResponse::Error {
                        code: if i == 2 { "denied" } else { "expired" }.into(),
                    }
                } else {
                    ParentResponse::Authorization {
                        authorization: format!("DPoP token-{i}"),
                        dpop: format!("proof-{i}"),
                        expires_at: (std::time::SystemTime::now() + Duration::from_secs(60))
                            .duration_since(UNIX_EPOCH)
                            .unwrap()
                            .as_secs(),
                    }
                };
                write_frame(&mut server, &response).await.unwrap();
            }
        });
        for i in 0..4 {
            let result = client
                .authorize(AuthorizationRequest {
                    audience: ResourceAudience::HostedModels,
                    method: "POST",
                    url: "https://api.example/api/v1/instances/responses",
                })
                .await;
            if i == 2 {
                assert_eq!(result.unwrap_err(), AuthorizationError::Denied);
            } else if i == 3 {
                assert_eq!(result.unwrap_err(), AuthorizationError::Expired);
            } else {
                assert_eq!(result.unwrap().dpop(), Some(format!("proof-{i}").as_str()));
            }
        }
        peer.await.unwrap();
    }
}
