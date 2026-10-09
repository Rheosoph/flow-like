//! The GVCP control worker: owns the control socket, serializes transactions
//! (one in flight at a time, as devices commonly require), matches
//! acknowledges by packet id, keeps control alive via heartbeat, and fans out
//! device-initiated events.

use std::collections::VecDeque;
use std::io::ErrorKind;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use flume::{Receiver, TryRecvError};
use snare::mio::net::UdpSocket;
use snare::mio::{Events, Interest, Poll, Token, Waker};

use crate::error::CameraError;
use crate::gige::proto::bootstrap;
use crate::gige::proto::gvcp::{self, Ack};
use crate::gige::{GigeConfig, GvcpEvent, Shared};
use crate::handle::ResponseHandle;
use crate::thread_util::ThreadHandle;

pub(crate) const TOK_SOCKET: Token = Token(0);
pub(crate) const TOK_WAKER: Token = Token(1);

const RECV_BUF: usize = 0xffff;

/// Messages from `GigECamera` clones to the worker.
pub(crate) enum ToWorker {
    ReadReg(u32, ResponseHandle<u32>),
    ReadRegs(Vec<u32>, ResponseHandle<Vec<u32>>),
    WriteRegs(Vec<(u32, u32)>, ResponseHandle<()>),
    ReadMem {
        addr: u32,
        len: u32,
        handle: ResponseHandle<Vec<u8>>,
    },
    WriteMem {
        addr: u32,
        data: Vec<u8>,
        handle: ResponseHandle<()>,
    },
    SubscribeEvents(flume::Sender<GvcpEvent>),
    Shutdown,
}

enum Op {
    ReadReg(ResponseHandle<u32>),
    ReadRegs {
        handle: ResponseHandle<Vec<u32>>,
        count: usize,
    },
    WriteRegs(ResponseHandle<()>),
    ReadMem {
        handle: ResponseHandle<Vec<u8>>,
        acc: Vec<u8>,
        want: usize,
        next_addr: u32,
    },
    WriteMem {
        handle: ResponseHandle<()>,
        data: Vec<u8>,
        offset: usize,
        base_addr: u32,
    },
    Heartbeat,
}

impl Op {
    fn fail(self, err: CameraError) {
        match self {
            Op::ReadReg(h) => h.fail(err),
            Op::ReadRegs { handle, .. } => handle.fail(err),
            Op::WriteRegs(h) => h.fail(err),
            Op::ReadMem { handle, .. } => handle.fail(err),
            Op::WriteMem { handle, .. } => handle.fail(err),
            Op::Heartbeat => {}
        }
    }

    fn expected_ack(&self) -> u16 {
        match self {
            Op::ReadReg(_) | Op::ReadRegs { .. } | Op::Heartbeat => gvcp::READ_REGISTER_ACK,
            Op::WriteRegs(_) => gvcp::WRITE_REGISTER_ACK,
            Op::ReadMem { .. } => gvcp::READ_MEMORY_ACK,
            Op::WriteMem { .. } => gvcp::WRITE_MEMORY_ACK,
        }
    }
}

struct Inflight {
    sent: Vec<u8>,
    id: u16,
    deadline: Instant,
    tries_left: u8,
    op: Op,
}

pub(crate) struct Runner {
    socket: UdpSocket,
    device_addr: SocketAddr,
    rx: Receiver<ToWorker>,
    shared: Arc<Shared>,
    thread: ThreadHandle,
    cfg: GigeConfig,

    queue: VecDeque<Op>,
    /// Pre-encoded datagram for each queued op, kept in lockstep with `queue`.
    queued_payloads: VecDeque<PendingSend>,
    inflight: Option<Inflight>,
    next_id: u16,
    event_txs: Vec<flume::Sender<GvcpEvent>>,
    heartbeat_period: Duration,
    heartbeat_due: Instant,
    control_lost: bool,
}

/// What to encode when an op reaches the head of the queue.
enum PendingSend {
    ReadRegs(Vec<u32>),
    WriteRegs(Vec<(u32, u32)>),
    ReadMemChunk,
    WriteMemChunk,
}

