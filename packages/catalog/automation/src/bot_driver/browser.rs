use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use flow_like_browser::attach::{
    AttachEndpoint, default_profile_dir, read_devtools_active_port, resolve_endpoint,
};
use flow_like_browser::input::paced::{LayoutViewport, Pacing};
use flow_like_browser::launch::{
    AutomationNotice, BrowserKind, LaunchOptions, ProxyConfig, default_cache_dir,
};
use flow_like_browser::script::{ScriptArg, ScriptValue};
use flow_like_browser::{Browser, Element, Page};
use flow_like_types::interaction_observation::*;
use flow_like_types::json::json;
use serde::Deserialize;

use super::human::{Gesture, HumanPacing};
use crate::browser::content::SCROLL_SCRIPT;
use crate::browser::driver::{PageContext, key_chord_paced, resolve_ref};
use crate::browser::policy::NavigationPolicy;
use crate::browser::refs::{SnapshotRequest, take_snapshot_with};
use crate::types::handles::BrowserSlot;

const WINDOW: (u32, u32) = (1280, 900);
const PAGE_LOAD: Duration = Duration::from_secs(30);
const LAUNCH: Duration = Duration::from_secs(30);
const SCROLL_PAGE_FRACTION: f64 = 0.8;
/// How long after the bot's last browser call the hand keeps moving now and then.
const IDLE_WINDOW: Duration = Duration::from_secs(120);
const IDLE_BUSY_RETRY: Duration = Duration::from_millis(150);
const IDLE_BACKOFF: Duration = Duration::from_secs(1);
const IDLE_GESTURE_LIMIT: Duration = Duration::from_secs(3);
/// Wheel scrolling is animated; the offset is read once it came to rest.
const WHEEL_SETTLE: Duration = Duration::from_millis(350);
const WHEEL_ROUNDS: usize = 12;
/// Farther than this many screens a person presses Home or End instead of turning the wheel.
const FAR_SCREENS: f64 = 6.0;

const FIELD_PROBE: &str = r#"
const field = arguments[0];
const text = 'value' in field ? String(field.value ?? '') : (field.isContentEditable ? field.textContent : '');
let active = document.activeElement;
while (active?.shadowRoot?.activeElement) active = active.shadowRoot.activeElement;
return {filled: text.length > 0, focused: active === field || (!!active && field.contains(active))};
"#;

/// Whether Home and End would move a caret instead of the page.
const EDITING_PROBE: &str = r#"
let active = document.activeElement;
while (active?.shadowRoot?.activeElement) active = active.shadowRoot.activeElement;
return !!active && (active.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(active.tagName));
"#;
/// What a page can read about the browser, gathered on the blank start page so the headless
/// browser can present the same identity a window of the same build does.
const IDENTITY_PROBE: &str = r#"(async () => {
  const data = navigator.userAgentData;
  if (!data) return null;
  const high = await data.getHighEntropyValues(
    ['architecture', 'bitness', 'fullVersionList', 'model', 'platformVersion', 'wow64']);
  return {brands: data.brands, mobile: data.mobile, platform: data.platform, high,
    languages: navigator.languages, navigator_platform: navigator.platform};
})()"#;

/// A bot check on the top document: a box to tick (Turnstile, reCAPTCHA, hCaptcha), a button to
/// hold (HUMAN/PerimeterX), a slider (DataDome) or a page that decides by itself (Akamai,
/// Cloudflare). Widgets count only while they show: an invisible reCAPTCHA or Turnstile on an
/// ordinary page is not a check.
const CHALLENGE_PROBE: &str = r#"
const shown = (el) => {
  if (!el) return null;
  const r = el.getBoundingClientRect();
  if (r.width < 4 || r.height < 4) return null;
  if (el.checkVisibility && !el.checkVisibility({visibilityProperty: true, opacityProperty: true})) return null;
  return {x: r.x, y: r.y, width: r.width, height: r.height};
};
const frames = [...document.querySelectorAll('iframe')];
const frame = (test) => frames.find((f) => test(f.src || ''));
const filled = (selector) => [...document.querySelectorAll(selector)].some((f) => (f.value || '').length > 0);
const param = (src, name) => {
  try { const u = new URL(src); return u.searchParams.get(name) || new URLSearchParams(u.hash.slice(1)).get(name); }
  catch (e) { return null; }
};
const check = (kind, fields) => Object.assign({kind, site_key: null, action: null, data: null,
  solved: false, puzzle: false, box: null}, fields);
const cloudflarePage = /^just a moment/i.test(document.title)
  || !!document.querySelector('#challenge-error-text, #challenge-stage, form#challenge-form');
const turnstile = document.querySelector('.cf-turnstile, #cf-turnstile, #turnstile-wrapper');
const turnstileFrame = frame((src) => src.includes('challenges.cloudflare.com'));
const turnstileBox = shown(turnstileFrame) || shown(turnstile);
if (cloudflarePage || turnstileBox) {
  return check(cloudflarePage ? 'cloudflare_page' : 'turnstile', {
    site_key: turnstile?.getAttribute('data-sitekey')
      || ((turnstileFrame?.src || '').match(/\/(0x[0-9A-Za-z_-]{6,})\//) || [])[1] || null,
    action: turnstile?.getAttribute('data-action') || null,
    data: turnstile?.getAttribute('data-cdata') || null,
    solved: filled('[name="cf-turnstile-response"]'), box: turnstileBox});
}
const anchor = frame((src) => /\/recaptcha\/(api2|enterprise)\/anchor/.test(src));
const recaptchaTask = shown(frame((src) => /\/recaptcha\/(api2|enterprise)\/bframe/.test(src)));
if (anchor && (shown(anchor) && param(anchor.src, 'size') !== 'invisible' || recaptchaTask)) {
  return check('recaptcha', {site_key: param(anchor.src, 'k'),
    solved: filled('textarea[name="g-recaptcha-response"]'), puzzle: !!recaptchaTask,
    box: param(anchor.src, 'size') === 'invisible' ? null : shown(anchor)});
}
const hbox = frame((src) => /hcaptcha\.com/.test(src) && /frame=checkbox/.test(src));
const hcaptchaTask = shown(frame((src) => /hcaptcha\.com/.test(src) && /frame=challenge/.test(src)));
if (shown(hbox) || hcaptchaTask) {
  return check('hcaptcha', {site_key: hbox ? param(hbox.src, 'sitekey') : null,
    solved: filled('textarea[name="h-captcha-response"]'), puzzle: !!hcaptchaTask, box: shown(hbox)});
}
const holdButton = document.querySelector('#px-captcha');
if (shown(holdButton)) return check('perimeter_x', {box: shown(holdButton)});
const slider = frame((src) => src.includes('captcha-delivery.com'));
if (slider) return check('datadome', {puzzle: true, box: shown(slider)});
if (document.querySelector('#sec-if-cpt-container, #sec-cpt-if')) return check('akamai', {});
return null;
"#;

/// Puts a solving service's token where the widget leaves its own and calls the page's callback,
/// in the page's world (the callback is the page's function).
const TOKEN_SCRIPT: &str = r#"
const [kind, token] = arguments;
const names = {recaptcha: ['g-recaptcha-response'], hcaptcha: ['h-captcha-response', 'g-recaptcha-response'],
  turnstile: ['cf-turnstile-response'], cloudflare_page: ['cf-turnstile-response']}[kind] || [];
let fields = 0;
for (const name of names) {
  for (const field of document.querySelectorAll(`[name="${name}"], textarea[id^="${name}"]`)) {
    field.value = token;
    if (field.tagName === 'TEXTAREA') field.textContent = token;
    fields++;
  }
}
const widget = document.querySelector({recaptcha: '.g-recaptcha', hcaptcha: '.h-captcha',
  turnstile: '.cf-turnstile', cloudflare_page: '.cf-turnstile'}[kind] || 'none-such');
const named = widget?.getAttribute('data-callback');
let callback = named ? named.split('.').reduce((scope, key) => scope?.[key], window) : null;
if (typeof callback !== 'function' && kind === 'recaptcha' && window.___grecaptcha_cfg) {
  const seen = new Set();
  const find = (node, depth) => {
    if (!node || typeof node !== 'object' || depth > 5 || seen.has(node) || node instanceof Node) return null;
    seen.add(node);
    for (const [key, value] of Object.entries(node)) {
      if (key === 'callback') return typeof value === 'function' ? value : window[value];
      const deeper = find(value, depth + 1);
      if (deeper) return deeper;
    }
    return null;
  };
  callback = find(window.___grecaptcha_cfg.clients, 0);
}
if (typeof callback === 'function') { callback(token); return 'callback'; }
return fields > 0 ? 'fields' : 'none';
"#;

const OWNER_BROWSERS: [BrowserKind; 2] = [BrowserKind::Chrome, BrowserKind::Edge];
const OWNER_BROWSER_OFF: &str = "Remote debugging is off in the owner's browser. The owner opens chrome://inspect/#remote-debugging in Chrome (edge://inspect/#remote-debugging in Edge), switches on 'Allow remote debugging for this browser instance' and leaves the browser open. Then call browser_open again.";
const OWNER_TAB_CLOSED: &str =
    "The bot's tab in the owner's browser was closed; call browser_open to open the page again";

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

/// A window on a high-density screen is captured in device pixels. The bot gets the page in
/// CSS pixels, the size of a headless capture, so a screenshot weighs the same on every screen.
async fn at_page_scale(
    png: Vec<u8>,
    size: (u32, u32),
    page_width: u32,
) -> flow_like_types::Result<(Vec<u8>, (u32, u32))> {
    if page_width == 0 || size.0 <= page_width {
        return Ok((png, size));
    }
    let ratio = f64::from(page_width) / f64::from(size.0);
    let height = ((f64::from(size.1) * ratio).round() as u32).max(1);
    tokio::task::spawn_blocking(move || {
        let image = image::load_from_memory_with_format(&png, image::ImageFormat::Png)?;
        let mut scaled = Vec::new();
        image
            .resize_exact(page_width, height, image::imageops::FilterType::Triangle)
            .write_to(
                &mut std::io::Cursor::new(&mut scaled),
                image::ImageFormat::Png,
            )?;
        Ok((scaled, (page_width, height)))
    })
    .await?
}

pub struct BotBrowserOptions {
    pub profile_dir: PathBuf,
    pub download_dir: PathBuf,
    pub headless: bool,
    /// Launch without the switches that tell pages the browser is automated, with the `Runtime`
    /// domain off and, without a screen, the identity of a windowed browser of the same build.
    pub conceal_automation: bool,
    pub proxy: Option<BotProxy>,
}

/// A proxy for the bot's own browser, read from an address such as
/// `http://user:secret@proxy.example:8080` or `socks5://proxy.example:1080`.
#[derive(Clone, PartialEq, Eq)]
pub struct BotProxy {
    server: String,
    credentials: Option<(String, String)>,
}

impl std::fmt::Debug for BotProxy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BotProxy")
            .field("server", &self.server)
            .field("signs_in", &self.credentials.is_some())
            .finish()
    }
}

