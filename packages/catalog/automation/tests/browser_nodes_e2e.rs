#![cfg(feature = "execute")]
//! Real node `run()` bodies against a real Chromium, one live run per test.
//!
//! Browser tests are ignored by default and panic when no browser can be found:
//! `cargo test -p flow-like-catalog-automation --features execute --test browser_nodes_e2e -- --include-ignored`.
//! The legacy pin errors of Open Browser and Attach to Browser need no browser and always run.
//!
//! Open Browser finds its browser as in the desktop app: installed Chrome or Chromium first, then
//! Chrome for Testing in `FLOW_LIKE_BROWSER_CACHE_DIR` (or the user cache folder). With
//! `FLOW_LIKE_BROWSER_E2E_PROVISION=stable` the suite first installs Chrome for Testing there with
//! `cft::install(CftVersion::Stable, ..)`, the call behind Settings > Automation > Install, so it
//! gets the newest Stable build users get; `download` installs the pinned `cft::CFT_PINNED` build
//! instead, for a reproducible run. `node_discovery_uses_the_downloaded_browser` fails unless Open
//! Browser then launches the installed build, so CI runs this suite with the runner's own browsers
//! hidden.
extern crate flow_like_runtime as flow_like;

use ahash::AHashMap;
use flow_like::{
    flow::{
        board::ExecutionStage,
        execution::{
            LogLevel, Run as FlowRun, context::ExecutionContext, internal_node::InternalNode,
            internal_pin::InternalPin, resources::RunResources,
        },
        node::{Node, NodeLogic},
    },
    profile::Profile,
    state::{FlowLikeConfig, FlowLikeState},
    utils::http::HTTPClient,
};
use flow_like_browser::launch::{
    self, BrowserKind, Executable, ExecutableSource, Flavor,
    cft::{self, CftVersion},
};
use flow_like_catalog_automation::{
    browser::{
        actions::{
            BrowserDragNode, BrowserExecutePlanNode, BrowserKeyChordNode, BrowserRightClickNode,
        },
        auth::{
            BrowserClearCookiesNode, BrowserLoadCookiesNode, BrowserSaveCookiesNode,
            BrowserSetBasicAuthNode,
        },
        capture::{BrowserScreenshotElementNode, BrowserScreenshotNode},
        conditions::BrowserWaitForConditionNode,
        content::{
            BrowserClickAtPointNode, BrowserGetPageTextNode, BrowserPrintPdfNode,
            BrowserScrollPageNode, BrowserZoomScreenshotNode,
        },
        context::{BrowserCloseNode, BrowserOpenNode},
        emulation::{BrowserBlockUrlsNode, BrowserSetEmulationNode, BrowserSetExtraHeadersNode},
        extract::{
            BrowserExecuteJsNode, BrowserGetAttributeNode, BrowserGetHtmlNode, BrowserGetTextNode,
        },
        files::{
            BrowserSetDownloadDirNode, BrowserTriggerDownloadNode, BrowserUploadFileNode,
            BrowserUploadMultipleFilesNode, BrowserWaitForDownloadNode,
        },
        form::{BrowserFillFormNode, BrowserTypeSecretNode},
        input::{BrowserPressKeyNode, BrowserSelectOptionNode, BrowserTypeTextNode},
        interact::{
            BrowserClickNode, BrowserDoubleClickNode, BrowserHoverNode, BrowserScrollIntoViewNode,
        },
        manage::{
            BrowserAttachNode, BrowserEnterFrameNode, BrowserHandleDialogNode,
            BrowserLeaveFrameNode, BrowserListTabsNode, BrowserSelectTabNode,
            BrowserStartConsoleObserverNode, BrowserStartDriverNode, BrowserStopDriverNode,
            BrowserWaitForUrlNode,
        },
        navigation::{BrowserBackNode, BrowserForwardNode, BrowserGotoNode, BrowserReloadNode},
        observe::{
            BrowserClearConsoleLogsNode, BrowserGetConsoleLogsNode, BrowserGetNetworkRequestsNode,
            BrowserStartNetworkObserverNode, BrowserWaitForNetworkIdleNode,
        },
        page::{BrowserClosePageNode, BrowserNewPageNode},
        persist::{BrowserLoadStorageStateNode, BrowserSaveStorageStateNode},
        policy::BrowserSetNavigationPolicyNode,
        refs::{BrowserFindElementsNode, BrowserSnapshotNode},
        snapshot::{
            BrowserGetAccessibilitySnapshotNode, BrowserGetDomSnapshotNode,
            BrowserGetElementSnapshotNode,
        },
        state::{BrowserCountElementsNode, BrowserGetElementStateNode, BrowserListElementsNode},
        storage::{
            BrowserClearStorageNode, BrowserGetAllStorageNode, BrowserGetLocalStorageNode,
            BrowserGetSessionStorageNode, BrowserSetLocalStorageNode, BrowserSetSessionStorageNode,
        },
        wait::{BrowserWaitForDelayNode, BrowserWaitForNode},
    },
    session::{StartSessionNode, StopSessionNode},
    types::{handles::AutomationSession, selectors::Selector},
};
use flow_like_catalog_core::FlowPath;
use flow_like_types::{
    Value,
    json::json,
    sync::{Mutex, RwLock},
};
use serde::de::DeserializeOwned;
use std::{
    collections::BTreeSet,
    future::Future,
    io::{ErrorKind, Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, PoisonError, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const TEST_BUDGET: Duration = Duration::from_secs(180);
const CLEANUP_BUDGET: Duration = Duration::from_secs(20);
const DIALOG_BUDGET: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(200);
const SLOW_TAIL_CAP: Duration = Duration::from_secs(30);
const PROFILE_PREFIX: &str = "flow-like-browser-";
const STAGING_PREFIX: &str = "flow-like-downloads-";
const BIDI_ERROR: &str = "Firefox and Safari are not supported by the new browser engine yet (WebDriver BiDi support is planned); use Chrome or Edge";
const REMOTE_ERROR: &str =
    "Remote WebDriver hosts are no longer supported; use Attach to Browser with a CDP endpoint";
const FOREIGN_WEBDRIVER_ERROR: &str = " server; remote WebDriver hosts are no longer supported. Use Attach to Browser with a CDP endpoint";
const ATTACH_TYPE_ERROR: &str = "Attachment supports Chrome and Edge";
const NO_BROWSER_ERROR: &str = "No browser attached to this session";
const DEVICE_AGENT: &str =
    "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) FlowLikeNodeE2E/1.0 Mobile/15E148";
const DEVICE_ZONE: &str = "Asia/Tokyo";
const SECRET: &str = "s3cr3t-node-e2e";
const REPORT: &str = "flow-like node e2e report\n";
const FRAME_PROBE_SCRIPT: &str = "return {width: window.innerWidth, agent: navigator.userAgent, zone: Intl.DateTimeFormat().resolvedOptions().timeZone, origin: location.origin};";
const AUTH_USER: &str = "ada";
const AUTH_PASSWORD: &str = "lovelace";
const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
const PROVISION_ENV: &str = "FLOW_LIKE_BROWSER_E2E_PROVISION";
const CACHE_ENV: &str = "FLOW_LIKE_BROWSER_CACHE_DIR";

static BROWSERS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static DISCOVERY: tokio::sync::OnceCell<Discovery> = tokio::sync::OnceCell::const_new();
static WORKSPACES: AtomicU64 = AtomicU64::new(0);
static RAN: std::sync::Mutex<BTreeSet<String>> = std::sync::Mutex::new(BTreeSet::new());

const BASIC_FLOW: &str = "basic flow";
const FRAMES_FLOW: &str = "frames flow";
const DOWNLOAD_AND_DIALOG_FLOW: &str = "download and dialog flow";
const POPUP_FLOW: &str = "popup flow";
const EMULATION_FLOW: &str = "emulation flow";
const NAVIGATION_AND_INPUT_FLOW: &str = "navigation and input flow";
const CONTENT_FLOW: &str = "content flow";
const FORMS_AND_POLICY_FLOW: &str = "forms and policy flow";
const COOKIES_AND_STORAGE_FLOW: &str = "cookies and storage flow";
const NETWORK_FLOW: &str = "network flow";
const TABS_AND_ATTACH_FLOW: &str = "tabs and attach flow";

/// Acceptance criterion 3, "every existing browser_* node passes on the new core": each browser_*
/// node of the contract golden is listed once, under the flow that runs it successfully.
const COVERAGE: &[(&str, &[&str])] = &[
    (
        BASIC_FLOW,
        &[
            "browser_open",
            "browser_goto",
            "browser_snapshot",
            "browser_click",
            "browser_type_text",
            "browser_get_text",
            "browser_wait_for_url",
            "browser_list_tabs",
            "browser_close",
        ],
    ),
    (
        FRAMES_FLOW,
        &[
            "browser_fill_form",
            "browser_get_element_state",
            "browser_type_secret",
            "browser_get_attribute",
            "browser_enter_frame",
            "browser_leave_frame",
        ],
    ),
    (
        DOWNLOAD_AND_DIALOG_FLOW,
        &[
            "browser_set_download_dir",
            "browser_trigger_download",
            "browser_wait_for_download",
            "browser_handle_dialog",
        ],
    ),
    (POPUP_FLOW, &["browser_select_tab"]),
    (
        EMULATION_FLOW,
        &["browser_set_emulation", "browser_execute_js"],
    ),
    (
        NAVIGATION_AND_INPUT_FLOW,
        &[
            "browser_back",
            "browser_forward",
            "browser_reload",
            "browser_click_at_point",
            "browser_hover",
            "browser_double_click",
            "browser_right_click",
            "browser_drag",
            "browser_select_option",
            "browser_press_key",
            "browser_key_chord",
            "browser_scroll_page",
            "browser_scroll_into_view",
        ],
    ),
    (
        CONTENT_FLOW,
        &[
            "browser_screenshot",
            "browser_screenshot_element",
            "browser_zoom_screenshot",
            "browser_print_pdf",
            "browser_get_html",
            "browser_get_page_text",
            "browser_get_dom_snapshot",
            "browser_get_accessibility_snapshot",
            "browser_get_element_snapshot",
            "browser_list_elements",
            "browser_count_elements",
            "browser_find_elements",
            "browser_wait_for",
            "browser_wait_for_condition",
            "browser_wait_delay",
        ],
    ),
    (
        FORMS_AND_POLICY_FLOW,
        &[
            "browser_start_driver",
            "browser_upload_file",
            "browser_upload_multiple_files",
            "browser_execute_plan",
            "browser_block_urls",
            "browser_set_extra_headers",
            "browser_set_navigation_policy",
            "browser_stop_driver",
        ],
    ),
    (
        COOKIES_AND_STORAGE_FLOW,
        &[
            "browser_save_cookies",
            "browser_clear_cookies",
            "browser_load_cookies",
            "browser_set_local_storage",
            "browser_get_local_storage",
            "browser_set_session_storage",
            "browser_get_session_storage",
            "browser_get_all_storage",
            "browser_clear_storage",
            "browser_save_storage_state",
            "browser_load_storage_state",
        ],
    ),
    (
        NETWORK_FLOW,
        &[
            "browser_start_console_observer",
            "browser_get_console_logs",
            "browser_clear_console_logs",
            "browser_start_network_observer",
            "browser_wait_for_network_idle",
            "browser_get_network_requests",
            "browser_set_basic_auth",
        ],
    ),
    (
        TABS_AND_ATTACH_FLOW,
        &["browser_new_page", "browser_close_page", "browser_attach"],
    ),
];

const HTML: &str = "text/html; charset=utf-8";
const ATTACHMENT: &str = "Content-Disposition: attachment; filename=report.txt\r\n";
const GRID_STATUS: &str = "{\"value\":{\"ready\":true,\"message\":\"Selenium Grid ready.\"}}";

const BASIC_PAGE: &str = "<!doctype html><html><head><title>Basic</title>
<script>
function greet() { document.getElementById('greeting').textContent = 'hello'; }
function echo(input) { document.getElementById('echo').textContent = input.value; }
function finishLater() { setTimeout(function () { location.hash = 'done'; }, 200); }
</script></head><body>
<h1>Basic</h1>
<button type='button' onclick='greet()'>Say hello</button>
<p id='greeting'>quiet</p>
<input aria-label='Your name' oninput='echo(this)'>
<p id='echo'></p>
<button type='button' id='finish' onclick='finishLater()'>Finish later</button>
</body></html>";

const OTHER_PAGE: &str =
    "<!doctype html><html><head><title>Other</title></head><body><p>Other page</p></body></html>";

const FRAMES_PAGE: &str = "<!doctype html><html><head><title>Frames</title></head><body>
<h1>Frames</h1>
<iframe id='same' title='Same site form' style='border:0' width='460' height='180' srcdoc='
<input aria-label=&quot;Srcdoc email&quot;>
<input type=&quot;password&quot; aria-label=&quot;Srcdoc secret&quot; oninput=&quot;this.dataset.typed = this.value&quot;>
<select aria-label=&quot;Srcdoc plan&quot;><option value=&quot;free&quot;>Free</option><option value=&quot;pro&quot;>Pro</option></select>
<input type=&quot;checkbox&quot; aria-label=&quot;Srcdoc agree&quot;>'></iframe>
<iframe id='cross' title='Cross site form' style='border:0' width='460' height='220' src='http://127.0.0.1:{port}/frame-form'></iframe>
</body></html>";

const FRAME_FORM_PAGE: &str = "<!doctype html><html><head><title>Cross form</title>
<script>function mark() { document.getElementById('clicked').textContent = 'clicked'; }</script>
</head><body>
<input aria-label='Oopif email'>
<input type='password' aria-label='Oopif secret' oninput='this.dataset.typed = this.value'>
<input type='checkbox' aria-label='Oopif agree'>
<button type='button' onclick='mark()'>Oopif button</button>
<p id='clicked'>idle</p>
<div style='height:1200px'></div>
<p id='frame-end'>Frame end</p>
</body></html>";

const DIALOGS_PAGE: &str = "<!doctype html><html><head><title>Dialogs</title>
<script>
function showAnswer(text) { document.getElementById('answer').textContent = text; }
function askName() { var name = prompt('Your name?', 'nobody'); showAnswer(name === null ? 'dismissed' : name); }
function proceed() { showAnswer(confirm('Proceed?') ? 'yes' : 'no'); }
</script></head><body>
<a id='report' href='/download/report.txt'>Download report</a>
<button type='button' id='ask' onclick='askName()'>Ask</button>
<button type='button' id='confirm' onclick='proceed()'>Confirm</button>
<p id='answer'>none</p>
</body></html>";

const POPUP_PAGE: &str = "<!doctype html><html><head><title>Popup</title>
<script>function openSlow() { window.open('/slow', '_blank'); }</script>
</head><body><button type='button' id='open' onclick='openSlow()'>Open slow page</button></body></html>";

const SLOW_HEAD: &str =
    "<!doctype html><html><head><title>Slow</title></head><body><p id='status'>loading</p>";
const SLOW_TAIL: &str = "<p id='done'>slow done</p>
<script>document.getElementById('status').textContent = 'loaded';</script></body></html>";

const EMULATION_PAGE: &str = "<!doctype html><html><head><title>Emulation</title>
<meta name='viewport' content='width=device-width, initial-scale=1'></head><body>
<iframe id='cross' title='Cross site probe' style='border:0' width='300' height='150' src='http://127.0.0.1:{port}/frame-probe'></iframe>
</body></html>";

const FRAME_PROBE_PAGE: &str =
    "<!doctype html><html><head><title>Probe</title></head><body><p>probe</p></body></html>";

/// Every handler logs the id of the element it belongs to, so each pointer or key node leaves one
/// entry at the end of `#log`.
const POINTER_PAGE: &str = "<!doctype html><html><head><title>Pointer</title>
<style>
body { margin: 0; height: 3000px; }
#spot { position: fixed; left: 0; top: 0; width: 200px; height: 100px; }
.pad { display: block; width: 160px; height: 40px; margin: 8px 8px 8px 240px; }
#bottom { position: absolute; top: 2800px; left: 240px; }
</style>
<script>
function log(entry) { document.getElementById('log').textContent += entry + ' '; }
document.addEventListener('keydown', function (event) { log('key:' + (event.shiftKey ? 'Shift+' : '') + event.key); });
</script></head><body>
<button type='button' id='spot' onclick='log(this.id)'>Spot</button>
<div class='pad' id='hover' onmouseover='log(this.id)'>Hover</div>
<div class='pad' id='double' ondblclick='log(this.id)'>Double</div>
<div class='pad' id='context' oncontextmenu='log(this.id); return false;'>Context</div>
<div class='pad' id='source' onmousedown='log(this.id)'>Source</div>
<div class='pad' id='target' onmouseup='log(this.id)'>Target</div>
<select class='pad' id='plan'><option value='free'>Free</option><option value='pro'>Pro</option></select>
<input class='pad' id='name' aria-label='Name'>
<p class='pad' id='log'></p>
<p id='bottom'>Bottom</p>
</body></html>";

/// Whole-pixel line heights and no margins keep every element on whole CSS pixels, so element
/// screenshots have exact sizes.
const CONTENT_PAGE: &str = "<!doctype html><html><head><title>Content</title>
<style>
body { margin: 0; font: 16px/20px sans-serif; }
h2, p, ul { margin: 0; }
#box { width: 120px; height: 40px; background: #c33; color: #fff; }
#tall { height: 2400px; }
</style>
<script>
function later() {
  setTimeout(function () {
    var late = document.createElement('p');
    late.id = 'late';
    late.textContent = 'late text';
    document.getElementById('slot').appendChild(late);
  }, 400);
}
</script></head><body>
<article id='article'><h2>Content heading</h2><p>First paragraph.</p></article>
<div id='box' aria-label='Red box'>box</div>
<ul><li>one</li><li>two</li><li>three</li></ul>
<button type='button' id='later' onclick='later()'>Later</button>
<button type='button'>Save</button>
<div id='slot'></div>
<div id='tall'></div>
</body></html>";

const FORMS_PAGE: &str = "<!doctype html><html><head><title>Forms</title>
<script>
function show(input) {
  var names = Array.prototype.map.call(input.files, function (file) { return file.name; });
  document.getElementById(input.id + '-names').textContent = names.join(',');
}
function submitted() {
  var fields = [document.getElementById('plan').value, document.getElementById('agree').checked, document.getElementById('name').value];
  document.getElementById('status').textContent = fields.join(' ');
}
</script></head><body>
<input type='file' id='single' onchange='show(this)'><p id='single-names'></p>
<input type='file' id='many' multiple onchange='show(this)'><p id='many-names'></p>
<select id='plan'><option value='free'>Free</option><option value='pro'>Pro</option></select>
<input type='checkbox' id='agree' aria-label='Agree'>
<input id='name' aria-label='Name'>
<button type='button' id='submit' onclick='submitted()'>Submit</button>
<p id='status'>idle</p>
</body></html>";

const BLOCKING_PAGE: &str = "<!doctype html><html><head><title>Blocking</title>
<script src='/blocked.js'></script></head><body><p>blocking</p></body></html>";

const BLOCKED_SCRIPT_RAN: &str = "return window.blockedScriptRan === true;";

const PRIVATE_PAGE: &str = "<!doctype html><html><head><title>Private</title></head><body><p id='who'>{user}</p></body></html>";

const COOKIES: &str = "Set-Cookie: visible=1; Path=/\r\nSet-Cookie: secret=2; Path=/; HttpOnly\r\n";

const CHALLENGE: &str = "WWW-Authenticate: Basic realm=\"node-e2e\"\r\n";

const ROUTES: &[(&str, &str, &str, &str)] = &[
    ("/basic", HTML, "", BASIC_PAGE),
    ("/pointer", HTML, "", POINTER_PAGE),
    ("/content", HTML, "", CONTENT_PAGE),
    ("/forms", HTML, "", FORMS_PAGE),
    ("/blocking", HTML, "", BLOCKING_PAGE),
    (
        "/blocked.js",
        "text/javascript",
        "",
        "window.blockedScriptRan = true;",
    ),
    ("/cookies", HTML, COOKIES, OTHER_PAGE),
    ("/api/data", "application/json", "", "{\"ok\":true}"),
    ("/other", HTML, "", OTHER_PAGE),
    ("/frames", HTML, "", FRAMES_PAGE),
    ("/frame-form", HTML, "", FRAME_FORM_PAGE),
    ("/dialogs", HTML, "", DIALOGS_PAGE),
    ("/popup", HTML, "", POPUP_PAGE),
    ("/emulation", HTML, "", EMULATION_PAGE),
    ("/frame-probe", HTML, "", FRAME_PROBE_PAGE),
    (
        "/download/report.txt",
        "application/octet-stream",
        ATTACHMENT,
        REPORT,
    ),
    ("/grid/status", "application/json", "", GRID_STATUS),
];

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires a Chromium browser; run with --include-ignored"]
async fn open_goto_snapshot_click_type_wait_and_close_leave_no_browser_behind() {
    with_browser(BASIC_FLOW, basic_flow).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires a Chromium browser; run with --include-ignored"]
async fn snapshot_refs_inside_same_site_and_cross_site_frames_drive_form_nodes() {
    with_browser(FRAMES_FLOW, frames_flow).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires a Chromium browser; run with --include-ignored"]
async fn download_and_dialog_nodes_then_stop_session_leave_no_browser_behind() {
    with_browser(DOWNLOAD_AND_DIALOG_FLOW, download_and_dialog_flow).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires a Chromium browser; run with --include-ignored"]
async fn popup_selected_by_url_is_read_after_its_slow_load() {
    with_browser(POPUP_FLOW, popup_flow).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires a Chromium browser; run with --include-ignored"]
async fn device_emulation_reaches_a_cross_site_frame_without_resizing_it() {
    with_browser(EMULATION_FLOW, emulation_flow).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires a Chromium browser; run with --include-ignored"]
async fn history_pointer_keyboard_and_scroll_nodes_act_on_the_page_and_an_entered_frame() {
    with_browser(NAVIGATION_AND_INPUT_FLOW, navigation_and_input_flow).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires a Chromium browser; run with --include-ignored"]
async fn capture_read_list_and_wait_nodes_report_the_page_content() {
    with_browser(CONTENT_FLOW, content_flow).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires a Chromium browser; run with --include-ignored"]
async fn upload_plan_request_policy_and_legacy_driver_nodes_shape_the_session() {
    with_browser(FORMS_AND_POLICY_FLOW, forms_and_policy_flow).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires a Chromium browser; run with --include-ignored"]
async fn cookies_storage_and_storage_state_round_trip_with_a_frame_entered() {
    with_browser(COOKIES_AND_STORAGE_FLOW, cookies_and_storage_flow).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires a Chromium browser; run with --include-ignored"]
async fn console_and_network_observers_and_basic_auth_see_real_traffic() {
    with_browser(NETWORK_FLOW, network_flow).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires a Chromium browser; run with --include-ignored"]
async fn new_and_closed_pages_and_a_second_session_attached_to_the_same_browser() {
    with_browser(TABS_AND_ATTACH_FLOW, tabs_and_attach_flow).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires a Chromium browser; run with --include-ignored"]
async fn persistent_profile_is_released_by_stop_session_and_reopens() {
    with_browser("persistent profile flow", persistent_profile_flow).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires a Chromium browser; run with --include-ignored"]
async fn failed_run_shuts_the_browser_down_and_removes_its_profile() {
    with_browser("failed run", failed_run_flow).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires a Chromium browser; run with --include-ignored"]
async fn cancelled_run_kills_the_browser_and_removes_its_profile() {
    with_browser("cancelled run", cancelled_run_flow).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires a Chromium browser; run with --include-ignored"]
async fn dropped_run_kills_the_browser_and_removes_its_profile() {
    with_browser("dropped run", dropped_run_flow).await;
}

/// Every flow launches the executable discovery picks (see `LaunchedBrowser::running`), so this
/// test passing in the same job proves the flows ran on the downloaded Chrome for Testing. It fails
/// wherever an installed Chrome or Chromium comes first in discovery, and on Windows on Arm, which
/// has no Chrome for Testing build.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires a cached Chrome for Testing and no installed Chrome; run with --include-ignored"]
async fn node_discovery_uses_the_downloaded_browser() {
    let Discovery { installed, found } = discovery().await;
    let cache = launch::default_cache_dir();
    let Some(cached) = cft::find_cached(&cache) else {
        panic!(
            "{} holds no complete Chrome for Testing {} or newer; set {PROVISION_ENV}=stable and {CACHE_ENV}, or install it in Settings > Automation",
            cache.display(),
            cft::CFT_PINNED
        );
    };
    assert!(
        found.source == ExecutableSource::CachedCft
            && found.flavor == Flavor::ChromeForTesting
            && found.path == cached.path,
        "Open Browser with Browser Type Chrome launches {} ({:?}, {:?}) instead of the downloaded {}: an installed browser comes first in discovery, so hide it before this suite runs",
        found.path.display(),
        found.source,
        found.flavor,
        cached.path.display()
    );
    if let Some(installed) = installed {
        assert!(
            found.path == installed.path,
            "Open Browser launches Chrome for Testing {} ({}), not the {} build {PROVISION_ENV} installed ({}): {} holds a newer one",
            version_of(found),
            found.path.display(),
            version_of(installed),
            installed.path.display(),
            cache.display()
        );
    }
    with_browser("downloaded browser", open_and_stop_once).await;
}

#[test]
fn every_browser_node_of_the_contract_golden_is_run_by_a_flow() {
    let golden = golden_browser_nodes();
    let mut listed = BTreeSet::new();
    for (flow, nodes) in COVERAGE {
        for node in *nodes {
            assert!(
                listed.insert(*node),
                "{node} is listed twice, again under {flow}"
            );
        }
    }
    let never_run: Vec<&str> = golden
        .iter()
        .map(String::as_str)
        .filter(|node| !listed.contains(node))
        .collect();
    let unknown: Vec<&str> = listed
        .iter()
        .copied()
        .filter(|node| !golden.contains(*node))
        .collect();
    assert!(
        never_run.is_empty() && unknown.is_empty(),
        "{} of {} browser nodes run in no flow: {never_run:?}; listed but not in the golden: {unknown:?}",
        never_run.len(),
        golden.len()
    );
}

fn golden_browser_nodes() -> BTreeSet<String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("browser_node_contracts.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    let golden: Value =
        serde_json::from_str(&text).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    golden["nodes"]
        .as_array()
        .unwrap_or_else(|| panic!("{} lists no nodes", path.display()))
        .iter()
        .filter_map(|node| node["name"].as_str())
        .filter(|name| name.starts_with("browser_"))
        .map(str::to_owned)
        .collect()
}

#[tokio::test]
async fn open_and_attach_reject_firefox_and_safari_with_a_clear_error() {
    without_browser(|site| async move {
        let flow = Flow::start().await;
        let no_webdriver = site.loopback("/no-webdriver");
        for browser_type in ["Firefox", "Safari"] {
            let inputs = [
                ("browser_type", json!(browser_type)),
                ("webdriver_url", json!(no_webdriver)),
            ];
            let open = flow.fails(BrowserOpenNode::new(), &inputs).await;
            assert_eq!(open, BIDI_ERROR, "{browser_type}");
            let attach = flow.fails(BrowserAttachNode::new(), &inputs).await;
            assert_eq!(attach, ATTACH_TYPE_ERROR, "{browser_type}");
        }
        flow.end().await;
    })
    .await;
}

#[tokio::test]
async fn open_and_attach_reject_remote_webdriver_hosts() {
    without_browser(|_| async move {
        let flow = Flow::start().await;
        for url in [
            "http://selenium.example.com:4444/wd/hub",
            "http://192.168.1.20:9515",
        ] {
            let inputs = [("webdriver_url", json!(url))];
            let open = flow.fails(BrowserOpenNode::new(), &inputs).await;
            assert_eq!(open, REMOTE_ERROR, "{url}");
            let attach = flow.fails(BrowserAttachNode::new(), &inputs).await;
            assert_eq!(attach, REMOTE_ERROR, "{url}");
        }
        flow.end().await;
    })
    .await;
}

#[tokio::test]
async fn open_and_attach_name_a_loopback_webdriver_that_is_not_chromedriver() {
    without_browser(|site| async move {
        let flow = Flow::start().await;
        let grid = site.loopback("/grid");
        let inputs = [("webdriver_url", json!(grid))];
        for error in [
            flow.fails(BrowserOpenNode::new(), &inputs).await,
            flow.fails(BrowserAttachNode::new(), &inputs).await,
        ] {
            assert_eq!(
                error,
                format!("{grid} is a Selenium Grid{FOREIGN_WEBDRIVER_ERROR}")
            );
        }
        flow.end().await;
    })
    .await;
}

async fn basic_flow(site: Site) {
    let mut flow = Flow::start().await;
    let browser = flow.open(&site).await;
    browser.assert_debugger_endpoint();
    flow.assert_only_tab_is_current().await;
    flow.goto(&site.local("/basic")).await;
    let elements = flow.snapshot().await;
    let greet = ref_named(&elements, "Say hello");
    flow.ok(BrowserClickNode::new(), &[("selector", json!(greet))])
        .await;
    assert_eq!(flow.text("#greeting").await, "hello");
    let name = ref_named(&elements, "Your name");
    flow.ok(
        BrowserTypeTextNode::new(),
        &[("selector", json!(name)), ("text", json!("Ada Lovelace"))],
    )
    .await;
    assert_eq!(flow.text("#echo").await, "Ada Lovelace");
    wait_for_done_hash(&mut flow, &site).await;
    flow.goto(&site.local("/other")).await;
    let stale = flow
        .fails(BrowserClickNode::new(), &[("selector", json!(greet))])
        .await;
    let expected = format!("Stale element ref '{greet}' — take a new browser snapshot");
    assert!(stale.contains(&expected), "{stale}");
    flow.ok(BrowserCloseNode::new(), &[]).await;
    assert!(flow.session.current_window_handle.is_none());
    browser.assert_gone("Close Browser").await;
    flow.stop().await;
    flow.end().await;
}

async fn wait_for_done_hash(flow: &mut Flow, site: &Site) {
    flow.ok(BrowserClickNode::new(), &[("selector", json!("#finish"))])
        .await;
    let waited = flow
        .ok(
            BrowserWaitForUrlNode::new(),
            &[
                ("expected_url", json!(site.local("/basic#done"))),
                ("timeout_ms", json!(10_000)),
            ],
        )
        .await;
    assert!(
        waited.output::<bool>("found").await,
        "Wait For URL never saw #done"
    );
}

async fn frames_flow(site: Site) {
    let mut flow = Flow::start().await;
    let browser = flow.open(&site).await;
    flow.goto(&site.local("/frames")).await;
    browser
        .assert_out_of_process_frame(&site.loopback("/frame-form"))
        .await;
    let elements = flow.snapshot().await;
    fill_both_frames(&mut flow, &elements).await;
    assert_frame_states(&mut flow, &elements).await;
    for name in ["Srcdoc secret", "Oopif secret"] {
        type_secret(&mut flow, &ref_named(&elements, name)).await;
    }
    click_inside_cross_site_frame(&mut flow, &elements).await;
    flow.stop().await;
    browser.assert_gone("Stop Session").await;
    flow.end().await;
}

async fn fill_both_frames(flow: &mut Flow, elements: &[Value]) {
    let target = |name: &str| ref_named(elements, name);
    let fields = json!([
        {"target": target("Srcdoc email"), "value": "same@example.com"},
        {"target": target("Srcdoc plan"), "value": "pro", "kind": "select"},
        {"target": target("Srcdoc agree"), "value": "true", "kind": "checkbox"},
        {"target": target("Oopif email"), "value": "cross@example.com"},
        {"target": target("Oopif agree"), "value": "true", "kind": "checkbox"},
    ]);
    let filled = flow
        .ok(
            BrowserFillFormNode::new(),
            &[("fields", fields), ("timeout_ms", json!(10_000))],
        )
        .await;
    assert_eq!(filled.output::<i64>("filled_count").await, 5);
}

async fn assert_frame_states(flow: &mut Flow, elements: &[Value]) {
    for (name, field, expected) in [
        ("Srcdoc email", "text", json!("same@example.com")),
        ("Srcdoc email", "editable", json!(true)),
        ("Srcdoc plan", "text", json!("pro")),
        ("Srcdoc agree", "checked", json!(true)),
        ("Oopif email", "text", json!("cross@example.com")),
        ("Oopif email", "visible", json!(true)),
        ("Oopif agree", "checked", json!(true)),
    ] {
        let state = flow.element_state(&ref_named(elements, name)).await;
        assert_eq!(state[field], expected, "{name}: {state}");
    }
}

/// Typing never changes the `value` content attribute, so each password field mirrors what it
/// received into `data-typed` from its input events.
async fn type_secret(flow: &mut Flow, target: &str) {
    flow.ok(
        BrowserTypeSecretNode::new(),
        &[("selector", json!(target)), ("secret", json!(SECRET))],
    )
    .await;
    let typed = flow
        .ok(
            BrowserGetAttributeNode::new(),
            &[
                ("selector", json!(target)),
                ("attribute", json!("data-typed")),
            ],
        )
        .await;
    assert_eq!(typed.output::<String>("value").await, SECRET, "{target}");
}

async fn click_inside_cross_site_frame(flow: &mut Flow, elements: &[Value]) {
    let button = ref_named(elements, "Oopif button");
    flow.ok(BrowserClickNode::new(), &[("selector", json!(button))])
        .await;
    flow.ok(
        BrowserEnterFrameNode::new(),
        &[("selector", json!("#cross"))],
    )
    .await;
    assert_eq!(flow.text("#clicked").await, "clicked");
    flow.ok(BrowserLeaveFrameNode::new(), &[]).await;
    assert!(flow.session.browser_frame_selectors.is_empty());
}

async fn download_and_dialog_flow(site: Site) {
    let mut flow = Flow::start().await;
    let browser = flow.open(&site).await;
    flow.goto(&site.local("/dialogs")).await;
    download_report(&mut flow).await;
    answer_prompt(&mut flow).await;
    dismiss_confirm(&mut flow).await;
    flow.stop().await;
    browser.assert_gone("Stop Session").await;
    flow.end().await;
}

async fn download_report(flow: &mut Flow) {
    let workspace = Workspace::new();
    let directory = flow.run.flow_dir(&workspace.root, "downloads").await;
    flow.ok(
        BrowserSetDownloadDirNode::new(),
        &[("download_path", json!(directory))],
    )
    .await;
    let triggered = flow
        .ok(
            BrowserTriggerDownloadNode::new(),
            &[("selector", json!("#report"))],
        )
        .await;
    assert!(triggered.activated("exec_out").await, "no #report link");
    let waited = flow
        .ok(
            BrowserWaitForDownloadNode::new(),
            &[
                ("download_dir", json!(directory)),
                ("file_pattern", json!("report*.txt")),
                ("timeout_ms", json!(30_000)),
            ],
        )
        .await;
    assert!(waited.activated("exec_out").await, "the download timed out");
    let file: FlowPath = waited.output("downloaded_file").await;
    assert_eq!(file.path, "report.txt");
    let saved = workspace.root.join("downloads").join("report.txt");
    let content = std::fs::read_to_string(&saved).unwrap_or_default();
    assert_eq!(content, REPORT, "{}", saved.display());
}

async fn answer_prompt(flow: &mut Flow) {
    let started = Instant::now();
    flow.ok(BrowserClickNode::new(), &[("selector", json!("#ask"))])
        .await;
    let elapsed = started.elapsed();
    assert!(elapsed < DIALOG_BUDGET, "opening a prompt took {elapsed:?}");
    let blocked = flow
        .fails(BrowserGetTextNode::new(), &[("selector", json!("#answer"))])
        .await;
    assert!(blocked.contains("unexpected alert open"), "{blocked}");
    assert_eq!(flow.dialog("read", "").await, "Your name?");
    assert_eq!(flow.dialog("accept", "Grace Hopper").await, "Your name?");
    assert_eq!(flow.text("#answer").await, "Grace Hopper");
    flow.ok(BrowserClickNode::new(), &[("selector", json!("#ask"))])
        .await;
    assert_eq!(flow.dialog("accept", "").await, "Your name?");
    assert_eq!(
        flow.text("#answer").await,
        "nobody",
        "an empty Prompt Text accepts the prompt's default value"
    );
}

async fn dismiss_confirm(flow: &mut Flow) {
    flow.ok(BrowserClickNode::new(), &[("selector", json!("#confirm"))])
        .await;
    assert_eq!(flow.dialog("dismiss", "").await, "Proceed?");
    assert_eq!(flow.text("#answer").await, "no");
    let none = flow
        .fails(BrowserHandleDialogNode::new(), &[("action", json!("read"))])
        .await;
    assert!(none.contains("no such alert"), "{none}");
}

async fn popup_flow(site: Site) {
    let mut flow = Flow::start().await;
    let browser = flow.open(&site).await;
    flow.goto(&site.loopback("/popup")).await;
    let opener = flow.current_tab();
    flow.ok(BrowserClickNode::new(), &[("selector", json!("#open"))])
        .await;
    flow.ok(
        BrowserSelectTabNode::new(),
        &[("url", json!(site.loopback("/slow")))],
    )
    .await;
    assert!(
        !site.slow_page_finished(),
        "Select Tab outlasted the {SLOW_TAIL_CAP:?} cap on the slow page, so Get Text cannot prove it waits for a loading popup"
    );
    site.release_slow_tail();
    let popup = flow.current_tab();
    assert_ne!(popup, opener);
    assert_eq!(flow.text("#done").await, "slow done");
    assert_eq!(flow.text("#status").await, "loaded");
    assert_eq!(handles(&flow.tabs().await), vec![opener, popup]);
    flow.stop().await;
    browser.assert_gone("Stop Session").await;
    flow.end().await;
}

async fn emulation_flow(site: Site) {
    let mut flow = Flow::start().await;
    let browser = flow.open(&site).await;
    flow.goto(&site.local("/emulation")).await;
    flow.ok(
        BrowserSetEmulationNode::new(),
        &[
            ("timezone_id", json!(DEVICE_ZONE)),
            ("viewport_width", json!(390)),
            ("viewport_height", json!(844)),
            ("device_scale_factor", json!(3.0)),
            ("mobile", json!(true)),
            ("user_agent", json!(DEVICE_AGENT)),
        ],
    )
    .await;
    flow.goto(&site.local("/emulation")).await;
    browser
        .assert_out_of_process_frame(&site.loopback("/frame-probe"))
        .await;
    let top = flow.script(FRAME_PROBE_SCRIPT).await;
    assert_probe(&top, 390.0, &site.local(""));
    flow.ok(
        BrowserEnterFrameNode::new(),
        &[("selector", json!("#cross"))],
    )
    .await;
    let frame = flow.script(FRAME_PROBE_SCRIPT).await;
    assert_probe(&frame, 300.0, &site.loopback(""));
    flow.ok(BrowserLeaveFrameNode::new(), &[]).await;
    flow.stop().await;
    browser.assert_gone("Stop Session").await;
    flow.end().await;
}

fn assert_probe(probe: &Value, width: f64, origin: &str) {
    assert_eq!(probe["width"].as_f64(), Some(width), "{probe}");
    assert_eq!(probe["agent"], DEVICE_AGENT, "{probe}");
    assert_eq!(probe["zone"], DEVICE_ZONE, "{probe}");
    assert_eq!(probe["origin"], origin, "{probe}");
}

async fn navigation_and_input_flow(site: Site) {
    let mut flow = Flow::start().await;
    let browser = flow.open(&site).await;
    history(&mut flow, &site).await;
    flow.goto(&site.local("/pointer")).await;
    pointer(&mut flow).await;
    keyboard(&mut flow).await;
    scrolling(&mut flow).await;
    flow.goto(&site.local("/frames")).await;
    browser
        .assert_out_of_process_frame(&site.loopback("/frame-form"))
        .await;
    input_inside_cross_site_frame(&mut flow).await;
    flow.stop().await;
    browser.assert_gone("Stop Session").await;
    flow.end().await;
}

/// Each history node is checked by the very next read, so a node that returned before its
/// navigation committed reads the old document.
async fn history(flow: &mut Flow, site: &Site) {
    flow.goto(&site.local("/basic")).await;
    flow.goto(&site.local("/other")).await;
    flow.ok(BrowserBackNode::new(), &[]).await;
    assert_eq!(flow.path().await, "/basic");
    flow.ok(BrowserForwardNode::new(), &[]).await;
    assert_eq!(flow.path().await, "/other");
    flow.script("window.beforeReload = true; return true;")
        .await;
    flow.ok(BrowserReloadNode::new(), &[]).await;
    assert_eq!(
        flow.script("return typeof window.beforeReload;").await,
        "undefined"
    );
    assert_eq!(flow.path().await, "/other");
}

async fn pointer(flow: &mut Flow) {
    flow.ok(
        BrowserClickAtPointNode::new(),
        &[("x", json!(50.0)), ("y", json!(50.0))],
    )
    .await;
    flow.assert_logged("spot").await;
    flow.ok(BrowserHoverNode::new(), &[("selector", json!("#hover"))])
        .await;
    flow.assert_logged("hover").await;
    flow.ok(
        BrowserDoubleClickNode::new(),
        &[("selector", json!("#double"))],
    )
    .await;
    flow.assert_logged("double").await;
    flow.ok(
        BrowserRightClickNode::new(),
        &[("selector", json!("#context"))],
    )
    .await;
    flow.assert_logged("context").await;
    flow.ok(
        BrowserDragNode::new(),
        &[
            ("source", json!(Selector::css("#source"))),
            ("target", json!(Selector::css("#target"))),
        ],
    )
    .await;
    flow.assert_logged("source target").await;
    flow.ok(
        BrowserSelectOptionNode::new(),
        &[("selector", json!("#plan")), ("value", json!("pro"))],
    )
    .await;
    assert_eq!(
        flow.script("return document.getElementById('plan').value;")
            .await,
        "pro"
    );
}

async fn keyboard(flow: &mut Flow) {
    flow.ok(
        BrowserPressKeyNode::new(),
        &[("selector", json!("#name")), ("key", json!("Enter"))],
    )
    .await;
    flow.assert_logged("key:Enter").await;
    assert_eq!(
        flow.script("return document.activeElement.id;").await,
        "name"
    );
    flow.ok(
        BrowserKeyChordNode::new(),
        &[("key", json!("k")), ("modifiers", json!(["Shift"]))],
    )
    .await;
    flow.assert_logged("key:Shift+K").await;
    assert_eq!(
        flow.script("return document.getElementById('name').value;")
            .await,
        "K"
    );
}

async fn scrolling(flow: &mut Flow) {
    let by = flow
        .ok(
            BrowserScrollPageNode::new(),
            &[("mode", json!("by")), ("delta_y", json!(600.0))],
        )
        .await;
    assert_eq!(by.output::<i64>("scroll_y").await, 600);
    assert!(!by.output::<bool>("at_bottom").await);
    let bottom = flow
        .ok(BrowserScrollPageNode::new(), &[("mode", json!("bottom"))])
        .await;
    assert!(bottom.output::<bool>("at_bottom").await);
    let top = flow
        .ok(BrowserScrollPageNode::new(), &[("mode", json!("top"))])
        .await;
    assert_eq!(top.output::<i64>("scroll_y").await, 0);
    flow.ok(
        BrowserScrollIntoViewNode::new(),
        &[("selector", json!("#bottom"))],
    )
    .await;
    flow.assert_scrolled().await;
}

async fn input_inside_cross_site_frame(flow: &mut Flow) {
    flow.ok(
        BrowserEnterFrameNode::new(),
        &[("selector", json!("#cross"))],
    )
    .await;
    flow.ok(
        BrowserPressKeyNode::new(),
        &[("selector", json!("input")), ("key", json!("x"))],
    )
    .await;
    assert_eq!(
        flow.script("return document.querySelector('input').value;")
            .await,
        "x"
    );
    flow.ok(
        BrowserScrollIntoViewNode::new(),
        &[("selector", json!("#frame-end"))],
    )
    .await;
    flow.assert_scrolled().await;
    flow.ok(BrowserLeaveFrameNode::new(), &[]).await;
}

async fn content_flow(site: Site) {
    let mut flow = Flow::start().await;
    let browser = flow.open(&site).await;
    flow.goto(&site.local("/content")).await;
    screenshots(&mut flow).await;
    print_pdf(&mut flow).await;
    read_markup_and_text(&mut flow, &site).await;
    snapshots(&mut flow).await;
    list_count_and_find(&mut flow).await;
    waits(&mut flow).await;
    flow.stop().await;
    browser.assert_gone("Stop Session").await;
    flow.end().await;
}

async fn screenshots(flow: &mut Flow) {
    let viewport = flow
        .script("return [innerWidth, innerHeight, devicePixelRatio];")
        .await;
    let css = |index: usize| viewport[index].as_f64().unwrap_or_default();
    let pixels = |length: f64| (length * css(2)).round() as u32;
    let shot = flow.screenshot(BrowserScreenshotNode::new(), &[]).await;
    assert_eq!(shot, (pixels(css(0)), pixels(css(1))), "{viewport}");
    let (_, full_height) = flow
        .screenshot(BrowserScreenshotNode::new(), &[("full_page", json!(true))])
        .await;
    assert!(full_height >= pixels(2400.0), "{full_height}");
    let element = flow
        .screenshot(
            BrowserScreenshotElementNode::new(),
            &[("selector", json!("#box"))],
        )
        .await;
    assert_eq!(element, (pixels(120.0), pixels(40.0)));
    let zoom = flow
        .screenshot(
            BrowserZoomScreenshotNode::new(),
            &[
                ("width", json!(100.0)),
                ("height", json!(50.0)),
                ("scale", json!(2.0)),
            ],
        )
        .await;
    assert_eq!(zoom, (pixels(200.0), pixels(100.0)));
}

async fn print_pdf(flow: &mut Flow) {
    let workspace = Workspace::new();
    let file = flow.run.flow_dir(&workspace.root, "page.pdf").await;
    let printed = flow
        .ok(BrowserPrintPdfNode::new(), &[("file_path", json!(file))])
        .await;
    let written: FlowPath = printed.output("pdf_path").await;
    assert_eq!(written.path, "page.pdf");
    let bytes = std::fs::read(workspace.root.join("page.pdf")).unwrap_or_default();
    assert!(
        bytes.starts_with(b"%PDF"),
        "page.pdf starts with {:?}",
        &bytes[..bytes.len().min(8)]
    );
}

async fn read_markup_and_text(flow: &mut Flow, site: &Site) {
    let outer = flow.html(&[("selector", json!("#article"))]).await;
    assert!(outer.starts_with("<article id=\"article\">"), "{outer}");
    let inner = flow
        .html(&[
            ("selector", json!("#article")),
            ("outer_html", json!(false)),
        ])
        .await;
    assert!(inner.starts_with("<h2>Content heading</h2>"), "{inner}");
    let source = flow.html(&[]).await;
    assert!(source.contains("<title>Content</title>"), "{source}");
    let text = flow
        .ok(
            BrowserGetPageTextNode::new(),
            &[("scope", json!("#article"))],
        )
        .await;
    let plain: String = text.output("text").await;
    assert!(
        plain.contains("Content heading") && plain.contains("First paragraph."),
        "{plain}"
    );
    let markdown: String = text.output("markdown").await;
    assert!(markdown.contains("Content heading"), "{markdown}");
    assert!(!text.output::<bool>("truncated").await);
    let short = flow
        .ok(BrowserGetPageTextNode::new(), &[("max_chars", json!(5))])
        .await;
    assert!(short.output::<bool>("truncated").await);
    assert!(short.output::<String>("text").await.chars().count() <= 5);
    let dom = flow.ok(BrowserGetDomSnapshotNode::new(), &[]).await;
    assert_eq!(dom.output::<String>("title").await, "Content");
    assert_eq!(dom.output::<String>("url").await, site.local("/content"));
    assert!(
        dom.output::<String>("html")
            .await
            .contains("id=\"article\"")
    );
}

async fn snapshots(flow: &mut Flow) {
    let tree = flow
        .ok(BrowserGetAccessibilitySnapshotNode::new(), &[])
        .await;
    let root: Value = tree.output("tree").await;
    assert_eq!(root["role"], "body", "{root}");
    let described: String = tree.output("tree_json").await;
    assert!(described.contains("Red box"), "{described}");
    let found = flow
        .ok(
            BrowserGetElementSnapshotNode::new(),
            &[("selector", json!("#box"))],
        )
        .await;
    assert!(found.activated("exec_out").await);
    assert!(
        found
            .output::<String>("tag")
            .await
            .eq_ignore_ascii_case("div")
    );
    assert_eq!(found.output::<String>("text").await, "box");
    assert_eq!(found.output::<i64>("width").await, 120);
    assert_eq!(found.output::<i64>("height").await, 40);
    assert!(found.output::<bool>("visible").await);
    let missing = flow
        .ok(
            BrowserGetElementSnapshotNode::new(),
            &[("selector", json!("#missing"))],
        )
        .await;
    assert!(missing.activated("exec_not_found").await);
    assert!(!missing.activated("exec_out").await);
}

async fn list_count_and_find(flow: &mut Flow) {
    let listed = flow
        .ok(BrowserListElementsNode::new(), &[("selector", json!("li"))])
        .await;
    assert_eq!(listed.output::<i64>("count").await, 3);
    let items: Vec<Value> = listed.output("elements").await;
    let texts: Vec<&str> = items
        .iter()
        .filter_map(|item| item["text"].as_str())
        .collect();
    assert_eq!(texts, ["one", "two", "three"]);
    let counted = flow
        .ok(
            BrowserCountElementsNode::new(),
            &[("selector", json!("li"))],
        )
        .await;
    assert_eq!(counted.output::<i64>("count").await, 3);
    let found = flow
        .ok(
            BrowserFindElementsNode::new(),
            &[
                ("role", json!("button")),
                ("name", json!("Save")),
                ("refresh", json!(true)),
            ],
        )
        .await;
    assert_eq!(found.output::<i64>("count").await, 1);
    let refs: Vec<String> = found.output("refs").await;
    let [save] = refs.as_slice() else {
        panic!("Find Elements returned the refs {refs:?}");
    };
    flow.ok(BrowserClickNode::new(), &[("selector", json!(save))])
        .await;
}

async fn waits(flow: &mut Flow) {
    flow.ok(BrowserClickNode::new(), &[("selector", json!("#later"))])
        .await;
    let appeared = flow
        .ok(
            BrowserWaitForNode::new(),
            &[("selector", json!("#late")), ("timeout_ms", json!(10_000))],
        )
        .await;
    assert!(appeared.output::<bool>("found").await);
    assert_eq!(flow.text("#late").await, "late text");
    let never = flow
        .ok(
            BrowserWaitForNode::new(),
            &[("selector", json!("#never")), ("timeout_ms", json!(200))],
        )
        .await;
    assert!(!never.output::<bool>("found").await);
    flow.script("setTimeout(function () { document.title = 'Content ready'; }, 300); return true;")
        .await;
    let met = flow
        .ok(
            BrowserWaitForConditionNode::new(),
            &[
                ("condition", json!("title_contains")),
                ("value", json!("ready")),
                ("timeout_ms", json!(10_000)),
            ],
        )
        .await;
    assert!(met.output::<bool>("met").await && met.activated("exec_out").await);
    let timed_out = flow
        .ok(
            BrowserWaitForConditionNode::new(),
            &[
                ("condition", json!("text_visible")),
                ("value", json!("never shown")),
                ("timeout_ms", json!(200)),
            ],
        )
        .await;
    assert!(!timed_out.output::<bool>("met").await);
    assert!(timed_out.activated("exec_timeout").await);
    let started = Instant::now();
    flow.ok(BrowserWaitForDelayNode::new(), &[("delay_ms", json!(100))])
        .await;
    assert!(started.elapsed() >= Duration::from_millis(100));
}

/// Start WebDriver and Stop WebDriver are kept for existing boards: the first only reports the
/// old URL, the second shuts the launched browser down.
async fn forms_and_policy_flow(site: Site) {
    let mut flow = Flow::start().await;
    let browser = flow.open(&site).await;
    let started = flow.ok(BrowserStartDriverNode::new(), &[]).await;
    assert_eq!(
        started.output::<String>("webdriver_url").await,
        "http://127.0.0.1:9515"
    );
    flow.goto(&site.local("/forms")).await;
    upload_files(&mut flow).await;
    execute_plan(&mut flow).await;
    block_script(&mut flow, &site).await;
    extra_headers(&mut flow, &site).await;
    navigation_policy(&mut flow, &site).await;
    flow.ok(BrowserStopDriverNode::new(), &[]).await;
    browser.assert_gone("Stop WebDriver").await;
    flow.stop().await;
    flow.end().await;
}

async fn upload_files(flow: &mut Flow) {
    let workspace = Workspace::new();
    let file = |name: &str| {
        let path = workspace.root.join(name);
        std::fs::write(&path, name).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        path.to_string_lossy().into_owned()
    };
    let single = flow
        .ok(
            BrowserUploadFileNode::new(),
            &[
                ("selector", json!("#single")),
                ("file_path", json!(file("single.txt"))),
            ],
        )
        .await;
    assert!(single.activated("exec_out").await);
    assert_eq!(flow.text("#single-names").await, "single.txt");
    let many = flow
        .ok(
            BrowserUploadMultipleFilesNode::new(),
            &[
                ("selector", json!("#many")),
                ("file_paths", json!([file("b.txt"), file("c.txt")])),
            ],
        )
        .await;
    assert_eq!(many.output::<i64>("uploaded_count").await, 2);
    assert_eq!(flow.text("#many-names").await, "b.txt,c.txt");
}

async fn execute_plan(flow: &mut Flow) {
    let step = |action: &str, parameters: Value| json!({"action_type": action, "target": "", "parameters": parameters, "reasoning": "node e2e"});
    let plan = json!({
        "goal_understood": true,
        "current_state_assessment": "the form is empty",
        "actions": [
            step("wait", json!({"duration_ms": 10})),
            step("select", json!({"selector": "#plan", "value": "pro"})),
            step("check", json!({"selector": "#agree"})),
            step("type", json!({"selector": "#name", "text": "Planned"})),
            step("click", json!({"selector": "#submit"})),
        ],
        "confidence": 1.0,
    });
    let executed = flow
        .ok(BrowserExecutePlanNode::new(), &[("plan", plan)])
        .await;
    assert_eq!(executed.output::<i64>("executed_count").await, 5);
    assert_eq!(flow.text("#status").await, "pro true Planned");
}

async fn block_script(flow: &mut Flow, site: &Site) {
    flow.goto(&site.local("/blocking")).await;
    assert_eq!(flow.script(BLOCKED_SCRIPT_RAN).await, true);
    let blocked = flow
        .ok(
            BrowserBlockUrlsNode::new(),
            &[("patterns", json!(["*/blocked.js"]))],
        )
        .await;
    assert_eq!(blocked.output::<i64>("blocked_count").await, 1);
    flow.goto(&site.local("/blocking")).await;
    assert_eq!(flow.script(BLOCKED_SCRIPT_RAN).await, false);
}

async fn extra_headers(flow: &mut Flow, site: &Site) {
    flow.ok(
        BrowserSetExtraHeadersNode::new(),
        &[("headers", json!({"X-Flow-Like-E2E": "node-e2e"}))],
    )
    .await;
    assert_eq!(
        flow.request_header(site, "x-flow-like-e2e")
            .await
            .as_deref(),
        Some("node-e2e")
    );
}

async fn navigation_policy(flow: &mut Flow, site: &Site) {
    let set = flow.ok(BrowserSetNavigationPolicyNode::new(), &[]).await;
    let policy: Value = set.output("policy").await;
    assert_eq!(policy["block_private_networks"], true, "{policy}");
    let blocked = flow
        .fails(
            BrowserGotoNode::new(),
            &[("url", json!(site.local("/other")))],
        )
        .await;
    assert!(
        blocked.contains("blocked by the navigation policy"),
        "{blocked}"
    );
    flow.ok(
        BrowserSetNavigationPolicyNode::new(),
        &[("enabled", json!(false))],
    )
    .await;
    flow.goto(&site.local("/other")).await;
}

async fn cookies_and_storage_flow(site: Site) {
    let mut flow = Flow::start().await;
    let browser = flow.open(&site).await;
    let workspace = Workspace::new();
    flow.goto(&site.local("/cookies")).await;
    cookie_round_trip(&mut flow, &site, &workspace).await;
    flow.goto(&site.local("/basic")).await;
    web_storage(&mut flow).await;
    flow.goto(&site.local("/frames")).await;
    browser
        .assert_out_of_process_frame(&site.loopback("/frame-form"))
        .await;
    storage_state_round_trip(&mut flow, &site, &workspace).await;
    flow.stop().await;
    browser.assert_gone("Stop Session").await;
    flow.end().await;
}

async fn cookie_round_trip(flow: &mut Flow, site: &Site, workspace: &Workspace) {
    let file = flow.run.flow_dir(&workspace.root, "cookies.json").await;
    let saved = flow
        .ok(BrowserSaveCookiesNode::new(), &[("file_path", json!(file))])
        .await;
    assert_eq!(saved.output::<i64>("cookie_count").await, 2);
    let cookies: Vec<Value> = read_json(&workspace.root.join("cookies.json"));
    assert!(
        cookies
            .iter()
            .any(|cookie| cookie["name"] == "secret" && cookie["http_only"] == true),
        "{cookies:?}"
    );
    flow.ok(BrowserClearCookiesNode::new(), &[]).await;
    assert_eq!(flow.request_header(site, "cookie").await, None);
    let loaded = flow
        .ok(BrowserLoadCookiesNode::new(), &[("file_path", json!(file))])
        .await;
    assert!(loaded.activated("exec_out").await);
    assert_eq!(loaded.output::<i64>("cookie_count").await, 2);
    assert_eq!(loaded.output::<i64>("failed_count").await, 0);
    assert_sends_both_cookies(flow, site).await;
}

async fn assert_sends_both_cookies(flow: &mut Flow, site: &Site) {
    let sent = flow
        .request_header(site, "cookie")
        .await
        .unwrap_or_default();
    let mut names: Vec<&str> = sent.split("; ").collect();
    names.sort_unstable();
    assert_eq!(names, ["secret=2", "visible=1"], "{sent}");
}

async fn web_storage(flow: &mut Flow) {
    flow.ok(
        BrowserSetLocalStorageNode::new(),
        &[("key", json!("left")), ("value", json!("1"))],
    )
    .await;
    flow.ok(
        BrowserSetSessionStorageNode::new(),
        &[("key", json!("right")), ("value", json!("2"))],
    )
    .await;
    assert_eq!(
        flow.storage_item(BrowserGetLocalStorageNode::new(), "left")
            .await,
        Some("1".to_owned())
    );
    assert_eq!(
        flow.storage_item(BrowserGetLocalStorageNode::new(), "missing")
            .await,
        None
    );
    assert_eq!(
        flow.storage_item(BrowserGetSessionStorageNode::new(), "right")
            .await,
        Some("2".to_owned())
    );
    assert_eq!(flow.all_storage("local").await, json!({"left": "1"}));
    assert_eq!(flow.all_storage("session").await, json!({"right": "2"}));
}

/// Storage State reads and writes the top-level document even while a frame of another origin
/// is entered, so it is run inside the cross-site frame.
async fn storage_state_round_trip(flow: &mut Flow, site: &Site, workspace: &Workspace) {
    let file = flow.run.flow_dir(&workspace.root, "state.json").await;
    let cross = [("selector", json!("#cross"))];
    flow.ok(BrowserEnterFrameNode::new(), &cross).await;
    let saved = flow
        .ok(
            BrowserSaveStorageStateNode::new(),
            &[
                ("file_path", json!(file)),
                ("include_session_storage", json!(true)),
            ],
        )
        .await;
    assert_eq!(saved.output::<i64>("cookie_count").await, 2);
    assert_eq!(saved.output::<i64>("origin_count").await, 1);
    let state: Value = read_json(&workspace.root.join("state.json"));
    assert_eq!(
        state["origins"],
        json!([{
            "origin": site.local(""),
            "localStorage": [{"name": "left", "value": "1"}],
            "sessionStorage": [{"name": "right", "value": "2"}],
        }])
    );
    flow.ok(BrowserLeaveFrameNode::new(), &[]).await;
    flow.ok(BrowserClearStorageNode::new(), &[]).await;
    flow.ok(BrowserClearCookiesNode::new(), &[]).await;
    assert_eq!(flow.all_storage("local").await, json!({}));
    assert_eq!(flow.all_storage("session").await, json!({}));
    flow.ok(BrowserEnterFrameNode::new(), &cross).await;
    let loaded = flow
        .ok(
            BrowserLoadStorageStateNode::new(),
            &[("file_path", json!(file))],
        )
        .await;
    assert_eq!(loaded.output::<i64>("cookies_applied").await, 2);
    assert_eq!(loaded.output::<i64>("cookies_failed").await, 0);
    assert_eq!(loaded.output::<i64>("origins_applied").await, 1);
    assert_eq!(
        loaded.output::<Vec<String>>("skipped_origins").await,
        Vec::<String>::new()
    );
    flow.ok(BrowserLeaveFrameNode::new(), &[]).await;
    assert_eq!(flow.all_storage("local").await, json!({"left": "1"}));
    assert_eq!(flow.all_storage("session").await, json!({"right": "2"}));
    assert_sends_both_cookies(flow, site).await;
}

fn read_json<T: DeserializeOwned>(path: &Path) -> T {
    let bytes = std::fs::read(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    serde_json::from_slice(&bytes).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

async fn network_flow(site: Site) {
    let mut flow = Flow::start().await;
    let browser = flow.open(&site).await;
    flow.goto(&site.local("/basic")).await;
    console_observer(&mut flow).await;
    network_observer(&mut flow).await;
    basic_auth(&mut flow, &site).await;
    flow.stop().await;
    browser.assert_gone("Stop Session").await;
    flow.end().await;
}

async fn console_observer(flow: &mut Flow) {
    flow.ok(BrowserStartConsoleObserverNode::new(), &[]).await;
    flow.script("console.log('hello-observer'); console.error('bad-observer'); return true;")
        .await;
    let deadline = Instant::now() + CLEANUP_BUDGET;
    loop {
        let logs = flow.console_logs("").await;
        if logged(&logs, "hello-observer") && logged(&logs, "bad-observer") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the console observer never reported both messages: {logs:?}"
        );
        tokio::time::sleep(POLL).await;
    }
    let all = flow.ok(BrowserGetConsoleLogsNode::new(), &[]).await;
    assert!(all.output::<bool>("has_errors").await);
    let errors = flow.console_logs("error").await;
    assert!(
        errors.iter().all(|entry| entry["level"] == "error")
            && logged(&errors, "bad-observer")
            && !logged(&errors, "hello-observer"),
        "{errors:?}"
    );
    flow.ok(BrowserClearConsoleLogsNode::new(), &[]).await;
    let cleared = flow.console_logs("").await;
    assert!(cleared.is_empty(), "{cleared:?}");
}

fn logged(entries: &[Value], text: &str) -> bool {
    entries.iter().any(|entry| {
        entry["text"]
            .as_str()
            .is_some_and(|line| line.contains(text))
    })
}

async fn network_observer(flow: &mut Flow) {
    flow.ok(
        BrowserStartNetworkObserverNode::new(),
        &[("url_pattern", json!("/api/"))],
    )
    .await;
    flow.script("fetch('/api/data').then(function (response) { return response.text(); }).then(function (text) { document.title = text; }); return true;")
        .await;
    let idle = flow
        .ok(
            BrowserWaitForNetworkIdleNode::new(),
            &[("idle_time_ms", json!(300)), ("timeout_ms", json!(10_000))],
        )
        .await;
    assert!(
        idle.activated("exec_out").await,
        "Wait For Network Idle timed out"
    );
    let deadline = Instant::now() + CLEANUP_BUDGET;
    loop {
        let requests = flow.network_requests(false).await;
        if requests.iter().any(|request| {
            request["url"]
                .as_str()
                .is_some_and(|url| url.ends_with("/api/data"))
                && request["status"] == 200
        }) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the network observer never reported /api/data: {requests:?}"
        );
        tokio::time::sleep(POLL).await;
    }
    assert!(!flow.network_requests(true).await.is_empty());
    let cleared = flow.network_requests(false).await;
    assert!(cleared.is_empty(), "{cleared:?}");
}

async fn basic_auth(flow: &mut Flow, site: &Site) {
    flow.ok(
        BrowserSetBasicAuthNode::new(),
        &[
            ("username", json!(AUTH_USER)),
            ("password", json!(AUTH_PASSWORD)),
            ("origin", json!(site.local(""))),
        ],
    )
    .await;
    flow.goto(&site.local("/private")).await;
    assert_eq!(flow.text("#who").await, AUTH_USER);
}

async fn tabs_and_attach_flow(site: Site) {
    let mut flow = Flow::start().await;
    let browser = flow.open(&site).await;
    flow.goto(&site.local("/basic")).await;
    let first = flow.current_tab();
    new_and_closed_page(&mut flow, &site).await;
    assert_eq!(flow.current_tab(), first);
    attach_a_second_session(&site, &browser, &first).await;
    browser.assert_running();
    assert_eq!(flow.text("#greeting").await, "quiet");
    flow.stop().await;
    browser.assert_gone("Stop Session").await;
    flow.end().await;
}

async fn new_and_closed_page(flow: &mut Flow, site: &Site) {
    let before = handles(&flow.tabs().await);
    flow.ok(BrowserNewPageNode::new(), &[]).await;
    let opened = flow.current_tab();
    assert!(!before.contains(&opened), "New Page kept the tab {opened}");
    assert_eq!(handles(&flow.tabs().await).len(), before.len() + 1);
    flow.goto(&site.local("/other")).await;
    assert_eq!(flow.text("p").await, "Other page");
    flow.ok(BrowserClosePageNode::new(), &[]).await;
    assert_eq!(handles(&flow.tabs().await), before);
}

/// A second run attaches to the launched browser through its debugger address, works in a tab of
/// its own and detaches; the browser stays up for the run that launched it.
async fn attach_a_second_session(site: &Site, browser: &LaunchedBrowser, tab: &str) {
    let mut attached = Flow::start().await;
    attached
        .ok(
            BrowserAttachNode::new(),
            &[
                ("webdriver_url", json!(site.loopback("/no-webdriver"))),
                ("debugger_address", json!(browser.debugger_address)),
                ("browser_type", json!("Chrome")),
            ],
        )
        .await;
    let tabs = handles(&attached.tabs().await);
    assert!(tabs.iter().any(|handle| handle == tab), "{tabs:?}");
    new_and_closed_page(&mut attached, site).await;
    attached.stop().await;
    attached.end().await;
}

/// The first run resolves Profile Directory (a local Flow path), the second reopens the same
/// folder through Profile Path, which only works once Stop Session released the first browser.
async fn persistent_profile_flow(site: Site) {
    let workspace = Workspace::new();
    let first = Flow::start().await;
    let directory = first.run.flow_dir(&workspace.root, "profile").await;
    let profile = open_and_stop(first, &site, ("user_data_dir", json!(directory))).await;
    assert_eq!(
        canonical(&profile),
        canonical(&workspace.root.join("profile"))
    );
    let path = profile.to_string_lossy().into_owned();
    let reopened = open_and_stop(Flow::start().await, &site, ("user_data_path", json!(path))).await;
    assert_eq!(reopened, profile);
    assert!(
        profile.join("Default").join("Preferences").is_file(),
        "the persistent profile {} lost its preferences",
        profile.display()
    );
}

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

async fn open_and_stop(mut flow: Flow, site: &Site, profile: (&str, Value)) -> PathBuf {
    let browser = flow.open_persistent(site, profile).await;
    flow.goto(&site.local("/basic")).await;
    flow.stop().await;
    browser.assert_gone("Stop Session").await;
    flow.end().await;
    browser.profile
}

async fn failed_run_flow(site: Site) {
    let mut flow = Flow::start().await;
    let browser = flow.open(&site).await;
    flow.goto(&site.local("/basic")).await;
    let error = flow
        .fails(BrowserClickNode::new(), &[("selector", json!("#missing"))])
        .await;
    assert!(error.contains("#missing"), "{error}");
    flow.end().await;
    browser.assert_gone("a failed run").await;
}

async fn cancelled_run_flow(site: Site) {
    let mut flow = Flow::start().await;
    let browser = flow.open(&site).await;
    flow.goto(&site.local("/basic")).await;
    flow.run.cancel();
    browser.assert_no_process("a cancelled run").await;
    let error = flow
        .fails(
            BrowserGetTextNode::new(),
            &[("selector", json!("#greeting"))],
        )
        .await;
    assert_eq!(error, NO_BROWSER_ERROR);
    flow.end().await;
    browser.assert_gone("a cancelled run").await;
}

async fn dropped_run_flow(site: Site) {
    let mut flow = Flow::start().await;
    let browser = flow.open(&site).await;
    flow.goto(&site.local("/basic")).await;
    drop(flow);
    browser.assert_gone("a dropped run").await;
}

async fn open_and_stop_once(site: Site) {
    let mut flow = Flow::start().await;
    let browser = flow.open(&site).await;
    flow.goto(&site.local("/basic")).await;
    flow.stop().await;
    browser.assert_gone("Stop Session").await;
    flow.end().await;
}

async fn with_browser<F, Fut>(name: &str, flow: F)
where
    F: FnOnce(Site) -> Fut,
    Fut: Future<Output = ()>,
{
    discovery().await;
    let _turn = BROWSERS.lock().await;
    with_ran_nodes(BTreeSet::clear);
    if tokio::time::timeout(TEST_BUDGET, flow(Site::start()))
        .await
        .is_err()
    {
        panic!("{name} did not finish within {TEST_BUDGET:?}");
    }
    assert_flow_ran_its_nodes(name);
}

fn with_ran_nodes<T>(use_nodes: impl FnOnce(&mut BTreeSet<String>) -> T) -> T {
    use_nodes(&mut RAN.lock().unwrap_or_else(PoisonError::into_inner))
}

/// Runs while the browser turn is held, so only the nodes of this flow were recorded.
fn assert_flow_ran_its_nodes(flow: &str) {
    let listed = COVERAGE
        .iter()
        .find(|(name, _)| *name == flow)
        .map_or(&[][..], |(_, nodes)| *nodes);
    let missing: Vec<&str> = with_ran_nodes(|ran| {
        listed
            .iter()
            .copied()
            .filter(|node| !ran.contains(*node))
            .collect()
    });
    assert!(
        missing.is_empty(),
        "{flow} is listed as running {missing:?}, but they never succeeded in it"
    );
}

/// Holds the browser turn so no browser test can race the folder check, and proves that the
/// node failed before anything was launched.
async fn without_browser<F, Fut>(flow: F)
where
    F: FnOnce(Site) -> Fut,
    Fut: Future<Output = ()>,
{
    let _turn = BROWSERS.lock().await;
    let before = owned_dirs();
    flow(Site::start()).await;
    let created: Vec<PathBuf> = owned_dirs().difference(&before).cloned().collect();
    assert!(created.is_empty(), "a rejected node created {created:?}");
}

struct Discovery {
    /// The build this run installed, when `FLOW_LIKE_BROWSER_E2E_PROVISION` is set.
    installed: Option<Executable>,
    /// The executable Open Browser launches for Browser Type Chrome, found the way the node finds it.
    found: Executable,
}

async fn discovery() -> &'static Discovery {
    DISCOVERY.get_or_init(discover).await
}

async fn discover() -> Discovery {
    let cache = launch::default_cache_dir();
    let installed = match provision_version() {
        Some(version) => Some(install_cft(version, &cache).await),
        None => None,
    };
    let found = launch::find(BrowserKind::Chrome, &cache).unwrap_or_else(|error| {
        panic!(
            "browser_nodes_e2e needs Chrome, Chromium or a cached Chrome for Testing and never skips: {error}"
        )
    });
    eprintln!(
        "browser_nodes_e2e: Open Browser launches {} ({:?}, {:?}, version {})",
        found.path.display(),
        found.source,
        found.flavor,
        version_of(&found)
    );
    Discovery { installed, found }
}

/// The Chrome for Testing build `FLOW_LIKE_BROWSER_E2E_PROVISION` installs: `stable` the newest
/// Stable one, as Settings > Automation > Install does, or `download` the pinned one.
fn provision_version() -> Option<CftVersion> {
    let mode = std::env::var(PROVISION_ENV)
        .ok()
        .filter(|mode| !mode.is_empty())?;
    match mode.as_str() {
        "stable" => Some(CftVersion::Stable),
        "download" => Some(CftVersion::Pinned),
        _ => panic!("{PROVISION_ENV}={mode} is neither stable nor download"),
    }
}

/// Installs into the folder Open Browser searches, never into (and prunes) the user's own cache.
async fn install_cft(version: CftVersion, cache: &Path) -> Executable {
    assert!(
        std::env::var_os(CACHE_ENV).is_some_and(|dir| !dir.is_empty()),
        "{PROVISION_ENV} installs Chrome for Testing where Open Browser looks for it, so it needs {CACHE_ENV}; without it that is your own cache {}",
        cache.display()
    );
    let installed = cft::install(version.clone(), cache, |_| {})
        .await
        .unwrap_or_else(|error| {
            panic!(
                "{PROVISION_ENV} could not install Chrome for Testing ({version:?}) into {}: {error}",
                cache.display()
            )
        });
    eprintln!(
        "browser_nodes_e2e: {PROVISION_ENV} installed Chrome for Testing {} ({version:?}) at {}",
        version_of(&installed),
        installed.path.display()
    );
    installed
}

fn version_of(executable: &Executable) -> &str {
    executable.version.as_deref().unwrap_or("unknown")
}

/// Installed browsers can start through wrapper scripts, so only a downloaded build has a known
/// executable.
async fn expected_program() -> Option<PathBuf> {
    let found = &discovery().await.found;
    (found.source == ExecutableSource::CachedCft).then(|| found.path.clone())
}

struct Run {
    root: ExecutionContext,
}

impl Run {
    async fn live() -> Self {
        let logic: Arc<dyn NodeLogic> = Arc::new(StartSessionNode::new());
        let node = internal_node(logic.clone(), logic.get_node());
        let state = Arc::new(FlowLikeState::new(
            FlowLikeConfig::new(),
            HTTPClient::new_without_refetch(),
        ));
        let variables = Arc::new(Mutex::new(AHashMap::new()));
        let cache = Arc::new(RwLock::new(AHashMap::new()));
        let run: Weak<Mutex<FlowRun>> = Weak::new();
        let mut root = ExecutionContext::new(
            Arc::new(AHashMap::from_iter([(
                node.node_id().to_string(),
                node.clone(),
            )])),
            &run,
            &state,
            &node,
            &variables,
            &cache,
            LogLevel::Debug,
            ExecutionStage::Dev,
            Arc::new(Profile::default()),
            None,
            Arc::new(RwLock::new(vec![])),
            None,
            None,
            Arc::new(AHashMap::new()),
            None,
        )
        .await;
        root.resources = Arc::new(RunResources::default());
        Self { root }
    }

    async fn node(&self, logic: impl IntoLogic, inputs: &[(&str, Value)]) -> NodeRun {
        let logic = logic.into_logic();
        let schema = logic.get_node();
        let name = schema.name.clone();
        let node = internal_node(logic.clone(), schema);
        let mut context = self.root.create_sub_context(&node).await;
        for (pin, value) in inputs {
            if let Err(error) = context.set_pin_value(pin, value.clone()).await {
                panic!("{name}: setting input {pin} failed: {error:#}");
            }
        }
        let result = logic.run(&mut context).await;
        if result.is_ok() {
            with_ran_nodes(|ran| ran.insert(name.clone()));
        }
        NodeRun {
            name,
            context,
            result,
        }
    }

    async fn flow_dir(&self, root: &Path, child: &str) -> FlowPath {
        let mut context = self.root.create_sub_context(&self.root.node).await;
        let store = FlowPath::from_pathbuf(root.to_path_buf(), &mut context)
            .await
            .unwrap_or_else(|error| panic!("local store for {}: {error:#}", root.display()));
        FlowPath::new(child.to_owned(), store.store_ref, None)
    }

    fn cancel(&self) {
        self.root.resources.abort();
    }

    async fn end(&self) {
        self.root.resources.shutdown().await;
        self.root.cache.write().await.clear();
    }
}

fn internal_node(logic: Arc<dyn NodeLogic>, schema: Node) -> Arc<InternalNode> {
    let pins = schema
        .pins
        .values()
        .map(|pin| (pin.id.clone(), Arc::new(InternalPin::new(pin, false))))
        .collect();
    let node = Arc::new(InternalNode::new(schema, pins, logic, AHashMap::new()));
    for pin in node.pins.iter() {
        pin.init_node(Arc::downgrade(&node));
        pin.init_connected_to(vec![]);
        pin.init_depends_on(vec![]);
    }
    node
}

struct NodeRun {
    name: String,
    context: ExecutionContext,
    result: flow_like_types::Result<()>,
}

impl NodeRun {
    fn expect_ok(self) -> Self {
        if let Err(error) = &self.result {
            panic!("{} failed: {error:#}", self.name);
        }
        self
    }

    fn expect_err(self) -> String {
        match self.result {
            Ok(()) => panic!("{} succeeded, but an error was expected", self.name),
            Err(error) => format!("{error:#}"),
        }
    }

    async fn output<T: DeserializeOwned>(&self, pin: &str) -> T {
        self.context
            .evaluate_pin(pin)
            .await
            .unwrap_or_else(|error| panic!("{} output {pin}: {error:#}", self.name))
    }

    async fn optional_output<T: DeserializeOwned>(&self, pin: &str) -> Option<T> {
        self.context.evaluate_pin(pin).await.ok()
    }

    async fn activated(&self, pin: &str) -> bool {
        self.optional_output(pin).await.unwrap_or(false)
    }
}

struct Flow {
    run: Run,
    session: AutomationSession,
}

impl Flow {
    async fn start() -> Self {
        let run = Run::live().await;
        let started = run
            .node(
                StartSessionNode::new(),
                &[("default_delay_ms", json!(0)), ("click_delay_ms", json!(0))],
            )
            .await
            .expect_ok();
        let session = started.output("session").await;
        Self { run, session }
    }

    async fn call(&self, logic: impl IntoLogic, inputs: &[(&str, Value)]) -> NodeRun {
        let mut all = vec![("session", json!(self.session))];
        all.extend_from_slice(inputs);
        self.run.node(logic, &all).await
    }

    async fn ok(&mut self, logic: impl IntoLogic, inputs: &[(&str, Value)]) -> NodeRun {
        let node = self.call(logic, inputs).await.expect_ok();
        if let Some(session) = node.optional_output("session_out").await {
            self.session = session;
        }
        node
    }

    async fn fails(&self, logic: impl IntoLogic, inputs: &[(&str, Value)]) -> String {
        self.call(logic, inputs).await.expect_err()
    }

    async fn open(&mut self, site: &Site) -> LaunchedBrowser {
        let before = owned_dirs();
        let address = self.launch(site, &[]).await;
        LaunchedBrowser::temporary(before, address, expected_program().await)
    }

    async fn open_persistent(&mut self, site: &Site, profile: (&str, Value)) -> LaunchedBrowser {
        let before = owned_dirs();
        let address = self.launch(site, &[profile]).await;
        let directory = self
            .session
            .browser_user_data_dir
            .clone()
            .map(PathBuf::from)
            .expect("Open Browser records the persistent profile in the session");
        let temporary: Vec<PathBuf> = owned_dirs()
            .difference(&before)
            .filter(|path| has_prefix(path, PROFILE_PREFIX))
            .cloned()
            .collect();
        assert!(
            temporary.is_empty(),
            "a persistent profile needs no temporary one: {temporary:?}"
        );
        LaunchedBrowser::running(before, directory, address, expected_program().await)
    }

    async fn launch(&mut self, site: &Site, extra: &[(&str, Value)]) -> String {
        let mut inputs = vec![
            ("webdriver_url", json!(site.loopback("/no-webdriver"))),
            ("headless", json!(true)),
            ("viewport_width", json!(1280)),
            ("viewport_height", json!(900)),
        ];
        inputs.extend_from_slice(extra);
        let opened = self.ok(BrowserOpenNode::new(), &inputs).await;
        let session = json!(self.session);
        assert_eq!(session["browser_type"], "Chrome", "{session}");
        assert_eq!(session["browser_headless"], true, "{session}");
        opened.output("debugger_address").await
    }

    async fn goto(&mut self, url: &str) {
        let landed = self
            .ok(BrowserGotoNode::new(), &[("url", json!(url))])
            .await;
        assert_eq!(landed.output::<String>("final_url").await, url);
    }

    async fn snapshot(&mut self) -> Vec<Value> {
        self.ok(BrowserSnapshotNode::new(), &[])
            .await
            .output("elements")
            .await
    }

    async fn text(&mut self, selector: &str) -> String {
        self.ok(BrowserGetTextNode::new(), &[("selector", json!(selector))])
            .await
            .output("text")
            .await
    }

    async fn script(&mut self, body: &str) -> Value {
        self.ok(BrowserExecuteJsNode::new(), &[("script", json!(body))])
            .await
            .output("result")
            .await
    }

    async fn element_state(&mut self, target: &str) -> Value {
        self.ok(
            BrowserGetElementStateNode::new(),
            &[("selector", json!(target))],
        )
        .await
        .output("state")
        .await
    }

    async fn dialog(&mut self, action: &str, text: &str) -> String {
        self.ok(
            BrowserHandleDialogNode::new(),
            &[("action", json!(action)), ("text", json!(text))],
        )
        .await
        .output("dialog_text")
        .await
    }

    async fn tabs(&mut self) -> Vec<Value> {
        self.ok(BrowserListTabsNode::new(), &[])
            .await
            .output("tabs")
            .await
    }

    async fn path(&mut self) -> Value {
        self.script("return location.pathname;").await
    }

    async fn assert_logged(&mut self, entries: &str) {
        let log = self
            .script("return document.getElementById('log').textContent;")
            .await;
        let log = log.as_str().unwrap_or_default();
        assert!(log.ends_with(&format!("{entries} ")), "#log is {log:?}");
    }

    async fn assert_scrolled(&mut self) {
        let offset = self.script("return window.scrollY;").await;
        assert!(offset.as_f64().is_some_and(|y| y > 0.0), "scrollY {offset}");
    }

    async fn screenshot(&mut self, logic: impl IntoLogic, inputs: &[(&str, Value)]) -> (u32, u32) {
        let taken = self.ok(logic, inputs).await;
        png_size(&taken.output::<String>("screenshot").await)
    }

    async fn html(&mut self, inputs: &[(&str, Value)]) -> String {
        self.ok(BrowserGetHtmlNode::new(), inputs)
            .await
            .output("html")
            .await
    }

    /// The header as the test site received it on a fresh request from the current tab.
    async fn request_header(&mut self, site: &Site, name: &str) -> Option<String> {
        self.goto(&site.local("/headers")).await;
        let head = self.text("#head").await;
        header(&head, name)
    }

    async fn storage_item(&mut self, logic: impl IntoLogic, key: &str) -> Option<String> {
        let read = self.ok(logic, &[("key", json!(key))]).await;
        let value: String = read.output("value").await;
        read.output::<bool>("exists").await.then_some(value)
    }

    async fn all_storage(&mut self, storage_type: &str) -> Value {
        let read = self
            .ok(
                BrowserGetAllStorageNode::new(),
                &[("storage_type", json!(storage_type))],
            )
            .await;
        let data: Value = read.output("data").await;
        let count: i64 = read.output("count").await;
        assert_eq!(
            data.as_object().map(|items| items.len() as i64),
            Some(count),
            "{data}"
        );
        data
    }

    async fn console_logs(&mut self, level: &str) -> Vec<Value> {
        self.ok(
            BrowserGetConsoleLogsNode::new(),
            &[("level_filter", json!(level))],
        )
        .await
        .output("logs")
        .await
    }

    async fn network_requests(&mut self, clear_after: bool) -> Vec<Value> {
        self.ok(
            BrowserGetNetworkRequestsNode::new(),
            &[("clear_after", json!(clear_after))],
        )
        .await
        .output("requests")
        .await
    }

    fn current_tab(&self) -> String {
        self.session
            .current_window_handle
            .clone()
            .expect("the session has a current tab")
    }

    async fn assert_only_tab_is_current(&mut self) {
        let current = self.current_tab();
        assert_eq!(handles(&self.tabs().await), vec![current]);
    }

    async fn stop(&mut self) {
        self.ok(StopSessionNode::new(), &[]).await;
    }

    async fn end(&self) {
        self.run.end().await;
    }
}

fn ref_named(elements: &[Value], name: &str) -> String {
    elements
        .iter()
        .filter(|element| element["name"] == name)
        .find_map(|element| element["ref"].as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| {
            let listed: Vec<String> = elements
                .iter()
                .map(|element| format!("{} {}", element["role"], element["name"]))
                .collect();
            panic!("the browser snapshot has no ref named {name:?}; it lists {listed:?}")
        })
}

fn handles(tabs: &[Value]) -> Vec<String> {
    tabs.iter()
        .filter_map(|tab| tab["handle"].as_str())
        .map(str::to_owned)
        .collect()
}

fn png_size(encoded: &str) -> (u32, u32) {
    use flow_like_types::base64::Engine;
    let bytes = flow_like_types::base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .unwrap_or_else(|error| panic!("the screenshot is not base64: {error}"));
    assert!(
        bytes.starts_with(PNG_SIGNATURE) && bytes.len() >= 24,
        "the screenshot is not a PNG"
    );
    let word =
        |at: usize| u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
    (word(16), word(20))
}

struct LaunchedBrowser {
    before: BTreeSet<PathBuf>,
    profile: PathBuf,
    debugger_address: String,
}

impl LaunchedBrowser {
    fn temporary(
        before: BTreeSet<PathBuf>,
        debugger_address: String,
        program: Option<PathBuf>,
    ) -> Self {
        let created: Vec<PathBuf> = owned_dirs()
            .difference(&before)
            .filter(|path| has_prefix(path, PROFILE_PREFIX))
            .cloned()
            .collect();
        let [profile] = created.as_slice() else {
            panic!("Open Browser should create one temporary profile, found {created:?}");
        };
        Self::running(before, profile.clone(), debugger_address, program)
    }

    /// The process checks below only mean something if the launched browser is found by its
    /// profile path in the process list. With `program` set, that browser must run it.
    fn running(
        before: BTreeSet<PathBuf>,
        profile: PathBuf,
        debugger_address: String,
        program: Option<PathBuf>,
    ) -> Self {
        let processes = processes_using(&profile);
        assert!(
            !processes.is_empty(),
            "no running process names the profile {}",
            profile.display()
        );
        if let Some(program) = program {
            assert!(
                runs_program(&processes, &program),
                "Open Browser did not launch {}; the processes using {} are {processes:?}",
                program.display(),
                profile.display()
            );
        }
        Self {
            before,
            profile,
            debugger_address,
        }
    }

    fn assert_running(&self) {
        assert!(
            !processes_using(&self.profile).is_empty(),
            "the browser using {} is gone",
            self.profile.display()
        );
    }

    fn debugger_port(&self) -> u16 {
        self.debugger_address
            .strip_prefix("localhost:")
            .and_then(|port| port.parse::<u16>().ok())
            .unwrap_or_else(|| panic!("Debugger Address {:?}", self.debugger_address))
    }

    fn assert_debugger_endpoint(&self) {
        let version = devtools_json(self.debugger_port(), "/json/version");
        assert!(
            version["Browser"]
                .as_str()
                .is_some_and(|name| !name.is_empty()),
            "{version}"
        );
        assert!(
            version["webSocketDebuggerUrl"]
                .as_str()
                .is_some_and(|url| url.contains("/devtools/browser/")),
            "{version}"
        );
    }

    /// The frame cases only cover child-session paths while Chrome lists the frame as its own
    /// `iframe` target, which it does when the frame is out of process.
    async fn assert_out_of_process_frame(&self, url: &str) {
        let port = self.debugger_port();
        eventually("the frame load", || {
            let list = devtools_json(port, "/json/list");
            let targets = list.as_array().map(Vec::as_slice).unwrap_or_default();
            if targets
                .iter()
                .any(|target| target["type"] == "iframe" && target["url"] == url)
            {
                return Ok(());
            }
            let listed: Vec<String> = targets
                .iter()
                .map(|target| format!("{} {}", target["type"], target["url"]))
                .collect();
            Err(format!(
                "no out-of-process iframe target for {url}; DevTools lists {listed:?}"
            ))
        })
        .await;
    }

    async fn assert_no_process(&self, after: &str) {
        eventually(after, || {
            let processes = processes_using(&self.profile);
            if processes.is_empty() {
                return Ok(());
            }
            Err(format!(
                "browser processes still use {}: {processes:?}",
                self.profile.display()
            ))
        })
        .await;
    }

    async fn assert_gone(&self, after: &str) {
        self.assert_no_process(after).await;
        eventually(after, || {
            let leftovers: Vec<PathBuf> = owned_dirs().difference(&self.before).cloned().collect();
            if leftovers.is_empty() {
                return Ok(());
            }
            Err(format!("browser folders remain: {leftovers:?}"))
        })
        .await;
    }
}

async fn eventually(after: &str, check: impl Fn() -> Result<(), String>) {
    let deadline = Instant::now() + CLEANUP_BUDGET;
    loop {
        let Err(problem) = check() else {
            return;
        };
        if Instant::now() >= deadline {
            panic!("after {after}: {problem}");
        }
        tokio::time::sleep(POLL).await;
    }
}

fn owned_dirs() -> BTreeSet<PathBuf> {
    let pid = std::process::id();
    let prefixes = [
        format!("{PROFILE_PREFIX}{pid}-"),
        format!("{STAGING_PREFIX}{pid}-"),
    ];
    scratch_roots()
        .iter()
        .flat_map(|root| entries(root))
        .filter(|path| prefixes.iter().any(|prefix| has_prefix(path, prefix)))
        .collect()
}

/// An unreadable folder fails the test instead of looking empty; only a missing one is empty.
fn entries(root: &Path) -> Vec<PathBuf> {
    match std::fs::read_dir(root) {
        Ok(entries) => entries
            .map(|entry| {
                entry
                    .unwrap_or_else(|error| panic!("listing {}: {error}", root.display()))
                    .path()
            })
            .collect(),
        Err(error) if error.kind() == ErrorKind::NotFound => Vec::new(),
        Err(error) => panic!("listing {}: {error}", root.display()),
    }
}

fn has_prefix(path: &Path, prefix: &str) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with(prefix))
}

fn scratch_roots() -> Vec<PathBuf> {
    let mut roots = vec![std::env::temp_dir()];
    if cfg!(target_os = "linux")
        && let Some(home) = std::env::var_os("HOME")
    {
        roots.push(PathBuf::from(home).join("snap/chromium/common"));
    }
    roots
}

/// "pid command line" of every process naming `profile`. A listing that fails or misses this
/// test process panics, so "no process left" can never come from a scan that saw nothing.
fn processes_using(profile: &Path) -> Vec<String> {
    let (tool, mut command) = process_lister();
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("{tool} lists the running processes: {error}"));
    let listing = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{tool} failed to list the running processes ({}): {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let own = std::process::id().to_string();
    assert!(
        listing
            .lines()
            .any(|line| line.split_whitespace().next() == Some(own.as_str())),
        "{tool} listed {} processes but not this test (pid {own})",
        listing.lines().count()
    );
    let needle = profile.to_string_lossy();
    listing
        .lines()
        .filter(|line| line.contains(needle.as_ref()))
        .map(str::to_owned)
        .collect()
}

#[cfg(unix)]
fn process_lister() -> (&'static str, std::process::Command) {
    let mut command = std::process::Command::new("ps");
    command.args(["-A", "-ww", "-o", "pid=", "-o", "args="]);
    ("ps", command)
}

#[cfg(windows)]
fn process_lister() -> (&'static str, std::process::Command) {
    let script = "[Console]::OutputEncoding = [Text.Encoding]::UTF8; Get-CimInstance Win32_Process | ForEach-Object { \"$($_.ProcessId) $($_.CommandLine)\" }";
    let mut command = std::process::Command::new("powershell");
    command.args(["-NoProfile", "-NonInteractive", "-Command", script]);
    ("PowerShell", command)
}

/// Chrome on Linux retitles itself with the resolved `/proc/self/exe`, so the executable of each
/// listed process is read from `/proc` and compared with the resolved `program`.
#[cfg(target_os = "linux")]
fn runs_program(processes: &[String], program: &Path) -> bool {
    let expected = std::fs::canonicalize(program)
        .unwrap_or_else(|error| panic!("resolving {}: {error}", program.display()));
    processes.iter().any(|line| {
        line.split_whitespace()
            .next()
            .and_then(|pid| std::fs::read_link(format!("/proc/{pid}/exe")).ok())
            .is_some_and(|executable| executable == expected)
    })
}

/// The launcher starts `std::path::absolute(program)`, and Windows paths ignore case.
#[cfg(windows)]
fn runs_program(processes: &[String], program: &Path) -> bool {
    let expected = std::path::absolute(program)
        .unwrap_or_else(|error| panic!("resolving {}: {error}", program.display()))
        .to_string_lossy()
        .to_lowercase();
    processes
        .iter()
        .any(|line| line.to_lowercase().contains(&expected))
}

/// macOS lists the path the launcher passed, unchanged.
#[cfg(all(unix, not(target_os = "linux")))]
fn runs_program(processes: &[String], program: &Path) -> bool {
    let expected = program.to_string_lossy();
    processes
        .iter()
        .any(|line| line.contains(expected.as_ref()))
}

fn devtools_json(port: u16, path: &str) -> Value {
    let mut stream = TcpStream::connect(("127.0.0.1", port))
        .unwrap_or_else(|error| panic!("DevTools endpoint 127.0.0.1:{port}: {error}"));
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("DevTools read timeout");
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: localhost:{port}\r\nConnection: close\r\n\r\n"
    )
    .expect("DevTools request");
    let (head, body) = read_response(&mut stream);
    assert!(head.starts_with("HTTP/1.1 200"), "{path}: {head}");
    serde_json::from_slice(&body)
        .unwrap_or_else(|error| panic!("{path}: {error}: {}", String::from_utf8_lossy(&body)))
}

fn read_response(stream: &mut TcpStream) -> (String, Vec<u8>) {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 4096];
    while !response_complete(&bytes) {
        match stream.read(&mut chunk) {
            Ok(read) if read > 0 => bytes.extend_from_slice(&chunk[..read]),
            _ => break,
        }
    }
    let split = head_end(&bytes).unwrap_or(bytes.len());
    (
        String::from_utf8_lossy(&bytes[..split]).into_owned(),
        bytes[split..].to_vec(),
    )
}

fn response_complete(bytes: &[u8]) -> bool {
    let Some(end) = head_end(bytes) else {
        return false;
    };
    content_length(&String::from_utf8_lossy(&bytes[..end]))
        .is_some_and(|length| bytes.len() >= end + length)
}

fn head_end(bytes: &[u8]) -> Option<usize> {
    bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
}

fn content_length(head: &str) -> Option<usize> {
    header(head, "content-length")?.parse().ok()
}

/// A header value of an HTTP request or response head; the first line is the request or status
/// line.
fn header(head: &str, name: &str) -> Option<String> {
    head.lines().skip(1).find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.trim()
            .eq_ignore_ascii_case(name)
            .then(|| value.trim().to_owned())
    })
}

struct Workspace {
    root: PathBuf,
}

impl Workspace {
    fn new() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        let root = std::env::temp_dir().join(format!(
            "flow-like-node-e2e-{}-{nanos}-{}",
            std::process::id(),
            WORKSPACES.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root)
            .unwrap_or_else(|error| panic!("workspace {}: {error}", root.display()));
        Self { root }
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Serves the fixture pages on one port: `localhost` and `127.0.0.1` are different sites, so a
/// `localhost` page embedding a `127.0.0.1` frame gets an out-of-process iframe.
struct Site {
    port: u16,
    stop: Arc<AtomicBool>,
    slow_tail: Arc<SlowTail>,
}

impl Site {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the fixture site");
        listener
            .set_nonblocking(true)
            .expect("nonblocking fixture listener");
        let port = listener.local_addr().expect("fixture site address").port();
        let site = Self {
            port,
            stop: Arc::new(AtomicBool::new(false)),
            slow_tail: Arc::new(SlowTail::default()),
        };
        let server = Server {
            port,
            slow_tail: site.slow_tail.clone(),
        };
        let stop = site.stop.clone();
        std::thread::spawn(move || server.accept_loop(&listener, &stop));
        site
    }

    fn local(&self, path: &str) -> String {
        format!("http://localhost:{}{path}", self.port)
    }

    fn loopback(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    fn slow_page_finished(&self) -> bool {
        self.slow_tail.sent.load(Ordering::Acquire)
    }

    fn release_slow_tail(&self) {
        self.slow_tail.release();
    }
}

impl Drop for Site {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.slow_tail.release();
    }
}

/// `/slow` sends its head at once and holds its tail until the test releases it, so the popup
/// is still loading when Select Tab picks it.
#[derive(Default)]
struct SlowTail {
    released: std::sync::Mutex<bool>,
    changed: Condvar,
    sent: AtomicBool,
}

impl SlowTail {
    fn release(&self) {
        *self.released.lock().unwrap_or_else(PoisonError::into_inner) = true;
        self.changed.notify_all();
    }

    fn wait_for_release(&self) {
        let released = self.released.lock().unwrap_or_else(PoisonError::into_inner);
        let (_released, _timeout) = self
            .changed
            .wait_timeout_while(released, SLOW_TAIL_CAP, |released| !*released)
            .unwrap_or_else(PoisonError::into_inner);
    }
}

#[derive(Clone)]
struct Server {
    port: u16,
    slow_tail: Arc<SlowTail>,
}

impl Server {
    fn accept_loop(&self, listener: &TcpListener, stop: &AtomicBool) {
        while !stop.load(Ordering::Acquire) {
            match listener.accept() {
                Ok((stream, _)) => {
                    let server = self.clone();
                    std::thread::spawn(move || server.serve(stream));
                }
                Err(error) if is_transient_accept_error(&error) => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!(
                    "the fixture site on port {} stopped accepting connections: {error}",
                    self.port
                ),
            }
        }
    }

    fn serve(&self, mut stream: TcpStream) {
        let _ = stream.set_nonblocking(false);
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        if let Some(head) = request_head(&mut stream) {
            let _ = self.respond(&mut stream, &head);
        }
    }

    fn respond(&self, stream: &mut TcpStream, head: &str) -> std::io::Result<()> {
        match request_path(head).unwrap_or_default() {
            "/slow" => self.slow_page(stream),
            "/headers" => send(stream, "200 OK", HTML, "", &headers_page(head)),
            "/private" => private_page(stream, head),
            "/favicon.ico" => send(stream, "204 No Content", "image/x-icon", "", ""),
            path => {
                let Some((_, content_type, extra, body)) =
                    ROUTES.iter().find(|route| route.0 == path)
                else {
                    return send(stream, "404 Not Found", "text/plain", "", "not found");
                };
                let body = body.replace("{port}", &self.port.to_string());
                send(stream, "200 OK", content_type, extra, &body)
            }
        }
    }

    fn slow_page(&self, stream: &mut TcpStream) -> std::io::Result<()> {
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: {HTML}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{SLOW_HEAD}"
        )?;
        stream.write_all(&[b' '; 2048])?;
        stream.flush()?;
        self.slow_tail.wait_for_release();
        stream.write_all(SLOW_TAIL.as_bytes())?;
        stream.flush()?;
        self.slow_tail.sent.store(true, Ordering::Release);
        Ok(())
    }
}

/// A connection reset while it waited in the backlog makes `accept` fail with
/// `ECONNABORTED` on macOS; the listener itself is still fine.
fn is_transient_accept_error(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        ErrorKind::WouldBlock
            | ErrorKind::Interrupted
            | ErrorKind::ConnectionAborted
            | ErrorKind::ConnectionReset
    )
}

fn request_head(stream: &mut TcpStream) -> Option<String> {
    let mut head = Vec::new();
    let mut chunk = [0_u8; 2048];
    while head_end(&head).is_none() && head.len() < 65_536 {
        let read = stream.read(&mut chunk).ok().filter(|read| *read > 0)?;
        head.extend_from_slice(&chunk[..read]);
    }
    Some(String::from_utf8_lossy(&head).into_owned())
}

fn request_path(head: &str) -> Option<&str> {
    let target = head.lines().next()?.split_whitespace().nth(1)?;
    target.split('?').next()
}

/// Echoes the request head, so a test can read the headers the browser really sent.
fn headers_page(head: &str) -> String {
    let escaped = head
        .trim_end()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    format!(
        "<!doctype html><html><head><title>Headers</title></head><body><pre id='head'>{escaped}</pre></body></html>"
    )
}

fn private_page(stream: &mut TcpStream, head: &str) -> std::io::Result<()> {
    use flow_like_types::base64::Engine;
    let credentials = flow_like_types::base64::engine::general_purpose::STANDARD
        .encode(format!("{AUTH_USER}:{AUTH_PASSWORD}"));
    if header(head, "authorization") == Some(format!("Basic {credentials}")) {
        let body = PRIVATE_PAGE.replace("{user}", AUTH_USER);
        return send(stream, "200 OK", HTML, "", &body);
    }
    send(stream, "401 Unauthorized", HTML, CHALLENGE, "denied")
}

fn send(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    extra: &str,
    body: &str,
) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\n{extra}Content-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    stream.flush()
}

trait IntoLogic {
    fn into_logic(self) -> Arc<dyn NodeLogic>;
}

impl<T: NodeLogic + 'static> IntoLogic for T {
    fn into_logic(self) -> Arc<dyn NodeLogic> {
        Arc::new(self)
    }
}
