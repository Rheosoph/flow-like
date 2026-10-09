//! The GigE Vision backend: the [`GigECamera`] transport handle and its
//! configuration, network [`discovery`], the GVCP/GVSP wire formats
//! ([`proto`]), and the GVSP [`stream`] receiver.
//!
//! `GigECamera` is a plain owned value whose connection is an `Option`:
//! construction is free and infallible, `connect()` is where I/O happens
//! (and doubles as the reconnect path), and `disconnect()` takes the
//! connection down by value, so a dead link is unreachable by construction.
//! All per-connection state — device identity, capabilities, the GenICam
//! XML and node graph — lives exactly as long as the connection.

pub mod discovery;
pub mod proto;
mod runner;
pub mod stream;

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use parking_lot::Mutex;

use crate::error::{CameraError, Result};
use crate::gige::proto::bootstrap;
use crate::gige::proto::gvcp;
use crate::gige::proto::gvsp;
use crate::gige::runner::ToWorker;
use crate::gige::stream::{StreamChannel, StreamConfig, StreamShared};
use crate::handle::{ResponseHandle, unwrap_arc};
use crate::thread_util::{ThreadConfig, ThreadHandle};

pub use proto::bootstrap::DeviceInfo;
pub use proto::gvcp::GvcpStatus;
pub use proto::gvsp::PixelFormat;

/// Best-effort SO_RCVBUF sizing for the stream socket; bursts of a full
/// frame must fit between two worker wakeups.
fn set_receive_buffer(socket: &snare::net::UdpSocket, cfg: &StreamConfig, payload_size: usize) {
    let request = if cfg.socket_buffer != 0 {
        cfg.socket_buffer
    } else {
        (payload_size + 1024).clamp(256 * 1024, 8 * 1024 * 1024)
    };
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        let value = request as libc::c_int;
        let rc = unsafe {
            libc::setsockopt(
                socket.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_RCVBUF,
                (&raw const value).cast(),
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            )
        };
        if rc != 0 {
            tracing::warn!(
                "SO_RCVBUF={request} not applied: {}",
                std::io::Error::last_os_error()
            );
        }
    }
    #[cfg(not(unix))]
    {
        tracing::debug!("SO_RCVBUF sizing not implemented on this platform ({request} requested)");
        let _ = socket;
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GigeConfig {
    /// Device GVCP endpoint, normally `<device ip>:3956`.
    pub addr: SocketAddr,
    /// Local address to bind the control socket on; `0.0.0.0:0` by default.
    /// Set the IP to pick an interface on a multi-homed host.
    pub local_addr: Option<SocketAddr>,
    /// Per-try acknowledge timeout.
    pub gvcp_timeout: Duration,
    /// Retries after the first try before a transaction fails.
    pub retries: u8,
    /// Written to the device's heartbeat timeout register at connect; the
    /// worker reads CCP at a third of this period to keep control alive.
    pub heartbeat_timeout_ms: u32,
    /// Take exclusive control (no other application may even read).
    pub exclusive: bool,
    /// Buffered events per subscribed [`EventChannel`].
    pub event_capacity: usize,
    pub thread_cfg: ThreadConfig,
}

impl GigeConfig {
    pub fn new(ip: impl Into<IpAddr>) -> Self {
        Self {
            addr: SocketAddr::new(ip.into(), gvcp::GVCP_PORT),
            local_addr: None,
            gvcp_timeout: Duration::from_millis(500),
            retries: 4,
            heartbeat_timeout_ms: 3000,
            exclusive: false,
            event_capacity: 64,
            thread_cfg: ThreadConfig::default(),
        }
    }

    /// Worst-case duration of one transaction, used as the blocking budget
    /// for the convenience waits in `connect`.
    pub(crate) fn transaction_budget(&self) -> Duration {
        self.gvcp_timeout * (u32::from(self.retries) + 1) + Duration::from_millis(500)
    }
}

/// Control-channel counters.
#[cfg_attr(feature = "valuable", derive(valuable::Valuable))]
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "py", pyo3::pyclass(get_all, skip_from_py_object))]
pub struct LinkStats {
    pub commands: u64,
    pub acks: u64,
    pub retries: u64,
    pub timeouts: u64,
    pub naks: u64,
    pub pending_acks: u64,
    pub heartbeats: u64,
    pub events: u64,
    pub unsolicited: u64,
}

