//! The GenICam feature layer: [`GenICamera`] wraps a transport camera and
//! exposes the device's feature tree (Width, ExposureTime, PixelFormat,
//! AcquisitionStart, ...) by name, evaluated through the device-description
//! XML's node graph.

mod acquisition;
pub mod evaluator;
pub(crate) mod node;
pub mod port;
mod snapshot;
pub mod url;
pub(crate) mod xml;
pub(crate) mod zip;

use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use crate::error::{GenicamError, GenicamResult};
use crate::gige::GigECamera;
use crate::gige::stream::{Frame, StreamConfig};

pub use acquisition::Acquisition;
pub use node::{AccessMode, Genicam, NodeId, NodeKind};
pub use snapshot::SnapshotSession;
pub use url::XmlUrl;

/// Parse a device description XML into a node graph.
pub fn parse_xml(xml: &str) -> GenicamResult<Genicam> {
    xml::parse(xml)
}

/// Extract the (first) file of a zipped device description.
pub fn unzip(data: &[u8]) -> GenicamResult<Vec<u8>> {
    zip::extract_first_file(data).map_err(GenicamError::Xml)
}

/// A camera with a GenICam feature model. One variant per transport, so
/// USB3 Vision can slot in later without touching call sites.
///
/// Like the transport it wraps, this is an owned value with an explicit
/// lifecycle: construction is free, [`connect`](Self::connect) dials the
/// device and loads its feature model, and the model lives exactly as long
/// as the connection. Feature access takes `&mut self` — evaluation updates
/// register caches — and that exclusivity is real; share explicitly (e.g.
/// `Arc<Mutex<GenICamera>>`) if you need to.
#[derive(Debug)]
pub enum GenICamera {
    GigE(GigECamera),
}

impl GenICamera {
    /// A disconnected camera targeting `ip:3956`. No I/O, infallible.
    pub fn new(ip: impl Into<IpAddr>) -> Self {
        Self::GigE(GigECamera::new(ip))
    }

    /// A disconnected camera with full transport configuration.
    pub fn with_config(cfg: crate::gige::GigeConfig) -> Self {
        Self::GigE(GigECamera::with_config(cfg))
    }

    /// Wrap an existing transport (connected or not).
    pub fn from_transport(camera: GigECamera) -> Self {
        Self::GigE(camera)
    }

    /// Establish the transport link (a no-op when already connected) and
    /// load the device's feature model. Blocking through the XML fetch on a
    /// fresh connection.
    pub fn connect(&mut self) -> GenicamResult<()> {
        let transport = self.transport_mut();
        transport.connect()?;
        transport.ensure_genicam()
    }

    /// Disconnect the transport, dropping the feature model with the
    /// connection; [`connect`](Self::connect) redials and reloads.
    pub fn disconnect(&mut self, deadline: Duration) {
        self.transport_mut().disconnect(deadline);
    }

    pub fn is_connected(&self) -> bool {
        self.transport().is_connected()
    }

    /// The transport-level handle, for register IO and stream control.
    pub fn transport(&self) -> &GigECamera {
        match self {
            Self::GigE(camera) => camera,
        }
    }

    pub fn transport_mut(&mut self) -> &mut GigECamera {
        match self {
            Self::GigE(camera) => camera,
        }
    }

    fn with_graph<T>(
        &mut self,
        name: &str,
        f: impl FnOnce(&mut Genicam, NodeId, &dyn port::PortIo) -> GenicamResult<T>,
    ) -> GenicamResult<T> {
        let (graph, device_port) = self.transport_mut().genicam_parts()?;
        let id = graph.lookup(name)?;
        f(graph, id, device_port)
    }

    pub fn has_feature(&self, name: &str) -> bool {
        self.transport()
            .genicam_ref()
            .is_some_and(|g| g.lookup(name).is_ok())
    }

    /// Every node name in the device description (features and the register/
    /// computation nodes behind them).
    pub fn feature_names(&self) -> GenicamResult<Vec<String>> {
        let graph = self
            .transport()
            .genicam_ref()
            .ok_or_else(|| GenicamError::Xml("feature model not loaded".into()))?;
        Ok(graph.node_names().map(str::to_string).collect())
    }

    pub fn node_kind(&self, name: &str) -> GenicamResult<NodeKind> {
        let graph = self
            .transport()
            .genicam_ref()
            .ok_or_else(|| GenicamError::Xml("feature model not loaded".into()))?;
        Ok(graph.kind_of(graph.lookup(name)?))
    }

