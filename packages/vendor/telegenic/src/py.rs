//! The `_telegenic_core` Python extension module (`py` feature).
//!
//! `#[pyclass]` attributes live on the simple core types; this module adds
//! the Python-facing method surface plus wrappers where the Rust shapes
//! don't map directly: [`Camera`] shares a [`GenICamera`] behind
//! `Arc<Mutex>`, and the snapshot session re-runs the core session sequence
//! against that shared camera because the Rust [`SnapshotSession`]
//! (crate::SnapshotSession) borrows the camera for its lifetime — a shape
//! Python objects can't hold. Python-friendly signatures (seconds as
//! `float`, IPs as `str`) wrap the Rust API, and every call that can block
//! on the device releases the GIL.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyBytes;

use crate::error::{GenicamError, GenicamResult};
use crate::genicam::{AccessMode, GenICamera};
use crate::gige::discovery::{self, DiscoveryConfig};
use crate::gige::stream::{
    FrameChannel, FrameStatus, PacketSize, ResendPolicy, StreamChannel, StreamConfig, StreamStats,
};
use crate::gige::{DeviceInfo, GigeConfig, LinkStats};

pyo3::create_exception!(
    telegenic,
    CameraException,
    pyo3::exceptions::PyRuntimeError,
    "Raised when the transport link fails: connect/acknowledge timeout, I/O \
     error, device NAK, lost control, or a malformed packet."
);

pyo3::create_exception!(
    telegenic,
    GenicamException,
    pyo3::exceptions::PyRuntimeError,
    "Raised by the GenICam feature layer: unknown feature, wrong type or \
     access mode, value out of range, or a broken device description."
);

fn parse_ip(ip: &str) -> PyResult<IpAddr> {
    ip.parse()
        .map_err(|_| PyValueError::new_err(format!("invalid IP address: {ip:?}")))
}

#[allow(clippy::fn_params_excessive_bools)]
fn build_stream_config(
    channel: u16,
    n_buffers: usize,
    packet_size: Option<u16>,
    packet_delay: Option<u32>,
    resend: bool,
) -> StreamConfig {
    let mut cfg = StreamConfig::new();
    cfg.channel = channel;
    cfg.n_buffers = n_buffers;
    if let Some(size) = packet_size {
        cfg.packet_size = PacketSize::Fixed(size);
    }
    cfg.packet_delay = packet_delay;
    if !resend {
        cfg.resend = ResendPolicy::Never;
    }
    cfg
}

/// A connected GenICam camera shared behind a mutex, so sessions and
/// channels created from it stay valid while Python holds them.
#[pyclass(frozen)]
#[derive(Debug)]
struct Camera {
    inner: Arc<Mutex<GenICamera>>,
    /// What currently owns the camera's lifecycle (an acquisition or a
    /// snapshot session), if anything. The Rust guards enforce exclusivity
    /// with a `&mut` borrow; the Python twins can't, so the camera enforces
    /// it at runtime instead.
    busy: Arc<Mutex<Option<&'static str>>>,
}

/// Clears the camera's busy marker when the owning guard goes away.
#[derive(Debug)]
struct BusyToken {
    flag: Arc<Mutex<Option<&'static str>>>,
}

impl Drop for BusyToken {
    fn drop(&mut self) {
        *self.flag.lock() = None;
    }
}

impl Camera {
    fn claim(&self, what: &'static str) -> PyResult<BusyToken> {
        let mut busy = self.busy.lock();
        if let Some(active) = *busy {
            return Err(PyValueError::new_err(format!(
                "camera is busy ({active} in progress); stop it first"
            )));
        }
        *busy = Some(what);
        Ok(BusyToken {
            flag: self.busy.clone(),
        })
    }
}

