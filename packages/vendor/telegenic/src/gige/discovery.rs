//! Network-level device discovery and IP forcing.
//!
//! A side channel that runs without a [`GigECamera`](crate::gige::GigECamera)
//! connection: discovery broadcasts a GVCP DISCOVERY_CMD on every Up IPv4
//! adapter and collects the device description blocks that come back; force
//! IP broadcasts a FORCEIP_CMD to repoint a device whose address no longer
//! matches the local subnet.
//!
//! Each adapter gets its own socket and the
//! beacon goes to both the subnet-directed broadcast and the limited
//! broadcast `255.255.255.255` — the latter is what a device with a
//! mis-configured IP (e.g. stuck on link-local while the host is not) will
//! actually accept, since a subnet-directed destination foreign to the
//! device's own subnet is dropped by its IP stack. The ALLOW_BROADCAST_ACK
//! flag is set for the same reason: it permits such a device to answer to
//! the broadcast address instead of routing a unicast reply.

use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

use snare::net::UdpSocket;

use crate::error::{CameraError, Result};
use crate::gige::proto::bootstrap::DeviceInfo;
use crate::gige::proto::gvcp::{self, Ack};

/// One local network adapter to probe.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NetworkAdapter {
    /// OS-level interface name (e.g. `eth0`, `en0`).
    pub name: String,
    /// IPv4 address bound on the adapter; the discovery socket's source.
    pub ip: Ipv4Addr,
    /// The adapter's subnet mask.
    pub netmask: Ipv4Addr,
    /// Subnet-directed broadcast address (e.g. `10.0.0.255`), computed from
    /// `ip | !mask` when the OS doesn't supply it.
    pub broadcast: Ipv4Addr,
}

/// Enumerate Up IPv4 adapters, skipping loopback and unspecified addresses.
pub fn enumerate_adapters() -> io::Result<Vec<NetworkAdapter>> {
    let mut out = Vec::new();
    for iface in if_addrs::get_if_addrs()? {
        if iface.is_loopback() {
            continue;
        }
        let if_addrs::IfAddr::V4(v4) = iface.addr.clone() else {
            continue;
        };
        if v4.ip.is_unspecified() {
            continue;
        }
        let broadcast = v4
            .broadcast
            .unwrap_or_else(|| Ipv4Addr::from(u32::from(v4.ip) | !u32::from(v4.netmask)));
        out.push(NetworkAdapter {
            name: iface.name,
            ip: v4.ip,
            netmask: v4.netmask,
            broadcast,
        });
    }
    Ok(out)
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DiscoveryConfig {
    /// Adapters to probe; `None` enumerates every Up IPv4 adapter.
    pub adapters: Option<Vec<NetworkAdapter>>,
    /// How long to collect replies after the beacons go out. Devices may
    /// legally delay their discovery ack, so keep this at one second or more.
    pub recv_window: Duration,
    /// Send to the limited broadcast `255.255.255.255` in addition to each
    /// adapter's subnet broadcast. Finds devices with mis-configured IPs.
    pub limited_broadcast: bool,
    /// Source port for the discovery sockets; 0 lets the OS pick.
    pub source_port: u16,
    /// Destination port, the GVCP port on real devices.
    pub device_port: u16,
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self {
            adapters: None,
            recv_window: Duration::from_secs(1),
            limited_broadcast: true,
            source_port: 0,
            device_port: gvcp::GVCP_PORT,
        }
    }
}

/// A device that answered discovery.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DiscoveredDevice {
    pub info: DeviceInfo,
    /// Address the ack came from. Usually `info.ip:3956`, but a device
    /// answering via broadcast may use something else; prefer `info.ip`.
    pub from: SocketAddr,
    /// The local adapter that heard the reply.
    pub adapter: NetworkAdapter,
}

/// Broadcast a discovery beacon on every adapter and collect the devices
/// that answer within the window. Duplicates (same MAC heard on several
/// adapters or via both broadcasts) are merged.
pub fn discover(cfg: &DiscoveryConfig) -> Result<Vec<DiscoveredDevice>> {
    let adapters = match &cfg.adapters {
        Some(a) => a.clone(),
        None => enumerate_adapters()?,
    };
    let beacon = gvcp::encode_discovery(true);

    let mut sockets = Vec::new();
    for adapter in adapters {
        let socket = match open_adapter_socket(&adapter, cfg.source_port) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("discovery: skipping {} ({}): {e}", adapter.name, adapter.ip);
                continue;
            }
        };
        let mut sent = false;
        for dest in beacon_destinations(&adapter, cfg.limited_broadcast, cfg.device_port) {
            match socket.send_to(&beacon, dest) {
                Ok(_) => sent = true,
                Err(e) => tracing::trace!("discovery: send to {dest} failed: {e}"),
            }
        }
        if sent {
            sockets.push((adapter, socket));
        }
    }
    if sockets.is_empty() {
        return Err(CameraError::Io(io::Error::new(
            io::ErrorKind::AddrNotAvailable,
            "no usable network adapter for discovery",
        )));
    }

    let mut found: Vec<DiscoveredDevice> = Vec::new();
    let deadline = Instant::now() + cfg.recv_window;
    let mut buf = [0u8; 0xffff];
    loop {
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        for (adapter, socket) in &sockets {
            socket
                .set_read_timeout(Some(Duration::from_millis(20)))
                .ok();
            let (n, from) = match socket.recv_from(&mut buf) {
                Ok(v) => v,
                Err(ref e)
                    if e.kind() == io::ErrorKind::WouldBlock
                        || e.kind() == io::ErrorKind::TimedOut =>
                {
                    continue;
                }
                Err(e) => {
                    tracing::trace!("discovery: recv on {} failed: {e}", adapter.name);
                    continue;
                }
            };
            let Some(device) = parse_discovery_ack(&buf[..n], from, adapter) else {
                continue;
            };
            if !found.iter().any(|d| d.info.mac == device.info.mac) {
                found.push(device);
            }
        }
    }
    tracing::debug!(devices = found.len(), "discovery finished");
    Ok(found)
}

