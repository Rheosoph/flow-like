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
use flow_like_types::{Result, anyhow, tokio};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;

const MAX_REDIRECTS: usize = 10;

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
}
