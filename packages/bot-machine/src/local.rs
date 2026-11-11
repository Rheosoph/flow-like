//! The playground in this process: the nodes' browser and desktop input behind the bot tools'
//! backends.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use dashmap::DashMap;
use flow_like_bot_tools::browser::{
    BrowserBackend, BrowserScreen, BrowserStep, Download, PageState, ScrollTo, SnapshotRequest,
    SnapshotView, no_owner_browser,
};
use flow_like_bot_tools::computer::{
    ComputerAction, ComputerBackend, PointerButton, Screen, ScrollDirection, WindowBounds,
    WindowOp, WindowSummary,
};
use flow_like_catalog_automation::bot_driver::{
    BotBrowser, BotBrowserOptions, BotComputer, BotInput, BotPointerButton, BotScroll,
    BotScrollDirection, BotStep, BotWindowOp,
};
use flow_like_catalog_automation::computer::window::WindowInfo;
use flow_like_types::interaction_observation::InteractionObservation;
use flow_like_types::{Bytes, Result, anyhow};

use crate::frames::{FrameKind, Shot};
use crate::input::InputEvent;
use crate::playground::Playground;

/// `<data_dir>/bots/<bot id with every character outside [A-Za-z0-9-] replaced by _>`.
pub fn bot_dir(data_dir: &Path, bot_id: &str) -> PathBuf {
    let safe: String = bot_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    data_dir.join("bots").join(safe)
}

/// Where the bot's own Chrome shows up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    /// Nowhere: Chrome runs headless.
    Hidden,
    /// As a window on the bot's virtual display, which the computer view shows whole.
    VirtualDisplay,
    /// As a window on the owner's screen (the desktop app), where the owner's own browser is
    /// at hand too.
    OwnerScreen,
}

pub struct LocalPlayground {
    root: PathBuf,
    surface: Surface,
    browsers: DashMap<String, Arc<LocalBrowser>>,
    computer: Arc<LocalComputer>,
    computer_used: AtomicBool,
}

impl LocalPlayground {
    pub fn new(data_dir: &Path, bot_id: &str, surface: Surface) -> Arc<Self> {
        Arc::new(Self {
            root: bot_dir(data_dir, bot_id),
            surface,
            browsers: DashMap::new(),
            computer: Arc::new(LocalComputer::default()),
            computer_used: AtomicBool::new(false),
        })
    }

    fn running_browsers(&self) -> Vec<Arc<LocalBrowser>> {
        let mut browsers: Vec<_> = self
            .browsers
            .iter()
            .map(|entry| entry.value().clone())
            .collect();
        browsers.sort_by_key(|browser| std::cmp::Reverse(browser.last_activity()));
        browsers
    }

    async fn browser_shot(&self) -> Option<Shot> {
        for browser in self.running_browsers() {
            let Some(running) = browser.running().await else {
                continue;
            };
            let Ok((png, observation)) = running.screenshot_with_observation().await else {
                continue;
            };
            let caption = running
                .location()
                .await
                .map(|(url, _)| url)
                .unwrap_or_default();
            return Some(Shot {
                png,
                kind: FrameKind::Browser,
                caption,
                observation: Some(observation),
            });
        }
        None
    }

    async fn screen_shot(&self) -> Option<Shot> {
        let screen = self.computer.inner.view().await.ok()?;
        Some(Shot {
            png: screen.png,
            kind: FrameKind::Computer,
            caption: "Screen".into(),
            observation: screen.observation,
        })
    }
}

#[async_trait]
impl Playground for LocalPlayground {
    fn browser(&self, conversation: Option<&str>) -> Arc<dyn BrowserBackend> {
        let key = conversation.unwrap_or("").to_string();
        self.browsers
            .entry(key)
            .or_insert_with(|| Arc::new(LocalBrowser::new(&self.root, self.surface)))
            .clone()
    }

    fn computer(&self) -> Arc<dyn ComputerBackend> {
        self.computer_used.store(true, Ordering::Release);
        self.computer.clone()
    }

    async fn capture(&self) -> Result<Option<Shot>> {
        let screen_visible =
            self.surface == Surface::VirtualDisplay && self.computer_used.load(Ordering::Acquire);
        if screen_visible && !self.running_browsers().is_empty() {
            return Ok(self.screen_shot().await);
        }
        let computer_activity = *self
            .computer
            .last_used
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if computer_activity.is_some_and(|at| {
            self.running_browsers()
                .iter()
                .all(|browser| at > browser.last_activity())
        }) {
            return Ok(self.screen_shot().await);
        }
        if let Some(shot) = self.browser_shot().await {
            return Ok(Some(shot));
        }
        if self.computer_used.load(Ordering::Acquire) {
            return Ok(self.screen_shot().await);
        }
        Ok(None)
    }

