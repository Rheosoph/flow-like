//! One execution's egress authority, enforced outside its untrusted sandbox.

use std::{
    io::Write,
    os::unix::{fs::PermissionsExt, net::UnixListener as StdUnixListener},
    sync::Arc,
};

use tokio::{
    io::{AsyncBufReadExt, BufReader},
    net::{TcpListener, UnixListener},
    sync::Semaphore,
};

pub mod policy;
mod proxy;

pub type BoxError = Box<dyn std::error::Error + Send + Sync>;
const MAX_BODY: u64 = 128 * 1024 * 1024;
const MAX_POLICY: usize = 65536;
// The manager permits a 24-hour run plus startup and terminal work. This
// defensive ceiling bounds policy input without truncating that full budget.
const MAX_CAPABILITY_SECONDS: u64 = 172800;

/// Privileges and environment are settled before starting Tokio worker threads.
pub fn main() -> Result<(), BoxError> {
    enum Mode {
        Unix(StdUnixListener),
        Tcp {
            token: Arc<String>,
            bind: &'static str,
            max_duration: u64,
        },
    }
    let mode = match std::env::args().skip(1).collect::<Vec<_>>().as_slice() {
        [mode] if mode == "--unix-warm" => Mode::Unix(prepare_unix_socket()?),
        [mode] if mode == "--tcp" => {
            let token = std::env::var("GATEWAY_TOKEN")?;
            if token.len() < 32 || token.len() > 4096 || token.bytes().any(|b| b <= 32 || b == 127)
            {
                return Err("Invalid gateway control token".into());
            }
            // No other thread exists yet, so removing this bootstrap credential
            // cannot race an environment read by a worker or child process.
            unsafe {
                std::env::remove_var("GATEWAY_TOKEN");
            }
            let max_duration = std::env::var("EXECUTION_TIMEOUT_SECONDS")
                .unwrap_or_else(|_| "3600".into())
                .parse()?;
            if !(1..=MAX_CAPABILITY_SECONDS).contains(&max_duration) {
                return Err("Invalid execution duration".into());
            }
            let bind = if std::env::var("POD_IP").is_ok_and(|value| value.contains(':')) {
                "[::]"
            } else {
                "0.0.0.0"
            };
            Mode::Tcp {
                token: Arc::new(token),
                bind,
                max_duration,
            }
        }
        _ => return Err("Usage: execution-gateway --unix-warm | --tcp".into()),
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(16)
        .enable_all()
        .build()?;
    let result = runtime.block_on(async move {
        let gateway = proxy::Gateway::new()?;
        match mode {
            Mode::Unix(listener) => run_unix(gateway, UnixListener::from_std(listener)?).await,
            Mode::Tcp {
                token,
                bind,
                max_duration,
            } => run_tcp(gateway, token, bind, max_duration).await,
        }
    });
    // Tokio stdin uses a blocking reader. An unused warm slot can be terminated
    // without waiting for that read to receive tenant input.
    runtime.shutdown_background();
    result
}

fn prepare_unix_socket() -> Result<StdUnixListener, BoxError> {
    if unsafe { libc::geteuid() } != 0 {
        return Err("Unix gateway bootstrap must start as root".into());
    }
    let path = std::ffi::CString::new("/gateway")?;
    std::fs::set_permissions("/gateway", std::fs::Permissions::from_mode(0o755))?;
    // The only privileged work is preparing this slot's private named volume.
    // The process drops supplementary groups and both IDs before listening.
    unsafe {
        if libc::chown(path.as_ptr(), 65532, 65532) != 0
            || libc::setgroups(0, std::ptr::null()) != 0
            || libc::setgid(65532) != 0
            || libc::setuid(65532) != 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        libc::umask(0o077);
    }
    let listener = StdUnixListener::bind("/gateway/proxy.sock")?;
    listener.set_nonblocking(true)?;
    // The runner has read-only access to this one volume and needs to connect
    // under its own UID. The socket grants no authority before assignment.
    std::fs::set_permissions(
        "/gateway/proxy.sock",
        std::fs::Permissions::from_mode(0o666),
    )?;
    Ok(listener)
}

async fn run_unix(gateway: Arc<proxy::Gateway>, listener: UnixListener) -> Result<(), BoxError> {
    println!("ready");
    std::io::stdout().flush()?;
    let assignment = gateway.clone();
    tokio::spawn(async move {
        let result = async {
            let mut stdin = BufReader::new(tokio::io::stdin());
            let mut raw = Vec::new();
            // fill_buf avoids allocating an unbounded line before checking it.
            loop {
                let available = stdin.fill_buf().await?;
                if available.is_empty() {
                    return Err("Incomplete gateway policy".into());
                }
                let count = available
                    .iter()
                    .position(|b| *b == b'\n')
                    .map_or(available.len(), |i| i + 1);
                if raw.len() + count > MAX_POLICY {
                    return Err("Gateway policy exceeds limit".into());
                }
                let complete = available[count - 1] == b'\n';
                raw.extend_from_slice(&available[..count]);
                stdin.consume(count);
                if complete {
                    break;
                }
            }
            let policy = policy::Policy::new(serde_json::from_slice(&raw)?)?;
            assignment.assign(policy)?;
            println!("assigned");
            std::io::stdout().flush()?;
            Ok::<_, BoxError>(())
        }
        .await;
        if result.is_err() {
            assignment.revoke();
        }
    });
    tokio::select! {
        result = async {
            loop {
                let (stream, _) = listener.accept().await?;
                if let Ok(permit) = gateway.connections.clone().try_acquire_owned() {
                    tokio::spawn(gateway.clone().serve_proxy(stream, permit));
                }
            }
            #[allow(unreachable_code)] Ok::<(), BoxError>(())
        } => result,
        _ = gateway.revoked.cancelled() => Ok(()),
        _ = terminate() => { gateway.revoke(); Ok(()) },
    }
}

async fn run_tcp(
    gateway: Arc<proxy::Gateway>,
    token: Arc<String>,
    bind: &str,
    max_duration: u64,
) -> Result<(), BoxError> {
    let proxy_listener = TcpListener::bind(format!("{bind}:3128")).await?;
    let control_listener = TcpListener::bind(format!("{bind}:9001")).await?;
    let controls = Arc::new(Semaphore::new(16));
    tokio::select! {
        result = async {
            loop {
                tokio::select! {
                    connection = proxy_listener.accept() => {
                        let (stream, _) = connection?; stream.set_nodelay(true)?;
                        if !gateway.revoked.is_cancelled() && let Ok(permit) = gateway.connections.clone().try_acquire_owned() {
                            tokio::spawn(gateway.clone().serve_proxy(stream, permit));
                        }
                    },
                    connection = control_listener.accept() => {
                        let (stream, _) = connection?; stream.set_nodelay(true)?;
                        if let Ok(permit) = controls.clone().try_acquire_owned() {
                            let gateway = gateway.clone(); let token = token.clone();
                            tokio::spawn(async move { gateway.serve_control(stream, token, max_duration).await; drop(permit); });
                        }
                    },
                }
            }
            #[allow(unreachable_code)] Ok::<(), BoxError>(())
        } => result,
        _ = terminate() => { gateway.revoke(); Ok(()) },
    }
}

async fn terminate() {
    #[cfg(unix)]
    {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            tokio::select! { _ = signal.recv() => {}, _ = tokio::signal::ctrl_c() => {} }
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}
