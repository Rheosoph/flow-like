use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use serde_json::json;
use tokio::sync::watch;

use crate::browser::{Browser, ConnectionKind};
use crate::connection::{Connection, EventHook};
use crate::error::BrowserError;
use crate::event_log::Event;
use crate::types::{
    AttachedToTarget, DownloadProgress, DownloadState, DownloadWillBegin, FrameId, SessionId,
    TargetId, TargetType,
};

const MAX_RECORDS: usize = 256;
const MAX_PAGE_CLAIMS: usize = 64;
const MAX_OPENER_LINKS: usize = 256;
const MAX_NAME_SUFFIX: u32 = 10_000;
const FALLBACK_NAME: &str = "download";
const RESERVED_NAME_CHARS: [char; 9] = ['/', '\\', ':', '*', '?', '"', '<', '>', '|'];
const DOUBLE_EXTENSION_SUFFIXES: [&str; 5] = ["gz", "xz", "bz2", "z", "bz"];
const INNER_EXTENSION_MAX_LEN: usize = 4;
const USER_SCRIPT_EXTENSION: &str = ".user.js";
const APPROVAL_DOWNLOAD_FOLDER: &str = "Setting the download folder would change how your own Chrome handles every download until it restarts; downloads go to Chrome's download folder";

#[derive(Clone, Debug)]
pub struct DownloadRecord {
    pub seq: u64,
    pub guid: String,
    pub url: String,
    pub suggested_filename: String,
    pub frame_id: Option<FrameId>,
    pub state: DownloadState,
    pub final_path: Option<std::path::PathBuf>,
    /// When this process received `Browser.downloadWillBegin`; the file is written after it.
    pub begun_at: std::time::SystemTime,
}

#[derive(Clone, Debug)]
pub enum DownloadDestination {
    Directory { path: std::path::PathBuf },
    OsDownloads,
}

#[derive(Clone)]
pub struct DownloadTracker {
    inner: Arc<TrackerInner>,
}

struct TrackerInner {
    kind: ConnectionKind,
    staging: Option<PathBuf>,
    connection: OnceLock<Connection>,
    state: Mutex<TrackerState>,
    changes: watch::Sender<u64>,
    moves: watch::Sender<usize>,
}

struct TrackerState {
    clock: u64,
    destination: DownloadDestination,
    allow_directory: Option<PathBuf>,
    downloads: VecDeque<Tracked>,
    page_claims: VecDeque<String>,
    page_targets: HashMap<SessionId, TargetId>,
    openers: VecDeque<(TargetId, TargetId)>,
}

struct Tracked {
    record: DownloadRecord,
    visible: bool,
    destination: DownloadDestination,
    finished: Option<u64>,
    failure: Option<MoveFailure>,
}

#[derive(Clone, Debug)]
struct MoveFailure {
    target: PathBuf,
    error: BrowserError,
}

struct Relocation {
    guid: String,
    source: PathBuf,
    name: String,
    destination: DownloadDestination,
}

#[derive(Default)]
struct Applied {
    notify: bool,
    relocation: Option<Relocation>,
}

impl Applied {
    fn notify(notify: bool) -> Self {
        Self {
            notify,
            relocation: None,
        }
    }
}

enum Candidate {
    Finished(DownloadRecord),
    Failed(DownloadRecord, MoveFailure),
}

impl TrackerState {
    fn tick(&mut self) -> u64 {
        self.clock += 1;
        self.clock
    }

    fn tracked_mut(&mut self, guid: &str) -> Option<&mut Tracked> {
        self.downloads
            .iter_mut()
            .find(|tracked| tracked.record.guid == guid)
    }

    fn is_page_target(&self, target: &str) -> bool {
        self.page_targets
            .values()
            .any(|page| page.as_str() == target)
    }

    fn opener_of(&self, target: &str) -> Option<&str> {
        self.openers
            .iter()
            .find(|(opened, _)| opened.as_str() == target)
            .map(|(_, opener)| opener.as_str())
    }

    fn is_page_frame(&self, frame: Option<&FrameId>) -> bool {
        let Some(mut current) = frame.map(FrameId::as_str) else {
            return false;
        };
        for _ in 0..=self.openers.len() {
            if self.is_page_target(current) {
                return true;
            }
            match self.opener_of(current) {
                Some(opener) => current = opener,
                None => return false,
            }
        }
        false
    }

    fn record_opener(&mut self, target: &str, opener: &str) {
        if opener.is_empty() || self.opener_of(target).is_some() {
            return;
        }
        self.openers
            .push_back((TargetId::from(target), TargetId::from(opener)));
        while self.openers.len() > MAX_OPENER_LINKS {
            self.openers.pop_front();
        }
    }

