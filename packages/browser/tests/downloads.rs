use std::path::Path;
use std::time::Duration;

use flow_like_browser::downloads::{DownloadDestination, DownloadRecord};
use flow_like_browser::test_hooks::TestSetup;
use flow_like_browser::testing::default_auto_reply;
use flow_like_browser::transport::memory::{InMemoryControl, InMemoryTransport, SentCommand};
use flow_like_browser::types::DownloadState;
use flow_like_browser::{Browser, BrowserError, ConnectionKind};
use serde_json::{Value, json};

const WAIT: Duration = Duration::from_secs(5);
const QUIET: Duration = Duration::from_millis(150);
const APPROVAL_ERROR: &str = "Setting the download folder would change how your own Chrome handles every download until it restarts; downloads go to Chrome's download folder";

struct Scripted {
    browser: Browser,
    control: InMemoryControl,
}

fn chrome_reply(command: &SentCommand) -> Option<Value> {
    match command.method.as_str() {
        "Browser.getVersion" => Some(json!({
            "protocolVersion": "1.3",
            "product": "Chrome/154.0.8037.92",
            "revision": "@0",
            "userAgent": "Mozilla/5.0 Chrome/154.0.8037.92",
            "jsVersion": "15.4",
        })),
        "Target.getTargets" => Some(json!({"targetInfos": []})),
        _ => Some(default_auto_reply(command).unwrap_or_else(|| json!({}))),
    }
}

fn setup(kind: ConnectionKind, staging: Option<&Path>, run_owned_setup: bool) -> TestSetup {
    TestSetup {
        kind,
        headless: true,
        page_load_timeout: Duration::from_secs(30),
        process: None,
        staging: staging.map(Path::to_path_buf),
        run_owned_setup,
    }
}

async fn connect(kind: ConnectionKind, staging: Option<&Path>) -> Scripted {
    let (transport, control) = InMemoryTransport::new();
    control.set_auto_reply(chrome_reply);
    let browser = Browser::connect_transport(Box::new(transport), setup(kind, staging, false))
        .await
        .expect("the scripted browser did not connect");
    let scripted = Scripted { browser, control };
    let tab = json!({"targetInfo": {"targetId": "T1", "type": "page", "url": "about:blank", "attached": false}});
    scripted.emit("Target.targetCreated", None, tab).await;
    scripted
}

fn domain(session: Option<&str>) -> &'static str {
    if session.is_some() { "Page" } else { "Browser" }
}

impl Scripted {
    async fn emit(&self, method: &str, session: Option<&str>, params: Value) {
        let events = self.browser.connection().events();
        let cursor = events.cursor();
        self.control.emit(method, session, params);
        let deadline = tokio::time::Instant::now() + WAIT;
        let seen = events
            .wait_for(cursor, deadline, |event| &*event.method == method)
            .await
            .expect("the event log closed");
        assert!(seen.is_some(), "the reader never processed {method}");
    }

    async fn begin(&self, session: Option<&str>, guid: &str, frame: &str, name: &str) {
        let params = json!({"frameId": frame, "guid": guid, "url": format!("http://127.0.0.1/{name}"), "suggestedFilename": name});
        let method = format!("{}.downloadWillBegin", domain(session));
        self.emit(&method, session, params).await;
    }

    async fn finish(&self, session: Option<&str>, guid: &str, state: &str, path: Option<&Path>) {
        let mut params =
            json!({"guid": guid, "totalBytes": 3.0, "receivedBytes": 3.0, "state": state});
        if let Some(path) = path {
            params["filePath"] = json!(path.to_str().unwrap());
        }
        let method = format!("{}.downloadProgress", domain(session));
        self.emit(&method, session, params).await;
    }

    async fn download(&self, since: u64, guid: &str) -> DownloadRecord {
        let deadline = tokio::time::Instant::now() + WAIT;
        self.browser
            .downloads()
            .wait_finished(since, false, deadline, |record| record.guid == guid)
            .await
            .expect("waiting for the download failed")
            .unwrap_or_else(|| panic!("download {guid} never finished"))
    }

    async fn no_download(&self, since: u64, guid: &str) {
        let deadline = tokio::time::Instant::now() + QUIET;
        let found = self
            .browser
            .downloads()
            .wait_finished(since, false, deadline, |record| record.guid == guid)
            .await
            .unwrap();
        assert!(
            found.is_none(),
            "download {guid} should not be reported: {found:?}"
        );
    }

    fn download_behaviors(&self) -> Vec<SentCommand> {
        self.control
            .commands_seen()
            .into_iter()
            .filter(|command| command.method.ends_with(".setDownloadBehavior"))
            .collect()
    }
}

