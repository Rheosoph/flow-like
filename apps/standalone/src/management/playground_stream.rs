//! The owner's live view of one bot's screen over a tunnel stream (`TunnelTarget::BotPlayground`).
//! Frames and state flow device to viewer, control and input flow back; both go through the
//! agent's shared `MachineHost`, never the hub.

use anyhow::{Context, Result};
use flow_like_device_protocol::{
    PLAYGROUND_MAX_JSON, PlaygroundControl, PlaygroundDecoder, PlaygroundDisplay, PlaygroundEvent,
    PlaygroundFrame, PlaygroundFrameKind, PlaygroundFrameMeta, PlaygroundInput, PlaygroundState,
};
use flow_like_types_contracts::interaction::InteractionObservation;
use std::{
    future::Future,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::mpsc,
};

/// At most five frames a second while a viewer is attached.
pub(crate) const FRAME_INTERVAL: Duration = Duration::from_millis(200);
/// An idle screen is captured again at this pace so the picture never freezes.
pub(crate) const REFRESH_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Clone, Copy)]
pub(crate) struct Pace {
    pub frame: Duration,
    pub refresh: Duration,
}

impl Default for Pace {
    fn default() -> Self {
        Self {
            frame: FRAME_INTERVAL,
            refresh: REFRESH_INTERVAL,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LiveFrame {
    pub jpeg: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub kind: PlaygroundFrameKind,
    pub caption: String,
    pub taken: Instant,
    pub observation: Option<InteractionObservation>,
}

/// What the stream needs from the agent's machine host.
pub(crate) trait Playground: Clone + Send + Sync + 'static {
    fn latest(&self, bot_id: &str) -> Option<LiveFrame>;
    /// The latest picture, captured again when it is old.
    fn refresh(&self, bot_id: &str) -> impl Future<Output = Option<LiveFrame>> + Send;
    /// A picture taken now.
    fn capture(&self, bot_id: &str) -> impl Future<Output = Option<LiveFrame>> + Send;
    fn is_taken_over(&self, bot_id: &str) -> bool;
    fn take_over(&self, bot_id: &str) -> Result<()>;
    fn hand_back(&self, bot_id: &str) -> Result<()>;
    fn inject(
        &self,
        bot_id: &str,
        event: PlaygroundInput,
    ) -> impl Future<Output = Result<()>> + Send;
    fn stop(&self, bot_id: &str) -> impl Future<Output = ()> + Send;
    fn display(&self) -> PlaygroundDisplay;
}

#[cfg(all(
    feature = "playground",
    any(target_os = "macos", windows, target_os = "linux")
))]
mod machine {
    use super::*;
    use flow_like_bot_machine::{Frame, FrameKind, InputEvent};

    fn live(frame: Frame) -> LiveFrame {
        LiveFrame {
            jpeg: frame.jpeg.to_vec(),
            width: frame.width,
            height: frame.height,
            kind: match frame.kind {
                FrameKind::Browser => PlaygroundFrameKind::Browser,
                FrameKind::Computer => PlaygroundFrameKind::Computer,
            },
            caption: frame.caption,
            taken: frame.at,
            observation: frame.observation,
        }
    }

    impl Playground for crate::playground::MachineHost {
        fn latest(&self, bot_id: &str) -> Option<LiveFrame> {
            self.frames(bot_id).latest().map(live)
        }

        async fn refresh(&self, bot_id: &str) -> Option<LiveFrame> {
            self.refresh_frame(bot_id).await.map(live)
        }

        async fn capture(&self, bot_id: &str) -> Option<LiveFrame> {
            self.capture_frame(bot_id).await.map(live)
        }

        fn is_taken_over(&self, bot_id: &str) -> bool {
            crate::playground::MachineHost::is_taken_over(self, bot_id)
        }

        fn take_over(&self, bot_id: &str) -> Result<()> {
            crate::playground::MachineHost::take_over(self, bot_id)
                .map_err(|e| anyhow::anyhow!("{e:#}"))
        }

        fn hand_back(&self, bot_id: &str) -> Result<()> {
            crate::playground::MachineHost::hand_back(self, bot_id)
                .map_err(|e| anyhow::anyhow!("{e:#}"))
        }

