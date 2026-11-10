//! Where a connector may reach. A server runs the owner's MCP servers and nothing on its own
//! network: public hosts over https only. A desktop reaches the LAN and localhost too, but never
//! a metadata or link-local address.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use flow_like_types::egress::is_public_ip;
use flow_like_types::reqwest::Url;
use flow_like_types::tokio::net::lookup_host;

use crate::env::Environment;

const METADATA_HOSTS: [&str; 4] = [
    "metadata",
    "metadata.goog",
    "metadata.google.internal",
    "instance-data",
];
const LOOPBACK_HOSTS: [&str; 3] = ["localhost", "ip6-localhost", "ip6-loopback"];
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

fn is_metadata_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_link_local()
                || v4 == Ipv4Addr::new(168, 63, 129, 16)
                || v4 == Ipv4Addr::new(100, 100, 100, 200)
        }
        IpAddr::V6(v6) => {
            v6.to_ipv4_mapped()
                .is_some_and(|v4| is_metadata_ip(IpAddr::V4(v4)))
                || v6.is_unicast_link_local()
                || v6 == Ipv6Addr::new(0xfd00, 0x0ec2, 0, 0, 0, 0, 0, 0x254)
                || v6 == Ipv6Addr::new(0xfd00, 0x0ec2, 0, 0, 0, 0, 0, 0x23)
                || v6 == Ipv6Addr::new(0xfd20, 0x00ce, 0, 0, 0, 0, 0, 0x254)
        }
    }
}

fn is_metadata_host(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    METADATA_HOSTS.contains(&host.as_str())
}

fn is_loopback_host(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    LOOPBACK_HOSTS.contains(&host.as_str())
}

/// What a desktop makes of a target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Locality {
    /// An IP literal, `localhost`, or a name that resolves only to public addresses.
    Direct,
    /// A DNS name that resolves to a private or loopback address.
    PrivateName,
}

/// The desktop decision over a host and the addresses it resolves to: metadata and link-local
/// targets are always refused, a DNS name that resolves privately is flagged.
pub fn local_locality(host: &str, resolved: &[IpAddr]) -> Result<Locality, String> {
    let host = host.trim_matches(['[', ']']);
    let refused = || format!("'{host}' is a metadata or link-local address and is never reachable");
    if is_metadata_host(host) {
        return Err(refused());
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        return if is_metadata_ip(ip) {
            Err(refused())
        } else {
            Ok(Locality::Direct)
        };
    }
    if resolved.iter().any(|ip| is_metadata_ip(*ip)) {
        return Err(format!(
            "'{host}' resolves to a metadata or link-local address and is never reachable"
        ));
    }
    if is_loopback_host(host) {
        return Ok(Locality::Direct);
    }
    if resolved.iter().any(|ip| !is_public_ip(*ip)) {
        return Ok(Locality::PrivateName);
    }
    Ok(Locality::Direct)
}

/// A URL that passed the egress check, with the addresses it passed on. A server connects to
/// these addresses and no others, so a name that resolves elsewhere a moment later (DNS
/// rebinding) is not followed. `addrs` is empty where nothing is pinned (a desktop, an IP literal).
#[derive(Debug, Clone)]
pub struct Reachable {
    pub url: Url,
    pub host: String,
    pub addrs: Vec<SocketAddr>,
    pub locality: Locality,
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
        locality: Locality::Direct,
    };
    if environment == Environment::Local {
        let host = url
            .host_str()
            .unwrap_or_default()
            .trim_matches(['[', ']'])
            .to_string();
        if host.parse::<IpAddr>().is_ok() || is_loopback_host(&host) || is_metadata_host(&host) {
            local_locality(&host, &[])?;
            return Ok(plain(url));
        }
        let port = url.port_or_known_default().unwrap_or(443);
        let addresses = lookup(host.clone(), port)
            .await
            .map_err(|error| format!("Cannot resolve '{host}': {error}"))?;
        if addresses.is_empty() {
            return Err(format!(
                "Cannot resolve '{host}': the name has no addresses"
            ));
        }
        let ips: Vec<IpAddr> = addresses.iter().map(SocketAddr::ip).collect();
        let locality = local_locality(&host, &ips)?;
        return Ok(Reachable {
            url,
            host,
            addrs: addresses,
            locality,
        });
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
        locality: Locality::Direct,
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
    async fn a_desktop_refuses_metadata_and_link_local_targets() {
        for raw in [
            "http://169.254.169.254/latest/meta-data",
            "http://[fe80::1]/",
            "http://[fd00:ec2::254]/",
            "http://metadata.google.internal/computeMetadata/v1/",
            "http://169.254.1.1:8080/",
        ] {
            assert!(
                ensure_reachable(Environment::Local, raw).await.is_err(),
                "{raw}"
            );
        }
        let result = resolve_with(
            Environment::Local,
            "https://rebind.example.com/",
            false,
            answer(&["169.254.169.254"]),
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn a_desktop_keeps_localhost_and_ip_literals() {
        for raw in [
            "http://localhost:8080/x",
            "http://127.0.0.1/",
            "http://192.168.1.4/",
        ] {
            let reachable = ensure_reachable(Environment::Local, raw).await;
            assert!(reachable.is_ok(), "{raw}");
        }
    }

    #[tokio::test]
    async fn a_desktop_pins_a_name_and_flags_a_private_one() {
        let public = resolve_with(
            Environment::Local,
            "https://api.example.com/",
            false,
            answer(&["93.184.216.34"]),
        )
        .await
        .unwrap();
        assert_eq!(public.locality, Locality::Direct);
        assert_eq!(public.addrs.len(), 1);
        let private = resolve_with(
            Environment::Local,
            "https://nas.example.com/",
            false,
            answer(&["192.168.1.20"]),
        )
        .await
        .unwrap();
        assert_eq!(private.locality, Locality::PrivateName);
        assert_eq!(private.addrs[0].ip().to_string(), "192.168.1.20");
        let unpinned = resolve_with(
            Environment::Local,
            "https://gone.example.com/",
            false,
            answer(&[]),
        )
        .await;
        assert!(unpinned.is_err());
    }

    #[test]
    fn the_local_decision_is_a_function_of_host_and_addresses() {
        let ip = |text: &str| -> IpAddr { text.parse().unwrap() };
        assert_eq!(
            local_locality("localhost", &[ip("127.0.0.1")]),
            Ok(Locality::Direct)
        );
        assert_eq!(
            local_locality("nas.example.com", &[ip("192.168.1.20")]),
            Ok(Locality::PrivateName)
        );
        assert_eq!(
            local_locality("nas.example.com", &[ip("127.0.0.1")]),
            Ok(Locality::PrivateName)
        );
        assert_eq!(
            local_locality("api.example.com", &[ip("93.184.216.34")]),
            Ok(Locality::Direct)
        );
        assert!(local_locality("evil.example.com", &[ip("169.254.169.254")]).is_err());
        assert!(local_locality("169.254.169.254", &[]).is_err());
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
