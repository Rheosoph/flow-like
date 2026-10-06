use crate::{Error, tunnel::Command};
use bytes::{Buf, Bytes};
use flow_like_device_protocol::{TUNNEL_INITIAL_WINDOW, TUNNEL_MAX_DATA, TunnelFrameBody};
use std::{
    collections::VecDeque,
    io,
    pin::Pin,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    task::{Context, Poll, Waker},
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    sync::mpsc,
};

/// Consumed bytes are returned to the device in batches of this size. The device never
/// stalls: it holds at least `TUNNEL_INITIAL_WINDOW - GRANT_BATCH` credit while the
/// application keeps reading.
const GRANT_BATCH: u32 = 32 * 1024;

/// State one stream shares between its handle and the tunnel driver.
pub(crate) struct Shared {
    id: u32,
    driver: mpsc::UnboundedSender<Command>,
    state: Mutex<State>,
}

struct State {
    received: VecDeque<Bytes>,
    receive_window: u32,
    unacknowledged: u32,
    remote_fin: bool,
    reader: Option<Waker>,
    send_credit: u32,
    outgoing: VecDeque<Bytes>,
    fin_requested: bool,
    fin_sent: bool,
    writer: Option<Waker>,
    scheduled: bool,
    failure: Option<Error>,
}

impl Shared {
    pub(crate) fn new(id: u32, driver: mpsc::UnboundedSender<Command>) -> Arc<Self> {
        Arc::new(Self {
            id,
            driver,
            state: Mutex::new(State {
                received: VecDeque::new(),
                receive_window: TUNNEL_INITIAL_WINDOW,
                unacknowledged: 0,
                remote_fin: false,
                reader: None,
                send_credit: TUNNEL_INITIAL_WINDOW,
                outgoing: VecDeque::new(),
                fin_requested: false,
                fin_sent: false,
                writer: None,
                scheduled: false,
                failure: None,
            }),
        })
    }

    /// Asks the driver for a send turn unless one is already queued.
    fn schedule(&self, state: &mut State) {
        if !state.scheduled {
            state.scheduled = true;
            let _ = self.driver.send(Command::Wake(self.id));
        }
    }

    pub(crate) fn receive(&self, bytes: Vec<u8>) -> Result<(), String> {
        let mut state = self.lock();
        if state.remote_fin || bytes.len() > state.receive_window as usize {
            return Err(format!(
                "stream {} received {} bytes beyond its window of {}",
                self.id,
                bytes.len(),
                state.receive_window
            ));
        }
        if state.failure.is_some() {
            return Ok(());
        }
        state.receive_window -= bytes.len() as u32;
        state.received.push_back(Bytes::from(bytes));
        wake(&mut state.reader);
        Ok(())
    }

    pub(crate) fn grant(&self, credit: u32) -> Result<(), String> {
        let mut state = self.lock();
        let total = state
            .send_credit
            .checked_add(credit)
            .filter(|total| credit > 0 && *total <= TUNNEL_INITIAL_WINDOW)
            .ok_or_else(|| {
                format!(
                    "stream {} was granted {credit} bytes on top of {}",
                    self.id, state.send_credit
                )
            })?;
        state.send_credit = total;
        wake(&mut state.writer);
        Ok(())
    }

    /// Returns whether the stream finished in both directions.
    pub(crate) fn finish_remote(&self) -> Result<bool, String> {
        let mut state = self.lock();
        if state.remote_fin {
            return Err(format!("stream {} finished twice", self.id));
        }
        state.remote_fin = true;
        state.unacknowledged = 0;
        wake(&mut state.reader);
        Ok(state.fin_sent)
    }

    pub(crate) fn finished(&self) -> bool {
        let state = self.lock();
        state.fin_sent && state.remote_fin
    }

    pub(crate) fn fail(&self, error: Error) {
        let mut state = self.lock();
        if state.failure.is_some() {
            return;
        }
        state.failure = Some(error);
        state.received.clear();
        state.outgoing.clear();
        wake(&mut state.reader);
        wake(&mut state.writer);
    }

    /// The next frame this stream may send. The bool says whether it needs another turn.
    pub(crate) fn next_frame(&self) -> (Option<TunnelFrameBody>, bool) {
        let mut state = self.lock();
        let frame = state.next_frame();
        let more = state.wants_turn();
        if !more {
            state.scheduled = false;
        }
        (frame, more)
    }
}

