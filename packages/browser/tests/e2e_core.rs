#[cfg(windows)]
#[path = "support/cft_fallback.rs"]
mod cft_fallback;
#[path = "support/e2e.rs"]
mod e2e;
#[path = "support/header_proxy.rs"]
mod header_proxy;
#[path = "support/tls.rs"]
mod tls;

use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use e2e::{E2e, Snapshot, TestServer};
use flow_like_browser::attach::{self, AttachEndpoint};
use flow_like_browser::connection::{Connection, ConnectionOptions};
use flow_like_browser::downloads::DownloadRecord;
use flow_like_browser::event_log::{Event, EventCursor};
use flow_like_browser::input::keys::{Key, Modifiers, NamedKey};
use flow_like_browser::input::{ActionOutcome, ClickOptions};
use flow_like_browser::launch::BrowserKind;
use flow_like_browser::output::{Clip, ScreenshotOptions};
use flow_like_browser::refs::RefTable;
use flow_like_browser::test_hooks;
use flow_like_browser::transport::ws::{ConnectOptions, WsTransport};
use flow_like_browser::types::{DialogType, DownloadState, SessionId, TargetId};
use flow_like_browser::{
    Browser, BrowserError, ConnectionKind, DialogAction, Frame, NavigationOutcome, Page,
};
use serde_json::{Value, json};
use tokio::time::Instant;

macro_rules! chromium_test {
    ($name:ident, |$ctx:ident| { $($body:tt)* }) => {
        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        #[ignore = "requires a Chromium browser; run with --include-ignored"]
        async fn $name() {
            e2e::run(stringify!($name), |$ctx: E2e| async move { $($body)* }).await;
        }
    };
}

const PROXY_TOKEN: &str = "Bearer flow-like-e2e";
const SUBPROTOCOL: &str = "cdp.flow-like-e2e.v1";
const MOBILE_USER_AGENT: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148 flow-like-e2e";
const PROBE_SCRIPT: &str = "return {tz: Intl.DateTimeFormat().resolvedOptions().timeZone, ua: navigator.userAgent, width: innerWidth}";
const TWENTY_MIB: usize = 20 * 1024 * 1024;
const TWENTY_MIB_EXPRESSION: &str = "'x'.repeat(20 * 1024 * 1024)";
const LONE_SURROGATE_EXPRESSION: &str = "'a' + String.fromCharCode(0xD800) + 'b'";
const G1_CLICKS: [(&str, &str, &str); 6] = [
    ("anchor", "NEXT", "/next"),
    ("form-get", "NEXT", "/next?q=x"),
    ("form-post", "NEXT", "/next"),
    ("anchor-slowbody", "NEXT", "/slowbody?ms=1500"),
    ("anchor-204", "START", "/case/anchor-204"),
    ("anchor-hash", "START", "/case/anchor-hash#sec"),
];

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires a Chromium browser; run with --include-ignored"]
async fn provision_browser() {
    let executable = e2e::executable().await;
    assert!(
        executable.path.is_file(),
        "{} is not a file",
        executable.path.display()
    );
    let record = e2e::record_provisioned(&executable);
    println!(
        "flow-like-browser e2e executable: {} ({:?}, {:?}, version {}); path written to {}",
        executable.path.display(),
        executable.flavor,
        executable.source,
        executable.version.as_deref().unwrap_or("unknown"),
        record.display()
    );
}

chromium_test!(
    launch_without_chromedriver_reports_version_and_debugger_address,
    |ctx| {
        let mut options = ctx.options();
        if !e2e::executable_overridden() {
            options.executable = None;
        }
        let browser = ctx.launch_with(options).await;
        assert_eq!(browser.kind(), ConnectionKind::Launched);
        assert!(browser.is_owned() && browser.is_alive());
        let port = e2e::debug_port(&browser);
        assert_eq!(
            browser.debugger_address(),
            Some(format!("localhost:{port}").as_str())
        );
        let version = browser
            .root()
            .send("Browser.getVersion", json!({}))
            .await
            .expect("Browser.getVersion on the root session");
        assert_eq!(
            version["product"].as_str(),
            Some(browser.version().product.as_str())
        );
        assert!(
            browser.version().product.contains("Chrome/"),
            "{}",
            browser.version().product
        );
        let endpoint =
            attach::resolve_endpoint_with(&format!("localhost:{port}"), BrowserKind::Chrome, None)
                .await
                .expect("the launched browser serves /json/version");
        let AttachEndpoint::PortMode { ws_url } = &endpoint else {
            panic!("expected a port-mode endpoint, got {endpoint:?}");
        };
        assert!(
            ws_url.starts_with(&format!("ws://localhost:{port}/devtools/browser/")),
            "{ws_url}"
        );
        browser.close().await.expect("close");
        assert!(!browser.is_alive());
    }
);

chromium_test!(goto_snapshot_click_by_ref_type_and_read_the_value, |ctx| {
    let server = TestServer::start().await;
    let browser = ctx.launch().await;
    form_round_trip(&browser, &server).await;
    browser.close().await.expect("close");
});

#[cfg(windows)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires a Chromium browser; run with --include-ignored"]
async fn cft_install_without_setup_exe_grants_the_sandbox_with_icacls() {
    let workspace = cft_fallback::Workspace::create();
    let inherited = cft_fallback::app_container_rules(workspace.path()).await;
    assert!(
        inherited.is_empty(),
        "{} already grants the sandbox access ({inherited:?}), so the install cannot show the icacls grant works",
        workspace.path().display()
    );
    let source = e2e::executable().await;
    let executable = tokio::time::timeout(
        cft_fallback::INSTALL_BUDGET,
        workspace.install_without_setup(&source),
    )
    .await
    .expect("installing Chrome for Testing without setup.exe finished in time");
    cft_fallback::assert_icacls_grant(&executable).await;
    e2e::run(
        "cft_install_without_setup_exe_grants_the_sandbox_with_icacls",
        |ctx: E2e| async move {
            let server = TestServer::start().await;
            let mut options = ctx.options();
            options.executable = Some(executable);
            let browser = ctx.launch_with(options).await;
            form_round_trip(&browser, &server).await;
            browser.close().await.expect("close");
        },
    )
    .await;
    workspace.remove().await;
}

async fn form_round_trip(browser: &Browser, server: &TestServer) {
    let page = e2e::first_page(browser).await;
    let outcome = page.goto(&server.url("/form")).await.expect("goto /form");
    assert!(
        matches!(&outcome, NavigationOutcome::Loaded { url } if url.ends_with("/form")),
        "{outcome:?}"
    );
    assert_eq!(page.title().await.expect("title"), "Form");
    let snapshot = Snapshot::take(&page, &RefTable::default()).await;
    let name = snapshot
        .resolve(&page, &snapshot.reference("textbox", "Name"))
        .await;
    assert_eq!(
        name.send_keys("Ada").await.expect("type into Name"),
        ActionOutcome::Completed
    );
    let greet = snapshot
        .resolve(&page, &snapshot.reference("button", "Greet"))
        .await;
    assert_eq!(
        greet
            .click(ClickOptions::default())
            .await
            .expect("click Greet"),
        ActionOutcome::Completed
    );
    assert_eq!(
        name.property("value").await.expect("read the value"),
        json!("Ada")
    );
    assert_eq!(e2e::text_of(&page.main_frame(), "#out").await, "Hello Ada");
}

chromium_test!(
    same_site_cross_site_and_nested_iframe_refs_resolve_and_click,
    |ctx| {
        let server = TestServer::start().await;
        let browser = ctx.launch().await;
        let page = e2e::first_page(&browser).await;
        page.goto(&server.localhost_url("/oopif"))
            .await
            .expect("goto the OOPIF page");
        let buttons = server.oopif_buttons();
        let snapshot = Snapshot::until(&page, &RefTable::default(), |snapshot| {
            buttons.iter().all(|name| snapshot.has("button", name))
        })
        .await;
        ctx.observe(format!(
            "three-site nest via [::1]: {}",
            server.has_third_site()
        ));
        for name in &buttons {
            click_both_ways(&page, &snapshot, name).await;
        }
        assert_local_roots(&page, &snapshot, server.has_third_site());
        let input = snapshot
            .resolve(&page, &snapshot.reference("textbox", "Frame input cross"))
            .await;
        assert_eq!(
            input
                .send_keys("typed across sites")
                .await
                .expect("type into the OOPIF"),
            ActionOutcome::Completed
        );
        assert_eq!(
            input.property("value").await.expect("read the OOPIF value"),
            json!("typed across sites")
        );
        let swapped = swap_the_cross_site_frame(&page, &server, &snapshot).await;
        refs_go_stale_after_a_navigation(&page, &swapped, &["Top button", "Frame button remote"])
            .await;
        browser.close().await.expect("close");
    }
);