    async fn inject(&self, event: &InputEvent, width: u32, height: u32) -> Result<()> {
        let input = bot_input(event, width, height)?;
        self.computer.inner.inject(input).await
    }

    async fn stop(&self) {
        self.computer.inner.stop();
        for browser in self.running_browsers() {
            browser.end().await;
        }
        self.browsers.clear();
    }

    async fn sweep(&self, idle: Duration) {
        for browser in self.running_browsers() {
            if browser.idle_for() > idle {
                browser.end().await;
            }
        }
        let entries: Vec<(String, Arc<LocalBrowser>)> = self
            .browsers
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect();
        for (key, browser) in entries {
            if browser.idle_for() > idle && browser.running().await.is_none() {
                self.browsers.remove(&key);
            }
        }
    }
}

fn pointer(button: PointerButton) -> BotPointerButton {
    match button {
        PointerButton::Left => BotPointerButton::Left,
        PointerButton::Right => BotPointerButton::Right,
        PointerButton::Middle => BotPointerButton::Middle,
    }
}

fn bot_input(event: &InputEvent, width: u32, height: u32) -> Result<BotInput> {
    let at = || {
        event
            .normalized(width, height)
            .ok_or_else(|| anyhow!("Input event {event:?} has no position"))
    };
    Ok(match event {
        InputEvent::PointerMove { .. } => {
            let (fx, fy) = at()?;
            BotInput::Move { fx, fy }
        }
        InputEvent::PointerDown { button, .. } | InputEvent::PointerUp { button, .. } => {
            let (fx, fy) = at()?;
            BotInput::Button {
                fx,
                fy,
                button: pointer(*button),
                down: matches!(event, InputEvent::PointerDown { .. }),
            }
        }
        InputEvent::Scroll { dx, dy, .. } => {
            let (fx, fy) = at()?;
            BotInput::Scroll {
                fx,
                fy,
                dx: *dx,
                dy: *dy,
            }
        }
        InputEvent::Key { chord } => BotInput::Chord(chord.clone()),
        InputEvent::Text { text } => BotInput::Text(text.clone()),
    })
}

type BrowserSlot = tokio::sync::Mutex<Option<Arc<BotBrowser>>>;

/// One browser per bot and conversation. Every bot has its own profile folder, so its own
/// Chrome never sees the owner's cookies, logins or history; its own logins survive between
/// sessions. On the owner's screen a session can also move into a tab of the owner's browser,
/// which the tools open only with the owner's approval.
pub struct LocalBrowser {
    profile_dir: PathBuf,
    download_dir: PathBuf,
    surface: Surface,
    own: BrowserSlot,
    owner_tab: BrowserSlot,
    in_owner_browser: AtomicBool,
    last_used: Mutex<Instant>,
}

impl LocalBrowser {
    fn new(root: &Path, surface: Surface) -> Self {
        Self {
            profile_dir: root.join("browser-profile"),
            download_dir: root.join("browser-downloads"),
            surface,
            own: tokio::sync::Mutex::new(None),
            owner_tab: tokio::sync::Mutex::new(None),
            in_owner_browser: AtomicBool::new(false),
            last_used: Mutex::new(Instant::now()),
        }
    }

    fn slot(&self) -> &BrowserSlot {
        if self.in_owner_browser.load(Ordering::Acquire) {
            &self.owner_tab
        } else {
            &self.own
        }
    }

    fn touch(&self) {
        *self.last_used.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
    }

    fn idle_for(&self) -> Duration {
        self.last_activity().elapsed()
    }

    fn last_activity(&self) -> Instant {
        *self.last_used.lock().unwrap_or_else(|e| e.into_inner())
    }

    async fn running(&self) -> Option<Arc<BotBrowser>> {
        self.slot().lock().await.clone().filter(|b| b.is_alive())
    }

    async fn launched(&self) -> Result<Arc<BotBrowser>> {
        let mut slot = self.own.lock().await;
        if let Some(browser) = slot.clone().filter(|b| b.is_alive()) {
            return Ok(browser);
        }
        let browser = Arc::new(
            BotBrowser::launch(BotBrowserOptions {
                profile_dir: self.profile_dir.clone(),
                download_dir: self.download_dir.clone(),
                headless: self.surface == Surface::Hidden,
            })
            .await?,
        );
        *slot = Some(browser.clone());
        Ok(browser)
    }

