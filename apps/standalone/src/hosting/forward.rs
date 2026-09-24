use super::{BODY_LIMIT, HostState, ReplicaContext, actions, channels};
use crate::config::PlacementConfig;
use anyhow::{Context, Result, ensure};
use axum::{body::to_bytes, extract::Request, http::StatusCode};
use flow_like_types::channel::InProcessPushResult;
use serde::{Deserialize, Serialize};
use std::{
    os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{UnixListener, UnixStream},
    sync::Semaphore,
};

const TIMEOUT: Duration = Duration::from_secs(10);
const HEADER_LIMIT: usize = 4096;
const FORWARD_LIMIT: usize = 64;

pub(super) struct Route {
    directory: PathBuf,
    pub incarnation: String,
    outbound: Semaphore,
}

pub(super) struct Listener {
    socket: UnixListener,
    pub route: Arc<Route>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
enum Header {
    Channel {
        channel_id: String,
        token: String,
    },
    ResolvePageAction {
        scope: actions::Scope,
        action_id: String,
        capability: String,
        fingerprint: String,
    },
}
impl Drop for Header {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        match self {
            Self::Channel { token, .. } => token.zeroize(),
            Self::ResolvePageAction { capability, .. } => capability.zeroize(),
        }
    }
}

pub(super) fn valid_incarnation(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

impl Listener {
    pub fn bind(config: &PlacementConfig, replica: ReplicaContext) -> Result<Self> {
        ensure!(
            replica.supervisor_pid > 0
                && replica.config_revision > 0
                && replica.slot < config.max_replicas,
            "Invalid replica routing context"
        );
        let root = config.project_path.canonicalize()?;
        let mut digest = blake3::Hasher::new();
        digest.update(b"flow-like-replica-replies-v1\0");
        for part in [
            root.as_os_str().as_encoded_bytes(),
            config.id.as_bytes(),
            config.project_id.as_bytes(),
        ] {
            digest.update(&(part.len() as u64).to_be_bytes());
            digest.update(part);
        }
        digest.update(&replica.supervisor_pid.to_be_bytes());
        digest.update(&replica.config_revision.to_be_bytes());
        let uid = unsafe { libc::geteuid() };
        // Use a short system temporary path: project roots can exceed Unix socket
        // path limits. Ownership and mode are checked before any socket is used.
        let directory = Path::new("/tmp")
            .canonicalize()?
            .join(format!("flc-{uid}-{}", &digest.finalize().to_hex()[..24]));
        private_directory(&directory)?;
        let incarnation = uuid::Uuid::new_v4().simple().to_string();
        let route = Arc::new(Route {
            directory,
            incarnation,
            outbound: Semaphore::new(FORWARD_LIMIT),
        });
        let path = route.path(&route.incarnation);
        ensure!(
            path.as_os_str().as_encoded_bytes().len() < 104,
            "Replica reply socket path is too long"
        );
        let socket = UnixListener::bind(&path).context("Bind private replica reply socket")?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        Ok(Self { socket, route })
    }

    pub async fn serve(self, host: Arc<HostState>) {
        let capacity = Arc::new(Semaphore::new(FORWARD_LIMIT));
        let mut requests = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                _ = host.cancel.cancelled() => break,
                _ = requests.join_next(), if !requests.is_empty() => {},
                accepted = self.socket.accept() => {
                    let Ok((socket, _)) = accepted else {
                        host.cancel.cancel();
                        break;
                    };
                    let Ok(permit) = capacity.clone().try_acquire_owned() else { continue };
                    let host = host.clone();
                    requests.spawn(async move {
                        let _permit = permit;
                        let _ = tokio::time::timeout(TIMEOUT, receive(socket, &host)).await;
                    });
                }
            }
        }
        requests.abort_all();
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        // Each process incarnation creates a fresh name; it never unlinks a
        // sibling's live socket or reuses a stale process-ID path.
        let _ = std::fs::remove_file(self.route.path(&self.route.incarnation));
    }
}

impl Route {
    #[cfg(test)]
    pub(super) fn path_for_test(&self) -> PathBuf {
        self.path(&self.incarnation)
    }
    fn path(&self, incarnation: &str) -> PathBuf {
        self.directory.join(format!("{incarnation}.sock"))
    }

    pub async fn forward(
        &self,
        request: Request,
        owner: &str,
        id: &str,
        token: &str,
    ) -> StatusCode {
        if !valid_incarnation(owner) || !channels::valid_id(id) {
            return StatusCode::NOT_FOUND;
        }
        let Ok(_permit) = self.outbound.try_acquire() else {
            return StatusCode::TOO_MANY_REQUESTS;
        };
        match tokio::time::timeout(TIMEOUT, self.send(request, owner, id, token)).await {
            Ok(Ok(status)) => status,
            Ok(Err(_)) => StatusCode::GONE,
            Err(_) => StatusCode::REQUEST_TIMEOUT,
        }
    }

    async fn send(
        &self,
        request: Request,
        owner: &str,
        id: &str,
        token: &str,
    ) -> Result<StatusCode> {
        let mut socket = self.connect(owner).await?;
        send_header(
            &mut socket,
            &Header::Channel {
                channel_id: id.into(),
                token: token.into(),
            },
        )
        .await?;
        let status = StatusCode::from_u16(socket.read_u16().await?)?;
        if status != StatusCode::CONTINUE {
            return Ok(status);
        }
        let body = match to_bytes(request.into_body(), BODY_LIMIT).await {
            Ok(body) => body,
            Err(_) => return Ok(StatusCode::BAD_REQUEST),
        };
        socket.write_u32(body.len() as u32).await?;
        socket.write_all(&body).await?;
        Ok(StatusCode::from_u16(socket.read_u16().await?)?)
    }