    fn take_page_claim(&mut self, guid: &str) -> bool {
        let position = self.page_claims.iter().position(|claim| claim == guid);
        position
            .and_then(|position| self.page_claims.remove(position))
            .is_some()
    }

    fn push(&mut self, tracked: Tracked) {
        self.downloads.push_back(tracked);
        while self.downloads.len() > MAX_RECORDS {
            self.downloads.pop_front();
        }
    }
}

struct MoveGuard(watch::Sender<usize>);

impl MoveGuard {
    fn start(moves: &watch::Sender<usize>) -> Self {
        moves.send_modify(|count| *count += 1);
        Self(moves.clone())
    }
}

impl Drop for MoveGuard {
    fn drop(&mut self) {
        self.0.send_modify(|count| *count = count.saturating_sub(1));
    }
}

impl DownloadTracker {
    pub(crate) fn new(
        kind: crate::browser::ConnectionKind,
        staging: Option<std::path::PathBuf>,
    ) -> Self {
        let state = TrackerState {
            clock: 0,
            destination: DownloadDestination::OsDownloads,
            allow_directory: None,
            downloads: VecDeque::new(),
            page_claims: VecDeque::new(),
            page_targets: HashMap::new(),
            openers: VecDeque::new(),
        };
        Self {
            inner: Arc::new(TrackerInner {
                kind,
                staging,
                connection: OnceLock::new(),
                state: Mutex::new(state),
                changes: watch::Sender::new(0),
                moves: watch::Sender::new(0),
            }),
        }
    }

    pub fn cursor(&self) -> u64 {
        self.with_state(|state| state.clock)
    }

    pub fn set_destination(&self, destination: DownloadDestination) {
        self.with_state(|state| state.destination = destination);
    }

    pub async fn wait_finished(
        &self,
        since: u64,
        include_in_progress: bool,
        deadline: tokio::time::Instant,
        matches: impl Fn(&DownloadRecord) -> bool,
    ) -> crate::Result<Option<DownloadRecord>> {
        let mut changes = self.inner.changes.subscribe();
        loop {
            if let Some(found) = self.first_finished(since, include_in_progress, &matches) {
                return found.map(Some);
            }
            let connection = self.inner.connection.get().cloned();
            if let Some(reason) = connection.as_ref().and_then(Connection::closed_reason) {
                return Err(BrowserError::Disconnected { reason });
            }
            tokio::select! {
                _ = changes.changed() => {}
                () = closed(connection.as_ref()) => {}
                () = tokio::time::sleep_until(deadline) => {
                    return self
                        .first_finished(since, include_in_progress, &matches)
                        .transpose();
                }
            }
        }
    }

    pub(crate) async fn moves_finished(&self, deadline: tokio::time::Instant) -> bool {
        let mut moves = self.inner.moves.subscribe();
        tokio::time::timeout_at(deadline, moves.wait_for(|count| *count == 0))
            .await
            .is_ok_and(|done| done.is_ok())
    }

    fn with_state<R>(&self, update: impl FnOnce(&mut TrackerState) -> R) -> R {
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        update(&mut state)
    }

    pub(crate) fn attach(&self, connection: &Connection) {
        if self.inner.connection.get().is_none()
            && self.inner.connection.set(connection.clone()).is_ok()
        {
            self.notify();
        }
    }

    fn notify(&self) {
        self.inner.changes.send_modify(|version| *version += 1);
    }

    fn first_finished(
        &self,
        since: u64,
        include_in_progress: bool,
        matches: &impl Fn(&DownloadRecord) -> bool,
    ) -> Option<crate::Result<DownloadRecord>> {
        let candidates: Vec<Candidate> = self.with_state(|state| {
            state
                .downloads
                .iter()
                .filter(|tracked| tracked.visible)
                .filter(|tracked| {
                    tracked.finished.is_some_and(|finished| {
                        finished > since && (include_in_progress || tracked.record.seq > since)
                    })
                })
                .map(|tracked| match &tracked.failure {
                    Some(failure) => Candidate::Failed(tracked.record.clone(), failure.clone()),
                    None => Candidate::Finished(tracked.record.clone()),
                })
                .collect()
        });
        candidates
            .into_iter()
            .find_map(|candidate| match candidate {
                Candidate::Finished(record) => matches(&record).then_some(Ok(record)),
                Candidate::Failed(mut record, failure) => {
                    record.final_path = Some(failure.target);
                    matches(&record).then_some(Err(failure.error))
                }
            })
    }

