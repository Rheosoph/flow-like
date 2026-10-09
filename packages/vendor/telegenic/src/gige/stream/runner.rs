//! The per-channel GVSP receiver worker: reassembles frames from packets,
//! requests resends for holes, and fans out completed frames.

use std::io::ErrorKind;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use flume::{Receiver, TryRecvError};
use snare::mio::net::UdpSocket;
use snare::mio::{Events, Interest, Poll, Token, Waker};

use crate::error::CameraError;
use crate::gige::proto::gvcp::{self, GvcpStatus};
use crate::gige::proto::gvsp::{self, ContentType, GvspView, ImageLeader};
use crate::gige::stream::frame::{BufSlot, Frame, FramePool, FrameStatus, PayloadKind, PooledBuf};
use crate::gige::stream::{StreamConfig, StreamShared, StreamStats};
use crate::thread_util::ThreadHandle;

pub(crate) const TOK_SOCKET: Token = Token(0);
pub(crate) const TOK_WAKER: Token = Token(1);

/// Frames arriving this many ids late are dropped instead of reopened.
const DISCARD_LATE_FRAME_THRESHOLD: i64 = 100;
/// Stream-side GVCP id counter seed, clear of the control channel's range.
const RESEND_ID_SEED: u16 = 65300;

pub(crate) enum ToStreamWorker {
    Subscribe(flume::Sender<Arc<Frame>>),
    Shutdown,
}

struct FrameInFlight {
    frame_id: u64,
    extended_ids: bool,
    slot: BufSlot,
    n_packets: usize,
    /// Data bytes per payload packet. Starts as the SCPS-derived estimate;
    /// replaced by the observed size of payload packet 1 — devices may cap
    /// or align their block below the theoretical `scps - overhead`.
    block_size: usize,
    block_adopted: bool,
    trailer_seen: bool,
    /// Highest packet id such that 0..=it are all received; -1 initially.
    last_valid_packet: i64,
    received_size: usize,
    /// Highest `offset + len` written so far; the frame's data extent.
    data_end: usize,
    last_packet_time: Instant,
    leader: Option<ImageLeader>,
    system_timestamp_ns: u64,
    /// Set on protocol violations; blocks further data writes and decides
    /// the closing status.
    error: Option<FrameStatus>,
    resend_disabled: bool,
    resend_ratio_reached: bool,
    n_resend_requests: usize,
}

pub(crate) struct StreamRunner {
    socket: UdpSocket,
    device_gvcp_addr: SocketAddr,
    rx: Receiver<ToStreamWorker>,
    shared: Arc<StreamShared>,
    thread: ThreadHandle,
    cfg: StreamConfig,

    pool: FramePool,
    frames: Vec<FrameInFlight>,
    subscribers: Vec<flume::Sender<Arc<Frame>>>,
    shutdown_requested: bool,
    stats: StreamStats,
    scps_packet_size: usize,
    payload_size: usize,
    resend_enabled: bool,
    resend_id: u16,
    resend_buf: [u8; gvcp::RESEND_MAX_LEN],
    last_frame_id: u64,
    first_packet: bool,
    tick_frequency: u64,
}

impl StreamRunner {
    pub(crate) fn run(mut self, mut poll: Poll) {
        self.cfg.thread_cfg.apply_logged();
        let mut events = Events::with_capacity(16);
        let mut buf = [0u8; 0xffff];
        loop {
            if !self.thread.should_live() {
                break;
            }
            let timeout = if self.frames.is_empty() {
                Duration::from_millis(100)
            } else {
                self.cfg.packet_timeout
            };
            if let Err(e) = poll.poll(&mut events, Some(timeout)) {
                if e.kind() == ErrorKind::Interrupted {
                    continue;
                }
                tracing::error!("gvsp worker poll error: {e}");
                break;
            }
            // Commands before packets: a Subscribe sent before a frame's
            // datagrams must be registered before that frame can complete,
            // or the frame fans out to nobody and is lost.
            self.drain_commands();
            if self.shutdown_requested {
                break;
            }
            for ev in events.iter() {
                if ev.token() == TOK_SOCKET {
                    self.drain_socket(&mut buf);
                }
            }
            self.check_frame_completion(Instant::now(), None);
            self.publish_stats();
        }
        self.flush_frames();
        self.publish_stats();
        self.subscribers.clear();
        tracing::debug!("gvsp worker shutting down");
        self.thread.has_died();
    }