    async fn attached(&self) -> Result<Arc<BotBrowser>> {
        let mut slot = self.owner_tab.lock().await;
        if let Some(browser) = slot.clone().filter(|b| b.is_alive()) {
            return Ok(browser);
        }
        let browser = Arc::new(BotBrowser::attach_owner().await?);
        *slot = Some(browser.clone());
        Ok(browser)
    }

    async fn opened(browser: &BotBrowser, url: &str) -> Result<PageState> {
        let (url, title) = browser.goto(url).await?;
        Ok(PageState { url, title })
    }

    async fn needed(&self) -> Result<Arc<BotBrowser>> {
        self.touch();
        self.running()
            .await
            .ok_or_else(|| anyhow!("No page is open; call browser_open first"))
    }

    async fn page(browser: &BotBrowser) -> Result<PageState> {
        let (url, title) = browser.location().await?;
        Ok(PageState { url, title })
    }

    async fn end(&self) {
        for slot in [&self.own, &self.owner_tab] {
            if let Some(browser) = slot.lock().await.take() {
                browser.close().await;
            }
        }
        self.in_owner_browser.store(false, Ordering::Release);
    }
}

fn step_of(step: &BrowserStep) -> BotStep {
    match step {
        BrowserStep::Click { reference } => BotStep::Click {
            reference: reference.clone(),
        },
        BrowserStep::Type { reference, text } => BotStep::Type {
            reference: reference.clone(),
            text: text.clone(),
        },
        BrowserStep::Select { reference, value } => BotStep::Select {
            reference: reference.clone(),
            value: value.clone(),
        },
        BrowserStep::Press { reference, key } => BotStep::Press {
            reference: reference.clone(),
            key: key.clone(),
        },
        BrowserStep::Scroll {
            reference,
            direction,
        } => BotStep::Scroll {
            reference: reference.clone(),
            to: match direction {
                ScrollTo::Up => BotScroll::Up,
                ScrollTo::Down => BotScroll::Down,
                ScrollTo::Top => BotScroll::Top,
                ScrollTo::Bottom => BotScroll::Bottom,
                ScrollTo::IntoView => BotScroll::IntoView,
            },
        },
    }
}

#[async_trait]
impl BrowserBackend for LocalBrowser {
    async fn open(&self, url: &str) -> Result<PageState> {
        self.touch();
        let browser = self.launched().await?;
        let page = Self::opened(&browser, url).await?;
        self.in_owner_browser.store(false, Ordering::Release);
        Ok(page)
    }

    async fn open_in_owner_browser(&self, url: &str) -> Result<PageState> {
        if self.surface != Surface::OwnerScreen {
            return Err(no_owner_browser());
        }
        self.touch();
        let browser = self.attached().await?;
        self.touch();
        let page = Self::opened(&browser, url).await?;
        self.in_owner_browser.store(true, Ordering::Release);
        Ok(page)
    }

    async fn snapshot(&self, request: SnapshotRequest) -> Result<SnapshotView> {
        let browser = self.needed().await?;
        let snapshot = browser
            .snapshot(request.interactive_only, request.max_chars)
            .await?;
        Ok(SnapshotView {
            page: PageState {
                url: snapshot.url,
                title: snapshot.title,
            },
            text: snapshot.text,
            truncated: snapshot.truncated,
        })
    }

    async fn act(&self, step: &BrowserStep) -> Result<PageState> {
        let browser = self.needed().await?;
        browser.act(step_of(step)).await?;
        Self::page(&browser).await
    }

    async fn screenshot(&self) -> Result<Vec<u8>> {
        self.needed().await?.screenshot().await
    }

    async fn capture(&self) -> Result<BrowserScreen> {
        let (png, observation) = self.needed().await?.screenshot_with_observation().await?;
        Ok(BrowserScreen {
            png,
            observation: Some(observation),
        })
    }

    async fn observation(&self) -> Result<Option<InteractionObservation>> {
        Ok(Some(self.needed().await?.observation().await?))
    }

    async fn current(&self) -> Result<Option<PageState>> {
        self.touch();
        match self.running().await {
            Some(browser) => Ok(Some(Self::page(&browser).await?)),
            None => Ok(None),
        }
    }

    async fn leave(&self) -> Result<()> {
        self.needed().await?.leave().await?;
        Ok(())
    }