/// A device-initiated message-channel event.
#[cfg_attr(feature = "valuable", derive(valuable::Valuable))]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GvcpEvent {
    /// `EVENT_CMD` or `EVENTDATA_CMD`.
    pub command: u16,
    pub event_id: u16,
    pub stream_channel: u16,
    pub block_id: u16,
    pub timestamp: u64,
    /// Trailing data of an EVENTDATA event; empty for plain events.
    pub data: Vec<u8>,
    /// The unparsed command payload, kept as an escape hatch.
    pub raw: Vec<u8>,
}

impl GvcpEvent {
    pub(crate) fn parse(command: u16, payload: &[u8]) -> Self {
        let u16_at = |i: usize| {
            payload
                .get(i..i + 2)
                .map_or(0, |b| u16::from_be_bytes([b[0], b[1]]))
        };
        let timestamp = payload.get(8..16).map_or(0, |b| {
            u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
        });
        Self {
            command,
            event_id: u16_at(2),
            stream_channel: u16_at(4),
            block_id: u16_at(6),
            timestamp,
            data: payload.get(16..).unwrap_or_default().to_vec(),
            raw: payload.to_vec(),
        }
    }
}

/// A clone-able receiver for device events. Each subscription has its own
/// bounded buffer; events are dropped (and logged) when it is full.
#[derive(Debug, Clone)]
pub struct EventChannel {
    rx: flume::Receiver<GvcpEvent>,
}

impl EventChannel {
    pub fn wait_for(&self, timeout: Duration) -> Option<GvcpEvent> {
        self.rx.recv_timeout(timeout).ok()
    }

    pub fn try_recv(&self) -> Option<GvcpEvent> {
        self.rx.try_recv().ok()
    }

    pub fn is_disconnected(&self) -> bool {
        self.rx.is_disconnected()
    }

    #[cfg(feature = "async")]
    pub async fn recv_async(&self) -> Option<GvcpEvent> {
        self.rx.recv_async().await.ok()
    }
}

/// The state the control worker genuinely shares with the camera side:
/// counters it writes continuously and the control-loss flag it raises on
/// its way down. Everything else lives in [`Connection`] as plain fields.
pub(crate) struct Shared {
    pub(crate) stats: Mutex<LinkStats>,
    control_lost: AtomicBool,
}

impl Shared {
    fn new() -> Self {
        Self {
            stats: Mutex::new(LinkStats::default()),
            control_lost: AtomicBool::new(false),
        }
    }

    pub(crate) fn set_control_lost(&self) {
        self.control_lost.store(true, Ordering::Relaxed);
    }

    fn control_lost(&self) -> bool {
        self.control_lost.load(Ordering::Relaxed)
    }
}

/// A lightweight submission path to the control worker: enough to issue
/// transactions without owning the connection. [`StreamChannel`] holds one
/// to close its channel registers on drop, and the GenICam layer uses it as
/// the device port.
pub(crate) struct ControlPort {
    to_worker: flume::Sender<ToWorker>,
    /// Non-owning twin; only liveness and wake, never join.
    thread: ThreadHandle,
    budget: Duration,
}

impl Clone for ControlPort {
    fn clone(&self) -> Self {
        Self {
            to_worker: self.to_worker.clone(),
            thread: self.thread.to_pass_in(),
            budget: self.budget,
        }
    }
}

impl ControlPort {
    pub(crate) fn budget(&self) -> Duration {
        self.budget
    }

    fn send(&self, msg: ToWorker) -> Result<()> {
        if !self.thread.is_alive() {
            return Err(CameraError::Disconnected);
        }
        self.to_worker
            .send(msg)
            .map_err(|_| CameraError::Disconnected)?;
        self.thread.wake().ok();
        // The worker may have died between the send and now, with its
        // shutdown drain already past. Its handle fulfilment is idempotent,
        // so failing here too is safe — and guarantees no handle is left
        // pending forever.
        if !self.thread.is_alive() {
            return Err(CameraError::Disconnected);
        }
        Ok(())
    }

    fn submit<T: Clone>(
        &self,
        build: impl FnOnce(ResponseHandle<T>) -> ToWorker,
    ) -> ResponseHandle<T> {
        let handle = ResponseHandle::new();
        if let Err(e) = self.send(build(handle.clone())) {
            handle.fail(e);
        }
        handle
    }

    pub(crate) fn read_register(&self, addr: u32) -> ResponseHandle<u32> {
        self.submit(|h| ToWorker::ReadReg(addr, h))
    }

    pub(crate) fn read_registers(&self, addrs: Vec<u32>) -> ResponseHandle<Vec<u32>> {
        self.submit(|h| ToWorker::ReadRegs(addrs, h))
    }