    async fn connect(&self, owner: &str) -> Result<UnixStream> {
        ensure!(valid_incarnation(owner), "Invalid replica incarnation");
        check_directory(&self.directory)?;
        let path = self.path(owner);
        let metadata = std::fs::symlink_metadata(&path)?;
        ensure!(
            metadata.file_type().is_socket()
                && metadata.uid() == unsafe { libc::geteuid() }
                && metadata.mode() & 0o077 == 0,
            "Invalid replica reply socket"
        );
        let socket = UnixStream::connect(path).await?;
        verify_peer(&socket)?;
        Ok(socket)
    }

    pub async fn resolve_action(
        &self,
        owner: &str,
        scope: &actions::Scope,
        action_id: &str,
        capability: &str,
        fingerprint: &str,
    ) -> Result<String> {
        let _permit = self
            .outbound
            .try_acquire()
            .context("Replica forwarding capacity reached")?;
        tokio::time::timeout(TIMEOUT, async {
            let mut socket = self.connect(owner).await?;
            send_header(
                &mut socket,
                &Header::ResolvePageAction {
                    scope: scope.clone(),
                    action_id: action_id.into(),
                    capability: capability.into(),
                    fingerprint: fingerprint.into(),
                },
            )
            .await?;
            ensure!(
                socket.read_u16().await? == StatusCode::OK.as_u16(),
                "Page action capability was refused"
            );
            let length = socket.read_u16().await? as usize;
            ensure!(length > 0 && length <= 128, "Invalid Page action target");
            let mut bytes = vec![0; length];
            socket.read_exact(&mut bytes).await?;
            Ok::<_, anyhow::Error>(String::from_utf8(bytes)?)
        })
        .await
        .context("Page action authority timed out")?
    }
}

async fn send_header(socket: &mut UnixStream, header: &Header) -> Result<()> {
    let bytes = zeroize::Zeroizing::new(serde_json::to_vec(header)?);
    ensure!(
        bytes.len() <= HEADER_LIMIT,
        "Replica header exceeds the limit"
    );
    socket.write_u32(bytes.len() as u32).await?;
    socket.write_all(&bytes).await?;
    Ok(())
}

async fn receive(mut socket: UnixStream, host: &HostState) -> Result<()> {
    verify_peer(&socket)?;
    let length = socket.read_u32().await? as usize;
    ensure!(length <= HEADER_LIMIT, "Reply header exceeds the limit");
    let mut bytes = zeroize::Zeroizing::new(vec![0; length]);
    socket.read_exact(&mut bytes).await?;
    let header: Header = serde_json::from_slice(&bytes)?;
    if let Header::ResolvePageAction {
        scope,
        action_id,
        capability,
        fingerprint,
    } = &header
    {
        match host
            .actions
            .resolve_local(host, scope, action_id, capability, fingerprint)
        {
            Ok(node) if node.len() <= 128 => {
                socket.write_u16(StatusCode::OK.as_u16()).await?;
                socket.write_u16(node.len() as u16).await?;
                socket.write_all(node.as_bytes()).await?;
            }
            _ => socket.write_u16(StatusCode::UNAUTHORIZED.as_u16()).await?,
        }
        return Ok(());
    }
    let Header::Channel { channel_id, token } = &header else {
        unreachable!()
    };
    ensure!(
        channels::valid_id(channel_id) && token.len() == 43,
        "Invalid reply header"
    );
    // Local receipt only. This endpoint cannot select another socket or proxy a
    // URL, and the owning process verifies its original live run authority.
    let grant = match channels::authorize(host, channel_id, token) {
        Ok(grant) => grant,
        Err(status) => {
            socket.write_u16(status.as_u16()).await?;
            return Ok(());
        }
    };
    socket.write_u16(StatusCode::CONTINUE.as_u16()).await?;
    let length = socket.read_u32().await? as usize;
    ensure!(length <= BODY_LIMIT, "Reply body exceeds the limit");
    let mut body = vec![0; length];
    socket.read_exact(&mut body).await?;
    let status = match channels::deliver(host, &grant, token, &body).await {
        Ok(InProcessPushResult::Delivered | InProcessPushResult::Duplicate) => {
            StatusCode::NO_CONTENT
        }
        Ok(InProcessPushResult::Full) => StatusCode::TOO_MANY_REQUESTS,
        Ok(_) => StatusCode::GONE,
        Err(_) => StatusCode::BAD_REQUEST,
    };
    socket.write_u16(status.as_u16()).await?;
    Ok(())
}

fn verify_peer(socket: &UnixStream) -> Result<()> {
    ensure!(
        socket.peer_cred()?.uid() == unsafe { libc::geteuid() },
        "Replica reply peer has a different owner"
    );
    Ok(())
}

fn private_directory(path: &Path) -> Result<()> {
    match std::fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => (),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(error) => return Err(error.into()),
    }
    check_directory(path)
}

fn check_directory(path: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_dir()
            && !metadata.file_type().is_symlink()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0,
        "Replica reply directory must be private and owned by this user"
    );
    Ok(())
}