impl BotProxy {
    pub fn parse(raw: &str) -> flow_like_types::Result<Self> {
        let url = flow_like_types::reqwest::Url::parse(raw.trim()).map_err(|error| {
            flow_like_types::anyhow!(
                "The proxy address is not a URL such as http://host:8080 ({error})"
            )
        })?;
        let scheme = url.scheme();
        let default_port = match scheme {
            "http" => 80,
            "https" => 443,
            "socks4" | "socks5" => 1080,
            other => {
                return Err(flow_like_types::anyhow!(
                    "Proxies are http, https, socks4 or socks5, not '{other}'"
                ));
            }
        };
        let host = url
            .host_str()
            .filter(|host| !host.is_empty())
            .ok_or_else(|| flow_like_types::anyhow!("The proxy address names no host"))?;
        let port = url.port().unwrap_or(default_port);
        let credentials = (!url.username().is_empty()).then(|| {
            (
                percent_decoded(url.username()),
                percent_decoded(url.password().unwrap_or_default()),
            )
        });
        if credentials.is_some() && scheme.starts_with("socks") {
            return Err(flow_like_types::anyhow!(
                "Chrome cannot sign in to a SOCKS proxy; use an http proxy for one that needs a password"
            ));
        }
        Ok(Self {
            server: format!("{scheme}://{host}:{port}"),
            credentials,
        })
    }

    /// The address without the sign-in.
    pub fn server(&self) -> &str {
        &self.server
    }
}

fn percent_decoded(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let hex = bytes
            .get(index + 1..index + 3)
            .and_then(|pair| std::str::from_utf8(pair).ok())
            .and_then(|pair| u8::from_str_radix(pair, 16).ok());
        match (bytes[index], hex) {
            (b'%', Some(byte)) => {
                decoded.push(byte);
                index += 3;
            }
            (byte, _) => {
                decoded.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

/// A bot check the page shows, in top-level viewport pixels.
#[derive(Debug, Clone, Deserialize)]
pub struct BotChallenge {
    /// `turnstile`, `cloudflare_page`, `recaptcha`, `hcaptcha`, `perimeter_x`, `datadome` or
    /// `akamai`.
    pub kind: String,
    pub site_key: Option<String>,
    pub action: Option<String>,
    pub data: Option<String>,
    pub solved: bool,
    /// An image or slider task is showing that a click does not answer.
    pub puzzle: bool,
    #[serde(rename = "box")]
    pub area: Option<InteractionRect>,
    #[serde(default)]
    pub url: String,
}

impl BotChallenge {
    /// Whether a box to tick or a button to hold is showing.
    pub fn pressable(&self) -> bool {
        !self.solved && !self.puzzle && self.target().is_some()
    }

    /// Where a person ticks the box or holds the button, and the size of that target.
    fn target(&self) -> Option<((f64, f64), (f64, f64))> {
        let area = self.area?;
        let middle = area.y + area.height / 2.0;
        match self.kind.as_str() {
            "turnstile" | "cloudflare_page" => Some(((area.x + 28.0, middle), (18.0, 18.0))),
            "recaptcha" => Some(((area.x + 27.0, middle), (22.0, 22.0))),
            "hcaptcha" => Some(((area.x + 30.0, middle), (22.0, 22.0))),
            "perimeter_x" => Some((
                (area.x + area.width / 2.0, middle),
                (area.width * 0.5, area.height * 0.5),
            )),
            _ => None,
        }
    }
}

/// The browser's languages as the override takes them; Chrome adds the `q` weights itself.
fn accept_language(languages: &[String]) -> String {
    languages.join(",")
}

/// A headless Chrome names itself `HeadlessChrome` in its user agent while its client hints
/// already read like a window's. The override keeps everything else the browser reports and
/// only drops the word, for pages, frames and workers alike.
async fn headless_identity(browser: &Browser, page: &Page) -> Option<flow_like_types::Value> {
    let agent = browser
        .version()
        .user_agent
        .replace("HeadlessChrome/", "Chrome/");
    let reply = page
        .cdp(
            "Runtime.evaluate",
            json!({"expression": IDENTITY_PROBE, "awaitPromise": true, "returnByValue": true}),
        )
        .await
        .inspect_err(
            |error| tracing::warn!(%error, "the headless browser's identity could not be read"),
        )
        .ok()?;
    let found = &reply["result"]["value"];
    if found.is_null() {
        tracing::warn!(
            "the headless browser reports no client hints; its user agent stays as it is"
        );
        return None;
    }
    let windowed = |list: &flow_like_types::Value| {
        let mut list = list.clone();
        for entry in list.as_array_mut().into_iter().flatten() {
            if entry["brand"] == "HeadlessChrome" {
                entry["brand"] = json!("Google Chrome");
            }
        }
        list
    };
    let high = &found["high"];
    let languages: Vec<String> =
        flow_like_types::json::from_value(found["languages"].clone()).unwrap_or_default();
    Some(json!({
        "userAgent": agent,
        "acceptLanguage": accept_language(&languages),
        "platform": found["navigator_platform"],
        "userAgentMetadata": {
            "brands": windowed(&found["brands"]),
            "fullVersionList": windowed(&high["fullVersionList"]),
            "platform": found["platform"],
            "platformVersion": high["platformVersion"],
            "architecture": high["architecture"],
            "model": high["model"],
            "mobile": found["mobile"],
            "bitness": high["bitness"],
            "wow64": high["wow64"],
        },
    }))
}

#[derive(Deserialize)]
struct FieldState {
    filled: bool,
    focused: bool,
}

/// When the bot last used the browser, and how far idle peeks left the page scrolled.
#[derive(Default)]
struct Activity {
    last: std::sync::Mutex<Option<Instant>>,
    owed: std::sync::Mutex<f64>,
}

impl Activity {
    fn touch(&self) {
        *self.last.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
    }

    fn quiet_for(&self) -> Duration {
        self.last
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .map_or(Duration::MAX, |at| at.elapsed())
    }

    fn owed(&self) -> f64 {
        *self.owed.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn settle_owed(&self, change: Option<f64>) {
        let mut owed = self.owed.lock().unwrap_or_else(|e| e.into_inner());
        *owed = change.map_or(0.0, |change| *owed + change);
    }
}

/// The feedback lock, held by every bot call; the idle hand only moves while it is free.
struct Busy<'a> {
    feedback: tokio::sync::MutexGuard<'a, Feedback>,
    activity: &'a Activity,
}

impl std::ops::Deref for Busy<'_> {
    type Target = Feedback;

    fn deref(&self) -> &Feedback {
        &self.feedback
    }
}

impl std::ops::DerefMut for Busy<'_> {
    fn deref_mut(&mut self) -> &mut Feedback {
        &mut self.feedback
    }
}

impl Drop for Busy<'_> {
    fn drop(&mut self) {
        self.activity.touch();
    }
}