#[pymethods]
impl Camera {
    /// `Camera(ip)` — create a disconnected camera; call `connect()` to
    /// dial the device and load its feature model.
    #[new]
    #[pyo3(signature = (
        ip,
        *,
        gvcp_timeout = 0.5,
        retries = 4,
        heartbeat_timeout = 3.0,
        exclusive = false,
        local_ip = None,
    ))]
    fn py_new(
        ip: &str,
        gvcp_timeout: f64,
        retries: u8,
        heartbeat_timeout: f64,
        exclusive: bool,
        local_ip: Option<&str>,
    ) -> PyResult<Self> {
        let mut cfg = GigeConfig::new(parse_ip(ip)?);
        cfg.gvcp_timeout = Duration::from_secs_f64(gvcp_timeout);
        cfg.retries = retries;
        cfg.heartbeat_timeout_ms = (heartbeat_timeout * 1000.0) as u32;
        cfg.exclusive = exclusive;
        if let Some(local) = local_ip {
            cfg.local_addr = Some(SocketAddr::new(parse_ip(local)?, 0));
        }
        Ok(Self {
            inner: Arc::new(Mutex::new(GenICamera::with_config(cfg))),
            busy: Arc::new(Mutex::new(None)),
        })
    }

    #[pyo3(name = "connect")]
    fn py_connect(&self, py: Python<'_>) -> PyResult<()> {
        let _token = self.claim("a connect")?;
        py.detach(|| self.inner.lock().connect())
            .map_err(Into::into)
    }

    #[pyo3(name = "disconnect", signature = (deadline = 0.5))]
    fn py_disconnect(&self, py: Python<'_>, deadline: f64) -> PyResult<()> {
        let _token = self.claim("a disconnect")?;
        py.detach(|| {
            self.inner
                .lock()
                .disconnect(Duration::from_secs_f64(deadline))
        });
        Ok(())
    }

    #[pyo3(name = "is_connected")]
    fn py_is_connected(&self) -> bool {
        self.inner.lock().is_connected()
    }

    #[pyo3(name = "device_info")]
    fn py_device_info(&self) -> PyResult<DeviceInfo> {
        Ok(self.inner.lock().transport().device_info()?.clone())
    }

    #[pyo3(name = "link_stats")]
    fn py_link_stats(&self) -> Option<LinkStats> {
        self.inner.lock().transport().stats()
    }

    #[pyo3(name = "feature_names")]
    fn py_feature_names(&self) -> PyResult<Vec<String>> {
        self.inner.lock().feature_names().map_err(Into::into)
    }

    #[pyo3(name = "has_feature")]
    fn py_has_feature(&self, name: &str) -> bool {
        self.inner.lock().has_feature(name)
    }

    #[pyo3(name = "get_integer")]
    fn py_get_integer(&self, py: Python<'_>, name: &str) -> PyResult<i64> {
        py.detach(|| self.inner.lock().get_integer(name))
            .map_err(Into::into)
    }

    #[pyo3(name = "set_integer")]
    fn py_set_integer(&self, py: Python<'_>, name: &str, value: i64) -> PyResult<()> {
        py.detach(|| self.inner.lock().set_integer(name, value))
            .map_err(Into::into)
    }

    #[pyo3(name = "integer_bounds")]
    fn py_integer_bounds(&self, py: Python<'_>, name: &str) -> PyResult<(i64, i64)> {
        py.detach(|| self.inner.lock().integer_bounds(name))
            .map_err(Into::into)
    }

    #[pyo3(name = "integer_increment")]
    fn py_integer_increment(&self, py: Python<'_>, name: &str) -> PyResult<i64> {
        py.detach(|| self.inner.lock().integer_increment(name))
            .map_err(Into::into)
    }

    #[pyo3(name = "get_float")]
    fn py_get_float(&self, py: Python<'_>, name: &str) -> PyResult<f64> {
        py.detach(|| self.inner.lock().get_float(name))
            .map_err(Into::into)
    }

    #[pyo3(name = "set_float")]
    fn py_set_float(&self, py: Python<'_>, name: &str, value: f64) -> PyResult<()> {
        py.detach(|| self.inner.lock().set_float(name, value))
            .map_err(Into::into)
    }

    #[pyo3(name = "float_bounds")]
    fn py_float_bounds(&self, py: Python<'_>, name: &str) -> PyResult<(f64, f64)> {
        py.detach(|| self.inner.lock().float_bounds(name))
            .map_err(Into::into)
    }

    #[pyo3(name = "get_boolean")]
    fn py_get_boolean(&self, py: Python<'_>, name: &str) -> PyResult<bool> {
        py.detach(|| self.inner.lock().get_boolean(name))
            .map_err(Into::into)
    }

    #[pyo3(name = "set_boolean")]
    fn py_set_boolean(&self, py: Python<'_>, name: &str, value: bool) -> PyResult<()> {
        py.detach(|| self.inner.lock().set_boolean(name, value))
            .map_err(Into::into)
    }

    #[pyo3(name = "get_string")]
    fn py_get_string(&self, py: Python<'_>, name: &str) -> PyResult<String> {
        py.detach(|| self.inner.lock().get_string(name))
            .map_err(Into::into)
    }

    #[pyo3(name = "set_string")]
    fn py_set_string(&self, py: Python<'_>, name: &str, value: &str) -> PyResult<()> {
        py.detach(|| self.inner.lock().set_string(name, value))
            .map_err(Into::into)
    }

    #[pyo3(name = "get_enum")]
    fn py_get_enum(&self, py: Python<'_>, name: &str) -> PyResult<String> {
        py.detach(|| self.inner.lock().get_enum(name))
            .map_err(Into::into)
    }

    #[pyo3(name = "set_enum")]
    fn py_set_enum(&self, py: Python<'_>, name: &str, entry: &str) -> PyResult<()> {
        py.detach(|| self.inner.lock().set_enum(name, entry))
            .map_err(Into::into)
    }

    #[pyo3(name = "enum_entries")]
    fn py_enum_entries(&self, py: Python<'_>, name: &str) -> PyResult<Vec<String>> {
        py.detach(|| self.inner.lock().enum_entries(name))
            .map_err(Into::into)
    }

    #[pyo3(name = "execute")]
    fn py_execute(&self, py: Python<'_>, name: &str) -> PyResult<()> {
        py.detach(|| self.inner.lock().execute(name))
            .map_err(Into::into)
    }

    #[pyo3(name = "access_mode")]
    fn py_access_mode(&self, py: Python<'_>, name: &str) -> PyResult<AccessMode> {
        py.detach(|| self.inner.lock().access_mode(name))
            .map_err(Into::into)
    }

    #[pyo3(name = "invalidate_caches")]
    fn py_invalidate_caches(&self) -> PyResult<()> {
        self.inner.lock().invalidate_caches().map_err(Into::into)
    }

    #[pyo3(name = "start_acquisition", signature = (
        *,
        channel = 0,
        n_buffers = 8,
        packet_size = None,
        packet_delay = None,
        resend = true,
    ))]
    fn py_start_acquisition(
        &self,
        py: Python<'_>,
        channel: u16,
        n_buffers: usize,
        packet_size: Option<u16>,
        packet_delay: Option<u32>,
        resend: bool,
    ) -> PyResult<PyAcquisition> {
        let token = self.claim("an acquisition")?;
        let cfg = build_stream_config(channel, n_buffers, packet_size, packet_delay, resend);
        let (state, frames) = py.detach(|| AcqState::start(&mut self.inner.lock(), cfg))?;
        Ok(PyAcquisition {
            cam: self.inner.clone(),
            frames,
            state: Mutex::new(Some(state)),
            busy: Mutex::new(Some(token)),
        })
    }

    #[pyo3(name = "snap", signature = (
        timeout = 5.0,
        *,
        channel = 0,
        n_buffers = 8,
        packet_size = None,
        packet_delay = None,
        resend = true,
    ))]
    #[allow(clippy::too_many_arguments)]
    fn py_snap(
        &self,
        py: Python<'_>,
        timeout: f64,
        channel: u16,
        n_buffers: usize,
        packet_size: Option<u16>,
        packet_delay: Option<u32>,
        resend: bool,
    ) -> PyResult<PyFrame> {
        let _token = self.claim("a snap")?;
        let cfg = build_stream_config(channel, n_buffers, packet_size, packet_delay, resend);
        let frame = py.detach(|| {
            self.inner
                .lock()
                .snap(cfg, Duration::from_secs_f64(timeout))
        })?;
        Ok(PyFrame { inner: frame })
    }

    #[pyo3(name = "snapshot_session", signature = (
        *,
        channel = 0,
        n_buffers = 8,
        packet_size = None,
        packet_delay = None,
        resend = true,
    ))]
    fn py_snapshot_session(
        &self,
        py: Python<'_>,
        channel: u16,
        n_buffers: usize,
        packet_size: Option<u16>,
        packet_delay: Option<u32>,
        resend: bool,
    ) -> PyResult<PySnapshotSession> {
        let token = self.claim("a snapshot session")?;
        let cfg = build_stream_config(channel, n_buffers, packet_size, packet_delay, resend);
        let state = py.detach(|| SessionState::open(&mut self.inner.lock(), cfg))?;
        Ok(PySnapshotSession {
            cam: self.inner.clone(),
            state: Mutex::new(Some(state)),
            busy: Mutex::new(Some(token)),
        })
    }

    fn __repr__(&self) -> String {
        let cam = self.inner.lock();
        format!(
            "Camera({}, connected={})",
            cam.transport().config().addr.ip(),
            if cam.is_connected() { "True" } else { "False" }
        )
    }
}

