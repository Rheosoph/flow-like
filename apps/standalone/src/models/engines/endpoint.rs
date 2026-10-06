#[cfg(target_os = "linux")]
use super::EngineLaunch;
#[cfg(any(target_os = "linux", not(unix)))]
use anyhow::ensure;
use anyhow::{Context, Result};
use std::path::PathBuf;

pub(super) const SOCKET_ENV: &str = "FLOW_LIKE_ENGINE_SOCKET";

/// The socket has its own private writable mount; model and runtime mounts remain read-only.
pub(super) struct EngineSocket {
    dir: PathBuf,
}

impl EngineSocket {
    #[cfg(all(test, unix, feature = "runtime"))]
    pub(super) fn new() -> Result<Self> {
        Self::in_directory("/tmp")
    }

    #[cfg(target_os = "linux")]
    pub(super) fn new_sandboxed() -> Result<Self> {
        Self::in_directory("/dev/shm")
    }

    #[cfg(any(target_os = "linux", all(test, unix, feature = "runtime")))]
    fn in_directory(parent: &str) -> Result<Self> {
        use std::os::unix::fs::DirBuilderExt;
        // A fixed short prefix avoids the platform's UNIX socket path-length limit.
        let dir = PathBuf::from(parent).join(format!("flow-engine-{}", uuid::Uuid::new_v4()));
        std::fs::DirBuilder::new().mode(0o700).create(&dir)?;
        Ok(Self { dir })
    }

    pub(super) fn path(&self) -> PathBuf {
        self.dir.join("engine.sock")
    }

    #[cfg(target_os = "linux")]
    pub(super) fn launch(&self, original: &EngineLaunch) -> Result<EngineLaunch> {
        let mut launch = original.clone();
        if let Some(index) = launch.args.iter().position(|arg| arg == "--host") {
            let host = launch
                .args
                .get_mut(index + 1)
                .context("Missing engine --host value")?;
            *host = self.path().into_os_string();
        } else {
            ensure!(
                launch.args.iter().any(|arg| arg == "model-worker"),
                "The engine does not support a private socket"
            );
            launch
                .env
                .push((SOCKET_ENV.into(), self.path().into_os_string()));
        }
        Ok(launch)
    }
}

impl Drop for EngineSocket {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

pub(super) fn engine_client(socket: Option<PathBuf>) -> Result<reqwest::Client> {
    let builder = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none());
    #[cfg(unix)]
    let builder = if let Some(path) = socket {
        builder.unix_socket(path)
    } else {
        builder
    };
    #[cfg(not(unix))]
    ensure!(
        socket.is_none(),
        "UNIX engine sockets are unavailable on this platform"
    );
    builder.build().context("Build the engine HTTP client")
}

#[cfg(all(test, unix, feature = "runtime"))]
mod tests {
    use super::*;
    use axum::{Router, routing::get};
    use std::os::unix::fs::PermissionsExt;

    #[tokio::test]
    async fn engine_socket_is_private_and_http_uses_it_without_a_tcp_listener() -> Result<()> {
        let socket = EngineSocket::new()?;
        assert_eq!(
            std::fs::metadata(&socket.dir)?.permissions().mode() & 0o777,
            0o700
        );
        let path = socket.path();
        let listener = tokio::net::UnixListener::bind(&path)?;
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route("/health", get(|| async { "ready" })),
            )
            .await
        });
        let client = engine_client(Some(path.clone()))?;
        assert_eq!(
            client
                .get("http://localhost/health")
                .send()
                .await?
                .text()
                .await?,
            "ready"
        );
        server.abort();
        let _ = server.await;
        drop(socket);
        assert!(!path.exists());
        Ok(())
    }
}