async fn set_frame_src(page: &Page, id: &str, url: &str) {
    let script = format!("document.getElementById({id:?}).src = {url:?}; return true");
    e2e::script_value(&page.main_frame(), &script).await;
}

/// Remote to local (the OOPIF session detaches before the parent commits) and back to remote:
/// refs of the swapped-out document go stale, and the frame's new buttons click both ways.
async fn swap_the_cross_site_frame(
    page: &Page,
    server: &TestServer,
    before: &Snapshot,
) -> Snapshot {
    let top = page.target_id().clone();
    let old = before.reference("button", "Frame button cross");
    set_frame_src(page, "cross", &server.localhost_url("/oopif/frame?n=local")).await;
    let local = Snapshot::until(page, &before.table, |snapshot| {
        snapshot.has("button", "Frame button local")
    })
    .await;
    let stale = before
        .element(page, &old)
        .await
        .err()
        .expect("a ref into the swapped-out OOPIF still resolved");
    assert_eq!(stale.to_string(), e2e::stale_ref_text(&old));
    let reference = local.reference("button", "Frame button local");
    assert_eq!(local.local_root(&reference), top, "the frame is local now");
    click_both_ways(page, &local, "Frame button local").await;
    set_frame_src(page, "cross", &server.url("/oopif/frame?n=remote")).await;
    let remote = Snapshot::until(page, &local.table, |snapshot| {
        snapshot.has("button", "Frame button remote")
    })
    .await;
    let reference = remote.reference("button", "Frame button remote");
    assert_ne!(
        remote.local_root(&reference),
        top,
        "the frame is an OOPIF again"
    );
    click_both_ways(page, &remote, "Frame button remote").await;
    remote
}

async fn click_both_ways(page: &Page, snapshot: &Snapshot, name: &str) {
    let button = snapshot
        .resolve(page, &snapshot.reference("button", name))
        .await;
    let clicked = button
        .click(ClickOptions::default())
        .await
        .unwrap_or_else(|error| panic!("W3C click on {name}: {error}"));
    assert_eq!(clicked, ActionOutcome::Completed, "{name}");
    assert_eq!(
        e2e::text_of(button.frame(), "#out").await,
        "1",
        "{name} after the W3C click"
    );
    let clicked = button
        .element_click()
        .await
        .unwrap_or_else(|error| panic!("element click on {name}: {error}"));
    assert_eq!(clicked, ActionOutcome::Completed, "{name}");
    assert_eq!(
        e2e::text_of(button.frame(), "#out").await,
        "2",
        "{name} after the element click"
    );
}

fn assert_local_roots(page: &Page, snapshot: &Snapshot, third_site: bool) {
    let root = |name: &str| snapshot.local_root(&snapshot.reference("button", name));
    let top = root("Top button");
    let cross = root("Frame button cross");
    let leaf = root("Frame button leaf");
    assert_eq!(&top, page.target_id());
    assert_eq!(
        root("Frame button same"),
        top,
        "a same-site iframe shares the page"
    );
    assert_ne!(cross, top, "the cross-site iframe is an OOPIF");
    assert!(
        leaf != top && leaf != cross,
        "the A-B-A leaf is its own local root"
    );
    if third_site {
        let third = root("Frame button third");
        assert!(
            third != top && third != cross && third != leaf,
            "the [::1] leaf"
        );
    }
}

async fn refs_go_stale_after_a_navigation(page: &Page, snapshot: &Snapshot, names: &[&str]) {
    let leave = snapshot
        .resolve(page, &snapshot.reference("link", "Leave"))
        .await;
    assert_eq!(
        leave
            .click(ClickOptions::default())
            .await
            .expect("click Leave"),
        ActionOutcome::Completed
    );
    for name in names {
        let reference = snapshot.reference("button", name);
        let error = snapshot
            .element(page, &reference)
            .await
            .err()
            .unwrap_or_else(|| panic!("{reference} ({name}) still resolved after the navigation"));
        assert_eq!(error.to_string(), e2e::stale_ref_text(&reference));
    }
    assert_eq!(e2e::text_of(&page.main_frame(), "#t").await, "START");
}

chromium_test!(dialogs_open_from_clicks_block_ops_and_are_handled, |ctx| {
    let server = TestServer::start().await;
    let browser = ctx.launch().await;
    let page = e2e::first_page(&browser).await;
    dialog_round_trip(&page, &server).await;
    browser.close().await.expect("close");
});

async fn dialog_round_trip(page: &Page, server: &TestServer) {
    page.goto(&server.url("/dialogs"))
        .await
        .expect("goto /dialogs");
    let main = page.main_frame();
    open_dialog(&main, "#alert", DialogType::Alert, "hi").await;
    let blocked = page
        .title()
        .await
        .expect_err("an op ran while the alert was open");
    assert!(
        matches!(blocked, BrowserError::DialogOpen { .. }),
        "{blocked:?}"
    );
    assert_eq!(
        blocked.to_string(),
        "unexpected alert open: {Alert text : hi}"
    );
    assert_eq!(
        page.pending_dialog().map(|dialog| dialog.message),
        Some("hi".to_owned())
    );
    let accepted = page
        .handle_dialog(DialogAction::Accept { prompt_text: None })
        .await
        .expect("accept the alert");
    assert_eq!(accepted.kind, DialogType::Alert);
    assert_eq!(e2e::text_of(&main, "#out").await, "alerted");
    open_dialog(&main, "#confirm", DialogType::Confirm, "sure?").await;
    page.handle_dialog(DialogAction::Dismiss)
        .await
        .expect("dismiss the confirm");
    assert_eq!(e2e::text_of(&main, "#out").await, "false");
    open_dialog(&main, "#prompt", DialogType::Prompt, "name?").await;
    let prompt_text = Some("flow".to_owned());
    page.handle_dialog(DialogAction::Accept { prompt_text })
        .await
        .expect("accept the prompt with text");
    assert_eq!(e2e::text_of(&main, "#out").await, "flow");
    let none = page
        .handle_dialog(DialogAction::Dismiss)
        .await
        .expect_err("handle_dialog without an open dialog succeeded");
    assert!(matches!(none, BrowserError::NoDialog), "{none:?}");
}

async fn open_dialog(frame: &Frame, css: &str, kind: DialogType, message: &str) {
    let button = e2e::first_element(frame, css).await;
    let started = Instant::now();
    let outcome = button
        .click(ClickOptions::default())
        .await
        .unwrap_or_else(|error| panic!("clicking {css}: {error}"));
    let elapsed = started.elapsed();
    let expected = ActionOutcome::DialogOpened {
        kind,
        message: message.to_owned(),
    };
    assert_eq!(outcome, expected, "{css}");
    assert!(
        elapsed < Duration::from_secs(1),
        "{css} reported its dialog after {elapsed:?}"
    );
}

chromium_test!(
    downloads_land_in_the_chosen_directory_with_the_suggested_name,
    |ctx| {
        let server = TestServer::start().await;
        let browser = ctx.launch().await;
        let page = e2e::first_page(&browser).await;
        let directory = ctx.scratch_dir("downloads");
        download_round_trip(&browser, &page, &server, &directory).await;
        assert_eq!(e2e::text_of(&page.main_frame(), "#t").await, "START");
        download_round_trip(&browser, &page, &server, &directory).await;
        let names: Vec<String> = std::fs::read_dir(&directory)
            .expect("list the download directory")
            .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
            .collect();
        assert_eq!(
            names,
            vec!["d.bin".to_owned()],
            "a second download replaces the first"
        );
        browser.close().await.expect("close");
    }
);

async fn download_round_trip(
    browser: &Browser,
    page: &Page,
    server: &TestServer,
    directory: &Path,
) -> DownloadRecord {
    browser
        .set_download_directory(directory)
        .await
        .expect("set the download directory");
    page.goto(&server.url("/case/download"))
        .await
        .expect("goto the download page");
    let since = browser.downloads().cursor();
    click(&page.main_frame(), "#go").await;
    let deadline = Instant::now() + Duration::from_secs(20);
    let record = browser
        .downloads()
        .wait_finished(since, false, deadline, |record| {
            record.suggested_filename == "d.bin"
        })
        .await
        .expect("wait for the download")
        .expect("the download finished within 20 s");
    assert_eq!(record.state, DownloadState::Completed, "{record:?}");
    let landed = record
        .final_path
        .clone()
        .expect("a completed download has a path");
    assert_eq!(
        std::fs::canonicalize(&landed).expect("the download exists"),
        std::fs::canonicalize(directory.join("d.bin")).expect("the expected file exists"),
        "{record:?}"
    );
    assert_eq!(
        std::fs::read_to_string(&landed).expect("read the download"),
        "hello download\n"
    );
    record
}

async fn click(frame: &Frame, css: &str) -> ActionOutcome {
    e2e::first_element(frame, css)
        .await
        .click(ClickOptions::default())
        .await
        .unwrap_or_else(|error| panic!("clicking {css}: {error}"))
}

