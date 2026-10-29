//! Local object operations anchored to directory handles.
//!
//! Each descendant is opened without following symlinks. Once opened, parent
//! directories and files stay pinned for the operation, including publication
//! and cleanup of streaming uploads.

use async_trait::async_trait;
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, OpenOptions};
use futures::{
    StreamExt,
    stream::{self, BoxStream},
};
use object_store::{
    Attributes, CopyMode, CopyOptions, GetOptions, GetResult, GetResultPayload, ListResult,
    MultipartUpload, ObjectMeta, ObjectStore, PutMode, PutMultipartOptions, PutOptions, PutPayload,
    PutResult, RenameOptions, RenameTargetMode, Result, UploadPart, path::Path,
};
use std::{
    collections::BTreeSet,
    fs::{File, Metadata},
    io::{self, Seek, SeekFrom, Write},
    path::{Component, Path as FsPath, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

static NEXT_STAGE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug)]
pub(super) struct SecureLocalStore {
    root: Arc<PinnedDir>,
    // Preserve the existing local compare-and-swap contract for calls through
    // this store instance. External filesystem writers do not hold this lock.
    mutations: Arc<Mutex<()>>,
}

/// Windows cap-std mutations resolve a path from a directory handle. Keep
/// no-delete-sharing handles for its complete ancestry until the operation ends.
#[derive(Clone, Debug)]
struct PinnedDir {
    dir: Arc<Dir>,
    #[cfg(windows)]
    ancestors: Vec<Arc<Dir>>,
}
impl std::ops::Deref for PinnedDir {
    type Target = Dir;
    fn deref(&self) -> &Self::Target {
        &self.dir
    }
}
impl PinnedDir {
    fn root(prefix: &FsPath) -> io::Result<Self> {
        #[cfg(not(windows))]
        {
            Ok(Self {
                dir: Arc::new(Dir::open_ambient_dir(prefix, cap_std::ambient_authority())?),
            })
        }
        #[cfg(windows)]
        {
            // Only the trusted constructor prefix is resolved ambiently. Pin
            // each component of the resolved root before exposing the store.
            let canonical = std::fs::canonicalize(prefix)?;
            let mut components = canonical.components();
            let Some(Component::Prefix(volume)) = components.next() else {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "Local root must be absolute",
                ));
            };
            if !matches!(components.next(), Some(Component::RootDir)) {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "Local root must be absolute",
                ));
            }
            let mut anchor = PathBuf::from(volume.as_os_str());
            anchor.push("\\");
            let mut current = Self {
                dir: Arc::new(Dir::open_ambient_dir(anchor, cap_std::ambient_authority())?),
                ancestors: Vec::new(),
            };
            for component in components {
                let Component::Normal(name) = component else {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "Invalid local root component",
                    ));
                };
                current = current.child(name)?;
            }
            Ok(current)
        }
    }

    fn child(&self, name: impl AsRef<FsPath>) -> io::Result<Self> {
        #[cfg(not(windows))]
        {
            Ok(Self {
                dir: Arc::new(self.dir.open_dir_nofollow(name)?),
            })
        }
        #[cfg(windows)]
        {
            // cap-std's Windows directory options exclude FILE_SHARE_DELETE.
            let dir = self.dir.open_dir_nofollow(name)?;
            let mut ancestors = self.ancestors.clone();
            ancestors.push(self.dir.clone());
            Ok(Self {
                dir: Arc::new(dir),
                ancestors,
            })
        }
    }
}

impl std::fmt::Display for SecureLocalStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecureLocalStore")
    }
}

fn error(location: &Path, source: io::Error) -> object_store::Error {
    match source.kind() {
        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory => object_store::Error::NotFound {
            path: location.to_string(),
            source: Box::new(source),
        },
        io::ErrorKind::AlreadyExists => object_store::Error::AlreadyExists {
            path: location.to_string(),
            source: Box::new(source),
        },
        io::ErrorKind::PermissionDenied => object_store::Error::PermissionDenied {
            path: location.to_string(),
            source: Box::new(source),
        },
        _ => object_store::Error::Generic {
            store: "SecureLocalStore",
            source: Box::new(source),
        },
    }
}

