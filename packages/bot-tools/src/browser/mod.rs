//! `browser_*`: a browser the bot drives (capability `browser_use`). The host owns the browser
//! behind [`BrowserBackend`]; these tools own the rules: a site is visited only after the owner
//! approved it (`browser_open` asks per site), a page that lands elsewhere is left, downloads
//! go to the workspace, and `{{secret:NAME}}` is filled in by the host and never shown back.

mod backend;
#[cfg(test)]
pub(crate) mod tests;

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use flow_like_types::Bytes;
use flow_like_types::reqwest::Url;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::env::ToolEnv;
use crate::error::{ToolError, ToolResult, code};
use crate::files::handle_for;
use crate::layout::sanitize_name;
use crate::outcome::{ToolImage, ToolOutput, shrink_string};
use crate::secret_text::{Scrubber, fill_secrets};
use crate::spec::{Approval, ToolSpec, schema_of};

pub use backend::{
    BrowserBackend, BrowserScreen, BrowserStep, Download, PageState, ScrollTo, SnapshotRequest,
    SnapshotView,
};

pub const BROWSER_OPEN: &str = "browser_open";
pub const BROWSER_SNAPSHOT: &str = "browser_snapshot";
pub const BROWSER_ACT: &str = "browser_act";
pub const BROWSER_SCREENSHOT: &str = "browser_screenshot";
pub const BROWSER_CLOSE: &str = "browser_close";