    pub(crate) fn write_register(&self, addr: u32, value: u32) -> ResponseHandle<()> {
        tracing::trace!("write register {addr:#06x} := {value:#010x}");
        self.submit(|h| ToWorker::WriteRegs(vec![(addr, value)], h))
    }

    pub(crate) fn write_registers(&self, pairs: Vec<(u32, u32)>) -> ResponseHandle<()> {
        tracing::trace!(?pairs, "write registers");
        self.submit(|h| ToWorker::WriteRegs(pairs, h))
    }

    pub(crate) fn read_memory(&self, addr: u32, len: u32) -> ResponseHandle<Vec<u8>> {
        let handle = ResponseHandle::new();
        if !addr.is_multiple_of(4) {
            handle.fail(CameraError::Protocol(
                "read address must be 4-byte aligned".into(),
            ));
            return handle;
        }
        if let Err(e) = self.send(ToWorker::ReadMem {
            addr,
            len,
            handle: handle.clone(),
        }) {
            handle.fail(e);
        }
        handle
    }

    pub(crate) fn write_memory(&self, addr: u32, data: Vec<u8>) -> ResponseHandle<()> {
        tracing::trace!("write memory {addr:#06x} ({} bytes)", data.len());
        let handle = ResponseHandle::new();
        if !addr.is_multiple_of(4) || !data.len().is_multiple_of(4) {
            handle.fail(CameraError::Protocol(
                "write address and length must be 4-byte aligned".into(),
            ));
            return handle;
        }
        if let Err(e) = self.send(ToWorker::WriteMem {
            addr,
            data,
            handle: handle.clone(),
        }) {
            handle.fail(e);
        }
        handle
    }
}

/// One established control link and everything scoped to it.
struct Connection {
    port: ControlPort,
    /// Owner handle; joins the worker on drop.
    thread: ThreadHandle,
    shared: Arc<Shared>,
    /// The device endpoint this connection was established to (the config
    /// may have been retargeted since).
    device_addr: SocketAddr,
    local_addr: SocketAddr,
    info: DeviceInfo,
    capabilities: u32,
    genicam_xml: Option<Arc<[u8]>>,
    genicam: Option<crate::genicam::Genicam>,
}

/// A GigE Vision device over its GVCP control channel.
///
/// Plain owned value: `new`/`with_config` store configuration without IO,
/// [`connect`](Self::connect) establishes the link (taking device control,
/// starting heartbeat), and [`disconnect`](Self::disconnect) — or drop —
/// releases it. Worker-facing methods return
/// [`CameraError::Disconnected`] while no link is up. Share explicitly
/// (e.g. `Arc<Mutex<GigECamera>>`) if you need to.
pub struct GigECamera {
    cfg: GigeConfig,
    connection: Option<Connection>,
}

impl std::fmt::Debug for GigECamera {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GigECamera")
            .field("addr", &self.cfg.addr)
            .field("connected", &self.is_connected())
            .field(
                "model",
                &self
                    .connection
                    .as_ref()
                    .map(|c| c.info.model.as_str())
                    .unwrap_or(""),
            )
            .finish()
    }
}

impl GigECamera {
    /// A disconnected camera targeting `ip:3956`. No I/O, infallible.
    pub fn new(ip: impl Into<IpAddr>) -> Self {
        Self::with_config(GigeConfig::new(ip))
    }

    /// A disconnected camera with full configuration. No I/O, infallible.
    pub fn with_config(cfg: GigeConfig) -> Self {
        Self {
            cfg,
            connection: None,
        }
    }

    pub fn config(&self) -> &GigeConfig {
        &self.cfg
    }

    /// Retarget or retune; takes effect on the next [`connect`](Self::connect).
    pub fn config_mut(&mut self) -> &mut GigeConfig {
        &mut self.cfg
    }

    /// Establish the control link: bind, take device control (CCP), write
    /// the heartbeat timeout, and read capabilities and identity. Blocking;
    /// a no-op when already connected. A previous dead connection is torn
    /// down first, so this is also the reconnect path.
    pub fn connect(&mut self) -> Result<()> {
        if let Some(conn) = &self.connection {
            if conn.thread.is_alive() {
                return Ok(());
            }
            // Dropping joins the dead worker and frees per-connection state.
            self.connection = None;
        }
        let conn = establish(self.cfg.clone())?;
        tracing::debug!(
            addr = %conn.device_addr,
            model = %conn.info.model,
            serial = %conn.info.serial,
            "connected"
        );
        self.connection = Some(conn);
        Ok(())
    }