    fn apply(&self, state: &mut TrackerState, event: &Event) -> Applied {
        let root = event.session.is_none();
        let (domain, on_root) = match self.inner.kind {
            ConnectionKind::AttachedApproval => ("Page.", false),
            _ => ("Browser.", true),
        };
        if root == on_root
            && let Some(name) = event.method.strip_prefix(domain)
        {
            return match name {
                "downloadWillBegin" => Applied::notify(self.begin(state, event)),
                "downloadProgress" => self.progress(state, event),
                _ => Applied::default(),
            };
        }
        match self.inner.kind {
            ConnectionKind::AttachedPort | ConnectionKind::Direct => {
                Applied::notify(track_page_ownership(state, event, root))
            }
            _ => Applied::default(),
        }
    }

    fn begin(&self, state: &mut TrackerState, event: &Event) -> bool {
        let Some(begun) = event.decode::<DownloadWillBegin>() else {
            return false;
        };
        if begun.guid.is_empty() || state.tracked_mut(&begun.guid).is_some() {
            return false;
        }
        let visible = match self.inner.kind {
            ConnectionKind::Launched | ConnectionKind::AttachedApproval => true,
            ConnectionKind::AttachedPort | ConnectionKind::Direct => {
                state.take_page_claim(&begun.guid) || state.is_page_frame(begun.frame_id.as_ref())
            }
        };
        let seq = state.tick();
        let destination = state.destination.clone();
        state.push(Tracked {
            record: DownloadRecord {
                seq,
                guid: begun.guid,
                url: begun.url,
                suggested_filename: begun.suggested_filename,
                frame_id: begun.frame_id,
                state: DownloadState::InProgress,
                final_path: None,
                begun_at: std::time::SystemTime::now(),
            },
            visible,
            destination,
            finished: None,
            failure: None,
        });
        visible
    }

    fn progress(&self, state: &mut TrackerState, event: &Event) -> Applied {
        let Some(progress) = event.decode::<DownloadProgress>() else {
            return Applied::default();
        };
        if !matches!(
            progress.state,
            DownloadState::Completed | DownloadState::Canceled
        ) {
            return Applied::default();
        }
        let allow_directory = state.allow_directory.clone();
        let finished = state.clock + 1;
        let Some(tracked) = state
            .tracked_mut(&progress.guid)
            .filter(|tracked| tracked.record.state == DownloadState::InProgress)
        else {
            return Applied::default();
        };
        tracked.record.state = progress.state.clone();
        if progress.state == DownloadState::Completed
            && let Some(relocation) = self.complete(tracked, progress.file_path, allow_directory)
        {
            return Applied {
                notify: false,
                relocation: Some(relocation),
            };
        }
        tracked.finished = Some(finished);
        let visible = tracked.visible;
        state.clock = finished;
        Applied::notify(visible)
    }

    fn complete(
        &self,
        tracked: &mut Tracked,
        file_path: Option<String>,
        allow_directory: Option<PathBuf>,
    ) -> Option<Relocation> {
        let reported = file_path.map(PathBuf::from);
        match self.inner.kind {
            ConnectionKind::Launched => {
                match relocation_for(tracked, reported, self.inner.staging.as_deref()) {
                    Ok(relocation) => return Some(relocation),
                    Err(failure) => tracked.failure = Some(failure),
                }
            }
            ConnectionKind::AttachedApproval => tracked.record.final_path = reported,
            ConnectionKind::AttachedPort | ConnectionKind::Direct => {
                let name = last_component(&tracked.record.suggested_filename);
                tracked.record.final_path =
                    reported.or_else(|| allow_directory.map(|directory| directory.join(name)));
            }
        }
        None
    }

    fn spawn_relocation(&self, relocation: Relocation) {
        let guard = MoveGuard::start(&self.inner.moves);
        let tracker = self.clone();
        tokio::spawn(async move {
            let Relocation {
                guid,
                source,
                name,
                destination,
            } = relocation;
            let outcome = move_download(&source, &destination, &name).await;
            tracker.finish_move(&guid, outcome);
            drop(guard);
        });
    }

    fn finish_move(&self, guid: &str, outcome: Result<PathBuf, MoveFailure>) {
        let visible = self.with_state(|state| {
            let finished = state.tick();
            let tracked = state.tracked_mut(guid)?;
            match outcome {
                Ok(path) => tracked.record.final_path = Some(path),
                Err(failure) => {
                    tracing::warn!(guid, error = %failure.error, "a finished download could not be moved to its destination");
                    tracked.failure = Some(failure);
                }
            }
            tracked.finished = Some(finished);
            Some(tracked.visible)
        });
        if visible == Some(true) {
            self.notify();
        }
    }
}