impl Runner {
    pub(crate) fn new(
        socket: UdpSocket,
        rx: Receiver<ToWorker>,
        shared: Arc<Shared>,
        thread: ThreadHandle,
        cfg: GigeConfig,
    ) -> Self {
        let heartbeat_period = heartbeat_period(&cfg);
        Self {
            socket,
            device_addr: cfg.addr,
            rx,
            shared,
            thread,
            cfg,
            queue: VecDeque::new(),
            queued_payloads: VecDeque::new(),
            inflight: None,
            next_id: 0,
            event_txs: Vec::new(),
            heartbeat_period,
            heartbeat_due: Instant::now() + heartbeat_period,
            control_lost: false,
        }
    }

    pub(crate) fn run(mut self, mut poll: Poll) {
        self.cfg.thread_cfg.apply_logged();
        let mut events = Events::with_capacity(16);
        let mut buf = [0u8; RECV_BUF];
        while self.thread.should_live() && !self.control_lost {
            if let Err(e) = poll.poll(&mut events, Some(Duration::from_millis(10))) {
                if e.kind() == ErrorKind::Interrupted {
                    continue;
                }
                tracing::error!("gvcp worker poll error: {e}");
                break;
            }
            for ev in events.iter() {
                if ev.token() == TOK_SOCKET {
                    self.drain_socket(&mut buf);
                }
            }
            if self.drain_commands() {
                break;
            }
            self.check_inflight_deadline();
            self.check_heartbeat();
            self.pump();
        }
        self.shutdown();
    }

    fn drain_socket(&mut self, buf: &mut [u8]) {
        loop {
            match self.socket.recv_from(buf) {
                Ok((n, src)) => {
                    // Events may come from a device source port other than
                    // 3956, so filter on IP only.
                    if src.ip() != self.device_addr.ip() {
                        continue;
                    }
                    self.on_datagram(&buf[..n], src);
                }
                Err(ref e) if e.kind() == ErrorKind::WouldBlock => return,
                Err(ref e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) => {
                    tracing::warn!("gvcp recv error: {e}");
                    return;
                }
            }
        }
    }

    fn on_datagram(&mut self, datagram: &[u8], src: SocketAddr) {
        if gvcp::is_cmd(datagram) {
            self.on_event(datagram, src);
            return;
        }
        let Some(ack) = Ack::parse(datagram) else {
            tracing::trace!("malformed ack ({} bytes)", datagram.len());
            return;
        };
        let Some(inflight) = &mut self.inflight else {
            self.shared.stats.lock().unsolicited += 1;
            return;
        };
        if ack.ack_id != inflight.id {
            self.shared.stats.lock().unsolicited += 1;
            return;
        }
        if let Some(ms) = ack.pending_ack_timeout_ms() {
            inflight.deadline = Instant::now() + Duration::from_millis(u64::from(ms));
            self.shared.stats.lock().pending_acks += 1;
            return;
        }
        let Some(inflight) = self.inflight.take() else {
            return;
        };
        self.on_ack(inflight, ack);
    }