struct Human {
    pacing: Arc<HumanPacing>,
    idle: tokio::task::JoinHandle<()>,
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

/// A Chrome of its own (separate profile), or one tab in the browser the owner already runs,
/// driven through the nodes' CDP engine.
pub struct BotBrowser {
    browser: Browser,
    slot: Arc<BrowserSlot>,
    page: Arc<tokio::sync::Mutex<Page>>,
    feedback: Arc<tokio::sync::Mutex<Feedback>>,
    downloads_since: std::sync::atomic::AtomicU64,
    policy: NavigationPolicy,
    declares_automation: bool,
    activity: Arc<Activity>,
    human: std::sync::Mutex<Option<Human>>,
    /// Sent to every page the bot works in: the headless identity fix and the proxy sign-in.
    identity: Option<flow_like_types::Value>,
    proxy: Option<BotProxy>,
}

/// Where remote debugging is switched on in one of the owner's everyday browsers.
async fn owner_endpoint() -> flow_like_types::Result<AttachEndpoint> {
    for kind in OWNER_BROWSERS {
        let enabled = match default_profile_dir(kind) {
            Some(dir) => read_devtools_active_port(&dir)?.is_some(),
            None => false,
        };
        if enabled {
            return Ok(resolve_endpoint("", kind).await?);
        }
    }
    Err(flow_like_types::anyhow!(OWNER_BROWSER_OFF))
}

impl BotBrowser {
    pub async fn launch(options: BotBrowserOptions) -> flow_like_types::Result<Self> {
        std::fs::create_dir_all(&options.profile_dir)?;
        std::fs::create_dir_all(&options.download_dir)?;
        let automation_notice = if options.conceal_automation {
            AutomationNotice::Withheld
        } else {
            AutomationNotice::Declared
        };
        let browser = Browser::launch(LaunchOptions {
            kind: BrowserKind::Chrome,
            executable: None,
            headless: options.headless,
            window_size: WINDOW,
            user_agent: None,
            user_data_dir: Some(options.profile_dir),
            proxy: options.proxy.as_ref().map(|proxy| ProxyConfig {
                server: proxy.server.clone(),
                bypass: Vec::new(),
            }),
            locale: None,
            ignore_https_errors: false,
            cache_dir: default_cache_dir(),
            page_load_timeout: PAGE_LOAD,
            launch_timeout: LAUNCH,
            automation_notice,
        })
        .await?;
        Self::configure(&browser, options.conceal_automation);
        browser
            .set_download_directory(&options.download_dir)
            .await?;
        // The tab Chrome started with: a visible window shows the bot's page alone.
        let page = match browser.pages().into_iter().next() {
            Some(first) => browser.page(&first.target_id).await?,
            None => browser.new_page().await?,
        };
        let identity = match options.headless && options.conceal_automation {
            true => headless_identity(&browser, &page).await,
            false => None,
        };
        let mut bot = Self::driving(
            browser,
            page.clone(),
            automation_notice == AutomationNotice::Declared,
        );
        bot.identity = identity;
        bot.proxy = options.proxy;
        bot.adopt(&page).await?;
        Ok(bot)
    }

    /// Settings for every page attached from now on: the bot settles navigations at
    /// `DOMContentLoaded` (a heavy page's `load` can take longer than the op timeout), and a
    /// concealed browser keeps the `Runtime` domain off.
    fn configure(browser: &Browser, conceal: bool) {
        browser.settings().set_page_load_timeout(PAGE_LOAD);
        browser.settings().set_eager_load(true);
        browser.settings().set_hidden_runtime(conceal);
    }

    /// Sends the browser-wide overrides to a page the bot starts working in.
    async fn adopt(&self, page: &Page) -> flow_like_types::Result<()> {
        if let Some(identity) = &self.identity {
            page.cdp("Emulation.setUserAgentOverride", identity.clone())
                .await?;
        }
        if let Some((username, password)) = self
            .proxy
            .as_ref()
            .and_then(|proxy| proxy.credentials.as_ref())
        {
            page.enable_proxy_auth(username, password).await?;
        }
        Ok(())
    }

    /// The proxy this browser was started with.
    pub fn proxy(&self) -> Option<&BotProxy> {
        self.proxy.as_ref()
    }

    /// A tab of the bot's own in the owner's everyday Chrome or Edge, with the owner's logins.
    /// The browser asks the owner to allow the connection; this waits for that answer.
    /// `hidden`: the bot's tab keeps the `Runtime` domain off (human mode).
    pub async fn attach_owner(hidden: bool) -> flow_like_types::Result<Self> {
        let endpoint = owner_endpoint().await?;
        Self::attach(endpoint, hidden).await.map_err(|error| {
            flow_like_types::anyhow!("Could not connect to the owner's browser: {error:#}")
        })
    }

    /// A new tab in a browser somebody else started. Its other tabs are never touched.
    pub async fn attach(endpoint: AttachEndpoint, hidden: bool) -> flow_like_types::Result<Self> {
        let browser = Browser::attach(endpoint).await?;
        Self::configure(&browser, hidden);
        match browser.new_page().await {
            Ok(page) => Ok(Self::driving(browser, page, false)),
            Err(error) => {
                browser.shutdown().await;
                Err(error.into())
            }
        }
    }

    fn driving(browser: Browser, page: Page, declares_automation: bool) -> Self {
        let since = browser.downloads().cursor();
        Self {
            browser,
            slot: Arc::new(BrowserSlot::default()),
            page: Arc::new(tokio::sync::Mutex::new(page)),
            feedback: Arc::new(tokio::sync::Mutex::new(Feedback::default())),
            downloads_since: since.into(),
            policy: NavigationPolicy::plan_default(),
            declares_automation,
            activity: Arc::new(Activity::default()),
            human: std::sync::Mutex::new(None),
            identity: None,
            proxy: None,
        }
    }

    /// Whether this browser was started with the switches that tell pages it is automated. A
    /// browser of the owner's never counts: its flags are the owner's.
    pub fn declares_automation(&self) -> bool {
        self.declares_automation
    }

    /// Human mode: pointer paths, paced keys and wheel scrolling for every action, and a hand
    /// that keeps moving now and then while the bot thinks. Off: input as fast as the engine
    /// sends it, nothing between actions.
    pub fn set_human(&self, on: bool) {
        let mut human = self.human.lock().unwrap_or_else(|e| e.into_inner());
        match (on, human.is_some()) {
            (true, false) => {
                let pacing = Arc::new(HumanPacing::new());
                let idle = tokio::spawn(idle_hand(
                    self.page.clone(),
                    self.feedback.clone(),
                    self.activity.clone(),
                    pacing.clone(),
                ));
                *human = Some(Human { pacing, idle });
            }
            (false, true) => {
                if let Some(human) = human.take() {
                    human.idle.abort();
                }
            }
            _ => {}
        }
    }

    pub fn is_human(&self) -> bool {
        self.pacing().is_some()
    }

    fn pacing(&self) -> Option<Arc<HumanPacing>> {
        self.human
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|human| human.pacing.clone())
    }