    /// Stop the worker, releasing device control, and drop all
    /// per-connection state. Blocks up to `deadline` for the release write
    /// to go out; [`connect`](Self::connect) redials afterwards.
    pub fn disconnect(&mut self, deadline: Duration) {
        let Some(conn) = self.connection.take() else {
            return;
        };
        tracing::debug!(addr = %conn.device_addr, "disconnecting");
        let _ = conn.port.send(ToWorker::Shutdown);
        let start = std::time::Instant::now();
        while conn.thread.is_alive() && start.elapsed() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        conn.thread.request_stop();
        // Dropping `conn` joins the worker.
    }

    pub fn is_connected(&self) -> bool {
        self.connection
            .as_ref()
            .is_some_and(|c| c.thread.is_alive())
    }

    fn conn(&self) -> Result<&Connection> {
        match &self.connection {
            None => Err(CameraError::Disconnected),
            Some(c) if !c.thread.is_alive() => Err(if c.shared.control_lost() {
                CameraError::ControlLost
            } else {
                CameraError::Disconnected
            }),
            Some(c) => Ok(c),
        }
    }

    fn conn_mut(&mut self) -> Result<&mut Connection> {
        self.conn()?;
        match &mut self.connection {
            Some(c) => Ok(c),
            None => Err(CameraError::Disconnected),
        }
    }

    pub fn read_register(&self, addr: u32) -> Result<ResponseHandle<u32>> {
        Ok(self.conn()?.port.read_register(addr))
    }

    /// Read several registers in one transaction.
    pub fn read_registers(&self, addrs: Vec<u32>) -> Result<ResponseHandle<Vec<u32>>> {
        Ok(self.conn()?.port.read_registers(addrs))
    }

    pub fn write_register(&self, addr: u32, value: u32) -> Result<ResponseHandle<()>> {
        Ok(self.conn()?.port.write_register(addr, value))
    }

    /// Write several registers in one transaction.
    pub fn write_registers(&self, pairs: Vec<(u32, u32)>) -> Result<ResponseHandle<()>> {
        Ok(self.conn()?.port.write_registers(pairs))
    }

    /// Read `len` bytes of device memory, transparently split into GVCP-sized
    /// chunks. `addr` must be 4-byte aligned.
    pub fn read_memory(&self, addr: u32, len: u32) -> Result<ResponseHandle<Vec<u8>>> {
        Ok(self.conn()?.port.read_memory(addr, len))
    }

    /// Write device memory, transparently split into GVCP-sized chunks.
    /// `addr` and `data.len()` must be 4-byte aligned.
    pub fn write_memory(&self, addr: u32, data: Vec<u8>) -> Result<ResponseHandle<()>> {
        Ok(self.conn()?.port.write_memory(addr, data))
    }

    /// Identity and addressing read from the device at connect.
    pub fn device_info(&self) -> Result<&DeviceInfo> {
        Ok(&self.conn()?.info)
    }

    /// The GVCP capability register (0x0934) read at connect; see the
    /// `bootstrap::CAP_*` bits.
    pub fn capabilities(&self) -> Result<u32> {
        Ok(self.conn()?.capabilities)
    }

    /// Subscribe to device-initiated events. Each call returns an independent
    /// buffered channel. Call [`enable_events`](Self::enable_events) to point
    /// the device's message channel at this host.
    pub fn events(&self) -> Result<EventChannel> {
        let conn = self.conn()?;
        let (tx, rx) = flume::bounded(self.cfg.event_capacity);
        conn.port.send(ToWorker::SubscribeEvents(tx))?;
        Ok(EventChannel { rx })
    }

    /// Open the device's message channel towards the control socket, so
    /// EVENT/EVENTDATA commands arrive on [`events`](Self::events) channels.
    pub fn enable_events(&self) -> Result<()> {
        let conn = self.conn()?;
        if conn.capabilities & bootstrap::CAP_EVENT == 0 {
            return Err(CameraError::Unsupported("message channel events"));
        }
        let IpAddr::V4(host_v4) = advertised_host_ip(conn)? else {
            return Err(CameraError::Unsupported(
                "IPv6 message channel destinations",
            ));
        };
        for (addr, value) in [
            (bootstrap::MESSAGE_CHANNEL_DEST_ADDRESS, u32::from(host_v4)),
            (bootstrap::MESSAGE_CHANNEL_TRANSMISSION_TIMEOUT, 1000),
            (bootstrap::MESSAGE_CHANNEL_RETRY_COUNT, 3),
            (
                bootstrap::MESSAGE_CHANNEL_PORT,
                u32::from(conn.local_addr.port()),
            ),
        ] {
            conn.port
                .write_register(addr, value)
                .wait_timeout(conn.port.budget())
                .map_err(unwrap_arc)?;
        }
        Ok(())
    }