impl EventHook for DownloadTracker {
    fn on_event(&self, connection: &Connection, event: &Event) {
        self.attach(connection);
        let applied = self.with_state(|state| self.apply(state, event));
        if let Some(relocation) = applied.relocation {
            self.spawn_relocation(relocation);
        }
        if applied.notify {
            self.notify();
        }
    }
}

impl Browser {
    pub(crate) async fn configure_downloads(&self) -> crate::Result<()> {
        let tracker = &self.inner.downloads;
        tracker.attach(self.connection());
        if self.kind() != ConnectionKind::Launched {
            return Ok(());
        }
        let staging = tracker
            .inner
            .staging
            .as_deref()
            .ok_or_else(|| BrowserError::Launch {
                message: "The launched browser has no download staging directory".to_owned(),
            })?;
        let params = json!({
            "behavior": "allowAndName",
            "downloadPath": utf8_path(staging)?,
            "eventsEnabled": true,
        });
        self.root()
            .send("Browser.setDownloadBehavior", params)
            .await
            .map(drop)
    }

    pub async fn set_download_directory(&self, directory: &std::path::Path) -> crate::Result<()> {
        let tracker = &self.inner.downloads;
        tracker.attach(self.connection());
        match self.kind() {
            ConnectionKind::Launched => {
                tracker.set_destination(DownloadDestination::Directory {
                    path: directory.to_path_buf(),
                });
                Ok(())
            }
            ConnectionKind::AttachedApproval => Err(BrowserError::Unsupported {
                message: APPROVAL_DOWNLOAD_FOLDER.to_owned(),
            }),
            ConnectionKind::AttachedPort | ConnectionKind::Direct => {
                let params = json!({
                    "behavior": "allow",
                    "downloadPath": utf8_path(directory)?,
                    "eventsEnabled": true,
                });
                self.root()
                    .send("Browser.setDownloadBehavior", params)
                    .await?;
                tracker.with_state(|state| state.allow_directory = Some(directory.to_path_buf()));
                Ok(())
            }
        }
    }
}

async fn closed(connection: Option<&Connection>) {
    match connection {
        Some(connection) => connection.closed().await,
        None => std::future::pending().await,
    }
}

fn claim_for_page(state: &mut TrackerState, event: &Event) -> bool {
    let Some(guid) = event.params["guid"]
        .as_str()
        .filter(|guid| !guid.is_empty())
    else {
        return false;
    };
    if let Some(tracked) = state.tracked_mut(guid) {
        let claimed = !tracked.visible;
        tracked.visible = true;
        return claimed && tracked.finished.is_some();
    }
    if !state.page_claims.iter().any(|claim| claim == guid) {
        state.page_claims.push_back(guid.to_owned());
        while state.page_claims.len() > MAX_PAGE_CLAIMS {
            state.page_claims.pop_front();
        }
    }
    false
}

/// Chrome sends `Page.downloadWillBegin` only to the page sessions of the frame tree that started
/// the download, so it proves a root `Browser.downloadWillBegin` belongs to one of our pages.
/// Popups stay unattached until selected; their root `targetCreated` carries the `openerId`.
fn track_page_ownership(state: &mut TrackerState, event: &Event, root: bool) -> bool {
    match &*event.method {
        "Page.downloadWillBegin" if !root => claim_for_page(state, event),
        "Target.attachedToTarget" => {
            track_page_target(state, event);
            false
        }
        "Target.targetCreated" | "Target.targetInfoChanged" if root => {
            let info = &event.params["targetInfo"];
            if let (Some(target), Some(opener)) =
                (info["targetId"].as_str(), info["openerId"].as_str())
            {
                state.record_opener(target, opener);
            }
            false
        }
        "Target.detachedFromTarget" => {
            if let Some(session) = event.params["sessionId"].as_str() {
                state.page_targets.remove(&SessionId::from(session));
            }
            false
        }
        _ => false,
    }
}

fn track_page_target(state: &mut TrackerState, event: &Event) {
    let Some(attached) = event.decode::<AttachedToTarget>() else {
        return;
    };
    if matches!(
        attached.target_info.type_,
        TargetType::Page | TargetType::Iframe
    ) {
        state
            .page_targets
            .insert(attached.session_id, attached.target_info.target_id);
    }
}

