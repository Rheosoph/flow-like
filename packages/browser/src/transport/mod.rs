use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

pub(crate) mod dial;
#[cfg(any(test, feature = "test-support"))]
pub mod memory;
pub mod tls;
pub mod ws;

pub const MAX_OUTBOUND_BYTES: usize = 96 * 1024 * 1024;

pub enum Inbound {
    Text(String),
    InvalidUtf8(Vec<u8>),
    Closed { reason: String },
}

pub struct Outbound {
    pub text: String,
    pub method: Arc<str>,
    pub deadline: Option<tokio::time::Instant>,
    pub ticket: WriteTicket,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteStatus {
    NotWritten,
    Written,
    Indeterminate,
}

impl WriteStatus {
    fn to_bits(self) -> u8 {
        match self {
            Self::NotWritten => 0,
            Self::Written => 1,
            Self::Indeterminate => 2,
        }
    }

    fn from_bits(bits: u8) -> Self {
        match bits {
            1 => Self::Written,
            2 => Self::Indeterminate,
            _ => Self::NotWritten,
        }
    }
}

#[derive(Clone, Default)]
pub struct WriteTicket(Arc<AtomicU8>);

impl WriteTicket {
    pub fn status(&self) -> WriteStatus {
        WriteStatus::from_bits(self.0.load(Ordering::Acquire))
    }

    pub fn set(&self, status: WriteStatus) {
        self.0.store(status.to_bits(), Ordering::Release);
    }
}

pub struct TransportChannels {
    pub inbound: tokio::sync::mpsc::UnboundedReceiver<Inbound>,
    pub outbound: tokio::sync::mpsc::UnboundedSender<Outbound>,
    pub control: TransportControl,
}

pub trait TransportShutdown: Send + Sync {
    fn abort(&self);
    fn close(&self) -> futures_util::future::BoxFuture<'static, ()>;
}

#[derive(Clone)]
pub struct TransportControl(Arc<dyn TransportShutdown>);

impl TransportControl {
    pub fn new(inner: Arc<dyn TransportShutdown>) -> Self {
        Self(inner)
    }

    pub fn abort(&self) {
        self.0.abort();
    }

    pub async fn close(&self) {
        self.0.close().await;
    }
}

pub trait Transport: Send + 'static {
    fn start(self: Box<Self>) -> TransportChannels;
}
