//! Address and host-name predicates shared by every outbound-network guard.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// True for addresses on the host / hypervisor plane: "this network", loopback,
/// link-local (cloud metadata and container credential endpoints), broadcast,
/// multicast, and the metadata and credential endpoints that clouds place in
/// shared or unique-local space. IPv4-mapped, IPv4-compatible and NAT64
/// (`64:ff9b::/96`) addresses are judged by the IPv4 address they embed.
pub fn is_host_plane_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_host_plane_ipv4(v4),
        IpAddr::V6(v6) => is_host_plane_ipv6(v6),
    }
}

fn is_host_plane_ipv4(ip: Ipv4Addr) -> bool {
    ip.octets()[0] == 0 // 0.0.0.0/8 — "this network", connects to loopback on Linux
        || ip.is_loopback()
        || ip.is_link_local() // 169.254.0.0/16: IMDS, ECS/EKS credential endpoints
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip == Ipv4Addr::new(168, 63, 129, 16) // Azure WireServer VIP
        || ip == Ipv4Addr::new(100, 100, 100, 200) // Alibaba Cloud ECS metadata
}

fn is_host_plane_ipv6(ip: Ipv6Addr) -> bool {
    if ip.is_loopback() || ip.is_unspecified() {
        return true;
    }
    if let Some(v4) = ip.to_ipv4() {
        return is_host_plane_ipv4(v4);
    }
    if let [0x0064, 0xff9b, 0, 0, 0, 0, high, low] = ip.segments() {
        return is_host_plane_ipv4(nat64_embedded_ipv4(high, low));
    }
    ip.is_multicast()
        || ip.is_unicast_link_local()
        || ip == Ipv6Addr::new(0xfd00, 0x0ec2, 0, 0, 0, 0, 0, 0x254) // AWS IMDS over IPv6
        || ip == Ipv6Addr::new(0xfd00, 0x0ec2, 0, 0, 0, 0, 0, 0x23) // EKS Pod Identity over IPv6
        || ip == Ipv6Addr::new(0xfd20, 0x00ce, 0, 0, 0, 0, 0, 0x254) // GCP metadata over IPv6
}

/// The IPv4 address that NAT64 (RFC 6052) carries in the last 32 bits.
fn nat64_embedded_ipv4(high: u16, low: u16) -> Ipv4Addr {
    Ipv4Addr::from((u32::from(high) << 16) | u32::from(low))
}

/// True for names that address the host / hypervisor plane by convention,
/// before any DNS lookup. Subdomains of `localhost` are left to the resolver:
/// deployments alias them to service containers (Docker Compose serves
/// signed storage URLs from `s3.localhost`), and one that answers with
/// loopback per RFC 6761 is refused on its address.
pub fn is_host_plane_name(host: &str) -> bool {
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
    )
}

/// Globally routable unicast: not the host plane, and not private, shared,
/// documentation, benchmarking or reserved space. NAT64 addresses
/// (`64:ff9b::/96`) are judged by the IPv4 address they embed.
pub fn is_public_ip(ip: IpAddr) -> bool {
    if is_host_plane_ip(ip) || ip.is_unspecified() {
        return false;
    }
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, c, _] = v4.octets();
            !(v4.is_private()
                || v4.is_documentation()
                || (a == 100 && (64..128).contains(&b))
                || (a == 192 && b == 0 && c == 0)
                || (a == 198 && (b == 18 || b == 19))
                || a >= 240)
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4() {
                return is_public_ip(IpAddr::V4(v4));
            }
            let segments = v6.segments();
            if let [0x0064, 0xff9b, .., high, low] = segments {
                return is_public_ip(IpAddr::V4(nat64_embedded_ipv4(high, low)));
            }
            !((segments[0] & 0xfe00) == 0xfc00
                || (segments[0] & 0xffc0) == 0xfec0
                || (segments[0] == 0x2001 && segments[1] == 0x0db8))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(raw: &str) -> IpAddr {
        raw.parse().unwrap()
    }

    #[test]
    fn host_plane_addresses() {
        for raw in [
            "169.254.169.254",
            "169.254.170.2",
            "169.254.170.23",
            "127.0.0.1",
            "127.1.2.3",
            "0.0.0.0",
            "0.1.2.3",
            "255.255.255.255",
            "224.0.0.1",
            "168.63.129.16",
            "100.100.100.200",
            "::1",
            "::",
            "::ffff:169.254.169.254",
            "::ffff:127.0.0.1",
            "::ffff:100.100.100.200",
            "64:ff9b::a9fe:a9fe",
            "64:ff9b::7f00:1",
            "fe80::1",
            "ff02::1",
            "fd00:ec2::254",
            "fd00:ec2::23",
            "fd20:ce::254",
        ] {
            assert!(is_host_plane_ip(ip(raw)), "{raw} is on the host plane");
        }
        for raw in [
            "10.0.0.5",
            "172.16.3.4",
            "192.168.1.1",
            "100.64.0.1",
            "8.8.8.8",
            "64:ff9b::808:808",
            "64:ff9b::a00:5",
            "2001:db8::1",
            "fd12::1",
            "fd00::1",
        ] {
            assert!(!is_host_plane_ip(ip(raw)), "{raw} is not on the host plane");
        }
    }

    #[test]
    fn host_plane_names() {
        for host in [
            "metadata.google.internal",
            "Metadata.Google.Internal.",
            "metadata",
            "metadata.goog",
            "instance-data",
            "localhost",
            "LocalHost.",
            "ip6-localhost",
            "host.docker.internal",
            "gateway.docker.internal",
        ] {
            assert!(is_host_plane_name(host), "{host} names the host plane");
        }
        for host in [
            "example.com",
            "api.internal.example.com",
            "s3.localhost",
            "localhost.example.com",
        ] {
            assert!(!is_host_plane_name(host), "{host} is left to the resolver");
        }
    }

    #[test]
    fn public_addresses() {
        for raw in [
            "10.0.0.1",
            "172.16.4.2",
            "192.168.1.1",
            "100.64.0.1",
            "127.0.0.1",
            "169.254.169.254",
            "0.0.0.0",
            "192.0.0.8",
            "192.0.2.10",
            "198.18.0.1",
            "198.51.100.7",
            "203.0.113.5",
            "224.0.0.1",
            "240.0.0.1",
            "255.255.255.255",
            "::",
            "::1",
            "fc00::1",
            "fd00::1",
            "fec0::1",
            "fe80::1",
            "ff02::1",
            "::ffff:10.0.0.1",
            "::ffff:127.0.0.1",
            "64:ff9b::a00:1",
            "64:ff9b::a9fe:a9fe",
            "2001:db8::1",
        ] {
            assert!(!is_public_ip(ip(raw)), "{raw} is not public");
        }
        for raw in [
            "93.184.216.34",
            "1.1.1.1",
            "8.8.8.8",
            "2606:4700:4700::1111",
            "::ffff:1.1.1.1",
            "64:ff9b::101:101",
        ] {
            assert!(is_public_ip(ip(raw)), "{raw} is public");
        }
    }
}