    /// Close the message channel (MCP := 0).
    pub fn disable_events(&self) -> Result<()> {
        let conn = self.conn()?;
        conn.port
            .write_register(bootstrap::MESSAGE_CHANNEL_PORT, 0)
            .wait_timeout(conn.port.budget())
            .map_err(unwrap_arc)?;
        Ok(())
    }

    /// Control-channel counters of the current connection, if any.
    pub fn stats(&self) -> Option<LinkStats> {
        self.connection.as_ref().map(|c| *c.shared.stats.lock())
    }

    /// Open a GVSP stream channel: bind a receive socket, point the device's
    /// SCDA/SCP at it, settle the packet size (negotiating it when
    /// configured `Auto`), and start the reassembly worker.
    ///
    /// The device only transmits once acquisition is started (the
    /// `AcquisitionStart` feature, or
    /// [`GenICamera::start_acquisition`](crate::GenICamera)).
    pub fn open_stream(&self, cfg: StreamConfig) -> Result<StreamChannel> {
        let payload_size = cfg
            .payload_size
            .filter(|&n| n > 0)
            .ok_or_else(|| CameraError::Protocol("payload_size must be set".into()))?;
        let conn = self.conn()?;
        let port = &conn.port;
        let budget = port.budget();
        let base = cfg.channel_base();

        let local = match cfg.local_addr {
            Some(a) => a,
            None => SocketAddr::new(advertised_host_ip(conn)?, 0),
        };
        let socket = snare::net::UdpSocket::bind(local)?;
        let bound = socket.local_addr()?;
        let IpAddr::V4(host_v4) = bound.ip() else {
            return Err(CameraError::Unsupported("IPv6 stream destinations"));
        };
        if host_v4.is_unspecified() {
            tracing::debug!(
                local = %bound,
                "stream host IP unresolved; advertising SCDA=0 (device streams to the GVCP requester)"
            );
        }
        set_receive_buffer(&socket, &cfg, payload_size);

        port.write_register(
            bootstrap::STREAM_CHANNEL_DEST_ADDRESS + base,
            u32::from(host_v4),
        )
        .wait_timeout(budget)
        .map_err(unwrap_arc)?;
        port.write_register(
            bootstrap::STREAM_CHANNEL_PORT + base,
            u32::from(bound.port()),
        )
        .wait_timeout(budget)
        .map_err(unwrap_arc)?;

        let scps_addr = bootstrap::STREAM_CHANNEL_PACKET_SIZE + base;
        let packet_size = match cfg.packet_size {
            crate::gige::stream::PacketSize::Fixed(n) => {
                port.write_register(scps_addr, u32::from(n))
                    .wait_timeout(budget)
                    .map_err(unwrap_arc)?;
                n
            }
            crate::gige::stream::PacketSize::Auto => {
                negotiate_packet_size(port, conn.device_addr.ip(), &socket, scps_addr)?
            }
        };
        if let Some(delay) = cfg.packet_delay {
            port.write_register(bootstrap::STREAM_CHANNEL_PACKET_DELAY + base, delay)
                .wait_timeout(budget)
                .map_err(unwrap_arc)?;
        }

        // These registers describe the clock frequency, not the advancing counter.
        // Single-register requests also work with cameras that do not support batches.
        let tick_frequency = (|| {
            let high = port
                .read_register(bootstrap::TIMESTAMP_TICK_FREQUENCY_HIGH)
                .wait_timeout(budget)?;
            let low = port
                .read_register(bootstrap::TIMESTAMP_TICK_FREQUENCY_LOW)
                .wait_timeout(budget)?;
            Ok::<_, Arc<CameraError>>((u64::from(high) << 32) | u64::from(low))
        })()
        .unwrap_or(0);

        let resend_enabled = cfg.resend == crate::gige::stream::ResendPolicy::Always
            && conn.capabilities & bootstrap::CAP_PACKET_RESEND != 0;

        let shared = Arc::new(StreamShared {
            stats: Mutex::new(Default::default()),
        });
        let (to_worker, rx) = flume::unbounded();
        let channel = cfg.channel;
        let thread = crate::gige::stream::runner::spawn(
            socket,
            crate::gige::stream::runner::LinkParams {
                device_gvcp_addr: conn.device_addr,
                scps_packet_size: packet_size,
                payload_size,
                resend_enabled,
                tick_frequency,
            },
            rx,
            shared.clone(),
            cfg,
        )?;

        tracing::debug!(channel, local = %bound, packet_size, "stream channel opened");
        Ok(StreamChannel {
            to_worker,
            thread,
            shared,
            control: port.clone(),
            channel_base: base,
            packet_size,
            local_addr: bound,
        })
    }

