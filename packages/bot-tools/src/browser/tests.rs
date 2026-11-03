use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use flow_like_model_provider::llm::ModelLogic;
use flow_like_types::{Bytes, Result, anyhow};
use serde_json::{Value, json};

use super::*;
use crate::computer::ComputerBackend;
use crate::embed::KnowledgeEmbedder;
use crate::env::{
    AppTables, Attachment, Environment, FileData, FileEntry, FileMeta, Limits, ModelRole, Scope,
    SessionCache, ToolEnv,
};
use crate::testing::FakeEnv;
use crate::{Status, ToolOutcome, run};

pub(crate) struct CapEnv {
    pub inner: FakeEnv,
    pub browser: Option<Arc<dyn BrowserBackend>>,
    pub computer: Option<Arc<dyn ComputerBackend>>,
}

impl CapEnv {
    pub async fn new() -> Self {
        Self {
            inner: FakeEnv::new().await,
            browser: None,
            computer: None,
        }
    }

    pub async fn with_browser(browser: Arc<FakeBrowser>) -> Self {
        let mut env = Self::new().await;
        env.browser = Some(browser);
        env
    }
}

#[async_trait]
impl ToolEnv for CapEnv {
    fn environment(&self) -> Environment {
        self.inner.environment()
    }

    fn scope(&self) -> &Scope {
        self.inner.scope()
    }

    fn limits(&self) -> &Limits {
        self.inner.limits()
    }

    fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::SeqCst)
    }

    fn sessions(&self) -> Option<Arc<dyn SessionCache>> {
        self.inner.sessions()
    }

    fn browser(&self) -> Option<Arc<dyn BrowserBackend>> {
        self.browser.clone()
    }

    fn computer(&self) -> Option<Arc<dyn ComputerBackend>> {
        self.computer.clone()
    }

    async fn read_file(&self, a: &str, p: &str, m: u64) -> Result<Option<FileData>> {
        self.inner.read_file(a, p, m).await
    }

    async fn stat_file(&self, a: &str, p: &str) -> Result<Option<FileMeta>> {
        self.inner.stat_file(a, p).await
    }

    async fn list_files(&self, a: &str, p: &str, l: usize) -> Result<Vec<FileEntry>> {
        self.inner.list_files(a, p, l).await
    }

    async fn write_workspace(&self, path: &str, data: Bytes, mime: &str) -> Result<()> {
        self.inner.write_workspace(path, data, mime).await
    }

    async fn message_attachment(&self, s: u64, p: &str) -> Result<Option<Attachment>> {
        self.inner.message_attachment(s, p).await
    }

    async fn app_tables(&self, app_id: &str) -> Result<AppTables> {
        self.inner.app_tables(app_id).await
    }

    async fn model(&self, role: ModelRole) -> Result<Arc<dyn ModelLogic>> {
        self.inner.model(role).await
    }

    async fn embedder(&self) -> Result<Option<Arc<dyn KnowledgeEmbedder>>> {
        self.inner.embedder().await
    }

    async fn secret(&self, name: &str) -> Result<Option<String>> {
        self.inner.secret(name).await
    }
}

struct Page {
    title: &'static str,
    body: &'static str,
}

/// Pages by address; a click on `go:<url>` follows the link, typed text shows in the field `e2`.
#[derive(Default)]
pub(crate) struct FakeBrowser {
    current: Mutex<Option<String>>,
    typed: Mutex<String>,
    pub steps: Mutex<Vec<BrowserStep>>,
    pub opened: Mutex<Vec<String>>,
    pub left: Mutex<u32>,
    pub closed: Mutex<u32>,
    pub downloads: Mutex<Vec<Download>>,
    pub redirects: Mutex<Vec<(String, String)>>,
    pub body_override: Mutex<Option<String>>,
}

impl FakeBrowser {
    fn page(url: &str) -> Page {
        match url {
            "https://shop.example/" => Page {
                title: "Shop",
                body: "link \"Partner\" [ref=e1]\ntextbox \"Password\" [ref=e2]",
            },
            "https://partner.example/" => Page {
                title: "Partner",
                body: "heading \"Partner site\"",
            },
            _ => Page {
                title: "Unknown",
                body: "text",
            },
        }
    }

    fn here(&self) -> Option<String> {
        self.current.lock().unwrap().clone()
    }