fn relocation_for(
    tracked: &Tracked,
    reported: Option<PathBuf>,
    staging: Option<&Path>,
) -> Result<Relocation, MoveFailure> {
    let name = file_name(&tracked.record.suggested_filename);
    let source = reported.or_else(|| staging.map(|staging| staging.join(&tracked.record.guid)));
    match source {
        Some(source) => Ok(Relocation {
            guid: tracked.record.guid.clone(),
            source,
            name,
            destination: tracked.destination.clone(),
        }),
        None => Err(MoveFailure {
            target: PathBuf::from(&name),
            error: BrowserError::Io {
                context: format!("Moving the download {name}"),
                message: "Chrome reported no file path and the browser has no staging directory"
                    .to_owned(),
            },
        }),
    }
}

fn utf8_path(path: &Path) -> crate::Result<&str> {
    path.to_str().ok_or_else(|| BrowserError::InvalidArgument {
        message: format!(
            "the download folder {} is not valid UTF-8; Chrome only accepts UTF-8 paths",
            path.display()
        ),
    })
}

fn last_component(name: &str) -> &str {
    name.rsplit(['/', '\\']).next().unwrap_or_default()
}

fn file_name(suggested: &str) -> String {
    let cleaned: String = last_component(suggested)
        .chars()
        .map(|character| {
            if character.is_control() || RESERVED_NAME_CHARS.contains(&character) {
                '_'
            } else {
                character
            }
        })
        .collect();
    match cleaned.as_str() {
        "" | "." | ".." => FALLBACK_NAME.to_owned(),
        _ => cleaned,
    }
}

fn extension_start(name: &str) -> Option<usize> {
    let user_script = name.len().saturating_sub(USER_SCRIPT_EXTENSION.len());
    let is_user_script = name
        .get(user_script..)
        .is_some_and(|tail| tail.eq_ignore_ascii_case(USER_SCRIPT_EXTENSION));
    if user_script > 0 && is_user_script {
        return Some(user_script);
    }
    let last = name.rfind('.').filter(|&last| last > 0)?;
    let final_extension = name[last + 1..].to_ascii_lowercase();
    if !DOUBLE_EXTENSION_SUFFIXES.contains(&final_extension.as_str()) {
        return Some(last);
    }
    let penultimate = name[..last].rfind('.').filter(|&dot| dot > 0);
    match penultimate {
        Some(dot) if (1..=INNER_EXTENSION_MAX_LEN).contains(&(last - dot - 1)) => Some(dot),
        _ => Some(last),
    }
}

fn numbered_name(name: &str, number: u32) -> String {
    let (stem, extension) = name.split_at(extension_start(name).unwrap_or(name.len()));
    format!("{stem} ({number}){extension}")
}

fn os_download_dir() -> Option<PathBuf> {
    dirs::download_dir().or_else(|| dirs::home_dir().map(|home| home.join("Downloads")))
}

async fn move_download(
    source: &Path,
    destination: &DownloadDestination,
    name: &str,
) -> Result<PathBuf, MoveFailure> {
    match destination {
        DownloadDestination::Directory { path } => place(source, path, name, false).await,
        DownloadDestination::OsDownloads => match os_download_dir() {
            Some(directory) => place(source, &directory, name, true).await,
            None => Err(MoveFailure {
                target: PathBuf::from(name),
                error: BrowserError::Io {
                    context: format!("Moving the download {name}"),
                    message: "this system has no download folder and no home directory".to_owned(),
                },
            }),
        },
    }
}

async fn place(
    source: &Path,
    directory: &Path,
    name: &str,
    keep_existing: bool,
) -> Result<PathBuf, MoveFailure> {
    let failed = |target: PathBuf, context: String, error: std::io::Error| MoveFailure {
        target,
        error: BrowserError::io(context, &error),
    };
    tokio::fs::create_dir_all(directory)
        .await
        .map_err(|error| {
            failed(
                directory.join(name),
                format!("Creating the download folder {}", directory.display()),
                error,
            )
        })?;
    let target = if keep_existing {
        reserve_free_name(directory, name).await?
    } else {
        directory.join(name)
    };
    if let Err(error) = relocate(source, &target).await {
        if keep_existing {
            let _ = tokio::fs::remove_file(&target).await;
        }
        let context = format!(
            "Moving the download {} to {}",
            source.display(),
            target.display()
        );
        return Err(failed(target, context, error));
    }
    Ok(target)
}