/// The Python twin of the Rust `SnapshotSession` sequence, run against the
/// shared camera per call instead of holding a borrow. Kept in lockstep
/// with `genicam::snapshot` — change one, change both.
#[derive(Debug)]
struct SessionState {
    stream: StreamChannel,
    frames: FrameChannel,
    auto_stops: bool,
    restore_mode: Option<String>,
}

impl SessionState {
    fn open(cam: &mut GenICamera, mut cfg: StreamConfig) -> GenicamResult<Self> {
        let mut restore_mode = None;
        let auto_stops = match cam.get_enum("AcquisitionMode") {
            Ok(mode) if mode == "SingleFrame" => true,
            Ok(mode) => match cam.set_enum("AcquisitionMode", "SingleFrame") {
                Ok(()) => {
                    restore_mode = Some(mode);
                    true
                }
                Err(e) => {
                    tracing::debug!("SingleFrame mode unavailable, stopping per snap: {e}");
                    false
                }
            },
            Err(e) => {
                tracing::debug!("AcquisitionMode unavailable, stopping per snap: {e}");
                false
            }
        };

        let stream = cam
            .fill_payload_size(&mut cfg)
            .and_then(|()| Ok(cam.transport().open_stream(cfg)?));
        let stream = match stream {
            Ok(stream) => stream,
            Err(e) => {
                if let Some(mode) = restore_mode {
                    let _ = cam.set_enum("AcquisitionMode", &mode);
                }
                return Err(e);
            }
        };
        if let Err(e) = cam.set_integer("TLParamsLocked", 1) {
            tracing::debug!("TLParamsLocked not set: {e}");
        }
        let frames = stream.subscribe(4);
        Ok(Self {
            stream,
            frames,
            auto_stops,
            restore_mode,
        })
    }