    fn state(&self, url: String) -> PageState {
        PageState {
            title: Self::page(&url).title.to_string(),
            url,
        }
    }
}

#[async_trait]
impl BrowserBackend for FakeBrowser {
    async fn open(&self, url: &str) -> Result<PageState> {
        self.opened.lock().unwrap().push(url.to_string());
        let landed = self
            .redirects
            .lock()
            .unwrap()
            .iter()
            .find(|(from, _)| from == url)
            .map_or(url.to_string(), |(_, to)| to.clone());
        *self.current.lock().unwrap() = Some(landed.clone());
        Ok(self.state(landed))
    }

    async fn snapshot(&self, request: SnapshotRequest) -> Result<SnapshotView> {
        let url = self.here().ok_or_else(|| anyhow!("no page"))?;
        let mut text = self
            .body_override
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| Self::page(&url).body.to_string());
        let typed = self.typed.lock().unwrap().clone();
        if !typed.is_empty() {
            text.push_str(&format!("\nfield value \"{typed}\""));
        }
        let truncated = text.len() > request.max_chars;
        Ok(SnapshotView {
            page: self.state(url),
            text,
            truncated,
        })
    }

    async fn act(&self, step: &BrowserStep) -> Result<PageState> {
        self.steps.lock().unwrap().push(step.clone());
        let url = self.here().ok_or_else(|| anyhow!("no page"))?;
        match step {
            BrowserStep::Click { reference } if reference == "e1" => {
                let next = "https://partner.example/".to_string();
                *self.current.lock().unwrap() = Some(next.clone());
                return Ok(self.state(next));
            }
            BrowserStep::Type { text, .. } => *self.typed.lock().unwrap() = text.clone(),
            BrowserStep::Click { reference } if reference == "boom" => {
                return Err(anyhow!("the field kept {}", self.typed.lock().unwrap()));
            }
            _ => {}
        }
        Ok(self.state(url))
    }

    async fn screenshot(&self) -> Result<Vec<u8>> {
        Ok(vec![0x89, b'P', b'N', b'G'])
    }

    async fn current(&self) -> Result<Option<PageState>> {
        Ok(self.here().map(|url| self.state(url)))
    }

    async fn leave(&self) -> Result<()> {
        *self.left.lock().unwrap() += 1;
        *self.current.lock().unwrap() = Some("about:blank".into());
        Ok(())
    }

    async fn take_downloads(&self) -> Result<Vec<Download>> {
        Ok(std::mem::take(&mut *self.downloads.lock().unwrap()))
    }

    async fn close(&self) -> Result<()> {
        *self.closed.lock().unwrap() += 1;
        *self.current.lock().unwrap() = None;
        Ok(())
    }
}

async fn call(env: &CapEnv, tool: &str, args: Value) -> ToolOutcome {
    run(tool, args, env).await
}

fn ok(outcome: &ToolOutcome) -> &ToolOutcome {
    assert!(outcome.is_ok(), "expected ok, got {:?}", outcome.error);
    outcome
}

fn error_code(outcome: &ToolOutcome) -> &'static str {
    outcome.error.as_ref().expect("an error").code
}

async fn shop() -> (Arc<FakeBrowser>, CapEnv) {
    let browser = Arc::new(FakeBrowser::default());
    let env = CapEnv::with_browser(browser.clone()).await;
    (browser, env)
}

#[tokio::test]
async fn a_session_opens_reads_acts_and_closes() {
    let (browser, env) = shop().await;
    let opened = call(
        &env,
        "browser_open",
        json!({ "url": "https://shop.example/" }),
    )
    .await;
    assert_eq!(ok(&opened).result["title"], "Shop");

    let snapshot = call(&env, "browser_snapshot", json!({})).await;
    assert!(
        ok(&snapshot).result["snapshot"]
            .as_str()
            .unwrap()
            .contains("[ref=e2]")
    );

    let typed = call(
        &env,
        "browser_act",
        json!({ "action": "type", "ref": "e2", "text": "hello" }),
    )
    .await;
    assert_eq!(ok(&typed).result["action"], "type");
    assert!(matches!(
        browser.steps.lock().unwrap()[0],
        BrowserStep::Type { .. }
    ));

    let shot = call(&env, "browser_screenshot", json!({})).await;
    assert_eq!(ok(&shot).images.len(), 1);
    assert_eq!(shot.images[0].mime, "image/png");

    let closed = call(&env, "browser_close", json!({})).await;
    ok(&closed);
    assert_eq!(*browser.closed.lock().unwrap(), 1);
    let after = call(&env, "browser_snapshot", json!({})).await;
    assert_eq!(error_code(&after), crate::error::code::NOT_FOUND);
}