pub const DOWNLOADS_DIR: &str = "bot/downloads";
pub const DEFAULT_SNAPSHOT_CHARS: usize = 12_000;
pub const MAX_SNAPSHOT_CHARS: usize = 40_000;
const MIN_SNAPSHOT_CHARS: usize = 500;
const STATE_KEY: &str = "browser:state";
const STATE_TTL: Duration = Duration::from_secs(60 * 60);

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BrowserOpenArgs {
    /// An http or https address. The owner approves every new site before the page loads.
    pub url: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BrowserSnapshotArgs {
    /// Most characters to return (default 12000, at most 40000); longer pages are cut.
    #[serde(default)]
    pub max_chars: Option<usize>,
    /// Only the elements you can act on (links, buttons, fields); shorter.
    #[serde(default)]
    pub interactive_only: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BrowserActionKind {
    Click,
    Type,
    Select,
    Press,
    Scroll,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BrowserActArgs {
    /// What to do. `press` sends a key chord (`text`: Enter, Tab, Control+a); `scroll` takes
    /// `text` up, down, top or bottom, or scrolls `ref` into view when `text` is empty.
    pub action: BrowserActionKind,
    /// An element reference from the latest `browser_snapshot`, such as `e12`. Optional for
    /// `press` and for `scroll` of the page.
    #[serde(default, rename = "ref")]
    pub reference: Option<String>,
    /// The text to type, the option to select, the key, or the scroll direction. To type a
    /// password or token, write `{{secret:NAME}}` with the secret's name: the value is filled
    /// in for you and never shown.
    #[serde(default)]
    pub text: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BrowserScreenshotArgs {}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BrowserCloseArgs {}

pub fn specs() -> Vec<ToolSpec> {
    let browser = |name, description, approval, mutating, input_schema| ToolSpec {
        name,
        description,
        capability: Some("browser_use"),
        module: "browser",
        approval,
        approval_args: &[],
        mutating,
        input_schema,
    };
    vec![
        browser(
            BROWSER_OPEN,
            "Open a web page in your own browser. The owner approves each new site first. Then take a browser_snapshot to see the page.",
            Approval::Ask,
            true,
            schema_of::<BrowserOpenArgs>(),
        ),
        browser(
            BROWSER_SNAPSHOT,
            "Read the current page as an accessibility tree. Interactive elements carry references such as e12 that browser_act takes.",
            Approval::None,
            false,
            schema_of::<BrowserSnapshotArgs>(),
        ),
        browser(
            BROWSER_ACT,
            "Click, type into, select in, press a key on or scroll the current page. Take a fresh browser_snapshot after the page changes. Write {{secret:NAME}} to type a stored secret.",
            Approval::None,
            true,
            schema_of::<BrowserActArgs>(),
        ),
        browser(
            BROWSER_SCREENSHOT,
            "Take a picture of the current page and look at it.",
            Approval::None,
            false,
            schema_of::<BrowserScreenshotArgs>(),
        ),
        browser(
            BROWSER_CLOSE,
            "Close your browser. Open pages and unsaved form input are lost; cookies stay for the next session.",
            Approval::None,
            true,
            schema_of::<BrowserCloseArgs>(),
        ),
    ]
}

/// What the tools remember of one browser session: the sites the owner approved and the names
/// of the secrets typed, so later output can be cleaned of them.
#[derive(Default)]
pub struct BrowserState {
    inner: Mutex<StateInner>,
}

#[derive(Default)]
struct StateInner {
    sites: BTreeSet<String>,
    secrets: BTreeSet<String>,
}

impl BrowserState {
    fn lock(&self) -> std::sync::MutexGuard<'_, StateInner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn approve(&self, site: &str) {
        self.lock().sites.insert(site.to_string());
    }

    pub fn approved(&self, site: &str) -> bool {
        self.lock().sites.contains(site)
    }

    fn remember_secrets(&self, names: &BTreeSet<String>) {
        self.lock().secrets.extend(names.iter().cloned());
    }

    fn secret_names(&self) -> BTreeSet<String> {
        self.lock().secrets.clone()
    }
}

fn state_of(env: &dyn ToolEnv) -> Arc<BrowserState> {
    let Some(sessions) = env.sessions() else {
        return Arc::new(BrowserState::default());
    };
    let state = sessions
        .get(STATE_KEY)
        .and_then(|value| value.downcast::<BrowserState>().ok())
        .unwrap_or_default();
    sessions.put(STATE_KEY, state.clone(), STATE_TTL);
    state
}

fn backend_of(env: &dyn ToolEnv) -> ToolResult<Arc<dyn BrowserBackend>> {
    env.browser().ok_or_else(|| {
        ToolError::new(
            code::NOT_YET,
            "This host has no browser for bots; browser use runs on the desktop and on device bots",
        )
    })
}

/// The site a URL belongs to: its lowercase host without a leading `www.`.
pub fn site_of(url: &str) -> Option<String> {
    let host = Url::parse(url.trim())
        .ok()?
        .host_str()?
        .to_ascii_lowercase();
    let host = host.trim_end_matches('.');
    let host = host.strip_prefix("www.").unwrap_or(host);
    (!host.is_empty()).then(|| host.to_string())
}

fn checked_url(raw: &str) -> ToolResult<(Url, String)> {
    let url = Url::parse(raw.trim()).map_err(|error| {
        ToolError::invalid(format!(
            "'{raw}' is not an absolute web address ({error}); write it with https://"
        ))
    })?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(ToolError::refused(format!(
            "Only http and https pages can be opened, not '{}:'",
            url.scheme()
        )));
    }
    let site = site_of(url.as_str())
        .ok_or_else(|| ToolError::invalid(format!("The address '{raw}' has no host")))?;
    Ok((url, site))
}

async fn enforce_site(
    backend: &dyn BrowserBackend,
    state: &BrowserState,
    page: &PageState,
) -> ToolResult<()> {
    if page.url == "about:blank" {
        return Ok(());
    }
    let allowed = site_of(&page.url).is_some_and(|site| state.approved(&site));
    if allowed {
        return Ok(());
    }
    let _ = backend.leave().await;
    let landed = site_of(&page.url).unwrap_or_else(|| page.url.clone());
    Err(ToolError::new(
        code::NEEDS_APPROVAL,
        format!(
            "The page moved to '{landed}', a site the owner has not approved. The browser went back to a blank page. Call browser_open with that address to ask the owner."
        ),
    ))
}

fn finish(result: ToolResult<ToolOutput>, scrubber: &Scrubber) -> ToolResult<ToolOutput> {
    match result {
        Ok(mut output) => {
            scrubber.scrub_value(&mut output.result);
            output.warnings = output
                .warnings
                .iter()
                .map(|warning| scrubber.scrub(warning))
                .collect();
            Ok(output)
        }
        Err(error) => Err(scrubber.scrub_error(error)),
    }
}

fn page_json(page: &PageState) -> Value {
    json!({ "url": page.url, "title": page.title })
}

async fn save_downloads(
    env: &dyn ToolEnv,
    backend: &dyn BrowserBackend,
    mut output: ToolOutput,
) -> ToolResult<ToolOutput> {
    let downloads = backend.take_downloads().await.map_err(ToolError::from)?;
    if downloads.is_empty() {
        return Ok(output);
    }
    let workspace = env
        .scope()
        .workspace
        .clone()
        .ok_or_else(|| ToolError::no_workspace("keep downloaded files"))?;
    let max = env.limits().max_write_bytes;
    let mut names = Vec::new();
    for download in downloads {
        if download.bytes.len() as u64 > max {
            output = output.warn(format!(
                "The download '{}' is larger than {max} bytes and was dropped",
                download.name
            ));
            continue;
        }
        let name = sanitize_name(&download.name);
        let path = format!("{DOWNLOADS_DIR}/{}-{name}", &hash_prefix(&download.bytes));
        let mime = crate::files::guess_mime(&name);
        env.write_workspace(&path, download.bytes.clone(), &mime)
            .await
            .map_err(ToolError::from)?;
        output = output.saved(handle_for(&workspace, &path, &download.bytes, &mime));
        names.push(name);
    }
    if let Some(object) = output.result.as_object_mut() {
        object.insert("downloads".into(), json!(names));
    }
    Ok(output)
}

fn hash_prefix(bytes: &Bytes) -> String {
    crate::files::sha256_hex(bytes).chars().take(8).collect()
}

pub async fn browser_open(env: &dyn ToolEnv, args: BrowserOpenArgs) -> ToolResult<ToolOutput> {
    let backend = backend_of(env)?;
    let state = state_of(env);
    let (url, site) = checked_url(&args.url)?;
    state.approve(&site);
    let page = backend.open(url.as_str()).await.map_err(ToolError::from)?;
    enforce_site(backend.as_ref(), &state, &page).await?;
    let scrubber = Scrubber::load(env, &state.secret_names()).await;
    let output = save_downloads(env, backend.as_ref(), ToolOutput::new(page_json(&page))).await;
    finish(output, &scrubber)
}

pub async fn browser_snapshot(
    env: &dyn ToolEnv,
    args: BrowserSnapshotArgs,
) -> ToolResult<ToolOutput> {
    let backend = backend_of(env)?;
    let state = state_of(env);
    let scrubber = Scrubber::load(env, &state.secret_names()).await;
    let max_chars = args
        .max_chars
        .unwrap_or(DEFAULT_SNAPSHOT_CHARS)
        .clamp(MIN_SNAPSHOT_CHARS, MAX_SNAPSHOT_CHARS);
    let result = async {
        if backend.current().await.map_err(ToolError::from)?.is_none() {
            return Err(no_page());
        }
        let view = backend
            .snapshot(SnapshotRequest {
                max_chars,
                interactive_only: args.interactive_only.unwrap_or(false),
            })
            .await
            .map_err(ToolError::from)?;
        enforce_site(backend.as_ref(), &state, &view.page).await?;
        let cut = view.text.len() > max_chars;
        let text = if cut {
            shrink_string(&view.text, max_chars)
        } else {
            view.text
        };
        Ok(ToolOutput::new(json!({
            "url": view.page.url,
            "title": view.page.title,
            "snapshot": text,
            "observation": backend.observation().await.ok().flatten(),
        }))
        .truncated(view.truncated || cut))
    }
    .await;
    finish(result, &scrubber)
}

fn no_page() -> ToolError {
    ToolError::not_found("No page is open; call browser_open first")
}

fn require_ref(args: &BrowserActArgs) -> ToolResult<String> {
    args.reference
        .as_deref()
        .map(str::trim)
        .filter(|reference| !reference.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            ToolError::invalid(
                "This action needs 'ref', an element reference from browser_snapshot",
            )
        })
}

fn require_text(args: &BrowserActArgs, what: &str) -> ToolResult<String> {
    args.text
        .clone()
        .filter(|text| !text.is_empty())
        .ok_or_else(|| ToolError::invalid(format!("This action needs 'text': {what}")))
}

fn scroll_target(text: Option<&str>) -> ToolResult<ScrollTo> {
    match text.map(|t| t.trim().to_ascii_lowercase()).as_deref() {
        None | Some("") => Ok(ScrollTo::IntoView),
        Some("up") => Ok(ScrollTo::Up),
        Some("down") => Ok(ScrollTo::Down),
        Some("top") => Ok(ScrollTo::Top),
        Some("bottom") => Ok(ScrollTo::Bottom),
        Some(other) => Err(ToolError::invalid(format!(
            "Scroll 'text' is up, down, top or bottom, not '{other}'"
        ))),
    }
}

pub async fn browser_act(env: &dyn ToolEnv, args: BrowserActArgs) -> ToolResult<ToolOutput> {
    let backend = backend_of(env)?;
    let state = state_of(env);
    let mut scrubber = Scrubber::load(env, &state.secret_names()).await;
    let mut used = BTreeSet::new();
    let result = act_inner(
        env,
        backend.as_ref(),
        &state,
        &args,
        &mut scrubber,
        &mut used,
    )
    .await;
    state.remember_secrets(&used);
    finish(result, &scrubber)
}

async fn act_inner(
    env: &dyn ToolEnv,
    backend: &dyn BrowserBackend,
    state: &BrowserState,
    args: &BrowserActArgs,
    scrubber: &mut Scrubber,
    used: &mut BTreeSet<String>,
) -> ToolResult<ToolOutput> {
    if backend.current().await.map_err(ToolError::from)?.is_none() {
        return Err(no_page());
    }
    let step = match args.action {
        BrowserActionKind::Click => BrowserStep::Click {
            reference: require_ref(args)?,
        },
        BrowserActionKind::Type => {
            let text = require_text(args, "what to type")?;
            BrowserStep::Type {
                reference: require_ref(args)?,
                text: fill_secrets(env, &text, scrubber, used).await?,
            }
        }
        BrowserActionKind::Select => {
            let value = require_text(args, "the option to select")?;
            BrowserStep::Select {
                reference: require_ref(args)?,
                value: fill_secrets(env, &value, scrubber, used).await?,
            }
        }
        BrowserActionKind::Press => BrowserStep::Press {
            reference: args.reference.clone().filter(|r| !r.trim().is_empty()),
            key: require_text(args, "the key, such as Enter or Control+a")?,
        },
        BrowserActionKind::Scroll => {
            let direction = scroll_target(args.text.as_deref())?;
            let reference = args.reference.clone().filter(|r| !r.trim().is_empty());
            if direction == ScrollTo::IntoView && reference.is_none() {
                return Err(ToolError::invalid(
                    "Scroll needs 'ref' or 'text' (up, down, top, bottom)",
                ));
            }
            BrowserStep::Scroll {
                reference,
                direction,
            }
        }
    };
    let page = backend.act(&step).await.map_err(ToolError::from)?;
    enforce_site(backend, state, &page).await?;
    let mut result = page_json(&page);
    if let Some(object) = result.as_object_mut() {
        if let Ok(Some(observation)) = backend.observation().await {
            object.insert("observation".into(), json!(observation));
        }
        object.insert("action".into(), json!(args.action));
        object.insert(
            "note".into(),
            json!("Take a new browser_snapshot before using more references"),
        );
    }
    save_downloads(env, backend, ToolOutput::new(result)).await
}

pub async fn browser_screenshot(
    env: &dyn ToolEnv,
    _args: BrowserScreenshotArgs,
) -> ToolResult<ToolOutput> {
    let backend = backend_of(env)?;
    let state = state_of(env);
    let page = backend
        .current()
        .await
        .map_err(ToolError::from)?
        .ok_or_else(no_page)?;
    enforce_site(backend.as_ref(), &state, &page).await?;
    let capture = backend.capture().await.map_err(ToolError::from)?;
    let png = capture.png;
    let mut output = ToolOutput::new(json!({
        "url": page.url,
        "title": page.title,
        "bytes": png.len(),
        "observation": &capture.observation,
    }));
    let mut annotated = ToolOutput::new(json!({}));
    if crate::interaction_image::append(&mut annotated, &png, capture.observation.as_ref()) {
        output.result["feedback_legend"] = json!(crate::interaction_image::LEGEND);
    }
    output.images.push(ToolImage {
        label: format!("Screenshot of {}", page.url),
        mime: "image/png".into(),
        data: Bytes::from(png),
    });
    output.images.extend(annotated.images);
    Ok(output)
}

pub async fn browser_close(env: &dyn ToolEnv, _args: BrowserCloseArgs) -> ToolResult<ToolOutput> {
    let backend = backend_of(env)?;
    backend.close().await.map_err(ToolError::from)?;
    if let Some(sessions) = env.sessions() {
        sessions.remove(STATE_KEY);
    }
    Ok(ToolOutput::new(json!({ "closed": true })))
}
