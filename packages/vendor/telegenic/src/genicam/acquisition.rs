//! Continuous acquisition as an RAII guard.
//!
//! [`Acquisition`] owns the whole streaming lifecycle: it subscribes before
//! the camera is told to start (so even the first frame is delivered),
//! and on drop it stops the camera, unlocks transport parameters, and
//! closes the stream channel — in that order. Because it borrows the
//! camera mutably, disconnecting or opening a snapshot session while
//! acquiring is rejected at compile time.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use crate::error::GenicamResult;
use crate::gige::stream::{Frame, FrameChannel, StreamChannel, StreamConfig, StreamStats};

use super::GenICamera;

/// A running continuous acquisition.
///
/// Frames arrive on the guard's built-in subscription, sized to the buffer
/// pool (`n_buffers`); read them with [`wait_for`](Self::wait_for) or hand
/// a clone of [`frames`](Self::frames) to another thread. Stop by dropping
/// the guard, or with [`stop`](Self::stop) to observe errors.
#[derive(Debug)]
pub struct Acquisition<'a> {
    cam: &'a mut GenICamera,
    stream: StreamChannel,
    frames: FrameChannel,
    stopped: bool,
}

impl<'a> Acquisition<'a> {
    pub(crate) fn start(cam: &'a mut GenICamera, mut cfg: StreamConfig) -> GenicamResult<Self> {
        cam.fill_payload_size(&mut cfg)?;
        // A subscription deeper than the pool could never fill up, so the
        // pool size is the natural capacity: the built-in subscriber never
        // drops a frame the pool managed to deliver.
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
        Ok(Self {
            cam,
            stream,
            frames,
            stopped: false,
        })
    }

    /// Block until a frame is available or `timeout` elapses.
    pub fn wait_for(&self, timeout: Duration) -> Option<Arc<Frame>> {
        self.frames.wait_for(timeout)
    }

    pub fn try_recv(&self) -> Option<Arc<Frame>> {
        self.frames.try_recv()
    }

    /// Drain and return every buffered frame.
    pub fn recv_all(&self) -> Vec<Arc<Frame>> {
        self.frames.recv_all()
    }

    /// Discard buffered frames.
    pub fn clear(&self) {
        self.frames.clear()
    }

    /// The built-in frame subscription; clone it to consume frames from
    /// another thread.
    pub fn frames(&self) -> &FrameChannel {
        &self.frames
    }

    /// An additional subscription with its own buffer. Unlike the built-in
    /// one it only sees frames completed after this call.
    pub fn subscribe(&self, capacity: usize) -> FrameChannel {
        self.stream.subscribe(capacity)
    }

    pub fn stream(&self) -> &StreamChannel {
        &self.stream
    }

    pub fn stats(&self) -> StreamStats {
        self.stream.stats()
    }

    /// The negotiated (or configured) SCPS packet size.
    pub fn packet_size(&self) -> u16 {
        self.stream.packet_size()
    }

    /// Where the device sends this stream (SCDA:SCP).
    pub fn local_addr(&self) -> SocketAddr {
        self.stream.local_addr()
    }

    /// The feature tree, for tuning mid-stream (e.g. `ExposureTime` or
    /// executing `TriggerSoftware`). Transport parameters stay locked
    /// (`TLParamsLocked`) while acquiring, so size and pixel format cannot
    /// change here — and lifecycle methods are deliberately out of reach.
    pub fn features(&mut self) -> super::Features<'_> {
        super::Features(self.cam)
    }

    /// Stop acquisition and unlock transport parameters, reporting errors.
    /// Returns the built-in frame channel: frames already buffered stay
    /// readable, and it disconnects once drained. Dropping the guard stops
    /// the same way, logging errors instead.
    pub fn stop(mut self) -> GenicamResult<FrameChannel> {
        let frames = self.frames.clone();
        self.stop_camera().map(|()| frames)
    }

    fn stop_camera(&mut self) -> GenicamResult<()> {
        if self.stopped {
            return Ok(());
        }
        self.stopped = true;
        let result = self.cam.execute("AcquisitionStop");
        if let Err(e) = self.cam.set_integer("TLParamsLocked", 0) {
            tracing::debug!("TLParamsLocked not cleared: {e}");
        }
        result
    }
}

impl Drop for Acquisition<'_> {
    fn drop(&mut self) {
        if let Err(e) = self.stop_camera() {
            tracing::warn!("AcquisitionStop on drop failed: {e}");
        }
        // StreamChannel's drop then closes the channel on the device
        // (SCP := 0) — after the stop, so the device is already quiet.
    }
}