    fn publish_stats(&self) {
        *self.shared.stats.lock() = self.stats;
    }

    fn drain_commands(&mut self) {
        loop {
            match self.rx.try_recv() {
                Ok(ToStreamWorker::Subscribe(tx)) => self.subscribers.push(tx),
                Ok(ToStreamWorker::Shutdown) => self.shutdown_requested = true,
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => return,
            }
        }
    }

    fn drain_socket(&mut self, buf: &mut [u8]) {
        loop {
            match self.socket.recv_from(buf) {
                Ok((n, src)) => {
                    if src.ip() != self.device_gvcp_addr.ip() {
                        continue;
                    }
                    self.stats.packets += 1;
                    self.stats.bytes += n as u64;
                    self.process_packet(&buf[..n], Instant::now());
                }
                Err(ref e) if e.kind() == ErrorKind::WouldBlock => return,
                Err(ref e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) => {
                    tracing::warn!("gvsp recv error: {e}");
                    return;
                }
            }
        }
    }

    fn process_packet(&mut self, datagram: &[u8], now: Instant) {
        let Some(view) = GvspView::parse(datagram) else {
            self.stats.ignored_packets += 1;
            return;
        };
        if self.first_packet {
            self.last_frame_id = view.frame_id.wrapping_sub(1);
            self.first_packet = false;
        }
        let Some(index) = self.find_or_create_frame(&view, now) else {
            return;
        };

        if view.status.is_error() {
            if matches!(
                view.status,
                GvcpStatus::PACKET_UNAVAILABLE
                    | GvcpStatus::PACKET_REMOVED_FROM_MEMORY
                    | GvcpStatus::PACKET_AND_PREV_REMOVED_FROM_MEMORY
            ) {
                self.frames[index].resend_disabled = true;
                self.stats.resend_disabled += 1;
            }
            self.stats.error_packets += 1;
            return;
        }

        let packet_id = view.packet_id as usize;
        {
            let frame = &mut self.frames[index];
            if packet_id < frame.n_packets && frame.slot.packets[packet_id].received {
                self.stats.duplicated_packets += 1;
                return;
            }
            if packet_id < frame.n_packets {
                if frame.slot.packets[packet_id].resend_requested {
                    self.stats.resent_packets += 1;
                }
                frame.slot.packets[packet_id].received = true;
            }
            let mut i = frame.last_valid_packet + 1;
            while (i as usize) < frame.n_packets && frame.slot.packets[i as usize].received {
                i += 1;
            }
            frame.last_valid_packet = i - 1;
        }

        match view.content_type {
            ContentType::Leader => self.process_leader(index, &view),
            ContentType::Payload => self.process_payload(index, &view),
            ContentType::Trailer => self.process_trailer(index, &view),
            _ => {
                self.stats.ignored_packets += 1;
            }
        }
        self.missing_packet_check(index, view.packet_id, now);
        let frame_id = self.frames[index].frame_id;
        self.check_frame_completion(now, Some(frame_id));
    }

