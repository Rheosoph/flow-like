//! Outbound network policy for flows running in the shared server executor.
//!
//! A flow that can open arbitrary connections from the executor process can
//! reach the host / hypervisor plane: cloud metadata services (`169.254.169.254`,
//! `metadata.google.internal`), container credential endpoints
//! (`169.254.170.2`, `169.254.170.23`), the Lambda runtime API on loopback,
//! and the Azure WireServer VIP. Every one of those hands out the executor's
//! own identity or the next tenant's job. Server-side, such destinations are
//! refused at three points: the request URL (catches IP literals and known
//! names), DNS resolution (catches names that resolve there, at connect time,
//! so rebinding does not help), and every redirect hop.
//!
//! RFC 1918 / ULA ranges are deliberately *not* blocked here — private space is
//! a deployment's own network and is governed at the VPC / NetworkPolicy layer.
//! Locally (desktop, `FLOW_LIKE_EXECUTION_ENVIRONMENT=local`) nothing is
//! filtered: the host is the user's own machine.

use super::ExecutionEnvironment;
use flow_like_types::reqwest::{
    self, Url,
    dns::{Addrs, Name, Resolve, Resolving},
    redirect,
};
use flow_like_types::tokio::sync::oneshot;
use flow_like_types::{Result, anyhow, tokio};
use std::any::Any;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Duration;

const MAX_REDIRECTS: usize = 10;

/// Pool limits of server-side shared clients. One executor process runs every
/// tenant's jobs and each idle socket holds a file descriptor (Lambda allows
/// 1024). The pool is keyed by scheme + authority, so the per-host cap does not
/// bound a flow looping over many hosts; the idle timeout does. hyper reaps
/// once per timeout, so an idle connection closes 5-10 s after its last use.
const SERVER_POOL_MAX_IDLE_PER_HOST: usize = 4;
const SERVER_POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(5);

/// Clients handed out by [`GuardedHttpClient::shared`], one per Tokio runtime
/// and environment, and server-side one per owner.
///
/// Keyed by runtime because a pooled connection is driven by a task hyper
/// spawned on the runtime that opened it: reused from another runtime it
/// fails once that runtime is dropped ("dispatch task is gone") and stalls
/// while an idle current-thread runtime is not being polled. The desktop app
/// (Tauri's runtime), the executor / server binaries (`#[tokio::main]`) and
/// Lambda run every flow on one process-lifetime runtime, so in practice this
/// is one local client, or one per live owner server-side; tests and embedders
/// that build a runtime per job get a pool of their own, pruned once that
/// runtime shuts down.
static SHARED_CLIENTS: Mutex<Vec<SharedClient>> = Mutex::new(Vec::new());

#[cfg(test)]
static SHARED_BUILDS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

struct SharedClient {
    runtime: tokio::runtime::Id,
    environment: ExecutionEnvironment,
    /// Server-side only; the entry is pruned once the owner is dropped. The
    /// weak reference keeps the owner's allocation, so its address is not
    /// reused while the entry exists and `ptr_eq` identifies the owner.
    owner: Option<Weak<dyn Any + Send + Sync>>,
    client: reqwest::Client,
    /// Closed once the sentinel task spawned on `runtime` is dropped, i.e. when
    /// the runtime shuts down. Also guards against runtime ID reuse.
    runtime_alive: oneshot::Sender<()>,
}

/// True for addresses on the host / hypervisor plane.
pub fn is_blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_blocked_ipv4(v4),
        IpAddr::V6(v6) => is_blocked_ipv6(v6),
    }
}

fn is_blocked_ipv4(ip: Ipv4Addr) -> bool {
    ip.octets()[0] == 0 // 0.0.0.0/8 — "this network", connects to loopback on Linux
        || ip.is_loopback()
        || ip.is_link_local() // 169.254.0.0/16: IMDS, ECS/EKS credential endpoints
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip == Ipv4Addr::new(168, 63, 129, 16) // Azure WireServer VIP
}