#[tokio::test]
async fn without_a_browser_the_tools_say_so() {
    let env = CapEnv::new().await;
    let outcome = call(
        &env,
        "browser_open",
        json!({ "url": "https://shop.example/" }),
    )
    .await;
    assert_eq!(error_code(&outcome), crate::error::code::NOT_YET);
}

#[tokio::test]
async fn only_web_addresses_open() {
    let (browser, env) = shop().await;
    for url in [
        "file:///etc/passwd",
        "javascript:alert(1)",
        "chrome://settings",
        "shop",
    ] {
        let outcome = call(&env, "browser_open", json!({ "url": url })).await;
        assert!(!outcome.is_ok(), "{url}");
    }
    assert!(browser.opened.lock().unwrap().is_empty());
}

#[tokio::test]
async fn the_snapshot_is_capped_and_says_so() {
    let (browser, env) = shop().await;
    call(
        &env,
        "browser_open",
        json!({ "url": "https://shop.example/" }),
    )
    .await;
    *browser.body_override.lock().unwrap() = Some("row\n".repeat(30_000));
    let outcome = call(&env, "browser_snapshot", json!({ "max_chars": 2000 })).await;
    ok(&outcome);
    let text = outcome.result["snapshot"].as_str().unwrap();
    assert!(text.len() < 2200, "{}", text.len());
    assert!(text.contains("truncated"));
    assert!(outcome.truncated);

    let huge = call(&env, "browser_snapshot", json!({ "max_chars": 10_000_000 })).await;
    assert!(huge.result["snapshot"].as_str().unwrap().len() <= MAX_SNAPSHOT_CHARS + 100);
}

#[tokio::test]
async fn a_click_that_lands_on_a_new_site_is_refused_and_left() {
    let (browser, env) = shop().await;
    call(
        &env,
        "browser_open",
        json!({ "url": "https://shop.example/" }),
    )
    .await;
    let clicked = call(
        &env,
        "browser_act",
        json!({ "action": "click", "ref": "e1" }),
    )
    .await;
    assert_eq!(clicked.status, Status::NeedsApproval);
    assert!(clicked.error.unwrap().message.contains("partner.example"));
    assert_eq!(*browser.left.lock().unwrap(), 1);
    assert_eq!(browser.current().await.unwrap().unwrap().url, "about:blank");
}

#[tokio::test]
async fn opening_the_new_site_approves_it_for_the_session() {
    let (browser, env) = shop().await;
    call(
        &env,
        "browser_open",
        json!({ "url": "https://shop.example/" }),
    )
    .await;
    let opened = call(
        &env,
        "browser_open",
        json!({ "url": "https://partner.example/" }),
    )
    .await;
    ok(&opened);
    call(
        &env,
        "browser_open",
        json!({ "url": "https://shop.example/" }),
    )
    .await;
    let clicked = call(
        &env,
        "browser_act",
        json!({ "action": "click", "ref": "e1" }),
    )
    .await;
    assert_eq!(ok(&clicked).result["url"], "https://partner.example/");
    assert_eq!(*browser.left.lock().unwrap(), 0);
}

#[tokio::test]
async fn a_redirect_to_another_site_is_not_followed_silently() {
    let (browser, env) = shop().await;
    browser.redirects.lock().unwrap().push((
        "https://shop.example/".into(),
        "https://partner.example/".into(),
    ));
    let outcome = call(
        &env,
        "browser_open",
        json!({ "url": "https://shop.example/" }),
    )
    .await;
    assert_eq!(outcome.status, Status::NeedsApproval);
    assert_eq!(*browser.left.lock().unwrap(), 1);
}

#[tokio::test]
async fn www_and_the_bare_host_are_one_site() {
    assert_eq!(
        site_of("https://WWW.Shop.Example./x"),
        Some("shop.example".into())
    );
    assert_eq!(
        site_of("https://shop.example:8443/"),
        Some("shop.example".into())
    );
    assert_eq!(site_of("not a url"), None);
}

