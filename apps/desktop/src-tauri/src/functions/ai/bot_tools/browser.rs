use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex, Once};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use dashmap::DashMap;
use flow_like_types::{Bytes, Result, anyhow};
use flow_like_bot_tools::browser::{
    BrowserBackend, BrowserStep, Download, PageState, ScrollTo, SnapshotRequest, SnapshotView,
};
use flow_like_catalog_automation::bot_driver::{BotBrowser, BotBrowserOptions, BotScroll, BotStep};
use tauri::{AppHandle, Manager};

use super::view::{self, FrameKind};

const IDLE_AFTER: Duration = Duration::from_secs(10 * 60);
const SWEEP_EVERY: Duration = Duration::from_secs(30);

static SESSIONS: LazyLock<DashMap<String, Arc<DesktopBrowser>>> = LazyLock::new(DashMap::new);
static SWEEPER: Once = Once::new();

/// One browser per bot and conversation. Every bot has its own profile folder, so it never
/// sees the owner's cookies, logins or history; its own logins survive between sessions.
pub(crate) struct DesktopBrowser {
    key: String,
    bot_id: String,
    profile_dir: PathBuf,
    download_dir: PathBuf,
    browser: tokio::sync::Mutex<Option<Arc<BotBrowser>>>,
    last_used: Mutex<Instant>,
}

pub(super) fn session(
    app: &AppHandle,
    bot_id: &str,
    conversation_id: Option<&str>,
) -> Arc<DesktopBrowser> {
    start_sweeper();
    let key = format!("{bot_id}\u{1f}{}", conversation_id.unwrap_or(""));
    let session = SESSIONS
        .entry(key.clone())
        .or_insert_with(|| Arc::new(DesktopBrowser::new(app, key, bot_id)))
        .clone();
    session.touch();
    session
}

fn bot_dir(app: &AppHandle, bot_id: &str) -> PathBuf {
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
    app.path()
        .app_local_data_dir()
        .unwrap_or_else(|_| std::env::temp_dir().join("flow-like"))
        .join("bots")
        .join(safe)
}

fn start_sweeper() {
    SWEEPER.call_once(|| {
        tauri::async_runtime::spawn(async {
            loop {
                tokio::time::sleep(SWEEP_EVERY).await;
                let idle: Vec<Arc<DesktopBrowser>> = SESSIONS
                    .iter()
                    .filter(|entry| entry.value().idle_for() > IDLE_AFTER)
                    .map(|entry| entry.value().clone())
                    .collect();
                for session in idle {
                    session.end().await;
                }
            }
        });
    });
}

pub(super) async fn stop(bot_id: &str) {
    let sessions: Vec<Arc<DesktopBrowser>> = SESSIONS
        .iter()
        .filter(|entry| entry.value().bot_id == bot_id)
        .map(|entry| entry.value().clone())
        .collect();
    for session in sessions {
        session.end().await;
    }
}

pub(super) async fn refresh_view(bot_id: &str) {
    if view::is_fresh(bot_id) {
        return;
    }
    let sessions: Vec<Arc<DesktopBrowser>> = SESSIONS
        .iter()
        .filter(|entry| entry.value().bot_id == bot_id)
        .map(|entry| entry.value().clone())
        .collect();
    for session in sessions {
        if session.publish_frame().await {
            return;
        }
    }
}

impl DesktopBrowser {
    fn new(app: &AppHandle, key: String, bot_id: &str) -> Self {
        let root = bot_dir(app, bot_id);
        Self {
            key,
            bot_id: bot_id.to_string(),
            profile_dir: root.join("browser-profile"),
            download_dir: root.join("browser-downloads"),
            browser: tokio::sync::Mutex::new(None),
            last_used: Mutex::new(Instant::now()),
        }
    }

    fn touch(&self) {
        *self.last_used.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
    }

    fn idle_for(&self) -> Duration {
        self.last_used
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .elapsed()
    }

    async fn running(&self) -> Option<Arc<BotBrowser>> {
        self.browser.lock().await.clone().filter(|b| b.is_alive())
    }

    async fn launched(&self) -> Result<Arc<BotBrowser>> {
        let mut slot = self.browser.lock().await;
        if let Some(browser) = slot.clone().filter(|b| b.is_alive()) {
            return Ok(browser);
        }
        let browser = Arc::new(
            BotBrowser::launch(BotBrowserOptions {
                profile_dir: self.profile_dir.clone(),
                download_dir: self.download_dir.clone(),
                headless: true,
            })
            .await?,
        );
        *slot = Some(browser.clone());
        Ok(browser)
    }

    async fn needed(&self) -> Result<Arc<BotBrowser>> {
        self.touch();
        self.running()
            .await
            .ok_or_else(|| anyhow!("No page is open; call browser_open first"))
    }

    async fn publish_frame(&self) -> bool {
        let Some(browser) = self.running().await else {
            return false;
        };
        let Ok(png) = browser.screenshot().await else {
            return false;
        };
        let caption = browser
            .location()
            .await
            .map(|(url, _)| url)
            .unwrap_or_default();
        view::publish(&self.bot_id, FrameKind::Browser, &caption, png);
        true
    }

    async fn state(&self, browser: &BotBrowser) -> Result<PageState> {
        let (url, title) = browser.location().await?;
        self.publish_frame().await;
        Ok(PageState { url, title })
    }

    async fn end(&self) {
        let browser = self.browser.lock().await.take();
        if let Some(browser) = browser {
            browser.close().await;
        }
        SESSIONS.remove(&self.key);
        if !SESSIONS
            .iter()
            .any(|entry| entry.value().bot_id == self.bot_id)
        {
            view::forget(&self.bot_id);
        }
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
impl BrowserBackend for DesktopBrowser {
    async fn open(&self, url: &str) -> Result<PageState> {
        self.touch();
        let browser = self.launched().await?;
        browser.goto(url).await?;
        self.state(&browser).await
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
        self.state(&browser).await
    }

    async fn screenshot(&self) -> Result<Vec<u8>> {
        let browser = self.needed().await?;
        let png = browser.screenshot().await?;
        let caption = browser
            .location()
            .await
            .map(|(url, _)| url)
            .unwrap_or_default();
        view::publish(&self.bot_id, FrameKind::Browser, &caption, png.clone());
        Ok(png)
    }

    async fn current(&self) -> Result<Option<PageState>> {
        self.touch();
        let Some(browser) = self.running().await else {
            return Ok(None);
        };
        let (url, title) = browser.location().await?;
        Ok(Some(PageState { url, title }))
    }

    async fn leave(&self) -> Result<()> {
        let browser = self.needed().await?;
        browser.leave().await?;
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
