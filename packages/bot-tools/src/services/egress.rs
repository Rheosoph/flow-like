//! Where a connector may reach. A server runs the owner's MCP servers and nothing on its own
//! network: public hosts over https only. A desktop reaches anything, localhost included.

use std::net::{IpAddr, SocketAddr};

use flow_like_types::egress::is_public_ip;
use flow_like_types::reqwest::Url;
use flow_like_types::tokio::net::lookup_host;

use crate::env::Environment;

const PRIVATE_SUFFIXES: [&str; 5] = [".local", ".internal", ".localdomain", ".lan", ".home.arpa"];

fn parse(raw: &str) -> Result<Url, String> {
    let url = Url::parse(raw.trim()).map_err(|error| {
        let shown = raw.split(['?', '#']).next().unwrap_or_default();
        format!("The URL '{shown}' is invalid: {error}")
    })?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("Only http and https URLs are supported".to_string());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("The URL must not carry a user name or password".to_string());
    }
    if url.host_str().is_none() {
        return Err("The URL has no host".to_string());
    }
    Ok(url)
}

fn domain_is_private(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    !host.contains('.')
        || flow_like_types::egress::is_host_plane_name(&host)
        || PRIVATE_SUFFIXES.iter().any(|suffix| host.ends_with(suffix))
}

/// A URL that passed the egress check, with the addresses it passed on. A server connects to
/// these addresses and no others, so a name that resolves elsewhere a moment later (DNS
/// rebinding) is not followed. `addrs` is empty where nothing is pinned (a desktop, an IP literal).
#[derive(Debug, Clone)]
pub struct Reachable {
    pub url: Url,
    pub host: String,
    pub addrs: Vec<SocketAddr>,
}

/// The URL if `environment` may connect to it.
pub async fn ensure_reachable(environment: Environment, raw: &str) -> Result<Url, String> {
    resolve_reachable(environment, raw).await.map(|r| r.url)
}

/// [`ensure_reachable`], keeping the addresses the check resolved.
pub async fn resolve_reachable(environment: Environment, raw: &str) -> Result<Reachable, String> {
    resolve(environment, raw, true).await
}

/// [`resolve_reachable`] that lets a server reach public hosts over plain http too.
pub async fn resolve_public(environment: Environment, raw: &str) -> Result<Reachable, String> {
    resolve(environment, raw, false).await
}

async fn resolve(
    environment: Environment,
    raw: &str,
    https_only: bool,
) -> Result<Reachable, String> {
    resolve_with(environment, raw, https_only, |host, port| async move {
        Ok(lookup_host((host.as_str(), port)).await?.collect())
    })
    .await
}

async fn resolve_with<L, F>(
    environment: Environment,
    raw: &str,
    https_only: bool,
    lookup: L,
) -> Result<Reachable, String>
where
    L: FnOnce(String, u16) -> F,
    F: std::future::Future<Output = std::io::Result<Vec<SocketAddr>>>,
{
    let url = parse(raw)?;
    let plain = |url: Url| Reachable {
        host: url.host_str().unwrap_or_default().to_string(),
        url,
        addrs: Vec::new(),
    };
    if environment == Environment::Local {
        return Ok(plain(url));
    }
    if https_only && url.scheme() != "https" {
        return Err("Servers reach MCP servers over https only".to_string());
    }
    let host = url
        .host_str()
        .unwrap_or_default()
        .trim_matches(['[', ']'])
        .to_string();
    let refused = || format!("'{host}' is not a public host; a server only reaches public hosts");
    if let Ok(ip) = host.parse::<IpAddr>() {
        return if is_public_ip(ip) {
            Ok(plain(url))
        } else {
            Err(refused())
        };
    }
    if domain_is_private(&host) {
        return Err(refused());
    }
    let port = url.port_or_known_default().unwrap_or(443);
    let addresses = lookup(host.clone(), port)
        .await
        .map_err(|error| format!("Cannot resolve '{host}': {error}"))?;
    if addresses.is_empty() || addresses.iter().any(|address| !is_public_ip(address.ip())) {
        return Err(refused());
    }
    Ok(Reachable {
        url,
        host,
        addrs: addresses,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_server_refuses_private_and_loopback_hosts() {
        for raw in [
            "https://127.0.0.1/mcp",
            "https://10.0.0.5/mcp",
            "https://192.168.1.4/mcp",
            "https://169.254.169.254/latest",
            "https://[::1]/mcp",
            "https://localhost/mcp",
            "https://db.internal/mcp",
            "https://intranet/mcp",
            "http://93.184.216.34/mcp",
        ] {
            assert!(
                ensure_reachable(Environment::Server, raw).await.is_err(),
                "{raw}"
            );
        }
        assert!(
            ensure_reachable(Environment::Server, "https://93.184.216.34/mcp")
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn a_desktop_reaches_localhost_but_not_other_schemes_or_credentials() {
        assert!(
            ensure_reachable(Environment::Local, "http://127.0.0.1:8931/mcp")
                .await
                .is_ok()
        );
        assert!(
            ensure_reachable(Environment::Local, "file:///etc/passwd")
                .await
                .is_err()
        );
        assert!(
            ensure_reachable(Environment::Local, "https://user:pw@example.com/")
                .await
                .is_err()
        );
    }

    fn answer(
        ips: &[&str],
    ) -> impl FnOnce(String, u16) -> std::future::Ready<std::io::Result<Vec<SocketAddr>>> {
        let addrs: Vec<SocketAddr> = ips
            .iter()
            .map(|ip| SocketAddr::new(ip.parse().unwrap(), 443))
            .collect();
        move |_, _| std::future::ready(Ok(addrs))
    }

    #[tokio::test]
    async fn a_server_connects_only_to_the_addresses_that_passed_the_check() {
        let reachable = resolve_with(
            Environment::Server,
            "https://mcp.example.com/mcp",
            true,
            answer(&["93.184.216.34", "93.184.216.35"]),
        )
        .await
        .unwrap();
        assert_eq!(reachable.host, "mcp.example.com");
        let pinned: Vec<String> = reachable.addrs.iter().map(|a| a.ip().to_string()).collect();
        assert_eq!(pinned, ["93.184.216.34", "93.184.216.35"]);
    }

    #[tokio::test]
    async fn a_name_that_resolves_anywhere_private_is_refused_not_pinned() {
        for ips in [
            vec!["127.0.0.1"],
            vec!["10.0.0.5"],
            vec!["169.254.169.254"],
            vec!["93.184.216.34", "192.168.1.4"],
            vec![],
        ] {
            let result = resolve_with(
                Environment::Server,
                "https://rebind.example.com/mcp",
                true,
                answer(&ips),
            )
            .await;
            assert!(result.is_err(), "{ips:?}");
        }
    }
}