    fn on_ack(&mut self, inflight: Inflight, ack: Ack<'_>) {
        self.shared.stats.lock().acks += 1;
        if ack.status.is_error() {
            self.shared.stats.lock().naks += 1;
            if matches!(inflight.op, Op::Heartbeat) {
                tracing::warn!("heartbeat rejected with {}, control lost", ack.status);
                self.control_lost = true;
                return;
            }
            let command = inflight.op.expected_ack().wrapping_sub(1);
            inflight.op.fail(CameraError::Nak {
                command,
                status: ack.status,
            });
            return;
        }
        let expected_ack = inflight.op.expected_ack();
        if ack.answer != expected_ack {
            inflight.op.fail(CameraError::Protocol(format!(
                "expected ack {expected_ack:#06x}, got {:#06x}",
                ack.answer
            )));
            return;
        }
        match inflight.op {
            Op::ReadReg(handle) => match ack.register_values().next() {
                Some(v) => handle.fulfill(Ok(v)),
                None => handle.fail(CameraError::Protocol("empty read register ack".into())),
            },
            Op::ReadRegs { handle, count } => {
                let values: Vec<u32> = ack.register_values().collect();
                if values.len() == count {
                    handle.fulfill(Ok(values));
                } else {
                    handle.fail(CameraError::Protocol(format!(
                        "read register ack carried {} values, expected {count}",
                        values.len()
                    )));
                }
            }
            Op::WriteRegs(handle) => handle.fulfill(Ok(())),
            Op::ReadMem {
                handle,
                mut acc,
                want,
                next_addr,
            } => {
                let Some(data) = ack.payload.get(4..) else {
                    handle.fail(CameraError::Protocol("short read memory ack".into()));
                    return;
                };
                let take = data.len().min(want - acc.len());
                acc.extend_from_slice(&data[..take]);
                if acc.len() >= want {
                    handle.fulfill(Ok(acc));
                } else if data.is_empty() {
                    handle.fail(CameraError::Protocol("empty read memory ack".into()));
                } else {
                    let next_addr = next_addr.wrapping_add(take as u32);
                    let op = Op::ReadMem {
                        handle,
                        acc,
                        want,
                        next_addr,
                    };
                    self.continue_op(op, PendingSend::ReadMemChunk);
                }
            }
            Op::WriteMem {
                handle,
                data,
                mut offset,
                base_addr,
            } => {
                offset += chunk_len(data.len() - offset);
                if offset >= data.len() {
                    handle.fulfill(Ok(()));
                } else {
                    let op = Op::WriteMem {
                        handle,
                        data,
                        offset,
                        base_addr,
                    };
                    self.continue_op(op, PendingSend::WriteMemChunk);
                }
            }
            Op::Heartbeat => {
                self.shared.stats.lock().heartbeats += 1;
                if ack
                    .register_values()
                    .next()
                    .is_none_or(|ccp| ccp & bootstrap::CCP_CONTROL == 0)
                {
                    tracing::warn!("device control was lost (CCP cleared)");
                    self.control_lost = true;
                }
            }
        }
    }

    fn on_event(&mut self, datagram: &[u8], src: SocketAddr) {
        let Some(cmd) = gvcp::Cmd::parse(datagram) else {
            tracing::trace!("malformed inbound command");
            return;
        };
        if cmd.command != gvcp::EVENT_CMD && cmd.command != gvcp::EVENTDATA_CMD {
            tracing::trace!("unexpected inbound command {:#06x}", cmd.command);
            return;
        }
        if cmd.flags & gvcp::FLAG_ACK_REQUIRED != 0 {
            // Acknowledge to the message channel's source socket, not the
            // device's GVCP port.
            let ack = gvcp::encode_event_ack(cmd.command, cmd.req_id);
            if let Err(e) = self.socket.send_to(&ack, src) {
                tracing::warn!("event ack send failed: {e}");
            }
        }
        self.shared.stats.lock().events += 1;
        let event = GvcpEvent::parse(cmd.command, cmd.payload);
        self.event_txs
            .retain(|tx| match tx.try_send(event.clone()) {
                Ok(()) => true,
                Err(flume::TrySendError::Full(_)) => {
                    tracing::trace!("event channel full, event dropped");
                    true
                }
                Err(flume::TrySendError::Disconnected(_)) => false,
            });
    }

    fn drain_commands(&mut self) -> bool {
        loop {
            match self.rx.try_recv() {
                Ok(ToWorker::ReadReg(addr, handle)) => {
                    self.enqueue(Op::ReadReg(handle), PendingSend::ReadRegs(vec![addr]));
                }
                Ok(ToWorker::ReadRegs(addrs, handle)) => {
                    let count = addrs.len();
                    self.enqueue(Op::ReadRegs { handle, count }, PendingSend::ReadRegs(addrs));
                }
                Ok(ToWorker::WriteRegs(pairs, handle)) => {
                    self.enqueue(Op::WriteRegs(handle), PendingSend::WriteRegs(pairs));
                }
                Ok(ToWorker::ReadMem { addr, len, handle }) => {
                    let op = Op::ReadMem {
                        handle,
                        acc: Vec::with_capacity(len as usize),
                        want: len as usize,
                        next_addr: addr,
                    };
                    self.enqueue(op, PendingSend::ReadMemChunk);
                }
                Ok(ToWorker::WriteMem { addr, data, handle }) => {
                    let op = Op::WriteMem {
                        handle,
                        data,
                        offset: 0,
                        base_addr: addr,
                    };
                    self.enqueue(op, PendingSend::WriteMemChunk);
                }
                Ok(ToWorker::SubscribeEvents(tx)) => self.event_txs.push(tx),
                Ok(ToWorker::Shutdown) => return true,
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => return false,
            }
        }
    }

