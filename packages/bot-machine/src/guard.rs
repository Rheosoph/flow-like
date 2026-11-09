//! What every bot call passes through, whichever process the playground runs in: the owner's
//! takeover, the desktop lock and the latest-frame store.

use std::sync::Arc;

use async_trait::async_trait;
use flow_like_bot_tools::browser::{
    BrowserBackend, BrowserScreen, BrowserStep, Download, PageState, SnapshotRequest, SnapshotView,
};
use flow_like_bot_tools::computer::{
    ComputerAction, ComputerBackend, Screen, WindowBounds, WindowOp, WindowSummary,
};
use flow_like_bot_tools::error::{ToolError, code};
use flow_like_types::Result;
use flow_like_types::interaction_observation::InteractionObservation;

use crate::desk_lock::DesktopLock;
use crate::frames::{FrameKind, FrameStore, Shot};
use crate::takeover::Takeover;

#[derive(Default)]
pub struct BotChannel {
    pub takeover: Takeover,
    pub(crate) frames: FrameStore,
}

impl BotChannel {
    pub(crate) fn publish(
        &self,
        png: &[u8],
        kind: FrameKind,
        caption: &str,
        observation: Option<InteractionObservation>,
    ) {
        self.frames.publish(&Shot {
            png: png.to_vec(),
            kind,
            caption: caption.to_string(),
            observation,
        });
    }
}

fn failure(error: ToolError) -> flow_like_types::Error {
    flow_like_types::Error::new(error)
}

pub struct GuardedBrowser {
    pub inner: Arc<dyn BrowserBackend>,
    pub channel: Arc<BotChannel>,
}

impl GuardedBrowser {
    fn guard(&self) -> Result<()> {
        self.channel.takeover.guard().map_err(failure)
    }

    fn fresh_look_needed(&self) -> Result<()> {
        self.channel.takeover.require_fresh(true).map_err(failure)
    }
}

#[async_trait]
impl BrowserBackend for GuardedBrowser {
    async fn open(&self, url: &str) -> Result<PageState> {
        let generation = self.channel.takeover.generation();
        self.guard()?;
        let page = self.inner.open(url).await?;
        if self.channel.takeover.observed(true, generation) {
            self.channel.takeover.take_note();
        }
        Ok(page)
    }

    async fn snapshot(&self, request: SnapshotRequest) -> Result<SnapshotView> {
        let generation = self.channel.takeover.generation();
        self.guard()?;
        let mut view = self.inner.snapshot(request).await?;
        let note = self
            .channel
            .takeover
            .observed(true, generation)
            .then(|| self.channel.takeover.take_note())
            .flatten();
        if let Some(note) = note {
            view.text = format!("[{note}]\n{}", view.text);
        }
        Ok(view)
    }

    async fn act(&self, step: &BrowserStep) -> Result<PageState> {
        self.guard()?;
        self.fresh_look_needed()?;
        self.inner.act(step).await
    }

    async fn screenshot(&self) -> Result<Vec<u8>> {
        Ok(self.capture().await?.png)
    }

    async fn observation(&self) -> Result<Option<InteractionObservation>> {
        self.guard()?;
        self.inner.observation().await
    }

    async fn capture(&self) -> Result<BrowserScreen> {
        self.guard()?;
        let capture = self.inner.capture().await?;
        let caption = self
            .inner
            .current()
            .await
            .ok()
            .flatten()
            .map(|page| page.url)
            .unwrap_or_default();
        self.channel.publish(
            &capture.png,
            FrameKind::Browser,
            &caption,
            capture.observation.clone(),
        );
        Ok(capture)
    }

    async fn current(&self) -> Result<Option<PageState>> {
        self.inner.current().await
    }

    async fn leave(&self) -> Result<()> {
        self.inner.leave().await
    }

    async fn take_downloads(&self) -> Result<Vec<Download>> {
        self.inner.take_downloads().await
    }