    async fn busy(&self) -> Busy<'_> {
        let feedback = self.feedback.lock().await;
        self.activity.touch();
        Busy {
            feedback,
            activity: &self.activity,
        }
    }

    /// A read script: in the page's world by default, in the engine's isolated world in human
    /// mode so the page cannot observe it.
    async fn read(
        &self,
        ctx: &PageContext,
        body: &str,
        args: Vec<ScriptArg>,
    ) -> flow_like_types::Result<ScriptValue> {
        if self.is_human() {
            ctx.inspect(body, args).await
        } else {
            ctx.probe(body, args).await
        }
    }

    async fn context(&self) -> flow_like_types::Result<PageContext> {
        let mut page = self.page.lock().await;
        if page.is_closed() {
            // Every other tab of a browser the bot did not start belongs to its owner.
            if !self.browser.is_owned() {
                return Err(flow_like_types::anyhow!(OWNER_TAB_CLOSED));
            }
            let next = self
                .browser
                .pages()
                .into_iter()
                .next()
                .ok_or_else(|| flow_like_types::anyhow!("The browser has no open page"))?;
            *page = self.browser.page(&next.target_id).await?;
            self.adopt(&page).await?;
        }
        Ok(PageContext::new(
            self.browser.clone(),
            page.clone(),
            self.slot.clone(),
            Vec::new(),
        ))
    }

    async fn reopen_closed_tab(&self) -> flow_like_types::Result<()> {
        let mut page = self.page.lock().await;
        if page.is_closed() && !self.browser.is_owned() {
            *page = self.browser.new_page().await?;
            self.adopt(&page).await?;
        }
        Ok(())
    }

    async fn close_own_tabs(&self) {
        let own = self.page.lock().await.target_id().clone();
        let opened: Vec<_> = self
            .browser
            .pages()
            .into_iter()
            .filter(|page| page.opener_id.as_ref() == Some(&own))
            .map(|page| page.target_id)
            .collect();
        for target in opened.into_iter().chain([own]) {
            if let Err(error) = self.browser.close_page(&target).await {
                tracing::debug!(%target, %error, "closing the bot's tab in the owner's browser failed");
            }
        }
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
        let mut feedback = self.busy().await;
        self.activity.settle_owed(None);
        feedback.begin("navigate");
        let result = self.goto_inner(url).await;
        feedback.finish(result.is_ok());
        result
    }

    async fn goto_inner(&self, url: &str) -> flow_like_types::Result<(String, String)> {
        let target = self.policy.check_blocking_metadata(url).await?;
        self.reopen_closed_tab().await?;
        let ctx = self.context().await?;
        ctx.page.goto(target.as_str()).await?;
        let landed = ctx.page.url().await?;
        if let Err(error) = self.policy.check_blocking_metadata(&landed).await {
            let _ = ctx.page.goto("about:blank").await;
            return Err(flow_like_types::anyhow!(
                "Navigation to '{url}' ended on a blocked page and was left: {error}"
            ));
        }
        self.location().await
    }

    pub async fn leave(&self) -> flow_like_types::Result<()> {
        let mut feedback = self.busy().await;
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
        let _busy = self.busy().await;
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
        let mut feedback = self.busy().await;
        feedback.begin(match &step {
            BotStep::Click { .. } => "click",
            BotStep::Type { .. } => "type",
            BotStep::Select { .. } => "select",
            BotStep::Press { .. } => "press",
            BotStep::Scroll { .. } => "scroll",
        });
        let result = match self.pacing() {
            Some(pacing) => {
                self.activity.settle_owed(None);
                self.act_human(step, &mut feedback, &pacing).await
            }
            None => self.act_inner(step, &mut feedback).await,
        };
        feedback.finish(result.is_ok());
        result
    }

    fn record_pointer(feedback: &mut Feedback, point: Option<(f64, f64)>) {
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
                Self::record_pointer(feedback, point);
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

    async fn act_human(
        &self,
        step: BotStep,
        feedback: &mut Feedback,
        pacing: &HumanPacing,
    ) -> flow_like_types::Result<()> {
        let ctx = self.context().await?;
        match step {
            BotStep::Click { reference } => {
                let element = resolve_ref(&ctx, &reference).await?;
                self.wheel_into_view(&ctx, &element, pacing).await?;
                self.record_target(&ctx, &element, feedback).await;
                let (_, point) = element.click_paced(pacing).await?;
                Self::record_pointer(feedback, point);
            }
            BotStep::Type { reference, text } => {
                let element = resolve_ref(&ctx, &reference).await?;
                self.wheel_into_view(&ctx, &element, pacing).await?;
                self.record_target(&ctx, &element, feedback).await;
                let (_, point) = element.click_paced(pacing).await?;
                Self::record_pointer(feedback, point);
                Self::empty_field(&ctx, &element, pacing).await?;
                element.send_keys_paced(&text, pacing).await?;
            }
            BotStep::Select { reference, value } => {
                let element = resolve_ref(&ctx, &reference).await?;
                self.wheel_into_view(&ctx, &element, pacing).await?;
                self.record_target(&ctx, &element, feedback).await;
                // A native select opens an OS menu no protocol reaches: point at it, then choose.
                element.hover_paced(pacing).await?;
                tokio::time::sleep(pacing.glance()).await;
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
                key_chord_paced(&ctx, key, &modifiers, pacing).await?;
            }
            BotStep::Scroll { reference, to } => {
                self.scroll_human(&ctx, reference, to, pacing).await?
            }
        }
        Ok(())
    }

    /// A person replaces a field's text by selecting all of it and deleting it; a field the
    /// click did not focus is cleared the plain way.
    async fn empty_field(
        ctx: &PageContext,
        element: &Element,
        pacing: &HumanPacing,
    ) -> flow_like_types::Result<()> {
        let probed = ctx
            .inspect(FIELD_PROBE, vec![ScriptArg::Element(element.clone())])
            .await?;
        let field: FieldState = flow_like_types::json::from_value(probed.json().clone())?;
        if !field.filled {
            return Ok(());
        }
        if !field.focused {
            element.clear().await?;
            return Ok(());
        }
        tokio::time::sleep(pacing.glance()).await;
        let command = if cfg!(target_os = "macos") {
            "Meta"
        } else {
            "Control"
        };
        key_chord_paced(ctx, "a", &[command.to_owned()], pacing).await?;
        key_chord_paced(ctx, "Backspace", &[], pacing).await
    }

    /// The element's box in the top viewport; `None` inside a frame or when it cannot be read.
    async fn top_rect(ctx: &PageContext, element: &Element) -> Option<InteractionRect> {
        let probed = ctx
            .inspect(TARGET_PROBE, vec![ScriptArg::Element(element.clone())])
            .await
            .ok()?;
        flow_like_types::json::from_value(probed.json().clone()).ok()
    }

    /// Turns the wheel until the element sits comfortably in view. Stops when the wheel does not
    /// move the page (an element in a scrolled box or a frame); the engine scrolls it the rest
    /// of the way before the pointer moves.
    async fn wheel_into_view(
        &self,
        ctx: &PageContext,
        element: &Element,
        pacing: &HumanPacing,
    ) -> flow_like_types::Result<()> {
        for _ in 0..WHEEL_ROUNDS {
            let Some(rect) = Self::top_rect(ctx, element).await else {
                return Ok(());
            };
            let viewport = ctx.page.layout_viewport().await?;
            let shown = rect.height.min(viewport.height * 0.6);
            if rect.y >= viewport.height * 0.08 && rect.y + shown <= viewport.height * 0.92 {
                return Ok(());
            }
            let delta = rect.y + shown / 2.0 - viewport.height * 0.42;
            if !Self::wheel_by(ctx, delta, pacing, viewport).await? {
                return Ok(());
            }
        }
        Ok(())
    }

    /// Turns the wheel by about `delta` pixels where the pointer is (bringing it onto the page
    /// first) and tells whether the page moved.
    async fn wheel_by(
        ctx: &PageContext,
        delta: f64,
        pacing: &HumanPacing,
        viewport: LayoutViewport,
    ) -> flow_like_types::Result<bool> {
        if ctx.page.pointer().is_none() {
            let size = (viewport.width, viewport.height);
            let to = (size.0 * 0.55, size.1 * 0.45);
            ctx.page
                .move_pointer(&pacing.reach(None, to, (240.0, 160.0), size))
                .await?;
        }
        ctx.page.wheel(&pacing.wheel(delta)).await?;
        tokio::time::sleep(WHEEL_SETTLE).await;
        let after = ctx.page.layout_viewport().await?;
        Ok((after.scroll.1 - viewport.scroll.1).abs() >= 1.0)
    }

    async fn scroll_human(
        &self,
        ctx: &PageContext,
        reference: Option<String>,
        to: BotScroll,
        pacing: &HumanPacing,
    ) -> flow_like_types::Result<()> {
        if let Some(reference) = reference
            && to == BotScroll::IntoView
        {
            let element = resolve_ref(ctx, &reference).await?;
            self.wheel_into_view(ctx, &element, pacing).await?;
            element.hover_paced(pacing).await?;
            return Ok(());
        }
        let viewport = ctx.page.layout_viewport().await?;
        let step = viewport.height * SCROLL_PAGE_FRACTION;
        match to {
            BotScroll::Up => {
                Self::wheel_by(ctx, -step, pacing, viewport).await?;
            }
            BotScroll::Down | BotScroll::IntoView => {
                Self::wheel_by(ctx, step, pacing, viewport).await?;
            }
            BotScroll::Top => self.wheel_to_end(ctx, pacing, viewport, false).await?,
            BotScroll::Bottom => self.wheel_to_end(ctx, pacing, viewport, true).await?,
        }
        Ok(())
    }

    /// To the top or the bottom: Home or End from far away when no field holds the caret, the
    /// wheel for the rest, the page's own scroll when neither gets there.
    async fn wheel_to_end(
        &self,
        ctx: &PageContext,
        pacing: &HumanPacing,
        mut viewport: LayoutViewport,
        down: bool,
    ) -> flow_like_types::Result<()> {
        let remaining = |viewport: &LayoutViewport| {
            if down {
                viewport.content_height.map_or(f64::INFINITY, |content| {
                    content - viewport.height - viewport.scroll.1
                })
            } else {
                viewport.scroll.1
            }
        };
        let editing = ctx
            .inspect(EDITING_PROBE, Vec::new())
            .await
            .ok()
            .and_then(|value| value.json().as_bool())
            .unwrap_or(true);
        if remaining(&viewport) > viewport.height * FAR_SCREENS && !editing {
            key_chord_paced(ctx, if down { "End" } else { "Home" }, &[], pacing).await?;
            tokio::time::sleep(WHEEL_SETTLE).await;
            viewport = ctx.page.layout_viewport().await?;
        }
        for _ in 0..WHEEL_ROUNDS {
            let left = remaining(&viewport).min(viewport.height * 2.0);
            if left < 1.0 {
                return Ok(());
            }
            let delta = if down { left } else { -left };
            if !Self::wheel_by(ctx, delta, pacing, viewport).await? {
                break;
            }
            viewport = ctx.page.layout_viewport().await?;
        }
        if remaining(&ctx.page.layout_viewport().await?) >= 1.0 {
            let end = if down {
                BotScroll::Bottom
            } else {
                BotScroll::Top
            };
            self.scroll(ctx, None, end).await?;
        }
        Ok(())
    }

    async fn probe_view(&self, ctx: &PageContext) -> flow_like_types::Result<ViewProbe> {
        let loader_before = ctx.page.main_frame().loader_id();
        let value = self.read(ctx, VIEW_PROBE, Vec::new()).await?;
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
        let Ok(probe) = self.probe_view(ctx).await else {
            return;
        };
        let bounds = self
            .read(ctx, TARGET_PROBE, vec![ScriptArg::Element(element.clone())])
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

    async fn find_challenge(ctx: &PageContext) -> flow_like_types::Result<Option<BotChallenge>> {
        let probed = ctx.inspect(CHALLENGE_PROBE, Vec::new()).await?;
        let mut found: Option<BotChallenge> =
            flow_like_types::json::from_value(probed.json().clone())?;
        if let Some(found) = &mut found {
            found.url = ctx.page.url().await?;
            // Turnstile draws its frame in a closed shadow root: the page sees only the
            // container, which may be wider than the widget, so the frame's own box wins.
            if matches!(found.kind.as_str(), "turnstile" | "cloudflare_page")
                && let Ok(Some((x, y, width, height))) =
                    ctx.page.frame_box("challenges.cloudflare.com").await
            {
                found.area = Some(InteractionRect {
                    x,
                    y,
                    width,
                    height,
                });
            }
        }
        Ok(found)
    }

    /// The bot check the page shows, read in the isolated world.
    pub async fn challenge(&self) -> flow_like_types::Result<Option<BotChallenge>> {
        let _busy = self.busy().await;
        let ctx = self.context().await?;
        Self::find_challenge(&ctx).await
    }

    /// Ticks the check's box, or presses and holds its button for 7–11 s, the way a person
    /// does (also outside human mode: a check is where input is judged). The pointer reaches
    /// a cross-origin widget through the page, so the frame sees real screen coordinates.
    /// Returns the check as it was before; one that is solved, shows a task or has nothing to
    /// press is left alone.
    pub async fn press_challenge(&self) -> flow_like_types::Result<Option<BotChallenge>> {
        let mut feedback = self.busy().await;
        feedback.begin("click");
        let result = self.press_challenge_inner().await;
        feedback.finish(result.is_ok());
        result
    }

    async fn press_challenge_inner(&self) -> flow_like_types::Result<Option<BotChallenge>> {
        let ctx = self.context().await?;
        let Some(found) = Self::find_challenge(&ctx).await? else {
            return Ok(None);
        };
        let Some((centre, size)) = found.target().filter(|_| !found.solved && !found.puzzle) else {
            return Ok(Some(found));
        };
        let pacing = self
            .pacing()
            .unwrap_or_else(|| Arc::new(HumanPacing::new()));
        let at = pacing.spot(centre, size);
        let held = match found.kind.as_str() {
            "perimeter_x" => pacing.hold_still(at, (7.0, 11.0)),
            _ => Vec::new(),
        };
        ctx.page
            .click_paced_at(at, size, pacing.as_ref(), &held)
            .await?;
        Ok(Some(found))
    }

    /// Puts a solving service's `token` into the page; tells whether the widget's callback ran
    /// (`callback`), only its response fields took it (`fields`) or nothing did (`none`).
    pub async fn apply_challenge_token(&self, token: &str) -> flow_like_types::Result<String> {
        let _busy = self.busy().await;
        let ctx = self.context().await?;
        let found = Self::find_challenge(&ctx)
            .await?
            .ok_or_else(|| flow_like_types::anyhow!("The page shows no bot check any more"))?;
        let applied = ctx
            .execute(
                TOKEN_SCRIPT,
                vec![
                    ScriptArg::Json(json!(found.kind)),
                    ScriptArg::Json(json!(token)),
                ],
            )
            .await?;
        Ok(applied.json().as_str().unwrap_or("none").to_owned())
    }

    pub async fn observation(&self) -> flow_like_types::Result<InteractionObservation> {
        let mut feedback = self.busy().await;
        let ctx = self.context().await?;
        Ok(Self::observed(self.probe_view(&ctx).await?, &mut feedback))
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
        self.read(ctx, "return window.innerHeight;", Vec::new())
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
        let mut feedback = self.busy().await;
        self.capture(&mut feedback).await
    }

    /// The same capture for the owner's live view: it is no call of the bot's, so the hand
    /// does not count it as activity.
    pub async fn live_view(&self) -> flow_like_types::Result<(Vec<u8>, InteractionObservation)> {
        let mut feedback = self.feedback.lock().await;
        self.capture(&mut feedback).await
    }

    async fn capture(
        &self,
        feedback: &mut Feedback,
    ) -> flow_like_types::Result<(Vec<u8>, InteractionObservation)> {
        let ctx = self.context().await?;
        for attempt in 0..2 {
            let before = self.probe_view(&ctx).await.ok();
            let png = ctx.page.screenshot_in_place(Default::default()).await?;
            let after = self.probe_view(&ctx).await.ok();
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
                let (png, dimensions) = at_page_scale(png, dimensions, after.stamp.width).await?;
                let mut observation = Self::observed(after, feedback);
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

    /// Ends the bot's Chrome. In the owner's browser only the bot's tab and the tabs it opened
    /// are closed, then the connection.
    pub async fn close(&self) {
        self.set_human(false);
        if !self.browser.is_owned() {
            self.close_own_tabs().await;
        }
        self.browser.shutdown().await;
    }

    pub fn abort(&self) {
        self.set_human(false);
        self.browser.abort();
    }
}

impl Drop for BotBrowser {
    fn drop(&mut self) {
        self.set_human(false);
    }
}

/// The hand on the mouse while the bot thinks: rests, a resting hand's tremor, glides, a line
/// read along, now and then a short look further down that is scrolled back later. It moves
/// only while no bot call holds the browser and stops [`IDLE_WINDOW`] after the last one.
async fn idle_hand(
    page: Arc<tokio::sync::Mutex<Page>>,
    feedback: Arc<tokio::sync::Mutex<Feedback>>,
    activity: Arc<Activity>,
    pacing: Arc<HumanPacing>,
) {
    loop {
        let quiet = activity.quiet_for();
        let pause = if quiet > IDLE_WINDOW {
            IDLE_BACKOFF
        } else {
            match feedback.try_lock() {
                Ok(_quiet_browser) => {
                    let page = page.lock().await.clone();
                    idle_gesture(&page, &activity, &pacing, quiet).await
                }
                Err(_) => IDLE_BUSY_RETRY,
            }
        };
        tokio::time::sleep(pause).await;
    }
}

/// Performs one gesture and returns how long the hand waits before the next.
async fn idle_gesture(
    page: &Page,
    activity: &Activity,
    pacing: &HumanPacing,
    quiet: Duration,
) -> Duration {
    if page.is_closed() || page.pending_dialog().is_some() {
        return IDLE_BACKOFF;
    }
    let Ok(viewport) = page.layout_viewport().await else {
        return IDLE_BACKOFF;
    };
    let size = (viewport.width, viewport.height);
    match pacing.idle(page.pointer(), size, quiet, activity.owed()) {
        Gesture::Rest(wait) => wait,
        Gesture::Move(steps) => {
            match tokio::time::timeout(IDLE_GESTURE_LIMIT, page.move_pointer(&steps)).await {
                Ok(Err(error)) => tracing::debug!(%error, "idle pointer move failed"),
                Err(_) => tracing::debug!("idle pointer move timed out"),
                Ok(Ok(())) => {}
            }
            pacing.glance()
        }
        Gesture::Peek(steps) | Gesture::Unpeek(steps) => {
            if let Ok(Err(error)) =
                tokio::time::timeout(IDLE_GESTURE_LIMIT, page.wheel(&steps)).await
            {
                tracing::debug!(%error, "idle wheel failed");
            }
            tokio::time::sleep(WHEEL_SETTLE).await;
            if let Ok(after) = page.layout_viewport().await {
                activity.settle_owed(Some(after.scroll.1 - viewport.scroll.1));
            }
            pacing.glance()
        }
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

    /// Records what a bot detector reads from input events.
    const DETECTOR_PAGE: &str = r#"<html><head><title>Probe</title></head>
<body style="margin:0;height:3200px;font:16px sans-serif">
<input aria-label="Name" style="position:absolute;left:520px;top:300px;width:220px;height:28px">
<button style="position:absolute;left:800px;top:520px;width:120px;height:36px">Near</button>
<button style="position:absolute;left:300px;top:2400px;width:160px;height:40px">Far</button>
<script>
const r = window.__probe = {webdriver: navigator.webdriver, moves: 0, coalesced: {}, movement_mismatch: 0,
  presses: [], wheels: [], keys: 0, inputs: 0, untrusted: 0, screen_offset: null, idle_moves: 0};
let last = null;
addEventListener('mousemove', e => {
  if (last && (e.movementX !== e.screenX - last.x || e.movementY !== e.screenY - last.y)) r.movement_mismatch++;
  last = {x: e.screenX, y: e.screenY};
  r.screen_offset = [e.screenX - e.clientX, e.screenY - e.clientY];
}, true);
addEventListener('pointermove', e => {
  r.moves++;
  if (!e.isTrusted) r.untrusted++;
  const n = e.getCoalescedEvents ? e.getCoalescedEvents().length : -1;
  r.coalesced[n] = (r.coalesced[n] || 0) + 1;
  r.last_move = {x: e.clientX, y: e.clientY, t: performance.now()};
}, true);
addEventListener('pointerdown', e => {
  const b = e.target.getBoundingClientRect();
  const m = r.last_move;
  r.presses.push({target: e.target.tagName, pressure: e.pressure,
    off_centre: [e.clientX - (b.left + b.width / 2), e.clientY - (b.top + b.height / 2)],
    since_move_ms: m ? performance.now() - m.t : -1,
    jump_px: m ? Math.hypot(e.clientX - m.x, e.clientY - m.y) : -1, down: performance.now()});
}, true);
addEventListener('pointerup', () => { const p = r.presses.at(-1); if (p) p.held_ms = performance.now() - p.down; }, true);
addEventListener('wheel', e => r.wheels.push(e.deltaY), {capture: true, passive: true});
addEventListener('keydown', () => r.keys++, true);
addEventListener('input', () => r.inputs++, true);
</script></body></html>"#;

    async fn detector_report(browser: &BotBrowser) -> flow_like_types::Value {
        let ctx = browser.context().await.unwrap();
        ctx.probe("return window.__probe;", Vec::new())
            .await
            .unwrap()
            .json()
            .clone()
    }

    async fn drive_detector_page(port: u16, human: bool) -> flow_like_types::Value {
        let dir = std::env::temp_dir().join(format!("bot-browser-detector-{port}-{human}"));
        let browser = BotBrowser::launch(BotBrowserOptions {
            profile_dir: dir.join("profile"),
            download_dir: dir.join("downloads"),
            headless: false,
            conceal_automation: human,
            proxy: None,
        })
        .await
        .expect("Chrome starts");
        browser.set_human(human);
        browser
            .goto(&format!("http://127.0.0.1:{port}/"))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_secs(3)).await;
        let snapshot = browser.snapshot(true, 5000).await.unwrap();
        let field = ref_of(&snapshot.text, "textbox \"Name\"");
        let near = ref_of(&snapshot.text, "button \"Near\"");
        let far = ref_of(&snapshot.text, "button \"Far\"");
        browser
            .act(BotStep::Type {
                reference: field,
                text: "Hello there".into(),
            })
            .await
            .unwrap();
        browser
            .act(BotStep::Click { reference: near })
            .await
            .unwrap();
        let before_idle = detector_report(&browser).await["moves"].as_u64().unwrap();
        tokio::time::sleep(Duration::from_secs(10)).await;
        let after_idle = detector_report(&browser).await["moves"].as_u64().unwrap();
        browser
            .act(BotStep::Click { reference: far })
            .await
            .unwrap();
        let mut report = detector_report(&browser).await;
        report["idle_moves"] = json!(after_idle - before_idle);
        browser.close().await;
        let _ = std::fs::remove_dir_all(dir);
        report
    }

    #[tokio::test]
    #[ignore = "launches two real Chrome windows; run with --ignored --nocapture"]
    async fn human_mode_gives_a_detector_page_a_persons_input() {
        let port = serve(DETECTOR_PAGE).await;
        let standard = drive_detector_page(port, false).await;
        let human = drive_detector_page(port, true).await;
        eprintln!("standard: {standard:#}\nhuman: {human:#}");

        assert_eq!(standard["webdriver"], true);
        assert_eq!(human["webdriver"], false);
        assert_eq!(human["untrusted"], 0);
        assert!(human["moves"].as_u64().unwrap() > 60, "the pointer travels");
        assert!(
            human["idle_moves"].as_u64().unwrap() > 0,
            "the hand moves while the bot thinks"
        );
        assert_eq!(standard["idle_moves"], 0);
        let presses = human["presses"].as_array().unwrap();
        assert_eq!(presses.len(), 3, "field, near and far were clicked");
        for press in presses {
            assert_eq!(press["pressure"], 0.5, "{press}");
            let held = press["held_ms"].as_f64().unwrap();
            assert!((30.0..300.0).contains(&held), "{press}");
            assert!(
                press["jump_px"].as_f64().unwrap() < 3.0,
                "no teleport: {press}"
            );
            assert!(press["since_move_ms"].as_f64().unwrap() > 30.0, "{press}");
        }
        assert!(
            presses
                .iter()
                .any(|press| press["off_centre"] != json!([0.0, 0.0])),
            "not every click lands on the centre"
        );
        assert!(
            !human["wheels"].as_array().unwrap().is_empty(),
            "wheel scrolling"
        );
        assert!(human["keys"].as_u64().unwrap() >= 11);
        assert_eq!(standard["presses"][0]["pressure"], 0.0);
    }

    /// mobile.de (Akamai) turned the bot away on 2026-11-10. Runs the same path in both modes,
    /// each in a fresh profile: the start page, the cookie banner, then the search. With
    /// `FLOW_LIKE_BOT_ABLATION` set it also runs the hidden flags alone and human input alone.
    #[tokio::test]
    #[ignore = "opens mobile.de in real Chrome windows; run with --ignored --nocapture"]
    async fn mobile_de_in_both_modes() {
        let mut variants = vec![(false, false), (true, true)];
        if std::env::var_os("FLOW_LIKE_BOT_ABLATION").is_some() {
            variants = vec![(true, false), (false, true)];
        }
        for (conceal, human) in variants {
            let dir = std::env::temp_dir().join(format!(
                "bot-browser-mobile-de-{}-{conceal}-{human}",
                std::process::id()
            ));
            let browser = BotBrowser::launch(BotBrowserOptions {
                profile_dir: dir.join("profile"),
                download_dir: dir.join("downloads"),
                headless: false,
                conceal_automation: conceal,
                proxy: None,
            })
            .await
            .expect("Chrome starts");
            browser.set_human(human);
            let mut log = Vec::new();
            let outcome = mobile_de_search(&browser, &mut log).await;
            eprintln!(
                "flags hidden {conceal}, human input {human}: {outcome:?}\n  {}",
                log.join("\n  ")
            );
            browser.close().await;
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    fn denied(text: &str) -> bool {
        [
            "Access Denied",
            "Access denied",
            "Zugriff verweigert",
            "Zugang verweigert",
        ]
        .iter()
        .any(|marker| text.contains(marker))
    }

    /// The first element of one of `roles` whose line holds a name of `names`, the names
    /// tried in order.
    fn first_ref(snapshot: &str, roles: &[&str], names: &[&str]) -> Option<String> {
        names.iter().find_map(|name| {
            snapshot.lines().find_map(|line| {
                let item = line.trim_start().trim_start_matches("- ");
                let role_ok = roles.iter().any(|role| item.starts_with(role));
                (role_ok && line.contains(name) && line.contains("[ref=")).then(|| ref_of(line, ""))
            })
        })
    }

    /// Title, address, the start of the document, the Akamai cookie's state and a screenshot
    /// (saved under `FLOW_LIKE_BOT_PROBE_DIR` when set).
    async fn describe(browser: &BotBrowser, label: &str) -> String {
        let Ok(ctx) = browser.context().await else {
            return format!("{label}: no page");
        };
        let state = ctx
            .probe(
                "const abck = document.cookie.split('; ').find(c => c.startsWith('_abck='));\
                 return {title: document.title, url: location.href, ready: document.readyState,\
                 html: document.documentElement.outerHTML.slice(0, 600), abck: abck ? abck.slice(0, 120) : null,\
                 text: (document.body?.innerText || '').slice(0, 300)};",
                Vec::new(),
            )
            .await
            .map(|value| value.json().clone())
            .unwrap_or_else(|error| json!({ "error": error.to_string() }));
        if let Ok(dir) = std::env::var("FLOW_LIKE_BOT_PROBE_DIR")
            && let Ok((png, _)) = browser.live_view().await
        {
            let _ = std::fs::write(format!("{dir}/{label}.png"), png);
        }
        format!("{label}: {state}")
    }

    async fn mobile_de_search(
        browser: &BotBrowser,
        log: &mut Vec<String>,
    ) -> Result<String, String> {
        let think = || tokio::time::sleep(Duration::from_secs(6));
        let human = if browser.is_human() {
            "human"
        } else {
            "standard"
        };
        let (url, title) = browser
            .goto("https://www.mobile.de/")
            .await
            .map_err(|e| e.to_string())?;
        log.push(format!("start page: {title} ({url})"));
        log.push(describe(browser, &format!("{human}-1-loaded")).await);
        think().await;
        log.push(describe(browser, &format!("{human}-2-after-thinking")).await);
        let page = browser
            .snapshot(false, 40_000)
            .await
            .map_err(|e| format!("snapshot of the start page: {e}"))?;
        if denied(&page.text) {
            return Err(format!("start page denied: {}", page.title));
        }
        let consent = first_ref(
            &page.text,
            &["button"],
            &["Einverstanden", "Akzeptieren", "Alle akzeptieren", "Accept"],
        );
        if let Some(consent) = consent {
            browser
                .act(BotStep::Click { reference: consent })
                .await
                .map_err(|e| e.to_string())?;
            log.push("cookie banner accepted".into());
            think().await;
        } else {
            log.push("no cookie banner found".into());
        }
        let page = browser
            .snapshot(true, 40_000)
            .await
            .map_err(|e| e.to_string())?;
        let search = first_ref(
            &page.text,
            &["button", "link"],
            &[" Angebote\"", "Treffer", "Fahrzeuge anzeigen"],
        )
        .ok_or_else(|| {
            format!(
                "no search button in:\n{}",
                &page.text[..page.text.len().min(3000)]
            )
        })?;
        browser
            .act(BotStep::Click { reference: search })
            .await
            .map_err(|e| e.to_string())?;
        tokio::time::sleep(Duration::from_secs(5)).await;
        let (url, title) = browser.location().await.map_err(|e| e.to_string())?;
        log.push(format!("after search: {title} ({url})"));
        let results = browser
            .snapshot(false, 40_000)
            .await
            .map_err(|e| e.to_string())?;
        if denied(&results.text) || denied(&title) {
            return Err(format!("search denied: {title}"));
        }
        if !url.contains("suchen.mobile.de") && !url.contains("search") {
            return Err(format!("the search did not leave the start page: {url}"));
        }
        let lines = results.text.lines().count();
        Ok(format!(
            "search results shown: {title}, {lines} snapshot lines"
        ))
    }

    /// Serves `body` for every path and keeps each request's head.
    async fn serve_recording(body: &'static str) -> (u16, Arc<std::sync::Mutex<Vec<String>>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = seen.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                let log = log.clone();
                tokio::spawn(async move {
                    let mut buffer = vec![0u8; 16384];
                    let read = stream.read(&mut buffer).await.unwrap_or(0);
                    log.lock()
                        .unwrap()
                        .push(String::from_utf8_lossy(&buffer[..read]).into_owned());
                    let reply = format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(reply.as_bytes()).await;
                });
            }
        });
        (port, seen)
    }

    async fn page_value(browser: &BotBrowser, expression: &str) -> flow_like_types::Value {
        let ctx = browser.context().await.unwrap();
        ctx.probe(&format!("return {expression};"), Vec::new())
            .await
            .unwrap()
            .json()
            .clone()
    }

    async fn launch_for_test(
        name: &str,
        headless: bool,
        conceal: bool,
        proxy: Option<BotProxy>,
    ) -> (BotBrowser, PathBuf) {
        let dir = std::env::temp_dir().join(format!("bot-browser-{name}-{}", std::process::id()));
        let browser = BotBrowser::launch(BotBrowserOptions {
            profile_dir: dir.join("profile"),
            download_dir: dir.join("downloads"),
            headless,
            conceal_automation: conceal,
            proxy,
        })
        .await
        .expect("Chrome starts");
        browser.set_human(conceal);
        (browser, dir)
    }

    /// The parent page of the cross-origin frame test; the frame posts its click up.
    const FRAME_PARENT: &str = r#"<html><head><title>Parent</title></head><body style="margin:0">
<div style="height:180px"></div>
<iframe id="child" style="margin-left:140px;width:400px;height:200px;border:0"></iframe>
<script>
window.__clicks = [];
const top_offset = [];
addEventListener('mousemove', (e) => { top_offset[0] = e.screenX - e.clientX; top_offset[1] = e.screenY - e.clientY; });
addEventListener('message', (e) => {
  const frame = document.getElementById('child').getBoundingClientRect();
  window.__clicks.push(Object.assign({}, e.data, {frame_x: frame.x, frame_y: frame.y,
    parent_offset: top_offset.slice()}));
});
document.getElementById('child').src = location.href.replace('127.0.0.1', 'localhost') + 'child';
</script></body></html>"#;

    const FRAME_CHILD: &str = r#"<html><head><title>Child</title></head><body style="margin:0">
<button style="margin:40px;width:160px;height:50px">Inside</button>
<script>
document.querySelector('button').addEventListener('click', (e) => parent.postMessage({
  screen_x: e.screenX, screen_y: e.screenY, client_x: e.clientX, client_y: e.clientY,
  trusted: e.isTrusted}, '*'));
</script></body></html>"#;

    /// Serves the parent on `/` and the frame on `/child`.
    async fn serve_frames() -> u16 {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                tokio::spawn(async move {
                    let mut buffer = [0u8; 4096];
                    let read = stream.read(&mut buffer).await.unwrap_or(0);
                    let head = String::from_utf8_lossy(&buffer[..read]);
                    let body = if head.starts_with("GET /child") {
                        FRAME_CHILD
                    } else {
                        FRAME_PARENT
                    };
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

    /// A click into a cross-origin frame reports the window's screen coordinates in human mode
    /// (the page session routes it, like a real mouse), and the page's own world stays reachable
    /// with the `Runtime` domain off.
    #[tokio::test]
    #[ignore = "launches real Chrome windows; run with --ignored --nocapture"]
    async fn a_click_into_a_cross_origin_frame_reports_real_screen_coordinates() {
        let port = serve_frames().await;
        for human in [false, true] {
            let (browser, dir) = launch_for_test("frames", false, human, None).await;
            browser
                .goto(&format!("http://127.0.0.1:{port}/"))
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_secs(2)).await;
            let snapshot = browser.snapshot(true, 5000).await.unwrap();
            let inside = ref_of(&snapshot.text, "button \"Inside\"");
            browser
                .act(BotStep::Click { reference: inside })
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_millis(500)).await;
            let clicks = page_value(&browser, "window.__clicks").await;
            eprintln!("human {human}: {clicks:#}");
            let click = &clicks[0];
            assert_eq!(click["trusted"], true);
            if human {
                let offset = &click["parent_offset"];
                let expected_x = offset[0].as_f64().unwrap()
                    + click["frame_x"].as_f64().unwrap()
                    + click["client_x"].as_f64().unwrap();
                let expected_y = offset[1].as_f64().unwrap()
                    + click["frame_y"].as_f64().unwrap()
                    + click["client_y"].as_f64().unwrap();
                assert!((click["screen_x"].as_f64().unwrap() - expected_x).abs() <= 2.0);
                assert!((click["screen_y"].as_f64().unwrap() - expected_y).abs() <= 2.0);
            }
            browser.close().await;
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    const IDENTITY_PAGE: &str = r#"<html><head><title>Identity</title></head><body><script>
window.__identity = {agent: navigator.userAgent,
  brands: (navigator.userAgentData?.brands || []).map((b) => b.brand), languages: navigator.languages};
const worker = new Worker(URL.createObjectURL(new Blob(
  ['postMessage([navigator.userAgent, (navigator.userAgentData?.brands || []).map((b) => b.brand)])'])));
worker.onmessage = (e) => { window.__identity.worker = e.data; };
</script></body></html>"#;

    /// Without a screen, human mode reads like a windowed Chrome to the page, its workers and
    /// its requests; standard mode still says HeadlessChrome.
    #[tokio::test]
    #[ignore = "launches headless Chrome; run with --ignored --nocapture"]
    async fn a_headless_browser_in_human_mode_names_no_headless_anywhere() {
        let (port, seen) = serve_recording(IDENTITY_PAGE).await;
        for human in [false, true] {
            seen.lock().unwrap().clear();
            let (browser, dir) = launch_for_test("identity", true, human, None).await;
            browser
                .goto(&format!("http://localhost:{port}/"))
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_millis(800)).await;
            let identity = page_value(&browser, "window.__identity").await;
            let heads = seen.lock().unwrap().join("\n---\n");
            eprintln!("human {human}: {identity:#}\n{heads}");
            let everywhere = format!("{identity}{heads}");
            assert_eq!(everywhere.contains("Headless"), !human, "{everywhere}");
            if human {
                assert!(
                    heads.contains("Accept-Language: en-US,en;q=0.9\r\n"),
                    "{heads}"
                );
                assert!(heads.to_ascii_lowercase().contains("sec-ch-ua"));
                assert!(identity["worker"][0].as_str().unwrap().contains("Chrome/"));
            }
            browser.close().await;
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    /// A proxy that wants a password: the browser's request is turned away with 407 until it
    /// signs in, then answered for any host.
    async fn proxy_with_password() -> (u16, Arc<std::sync::Mutex<Vec<String>>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = seen.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                let log = log.clone();
                tokio::spawn(async move {
                    let mut buffer = vec![0u8; 8192];
                    let read = stream.read(&mut buffer).await.unwrap_or(0);
                    let head = String::from_utf8_lossy(&buffer[..read]).into_owned();
                    log.lock().unwrap().push(head.clone());
                    // base64("bot:p@ss word")
                    let signed_in =
                        head.contains("Proxy-Authorization: Basic Ym90OnBAc3Mgd29yZA==");
                    let reply = if signed_in {
                        let body = "<html><head><title>Proxied</title></head><body>through the proxy</body></html>";
                        format!(
                            "HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                            body.len()
                        )
                    } else {
                        "HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"bots\"\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".to_owned()
                    };
                    let _ = stream.write_all(reply.as_bytes()).await;
                });
            }
        });
        (port, seen)
    }

    #[tokio::test]
    #[ignore = "launches headless Chrome; run with --ignored --nocapture"]
    async fn the_bot_browser_signs_in_to_a_proxy_with_a_password() {
        let (port, seen) = proxy_with_password().await;
        let proxy = BotProxy::parse(&format!("http://bot:p%40ss%20word@127.0.0.1:{port}")).unwrap();
        assert_eq!(proxy.server(), format!("http://127.0.0.1:{port}"));
        let (browser, dir) = launch_for_test("proxy", true, false, Some(proxy)).await;
        // Chrome upgrades the address to https and tunnels it; this proxy answers the tunnel
        // with a page, so the navigation fails after the sign-in this test is about.
        let _ = browser.goto("http://example.com/").await;
        let heads = seen.lock().unwrap().clone();
        let page_requests: Vec<_> = heads
            .iter()
            .filter(|head| head.contains("example.com"))
            .collect();
        eprintln!("{page_requests:#?}");
        assert!(
            page_requests
                .iter()
                .any(|head| !head.contains("Proxy-Authorization")),
            "the first try goes without a password"
        );
        assert!(
            page_requests
                .iter()
                .any(|head| head.contains("Proxy-Authorization: Basic Ym90OnBAc3Mgd29yZA==")),
            "after the 407 the browser signs in with the decoded password"
        );
        browser.close().await;
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn proxy_addresses_keep_their_sign_in_apart() {
        let proxy = BotProxy::parse("http://user:se%3Acret@proxy.example").unwrap();
        assert_eq!(proxy.server(), "http://proxy.example:80");
        assert_eq!(
            proxy.credentials,
            Some(("user".to_owned(), "se:cret".to_owned()))
        );
        assert!(!format!("{proxy:?}").contains("cret"));
        assert_eq!(
            BotProxy::parse("socks5://proxy.example").unwrap().server(),
            "socks5://proxy.example:1080"
        );
        assert!(BotProxy::parse("socks5://user:pw@proxy.example").is_err());
        assert!(BotProxy::parse("ftp://proxy.example").is_err());
        assert!(BotProxy::parse("proxy.example:8080").is_err());
    }

    #[test]
    fn accept_language_leaves_the_weights_to_chrome() {
        let languages = ["de-DE", "de", "en-US", "en"].map(String::from);
        assert_eq!(accept_language(&languages), "de-DE,de,en-US,en");
    }

    const TURNSTILE_PAGE: &str = r#"<html><head><title>Turnstile</title>
<script src="https://challenges.cloudflare.com/turnstile/v0/api.js" async defer></script></head>
<body><form><div class="cf-turnstile" style="display:flex;justify-content:center" data-sitekey="3x00000000000000000000FF" data-callback="passed"></div></form>
<script>function passed(token) { document.title = 'passed'; }</script></body></html>"#;

    const RECAPTCHA_PAGE: &str = r#"<html><head><title>reCAPTCHA</title>
<script src="https://www.google.com/recaptcha/api.js" async defer></script></head>
<body><form><div class="g-recaptcha" data-sitekey="6LeIxAcTAAAAAJcZVRqyHh71UMIEGNQ_MXjiZKhI" data-callback="passed"></div></form>
<script>function passed(token) { document.title = 'passed'; }</script></body></html>"#;

    const HCAPTCHA_PAGE: &str = r#"<html><head><title>hCaptcha</title>
<script src="https://js.hcaptcha.com/1/api.js" async defer></script></head>
<body><form><div class="h-captcha" data-sitekey="10000000-ffff-ffff-ffff-000000000001" data-callback="passed"></div></form>
<script>function passed(token) { document.title = 'passed'; }</script></body></html>"#;

    /// The check-box widgets with their vendors' test keys, ticked like a person.
    #[tokio::test]
    #[ignore = "loads Turnstile, reCAPTCHA and hCaptcha in a real Chrome window; run with --ignored --nocapture"]
    async fn check_boxes_with_test_keys_are_ticked_and_pass() {
        let mut failures = Vec::new();
        for (name, page) in [
            ("turnstile", TURNSTILE_PAGE),
            ("recaptcha", RECAPTCHA_PAGE),
            ("hcaptcha", HCAPTCHA_PAGE),
        ] {
            let port = serve(page).await;
            let (browser, dir) = launch_for_test(name, false, true, None).await;
            browser
                .goto(&format!("http://localhost:{port}/"))
                .await
                .unwrap();
            let mut found = None;
            for _ in 0..20 {
                tokio::time::sleep(Duration::from_millis(500)).await;
                found = browser.challenge().await.unwrap();
                if found.as_ref().is_some_and(BotChallenge::pressable) {
                    break;
                }
            }
            eprintln!("{name} before: {found:?}");
            browser.press_challenge().await.unwrap();
            let mut title = String::new();
            for _ in 0..30 {
                tokio::time::sleep(Duration::from_millis(500)).await;
                title = browser.location().await.unwrap().1;
                if title == "passed" {
                    break;
                }
            }
            let after = browser.challenge().await.unwrap();
            eprintln!("{name} after: title {title}, {after:?}");
            if title != "passed" {
                failures.push(name);
            }
            browser.close().await;
            let _ = std::fs::remove_dir_all(dir);
        }
        assert!(failures.is_empty(), "not passed: {failures:?}");
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
            conceal_automation: false,
            proxy: None,
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
            conceal_automation: false,
            proxy: None,
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
    #[ignore = "launches a real Chrome window; run with --ignored"]
    async fn a_visible_browser_shows_one_tab_and_shoots_it_in_page_pixels() {
        let port = serve("<html><head><title>Shop</title></head><body>Open</body></html>").await;
        let dir = std::env::temp_dir().join(format!("bot-browser-visible-{port}"));
        let browser = BotBrowser::launch(BotBrowserOptions {
            profile_dir: dir.join("profile"),
            download_dir: dir.join("downloads"),
            headless: false,
            conceal_automation: false,
            proxy: None,
        })
        .await
        .expect("Chrome starts");
        browser
            .goto(&format!("http://127.0.0.1:{port}/"))
            .await
            .unwrap();
        assert_eq!(browser.browser.pages().len(), 1);
        let (png, shot) = browser.screenshot_with_observation().await.unwrap();
        let page = browser.observation().await.unwrap();
        assert_eq!(png_size(&png), (shot.width, shot.height));
        assert_eq!(shot.width, page.width);
        assert!(shot.height.abs_diff(page.height) <= 1, "{shot:?} {page:?}");
        browser.close().await;
        let _ = std::fs::remove_dir_all(dir);
    }

    fn png_size(png: &[u8]) -> (u32, u32) {
        image::ImageReader::with_format(std::io::Cursor::new(png), image::ImageFormat::Png)
            .into_dimensions()
            .unwrap()
    }

    #[tokio::test]
    async fn a_capture_in_device_pixels_shrinks_to_the_page_width() {
        let mut dense = Vec::new();
        image::DynamicImage::ImageRgb8(image::RgbImage::new(200, 101))
            .write_to(
                &mut std::io::Cursor::new(&mut dense),
                image::ImageFormat::Png,
            )
            .unwrap();
        let (scaled, size) = at_page_scale(dense.clone(), (200, 101), 100).await.unwrap();
        assert_eq!(size, (100, 51));
        assert_eq!(png_size(&scaled), size);
        for page_width in [0, 200, 400] {
            let (same, size) = at_page_scale(dense.clone(), (200, 101), page_width)
                .await
                .unwrap();
            assert_eq!(size, (200, 101), "{page_width}");
            assert_eq!(same, dense, "{page_width}");
        }
    }

    #[tokio::test]
    #[ignore = "launches a real Chrome; run with --ignored"]
    async fn an_attached_session_keeps_to_its_own_tab() {
        let port = serve(
            "<html><head><title>Shop</title></head><body>\
             <button onclick=\"document.title='clicked'\">Go</button></body></html>",
        )
        .await;
        let url = format!("http://127.0.0.1:{port}/");
        let dir = std::env::temp_dir().join(format!("bot-browser-attached-{port}"));
        let owner = Browser::launch(LaunchOptions {
            kind: BrowserKind::Chrome,
            executable: None,
            headless: true,
            window_size: WINDOW,
            user_agent: None,
            user_data_dir: Some(dir.join("owner")),
            proxy: None,
            locale: None,
            ignore_https_errors: false,
            cache_dir: default_cache_dir(),
            page_load_timeout: PAGE_LOAD,
            launch_timeout: LAUNCH,
            automation_notice: Default::default(),
        })
        .await
        .expect("the owner's Chrome starts");
        let targets = |browser: &Browser| -> Vec<String> {
            browser
                .pages()
                .into_iter()
                .map(|page| page.target_id.to_string())
                .collect()
        };
        let owner_tabs = targets(&owner);
        let endpoint = resolve_endpoint(owner.debugger_address().unwrap(), BrowserKind::Chrome)
            .await
            .unwrap();

        let bot = BotBrowser::attach(endpoint, false)
            .await
            .expect("the bot attaches");
        bot.goto(&url).await.unwrap();
        let snapshot = bot.snapshot(true, 5000).await.unwrap();
        bot.act(BotStep::Click {
            reference: ref_of(&snapshot.text, "button \"Go\""),
        })
        .await
        .unwrap();
        assert_eq!(bot.location().await.unwrap().1, "clicked");
        assert!(bot.screenshot().await.unwrap().starts_with(b"\x89PNG"));

        let bot_tab = bot.page.lock().await.target_id().clone();
        owner.close_page(&bot_tab).await.unwrap();
        tokio::time::sleep(Duration::from_millis(500)).await;
        let error = bot
            .snapshot(true, 5000)
            .await
            .err()
            .expect("no tab to read");
        assert!(error.to_string().contains("was closed"), "{error}");
        assert_eq!(bot.goto(&url).await.unwrap().1, "Shop");
        assert_eq!(targets(&owner).len(), owner_tabs.len() + 1);

        bot.close().await;
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(owner.is_alive());
        assert_eq!(targets(&owner), owner_tabs);
        owner.shutdown().await;
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

    #[tokio::test]
    async fn metadata_addresses_are_blocked_but_local_development_is_not() {
        let policy = NavigationPolicy::plan_default();
        for url in [
            "http://169.254.169.254/latest/meta-data/",
            "http://[fe80::1]/",
            "http://metadata.google.internal/",
        ] {
            assert!(policy.check_blocking_metadata(url).await.is_err(), "{url}");
        }
        for url in ["http://localhost:3000/", "http://192.168.1.20/"] {
            assert!(policy.check_blocking_metadata(url).await.is_ok(), "{url}");
        }
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