chromium_test!(full_page_screenshot_and_streamed_pdf, |ctx| {
    let server = TestServer::start().await;
    let browser = ctx.launch().await;
    let page = e2e::first_page(&browser).await;
    page.goto(&server.url("/tall")).await.expect("goto /tall");
    let metrics = page
        .cdp("Page.getLayoutMetrics", json!({}))
        .await
        .expect("layout metrics");
    let width = metrics["cssContentSize"]["width"]
        .as_f64()
        .expect("content width");
    let height = metrics["cssContentSize"]["height"]
        .as_f64()
        .expect("content height");
    let viewport = metrics["cssLayoutViewport"]["clientHeight"]
        .as_f64()
        .expect("viewport height");
    assert!(
        height > viewport,
        "the page ({height}) is not taller than the viewport ({viewport})"
    );
    let options = ScreenshotOptions {
        clip: Some(Clip {
            x: 0.0,
            y: 0.0,
            width,
            height,
            scale: 1.0,
        }),
        capture_beyond_viewport: true,
    };
    let full = page
        .screenshot(options)
        .await
        .expect("full-page screenshot");
    let (_, full_height) = e2e::png_size(&full).expect("the full-page screenshot is a PNG");
    assert!(
        f64::from(full_height) >= height.floor(),
        "{full_height} px for {height} CSS px"
    );
    let visible = page
        .screenshot(ScreenshotOptions::default())
        .await
        .expect("viewport screenshot");
    let (_, visible_height) = e2e::png_size(&visible).expect("the viewport screenshot is a PNG");
    assert!(
        full_height > visible_height,
        "{full_height} <= {visible_height}"
    );
    let pdf = page
        .print_pdf(json!({"printBackground": true}))
        .await
        .expect("print to PDF through IO.read");
    assert!(pdf.starts_with(b"%PDF"), "{:?}", &pdf[..pdf.len().min(8)]);
    browser.close().await.expect("close");
});

chromium_test!(
    custom_headers_and_subprotocol_reach_chrome_through_ws_and_wss_proxies,
    |ctx| {
        let server = TestServer::start().await;
        let launched = ctx.launch().await;
        let upstream = e2e::browser_ws_url(&launched).await;
        let plain = header_proxy::start(upstream.clone(), Some(PROXY_TOKEN.to_owned()), None).await;
        let refused = Browser::connect(ConnectOptions::new(plain.url.clone()))
            .await
            .err()
            .expect("the proxy let a connection without the header through");
        assert!(refused.to_string().contains("HTTP 401"), "{refused}");
        let browser = Browser::connect(proxied(&plain.url, None))
            .await
            .expect("connect with the header and the subprotocol");
        ctx.watch(&browser);
        let offered = plain
            .subprotocol_seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        assert_eq!(offered.as_deref(), Some(SUBPROTOCOL));
        drive_proxied_browser(&browser, &server).await;
        plain.binary_frames.store(true, Ordering::SeqCst);
        browser
            .root()
            .send("Browser.getVersion", json!({}))
            .await
            .expect("CDP replies re-framed as Binary");
        browser.close().await.expect("disconnect from the ws proxy");
        secure_proxy_round_trip(&ctx, &server, &upstream).await;
        launched.close().await.expect("close");
    }
);

fn proxied(url: &str, ca: Option<&tls::TestCa>) -> ConnectOptions {
    let mut options = ConnectOptions::new(url);
    options.headers = vec![("Authorization".to_owned(), PROXY_TOKEN.to_owned())];
    options.subprotocols = vec![SUBPROTOCOL.to_owned()];
    options.extra_root_certificates = ca.map(|ca| vec![ca.ca_der.clone()]).unwrap_or_default();
    options
}

async fn secure_proxy_round_trip(ctx: &E2e, server: &TestServer, upstream: &str) {
    let ca = tls::test_ca();
    let config = tls::server_config(&ca);
    let secure = header_proxy::start(
        upstream.to_owned(),
        Some(PROXY_TOKEN.to_owned()),
        Some(config),
    )
    .await;
    assert!(secure.url.starts_with("wss://"), "{}", secure.url);
    let untrusted = Browser::connect(proxied(&secure.url, None))
        .await
        .err()
        .expect("a certificate from an unknown CA was trusted");
    assert!(untrusted.to_string().contains("not trusted"), "{untrusted}");
    let browser = Browser::connect(proxied(&secure.url, Some(&ca)))
        .await
        .expect("connect over wss with the test CA");
    ctx.watch(&browser);
    drive_proxied_browser(&browser, server).await;
    browser
        .close()
        .await
        .expect("disconnect from the wss proxy");
}

async fn drive_proxied_browser(browser: &Browser, server: &TestServer) {
    assert_eq!(browser.kind(), ConnectionKind::Direct);
    assert!(browser.version().product.contains("Chrome/"));
    let page = browser
        .new_page()
        .await
        .expect("open a tab through the proxy");
    page.goto(&server.url("/form"))
        .await
        .expect("navigate through the proxy");
    assert_eq!(page.title().await.expect("title through the proxy"), "Form");
    browser
        .close_page(page.target_id())
        .await
        .expect("close the proxied tab");
}

chromium_test!(
    close_and_abort_on_a_run_error_leave_no_process_or_profile,
    |ctx| {
        let server = TestServer::start().await;
        let browser = ctx.launch_untracked().await;
        let profile = e2e::profile_of(&browser);
        let staging = test_hooks::staging_dir(&browser).expect("a staging dir");
        assert!(
            !e2e::processes_using(&profile).is_empty(),
            "the process scan does not see the launched browser"
        );
        browser.close().await.expect("close");
        e2e::assert_gone(&profile).await;
        e2e::assert_removed(&staging, "close removes the staging dir").await;
        let browser = ctx.launch_untracked().await;
        let profile = e2e::profile_of(&browser);
        let staging = test_hooks::staging_dir(&browser).expect("a staging dir");
        let failure = simulated_run(&browser, &server)
            .await
            .expect_err("the simulated run fails");
        ctx.note(format!("simulated run error: {failure}"));
        browser.abort();
        drop(browser);
        e2e::assert_gone(&profile).await;
        e2e::assert_removed(&staging, "abort removes the staging dir").await;
    }
);

fn by_value(expression: &str) -> Value {
    json!({"expression": expression, "returnByValue": true})
}

async fn simulated_run(browser: &Browser, server: &TestServer) -> Result<(), String> {
    let page = e2e::first_page(browser).await;
    page.goto(&server.url("/form"))
        .await
        .map_err(|error| error.to_string())?;
    let missing = page
        .main_frame()
        .find_css("#missing", None)
        .await
        .map_err(|error| error.to_string())?;
    if missing.is_empty() {
        return Err("the flow expected #missing on /form".to_owned());
    }
    Ok(())
}

chromium_test!(
    twenty_mib_replies_and_lone_surrogates_keep_the_connection,
    |ctx| {
        let server = TestServer::start().await;
        let browser = ctx.launch().await;
        let page = e2e::first_page(&browser).await;
        page.goto(&server.url("/form")).await.expect("goto /form");
        let big = page
            .cdp("Runtime.evaluate", by_value(TWENTY_MIB_EXPRESSION))
            .await
            .expect("a 20 MiB reply");
        assert_eq!(
            big["result"]["value"].as_str().map(str::len),
            Some(TWENTY_MIB)
        );
        let odd = page
            .cdp("Runtime.evaluate", by_value(LONE_SURROGATE_EXPRESSION))
            .await
            .expect("a reply carrying a lone surrogate");
        assert_eq!(odd["result"]["value"], json!("a\u{FFFD}b"));
        assert_eq!(
            page.title().await.expect("title after both replies"),
            "Form"
        );
        assert!(browser.is_alive());
        browser.close().await.expect("close");
    }
);

chromium_test!(renderer_crash_fails_pending_and_new_commands_fast, |ctx| {
    let server = TestServer::start().await;
    let browser = ctx.launch().await;
    let page = e2e::first_page(&browser).await;
    crash_round_trip(&ctx, &page, &server).await;
    browser.close().await.expect("close");
});

async fn crash_round_trip(ctx: &E2e, page: &Page, server: &TestServer) {
    page.goto(&server.url("/form")).await.expect("goto /form");
    let session = page.session();
    let never = json!({"expression": "new Promise(() => {})", "awaitPromise": true});
    let hanging = session
        .enqueue("Runtime.evaluate", never, Some(Duration::from_secs(30)))
        .expect("enqueue a command that never resolves");
    let crashed_at = Instant::now();
    session
        .send_nowait("Page.crash", json!({}))
        .expect("send Page.crash");
    let pending = tokio::time::timeout(Duration::from_secs(10), hanging)
        .await
        .expect("the pending command outlived the renderer crash by 10 s");
    let failure = pending.expect_err("the pending command succeeded on a crashed renderer");
    ctx.observe(format!(
        "pending command after Page.crash: {failure:?} after {:?}",
        crashed_at.elapsed()
    ));
    assert!(
        matches!(
            failure,
            BrowserError::TargetCrashed { .. } | BrowserError::Protocol { .. }
        ),
        "{failure:?}"
    );
    new_commands_fail_fast(page).await;
    page.reload().await.expect("reload the crashed page");
    assert_eq!(page.title().await.expect("title after the reload"), "Form");
}