    /// The device description XML, fetched (and unzipped) on first call,
    /// then cached for the lifetime of the connection. Tries URL register 0,
    /// falls back to URL register 1.
    pub fn genicam_xml(&mut self) -> Result<Arc<[u8]>> {
        let conn = self.conn_mut()?;
        if let Some(xml) = &conn.genicam_xml {
            return Ok(xml.clone());
        }
        let mut last_err = CameraError::Protocol("no GenICam URL".into());
        for url_addr in [bootstrap::XML_URL_0, bootstrap::XML_URL_1] {
            match fetch_genicam_xml(&conn.port, url_addr) {
                Ok(xml) => {
                    conn.genicam_xml = Some(xml.clone());
                    return Ok(xml);
                }
                Err(e) => {
                    tracing::debug!("GenICam URL at {url_addr:#x} failed: {e}");
                    last_err = e;
                }
            }
        }
        Err(last_err)
    }

    /// Parse and cache the GenICam node graph for this connection.
    pub(crate) fn ensure_genicam(&mut self) -> crate::error::GenicamResult<()> {
        if self.conn()?.genicam.is_some() {
            return Ok(());
        }
        let xml = self.genicam_xml()?;
        let text = String::from_utf8_lossy(&xml);
        let graph = crate::genicam::parse_xml(&text)?;
        tracing::debug!("GenICam feature model loaded");
        self.conn_mut()?.genicam = Some(graph);
        Ok(())
    }

    /// The node graph and the device port to evaluate it against. The
    /// disjoint borrows are what let feature access take a true `&mut`
    /// graph while register IO goes through the port.
    pub(crate) fn genicam_parts(
        &mut self,
    ) -> crate::error::GenicamResult<(&mut crate::genicam::Genicam, &ControlPort)> {
        let conn = self.conn_mut()?;
        let Connection { genicam, port, .. } = conn;
        match genicam {
            Some(graph) => Ok((graph, port)),
            None => Err(crate::error::GenicamError::Xml(
                "feature model not loaded".into(),
            )),
        }
    }

    pub(crate) fn genicam_ref(&self) -> Option<&crate::genicam::Genicam> {
        self.connection.as_ref()?.genicam.as_ref()
    }
}

/// The host IP to advertise to the device for a return channel (GVSP stream
/// or message channel). Prefers the control socket's bound IP — it provably
/// reaches the device — then a probe socket, whose `connect` a real kernel
/// resolves to the outbound interface's address. On a host that only has
/// wildcard binds (e.g. a virtual network shim) both stay unspecified; the
/// caller then advertises an address of 0 and relies on the device streaming
/// back to the GVCP requester (see `GigeDevice::stream_dest`).
fn advertised_host_ip(conn: &Connection) -> Result<IpAddr> {
    if !conn.local_addr.ip().is_unspecified() {
        return Ok(conn.local_addr.ip());
    }
    let probe = snare::net::UdpSocket::bind("0.0.0.0:0")?;
    probe.connect(conn.device_addr)?;
    Ok(probe.local_addr()?.ip())
}

