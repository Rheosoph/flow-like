use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use dashmap::DashMap;
use flow_like_bot_tools::browser::BrowserBackend;
use flow_like_bot_tools::computer::ComputerBackend;
use flow_like_bot_tools::error::{ToolError, code};
use flow_like_types::Result;

use crate::desk_lock::DesktopLock;
use crate::display::{DisplayKind, DisplayPolicy, Facts, resolve_display};
use crate::frames::{Frame, FrameKind, FrameStream};
use crate::guard::{BotChannel, GuardedBrowser, GuardedComputer, NoDisplay};
use crate::input::InputEvent;
use crate::local::LocalPlayground;
use crate::playground::Playground;
use crate::rpc::RemotePlayground;
use crate::worker::ProcessSpawner;

const IDLE_AFTER: Duration = Duration::from_secs(10 * 60);
const SWEEP_EVERY: Duration = Duration::from_secs(30);

/// Where the bots' browsers and computers run.
#[derive(Clone, Debug)]
pub enum Mode {
    /// In this process, on the logged-in screen (the desktop app).
    InProcess,
    /// One worker process per bot, started from `program` as `<program> playground-worker …`
    /// (a device agent).
    Worker { program: PathBuf },
}

/// The backends one tool call may use.
#[derive(Default, Clone)]
pub struct Machine {
    pub browser: Option<Arc<dyn BrowserBackend>>,
    pub computer: Option<Arc<dyn ComputerBackend>>,
}

struct BotEntry {
    playground: Arc<dyn Playground>,
    channel: Arc<BotChannel>,
}

type PlaygroundFactory = Box<dyn Fn(&str) -> Arc<dyn Playground> + Send + Sync>;

struct Inner {
    in_process: bool,
    display: DisplayKind,
    desk: Arc<DesktopLock>,
    factory: PlaygroundFactory,
    bots: DashMap<String, Arc<BotEntry>>,
    sweeper: AtomicBool,
}

/// All bots' machines on this device. Cheap to clone.
#[derive(Clone)]
pub struct MachineHost {
    inner: Arc<Inner>,
}

impl MachineHost {
    pub fn new(data_dir: PathBuf, mode: Mode) -> Self {
        match mode {
            Mode::InProcess => Self::build(true, DisplayKind::Session, move |bot| {
                LocalPlayground::new(&data_dir, bot, true) as Arc<dyn Playground>
            }),
            Mode::Worker { program } => {
                let display = resolve_display(DisplayPolicy::from_env(), Facts::detect());
                Self::with_display(display.clone(), move |bot| {
                    let spawner = Arc::new(ProcessSpawner {
                        program: program.clone(),
                        data_dir: data_dir.clone(),
                        bot_id: bot.to_string(),
                        display: display.clone(),
                    });
                    Arc::new(RemotePlayground::new(spawner)) as Arc<dyn Playground>
                })
            }
        }
    }

    /// A host whose bots' playgrounds come from `factory` and whose computer is `display`.
    pub fn with_display(
        display: DisplayKind,
        factory: impl Fn(&str) -> Arc<dyn Playground> + Send + Sync + 'static,
    ) -> Self {
        Self::build(false, display, factory)
    }