    fn snap(&self, cam: &mut GenICamera, timeout: Duration) -> GenicamResult<Arc<crate::Frame>> {
        self.frames.clear();
        cam.execute("AcquisitionStart")?;
        let frame = self.frames.wait_for(timeout);
        if (!self.auto_stops || frame.is_none())
            && let Err(e) = cam.execute("AcquisitionStop")
        {
            tracing::warn!("AcquisitionStop after snap failed: {e}");
        }
        frame.ok_or(GenicamError::FrameTimeout)
    }

    fn close(mut self, cam: &mut GenICamera) {
        if let Err(e) = cam.set_integer("TLParamsLocked", 0) {
            tracing::debug!("TLParamsLocked not cleared: {e}");
        }
        if let Some(mode) = self.restore_mode.take()
            && let Err(e) = cam.set_enum("AcquisitionMode", &mode)
        {
            tracing::debug!("AcquisitionMode not restored: {e}");
        }
        // Dropping self closes the stream channel on the device (SCP := 0).
    }
}

/// An open stream channel dedicated to on-demand single-frame capture; the
/// camera transmits only while a `snap()` is in flight. Use as a context
/// manager or call `close()` to restore the camera's acquisition mode.
#[pyclass(name = "SnapshotSession", frozen)]
#[derive(Debug)]
struct PySnapshotSession {
    cam: Arc<Mutex<GenICamera>>,
    state: Mutex<Option<SessionState>>,
    busy: Mutex<Option<BusyToken>>,
}