#[tokio::test]
async fn a_launched_browser_stages_downloads_with_allow_and_name() {
    let staging = tempfile::tempdir().unwrap();
    let (transport, mut control) = InMemoryTransport::new();
    control.set_auto_reply(|command| {
        (command.method != "Browser.setDownloadBehavior")
            .then(|| chrome_reply(command))
            .flatten()
    });
    let setup = setup(ConnectionKind::Launched, Some(staging.path()), true);
    let connecting = Browser::connect_transport(Box::new(transport), setup);
    tokio::pin!(connecting);
    let command = tokio::select! {
        command = control.wait_for("Browser.setDownloadBehavior", None) => command,
        outcome = &mut connecting => panic!("construction ended without configuring downloads: {:?}", outcome.err()),
    };
    let staging_path = staging.path().to_str().unwrap();
    assert_eq!(
        command.params,
        json!({"behavior": "allowAndName", "downloadPath": staging_path, "eventsEnabled": true})
    );
    let page_level = control
        .commands_seen()
        .into_iter()
        .filter(|command| command.method == "Page.setDownloadBehavior")
        .count();
    assert_eq!(page_level, 0);
}

#[tokio::test]
async fn a_launched_download_moves_from_staging_and_replaces_an_existing_file() {
    let staging = tempfile::tempdir().unwrap();
    let folder = tempfile::tempdir().unwrap();
    let scripted = connect(ConnectionKind::Launched, Some(staging.path())).await;
    let browser = &scripted.browser;
    browser.set_download_directory(folder.path()).await.unwrap();
    assert!(scripted.download_behaviors().is_empty());
    let target = folder.path().join("report.txt");
    std::fs::write(&target, "old").unwrap();
    let staged = staging.path().join("g1");
    std::fs::write(&staged, "new").unwrap();
    let since = browser.downloads().cursor();
    let before_begin = std::time::SystemTime::now();
    scripted.begin(None, "g1", "T1", "report.txt").await;
    let after_begin = std::time::SystemTime::now();
    scripted.finish(None, "g1", "inProgress", None).await;
    scripted
        .finish(None, "g1", "completed", Some(&staged))
        .await;
    let record = scripted.download(since, "g1").await;
    assert!(record.seq > since);
    assert!(
        (before_begin..=after_begin).contains(&record.begun_at),
        "begun_at is taken when downloadWillBegin arrives"
    );
    assert_eq!(record.state, DownloadState::Completed);
    assert_eq!(record.suggested_filename, "report.txt");
    assert_eq!(
        record.frame_id.as_ref().map(|frame| frame.as_str()),
        Some("T1")
    );
    assert_eq!(record.final_path.as_deref(), Some(target.as_path()));
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "new");
    assert!(!staged.exists());
}

#[tokio::test]
async fn a_launched_download_without_a_file_path_uses_the_guid_in_staging() {
    let staging = tempfile::tempdir().unwrap();
    let folder = tempfile::tempdir().unwrap();
    let scripted = connect(ConnectionKind::Launched, Some(staging.path())).await;
    let path = folder.path().to_path_buf();
    let downloads = scripted.browser.downloads();
    downloads.set_destination(DownloadDestination::Directory { path });
    std::fs::write(staging.path().join("g2"), "data").unwrap();
    let since = downloads.cursor();
    scripted.begin(None, "g2", "T1", "../evil:name.txt").await;
    scripted.finish(None, "g2", "completed", None).await;
    let record = scripted.download(since, "g2").await;
    let expected = folder.path().join("evil_name.txt");
    assert_eq!(record.final_path.as_deref(), Some(expected.as_path()));
    assert_eq!(std::fs::read_to_string(expected).unwrap(), "data");
}

#[tokio::test]
async fn a_canceled_download_is_reported_as_canceled() {
    let staging = tempfile::tempdir().unwrap();
    let scripted = connect(ConnectionKind::Launched, Some(staging.path())).await;
    let since = scripted.browser.downloads().cursor();
    scripted.begin(None, "g3", "T1", "big.iso").await;
    scripted.finish(None, "g3", "canceled", None).await;
    let record = scripted.download(since, "g3").await;
    assert_eq!(record.state, DownloadState::Canceled);
    assert_eq!(record.final_path, None);
}

#[tokio::test]
async fn a_launched_browser_ignores_page_download_events() {
    let staging = tempfile::tempdir().unwrap();
    let folder = tempfile::tempdir().unwrap();
    let scripted = connect(ConnectionKind::Launched, Some(staging.path())).await;
    let browser = &scripted.browser;
    browser.set_download_directory(folder.path()).await.unwrap();
    let since = browser.downloads().cursor();
    scripted.begin(Some("S1"), "page-only", "T1", "a.txt").await;
    scripted
        .finish(Some("S1"), "page-only", "completed", None)
        .await;
    scripted.begin(None, "g4", "T1", "b.txt").await;
    scripted.begin(Some("S1"), "g4", "T1", "b.txt").await;
    scripted.finish(Some("S1"), "g4", "completed", None).await;
    scripted.no_download(since, "page-only").await;
    scripted.no_download(since, "g4").await;
    let staged = staging.path().join("g4");
    std::fs::write(&staged, "b").unwrap();
    scripted
        .finish(None, "g4", "completed", Some(&staged))
        .await;
    let record = scripted.download(since, "g4").await;
    assert_eq!(record.final_path, Some(folder.path().join("b.txt")));
}