        async fn inject(&self, bot_id: &str, event: PlaygroundInput) -> Result<()> {
            let event: InputEvent = serde_json::from_value(
                serde_json::to_value(&event).context("Encode playground input")?,
            )
            .context("Map playground input to the machine's input event")?;
            crate::playground::MachineHost::inject(self, bot_id, event)
                .await
                .map_err(|e| anyhow::anyhow!("{e:#}"))
        }

        async fn stop(&self, bot_id: &str) {
            crate::playground::MachineHost::stop(self, bot_id).await
        }

        fn display(&self) -> PlaygroundDisplay {
            match crate::playground::MachineHost::display(self) {
                flow_like_bot_machine::DisplayKind::Virtual => PlaygroundDisplay::Virtual,
                flow_like_bot_machine::DisplayKind::Session => PlaygroundDisplay::Session,
                flow_like_bot_machine::DisplayKind::None(_) => PlaygroundDisplay::None,
            }
        }
    }
}

struct Session<P: Playground> {
    host: P,
    bot_id: String,
    took_over: bool,
    sent_state: Option<PlaygroundState>,
    sent_frame: Option<(Vec<u8>, String, Option<InteractionObservation>)>,
    observations: bool,
}

const MAX_CAPTION_CHARS: usize = 512;

/// A picture is worth more than its overlay: an oversized observation is dropped, not the stream.
fn fit_meta(meta: &mut PlaygroundFrameMeta) {
    let oversized = |meta: &PlaygroundFrameMeta| {
        serde_json::to_vec(meta).map_or(true, |bytes| bytes.len() > PLAYGROUND_MAX_JSON)
    };
    if oversized(meta) {
        meta.observation = None;
    }
    if oversized(meta) {
        meta.caption = meta.caption.chars().take(MAX_CAPTION_CHARS).collect();
    }
}

impl<P: Playground> Drop for Session<P> {
    fn drop(&mut self) {
        if self.took_over {
            let _ = self.host.hand_back(&self.bot_id);
        }
    }
}

impl<P: Playground> Session<P> {
    async fn send<W: AsyncWrite + Unpin>(
        &self,
        write: &mut W,
        event: PlaygroundEvent,
    ) -> Result<()> {
        let bytes = event.encode().context("Encode playground message")?;
        write
            .write_all(&bytes)
            .await
            .context("Send playground message")
    }

    async fn error<W: AsyncWrite + Unpin>(
        &self,
        write: &mut W,
        code: &str,
        error: &anyhow::Error,
    ) -> Result<()> {
        self.send(
            write,
            PlaygroundEvent::Error {
                code: code.into(),
                message: format!("{error:#}"),
            },
        )
        .await
    }

    async fn state<W: AsyncWrite + Unpin>(
        &mut self,
        write: &mut W,
        frame: Option<&LiveFrame>,
    ) -> Result<()> {
        let latest;
        let frame = match frame {
            Some(frame) => Some(frame),
            None => {
                latest = self.host.latest(&self.bot_id);
                latest.as_ref()
            }
        };
        let state = PlaygroundState {
            taken_over: self.host.is_taken_over(&self.bot_id),
            asleep: frame.is_none(),
            display: self.host.display(),
            kind: frame.map(|frame| frame.kind),
        };
        if self.sent_state.as_ref() != Some(&state) {
            self.send(write, PlaygroundEvent::State(state.clone()))
                .await?;
            self.sent_state = Some(state);
        }
        Ok(())
    }