    fn find_or_create_frame(&mut self, view: &GvspView<'_>, now: Instant) -> Option<usize> {
        if let Some(i) = self.frames.iter().position(|f| f.frame_id == view.frame_id) {
            self.frames[i].last_packet_time = now;
            return Some(i);
        }

        let inc = if view.extended_ids {
            let mut inc = view.frame_id as i64 - self.last_frame_id as i64;
            if view.frame_id as i64 > 0 && (self.last_frame_id as i64) < 0 {
                inc -= 1;
            }
            inc
        } else {
            let mut inc = i64::from(view.frame_id as i16) - i64::from(self.last_frame_id as i16);
            if view.frame_id as i16 > 0 && (self.last_frame_id as i16) < 0 {
                inc -= 1;
            }
            inc
        };
        if inc < 1 && inc > -DISCARD_LATE_FRAME_THRESHOLD {
            tracing::trace!(
                frame_id = view.frame_id,
                last = self.last_frame_id,
                "discarding late frame"
            );
            self.stats.ignored_packets += 1;
            return None;
        }

        let n_packets = self.compute_n_expected_packets(view);
        if n_packets == 0 {
            // Unsupported payload (multipart/H264/GenDC/...) or an
            // unparsable first packet: count and drop the whole frame.
            self.stats.unsupported_frames += 1;
            self.stats.ignored_packets += 1;
            self.last_frame_id = view.frame_id;
            return None;
        }

        let Some(mut slot) = self.pool.try_claim() else {
            // Advance the id so the frame's remaining packets are discarded
            // as late instead of each counting another underrun.
            self.stats.underruns += 1;
            self.last_frame_id = view.frame_id;
            return None;
        };
        slot.reset(n_packets);

        if inc > 1 {
            tracing::trace!(
                skipped = inc - 1,
                after = self.last_frame_id,
                "frame ids skipped"
            );
            self.stats.missing_frames += inc as u64 - 1;
        }
        self.last_frame_id = view.frame_id;
        let block_size = self
            .scps_packet_size
            .saturating_sub(gvsp::packet_protocol_overhead(view.extended_ids));
        self.frames.push(FrameInFlight {
            frame_id: view.frame_id,
            extended_ids: view.extended_ids,
            slot,
            n_packets,
            block_size,
            block_adopted: false,
            trailer_seen: false,
            last_valid_packet: -1,
            received_size: 0,
            data_end: 0,
            last_packet_time: now,
            leader: None,
            system_timestamp_ns: 0,
            error: None,
            resend_disabled: false,
            resend_ratio_reached: false,
            n_resend_requests: 0,
        });
        Some(self.frames.len() - 1)
    }

    /// Expected packets for a frame, judged from whichever packet arrives
    /// first (the leader may be lost). Mirrors `_compute_n_expected_packets`.
    fn compute_n_expected_packets(&self, view: &GvspView<'_>) -> usize {
        let block_size = self
            .scps_packet_size
            .saturating_sub(gvsp::packet_protocol_overhead(view.extended_ids));
        if block_size == 0 {
            return 0;
        }
        let payload_packets = self.payload_size.div_ceil(block_size);
        match view.content_type {
            ContentType::Leader => {
                let payload_type = ImageLeader::parse(view.data).map(|l| l.payload_type);
                match payload_type {
                    Some(
                        gvsp::PAYLOAD_TYPE_IMAGE
                        | gvsp::PAYLOAD_TYPE_CHUNK_DATA
                        | gvsp::PAYLOAD_TYPE_EXTENDED_CHUNK_DATA,
                    ) => payload_packets + 2,
                    _ => 0,
                }
            }
            ContentType::Payload => payload_packets + 2,
            ContentType::Trailer => view.packet_id as usize + 1,
            ContentType::AllIn => 1,
            _ => 0,
        }
    }

