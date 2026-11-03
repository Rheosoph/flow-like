use std::collections::HashSet;
use std::net::IpAddr;
use std::sync::Arc;

use flow_like::flow::execution::egress::is_blocked_ip;
use flow_like_wasm::IsolatedWasiCtxBuilder;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum GuestNetwork {
    #[default]
    Off,
    PublicInternet,
    Hosts(HashSet<IpAddr>),
}

impl GuestNetwork {
    pub fn permits(&self, ip: IpAddr) -> bool {
        match self {
            Self::Off => false,
            Self::PublicInternet => !is_blocked_ip(ip),
            Self::Hosts(allowed) => allowed.contains(&ip) && !is_blocked_ip(ip),
        }
    }

    pub fn is_enabled(&self) -> bool {
        !matches!(self, Self::Off)
    }

    pub(crate) fn apply(&self, builder: &mut IsolatedWasiCtxBuilder) {
        if !self.is_enabled() {
            return;
        }
        let policy = Arc::new(self.clone());
        builder
            .socket_addr_check(move |addr, _use| {
                let permitted = policy.permits(addr.ip());
                Box::pin(async move { permitted })
            })
            .allow_tcp(true)
            .allow_udp(true);
    }

    pub async fn resolve_hosts(hosts: &[String]) -> Self {
        let mut allowed = HashSet::new();
        for host in hosts {
            let name = host_without_port(host);
            if name.is_empty() {
                continue;
            }
            if let Ok(ip) = name.parse::<IpAddr>() {
                allowed.insert(ip);
                continue;
            }
            match tokio::net::lookup_host((name, 0)).await {
                Ok(addrs) => allowed.extend(addrs.map(|a| a.ip())),
                Err(e) => tracing::warn!("sandbox network: cannot resolve '{name}': {e}"),
            }
        }
        Self::Hosts(allowed)
    }
}

fn host_without_port(host: &str) -> &str {
    let host = host.trim();
    if host.starts_with('[') {
        return host
            .trim_start_matches('[')
            .split(']')
            .next()
            .unwrap_or_default();
    }
    match host.rsplit_once(':') {
        Some((name, port)) if !name.contains(':') && port.chars().all(|c| c.is_ascii_digit()) => {
            name
        }
        _ => host,
    }
}