fn is_blocked_ipv6(ip: Ipv6Addr) -> bool {
    if ip.is_loopback() || ip.is_unspecified() {
        return true;
    }
    if let Some(v4) = ip.to_ipv4() {
        return is_blocked_ipv4(v4);
    }
    ip.is_multicast()
        || ip.is_unicast_link_local()
        || ip == Ipv6Addr::new(0xfd00, 0x0ec2, 0, 0, 0, 0, 0, 0x254) // AWS IMDS over IPv6
}

/// True for names that address the host / hypervisor plane by convention,
/// before any DNS lookup.
pub fn is_blocked_host(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    matches!(
        host.as_str(),
        "localhost"
            | "ip6-localhost"
            | "ip6-loopback"
            | "metadata"
            | "metadata.goog"
            | "metadata.google.internal"
            | "instance-data"
            | "host.docker.internal"
            | "gateway.docker.internal"
    ) || host.ends_with(".localhost")
}

/// Refuses a URL whose host is on the host / hypervisor plane when running
/// server-side. Names are only checked against the known list here; the
/// resolver installed by [`client_builder`] checks what they resolve to.
pub fn ensure_url_allowed(environment: ExecutionEnvironment, url: &Url) -> Result<()> {
    if environment != ExecutionEnvironment::Server {
        return Ok(());
    }
    let blocked = match url.host() {
        Some(url::Host::Ipv4(ip)) => is_blocked_ipv4(ip),
        Some(url::Host::Ipv6(ip)) => is_blocked_ipv6(ip),
        Some(url::Host::Domain(host)) => is_blocked_host(host),
        None => false,
    };
    if blocked {
        return Err(anyhow!(
            "Outbound request to '{}' refused: the host is on the executor's own network plane \
             (metadata / credential endpoints, loopback, link-local), which flows may not reach \
             in server-side execution.",
            url.host_str().unwrap_or_default()
        ));
    }
    Ok(())
}

/// Resolves `host:port` for a raw socket connect and, server-side, refuses
/// the name or any address on the host plane. Callers connect to the
/// returned addresses rather than re-resolving.
pub async fn resolve_socket_addrs(
    environment: ExecutionEnvironment,
    host: &str,
    port: u16,
) -> Result<Vec<SocketAddr>> {
    let guarded = environment == ExecutionEnvironment::Server;
    if guarded && is_blocked_host(host) {
        return Err(anyhow!(
            "Connection to '{host}' refused: the host is on the executor's own network plane, \
             which flows may not reach in server-side execution."
        ));
    }
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host, port))
        .await
        .map_err(|e| anyhow!("Failed to resolve '{host}': {e}"))?
        .collect();
    if addrs.is_empty() {
        return Err(anyhow!("'{host}' did not resolve to any address"));
    }
    if guarded && let Some(blocked) = addrs.iter().find(|addr| is_blocked_ip(addr.ip())) {
        return Err(anyhow!(
            "Connection to '{host}' refused: it resolves to {}, which is on the executor's \
                 own network plane and may not be reached in server-side execution.",
            blocked.ip()
        ));
    }
    Ok(addrs)
}

/// DNS resolver that refuses names resolving to the host plane. Installed
/// into every server-side reqwest client so the check happens at connect
/// time, per connection — DNS rebinding after an initial check does not help.
struct GuardedResolver;

impl Resolve for GuardedResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move {
            let addrs = resolve_socket_addrs(ExecutionEnvironment::Server, &host, 0)
                .await
                .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;
            Ok(Box::new(addrs.into_iter()) as Addrs)
        })
    }
}

fn guarded_redirect_policy() -> redirect::Policy {
    redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= MAX_REDIRECTS {
            return attempt.error("too many redirects");
        }
        match ensure_url_allowed(ExecutionEnvironment::Server, attempt.url()) {
            Ok(()) => attempt.follow(),
            Err(e) => attempt.error(e.to_string()),
        }
    })
}

/// A `reqwest::ClientBuilder` with the server-side egress policy installed
/// (guarded resolver + redirect check) when `environment` is `Server`, and a
/// plain builder otherwise. Prefer [`GuardedHttpClient`], which also checks
/// the initial request URL — a resolver never sees IP-literal hosts.
pub fn client_builder(environment: ExecutionEnvironment) -> reqwest::ClientBuilder {
    let builder = reqwest::Client::builder();
    if environment != ExecutionEnvironment::Server {
        return builder;
    }
    builder
        .dns_resolver(Arc::new(GuardedResolver))
        .redirect(guarded_redirect_policy())
}