async fn new_commands_fail_fast(page: &Page) {
    let refused_at = Instant::now();
    let refused = page
        .title()
        .await
        .expect_err("title() ran on a crashed page");
    assert!(
        matches!(refused, BrowserError::TargetCrashed { .. }),
        "{refused:?}"
    );
    let raw = page
        .session()
        .send("Runtime.evaluate", json!({"expression": "1"}))
        .await
        .expect_err("a raw command ran on a crashed renderer");
    assert!(matches!(raw, BrowserError::TargetCrashed { .. }), "{raw:?}");
    let refusal = refused_at.elapsed();
    assert!(
        refusal < Duration::from_secs(1),
        "failing fast took {refusal:?}"
    );
}

chromium_test!(panic_between_spawn_and_hand_off_leaves_no_chrome, |ctx| {
    let executable = ctx.executable().clone();
    let options = ctx.options();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let task: tokio::task::JoinHandle<()> = tokio::spawn(async move {
        let launched = test_hooks::spawn(&executable, &options)
            .await
            .expect("spawn Chrome");
        let process = &launched.process;
        let staging = process.staging_dir().map(Path::to_path_buf);
        let _ = sender.send((process.profile().dir.clone(), staging));
        panic!(
            "simulated panic after spawning Chrome (pid {})",
            process.pid()
        );
    });
    let joined = task.await;
    assert!(
        joined.as_ref().is_err_and(|error| error.is_panic()),
        "the launch task did not panic"
    );
    let (profile, staging) = receiver
        .await
        .expect("the task reported the profile before panicking");
    e2e::assert_gone(&profile).await;
    assert!(
        staging.is_some_and(|staging| !staging.exists()),
        "the download staging dir survived the panic"
    );
});

chromium_test!(
    tabs_keep_first_seen_order_and_untitled_tabs_report_empty_titles,
    |ctx| {
        let server = TestServer::start().await;
        let browser = ctx.launch().await;
        let first = e2e::first_page(&browser).await;
        let second = browser.new_page().await.expect("second tab");
        let third = browser.new_page().await.expect("third tab");
        second
            .goto(&server.url("/untitled"))
            .await
            .expect("goto /untitled");
        first
            .goto(&server.url("/popup-untitled"))
            .await
            .expect("goto /popup-untitled");
        click(&first.main_frame(), "#open").await;
        let popup = e2e::wait_for_popup(&browser, first.target_id(), "/untitled").await;
        let titled = open_background_tab(&browser, &server.url("/titled")).await;
        let order: Vec<TargetId> = browser
            .pages()
            .into_iter()
            .map(|info| info.target_id)
            .collect();
        let expected = vec![
            first.target_id().clone(),
            second.target_id().clone(),
            third.target_id().clone(),
            popup.target_id.clone(),
            titled.clone(),
        ];
        assert_eq!(order, expected);
        let placeholder = format!("127.0.0.1:{}/untitled", server.port());
        wait_for_target_title(&browser, &popup.target_id, &placeholder).await;
        wait_for_target_title(&browser, &titled, "Titled").await;
        let listed = browser.list_pages(Duration::from_secs(2)).await;
        for (target, title) in [
            (second.target_id(), ""),
            (third.target_id(), ""),
            (&popup.target_id, ""),
            (&titled, "Titled"),
        ] {
            assert_eq!(
                e2e::title_of(&listed, target).as_deref(),
                Some(title),
                "{target}: {listed:?}"
            );
        }
        browser.close().await.expect("close");
    }
);