    /// The feature names a `Category` lists, in XML order (empty for
    /// non-categories). Walk from `"Root"` to traverse the whole tree.
    pub fn category_features(&self, name: &str) -> GenicamResult<Vec<String>> {
        let graph = self
            .transport()
            .genicam_ref()
            .ok_or_else(|| GenicamError::Xml("feature model not loaded".into()))?;
        let id = graph.lookup(name)?;
        Ok(graph
            .category_features(id)
            .into_iter()
            .map(|f| graph.node_name(f).to_string())
            .collect())
    }

    pub fn get_integer(&mut self, name: &str) -> GenicamResult<i64> {
        self.with_graph(name, |g, id, p| g.int_value(id, p))
    }

    pub fn set_integer(&mut self, name: &str, value: i64) -> GenicamResult<()> {
        tracing::trace!(feature = name, value, "set integer");
        self.with_graph(name, |g, id, p| g.set_int_value(id, value, p))
    }

    pub fn integer_bounds(&mut self, name: &str) -> GenicamResult<(i64, i64)> {
        self.with_graph(name, |g, id, p| g.int_bounds(id, p))
    }

    pub fn integer_increment(&mut self, name: &str) -> GenicamResult<i64> {
        self.with_graph(name, |g, id, p| g.int_increment(id, p))
    }

    pub fn get_float(&mut self, name: &str) -> GenicamResult<f64> {
        self.with_graph(name, |g, id, p| g.float_value(id, p))
    }

    pub fn set_float(&mut self, name: &str, value: f64) -> GenicamResult<()> {
        tracing::trace!(feature = name, value, "set float");
        self.with_graph(name, |g, id, p| g.set_float_value(id, value, p))
    }

    pub fn float_bounds(&mut self, name: &str) -> GenicamResult<(f64, f64)> {
        self.with_graph(name, |g, id, p| g.float_bounds(id, p))
    }

    pub fn get_boolean(&mut self, name: &str) -> GenicamResult<bool> {
        self.with_graph(name, |g, id, p| g.bool_value(id, p))
    }

    pub fn set_boolean(&mut self, name: &str, value: bool) -> GenicamResult<()> {
        tracing::trace!(feature = name, value, "set boolean");
        self.with_graph(name, |g, id, p| g.set_bool_value(id, value, p))
    }

    pub fn get_string(&mut self, name: &str) -> GenicamResult<String> {
        self.with_graph(name, |g, id, p| g.string_value(id, p))
    }

    pub fn set_string(&mut self, name: &str, value: &str) -> GenicamResult<()> {
        tracing::trace!(feature = name, value, "set string");
        self.with_graph(name, |g, id, p| g.set_string_value(id, value, p))
    }

    /// Current entry name of an enumeration feature.
    pub fn get_enum(&mut self, name: &str) -> GenicamResult<String> {
        self.with_graph(name, |g, id, p| g.string_value(id, p))
    }

    pub fn set_enum(&mut self, name: &str, entry: &str) -> GenicamResult<()> {
        tracing::trace!(feature = name, entry, "set enum");
        self.with_graph(name, |g, id, p| g.set_enum_entry(id, entry, p))
    }

    pub fn enum_entries(&mut self, name: &str) -> GenicamResult<Vec<String>> {
        self.with_graph(name, |g, id, _| g.enum_entries(id))
    }

    /// Execute a command feature (e.g. `AcquisitionStart`).
    pub fn execute(&mut self, name: &str) -> GenicamResult<()> {
        tracing::trace!(feature = name, "execute command");
        self.with_graph(name, |g, id, p| g.execute(id, p))
    }

    pub fn access_mode(&mut self, name: &str) -> GenicamResult<AccessMode> {
        self.with_graph(name, |g, id, _| Ok(g.access_mode(id)))
    }

    /// Drop every cached register value.
    pub fn invalidate_caches(&mut self) -> GenicamResult<()> {
        let (graph, _) = self.transport_mut().genicam_parts()?;
        graph.invalidate_caches();
        Ok(())
    }