#[tokio::test]
async fn port_mode_allows_downloads_into_the_folder_and_ignores_foreign_frames() {
    let folder = tempfile::tempdir().unwrap();
    let scripted = connect(ConnectionKind::AttachedPort, None).await;
    let browser = &scripted.browser;
    browser.set_download_directory(folder.path()).await.unwrap();
    let behaviors = scripted.download_behaviors();
    assert_eq!(behaviors.len(), 1);
    assert_eq!(behaviors[0].session, None);
    let folder_path = folder.path().to_str().unwrap();
    assert_eq!(
        behaviors[0].params,
        json!({"behavior": "allow", "downloadPath": folder_path, "eventsEnabled": true})
    );
    let page = json!({"sessionId": "S1", "targetInfo": {"targetId": "T1", "type": "page", "url": "http://127.0.0.1/", "attached": true}, "waitingForDebugger": false});
    scripted.emit("Target.attachedToTarget", None, page).await;
    let since = browser.downloads().cursor();
    let foreign = folder.path().join("foreign.txt");
    scripted.begin(None, "foreign", "FX", "foreign.txt").await;
    scripted
        .finish(None, "foreign", "completed", Some(&foreign))
        .await;
    scripted.begin(None, "main", "T1", "main.txt").await;
    scripted.finish(None, "main", "completed", None).await;
    let child = folder.path().join("child.txt");
    scripted.begin(None, "child", "F5", "child.txt").await;
    scripted.begin(Some("S1"), "child", "F5", "child.txt").await;
    scripted
        .finish(None, "child", "completed", Some(&child))
        .await;
    let main = scripted.download(since, "main").await;
    assert_eq!(main.final_path, Some(folder.path().join("main.txt")));
    assert_eq!(
        scripted.download(since, "child").await.final_path,
        Some(child)
    );
    scripted.no_download(since, "foreign").await;
}

#[tokio::test]
async fn approval_mode_uses_page_events_and_never_sets_download_behavior() {
    let folder = tempfile::tempdir().unwrap();
    let scripted = connect(ConnectionKind::AttachedApproval, None).await;
    let browser = &scripted.browser;
    let error = browser
        .set_download_directory(folder.path())
        .await
        .unwrap_err();
    assert!(
        matches!(&error, BrowserError::Unsupported { message } if message == APPROVAL_ERROR),
        "{error:?}"
    );
    let since = browser.downloads().cursor();
    scripted.begin(None, "root", "T1", "root.txt").await;
    scripted.finish(None, "root", "completed", None).await;
    scripted.begin(Some("S1"), "page", "T1", "a.txt").await;
    scripted.finish(Some("S1"), "page", "completed", None).await;
    let record = scripted.download(since, "page").await;
    assert_eq!(record.state, DownloadState::Completed);
    assert_eq!(record.suggested_filename, "a.txt");
    assert_eq!(record.final_path, None);
    scripted.no_download(since, "root").await;
    assert!(scripted.download_behaviors().is_empty());
}

#[tokio::test]
async fn waiting_fails_with_disconnected_when_the_connection_closes() {
    let staging = tempfile::tempdir().unwrap();
    let scripted = connect(ConnectionKind::Launched, Some(staging.path())).await;
    let downloads = scripted.browser.downloads().clone();
    let since = downloads.cursor();
    let waiter = tokio::spawn(async move {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        downloads
            .wait_finished(since, true, deadline, |_| true)
            .await
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    scripted.control.close("the browser went away");
    let outcome = tokio::time::timeout(WAIT, waiter).await.unwrap().unwrap();
    assert!(
        matches!(&outcome, Err(BrowserError::Disconnected { reason }) if reason.contains("went away")),
        "{outcome:?}"
    );
}

#[tokio::test]
async fn include_in_progress_counts_downloads_running_at_the_cursor() {
    let scripted = connect(ConnectionKind::AttachedApproval, None).await;
    let downloads = scripted.browser.downloads();
    scripted.begin(Some("S1"), "done", "T1", "done.txt").await;
    scripted.finish(Some("S1"), "done", "completed", None).await;
    scripted
        .begin(Some("S1"), "running", "T1", "running.txt")
        .await;
    let since = downloads.cursor();
    scripted
        .finish(Some("S1"), "running", "completed", None)
        .await;
    let quiet = tokio::time::Instant::now() + QUIET;
    let without = downloads.wait_finished(since, false, quiet, |_| true).await;
    assert!(without.unwrap().is_none());
    let deadline = tokio::time::Instant::now() + WAIT;
    let with = downloads
        .wait_finished(since, true, deadline, |_| true)
        .await;
    let running = with
        .unwrap()
        .expect("the running download was not reported");
    assert_eq!(running.guid, "running");
    let quiet = tokio::time::Instant::now() + QUIET;
    let before = downloads
        .wait_finished(since, true, quiet, |record| record.guid == "done")
        .await;
    assert!(
        before.unwrap().is_none(),
        "a download finished before the cursor counted"
    );
}