impl PySnapshotSession {
    fn close_now(&self) {
        let mut cam = self.cam.lock();
        if let Some(state) = self.state.lock().take() {
            state.close(&mut cam);
        }
        drop(self.busy.lock().take());
    }
}

#[pymethods]
impl PySnapshotSession {
    #[pyo3(name = "snap", signature = (timeout = 5.0))]
    fn py_snap(&self, py: Python<'_>, timeout: f64) -> PyResult<PyFrame> {
        let frame = py.detach(|| -> PyResult<Arc<crate::Frame>> {
            let mut cam = self.cam.lock();
            let state = self.state.lock();
            let state = state
                .as_ref()
                .ok_or_else(|| PyValueError::new_err("snapshot session is closed"))?;
            Ok(state.snap(&mut cam, Duration::from_secs_f64(timeout))?)
        })?;
        Ok(PyFrame { inner: frame })
    }

    #[pyo3(name = "stats")]
    fn py_stats(&self) -> PyResult<StreamStats> {
        self.state
            .lock()
            .as_ref()
            .map(|s| s.stream.stats())
            .ok_or_else(|| PyValueError::new_err("snapshot session is closed"))
    }

    #[pyo3(name = "packet_size")]
    fn py_packet_size(&self) -> PyResult<u16> {
        self.state
            .lock()
            .as_ref()
            .map(|s| s.stream.packet_size())
            .ok_or_else(|| PyValueError::new_err("snapshot session is closed"))
    }

    #[pyo3(name = "is_closed")]
    fn py_is_closed(&self) -> bool {
        self.state.lock().is_none()
    }

    /// Restore `AcquisitionMode`, unlock transport parameters, and close the
    /// stream channel. Idempotent.
    #[pyo3(name = "close")]
    fn py_close(&self, py: Python<'_>) {
        py.detach(|| self.close_now());
    }

    fn __enter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    #[pyo3(signature = (*_args))]
    fn __exit__(&self, py: Python<'_>, _args: &Bound<'_, pyo3::types::PyTuple>) -> bool {
        self.py_close(py);
        false
    }

    fn __repr__(&self) -> String {
        format!(
            "SnapshotSession(closed={})",
            if self.state.lock().is_none() {
                "True"
            } else {
                "False"
            }
        )
    }
}

impl Drop for PySnapshotSession {
    fn drop(&mut self) {
        let Some(state) = self.state.lock().take() else {
            return;
        };
        // Bounded wait: drop may run with the GIL held while another thread
        // holds the camera inside a detached call; never deadlock here. The
        // stream channel still closes when `state` drops either way.
        if let Some(mut cam) = self.cam.try_lock_for(Duration::from_millis(200)) {
            state.close(&mut cam);
        } else {
            tracing::warn!("snapshot session dropped while camera busy; mode not restored");
        }
    }
}

/// The Python twin of the Rust `Acquisition` start/stop sequence, run
/// against the shared camera per call instead of holding a borrow. Kept in
/// lockstep with `genicam::acquisition` — change one, change both.
#[derive(Debug)]
struct AcqState {
    stream: StreamChannel,
}

impl AcqState {
    fn start(cam: &mut GenICamera, mut cfg: StreamConfig) -> GenicamResult<(Self, FrameChannel)> {
        cam.fill_payload_size(&mut cfg)?;
        let capacity = cfg.n_buffers;
        let stream = cam.transport().open_stream(cfg)?;
        let frames = stream.subscribe(capacity);
        if let Err(e) = cam.set_integer("TLParamsLocked", 1) {
            tracing::debug!("TLParamsLocked not set: {e}");
        }
        if let Err(e) = cam.execute("AcquisitionStart") {
            if let Err(e) = cam.set_integer("TLParamsLocked", 0) {
                tracing::debug!("TLParamsLocked not cleared: {e}");
            }
            return Err(e);
        }
        Ok((Self { stream }, frames))
    }