    /// Start continuous acquisition: fills `payload_size` from the device's
    /// `PayloadSize` if unset, opens the GVSP channel, subscribes, sets
    /// `TLParamsLocked`, and executes `AcquisitionStart`.
    ///
    /// The returned [`Acquisition`] guard owns the whole lifecycle — its
    /// subscription exists before the camera is told to start (so even the
    /// first frame is delivered), and dropping it stops the camera, unlocks
    /// transport parameters, and closes the stream channel. The guard
    /// borrows the camera mutably: feature access while acquiring goes
    /// through [`Acquisition::camera`], and disconnecting or opening a
    /// snapshot session mid-acquisition is rejected at compile time.
    pub fn start_acquisition(&mut self, cfg: StreamConfig) -> GenicamResult<Acquisition<'_>> {
        Acquisition::start(self, cfg)
    }

    /// Capture exactly one frame, opening and closing the stream around it.
    ///
    /// Convenience over [`snapshot_session`](Self::snapshot_session) for a
    /// one-off grab. For repeated captures keep a session instead: it pays
    /// the channel setup and packet-size negotiation once (or skip the
    /// negotiation entirely with [`PacketSize::Fixed`](crate::PacketSize)).
    pub fn snap(&mut self, cfg: StreamConfig, timeout: Duration) -> GenicamResult<Arc<Frame>> {
        self.snapshot_session(cfg)?.snap(timeout)
    }

    /// Open a stream channel for on-demand single-frame capture.
    ///
    /// Unlike [`start_acquisition`](Self::start_acquisition) the camera is
    /// left idle: it transmits only while a [`snap`](SnapshotSession::snap)
    /// is in flight, so an open session uses no link bandwidth between
    /// captures. Switches `AcquisitionMode` to `SingleFrame` when the device
    /// offers it (restored when the session drops); otherwise each snap
    /// stops acquisition explicitly after its frame arrives.
    pub fn snapshot_session(&mut self, cfg: StreamConfig) -> GenicamResult<SnapshotSession<'_>> {
        SnapshotSession::open(self, cfg)
    }

    pub(crate) fn fill_payload_size(&mut self, cfg: &mut StreamConfig) -> GenicamResult<()> {
        if cfg.payload_size.is_none() {
            cfg.payload_size = Some(
                usize::try_from(self.get_integer("PayloadSize")?)
                    .map_err(|_| GenicamError::Xml("negative PayloadSize".into()))?,
            );
        }
        Ok(())
    }
}

/// Mutable feature-tree access without the lifecycle methods, handed out by
/// the acquisition guards ([`Acquisition::features`],
/// [`SnapshotSession::features`]): tuning `ExposureTime` mid-stream is fine,
/// disconnecting mid-stream is not.
#[derive(Debug)]
pub struct Features<'a>(pub(crate) &'a mut GenICamera);

impl Features<'_> {
    pub fn has_feature(&self, name: &str) -> bool {
        self.0.has_feature(name)
    }

    pub fn feature_names(&self) -> GenicamResult<Vec<String>> {
        self.0.feature_names()
    }

    pub fn get_integer(&mut self, name: &str) -> GenicamResult<i64> {
        self.0.get_integer(name)
    }

    pub fn set_integer(&mut self, name: &str, value: i64) -> GenicamResult<()> {
        self.0.set_integer(name, value)
    }

    pub fn integer_bounds(&mut self, name: &str) -> GenicamResult<(i64, i64)> {
        self.0.integer_bounds(name)
    }

    pub fn integer_increment(&mut self, name: &str) -> GenicamResult<i64> {
        self.0.integer_increment(name)
    }

    pub fn get_float(&mut self, name: &str) -> GenicamResult<f64> {
        self.0.get_float(name)
    }

    pub fn set_float(&mut self, name: &str, value: f64) -> GenicamResult<()> {
        self.0.set_float(name, value)
    }

    pub fn float_bounds(&mut self, name: &str) -> GenicamResult<(f64, f64)> {
        self.0.float_bounds(name)
    }

    pub fn get_boolean(&mut self, name: &str) -> GenicamResult<bool> {
        self.0.get_boolean(name)
    }

    pub fn set_boolean(&mut self, name: &str, value: bool) -> GenicamResult<()> {
        self.0.set_boolean(name, value)
    }

    pub fn get_string(&mut self, name: &str) -> GenicamResult<String> {
        self.0.get_string(name)
    }

    pub fn set_string(&mut self, name: &str, value: &str) -> GenicamResult<()> {
        self.0.set_string(name, value)
    }

    pub fn get_enum(&mut self, name: &str) -> GenicamResult<String> {
        self.0.get_enum(name)
    }

    pub fn set_enum(&mut self, name: &str, entry: &str) -> GenicamResult<()> {
        self.0.set_enum(name, entry)
    }

    pub fn enum_entries(&mut self, name: &str) -> GenicamResult<Vec<String>> {
        self.0.enum_entries(name)
    }

    /// Execute a command feature (e.g. `TriggerSoftware`).
    pub fn execute(&mut self, name: &str) -> GenicamResult<()> {
        self.0.execute(name)
    }

    pub fn access_mode(&mut self, name: &str) -> GenicamResult<AccessMode> {
        self.0.access_mode(name)
    }

    pub fn node_kind(&self, name: &str) -> GenicamResult<NodeKind> {
        self.0.node_kind(name)
    }

    pub fn category_features(&self, name: &str) -> GenicamResult<Vec<String>> {
        self.0.category_features(name)
    }

    pub fn invalidate_caches(&mut self) -> GenicamResult<()> {
        self.0.invalidate_caches()
    }
}