    fn process_leader(&mut self, index: usize, view: &GvspView<'_>) {
        let frame = &mut self.frames[index];
        if frame.error.is_some() {
            return;
        }
        if view.packet_id != 0 {
            frame.error = Some(FrameStatus::WrongPacketId);
            return;
        }
        frame.system_timestamp_ns = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64);
        frame.leader = ImageLeader::parse(view.data);
    }

    fn process_payload(&mut self, index: usize, view: &GvspView<'_>) {
        let frame = &mut self.frames[index];
        if frame.error.is_some() {
            return;
        }
        let packet_id = view.packet_id as usize;

        // Adopt the device's actual block size from payload packet 1; it
        // may be capped or aligned below the SCPS-derived estimate (real
        // devices send e.g. 1448-byte blocks at SCPS 1500).
        if packet_id == 1 && !frame.block_adopted && !view.data.is_empty() {
            frame.block_adopted = true;
            if view.data.len() != frame.block_size {
                frame.block_size = view.data.len();
                if !frame.trailer_seen {
                    frame.n_packets = self.payload_size.div_ceil(frame.block_size) + 2;
                    frame
                        .slot
                        .packets
                        .resize(frame.n_packets, Default::default());
                }
            }
        }

        if packet_id < 1 || packet_id > frame.n_packets.saturating_sub(2) {
            tracing::trace!(
                "payload packet id {packet_id} outside 1..={} (frame {}, {} data bytes, ext={})",
                frame.n_packets.saturating_sub(2),
                frame.frame_id,
                view.data.len(),
                view.extended_ids,
            );
            frame.error = Some(FrameStatus::WrongPacketId);
            return;
        }
        let offset = (packet_id - 1) * frame.block_size;
        let mut data = view.data;
        let capacity = frame.slot.data.len();
        if offset + data.len() > capacity {
            // The final payload packet may legally be padded past the
            // payload size; anything else is a real mismatch.
            if packet_id != frame.n_packets - 2 {
                self.stats.size_mismatch_errors += 1;
            }
            if offset >= capacity {
                return;
            }
            data = &data[..capacity - offset];
        }
        frame.slot.data[offset..offset + data.len()].copy_from_slice(data);
        frame.received_size += data.len();
        frame.data_end = frame.data_end.max(offset + data.len());
    }

    fn process_trailer(&mut self, index: usize, view: &GvspView<'_>) {
        let frame = &mut self.frames[index];
        frame.trailer_seen = true;
        if frame.error.is_some() {
            return;
        }
        let packet_id = view.packet_id as usize;
        if packet_id > frame.n_packets - 1 {
            frame.error = Some(FrameStatus::WrongPacketId);
            return;
        }
        // An early trailer means the actual payload is smaller than the
        // buffer; shrink the expectation.
        if frame.n_packets != packet_id + 1 {
            frame.n_packets = packet_id + 1;
        }
    }

    /// Port of `_missing_packet_check`: walk the hole span behind
    /// `packet_id`, arm per-packet deadlines on first sight, and coalesce
    /// expired holes into ranged resend requests.
    fn missing_packet_check(&mut self, index: usize, packet_id: u32, now: Instant) {
        let frame = &mut self.frames[index];
        if !self.resend_enabled || frame.resend_disabled || frame.resend_ratio_reached {
            return;
        }
        let max_requests = (frame.n_packets as f64 * self.cfg.packet_request_ratio) as usize;
        if max_requests == 0 {
            return;
        }
        let packet_id = packet_id as usize;
        if packet_id >= frame.n_packets {
            return;
        }

        let mut first_missing: Option<usize> = None;
        let mut i = (frame.last_valid_packet + 1).max(0) as usize;
        let mut ranges: Vec<(usize, usize)> = Vec::new();
        while i <= packet_id + 1 {
            let need_resend = if i <= packet_id && !frame.slot.packets[i].received {
                let deadline = frame.slot.packets[i]
                    .resend_deadline
                    .get_or_insert(now + self.cfg.initial_packet_timeout);
                now > *deadline
            } else {
                false
            };

            if need_resend && first_missing.is_none() {
                first_missing = Some(i);
            }
            if (i > packet_id || !need_resend)
                && let Some(first) = first_missing.take()
            {
                let last = i - 1;
                let n_missing = last - first + 1;
                if frame.n_resend_requests + n_missing > max_requests {
                    frame.n_resend_requests += n_missing;
                    frame.resend_ratio_reached = true;
                    self.stats.resend_ratio_reached += 1;
                    return;
                }
                frame.n_resend_requests += n_missing;
                for p in &mut frame.slot.packets[first..=last] {
                    p.resend_requested = true;
                    p.resend_deadline = Some(now + self.cfg.packet_timeout);
                }
                ranges.push((first, last));
            }
            i += 1;
        }

        let (frame_id, extended_ids) = (frame.frame_id, frame.extended_ids);
        for (first, last) in ranges {
            self.send_resend_request(frame_id, first as u32, last as u32, extended_ids);
            self.stats.resend_requests += (last - first + 1) as u64;
        }
    }

    fn send_resend_request(&mut self, frame_id: u64, first: u32, last: u32, extended_ids: bool) {
        tracing::trace!(frame_id, first, last, "requesting packet resend");
        // Private wrap, back to the seed: resend ids must stay out of the
        // control worker's 1..=0xfffe range.
        self.resend_id = if self.resend_id == u16::MAX {
            RESEND_ID_SEED
        } else {
            self.resend_id + 1
        };
        let len = gvcp::encode_packet_resend(
            &mut self.resend_buf,
            frame_id,
            first,
            last,
            extended_ids,
            self.resend_id,
        );
        if let Err(e) = self
            .socket
            .send_to(&self.resend_buf[..len], self.device_gvcp_addr)
        {
            tracing::trace!("resend request send failed: {e}");
        }
    }

    /// Port of `_check_frame_completion`: frames close strictly head-of-line.
    fn check_frame_completion(&mut self, now: Instant, current_frame_id: Option<u64>) {
        let mut index = 0;
        let mut can_close = true;
        while index < self.frames.len() {
            let frame = &self.frames[index];

            if can_close && frame.last_valid_packet == frame.n_packets as i64 - 1 {
                let status = frame.error.unwrap_or(FrameStatus::Complete);
                self.close_frame(index, status);
                continue;
            }
            // Completeness is tested first: a head frame whose last packet
            // arrived after a newer frame opened is still a good frame.
            if can_close && !self.resend_enabled && self.frames.len() > index + 1 {
                self.close_frame(index, FrameStatus::MissingPackets);
                continue;
            }
            // Never time out the newest frame whose only packet so far is
            // the leader — some devices send the leader at trigger time,
            // long before the data.
            if can_close
                && (frame.frame_id != self.last_frame_id || frame.last_valid_packet != 0)
                && now.duration_since(frame.last_packet_time) >= self.cfg.frame_retention
            {
                let status = frame.error.unwrap_or(FrameStatus::Timeout);
                self.close_frame(index, status);
                continue;
            }

            can_close = false;
            if current_frame_id != Some(frame.frame_id)
                && now.duration_since(frame.last_packet_time) >= self.cfg.packet_timeout
            {
                let last = self.frames[index].n_packets as u32 - 1;
                self.missing_packet_check(index, last, now);
            }
            index += 1;
        }
    }

    fn close_frame(&mut self, index: usize, status: FrameStatus) {
        // A Subscribe can land while drain_socket is mid-pass; pick it up
        // here so a subscription sent before this frame's packets arrived
        // never misses the frame.
        self.drain_commands();
        let frame = self.frames.remove(index);
        if status != FrameStatus::Complete {
            tracing::trace!(
                frame_id = frame.frame_id,
                ?status,
                "frame closed incomplete"
            );
        }
        match status {
            FrameStatus::Complete => self.stats.completed_frames += 1,
            FrameStatus::Timeout => {
                self.stats.timed_out_frames += 1;
                self.stats.failed_frames += 1;
            }
            FrameStatus::Aborted => self.stats.aborted_frames += 1,
            _ => self.stats.failed_frames += 1,
        }
        if status != FrameStatus::Complete && status != FrameStatus::Aborted {
            self.stats.missing_packets +=
                (frame.n_packets as i64 - (frame.last_valid_packet + 1)).max(0) as u64;
        }

        let leader = frame.leader;
        let payload = match leader.map(|l| l.payload_type) {
            Some(gvsp::PAYLOAD_TYPE_IMAGE | gvsp::PAYLOAD_TYPE_EXTENDED_CHUNK_DATA) => {
                PayloadKind::Image {
                    has_chunks: leader.is_some_and(|l| l.has_chunks),
                }
            }
            Some(gvsp::PAYLOAD_TYPE_CHUNK_DATA) => PayloadKind::ChunkData,
            Some(other) => PayloadKind::Unknown(other),
            None => PayloadKind::Unknown(0),
        };
        let timestamp_ticks = leader.map_or(0, |l| l.timestamp_ticks);
        let timestamp_ns = if self.tick_frequency != 0 {
            gvsp::timestamp_to_ns(timestamp_ticks, self.tick_frequency)
        } else {
            frame.system_timestamp_ns
        };
        let out = Frame {
            status,
            frame_id: frame.frame_id,
            payload,
            pixel_format: leader.map(|l| l.pixel_format).unwrap_or_default(),
            width: leader.map_or(0, |l| l.width),
            height: leader.map_or(0, |l| l.height),
            x_offset: leader.map_or(0, |l| l.x_offset),
            y_offset: leader.map_or(0, |l| l.y_offset),
            x_padding: leader.map_or(0, |l| l.x_padding),
            y_padding: leader.map_or(0, |l| l.y_padding),
            timestamp_ticks,
            timestamp_ns,
            system_timestamp_ns: frame.system_timestamp_ns,
            received_size: frame.received_size,
            data_end: frame.data_end,
            data: PooledBuf::new(frame.slot, self.pool.returner()),
        };
        let out = Arc::new(out);
        // Publish before fan-out so a subscriber that wakes on this frame
        // already sees it reflected in the stats snapshot.
        self.publish_stats();
        let mut dropped = 0u64;
        self.subscribers
            .retain(|tx| match tx.try_send(out.clone()) {
                Ok(()) => true,
                Err(flume::TrySendError::Full(_)) => {
                    dropped += 1;
                    true
                }
                Err(flume::TrySendError::Disconnected(_)) => false,
            });
        if dropped > 0 {
            self.stats.frames_dropped += dropped;
            self.publish_stats();
        }
    }

    fn flush_frames(&mut self) {
        while !self.frames.is_empty() {
            self.close_frame(0, FrameStatus::Aborted);
        }
    }
}