    fn stop(self, cam: &mut GenICamera) -> GenicamResult<()> {
        let result = cam.execute("AcquisitionStop");
        if let Err(e) = cam.set_integer("TLParamsLocked", 0) {
            tracing::debug!("TLParamsLocked not cleared: {e}");
        }
        // Dropping self closes the stream channel on the device (SCP := 0).
        result
    }
}

/// A running continuous acquisition. The built-in subscription is created
/// before the camera starts, so even the first frame is delivered. Use as a
/// context manager or call `stop()`; buffered frames stay readable after.
#[pyclass(name = "Acquisition", frozen)]
#[derive(Debug)]
struct PyAcquisition {
    cam: Arc<Mutex<GenICamera>>,
    frames: FrameChannel,
    state: Mutex<Option<AcqState>>,
    busy: Mutex<Option<BusyToken>>,
}

impl PyAcquisition {
    fn stop_now(&self) -> GenicamResult<()> {
        let mut cam = self.cam.lock();
        let result = match self.state.lock().take() {
            Some(state) => state.stop(&mut cam),
            None => Ok(()),
        };
        drop(self.busy.lock().take());
        result
    }

    fn with_stream<T>(&self, f: impl FnOnce(&StreamChannel) -> T) -> PyResult<T> {
        self.state
            .lock()
            .as_ref()
            .map(|s| f(&s.stream))
            .ok_or_else(|| PyValueError::new_err("acquisition is stopped"))
    }
}

#[pymethods]
impl PyAcquisition {
    #[pyo3(name = "wait_for", signature = (timeout = 1.0))]
    fn py_wait_for(&self, py: Python<'_>, timeout: f64) -> Option<PyFrame> {
        py.detach(|| self.frames.wait_for(Duration::from_secs_f64(timeout)))
            .map(|inner| PyFrame { inner })
    }

    #[pyo3(name = "try_recv")]
    fn py_try_recv(&self) -> Option<PyFrame> {
        self.frames.try_recv().map(|inner| PyFrame { inner })
    }

    #[pyo3(name = "recv_all")]
    fn py_recv_all(&self) -> Vec<PyFrame> {
        self.frames
            .recv_all()
            .into_iter()
            .map(|inner| PyFrame { inner })
            .collect()
    }

    #[pyo3(name = "clear")]
    fn py_clear(&self) {
        self.frames.clear();
    }

    /// An additional subscription with its own buffer; unlike the built-in
    /// one it only sees frames completed after this call.
    #[pyo3(name = "subscribe", signature = (capacity = 16))]
    fn py_subscribe(&self, capacity: usize) -> PyResult<FrameChannel> {
        self.with_stream(|s| s.subscribe(capacity))
    }

    #[pyo3(name = "stats")]
    fn py_stats(&self) -> PyResult<StreamStats> {
        self.with_stream(StreamChannel::stats)
    }

    #[pyo3(name = "packet_size")]
    fn py_packet_size(&self) -> PyResult<u16> {
        self.with_stream(StreamChannel::packet_size)
    }

    #[pyo3(name = "local_addr")]
    fn py_local_addr(&self) -> PyResult<String> {
        self.with_stream(|s| s.local_addr().to_string())
    }

    #[pyo3(name = "is_stopped")]
    fn py_is_stopped(&self) -> bool {
        self.state.lock().is_none()
    }

    /// Stop the camera, unlock transport parameters, and close the stream
    /// channel. Idempotent; frames already buffered stay readable.
    #[pyo3(name = "stop")]
    fn py_stop(&self, py: Python<'_>) -> PyResult<()> {
        py.detach(|| self.stop_now()).map_err(Into::into)
    }