fn denied(location: &Path) -> object_store::Error {
    error(
        location,
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Local storage does not follow symlinks or special filesystem objects",
        ),
    )
}

fn generic(message: impl Into<String>) -> object_store::Error {
    object_store::Error::Generic {
        store: "SecureLocalStore",
        source: message.into().into(),
    }
}

fn rename_create(from_dir: &Dir, from: &str, to_dir: &Dir, to: &str) -> io::Result<()> {
    #[cfg(any(target_os = "linux", target_os = "android", target_vendor = "apple"))]
    {
        rustix::fs::renameat_with(
            from_dir,
            from,
            to_dir,
            to,
            rustix::fs::RenameFlags::NOREPLACE,
        )
        .map_err(io::Error::from)
    }
    #[cfg(not(any(target_os = "linux", target_os = "android", target_vendor = "apple")))]
    {
        from_dir.hard_link(from, to_dir, to)?;
        from_dir.remove_file(from)
    }
}

#[cfg(any(windows, test))]
fn windows_alias(name: &str) -> bool {
    if name.ends_with(['.', ' ']) || name.contains(':') {
        return true;
    }
    let base = name
        .split('.')
        .next()
        .unwrap_or(name)
        .trim_end_matches(' ')
        .to_ascii_uppercase();
    matches!(
        base.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || ["COM", "LPT"].iter().any(|prefix| {
        base.strip_prefix(*prefix).is_some_and(|suffix| {
            matches!(
                suffix,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
    })
}

fn segments(location: &Path) -> Result<Vec<&str>> {
    location
        .as_ref()
        .split('/')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut components = FsPath::new(part).components();
            if !matches!(components.next(), Some(Component::Normal(_)))
                || components.next().is_some()
                || part.contains('\\')
            {
                return Err(denied(location));
            }
            #[cfg(windows)]
            if windows_alias(part) {
                return Err(denied(location));
            }
            Ok(part)
        })
        .collect()
}

fn symlink_check(parent: &Dir, name: &str, location: &Path, missing_ok: bool) -> Result<()> {
    match parent.symlink_metadata(name) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(denied(location)),
        Ok(_) => Ok(()),
        Err(source) if missing_ok && source.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(error(location, source)),
    }
}

fn open_child(parent: &PinnedDir, name: &str, location: &Path, create: bool) -> Result<PinnedDir> {
    symlink_check(parent, name, location, create)?;
    match parent.child(name) {
        Ok(dir) => Ok(dir),
        Err(source) if create && source.kind() == io::ErrorKind::NotFound => {
            match parent.create_dir(name) {
                Ok(()) => {}
                Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {}
                Err(source) => return Err(error(location, source)),
            }
            // A concurrent replacement is resolved relative to the pinned parent
            // with no-follow semantics, never by reopening an ambient pathname.
            parent.child(name).map_err(|source| error(location, source))
        }
        Err(source) => Err(error(location, source)),
    }
}

fn open_regular(parent: &Dir, name: &str, location: &Path) -> Result<File> {
    symlink_check(parent, name, location, false)?;
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt;
        // A file can become a FIFO between the metadata check and open.
        // Opening nonblocking lets us reject its descriptor without waiting.
        options.custom_flags(rustix::fs::OFlags::NONBLOCK.bits() as i32);
    }
    let file = parent
        .open_with(name, &options)
        .map_err(|source| error(location, source))?
        .into_std();
    if !file
        .metadata()
        .map_err(|source| error(location, source))?
        .is_file()
    {
        return Err(denied(location));
    }
    Ok(file)
}

fn object_meta(location: Path, metadata: &Metadata) -> ObjectMeta {
    let modified = metadata.modified().unwrap_or(UNIX_EPOCH);
    let micros = modified
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros();
    #[cfg(unix)]
    let inode = std::os::unix::fs::MetadataExt::ino(metadata);
    #[cfg(not(unix))]
    let inode = 0_u64;
    ObjectMeta {
        location,
        last_modified: modified.into(),
        size: metadata.len(),
        e_tag: Some(format!("{inode:x}-{micros:x}-{:x}", metadata.len())),
        version: None,
    }
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    if tokio::runtime::Handle::try_current().is_ok() {
        tokio::task::spawn_blocking(work)
            .await
            .map_err(|source| generic(format!("Local storage task failed: {source}")))?
    } else {
        work()
    }
}