    async fn close(&self) -> Result<()> {
        self.inner.close().await
    }
}

pub struct GuardedComputer {
    pub bot_id: String,
    pub inner: Arc<dyn ComputerBackend>,
    pub channel: Arc<BotChannel>,
    pub desk: Option<Arc<DesktopLock>>,
}

impl GuardedComputer {
    async fn enter(&self) -> Result<Option<String>> {
        self.channel.takeover.guard().map_err(failure)?;
        if let Some(desk) = &self.desk {
            desk.acquire(&self.bot_id).await.map_err(failure)?;
        }
        self.channel.takeover.guard().map_err(failure)?;
        Ok(self.channel.takeover.take_note())
    }

    async fn enter_fresh(&self) -> Result<Option<String>> {
        self.channel.takeover.guard().map_err(failure)?;
        self.channel
            .takeover
            .require_fresh(false)
            .map_err(failure)?;
        let note = self.enter().await?;
        self.channel
            .takeover
            .require_fresh(false)
            .map_err(failure)?;
        Ok(note)
    }

    fn shown(&self, mut screen: Screen, note: Option<String>) -> Screen {
        self.channel.publish(
            &screen.png,
            FrameKind::Computer,
            "Screen",
            screen.observation.clone(),
        );
        screen.note = note.or(screen.note);
        screen
    }
}

#[async_trait]
impl ComputerBackend for GuardedComputer {
    async fn screenshot(&self) -> Result<Screen> {
        let generation = self.channel.takeover.generation();
        let note = self.enter().await?;
        let screen = self.inner.screenshot().await?;
        self.channel.takeover.observed(false, generation);
        Ok(self.shown(screen, note))
    }

    async fn act(&self, action: &ComputerAction) -> Result<String> {
        let note = self.enter_fresh().await?;
        let done = self.inner.act(action).await?;
        Ok(match note {
            Some(note) => format!("{done}. {note}"),
            None => done,
        })
    }

    async fn screenshot_window(&self, window: &str) -> Result<Screen> {
        let generation = self.channel.takeover.generation();
        let note = self.enter().await?;
        let screen = self.inner.screenshot_window(window).await?;
        self.channel.takeover.observed(false, generation);
        Ok(self.shown(screen, note))
    }

    async fn windows(&self) -> Result<Vec<WindowSummary>> {
        self.enter().await?;
        self.inner.windows().await
    }

    async fn window(&self, window: &str, op: WindowOp, bounds: WindowBounds) -> Result<String> {
        let note = self.enter_fresh().await?;
        let done = self.inner.window(window, op, bounds).await?;
        Ok(match note {
            Some(note) => format!("{done}. {note}"),
            None => done,
        })
    }

    async fn launch(&self, program: &str, args: &[String]) -> Result<String> {
        let note = self.enter_fresh().await?;
        let done = self.inner.launch(program, args).await?;
        Ok(match note {
            Some(note) => format!("{done}. {note}"),
            None => done,
        })
    }
}

/// The computer of a machine with no display: every call says what to install.
pub struct NoDisplay(pub String);

impl NoDisplay {
    fn error<T>(&self) -> Result<T> {
        Err(failure(ToolError::new(code::NO_DISPLAY, self.0.clone())))
    }
}

#[async_trait]
impl ComputerBackend for NoDisplay {
    async fn screenshot(&self) -> Result<Screen> {
        self.error()
    }

    async fn act(&self, _action: &ComputerAction) -> Result<String> {
        self.error()
    }

    async fn screenshot_window(&self, _window: &str) -> Result<Screen> {
        self.error()
    }

    async fn windows(&self) -> Result<Vec<WindowSummary>> {
        self.error()
    }

    async fn window(&self, _window: &str, _op: WindowOp, _bounds: WindowBounds) -> Result<String> {
        self.error()
    }

    async fn launch(&self, _program: &str, _args: &[String]) -> Result<String> {
        self.error()
    }
}
