//! Guest stdout and stderr, captured for the node's run log.
//!
//! Guests never write to the host process's own streams. The single-use
//! runners report run events over stdout, so a guest line there would break or
//! forge that channel; elsewhere it would land in operator logs with no run
//! attribution.

use crate::host_functions::logging::LogLevel;
use crate::host_functions::HostState;
use bytes::Bytes;
use parking_lot::Mutex;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::io::AsyncWrite;
use wasmtime_wasi::cli::{IsTerminal, StdoutStream};
use wasmtime_wasi::p2::{OutputStream, Pollable, StreamResult};

/// Output kept per call. The rest is dropped so a guest cannot grow host
/// memory or log volume without bound.
const MAX_CALL_OUTPUT: usize = 1 << 20;
/// Unterminated output is split at this length to keep entries readable.
const MAX_LINE: usize = 8 << 10;

#[derive(Clone, Copy)]
enum Channel {
    Stdout,
    Stderr,
}

impl Channel {
    fn level(self) -> LogLevel {
        match self {
            Channel::Stdout => LogLevel::Info,
            Channel::Stderr => LogLevel::Warn,
        }
    }
}

#[derive(Default)]
struct Captured {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    lines: Vec<(LogLevel, String)>,
    bytes: usize,
    dropped: bool,
}

impl Captured {
    fn pending(&mut self, channel: Channel) -> &mut Vec<u8> {
        match channel {
            Channel::Stdout => &mut self.stdout,
            Channel::Stderr => &mut self.stderr,
        }
    }

    fn append(&mut self, channel: Channel, bytes: &[u8]) {
        let accepted = bytes.len().min(MAX_CALL_OUTPUT - self.bytes);
        self.dropped |= accepted < bytes.len();
        self.bytes += accepted;
        let mut rest = &bytes[..accepted];
        while !rest.is_empty() {
            let pending = self.pending(channel);
            let room = (MAX_LINE - pending.len()).min(rest.len());
            let take = rest[..room]
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(room, |newline| newline + 1);
            pending.extend_from_slice(&rest[..take]);
            rest = &rest[take..];
            if pending.ends_with(b"\n") || pending.len() == MAX_LINE {
                self.end_line(channel);
            }
        }
    }

    fn end_line(&mut self, channel: Channel) {
        let raw = std::mem::take(self.pending(channel));
        let text = String::from_utf8_lossy(&raw);
        let line = text.trim_end_matches(['\n', '\r']);
        if !line.is_empty() {
            self.lines.push((channel.level(), line.to_owned()));
        }
    }
}

/// Output a store's guest has written since the last drain.
#[derive(Default)]
pub(crate) struct GuestOutput(Arc<Mutex<Captured>>);

impl GuestOutput {
    pub(crate) fn stdout(&self) -> GuestOutputStream {
        self.stream(Channel::Stdout)
    }

    pub(crate) fn stderr(&self) -> GuestOutputStream {
        self.stream(Channel::Stderr)
    }

    fn stream(&self, channel: Channel) -> GuestOutputStream {
        GuestOutputStream {
            captured: self.0.clone(),
            channel,
        }
    }

    /// Move everything written since the last drain, including an unterminated
    /// final line, into the invocation's log and reset the per-call budget.
    pub(crate) fn drain_into(&self, host: &HostState) {
        let mut captured = std::mem::take(&mut *self.0.lock());
        captured.end_line(Channel::Stdout);
        captured.end_line(Channel::Stderr);
        for (level, line) in captured.lines {
            host.log(level as u8, line, None);
        }
        if captured.dropped {
            host.log(
                LogLevel::Warn as u8,
                format!(
                    "Guest output exceeded {} MiB in one call; the rest was dropped",
                    MAX_CALL_OUTPUT >> 20
                ),
                None,
            );
        }
    }
}

#[derive(Clone)]
pub(crate) struct GuestOutputStream {
    captured: Arc<Mutex<Captured>>,
    channel: Channel,
}

impl GuestOutputStream {
    fn append(&self, bytes: &[u8]) {
        self.captured.lock().append(self.channel, bytes);
    }
}

impl IsTerminal for GuestOutputStream {
    fn is_terminal(&self) -> bool {
        false
    }
}

impl StdoutStream for GuestOutputStream {
    fn p2_stream(&self) -> Box<dyn OutputStream> {
        Box::new(self.clone())
    }

    fn async_stream(&self) -> Box<dyn AsyncWrite + Send + Sync> {
        Box::new(self.clone())
    }
}

#[async_trait::async_trait]
impl Pollable for GuestOutputStream {
    async fn ready(&mut self) {}
}

impl OutputStream for GuestOutputStream {
    fn write(&mut self, bytes: Bytes) -> StreamResult<()> {
        self.append(&bytes);
        Ok(())
    }

    fn flush(&mut self) -> StreamResult<()> {
        Ok(())
    }

    fn check_write(&mut self) -> StreamResult<usize> {
        Ok(MAX_LINE)
    }
}

impl AsyncWrite for GuestOutputStream {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.append(buf);
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::WasmCapabilities;

    fn drained(output: &GuestOutput) -> Vec<(u8, String)> {
        let host = HostState::new(WasmCapabilities::empty());
        output.drain_into(&host);
        host.get_logs()
            .into_iter()
            .map(|entry| (entry.level, entry.message))
            .collect()
    }

    #[test]
    fn lines_are_logged_per_channel_once_complete() {
        let output = GuestOutput::default();
        output.stdout().append(b"hel");
        output.stderr().append(b"careful\n");
        output.stdout().append(b"lo\r\n\nunterminated");

        assert_eq!(
            drained(&output),
            vec![
                (LogLevel::Warn as u8, "careful".to_owned()),
                (LogLevel::Info as u8, "hello".to_owned()),
                (LogLevel::Info as u8, "unterminated".to_owned()),
            ]
        );
        assert!(drained(&output).is_empty());
    }

    #[test]
    fn output_beyond_the_call_budget_is_dropped_until_the_next_drain() {
        let output = GuestOutput::default();
        output.stdout().append(&vec![b'x'; MAX_CALL_OUTPUT + 1]);

        let logs = drained(&output);
        let (dropped, lines) = logs.split_last().expect("output should be logged");
        assert_eq!(lines.len(), MAX_CALL_OUTPUT / MAX_LINE);
        assert!(lines
            .iter()
            .all(|(level, line)| *level == LogLevel::Info as u8 && line.len() == MAX_LINE));
        assert_eq!(dropped.0, LogLevel::Warn as u8);
        assert!(dropped.1.contains("dropped"));

        output.stdout().append(b"next call\n");
        assert_eq!(
            drained(&output),
            vec![(LogLevel::Info as u8, "next call".to_owned())]
        );
    }
}
