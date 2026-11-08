use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use flow_like_browser::launch::{BrowserKind, LaunchOptions, default_cache_dir};
use flow_like_browser::script::ScriptArg;
use flow_like_browser::{Browser, Element, Page};
use flow_like_types::interaction_observation::*;
use flow_like_types::json::json;
use serde::Deserialize;

use crate::browser::content::SCROLL_SCRIPT;
use crate::browser::driver::{PageContext, resolve_ref};
use crate::browser::policy::NavigationPolicy;
use crate::browser::refs::{SnapshotRequest, take_snapshot_with};
use crate::types::handles::BrowserSlot;

const WINDOW: (u32, u32) = (1280, 900);
const PAGE_LOAD: Duration = Duration::from_secs(30);
const LAUNCH: Duration = Duration::from_secs(30);
const SCROLL_PAGE_FRACTION: f64 = 0.8;

// Read labels, never the active element's value or text content. Cross-origin frames remain
// represented by their iframe element because their inner geometry cannot be read here.
const VIEW_PROBE: &str = r#"
let element = document.activeElement;
while (element?.shadowRoot?.activeElement) element = element.shadowRoot.activeElement;
const rect = element && element !== document.body && element !== document.documentElement
    ? element.getBoundingClientRect() : null;
const focus = rect ? {
    label: (element.getAttribute('aria-label') || element.getAttribute('title') ||
        element.getAttribute('role') || element.tagName.toLowerCase()).slice(0, 160),
    bounds: {x: rect.x, y: rect.y, width: rect.width, height: rect.height}
} : null;
const visual = window.visualViewport;
return {url: location.href, width: innerWidth, height: innerHeight,
    scroll_x: scrollX, scroll_y: scrollY, scale: visual?.scale || 1,
    offset_x: visual?.offsetLeft || 0, offset_y: visual?.offsetTop || 0,
    focus};
"#;

const TARGET_PROBE: &str = r#"
if (window !== window.top) return null;
const rect = arguments[0].getBoundingClientRect();
return {x: rect.x, y: rect.y, width: rect.width, height: rect.height};
"#;

#[derive(Debug, Clone, Deserialize, PartialEq)]
struct ViewStamp {
    url: String,
    width: u32,
    height: u32,
    scroll_x: f64,
    scroll_y: f64,
    scale: f64,
    offset_x: f64,
    offset_y: f64,
    #[serde(skip)]
    page: String,
    #[serde(skip)]
    loader: Option<String>,
}

#[derive(Deserialize)]
struct ViewProbe {
    #[serde(flatten)]
    stamp: ViewStamp,
    focus: Option<InteractionFocus>,
}

#[derive(Default)]
struct Feedback {
    stamp: Option<ViewStamp>,
    action: Option<InteractionAction>,
    pointer: Option<InteractionPointer>,
}

impl Feedback {
    fn begin(&mut self, kind: &str) {
        self.stamp = None;
        self.pointer = None;
        self.action = Some(InteractionAction {
            kind: kind.into(),
            phase: InteractionPhase::Running,
            at_ms: timestamp_ms(),
            target: None,
            target_bounds: None,
        });
    }

    fn finish(&mut self, succeeded: bool) {
        if let Some(action) = &mut self.action {
            action.at_ms = timestamp_ms();
            action.phase = if succeeded {
                InteractionPhase::Completed
            } else {
                InteractionPhase::Failed
            };
        }
    }

    fn validate(&mut self, stamp: &ViewStamp) {
        if self.stamp.as_ref() != Some(stamp) {
            self.clear_geometry();
        }
    }

    fn clear_geometry(&mut self) {
        self.pointer = None;
        if let Some(action) = &mut self.action {
            action.target = None;
            action.target_bounds = None;
        }
        self.stamp = None;
    }
}

fn timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

fn normalized_rect(rect: InteractionRect, stamp: &ViewStamp) -> Option<InteractionRect> {
    if stamp.width == 0
        || stamp.height == 0
        || ![rect.x, rect.y, rect.width, rect.height]
            .iter()
            .all(|n| n.is_finite())
    {
        return None;
    }
    let width = f64::from(stamp.width);
    let height = f64::from(stamp.height);
    let left = rect.x.clamp(0.0, width);
    let top = rect.y.clamp(0.0, height);
    let right = (rect.x + rect.width).clamp(0.0, width);
    let bottom = (rect.y + rect.height).clamp(0.0, height);
    (right > left && bottom > top).then_some(InteractionRect {
        x: left / width,
        y: top / height,
        width: (right - left) / width,
        height: (bottom - top) / height,
    })
}