async fn reserve_free_name(directory: &Path, name: &str) -> Result<PathBuf, MoveFailure> {
    for number in 0..=MAX_NAME_SUFFIX {
        let candidate = match number {
            0 => directory.join(name),
            _ => directory.join(numbered_name(name, number)),
        };
        let created = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
            .await;
        match created {
            Ok(_) => return Ok(candidate),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(MoveFailure {
                    error: BrowserError::io(
                        format!("Reserving the download name {}", candidate.display()),
                        &error,
                    ),
                    target: candidate,
                });
            }
        }
    }
    Err(MoveFailure {
        target: directory.join(name),
        error: BrowserError::Io {
            context: format!("Choosing a free name for the download {name}"),
            message: format!(
                "{} already holds {name} and {MAX_NAME_SUFFIX} numbered copies",
                directory.display()
            ),
        },
    })
}

async fn relocate(source: &Path, target: &Path) -> std::io::Result<()> {
    let Err(error) = tokio::fs::rename(source, target).await else {
        return Ok(());
    };
    tracing::debug!(%error, "renaming a finished download failed; copying it instead");
    tokio::fs::copy(source, target).await?;
    if let Err(error) = tokio::fs::remove_file(source).await {
        tracing::debug!(%error, "the staged copy of a moved download could not be removed");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::ConnectionOptions;
    use crate::transport::memory::{InMemoryControl, InMemoryTransport};
    use serde_json::Value;
    use std::time::Duration;

    const WAIT: Duration = Duration::from_secs(5);
    const QUIET: Duration = Duration::from_millis(150);

    struct Scripted {
        tracker: DownloadTracker,
        control: InMemoryControl,
        connection: Connection,
    }

    fn scripted(kind: ConnectionKind, staging: Option<PathBuf>) -> Scripted {
        let (transport, control) = InMemoryTransport::new();
        let connection = Connection::start(Box::new(transport), ConnectionOptions::default());
        let tracker = DownloadTracker::new(kind, staging);
        connection.add_hook(Arc::new(tracker.clone()));
        Scripted {
            tracker,
            control,
            connection,
        }
    }

    fn domain(session: Option<&str>) -> &str {
        if session.is_some() { "Page" } else { "Browser" }
    }

    impl Scripted {
        async fn emit(&self, method: &str, session: Option<&str>, params: Value) {
            let cursor = self.connection.events().cursor();
            self.control.emit(method, session, params);
            let deadline = tokio::time::Instant::now() + WAIT;
            let seen = self
                .connection
                .events()
                .wait_for(cursor, deadline, |event| &*event.method == method)
                .await
                .expect("the event log closed");
            assert!(seen.is_some(), "the reader never processed {method}");
        }

        async fn begin(&self, session: Option<&str>, guid: &str, frame: &str, name: &str) {
            let params = json!({"frameId": frame, "guid": guid, "url": "http://127.0.0.1/file", "suggestedFilename": name});
            let method = format!("{}.downloadWillBegin", domain(session));
            self.emit(&method, session, params).await;
        }

        async fn finish(
            &self,
            session: Option<&str>,
            guid: &str,
            state: &str,
            path: Option<&Path>,
        ) {
            let mut params =
                json!({"guid": guid, "totalBytes": 3, "receivedBytes": 3, "state": state});
            if let Some(path) = path {
                params["filePath"] = json!(path.to_str().unwrap());
            }
            let method = format!("{}.downloadProgress", domain(session));
            self.emit(&method, session, params).await;
        }

        async fn wait(
            &self,
            since: u64,
            window: Duration,
            guid: &str,
        ) -> crate::Result<Option<DownloadRecord>> {
            let deadline = tokio::time::Instant::now() + window;
            self.tracker
                .wait_finished(since, false, deadline, |record| record.guid == guid)
                .await
        }
    }

    #[test]
    fn file_names_are_reduced_to_one_safe_component() {
        assert_eq!(file_name("report.txt"), "report.txt");
        assert_eq!(file_name("../../etc/passwd"), "passwd");
        assert_eq!(file_name("C:\\Users\\me\\a.txt"), "a.txt");
        assert_eq!(
            file_name("a:b*c?d\"e<f>g|h\u{7}.txt"),
            "a_b_c_d_e_f_g_h_.txt"
        );
        assert_eq!(file_name(""), "download");
        assert_eq!(file_name("dir/"), "download");
        assert_eq!(file_name(".."), "download");
    }

    #[test]
    fn numbered_names_go_before_the_extension() {
        assert_eq!(numbered_name("report.txt", 1), "report (1).txt");
        assert_eq!(numbered_name("archive.tar.gz", 2), "archive (2).tar.gz");
        assert_eq!(numbered_name("data.JSON.bz2", 1), "data (1).JSON.bz2");
        assert_eq!(numbered_name("photo.backup.gz", 1), "photo.backup (1).gz");
        assert_eq!(numbered_name("tool.user.js", 1), "tool (1).user.js");
        assert_eq!(numbered_name(".bashrc", 1), ".bashrc (1)");
        assert_eq!(numbered_name("README", 3), "README (3)");
        assert_eq!(numbered_name("v1.2.zip", 1), "v1.2 (1).zip");
        assert_eq!(numbered_name("üüüüa", 1), "üüüüa (1)");
        assert_eq!(numbered_name("日本語.user.js", 1), "日本語 (1).user.js");
        assert_eq!(numbered_name("résumé.pdf", 2), "résumé (2).pdf");
    }

    #[tokio::test]
    async fn placing_into_a_directory_replaces_while_the_os_folder_keeps_existing_files() {
        let staging = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        std::fs::write(target.path().join("report.txt"), "old").unwrap();
        let first = staging.path().join("g1");
        std::fs::write(&first, "new").unwrap();
        let replaced = place(&first, target.path(), "report.txt", false)
            .await
            .unwrap();
        assert_eq!(replaced, target.path().join("report.txt"));
        assert_eq!(std::fs::read_to_string(&replaced).unwrap(), "new");
        assert!(!first.exists());
        for (guid, expected) in [("g2", "report (1).txt"), ("g3", "report (2).txt")] {
            let source = staging.path().join(guid);
            std::fs::write(&source, guid).unwrap();
            let kept = place(&source, target.path(), "report.txt", true)
                .await
                .unwrap();
            assert_eq!(kept, target.path().join(expected));
            assert_eq!(std::fs::read_to_string(&kept).unwrap(), guid);
        }
        let original = std::fs::read_to_string(target.path().join("report.txt")).unwrap();
        assert_eq!(original, "new");
    }

    #[tokio::test]
    async fn a_failed_move_is_reported_to_the_matching_waiter() {
        let staging = tempfile::tempdir().unwrap();
        let blocker = tempfile::NamedTempFile::new().unwrap();
        let session = scripted(ConnectionKind::Launched, Some(staging.path().to_path_buf()));
        let unwritable = blocker.path().join("sub");
        let path = unwritable.clone();
        session
            .tracker
            .set_destination(DownloadDestination::Directory { path });
        std::fs::write(staging.path().join("g1"), "x").unwrap();
        let since = session.tracker.cursor();
        session.begin(None, "g1", "T1", "a.txt").await;
        session.finish(None, "g1", "completed", None).await;
        let deadline = tokio::time::Instant::now() + WAIT;
        let expected = unwritable.join("a.txt");
        let error = session
            .tracker
            .wait_finished(since, false, deadline, |record| {
                record.final_path.as_deref() == Some(expected.as_path())
            })
            .await
            .unwrap_err();
        assert!(matches!(error, BrowserError::Io { .. }), "{error:?}");
        assert!(error.to_string().contains("download folder"), "{error}");
        assert!(
            staging.path().join("g1").exists(),
            "the staged file stays in place"
        );
    }

    #[tokio::test]
    async fn port_mode_claims_downloads_by_page_events_in_either_order() {
        let folder = tempfile::tempdir().unwrap();
        let session = scripted(ConnectionKind::AttachedPort, None);
        let folder_path = folder.path().to_path_buf();
        session
            .tracker
            .with_state(|state| state.allow_directory = Some(folder_path));
        let since = session.tracker.cursor();
        session.begin(Some("S1"), "early", "F2", "early.txt").await;
        session.begin(None, "early", "F2", "early.txt").await;
        session.begin(None, "late", "F3", "late.txt").await;
        session.finish(None, "late", "completed", None).await;
        assert!(session.wait(since, QUIET, "late").await.unwrap().is_none());
        session.begin(Some("S1"), "late", "F3", "late.txt").await;
        let late = session.wait(since, WAIT, "late").await.unwrap().unwrap();
        assert_eq!(late.final_path, Some(folder.path().join("late.txt")));
        session.finish(None, "early", "canceled", None).await;
        let early = session.wait(since, WAIT, "early").await.unwrap().unwrap();
        assert_eq!(early.state, DownloadState::Canceled);
        assert_eq!(early.final_path, None);
    }

    #[tokio::test]
    async fn port_mode_forgets_page_targets_after_detach() {
        let session = scripted(ConnectionKind::Direct, None);
        let page = json!({"sessionId": "S1", "targetInfo": {"targetId": "T1", "type": "page"}, "waitingForDebugger": false});
        session.emit("Target.attachedToTarget", None, page).await;
        let since = session.tracker.cursor();
        let reported = Path::new("/tmp/a.txt");
        session.begin(None, "a", "T1", "a.txt").await;
        session.finish(None, "a", "completed", Some(reported)).await;
        let found = session.wait(since, WAIT, "a").await.unwrap().unwrap();
        assert_eq!(found.final_path.as_deref(), Some(reported));
        let detached = json!({"sessionId": "S1", "targetId": "T1"});
        session
            .emit("Target.detachedFromTarget", None, detached)
            .await;
        let since = session.tracker.cursor();
        session.begin(None, "b", "T1", "b.txt").await;
        session.finish(None, "b", "completed", None).await;
        assert!(session.wait(since, QUIET, "b").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn port_mode_counts_downloads_from_popups_our_pages_opened() {
        let session = scripted(ConnectionKind::AttachedPort, None);
        let page = json!({"sessionId": "S1", "targetInfo": {"targetId": "T1", "type": "page"}, "waitingForDebugger": false});
        session.emit("Target.attachedToTarget", None, page).await;
        for (target, opener) in [("P2", "T1"), ("P3", "P2"), ("X1", "USER")] {
            let info =
                json!({"targetInfo": {"targetId": target, "type": "page", "openerId": opener}});
            session.emit("Target.targetCreated", None, info).await;
        }
        let since = session.tracker.cursor();
        let reported = Path::new("/tmp/popup.txt");
        for (guid, frame) in [("popup", "P3"), ("user", "X1")] {
            session.begin(None, guid, frame, "popup.txt").await;
            session
                .finish(None, guid, "completed", Some(reported))
                .await;
        }
        let popup = session.wait(since, WAIT, "popup").await.unwrap().unwrap();
        assert_eq!(popup.final_path.as_deref(), Some(reported));
        assert!(session.wait(since, QUIET, "user").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn a_repeated_completion_moves_the_download_once() {
        let staging = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        let session = scripted(ConnectionKind::Launched, Some(staging.path().to_path_buf()));
        let path = folder.path().to_path_buf();
        session
            .tracker
            .set_destination(DownloadDestination::Directory { path });
        std::fs::write(staging.path().join("g1"), "x").unwrap();
        let since = session.tracker.cursor();
        session.begin(None, "g1", "T1", "a.txt").await;
        session.finish(None, "g1", "completed", None).await;
        session.finish(None, "g1", "completed", None).await;
        let record = session.wait(since, WAIT, "g1").await.unwrap().unwrap();
        assert_eq!(record.final_path, Some(folder.path().join("a.txt")));
        let later = tokio::time::Instant::now() + WAIT;
        assert!(session.tracker.moves_finished(later).await);
        let moved_cleanly = session.tracker.with_state(|state| {
            state
                .downloads
                .iter()
                .all(|tracked| tracked.failure.is_none())
        });
        assert!(moved_cleanly);
    }

    #[tokio::test]
    async fn moves_finished_waits_for_in_flight_moves() {
        let tracker = DownloadTracker::new(ConnectionKind::Launched, None);
        let guard = MoveGuard::start(&tracker.inner.moves);
        let soon = tokio::time::Instant::now() + Duration::from_millis(50);
        assert!(!tracker.moves_finished(soon).await);
        drop(guard);
        let later = tokio::time::Instant::now() + WAIT;
        assert!(tracker.moves_finished(later).await);
    }

    #[tokio::test]
    async fn a_waiter_sees_the_connection_close() {
        let session = scripted(ConnectionKind::Launched, None);
        let page = json!({"targetInfo": {"targetId": "T1", "type": "page"}});
        session.emit("Target.targetCreated", None, page).await;
        let waiter = tokio::spawn({
            let tracker = session.tracker.clone();
            async move {
                let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
                tracker.wait_finished(0, true, deadline, |_| true).await
            }
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        session.control.close("the browser went away");
        let outcome = tokio::time::timeout(WAIT, waiter).await.unwrap().unwrap();
        assert!(
            matches!(outcome, Err(BrowserError::Disconnected { ref reason }) if reason.contains("went away")),
            "{outcome:?}"
        );
    }

    #[tokio::test]
    async fn an_attached_tracker_sees_a_close_before_any_event() {
        let session = scripted(ConnectionKind::AttachedApproval, None);
        session.tracker.attach(&session.connection);
        let waiter = tokio::spawn({
            let tracker = session.tracker.clone();
            async move {
                let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
                tracker.wait_finished(0, true, deadline, |_| true).await
            }
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        session.control.close("the browser went away");
        let outcome = tokio::time::timeout(WAIT, waiter).await.unwrap().unwrap();
        assert!(
            matches!(outcome, Err(BrowserError::Disconnected { ref reason }) if reason.contains("went away")),
            "{outcome:?}"
        );
    }
}
