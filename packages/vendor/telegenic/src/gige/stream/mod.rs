//! The GVSP stream receiver: configuration, the per-channel handle, and the
//! frame delivery channel.

pub(crate) mod frame;
pub(crate) mod runner;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;

use crate::gige::ControlPort;
use crate::gige::proto::bootstrap;
use crate::thread_util::{ThreadConfig, ThreadHandle};

pub use frame::{Frame, FrameStatus, PayloadKind};

#[cfg_attr(feature = "valuable", derive(valuable::Valuable))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum PacketSize {
    /// Negotiate the largest size the link carries (fire-test bisection).
    #[default]
    Auto,
    /// Write this SCPS packet size as-is (bytes on the wire, incl. IP+UDP).
    Fixed(u16),
}

#[cfg_attr(feature = "valuable", derive(valuable::Valuable))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum ResendPolicy {
    /// Request resends for missing packets (when the device supports it).
    #[default]
    Always,
    Never,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StreamConfig {
    /// Stream channel index; almost always 0.
    pub channel: u16,
    /// Frame buffer size in bytes — the device's `PayloadSize` feature.
    /// `None` lets the feature layer fill it in from the device;
    /// transport-level users must supply it themselves.
    pub payload_size: Option<usize>,
    /// Buffers in the pool. Frames arriving while every buffer is held
    /// (filling or undelivered) are counted as underruns and dropped.
    pub n_buffers: usize,
    pub packet_size: PacketSize,
    /// Inter-packet delay in timestamp ticks, written to SCPD.
    pub packet_delay: Option<u32>,
    pub resend: ResendPolicy,
    /// How long a hole may trail the newest packet before the first resend.
    pub initial_packet_timeout: Duration,
    /// Re-request period for a hole that stays open.
    pub packet_timeout: Duration,
    /// A frame with no packet for this long is closed as timed out.
    pub frame_retention: Duration,
    /// Cap on resend requests per frame, as a fraction of its packet count.
    pub packet_request_ratio: f64,
    /// SO_RCVBUF for the stream socket; 0 picks `max(256 KiB, 8 * packet
    /// size)`.
    pub socket_buffer: usize,
    /// Local address for the stream socket. The IP must be device-reachable;
    /// `None` auto-detects via a connected probe socket.
    pub local_addr: Option<SocketAddr>,
    pub thread_cfg: ThreadConfig,
}

impl Default for StreamConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamConfig {
    pub fn new() -> Self {
        Self {
            channel: 0,
            payload_size: None,
            n_buffers: 8,
            packet_size: PacketSize::Auto,
            packet_delay: None,
            resend: ResendPolicy::Always,
            initial_packet_timeout: Duration::from_millis(1),
            packet_timeout: Duration::from_millis(20),
            frame_retention: Duration::from_millis(100),
            packet_request_ratio: 0.25,
            socket_buffer: 0,
            local_addr: None,
            thread_cfg: ThreadConfig::default(),
        }
    }

    /// Register block base for this config's channel.
    pub(crate) fn channel_base(&self) -> u32 {
        u32::from(self.channel) * bootstrap::STREAM_CHANNEL_STRIDE
    }
}

/// Stream receiver counters, all monotonic since stream open.
#[cfg_attr(feature = "valuable", derive(valuable::Valuable))]
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "py", pyo3::pyclass(get_all, skip_from_py_object))]
pub struct StreamStats {
    pub packets: u64,
    pub bytes: u64,
    pub completed_frames: u64,
    pub failed_frames: u64,
    pub timed_out_frames: u64,
    pub aborted_frames: u64,
    pub missing_frames: u64,
    pub underruns: u64,
    pub missing_packets: u64,
    pub resend_requests: u64,
    pub resent_packets: u64,
    pub resend_ratio_reached: u64,
    pub resend_disabled: u64,
    pub duplicated_packets: u64,
    pub error_packets: u64,
    pub ignored_packets: u64,
    pub unsupported_frames: u64,
    pub size_mismatch_errors: u64,
    /// Completed frames a subscriber could not take (its channel was full).
    pub frames_dropped: u64,
}