    async fn pump<W: AsyncWrite + Unpin>(
        &mut self,
        write: &mut W,
        refresh_due: bool,
    ) -> Result<()> {
        let frame = if self.host.is_taken_over(&self.bot_id) {
            self.host.capture(&self.bot_id).await
        } else if refresh_due {
            self.host.refresh(&self.bot_id).await
        } else {
            self.host.latest(&self.bot_id)
        };
        self.state(write, frame.as_ref()).await?;
        let Some(frame) = frame else {
            self.sent_frame = None;
            return Ok(());
        };
        let observation = self.observations.then_some(frame.observation).flatten();
        let key = (
            frame.jpeg.clone(),
            frame.caption.clone(),
            observation.clone(),
        );
        if self.sent_frame.as_ref() == Some(&key) {
            return Ok(());
        }
        self.sent_frame = Some(key);
        let age_ms = frame.taken.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
        let mut meta = PlaygroundFrameMeta {
            width: frame.width,
            height: frame.height,
            kind: frame.kind,
            caption: frame.caption,
            age_ms,
            observation,
        };
        fit_meta(&mut meta);
        self.send(
            write,
            PlaygroundEvent::Frame(PlaygroundFrame {
                meta,
                jpeg: frame.jpeg,
            }),
        )
        .await
    }

    async fn apply<W: AsyncWrite + Unpin>(
        &mut self,
        write: &mut W,
        control: PlaygroundControl,
    ) -> Result<()> {
        match control {
            PlaygroundControl::TakeOver => match self.host.take_over(&self.bot_id) {
                Ok(()) => self.took_over = true,
                Err(error) => self.error(write, "take_over_failed", &error).await?,
            },
            PlaygroundControl::HandBack => {
                if let Err(error) = self.host.hand_back(&self.bot_id) {
                    self.error(write, "hand_back_failed", &error).await?;
                }
                self.took_over = false;
            }
            PlaygroundControl::Stop => {
                self.host.stop(&self.bot_id).await;
                self.took_over = false;
                self.sent_frame = None;
            }
            PlaygroundControl::Input { event } => {
                if let Err(error) = self.host.inject(&self.bot_id, event).await {
                    self.error(write, "input_refused", &error).await?;
                }
            }
        }
        self.state(write, None).await
    }
}

async fn read_controls<R: AsyncRead + Unpin>(
    mut read: R,
    controls: mpsc::Sender<Result<PlaygroundControl>>,
) {
    let mut decoder = PlaygroundDecoder::default();
    let mut buffer = vec![0u8; 8 * 1024];
    loop {
        match read.read(&mut buffer).await {
            Ok(0) | Err(_) => return,
            Ok(count) => decoder.push(&buffer[..count]),
        }
        loop {
            match decoder.next_control() {
                Ok(Some(control)) => {
                    if controls.send(Ok(control)).await.is_err() {
                        return;
                    }
                }
                Ok(None) => break,
                Err(error) => {
                    let _ = controls
                        .send(Err(anyhow::anyhow!(
                            "Playground control from the viewer: {error}"
                        )))
                        .await;
                    return;
                }
            }
        }
    }
}

/// Serves one viewer until it disconnects, sends something malformed, or the stream is cancelled.
/// A viewer that leaves while in control hands the bot back.
pub(crate) async fn serve<P, IO>(
    io: IO,
    host: P,
    bot_id: String,
    observations: bool,
    pace: Pace,
) -> Result<()>
where
    P: Playground,
    IO: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (read, mut write) = tokio::io::split(io);
    let (sender, mut controls) = mpsc::channel(64);
    let reader = tokio::spawn(read_controls(read, sender));
    let mut session = Session {
        host,
        bot_id,
        took_over: false,
        sent_state: None,
        sent_frame: None,
        observations,
    };
    let result = run(&mut session, &mut write, &mut controls, pace).await;
    reader.abort();
    result
}

async fn run<P: Playground, W: AsyncWrite + Unpin>(
    session: &mut Session<P>,
    write: &mut W,
    controls: &mut mpsc::Receiver<Result<PlaygroundControl>>,
    pace: Pace,
) -> Result<()> {
    let mut ticker = tokio::time::interval(pace.frame);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut refreshed: Option<tokio::time::Instant> = None;
    loop {
        tokio::select! {
            control = controls.recv() => match control {
                None => return Ok(()),
                Some(Err(error)) => return Err(error),
                Some(Ok(control)) => session.apply(write, control).await?,
            },
            _ = ticker.tick() => {
                let due = refreshed.is_none_or(|at| at.elapsed() >= pace.refresh);
                if due {
                    refreshed = Some(tokio::time::Instant::now());
                }
                session.pump(write, due).await?;
            }
        }
    }
}

#[cfg(test)]
mod tests;