    fn __enter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    #[pyo3(signature = (*_args))]
    fn __exit__(&self, py: Python<'_>, _args: &Bound<'_, pyo3::types::PyTuple>) -> PyResult<bool> {
        self.py_stop(py)?;
        Ok(false)
    }

    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&self, py: Python<'_>) -> PyResult<Option<PyFrame>> {
        loop {
            if let Some(inner) = py.detach(|| self.frames.wait_for(NEXT_POLL)) {
                return Ok(Some(PyFrame { inner }));
            }
            py.check_signals()?;
            if self.frames.is_disconnected() {
                return Ok(None);
            }
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "Acquisition(stopped={})",
            if self.state.lock().is_none() {
                "True"
            } else {
                "False"
            }
        )
    }
}

impl Drop for PyAcquisition {
    fn drop(&mut self) {
        let Some(state) = self.state.lock().take() else {
            return;
        };
        // Bounded wait, mirroring PySnapshotSession::drop: never deadlock
        // with the GIL held. The stream channel still closes either way.
        if let Some(mut cam) = self.cam.try_lock_for(Duration::from_millis(200)) {
            if let Err(e) = state.stop(&mut cam) {
                tracing::warn!("AcquisitionStop on drop failed: {e}");
            }
        } else {
            tracing::warn!("acquisition dropped while camera busy; AcquisitionStop not sent");
        }
    }
}

/// One captured frame. The pixel payload is copied out by `data()`.
#[pyclass(name = "Frame", frozen)]
#[derive(Debug)]
struct PyFrame {
    inner: Arc<crate::Frame>,
}

#[pymethods]
impl PyFrame {
    #[getter]
    fn status(&self) -> FrameStatus {
        self.inner.status
    }

    /// `True` when every packet of the frame arrived.
    #[getter]
    fn is_complete(&self) -> bool {
        self.inner.status == FrameStatus::Complete
    }

    #[getter]
    fn frame_id(&self) -> u64 {
        self.inner.frame_id
    }

    #[getter]
    fn width(&self) -> u32 {
        self.inner.width
    }

    #[getter]
    fn height(&self) -> u32 {
        self.inner.height
    }

    #[getter]
    fn x_offset(&self) -> u32 {
        self.inner.x_offset
    }

    #[getter]
    fn y_offset(&self) -> u32 {
        self.inner.y_offset
    }

    /// Pixel format name (e.g. `"Mono8"`), or the raw GigE Vision code in
    /// hex for formats without a known name.
    #[getter]
    fn pixel_format(&self) -> String {
        self.inner.pixel_format.to_string()
    }

    /// The raw 32-bit GigE Vision pixel format code.
    #[getter]
    fn pixel_format_code(&self) -> u32 {
        self.inner.pixel_format.0
    }

    #[getter]
    fn bits_per_pixel(&self) -> u32 {
        self.inner.pixel_format.bits_per_pixel()
    }

    #[getter]
    fn timestamp_ns(&self) -> u64 {
        self.inner.timestamp_ns
    }

    #[getter]
    fn timestamp_ticks(&self) -> u64 {
        self.inner.timestamp_ticks
    }

    #[getter]
    fn system_timestamp_ns(&self) -> u64 {
        self.inner.system_timestamp_ns
    }

    #[getter]
    fn received_size(&self) -> usize {
        self.inner.received_size
    }

    /// The payload bytes (e.g. for `numpy.frombuffer`). For an incomplete
    /// frame the holes read as stale buffer content — check `status` first.
    fn data<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, self.inner.data())
    }

    fn __repr__(&self) -> String {
        format!(
            "Frame(id={}, {:?}, {}x{} {}, {} bytes)",
            self.inner.frame_id,
            self.inner.status,
            self.inner.width,
            self.inner.height,
            self.inner.pixel_format,
            self.inner.received_size,
        )
    }
}

/// How long `FrameChannel.__next__` sleeps between signal checks, so Ctrl-C
/// interrupts a blocking iteration promptly.
const NEXT_POLL: Duration = Duration::from_millis(100);

#[pymethods]
impl FrameChannel {
    #[pyo3(name = "wait_for")]
    fn py_wait_for(&self, py: Python<'_>, timeout: f64) -> Option<PyFrame> {
        py.detach(|| self.wait_for(Duration::from_secs_f64(timeout)))
            .map(|inner| PyFrame { inner })
    }