impl SecureLocalStore {
    pub(super) fn new(prefix: &FsPath) -> Result<Self> {
        let root = PinnedDir::root(prefix).map_err(|source| error(&Path::default(), source))?;
        Ok(Self {
            root: Arc::new(root),
            mutations: Arc::new(Mutex::new(())),
        })
    }

    fn directory(&self, location: &Path, create: bool) -> Result<PinnedDir> {
        let mut current = self.root.as_ref().clone();
        for segment in segments(location)? {
            current = open_child(&current, segment, location, create)?;
        }
        Ok(current)
    }

    fn parent(&self, location: &Path, create: bool) -> Result<(PinnedDir, String)> {
        let mut parts = segments(location)?;
        let name = parts.pop().ok_or_else(|| denied(location))?.to_owned();
        let mut current = self.root.as_ref().clone();
        for segment in parts {
            current = open_child(&current, segment, location, create)?;
        }
        Ok((current, name))
    }

    pub(super) fn metadata(&self, location: &Path) -> Result<Option<Metadata>> {
        let result = (|| {
            if location.as_ref().is_empty() {
                return self
                    .root
                    .try_clone()
                    .and_then(|dir| dir.into_std_file().metadata())
                    .map_err(|source| error(location, source));
            }
            let (parent, name) = self.parent(location, false)?;
            let metadata = parent
                .symlink_metadata(&name)
                .map_err(|source| error(location, source))?;
            if metadata.file_type().is_symlink() {
                return Err(denied(location));
            }
            if metadata.is_dir() {
                return parent
                    .open_dir_nofollow(&name)
                    .and_then(|dir| dir.into_std_file().metadata())
                    .map_err(|source| error(location, source));
            }
            open_regular(&parent, &name, location)?
                .metadata()
                .map_err(|source| error(location, source))
        })();
        match result {
            Ok(metadata) => Ok(Some(metadata)),
            Err(object_store::Error::NotFound { .. }) => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn read(&self, location: &Path) -> Result<File> {
        let (parent, name) = self.parent(location, false)?;
        open_regular(&parent, &name, location)
    }

    fn delete(&self, location: &Path) -> Result<()> {
        if location.filename().is_some_and(staged_name) {
            return Err(denied(location));
        }
        let _guard = self
            .mutations
            .lock()
            .map_err(|_| generic("Local mutation lock poisoned"))?;
        let (parent, name) = self.parent(location, false)?;
        symlink_check(&parent, &name, location, false)?;
        // unlink never follows the final component if another process swaps it.
        parent
            .remove_file(&name)
            .map_err(|source| error(location, source))
    }

    fn stage(&self, location: &Path) -> Result<Stage> {
        let (parent, destination) = self.parent(location, true)?;
        symlink_check(&parent, &destination, location, true)?;
        Stage::new(parent, destination, location.clone())
    }

    fn publish(&self, stage: &mut Stage, mode: &PutMode) -> Result<PutResult> {
        symlink_check(&stage.parent, &stage.destination, &stage.location, true)?;
        if let PutMode::Update(expected) = mode {
            let current = open_regular(&stage.parent, &stage.destination, &stage.location)?;
            let metadata = object_meta(
                stage.location.clone(),
                &current
                    .metadata()
                    .map_err(|source| error(&stage.location, source))?,
            );
            if (expected.e_tag.is_some() && expected.e_tag != metadata.e_tag)
                || (expected.version.is_some() && expected.version != metadata.version)
            {
                return Err(object_store::Error::Precondition {
                    path: stage.location.to_string(),
                    source: "Local object did not match update precondition".into(),
                });
            }
        }
        let file = stage
            .file
            .as_mut()
            .ok_or_else(|| generic("Upload has already ended"))?;
        file.flush()
            .map_err(|source| error(&stage.location, source))?;
        let metadata = file
            .metadata()
            .map_err(|source| error(&stage.location, source))?;
        match mode {
            PutMode::Create => {
                // No-replace publication is atomic, including on Android where
                // hard links are unavailable under common SELinux policies.
                rename_create(
                    &stage.parent,
                    &stage.name,
                    &stage.parent,
                    &stage.destination,
                )
                .map_err(|source| error(&stage.location, source))?;
            }
            PutMode::Overwrite | PutMode::Update(_) => stage
                .parent
                .rename(&stage.name, &stage.parent, &stage.destination)
                .map_err(|source| error(&stage.location, source))?,
        }
        stage.published = true;
        stage.file.take();
        Ok(PutResult {
            e_tag: object_meta(stage.location.clone(), &metadata).e_tag,
            version: None,
        })
    }
}

#[derive(Debug)]
struct Stage {
    parent: PinnedDir,
    destination: String,
    location: Path,
    name: String,
    file: Option<File>,
    published: bool,
}

impl Stage {
    fn new(parent: PinnedDir, destination: String, location: Path) -> Result<Self> {
        for _ in 0..16 {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let count = NEXT_STAGE.fetch_add(1, Ordering::Relaxed);
            let name = format!(
                ".flow-like-upload#{}{:020}{count:020}",
                std::process::id(),
                nonce
            );
            let mut options = OpenOptions::new();
            options
                .read(true)
                .write(true)
                .create_new(true)
                .follow(FollowSymlinks::No);
            match parent.open_with(&name, &options) {
                Ok(file) => {
                    return Ok(Self {
                        parent,
                        destination,
                        location,
                        name,
                        file: Some(file.into_std()),
                        published: false,
                    });
                }
                Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {}
                Err(source) => return Err(error(&location, source)),
            }
        }
        Err(generic("Could not allocate a local upload staging file"))
    }
}
impl Drop for Stage {
    fn drop(&mut self) {
        self.file.take();
        if !self.published {
            let _ = self.parent.remove_file(&self.name);
        }
    }
}

#[derive(Debug)]
struct UploadState {
    stage: Option<Stage>,
    completed_parts: u64,
    failed: bool,
}

#[derive(Debug)]
struct Upload {
    store: SecureLocalStore,
    state: Arc<Mutex<UploadState>>,
    next_offset: u64,
    parts: u64,
}

#[async_trait]
impl MultipartUpload for Upload {
    fn put_part(&mut self, data: PutPayload) -> UploadPart {
        let state = self.state.clone();
        let offset = self.next_offset;
        let size = data.content_length() as u64;
        let Some(next) = offset.checked_add(size) else {
            return Box::pin(async { Err(generic("Multipart offset overflow")) });
        };
        self.next_offset = next;
        self.parts += 1;
        Box::pin(async move {
            blocking(move || {
                let mut state = state.lock().map_err(|_| generic("Upload lock poisoned"))?;
                let result = (|| {
                    let stage = state
                        .stage
                        .as_mut()
                        .ok_or_else(|| generic("Upload has already ended"))?;
                    let file = stage
                        .file
                        .as_mut()
                        .ok_or_else(|| generic("Upload has already ended"))?;
                    file.seek(SeekFrom::Start(offset))
                        .map_err(|source| error(&stage.location, source))?;
                    for chunk in data {
                        file.write_all(&chunk)
                            .map_err(|source| error(&stage.location, source))?;
                    }
                    Ok(())
                })();
                if result.is_ok() {
                    state.completed_parts += 1;
                } else {
                    state.failed = true;
                }
                result
            })
            .await
        })
    }

    async fn complete(&mut self) -> Result<PutResult> {
        let store = self.store.clone();
        let state = self.state.clone();
        let expected = self.parts;
        blocking(move || {
            let _guard = store
                .mutations
                .lock()
                .map_err(|_| generic("Local mutation lock poisoned"))?;
            let mut state = state.lock().map_err(|_| generic("Upload lock poisoned"))?;
            if state.failed || state.completed_parts != expected {
                return Err(generic("Multipart upload has incomplete or failed parts"));
            }
            let stage = state
                .stage
                .as_mut()
                .ok_or_else(|| generic("Upload has already ended"))?;
            let result = store.publish(stage, &PutMode::Overwrite)?;
            state.stage.take();
            Ok(result)
        })
        .await
    }

    async fn abort(&mut self) -> Result<()> {
        let state = self.state.clone();
        blocking(move || {
            state
                .lock()
                .map_err(|_| generic("Upload lock poisoned"))?
                .stage
                .take();
            Ok(())
        })
        .await
    }
}

impl Drop for Upload {
    fn drop(&mut self) {
        // Dropping the upload invalidates outstanding part futures as well.
        if let Ok(mut state) = self.state.lock() {
            state.stage.take();
        }
    }
}

fn staged_name(name: &str) -> bool {
    name.split_once('#').is_some_and(|(_, suffix)| {
        !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
    })
}

struct WalkFrame {
    parent: Arc<PinnedDir>,
    entries: cap_std::fs::ReadDir,
    prefix: Path,
}

impl SecureLocalStore {
    fn walk(
        &self,
        prefix: &Path,
        recursive: bool,
        mut emit: impl FnMut(Result<WalkEntry>) -> bool,
    ) {
        let start = match self.directory(prefix, false) {
            Ok(dir) => dir,
            Err(object_store::Error::NotFound { .. }) => return,
            Err(error) => {
                emit(Err(error));
                return;
            }
        };
        let entries = match start.entries() {
            Ok(entries) => entries,
            Err(source) => {
                emit(Err(error(prefix, source)));
                return;
            }
        };
        let mut stack = vec![WalkFrame {
            parent: Arc::new(start),
            entries,
            prefix: prefix.clone(),
        }];
        while let Some(frame) = stack.last_mut() {
            let entry = match frame.entries.next() {
                Some(Ok(entry)) => entry,
                Some(Err(source)) => {
                    emit(Err(error(&frame.prefix, source)));
                    return;
                }
                None => {
                    stack.pop();
                    continue;
                }
            };
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            let location = match Path::parse(format!(
                "{}{name}",
                if frame.prefix.as_ref().is_empty() {
                    String::new()
                } else {
                    format!("{}/", frame.prefix)
                }
            )) {
                Ok(location) => location,
                Err(source) => {
                    emit(Err(source.into()));
                    return;
                }
            };
            if segments(&location).is_err() {
                continue;
            }
            let metadata = match frame.parent.symlink_metadata(&name) {
                Ok(metadata) => metadata,
                Err(source) if source.kind() == io::ErrorKind::NotFound => continue,
                Err(source) => {
                    emit(Err(error(&location, source)));
                    return;
                }
            };
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                let dir = match open_child(&frame.parent, &name, &location, false) {
                    Ok(dir) => dir,
                    Err(object_store::Error::NotFound { .. }) => continue,
                    Err(error) => {
                        emit(Err(error));
                        return;
                    }
                };
                if recursive {
                    if stack.len() >= 256 {
                        emit(Err(generic(
                            "Local listing exceeds the maximum directory depth",
                        )));
                        return;
                    }
                    let entries = match dir.entries() {
                        Ok(entries) => entries,
                        Err(source) => {
                            emit(Err(error(&location, source)));
                            return;
                        }
                    };
                    stack.push(WalkFrame {
                        parent: Arc::new(dir),
                        entries,
                        prefix: location,
                    });
                } else if !emit(Ok(WalkEntry::Directory(location))) {
                    return;
                }
            } else if metadata.is_file() {
                if staged_name(&name) {
                    continue;
                }
                let file = match open_regular(&frame.parent, &name, &location) {
                    Ok(file) => file,
                    Err(object_store::Error::NotFound { .. }) => continue,
                    Err(error) => {
                        emit(Err(error));
                        return;
                    }
                };
                let metadata = file.metadata();
                // A streaming consumer may pause in emit. Release the file first.
                drop(file);
                match metadata {
                    Ok(metadata) => {
                        if !emit(Ok(WalkEntry::File(object_meta(location, &metadata)))) {
                            return;
                        }
                    }
                    Err(source) => {
                        emit(Err(error(&location, source)));
                        return;
                    }
                }
            }
        }
    }
}

enum WalkEntry {
    File(ObjectMeta),
    Directory(Path),
}

#[async_trait]
impl ObjectStore for SecureLocalStore {
    async fn put_opts(
        &self,
        location: &Path,
        payload: PutPayload,
        options: PutOptions,
    ) -> Result<PutResult> {
        if !options.attributes.is_empty() {
            return Err(object_store::Error::NotSupported {
                source: "Local object attributes are not supported".into(),
            });
        }
        let store = self.clone();
        let location = location.clone();
        blocking(move || {
            let _guard = store
                .mutations
                .lock()
                .map_err(|_| generic("Local mutation lock poisoned"))?;
            let mut stage = store.stage(&location)?;
            let file = stage.file.as_mut().expect("new staging file");
            for chunk in payload {
                file.write_all(&chunk)
                    .map_err(|source| error(&location, source))?;
            }
            store.publish(&mut stage, &options.mode)
        })
        .await
    }