    fn enqueue(&mut self, op: Op, send: PendingSend) {
        self.queue.push_back(op);
        self.queued_payloads.push_back(send);
    }

    /// Send the next chunk of a multi-transaction memory op, letting a due
    /// heartbeat cut in at the chunk boundary — a long transfer (e.g. the
    /// GenICam XML fetch) must not hold the device's heartbeat window shut.
    fn continue_op(&mut self, op: Op, send: PendingSend) {
        if matches!(self.queue.front(), Some(Op::Heartbeat)) {
            self.queue.insert(1, op);
            self.queued_payloads.insert(1, send);
        } else {
            self.send_op(op, send);
        }
    }

    /// Start the next queued op if nothing is in flight.
    fn pump(&mut self) {
        if self.inflight.is_some() {
            return;
        }
        let (Some(op), Some(send)) = (self.queue.pop_front(), self.queued_payloads.pop_front())
        else {
            return;
        };
        self.send_op(op, send);
    }

    fn send_op(&mut self, op: Op, send: PendingSend) {
        self.next_id = gvcp::next_id(self.next_id);
        let id = self.next_id;
        let datagram = match (&op, send) {
            (_, PendingSend::ReadRegs(addrs)) => gvcp::encode_read_reg(&addrs, id),
            (_, PendingSend::WriteRegs(pairs)) => gvcp::encode_write_reg(&pairs, id),
            (
                Op::ReadMem {
                    acc,
                    want,
                    next_addr,
                    ..
                },
                PendingSend::ReadMemChunk,
            ) => {
                let remaining = want - acc.len();
                let count = chunk_len(remaining.next_multiple_of(4));
                gvcp::encode_read_mem(*next_addr, count as u16, id).to_vec()
            }
            (
                Op::WriteMem {
                    data,
                    offset,
                    base_addr,
                    ..
                },
                PendingSend::WriteMemChunk,
            ) => {
                let take = chunk_len(data.len() - offset);
                gvcp::encode_write_mem(
                    base_addr.wrapping_add(*offset as u32),
                    &data[*offset..offset + take],
                    id,
                )
            }
            (_, PendingSend::ReadMemChunk | PendingSend::WriteMemChunk) => {
                op.fail(CameraError::Protocol("internal op/payload mismatch".into()));
                return;
            }
        };
        if let Err(e) = self.socket.send_to(&datagram, self.device_addr) {
            op.fail(CameraError::Io(e));
            return;
        }
        self.shared.stats.lock().commands += 1;
        self.inflight = Some(Inflight {
            sent: datagram,
            id,
            deadline: Instant::now() + self.cfg.gvcp_timeout,
            tries_left: self.cfg.retries,
            op,
        });
    }

    fn check_inflight_deadline(&mut self) {
        let Some(inflight) = &mut self.inflight else {
            return;
        };
        if Instant::now() < inflight.deadline {
            return;
        }
        if inflight.tries_left > 0 {
            inflight.tries_left -= 1;
            inflight.deadline = Instant::now() + self.cfg.gvcp_timeout;
            self.shared.stats.lock().retries += 1;
            tracing::trace!(
                id = inflight.id,
                tries_left = inflight.tries_left,
                "ack overdue, retrying transaction"
            );
            if let Err(e) = self.socket.send_to(&inflight.sent, self.device_addr) {
                tracing::warn!("retry send failed: {e}");
            }
            return;
        }
        self.shared.stats.lock().timeouts += 1;
        let Some(inflight) = self.inflight.take() else {
            return;
        };
        if matches!(inflight.op, Op::Heartbeat) {
            tracing::warn!("heartbeat timed out, considering control lost");
            self.control_lost = true;
            return;
        }
        inflight.op.fail(CameraError::Timeout);
    }