fn server_pool_limits(builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
    builder
        .pool_max_idle_per_host(SERVER_POOL_MAX_IDLE_PER_HOST)
        .pool_idle_timeout(SERVER_POOL_IDLE_TIMEOUT)
}

/// An HTTP client for flow-supplied URLs. Server-side it refuses host-plane
/// destinations on the request URL, at DNS resolution and on every redirect;
/// locally it is a plain client.
#[derive(Clone, Debug)]
pub struct GuardedHttpClient {
    client: reqwest::Client,
    environment: ExecutionEnvironment,
}

impl GuardedHttpClient {
    pub fn new(environment: ExecutionEnvironment) -> Result<Self> {
        Self::configured(environment, |builder| builder)
    }

    /// Like [`Self::new`], with extra builder configuration (timeouts, user
    /// agent, …) applied on top of the guarded builder.
    pub fn configured(
        environment: ExecutionEnvironment,
        configure: impl FnOnce(reqwest::ClientBuilder) -> reqwest::ClientBuilder,
    ) -> Result<Self> {
        let client = configure(client_builder(environment))
            .build()
            .map_err(|e| anyhow!("Failed to build HTTP client: {e}"))?;
        Ok(Self {
            client,
            environment,
        })
    }

    /// Like [`Self::new`], but a clone of a cached client (see
    /// `SHARED_CLIENTS`), so repeated calls share one connection pool instead
    /// of paying a TCP + TLS handshake per request. Egress checks are
    /// unchanged: [`Self::request`] still vets every URL and the server-side
    /// client carries the guarded resolver and redirect policy. Put per-call
    /// options such as timeouts on the `RequestBuilder`. Outside a runtime this
    /// is just [`Self::new`].
    ///
    /// Locally every caller on the runtime shares the pool: the machine has one
    /// user. Server-side, where one process runs every tenant's jobs, only
    /// callers passing the same `owner` (a run's own resources) share it, and
    /// the first call after `owner` is dropped releases it. Guests set
    /// arbitrary headers, so a kept-alive connection can carry
    /// connection-bound state (NTLM / Negotiate auth, a saturated HTTP/2
    /// stream budget) that must not reach another run or tenant. Its idle
    /// connections close after `SERVER_POOL_IDLE_TIMEOUT` either way.
    pub fn shared(
        environment: ExecutionEnvironment,
        owner: &Arc<impl Any + Send + Sync>,
    ) -> Result<Self> {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return Self::new(environment);
        };
        let id = runtime.id();
        let server = environment == ExecutionEnvironment::Server;
        let owner = server.then(|| Arc::downgrade(owner) as Weak<dyn Any + Send + Sync>);
        let mut clients = SHARED_CLIENTS
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        clients.retain(|entry| {
            !entry.runtime_alive.is_closed()
                && entry
                    .owner
                    .as_ref()
                    .is_none_or(|weak| weak.strong_count() > 0)
        });
        if let Some(entry) = clients.iter().find(|entry| {
            entry.runtime == id
                && entry.environment == environment
                && match (&entry.owner, &owner) {
                    (Some(entry_owner), Some(owner)) => entry_owner.ptr_eq(owner),
                    (None, None) => true,
                    _ => false,
                }
        }) {
            return Ok(Self {
                client: entry.client.clone(),
                environment,
            });
        }
        let client = if server {
            Self::configured(environment, server_pool_limits)
        } else {
            Self::new(environment)
        }?
        .client;
        #[cfg(test)]
        SHARED_BUILDS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let (runtime_alive, sentinel) = oneshot::channel::<()>();
        // Pending for the runtime's whole life; shutdown drops it with the
        // runtime's other tasks, which closes `runtime_alive`.
        runtime.spawn(async move {
            let _ = sentinel.await;
        });
        clients.push(SharedClient {
            runtime: id,
            environment,
            owner,
            client: client.clone(),
            runtime_alive,
        });
        Ok(Self {
            client,
            environment,
        })
    }

    pub fn environment(&self) -> ExecutionEnvironment {
        self.environment
    }

    /// Starts a request after checking the URL against the egress policy.
    pub fn request(&self, method: reqwest::Method, url: &str) -> Result<reqwest::RequestBuilder> {
        let url = Url::parse(url).map_err(|e| anyhow!("Invalid URL '{url}': {e}"))?;
        ensure_url_allowed(self.environment, &url)?;
        Ok(self.client.request(method, url))
    }

    pub fn get(&self, url: &str) -> Result<reqwest::RequestBuilder> {
        self.request(reqwest::Method::GET, url)
    }

    pub fn post(&self, url: &str) -> Result<reqwest::RequestBuilder> {
        self.request(reqwest::Method::POST, url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_and_loopback_addresses_are_blocked() {
        for ip in [
            "169.254.169.254",
            "169.254.170.2",
            "169.254.170.23",
            "127.0.0.1",
            "127.1.2.3",
            "0.0.0.0",
            "168.63.129.16",
            "::1",
            "::",
            "::ffff:169.254.169.254",
            "::ffff:127.0.0.1",
            "fe80::1",
            "fd00:ec2::254",
        ] {
            assert!(is_blocked_ip(ip.parse().unwrap()), "{ip} must be blocked");
        }
    }

    #[test]
    fn private_and_public_addresses_are_not_blocked() {
        for ip in [
            "10.0.0.5",
            "172.16.3.4",
            "192.168.1.1",
            "8.8.8.8",
            "2001:db8::1",
            "fd12::1",
        ] {
            assert!(
                !is_blocked_ip(ip.parse().unwrap()),
                "{ip} must not be blocked"
            );
        }
    }

    #[test]
    fn metadata_hostnames_are_blocked_by_name() {
        for host in [
            "metadata.google.internal",
            "Metadata.Google.Internal.",
            "metadata",
            "localhost",
            "api.localhost",
            "host.docker.internal",
        ] {
            assert!(is_blocked_host(host), "{host} must be blocked");
        }
        assert!(!is_blocked_host("example.com"));
        assert!(!is_blocked_host("api.internal.example.com"));
    }

    #[test]
    fn url_check_only_applies_server_side() {
        let url = Url::parse("http://169.254.169.254/latest/meta-data/").unwrap();
        assert!(ensure_url_allowed(ExecutionEnvironment::Server, &url).is_err());
        assert!(ensure_url_allowed(ExecutionEnvironment::Desktop, &url).is_ok());
        assert!(ensure_url_allowed(ExecutionEnvironment::Local, &url).is_ok());

        let decimal = Url::parse("http://2130706433/").unwrap();
        assert!(
            ensure_url_allowed(ExecutionEnvironment::Server, &decimal).is_err(),
            "decimal IPv4 literals normalise to loopback"
        );
        let public = Url::parse("https://example.com/").unwrap();
        assert!(ensure_url_allowed(ExecutionEnvironment::Server, &public).is_ok());
    }

    #[test]
    fn guarded_client_refuses_metadata_url_before_sending() {
        let client = GuardedHttpClient::new(ExecutionEnvironment::Server).unwrap();
        assert!(client.get("http://169.254.169.254/").is_err());
        assert!(client.get("http://metadata.google.internal/").is_err());
        assert!(client.get("https://example.com/").is_ok());

        let local = GuardedHttpClient::new(ExecutionEnvironment::Desktop).unwrap();
        assert!(local.get("http://127.0.0.1:11434/").is_ok());
    }

    // The only test calling `shared`, so the build counter is not raced.
    #[test]
    fn shared_client_is_built_once_per_runtime_environment_and_server_owner() {
        let runtime = || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
        };
        let builds = || SHARED_BUILDS.load(std::sync::atomic::Ordering::SeqCst);
        let before = builds();
        let (first_run, second_run) = (Arc::new(()), Arc::new(()));

        let first = runtime();
        let first_id = first.block_on(async {
            for _ in 0..3 {
                let server =
                    GuardedHttpClient::shared(ExecutionEnvironment::Server, &first_run).unwrap();
                assert_eq!(server.environment(), ExecutionEnvironment::Server);
                assert!(server.get("http://169.254.169.254/").is_err());
                assert!(server.get("https://example.com/").is_ok());
            }
            for run in [&first_run, &second_run] {
                let local = GuardedHttpClient::shared(ExecutionEnvironment::Desktop, run).unwrap();
                assert_eq!(local.environment(), ExecutionEnvironment::Desktop);
                assert!(local.get("http://127.0.0.1:11434/").is_ok());
            }
            tokio::runtime::Handle::current().id()
        });
        assert_eq!(builds() - before, 2, "one build per environment");

        let server_pools = || {
            SHARED_CLIENTS
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .iter()
                .filter(|entry| entry.environment == ExecutionEnvironment::Server)
                .count()
        };
        let second_run_request = || {
            first.block_on(async {
                GuardedHttpClient::shared(ExecutionEnvironment::Server, &second_run).unwrap();
            })
        };
        second_run_request();
        assert_eq!(builds() - before, 3, "runs never share a server pool");
        assert_eq!(server_pools(), 2);
        drop(first_run);
        second_run_request();
        assert_eq!(builds() - before, 3);
        assert_eq!(server_pools(), 1, "a dropped owner's pool is released");

        drop(first);
        runtime().block_on(async {
            GuardedHttpClient::shared(ExecutionEnvironment::Server, &second_run).unwrap();
        });
        assert_eq!(builds() - before, 4, "a new runtime never reuses a pool");
        let clients = SHARED_CLIENTS
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        assert!(
            clients.iter().all(|entry| entry.runtime != first_id),
            "entries of a dropped runtime are pruned"
        );
    }

    #[test]
    fn server_pool_limits_close_surplus_and_idle_connections() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        const CONNECTIONS: usize = SERVER_POOL_MAX_IDLE_PER_HOST + 2;

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
                .await
                .unwrap();
            let url = format!("http://{}/", listener.local_addr().unwrap());
            let (closed_tx, mut closed) = tokio::sync::mpsc::unbounded_channel();
            // Answers once every request holds its own connection, then
            // reports each connection the client closes.
            let barrier = Arc::new(tokio::sync::Barrier::new(CONNECTIONS));
            tokio::spawn(async move {
                loop {
                    let (mut socket, _) = listener.accept().await.unwrap();
                    let (barrier, closed_tx) = (barrier.clone(), closed_tx.clone());
                    tokio::spawn(async move {
                        let (mut buf, mut request) = ([0; 1024], Vec::new());
                        while !request.ends_with(b"\r\n\r\n") {
                            let read = socket.read(&mut buf).await.unwrap();
                            assert!(read > 0, "request cut short");
                            request.extend_from_slice(&buf[..read]);
                        }
                        barrier.wait().await;
                        socket
                            .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n")
                            .await
                            .unwrap();
                        while socket.read(&mut buf).await.is_ok_and(|read| read > 0) {}
                        let _ = closed_tx.send(());
                    });
                }
            });

            // Desktop only because the server resolver refuses loopback.
            let client =
                server_pool_limits(client_builder(ExecutionEnvironment::Desktop).no_proxy())
                    .build()
                    .unwrap();
            let requests: Vec<_> = (0..CONNECTIONS)
                .map(|_| tokio::spawn(client.get(&url).send()))
                .collect();
            tokio::time::timeout(Duration::from_secs(10), async {
                for request in requests {
                    let response = request.await.unwrap().unwrap();
                    assert!(response.status().is_success());
                    response.bytes().await.unwrap();
                }
            })
            .await
            .expect("every request is answered");

            for _ in SERVER_POOL_MAX_IDLE_PER_HOST..CONNECTIONS {
                tokio::time::timeout(Duration::from_secs(2), closed.recv())
                    .await
                    .expect("connections beyond the per-host cap close on return");
            }
            assert!(
                tokio::time::timeout(Duration::from_millis(500), closed.recv())
                    .await
                    .is_err(),
                "connections within the cap stay pooled"
            );
            let idle_limit = 2 * SERVER_POOL_IDLE_TIMEOUT + Duration::from_secs(2);
            for _ in 0..SERVER_POOL_MAX_IDLE_PER_HOST {
                tokio::time::timeout(idle_limit, closed.recv())
                    .await
                    .expect("idle connections close after the idle timeout, not reqwest's 90 s");
            }
            drop(client);
        });
    }
}