    async fn put_multipart_opts(
        &self,
        location: &Path,
        options: PutMultipartOptions,
    ) -> Result<Box<dyn MultipartUpload>> {
        if !options.attributes.is_empty() {
            return Err(object_store::Error::NotSupported {
                source: "Local object attributes are not supported".into(),
            });
        }
        let store = self.clone();
        let location = location.clone();
        let stage = blocking(move || store.stage(&location)).await?;
        Ok(Box::new(Upload {
            store: self.clone(),
            state: Arc::new(Mutex::new(UploadState {
                stage: Some(stage),
                completed_parts: 0,
                failed: false,
            })),
            next_offset: 0,
            parts: 0,
        }))
    }

    async fn get_opts(&self, location: &Path, options: GetOptions) -> Result<GetResult> {
        let store = self.clone();
        let location = location.clone();
        blocking(move || {
            let file = store.read(&location)?;
            let metadata = file.metadata().map_err(|source| error(&location, source))?;
            let meta = object_meta(location.clone(), &metadata);
            options.check_preconditions(&meta)?;
            let range = match options.range {
                Some(range) => range
                    .as_range(meta.size)
                    .map_err(|source| generic(source.to_string()))?,
                None => 0..meta.size,
            };
            Ok(GetResult {
                payload: GetResultPayload::File(file, PathBuf::from(location.as_ref())),
                meta,
                range,
                attributes: Attributes::default(),
            })
        })
        .await
    }