    fn check_heartbeat(&mut self) {
        if Instant::now() < self.heartbeat_due {
            return;
        }
        self.heartbeat_due = Instant::now() + self.heartbeat_period;
        let pending_heartbeat = matches!(
            self.inflight,
            Some(Inflight {
                op: Op::Heartbeat,
                ..
            })
        ) || self.queue.iter().any(|op| matches!(op, Op::Heartbeat));
        if !pending_heartbeat {
            // Jump the queue: heartbeats keep device control alive and must
            // not wait behind a backlog of user transactions.
            self.queue.push_front(Op::Heartbeat);
            self.queued_payloads.push_front(PendingSend::ReadRegs(vec![
                bootstrap::CONTROL_CHANNEL_PRIVILEGE,
            ]));
        }
    }

    fn shutdown(&mut self) {
        tracing::debug!(
            control_lost = self.control_lost,
            "gvcp worker shutting down"
        );
        let err = if self.control_lost {
            self.shared.set_control_lost();
            CameraError::ControlLost
        } else {
            CameraError::Disconnected
        };
        if let Some(inflight) = self.inflight.take() {
            inflight.op.fail(clone_err(&err));
        }
        for op in self.queue.drain(..) {
            op.fail(clone_err(&err));
        }
        self.queued_payloads.clear();
        if !self.control_lost {
            // Best-effort control release so the device is immediately
            // claimable by the next application.
            self.next_id = gvcp::next_id(self.next_id);
            let release =
                gvcp::encode_write_reg(&[(bootstrap::CONTROL_CHANNEL_PRIVILEGE, 0)], self.next_id);
            let _ = self.socket.send_to(&release, self.device_addr);
        }
        self.event_txs.clear();
        self.thread.has_died();
        // Requests can sit behind the Shutdown message or land while this
        // teardown runs; fail them so no ResponseHandle is left pending
        // forever. `ControlPort::send` re-checks liveness after sending (and
        // has_died is already set above), so anything that slips past this
        // drain is failed by the sender instead — fulfilment is idempotent.
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                ToWorker::ReadReg(_, h) => h.fail(clone_err(&err)),
                ToWorker::ReadRegs(_, h) => h.fail(clone_err(&err)),
                ToWorker::WriteRegs(_, h) => h.fail(clone_err(&err)),
                ToWorker::ReadMem { handle, .. } => handle.fail(clone_err(&err)),
                ToWorker::WriteMem { handle, .. } => handle.fail(clone_err(&err)),
                ToWorker::SubscribeEvents(_) | ToWorker::Shutdown => {}
            }
        }
    }
}

fn clone_err(e: &CameraError) -> CameraError {
    match e {
        CameraError::ControlLost => CameraError::ControlLost,
        _ => CameraError::Disconnected,
    }
}

fn chunk_len(remaining: usize) -> usize {
    remaining.min(gvcp::DATA_SIZE_MAX)
}

fn heartbeat_period(cfg: &GigeConfig) -> Duration {
    Duration::from_millis(u64::from(cfg.heartbeat_timeout_ms / 3).max(10))
        .min(Duration::from_secs(1))
}

/// Bind the control socket and launch the worker thread. Returns the owner
/// [`ThreadHandle`] and the socket's local address.
pub(crate) fn spawn(
    rx: Receiver<ToWorker>,
    shared: Arc<Shared>,
    cfg: GigeConfig,
) -> Result<(ThreadHandle, SocketAddr), CameraError> {
    let bind_addr = cfg
        .local_addr
        .unwrap_or_else(|| SocketAddr::new(std::net::Ipv4Addr::UNSPECIFIED.into(), 0));
    let mut socket = UdpSocket::bind(bind_addr)
        .map_err(|e| CameraError::Spawn(format!("bind control socket {bind_addr}: {e}")))?;
    let local_addr = socket
        .local_addr()
        .map_err(|e| CameraError::Spawn(e.to_string()))?;

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

    let join = snare::thread::Builder::new()
        .name("telegenic-gvcp".into())
        .spawn(move || {
            let runner = Runner::new(socket, rx, shared, thread_for_worker, cfg);
            runner.run(poll);
        })
        .map_err(|e| CameraError::Spawn(e.to_string()))?;
    thread.set_handle(join);

    Ok((thread, local_addr))
}