/// Dial and bootstrap one connection. On any failure the worker is torn
/// down (joined, control released) by `ThreadHandle`'s drop before the
/// error propagates — nothing leaks out of a half-built connection.
fn establish(cfg: GigeConfig) -> Result<Connection> {
    let shared = Arc::new(Shared::new());
    let (to_worker, rx) = flume::unbounded();
    let (thread, local_addr) = runner::spawn(rx, shared.clone(), cfg.clone())?;
    let port = ControlPort {
        to_worker,
        thread: thread.to_pass_in(),
        budget: cfg.transaction_budget(),
    };
    let budget = port.budget();

    let ccp = bootstrap::CCP_CONTROL
        | if cfg.exclusive {
            bootstrap::CCP_EXCLUSIVE
        } else {
            0
        };
    port.write_register(bootstrap::CONTROL_CHANNEL_PRIVILEGE, ccp)
        .wait_timeout(budget)
        .map_err(|e| match &*e {
            CameraError::Nak { status, .. }
                if *status == GvcpStatus::ACCESS_DENIED || *status == GvcpStatus::WRITE_PROTECT =>
            {
                CameraError::ControlDenied
            }
            CameraError::Timeout => CameraError::ConnectTimeout,
            _ => unwrap_arc(e),
        })?;

    port.write_register(bootstrap::HEARTBEAT_TIMEOUT, cfg.heartbeat_timeout_ms)
        .wait_timeout(budget)
        .map_err(unwrap_arc)?;

    let capabilities = port
        .read_register(bootstrap::GVCP_CAPABILITY)
        .wait_timeout(budget)
        .map_err(unwrap_arc)?;

    let info = fetch_device_info(&port)?;

    Ok(Connection {
        port,
        thread,
        shared,
        device_addr: cfg.addr,
        local_addr,
        info,
        capabilities,
        genicam_xml: None,
        genicam: None,
    })
}

/// Assemble the device identity from individual register and string reads.
/// A single bulk read of the description block would be one transaction, but
/// real devices commonly reject READMEM spanning the reserved gaps in it.
fn fetch_device_info(port: &ControlPort) -> Result<DeviceInfo> {
    let budget = port.budget();
    // Some camera implementations answer only the first address in a batch.
    // Read each bootstrap register independently and validate every response.
    let regs = [
        bootstrap::VERSION,
        bootstrap::DEVICE_MODE,
        bootstrap::DEVICE_MAC_HIGH,
        bootstrap::DEVICE_MAC_LOW,
        bootstrap::SUPPORTED_IP_CONFIG,
        bootstrap::CURRENT_IP_CONFIG,
        bootstrap::CURRENT_IP_ADDRESS,
        bootstrap::CURRENT_SUBNET_MASK,
        bootstrap::CURRENT_GATEWAY,
    ]
    .into_iter()
    .map(|address| {
        port.read_register(address)
            .wait_timeout(budget)
            .map_err(unwrap_arc)
    })
    .collect::<Result<Vec<_>>>()?;
    let [
        version,
        device_mode,
        mac_high,
        mac_low,
        supported_ip_config,
        current_ip_config,
        ip,
        mask,
        gateway,
    ] = regs[..]
    else {
        return Err(CameraError::Protocol(
            "short bootstrap register read".into(),
        ));
    };

    let string_field = |addr: u32, size: usize| -> Result<String> {
        match port.read_memory(addr, size as u32).wait_timeout(budget) {
            Ok(raw) => {
                let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
                Ok(String::from_utf8_lossy(&raw[..end]).into_owned())
            }
            // Optional fields on some devices; identity is still usable.
            Err(e) if matches!(&*e, CameraError::Nak { .. }) => {
                tracing::trace!("string register {addr:#06x} unreadable: {e}");
                Ok(String::new())
            }
            Err(e) => Err(unwrap_arc(e)),
        }
    };

    let mut mac = [0u8; 6];
    mac[..2].copy_from_slice(&mac_high.to_be_bytes()[2..]);
    mac[2..].copy_from_slice(&mac_low.to_be_bytes());

    Ok(DeviceInfo {
        spec_version: ((version >> 16) as u16, version as u16),
        device_mode,
        mac,
        supported_ip_config,
        current_ip_config,
        ip: ip.into(),
        subnet_mask: mask.into(),
        gateway: gateway.into(),
        manufacturer: string_field(
            bootstrap::MANUFACTURER_NAME,
            bootstrap::MANUFACTURER_NAME_SIZE,
        )?,
        model: string_field(bootstrap::MODEL_NAME, bootstrap::MODEL_NAME_SIZE)?,
        device_version: string_field(bootstrap::DEVICE_VERSION, bootstrap::DEVICE_VERSION_SIZE)?,
        manufacturer_info: string_field(
            bootstrap::MANUFACTURER_INFO,
            bootstrap::MANUFACTURER_INFO_SIZE,
        )?,
        serial: string_field(bootstrap::SERIAL_NUMBER, bootstrap::SERIAL_NUMBER_SIZE)?,
        user_defined_name: string_field(
            bootstrap::USER_DEFINED_NAME,
            bootstrap::USER_DEFINED_NAME_SIZE,
        )?,
    })
}

