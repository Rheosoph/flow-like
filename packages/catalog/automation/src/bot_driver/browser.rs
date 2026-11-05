use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use flow_like_browser::launch::{BrowserKind, LaunchOptions, default_cache_dir};
use flow_like_browser::script::ScriptArg;
use flow_like_browser::snapshot::SnapshotOptions;
use flow_like_browser::{Browser, Page};
use flow_like_types::json::json;

use crate::browser::content::SCROLL_SCRIPT;
use crate::browser::driver::{PageContext, click_element, resolve_ref};
use crate::browser::policy::NavigationPolicy;
use crate::browser::refs::{SnapshotRequest, take_snapshot_with};
use crate::types::handles::BrowserSlot;

const WINDOW: (u32, u32) = (1280, 900);
const PAGE_LOAD: Duration = Duration::from_secs(30);
const LAUNCH: Duration = Duration::from_secs(30);
const SCROLL_PAGE_FRACTION: f64 = 0.8;

pub struct BotBrowserOptions {
    pub profile_dir: PathBuf,
    pub download_dir: PathBuf,
    pub headless: bool,
}

pub struct BotSnapshot {
    pub url: String,
    pub title: String,
    pub text: String,
    pub truncated: bool,
}

pub struct BotDownload {
    pub name: String,
    pub path: PathBuf,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BotScroll {
    Up,
    Down,
    Top,
    Bottom,
    IntoView,
}

pub enum BotStep {
    Click {
        reference: String,
    },
    Type {
        reference: String,
        text: String,
    },
    Select {
        reference: String,
        value: String,
    },
    Press {
        reference: Option<String>,
        key: String,
    },
    Scroll {
        reference: Option<String>,
        to: BotScroll,
    },
}

/// A Chrome of its own (separate profile) driven through the nodes' CDP engine.
pub struct BotBrowser {
    browser: Browser,
    slot: Arc<BrowserSlot>,
    page: tokio::sync::Mutex<Page>,
    downloads_since: std::sync::atomic::AtomicU64,
    policy: NavigationPolicy,
}

impl BotBrowser {
    pub async fn launch(options: BotBrowserOptions) -> flow_like_types::Result<Self> {
        std::fs::create_dir_all(&options.profile_dir)?;
        std::fs::create_dir_all(&options.download_dir)?;
        let browser = Browser::launch(LaunchOptions {
            kind: BrowserKind::Chrome,
            executable: None,
            headless: options.headless,
            window_size: WINDOW,
            user_agent: None,
            user_data_dir: Some(options.profile_dir),
            proxy: None,
            locale: None,
            ignore_https_errors: false,
            cache_dir: default_cache_dir(),
            page_load_timeout: PAGE_LOAD,
            launch_timeout: LAUNCH,
        })
        .await?;
        browser
            .set_download_directory(&options.download_dir)
            .await?;
        let page = browser.new_page().await?;
        let since = browser.downloads().cursor();
        Ok(Self {
            browser,
            slot: Arc::new(BrowserSlot::default()),
            page: tokio::sync::Mutex::new(page),
            downloads_since: since.into(),
            policy: NavigationPolicy::plan_default(),
        })
    }

    async fn context(&self) -> flow_like_types::Result<PageContext> {
        let mut page = self.page.lock().await;
        if page.is_closed() {
            let next = self
                .browser
                .pages()
                .into_iter()
                .next()
                .ok_or_else(|| flow_like_types::anyhow!("The browser has no open page"))?;
            *page = self.browser.page(&next.target_id).await?;
        }
        Ok(PageContext::new(
            self.browser.clone(),
            page.clone(),
            self.slot.clone(),
            Vec::new(),
        ))
    }

    pub fn is_alive(&self) -> bool {
        self.browser.is_alive()
    }

    pub async fn location(&self) -> flow_like_types::Result<(String, String)> {
        let ctx = self.context().await?;
        Ok((
            ctx.page.url().await?,
            ctx.page.title().await.unwrap_or_default(),
        ))
    }