    fn delete_stream(
        &self,
        locations: BoxStream<'static, Result<Path>>,
    ) -> BoxStream<'static, Result<Path>> {
        let store = self.clone();
        locations
            .map(move |location| {
                let store = store.clone();
                async move {
                    let location = location?;
                    blocking(move || {
                        store.delete(&location)?;
                        Ok(location)
                    })
                    .await
                }
            })
            .buffered(10)
            .boxed()
    }

    fn list(&self, prefix: Option<&Path>) -> BoxStream<'static, Result<ObjectMeta>> {
        let store = self.clone();
        let prefix = prefix.cloned().unwrap_or_default();
        stream::once(async move {
            let (send, receive) = tokio::sync::mpsc::channel(32);
            let errors = send.clone();
            let walk = move || {
                store.walk(&prefix, true, |entry| match entry {
                    Ok(WalkEntry::File(meta)) => send.blocking_send(Ok(meta)).is_ok(),
                    Ok(WalkEntry::Directory(_)) => true,
                    Err(error) => send.blocking_send(Err(error)).is_ok(),
                });
            };
            if tokio::runtime::Handle::try_current().is_ok() {
                tokio::task::spawn_blocking(walk);
            } else if let Err(source) = std::thread::Builder::new()
                .name("local-store-list".into())
                .spawn(walk)
            {
                let _ = errors
                    .send(Err(generic(format!(
                        "Could not start local listing: {source}"
                    ))))
                    .await;
            }
            receive
        })
        .flat_map(|receive| {
            stream::unfold(receive, |mut receive| async {
                receive.recv().await.map(|entry| (entry, receive))
            })
        })
        .boxed()
    }

    async fn list_with_delimiter(&self, prefix: Option<&Path>) -> Result<ListResult> {
        let store = self.clone();
        let prefix = prefix.cloned().unwrap_or_default();
        blocking(move || {
            let mut objects = Vec::new();
            let mut directories = BTreeSet::new();
            let mut failure = None;
            store.walk(&prefix, false, |entry| {
                match entry {
                    Ok(WalkEntry::File(meta)) => objects.push(meta),
                    Ok(WalkEntry::Directory(path)) => {
                        directories.insert(path);
                    }
                    Err(error) => {
                        failure = Some(error);
                        return false;
                    }
                }
                true
            });
            if let Some(error) = failure {
                return Err(error);
            }
            Ok(ListResult {
                common_prefixes: directories.into_iter().collect(),
                objects,
            })
        })
        .await
    }

    async fn copy_opts(&self, from: &Path, to: &Path, options: CopyOptions) -> Result<()> {
        let store = self.clone();
        let from = from.clone();
        let to = to.clone();
        blocking(move || {
            let _guard = store
                .mutations
                .lock()
                .map_err(|_| generic("Local mutation lock poisoned"))?;
            let mut source = store.read(&from)?;
            let mut stage = store.stage(&to)?;
            io::copy(&mut source, stage.file.as_mut().expect("new staging file"))
                .map_err(|source| error(&to, source))?;
            store.publish(
                &mut stage,
                &if options.mode == CopyMode::Create {
                    PutMode::Create
                } else {
                    PutMode::Overwrite
                },
            )?;
            Ok(())
        })
        .await
    }

    async fn rename_opts(&self, from: &Path, to: &Path, options: RenameOptions) -> Result<()> {
        let store = self.clone();
        let from = from.clone();
        let to = to.clone();
        blocking(move || {
            let _guard = store
                .mutations
                .lock()
                .map_err(|_| generic("Local mutation lock poisoned"))?;
            let (source_parent, source_name) = store.parent(&from, false)?;
            let source_file = open_regular(&source_parent, &source_name, &from)?;
            let (destination_parent, destination_name) = store.parent(&to, true)?;
            symlink_check(&destination_parent, &destination_name, &to, true)?;
            drop(source_file);
            if options.target_mode == RenameTargetMode::Create {
                rename_create(
                    &source_parent,
                    &source_name,
                    &destination_parent,
                    &destination_name,
                )
                .map_err(|source| error(&to, source))?;
            } else {
                source_parent
                    .rename(&source_name, &destination_parent, &destination_name)
                    .map_err(|source| error(&to, source))?;
            }
            Ok(())
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::{SecureLocalStore, windows_alias};
    use futures::TryStreamExt;
    use object_store::{ObjectStore, path::Path};

    #[test]
    fn listing_runs_without_a_tokio_runtime() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("file.txt"), b"content").unwrap();
        let store = SecureLocalStore::new(root.path()).unwrap();
        let files = futures::executor::block_on(store.list(None).try_collect::<Vec<_>>()).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].location, Path::from("file.txt"));
    }

    #[test]
    fn listing_a_regular_file_prefix_returns_no_entries() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("file.txt"), b"content").unwrap();
        let store = SecureLocalStore::new(root.path()).unwrap();
        let files = futures::executor::block_on(
            store
                .list(Some(&Path::from("file.txt")))
                .try_collect::<Vec<_>>(),
        )
        .unwrap();
        assert!(files.is_empty());
        let children =
            futures::executor::block_on(store.list_with_delimiter(Some(&Path::from("file.txt"))))
                .unwrap();
        assert!(children.objects.is_empty());
        assert!(children.common_prefixes.is_empty());
    }

    #[test]
    fn windows_segments_reject_filesystem_aliases() {
        for name in [
            ".. ",
            "file.",
            "file ",
            "stream:data",
            "NUL",
            "NUL.txt",
            "COM1",
            "LPT².txt",
            "CONIN$",
        ] {
            assert!(windows_alias(name), "{name}");
        }
        for name in ["normal", "nulled", "com10", "names.with.dots"] {
            assert!(!windows_alias(name), "{name}");
        }
    }
}
