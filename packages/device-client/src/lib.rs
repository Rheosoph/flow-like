//! Controller-side transport to Flow-Like devices, the native counterpart of the browser
//! tunnel in `packages/ui/lib/device-management/tunnel*.ts`.
//!
//! A [`DeviceClient`] asks the hub for a rendezvous admission, prefers a WebRTC data
//! channel to the device and falls back to the hub's WebSocket relay. Inside either
//! transport a Noise XX session, certified by the [`ControllerKeys`], carries up to 16
//! multiplexed [`TunnelStream`]s and renews its certificate in band. The hub only relays
//! ciphertext; trust comes from the device identity pinned in [`DeviceTarget`].

mod client;
mod endpoint;
mod error;
mod forward;
mod http;
mod hub;
mod keys;
mod pipe;
mod proxy;
mod relay;
mod rtc;
mod stream;
mod tls;
mod tunnel;

#[cfg(test)]
mod tests;

pub use client::{DeviceClient, DeviceSession, DeviceTarget, TransportKind};
pub use error::{Error, Result};
pub use flow_like_device_protocol::{TunnelDataOpen, TunnelMode, TunnelOpen, TunnelTarget};
pub use forward::LoopbackForward;
pub use http::{TunnelConnector, TunnelIo};
pub use hub::{AccessToken, HttpHubClient, HubClient};
pub use keys::ControllerKeys;
pub use proxy::{LoopbackPort, LoopbackProxy, ProxyActivity};
pub use stream::TunnelStream;

pub(crate) fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64)
}

pub(crate) fn random_bytes<const N: usize>() -> [u8; N] {
    use rand_core::RngCore;
    let mut bytes = [0; N];
    rand_core::OsRng.fill_bytes(&mut bytes);
    bytes
}