fn normalized_point(point: (f64, f64), stamp: &ViewStamp) -> Option<InteractionPoint> {
    let (x, y) = (
        point.0 / f64::from(stamp.width),
        point.1 / f64::from(stamp.height),
    );
    (x.is_finite() && y.is_finite() && (0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y))
        .then_some(InteractionPoint { x, y })
}

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
    feedback: tokio::sync::Mutex<Feedback>,
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
            feedback: tokio::sync::Mutex::new(Feedback::default()),
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
        let mut feedback = self.feedback.lock().await;
        feedback.begin("navigate");
        let result = self.goto_inner(url).await;
        feedback.finish(result.is_ok());
        result
    }

    async fn goto_inner(&self, url: &str) -> flow_like_types::Result<(String, String)> {
        let target = self.policy.check(url).await?;
        let ctx = self.context().await?;
        ctx.page.goto(target.as_str()).await?;
        self.location().await
    }

    pub async fn leave(&self) -> flow_like_types::Result<()> {
        let mut feedback = self.feedback.lock().await;
        feedback.begin("navigate");
        let result: flow_like_types::Result<()> = async {
            let ctx = self.context().await?;
            ctx.page.goto("about:blank").await?;
            Ok(())
        }
        .await;
        feedback.finish(result.is_ok());
        result
    }

    pub async fn snapshot(
        &self,
        interactive_only: bool,
        max_chars: usize,
    ) -> flow_like_types::Result<BotSnapshot> {
        let _feedback = self.feedback.lock().await;
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
        let mut feedback = self.feedback.lock().await;
        feedback.begin(match &step {
            BotStep::Click { .. } => "click",
            BotStep::Type { .. } => "type",
            BotStep::Select { .. } => "select",
            BotStep::Press { .. } => "press",
            BotStep::Scroll { .. } => "scroll",
        });
        let result = self.act_inner(step, &mut feedback).await;
        feedback.finish(result.is_ok());
        result
    }

    async fn act_inner(
        &self,
        step: BotStep,
        feedback: &mut Feedback,
    ) -> flow_like_types::Result<()> {
        let ctx = self.context().await?;
        match step {
            BotStep::Click { reference } => {
                let element = resolve_ref(&ctx, &reference).await?;
                element.scroll_into_view().await?;
                self.record_target(&ctx, &element, feedback).await;
                let (_, point) = element.element_click_with_point().await?;
                if let Some(stamp) = &feedback.stamp {
                    feedback.pointer =
                        point
                            .and_then(|point| normalized_point(point, stamp))
                            .map(|position| InteractionPointer {
                                position,
                                source: PointerSource::Dispatched,
                            });
                }
            }
            BotStep::Type { reference, text } => {
                let element = resolve_ref(&ctx, &reference).await?;
                self.record_target(&ctx, &element, feedback).await;
                element.clear().await?;
                element.send_keys(&text).await?;
            }
            BotStep::Select { reference, value } => {
                let element = resolve_ref(&ctx, &reference).await?;
                self.record_target(&ctx, &element, feedback).await;
                if element.select_by_value(&value).await.is_err() {
                    element.select_by_exact_text(&value).await?;
                }
            }
            BotStep::Press { reference, key } => {
                if let Some(reference) = reference {
                    let element = resolve_ref(&ctx, &reference).await?;
                    self.record_target(&ctx, &element, feedback).await;
                    element.focus().await?;
                }
                let (modifiers, key) = split_chord(&key);
                crate::browser::driver::key_chord(&ctx, key, &modifiers).await?;
            }
            BotStep::Scroll { reference, to } => self.scroll(&ctx, reference, to).await?,
        }
        Ok(())
    }

    async fn probe_view(ctx: &PageContext) -> flow_like_types::Result<ViewProbe> {
        let loader_before = ctx.page.main_frame().loader_id();
        let value = ctx.probe(VIEW_PROBE, Vec::new()).await?;
        let mut probe: ViewProbe = flow_like_types::json::from_value(value.json().clone())?;
        probe.stamp.page = ctx.page.target_id().to_string();
        probe.stamp.loader = ctx.page.main_frame().loader_id().map(|id| id.to_string());
        if loader_before.map(|id| id.to_string()) != probe.stamp.loader {
            return Err(flow_like_types::anyhow!(
                "The browser navigated while reading interaction feedback"
            ));
        }
        if probe.stamp.width == 0 || probe.stamp.height == 0 {
            return Err(flow_like_types::anyhow!("The browser viewport has no size"));
        }
        if let Some(focus) = &mut probe.focus {
            focus.bounds = focus
                .bounds
                .and_then(|bounds| normalized_rect(bounds, &probe.stamp));
        }
        Ok(probe)
    }

    async fn record_target(&self, ctx: &PageContext, element: &Element, feedback: &mut Feedback) {
        let Ok(probe) = Self::probe_view(ctx).await else {
            return;
        };
        let bounds = ctx
            .probe(TARGET_PROBE, vec![ScriptArg::Element(element.clone())])
            .await
            .ok()
            .and_then(|value| flow_like_types::json::from_value(value.json().clone()).ok())
            .and_then(|rect| normalized_rect(rect, &probe.stamp));
        if let Some(action) = &mut feedback.action {
            action.target_bounds = bounds;
            action.target = bounds.map(|rect| InteractionPoint {
                x: rect.x + rect.width / 2.0,
                y: rect.y + rect.height / 2.0,
            });
        }
        feedback.stamp = Some(probe.stamp);
    }

    fn observed(probe: ViewProbe, feedback: &mut Feedback) -> InteractionObservation {
        feedback.validate(&probe.stamp);
        InteractionObservation {
            captured_at_ms: timestamp_ms(),
            width: probe.stamp.width,
            height: probe.stamp.height,
            pointer: feedback.pointer.clone(),
            focus: probe.focus,
            action: feedback.action.clone(),
            mapping: None,
        }
    }

    pub async fn observation(&self) -> flow_like_types::Result<InteractionObservation> {
        let mut feedback = self.feedback.lock().await;
        let ctx = self.context().await?;
        Ok(Self::observed(Self::probe_view(&ctx).await?, &mut feedback))
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
        self.screenshot_with_observation().await.map(|(png, _)| png)
    }

    /// Captures pixels and feedback while bot actions are excluded. A page that navigates or
    /// changes its viewport during capture gets another capture before metadata is attached.
    pub async fn screenshot_with_observation(
        &self,
    ) -> flow_like_types::Result<(Vec<u8>, InteractionObservation)> {
        let mut feedback = self.feedback.lock().await;
        let ctx = self.context().await?;
        for attempt in 0..2 {
            let before = Self::probe_view(&ctx).await.ok();
            let png = ctx.page.screenshot(Default::default()).await?;
            let after = Self::probe_view(&ctx).await.ok();
            let dimensions = image::ImageReader::with_format(
                std::io::Cursor::new(&png),
                image::ImageFormat::Png,
            )
            .into_dimensions()?;
            if let (Some(before), Some(after)) = (before.as_ref(), after.as_ref())
                && (before.stamp != after.stamp || before.focus != after.focus)
                && attempt == 0
            {
                continue;
            }
            if let (Some(before), Some(after)) = (before, after)
                && before.stamp == after.stamp
                && before.focus == after.focus
            {
                let mut observation = Self::observed(after, &mut feedback);
                observation.width = dimensions.0;
                observation.height = dimensions.1;
                return Ok((png, observation));
            }
            // The image remains useful if a dialog or a navigation prevented a metadata probe.
            // Omit geometry rather than attaching positions from another capture.
            feedback.clear_geometry();
            return Ok((
                png,
                InteractionObservation {
                    captured_at_ms: timestamp_ms(),
                    width: dimensions.0,
                    height: dimensions.1,
                    pointer: None,
                    focus: None,
                    action: feedback.action.clone(),
                    mapping: None,
                },
            ));
        }
        unreachable!("the last capture returns with or without geometry")
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

    fn stamp() -> ViewStamp {
        ViewStamp {
            page: "page-1".into(),
            loader: Some("document-1".into()),
            url: "https://example.com/".into(),
            width: 1280,
            height: 900,
            scroll_x: 0.0,
            scroll_y: 0.0,
            scale: 1.0,
            offset_x: 0.0,
            offset_y: 0.0,
        }
    }

    #[test]
    fn navigation_or_viewport_changes_remove_old_click_positions() {
        let initial = stamp();
        let variants = [
            ViewStamp {
                loader: Some("document-2".into()),
                ..initial.clone()
            },
            ViewStamp {
                page: "page-2".into(),
                ..initial.clone()
            },
            ViewStamp {
                url: "https://example.com/#other".into(),
                ..initial.clone()
            },
            ViewStamp {
                width: 900,
                ..initial.clone()
            },
            ViewStamp {
                scroll_y: 50.0,
                ..initial.clone()
            },
            ViewStamp {
                scale: 2.0,
                ..initial.clone()
            },
        ];
        for next in variants {
            let mut feedback = Feedback::default();
            feedback.begin("click");
            feedback.stamp = Some(initial.clone());
            let position = InteractionPoint { x: 0.3, y: 0.4 };
            feedback.pointer = Some(InteractionPointer {
                position,
                source: PointerSource::Dispatched,
            });
            feedback.action.as_mut().unwrap().target = Some(position);
            feedback.action.as_mut().unwrap().target_bounds = Some(InteractionRect {
                x: 0.2,
                y: 0.3,
                width: 0.2,
                height: 0.2,
            });
            feedback.finish(true);
            let observed = BotBrowser::observed(
                ViewProbe {
                    stamp: next,
                    focus: None,
                },
                &mut feedback,
            );
            assert!(observed.pointer.is_none());
            let action = observed.action.unwrap();
            assert_eq!(action.phase, InteractionPhase::Completed);
            assert!(action.target.is_none());
            assert!(action.target_bounds.is_none());
        }
    }

    #[test]
    fn normalized_feedback_clips_visible_bounds_and_rejects_invalid_positions() {
        assert_eq!(
            normalized_rect(
                InteractionRect {
                    x: -10.0,
                    y: 450.0,
                    width: 138.0,
                    height: 500.0,
                },
                &stamp()
            ),
            Some(InteractionRect {
                x: 0.0,
                y: 0.5,
                width: 0.1,
                height: 0.5
            })
        );
        assert!(normalized_point((f64::NAN, 4.0), &stamp()).is_none());
        assert!(normalized_point((1281.0, 4.0), &stamp()).is_none());
    }

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
                text: "Ada secret value".into(),
            })
            .await
            .unwrap();
        let typed = browser.observation().await.unwrap();
        assert_eq!(typed.focus.as_ref().unwrap().label, "Name");
        assert!(typed.focus.as_ref().unwrap().bounds.is_some());
        assert!(
            !flow_like_types::json::to_string(&typed)
                .unwrap()
                .contains("secret value")
        );
        assert!(typed.pointer.is_none());
        browser
            .act(BotStep::Click { reference: button })
            .await
            .unwrap();
        assert_eq!(browser.location().await.unwrap().1, "clicked");
        let (image, clicked) = browser.screenshot_with_observation().await.unwrap();
        assert!(image.starts_with(b"\x89PNG"));
        assert_eq!(clicked.pointer.unwrap().source, PointerSource::Dispatched);
        assert_eq!(clicked.action.unwrap().phase, InteractionPhase::Completed);
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
        browser
            .goto(&format!("http://127.0.0.1:{port}/next"))
            .await
            .unwrap();
        let navigated = browser.observation().await.unwrap();
        assert!(navigated.pointer.is_none());
        assert!(navigated.action.unwrap().target.is_none());
        browser.close().await;
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    #[ignore = "launches a real Chrome; run with --ignored"]
    async fn a_covered_button_fails_without_claiming_a_dispatched_click() {
        let port = serve(
            "<html><head><title>Unclicked</title></head><body>\
             <button onclick=\"document.title='clicked'\">Go</button>\
             <div style=\"position:fixed;inset:0;z-index:10\"></div></body></html>",
        )
        .await;
        let dir = std::env::temp_dir().join(format!("bot-browser-blocked-{port}"));
        let browser = BotBrowser::launch(BotBrowserOptions {
            profile_dir: dir.join("profile"),
            download_dir: dir.join("downloads"),
            headless: true,
        })
        .await
        .unwrap();
        browser
            .goto(&format!("http://127.0.0.1:{port}/"))
            .await
            .unwrap();
        let snapshot = browser.snapshot(true, 5000).await.unwrap();
        let error = browser
            .act(BotStep::Click {
                reference: ref_of(&snapshot.text, "button \"Go\""),
            })
            .await
            .unwrap_err();
        assert!(error.to_string().contains("intercepted"), "{error}");
        let observation = browser.observation().await.unwrap();
        assert_eq!(
            observation.action.as_ref().unwrap().phase,
            InteractionPhase::Failed
        );
        assert!(observation.action.unwrap().target.is_some());
        assert!(observation.pointer.is_none());
        assert_eq!(browser.location().await.unwrap().1, "Unclicked");
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