impl State {
    /// Returned credit first, then data, then Fin.
    fn next_frame(&mut self) -> Option<TunnelFrameBody> {
        if self.failure.is_some() {
            return None;
        }
        if self.unacknowledged >= GRANT_BATCH {
            let credit = std::mem::take(&mut self.unacknowledged);
            self.receive_window += credit;
            return Some(TunnelFrameBody::Window(credit));
        }
        if let Some(chunk) = self.next_chunk() {
            return Some(TunnelFrameBody::Data(chunk));
        }
        if self.fin_requested && !self.fin_sent {
            self.fin_sent = true;
            wake(&mut self.writer);
            return Some(TunnelFrameBody::Fin);
        }
        None
    }

    fn next_chunk(&mut self) -> Option<Vec<u8>> {
        let front = self.outgoing.front_mut()?;
        let chunk = front.split_to(front.len().min(TUNNEL_MAX_DATA));
        if front.is_empty() {
            self.outgoing.pop_front();
        }
        if self.outgoing.is_empty() {
            wake(&mut self.writer);
        }
        Some(chunk.to_vec())
    }

    fn wants_turn(&self) -> bool {
        self.failure.is_none()
            && (self.unacknowledged >= GRANT_BATCH
                || !self.outgoing.is_empty()
                || (self.fin_requested && !self.fin_sent))
    }
}

fn wake(waker: &mut Option<Waker>) {
    if let Some(waker) = waker.take() {
        waker.wake();
    }
}

/// A half-closeable byte stream to a device service or internal data target. Writes wait
/// for credit from the device, and only bytes the application reads return credit to it.
/// Dropping it before both directions finish resets the stream on the device.
pub struct TunnelStream {
    shared: Arc<Shared>,
}

impl TunnelStream {
    pub(crate) fn new(shared: Arc<Shared>) -> Self {
        Self { shared }
    }

    pub fn id(&self) -> u32 {
        self.shared.id
    }
}

impl std::fmt::Debug for TunnelStream {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.debug_struct("TunnelStream")
            .field("id", &self.shared.id)
            .finish_non_exhaustive()
    }
}

impl Drop for TunnelStream {
    fn drop(&mut self) {
        let _ = self.shared.driver.send(Command::Release(self.shared.id));
    }
}

impl AsyncRead for TunnelStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context,
        buf: &mut ReadBuf,
    ) -> Poll<io::Result<()>> {
        let shared = &self.shared;
        let mut guard = shared.lock();
        let state = &mut *guard;
        if let Some(error) = &state.failure {
            return Poll::Ready(Err(error.clone().into()));
        }
        if let Some(front) = state.received.front_mut() {
            let count = front.len().min(buf.remaining());
            buf.put_slice(&front[..count]);
            front.advance(count);
            if front.is_empty() {
                state.received.pop_front();
            }
            if !state.remote_fin {
                state.unacknowledged += count as u32;
                if state.unacknowledged >= GRANT_BATCH {
                    shared.schedule(state);
                }
            }
            return Poll::Ready(Ok(()));
        }
        if state.remote_fin {
            return Poll::Ready(Ok(()));
        }
        state.reader = Some(cx.waker().clone());
        Poll::Pending
    }
}

impl AsyncWrite for TunnelStream {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context, buf: &[u8]) -> Poll<io::Result<usize>> {
        let shared = &self.shared;
        let mut state = shared.lock();
        if let Some(error) = &state.failure {
            return Poll::Ready(Err(error.clone().into()));
        }
        if state.fin_requested {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                format!("stream {} was already shut down for writing", shared.id),
            )));
        }
        if buf.is_empty() {
            return Poll::Ready(Ok(0));
        }
        if state.send_credit == 0 {
            state.writer = Some(cx.waker().clone());
            return Poll::Pending;
        }
        let count = buf.len().min(state.send_credit as usize);
        state.send_credit -= count as u32;
        state
            .outgoing
            .push_back(Bytes::copy_from_slice(&buf[..count]));
        shared.schedule(&mut state);
        Poll::Ready(Ok(count))
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context) -> Poll<io::Result<()>> {
        let mut state = self.shared.lock();
        if let Some(error) = &state.failure {
            return Poll::Ready(Err(error.clone().into()));
        }
        if state.outgoing.is_empty() {
            return Poll::Ready(Ok(()));
        }
        state.writer = Some(cx.waker().clone());
        Poll::Pending
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context) -> Poll<io::Result<()>> {
        let shared = &self.shared;
        let mut state = shared.lock();
        if let Some(error) = &state.failure {
            return Poll::Ready(Err(error.clone().into()));
        }
        if state.fin_sent {
            return Poll::Ready(Ok(()));
        }
        if !state.fin_requested {
            state.fin_requested = true;
            shared.schedule(&mut state);
        }
        state.writer = Some(cx.waker().clone());
        Poll::Pending
    }
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