    pub async fn goto(&self, url: &str) -> flow_like_types::Result<(String, String)> {
        let target = self.policy.check(url).await?;
        let ctx = self.context().await?;
        ctx.page.goto(target.as_str()).await?;
        self.location().await
    }

    pub async fn leave(&self) -> flow_like_types::Result<()> {
        let ctx = self.context().await?;
        ctx.page.goto("about:blank").await?;
        Ok(())
    }

    pub async fn snapshot(
        &self,
        interactive_only: bool,
        max_chars: usize,
    ) -> flow_like_types::Result<BotSnapshot> {
        let ctx = self.context().await?;
        let request = SnapshotRequest {
            interactive_only,
            max_depth: None,
            scope_ref: None,
            max_chars,
            detect_clickable: true,
        };
        let output = take_snapshot_with(&ctx, &request, |warning| {
            tracing::debug!("bot browser snapshot: {warning}")
        })
        .await?;
        Ok(BotSnapshot {
            url: output.url,
            title: output.title,
            text: output.text,
            truncated: output.truncated,
        })
    }

    pub async fn act(&self, step: BotStep) -> flow_like_types::Result<()> {
        let ctx = self.context().await?;
        match step {
            BotStep::Click { reference } => {
                let element = resolve_ref(&ctx, &reference).await?;
                click_element(&ctx, &element, "left", &[], 1).await?;
            }
            BotStep::Type { reference, text } => {
                let element = resolve_ref(&ctx, &reference).await?;
                element.clear().await?;
                element.send_keys(&text).await?;
            }
            BotStep::Select { reference, value } => {
                let element = resolve_ref(&ctx, &reference).await?;
                if element.select_by_value(&value).await.is_err() {
                    element.select_by_exact_text(&value).await?;
                }
            }
            BotStep::Press { reference, key } => {
                if let Some(reference) = reference {
                    resolve_ref(&ctx, &reference).await?.focus().await?;
                }
                let (modifiers, key) = split_chord(&key);
                crate::browser::driver::key_chord(&ctx, key, &modifiers).await?;
            }
            BotStep::Scroll { reference, to } => self.scroll(&ctx, reference, to).await?,
        }
        Ok(())
    }

    async fn scroll(
        &self,
        ctx: &PageContext,
        reference: Option<String>,
        to: BotScroll,
    ) -> flow_like_types::Result<()> {
        if let Some(reference) = reference
            && to == BotScroll::IntoView
        {
            resolve_ref(ctx, &reference)
                .await?
                .scroll_into_view()
                .await?;
            return Ok(());
        }
        let (mode, dy) = match to {
            BotScroll::Top => ("top", 0.0),
            BotScroll::Bottom => ("bottom", 0.0),
            BotScroll::Up => ("by", -self.viewport_step(ctx).await),
            BotScroll::Down | BotScroll::IntoView => ("by", self.viewport_step(ctx).await),
        };
        ctx.execute(
            SCROLL_SCRIPT,
            vec![
                ScriptArg::Json(json!(null)),
                ScriptArg::Json(json!(mode)),
                ScriptArg::Json(json!(0.0)),
                ScriptArg::Json(json!(dy)),
            ],
        )
        .await?;
        Ok(())
    }

    async fn viewport_step(&self, ctx: &PageContext) -> f64 {
        ctx.probe("return window.innerHeight;", Vec::new())
            .await
            .ok()
            .and_then(|value| value.json().as_f64())
            .unwrap_or(WINDOW.1 as f64)
            * SCROLL_PAGE_FRACTION
    }

    pub async fn screenshot(&self) -> flow_like_types::Result<Vec<u8>> {
        let ctx = self.context().await?;
        Ok(ctx.page.screenshot(Default::default()).await?)
    }

