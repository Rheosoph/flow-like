//! On-demand single-frame capture without continuous streaming.
//!
//! GVSP only carries traffic while a frame is in flight, so an open but
//! quiet stream channel costs no link bandwidth. [`SnapshotSession`] exploits
//! that: it opens the channel once (paying socket setup and packet-size
//! negotiation up front) and leaves the camera idle, arming it per
//! [`snap`](SnapshotSession::snap) via `AcquisitionMode = SingleFrame` when
//! the device offers it.

use std::sync::Arc;
use std::time::Duration;

use crate::error::{GenicamError, GenicamResult};
use crate::gige::stream::{Frame, FrameChannel, StreamChannel, StreamConfig};

use super::GenICamera;

/// An open stream channel dedicated to single-frame capture.
///
/// The camera transmits only while a [`snap`](Self::snap) is in flight;
/// between captures the session holds the channel open at zero bandwidth.
/// On drop it unlocks `TLParamsLocked`, restores the original
/// `AcquisitionMode`, and closes the stream channel on the device.
#[derive(Debug)]
pub struct SnapshotSession<'a> {
    cam: &'a mut GenICamera,
    stream: StreamChannel,
    frames: FrameChannel,
    /// The device is in `SingleFrame` mode and stops itself after one frame;
    /// otherwise every snap issues an explicit `AcquisitionStop`.
    auto_stops: bool,
    restore_mode: Option<String>,
}

impl<'a> SnapshotSession<'a> {
    pub(crate) fn open(cam: &'a mut GenICamera, mut cfg: StreamConfig) -> GenicamResult<Self> {
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
            cam,
            stream,
            frames,
            auto_stops,
            restore_mode,
        })
    }

    /// Arm the camera, wait for the resulting frame, and leave the camera
    /// idle again.
    ///
    /// `timeout` must cover exposure plus transfer — and the trigger wait,
    /// when a trigger is configured. The frame is returned with whatever
    /// [`status`](Frame::status) it completed with; check it before trusting
    /// the pixels.
    pub fn snap(&mut self, timeout: Duration) -> GenicamResult<Arc<Frame>> {
        self.frames.clear();
        self.cam.execute("AcquisitionStart")?;
        let frame = self.frames.wait_for(timeout);
        if (!self.auto_stops || frame.is_none())
            && let Err(e) = self.cam.execute("AcquisitionStop")
        {
            tracing::warn!("AcquisitionStop after snap failed: {e}");
        }
        frame.ok_or(GenicamError::FrameTimeout)
    }

    pub fn stream(&self) -> &StreamChannel {
        &self.stream
    }

    /// The feature tree, for tuning between snaps (e.g. `ExposureTime`).
    /// Transport parameters stay locked (`TLParamsLocked`) for the session's
    /// lifetime, so size and pixel format cannot change here — and lifecycle
    /// methods are deliberately out of reach.
    pub fn features(&mut self) -> super::Features<'_> {
        super::Features(self.cam)
    }
}

impl Drop for SnapshotSession<'_> {
    fn drop(&mut self) {
        if let Err(e) = self.cam.set_integer("TLParamsLocked", 0) {
            tracing::debug!("TLParamsLocked not cleared: {e}");
        }
        if let Some(mode) = self.restore_mode.take()
            && let Err(e) = self.cam.set_enum("AcquisitionMode", &mode)
        {
            tracing::debug!("AcquisitionMode not restored: {e}");
        }
        // StreamChannel's drop closes the channel on the device (SCP := 0).
    }
}