fn fetch_genicam_xml(port: &ControlPort, url_addr: u32) -> Result<Arc<[u8]>> {
    let budget = port.budget();
    let raw = port
        .read_memory(url_addr, bootstrap::XML_URL_SIZE as u32)
        .wait_timeout(budget * 2)
        .map_err(unwrap_arc)?;
    let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    let url_str = String::from_utf8_lossy(&raw[..end]);
    let url = crate::genicam::XmlUrl::parse(&url_str).map_err(CameraError::Protocol)?;

    let data = match &url {
        crate::genicam::XmlUrl::Local { address, size, .. } => {
            let address = u32::try_from(*address)
                .map_err(|_| CameraError::Protocol("XML address beyond 32 bit".into()))?;
            let budget = budget * (*size as u32 / 512 + 2);
            port.read_memory(address, *size as u32)
                .wait_timeout(budget)
                .map_err(unwrap_arc)?
        }
        crate::genicam::XmlUrl::File(path) => std::fs::read(path)?,
        crate::genicam::XmlUrl::Http(_) => {
            return Err(CameraError::Unsupported("http-hosted GenICam XML"));
        }
    };
    let data = if url.is_zip() {
        crate::genicam::zip::extract_first_file(&data).map_err(CameraError::Protocol)?
    } else {
        data
    };
    Ok(Arc::from(data.into_boxed_slice()))
}

/// Fire-test bisection of the SCPS packet size: write a candidate size with the
/// fire-test and do-not-fragment bits and watch for the device's test
/// packet on the stream socket. Devices may align or cap the size they
/// actually use (e.g. 16-byte alignment), so whatever size the test packet
/// *arrives* at is taken as the verified value — the probe only decides the
/// search direction.
fn negotiate_packet_size(
    port: &ControlPort,
    device_ip: IpAddr,
    socket: &snare::net::UdpSocket,
    scps_addr: u32,
) -> Result<u16> {
    // Probes are 16-aligned: devices round requests down to their own
    // increment (commonly 4, sometimes 16) and some only answer the fire
    // test for sizes they support exactly. 16 is a multiple of every
    // increment seen in the wild.
    const INC: u16 = 16;
    const MIN: u16 = 560;
    const MAX: u16 = 9152;

    let budget = port.budget();
    socket
        .set_read_timeout(Some(Duration::from_millis(25)))
        .ok();
    let mut buf = vec![0u8; 0x10000];
    // Returns the SCPS-equivalent size of the test packet the device
    // managed to deliver, if any.
    let mut probe = |size: u16| -> Option<u16> {
        let value =
            u32::from(size) | bootstrap::SCPS_FIRE_TEST_PACKET | bootstrap::SCPS_DO_NOT_FRAGMENT;
        port.write_register(scps_addr, value)
            .wait_timeout(budget)
            .ok()?;
        let mut achieved = None;
        for _ in 0..3 {
            match socket.recv_from(&mut buf) {
                Ok((n, src)) if src.ip() == device_ip => {
                    let delivered = (n + gvsp::PACKET_UDP_OVERHEAD) as u16;
                    if delivered <= size {
                        achieved = achieved.max(Some(delivered));
                        if delivered == size {
                            break;
                        }
                    }
                    // Oversized replies are stale probes; keep draining.
                }
                Ok(_) => {}
                Err(_) => {}
            }
        }
        achieved
    };

    let mut best = 0u16;
    let size = if let Some(a) = probe(MAX) {
        best = a;
        MAX
    } else if let Some(mut good) = [1488u16, 1008, MIN].into_iter().find(|&c| match probe(c) {
        Some(a) => {
            best = best.max(a);
            true
        }
        None => false,
    }) {
        let mut bad = MAX;
        while bad - good > INC {
            let mid = good + (bad - good) / 2 / INC * INC;
            match probe(mid) {
                Some(a) => {
                    best = best.max(a);
                    good = mid;
                }
                None => bad = mid,
            }
        }
        best
    } else {
        // Device never produced a test packet — it likely doesn't implement
        // the mechanism. Fall back without probing further.
        tracing::warn!("packet size fire-test unsupported, falling back to 1500");
        1500
    };
    let size = size.max(best).max(MIN);

    port.write_register(scps_addr, u32::from(size))
        .wait_timeout(budget)
        .map_err(unwrap_arc)?;
    // A test packet can land after its probe stopped listening; drain such
    // residue so the stream worker doesn't mistake it for frame data.
    socket.set_read_timeout(Some(Duration::from_millis(5))).ok();
    while socket.recv_from(&mut buf).is_ok() {}
    tracing::debug!("negotiated stream packet size {size}");
    Ok(size)
}
