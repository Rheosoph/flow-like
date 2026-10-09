//! Completed frames and the buffer pool they draw from.
//!
//! [`FramePool`] is a flume channel of preallocated slots — the channel *is*
//! the lock-free free list. A slot travels: pool → stream worker (filling) →
//! [`Frame`] (delivered as `Arc<Frame>`) → back to the pool when the last
//! `Arc` drops. A slow consumer holding frames therefore degrades into pool
//! underruns at the worker, never blocking it.

use std::time::Instant;

use crate::gige::proto::gvsp::PixelFormat;

/// Per-packet reassembly bookkeeping, pooled with the buffer so steady-state
/// streaming does no per-frame allocation.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct PacketState {
    pub received: bool,
    pub resend_requested: bool,
    /// Set when the hole is first noticed; a resend goes out once `now`
    /// passes it.
    pub resend_deadline: Option<Instant>,
}

#[derive(Debug)]
pub(crate) struct BufSlot {
    pub data: Box<[u8]>,
    pub packets: Vec<PacketState>,
}

impl BufSlot {
    /// Prepare for a new frame of `n_packets`.
    pub fn reset(&mut self, n_packets: usize) {
        self.packets.clear();
        self.packets.resize(n_packets, PacketState::default());
    }
}

pub(crate) struct FramePool {
    rx: flume::Receiver<BufSlot>,
    tx: flume::Sender<BufSlot>,
}

impl FramePool {
    pub fn new(n_buffers: usize, payload_size: usize) -> Self {
        let (tx, rx) = flume::bounded(n_buffers);
        for _ in 0..n_buffers {
            let _ = tx.send(BufSlot {
                data: vec![0u8; payload_size].into_boxed_slice(),
                packets: Vec::new(),
            });
        }
        Self { rx, tx }
    }

    pub fn try_claim(&self) -> Option<BufSlot> {
        self.rx.try_recv().ok()
    }

    /// A sender that returns slots to this pool, for [`PooledBuf`].
    pub fn returner(&self) -> flume::Sender<BufSlot> {
        self.tx.clone()
    }
}

/// A pool slot owned by a delivered [`Frame`]; returns to the pool on drop.
#[derive(Debug)]
pub(crate) struct PooledBuf {
    slot: Option<BufSlot>,
    home: flume::Sender<BufSlot>,
}

impl PooledBuf {
    pub fn new(slot: BufSlot, home: flume::Sender<BufSlot>) -> Self {
        Self {
            slot: Some(slot),
            home,
        }
    }

    fn bytes(&self) -> &[u8] {
        self.slot.as_ref().map_or(&[], |s| &s.data)
    }
}

impl Drop for PooledBuf {
    fn drop(&mut self) {
        if let Some(slot) = self.slot.take() {
            let _ = self.home.try_send(slot);
        }
    }
}

/// How a frame ended.
#[cfg_attr(feature = "valuable", derive(valuable::Valuable))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "py", pyo3::pyclass(eq, eq_int, skip_from_py_object))]
pub enum FrameStatus {
    /// All packets received.
    Complete,
    /// Closed with holes left (resend disabled or another frame completed
    /// past it).
    MissingPackets,
    /// No packet arrived for the retention window.
    Timeout,
    /// The stream was stopped while the frame was filling.
    Aborted,
    /// A packet id outside the expected range was seen.
    WrongPacketId,
    /// The payload type cannot be reassembled by this receiver.
    PayloadUnsupported,
}

/// What the frame carries.
#[cfg_attr(feature = "valuable", derive(valuable::Valuable))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PayloadKind {
    Image {
        /// Chunk data follows the pixel data (GEV chunk extension).
        has_chunks: bool,
    },
    /// Pure chunk-data payload; `data()` is the raw chunk bytes.
    ChunkData,
    Unknown(u16),
}

/// One reassembled frame. Cheap to share as `Arc<Frame>`; the underlying
/// buffer re-enters the pool when the last clone drops.
#[derive(Debug)]
pub struct Frame {
    pub status: FrameStatus,
    pub frame_id: u64,
    pub payload: PayloadKind,
    pub pixel_format: PixelFormat,
    pub width: u32,
    pub height: u32,
    pub x_offset: u32,
    pub y_offset: u32,
    pub x_padding: u16,
    pub y_padding: u16,
    /// Device timestamp in ticks, and in nanoseconds when the tick frequency
    /// is known (0 otherwise).
    pub timestamp_ticks: u64,
    pub timestamp_ns: u64,
    /// Host CLOCK_REALTIME at leader reception, nanoseconds.
    pub system_timestamp_ns: u64,
    /// Payload bytes actually received.
    pub received_size: usize,
    /// End of the written payload region: the highest `offset + len` any
    /// received packet covered. Equal to `received_size` for complete
    /// frames; larger when interior packets are missing.
    pub(crate) data_end: usize,
    pub(crate) data: PooledBuf,
}

impl Frame {
    /// The payload bytes. For a [`FrameStatus::Complete`] image this is the
    /// full image (plus trailing chunk data when `payload` says so). For
    /// failed frames the slice extends to the last received packet — data
    /// past a hole is not cut off — and the holes themselves read as stale
    /// pool content; compare the length with
    /// [`received_size`](Self::received_size) to detect that.
    pub fn data(&self) -> &[u8] {
        let bytes = self.data.bytes();
        &bytes[..self.data_end.min(bytes.len())]
    }
}

impl serde::Serialize for Frame {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut s = serializer.serialize_struct("Frame", 15)?;
        s.serialize_field("status", &self.status)?;
        s.serialize_field("frame_id", &self.frame_id)?;
        s.serialize_field("payload", &self.payload)?;
        s.serialize_field("pixel_format", &self.pixel_format)?;
        s.serialize_field("width", &self.width)?;
        s.serialize_field("height", &self.height)?;
        s.serialize_field("x_offset", &self.x_offset)?;
        s.serialize_field("y_offset", &self.y_offset)?;
        s.serialize_field("x_padding", &self.x_padding)?;
        s.serialize_field("y_padding", &self.y_padding)?;
        s.serialize_field("timestamp_ticks", &self.timestamp_ticks)?;
        s.serialize_field("timestamp_ns", &self.timestamp_ns)?;
        s.serialize_field("system_timestamp_ns", &self.system_timestamp_ns)?;
        s.serialize_field("received_size", &self.received_size)?;
        let data = if self.status == FrameStatus::Complete {
            serde_bytes::Bytes::new(self.data())
        } else {
            serde_bytes::Bytes::new(&[])
        };
        s.serialize_field("data", data)?;
        s.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_claims_and_returns() {
        let pool = FramePool::new(2, 64);
        let a = pool.try_claim().unwrap();
        let _b = pool.try_claim().unwrap();
        assert!(pool.try_claim().is_none(), "pool exhausted");

        drop(PooledBuf::new(a, pool.returner()));
        let again = pool.try_claim();
        assert!(again.is_some(), "dropped buf re-pools");
    }

    #[test]
    fn slot_reset_clears_packet_state() {
        let pool = FramePool::new(1, 8);
        let mut slot = pool.try_claim().unwrap();
        slot.reset(4);
        slot.packets[2].received = true;
        slot.reset(6);
        assert_eq!(slot.packets.len(), 6);
        assert!(!slot.packets[2].received);
    }
}