async fn target_title(browser: &Browser, target: &TargetId) -> String {
    let info = browser
        .root()
        .send("Target.getTargetInfo", json!({"targetId": target}))
        .await
        .expect("Target.getTargetInfo");
    info["targetInfo"]["title"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

/// Waits until Chrome's own target title is `title`, so the document is known to have loaded.
async fn wait_for_target_title(browser: &Browser, target: &TargetId, title: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let current = target_title(browser, target).await;
        if current == title {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{target} kept the target title {current:?}, expected {title:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires a Chromium browser; run with --include-ignored"]
async fn persistent_profile_cookie_survives_abort_and_relaunch() {
    e2e::run(
        "persistent_profile_cookie_survives_abort_and_relaunch",
        cookie_survives_abort,
    )
    .await;
}

#[cfg(unix)]
async fn cookie_survives_abort(ctx: E2e) {
    let server = TestServer::start().await;
    let profile = ctx.scratch_dir("persistent-profile");
    let mut options = ctx.options();
    options.user_data_dir = Some(profile.clone());
    let browser = ctx.launch_untracked_with(options.clone()).await;
    let staging = test_hooks::staging_dir(&browser).expect("a launched browser stages downloads");
    let page = e2e::first_page(&browser).await;
    page.goto(&server.url("/setcookie"))
        .await
        .expect("goto /setcookie");
    assert!(cookies(&page).await.contains("e2e_cookie=kept"));
    browser.abort();
    drop((page, browser));
    e2e::assert_no_process(&profile).await;
    assert!(profile.is_dir(), "abort removed a persistent profile");
    e2e::assert_removed(
        &staging,
        "the watchdog removes the staging dir after its grace",
    )
    .await;
    let browser = ctx.launch_with(options).await;
    let page = e2e::first_page(&browser).await;
    page.goto(&server.url("/titled"))
        .await
        .expect("goto /titled");
    let kept = cookies(&page).await;
    assert!(
        kept.contains("e2e_cookie=kept"),
        "cookies after the relaunch: {kept:?}"
    );
    browser.close().await.expect("close");
}

#[cfg(unix)]
async fn cookies(page: &Page) -> String {
    e2e::script_value(&page.main_frame(), "return document.cookie")
        .await
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

chromium_test!(g1_clicks_settle_before_the_next_read, |ctx| {
    let server = TestServer::start().await;
    let browser = ctx.launch().await;
    let page = e2e::first_page(&browser).await;
    for (case, heading, url_suffix) in G1_CLICKS {
        page.goto(&server.url(&format!("/case/{case}")))
            .await
            .unwrap_or_else(|error| panic!("goto {case}: {error}"));
        assert_eq!(
            click(&page.main_frame(), "#go").await,
            ActionOutcome::Completed,
            "{case}"
        );
        assert_eq!(
            e2e::text_of(&page.main_frame(), "#t").await,
            heading,
            "{case}"
        );
        let url = page
            .url()
            .await
            .unwrap_or_else(|error| panic!("url after {case}: {error}"));
        assert!(url.ends_with(url_suffix), "{case} ended on {url}");
    }
    let requests = server.requests();
    assert!(
        requests.iter().any(|request| request == "POST /next"),
        "{requests:?}"
    );
    browser.close().await.expect("close");
});

chromium_test!(g1_enter_download_link_and_iframe_navigation_settle, |ctx| {
    let server = TestServer::start().await;
    let browser = ctx.launch().await;
    let page = e2e::first_page(&browser).await;
    page.goto(&server.url("/case/form-get"))
        .await
        .expect("goto form-get");
    let field = e2e::first_element(&page.main_frame(), "input[name=q]").await;
    assert_eq!(
        field.send_keys("\n").await.expect("Enter in the form"),
        ActionOutcome::Completed
    );
    assert_eq!(e2e::text_of(&page.main_frame(), "#t").await, "NEXT");
    assert!(page.url().await.expect("url").ends_with("/next?q=x"));
    page.goto(&server.url("/case/form-get"))
        .await
        .expect("goto form-get again");
    let field = e2e::first_element(&page.main_frame(), "input[name=q]").await;
    field.focus().await.expect("focus the field");
    let chord = page
        .key_chord(Key::Named(NamedKey::Enter), Modifiers::NONE)
        .await
        .expect("Enter chord");
    assert_eq!(chord, ActionOutcome::Completed);
    assert_eq!(e2e::text_of(&page.main_frame(), "#t").await, "NEXT");
    let directory = ctx.scratch_dir("g1-downloads");
    download_round_trip(&browser, &page, &server, &directory).await;
    assert_eq!(e2e::text_of(&page.main_frame(), "#t").await, "START");
    page.goto(&server.url("/case/iframe-nav"))
        .await
        .expect("goto iframe-nav");
    click(&page.main_frame(), "#go").await;
    let frame = e2e::child_frame(&page.main_frame(), "iframe#f").await;
    assert_eq!(e2e::text_of(&frame, "#t").await, "NEXT");
    browser.close().await.expect("close");
});

chromium_test!(g1_goto_fails_only_on_connection_errors, |ctx| {
    let server = TestServer::start().await;
    let browser = ctx.launch().await;
    let page = e2e::first_page(&browser).await;
    let unresolvable = "http://flow-like-e2e.invalid/";
    let failure = page
        .goto(unresolvable)
        .await
        .expect_err("goto to an unresolvable host succeeded");
    ctx.observe(format!("unresolvable host: {failure}"));
    assert!(
        matches!(&failure, BrowserError::NavigationFailed { url, error }
            if url == unresolvable && error.starts_with("net::ERR_")),
        "{failure:?}"
    );
    page.cdp("Network.enable", json!({}))
        .await
        .expect("Network.enable");
    page.cdp("Network.setBlockedURLs", json!({"urls": ["*/blocked*"]}))
        .await
        .expect("block /blocked");
    let blocked = page
        .goto(&server.url("/blocked"))
        .await
        .expect("goto a blocked URL");
    assert!(
        matches!(blocked, NavigationOutcome::Loaded { .. }),
        "{blocked:?}"
    );
    assert_eq!(
        page.url().await.expect("url of the error page"),
        server.url("/blocked"),
        "an error page reports the URL it was opened for"
    );
    page.goto(&server.url("/case/noop"))
        .await
        .expect("goto noop");
    let no_content = page.goto(&server.url("/204")).await.expect("goto a 204");
    assert!(
        matches!(&no_content, NavigationOutcome::Loaded { url } if url.ends_with("/case/noop")),
        "{no_content:?}"
    );
    let script = page
        .goto("javascript:void(0)")
        .await
        .expect_err("goto accepted a javascript: URL");
    assert!(
        matches!(script, BrowserError::InvalidArgument { .. }),
        "{script:?}"
    );
    browser.close().await.expect("close");
});

chromium_test!(
    uploads_append_to_multiple_inputs_and_replace_single_ones,
    |ctx| {
        let server = TestServer::start().await;
        let browser = ctx.launch().await;
        let page = e2e::first_page(&browser).await;
        page.goto(&server.url("/upload"))
            .await
            .expect("goto /upload");
        let directory = ctx.scratch_dir("uploads");
        let files: Vec<String> = ["a.txt", "b.txt", "c.txt"]
            .into_iter()
            .map(|name| {
                let path = directory.join(name);
                std::fs::write(&path, name).expect("write an upload file");
                path.display().to_string()
            })
            .collect();
        let frame = page.main_frame();
        let names = |id: &str| {
            format!("return Array.from(document.getElementById('{id}').files, file => file.name)")
        };
        let many = e2e::first_element(&frame, "#many").await;
        many.send_keys(&files[..2].join("\n"))
            .await
            .expect("upload two files");
        many.send_keys(&files[2])
            .await
            .expect("upload a third file");
        assert_eq!(
            e2e::script_value(&frame, &names("many")).await,
            json!(["a.txt", "b.txt", "c.txt"])
        );
        let single = e2e::first_element(&frame, "#single").await;
        single.send_keys(&files[0]).await.expect("upload one file");
        single.send_keys(&files[1]).await.expect("replace it");
        assert_eq!(
            e2e::script_value(&frame, &names("single")).await,
            json!(["b.txt"])
        );
        browser.close().await.expect("close");
    }
);

chromium_test!(back_and_forward_with_bfcache_keep_refs_honest, |ctx| {
    let server = TestServer::start().await;
    let browser = ctx.launch().await;
    let page = e2e::first_page(&browser).await;
    page.goto(&server.url("/bf/a")).await.expect("goto A");
    let first_a = Snapshot::take(&page, &RefTable::default()).await;
    page.goto(&server.url("/bf/b")).await.expect("goto B");
    let on_b = Snapshot::take(&page, &first_a.table).await;
    let b_button = on_b.reference("button", "B button");
    let cursor = browser.connection().events().cursor();
    page.back().await.expect("back to A");
    assert_eq!(e2e::text_of(&page.main_frame(), "#t").await, "Page A");
    assert_restored_from_bfcache(&browser, &page, cursor).await;
    let stale = on_b
        .element(&page, &b_button)
        .await
        .err()
        .expect("a ref of B resolved on A");
    assert_eq!(stale.to_string(), e2e::stale_ref_text(&b_button));
    let restored = first_a
        .resolve(&page, &first_a.reference("button", "A button"))
        .await;
    assert_eq!(
        restored
            .click(ClickOptions::default())
            .await
            .expect("click A through a ref taken before leaving A"),
        ActionOutcome::Completed
    );
    assert_eq!(e2e::text_of(&page.main_frame(), "#out").await, "1");
    let on_a = Snapshot::take(&page, &on_b.table).await;
    let a_button = on_a
        .resolve(&page, &on_a.reference("button", "A button"))
        .await;
    assert_eq!(
        a_button
            .click(ClickOptions::default())
            .await
            .expect("click A"),
        ActionOutcome::Completed
    );
    assert_eq!(e2e::text_of(&page.main_frame(), "#out").await, "2");
    page.forward().await.expect("forward to B");
    assert_eq!(e2e::text_of(&page.main_frame(), "#t").await, "Page B");
    browser.close().await.expect("close");
});

/// A restore keeps the document (and so its loader and backend node ids); a blocker is reported
/// with Chrome's own explanation instead of letting an ordinary back navigation pass.
async fn assert_restored_from_bfcache(browser: &Browser, page: &Page, cursor: EventCursor) {
    let persisted = e2e::script_value(
        &page.main_frame(),
        "return document.body.dataset.persisted || 'unknown'",
    )
    .await;
    if persisted == "true" {
        return;
    }
    let not_used = browser
        .connection()
        .events()
        .wait_for(cursor, Instant::now(), |event| {
            event.method.as_ref() == "Page.backForwardCacheNotUsed"
        })
        .await
        .expect("read the event log");
    panic!(
        "back did not restore A from the BFCache (pageshow persisted: {persisted}); Page.backForwardCacheNotUsed: {}",
        not_used.map_or_else(
            || "not reported".to_owned(),
            |event| event.params.to_string()
        )
    );
}

chromium_test!(
    prerender_is_refused_for_held_pages_and_an_unheld_tab_hands_over,
    |ctx| {
        let server = TestServer::start().await;
        let launched = ctx.launch().await;
        let recording = Recording::start(&ctx, &launched, &server).await;
        held_page_is_never_prerendered(&ctx, &recording.browser, &server).await;
        let direct = e2e::browser_ws_url(&launched).await;
        unheld_tab_hands_over_to_its_prerender(&recording.browser, &direct, &server).await;
        let frames = recording.finish(&ctx, &launched, "prerender").await;
        ctx.observe(format!(
            "Target events seen by the browser under test: {}",
            e2e::target_events(&frames).join(" | ")
        ));
        launched.close().await.expect("close");
    }
);

/// Chrome refuses to prerender for a page that has a session attached to its page target
/// (content/browser/devtools/devtools_instrumentation.cc IsPrerenderAllowed:
/// HasSessionsWithoutTabTargetSupport), which is how this crate holds every page. So a held page
/// is never replaced by a prerender, and the op after the click runs on the same handle.
async fn held_page_is_never_prerendered(ctx: &E2e, browser: &Browser, server: &TestServer) {
    let page = browser.new_page().await.expect("open a tab");
    let preload = e2e::Captured::on(browser.connection(), "Preload.");
    page.cdp("Preload.enable", json!({}))
        .await
        .expect("Preload.enable");
    let events = browser.connection().events();
    let cursor = events.cursor();
    page.goto(&server.url("/pr/start"))
        .await
        .expect("goto /pr/start");
    let refused = preload
        .wait_for(Duration::from_secs(10), |event| {
            event.method.as_ref() == "Preload.prerenderStatusUpdated"
                && event.params["status"] == "Failure"
        })
        .await;
    let status = refused.map(|event| event.params["prerenderStatus"].clone());
    ctx.observe(format!("prerender status on a held page: {status:?}"));
    assert_eq!(
        status,
        Some(json!("PrerenderingDisabledByDevTools")),
        "Chrome no longer refuses to prerender for a held page: the prerender successor path of spec section 2.16 now applies to held pages"
    );
    let prerendered = events
        .wait_for(cursor, Instant::now(), is_prerender_target)
        .await
        .expect("read the event log");
    assert!(prerendered.is_none(), "{prerendered:?}");
    click(&page.main_frame(), "#go").await;
    let url = page.url().await.expect("url on the same page handle");
    assert!(url.ends_with("/pr/next"), "{url}");
    assert_eq!(e2e::text_of(&page.main_frame(), "#t").await, "PRERENDERED");
    assert!(
        browser
            .pages()
            .iter()
            .any(|info| info.target_id == *page.target_id())
    );
    browser
        .close_page(page.target_id())
        .await
        .expect("close the held tab");
}

/// Another client that holds a tab through a tab target (as Puppeteer does) lets Chrome
/// prerender. The browser under test holds no session on that tab, sees the replaced page turn
/// `disconnected` and the prerender turn into a tab, and binds the successor in place.
async fn unheld_tab_hands_over_to_its_prerender(
    browser: &Browser,
    direct: &str,
    server: &TestServer,
) {
    let cursor = browser.connection().events().cursor();
    let client = TabClient::open(direct).await;
    let replaced = client.page.clone();
    let position = wait_for_tab(browser, &replaced).await;
    client.navigate(&server.url("/pr/start")).await;
    let successor = wait_for_prerender(browser, cursor).await;
    assert!(
        tab_position(browser, &successor).is_none(),
        "a prerender page was listed as a tab"
    );
    client.wait_until_prerender_ready().await;
    client.click("#go").await;
    let activated = browser
        .connection()
        .events()
        .wait_for(cursor, Instant::now() + Duration::from_secs(10), |event| {
            is_activation_of(event, &successor)
        })
        .await
        .expect("read the event log");
    assert!(activated.is_some(), "{successor} was never activated");
    e2e::wait_until(Duration::from_secs(5), "the successor tab", || {
        tab_position(browser, &replaced).is_none()
            && tab_position(browser, &successor) == Some(position)
    })
    .await;
    let page = browser
        .page(&replaced)
        .await
        .expect("page() of the replaced tab follows its successor");
    assert_eq!(page.target_id(), &successor);
    let url = page.url().await.expect("url of the successor");
    assert!(url.ends_with("/pr/next"), "{url}");
    assert_eq!(e2e::text_of(&page.main_frame(), "#t").await, "PRERENDERED");
    client.close().await;
}

fn tab_position(browser: &Browser, target: &TargetId) -> Option<usize> {
    browser
        .pages()
        .iter()
        .position(|info| info.target_id == *target)
}

async fn wait_for_tab(browser: &Browser, target: &TargetId) -> usize {
    let mut position = None;
    e2e::wait_until(Duration::from_secs(10), "the tab client's page", || {
        position = tab_position(browser, target);
        position.is_some()
    })
    .await;
    position.expect("wait_until returns only after the tab appeared")
}

async fn wait_for_prerender(browser: &Browser, cursor: EventCursor) -> TargetId {
    let deadline = Instant::now() + Duration::from_secs(10);
    let created = browser
        .connection()
        .events()
        .wait_for(cursor, deadline, is_prerender_target)
        .await
        .expect("read the event log")
        .expect("Chrome did not prerender for a tab-target client");
    TargetId::from(
        created.params["targetInfo"]["targetId"]
            .as_str()
            .expect("the prerender target has an id"),
    )
}

fn is_prerender_target(event: &Event) -> bool {
    event.method.as_ref() == "Target.targetCreated"
        && event.params["targetInfo"]["subtype"] == "prerender"
}

fn is_activation_of(event: &Event, target: &TargetId) -> bool {
    let info = &event.params["targetInfo"];
    event.method.as_ref() == "Target.targetInfoChanged"
        && info["targetId"] == target.as_str()
        && info["subtype"].as_str().unwrap_or_default().is_empty()
}

async fn attach_new_tab(connection: &Connection) -> SessionId {
    let root = connection.session(None);
    let created = root
        .send(
            "Target.createTarget",
            json!({"url": "about:blank", "forTab": true}),
        )
        .await
        .expect("create a tab target");
    let tab = created["targetId"].as_str().expect("a tab target id");
    let attached = root
        .send(
            "Target.attachToTarget",
            json!({"targetId": tab, "flatten": true}),
        )
        .await
        .expect("attach the tab target");
    SessionId::from(attached["sessionId"].as_str().expect("a tab session id"))
}

async fn auto_attached_page(connection: &Connection, tab_session: &SessionId) -> Event {
    let cursor = connection.events().cursor();
    let auto_attach = json!({"autoAttach": true, "flatten": true, "waitForDebuggerOnStart": false});
    connection
        .session(Some(tab_session.clone()))
        .send("Target.setAutoAttach", auto_attach)
        .await
        .expect("auto-attach the tab's page");
    let deadline = Instant::now() + Duration::from_secs(10);
    connection
        .events()
        .wait_for(cursor, deadline, |event| {
            event.method.as_ref() == "Target.attachedToTarget"
                && event.session.as_ref() == Some(tab_session)
                && event.params["targetInfo"]["type"] == "page"
        })
        .await
        .expect("read the tab client's event log")
        .expect("the tab target never attached its page")
}

/// A second CDP client that holds one tab through its tab target, like Puppeteer.
struct TabClient {
    connection: Connection,
    preload: Arc<e2e::Captured>,
    page: TargetId,
    page_session: SessionId,
}

impl TabClient {
    async fn open(ws_url: &str) -> TabClient {
        let transport = WsTransport::connect(&ConnectOptions::new(ws_url))
            .await
            .expect("connect the tab-target client");
        let connection = Connection::start(Box::new(transport), ConnectionOptions::default());
        let preload = e2e::Captured::on(&connection, "Preload.");
        let tab_session = attach_new_tab(&connection).await;
        let page = auto_attached_page(&connection, &tab_session).await;
        let client = TabClient {
            page: TargetId::from(
                page.params["targetInfo"]["targetId"]
                    .as_str()
                    .expect("a page id"),
            ),
            page_session: SessionId::from(page.params["sessionId"].as_str().expect("a session")),
            connection,
            preload,
        };
        client.send("Preload.enable", json!({})).await;
        client
    }

    async fn send(&self, method: &str, params: Value) -> Value {
        self.connection
            .session(Some(self.page_session.clone()))
            .send(method, params)
            .await
            .unwrap_or_else(|error| panic!("{method} from the tab client: {error}"))
    }

    async fn navigate(&self, url: &str) {
        self.send("Page.navigate", json!({"url": url})).await;
    }

    async fn wait_until_prerender_ready(&self) {
        let ready = self
            .preload
            .wait_for(Duration::from_secs(10), |event| {
                event.method.as_ref() == "Preload.prerenderStatusUpdated"
                    && event.params["status"] == "Ready"
            })
            .await;
        assert!(ready.is_some(), "the prerender never became ready");
    }

    async fn click(&self, css: &str) {
        let expression = format!("document.querySelector({css:?}).click()");
        let params = json!({"expression": expression, "userGesture": true});
        self.send("Runtime.evaluate", params).await;
    }

    async fn close(self) {
        self.connection.close().await;
    }
}

chromium_test!(popup_read_waits_for_the_seeded_load, |ctx| {
    let server = TestServer::start().await;
    let browser = ctx.launch().await;
    let page = e2e::first_page(&browser).await;
    page.goto(&server.url("/popup")).await.expect("goto /popup");
    click(&page.main_frame(), "#open").await;
    let info = e2e::wait_for_popup(&browser, page.target_id(), "/slowbody").await;
    let popup = browser
        .page(&info.target_id)
        .await
        .expect("attach the popup");
    let started = Instant::now();
    assert_eq!(e2e::text_of(&popup.main_frame(), "#t").await, "NEXT");
    ctx.observe(format!("the popup read waited {:?}", started.elapsed()));
    browser.close().await.expect("close");
});

chromium_test!(
    device_emulation_reaches_oopifs_without_resizing_them,
    |ctx| {
        let server = TestServer::start().await;
        let browser = ctx.launch().await;
        let page = e2e::first_page(&browser).await;
        page.cdp(
            "Emulation.setUserAgentOverride",
            json!({"userAgent": MOBILE_USER_AGENT}),
        )
        .await
        .expect("user agent override");
        let device = json!({"width": 390, "height": 844, "deviceScaleFactor": 3, "mobile": true});
        page.cdp("Emulation.setDeviceMetricsOverride", device)
            .await
            .expect("device metrics override");
        page.goto(&server.localhost_url("/emulation"))
            .await
            .expect("goto /emulation");
        page.cdp(
            "Emulation.setTimezoneOverride",
            json!({"timezoneId": "Asia/Tokyo"}),
        )
        .await
        .expect("timezone override");
        let frame = e2e::child_frame(&page.main_frame(), "iframe#probe").await;
        let inside = e2e::script_value(&frame, PROBE_SCRIPT).await;
        assert_eq!(inside["tz"], "Asia/Tokyo", "OOPIF {inside}");
        assert_eq!(inside["ua"], MOBILE_USER_AGENT, "OOPIF {inside}");
        assert_eq!(inside["width"], 300, "OOPIF {inside}");
        let top = e2e::script_value(&page.main_frame(), PROBE_SCRIPT).await;
        assert_eq!(top["width"], 390, "top {top}");
        assert_eq!(top["tz"], "Asia/Tokyo", "top {top}");
        browser.close().await.expect("close");
    }
);

chromium_test!(a_parent_overlay_intercepts_clicks_into_an_oopif, |ctx| {
    let server = TestServer::start().await;
    let browser = ctx.launch().await;
    let page = e2e::first_page(&browser).await;
    page.goto(&server.localhost_url("/overlay"))
        .await
        .expect("goto /overlay");
    let frame = e2e::child_frame(&page.main_frame(), "iframe#covered").await;
    let button = e2e::first_element(&frame, "#b").await;
    assert_eq!(
        button
            .click(ClickOptions::default())
            .await
            .expect("W3C click"),
        ActionOutcome::Completed
    );
    assert_eq!(e2e::text_of(&page.main_frame(), "#hits").await, "1");
    assert_eq!(e2e::text_of(&frame, "#out").await, "0");
    let intercepted = button
        .element_click()
        .await
        .expect_err("an element click went through the overlay");
    assert!(
        matches!(intercepted, BrowserError::ClickIntercepted { .. }),
        "{intercepted:?}"
    );
    assert!(
        intercepted
            .to_string()
            .starts_with("element click intercepted: Element <button> is not clickable at point ("),
        "{intercepted}"
    );
    assert_eq!(e2e::text_of(&frame, "#out").await, "0");
    browser.close().await.expect("close");
});

const FAR_SPACER: &str = "document.body.insertAdjacentHTML('afterbegin', '<div id=spacer style=\"height:3000px\"></div>'); return true";
const NEAR_SPACER: &str = "document.body.insertAdjacentHTML('afterbegin', '<div id=spacer style=\"height:600px\"></div>'); scrollTo(0, 200); return scrollY";
const FRAME_SPACER: &str = "document.body.insertAdjacentHTML('afterbegin', '<div style=\"height:1000px\"></div>'); return true";

async fn scrolled_oopif_page(page: &Page, server: &TestServer, spacer: &str) -> Frame {
    page.goto(&server.localhost_url("/oopif"))
        .await
        .expect("goto the OOPIF page");
    let main = page.main_frame();
    let cross = e2e::child_frame(&main, "#cross").await;
    e2e::first_element(&cross, "#b").await;
    e2e::script_value(&main, spacer).await;
    cross
}

async fn element_click_counts(frame: &Frame, case: &str) -> String {
    let button = e2e::first_element(frame, "#b").await;
    let clicked = button
        .element_click()
        .await
        .unwrap_or_else(|error| panic!("element click, {case}: {error}"));
    assert_eq!(clicked, ActionOutcome::Completed, "{case}");
    e2e::text_of(frame, "#out").await
}

chromium_test!(
    element_click_hit_tests_scrolled_documents_in_document_coordinates,
    |ctx| {
        let server = TestServer::start().await;
        let browser = ctx.launch().await;
        let page = e2e::first_page(&browser).await;
        let cross = scrolled_oopif_page(&page, &server, FAR_SPACER).await;
        let main = page.main_frame();
        assert_eq!(
            element_click_counts(&main, "main frame below the fold").await,
            "1"
        );
        assert_eq!(
            element_click_counts(&cross, "OOPIF on a scrolled parent").await,
            "1"
        );
        e2e::script_value(&cross, FRAME_SPACER).await;
        assert_eq!(
            element_click_counts(&cross, "scrolled OOPIF document").await,
            "2"
        );
        scrolled_oopif_page(&page, &server, NEAR_SPACER).await;
        let main = page.main_frame();
        assert_eq!(
            element_click_counts(&main, "lightly scrolled main frame").await,
            "1",
            "the viewport point must not be hit-tested as a document point"
        );
        browser.close().await.expect("close");
    }
);

chromium_test!(
    attached_port_lists_tabs_without_attaching_and_downloads_with_allow,
    |ctx| {
        let server = TestServer::start().await;
        let profile = ctx.scratch_dir("attached-profile");
        let chrome = ctx.spawn_unconnected(&profile).await;
        let address = format!("127.0.0.1:{}", chrome.port);
        let endpoint = attach::resolve_endpoint_with(&address, BrowserKind::Chrome, None)
            .await
            .expect("resolve the debugging port");
        assert!(
            matches!(endpoint, AttachEndpoint::PortMode { .. }),
            "{endpoint:?}"
        );
        let browser = Browser::attach(endpoint)
            .await
            .expect("attach in port mode");
        ctx.watch(&browser);
        assert_eq!(browser.kind(), ConnectionKind::AttachedPort);
        let background = open_background_tab(&browser, &server.url("/titled")).await;
        wait_for_target_title(&browser, &background, "Titled").await;
        let events = browser.connection().events();
        let cursor = events.cursor();
        let listed = browser.list_pages(Duration::from_secs(2)).await;
        assert_eq!(
            e2e::title_of(&listed, &background).as_deref(),
            Some("Titled"),
            "{listed:?}"
        );
        let attach = events
            .wait_for(cursor, Instant::now(), |event| {
                event.method.as_ref() == "Target.attachedToTarget"
                    && event.params["targetInfo"]["targetId"] == background.as_str()
            })
            .await
            .expect("read the event log");
        assert!(
            attach.is_none(),
            "list_pages attached the background tab: {attach:?}"
        );
        assert!(
            !is_attached(&browser, &background).await,
            "the background tab is attached after list_pages"
        );
        let page = browser
            .new_page()
            .await
            .expect("open a tab to download from");
        let directory = ctx.scratch_dir("attached-downloads");
        download_round_trip(&browser, &page, &server, &directory).await;
        browser.close().await.expect("disconnect");
        assert!(
            !e2e::processes_using(&profile).is_empty(),
            "disconnecting closed the attached Chrome"
        );
        e2e::stop_spawned(&chrome).await;
        e2e::assert_no_process(&profile).await;
    }
);

async fn open_background_tab(browser: &Browser, url: &str) -> TargetId {
    let created = browser
        .root()
        .send(
            "Target.createTarget",
            json!({"url": url, "background": true}),
        )
        .await
        .expect("Target.createTarget");
    TargetId::from(
        created["targetId"]
            .as_str()
            .expect("createTarget returns a targetId"),
    )
}

async fn is_attached(browser: &Browser, target: &TargetId) -> bool {
    let info = browser
        .root()
        .send("Target.getTargetInfo", json!({"targetId": target}))
        .await
        .expect("Target.getTargetInfo");
    info["targetInfo"]["attached"].as_bool().unwrap_or(false)
}

struct Recording {
    browser: Browser,
    proxy: header_proxy::ProxyHandle,
    ports: Vec<u16>,
    chrome: String,
}

impl Recording {
    async fn start(ctx: &E2e, launched: &Browser, server: &TestServer) -> Recording {
        let proxy = header_proxy::start(e2e::browser_ws_url(launched).await, None, None).await;
        let browser = Browser::connect(ConnectOptions::new(proxy.url.clone()))
            .await
            .expect("connect through the recording proxy");
        ctx.watch(&browser);
        let ports = vec![
            server.port(),
            e2e::debug_port(launched),
            e2e::url_port(&proxy.url),
        ];
        Recording {
            browser,
            proxy,
            ports,
            chrome: e2e::chrome_version(launched),
        }
    }

    async fn finish(self, ctx: &E2e, launched: &Browser, name: &str) -> Vec<Value> {
        self.browser
            .close()
            .await
            .expect("close the recording connection");
        let lines = self
            .proxy
            .transcript
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let mut dirs = vec![
            e2e::profile_of(launched),
            ctx.scratch_root().to_path_buf(),
            e2e::profile_root(),
            std::env::temp_dir(),
        ];
        dirs.extend(dirs::home_dir());
        let frames = e2e::normalize_transcript(&lines, &self.ports, &dirs);
        if e2e::recording_enabled() {
            let path = e2e::write_fixture(name, &self.chrome, &frames);
            eprintln!("recorded {} frames into {}", frames.len(), path.display());
        }
        frames
    }
}

#[derive(Clone, Copy)]
enum Fixture {
    OopifSwap,
    Dialogs,
    Downloads,
    Navigation,
    Crash,
    AxTree,
}

impl Fixture {
    fn name(self) -> &'static str {
        match self {
            Fixture::OopifSwap => "oopif_swap",
            Fixture::Dialogs => "dialogs",
            Fixture::Downloads => "downloads",
            Fixture::Navigation => "navigation",
            Fixture::Crash => "crash",
            Fixture::AxTree => "ax_tree",
        }
    }

    async fn play(self, ctx: &E2e, browser: &Browser, server: &TestServer) {
        let page = browser.new_page().await.expect("open a recording tab");
        match self {
            Fixture::OopifSwap => oopif_swaps(&page, server).await,
            Fixture::Dialogs => dialog_round_trip(&page, server).await,
            Fixture::Downloads => {
                staged_download(browser, &page, server, &ctx.scratch_dir("staging")).await;
                let directory = ctx.scratch_dir("recorded-downloads");
                download_round_trip(browser, &page, server, &directory).await;
            }
            Fixture::Navigation => navigation_round_trip(&page, server).await,
            Fixture::Crash => crash_round_trip(ctx, &page, server).await,
            Fixture::AxTree => {
                page.goto(&server.url("/form")).await.expect("goto /form");
                Snapshot::take(&page, &RefTable::default()).await;
            }
        }
    }
}

async fn oopif_swaps(page: &Page, server: &TestServer) {
    page.goto(&server.localhost_url("/oopif"))
        .await
        .expect("goto /oopif");
    let buttons = server.oopif_buttons();
    let snapshot = Snapshot::until(page, &RefTable::default(), |snapshot| {
        buttons.iter().all(|name| snapshot.has("button", name))
    })
    .await;
    snapshot.reference("button", "Frame button leaf");
    swap_the_cross_site_frame(page, server, &snapshot).await;
}

/// The event shape of a launched browser: `allowAndName` stages the file under its GUID.
async fn staged_download(browser: &Browser, page: &Page, server: &TestServer, staging: &Path) {
    let behavior =
        json!({"behavior": "allowAndName", "downloadPath": staging, "eventsEnabled": true});
    browser
        .root()
        .send("Browser.setDownloadBehavior", behavior)
        .await
        .expect("allowAndName on the recording connection");
    page.goto(&server.url("/case/download"))
        .await
        .expect("goto the download page");
    let since = browser.downloads().cursor();
    click(&page.main_frame(), "#go").await;
    let deadline = Instant::now() + Duration::from_secs(20);
    let record = browser
        .downloads()
        .wait_finished(since, false, deadline, |record| {
            record.suggested_filename == "d.bin"
        })
        .await
        .expect("wait for the staged download")
        .expect("the staged download finished within 20 s");
    assert_eq!(record.state, DownloadState::Completed, "{record:?}");
}

async fn navigation_round_trip(page: &Page, server: &TestServer) {
    page.goto(&server.url("/case/anchor"))
        .await
        .expect("goto anchor");
    click(&page.main_frame(), "#go").await;
    assert_eq!(e2e::text_of(&page.main_frame(), "#t").await, "NEXT");
    page.back().await.expect("back");
    assert_eq!(e2e::text_of(&page.main_frame(), "#t").await, "START");
    page.forward().await.expect("forward");
    page.reload().await.expect("reload");
    assert_eq!(e2e::text_of(&page.main_frame(), "#t").await, "NEXT");
    page.goto(&server.url("/case/anchor-hash"))
        .await
        .expect("goto anchor-hash");
    click(&page.main_frame(), "#go").await;
    assert!(page.url().await.expect("url").ends_with("#sec"));
}

async fn record_fixture(ctx: E2e, fixture: Fixture) {
    if !e2e::recording_enabled() {
        return;
    }
    let server = TestServer::start().await;
    let launched = ctx.launch().await;
    let recording = Recording::start(&ctx, &launched, &server).await;
    fixture.play(&ctx, &recording.browser, &server).await;
    let frames = recording.finish(&ctx, &launched, fixture.name()).await;
    assert!(!frames.is_empty(), "{} recorded nothing", fixture.name());
    launched.close().await.expect("close");
}

chromium_test!(record_oopif_swap_fixture, |ctx| {
    record_fixture(ctx, Fixture::OopifSwap).await;
});

chromium_test!(record_dialogs_fixture, |ctx| {
    record_fixture(ctx, Fixture::Dialogs).await;
});

chromium_test!(record_downloads_fixture, |ctx| {
    record_fixture(ctx, Fixture::Downloads).await;
});

chromium_test!(record_navigation_fixture, |ctx| {
    record_fixture(ctx, Fixture::Navigation).await;
});

chromium_test!(record_crash_fixture, |ctx| {
    record_fixture(ctx, Fixture::Crash).await;
});

chromium_test!(record_ax_tree_fixture, |ctx| {
    record_fixture(ctx, Fixture::AxTree).await;
});

#[test]
fn transcript_normaliser_aliases_ids_by_value_and_masks_ports_and_payloads() {
    let target = "AAAA1111BBBB2222CCCC3333DDDD4444";
    let session = "5555EEEE6666FFFF7777AAAA8888BBBB";
    let loader = "9999CCCC0000DDDD1111EEEE2222FFFF";
    let context = "C0FFEE00C0FFEE00C0FFEE00C0FFEE00";
    let lines = [
        json!({"t": 1, "dir": "recv", "frame": {"method": "Target.targetCreated", "params": {"targetInfo": {"targetId": target, "type": "page", "url": "http://localhost:4321/top", "browserContextId": context}}}}),
        json!({"t": 2, "dir": "recv", "frame": {"method": "Page.frameNavigated", "sessionId": session, "params": {"frame": {"id": target, "loaderId": loader, "url": "http://127.0.0.1:43210/x"}}}}),
        json!({"t": 3, "dir": "send", "frame": {"id": 7, "method": "Page.captureScreenshot", "sessionId": session}}),
        json!({"t": 3, "dir": "send", "frame": {"id": 8, "method": "Runtime.evaluate", "params": {"expression": "x".repeat(2000)}}}),
        json!({"t": 4, "dir": "recv", "frame": {"id": 7, "sessionId": session, "result": {"data": "A".repeat(5000)}}}),
        json!({"t": 5, "dir": "recv", "frame": {"method": "Target.targetInfoChanged", "params": {"targetInfo": {"targetId": target, "url": format!("devtools://devtools/page/{target}")}}}}),
    ]
    .map(|line| line.to_string());
    let frames = e2e::normalize_transcript(&lines, &[4321], &[]);
    let created = json!({"dir": "recv", "frame": {"method": "Target.targetCreated", "params": {"targetInfo": {"targetId": "T1", "type": "page", "url": "http://localhost:{port}/top", "browserContextId": "C1"}}}});
    assert_eq!(frames[0], created);
    let navigated = &frames[1]["frame"];
    assert_eq!(navigated["sessionId"], "S1");
    assert_eq!(navigated["params"]["frame"]["id"], "T1");
    assert_eq!(navigated["params"]["frame"]["loaderId"], "L1");
    assert_eq!(
        navigated["params"]["frame"]["url"],
        "http://127.0.0.1:43210/x"
    );
    let sent = json!({"dir": "send", "frame": {"id": 7, "method": "Page.captureScreenshot", "sessionId": "S1"}});
    assert_eq!(frames[2], sent);
    assert_eq!(
        frames[3]["frame"]["params"]["expression"],
        "<text 2000 chars>"
    );
    assert_eq!(frames[4]["frame"]["result"]["data"], "<base64 5000 bytes>");
    assert_eq!(
        frames[5]["frame"]["params"]["targetInfo"]["url"],
        "devtools://devtools/page/T1"
    );
}

#[test]
fn transcript_normaliser_leaves_short_node_ids_alone_and_scrubs_local_dirs() {
    let frame = "0123456789ABCDEF0123456789ABCDEF";
    let request = "FEDCBA9876543210FEDCBA9876543210";
    let guid = "0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0";
    let dir = std::env::temp_dir().join("flow-like-e2e-normaliser");
    let staged = dir.join(guid);
    let ax = json!({"nodes": [
        {"nodeId": "4", "parentId": "1", "childIds": ["12"], "frameId": frame},
        {"nodeId": "12", "parentId": "4", "name": {"value": "4"}},
    ]});
    let lines = [
        json!({"dir": "recv", "frame": {"id": 1, "result": ax}}),
        json!({"dir": "recv", "frame": {"method": "Network.requestWillBeSent", "params": {"requestId": request, "frameId": frame}}}),
        json!({"dir": "recv", "frame": {"method": "Browser.downloadProgress", "params": {"guid": guid, "filePath": staged}}}),
    ]
    .map(|line| line.to_string());
    let frames = e2e::normalize_transcript(&lines, &[], &[dir]);
    let nodes = &frames[0]["frame"]["result"]["nodes"];
    assert_eq!(nodes[0]["nodeId"], "4");
    assert_eq!(nodes[0]["parentId"], "1");
    assert_eq!(nodes[0]["childIds"], json!(["12"]));
    assert_eq!(nodes[0]["frameId"], "F1");
    assert_eq!(nodes[1]["parentId"], "4");
    assert_eq!(nodes[1]["name"]["value"], "4");
    assert_eq!(frames[1]["frame"]["params"]["requestId"], "X1");
    let progress = &frames[2]["frame"]["params"];
    assert_eq!(progress["guid"], "G1");
    assert_eq!(
        progress["filePath"]
            .as_str()
            .map(|path| path.replace('\\', "/")),
        Some("{dir}/G1".to_owned())
    );
}

#[test]
fn fixture_dates_follow_the_gregorian_calendar() {
    assert_eq!(e2e::civil_from_days(0), (1970, 1, 1));
    assert_eq!(e2e::civil_from_days(11_016), (2000, 2, 29));
    assert_eq!(e2e::civil_from_days(20_726), (2026, 9, 30));
}