    #[pyo3(name = "try_recv")]
    fn py_try_recv(&self) -> Option<PyFrame> {
        self.try_recv().map(|inner| PyFrame { inner })
    }

    #[pyo3(name = "recv_all")]
    fn py_recv_all(&self) -> Vec<PyFrame> {
        self.recv_all()
            .into_iter()
            .map(|inner| PyFrame { inner })
            .collect()
    }

    #[pyo3(name = "clear")]
    fn py_clear(&self) {
        self.clear();
    }

    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&self, py: Python<'_>) -> PyResult<Option<PyFrame>> {
        loop {
            if let Some(inner) = py.detach(|| self.wait_for(NEXT_POLL)) {
                return Ok(Some(PyFrame { inner }));
            }
            py.check_signals()?;
            if self.is_disconnected() {
                return Ok(None);
            }
        }
    }
}

#[pymethods]
impl DeviceInfo {
    #[getter]
    fn manufacturer(&self) -> String {
        self.manufacturer.clone()
    }

    #[getter]
    fn model(&self) -> String {
        self.model.clone()
    }

    #[getter]
    fn serial(&self) -> String {
        self.serial.clone()
    }

    #[getter]
    fn device_version(&self) -> String {
        self.device_version.clone()
    }

    #[getter]
    fn manufacturer_info(&self) -> String {
        self.manufacturer_info.clone()
    }

    #[getter]
    fn user_defined_name(&self) -> String {
        self.user_defined_name.clone()
    }

    #[getter]
    fn ip(&self) -> String {
        self.ip.to_string()
    }

    #[getter]
    fn subnet_mask(&self) -> String {
        self.subnet_mask.to_string()
    }

    #[getter]
    fn gateway(&self) -> String {
        self.gateway.to_string()
    }

    #[getter]
    fn mac(&self) -> String {
        let m = self.mac;
        format!(
            "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            m[0], m[1], m[2], m[3], m[4], m[5]
        )
    }

    /// GigE Vision spec version as `(major, minor)`.
    #[getter]
    fn spec_version(&self) -> (u16, u16) {
        self.spec_version
    }

    fn __repr__(&self) -> String {
        format!(
            "DeviceInfo({} {} serial={} ip={})",
            self.manufacturer, self.model, self.serial, self.ip
        )
    }
}

#[pymethods]
impl StreamStats {
    fn __repr__(&self) -> String {
        format!("{self:?}")
    }
}

#[pymethods]
impl LinkStats {
    fn __repr__(&self) -> String {
        format!("{self:?}")
    }
}

/// Broadcast a GigE Vision discovery beacon on every Up IPv4 adapter and
/// return the devices that answer within `timeout` seconds.
#[pyfunction]
#[pyo3(signature = (timeout = 1.0))]
fn discover(py: Python<'_>, timeout: f64) -> PyResult<Vec<DeviceInfo>> {
    let cfg = DiscoveryConfig {
        recv_window: Duration::from_secs_f64(timeout),
        ..DiscoveryConfig::default()
    };
    let devices = py.detach(|| discovery::discover(&cfg))?;
    Ok(devices.into_iter().map(|d| d.info).collect())
}

#[pymodule(name = "_telegenic_core")]
fn telegenic_core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("CameraError", m.py().get_type::<CameraException>())?;
    m.add("GenicamError", m.py().get_type::<GenicamException>())?;

    m.add_class::<Camera>()?;
    m.add_class::<PySnapshotSession>()?;
    m.add_class::<PyAcquisition>()?;
    m.add_class::<PyFrame>()?;
    m.add_class::<FrameChannel>()?;
    m.add_class::<FrameStatus>()?;
    m.add_class::<AccessMode>()?;
    m.add_class::<StreamStats>()?;
    m.add_class::<LinkStats>()?;
    m.add_class::<DeviceInfo>()?;

    m.add_function(wrap_pyfunction!(discover, m)?)?;
    Ok(())
}