fn beacon_destinations(adapter: &NetworkAdapter, limited: bool, port: u16) -> Vec<SocketAddr> {
    let mut dests = vec![SocketAddr::new(adapter.broadcast.into(), port)];
    if limited && adapter.broadcast != Ipv4Addr::BROADCAST {
        dests.push(SocketAddr::new(Ipv4Addr::BROADCAST.into(), port));
    }
    dests
}

fn open_adapter_socket(adapter: &NetworkAdapter, source_port: u16) -> io::Result<UdpSocket> {
    let socket = UdpSocket::bind(SocketAddr::new(adapter.ip.into(), source_port))?;
    socket.set_broadcast(true)?;
    Ok(socket)
}

fn parse_discovery_ack(
    datagram: &[u8],
    from: SocketAddr,
    adapter: &NetworkAdapter,
) -> Option<DiscoveredDevice> {
    let ack = Ack::parse(datagram)?;
    if ack.answer != gvcp::DISCOVERY_ACK || ack.status.is_error() {
        return None;
    }
    let info = DeviceInfo::parse(ack.payload)?;
    Some(DiscoveredDevice {
        info,
        from,
        adapter: adapter.clone(),
    })
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ForceIpConfig {
    pub discovery: DiscoveryConfig,
    /// How long to wait for the (optional) FORCEIP_ACK.
    pub ack_window: Duration,
}

impl Default for ForceIpConfig {
    fn default() -> Self {
        Self {
            discovery: DiscoveryConfig::default(),
            ack_window: Duration::from_secs(1),
        }
    }
}

/// Broadcast a FORCEIP_CMD repointing the device with `mac` to a new static
/// network configuration. The device applies it immediately, without any
/// established control connection.
///
/// Returns `Ok(true)` if the device acknowledged, `Ok(false)` if the window
/// passed without an ack — the command may still have been applied (the ack
/// can be sent from the new IP and get filtered on its way back); re-discover
/// to confirm.
pub fn force_ip(
    mac: [u8; 6],
    ip: Ipv4Addr,
    mask: Ipv4Addr,
    gateway: Ipv4Addr,
    cfg: &ForceIpConfig,
) -> Result<bool> {
    let adapters = match &cfg.discovery.adapters {
        Some(a) => a.clone(),
        None => enumerate_adapters()?,
    };
    let packet = gvcp::encode_force_ip(mac, ip, mask, gateway, 1);

    let mut sockets = Vec::new();
    for adapter in adapters {
        let socket = match open_adapter_socket(&adapter, cfg.discovery.source_port) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("force-ip: skipping {} ({}): {e}", adapter.name, adapter.ip);
                continue;
            }
        };
        for dest in beacon_destinations(
            &adapter,
            cfg.discovery.limited_broadcast,
            cfg.discovery.device_port,
        ) {
            if let Err(e) = socket.send_to(&packet, dest) {
                tracing::trace!("force-ip: send to {dest} failed: {e}");
            }
        }
        sockets.push(socket);
    }

    let deadline = Instant::now() + cfg.ack_window;
    let mut buf = [0u8; 256];
    while Instant::now() < deadline {
        for socket in &sockets {
            socket
                .set_read_timeout(Some(Duration::from_millis(20)))
                .ok();
            let Ok((n, _)) = socket.recv_from(&mut buf) else {
                continue;
            };
            if let Some(ack) = Ack::parse(&buf[..n])
                && ack.answer == gvcp::FORCEIP_ACK
                && !ack.status.is_error()
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// `true` if `device` is reachable as-is: its IP is inside the subnet of the
/// adapter that heard it.
pub fn is_reachable(device: &DiscoveredDevice) -> bool {
    let mask = u32::from(device.adapter.netmask);
    (u32::from(device.info.ip) & mask) == (u32::from(device.adapter.ip) & mask)
}

/// Convenience for the common "find my camera" case: discover on the adapter
/// with this local IP only.
pub fn discover_on(interface_ip: Ipv4Addr, recv_window: Duration) -> Result<Vec<DiscoveredDevice>> {
    let adapter = enumerate_adapters()?
        .into_iter()
        .find(|a| a.ip == interface_ip)
        .ok_or_else(|| {
            CameraError::Io(io::Error::new(
                io::ErrorKind::AddrNotAvailable,
                format!("no Up IPv4 adapter with address {interface_ip}"),
            ))
        })?;
    let cfg = DiscoveryConfig {
        adapters: Some(vec![adapter]),
        recv_window,
        ..DiscoveryConfig::default()
    };
    discover(&cfg)
}

/// `IpAddr` helper for [`GigeConfig::new`](crate::gige::GigeConfig::new).
pub fn device_ip(device: &DiscoveredDevice) -> IpAddr {
    device.info.ip.into()
}