/// A clone-able receiver for completed frames. Each subscription has its own
/// bounded buffer; when it is full new frames are dropped for that
/// subscriber and counted in [`StreamStats::frames_dropped`].
#[derive(Debug, Clone)]
#[cfg_attr(feature = "py", pyo3::pyclass(skip_from_py_object))]
pub struct FrameChannel {
    rx: flume::Receiver<Arc<Frame>>,
}

impl FrameChannel {
    pub(crate) fn new(rx: flume::Receiver<Arc<Frame>>) -> Self {
        Self { rx }
    }

    /// Block until a frame is buffered or `timeout` elapses.
    pub fn wait_for(&self, timeout: Duration) -> Option<Arc<Frame>> {
        self.rx.recv_timeout(timeout).ok()
    }

    pub fn try_recv(&self) -> Option<Arc<Frame>> {
        self.rx.try_recv().ok()
    }

    /// Drain and return every buffered frame.
    pub fn recv_all(&self) -> Vec<Arc<Frame>> {
        let mut out = Vec::new();
        while let Ok(f) = self.rx.try_recv() {
            out.push(f);
        }
        out
    }

    /// Discard buffered frames — pair with [`wait_for`](Self::wait_for) to
    /// grab a freshly acquired frame instead of a stale one.
    pub fn clear(&self) {
        while self.rx.try_recv().is_ok() {}
    }

    pub fn is_disconnected(&self) -> bool {
        self.rx.is_disconnected()
    }

    #[cfg(feature = "async")]
    pub async fn recv_async(&self) -> Option<Arc<Frame>> {
        self.rx.recv_async().await.ok()
    }
}

pub(crate) struct StreamShared {
    pub stats: Mutex<StreamStats>,
}

/// An open stream channel. Owns the receiver worker; dropping the handle
/// stops the worker and closes the channel on the device (SCP := 0).
pub struct StreamChannel {
    pub(crate) to_worker: flume::Sender<runner::ToStreamWorker>,
    pub(crate) thread: ThreadHandle,
    pub(crate) shared: Arc<StreamShared>,
    /// Submission path to the control worker, for closing the channel
    /// registers on drop without owning the camera.
    pub(crate) control: ControlPort,
    pub(crate) channel_base: u32,
    pub(crate) packet_size: u16,
    pub(crate) local_addr: SocketAddr,
}

impl std::fmt::Debug for StreamChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamChannel")
            .field("local_addr", &self.local_addr)
            .field("packet_size", &self.packet_size)
            .finish()
    }
}

impl StreamChannel {
    /// Subscribe to completed frames with a buffer of `capacity` frames.
    pub fn subscribe(&self, capacity: usize) -> FrameChannel {
        let (tx, rx) = flume::bounded(capacity);
        let _ = self.to_worker.send(runner::ToStreamWorker::Subscribe(tx));
        self.thread.wake().ok();
        FrameChannel::new(rx)
    }

    pub fn stats(&self) -> StreamStats {
        *self.shared.stats.lock()
    }

    /// The negotiated (or configured) SCPS packet size.
    pub fn packet_size(&self) -> u16 {
        self.packet_size
    }

    /// Where the device sends this stream (SCDA:SCP).
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub fn is_running(&self) -> bool {
        self.thread.is_alive()
    }
}

impl Drop for StreamChannel {
    fn drop(&mut self) {
        // Wait (bounded) for the SCP := 0 ack so the device has stopped
        // transmitting before the receiving socket closes — otherwise every
        // in-flight packet triggers an ICMP port-unreachable.
        let _ = self
            .control
            .write_register(bootstrap::STREAM_CHANNEL_PORT + self.channel_base, 0)
            .wait_timeout(self.control.budget());
        let _ = self.to_worker.send(runner::ToStreamWorker::Shutdown);
        self.thread.wake().ok();
        // ThreadHandle::drop joins the worker.
    }
}