    pub async fn take_downloads(&self) -> flow_like_types::Result<Vec<BotDownload>> {
        use std::sync::atomic::Ordering;
        let tracker = self.browser.downloads();
        let mut found = Vec::new();
        loop {
            let since = self.downloads_since.load(Ordering::Acquire);
            let deadline = tokio::time::Instant::now();
            let Some(record) = tracker
                .wait_finished(since, false, deadline, |_| true)
                .await?
            else {
                break;
            };
            self.downloads_since
                .store(record.seq.max(since + 1), Ordering::Release);
            if let Some(path) = record.final_path {
                found.push(BotDownload {
                    name: record.suggested_filename,
                    path,
                });
            }
        }
        Ok(found)
    }

    pub async fn close(&self) {
        self.browser.shutdown().await;
    }

    pub fn abort(&self) {
        self.browser.abort();
    }
}

fn split_chord(chord: &str) -> (Vec<String>, &str) {
    let chord = chord.trim();
    if chord == "+" {
        return (Vec::new(), chord);
    }
    match chord.rsplit_once('+') {
        Some((head, key)) if !key.is_empty() => (
            head.split('+').map(|m| m.trim().to_string()).collect(),
            key.trim(),
        ),
        _ => (Vec::new(), chord),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ref_of(snapshot: &str, role_and_name: &str) -> String {
        let line = snapshot
            .lines()
            .find(|line| line.contains(role_and_name))
            .unwrap_or_else(|| panic!("no '{role_and_name}' in:\n{snapshot}"));
        let start = line.find("[ref=").expect("a ref") + "[ref=".len();
        line[start..]
            .split(']')
            .next()
            .expect("a ref end")
            .to_string()
    }

    async fn serve(body: &'static str) -> u16 {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                tokio::spawn(async move {
                    let mut buffer = [0u8; 2048];
                    let _ = stream.read(&mut buffer).await;
                    let reply = format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(reply.as_bytes()).await;
                });
            }
        });
        port
    }

    #[tokio::test]
    #[ignore = "launches a real Chrome; run with --ignored"]
    async fn a_real_browser_session_reads_types_clicks_and_shoots() {
        let port = serve(
            "<html><head><title>Shop</title></head><body><input aria-label=\"Name\">\
             <button onclick=\"document.title='clicked'\">Go</button></body></html>",
        )
        .await;
        let dir = std::env::temp_dir().join(format!("bot-browser-test-{port}"));
        let browser = BotBrowser::launch(BotBrowserOptions {
            profile_dir: dir.join("profile"),
            download_dir: dir.join("downloads"),
            headless: true,
        })
        .await
        .expect("Chrome starts");
        let (url, title) = browser
            .goto(&format!("http://127.0.0.1:{port}/"))
            .await
            .unwrap();
        assert!(url.starts_with("http://127.0.0.1"), "{url}");
        assert_eq!(title, "Shop");

        let snapshot = browser.snapshot(true, 5000).await.unwrap();
        let field = ref_of(&snapshot.text, "textbox \"Name\"");
        let button = ref_of(&snapshot.text, "button \"Go\"");
        browser
            .act(BotStep::Type {
                reference: field,
                text: "Ada".into(),
            })
            .await
            .unwrap();
        browser
            .act(BotStep::Click { reference: button })
            .await
            .unwrap();
        assert_eq!(browser.location().await.unwrap().1, "clicked");
        assert!(
            browser
                .snapshot(false, 5000)
                .await
                .unwrap()
                .text
                .contains("Ada")
        );
        assert!(browser.screenshot().await.unwrap().starts_with(b"\x89PNG"));
        assert!(browser.take_downloads().await.unwrap().is_empty());
        browser.close().await;
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_local_scheme_never_navigates() {
        let policy = NavigationPolicy::plan_default();
        for url in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "chrome://settings",
        ] {
            assert!(policy.check(url).await.is_err(), "{url}");
        }
        assert!(policy.check("https://example.com/").await.is_ok());
    }

    #[test]
    fn chords_split_into_modifiers_and_key() {
        assert_eq!(split_chord("Enter"), (vec![], "Enter"));
        assert_eq!(
            split_chord("Control+Shift+a"),
            (vec!["Control".to_string(), "Shift".to_string()], "a")
        );
        assert_eq!(split_chord("+"), (vec![], "+"));
    }
}