#[tokio::test]
async fn a_secret_is_typed_but_never_shown() {
    let (browser, env) = shop().await;
    env.inner
        .secrets
        .lock()
        .unwrap()
        .insert("SHOP_PASSWORD".into(), "hunter2-secret".into());
    call(
        &env,
        "browser_open",
        json!({ "url": "https://shop.example/" }),
    )
    .await;
    let typed = call(
        &env,
        "browser_act",
        json!({ "action": "type", "ref": "e2", "text": "{{secret:SHOP_PASSWORD}}" }),
    )
    .await;
    ok(&typed);
    let BrowserStep::Type { text, .. } = browser.steps.lock().unwrap()[0].clone() else {
        panic!("a type step");
    };
    assert_eq!(text, "hunter2-secret");
    assert!(!typed.envelope().to_string().contains("hunter2-secret"));

    let snapshot = call(&env, "browser_snapshot", json!({})).await;
    let shown = snapshot.envelope().to_string();
    assert!(!shown.contains("hunter2-secret"), "{shown}");
    assert!(shown.contains("{{secret:SHOP_PASSWORD}}"));

    let failed = call(
        &env,
        "browser_act",
        json!({ "action": "click", "ref": "boom" }),
    )
    .await;
    assert!(!failed.is_ok());
    assert!(!failed.envelope().to_string().contains("hunter2-secret"));
}

#[tokio::test]
async fn a_missing_secret_is_named_not_typed() {
    let (browser, env) = shop().await;
    call(
        &env,
        "browser_open",
        json!({ "url": "https://shop.example/" }),
    )
    .await;
    let typed = call(
        &env,
        "browser_act",
        json!({ "action": "type", "ref": "e2", "text": "{{secret:NOPE}}" }),
    )
    .await;
    assert_eq!(error_code(&typed), crate::error::code::NOT_FOUND);
    assert!(browser.steps.lock().unwrap().is_empty());
}

#[tokio::test]
async fn downloads_land_in_the_workspace() {
    let (browser, env) = shop().await;
    call(
        &env,
        "browser_open",
        json!({ "url": "https://shop.example/" }),
    )
    .await;
    browser.downloads.lock().unwrap().push(Download {
        name: "../invoice.pdf".into(),
        bytes: Bytes::from_static(b"%PDF-1.4"),
    });
    let outcome = call(
        &env,
        "browser_act",
        json!({ "action": "press", "text": "Enter" }),
    )
    .await;
    ok(&outcome);
    assert_eq!(outcome.saved.len(), 1);
    assert!(outcome.saved[0].path.starts_with("bot/downloads/"));
    assert!(outcome.saved[0].path.ends_with("-invoice.pdf"));
    assert_eq!(outcome.result["downloads"], json!(["invoice.pdf"]));
}

#[tokio::test]
async fn actions_check_their_arguments() {
    let (browser, env) = shop().await;
    call(
        &env,
        "browser_open",
        json!({ "url": "https://shop.example/" }),
    )
    .await;
    for args in [
        json!({ "action": "click" }),
        json!({ "action": "type", "ref": "e2" }),
        json!({ "action": "scroll" }),
        json!({ "action": "scroll", "text": "sideways" }),
        json!({ "action": "press" }),
    ] {
        let outcome = call(&env, "browser_act", args.clone()).await;
        assert_eq!(
            error_code(&outcome),
            crate::error::code::INVALID_ARGS,
            "{args}"
        );
    }
    assert!(browser.steps.lock().unwrap().is_empty());
}

#[test]
fn opening_asks_and_the_rest_do_not() {
    let specs = super::specs();
    let approval = |name: &str| specs.iter().find(|s| s.name == name).unwrap().approval;
    assert_eq!(approval(BROWSER_OPEN), crate::Approval::Ask);
    for name in [
        BROWSER_SNAPSHOT,
        BROWSER_ACT,
        BROWSER_SCREENSHOT,
        BROWSER_CLOSE,
    ] {
        assert_eq!(approval(name), crate::Approval::None, "{name}");
    }
    assert!(specs.iter().all(|s| s.capability == Some("browser_use")));
    let act = specs.iter().find(|s| s.name == BROWSER_ACT).unwrap();
    assert!(act.input_schema.to_string().contains("\"ref\""));
}