    fn build(
        in_process: bool,
        display: DisplayKind,
        factory: impl Fn(&str) -> Arc<dyn Playground> + Send + Sync + 'static,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                in_process,
                display,
                desk: Arc::new(DesktopLock::default()),
                factory: Box::new(factory),
                bots: DashMap::new(),
                sweeper: AtomicBool::new(false),
            }),
        }
    }

    pub fn display(&self) -> &DisplayKind {
        &self.inner.display
    }

    fn entry(&self, bot_id: &str) -> Arc<BotEntry> {
        self.inner
            .bots
            .entry(bot_id.to_string())
            .or_insert_with(|| {
                Arc::new(BotEntry {
                    playground: (self.inner.factory)(bot_id),
                    channel: Arc::new(BotChannel::default()),
                })
            })
            .clone()
    }

    fn existing(&self, bot_id: &str) -> Option<Arc<BotEntry>> {
        self.inner.bots.get(bot_id).map(|entry| entry.clone())
    }

    fn ensure_sweeper(&self) {
        if self.inner.sweeper.swap(true, Ordering::AcqRel) {
            return;
        }
        if tokio::runtime::Handle::try_current().is_err() {
            self.inner.sweeper.store(false, Ordering::Release);
            return;
        }
        let weak = Arc::downgrade(&self.inner);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(SWEEP_EVERY).await;
                let Some(inner) = weak.upgrade() else { return };
                let entries: Vec<_> = inner.bots.iter().map(|e| e.value().clone()).collect();
                for entry in entries {
                    entry.playground.sweep(IDLE_AFTER).await;
                }
            }
        });
    }

    pub fn machine(
        &self,
        bot_id: &str,
        conversation_id: Option<&str>,
        browser_use: bool,
        computer_use: bool,
    ) -> Machine {
        self.ensure_sweeper();
        let entry = self.entry(bot_id);
        let browser = browser_use.then(|| {
            Arc::new(GuardedBrowser {
                inner: entry.playground.browser(conversation_id),
                channel: entry.channel.clone(),
            }) as Arc<dyn BrowserBackend>
        });
        let computer = computer_use.then(|| self.computer_of(bot_id, &entry));
        Machine { browser, computer }
    }

    fn computer_of(&self, bot_id: &str, entry: &BotEntry) -> Arc<dyn ComputerBackend> {
        if let DisplayKind::None(why) = &self.inner.display {
            return Arc::new(NoDisplay(why.clone()));
        }
        let shared_screen = !self.inner.in_process && self.inner.display == DisplayKind::Session;
        Arc::new(GuardedComputer {
            bot_id: bot_id.to_string(),
            inner: entry.playground.computer(),
            channel: entry.channel.clone(),
            desk: shared_screen.then(|| self.inner.desk.clone()),
        })
    }

    /// Ends the bot's browsers and computer (and its worker and virtual display).
    pub async fn stop(&self, bot_id: &str) {
        if let Some((_, entry)) = self.inner.bots.remove(bot_id) {
            entry.playground.stop().await;
            entry.channel.frames.clear();
            entry.channel.takeover.hand_back();
            entry.channel.takeover.take_note();
        }
        self.inner.desk.release(bot_id);
    }

    pub async fn shutdown(&self) {
        let bots: Vec<String> = self.inner.bots.iter().map(|e| e.key().clone()).collect();
        for bot in bots {
            self.stop(&bot).await;
        }
    }

    pub fn frames(&self, bot_id: &str) -> FrameStream {
        self.entry(bot_id).channel.frames.stream()
    }

    /// The latest frame, captured now when it is older than 1.5 s and the bot has anything to show.
    pub async fn refresh_frame(&self, bot_id: &str) -> Option<Frame> {
        let entry = self.existing(bot_id)?;
        if let Some(frame) = entry.channel.frames.latest().filter(Frame::is_fresh) {
            return Some(frame);
        }
        self.capture_frame(bot_id).await
    }

    /// A picture taken now, however fresh the latest one is; the live view uses it while the owner is
    /// in control. Never starts a worker.
    pub async fn capture_frame(&self, bot_id: &str) -> Option<Frame> {
        let entry = self.existing(bot_id)?;
        let shot = entry.playground.capture().await.ok().flatten()?;
        entry.channel.frames.publish(&shot)
    }

    pub fn take_over(&self, bot_id: &str) -> Result<()> {
        self.entry(bot_id).channel.takeover.take();
        Ok(())
    }

    pub fn hand_back(&self, bot_id: &str) -> Result<()> {
        if let Some(entry) = self.existing(bot_id) {
            entry.channel.takeover.hand_back();
        }
        Ok(())
    }

    pub fn is_taken_over(&self, bot_id: &str) -> bool {
        self.existing(bot_id)
            .is_some_and(|entry| entry.channel.takeover.is_taken())
    }

    pub async fn inject(&self, bot_id: &str, event: InputEvent) -> Result<()> {
        let refused = |message: &str| flow_like_types::Error::new(ToolError::refused(message));
        let entry = self
            .existing(bot_id)
            .filter(|entry| entry.channel.takeover.is_taken())
            .ok_or_else(|| refused("Take over the computer before sending it input"))?;
        if self.inner.in_process {
            return Err(flow_like_types::Error::new(ToolError::new(
                code::NOT_SUPPORTED,
                "On this computer you use your own mouse and keyboard; there is nothing to inject",
            )));
        }
        let frame = entry
            .channel
            .frames
            .latest()
            .filter(|frame| frame.kind == FrameKind::Computer)
            .ok_or_else(|| {
                flow_like_types::Error::new(ToolError::new(
                    code::NOT_SUPPORTED,
                    "The bot's browser has no window of its own to click in; watch only",
                ))
            })?;
        entry
            .playground
            .inject(&event, frame.width, frame.height)
            .await
    }
}

#[cfg(test)]
mod tests;