/// Negotiated link parameters the worker needs alongside the user config.
pub(crate) struct LinkParams {
    pub device_gvcp_addr: SocketAddr,
    pub scps_packet_size: u16,
    /// Resolved frame buffer size (`StreamConfig::payload_size` is optional
    /// at the API surface; it is mandatory by the time a worker spawns).
    pub payload_size: usize,
    pub resend_enabled: bool,
    pub tick_frequency: u64,
}

/// Take a prepared (bound, buffer-sized, negotiated) std socket and launch
/// the stream worker thread.
pub(crate) fn spawn(
    std_socket: snare::net::UdpSocket,
    link: LinkParams,
    rx: Receiver<ToStreamWorker>,
    shared: Arc<StreamShared>,
    cfg: StreamConfig,
) -> Result<ThreadHandle, CameraError> {
    std_socket
        .set_nonblocking(true)
        .map_err(|e| CameraError::Spawn(e.to_string()))?;
    let mut socket = UdpSocket::from_std(std_socket);

    let poll = Poll::new().map_err(|e| CameraError::Spawn(e.to_string()))?;
    poll.registry()
        .register(&mut socket, TOK_SOCKET, Interest::READABLE)
        .map_err(|e| CameraError::Spawn(e.to_string()))?;
    let waker = Arc::new(
        Waker::new(poll.registry(), TOK_WAKER).map_err(|e| CameraError::Spawn(e.to_string()))?,
    );

    let mut thread = ThreadHandle::new();
    thread.set_waker(waker);
    let thread_for_worker = thread.to_pass_in();

    let pool = FramePool::new(cfg.n_buffers, link.payload_size);
    let join = snare::thread::Builder::new()
        .name(format!("telegenic-gvsp{}", cfg.channel))
        .spawn(move || {
            let runner = StreamRunner {
                socket,
                device_gvcp_addr: link.device_gvcp_addr,
                rx,
                shared,
                thread: thread_for_worker,
                pool,
                frames: Vec::with_capacity(4),
                subscribers: Vec::new(),
                shutdown_requested: false,
                stats: StreamStats::default(),
                scps_packet_size: usize::from(link.scps_packet_size),
                payload_size: link.payload_size,
                resend_enabled: link.resend_enabled,
                resend_id: RESEND_ID_SEED,
                resend_buf: [0u8; gvcp::RESEND_MAX_LEN],
                last_frame_id: 0,
                first_packet: true,
                tick_frequency: link.tick_frequency,
                cfg,
            };
            runner.run(poll);
        })
        .map_err(|e| CameraError::Spawn(e.to_string()))?;
    thread.set_handle(join);

    Ok(thread)
}