    async fn take_downloads(&self) -> Result<Vec<Download>> {
        let Some(browser) = self.running().await else {
            return Ok(Vec::new());
        };
        let mut files = Vec::new();
        for download in browser.take_downloads().await? {
            let bytes = tokio::fs::read(&download.path).await?;
            let _ = tokio::fs::remove_file(&download.path).await;
            files.push(Download {
                name: download.name,
                bytes: Bytes::from(bytes),
            });
        }
        Ok(files)
    }

    async fn close(&self) -> Result<()> {
        self.end().await;
        Ok(())
    }
}

#[derive(Default)]
pub struct LocalComputer {
    inner: BotComputer,
    last_used: Mutex<Option<Instant>>,
}

impl LocalComputer {
    fn touch(&self) {
        *self.last_used.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
    }
}

fn direction(direction: ScrollDirection) -> BotScrollDirection {
    match direction {
        ScrollDirection::Up => BotScrollDirection::Up,
        ScrollDirection::Down => BotScrollDirection::Down,
        ScrollDirection::Left => BotScrollDirection::Left,
        ScrollDirection::Right => BotScrollDirection::Right,
    }
}

fn window_op(op: WindowOp) -> BotWindowOp {
    match op {
        WindowOp::Focus => BotWindowOp::Focus,
        WindowOp::Close => BotWindowOp::Close,
        WindowOp::Minimize => BotWindowOp::Minimize,
        WindowOp::Maximize => BotWindowOp::Maximize,
        WindowOp::Move => BotWindowOp::Move,
        WindowOp::Resize => BotWindowOp::Resize,
    }
}

fn summary(window: WindowInfo) -> WindowSummary {
    WindowSummary {
        id: window.id,
        title: window.title,
        app: window.app_name,
        x: window.x,
        y: window.y,
        width: window.width,
        height: window.height,
        focused: window.is_focused,
        minimized: window.is_minimized,
    }
}

fn screen(shot: flow_like_catalog_automation::bot_driver::BotScreen) -> Screen {
    let mut screen = Screen::new(shot.png, shot.width, shot.height);
    screen.observation = shot.observation;
    screen
}

#[async_trait]
impl ComputerBackend for LocalComputer {
    async fn screenshot(&self) -> Result<Screen> {
        self.touch();
        Ok(screen(self.inner.screenshot().await?))
    }

    async fn act(&self, action: &ComputerAction) -> Result<String> {
        self.touch();
        let done = match action {
            ComputerAction::Click {
                x,
                y,
                button,
                count,
            } => self.inner.click(*x, *y, pointer(*button), *count).await,
            ComputerAction::Move { x, y } => self.inner.move_to(*x, *y).await,
            ComputerAction::Type { text } => self.inner.type_text(text).await,
            ComputerAction::Key { chord } => self.inner.press(chord).await,
            ComputerAction::Scroll {
                x,
                y,
                direction: toward,
                ticks,
            } => self.inner.scroll(*x, *y, direction(*toward), *ticks).await,
        };
        done.map_err(|error| anyhow!("{error:#}"))
    }

    async fn screenshot_window(&self, window: &str) -> Result<Screen> {
        self.touch();
        Ok(screen(self.inner.screenshot_window(window).await?))
    }

    async fn windows(&self) -> Result<Vec<WindowSummary>> {
        self.touch();
        Ok(self
            .inner
            .windows()
            .await?
            .into_iter()
            .map(summary)
            .collect())
    }

    async fn window(&self, window: &str, op: WindowOp, bounds: WindowBounds) -> Result<String> {
        self.touch();
        self.inner
            .window_action(
                window,
                window_op(op),
                bounds.x,
                bounds.y,
                bounds.width,
                bounds.height,
            )
            .await
    }

    async fn launch(&self, program: &str, args: &[String]) -> Result<String> {
        self.touch();
        self.inner.launch(program, args).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bot_folders_keep_only_safe_characters() {
        let dir = bot_dir(Path::new("/data"), "bot/../1:x");
        assert_eq!(dir, Path::new("/data/bots/bot____1_x"));
    }

    #[test]
    fn owner_input_maps_to_fractions_of_the_screen() {
        let input = bot_input(
            &InputEvent::PointerDown {
                x: 320.0,
                y: 400.0,
                button: PointerButton::Right,
            },
            1280,
            800,
        )
        .unwrap();
        assert_eq!(
            input,
            BotInput::Button {
                fx: 0.25,
                fy: 0.5,
                button: BotPointerButton::Right,
                down: true
            }
        );
        assert_eq!(
            bot_input(
                &InputEvent::Key {
                    chord: "Enter".into()
                },
                1,
                1
            )
            .unwrap(),
            BotInput::Chord("Enter".into())
        );
    }
}
